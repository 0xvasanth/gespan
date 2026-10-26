use crate::meta::{card_set, cards_on_one_board};
use crate::ops::{clean_text, db, get_card, log_edit, new_id, touch, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::BTreeSet;

const MAX_LABEL: usize = 40;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub color: String,
    pub position: i64,
}

pub fn list_labels(conn: &Connection, board_id: &str) -> Result<Vec<Label>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, board_id, name, color, position FROM board_labels
             WHERE board_id = ?1 ORDER BY position, lower(name)",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| {
            Ok(Label {
                id: r.get(0)?,
                board_id: r.get(1)?,
                name: r.get(2)?,
                color: r.get(3)?,
                position: r.get(4)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

/// Adds a label at the end of the board's list. Names are unique per board,
/// ignoring case.
pub fn create_label(
    conn: &Connection,
    board_id: &str,
    name: &str,
    color: &str,
) -> Result<Label, String> {
    let name = clean_text(name, "Label name", MAX_LABEL)?;
    if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Label colour must be six hex digits".into());
    }
    let tx = write_tx(conn)?;
    // SQLite's lower() folds only ASCII, so compare names in Rust too.
    let folded = name.to_lowercase();
    let taken = {
        let mut statement = tx
            .prepare("SELECT name FROM board_labels WHERE board_id = ?1")
            .map_err(db)?;
        let names = statement
            .query_map([board_id], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        names
            .iter()
            .any(|existing| existing.to_lowercase() == folded)
    };
    if taken {
        return Err(format!("A label named {name} already exists"));
    }
    let position: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM board_labels WHERE board_id = ?1",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let label = Label {
        id: new_id(),
        board_id: board_id.to_string(),
        name,
        color: color.to_string(),
        position,
    };
    tx.execute(
        "INSERT INTO board_labels (id, board_id, name, color, position)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            label.id,
            label.board_id,
            label.name,
            label.color,
            label.position
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(label)
}

/// Replaces the card's labels. Returns the owning board id.
pub fn set_card_labels(
    conn: &Connection,
    card_id: &str,
    label_ids: &[String],
) -> Result<String, String> {
    let ids: BTreeSet<&str> = label_ids.iter().map(String::as_str).collect();
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let current = card_set(
        &tx,
        "SELECT label_id FROM board_card_labels WHERE card_id = ?1",
        card_id,
    )?;
    if current.iter().map(String::as_str).eq(ids.iter().copied()) {
        return Ok(card.board_id);
    }
    tx.execute(
        "DELETE FROM board_card_labels WHERE card_id = ?1",
        [card_id],
    )
    .map_err(db)?;
    let mut names = Vec::new();
    for id in &ids {
        let name: Option<String> = tx
            .query_row(
                "SELECT name FROM board_labels WHERE id = ?1 AND board_id = ?2",
                params![id, card.board_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        let Some(name) = name else {
            return Err("Label not on this board".into());
        };
        tx.execute(
            "INSERT INTO board_card_labels (card_id, label_id) VALUES (?1, ?2)",
            params![card_id, id],
        )
        .map_err(db)?;
        names.push(name);
    }
    touch(&tx, card_id)?;
    log_edit(&tx, &card.board_id, card_id, &labels_detail(names))?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// The activity detail for a card's labels: "labels: A, b" (by name, case
/// folded) or "labels: none".
fn labels_detail(mut names: Vec<String>) -> String {
    if names.is_empty() {
        return "labels: none".into();
    }
    names.sort_by_key(|name| name.to_lowercase());
    format!("labels: {}", names.join(", "))
}

/// Adds (`on`) or removes one label on several cards (all on the label's
/// board) in one transaction. Each card that changed gets one row naming all
/// its labels after the change. Returns the board id.
pub fn set_label_many(
    conn: &Connection,
    card_ids: &[String],
    label_id: &str,
    on: bool,
) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let (board_id, cards) = cards_on_one_board(&tx, card_ids, "No cards to edit")?;
    let on_board: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM board_labels WHERE id = ?1 AND board_id = ?2)",
            params![label_id, board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    if !on_board {
        return Err("Label not on this board".into());
    }
    for card in &cards {
        let changed = if on {
            tx.execute(
                "INSERT OR IGNORE INTO board_card_labels (card_id, label_id) VALUES (?1, ?2)",
                params![card.id, label_id],
            )
        } else {
            tx.execute(
                "DELETE FROM board_card_labels WHERE card_id = ?1 AND label_id = ?2",
                params![card.id, label_id],
            )
        }
        .map_err(db)?;
        if changed == 0 {
            continue;
        }
        let names: Vec<String> = card_set(
            &tx,
            "SELECT l.name FROM board_card_labels cl JOIN board_labels l ON l.id = cl.label_id
             WHERE cl.card_id = ?1",
            &card.id,
        )?
        .into_iter()
        .collect();
        touch(&tx, &card.id)?;
        log_edit(&tx, &board_id, &card.id, &labels_detail(names))?;
    }
    tx.commit().map_err(db)?;
    Ok(board_id)
}
