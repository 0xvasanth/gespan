//! Board templates: four built in, plus boards saved as templates
//! (`board_templates`). A template holds a board's structure (columns,
//! labels, fields, views and settings) and never its cards.
//!
//! Every id a template holds is replaced when a board is made from it. Column,
//! label and field ids inside view specs, layouts, display options and
//! auto-move rules are remapped to the new ids; a value with no match is
//! dropped (the saved-view rule), and an auto-move target with no match stays.
use crate::ops::{clean_text, db, new_id, write_tx};
use crate::settings::{board_settings, BoardSettings};
use crate::{list_fields, list_labels, list_views, now_millis, Board};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Built-in template ids, in list order.
pub const BUILT_IN_TEMPLATES: [&str; 4] =
    ["blank", "agent-pipeline", "bug-triage", "release-checklist"];
const MAX_TEMPLATE_NAME: usize = 60;
const MAX_BOARD_NAME: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub built_in: bool,
}

/// What `board_templates.spec_json` holds. Ids are the source board's (or,
/// for built-ins, placeholders) and only link the parts to each other.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TemplateSpec {
    columns: Vec<TemplateColumn>,
    labels: Vec<TemplateLabel>,
    fields: Vec<TemplateField>,
    views: Vec<TemplateView>,
    settings: BoardSettings,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TemplateColumn {
    id: String,
    name: String,
    role: Option<String>,
    color: Option<String>,
    wip_limit: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TemplateLabel {
    id: String,
    name: String,
    color: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TemplateField {
    id: String,
    name: String,
    kind: String,
    options_json: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TemplateView {
    id: String,
    name: String,
    spec_json: String,
    layout_json: String,
}

fn built_in_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "blank" => "Blank",
        "agent-pipeline" => "Agent pipeline",
        "bug-triage" => "Bug triage",
        "release-checklist" => "Release checklist",
        _ => return None,
    })
}

fn columns(list: &[(&str, Option<&str>, Option<&str>)]) -> Vec<TemplateColumn> {
    list.iter()
        .enumerate()
        .map(|(index, (name, role, color))| TemplateColumn {
            id: format!("c{index}"),
            name: name.to_string(),
            role: role.map(str::to_string),
            color: color.map(str::to_string),
            wip_limit: None,
        })
        .collect()
}

fn labels(list: &[(&str, &str)]) -> Vec<TemplateLabel> {
    list.iter()
        .enumerate()
        .map(|(index, (name, color))| TemplateLabel {
            id: format!("l{index}"),
            name: name.to_string(),
            color: color.to_string(),
        })
        .collect()
}

fn built_in_spec(id: &str) -> Option<TemplateSpec> {
    let mut spec = TemplateSpec::default();
    match id {
        "blank" => {
            spec.columns = columns(&[
                ("Backlog", Some("todo"), None),
                ("In Progress", Some("doing"), None),
                ("Review", Some("review"), None),
                ("Done", Some("done"), None),
            ]);
        }
        "agent-pipeline" => {
            spec.columns = columns(&[
                ("Queued", Some("todo"), None),
                ("Running", Some("doing"), None),
                ("Needs review", Some("review"), None),
                ("Merged", None, Some("a371f7")),
                ("Done", Some("done"), None),
            ]);
            spec.settings.auto_move_json = Some(
                json!({
                    "started": "default",
                    "needs_input": "stay",
                    "completed": "default",
                    "failed": "stay",
                    "pr_opened": "stay",
                    "pr_merged": {"column": "c3"},
                })
                .to_string(),
            );
        }
        "bug-triage" => {
            spec.columns = columns(&[
                ("New", Some("todo"), None),
                ("Triaged", None, None),
                ("In progress", Some("doing"), None),
                ("Fixed", Some("review"), None),
                ("Closed", Some("done"), None),
                ("Won't fix", Some("cancelled"), None),
            ]);
            spec.labels = labels(&[
                ("Bug", "e5534b"),
                ("Regression", "e0823d"),
                ("Needs repro", "e8c547"),
            ]);
            spec.fields = vec![TemplateField {
                id: "f0".into(),
                name: "Severity".into(),
                kind: "select".into(),
                options_json: Some(json!(["S1", "S2", "S3", "S4"]).to_string()),
            }];
        }
        "release-checklist" => {
            spec.columns = columns(&[
                ("To do", Some("todo"), None),
                ("In progress", Some("doing"), None),
                ("Blocked", None, Some("e5534b")),
                ("Done", Some("done"), None),
            ]);
            spec.labels = labels(&[("Docs", "4da3f5"), ("QA", "3fb950"), ("Ops", "2bb3a8")]);
            spec.fields = vec![TemplateField {
                id: "f0".into(),
                name: "Release".into(),
                kind: "text".into(),
                options_json: None,
            }];
            spec.views = vec![TemplateView {
                id: "v0".into(),
                name: "By label".into(),
                spec_json: "{}".into(),
                layout_json: json!({"layout": "list", "groupBy": "label"}).to_string(),
            }];
        }
        _ => return None,
    }
    Some(spec)
}

/// Built-ins first, then saved templates by name.
pub fn list_templates(conn: &Connection) -> Result<Vec<TemplateInfo>, String> {
    let mut templates: Vec<TemplateInfo> = BUILT_IN_TEMPLATES
        .iter()
        .map(|id| TemplateInfo {
            id: id.to_string(),
            name: built_in_name(id).unwrap_or(id).to_string(),
            built_in: true,
        })
        .collect();
    let mut statement = conn
        .prepare("SELECT id, name FROM board_templates ORDER BY lower(name), name, id")
        .map_err(db)?;
    let saved = statement
        .query_map([], |r| {
            Ok(TemplateInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                built_in: false,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    templates.extend(saved);
    Ok(templates)
}

/// Old id → new id for one kind of part.
type IdMap = HashMap<String, String>;

struct Remap {
    columns: IdMap,
    labels: IdMap,
    fields: IdMap,
}

impl Remap {
    /// `field:<old>` → `field:<new>`; `None` when the field has no match.
    fn field_ref(&self, value: &str) -> Option<String> {
        let old = value.strip_prefix("field:")?;
        self.fields.get(old).map(|new| format!("field:{new}"))
    }

    /// A table column or width key: built-in ids stay, field ids remap.
    fn table_column(&self, id: &str) -> Option<String> {
        if id.starts_with("field:") {
            self.field_ref(id)
        } else {
            Some(id.to_string())
        }
    }

    /// A collapsed group key (`mode:value`): status and label values remap.
    fn group_key(&self, key: &str) -> Option<String> {
        let (mode, value) = key.split_once(':')?;
        let map = match mode {
            "status" => &self.columns,
            "label" => &self.labels,
            _ => return Some(key.to_string()),
        };
        if value == "none" {
            return Some(key.to_string());
        }
        map.get(value).map(|new| format!("{mode}:{new}"))
    }

    fn view_spec(&self, raw: &str) -> String {
        let Ok(mut spec) = serde_json::from_str::<Value>(raw) else {
            return "{}".into();
        };
        if let Some(rows) = spec.get_mut("rows").and_then(Value::as_array_mut) {
            rows.retain_mut(|row| self.filter_row(row));
        }
        spec.to_string()
    }

    /// Remaps one filter row in place; `false` drops it.
    fn filter_row(&self, row: &mut Value) -> bool {
        let Some(field) = row.get("field").and_then(Value::as_str).map(str::to_string) else {
            return true;
        };
        let map = match field.as_str() {
            "status" => &self.columns,
            "labels" => &self.labels,
            _ if field.starts_with("field:") => {
                let Some(new) = self.field_ref(&field) else {
                    return false;
                };
                row["field"] = Value::String(new);
                return true;
            }
            _ => return true,
        };
        let Some(values) = row.get_mut("values").and_then(Value::as_array_mut) else {
            return true;
        };
        let had = !values.is_empty();
        *values = values
            .iter()
            .filter_map(|value| map.get(value.as_str()?.trim()))
            .map(|new| Value::String(new.clone()))
            .collect();
        !(had && values.is_empty())
    }

    fn strings(&self, value: &mut Value, each: impl Fn(&str) -> Option<String>) {
        if let Some(items) = value.as_array_mut() {
            *items = items
                .iter()
                .filter_map(|item| item.as_str().and_then(&each))
                .map(Value::String)
                .collect();
        }
    }

    fn layout(&self, raw: &str) -> String {
        let Ok(mut layout) = serde_json::from_str::<Value>(raw) else {
            return "{}".into();
        };
        let Some(object) = layout.as_object_mut() else {
            return "{}".into();
        };
        if let Some(columns) = object.get_mut("columns") {
            self.strings(columns, |id| self.table_column(id));
        }
        if let Some(collapsed) = object.get_mut("collapsed") {
            self.strings(collapsed, |key| self.group_key(key));
        }
        if let Some(Value::Object(widths)) = object.get_mut("widths") {
            *widths = std::mem::take(widths)
                .into_iter()
                .filter_map(|(id, width)| Some((self.table_column(&id)?, width)))
                .collect::<Map<_, _>>();
        }
        if let Some(display) = object.get_mut("display") {
            self.display_value(display);
        }
        layout.to_string()
    }

    fn display_value(&self, display: &mut Value) {
        if let Some(fields) = display.pointer_mut("/face/fields") {
            self.strings(fields, |id| self.fields.get(id).cloned());
        }
        if let Some(swimlane) = display.get_mut("swimlane") {
            if let Some(lane) = swimlane.as_str().filter(|lane| lane.starts_with("field:")) {
                *swimlane = Value::String(self.field_ref(lane).unwrap_or_else(|| "none".into()));
            }
        }
    }

    fn display(&self, raw: &str) -> Option<String> {
        let mut display = serde_json::from_str::<Value>(raw).ok()?;
        self.display_value(&mut display);
        Some(display.to_string())
    }

    fn auto_move(&self, raw: &str) -> Option<String> {
        let mut rules = serde_json::from_str::<Value>(raw).ok()?;
        if let Some(rules) = rules.as_object_mut() {
            for rule in rules.values_mut() {
                let Some(old) = rule.get("column").and_then(Value::as_str) else {
                    continue;
                };
                *rule = match self.columns.get(old) {
                    Some(new) => json!({ "column": new }),
                    None => Value::String("stay".into()),
                };
            }
        }
        Some(rules.to_string())
    }
}

fn create_from_spec(
    tx: &Transaction<'_>,
    name: &str,
    spec: &TemplateSpec,
) -> Result<Board, String> {
    let now = now_millis();
    let board = Board {
        id: new_id(),
        name: name.to_string(),
        created_at: now,
        updated_at: now,
    };
    tx.execute(
        "INSERT INTO boards (id, name, card_seq, created_at, updated_at)
         VALUES (?1, ?2, 0, ?3, ?3)",
        params![board.id, board.name, now],
    )
    .map_err(db)?;
    let mut remap = Remap {
        columns: IdMap::new(),
        labels: IdMap::new(),
        fields: IdMap::new(),
    };
    for (position, column) in spec.columns.iter().enumerate() {
        let id = new_id();
        tx.execute(
            "INSERT INTO board_columns (id, board_id, name, position, role, color, wip_limit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                board.id,
                column.name,
                position as i64,
                column.role,
                column.color,
                column.wip_limit
            ],
        )
        .map_err(db)?;
        remap.columns.insert(column.id.clone(), id);
    }
    for (position, label) in spec.labels.iter().enumerate() {
        let id = new_id();
        tx.execute(
            "INSERT INTO board_labels (id, board_id, name, color, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, board.id, label.name, label.color, position as i64],
        )
        .map_err(db)?;
        remap.labels.insert(label.id.clone(), id);
    }
    for (position, field) in spec.fields.iter().enumerate() {
        let id = new_id();
        tx.execute(
            "INSERT INTO board_fields (id, board_id, name, kind, options_json, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                board.id,
                field.name,
                field.kind,
                field.options_json,
                position as i64
            ],
        )
        .map_err(db)?;
        remap.fields.insert(field.id.clone(), id);
    }
    let mut views = IdMap::new();
    for (position, view) in spec.views.iter().enumerate() {
        let id = new_id();
        tx.execute(
            "INSERT INTO board_views
               (id, board_id, name, position, spec_json, layout_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                board.id,
                view.name,
                position as i64,
                remap.view_spec(&view.spec_json),
                remap.layout(&view.layout_json),
                now
            ],
        )
        .map_err(db)?;
        views.insert(view.id.clone(), id);
    }
    let settings = &spec.settings;
    tx.execute(
        "INSERT INTO board_settings
           (board_id, run_with, write_back, ticket_status_changes, auto_move_json,
            icon, color, default_view_id, display_json, priority_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            board.id,
            i64::from(settings.run_with),
            i64::from(settings.write_back),
            i64::from(settings.ticket_status_changes),
            settings
                .auto_move_json
                .as_deref()
                .and_then(|raw| remap.auto_move(raw)),
            settings.icon,
            settings.color,
            settings
                .default_view_id
                .as_ref()
                .and_then(|id| views.get(id)),
            settings
                .display_json
                .as_deref()
                .and_then(|raw| remap.display(raw)),
            settings.priority_json
        ],
    )
    .map_err(db)?;
    Ok(board)
}

/// Makes a board named `name` from a built-in or saved template.
pub fn create_board_from_template(
    conn: &Connection,
    name: &str,
    template_id: &str,
) -> Result<Board, String> {
    let name = clean_text(name, "Board name", MAX_BOARD_NAME)?;
    let tx = write_tx(conn)?;
    let spec = match built_in_spec(template_id) {
        Some(spec) => spec,
        None => {
            let raw: String = tx
                .query_row(
                    "SELECT spec_json FROM board_templates WHERE id = ?1",
                    [template_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?
                .ok_or_else(|| "Template not found".to_string())?;
            serde_json::from_str(&raw).map_err(|_| "Template is not valid".to_string())?
        }
    };
    let board = create_from_spec(&tx, &name, &spec)?;
    tx.commit().map_err(db)?;
    Ok(board)
}

/// Saves the board's columns, labels, fields, views and settings (no cards)
/// as a template named `name`.
pub fn save_board_as_template(
    conn: &Connection,
    board_id: &str,
    name: &str,
) -> Result<TemplateInfo, String> {
    let name = clean_text(name, "Template name", MAX_TEMPLATE_NAME)?;
    let tx = write_tx(conn)?;
    let settings = board_settings(&tx, board_id)?;
    let columns = crate::get_board(&tx, board_id)?
        .columns
        .into_iter()
        .map(|column| TemplateColumn {
            id: column.id,
            name: column.name,
            role: column.role,
            color: column.color,
            wip_limit: column.wip_limit,
        })
        .collect();
    let spec = TemplateSpec {
        columns,
        labels: list_labels(&tx, board_id)?
            .into_iter()
            .map(|label| TemplateLabel {
                id: label.id,
                name: label.name,
                color: label.color,
            })
            .collect(),
        fields: list_fields(&tx, board_id)?
            .into_iter()
            .map(|field| TemplateField {
                id: field.id,
                name: field.name,
                kind: field.kind,
                options_json: field.options_json,
            })
            .collect(),
        views: list_views(&tx, board_id)?
            .into_iter()
            .map(|view| TemplateView {
                id: view.id,
                name: view.name,
                spec_json: view.spec_json,
                layout_json: view.layout_json,
            })
            .collect(),
        settings,
    };
    let template = TemplateInfo {
        id: new_id(),
        name,
        built_in: false,
    };
    tx.execute(
        "INSERT INTO board_templates (id, name, spec_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            template.id,
            template.name,
            serde_json::to_string(&spec).map_err(|error| error.to_string())?,
            now_millis()
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(template)
}

pub fn delete_template(conn: &Connection, id: &str) -> Result<(), String> {
    if BUILT_IN_TEMPLATES.contains(&id) {
        return Err("Built-in templates can't be deleted".into());
    }
    let deleted = conn
        .execute("DELETE FROM board_templates WHERE id = ?1", [id])
        .map_err(db)?;
    if deleted == 0 {
        return Err("Template not found".into());
    }
    Ok(())
}

/// The built-in "blank" board, as `create_board` makes it.
pub(crate) fn create_blank(tx: &Transaction<'_>, name: &str) -> Result<Board, String> {
    let spec = built_in_spec("blank").expect("blank is built in");
    create_from_spec(tx, name, &spec)
}
