//! A deterministic large board for performance tests and manual scale checks
//! (`examples/seed_board.rs`). Not part of either app's behaviour.
use crate::ops::{db, new_id, write_tx, LIVE};
use crate::validate_id;
use rusqlite::{params, OptionalExtension};
use rusqlite::{Connection, Transaction};

/// 2026-10-26 00:00 UTC. Every seeded timestamp is an offset from it, so two
/// seeds hold the same data.
const SEED_EPOCH_MS: i64 = 1_793_404_800_000;
const DAY_MS: i64 = 86_400_000;
const LABEL_COLORS: [&str; 8] = [
    "e5534b", "4da3f5", "57ab5a", "c69026", "986ee2", "e0823d", "39c5cf", "8b949e",
];
const SELECT_OPTIONS: [&str; 4] = ["A", "B", "C", "D"];
const RUN_KINDS: [&str; 3] = ["started", "completed", "failed"];

/// What to seed. Counts of 0 seed none of that part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeedSpec {
    pub cards: usize,
    pub labels: usize,
    pub fields: usize,
    /// Every `link_every`-th card gets a session link (0: none).
    pub link_every: usize,
    pub activity_per_card: usize,
    pub subtasks_per_card: usize,
}

impl Default for SeedSpec {
    fn default() -> Self {
        Self {
            cards: 1000,
            labels: 8,
            fields: 2,
            link_every: 10,
            activity_per_card: 3,
            subtasks_per_card: 2,
        }
    }
}

/// The ids a seed created (columns: the board's columns, in order). Cards are
/// in seed order, card `n` at index `n - 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedReport {
    pub board_id: String,
    pub card_ids: Vec<String>,
    pub column_ids: Vec<String>,
    pub label_ids: Vec<String>,
    pub field_ids: Vec<String>,
}

/// Adds `spec.cards` cards, with labels, fields, links, activity and
/// subtasks, to an existing board in one write transaction. The content is
/// deterministic (only ids are random):
/// - card `n` (1-based) is titled "Card {n}" and goes to column
///   `(n - 1) % columns`, after that column's live cards;
/// - priority `n % 5`; every 7th card has a due date;
/// - label `n % labels`;
/// - field 0 (and every even field) is a select with options A B C D and holds
///   option `n % 4`; field 1 (and every odd field) is text holding "Note {n}";
/// - every `link_every`-th card has a `session` link `seed-{n}`;
/// - activity rows cycle started / completed / failed.
pub fn seed_board(
    conn: &Connection,
    board_id: &str,
    spec: &SeedSpec,
) -> Result<SeedReport, String> {
    validate_id(board_id, "board")?;
    let tx = write_tx(conn)?;
    let card_seq: i64 = tx
        .query_row(
            "SELECT card_seq FROM boards WHERE id = ?1",
            [board_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db)?
        .ok_or_else(|| "Board not found".to_string())?;
    let columns = board_columns(&tx, board_id)?;
    if columns.is_empty() && spec.cards > 0 {
        return Err("Board has no columns".into());
    }
    let label_ids = seed_labels(&tx, board_id, spec.labels)?;
    let fields = seed_fields(&tx, board_id, spec.fields)?;

    let mut card = tx
        .prepare(
            "INSERT INTO board_cards
               (id, board_id, column_id, number, title, body, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '', ?6, ?7, ?7)",
        )
        .map_err(db)?;
    let mut meta = tx
        .prepare("INSERT INTO board_card_meta (card_id, priority, due_at) VALUES (?1, ?2, ?3)")
        .map_err(db)?;
    let mut label = tx
        .prepare("INSERT INTO board_card_labels (card_id, label_id) VALUES (?1, ?2)")
        .map_err(db)?;
    let mut value = tx
        .prepare(
            "INSERT INTO board_card_field_values (card_id, field_id, value_json)
             VALUES (?1, ?2, ?3)",
        )
        .map_err(db)?;
    let mut link = tx
        .prepare(
            "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, 'session', ?2, NULL, ?3)",
        )
        .map_err(db)?;
    let mut activity = tx
        .prepare(
            "INSERT INTO board_activity (id, board_id, card_id, kind, actor, created_at)
             VALUES (?1, ?2, ?3, ?4, 'system', ?5)",
        )
        .map_err(db)?;
    let mut subtask = tx
        .prepare(
            "INSERT INTO board_subtasks (id, card_id, title, done, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(db)?;

    let mut next_position: Vec<i64> = columns.iter().map(|(_, live)| *live).collect();
    let mut card_ids = Vec::with_capacity(spec.cards);
    for n in 1..=spec.cards {
        let id = new_id();
        let at = (n - 1) % columns.len();
        let created_at = SEED_EPOCH_MS + n as i64 * 1_000;
        card.execute(params![
            id,
            board_id,
            columns[at].0,
            card_seq + n as i64,
            format!("Card {n}"),
            next_position[at],
            created_at,
        ])
        .map_err(db)?;
        next_position[at] += 1;
        let due_at = (n % 7 == 0).then(|| SEED_EPOCH_MS + ((n / 7) % 30) as i64 * DAY_MS);
        meta.execute(params![id, (n % 5) as i64, due_at])
            .map_err(db)?;
        if !label_ids.is_empty() {
            label
                .execute(params![id, label_ids[n % label_ids.len()]])
                .map_err(db)?;
        }
        for (field_id, is_select) in &fields {
            let json = if *is_select {
                serde_json::Value::from(SELECT_OPTIONS[n % SELECT_OPTIONS.len()])
            } else {
                serde_json::Value::from(format!("Note {n}"))
            };
            value
                .execute(params![id, field_id, json.to_string()])
                .map_err(db)?;
        }
        if spec.link_every > 0 && n % spec.link_every == 0 {
            link.execute(params![id, format!("seed-{n}"), created_at])
                .map_err(db)?;
        }
        for j in 0..spec.activity_per_card {
            activity
                .execute(params![
                    new_id(),
                    board_id,
                    id,
                    RUN_KINDS[(n + j) % RUN_KINDS.len()],
                    created_at + 1 + j as i64,
                ])
                .map_err(db)?;
        }
        for j in 0..spec.subtasks_per_card {
            subtask
                .execute(params![
                    new_id(),
                    id,
                    format!("Subtask {}", j + 1),
                    (j % 2 == 0) as i64,
                    j as i64,
                ])
                .map_err(db)?;
        }
        card_ids.push(id);
    }
    drop((card, meta, label, value, link, activity, subtask));
    tx.execute(
        "UPDATE boards SET card_seq = card_seq + ?1 WHERE id = ?2",
        params![spec.cards as i64, board_id],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(SeedReport {
        board_id: board_id.to_string(),
        card_ids,
        column_ids: columns.into_iter().map(|(id, _)| id).collect(),
        label_ids,
        field_ids: fields.into_iter().map(|(id, _)| id).collect(),
    })
}

/// The board's columns in order, each with its live card count.
fn board_columns(tx: &Transaction<'_>, board_id: &str) -> Result<Vec<(String, i64)>, String> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT col.id, (SELECT COUNT(*) FROM board_cards
                             WHERE column_id = col.id AND {LIVE})
             FROM board_columns col WHERE col.board_id = ?1
             ORDER BY col.position, col.rowid"
        ))
        .map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

/// Labels "Seed {i}" after the board's own, with a fixed palette.
fn seed_labels(tx: &Transaction<'_>, board_id: &str, count: usize) -> Result<Vec<String>, String> {
    let start: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM board_labels WHERE board_id = ?1",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let id = new_id();
        tx.execute(
            "INSERT INTO board_labels (id, board_id, name, color, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                board_id,
                format!("Seed {}", start + i as i64 + 1),
                LABEL_COLORS[i % LABEL_COLORS.len()],
                start + i as i64,
            ],
        )
        .map_err(db)?;
        ids.push(id);
    }
    Ok(ids)
}

/// Fields after the board's own: even ones select (A B C D), odd ones text.
fn seed_fields(
    tx: &Transaction<'_>,
    board_id: &str,
    count: usize,
) -> Result<Vec<(String, bool)>, String> {
    let start: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM board_fields WHERE board_id = ?1",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let options = serde_json::to_string(&SELECT_OPTIONS).map_err(|e| e.to_string())?;
    let mut fields = Vec::with_capacity(count);
    for i in 0..count {
        let id = new_id();
        let is_select = i % 2 == 0;
        let (name, kind, options) = if is_select {
            (format!("Stage {}", i + 1), "select", Some(options.as_str()))
        } else {
            (format!("Notes {}", i + 1), "text", None)
        };
        tx.execute(
            "INSERT INTO board_fields (id, board_id, name, kind, options_json, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, board_id, name, kind, options, start + i as i64],
        )
        .map_err(db)?;
        fields.push((id, is_select));
    }
    Ok(fields)
}
