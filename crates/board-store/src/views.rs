//! Finding and organising cards: the one query engine both apps call, the
//! board-wide card index it reads, and saved views (`board_views`).
//!
//! The engine never reads the clock or the time zone. Callers pass `now_ms`
//! and a local-day function (normally [`local_day_chrono`]), so date filters
//! are the same in both apps and testable across midnight and DST changes.
use crate::meta::priority_of;
use crate::now_millis;
use crate::ops::{clean_text, db, new_id, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
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
pub fn latest_run_events(
    conn: &Connection,
    board_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut events = HashMap::new();
    pairs(conn, LATEST_RUN_SQL, board_id, |card, kind| {
        events.insert(card, kind);
    })?;
    Ok(events)
}

fn pairs(
    conn: &Connection,
    sql: &str,
    board_id: &str,
    mut each: impl FnMut(String, String),
) -> Result<(), String> {
    let mut statement = conn.prepare(sql).map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(db)?;
    for row in rows {
        let (card, value) = row.map_err(db)?;
        each(card, value);
    }
    Ok(())
}

/// Every live card on the board in manual order (column order, then
/// position), read board-wide: one statement per table, never one per card.
fn load(conn: &Connection, board_id: &str, with_runs: bool) -> Result<Vec<Entry>, String> {
    let mut statement = conn
        .prepare(
            "SELECT c.id, c.column_id, c.number, c.title, c.body, c.created_at, c.updated_at,
                    col.role, m.priority, m.due_at, m.start_at, m.estimate_minutes
             FROM board_cards c
             JOIN board_columns col ON col.id = c.column_id
             LEFT JOIN board_card_meta m ON m.card_id = c.id
             WHERE c.board_id = ?1 AND m.archived_at IS NULL
             ORDER BY col.position, col.rowid, c.position, c.number",
        )
        .map_err(db)?;
    let mut entries = statement
        .query_map([board_id], |r| {
            Ok(Entry {
                row: CardIndexRow {
                    card_id: r.get(0)?,
                    column_id: r.get(1)?,
                    number: r.get(2)?,
                    title: r.get(3)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                    priority: priority_of(r.get::<_, Option<i64>>(8)?.unwrap_or(0)),
                    due_at: r.get(9)?,
                    start_at: r.get(10)?,
                    estimate_minutes: r.get(11)?,
                    ..CardIndexRow::default()
                },
                body: r.get(4)?,
                role: r.get(7)?,
                has_links: false,
                last_run: None,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    let at: HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.row.card_id.clone(), index))
        .collect();
    let entry = |card: &str| at.get(card).copied();

    let scope = "SELECT id FROM board_cards WHERE board_id = ?1";
    let mut labels = Vec::new();
    pairs(
        conn,
        &format!(
            "SELECT cl.card_id, cl.label_id FROM board_card_labels cl
             JOIN board_labels l ON l.id = cl.label_id
             WHERE cl.card_id IN ({scope}) ORDER BY l.position, lower(l.name)"
        ),
        board_id,
        |card, label| labels.push((card, label)),
    )?;
    for (card, label) in labels {
        if let Some(index) = entry(&card) {
            entries[index].row.labels.push(label);
        }
    }
    let mut projects = Vec::new();
    pairs(
        conn,
        &format!(
            "SELECT card_id, project FROM board_card_projects
             WHERE card_id IN ({scope}) ORDER BY project"
        ),
        board_id,
        |card, project| projects.push((card, project)),
    )?;
    for (card, project) in projects {
        if let Some(index) = entry(&card) {
            entries[index].row.projects.push(project);
        }
    }
    let mut statement = conn
        .prepare(
            "SELECT v.card_id, v.field_id, v.value_json FROM board_card_field_values v
             JOIN board_fields f ON f.id = v.field_id
             WHERE f.board_id = ?1",
        )
        .map_err(db)?;
    let values = statement
        .query_map([board_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(db)?;
    for value in values {
        let (card, field, json) = value.map_err(db)?;
        if let Some(index) = entry(&card) {
            entries[index].row.fields.insert(field, json);
        }
    }
    if with_runs {
        let mut linked = Vec::new();
        pairs(
            conn,
            &format!(
                "SELECT card_id, kind FROM board_card_links
                 WHERE card_id IN ({scope}) GROUP BY card_id"
            ),
            board_id,
            |card, _| linked.push(card),
        )?;
        for card in linked {
            if let Some(index) = entry(&card) {
                entries[index].has_links = true;
            }
        }
        let mut runs = Vec::new();
        pairs(conn, LATEST_RUN_SQL, board_id, |card, kind| {
            runs.push((card, kind))
        })?;
        for (card, kind) in runs {
            if let Some(index) = entry(&card) {
                entries[index].last_run = Some(kind);
            }
        }
    }
    Ok(entries)
}

/// Every live card's index row, in manual order, in one board-wide read.
pub fn card_index(conn: &Connection, board_id: &str) -> Result<Vec<CardIndexRow>, String> {
    Ok(load(conn, board_id, false)?
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

fn board_facts(conn: &Connection, board_id: &str, entries: &[Entry]) -> Result<BoardFacts, String> {
    let ids = |sql: &str| -> Result<HashSet<String>, String> {
        let mut statement = conn.prepare(sql).map_err(db)?;
        let rows = statement
            .query_map([board_id], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(db);
        rows
    };
    let mut fields = HashMap::new();
    let mut statement = conn
        .prepare("SELECT id, kind, options_json FROM board_fields WHERE board_id = ?1")
        .map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(db)?;
    for row in rows {
        let (id, kind, options) = row.map_err(db)?;
        let options = (kind == "select" || kind == "multi-select")
            .then(|| options.and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok()))
            .flatten();
        fields.insert(id, options);
    }
    Ok(BoardFacts {
        columns: ids("SELECT id FROM board_columns WHERE board_id = ?1")?,
        labels: ids("SELECT id FROM board_labels WHERE board_id = ?1")?,
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
    conn: &Connection,
    board_id: &str,
    spec: &ViewSpec,
    now_ms: i64,
    local_day: &dyn Fn(i64) -> i64,
    running: &HashSet<String>,
) -> Result<Vec<String>, String> {
    let needs_runs = spec.rows.iter().any(|row| row.field == "linked");
    let entries = load(conn, board_id, needs_runs)?;
    let facts = board_facts(conn, board_id, &entries)?;
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

fn view_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<View> {
    Ok(View {
        id: r.get("id")?,
        board_id: r.get("board_id")?,
        name: r.get("name")?,
        position: r.get("position")?,
        spec_json: r.get("spec_json")?,
        layout_json: r.get("layout_json")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

pub fn list_views(conn: &Connection, board_id: &str) -> Result<Vec<View>, String> {
    let mut statement = conn
        .prepare("SELECT * FROM board_views WHERE board_id = ?1 ORDER BY position, rowid")
        .map_err(db)?;
    let views = statement
        .query_map([board_id], view_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    views
}

fn view_board(conn: &Connection, view_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT board_id FROM board_views WHERE id = ?1",
        [view_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| "View not found".to_string())
}

fn view_ids(conn: &Connection, board_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM board_views WHERE board_id = ?1 ORDER BY position, rowid")
        .map_err(db)?;
    let ids = statement
        .query_map([board_id], |r| r.get(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(db);
    ids
}

fn write_view_positions(conn: &Connection, ids: &[String]) -> Result<(), String> {
    let mut statement = conn
        .prepare("UPDATE board_views SET position = ?1 WHERE id = ?2 AND position <> ?1")
        .map_err(db)?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![index as i64, id]).map_err(db)?;
    }
    Ok(())
}

/// Adds a view at the end of the board's tabs.
pub fn create_view(
    conn: &Connection,
    board_id: &str,
    name: &str,
    spec_json: &str,
    layout_json: &str,
) -> Result<View, String> {
    let name = clean_text(name, "View name", MAX_VIEW_NAME)?;
    check_spec(spec_json)?;
    check_layout(layout_json)?;
    let tx = write_tx(conn)?;
    let board_exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM boards WHERE id = ?1)",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    if !board_exists {
        return Err("Board not found".into());
    }
    let position: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM board_views WHERE board_id = ?1",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let now = now_millis();
    let view = View {
        id: new_id(),
        board_id: board_id.to_string(),
        name,
        position,
        spec_json: spec_json.to_string(),
        layout_json: layout_json.to_string(),
        created_at: now,
        updated_at: now,
    };
    tx.execute(
        "INSERT INTO board_views
           (id, board_id, name, position, spec_json, layout_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            view.id,
            view.board_id,
            view.name,
            view.position,
            view.spec_json,
            view.layout_json,
            view.created_at,
            view.updated_at
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(view)
}

/// Changes the given parts of a view. Returns the owning board id.
pub fn update_view(
    conn: &Connection,
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
    let tx = write_tx(conn)?;
    let board_id = view_board(&tx, view_id)?;
    tx.execute(
        "UPDATE board_views SET name = COALESCE(?1, name), spec_json = COALESCE(?2, spec_json),
           layout_json = COALESCE(?3, layout_json), updated_at = ?4
         WHERE id = ?5",
        params![name, spec_json, layout_json, now_millis(), view_id],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(board_id)
}

/// Returns the owning board id. A board whose default view this was goes
/// back to "All cards".
pub fn delete_view(conn: &Connection, view_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let board_id = view_board(&tx, view_id)?;
    tx.execute("DELETE FROM board_views WHERE id = ?1", [view_id])
        .map_err(db)?;
    tx.execute(
        "UPDATE board_settings SET default_view_id = NULL
         WHERE board_id = ?1 AND default_view_id = ?2",
        params![board_id, view_id],
    )
    .map_err(db)?;
    write_view_positions(&tx, &view_ids(&tx, &board_id)?)?;
    tx.commit().map_err(db)?;
    Ok(board_id)
}

/// Moves a view to `index` (clamped) among its board's views. Returns the
/// owning board id.
pub fn move_view(conn: &Connection, view_id: &str, index: usize) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let board_id = view_board(&tx, view_id)?;
    let mut ids = view_ids(&tx, &board_id)?;
    ids.retain(|id| id != view_id);
    ids.insert(index.min(ids.len()), view_id.to_string());
    write_view_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(board_id)
}

/// Rewrites every saved view on the board inside the caller's transaction:
/// `edit` gets each view's parsed spec and layout and changes them in place.
/// Only views that changed are written. A view whose stored JSON doesn't
/// parse is left as it is (the engine already reads it as empty).
pub(crate) fn rewrite_views(
    conn: &Connection,
    board_id: &str,
    mut edit: impl FnMut(&mut serde_json::Value, &mut serde_json::Value),
) -> Result<(), String> {
    let views = list_views(conn, board_id)?;
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
        conn.execute(
            "UPDATE board_views SET spec_json = ?1, layout_json = ?2, updated_at = ?3
             WHERE id = ?4",
            params![spec.to_string(), layout.to_string(), now, view.id],
        )
        .map_err(db)?;
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
