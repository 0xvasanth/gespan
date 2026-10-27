//! Operations that touch several tables at once: converting a subtask to a
//! card, duplicating a card, and moving a card to another board. Each runs in
//! one write transaction.
use crate::fields::carry_value;
use crate::ops::{
    column_card_ids, db, get_card, get_column, is_archived, log_activity, log_edit, new_id,
    write_card_positions, write_tx, NewActivity, LIVE, MAX_TITLE,
};
use crate::{now_millis, Card};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashMap;

const COPY_SUFFIX: &str = " (copy)";

fn next_number(tx: &Transaction<'_>, board_id: &str) -> Result<i64, String> {
    tx.execute(
        "UPDATE boards SET card_seq = card_seq + 1 WHERE id = ?1",
        [board_id],
    )
    .map_err(db)?;
    tx.query_row(
        "SELECT card_seq FROM boards WHERE id = ?1",
        [board_id],
        |r| r.get(0),
    )
    .map_err(db)
}

/// Inserts a card into `column_id` at `index` (clamped) with a fresh number
/// on the column's board, and rewrites the column's positions. Writes no
/// activity.
fn insert_card_at(
    tx: &Transaction<'_>,
    column_id: &str,
    title: &str,
    body: &str,
    index: usize,
) -> Result<Card, String> {
    let column = get_column(tx, column_id)?;
    let number = next_number(tx, &column.board_id)?;
    let mut ids = column_card_ids(tx, column_id)?;
    let index = index.min(ids.len());
    let now = now_millis();
    let card = Card {
        id: new_id(),
        board_id: column.board_id,
        column_id: column.id,
        number,
        title: title.to_string(),
        body: body.to_string(),
        position: index as i64,
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
            now
        ],
    )
    .map_err(db)?;
    ids.insert(index, card.id.clone());
    write_card_positions(tx, &ids)?;
    Ok(card)
}

fn live_index(tx: &Transaction<'_>, card: &Card) -> Result<usize, String> {
    Ok(column_card_ids(tx, &card.column_id)?
        .iter()
        .position(|id| *id == card.id)
        .map_or(0, |i| i + 1))
}

fn ensure_live(tx: &Transaction<'_>, card_id: &str) -> Result<(), String> {
    if is_archived(tx, card_id)? {
        return Err("Card is archived".into());
    }
    Ok(())
}

/// Turns a subtask into a card in the parent's column, right after the
/// parent, related to the parent by `relates`. The subtask is deleted and the
/// parent logs "subtask converted: T → #M".
pub fn convert_subtask(conn: &Connection, subtask_id: &str) -> Result<Card, String> {
    let tx = write_tx(conn)?;
    let found: Option<(String, String)> = tx
        .query_row(
            "SELECT card_id, title FROM board_subtasks WHERE id = ?1",
            [subtask_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db)?;
    let Some((parent_id, title)) = found else {
        return Err("Subtask not found".into());
    };
    let parent = get_card(&tx, &parent_id)?;
    ensure_live(&tx, &parent.id)?;
    let index = live_index(&tx, &parent)?;
    let card = insert_card_at(&tx, &parent.column_id, &title, "", index)?;
    tx.execute(
        "INSERT INTO board_card_relations (card_id, related_card_id, kind)
         VALUES (?1, ?2, 'relates')",
        params![parent.id, card.id],
    )
    .map_err(db)?;
    tx.execute("DELETE FROM board_subtasks WHERE id = ?1", [subtask_id])
        .map_err(db)?;
    let remaining: Vec<String> = {
        let mut statement = tx
            .prepare("SELECT id FROM board_subtasks WHERE card_id = ?1 ORDER BY position, rowid")
            .map_err(db)?;
        let rows = statement
            .query_map([&parent.id], |r| r.get(0))
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(db)?;
        rows
    };
    for (position, id) in remaining.iter().enumerate() {
        tx.execute(
            "UPDATE board_subtasks SET position = ?1 WHERE id = ?2",
            params![position as i64, id],
        )
        .map_err(db)?;
    }
    tx.execute(
        "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
        params![now_millis(), parent.id],
    )
    .map_err(db)?;
    log_edit(
        &tx,
        &parent.board_id,
        &parent.id,
        &format!("subtask converted: {title} → #{}", card.number),
    )?;
    tx.commit().map_err(db)?;
    Ok(card)
}

/// Copies a card to "{title} (copy)" right after the original, with its body,
/// priority, due, start, estimate, labels, projects, field values and
/// subtasks (all reset to not done), and the mark that its title came from a
/// ticket. Links, ticket, comments, relations,
/// archive and completion state are not copied.
pub fn duplicate_card(conn: &Connection, card_id: &str) -> Result<Card, String> {
    let tx = write_tx(conn)?;
    let original = get_card(&tx, card_id)?;
    ensure_live(&tx, card_id)?;
    let keep = MAX_TITLE - COPY_SUFFIX.chars().count();
    let base: String = original.title.chars().take(keep).collect();
    let title = format!("{base}{COPY_SUFFIX}");
    let index = live_index(&tx, &original)?;
    let copy = insert_card_at(&tx, &original.column_id, &title, &original.body, index)?;
    // The copy drops the ticket link but keeps its title, so a title that
    // came from a ticket stays marked as ticket text.
    tx.execute(
        "UPDATE board_cards SET title_from_ticket =
           (SELECT title_from_ticket FROM board_cards WHERE id = ?1)
         WHERE id = ?2",
        params![original.id, copy.id],
    )
    .map_err(db)?;
    tx.execute(
        "INSERT INTO board_card_meta (card_id, priority, due_at, start_at, estimate_minutes)
         SELECT ?2, priority, due_at, start_at, estimate_minutes
         FROM board_card_meta WHERE card_id = ?1",
        params![original.id, copy.id],
    )
    .map_err(db)?;
    tx.execute(
        "INSERT INTO board_card_labels (card_id, label_id)
         SELECT ?2, label_id FROM board_card_labels WHERE card_id = ?1",
        params![original.id, copy.id],
    )
    .map_err(db)?;
    tx.execute(
        "INSERT INTO board_card_projects (card_id, project)
         SELECT ?2, project FROM board_card_projects WHERE card_id = ?1",
        params![original.id, copy.id],
    )
    .map_err(db)?;
    tx.execute(
        "INSERT INTO board_card_field_values (card_id, field_id, value_json)
         SELECT ?2, field_id, value_json FROM board_card_field_values WHERE card_id = ?1",
        params![original.id, copy.id],
    )
    .map_err(db)?;
    let subtasks: Vec<String> = {
        let mut statement = tx
            .prepare("SELECT title FROM board_subtasks WHERE card_id = ?1 ORDER BY position, rowid")
            .map_err(db)?;
        let rows = statement
            .query_map([&original.id], |r| r.get(0))
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(db)?;
        rows
    };
    for (position, subtask_title) in subtasks.iter().enumerate() {
        tx.execute(
            "INSERT INTO board_subtasks (id, card_id, title, done, position)
             VALUES (?1, ?2, ?3, 0, ?4)",
            params![new_id(), copy.id, subtask_title, position as i64],
        )
        .map_err(db)?;
    }
    let column = get_column(&tx, &copy.column_id)?;
    log_activity(
        &tx,
        NewActivity {
            board_id: &copy.board_id,
            card_id: Some(&copy.id),
            kind: "created",
            actor: "user",
            from_column: None,
            to_column: Some(&column.name),
            detail: Some(&format!("duplicated from #{}", original.number)),
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(copy)
}

/// Moves a card to the end of the target board's first `todo` column (else its
/// first column) under a new number there. Labels map by case-insensitive
/// name and field values by name and kind; what has no match is dropped.
/// Relations are removed; links, ticket, comments, subtasks, meta and
/// projects stay. Activity goes on both boards.
pub fn move_card_to_board(
    conn: &Connection,
    card_id: &str,
    board_id: &str,
) -> Result<Card, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    ensure_live(&tx, card_id)?;
    if card.board_id == board_id {
        return Err("Card is already on this board".into());
    }
    let target_name: Option<String> = tx
        .query_row("SELECT name FROM boards WHERE id = ?1", [board_id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db)?;
    let Some(target_name) = target_name else {
        return Err("Board not found".into());
    };
    let source_name: String = tx
        .query_row(
            "SELECT name FROM boards WHERE id = ?1",
            [&card.board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let source_column = get_column(&tx, &card.column_id)?;
    let (column_id, column_name): (String, String) = tx
        .query_row(
            "SELECT id, name FROM board_columns WHERE board_id = ?1
             ORDER BY (role IS NOT 'todo'), position, rowid LIMIT 1",
            [board_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db)?
        .ok_or_else(|| "Board has no columns".to_string())?;

    // Labels: keep those whose name exists on the target board.
    let target_labels: HashMap<String, String> = {
        let mut statement = tx
            .prepare("SELECT id, name FROM board_labels WHERE board_id = ?1")
            .map_err(db)?;
        let rows = statement
            .query_map([board_id], |r| {
                Ok((
                    r.get::<_, String>(1)?.to_lowercase(),
                    r.get::<_, String>(0)?,
                ))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(db)?;
        rows
    };
    let card_labels: Vec<String> = {
        let mut statement = tx
            .prepare(
                "SELECT l.name FROM board_card_labels cl
                 JOIN board_labels l ON l.id = cl.label_id WHERE cl.card_id = ?1",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([card_id], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        rows
    };
    tx.execute(
        "DELETE FROM board_card_labels WHERE card_id = ?1",
        [card_id],
    )
    .map_err(db)?;
    for name in card_labels {
        if let Some(label_id) = target_labels.get(&name.to_lowercase()) {
            tx.execute(
                "INSERT OR IGNORE INTO board_card_labels (card_id, label_id) VALUES (?1, ?2)",
                params![card_id, label_id],
            )
            .map_err(db)?;
        }
    }

    // Field values: keep those whose field (name, kind) exists on the target
    // and that are valid for it (multi-select loses options it lacks).
    let target_fields: HashMap<(String, String), (String, Option<String>)> = {
        let mut statement = tx
            .prepare("SELECT id, name, kind, options_json FROM board_fields WHERE board_id = ?1 ORDER BY position, rowid")
            .map_err(db)?;
        let rows = statement
            .query_map([board_id], |r| {
                Ok((
                    (
                        r.get::<_, String>(1)?.to_lowercase(),
                        r.get::<_, String>(2)?,
                    ),
                    (r.get::<_, String>(0)?, r.get::<_, Option<String>>(3)?),
                ))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        // On a duplicate name, the earliest field wins.
        let mut map = HashMap::new();
        for (key, id) in rows {
            map.entry(key).or_insert(id);
        }
        map
    };
    let values: Vec<(String, String, String, String)> = {
        let mut statement = tx
            .prepare(
                "SELECT v.field_id, f.name, f.kind, v.value_json
                 FROM board_card_field_values v
                 JOIN board_fields f ON f.id = v.field_id WHERE v.card_id = ?1",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([card_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        rows
    };
    tx.execute(
        "DELETE FROM board_card_field_values WHERE card_id = ?1",
        [card_id],
    )
    .map_err(db)?;
    for (_, name, kind, value_json) in values {
        let key = (name.to_lowercase(), kind);
        let Some((field_id, options)) = target_fields.get(&key) else {
            continue;
        };
        if let Some(value) = carry_value(&key.1, options.as_deref(), &value_json) {
            tx.execute(
                "INSERT OR IGNORE INTO board_card_field_values (card_id, field_id, value_json)
                 VALUES (?1, ?2, ?3)",
                params![card_id, field_id, value.to_string()],
            )
            .map_err(db)?;
        }
    }

    tx.execute(
        "DELETE FROM board_card_relations WHERE card_id = ?1 OR related_card_id = ?1",
        [card_id],
    )
    .map_err(db)?;

    let number = next_number(&tx, board_id)?;
    let position: i64 = tx
        .query_row(
            &format!("SELECT COUNT(*) FROM board_cards WHERE column_id = ?1 AND {LIVE}"),
            [&column_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    tx.execute(
        "UPDATE board_cards
         SET board_id = ?1, column_id = ?2, number = ?3, position = ?4, updated_at = ?5
         WHERE id = ?6",
        params![board_id, column_id, number, position, now_millis(), card_id],
    )
    .map_err(db)?;
    // The card already sits on the target board, so this lists only the cards left behind.
    let source_ids = column_card_ids(&tx, &card.column_id)?;
    write_card_positions(&tx, &source_ids)?;

    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind: "moved",
            actor: "user",
            from_column: Some(&source_column.name),
            to_column: None,
            detail: Some(&format!("moved to board {target_name}")),
        },
    )?;
    log_activity(
        &tx,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind: "moved",
            actor: "user",
            from_column: None,
            to_column: Some(&column_name),
            detail: Some(&format!("moved from board {source_name}")),
        },
    )?;
    let moved = get_card(&tx, card_id)?;
    tx.commit().map_err(db)?;
    Ok(moved)
}
