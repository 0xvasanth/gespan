//! Custom fields a board defines, and each card's values for them. Values are
//! stored as JSON so every kind shares one column.
use crate::ops::{clean_text, db, get_card, log_edit, new_id, touch, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

const MAX_FIELD_NAME: usize = 60;
const MAX_JSON: usize = 20_000;
pub const FIELD_KINDS: [&str; 5] = ["text", "number", "date", "select", "checkbox"];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub kind: String,
    pub options_json: Option<String>,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldValue {
    pub card_id: String,
    pub field_id: String,
    pub value_json: String,
}

fn check_json(raw: &str, label: &str) -> Result<serde_json::Value, String> {
    if raw.len() > MAX_JSON {
        return Err(format!("{label} is too long"));
    }
    serde_json::from_str(raw).map_err(|_| format!("{label} is not valid JSON"))
}

pub fn create_field(
    conn: &Connection,
    board_id: &str,
    name: &str,
    kind: &str,
    options_json: Option<&str>,
) -> Result<Field, String> {
    let name = clean_text(name, "Field name", MAX_FIELD_NAME)?;
    if !FIELD_KINDS.contains(&kind) {
        return Err(format!("Unknown field kind: {kind}"));
    }
    if let Some(options) = options_json {
        check_json(options, "Field options")?;
    }
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
            "SELECT COALESCE(MAX(position) + 1, 0) FROM board_fields WHERE board_id = ?1",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let field = Field {
        id: new_id(),
        board_id: board_id.to_string(),
        name,
        kind: kind.to_string(),
        options_json: options_json.map(str::to_string),
        position,
    };
    tx.execute(
        "INSERT INTO board_fields (id, board_id, name, kind, options_json, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            field.id,
            field.board_id,
            field.name,
            field.kind,
            field.options_json,
            field.position
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(field)
}

pub fn list_fields(conn: &Connection, board_id: &str) -> Result<Vec<Field>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, board_id, name, kind, options_json, position FROM board_fields
             WHERE board_id = ?1 ORDER BY position, rowid",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| {
            Ok(Field {
                id: r.get(0)?,
                board_id: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                options_json: r.get(4)?,
                position: r.get(5)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

/// Sets (or with `None`, clears) a card's value for a field on its board.
/// Returns the owning board id.
pub fn set_field_value(
    conn: &Connection,
    card_id: &str,
    field_id: &str,
    value_json: Option<&str>,
) -> Result<String, String> {
    let value = value_json
        .map(|raw| check_json(raw, "Field value"))
        .transpose()?;
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let name: Option<String> = tx
        .query_row(
            "SELECT name FROM board_fields WHERE id = ?1 AND board_id = ?2",
            params![field_id, card.board_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db)?;
    let Some(name) = name else {
        return Err("Field not on this board".into());
    };
    let current: Option<String> = tx
        .query_row(
            "SELECT value_json FROM board_card_field_values WHERE card_id = ?1 AND field_id = ?2",
            params![card_id, field_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db)?;
    let current = current.and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    if current == value {
        return Ok(card.board_id);
    }
    match &value {
        Some(value) => tx
            .execute(
                "INSERT INTO board_card_field_values (card_id, field_id, value_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (card_id, field_id) DO UPDATE SET value_json = excluded.value_json",
                params![card_id, field_id, value.to_string()],
            )
            .map_err(db)?,
        None => tx
            .execute(
                "DELETE FROM board_card_field_values WHERE card_id = ?1 AND field_id = ?2",
                params![card_id, field_id],
            )
            .map_err(db)?,
    };
    touch(&tx, card_id)?;
    let shown = match &value {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "none".into(),
    };
    log_edit(&tx, &card.board_id, card_id, &format!("{name}: {shown}"))?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

pub fn field_values(conn: &Connection, card_id: &str) -> Result<Vec<FieldValue>, String> {
    let mut statement = conn
        .prepare(
            "SELECT v.card_id, v.field_id, v.value_json FROM board_card_field_values v
             JOIN board_fields f ON f.id = v.field_id
             WHERE v.card_id = ?1 ORDER BY f.position, f.rowid",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| {
            Ok(FieldValue {
                card_id: r.get(0)?,
                field_id: r.get(1)?,
                value_json: r.get(2)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}
