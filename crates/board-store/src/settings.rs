use crate::ops::{db, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Per-board switches for agent work. A board without a row (made before v3)
/// reads as the defaults: everything on.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSettings {
    pub run_with: bool,
    pub write_back: bool,
    pub ticket_status_changes: bool,
    pub auto_move_json: Option<String>,
}

impl Default for BoardSettings {
    fn default() -> Self {
        Self {
            run_with: true,
            write_back: true,
            ticket_status_changes: true,
            auto_move_json: None,
        }
    }
}

pub fn board_settings(conn: &Connection, board_id: &str) -> Result<BoardSettings, String> {
    let row = conn
        .query_row(
            "SELECT s.run_with, s.write_back, s.ticket_status_changes, s.auto_move_json, b.id
             FROM boards b LEFT JOIN board_settings s ON s.board_id = b.id
             WHERE b.id = ?1",
            [board_id],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db)?;
    let Some((run_with, write_back, ticket_status_changes, auto_move_json)) = row else {
        return Err("Board not found".into());
    };
    let on = |value: Option<i64>| value.is_none_or(|v| v != 0);
    Ok(BoardSettings {
        run_with: on(run_with),
        write_back: on(write_back),
        ticket_status_changes: on(ticket_status_changes),
        auto_move_json,
    })
}

pub fn set_board_settings(
    conn: &Connection,
    board_id: &str,
    settings: &BoardSettings,
) -> Result<(), String> {
    if let Some(raw) = &settings.auto_move_json {
        serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|_| "Auto-move rules are not valid JSON".to_string())?;
    }
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
    tx.execute(
        "INSERT INTO board_settings
           (board_id, run_with, write_back, ticket_status_changes, auto_move_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (board_id) DO UPDATE SET
           run_with = excluded.run_with,
           write_back = excluded.write_back,
           ticket_status_changes = excluded.ticket_status_changes,
           auto_move_json = excluded.auto_move_json",
        params![
            board_id,
            i64::from(settings.run_with),
            i64::from(settings.write_back),
            i64::from(settings.ticket_status_changes),
            settings.auto_move_json
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)
}
