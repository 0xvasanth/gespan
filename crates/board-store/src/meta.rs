//! Per-card properties in `board_card_meta` and `board_card_projects`, archive,
//! and the per-card summary the board face shows.
use crate::db::{args, err, query, with_db, with_tx, Conn, Ex};
use crate::models::{CardLabelRow, CardMetaRow, CardProjectRow, CardRow, LabelRow};
use crate::ops::{
    column_card_ids, count, get_card_in, get_column, is_archived, live, log_activity, log_edit,
    touch, write_card_positions, NewActivity, LIVE,
};
use crate::{now_millis, Card};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use toasty::stmt::{List, Query};

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

pub(crate) fn priority_of(stored: i64) -> u8 {
    stored.clamp(0, PRIORITIES.len() as i64 - 1) as u8
}

/// A card without a meta row has the defaults.
pub(crate) async fn card_meta_in(ex: &mut Ex<'_>, card_id: &str) -> Result<CardMeta, String> {
    let meta = CardMetaRow::filter_by_card_id(card_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?;
    Ok(meta
        .map(|row| CardMeta {
            priority: priority_of(row.priority),
            due_at: row.due_at,
            start_at: row.start_at,
            estimate_minutes: row.estimate_minutes,
            archived_at: row.archived_at,
            completed_at: row.completed_at,
        })
        .unwrap_or_default())
}

pub fn card_meta(conn: &Conn, card_id: &str) -> Result<CardMeta, String> {
    with_db!(conn, |ex| card_meta_in(ex, card_id).await)
}

/// A `board_card_meta` column [`write_meta`] sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MetaField {
    Priority,
    DueAt,
    StartAt,
    EstimateMinutes,
    ArchivedAt,
}

/// Sets one `board_card_meta` column, adding the row when the card has none.
/// A new row's priority is 0, as the hand-made table's default had it.
/// Priority is never empty.
pub(crate) async fn write_meta(
    ex: &mut Ex<'_>,
    card_id: &str,
    field: MetaField,
    value: Option<i64>,
) -> Result<(), String> {
    if field == MetaField::Priority && value.is_none() {
        return Err("Priority is required".into());
    }
    let exists = CardMetaRow::filter_by_card_id(card_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .is_some();
    if !exists {
        let pick = |wanted: MetaField| if field == wanted { value } else { None };
        return CardMetaRow::create()
            .card_id(card_id)
            .priority(pick(MetaField::Priority).unwrap_or(0))
            .due_at(pick(MetaField::DueAt))
            .start_at(pick(MetaField::StartAt))
            .estimate_minutes(pick(MetaField::EstimateMinutes))
            .archived_at(pick(MetaField::ArchivedAt))
            .completed_at(None::<i64>)
            .exec(ex)
            .await
            .map(drop)
            .map_err(err);
    }
    let row = CardMetaRow::filter_by_card_id(card_id).update();
    match field {
        MetaField::Priority => row.priority(value.unwrap_or(0)).exec(ex).await,
        MetaField::DueAt => row.due_at(value).exec(ex).await,
        MetaField::StartAt => row.start_at(value).exec(ex).await,
        MetaField::EstimateMinutes => row.estimate_minutes(value).exec(ex).await,
        MetaField::ArchivedAt => row.archived_at(value).exec(ex).await,
    }
    .map_err(err)
}

/// Shared shape of the single-value meta setters: no row and no activity
/// when the value is unchanged. Returns the owning board id.
fn set_meta(
    conn: &Conn,
    card_id: &str,
    field: MetaField,
    value: Option<i64>,
    current: impl Fn(&CardMeta) -> Option<i64> + Send + Sync,
    detail: impl Fn(Option<i64>) -> String + Send + Sync,
) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        if current(&card_meta_in(ex, card_id).await?) == value {
            return Ok(card.board_id);
        }
        write_meta(ex, card_id, field, value).await?;
        touch(ex, card_id).await?;
        log_edit(ex, &card.board_id, card_id, &detail(value)).await?;
        Ok(card.board_id)
    })
}

fn day_detail(label: &str, at: Option<i64>) -> String {
    match at {
        Some(at) => format!("{label}: {}", crate::ops::day_label(at)),
        None => format!("{label}: none"),
    }
}

pub fn set_priority(conn: &Conn, card_id: &str, priority: u8) -> Result<String, String> {
    if PRIORITIES.get(priority as usize).is_none() {
        return Err(format!("Unknown priority: {priority}"));
    }
    // A missing row reads as priority 0, which a write of 0 leaves alone.
    set_meta(
        conn,
        card_id,
        MetaField::Priority,
        Some(i64::from(priority)),
        |meta| Some(i64::from(meta.priority)),
        |_| format!("priority: {}", PRIORITIES[priority as usize]),
    )
}

pub fn set_due(conn: &Conn, card_id: &str, due_at: Option<i64>) -> Result<String, String> {
    set_meta(
        conn,
        card_id,
        MetaField::DueAt,
        due_at,
        |meta| meta.due_at,
        |at| day_detail("due", at),
    )
}

pub fn set_start(conn: &Conn, card_id: &str, start_at: Option<i64>) -> Result<String, String> {
    set_meta(
        conn,
        card_id,
        MetaField::StartAt,
        start_at,
        |meta| meta.start_at,
        |at| day_detail("start", at),
    )
}

pub fn set_estimate(conn: &Conn, card_id: &str, minutes: Option<i64>) -> Result<String, String> {
    if minutes.is_some_and(|m| !(1..=MAX_ESTIMATE_MINUTES).contains(&m)) {
        return Err("Estimate is out of range".into());
    }
    set_meta(
        conn,
        card_id,
        MetaField::EstimateMinutes,
        minutes,
        |meta| meta.estimate_minutes,
        |minutes| match minutes {
            Some(minutes) => format!("estimate: {minutes}m"),
            None => "estimate: none".into(),
        },
    )
}

/// The card's projects.
async fn card_projects(ex: &mut Ex<'_>, card_id: &str) -> Result<BTreeSet<String>, String> {
    Ok(CardProjectRow::filter_by_card_id(card_id)
        .select(CardProjectRow::fields().project())
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .collect())
}

/// The card's label ids.
pub(crate) async fn card_label_ids(
    ex: &mut Ex<'_>,
    card_id: &str,
) -> Result<BTreeSet<String>, String> {
    Ok(CardLabelRow::filter_by_card_id(card_id)
        .select(CardLabelRow::fields().label_id())
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .collect())
}

/// The names of the card's labels.
pub(crate) async fn card_label_names(
    ex: &mut Ex<'_>,
    card_id: &str,
) -> Result<BTreeSet<String>, String> {
    let label_ids =
        CardLabelRow::filter_by_card_id(card_id).select(CardLabelRow::fields().label_id());
    Ok(
        LabelRow::filter(LabelRow::fields().id().in_query(label_ids))
            .select(LabelRow::fields().name())
            .exec(ex)
            .await
            .map_err(err)?
            .into_iter()
            .collect(),
    )
}

pub fn set_projects(conn: &Conn, card_id: &str, projects: &[String]) -> Result<String, String> {
    let projects: BTreeSet<&str> = projects
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let current = card_projects(ex, card_id).await?;
        if current
            .iter()
            .map(String::as_str)
            .eq(projects.iter().copied())
        {
            return Ok(card.board_id);
        }
        CardProjectRow::filter_by_card_id(card_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        for project in &projects {
            CardProjectRow::create()
                .card_id(card_id)
                .project(*project)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        touch(ex, card_id).await?;
        log_edit(
            ex,
            &card.board_id,
            card_id,
            &format!("projects: {}", projects.len()),
        )
        .await?;
        Ok(card.board_id)
    })
}

/// Takes the card off the board view. Its column closes the gap; the card keeps
/// its column so unarchiving puts it back there. Returns the owning board id.
pub fn archive_card(conn: &Conn, card_id: &str) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        if is_archived(ex, card_id).await? {
            return Ok(card.board_id);
        }
        write_meta(ex, card_id, MetaField::ArchivedAt, Some(now_millis())).await?;
        let ids = column_card_ids(ex, &card.column_id).await?;
        write_card_positions(ex, &ids).await?;
        touch(ex, card_id).await?;
        log_edit(ex, &card.board_id, card_id, "archived").await?;
        Ok(card.board_id)
    })
}

/// Puts an archived card back at the end of its column. Returns the owning board id.
pub fn unarchive_card(conn: &Conn, card_id: &str) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        if !is_archived(ex, card_id).await? {
            return Ok(card.board_id);
        }
        let mut ids = column_card_ids(ex, &card.column_id).await?;
        write_meta(ex, card_id, MetaField::ArchivedAt, None).await?;
        ids.push(card.id.clone());
        write_card_positions(ex, &ids).await?;
        touch(ex, card_id).await?;
        log_edit(ex, &card.board_id, card_id, "unarchived").await?;
        Ok(card.board_id)
    })
}

/// Archives several cards in one transaction. Each column closes its gap once
/// and every card that changed gets an "archived" row; cards already archived
/// are skipped. All cards must be on one board. Returns that board id.
pub fn archive_cards(conn: &Conn, card_ids: &[String]) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let (board_id, cards) = cards_on_one_board(ex, card_ids, "No cards to archive").await?;
        let mut columns: Vec<String> = Vec::new();
        for card in &cards {
            if is_archived(ex, &card.id).await? {
                continue;
            }
            write_meta(ex, &card.id, MetaField::ArchivedAt, Some(now_millis())).await?;
            touch(ex, &card.id).await?;
            log_edit(ex, &board_id, &card.id, "archived").await?;
            if !columns.contains(&card.column_id) {
                columns.push(card.column_id.clone());
            }
        }
        for column_id in &columns {
            let ids = column_card_ids(ex, column_id).await?;
            write_card_positions(ex, &ids).await?;
        }
        Ok(board_id)
    })
}

/// The distinct cards named, in the order given, and the one board they are
/// all on. Errors when a card is missing, the cards span boards, or there are
/// none (with `none_error`).
pub(crate) async fn cards_on_one_board(
    ex: &mut Ex<'_>,
    card_ids: &[String],
    none_error: &str,
) -> Result<(String, Vec<Card>), String> {
    let mut cards: Vec<Card> = Vec::new();
    for card_id in card_ids {
        if cards.iter().any(|card| &card.id == card_id) {
            continue;
        }
        let card = get_card_in(ex, card_id).await?;
        if cards
            .first()
            .is_some_and(|first| first.board_id != card.board_id)
        {
            return Err("Cards are not on one board".into());
        }
        cards.push(card);
    }
    let Some(board_id) = cards.first().map(|card| card.board_id.clone()) else {
        return Err(none_error.into());
    };
    Ok((board_id, cards))
}

/// Sets one priority on several cards (all on one board) in one transaction,
/// with one "priority: …" row per card that changed. Returns the board id.
pub fn set_priority_many(conn: &Conn, card_ids: &[String], priority: u8) -> Result<String, String> {
    if PRIORITIES.get(priority as usize).is_none() {
        return Err(format!("Unknown priority: {priority}"));
    }
    with_tx!(conn, |ex| {
        let (board_id, cards) = cards_on_one_board(ex, card_ids, "No cards to edit").await?;
        let detail = format!("priority: {}", PRIORITIES[priority as usize]);
        for card in &cards {
            if card_meta_in(ex, &card.id).await?.priority == priority {
                continue;
            }
            write_meta(ex, &card.id, MetaField::Priority, Some(i64::from(priority))).await?;
            touch(ex, &card.id).await?;
            log_edit(ex, &board_id, &card.id, &detail).await?;
        }
        Ok(board_id)
    })
}

/// Unarchives a card and places it at `index` (clamped) in `column_id`, in one
/// transaction. Logs "unarchived", plus "moved" when the column differs from
/// the one the card was archived from. Returns the owning board id.
pub fn restore_card(
    conn: &Conn,
    card_id: &str,
    column_id: &str,
    index: usize,
) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let target = match get_column(ex, column_id).await {
            Ok(column) if column.board_id == card.board_id => column,
            _ => return Err("Column not on this board".into()),
        };
        let source = get_column(ex, &card.column_id).await?;
        let was_archived = is_archived(ex, card_id).await?;
        let same_column = card.column_id == column_id;
        if was_archived {
            write_meta(ex, card_id, MetaField::ArchivedAt, None).await?;
        }
        if !same_column {
            CardRow::filter_by_id(card_id)
                .update()
                .column_id(column_id)
                .exec(ex)
                .await
                .map_err(err)?;
            let source_ids = column_card_ids(ex, &card.column_id).await?;
            write_card_positions(ex, &source_ids).await?;
        }
        let mut ids = column_card_ids(ex, column_id).await?;
        ids.retain(|id| id != card_id);
        let at = index.min(ids.len());
        ids.insert(at, card_id.to_string());
        write_card_positions(ex, &ids).await?;
        touch(ex, card_id).await?;
        if was_archived {
            log_edit(ex, &card.board_id, card_id, "unarchived").await?;
        }
        if !same_column {
            log_activity(
                ex,
                NewActivity {
                    board_id: &card.board_id,
                    card_id: Some(card_id),
                    kind: "moved",
                    actor: "user",
                    from_column: Some(&source.name),
                    to_column: Some(&target.name),
                    detail: None,
                },
            )
            .await?;
        }
        Ok(card.board_id)
    })
}

/// A column's `(live, archived)` card counts.
pub fn column_card_count(conn: &Conn, column_id: &str) -> Result<(usize, usize), String> {
    with_db!(conn, |ex| {
        get_column(ex, column_id).await?;
        let card = CardRow::fields();
        let total = count(ex, CardRow::filter(card.column_id().eq(column_id)).count()).await?;
        let live = count(
            ex,
            CardRow::filter(card.column_id().eq(column_id).and(live())).count(),
        )
        .await?;
        let archived = total - live;
        Ok(((total - archived) as usize, archived as usize))
    })
}

/// Every column's `(live, archived)` card counts on a board, keyed by column
/// id, in one statement: each value equals [`column_card_count`] for that
/// column. A board with no columns gives an empty map.
pub fn column_card_counts(
    conn: &Conn,
    board_id: &str,
) -> Result<HashMap<String, (usize, usize)>, String> {
    let rows = with_db!(conn, |ex| {
        // Raw SQL: per-column counts in one statement need GROUP BY, which
        // Toasty 0.11 cannot express.
        query(
            ex,
            "SELECT b.id, col.id,
                    COUNT(c.id) - COUNT(m.archived_at), COUNT(m.archived_at)
             FROM boards b
             LEFT JOIN board_columns col ON col.board_id = b.id
             LEFT JOIN board_cards c ON c.column_id = col.id
             LEFT JOIN board_card_meta m ON m.card_id = c.id
             WHERE b.id = ?1
             GROUP BY b.id, col.id",
            args![board_id],
        )
        .await?
        .iter()
        .map(|r| {
            Ok((
                r.get::<Option<String>>(1)?,
                r.get::<i64>(2)?,
                r.get::<i64>(3)?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()
    })?;
    if rows.is_empty() {
        return Err("Board not found".into());
    }
    Ok(rows
        .into_iter()
        .filter_map(|(column, live, archived)| {
            column.map(|id| (id, (live.max(0) as usize, archived.max(0) as usize)))
        })
        .collect())
}

/// A board's archived cards, most recently archived first.
pub fn archived_cards(conn: &Conn, board_id: &str) -> Result<Vec<Card>, String> {
    with_db!(conn, |ex| {
        let card = CardRow::fields();
        let on_board = CardRow::filter(card.board_id().eq(board_id)).select(card.id());
        let meta = CardMetaRow::fields();
        let archived_at: HashMap<String, i64> = CardMetaRow::filter(
            meta.archived_at()
                .is_some()
                .and(meta.card_id().in_query(on_board)),
        )
        .select((meta.card_id(), meta.archived_at()))
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .filter_map(|(card_id, at): (String, Option<i64>)| Some((card_id, at?)))
        .collect();
        let mut cards: Vec<Card> = CardRow::filter(
            card.board_id().eq(board_id).and(
                card.id()
                    .in_list(archived_at.keys().cloned().collect::<Vec<_>>()),
            ),
        )
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .map(Card::from)
        .collect();
        cards.sort_by(|a, b| {
            (archived_at.get(&b.id), b.number).cmp(&(archived_at.get(&a.id), a.number))
        });
        Ok(cards)
    })
}

/// The (card, label) pairs of the cards `cards` selects, in label order:
/// position, then name ignoring ASCII case, as SQLite's `lower` compares it.
pub(crate) async fn card_labels_in_order(
    ex: &mut Ex<'_>,
    cards: Query<List<String>>,
) -> Result<Vec<(String, String)>, String> {
    let card_label = CardLabelRow::fields();
    let pairs: Vec<(String, String)> = CardLabelRow::filter(card_label.card_id().in_query(cards))
        .select((card_label.card_id(), card_label.label_id()))
        .exec(ex)
        .await
        .map_err(err)?;
    let used: BTreeSet<&String> = pairs.iter().map(|(_, label_id)| label_id).collect();
    let used: Vec<String> = used.into_iter().cloned().collect();
    let order: HashMap<String, (i64, String)> =
        LabelRow::filter(LabelRow::fields().id().in_list(used))
            .exec(ex)
            .await
            .map_err(err)?
            .into_iter()
            .map(|label| (label.id, (label.position, label.name.to_ascii_lowercase())))
            .collect();
    let mut pairs: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(_, label_id)| order.contains_key(label_id))
        .collect();
    pairs.sort_by(|a, b| order[&a.1].cmp(&order[&b.1]));
    Ok(pairs)
}

/// Every live card's extras on a board, keyed by card id. Cards with none are absent.
pub fn card_extras(conn: &Conn, board_id: &str) -> Result<HashMap<String, CardExtras>, String> {
    with_db!(conn, |ex| card_extras_in(ex, board_id).await)
}

async fn card_extras_in(
    ex: &mut Ex<'_>,
    board_id: &str,
) -> Result<HashMap<String, CardExtras>, String> {
    let mut extras: HashMap<String, CardExtras> = HashMap::new();
    let card = CardRow::fields();
    let scope = || CardRow::filter(card.board_id().eq(board_id).and(live())).select(card.id());
    let meta = CardMetaRow::fields();
    for (card_id, priority, due_at) in CardMetaRow::filter(meta.card_id().in_query(scope()))
        .select((meta.card_id(), meta.priority(), meta.due_at()))
        .exec(ex)
        .await
        .map_err(err)?
    {
        let entry = extras.entry(card_id).or_default();
        entry.priority = priority_of(priority);
        entry.due_at = due_at;
    }
    let project = CardProjectRow::fields();
    for (card_id, name) in CardProjectRow::filter(project.card_id().in_query(scope()))
        .order_by(project.project().asc())
        .select((project.card_id(), project.project()))
        .exec(ex)
        .await
        .map_err(err)?
    {
        extras.entry(card_id).or_default().projects.push(name);
    }
    for (card_id, label_id) in card_labels_in_order(ex, scope()).await? {
        extras.entry(card_id).or_default().labels.push(label_id);
    }
    // Raw SQL from here: per-card counts need GROUP BY, which Toasty 0.11
    // cannot express.
    let scope = format!("SELECT id FROM board_cards WHERE board_id = ?1 AND {LIVE}");
    for mut r in query(
        ex,
        &format!(
            "SELECT card_id, SUM(done), COUNT(*) FROM board_subtasks
             WHERE card_id IN ({scope}) GROUP BY card_id"
        ),
        args![board_id],
    )
    .await?
    {
        let entry = extras.entry(r.take::<String>(0)?).or_default();
        entry.subtasks_done = r.get::<i64>(1)?.max(0) as usize;
        entry.subtasks_total = r.get::<i64>(2)?.max(0) as usize;
    }
    for mut r in query(
        ex,
        &format!(
            "SELECT card_id, COUNT(*) FROM board_comments
             WHERE card_id IN ({scope}) GROUP BY card_id"
        ),
        args![board_id],
    )
    .await?
    {
        extras.entry(r.take::<String>(0)?).or_default().comments = r.get::<i64>(1)?.max(0) as usize;
    }
    Ok(extras)
}
