use crate::meta::{card_set, cards_on_one_board};
use crate::ops::{clean_text, db, get_card, log_edit, new_id, touch, write_tx};
use crate::views::{map_strings, rewrite_views, row_is, spec_rows};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
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
    check_color(color)?;
    let tx = write_tx(conn)?;
    check_name_free(&tx, board_id, &name, None)?;
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

fn check_color(color: &str) -> Result<(), String> {
    if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Label colour must be six hex digits".into());
    }
    Ok(())
}

/// Refuses a name another label on the board has, ignoring case. SQLite's
/// lower() folds only ASCII, so names are compared in Rust.
fn check_name_free(
    tx: &Transaction<'_>,
    board_id: &str,
    name: &str,
    except: Option<&str>,
) -> Result<(), String> {
    let folded = name.to_lowercase();
    let mut statement = tx
        .prepare("SELECT id, name FROM board_labels WHERE board_id = ?1")
        .map_err(db)?;
    let labels = statement
        .query_map([board_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    let taken = labels
        .iter()
        .any(|(id, existing)| Some(id.as_str()) != except && existing.to_lowercase() == folded);
    if taken {
        return Err(format!("A label named {name} already exists"));
    }
    Ok(())
}

fn get_label(tx: &Transaction<'_>, label_id: &str) -> Result<Label, String> {
    tx.query_row(
        "SELECT id, board_id, name, color, position FROM board_labels WHERE id = ?1",
        [label_id],
        |r| {
            Ok(Label {
                id: r.get(0)?,
                board_id: r.get(1)?,
                name: r.get(2)?,
                color: r.get(3)?,
                position: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| "Label not found".to_string())
}

/// The board's label ids in list order.
fn label_ids(tx: &Transaction<'_>, board_id: &str) -> Result<Vec<String>, String> {
    let mut statement = tx
        .prepare("SELECT id FROM board_labels WHERE board_id = ?1 ORDER BY position, lower(name)")
        .map_err(db)?;
    let ids = statement
        .query_map([board_id], |r| r.get(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(db);
    ids
}

fn write_label_positions(tx: &Transaction<'_>, ids: &[String]) -> Result<(), String> {
    let mut statement = tx
        .prepare("UPDATE board_labels SET position = ?1 WHERE id = ?2 AND position <> ?1")
        .map_err(db)?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![index as i64, id]).map_err(db)?;
    }
    Ok(())
}

/// The cards (live and archived) that carry the label.
fn cards_with(tx: &Transaction<'_>, label_id: &str) -> Result<Vec<String>, String> {
    Ok(card_set(
        tx,
        "SELECT card_id FROM board_card_labels WHERE card_id IN (SELECT id FROM board_cards)
           AND label_id = ?1",
        label_id,
    )?
    .into_iter()
    .collect())
}

/// Renames and/or recolours a label. The name stays unique on its board,
/// ignoring case. Returns the owning board id.
pub fn update_label(
    conn: &Connection,
    label_id: &str,
    name: Option<&str>,
    color: Option<&str>,
) -> Result<String, String> {
    let name = name
        .map(|name| clean_text(name, "Label name", MAX_LABEL))
        .transpose()?;
    color.map(check_color).transpose()?;
    let tx = write_tx(conn)?;
    let label = get_label(&tx, label_id)?;
    if let Some(name) = &name {
        check_name_free(&tx, &label.board_id, name, Some(label_id))?;
    }
    tx.execute(
        "UPDATE board_labels SET name = COALESCE(?1, name), color = COALESCE(?2, color)
         WHERE id = ?3",
        params![name, color, label_id],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(label.board_id)
}

/// Moves a label to `index` (clamped) in its board's list. Returns the
/// owning board id.
pub fn move_label(conn: &Connection, label_id: &str, index: usize) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let label = get_label(&tx, label_id)?;
    let mut ids = label_ids(&tx, &label.board_id)?;
    ids.retain(|id| id != label_id);
    ids.insert(index.min(ids.len()), label_id.to_string());
    write_label_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(label.board_id)
}

/// Deletes a label. Every card that had it (archived ones too) gets one
/// "label removed: {name}" row. Saved views lose the label from their
/// `labels` rows (a row left with no value goes) and their collapsed list
/// groups. Returns the owning board id.
pub fn delete_label(conn: &Connection, label_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let label = get_label(&tx, label_id)?;
    let detail = format!("label removed: {}", label.name);
    for card_id in cards_with(&tx, label_id)? {
        touch(&tx, &card_id)?;
        log_edit(&tx, &label.board_id, &card_id, &detail)?;
    }
    tx.execute(
        "DELETE FROM board_card_labels WHERE label_id = ?1",
        [label_id],
    )
    .map_err(db)?;
    tx.execute("DELETE FROM board_labels WHERE id = ?1", [label_id])
        .map_err(db)?;
    write_label_positions(&tx, &label_ids(&tx, &label.board_id)?)?;
    let group = format!("label:{label_id}");
    rewrite_views(&tx, &label.board_id, |spec, layout| {
        rewrite_label_rows(spec, |id| (id != label_id).then(|| id.to_string()));
        if let Some(collapsed) = layout.get_mut("collapsed") {
            map_strings(collapsed, |key| (key != group).then(|| key.to_string()));
        }
    })?;
    tx.commit().map_err(db)?;
    Ok(label.board_id)
}

/// Merges `from_id` into `into_id` (both on one board): every card with
/// `from_id` gets `into_id` instead, `from_id` is deleted, and saved views'
/// `labels` rows and collapsed list groups point at `into_id`. Each card that
/// changed gets one row naming its labels after. Returns the board id.
pub fn merge_labels(conn: &Connection, from_id: &str, into_id: &str) -> Result<String, String> {
    if from_id == into_id {
        return Err("Pick a different label to merge into".into());
    }
    let tx = write_tx(conn)?;
    let from = get_label(&tx, from_id)?;
    let into = get_label(&tx, into_id)?;
    if from.board_id != into.board_id {
        return Err("Label not on this board".into());
    }
    let board_id = from.board_id;
    for card_id in cards_with(&tx, from_id)? {
        tx.execute(
            "INSERT OR IGNORE INTO board_card_labels (card_id, label_id) VALUES (?1, ?2)",
            params![card_id, into_id],
        )
        .map_err(db)?;
        tx.execute(
            "DELETE FROM board_card_labels WHERE card_id = ?1 AND label_id = ?2",
            params![card_id, from_id],
        )
        .map_err(db)?;
        let names: Vec<String> = card_set(
            &tx,
            "SELECT l.name FROM board_card_labels cl JOIN board_labels l ON l.id = cl.label_id
             WHERE cl.card_id = ?1",
            &card_id,
        )?
        .into_iter()
        .collect();
        touch(&tx, &card_id)?;
        log_edit(&tx, &board_id, &card_id, &labels_detail(names))?;
    }
    tx.execute(
        "DELETE FROM board_card_labels WHERE label_id = ?1",
        [from_id],
    )
    .map_err(db)?;
    tx.execute("DELETE FROM board_labels WHERE id = ?1", [from_id])
        .map_err(db)?;
    write_label_positions(&tx, &label_ids(&tx, &board_id)?)?;
    let (from_group, into_group) = (format!("label:{from_id}"), format!("label:{into_id}"));
    rewrite_views(&tx, &board_id, |spec, layout| {
        rewrite_label_rows(spec, |id| {
            Some(if id == from_id { into_id } else { id }.to_string())
        });
        if let Some(collapsed) = layout.get_mut("collapsed") {
            map_strings(collapsed, |key| {
                Some(if key == from_group { &into_group } else { key }.to_string())
            });
        }
    })?;
    tx.commit().map_err(db)?;
    Ok(board_id)
}

/// Maps the (trimmed) values of a spec's `labels` rows through `each`,
/// dropping repeats. A row that had values and has none left is removed:
/// the engine would ignore it anyway.
fn rewrite_label_rows(spec: &mut serde_json::Value, each: impl Fn(&str) -> Option<String>) {
    let Some(rows) = spec_rows(spec) else {
        return;
    };
    rows.retain_mut(|row| {
        if !row_is(row, "labels") {
            return true;
        }
        let Some(values) = row.get_mut("values") else {
            return true;
        };
        let had = values.as_array().is_some_and(|v| !v.is_empty());
        map_strings(values, |value| each(value.trim()));
        !(had && values.as_array().is_some_and(Vec::is_empty))
    });
}
