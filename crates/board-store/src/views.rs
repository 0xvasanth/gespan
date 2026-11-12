//! Finding and organising cards: the one query engine both apps call, the
//! board-wide card index it reads, and saved views (`board_views`).
//!
//! The engine never reads the clock or the time zone. Callers pass `now_ms`
//! and a local-day function (normally [`local_day_chrono`]), so date filters
//! are the same in both apps and testable across midnight and DST changes.
use crate::db::{args, err, query, with_db, with_tx, Conn, Ex, Row};
use crate::meta::{card_labels_in_order, priority_of};
use crate::models::{
    BoardRow, CardProjectRow, CardRow, ColumnRow, FieldRow, FieldValueRow, LabelRow, LinkRow,
    SettingsRow, ViewRow,
};
use crate::now_millis;
use crate::ops::{clean_text, count, new_id};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_VIEW_NAME: usize = 60;
const MAX_VIEW_JSON: usize = 100_000;
const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

/// What a view shows: a search, filter rows (ANDed), and a sort.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewSpec {
    pub query: String,
    pub rows: Vec<FilterRow>,
    pub sort: SortSpec,
}

/// One "Where" / "and" row. Its values OR together.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterRow {
    pub field: String,
    pub op: Op,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Is,
    IsNot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SortSpec {
    pub key: SortKey,
    pub dir: Dir,
}

/// `Manual` is column order, then position; it has no direction, so
/// `manual` + `desc` gives the same order as `manual` + `asc`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Manual,
    Priority,
    Due,
    Start,
    Title,
    Created,
    Updated,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    #[default]
    Asc,
    Desc,
}

/// Everything list and table layouts show for a live card.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardIndexRow {
    pub card_id: String,
    pub column_id: String,
    pub number: i64,
    pub title: String,
    pub priority: u8,
    pub due_at: Option<i64>,
    pub start_at: Option<i64>,
    pub estimate_minutes: Option<i64>,
    /// Label ids, in label order.
    pub labels: Vec<String>,
    /// Project paths, sorted.
    pub projects: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Field id → the stored JSON value, for fields still on the board.
    pub fields: BTreeMap<String, String>,
}

/// A card as the engine sees it: its index row plus what only filters need.
struct Entry {
    row: CardIndexRow,
    body: String,
    role: Option<String>,
    has_links: bool,
    /// Kind of the newest started/needs_input/completed/failed activity row for the
    /// card, on any board (a card moved here keeps its history).
    last_run: Option<String>,
}

/// The newest started/needs_input/completed/failed activity row per card of
/// a board, in one windowed read over the whole activity log, so any number
/// of newer comments or moves can't hide it. Rows logged on another board
/// count too: a card moved here keeps its history.
const LATEST_RUN_SQL: &str = "SELECT card_id, kind FROM (
       SELECT card_id, kind, ROW_NUMBER() OVER (
         PARTITION BY card_id ORDER BY created_at DESC, rowid DESC) AS newest
       FROM board_activity
       WHERE card_id IN (SELECT id FROM board_cards WHERE board_id = ?1)
         AND kind IN ('started', 'needs_input', 'completed', 'failed'))
     WHERE newest = 1";

/// Card id to the kind of its newest run event (`started`, `needs_input`,
/// `completed` or `failed`), for every card of the board that has one; the
/// same rule the `linked: failed` filter uses. Read-only.
pub fn latest_run_events(conn: &Conn, board_id: &str) -> Result<HashMap<String, String>, String> {
    let pairs = with_db!(conn, |ex| pairs(ex, LATEST_RUN_SQL, board_id).await)?;
    Ok(pairs.into_iter().collect())
}

/// The (card, value) rows of a board-wide read keyed by `?1` = board id.
async fn pairs(
    ex: &mut Ex<'_>,
    sql: &str,
    board_id: &str,
) -> Result<Vec<(String, String)>, String> {
    query(ex, sql, args![board_id])
        .await?
        .into_iter()
        .map(|mut r| Ok((r.take::<String>(0)?, r.take::<String>(1)?)))
        .collect()
}

/// Every live card on the board in manual order (column order, then
/// position), read board-wide: one statement per table, never one per card.
async fn load(ex: &mut Ex<'_>, board_id: &str, with_runs: bool) -> Result<Vec<Entry>, String> {
    // Raw SQL: columns tied on position keep insertion order (`rowid`), which
    // Toasty 0.11 cannot order by.
    let mut entries = query(
        ex,
        "SELECT c.id, c.column_id, c.number, c.title, c.body, c.created_at, c.updated_at,
                col.role, m.priority, m.due_at, m.start_at, m.estimate_minutes
         FROM board_cards c
         JOIN board_columns col ON col.id = c.column_id
         LEFT JOIN board_card_meta m ON m.card_id = c.id
         WHERE c.board_id = ?1 AND m.archived_at IS NULL
         ORDER BY col.position, col.rowid, c.position, c.number",
        args![board_id],
    )
    .await?
    .into_iter()
    .map(|mut r: Row| {
        Ok(Entry {
            row: CardIndexRow {
                card_id: r.take(0)?,
                column_id: r.take(1)?,
                number: r.take(2)?,
                title: r.take(3)?,
                created_at: r.take(5)?,
                updated_at: r.take(6)?,
                priority: priority_of(r.take::<Option<i64>>(8)?.unwrap_or(0)),
                due_at: r.take(9)?,
                start_at: r.take(10)?,
                estimate_minutes: r.take(11)?,
                ..CardIndexRow::default()
            },
            body: r.take(4)?,
            role: r.take(7)?,
            has_links: false,
            last_run: None,
        })
    })
    .collect::<Result<Vec<_>, String>>()?;
    let at: HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.row.card_id.clone(), index))
        .collect();
    let entry = |card: &str| at.get(card).copied();

    let card = CardRow::fields();
    let scope = || CardRow::filter(card.board_id().eq(board_id)).select(card.id());
    for (card, label) in card_labels_in_order(ex, scope()).await? {
        if let Some(index) = entry(&card) {
            entries[index].row.labels.push(label);
        }
    }
    let project = CardProjectRow::fields();
    let projects: Vec<(String, String)> =
        CardProjectRow::filter(project.card_id().in_query(scope()))
            .order_by(project.project().asc())
            .select((project.card_id(), project.project()))
            .exec(ex)
            .await
            .map_err(err)?;
    for (card, project) in projects {
        if let Some(index) = entry(&card) {
            entries[index].row.projects.push(project);
        }
    }
    let field = FieldRow::fields();
    let board_fields = FieldRow::filter(field.board_id().eq(board_id)).select(field.id());
    let value = FieldValueRow::fields();
    let values: Vec<(String, String, String)> =
        FieldValueRow::filter(value.field_id().in_query(board_fields))
            .select((value.card_id(), value.field_id(), value.value_json()))
            .exec(ex)
            .await
            .map_err(err)?;
    for (card, field, json) in values {
        if let Some(index) = entry(&card) {
            entries[index].row.fields.insert(field, json);
        }
    }
    if with_runs {
        // A link an older build wrote only into the card's legacy
        // `link_kind`/`link_id` columns counts too, as it does for
        // `board_links` and `cards_for_link`.
        let link = LinkRow::fields();
        let mut linked: Vec<String> = LinkRow::filter(link.card_id().in_query(scope()))
            .select(link.card_id())
            .exec(ex)
            .await
            .map_err(err)?;
        linked.extend(
            CardRow::filter(
                card.board_id()
                    .eq(board_id)
                    .and(card.link_kind().is_some())
                    .and(card.link_id().is_some()),
            )
            .select(card.id())
            .exec(ex)
            .await
            .map_err(err)?,
        );
        for card in linked {
            if let Some(index) = entry(&card) {
                entries[index].has_links = true;
            }
        }
        // `ROW_NUMBER() OVER`, so raw SQL.
        let runs = pairs(ex, LATEST_RUN_SQL, board_id).await?;
        for (card, kind) in runs {
            if let Some(index) = entry(&card) {
                entries[index].last_run = Some(kind);
            }
        }
    }
    Ok(entries)
}

/// Every live card's index row, in manual order, in one board-wide read.
pub fn card_index(conn: &Conn, board_id: &str) -> Result<Vec<CardIndexRow>, String> {
    Ok(with_db!(conn, |ex| load(ex, board_id, false).await)?
        .into_iter()
        .map(|entry| entry.row)
        .collect())
}

/// A filter row with its values checked against the board as it is now.
enum Test {
    Status(HashSet<String>),
    Priority(HashSet<u8>),
    Labels(HashSet<String>),
    Projects(HashSet<String>),
    Due(Vec<DueBucket>),
    Created(Vec<i64>),
    Updated(Vec<i64>),
    Linked(Vec<Linked>),
    Field(String, Vec<serde_json::Value>),
}

#[derive(Clone, Copy)]
enum DueBucket {
    Overdue,
    Today,
    Next7,
    Later,
    None,
}

/// `linked` values. `Failed` reads only the activity log: the newest run row
/// is `failed`, whether or not the card still has links.
#[derive(Clone, Copy)]
enum Linked {
    Running,
    Failed,
    NeedsReview,
    None,
}

struct BoardFacts {
    columns: HashSet<String>,
    labels: HashSet<String>,
    projects: HashSet<String>,
    /// Field id → its options, when it is a select or multi-select field with
    /// options.
    fields: HashMap<String, Option<Vec<String>>>,
}

async fn board_facts(
    ex: &mut Ex<'_>,
    board_id: &str,
    entries: &[Entry],
) -> Result<BoardFacts, String> {
    let field = FieldRow::fields();
    let mut fields = HashMap::new();
    for (id, kind, options) in FieldRow::filter(field.board_id().eq(board_id))
        .select((field.id(), field.kind(), field.options_json()))
        .exec(ex)
        .await
        .map_err(err)?
    {
        let (id, kind, options): (String, String, Option<String>) = (id, kind, options);
        let options = (kind == "select" || kind == "multi-select")
            .then(|| options.and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok()))
            .flatten();
        fields.insert(id, options);
    }
    let column = ColumnRow::fields();
    let columns = ColumnRow::filter(column.board_id().eq(board_id))
        .select(column.id())
        .exec(ex)
        .await
        .map_err(err)?;
    let label = LabelRow::fields();
    let labels = LabelRow::filter(label.board_id().eq(board_id))
        .select(label.id())
        .exec(ex)
        .await
        .map_err(err)?;
    Ok(BoardFacts {
        columns: columns.into_iter().collect(),
        labels: labels.into_iter().collect(),
        projects: entries
            .iter()
            .flat_map(|entry| entry.row.projects.iter().cloned())
            .collect(),
        fields,
    })
}

/// The row's test, keeping only values that still exist. `None` when no
/// value is left (or the field is unknown): the row is then ignored rather
/// than matching nothing.
fn compile(row: &FilterRow, facts: &BoardFacts) -> Option<Test> {
    let values = row.values.iter().map(|value| value.trim());
    let known = |set: &HashSet<String>| -> HashSet<String> {
        values
            .clone()
            .filter(|value| set.contains(*value))
            .map(str::to_string)
            .collect()
    };
    let window = |value: &str| match value {
        "24h" => Some(24 * HOUR_MS),
        "7d" => Some(7 * DAY_MS),
        "30d" => Some(30 * DAY_MS),
        _ => None,
    };
    let test = match row.field.as_str() {
        "status" => Test::Status(known(&facts.columns)),
        "labels" => Test::Labels(known(&facts.labels)),
        "projects" => Test::Projects(known(&facts.projects)),
        "priority" => Test::Priority(
            values
                .filter_map(|value| value.parse::<u8>().ok())
                .filter(|&p| (p as usize) < crate::PRIORITIES.len())
                .collect(),
        ),
        "due" => Test::Due(
            values
                .filter_map(|value| match value {
                    "overdue" => Some(DueBucket::Overdue),
                    "today" => Some(DueBucket::Today),
                    "next7" => Some(DueBucket::Next7),
                    "later" => Some(DueBucket::Later),
                    "none" => Some(DueBucket::None),
                    _ => None,
                })
                .collect(),
        ),
        "created" => Test::Created(values.filter_map(window).collect()),
        "updated" => Test::Updated(values.filter_map(window).collect()),
        "linked" => Test::Linked(
            values
                .filter_map(|value| match value {
                    "running" => Some(Linked::Running),
                    "failed" => Some(Linked::Failed),
                    "needs_review" => Some(Linked::NeedsReview),
                    "none" => Some(Linked::None),
                    _ => None,
                })
                .collect(),
        ),
        field => {
            let id = field.strip_prefix("field:")?;
            let options = facts.fields.get(id)?;
            let parsed: Vec<serde_json::Value> = values
                .filter_map(|value| serde_json::from_str(value).ok())
                .filter(|value: &serde_json::Value| match (options, value) {
                    (Some(options), serde_json::Value::String(text)) => options.contains(text),
                    _ => true,
                })
                .collect();
            Test::Field(id.to_string(), parsed)
        }
    };
    let empty = match &test {
        Test::Status(v) | Test::Labels(v) | Test::Projects(v) => v.is_empty(),
        Test::Priority(v) => v.is_empty(),
        Test::Due(v) => v.is_empty(),
        Test::Created(v) | Test::Updated(v) => v.is_empty(),
        Test::Linked(v) => v.is_empty(),
        Test::Field(_, v) => v.is_empty(),
    };
    (!empty).then_some(test)
}

/// The local calendar day (days since 1970-01-01) a moment falls on in the
/// machine's time zone, using the offset in force at that moment. A due date
/// stored at local midnight under summer time stays on its own day when read
/// in winter, which a single "today's offset" would get wrong.
pub fn local_day_chrono(ms: i64) -> i64 {
    use chrono::{Datelike, Local, TimeZone};
    match Local.timestamp_millis_opt(ms).earliest() {
        Some(at) => i64::from(at.date_naive().num_days_from_ce()) - DAYS_CE_TO_EPOCH,
        None => ms.div_euclid(DAY_MS),
    }
}

/// `num_days_from_ce` of 1970-01-01.
const DAYS_CE_TO_EPOCH: i64 = 719_163;

struct Now<'a> {
    ms: i64,
    today: i64,
    local_day: &'a dyn Fn(i64) -> i64,
    running: &'a HashSet<String>,
}

fn in_bucket(entry: &Entry, bucket: DueBucket, now: &Now<'_>) -> bool {
    let Some(due) = entry.row.due_at else {
        return matches!(bucket, DueBucket::None);
    };
    let day = (now.local_day)(due) - now.today;
    match bucket {
        DueBucket::Overdue => day < 0 && entry.role.as_deref() != Some("done"),
        DueBucket::Today => day == 0,
        DueBucket::Next7 => (1..=7).contains(&day),
        DueBucket::Later => day > 7,
        DueBucket::None => false,
    }
}

/// Whether any of the row's values holds for the card.
fn any_value(test: &Test, entry: &Entry, now: &Now<'_>) -> bool {
    let row = &entry.row;
    match test {
        Test::Status(columns) => columns.contains(&row.column_id),
        Test::Priority(priorities) => priorities.contains(&row.priority),
        Test::Labels(labels) => row.labels.iter().any(|label| labels.contains(label)),
        Test::Projects(projects) => row.projects.iter().any(|p| projects.contains(p)),
        Test::Due(buckets) => buckets.iter().any(|&bucket| in_bucket(entry, bucket, now)),
        Test::Created(windows) => windows.iter().any(|w| now.ms - row.created_at <= *w),
        Test::Updated(windows) => windows.iter().any(|w| now.ms - row.updated_at <= *w),
        Test::Linked(states) => states.iter().any(|state| match state {
            Linked::Running => now.running.contains(&row.card_id),
            Linked::Failed => entry.last_run.as_deref() == Some("failed"),
            Linked::NeedsReview => entry.role.as_deref() == Some("review"),
            Linked::None => !entry.has_links,
        }),
        // A multi-select value (an array) matches when it holds any wanted value.
        Test::Field(id, wanted) => row
            .fields
            .get(id)
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .is_some_and(|value| match &value {
                serde_json::Value::Array(items) => items
                    .iter()
                    .any(|item| wanted.iter().any(|w| same_value(w, item))),
                value => wanted.iter().any(|w| same_value(w, value)),
            }),
    }
}

/// JSON equality, except that numbers compare by value (5 equals 5.0).
fn same_value(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => a == b,
    }
}

fn matches_search(entry: &Entry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if let Some(number) = query.strip_prefix('#').and_then(|n| n.parse::<i64>().ok()) {
        return entry.row.number == number;
    }
    let query = query.to_lowercase();
    entry.row.title.to_lowercase().contains(&query) || entry.body.to_lowercase().contains(&query)
}

/// The sort value of a card; `None` sorts last in both directions.
fn sort_value(entry: &Entry, key: SortKey) -> Option<SortValue> {
    let row = &entry.row;
    match key {
        SortKey::Manual => None,
        SortKey::Priority => (row.priority > 0).then(|| SortValue::Int(i64::from(row.priority))),
        SortKey::Due => row.due_at.map(SortValue::Int),
        SortKey::Start => row.start_at.map(SortValue::Int),
        SortKey::Title => Some(SortValue::Text(row.title.to_lowercase())),
        SortKey::Created => Some(SortValue::Int(row.created_at)),
        SortKey::Updated => Some(SortValue::Int(row.updated_at)),
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SortValue {
    Int(i64),
    Text(String),
}

/// The ids of the board's live cards that match `spec`, in its order.
///
/// - `local_day` maps a moment (ms) to its local day number, DST-aware; due
///   buckets compare `local_day(due)` with `local_day(now_ms)`. Apps pass
///   [`local_day_chrono`]. The created/updated windows are plain durations
///   back from `now_ms`.
/// - `running` holds the cards whose linked session or Pilot runs now, which
///   only the app knows.
///
/// Filter values that no longer exist on the board are dropped, and a row
/// left with none is ignored.
pub fn query_cards(
    conn: &Conn,
    board_id: &str,
    spec: &ViewSpec,
    now_ms: i64,
    local_day: &dyn Fn(i64) -> i64,
    running: &HashSet<String>,
) -> Result<Vec<String>, String> {
    let needs_runs = spec.rows.iter().any(|row| row.field == "linked");
    let (entries, facts) = with_db!(conn, |ex| {
        let entries = load(ex, board_id, needs_runs).await?;
        let facts = board_facts(ex, board_id, &entries).await?;
        Ok((entries, facts))
    })?;
    let tests: Vec<(Op, Test)> = spec
        .rows
        .iter()
        .filter_map(|row| compile(row, &facts).map(|test| (row.op, test)))
        .collect();
    let now = Now {
        ms: now_ms,
        today: local_day(now_ms),
        local_day,
        running,
    };
    let query = spec.query.trim();
    let mut shown: Vec<(Option<SortValue>, Entry)> = entries
        .into_iter()
        .filter(|entry| matches_search(entry, query))
        .filter(|entry| {
            tests
                .iter()
                .all(|(op, test)| any_value(test, entry, &now) == (*op == Op::Is))
        })
        .map(|entry| (sort_value(&entry, spec.sort.key), entry))
        .collect();
    // A stable sort over cards already in manual order keeps ties in it.
    shown.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) => match spec.sort.dir {
            Dir::Asc => a.cmp(b),
            Dir::Desc => b.cmp(a),
        },
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    Ok(shown
        .into_iter()
        .map(|(_, entry)| entry.row.card_id)
        .collect())
}

/// A saved view: one tab after the built-in "All cards".
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub position: i64,
    pub spec_json: String,
    pub layout_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn check_spec(raw: &str) -> Result<(), String> {
    if raw.len() > MAX_VIEW_JSON {
        return Err("View spec is too long".into());
    }
    serde_json::from_str::<ViewSpec>(raw)
        .map(drop)
        .map_err(|_| "View spec is not valid".into())
}

fn check_layout(raw: &str) -> Result<(), String> {
    if raw.len() > MAX_VIEW_JSON {
        return Err("View layout is too long".into());
    }
    serde_json::from_str::<serde_json::Value>(raw)
        .map(drop)
        .map_err(|_| "View layout is not valid JSON".into())
}

fn view_from_row(r: &Row) -> Result<View, String> {
    Ok(View {
        id: r.get(0)?,
        board_id: r.get(1)?,
        name: r.get(2)?,
        position: r.get(3)?,
        spec_json: r.get(4)?,
        layout_json: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

impl From<ViewRow> for View {
    fn from(row: ViewRow) -> Self {
        View {
            id: row.id,
            board_id: row.board_id,
            name: row.name,
            position: row.position,
            spec_json: row.spec_json,
            layout_json: row.layout_json,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub(crate) async fn list_views_in(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<View>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        "SELECT id, board_id, name, position, spec_json, layout_json, created_at, updated_at
         FROM board_views WHERE board_id = ?1 ORDER BY position, rowid",
        args![board_id],
    )
    .await?
    .iter()
    .map(view_from_row)
    .collect()
}

pub fn list_views(conn: &Conn, board_id: &str) -> Result<Vec<View>, String> {
    with_db!(conn, |ex| list_views_in(ex, board_id).await)
}

async fn view_board(ex: &mut Ex<'_>, view_id: &str) -> Result<String, String> {
    ViewRow::filter_by_id(view_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(|view| view.board_id)
        .ok_or_else(|| "View not found".to_string())
}

async fn view_ids(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<String>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        "SELECT id FROM board_views WHERE board_id = ?1 ORDER BY position, rowid",
        args![board_id],
    )
    .await?
    .iter()
    .map(|r| r.get(0))
    .collect()
}

async fn write_view_positions(ex: &mut Ex<'_>, ids: &[String]) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        let view = ViewRow::fields();
        ViewRow::filter(view.id().eq(id).and(view.position().ne(index as i64)))
            .update()
            .position(index as i64)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    Ok(())
}

/// Adds a view at the end of the board's tabs.
pub fn create_view(
    conn: &Conn,
    board_id: &str,
    name: &str,
    spec_json: &str,
    layout_json: &str,
) -> Result<View, String> {
    let name = clean_text(name, "View name", MAX_VIEW_NAME)?;
    check_spec(spec_json)?;
    check_layout(layout_json)?;
    with_tx!(conn, |ex| {
        if count(ex, BoardRow::filter_by_id(board_id).count()).await? == 0 {
            return Err("Board not found".into());
        }
        let view = ViewRow::fields();
        let position = ViewRow::filter(view.board_id().eq(board_id))
            .order_by(view.position().desc())
            .select(view.position())
            .first()
            .exec(ex)
            .await
            .map_err(err)?
            .map_or(0, |last| last + 1);
        let now = now_millis();
        ViewRow::create()
            .id(new_id())
            .board_id(board_id)
            .name(name)
            .position(position)
            .spec_json(spec_json)
            .layout_json(layout_json)
            .created_at(now)
            .updated_at(now)
            .exec(ex)
            .await
            .map(View::from)
            .map_err(err)
    })
}

/// Changes the given parts of a view. Returns the owning board id.
pub fn update_view(
    conn: &Conn,
    view_id: &str,
    name: Option<&str>,
    spec_json: Option<&str>,
    layout_json: Option<&str>,
) -> Result<String, String> {
    let name = name
        .map(|name| clean_text(name, "View name", MAX_VIEW_NAME))
        .transpose()?;
    spec_json.map(check_spec).transpose()?;
    layout_json.map(check_layout).transpose()?;
    with_tx!(conn, |ex| {
        let Some(view) = ViewRow::filter_by_id(view_id)
            .first()
            .exec(ex)
            .await
            .map_err(err)?
        else {
            return Err("View not found".into());
        };
        // The parts not given keep their stored values.
        ViewRow::filter_by_id(view_id)
            .update()
            .name(name.unwrap_or(view.name))
            .spec_json(spec_json.map_or(view.spec_json, str::to_string))
            .layout_json(layout_json.map_or(view.layout_json, str::to_string))
            .updated_at(now_millis())
            .exec(ex)
            .await
            .map_err(err)?;
        Ok(view.board_id)
    })
}

/// Returns the owning board id. A board whose default view this was goes
/// back to "All cards".
pub fn delete_view(conn: &Conn, view_id: &str) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let board_id = view_board(ex, view_id).await?;
        ViewRow::filter_by_id(view_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        let settings = SettingsRow::fields();
        SettingsRow::filter(
            settings
                .board_id()
                .eq(&board_id)
                .and(settings.default_view_id().eq(Some(view_id.to_string()))),
        )
        .update()
        .default_view_id(None::<String>)
        .exec(ex)
        .await
        .map_err(err)?;
        let ids = view_ids(ex, &board_id).await?;
        write_view_positions(ex, &ids).await?;
        Ok(board_id)
    })
}

/// Moves a view to `index` (clamped) among its board's views. Returns the
/// owning board id.
pub fn move_view(conn: &Conn, view_id: &str, index: usize) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let board_id = view_board(ex, view_id).await?;
        let mut ids = view_ids(ex, &board_id).await?;
        ids.retain(|id| id != view_id);
        ids.insert(index.min(ids.len()), view_id.to_string());
        write_view_positions(ex, &ids).await?;
        Ok(board_id)
    })
}

/// Rewrites every saved view on the board inside the caller's transaction:
/// `edit` gets each view's parsed spec and layout and changes them in place.
/// Only views that changed are written. A view whose stored JSON doesn't
/// parse is left as it is (the engine already reads it as empty).
pub(crate) async fn rewrite_views(
    ex: &mut Ex<'_>,
    board_id: &str,
    mut edit: impl FnMut(&mut serde_json::Value, &mut serde_json::Value),
) -> Result<(), String> {
    let views = list_views_in(ex, board_id).await?;
    let now = now_millis();
    for view in views {
        let (Ok(mut spec), Ok(mut layout)) = (
            serde_json::from_str::<serde_json::Value>(&view.spec_json),
            serde_json::from_str::<serde_json::Value>(&view.layout_json),
        ) else {
            continue;
        };
        let (spec_before, layout_before) = (spec.clone(), layout.clone());
        edit(&mut spec, &mut layout);
        if spec == spec_before && layout == layout_before {
            continue;
        }
        ViewRow::filter_by_id(&view.id)
            .update()
            .spec_json(spec.to_string())
            .layout_json(layout.to_string())
            .updated_at(now)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    Ok(())
}

/// The filter rows of a parsed view spec, when it has any.
pub(crate) fn spec_rows(spec: &mut serde_json::Value) -> Option<&mut Vec<serde_json::Value>> {
    spec.get_mut("rows")
        .and_then(serde_json::Value::as_array_mut)
}

/// Whether a parsed filter row is on `field`.
pub(crate) fn row_is(row: &serde_json::Value, field: &str) -> bool {
    row.get("field").and_then(serde_json::Value::as_str) == Some(field)
}

/// Maps each string in a JSON array through `each` (`None` drops it), then
/// drops repeats, keeping the first. Not an array: left as it is.
pub(crate) fn map_strings(
    value: &mut serde_json::Value,
    mut each: impl FnMut(&str) -> Option<String>,
) {
    let Some(items) = value.as_array_mut() else {
        return;
    };
    let mut seen = HashSet::new();
    items.retain_mut(|item| {
        let Some(text) = item.as_str() else {
            return true;
        };
        match each(text) {
            Some(new) if seen.insert(new.clone()) => {
                *item = serde_json::Value::String(new);
                true
            }
            _ => false,
        }
    });
}
