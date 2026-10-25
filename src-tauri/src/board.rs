use gespan_board_schema::{sync_primary_link, ROLES};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::session_store::{now_millis, validate_id, SessionStore};

pub(crate) const CHANGED: &str = "gespan:board-changed";

const MAX_NAME: usize = 200;
const MAX_TITLE: usize = 500;
const MAX_BODY: usize = 1_000_000;
const MAX_ACTIVITY: usize = 500;

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
    id: String,
    name: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    id: String,
    board_id: String,
    name: String,
    position: i64,
    role: Option<String>,
    color: Option<String>,
    wip_limit: Option<i64>,
    collapsed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    id: String,
    board_id: String,
    column_id: String,
    number: i64,
    title: String,
    body: String,
    position: i64,
    link_kind: Option<String>,
    link_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    id: String,
    board_id: String,
    card_id: Option<String>,
    kind: String,
    actor: String,
    from_column: Option<String>,
    to_column: Option<String>,
    detail: Option<String>,
    created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDetail {
    board: Board,
    columns: Vec<Column>,
    cards: Vec<Card>,
}

/// Creates or upgrades the board tables `gespan.db` shares with Native; the
/// schema and its migrations live in `gespan-board-schema`.
pub(crate) fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    gespan_board_schema::ensure(conn)
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
        color: row.get("color")?,
        wip_limit: row.get("wip_limit")?,
        collapsed: row.get::<_, i64>("collapsed")? != 0,
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

pub(crate) fn create_board(conn: &Connection, name: &str) -> Result<Board, String> {
    let name = clean_text(name, "Board name", MAX_NAME)?;
    let now = now_millis();
    let board = Board {
        id: new_id(),
        name,
        created_at: now,
        updated_at: now,
    };
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    board_exists(conn, board_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
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
        color: None,
        wip_limit: None,
        collapsed: false,
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
    let column = get_column(conn, column_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    let column = get_column(conn, column_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
    let mut ids = board_column_ids(&tx, &column.board_id)?;
    ids.retain(|id| id != column_id);
    ids.insert(index.min(ids.len()), column_id.to_string());
    write_column_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(column.board_id)
}

/// Returns the owning board id.
pub(crate) fn delete_column(conn: &Connection, column_id: &str) -> Result<String, String> {
    let column = get_column(conn, column_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    let column = get_column(conn, column_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    let card = get_card(conn, card_id)?;
    if card.title == title && card.body == body {
        return Ok(card.board_id);
    }
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    let card = get_card(conn, card_id)?;
    let target = get_column(conn, column_id)?;
    if target.board_id != card.board_id {
        return Err("Column not on this board".into());
    }
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    let card = get_card(conn, card_id)?;
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
    let tx = conn.unchecked_transaction().map_err(db)?;
    let now = now_millis();
    tx.execute(
        "UPDATE board_cards SET link_kind = ?1, link_id = ?2, updated_at = ?3 WHERE id = ?4",
        params![link.map(|l| l.0), link.map(|l| l.1), now, card_id],
    )
    .map_err(db)?;
    sync_primary_link(&tx, card_id, link, now).map_err(db)?;
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
    let card = get_card(conn, card_id)?;
    let column = get_column(conn, &card.column_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
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
    let card = get_card(conn, card_id)?;
    let tx = conn.unchecked_transaction().map_err(db)?;
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

fn notify(app: &AppHandle, board_id: &str) {
    let _ = app.emit(CHANGED, serde_json::json!({ "boardId": board_id }));
}

#[tauri::command(async)]
pub fn board_list(store: State<'_, SessionStore>) -> Result<Vec<Board>, String> {
    let conn = store.lock_conn()?;
    list_boards(&conn)
}

#[tauri::command(async)]
pub fn board_get(store: State<'_, SessionStore>, board_id: String) -> Result<BoardDetail, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    get_board(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    name: String,
) -> Result<Board, String> {
    let conn = store.lock_conn()?;
    let board = create_board(&conn, &name)?;
    drop(conn);
    notify(&app, &board.id);
    Ok(board)
}

#[tauri::command(async)]
pub fn board_rename(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
) -> Result<(), String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    rename_board(&conn, &board_id, &name)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<(), String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    delete_board(&conn, &board_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
    role: Option<String>,
) -> Result<Column, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    let column = create_column(&conn, &board_id, &name, role.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(column)
}

#[tauri::command(async)]
pub fn board_column_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    name: String,
    role: Option<String>,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = update_column(&conn, &column_id, &name, role.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = move_column(&conn, &column_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = delete_column(&conn, &column_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    title: String,
    body: Option<String>,
) -> Result<Card, String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let card = create_card(&conn, &column_id, &title, body.as_deref().unwrap_or(""))?;
    drop(conn);
    notify(&app, &card.board_id);
    Ok(card)
}

#[tauri::command(async)]
pub fn board_card_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    title: String,
    body: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = update_card(&conn, &card_id, &title, &body)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    column_id: String,
    index: usize,
    actor: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = move_card(&conn, &card_id, &column_id, index, &actor)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_link(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    link_kind: Option<String>,
    link_id: Option<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = link_card(&conn, &card_id, link_kind.as_deref(), link_id.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = delete_card(&conn, &card_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_cards_for_link(
    store: State<'_, SessionStore>,
    link_kind: String,
    link_id: String,
) -> Result<Vec<Card>, String> {
    let conn = store.lock_conn()?;
    cards_for_link(&conn, &link_kind, &link_id)
}

#[tauri::command(async)]
pub fn board_activity(
    store: State<'_, SessionStore>,
    board_id: String,
    card_id: Option<String>,
    limit: usize,
) -> Result<Vec<Activity>, String> {
    validate_id(&board_id, "board")?;
    if let Some(card_id) = &card_id {
        validate_id(card_id, "card")?;
    }
    let conn = store.lock_conn()?;
    list_activity(&conn, &board_id, card_id.as_deref(), limit)
}

#[tauri::command(async)]
pub fn board_card_record(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    kind: String,
    detail: Option<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = record_card(&conn, &card_id, &kind, detail.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
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
    fn strings(conn: &Connection, sql: &str) -> Vec<String> {
        let mut statement = conn.prepare(sql).unwrap();
        let rows = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        rows
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn native_v2_database_opens_in_tauri_with_every_value() {
        use gespan_board_schema::testing::{assert_native_v2_migrated, seed_native_v2};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let seed = {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            seed_native_v2(&conn)
        };
        let store = SessionStore::open(path).unwrap();
        let conn = store.lock_conn().unwrap();
        assert_native_v2_migrated(&conn, &seed);
        let detail = get_board(&conn, &seed.board_id).unwrap();
        let cards: Vec<(String, i64)> = detail
            .cards
            .iter()
            .map(|card| (card.title.clone(), card.position))
            .collect();
        assert_eq!(
            cards,
            [("A".into(), 0), ("B".into(), 1), ("C".into(), 2)]
        );
        let a = &detail.cards[0];
        assert_eq!(
            (a.link_kind.as_deref(), a.link_id.as_deref()),
            (Some("session"), Some("s-1"))
        );
    }

    #[test]
    fn opening_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let version = {
            let store = SessionStore::open(path.clone()).unwrap();
            let conn = store.lock_conn().unwrap();
            count(&conn, "PRAGMA schema_version")
        };
        let store = SessionStore::open(path).unwrap();
        let conn = store.lock_conn().unwrap();
        assert_eq!(count(&conn, "PRAGMA schema_version"), version);
        assert_eq!(
            strings(&conn, "SELECT CAST(version AS TEXT) FROM board_schema_migrations"),
            ["3"]
        );
    }

    #[test]
    fn deleting_a_card_removes_its_v3_rows() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["gone", "kept", "other"]);
        let (gone, kept, other) = (&cards[0].id, &cards[1].id, &cards[2].id);
        conn.execute(
            "INSERT INTO board_labels (id, board_id, name, color, position) VALUES ('l', ?1, 'Bug', 'd73a4a', 0)",
            [&board.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO board_fields (id, board_id, name, kind, position) VALUES ('f', ?1, 'Size', 'number', 0)",
            [&board.id],
        )
        .unwrap();
        for card in [gone, kept] {
            conn.execute_batch(&format!(
                "INSERT INTO board_card_meta (card_id, priority) VALUES ('{card}', 2);
                 INSERT INTO board_card_labels (card_id, label_id) VALUES ('{card}', 'l');
                 INSERT INTO board_subtasks (id, card_id, title, position) VALUES ('s-{card}', '{card}', 'Do', 0);
                 INSERT INTO board_comments (id, card_id, body, created_at) VALUES ('c-{card}', '{card}', 'Hi', 1);
                 INSERT INTO board_card_projects (card_id, project) VALUES ('{card}', '/work');
                 INSERT INTO board_card_links (card_id, kind, ref, created_at) VALUES ('{card}', 'pr', 'gh:1', 1);
                 INSERT INTO board_card_field_values (card_id, field_id, value_json) VALUES ('{card}', 'f', '3');"
            ))
            .unwrap();
        }
        conn.execute_batch(&format!(
            "INSERT INTO board_card_relations (card_id, related_card_id, kind, created_at) VALUES
               ('{gone}', '{kept}', 'blocks', 1),
               ('{other}', '{gone}', 'relates', 1),
               ('{kept}', '{other}', 'duplicates', 1);
             INSERT INTO board_subtasks (id, card_id, title, position, converted_card_id)
               VALUES ('s-converted', '{kept}', 'Split out', 1, '{gone}');"
        ))
        .unwrap();
        delete_card(&conn, gone).unwrap();
        for table in [
            "board_card_meta",
            "board_card_labels",
            "board_subtasks",
            "board_comments",
            "board_card_projects",
            "board_card_links",
            "board_card_field_values",
        ] {
            assert_eq!(
                strings(&conn, &format!("SELECT DISTINCT card_id FROM {table} ORDER BY card_id")),
                [kept.clone()],
                "{table}"
            );
        }
        assert_eq!(
            strings(
                &conn,
                "SELECT card_id || '>' || related_card_id FROM board_card_relations"
            ),
            [format!("{kept}>{other}")]
        );
        // A subtask converted into the deleted card stays, unlinked.
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM board_subtasks WHERE id = 's-converted' AND converted_card_id IS NULL"
            ),
            1
        );
    }

    #[test]
    fn link_card_keeps_the_primary_link_row_in_step() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        let primary = |conn: &Connection| {
            strings(
                conn,
                &format!(
                    "SELECT kind || '/' || ref FROM board_card_links
                     WHERE card_id = '{}' AND role = 'primary'",
                    card.id
                ),
            )
        };
        link_card(&conn, &card.id, Some("session"), Some("x")).unwrap();
        assert_eq!(primary(&conn), ["session/x"]);
        link_card(&conn, &card.id, Some("pilot"), Some("y")).unwrap();
        assert_eq!(primary(&conn), ["pilot/y"]);
        assert!(link_card(&conn, &card.id, Some("bogus"), Some("z")).is_err());
        assert_eq!(primary(&conn), ["pilot/y"]);
        link_card(&conn, &card.id, None, None).unwrap();
        assert!(primary(&conn).is_empty());
    }

    #[test]
    fn cancelled_role_is_accepted_and_unique() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let dropped = create_column(&conn, &board.id, "Dropped", Some("cancelled")).unwrap();
        assert_eq!(dropped.role.as_deref(), Some("cancelled"));
        let error = create_column(&conn, &board.id, "Won't do", Some("cancelled")).unwrap_err();
        assert!(error.contains("already has the cancelled role"), "{error}");
        let error = update_column(&conn, &columns[0].id, "Backlog", Some("cancelled")).unwrap_err();
        assert!(error.contains("already has the cancelled role"), "{error}");
    }

    #[test]
    fn columns_serialize_colour_wip_limit_and_collapsed() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        conn.execute(
            "UPDATE board_columns SET color = 'd73a4a', wip_limit = 3, collapsed = 1 WHERE id = ?1",
            [&columns[1].id],
        )
        .unwrap();
        let read = get_board(&conn, &board.id).unwrap().columns;
        let json = serde_json::to_value(&read[..2]).unwrap();
        assert_eq!(json[0]["color"], serde_json::Value::Null);
        assert_eq!(json[0]["wipLimit"], serde_json::Value::Null);
        assert_eq!(json[0]["collapsed"], false);
        assert_eq!(json[1]["color"], "d73a4a");
        assert_eq!(json[1]["wipLimit"], 3);
        assert_eq!(json[1]["collapsed"], true);
    }

}
