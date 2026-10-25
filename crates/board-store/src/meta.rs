//! Per-card properties in `board_card_meta` and `board_card_projects`, archive,
//! and the per-card summary the board face shows.
use crate::ops::{
    card_from_row, column_card_ids, db, get_card, is_archived, log_edit, touch,
    write_card_positions, write_tx, LIVE,
};
use crate::{now_millis, Card};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

pub const PRIORITIES: [&str; 5] = ["None", "Low", "Medium", "High", "Urgent"];
const MAX_ESTIMATE_MINUTES: i64 = 100_000 * 60;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardMeta {
    pub priority: u8,
    pub due_at: Option<i64>,
    pub start_at: Option<i64>,
    pub estimate_minutes: Option<i64>,
    pub archived_at: Option<i64>,
    pub completed_at: Option<i64>,
}

/// What a card face shows besides the card itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardExtras {
    pub priority: u8,
    pub due_at: Option<i64>,
    pub projects: Vec<String>,
    /// Label ids, in label order.
    pub labels: Vec<String>,
    pub subtasks_done: usize,
    pub subtasks_total: usize,
    pub comments: usize,
}

fn priority_of(stored: i64) -> u8 {
    stored.clamp(0, PRIORITIES.len() as i64 - 1) as u8
}

/// A card without a meta row has the defaults.
pub fn card_meta(conn: &Connection, card_id: &str) -> Result<CardMeta, String> {
    let meta = conn
        .query_row(
            "SELECT priority, due_at, start_at, estimate_minutes, archived_at, completed_at
             FROM board_card_meta WHERE card_id = ?1",
            [card_id],
            |r| {
                Ok(CardMeta {
                    priority: priority_of(r.get(0)?),
                    due_at: r.get(1)?,
                    start_at: r.get(2)?,
                    estimate_minutes: r.get(3)?,
                    archived_at: r.get(4)?,
                    completed_at: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(db)?;
    Ok(meta.unwrap_or_default())
}

fn write_meta(
    tx: &Transaction<'_>,
    card_id: &str,
    column: &str,
    value: Option<i64>,
) -> Result<(), String> {
    tx.execute(
        &format!(
            "INSERT INTO board_card_meta (card_id, {column}) VALUES (?1, ?2)
             ON CONFLICT (card_id) DO UPDATE SET {column} = excluded.{column}"
        ),
        params![card_id, value],
    )
    .map(drop)
    .map_err(db)
}

/// Shared shape of the single-value meta setters: no row and no activity
/// when the value is unchanged. Returns the owning board id.
fn set_meta(
    conn: &Connection,
    card_id: &str,
    column: &str,
    value: Option<i64>,
    current: impl Fn(&CardMeta) -> Option<i64>,
    detail: impl Fn(Option<i64>) -> String,
) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    if current(&card_meta(&tx, card_id)?) == value {
        return Ok(card.board_id);
    }
    write_meta(&tx, card_id, column, value)?;
    touch(&tx, card_id)?;
    log_edit(&tx, &card.board_id, card_id, &detail(value))?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

fn day_detail(label: &str, at: Option<i64>) -> String {
    match at {
        Some(at) => format!("{label}: {}", crate::ops::day_label(at)),
        None => format!("{label}: none"),
    }
}

pub fn set_priority(conn: &Connection, card_id: &str, priority: u8) -> Result<String, String> {
    if PRIORITIES.get(priority as usize).is_none() {
        return Err(format!("Unknown priority: {priority}"));
    }
    // A missing row reads as priority 0, which a write of 0 leaves alone.
    set_meta(
        conn,
        card_id,
        "priority",
        Some(i64::from(priority)),
        |meta| Some(i64::from(meta.priority)),
        |_| format!("priority: {}", PRIORITIES[priority as usize]),
    )
}

pub fn set_due(conn: &Connection, card_id: &str, due_at: Option<i64>) -> Result<String, String> {
    set_meta(
        conn,
        card_id,
        "due_at",
        due_at,
        |meta| meta.due_at,
        |at| day_detail("due", at),
    )
}

pub fn set_start(
    conn: &Connection,
    card_id: &str,
    start_at: Option<i64>,
) -> Result<String, String> {
    set_meta(
        conn,
        card_id,
        "start_at",
        start_at,
        |meta| meta.start_at,
        |at| day_detail("start", at),
    )
}

pub fn set_estimate(
    conn: &Connection,
    card_id: &str,
    minutes: Option<i64>,
) -> Result<String, String> {
    if minutes.is_some_and(|m| !(1..=MAX_ESTIMATE_MINUTES).contains(&m)) {
        return Err("Estimate is out of range".into());
    }
    set_meta(
        conn,
        card_id,
        "estimate_minutes",
        minutes,
        |meta| meta.estimate_minutes,
        |minutes| match minutes {
            Some(minutes) => format!("estimate: {minutes}m"),
            None => "estimate: none".into(),
        },
    )
}

/// The values one card holds in a set table, such as its projects.
pub(crate) fn card_set(
    conn: &Connection,
    sql: &str,
    card_id: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = conn.prepare(sql).map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| r.get::<_, String>(0))
        .map_err(db)?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(db);
    rows
}

pub fn set_projects(
    conn: &Connection,
    card_id: &str,
    projects: &[String],
) -> Result<String, String> {
    let projects: BTreeSet<&str> = projects
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let current = card_set(
        &tx,
        "SELECT project FROM board_card_projects WHERE card_id = ?1",
        card_id,
    )?;
    if current
        .iter()
        .map(String::as_str)
        .eq(projects.iter().copied())
    {
        return Ok(card.board_id);
    }
    tx.execute(
        "DELETE FROM board_card_projects WHERE card_id = ?1",
        [card_id],
    )
    .map_err(db)?;
    for project in &projects {
        tx.execute(
            "INSERT INTO board_card_projects (card_id, project) VALUES (?1, ?2)",
            params![card_id, project],
        )
        .map_err(db)?;
    }
    touch(&tx, card_id)?;
    log_edit(
        &tx,
        &card.board_id,
        card_id,
        &format!("projects: {}", projects.len()),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Takes the card off the board view. Its column closes the gap; the card keeps
/// its column so unarchiving puts it back there. Returns the owning board id.
pub fn archive_card(conn: &Connection, card_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    if is_archived(&tx, card_id)? {
        return Ok(card.board_id);
    }
    write_meta(&tx, card_id, "archived_at", Some(now_millis()))?;
    let ids = column_card_ids(&tx, &card.column_id)?;
    write_card_positions(&tx, &ids)?;
    touch(&tx, card_id)?;
    log_edit(&tx, &card.board_id, card_id, "archived")?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Puts an archived card back at the end of its column. Returns the owning board id.
pub fn unarchive_card(conn: &Connection, card_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    if !is_archived(&tx, card_id)? {
        return Ok(card.board_id);
    }
    let mut ids = column_card_ids(&tx, &card.column_id)?;
    write_meta(&tx, card_id, "archived_at", None)?;
    ids.push(card.id.clone());
    write_card_positions(&tx, &ids)?;
    touch(&tx, card_id)?;
    log_edit(&tx, &card.board_id, card_id, "unarchived")?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// A board's archived cards, most recently archived first.
pub fn archived_cards(conn: &Connection, board_id: &str) -> Result<Vec<Card>, String> {
    let mut statement = conn
        .prepare(
            "SELECT c.* FROM board_cards c JOIN board_card_meta m ON m.card_id = c.id
             WHERE c.board_id = ?1 AND m.archived_at IS NOT NULL
             ORDER BY m.archived_at DESC, c.number DESC",
        )
        .map_err(db)?;
    let cards = statement
        .query_map([board_id], card_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    cards
}

/// Every live card's extras on a board, keyed by card id. Cards with none are absent.
pub fn card_extras(
    conn: &Connection,
    board_id: &str,
) -> Result<HashMap<String, CardExtras>, String> {
    let mut extras: HashMap<String, CardExtras> = HashMap::new();
    let scope = format!("SELECT id FROM board_cards WHERE board_id = ?1 AND {LIVE}");
    let mut meta = conn
        .prepare(&format!(
            "SELECT card_id, priority, due_at FROM board_card_meta WHERE card_id IN ({scope})"
        ))
        .map_err(db)?;
    for row in meta
        .query_map([board_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(db)?
    {
        let (card, priority, due_at) = row.map_err(db)?;
        let entry = extras.entry(card).or_default();
        entry.priority = priority_of(priority);
        entry.due_at = due_at;
    }
    let mut projects = conn
        .prepare(&format!(
            "SELECT card_id, project FROM board_card_projects WHERE card_id IN ({scope}) ORDER BY project"
        ))
        .map_err(db)?;
    for row in projects
        .query_map([board_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(db)?
    {
        let (card, project) = row.map_err(db)?;
        extras.entry(card).or_default().projects.push(project);
    }
    let mut labels = conn
        .prepare(&format!(
            "SELECT cl.card_id, cl.label_id FROM board_card_labels cl
             JOIN board_labels l ON l.id = cl.label_id
             WHERE cl.card_id IN ({scope}) ORDER BY l.position, lower(l.name)"
        ))
        .map_err(db)?;
    for row in labels
        .query_map([board_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(db)?
    {
        let (card, label) = row.map_err(db)?;
        extras.entry(card).or_default().labels.push(label);
    }
    let mut subtasks = conn
        .prepare(&format!(
            "SELECT card_id, SUM(done), COUNT(*) FROM board_subtasks
             WHERE card_id IN ({scope}) GROUP BY card_id"
        ))
        .map_err(db)?;
    for row in subtasks
        .query_map([board_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(db)?
    {
        let (card, done, total) = row.map_err(db)?;
        let entry = extras.entry(card).or_default();
        entry.subtasks_done = done.max(0) as usize;
        entry.subtasks_total = total.max(0) as usize;
    }
    let mut comments = conn
        .prepare(&format!(
            "SELECT card_id, COUNT(*) FROM board_comments
             WHERE card_id IN ({scope}) GROUP BY card_id"
        ))
        .map_err(db)?;
    for row in comments
        .query_map([board_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(db)?
    {
        let (card, count) = row.map_err(db)?;
        extras.entry(card).or_default().comments = count.max(0) as usize;
    }
    Ok(extras)
}
