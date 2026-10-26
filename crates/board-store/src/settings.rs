//! Per-board settings (`board_settings`), the display options and priority
//! scheme stored in them, and the per-session ticket status override
//! (`board_session_settings`).
//!
//! Both apps write settings at once (one toggles write-back while the other
//! renames the board), so writes are patches: [`update_board_settings`]
//! changes only the fields it is given.
use crate::ops::{check_color, db, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// The icons a board can show.
pub const BOARD_ICONS: [&str; 8] = [
    "board", "bug", "rocket", "check", "star", "flag", "bolt", "book",
];
const MAX_SETTINGS_JSON: usize = 20_000;
const MAX_SESSION_KEY: usize = 500;
const MAX_PRIORITY_NAME: usize = 20;
const SWIMLANES: [&str; 4] = ["none", "priority", "label", "project"];
const AGENT_FIELDS: [&str; 4] = ["provider", "model", "cost", "runTime"];

/// Per-board settings. A board without a row (made before v3), or a NULL
/// column, reads as the defaults: switches on, nothing else set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardSettings {
    pub run_with: bool,
    pub write_back: bool,
    pub ticket_status_changes: bool,
    pub auto_move_json: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub default_view_id: Option<String>,
    pub display_json: Option<String>,
    pub priority_json: Option<String>,
}

impl Default for BoardSettings {
    fn default() -> Self {
        Self {
            run_with: true,
            write_back: true,
            ticket_status_changes: true,
            auto_move_json: None,
            icon: None,
            color: None,
            default_view_id: None,
            display_json: None,
            priority_json: None,
        }
    }
}

/// The settings fields to change. `None` leaves a field as it is; for the
/// optional fields `Some(None)` clears it back to the default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsPatch {
    pub run_with: Option<bool>,
    pub write_back: Option<bool>,
    pub ticket_status_changes: Option<bool>,
    pub auto_move_json: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub default_view_id: Option<Option<String>>,
    pub display_json: Option<Option<String>>,
    pub priority_json: Option<Option<String>>,
}

impl SettingsPatch {
    /// A patch naming every field, set to `settings`.
    fn full(settings: &BoardSettings) -> Self {
        Self {
            run_with: Some(settings.run_with),
            write_back: Some(settings.write_back),
            ticket_status_changes: Some(settings.ticket_status_changes),
            auto_move_json: Some(settings.auto_move_json.clone()),
            icon: Some(settings.icon.clone()),
            color: Some(settings.color.clone()),
            default_view_id: Some(settings.default_view_id.clone()),
            display_json: Some(settings.display_json.clone()),
            priority_json: Some(settings.priority_json.clone()),
        }
    }
}

/// Which parts a card face shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CardFace {
    pub labels: bool,
    pub priority: bool,
    pub due: bool,
    pub subtasks: bool,
    pub comments: bool,
    pub link_status: bool,
    pub number: bool,
    /// Custom field ids, in the order shown. A field deleted since is ignored
    /// by whoever draws the face.
    pub fields: Vec<String>,
    /// Any of `provider`, `model`, `cost`, `runTime`.
    pub agent_fields: Vec<String>,
}

impl Default for CardFace {
    fn default() -> Self {
        Self {
            labels: true,
            priority: true,
            due: true,
            subtasks: true,
            comments: true,
            link_status: true,
            number: true,
            fields: Vec::new(),
            agent_fields: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// How a board (or a saved view, under `layout_json.display`) draws cards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DisplayOptions {
    pub face: CardFace,
    pub density: Density,
    /// `none`, `priority`, `label`, `project` or `field:<id>`.
    pub swimlane: String,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            face: CardFace::default(),
            density: Density::Comfortable,
            swimlane: "none".into(),
        }
    }
}

/// The five priority names, and the levels hidden from pickers and filters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PriorityScheme {
    pub names: [String; 5],
    pub hidden: Vec<u8>,
}

impl Default for PriorityScheme {
    fn default() -> Self {
        Self {
            names: crate::PRIORITIES.map(str::to_string),
            hidden: Vec::new(),
        }
    }
}

fn valid_swimlane(swimlane: &str) -> bool {
    SWIMLANES.contains(&swimlane)
        || swimlane
            .strip_prefix("field:")
            .is_some_and(|id| crate::validate_id(id, "field").is_ok())
}

fn check_display(display: &DisplayOptions) -> Result<(), String> {
    if !valid_swimlane(&display.swimlane) {
        return Err(format!("Unknown swimlane: {}", display.swimlane));
    }
    if let Some(bad) = display
        .face
        .agent_fields
        .iter()
        .find(|field| !AGENT_FIELDS.contains(&field.as_str()))
    {
        return Err(format!("Unknown agent field: {bad}"));
    }
    Ok(())
}

fn check_priority(scheme: &PriorityScheme) -> Result<(), String> {
    if scheme
        .names
        .iter()
        .any(|name| name.trim().is_empty() || name.trim().chars().count() > MAX_PRIORITY_NAME)
    {
        return Err(format!(
            "Priority names must be 1 to {MAX_PRIORITY_NAME} characters"
        ));
    }
    let mut seen = Vec::new();
    for &level in &scheme.hidden {
        if level == 0 {
            return Err("Priority level 0 can't be hidden".into());
        }
        if usize::from(level) >= scheme.names.len() || seen.contains(&level) {
            return Err("Hidden priority levels are not valid".into());
        }
        seen.push(level);
    }
    Ok(())
}

fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str, label: &str) -> Result<T, String> {
    if raw.len() > MAX_SETTINGS_JSON {
        return Err(format!("{label} is too long"));
    }
    serde_json::from_str(raw).map_err(|_| format!("{label} is not valid"))
}

/// The board's display options. Missing, unreadable or invalid JSON reads as
/// the defaults; unknown keys are ignored.
pub fn display_options(settings: &BoardSettings) -> DisplayOptions {
    settings
        .display_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<DisplayOptions>(raw).ok())
        .filter(|display| check_display(display).is_ok())
        .unwrap_or_default()
}

/// The board's priority scheme. Missing, unreadable or invalid JSON reads as
/// the defaults; unknown keys are ignored.
pub fn priority_scheme(settings: &BoardSettings) -> PriorityScheme {
    settings
        .priority_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<PriorityScheme>(raw).ok())
        .filter(|scheme| check_priority(scheme).is_ok())
        .unwrap_or_default()
}

pub fn board_settings(conn: &Connection, board_id: &str) -> Result<BoardSettings, String> {
    let row = conn
        .query_row(
            "SELECT s.run_with, s.write_back, s.ticket_status_changes, s.auto_move_json,
                    s.icon, s.color, s.default_view_id, s.display_json, s.priority_json
             FROM boards b LEFT JOIN board_settings s ON s.board_id = b.id
             WHERE b.id = ?1",
            [board_id],
            |r| {
                let on = |value: Option<i64>| value.is_none_or(|v| v != 0);
                Ok(BoardSettings {
                    run_with: on(r.get(0)?),
                    write_back: on(r.get(1)?),
                    ticket_status_changes: on(r.get(2)?),
                    auto_move_json: r.get(3)?,
                    icon: r.get(4)?,
                    color: r.get(5)?,
                    default_view_id: r.get(6)?,
                    display_json: r.get(7)?,
                    priority_json: r.get(8)?,
                })
            },
        )
        .optional()
        .map_err(db)?;
    row.ok_or_else(|| "Board not found".into())
}

/// Checks every value the patch sets that needs no database.
fn check_patch(patch: &SettingsPatch) -> Result<(), String> {
    if let Some(Some(raw)) = &patch.auto_move_json {
        parse_json::<serde_json::Value>(raw, "Auto-move rules")
            .map_err(|_| "Auto-move rules are not valid JSON".to_string())?;
    }
    if let Some(Some(icon)) = &patch.icon {
        if !BOARD_ICONS.contains(&icon.as_str()) {
            return Err(format!("Unknown board icon: {icon}"));
        }
    }
    if let Some(Some(color)) = &patch.color {
        check_color(color)?;
    }
    if let Some(Some(raw)) = &patch.display_json {
        check_display(&parse_json(raw, "Display options")?)?;
    }
    if let Some(Some(raw)) = &patch.priority_json {
        check_priority(&parse_json(raw, "Priority scheme")?)?;
    }
    Ok(())
}

/// Changes only the fields `patch` names and returns the settings after.
pub fn update_board_settings(
    conn: &Connection,
    board_id: &str,
    patch: &SettingsPatch,
) -> Result<BoardSettings, String> {
    check_patch(patch)?;
    let tx = write_tx(conn)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM boards WHERE id = ?1)",
            [board_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    if !exists {
        return Err("Board not found".into());
    }
    if let Some(Some(view_id)) = &patch.default_view_id {
        let on_board: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_views WHERE id = ?1 AND board_id = ?2)",
                params![view_id, board_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !on_board {
            return Err("The default view must be a view on this board".into());
        }
    }
    tx.execute(
        "INSERT OR IGNORE INTO board_settings (board_id) VALUES (?1)",
        [board_id],
    )
    .map_err(db)?;
    let flag = |value: Option<bool>| value.map(i64::from);
    let set = |value: &Option<Option<String>>| value.clone();
    let assignments: [(&str, Option<rusqlite::types::Value>); 9] = [
        ("run_with", flag(patch.run_with).map(Into::into)),
        ("write_back", flag(patch.write_back).map(Into::into)),
        (
            "ticket_status_changes",
            flag(patch.ticket_status_changes).map(Into::into),
        ),
        ("auto_move_json", set(&patch.auto_move_json).map(Into::into)),
        ("icon", set(&patch.icon).map(Into::into)),
        ("color", set(&patch.color).map(Into::into)),
        (
            "default_view_id",
            set(&patch.default_view_id).map(Into::into),
        ),
        ("display_json", set(&patch.display_json).map(Into::into)),
        ("priority_json", set(&patch.priority_json).map(Into::into)),
    ];
    for (column, value) in assignments {
        if let Some(value) = value {
            tx.execute(
                &format!("UPDATE board_settings SET {column} = ?1 WHERE board_id = ?2"),
                params![value, board_id],
            )
            .map_err(db)?;
        }
    }
    let settings = board_settings(&tx, board_id)?;
    tx.commit().map_err(db)?;
    Ok(settings)
}

/// Writes every field of `settings` (a patch naming all of them).
pub fn set_board_settings(
    conn: &Connection,
    board_id: &str,
    settings: &BoardSettings,
) -> Result<(), String> {
    update_board_settings(conn, board_id, &SettingsPatch::full(settings)).map(drop)
}

fn check_session_key(session_key: &str) -> Result<(), String> {
    if session_key.trim().is_empty() || session_key.len() > MAX_SESSION_KEY {
        return Err("Invalid session key".into());
    }
    Ok(())
}

/// The session's own "may change ticket status" override; `None` when it
/// follows its boards (or cannot be read).
pub fn session_status_setting(conn: &Connection, session_key: &str) -> Option<bool> {
    conn.query_row(
        "SELECT may_change_status FROM board_session_settings WHERE session_key = ?1",
        [session_key],
        |r| r.get::<_, Option<i64>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .flatten()
    .map(|value| value != 0)
}

/// Sets the session's override, or with `None` makes it follow its boards.
pub fn set_session_status_setting(
    conn: &Connection,
    session_key: &str,
    may_change: Option<bool>,
) -> Result<(), String> {
    check_session_key(session_key)?;
    match may_change {
        Some(value) => conn.execute(
            "INSERT INTO board_session_settings (session_key, may_change_status)
             VALUES (?1, ?2)
             ON CONFLICT (session_key) DO UPDATE SET
               may_change_status = excluded.may_change_status",
            params![session_key, i64::from(value)],
        ),
        None => conn.execute(
            "DELETE FROM board_session_settings WHERE session_key = ?1",
            [session_key],
        ),
    }
    .map(drop)
    .map_err(db)
}

/// Whether the agent in this session may change its ticket's status: the
/// session override when set; else the strictest `ticket_status_changes` of
/// the boards whose cards link the session; else `true`. A read that fails
/// answers `false`.
pub fn may_change_ticket_status(conn: &Connection, session_key: &str) -> bool {
    if let Some(value) = session_status_setting(conn, session_key) {
        return value;
    }
    conn.query_row(
        "SELECT COALESCE(MIN(COALESCE(s.ticket_status_changes, 1)), 1)
         FROM board_card_links l
         JOIN board_cards c ON c.id = l.card_id
         LEFT JOIN board_settings s ON s.board_id = c.board_id
         WHERE l.kind = 'session' AND l.ref = ?1",
        [session_key],
        |r| r.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .unwrap_or(false)
}
