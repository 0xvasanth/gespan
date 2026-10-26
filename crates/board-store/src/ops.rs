use crate::links::TICKET_KIND_ERROR;
use crate::{now_millis, Activity, Board, BoardDetail, Card, Column};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

pub(crate) const MAX_NAME: usize = 200;
pub(crate) const MAX_TITLE: usize = 500;
const MAX_BODY: usize = 1_000_000;
const MAX_ACTIVITY: usize = 500;

const ROLES: [&str; 5] = ["todo", "doing", "review", "done", "cancelled"];
/// What a card can link to. Ticket kinds (`jira`, `linear`, `plane`) carry the
/// ticket's `TicketRef` JSON as their reference.
pub const LINK_KINDS: [&str; 8] = [
    "session", "pilot", "flow-run", "pr", "worktree", "jira", "linear", "plane",
];
/// The `board_card_links` role of the link `link_card` manages, so the single
/// link both apps show today stays one row among a card's links.
const PRIMARY: &str = "primary";
/// SQL true for a card that is not archived. Archived cards leave their
/// column's order; live cards keep positions 0..n-1 without them.
pub(crate) const LIVE: &str = "NOT EXISTS (SELECT 1 FROM board_card_meta m
   WHERE m.card_id = board_cards.id AND m.archived_at IS NOT NULL)";
const ACTORS: [&str; 2] = ["user", "system"];
/// Kinds `board_card_record` may append. Every other kind is written by the
/// mutation that causes it.
const RECORD_KINDS: [&str; 3] = ["started", "completed", "failed"];

pub fn db(error: rusqlite::Error) -> String {
    error.to_string()
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn clean_text(value: &str, label: &str, max: usize) -> Result<String, String> {
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

pub(crate) fn card_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
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

pub fn get_card(conn: &Connection, card_id: &str) -> Result<Card, String> {
    conn.query_row(
        "SELECT * FROM board_cards WHERE id = ?1",
        [card_id],
        card_from_row,
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| "Card not found".to_string())
}

pub(crate) fn get_column(conn: &Connection, column_id: &str) -> Result<Column, String> {
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

/// The column's live cards in order.
pub(crate) fn column_card_ids(conn: &Connection, column_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT id FROM board_cards WHERE column_id = ?1 AND {LIVE} ORDER BY position, number"
        ))
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
pub(crate) fn write_card_positions(tx: &Transaction<'_>, ids: &[String]) -> Result<(), String> {
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

pub struct NewActivity<'a> {
    pub board_id: &'a str,
    pub card_id: Option<&'a str>,
    pub kind: &'a str,
    pub actor: &'a str,
    pub from_column: Option<&'a str>,
    pub to_column: Option<&'a str>,
    pub detail: Option<&'a str>,
}

pub(crate) fn is_archived(conn: &Connection, card_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM board_card_meta WHERE card_id = ?1 AND archived_at IS NOT NULL)",
        [card_id],
        |row| row.get(0),
    )
    .map_err(db)
}

/// Bumps the card's `updated_at`, for edits that live in other tables.
pub(crate) fn touch(tx: &Transaction<'_>, card_id: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
        params![now_millis(), card_id],
    )
    .map(drop)
    .map_err(db)
}

/// One `edited` row by the user saying what changed.
pub(crate) fn log_edit(
    tx: &Transaction<'_>,
    board_id: &str,
    card_id: &str,
    detail: &str,
) -> Result<(), String> {
    log_activity(
        tx,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind: "edited",
            actor: "user",
            from_column: None,
            to_column: None,
            detail: Some(detail),
        },
    )
}

pub fn log_activity(tx: &Transaction<'_>, entry: NewActivity<'_>) -> Result<(), String> {
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

pub fn list_boards(conn: &Connection) -> Result<Vec<Board>, String> {
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

pub fn get_board(conn: &Connection, board_id: &str) -> Result<BoardDetail, String> {
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
        .prepare(&format!(
            "SELECT * FROM board_cards WHERE board_id = ?1 AND {LIVE} ORDER BY position, number"
        ))
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
pub fn write_tx(conn: &Connection) -> Result<Transaction<'_>, String> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate).map_err(db)
}

/// A board with the four default columns (the "blank" template).
pub fn create_board(conn: &Connection, name: &str) -> Result<Board, String> {
    let name = clean_text(name, "Board name", MAX_NAME)?;
    let tx = write_tx(conn)?;
    let board = crate::templates::create_blank(&tx, &name)?;
    tx.commit().map_err(db)?;
    Ok(board)
}

pub fn rename_board(conn: &Connection, board_id: &str, name: &str) -> Result<(), String> {
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

pub fn delete_board(conn: &Connection, board_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM boards WHERE id = ?1", [board_id])
        .map_err(db)?;
    Ok(())
}

pub fn create_column(
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
pub fn update_column(
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
pub fn move_column(conn: &Connection, column_id: &str, index: usize) -> Result<String, String> {
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
pub fn delete_column(conn: &Connection, column_id: &str) -> Result<String, String> {
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

pub fn create_card(
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
            &format!("SELECT COUNT(*) FROM board_cards WHERE column_id = ?1 AND {LIVE}"),
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
pub fn update_card(
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
pub fn move_card(
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
    if is_archived(&tx, card_id)? {
        return Err("Card is archived".into());
    }
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
pub fn link_card(
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
            if crate::links::is_ticket_kind(kind) {
                return Err(TICKET_KIND_ERROR.into());
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
    let now = now_millis();
    tx.execute(
        "UPDATE board_cards SET link_kind = ?1, link_id = ?2, updated_at = ?3 WHERE id = ?4",
        params![link.map(|l| l.0), link.map(|l| l.1), now, card_id],
    )
    .map_err(db)?;
    tx.execute(
        "DELETE FROM board_card_links WHERE card_id = ?1 AND role = ?2",
        params![card_id, PRIMARY],
    )
    .map_err(db)?;
    if let Some((kind, id)) = link {
        tx.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (card_id, kind, ref) DO UPDATE SET role = excluded.role",
            params![card_id, kind, id, PRIMARY, now],
        )
        .map_err(db)?;
    }
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
pub fn delete_card(conn: &Connection, card_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    delete_card_in(&tx, &card)?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Deletes the card only while it is still archived, all in one write
/// transaction, so a card another app restored meanwhile is never deleted.
/// Returns the owning board id when it deleted the card, `None` when the card
/// is live again or already gone.
pub fn delete_archived_card(conn: &Connection, card_id: &str) -> Result<Option<String>, String> {
    let tx = write_tx(conn)?;
    let card = match get_card(&tx, card_id) {
        Ok(card) => card,
        Err(error) if error == "Card not found" => return Ok(None),
        Err(error) => return Err(error),
    };
    if !is_archived(&tx, card_id)? {
        return Ok(None);
    }
    delete_card_in(&tx, &card)?;
    tx.commit().map_err(db)?;
    Ok(Some(card.board_id))
}

/// Logs the tombstone, deletes the card and closes its column's gap.
fn delete_card_in(tx: &Transaction<'_>, card: &Card) -> Result<(), String> {
    let card_id = card.id.as_str();
    let column = get_column(tx, &card.column_id)?;
    // The tombstone has no card_id: the card's own rows cascade away with it.
    let detail = format!("#{} {}", card.number, card.title);
    log_activity(
        tx,
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
    let ids = column_card_ids(tx, &card.column_id)?;
    write_card_positions(tx, &ids)
}

pub fn cards_for_link(
    conn: &Connection,
    link_kind: &str,
    link_id: &str,
) -> Result<Vec<Card>, String> {
    // The link columns are read too, for links an older build wrote after the
    // v3 migration copied them.
    let mut statement = conn
        .prepare(&format!(
            "SELECT * FROM board_cards
             WHERE (id IN (SELECT card_id FROM board_card_links WHERE kind = ?1 AND ref = ?2)
                    OR (link_kind = ?1 AND link_id = ?2))
               AND {LIVE}
             ORDER BY board_id, number"
        ))
        .map_err(db)?;
    let cards = statement
        .query_map(params![link_kind, link_id], card_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    Ok(cards)
}

pub fn list_activity(
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
pub fn record_card(
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

/// Moves several cards into one column at `index`, keeping the order given.
/// Writes one `moved` row per card whose column or position changed. Returns
/// the owning board id.
pub fn move_cards(
    conn: &Connection,
    card_ids: &[String],
    column_id: &str,
    index: usize,
    actor: &str,
) -> Result<String, String> {
    if !ACTORS.contains(&actor) {
        return Err(format!("Unknown actor: {actor}"));
    }
    let tx = write_tx(conn)?;
    let target = get_column(&tx, column_id)?;
    let mut moving: Vec<Card> = Vec::new();
    for card_id in card_ids {
        if moving.iter().any(|card| &card.id == card_id) {
            continue;
        }
        let card = get_card(&tx, card_id)?;
        if card.board_id != target.board_id {
            return Err("Column not on this board".into());
        }
        if is_archived(&tx, card_id)? {
            return Err("Card is archived".into());
        }
        moving.push(card);
    }
    if moving.is_empty() {
        return Ok(target.board_id);
    }
    let mut target_ids = column_card_ids(&tx, column_id)?;
    target_ids.retain(|id| moving.iter().all(|card| &card.id != id));
    let at = index.min(target_ids.len());
    target_ids.splice(at..at, moving.iter().map(|card| card.id.clone()));

    let mut sources: Vec<&str> = Vec::new();
    for card in &moving {
        if card.column_id != column_id && !sources.contains(&card.column_id.as_str()) {
            sources.push(&card.column_id);
        }
    }
    tx.execute(
        &format!(
            "UPDATE board_cards SET column_id = ?1 WHERE id IN ({})",
            vec!["?"; moving.len()].join(", ")
        ),
        rusqlite::params_from_iter(
            std::iter::once(column_id).chain(moving.iter().map(|card| card.id.as_str())),
        ),
    )
    .map_err(db)?;
    for source in &sources {
        let ids = column_card_ids(&tx, source)?;
        write_card_positions(&tx, &ids)?;
    }
    write_card_positions(&tx, &target_ids)?;

    let now = now_millis();
    for card in &moving {
        let position = target_ids.iter().position(|id| id == &card.id).unwrap_or(0) as i64;
        if card.column_id == column_id && card.position == position {
            continue;
        }
        let source = get_column(&tx, &card.column_id)?;
        tx.execute(
            "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
            params![now, card.id],
        )
        .map_err(db)?;
        log_activity(
            &tx,
            NewActivity {
                board_id: &card.board_id,
                card_id: Some(&card.id),
                kind: "moved",
                actor,
                from_column: Some(&source.name),
                to_column: Some(&target.name),
                detail: None,
            },
        )?;
    }
    tx.commit().map_err(db)?;
    Ok(target.board_id)
}

/// Sets how a column looks: its colour (six hex digits), WIP limit and
/// collapsed state. Returns the owning board id.
pub fn set_column_display(
    conn: &Connection,
    column_id: &str,
    color: Option<&str>,
    wip_limit: Option<i64>,
    collapsed: bool,
) -> Result<String, String> {
    if let Some(color) = color {
        check_color(color)?;
    }
    if wip_limit.is_some_and(|limit| limit < 1) {
        return Err("A WIP limit must be at least 1".into());
    }
    let tx = write_tx(conn)?;
    let column = get_column(&tx, column_id)?;
    tx.execute(
        "UPDATE board_columns SET color = ?1, wip_limit = ?2, collapsed = ?3 WHERE id = ?4",
        params![color, wip_limit, i64::from(collapsed), column_id],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(column.board_id)
}

pub(crate) fn check_color(color: &str) -> Result<(), String> {
    if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Colour must be six hex digits".into());
    }
    Ok(())
}

/// The local calendar day of a timestamp, as the user picked it, for the log.
pub(crate) fn day_label(at: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| at.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ensure_tables;
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
}
