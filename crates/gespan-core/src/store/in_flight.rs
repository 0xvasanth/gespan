//! Chats with a turn in flight, kept current while they run so a quit or a
//! crash can continue them on the next launch. The desktop table is the one
//! the Tauri app writes (src-tauri/src/session_store.rs).
use super::{shared::has_table, Store};
use crate::interrupted;
use rusqlite::params;

const LEGACY_KEY: &str = "in-flight";
const TABLE: &str = "in_flight_sessions";

impl Store {
    /// Replace the list, in the given order. Remote and unknown sessions are
    /// skipped: the host owns remote turns.
    pub fn set_in_flight(&self, ids: &[i64]) -> Result<(), String> {
        if self.shared.is_none() {
            let ids: Vec<i64> = ids.iter().copied().filter(|id| *id > 0).collect();
            return self.save_setting(LEGACY_KEY, &serde_json::to_string(&ids).unwrap());
        }
        if !has_table(&self.conn, TABLE)? {
            return Ok(());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM in_flight_sessions", [])
            .map_err(|e| e.to_string())?;
        let mut index = 0_i64;
        for id in ids.iter().filter(|id| **id > 0) {
            let Ok(persisted) = self.persisted_session_id(*id) else {
                continue;
            };
            index += tx
                .execute(
                    "INSERT OR REPLACE INTO in_flight_sessions(session_id,cwd,sort_index)
                     SELECT id,cwd,?2 FROM sessions WHERE id=?1",
                    params![persisted, index],
                )
                .map_err(|e| e.to_string())? as i64;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Read and clear the list, so a second window or launch cannot take it twice.
    pub fn take_in_flight(&self) -> Result<Vec<i64>, String> {
        if self.shared.is_none() {
            let ids = match self.setting(LEGACY_KEY)? {
                Some(text) if !text.is_empty() => serde_json::from_str(&text).unwrap_or_default(),
                _ => Vec::new(),
            };
            self.save_setting(LEGACY_KEY, "")?;
            return Ok(ids);
        }
        if !has_table(&self.conn, TABLE)? {
            return Ok(Vec::new());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let persisted = tx
            .prepare("SELECT session_id FROM in_flight_sessions ORDER BY sort_index, session_id")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM in_flight_sessions", [])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(persisted
            .iter()
            .filter_map(|id| self.resolve_existing_session_id(id))
            .collect())
    }

    /// End a session's last turn as interrupted (see [`interrupted::interrupt`]).
    /// Returns whether anything was written.
    pub fn mark_turn_interrupted(&mut self, id: i64, now: u64) -> Result<bool, String> {
        let rows = self.load(id)?.rows;
        let writes = interrupted::interrupt(&rows, now);
        for (ix, row) in &writes {
            self.write_rows(id, *ix, std::slice::from_ref(row))?;
        }
        Ok(!writes.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolCall, ToolState};
    use crate::transcript::Row;
    use std::path::Path;

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }

    fn stores() -> Vec<Store> {
        vec![
            Store::open_gespan(Path::new(":memory:")).unwrap(),
            Store::open(Path::new(":memory:")).unwrap(),
        ]
    }

    #[test]
    fn the_list_is_ordered_and_taken_once() {
        for store in stores() {
            let a = store.create("codex", "/a").unwrap().id;
            let b = store.create("claude", "/b").unwrap().id;
            store.set_in_flight(&[b, a]).unwrap();
            assert_eq!(store.take_in_flight().unwrap(), vec![b, a]);
            assert!(store.take_in_flight().unwrap().is_empty());
        }
    }

    #[test]
    fn setting_the_list_replaces_it_and_skips_unknown_sessions() {
        for store in stores() {
            let a = store.create("codex", "/a").unwrap().id;
            let b = store.create("codex", "/b").unwrap().id;
            store.set_in_flight(&[a, b]).unwrap();
            store.set_in_flight(&[b, -7]).unwrap();
            assert_eq!(store.take_in_flight().unwrap(), vec![b]);
            store.set_in_flight(&[a]).unwrap();
            store.set_in_flight(&[]).unwrap();
            assert!(store.take_in_flight().unwrap().is_empty());
        }
    }

    #[test]
    fn the_desktop_table_holds_the_desktop_id_and_cwd() {
        // The Tauri app reads these rows on its next launch.
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = store.create("codex", "/work/app").unwrap().id;
        store.set_in_flight(&[id]).unwrap();
        let (session, cwd): (String, String) = store
            .conn
            .query_row("SELECT session_id,cwd FROM in_flight_sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(session, store.persisted_session_id(id).unwrap());
        assert_eq!(cwd, "/work/app");
    }

    #[test]
    fn a_list_the_desktop_app_wrote_is_taken_by_native() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = store.create("codex", "/work/app").unwrap().id;
        let persisted = store.persisted_session_id(id).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO in_flight_sessions(session_id,cwd,sort_index) VALUES(?1,'/work/app',0),('gone','/x',1)",
                [&persisted],
            )
            .unwrap();
        assert_eq!(store.take_in_flight().unwrap(), vec![id]);
    }

    #[test]
    fn marking_a_turn_persists_the_marker_and_cancelled_tools_once() {
        for mut store in stores() {
            let id = store.create("codex", "/a").unwrap().id;
            let mut running = ToolCall::new("t1", "Bash");
            running.state = ToolState::Running;
            store
                .write_rows(
                    id,
                    0,
                    &[
                        row("user", "Fix it"),
                        row("assistant", "On it"),
                        row("tool", &serde_json::to_string(&running).unwrap()),
                    ],
                )
                .unwrap();
            assert!(store.mark_turn_interrupted(id, 7).unwrap());
            let rows = store.load(id).unwrap().rows;
            assert!(interrupted::ends_interrupted(&rows));
            let tool: ToolCall = serde_json::from_str(&rows[2].text).unwrap();
            assert_eq!(tool.state, ToolState::Cancelled);
            assert!(!store.mark_turn_interrupted(id, 8).unwrap());
            assert_eq!(store.load(id).unwrap().rows.len(), 4);
        }
    }

    #[test]
    fn the_marker_is_a_desktop_interrupt_notice() {
        // The Tauri app auto-continues a chat whose last block is this notice.
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = store.create("codex", "/a").unwrap().id;
        store.write_rows(id, 0, &[row("user", "Go")]).unwrap();
        store.mark_turn_interrupted(id, 7).unwrap();
        let blocks = store.shared_blocks(id).unwrap();
        let last = blocks.last().unwrap();
        assert_eq!(last["role"], "system");
        assert_eq!(last["text"], interrupted::MESSAGE);
        assert_eq!(last["notice"], "interrupt");
    }
}
