//! A deterministic large board for performance tests and manual scale checks
//! (`examples/seed_board.rs`). Not part of either app's behaviour.
use crate::db::{args, err, query, with_tx, Conn, Ex};
use crate::models::{
    ActivityRow, BoardRow, CardLabelRow, CardMetaRow, CardRow, FieldRow, FieldValueRow, LabelRow,
    LinkRow, SubtaskRow,
};
use crate::ops::{new_id, LIVE};
use crate::validate_id;

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
pub fn seed_board(conn: &Conn, board_id: &str, spec: &SeedSpec) -> Result<SeedReport, String> {
    validate_id(board_id, "board")?;
    with_tx!(conn, |ex| seed_board_in(ex, board_id, spec).await)
}

async fn seed_board_in(
    ex: &mut Ex<'_>,
    board_id: &str,
    spec: &SeedSpec,
) -> Result<SeedReport, String> {
    let card_seq: i64 = BoardRow::filter_by_id(board_id)
        .select(BoardRow::fields().card_seq())
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .ok_or_else(|| "Board not found".to_string())?;
    let columns = board_columns(ex, board_id).await?;
    if columns.is_empty() && spec.cards > 0 {
        return Err("Board has no columns".into());
    }
    let label_ids = seed_labels(ex, board_id, spec.labels).await?;
    let fields = seed_fields(ex, board_id, spec.fields).await?;

    let mut next_position: Vec<i64> = columns.iter().map(|(_, live)| *live).collect();
    let mut card_ids = Vec::with_capacity(spec.cards);
    for n in 1..=spec.cards {
        let id = new_id();
        let at = (n - 1) % columns.len();
        let created_at = SEED_EPOCH_MS + n as i64 * 1_000;
        CardRow::create()
            .id(&id)
            .board_id(board_id)
            .column_id(&columns[at].0)
            .number(card_seq + n as i64)
            .title(format!("Card {n}"))
            .body("")
            .position(next_position[at])
            .link_kind(None::<String>)
            .link_id(None::<String>)
            .created_at(created_at)
            .updated_at(created_at)
            .title_from_ticket(false)
            .body_from_ticket(false)
            .exec(ex)
            .await
            .map_err(err)?;
        next_position[at] += 1;
        let due_at = (n % 7 == 0).then(|| SEED_EPOCH_MS + ((n / 7) % 30) as i64 * DAY_MS);
        CardMetaRow::create()
            .card_id(&id)
            .priority((n % 5) as i64)
            .due_at(due_at)
            .start_at(None::<i64>)
            .estimate_minutes(None::<i64>)
            .archived_at(None::<i64>)
            .completed_at(None::<i64>)
            .exec(ex)
            .await
            .map_err(err)?;
        if !label_ids.is_empty() {
            CardLabelRow::create()
                .card_id(&id)
                .label_id(&label_ids[n % label_ids.len()])
                .exec(ex)
                .await
                .map_err(err)?;
        }
        for (field_id, is_select) in &fields {
            let json = if *is_select {
                serde_json::Value::from(SELECT_OPTIONS[n % SELECT_OPTIONS.len()])
            } else {
                serde_json::Value::from(format!("Note {n}"))
            };
            FieldValueRow::create()
                .card_id(&id)
                .field_id(field_id)
                .value_json(json.to_string())
                .exec(ex)
                .await
                .map_err(err)?;
        }
        if spec.link_every > 0 && n % spec.link_every == 0 {
            LinkRow::create()
                .card_id(&id)
                .kind("session")
                .reference(format!("seed-{n}"))
                .role(None::<String>)
                .created_at(created_at)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        for j in 0..spec.activity_per_card {
            ActivityRow::create()
                .id(new_id())
                .board_id(board_id)
                .card_id(Some(id.clone()))
                .kind(RUN_KINDS[(n + j) % RUN_KINDS.len()])
                .actor("system")
                .moved_from(None::<String>)
                .to_column(None::<String>)
                .detail(None::<String>)
                .created_at(created_at + 1 + j as i64)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        for j in 0..spec.subtasks_per_card {
            SubtaskRow::create()
                .id(new_id())
                .card_id(&id)
                .title(format!("Subtask {}", j + 1))
                .done(j % 2 == 0)
                .position(j as i64)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        card_ids.push(id);
    }
    BoardRow::filter_by_id(board_id)
        .update()
        .card_seq(toasty::stmt::add(spec.cards as i64))
        .exec(ex)
        .await
        .map_err(err)?;
    Ok(SeedReport {
        board_id: board_id.to_string(),
        card_ids,
        column_ids: columns.into_iter().map(|(id, _)| id).collect(),
        label_ids,
        field_ids: fields.into_iter().map(|(id, _)| id).collect(),
    })
}

/// The board's columns in order, each with its live card count.
async fn board_columns(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<(String, i64)>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        &format!(
            "SELECT col.id, (SELECT COUNT(*) FROM board_cards
                             WHERE column_id = col.id AND {LIVE})
             FROM board_columns col WHERE col.board_id = ?1
             ORDER BY col.position, col.rowid"
        ),
        args![board_id],
    )
    .await?
    .iter()
    .map(|r| Ok((r.get(0)?, r.get(1)?)))
    .collect()
}

/// Labels "Seed {i}" after the board's own, with a fixed palette.
async fn seed_labels(ex: &mut Ex<'_>, board_id: &str, count: usize) -> Result<Vec<String>, String> {
    let label = LabelRow::fields();
    let start = LabelRow::filter(label.board_id().eq(board_id))
        .order_by(label.position().desc())
        .select(label.position())
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map_or(0, |last| last + 1);
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let id = new_id();
        LabelRow::create()
            .id(&id)
            .board_id(board_id)
            .name(format!("Seed {}", start + i as i64 + 1))
            .color(LABEL_COLORS[i % LABEL_COLORS.len()])
            .position(start + i as i64)
            .exec(ex)
            .await
            .map_err(err)?;
        ids.push(id);
    }
    Ok(ids)
}

/// Fields after the board's own: even ones select (A B C D), odd ones text.
async fn seed_fields(
    ex: &mut Ex<'_>,
    board_id: &str,
    count: usize,
) -> Result<Vec<(String, bool)>, String> {
    let field = FieldRow::fields();
    let start = FieldRow::filter(field.board_id().eq(board_id))
        .order_by(field.position().desc())
        .select(field.position())
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map_or(0, |last| last + 1);
    let options = serde_json::to_string(&SELECT_OPTIONS).map_err(|e| e.to_string())?;
    let mut fields = Vec::with_capacity(count);
    for i in 0..count {
        let id = new_id();
        let is_select = i % 2 == 0;
        let (name, kind, options) = if is_select {
            (format!("Stage {}", i + 1), "select", Some(options.clone()))
        } else {
            (format!("Notes {}", i + 1), "text", None)
        };
        FieldRow::create()
            .id(&id)
            .board_id(board_id)
            .name(name)
            .kind(kind)
            .options_json(options)
            .position(start + i as i64)
            .exec(ex)
            .await
            .map_err(err)?;
        fields.push((id, is_select));
    }
    Ok(fields)
}
