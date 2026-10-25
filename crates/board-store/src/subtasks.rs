use crate::ops::{clean_text, db, get_card, log_edit, new_id, touch, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

const MAX_SUBTASK: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subtask {
    pub id: String,
    pub card_id: String,
    pub title: String,
    pub done: bool,
    pub position: i64,
}

fn subtask_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Subtask> {
    Ok(Subtask {
        id: r.get(0)?,
        card_id: r.get(1)?,
        title: r.get(2)?,
        done: r.get::<_, i64>(3)? != 0,
        position: r.get(4)?,
    })
}

fn get_subtask(conn: &Connection, subtask_id: &str) -> Result<Option<Subtask>, String> {
    conn.query_row(
        "SELECT id, card_id, title, done, position FROM board_subtasks WHERE id = ?1",
        [subtask_id],
        subtask_from_row,
    )
    .optional()
    .map_err(db)
}

pub fn list_subtasks(conn: &Connection, card_id: &str) -> Result<Vec<Subtask>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, card_id, title, done, position FROM board_subtasks
             WHERE card_id = ?1 ORDER BY position, rowid",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([card_id], subtask_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

pub fn add_subtask(conn: &Connection, card_id: &str, title: &str) -> Result<Subtask, String> {
    let title = clean_text(title, "Subtask", MAX_SUBTASK)?;
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let position: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM board_subtasks WHERE card_id = ?1",
            [card_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let subtask = Subtask {
        id: new_id(),
        card_id: card_id.to_string(),
        title,
        done: false,
        position,
    };
    tx.execute(
        "INSERT INTO board_subtasks (id, card_id, title, done, position)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![subtask.id, subtask.card_id, subtask.title, subtask.position],
    )
    .map_err(db)?;
    touch(&tx, card_id)?;
    log_edit(
        &tx,
        &card.board_id,
        card_id,
        &format!("subtask added: {}", subtask.title),
    )?;
    tx.commit().map_err(db)?;
    Ok(subtask)
}

/// Returns the owning board id.
pub fn toggle_subtask(conn: &Connection, subtask_id: &str, done: bool) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let Some(subtask) = get_subtask(&tx, subtask_id)? else {
        return Err("Subtask not found".into());
    };
    let card = get_card(&tx, &subtask.card_id)?;
    if subtask.done == done {
        return Ok(card.board_id);
    }
    tx.execute(
        "UPDATE board_subtasks SET done = ?1 WHERE id = ?2",
        params![i64::from(done), subtask_id],
    )
    .map_err(db)?;
    touch(&tx, &card.id)?;
    let verb = if done { "done" } else { "reopened" };
    log_edit(
        &tx,
        &card.board_id,
        &card.id,
        &format!("subtask {verb}: {}", subtask.title),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Deleting a subtask that is already gone is a quiet no-op. Returns the
/// owning board id when there was something to delete.
pub fn delete_subtask(conn: &Connection, subtask_id: &str) -> Result<Option<String>, String> {
    let tx = write_tx(conn)?;
    let Some(subtask) = get_subtask(&tx, subtask_id)? else {
        return Ok(None);
    };
    let card = get_card(&tx, &subtask.card_id)?;
    tx.execute("DELETE FROM board_subtasks WHERE id = ?1", [subtask_id])
        .map_err(db)?;
    touch(&tx, &card.id)?;
    log_edit(
        &tx,
        &card.board_id,
        &card.id,
        &format!("subtask deleted: {}", subtask.title),
    )?;
    tx.commit().map_err(db)?;
    Ok(Some(card.board_id))
}
