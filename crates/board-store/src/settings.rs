//! Per-board settings (`board_settings`), the display options and priority
//! scheme stored in them, and the per-session ticket status override
//! (`board_session_settings`).
//!
//! Both apps write settings at once (one toggles write-back while the other
//! renames the board), so writes are patches: [`update_board_settings`]
//! changes only the fields it is given.
use crate::db::{args, execute, query_opt, query_value, with_db, with_tx, Arg, Conn, Ex};
use crate::models::SessionSettingRow;
use crate::ops::check_color;
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
/// column, reads as the defaults: switches on, nothing else set. The exception
/// is `run_in_worktree`: only a board made since v8 has it on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardSettings {
    pub run_with: bool,
    pub run_in_worktree: bool,
    /// What "Run with…" last had ticked on this board, if it was ever used.
    pub run_worktree_choice: Option<bool>,
    pub write_back: bool,
    pub ticket_status_changes: bool,
    pub auto_move_json: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub default_view_id: Option<String>,
    pub display_json: Option<String>,
    pub priority_json: Option<String>,
}

impl BoardSettings {
    /// Whether "Run with…" opens with a new worktree ticked.
    pub fn starts_in_worktree(&self) -> bool {
        self.run_worktree_choice.unwrap_or(self.run_in_worktree)
    }
}

impl Default for BoardSettings {
    fn default() -> Self {
        Self {
            run_with: true,
            run_in_worktree: true,
            run_worktree_choice: None,
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
    pub run_in_worktree: Option<bool>,
    pub run_worktree_choice: Option<Option<bool>>,
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
            run_in_worktree: Some(settings.run_in_worktree),
            run_worktree_choice: Some(settings.run_worktree_choice),
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

pub(crate) async fn board_settings_in(
    ex: &mut Ex<'_>,
    board_id: &str,
) -> Result<BoardSettings, String> {
    let row = query_opt(
        ex,
        "SELECT s.run_with, s.write_back, s.ticket_status_changes, s.auto_move_json,
                s.icon, s.color, s.default_view_id, s.display_json, s.priority_json,
                s.run_in_worktree, s.run_worktree_choice
         FROM boards b LEFT JOIN board_settings s ON s.board_id = b.id
         WHERE b.id = ?1",
        args![board_id],
    )
    .await?
    .ok_or_else(|| "Board not found".to_string())?;
    let on = |value: Option<i64>| value.is_none_or(|v| v != 0);
    Ok(BoardSettings {
        run_with: on(row.get(0)?),
        write_back: on(row.get(1)?),
        ticket_status_changes: on(row.get(2)?),
        auto_move_json: row.get(3)?,
        icon: row.get(4)?,
        color: row.get(5)?,
        default_view_id: row.get(6)?,
        display_json: row.get(7)?,
        priority_json: row.get(8)?,
        run_in_worktree: row.get::<Option<i64>>(9)?.is_some_and(|v| v != 0),
        run_worktree_choice: row.get::<Option<i64>>(10)?.map(|v| v != 0),
    })
}

pub fn board_settings(conn: &Conn, board_id: &str) -> Result<BoardSettings, String> {
    with_db!(conn, |ex| board_settings_in(ex, board_id).await)
}

/// Checks every value the patch sets that needs no database. Returns the
/// priority scheme to store when the patch sets one: its names trimmed, as
/// normalised JSON.
fn check_patch(patch: &SettingsPatch) -> Result<Option<Option<String>>, String> {
    if let Some(Some(raw)) = &patch.auto_move_json {
        if raw.len() > MAX_SETTINGS_JSON {
            return Err("Auto-move rules are too long".into());
        }
        serde_json::from_str::<serde_json::Value>(raw)
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
    let priority = match &patch.priority_json {
        Some(Some(raw)) => {
            let mut scheme: PriorityScheme = parse_json(raw, "Priority scheme")?;
            check_priority(&scheme)?;
            for name in &mut scheme.names {
                *name = name.trim().to_string();
            }
            Some(Some(
                serde_json::to_string(&scheme).map_err(|error| error.to_string())?,
            ))
        }
        other => other.clone(),
    };
    Ok(priority)
}

/// Changes only the fields `patch` names and returns the settings after.
pub fn update_board_settings(
    conn: &Conn,
    board_id: &str,
    patch: &SettingsPatch,
) -> Result<BoardSettings, String> {
    let priority_json = check_patch(patch)?;
    with_tx!(conn, |ex| {
        let exists: bool = query_value(
            ex,
            "SELECT EXISTS(SELECT 1 FROM boards WHERE id = ?1)",
            args![board_id],
        )
        .await?;
        if !exists {
            return Err("Board not found".into());
        }
        if let Some(Some(view_id)) = &patch.default_view_id {
            let on_board: bool = query_value(
                ex,
                "SELECT EXISTS(SELECT 1 FROM board_views WHERE id = ?1 AND board_id = ?2)",
                args![view_id, board_id],
            )
            .await?;
            if !on_board {
                return Err("The default view must be a view on this board".into());
            }
        }
        // The values the hand-made table's defaults gave a new row.
        execute(
            ex,
            "INSERT OR IGNORE INTO board_settings
               (board_id, run_with, write_back, ticket_status_changes)
             VALUES (?1, 1, 1, 1)",
            args![board_id],
        )
        .await?;
        let flag = |value: Option<bool>| value.map(Arg::from);
        let set = |value: &Option<Option<String>>| value.clone().map(Arg::from);
        let assignments: [(&str, Option<Arg>); 11] = [
            ("run_with", flag(patch.run_with)),
            ("run_in_worktree", flag(patch.run_in_worktree)),
            (
                "run_worktree_choice",
                patch.run_worktree_choice.map(Arg::from),
            ),
            ("write_back", flag(patch.write_back)),
            ("ticket_status_changes", flag(patch.ticket_status_changes)),
            ("auto_move_json", set(&patch.auto_move_json)),
            ("icon", set(&patch.icon)),
            ("color", set(&patch.color)),
            ("default_view_id", set(&patch.default_view_id)),
            ("display_json", set(&patch.display_json)),
            ("priority_json", priority_json.map(Arg::from)),
        ];
        for (column, value) in assignments {
            if let Some(value) = value {
                execute(
                    ex,
                    &format!("UPDATE board_settings SET {column} = ?1 WHERE board_id = ?2"),
                    vec![value, Arg::from(board_id)],
                )
                .await?;
            }
        }
        board_settings_in(ex, board_id).await
    })
}

/// Overwrites every field of the board's settings with `settings` (a patch
/// naming all of them), including any another app changed meanwhile. For a
/// partial change, call [`update_board_settings`] with just those fields.
pub fn set_board_settings(
    conn: &Conn,
    board_id: &str,
    settings: &BoardSettings,
) -> Result<(), String> {
    update_board_settings(conn, board_id, &SettingsPatch::full(settings)).map(drop)
}

/// A session key as both apps persist it (a local id, or
/// `remote:{env}:{session}`): not blank, at most 500 bytes, no control
/// characters.
pub fn check_session_key(session_key: &str) -> Result<(), String> {
    if session_key.trim().is_empty()
        || session_key.len() > MAX_SESSION_KEY
        || session_key.chars().any(char::is_control)
    {
        return Err("Invalid session key".into());
    }
    Ok(())
}

async fn override_of(ex: &mut Ex<'_>, session_key: &str) -> Result<Option<bool>, String> {
    Ok(SessionSettingRow::filter_by_session_key(session_key)
        .first()
        .exec(ex)
        .await
        .map_err(crate::db::err)?
        .and_then(|row| row.may_change_status))
}

/// The session's own "may change ticket status" override; `Ok(None)` when it
/// follows its boards. A read that fails is an error, never `Ok(None)`, so a
/// caller can't mistake it for "follow the boards" ([`may_change_ticket_status`]
/// refuses in that case rather than falling back to the boards).
pub fn session_status_setting(conn: &Conn, session_key: &str) -> Result<Option<bool>, String> {
    with_db!(conn, |ex| override_of(ex, session_key).await)
}

/// Sets the session's override, or with `None` makes it follow its boards.
pub fn set_session_status_setting(
    conn: &Conn,
    session_key: &str,
    may_change: Option<bool>,
) -> Result<(), String> {
    check_session_key(session_key)?;
    with_db!(conn, |ex| {
        match may_change {
            Some(value) => {
                execute(
                    ex,
                    "INSERT INTO board_session_settings (session_key, may_change_status)
                     VALUES (?1, ?2)
                     ON CONFLICT (session_key) DO UPDATE SET
                       may_change_status = excluded.may_change_status",
                    args![session_key, value],
                )
                .await
            }
            None => {
                execute(
                    ex,
                    "DELETE FROM board_session_settings WHERE session_key = ?1",
                    args![session_key],
                )
                .await
            }
        }
        .map(drop)
    })
}

/// Whether the agent in this session may change its ticket's status: the
/// session override when set; else the strictest `ticket_status_changes` of
/// the boards whose cards link the session (through `board_card_links` or
/// the card's own legacy `link_kind`/`link_id` columns); else `true`. A read
/// that fails answers `false`.
pub fn may_change_ticket_status(conn: &Conn, session_key: &str) -> bool {
    with_db!(conn, |ex| {
        match override_of(ex, session_key).await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(_) => return Ok(false),
        }
        Ok(query_value::<i64>(
            ex,
            "SELECT COALESCE(MIN(COALESCE(s.ticket_status_changes, 1)), 1)
             FROM board_cards c
             LEFT JOIN board_settings s ON s.board_id = c.board_id
             WHERE c.id IN (SELECT card_id FROM board_card_links
                            WHERE kind = 'session' AND ref = ?1)
                OR (c.link_kind = 'session' AND c.link_id = ?1)",
            args![session_key],
        )
        .await
        .map(|value| value != 0)
        .unwrap_or(false))
    })
    .unwrap_or(false)
}
