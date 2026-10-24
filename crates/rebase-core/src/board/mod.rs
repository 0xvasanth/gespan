//! The board, over the same `board*` tables the Tauri app keeps in `rebase.db`.
//! The logic mirrors `src-tauri/src/board.rs`; `tests/shared_contracts.rs` pins
//! the DDL and error strings to it.
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

pub mod auto_move;
pub mod extras;
pub mod linked_turns;

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn validate_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("Invalid {label} id"));
    }
    Ok(())
}

const MAX_NAME: usize = 200;
const MAX_TITLE: usize = 500;
const MAX_BODY: usize = 1_000_000;
const MAX_ACTIVITY: usize = 500;

const ROLES: [&str; 4] = ["todo", "doing", "review", "done"];
const LINK_KINDS: [&str; 3] = ["session", "pilot", "flow-run"];
const ACTORS: [&str; 2] = ["user", "system"];
/// Kinds `board_card_record` may append. Every other kind is written by the
/// mutation that causes it.
const RECORD_KINDS: [&str; 3] = ["started", "completed", "failed"];
const DEFAULT_COLUMNS: [(&str, &str); 4] = [
    ("Backlog", "todo"),
    ("In Progress", "doing"),
    ("Review", "review"),
    ("Done", "done"),
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub position: i64,
    pub role: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub board_id: String,
    pub column_id: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub position: i64,
    pub link_kind: Option<String>,
    pub link_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    pub board_id: String,
    pub card_id: Option<String>,
    pub kind: String,
    pub actor: String,
    pub from_column: Option<String>,
    pub to_column: Option<String>,
    pub detail: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDetail {
    pub board: Board,
    pub columns: Vec<Column>,
    pub cards: Vec<Card>,
}

pub(crate) fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS boards (
           id TEXT PRIMARY KEY,
           name TEXT NOT NULL,
           card_seq INTEGER NOT NULL DEFAULT 0,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS board_columns (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           name TEXT NOT NULL,
           position INTEGER NOT NULL,
           role TEXT
         );
         CREATE UNIQUE INDEX IF NOT EXISTS board_columns_role_idx
           ON board_columns (board_id, role) WHERE role IS NOT NULL;
         CREATE TABLE IF NOT EXISTS board_cards (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           column_id TEXT NOT NULL REFERENCES board_columns(id),
           number INTEGER NOT NULL,
           title TEXT NOT NULL,
           body TEXT NOT NULL DEFAULT '',
           position INTEGER NOT NULL,
           link_kind TEXT,
           link_id TEXT,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           UNIQUE (board_id, number)
         );
         CREATE INDEX IF NOT EXISTS board_cards_column_idx
           ON board_cards (column_id, position);
         CREATE INDEX IF NOT EXISTS board_cards_link_idx
           ON board_cards (link_kind, link_id);
         CREATE TABLE IF NOT EXISTS board_activity (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           card_id TEXT REFERENCES board_cards(id) ON DELETE CASCADE,
           kind TEXT NOT NULL,
           actor TEXT NOT NULL,
           from_column TEXT,
           to_column TEXT,
           detail TEXT,
           created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS board_activity_board_idx
           ON board_activity (board_id, created_at DESC);
         CREATE INDEX IF NOT EXISTS board_activity_card_idx
           ON board_activity (card_id, created_at DESC);",
    )?;
    extras::ensure_tables(conn)
}

fn db(error: rusqlite::Error) -> String {
    error.to_string()
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn clean_text(value: &str, label: &str, max: usize) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    if trimmed.chars().count() > max {
        return Err(format!("{label} is too long"));
    }
    Ok(trimmed.to_string())
}

fn check_body(body: &str) -> Result<(), String> {
    if body.len() > MAX_BODY {
        return Err("Card body is too long".into());
    }
    Ok(())
}

fn check_role(role: Option<&str>) -> Result<(), String> {
    match role {
        Some(role) if !ROLES.contains(&role) => Err(format!("Unknown column role: {role}")),
        _ => Ok(()),
    }
}

fn card_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    Ok(Card {
        id: row.get("id")?,
        board_id: row.get("board_id")?,
        column_id: row.get("column_id")?,
        number: row.get("number")?,
        title: row.get("title")?,
        body: row.get("body")?,
        position: row.get("position")?,
        link_kind: row.get("link_kind")?,
        link_id: row.get("link_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn column_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Column> {
    Ok(Column {
        id: row.get("id")?,
        board_id: row.get("board_id")?,
        name: row.get("name")?,
        position: row.get("position")?,
        role: row.get("role")?,
    })
}

fn board_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Board> {
    Ok(Board {
        id: row.get("id")?,
        name: row.get("name")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn get_card(conn: &Connection, card_id: &str) -> Result<Card, String> {
    conn.query_row(
        "SELECT * FROM board_cards WHERE id = ?1",
        [card_id],
        card_from_row,
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| "Card not found".to_string())
}

fn get_column(conn: &Connection, column_id: &str) -> Result<Column, String> {
    conn.query_row(
        "SELECT * FROM board_columns WHERE id = ?1",
        [column_id],
        column_from_row,
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| "Column not found".to_string())
}

fn board_exists(conn: &Connection, board_id: &str) -> Result<(), String> {
    let found: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM boards WHERE id = ?1)",
            [board_id],
            |row| row.get(0),
        )
        .map_err(db)?;
    if found {
        Ok(())
    } else {
        Err("Board not found".into())
    }
}

fn column_card_ids(conn: &Connection, column_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM board_cards WHERE column_id = ?1 ORDER BY position, number")
        .map_err(db)?;
    let ids = statement
        .query_map([column_id], |row| row.get(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(db)?;
    Ok(ids)
}

fn board_column_ids(conn: &Connection, board_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM board_columns WHERE board_id = ?1 ORDER BY position, rowid")
        .map_err(db)?;
    let ids = statement
        .query_map([board_id], |row| row.get(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(db)?;
    Ok(ids)
}

/// Rewrite positions to 0..n-1 in the given order.
fn write_card_positions(tx: &Transaction<'_>, ids: &[String]) -> Result<(), String> {
    let mut statement = tx
        .prepare("UPDATE board_cards SET position = ?1 WHERE id = ?2 AND position <> ?1")
        .map_err(db)?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![index as i64, id]).map_err(db)?;
    }
    Ok(())
}

fn write_column_positions(tx: &Transaction<'_>, ids: &[String]) -> Result<(), String> {
    let mut statement = tx
        .prepare("UPDATE board_columns SET position = ?1 WHERE id = ?2 AND position <> ?1")
        .map_err(db)?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![index as i64, id]).map_err(db)?;
    }
    Ok(())
}

struct NewActivity<'a> {
    board_id: &'a str,
    card_id: Option<&'a str>,
    kind: &'a str,
    actor: &'a str,
    from_column: Option<&'a str>,
    to_column: Option<&'a str>,
    detail: Option<&'a str>,
}

fn log_activity(tx: &Transaction<'_>, entry: NewActivity<'_>) -> Result<(), String> {
    tx.execute(
        "INSERT INTO board_activity
           (id, board_id, card_id, kind, actor, from_column, to_column, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new_id(),
            entry.board_id,
            entry.card_id,
            entry.kind,
            entry.actor,
            entry.from_column,
            entry.to_column,
            entry.detail,
            now_millis(),
        ],
    )
    .map_err(db)?;
    Ok(())
}

fn ensure_role_free(
    tx: &Transaction<'_>,
    board_id: &str,
    role: Option<&str>,
    except_column: Option<&str>,
) -> Result<(), String> {
    check_role(role)?;
    let Some(role) = role else {
        return Ok(());
    };
    let taken: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM board_columns
               WHERE board_id = ?1 AND role = ?2 AND id <> ?3)",
            params![board_id, role, except_column.unwrap_or("")],
            |row| row.get(0),
        )
        .map_err(db)?;
    if taken {
        return Err(format!(
            "Another column on this board already has the {role} role"
        ));
    }
    Ok(())
}

pub(crate) fn list_boards(conn: &Connection) -> Result<Vec<Board>, String> {
    let mut statement = conn
        .prepare("SELECT * FROM boards ORDER BY created_at, rowid")
        .map_err(db)?;
    let boards = statement
        .query_map([], board_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    Ok(boards)
}

pub(crate) fn get_board(conn: &Connection, board_id: &str) -> Result<BoardDetail, String> {
    let board = conn
        .query_row(
            "SELECT * FROM boards WHERE id = ?1",
            [board_id],
            board_from_row,
        )
        .optional()
        .map_err(db)?
        .ok_or_else(|| "Board not found".to_string())?;
    let columns = conn
        .prepare("SELECT * FROM board_columns WHERE board_id = ?1 ORDER BY position, rowid")
        .map_err(db)?
        .query_map([board_id], column_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    let cards = conn
        .prepare("SELECT * FROM board_cards WHERE board_id = ?1 ORDER BY position, number")
        .map_err(db)?
        .query_map([board_id], card_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    Ok(BoardDetail {
        board,
        columns,
        cards,
    })
}

/// Board writes take SQLite's write lock before reading what they change, so a
/// second app (or connection) writing the same board waits out `busy_timeout`
/// instead of failing at once or acting on rows read before the other write.
fn write_tx(conn: &Connection) -> Result<Transaction<'_>, String> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate).map_err(db)
}

pub(crate) fn create_board(conn: &Connection, name: &str) -> Result<Board, String> {
    let name = clean_text(name, "Board name", MAX_NAME)?;
    let now = now_millis();
    let board = Board {
        id: new_id(),
        name,
        created_at: now,
        updated_at: now,
    };
    let tx = write_tx(conn)?;
    tx.execute(
        "INSERT INTO boards (id, name, card_seq, created_at, updated_at)
         VALUES (?1, ?2, 0, ?3, ?3)",
        params![board.id, board.name, now],
    )
    .map_err(db)?;
    for (position, (column_name, role)) in DEFAULT_COLUMNS.iter().enumerate() {
        tx.execute(
            "INSERT INTO board_columns (id, board_id, name, position, role)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_id(), board.id, column_name, position as i64, role],
        )
        .map_err(db)?;
    }
    tx.commit().map_err(db)?;
    Ok(board)
}

pub(crate) fn rename_board(conn: &Connection, board_id: &str, name: &str) -> Result<(), String> {
    let name = clean_text(name, "Board name", MAX_NAME)?;
    let changed = conn
        .execute(
            "UPDATE boards SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now_millis(), board_id],
        )
        .map_err(db)?;
    if changed == 0 {
        return Err("Board not found".into());
    }
    Ok(())
}

pub(crate) fn delete_board(conn: &Connection, board_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM boards WHERE id = ?1", [board_id])
        .map_err(db)?;
    Ok(())
}

pub(crate) fn create_column(
    conn: &Connection,
    board_id: &str,
    name: &str,
    role: Option<&str>,
) -> Result<Column, String> {
    let name = clean_text(name, "Column name", MAX_NAME)?;
    let tx = write_tx(conn)?;
    board_exists(&tx, board_id)?;
    ensure_role_free(&tx, board_id, role, None)?;
    let position: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM board_columns WHERE board_id = ?1",
            [board_id],
            |row| row.get(0),
        )
        .map_err(db)?;
    let column = Column {
        id: new_id(),
        board_id: board_id.to_string(),
        name,
        position,
        role: role.map(str::to_string),
    };
    tx.execute(
        "INSERT INTO board_columns (id, board_id, name, position, role)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            column.id,
            column.board_id,
            column.name,
            column.position,
            column.role
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(column)
}

/// Returns the owning board id.
pub(crate) fn update_column(
    conn: &Connection,
    column_id: &str,
    name: &str,
    role: Option<&str>,
) -> Result<String, String> {
    let name = clean_text(name, "Column name", MAX_NAME)?;
    let tx = write_tx(conn)?;
    let column = get_column(&tx, column_id)?;
    ensure_role_free(&tx, &column.board_id, role, Some(column_id))?;
    tx.execute(
        "UPDATE board_columns SET name = ?1, role = ?2 WHERE id = ?3",
        params![name, role, column_id],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(column.board_id)
}

/// Returns the owning board id.
pub(crate) fn move_column(
    conn: &Connection,
    column_id: &str,
    index: usize,
) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let column = get_column(&tx, column_id)?;
    let mut ids = board_column_ids(&tx, &column.board_id)?;
    ids.retain(|id| id != column_id);
    ids.insert(index.min(ids.len()), column_id.to_string());
    write_column_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(column.board_id)
}

/// Returns the owning board id.
pub(crate) fn delete_column(conn: &Connection, column_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let column = get_column(&tx, column_id)?;
    let cards: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM board_cards WHERE column_id = ?1",
            [column_id],
            |row| row.get(0),
        )
        .map_err(db)?;
    if cards > 0 {
        let plural = if cards == 1 { "card" } else { "cards" };
        return Err(format!("Column has {cards} {plural}"));
    }
    tx.execute("DELETE FROM board_columns WHERE id = ?1", [column_id])
        .map_err(db)?;
    let ids = board_column_ids(&tx, &column.board_id)?;
    write_column_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(column.board_id)
}

pub(crate) fn create_card(
    conn: &Connection,
    column_id: &str,
    title: &str,
    body: &str,
) -> Result<Card, String> {
    let title = clean_text(title, "Card title", MAX_TITLE)?;
    check_body(body)?;
    let tx = write_tx(conn)?;
    let column = get_column(&tx, column_id)?;
    tx.execute(
        "UPDATE boards SET card_seq = card_seq + 1 WHERE id = ?1",
        [&column.board_id],
    )
    .map_err(db)?;
    let number: i64 = tx
        .query_row(
            "SELECT card_seq FROM boards WHERE id = ?1",
            [&column.board_id],
            |row| row.get(0),
        )
        .map_err(db)?;
    let position: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM board_cards WHERE column_id = ?1",
            [column_id],
            |row| row.get(0),
        )
        .map_err(db)?;
    let now = now_millis();
    let card = Card {
        id: new_id(),
        board_id: column.board_id.clone(),
        column_id: column.id.clone(),
        number,
        title,
        body: body.to_string(),
        position,
        link_kind: None,
        link_id: None,
        created_at: now,
        updated_at: now,
    };
    tx.execute(
        "INSERT INTO board_cards
           (id, board_id, column_id, number, title, body, position, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            card.id,
            card.board_id,
            card.column_id,
            card.number,
            card.title,
            card.body,
            card.position,
            now,
        ],
    )
    .map_err(db)?;
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(&card.id),
            kind: "created",
            actor: "user",
            from_column: None,
            to_column: Some(&column.name),
            detail: None,
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(card)
}

/// Returns the owning board id.
pub(crate) fn update_card(
    conn: &Connection,
    card_id: &str,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let title = clean_text(title, "Card title", MAX_TITLE)?;
    check_body(body)?;
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    if card.title == title && card.body == body {
        return Ok(card.board_id);
    }
    tx.execute(
        "UPDATE board_cards SET title = ?1, body = ?2, updated_at = ?3 WHERE id = ?4",
        params![title, body, now_millis(), card_id],
    )
    .map_err(db)?;
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind: "edited",
            actor: "user",
            from_column: None,
            to_column: None,
            detail: None,
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Returns the owning board id.
pub(crate) fn move_card(
    conn: &Connection,
    card_id: &str,
    column_id: &str,
    index: usize,
    actor: &str,
) -> Result<String, String> {
    if !ACTORS.contains(&actor) {
        return Err(format!("Unknown actor: {actor}"));
    }
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let target = get_column(&tx, column_id)?;
    if target.board_id != card.board_id {
        return Err("Column not on this board".into());
    }
    let same_column = card.column_id == column_id;
    let mut target_ids = column_card_ids(&tx, column_id)?;
    let old_index = target_ids.iter().position(|id| id == card_id);
    target_ids.retain(|id| id != card_id);
    let new_index = index.min(target_ids.len());
    if same_column && old_index == Some(new_index) {
        return Ok(card.board_id);
    }
    target_ids.insert(new_index, card_id.to_string());

    let source = get_column(&tx, &card.column_id)?;
    if !same_column {
        tx.execute(
            "UPDATE board_cards SET column_id = ?1 WHERE id = ?2",
            params![column_id, card_id],
        )
        .map_err(db)?;
        let mut source_ids = column_card_ids(&tx, &card.column_id)?;
        source_ids.retain(|id| id != card_id);
        write_card_positions(&tx, &source_ids)?;
    }
    write_card_positions(&tx, &target_ids)?;
    tx.execute(
        "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
        params![now_millis(), card_id],
    )
    .map_err(db)?;
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind: "moved",
            actor,
            from_column: Some(&source.name),
            to_column: Some(&target.name),
            detail: None,
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Passing `None` for both unlinks. Returns the owning board id.
pub(crate) fn link_card(
    conn: &Connection,
    card_id: &str,
    link_kind: Option<&str>,
    link_id: Option<&str>,
) -> Result<String, String> {
    let link = match (link_kind, link_id) {
        (Some(kind), Some(id)) => {
            if !LINK_KINDS.contains(&kind) {
                return Err(format!("Unknown link kind: {kind}"));
            }
            if id.is_empty() {
                return Err("Link id is required".into());
            }
            Some((kind, id))
        }
        (None, None) => None,
        _ => return Err("A link needs both a kind and an id".into()),
    };
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let previous = card.link_kind.as_deref().zip(card.link_id.as_deref());
    if previous == link {
        return Ok(card.board_id);
    }
    let (kind, detail) = match (link, previous) {
        (Some((kind, id)), _) => ("linked", format!("{kind}:{id}")),
        (None, Some((kind, id))) => ("unlinked", format!("{kind}:{id}")),
        // Unlinking an unlinked card equals `previous == link`, handled above.
        (None, None) => return Ok(card.board_id),
    };
    tx.execute(
        "UPDATE board_cards SET link_kind = ?1, link_id = ?2, updated_at = ?3 WHERE id = ?4",
        params![link.map(|l| l.0), link.map(|l| l.1), now_millis(), card_id],
    )
    .map_err(db)?;
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind,
            actor: "user",
            from_column: None,
            to_column: None,
            detail: Some(&detail),
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Returns the owning board id.
pub(crate) fn delete_card(conn: &Connection, card_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let column = get_column(&tx, &card.column_id)?;
    // The tombstone has no card_id: the card's own rows cascade away with it.
    let detail = format!("#{} {}", card.number, card.title);
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: None,
            kind: "deleted",
            actor: "user",
            from_column: Some(&column.name),
            to_column: None,
            detail: Some(&detail),
        },
    )?;
    tx.execute("DELETE FROM board_cards WHERE id = ?1", [card_id])
        .map_err(db)?;
    let ids = column_card_ids(&tx, &card.column_id)?;
    write_card_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

pub(crate) fn cards_for_link(
    conn: &Connection,
    link_kind: &str,
    link_id: &str,
) -> Result<Vec<Card>, String> {
    let mut statement = conn
        .prepare(
            "SELECT * FROM board_cards WHERE link_kind = ?1 AND link_id = ?2
             ORDER BY board_id, number",
        )
        .map_err(db)?;
    let cards = statement
        .query_map(params![link_kind, link_id], card_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    Ok(cards)
}

pub(crate) fn list_activity(
    conn: &Connection,
    board_id: &str,
    card_id: Option<&str>,
    limit: usize,
) -> Result<Vec<Activity>, String> {
    let limit = limit.clamp(1, MAX_ACTIVITY) as i64;
    let mut statement = conn
        .prepare(
            "SELECT id, board_id, card_id, kind, actor, from_column, to_column, detail, created_at
             FROM board_activity
             WHERE board_id = ?1 AND (?2 IS NULL OR card_id = ?2)
             ORDER BY created_at DESC, rowid DESC
             LIMIT ?3",
        )
        .map_err(db)?;
    let rows = statement
        .query_map(params![board_id, card_id, limit], |row| {
            Ok(Activity {
                id: row.get(0)?,
                board_id: row.get(1)?,
                card_id: row.get(2)?,
                kind: row.get(3)?,
                actor: row.get(4)?,
                from_column: row.get(5)?,
                to_column: row.get(6)?,
                detail: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    Ok(rows)
}

/// Appends a `system` row without touching the card. Returns the owning board id.
pub(crate) fn record_card(
    conn: &Connection,
    card_id: &str,
    kind: &str,
    detail: Option<&str>,
) -> Result<String, String> {
    if !RECORD_KINDS.contains(&kind) {
        return Err(format!("Cannot record activity kind: {kind}"));
    }
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind,
            actor: "system",
            from_column: None,
            to_column: None,
            detail,
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// The native app's handle on the shared board tables. Open it with the
/// session database path so boards made in either app are the same boards.
pub struct Service {
    conn: Connection,
}

impl Service {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        ensure_tables(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn list(&mut self) -> Result<Vec<Board>, String> {
        list_boards(&self.conn)
    }

    pub fn get(&mut self, board_id: &str) -> Result<BoardDetail, String> {
        validate_id(board_id, "board")?;
        get_board(&self.conn, board_id)
    }

    pub fn create(&mut self, name: &str) -> Result<Board, String> {
        create_board(&self.conn, name)
    }

    pub fn rename(&mut self, board_id: &str, name: &str) -> Result<(), String> {
        validate_id(board_id, "board")?;
        rename_board(&self.conn, board_id, name)
    }

    pub fn delete(&mut self, board_id: &str) -> Result<(), String> {
        validate_id(board_id, "board")?;
        delete_board(&self.conn, board_id)
    }

    pub fn column_create(
        &mut self,
        board_id: &str,
        name: &str,
        role: Option<&str>,
    ) -> Result<Column, String> {
        validate_id(board_id, "board")?;
        create_column(&self.conn, board_id, name, role)
    }

    pub fn column_update(
        &mut self,
        column_id: &str,
        name: &str,
        role: Option<&str>,
    ) -> Result<(), String> {
        validate_id(column_id, "column")?;
        update_column(&self.conn, column_id, name, role).map(drop)
    }

    pub fn column_move(&mut self, column_id: &str, index: usize) -> Result<(), String> {
        validate_id(column_id, "column")?;
        move_column(&self.conn, column_id, index).map(drop)
    }

    pub fn column_delete(&mut self, column_id: &str) -> Result<(), String> {
        validate_id(column_id, "column")?;
        delete_column(&self.conn, column_id).map(drop)
    }

    pub fn card_create(
        &mut self,
        column_id: &str,
        title: &str,
        body: &str,
    ) -> Result<Card, String> {
        validate_id(column_id, "column")?;
        create_card(&self.conn, column_id, title, body)
    }

    pub fn card_update(&mut self, card_id: &str, title: &str, body: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        update_card(&self.conn, card_id, title, body).map(drop)
    }

    pub fn card_move(
        &mut self,
        card_id: &str,
        column_id: &str,
        index: usize,
        actor: &str,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        validate_id(column_id, "column")?;
        move_card(&self.conn, card_id, column_id, index, actor).map(drop)
    }

    pub fn card_link(
        &mut self,
        card_id: &str,
        kind: Option<&str>,
        id: Option<&str>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        link_card(&self.conn, card_id, kind, id).map(drop)
    }

    pub fn card_delete(&mut self, card_id: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        delete_card(&self.conn, card_id).map(drop)
    }

    pub fn cards_for_link(&mut self, kind: &str, id: &str) -> Result<Vec<Card>, String> {
        cards_for_link(&self.conn, kind, id)
    }

    pub fn activity(
        &mut self,
        board_id: &str,
        card_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Activity>, String> {
        validate_id(board_id, "board")?;
        if let Some(card_id) = card_id {
            validate_id(card_id, "card")?;
        }
        list_activity(&self.conn, board_id, card_id, limit)
    }

    pub fn card_record(
        &mut self,
        card_id: &str,
        kind: &str,
        detail: Option<&str>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        record_card(&self.conn, card_id, kind, detail).map(drop)
    }

    /// SQLite's counter of commits made by other connections; a change means
    /// another app or window wrote to the database.
    pub fn data_version(&mut self) -> Result<i64, String> {
        self.conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        ensure_tables(&conn).unwrap();
        conn
    }

    fn col_positions(conn: &Connection, column_id: &str) -> Vec<(String, i64)> {
        let mut statement = conn
            .prepare("SELECT id, position FROM board_cards WHERE column_id = ?1 ORDER BY position")
            .unwrap();
        let rows = statement
            .query_map([column_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    }

    fn assert_contiguous(conn: &Connection, column_id: &str) {
        let positions: Vec<i64> = col_positions(conn, column_id)
            .into_iter()
            .map(|(_, position)| position)
            .collect();
        let expected: Vec<i64> = (0..positions.len() as i64).collect();
        assert_eq!(positions, expected, "column {column_id} not contiguous");
    }

    fn titles(conn: &Connection, column_id: &str) -> Vec<String> {
        col_positions(conn, column_id)
            .into_iter()
            .map(|(id, _)| get_card(conn, &id).unwrap().title)
            .collect()
    }

    /// A board plus its default columns (todo, doing, review, done).
    fn board_with_columns(conn: &Connection) -> (Board, Vec<Column>) {
        let board = create_board(conn, "Work").unwrap();
        let columns = get_board(conn, &board.id).unwrap().columns;
        (board, columns)
    }

    fn add_cards(conn: &Connection, column_id: &str, titles: &[&str]) -> Vec<Card> {
        titles
            .iter()
            .map(|title| create_card(conn, column_id, title, "").unwrap())
            .collect()
    }

    #[test]
    fn create_board_makes_four_default_columns_with_roles() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        assert_eq!(board.name, "Work");
        let summary: Vec<(&str, i64, Option<&str>)> = columns
            .iter()
            .map(|column| {
                (
                    column.name.as_str(),
                    column.position,
                    column.role.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Backlog", 0, Some("todo")),
                ("In Progress", 1, Some("doing")),
                ("Review", 2, Some("review")),
                ("Done", 3, Some("done")),
            ]
        );
        assert_eq!(list_boards(&conn).unwrap().len(), 1);
    }

    #[test]
    fn board_serializes_without_card_seq() {
        let conn = setup();
        let board = create_board(&conn, "Work").unwrap();
        let value = serde_json::to_value(&board).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["createdAt", "id", "name", "updatedAt"]);
    }

    #[test]
    fn card_create_appends_at_end_and_logs_created() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        assert_eq!(
            cards.iter().map(|card| card.position).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "b", "c"]);

        let log = list_activity(&conn, &board.id, Some(&cards[2].id), 50).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, "created");
        assert_eq!(log[0].actor, "user");
        assert_eq!(log[0].card_id.as_deref(), Some(cards[2].id.as_str()));
        assert_eq!(log[0].to_column.as_deref(), Some("Backlog"));
    }

    #[test]
    fn card_numbers_increase_per_board_and_are_never_reused() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        assert_eq!(
            cards.iter().map(|card| card.number).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        delete_card(&conn, &cards[2].id).unwrap();
        let next = create_card(&conn, &columns[0].id, "d", "").unwrap();
        assert_eq!(next.number, 4);

        let (_, other_columns) = board_with_columns(&conn);
        let first = create_card(&conn, &other_columns[0].id, "x", "").unwrap();
        assert_eq!(first.number, 1);
    }

    #[test]
    fn move_within_column_renumbers_contiguously() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c", "d"]);
        move_card(&conn, &cards[0].id, &columns[0].id, 2, "user").unwrap();
        assert_eq!(titles(&conn, &columns[0].id), vec!["b", "c", "a", "d"]);
        assert_contiguous(&conn, &columns[0].id);
    }

    #[test]
    fn move_across_columns_renumbers_both_and_logs_from_to() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        add_cards(&conn, &columns[1].id, &["x", "y"]);
        move_card(&conn, &cards[1].id, &columns[1].id, 1, "system").unwrap();

        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "c"]);
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "b", "y"]);
        assert_contiguous(&conn, &columns[0].id);
        assert_contiguous(&conn, &columns[1].id);
        assert_eq!(
            get_card(&conn, &cards[1].id).unwrap().column_id,
            columns[1].id
        );

        let log = list_activity(&conn, &board.id, Some(&cards[1].id), 50).unwrap();
        assert_eq!(log[0].kind, "moved");
        assert_eq!(log[0].actor, "system");
        assert_eq!(log[0].from_column.as_deref(), Some("Backlog"));
        assert_eq!(log[0].to_column.as_deref(), Some("In Progress"));
    }

    #[test]
    fn move_index_past_end_clamps_to_end() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        add_cards(&conn, &columns[1].id, &["x"]);
        move_card(&conn, &cards[0].id, &columns[1].id, 99, "user").unwrap();
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "a"]);
        move_card(&conn, &cards[1].id, &columns[1].id, 1_000, "user").unwrap();
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "a", "b"]);
        assert_contiguous(&conn, &columns[1].id);
        assert_contiguous(&conn, &columns[0].id);
    }

    #[test]
    fn move_to_same_position_writes_no_activity() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        let before = list_activity(&conn, &board.id, None, 500).unwrap().len();
        move_card(&conn, &cards[1].id, &columns[0].id, 1, "user").unwrap();
        // Dropping onto the end of its own column when it is already last.
        move_card(&conn, &cards[2].id, &columns[0].id, 50, "user").unwrap();
        assert_eq!(
            list_activity(&conn, &board.id, None, 500).unwrap().len(),
            before
        );
        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "b", "c"]);
    }

    #[test]
    fn move_to_column_of_other_board_rejected() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let (_, other_columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a"]);
        let error = move_card(&conn, &cards[0].id, &other_columns[0].id, 0, "user").unwrap_err();
        assert_eq!(error, "Column not on this board");
        assert_eq!(titles(&conn, &columns[0].id), vec!["a"]);
        assert!(list_activity(&conn, &board.id, Some(&cards[0].id), 50)
            .unwrap()
            .iter()
            .all(|row| row.kind != "moved"));
    }

    #[test]
    fn move_rejects_unknown_actor() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a"]);
        assert!(move_card(&conn, &cards[0].id, &columns[1].id, 0, "robot").is_err());
    }

    #[test]
    fn interleaved_moves_keep_positions_contiguous() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let mut cards = add_cards(&conn, &columns[0].id, &["a", "b", "c", "d", "e", "f"]);
        cards.extend(add_cards(&conn, &columns[2].id, &["g", "h"]));
        // Deterministic xorshift so a failure is reproducible.
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        for _ in 0..20 {
            let card = &cards[next(cards.len())];
            let column = &columns[next(columns.len())];
            move_card(&conn, &card.id, &column.id, next(6), "user").unwrap();
            for column in &columns {
                assert_contiguous(&conn, &column.id);
            }
        }
        let total: usize = columns
            .iter()
            .map(|column| col_positions(&conn, &column.id).len())
            .sum();
        assert_eq!(total, cards.len());
    }

    #[test]
    fn column_move_renumbers_columns_contiguously() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        move_column(&conn, &columns[3].id, 0).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        let order: Vec<(&str, i64)> = detail
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.position))
            .collect();
        assert_eq!(
            order,
            vec![
                ("Done", 0),
                ("Backlog", 1),
                ("In Progress", 2),
                ("Review", 3)
            ]
        );
        move_column(&conn, &columns[3].id, 99).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        assert_eq!(detail.columns.last().unwrap().name, "Done");
        let positions: Vec<i64> = detail
            .columns
            .iter()
            .map(|column| column.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
    }

    #[test]
    fn column_create_appends_and_delete_closes_the_gap() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let created = create_column(&conn, &board.id, "QA", None).unwrap();
        assert_eq!((created.position, created.role.as_deref()), (4, None));
        delete_column(&conn, &columns[1].id).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        let positions: Vec<i64> = detail
            .columns
            .iter()
            .map(|column| column.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
        assert_eq!(detail.columns.last().unwrap().name, "QA");
    }

    #[test]
    fn column_delete_refused_when_not_empty() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        add_cards(&conn, &columns[0].id, &["a", "b"]);
        let error = delete_column(&conn, &columns[0].id).unwrap_err();
        assert_eq!(error, "Column has 2 cards");
    }

    #[test]
    fn duplicate_role_rejected() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let error = create_column(&conn, &board.id, "Also done", Some("done")).unwrap_err();
        assert!(error.contains("role"), "{error}");
        let error = update_column(&conn, &columns[0].id, "Backlog", Some("review")).unwrap_err();
        assert!(error.contains("role"), "{error}");
        // Re-saving a column with its own role is fine, and clearing frees it.
        update_column(&conn, &columns[0].id, "Todo", Some("todo")).unwrap();
        update_column(&conn, &columns[3].id, "Done", None).unwrap();
        create_column(&conn, &board.id, "Shipped", Some("done")).unwrap();
        assert!(create_column(&conn, &board.id, "Bad", Some("nonsense")).is_err());
    }

    #[test]
    fn board_delete_cascades_columns_cards_activity() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        move_card(&conn, &cards[0].id, &columns[1].id, 0, "user").unwrap();
        delete_card(&conn, &cards[1].id).unwrap();
        let (_, other_columns) = board_with_columns(&conn);
        add_cards(&conn, &other_columns[0].id, &["keep"]);

        delete_board(&conn, &board.id).unwrap();
        let count = |sql: &str| -> i64 {
            conn.query_row(sql, [board.id.as_str()], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(count("SELECT COUNT(*) FROM boards WHERE id = ?1"), 0);
        assert_eq!(
            count("SELECT COUNT(*) FROM board_columns WHERE board_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM board_cards WHERE board_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM board_activity WHERE board_id = ?1"),
            0
        );
        assert_eq!(list_boards(&conn).unwrap().len(), 1);
        assert_eq!(col_positions(&conn, &other_columns[0].id).len(), 1);
    }

    #[test]
    fn card_delete_logs_deleted_with_number_title_and_null_card_id() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "Fix login", "c"]);
        delete_card(&conn, &cards[1].id).unwrap();

        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "c"]);
        assert_contiguous(&conn, &columns[0].id);
        let log = list_activity(&conn, &board.id, None, 50).unwrap();
        assert_eq!(log[0].kind, "deleted");
        assert_eq!(log[0].card_id, None);
        assert_eq!(log[0].detail.as_deref(), Some("#2 Fix login"));
        // The deleted card's own history goes with it; only the tombstone stays.
        assert!(log
            .iter()
            .all(|row| row.card_id.as_deref() != Some(cards[1].id.as_str())));
    }

    #[test]
    fn cards_for_link_finds_linked_cards() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let (_, other_columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        let other = add_cards(&conn, &other_columns[0].id, &["c"]);
        link_card(&conn, &cards[0].id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &other[0].id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &cards[1].id, Some("pilot"), Some("s-1")).unwrap();

        let found = cards_for_link(&conn, "session", "s-1").unwrap();
        let mut ids: Vec<&str> = found.iter().map(|card| card.id.as_str()).collect();
        ids.sort_unstable();
        let mut expected = vec![cards[0].id.as_str(), other[0].id.as_str()];
        expected.sort_unstable();
        assert_eq!(ids, expected);
        assert!(cards_for_link(&conn, "session", "nope").unwrap().is_empty());
    }

    #[test]
    fn link_and_unlink_log_and_validate() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        assert!(link_card(&conn, &card.id, Some("session"), None).is_err());
        assert!(link_card(&conn, &card.id, Some("bogus"), Some("x")).is_err());

        link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &card.id, None, None).unwrap();
        link_card(&conn, &card.id, None, None).unwrap();
        let kinds: Vec<String> = list_activity(&conn, &board.id, Some(&card.id), 50)
            .unwrap()
            .into_iter()
            .map(|row| row.kind)
            .collect();
        assert_eq!(kinds, vec!["unlinked", "linked", "created"]);
        let card = get_card(&conn, &card.id).unwrap();
        assert_eq!((card.link_kind, card.link_id), (None, None));
    }

    #[test]
    fn update_logs_edited_only_when_changed() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        update_card(&conn, &card.id, "a", "").unwrap();
        update_card(&conn, &card.id, "  b  ", "notes").unwrap();
        let saved = get_card(&conn, &card.id).unwrap();
        assert_eq!((saved.title.as_str(), saved.body.as_str()), ("b", "notes"));
        assert!(update_card(&conn, &card.id, "   ", "x").is_err());
        let edited = list_activity(&conn, &board.id, Some(&card.id), 50)
            .unwrap()
            .iter()
            .filter(|row| row.kind == "edited")
            .count();
        assert_eq!(edited, 1);
    }

    #[test]
    fn record_appends_system_row_without_moving() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        record_card(&conn, &card.id, "failed", Some("exit 1")).unwrap();
        let log = list_activity(&conn, &board.id, Some(&card.id), 50).unwrap();
        assert_eq!(log[0].kind, "failed");
        assert_eq!(log[0].actor, "system");
        assert_eq!(log[0].detail.as_deref(), Some("exit 1"));
        assert_eq!(get_card(&conn, &card.id).unwrap().column_id, columns[0].id);
        assert!(record_card(&conn, &card.id, "deleted", None).is_err());
    }

    #[test]
    fn activity_is_newest_first_and_limited() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        record_card(&conn, &card.id, "started", None).unwrap();
        record_card(&conn, &card.id, "completed", None).unwrap();
        let log = list_activity(&conn, &board.id, None, 2).unwrap();
        let kinds: Vec<&str> = log.iter().map(|row| row.kind.as_str()).collect();
        assert_eq!(kinds, vec!["completed", "started"]);
    }

    #[test]
    fn two_connections_interleaving_moves_keep_positions_contiguous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rebase.db");
        let mut a = Service::open(&path).unwrap();
        let mut b = Service::open(&path).unwrap();
        let board = a.create("Shared").unwrap();
        let columns = a.get(&board.id).unwrap().columns;
        let mut cards = Vec::new();
        for index in 0..6 {
            cards.push(
                a.card_create(&columns[0].id, &format!("Card {index}"), "")
                    .unwrap(),
            );
        }
        let mut seed: u64 = 7;
        for step in 0..20 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let card = &cards[(seed >> 33) as usize % cards.len()];
            let column = &columns[(seed >> 13) as usize % columns.len()];
            let index = (seed >> 7) as usize % 7;
            let service = if step % 2 == 0 { &mut a } else { &mut b };
            service
                .card_move(&card.id, &column.id, index, "user")
                .unwrap();
        }
        let detail = b.get(&board.id).unwrap();
        for column in &detail.columns {
            let mut positions: Vec<i64> = detail
                .cards
                .iter()
                .filter(|card| card.column_id == column.id)
                .map(|card| card.position)
                .collect();
            positions.sort();
            let expected: Vec<i64> = (0..positions.len() as i64).collect();
            assert_eq!(positions, expected, "column {} not contiguous", column.name);
        }
        assert_eq!(detail.cards.len(), 6);
    }

    #[test]
    fn service_rejects_malformed_ids_like_the_tauri_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut service = Service::open(&dir.path().join("rebase.db")).unwrap();
        let bad = "x'; DROP TABLE boards; --";
        assert_eq!(service.get(bad).unwrap_err(), "Invalid board id");
        assert_eq!(service.delete(bad).unwrap_err(), "Invalid board id");
        assert_eq!(
            service.column_move(bad, 0).unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            service.card_create(bad, "Title", "").unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            service.card_move(bad, "column", 0, "user").unwrap_err(),
            "Invalid card id"
        );
        assert_eq!(
            service.card_move("card", bad, 0, "user").unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            service.activity("board", Some(bad), 10).unwrap_err(),
            "Invalid card id"
        );
        assert_eq!(
            service.card_record(bad, "started", None).unwrap_err(),
            "Invalid card id"
        );
    }

    #[test]
    fn two_connections_moving_cards_at_once_lose_no_card_and_keep_columns_contiguous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rebase.db");
        let mut setup = Service::open(&path).unwrap();
        let board = setup.create("Shared").unwrap();
        let columns = setup.get(&board.id).unwrap().columns;
        let cards: Vec<String> = (0..8)
            .map(|n| {
                setup
                    .card_create(&columns[n % 4].id, &format!("Card {n}"), "")
                    .unwrap()
                    .id
            })
            .collect();
        let column_ids: Vec<String> = columns.iter().map(|c| c.id.clone()).collect();
        let workers: Vec<_> = (0..2u64)
            .map(|worker| {
                let (path, cards, column_ids) = (path.clone(), cards.clone(), column_ids.clone());
                std::thread::spawn(move || {
                    let mut service = Service::open(&path).unwrap();
                    let mut seed = 11 + worker * 7919;
                    for _ in 0..150 {
                        seed = seed
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        let card = &cards[(seed >> 33) as usize % cards.len()];
                        let column = &column_ids[(seed >> 13) as usize % column_ids.len()];
                        let index = (seed >> 7) as usize % 9;
                        service.card_move(card, column, index, "user").unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let detail = setup.get(&board.id).unwrap();
        assert_eq!(detail.cards.len(), cards.len());
        for column in &detail.columns {
            let mut positions: Vec<i64> = detail
                .cards
                .iter()
                .filter(|card| card.column_id == column.id)
                .map(|card| card.position)
                .collect();
            positions.sort();
            let expected: Vec<i64> = (0..positions.len() as i64).collect();
            assert_eq!(positions, expected, "column {} has a gap", column.name);
        }
    }

    #[test]
    fn data_version_changes_after_another_connection_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rebase.db");
        let mut a = Service::open(&path).unwrap();
        let mut b = Service::open(&path).unwrap();
        let before = a.data_version().unwrap();
        b.create("Elsewhere").unwrap();
        assert_ne!(a.data_version().unwrap(), before);
    }
}
