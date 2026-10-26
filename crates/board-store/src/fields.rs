//! Custom fields a board defines, and each card's values for them. Values are
//! stored as JSON so every kind shares one column.
use crate::ops::{clean_text, db, get_card, log_edit, new_id, touch, write_tx};
use crate::views::{map_strings, rewrite_views, row_is, spec_rows};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;

const MAX_FIELD_NAME: usize = 60;
const MAX_JSON: usize = 20_000;
const MAX_TEXT: usize = 2_000;
const MAX_OPTION: usize = 60;
const MAX_DISTINCT: usize = 50;
pub const FIELD_KINDS: [&str; 7] = [
    "text",
    "number",
    "date",
    "select",
    "multi-select",
    "checkbox",
    "url",
];

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
    let options_json = match options_json {
        Some(raw) if has_options(kind) => Some(check_options(raw)?),
        Some(raw) => {
            check_json(raw, "Field options")?;
            Some(raw.to_string())
        }
        None => None,
    };
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
        options_json,
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
    let field: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT name, kind, options_json FROM board_fields WHERE id = ?1 AND board_id = ?2",
            params![field_id, card.board_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(db)?;
    let Some((name, kind, options)) = field else {
        return Err("Field not on this board".into());
    };
    let options = parse_options(options.as_deref());
    let value = value
        .map(|value| validate_value(&kind, &options, &value))
        .transpose()?
        // No options picked clears a multi-select.
        .filter(|value| value.as_array().is_none_or(|items| !items.is_empty()));
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
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
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

fn has_options(kind: &str) -> bool {
    kind == "select" || kind == "multi-select"
}

/// A select field's options as stored. Unreadable options read as none, so
/// no value validates against them.
fn parse_options(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

/// Options must be a JSON array of distinct (ignoring case) names. Returns
/// them re-encoded with each name trimmed.
fn check_options(raw: &str) -> Result<String, String> {
    const BAD: &str = "Field options must be a list of distinct names";
    let value = check_json(raw, "Field options")?;
    let Some(items) = value.as_array() else {
        return Err(BAD.into());
    };
    let mut names: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let name = item.as_str().ok_or(BAD)?;
        let name = clean_text(name, "Option name", MAX_OPTION)?;
        if names
            .iter()
            .any(|n| n.to_lowercase() == name.to_lowercase())
        {
            return Err(BAD.into());
        }
        names.push(name);
    }
    serde_json::to_string(&names).map_err(|error| error.to_string())
}

fn text_of(value: &Value, wrong: &str) -> Result<String, String> {
    let text = value.as_str().ok_or_else(|| wrong.to_string())?;
    if text.chars().count() > MAX_TEXT {
        return Err("Text is too long".into());
    }
    Ok(text.to_string())
}

/// Checks a value against a field's kind (and options, for select kinds) and
/// returns it as stored. Multi-select values come back in option order.
pub fn validate_value(kind: &str, options: &[String], value: &Value) -> Result<Value, String> {
    const PICK: &str = "Pick one of the field's options";
    match kind {
        "text" => Ok(Value::String(text_of(value, "Text must be a string")?)),
        "number" => match value.as_f64() {
            Some(n) if value.is_number() && n.is_finite() => Ok(normal_number(value)),
            _ => Err("Number must be a number".into()),
        },
        "date" => match value.as_i64() {
            Some(ms) => Ok(Value::from(ms)),
            None => Err("Date must be a timestamp".into()),
        },
        "select" => match value.as_str() {
            Some(option) if options.iter().any(|o| o == option) => Ok(value.clone()),
            _ => Err(PICK.into()),
        },
        "multi-select" => {
            let items = value.as_array().ok_or(PICK)?;
            let mut picked = Vec::with_capacity(items.len());
            for item in items {
                let option = item.as_str().ok_or(PICK)?;
                if !options.iter().any(|o| o == option) || picked.contains(&option) {
                    return Err(PICK.into());
                }
                picked.push(option);
            }
            Ok(Value::from(
                options
                    .iter()
                    .filter(|o| picked.contains(&o.as_str()))
                    .cloned()
                    .collect::<Vec<_>>(),
            ))
        }
        "checkbox" => match value {
            Value::Bool(_) => Ok(value.clone()),
            _ => Err("Checkbox must be true or false".into()),
        },
        "url" => {
            const URL: &str = "URL must start with http:// or https://";
            let url = text_of(value, URL)?;
            let lower = url.to_ascii_lowercase();
            let rest = lower
                .strip_prefix("https://")
                .or_else(|| lower.strip_prefix("http://"))
                .ok_or(URL)?;
            if rest.trim().is_empty() {
                return Err(URL.into());
            }
            Ok(Value::String(url))
        }
        _ => Err(format!("Unknown field kind: {kind}")),
    }
}

/// Largest integer an f64 holds exactly (2^53).
const EXACT_INT: f64 = 9_007_199_254_740_992.0;

/// A whole-number float as an integer (5.0 is stored as 5), so one number
/// has one stored form. Anything else is returned as it is.
fn normal_number(value: &Value) -> Value {
    match value.as_f64() {
        Some(n) if value.is_f64() && n.fract() == 0.0 && n.abs() <= EXACT_INT => {
            Value::from(n as i64)
        }
        _ => value.clone(),
    }
}

/// A value copied to a field on another board (by name and kind): it is
/// validated against the target field's options. Multi-select options the
/// target lacks are stripped first; `None` drops the value.
pub(crate) fn carry_value(kind: &str, options_json: Option<&str>, raw: &str) -> Option<Value> {
    let options = parse_options(options_json);
    let mut value = serde_json::from_str::<Value>(raw).ok()?;
    if kind == "multi-select" {
        if let Value::Array(items) = &mut value {
            items.retain(|item| {
                item.as_str()
                    .is_some_and(|o| options.iter().any(|k| k == o))
            });
            if items.is_empty() {
                return None;
            }
        }
    }
    validate_value(kind, &options, &value).ok()
}

fn get_field(tx: &Transaction<'_>, field_id: &str) -> Result<Field, String> {
    tx.query_row(
        "SELECT id, board_id, name, kind, options_json, position FROM board_fields WHERE id = ?1",
        [field_id],
        |r| {
            Ok(Field {
                id: r.get(0)?,
                board_id: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                options_json: r.get(4)?,
                position: r.get(5)?,
            })
        },
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| "Field not found".to_string())
}

fn field_ids(tx: &Transaction<'_>, board_id: &str) -> Result<Vec<String>, String> {
    let mut statement = tx
        .prepare("SELECT id FROM board_fields WHERE board_id = ?1 ORDER BY position, rowid")
        .map_err(db)?;
    let ids = statement
        .query_map([board_id], |r| r.get(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(db);
    ids
}

fn write_field_positions(tx: &Transaction<'_>, ids: &[String]) -> Result<(), String> {
    let mut statement = tx
        .prepare("UPDATE board_fields SET position = ?1 WHERE id = ?2 AND position <> ?1")
        .map_err(db)?;
    for (index, id) in ids.iter().enumerate() {
        statement.execute(params![index as i64, id]).map_err(db)?;
    }
    Ok(())
}

/// Rewrites every stored value of a field: `each` returns the new value, or
/// `None` to clear it. Values that don't parse are left alone.
fn rewrite_values(
    tx: &Transaction<'_>,
    field_id: &str,
    each: impl Fn(&Value) -> Option<Value>,
) -> Result<(), String> {
    let values = {
        let mut statement = tx
            .prepare("SELECT card_id, value_json FROM board_card_field_values WHERE field_id = ?1")
            .map_err(db)?;
        let rows = statement
            .query_map([field_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        rows
    };
    for (card_id, raw) in values {
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        match each(&value) {
            Some(new) if new == value => {}
            Some(new) => {
                tx.execute(
                    "UPDATE board_card_field_values SET value_json = ?1
                     WHERE card_id = ?2 AND field_id = ?3",
                    params![new.to_string(), card_id, field_id],
                )
                .map_err(db)?;
            }
            None => {
                tx.execute(
                    "DELETE FROM board_card_field_values WHERE card_id = ?1 AND field_id = ?2",
                    params![card_id, field_id],
                )
                .map_err(db)?;
            }
        }
    }
    Ok(())
}

/// Renames a field and/or replaces a select field's options (its kind never
/// changes). Values naming a removed option are cleared (select) or lose it
/// (multi-select; an emptied list clears). To rename an option and keep its
/// values, use [`rename_field_option`]. Returns the owning board id.
pub fn update_field(
    conn: &Connection,
    field_id: &str,
    name: Option<&str>,
    options_json: Option<&str>,
) -> Result<String, String> {
    let name = name
        .map(|name| clean_text(name, "Field name", MAX_FIELD_NAME))
        .transpose()?;
    let tx = write_tx(conn)?;
    let field = get_field(&tx, field_id)?;
    let options = match options_json {
        Some(_) if !has_options(&field.kind) => {
            return Err("Only select fields have options".into());
        }
        Some(raw) => Some(check_options(raw)?),
        None => None,
    };
    tx.execute(
        "UPDATE board_fields SET name = COALESCE(?1, name),
           options_json = COALESCE(?2, options_json)
         WHERE id = ?3",
        params![name, options, field_id],
    )
    .map_err(db)?;
    if let Some(options) = &options {
        let kept = parse_options(Some(options));
        let keeps = |value: &Value| value.as_str().is_some_and(|o| kept.iter().any(|k| k == o));
        rewrite_values(&tx, field_id, |value| match value {
            Value::Array(items) => {
                let left: Vec<Value> = items.iter().filter(|item| keeps(item)).cloned().collect();
                (!left.is_empty()).then_some(Value::Array(left))
            }
            value => keeps(value).then(|| value.clone()),
        })?;
        // Saved-view filter values (JSON-encoded) naming a removed option go;
        // a row left with no value goes too (the engine would ignore it).
        let key = format!("field:{field_id}");
        rewrite_views(&tx, &field.board_id, |spec, _| {
            let Some(rows) = spec_rows(spec) else {
                return;
            };
            rows.retain_mut(|row| {
                if !row_is(row, &key) {
                    return true;
                }
                let Some(values) = row.get_mut("values") else {
                    return true;
                };
                let had = values.as_array().is_some_and(|v| !v.is_empty());
                map_strings(values, |raw| {
                    match serde_json::from_str::<Value>(raw.trim()) {
                        Ok(value @ Value::String(_)) if !keeps(&value) => None,
                        _ => Some(raw.to_string()),
                    }
                });
                !(had && values.as_array().is_some_and(Vec::is_empty))
            });
        })?;
    }
    tx.commit().map_err(db)?;
    Ok(field.board_id)
}

/// Renames one option of a select or multi-select field, rewriting the
/// stored values and saved-view `field:<id>` filter values that name it.
/// Returns the owning board id.
pub fn rename_field_option(
    conn: &Connection,
    field_id: &str,
    from: &str,
    to: &str,
) -> Result<String, String> {
    let to = clean_text(to, "Option name", MAX_OPTION)?;
    let tx = write_tx(conn)?;
    let field = get_field(&tx, field_id)?;
    if !has_options(&field.kind) {
        return Err("Only select fields have options".into());
    }
    let mut options = parse_options(field.options_json.as_deref());
    let Some(at) = options.iter().position(|o| o == from) else {
        return Err("Pick one of the field's options".into());
    };
    if options[at] == to {
        return Ok(field.board_id);
    }
    let folded = to.to_lowercase();
    if options
        .iter()
        .enumerate()
        .any(|(index, o)| index != at && o.to_lowercase() == folded)
    {
        return Err(format!("An option named {to} already exists"));
    }
    options[at] = to.clone();
    tx.execute(
        "UPDATE board_fields SET options_json = ?1 WHERE id = ?2",
        params![
            serde_json::to_string(&options).map_err(|error| error.to_string())?,
            field_id
        ],
    )
    .map_err(db)?;
    let (old, new) = (Value::String(from.to_string()), Value::String(to));
    let rename = |value: &Value| {
        if *value == old {
            new.clone()
        } else {
            value.clone()
        }
    };
    rewrite_values(&tx, field_id, |value| {
        Some(match value {
            Value::Array(items) => Value::Array(items.iter().map(rename).collect()),
            value => rename(value),
        })
    })?;
    let key = format!("field:{field_id}");
    rewrite_views(&tx, &field.board_id, |spec, _| {
        let Some(rows) = spec_rows(spec) else {
            return;
        };
        for row in rows.iter_mut().filter(|row| row_is(row, &key)) {
            if let Some(values) = row.get_mut("values") {
                // Filter values are JSON-encoded, so `"S"` names option S.
                map_strings(values, |raw| {
                    Some(match serde_json::from_str::<Value>(raw.trim()) {
                        Ok(value) if value == old => new.to_string(),
                        _ => raw.to_string(),
                    })
                });
            }
        }
    })?;
    tx.commit().map_err(db)?;
    Ok(field.board_id)
}

/// Moves a field to `index` (clamped) among its board's fields. Returns the
/// owning board id.
pub fn move_field(conn: &Connection, field_id: &str, index: usize) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let field = get_field(&tx, field_id)?;
    let mut ids = field_ids(&tx, &field.board_id)?;
    ids.retain(|id| id != field_id);
    ids.insert(index.min(ids.len()), field_id.to_string());
    write_field_positions(&tx, &ids)?;
    tx.commit().map_err(db)?;
    Ok(field.board_id)
}

/// Deletes a field with its values. Saved views lose its filter rows, table
/// columns and widths, card-face entry, and a swimlane on it goes back to
/// "none". Returns the owning board id.
pub fn delete_field(conn: &Connection, field_id: &str) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let field = get_field(&tx, field_id)?;
    tx.execute(
        "DELETE FROM board_card_field_values WHERE field_id = ?1",
        [field_id],
    )
    .map_err(db)?;
    tx.execute("DELETE FROM board_fields WHERE id = ?1", [field_id])
        .map_err(db)?;
    write_field_positions(&tx, &field_ids(&tx, &field.board_id)?)?;
    let key = format!("field:{field_id}");
    rewrite_views(&tx, &field.board_id, |spec, layout| {
        if let Some(rows) = spec_rows(spec) {
            rows.retain(|row| !row_is(row, &key));
        }
        let Some(layout) = layout.as_object_mut() else {
            return;
        };
        if let Some(columns) = layout.get_mut("columns") {
            map_strings(columns, |id| (id != key).then(|| id.to_string()));
        }
        if let Some(Value::Object(widths)) = layout.get_mut("widths") {
            widths.remove(&key);
        }
        if let Some(display) = layout.get_mut("display") {
            if let Some(fields) = display.pointer_mut("/face/fields") {
                map_strings(fields, |id| (id != field_id).then(|| id.to_string()));
            }
            if let Some(lane) = display.get_mut("swimlane") {
                if lane.as_str() == Some(key.as_str()) {
                    *lane = Value::String("none".into());
                }
            }
        }
    })?;
    tx.commit().map_err(db)?;
    Ok(field.board_id)
}

/// Orders values: numbers numerically, then by their JSON text.
fn by_value(a: &Value, b: &Value) -> Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_number() && b.is_number() => {
            x.partial_cmp(&y).unwrap_or(Ordering::Equal)
        }
        _ => match (a.as_str(), b.as_str()) {
            (Some(x), Some(y)) => x.cmp(y),
            _ => a.to_string().cmp(&b.to_string()),
        },
    }
}

/// The distinct values (as JSON) a field has on the board's live cards, for
/// filter choices: most used first, then by value; multi-select lists count
/// each option. At most `limit` values, and never more than 50.
pub fn field_distinct_values(
    conn: &Connection,
    board_id: &str,
    field_id: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let on_board: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM board_fields WHERE id = ?1 AND board_id = ?2)",
            params![field_id, board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    if !on_board {
        return Err("Field not on this board".into());
    }
    let mut statement = conn
        .prepare(
            "SELECT v.value_json FROM board_card_field_values v
             JOIN board_cards c ON c.id = v.card_id
             LEFT JOIN board_card_meta m ON m.card_id = c.id
             WHERE v.field_id = ?1 AND c.board_id = ?2 AND m.archived_at IS NULL",
        )
        .map_err(db)?;
    let raws = statement
        .query_map(params![field_id, board_id], |r| r.get::<_, String>(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    let mut counts: HashMap<String, (Value, usize)> = HashMap::new();
    let mut add = |value: Value| {
        counts.entry(value.to_string()).or_insert((value, 0)).1 += 1;
    };
    for raw in raws {
        match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Array(items)) => items.into_iter().for_each(&mut add),
            // A 5.0 stored before numbers were normalised counts as 5.
            Ok(value) if value.is_number() => add(normal_number(&value)),
            Ok(value) => add(value),
            // A value that isn't JSON can't be offered as a filter choice.
            Err(_) => {}
        }
    }
    let mut values: Vec<(String, Value, usize)> = counts
        .into_iter()
        .map(|(json, (value, count))| (json, value, count))
        .collect();
    values.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| by_value(&a.1, &b.1)));
    Ok(values
        .into_iter()
        .take(limit.min(MAX_DISTINCT))
        .map(|(json, _, _)| json)
        .collect())
}
