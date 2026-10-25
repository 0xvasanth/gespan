//! The board, over the same `board*` tables the Tauri app keeps in `gespan.db`.
//! The storage itself (tables, reads, writes, validation) is the shared
//! `gespan-board-store` crate both apps call; this module adds the native
//! app's `Service` handle and its extras.
use gespan_board_store::{
    cards_for_link, create_board, create_card, create_column, delete_board, delete_card,
    delete_column, get_board, link_card, list_activity, list_boards, move_card, move_column,
    record_card, rename_board, update_card, update_column,
};
// What `extras` builds on.
use gespan_board_store::{
    clean_text, db, get_card, log_activity, new_id, now_millis, validate_id, write_tx, NewActivity,
};
use rusqlite::Connection;
use std::path::Path;

pub use gespan_board_store::{Activity, Board, BoardDetail, Card, Column};

pub mod auto_move;
pub mod extras;
pub mod linked_turns;

/// The shared board tables plus the native-only `board_native_*` extras.
pub(crate) fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    gespan_board_store::ensure_tables(conn)?;
    extras::ensure_tables(conn)
}

/// The native app's handle on the shared board tables. Open it with the
/// session database path so boards made in either app are the same boards.
pub struct Service {
    conn: Connection,
}

impl Service {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        // WAL, as the native store and the Tauri app open this file, so two
        // writers wait on the busy timeout rather than on rollback-journal
        // file locks, which on Windows can outlast it.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        ensure_tables(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn list(&mut self) -> Result<Vec<Board>, String> {
        list_boards(&self.conn)
    }

    pub fn get(&mut self, board_id: &str) -> Result<BoardDetail, String> {
        validate_id(board_id, "board")?;
        get_board(&self.conn, board_id)
    }

    pub fn create(&mut self, name: &str) -> Result<Board, String> {
        create_board(&self.conn, name)
    }

    pub fn rename(&mut self, board_id: &str, name: &str) -> Result<(), String> {
        validate_id(board_id, "board")?;
        rename_board(&self.conn, board_id, name)
    }

    pub fn delete(&mut self, board_id: &str) -> Result<(), String> {
        validate_id(board_id, "board")?;
        delete_board(&self.conn, board_id)
    }

    pub fn column_create(
        &mut self,
        board_id: &str,
        name: &str,
        role: Option<&str>,
    ) -> Result<Column, String> {
        validate_id(board_id, "board")?;
        create_column(&self.conn, board_id, name, role)
    }

    pub fn column_update(
        &mut self,
        column_id: &str,
        name: &str,
        role: Option<&str>,
    ) -> Result<(), String> {
        validate_id(column_id, "column")?;
        update_column(&self.conn, column_id, name, role).map(drop)
    }

    pub fn column_move(&mut self, column_id: &str, index: usize) -> Result<(), String> {
        validate_id(column_id, "column")?;
        move_column(&self.conn, column_id, index).map(drop)
    }

    pub fn column_delete(&mut self, column_id: &str) -> Result<(), String> {
        validate_id(column_id, "column")?;
        delete_column(&self.conn, column_id).map(drop)
    }

    pub fn card_create(
        &mut self,
        column_id: &str,
        title: &str,
        body: &str,
    ) -> Result<Card, String> {
        validate_id(column_id, "column")?;
        create_card(&self.conn, column_id, title, body)
    }

    pub fn card_update(&mut self, card_id: &str, title: &str, body: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        update_card(&self.conn, card_id, title, body).map(drop)
    }

    pub fn card_move(
        &mut self,
        card_id: &str,
        column_id: &str,
        index: usize,
        actor: &str,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        validate_id(column_id, "column")?;
        move_card(&self.conn, card_id, column_id, index, actor).map(drop)
    }

    pub fn card_link(
        &mut self,
        card_id: &str,
        kind: Option<&str>,
        id: Option<&str>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        link_card(&self.conn, card_id, kind, id).map(drop)
    }

    pub fn card_delete(&mut self, card_id: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        delete_card(&self.conn, card_id).map(drop)
    }

    pub fn cards_for_link(&mut self, kind: &str, id: &str) -> Result<Vec<Card>, String> {
        cards_for_link(&self.conn, kind, id)
    }

    pub fn activity(
        &mut self,
        board_id: &str,
        card_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Activity>, String> {
        validate_id(board_id, "board")?;
        if let Some(card_id) = card_id {
            validate_id(card_id, "card")?;
        }
        list_activity(&self.conn, board_id, card_id, limit)
    }

    pub fn card_record(
        &mut self,
        card_id: &str,
        kind: &str,
        detail: Option<&str>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        record_card(&self.conn, card_id, kind, detail).map(drop)
    }

    /// SQLite's counter of commits made by other connections; a change means
    /// another app or window wrote to the database.
    pub fn data_version(&mut self) -> Result<i64, String> {
        self.conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn a_fresh_board_database_is_opened_in_wal_mode_like_the_apps_open_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        Service::open(&path).unwrap();
        let mode: String = Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn two_connections_interleaving_moves_keep_positions_contiguous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let mut a = Service::open(&path).unwrap();
        let mut b = Service::open(&path).unwrap();
        let board = a.create("Shared").unwrap();
        let columns = a.get(&board.id).unwrap().columns;
        let mut cards = Vec::new();
        for index in 0..6 {
            cards.push(
                a.card_create(&columns[0].id, &format!("Card {index}"), "")
                    .unwrap(),
            );
        }
        let mut seed: u64 = 7;
        for step in 0..20 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let card = &cards[(seed >> 33) as usize % cards.len()];
            let column = &columns[(seed >> 13) as usize % columns.len()];
            let index = (seed >> 7) as usize % 7;
            let service = if step % 2 == 0 { &mut a } else { &mut b };
            service
                .card_move(&card.id, &column.id, index, "user")
                .unwrap();
        }
        let detail = b.get(&board.id).unwrap();
        for column in &detail.columns {
            let mut positions: Vec<i64> = detail
                .cards
                .iter()
                .filter(|card| card.column_id == column.id)
                .map(|card| card.position)
                .collect();
            positions.sort();
            let expected: Vec<i64> = (0..positions.len() as i64).collect();
            assert_eq!(positions, expected, "column {} not contiguous", column.name);
        }
        assert_eq!(detail.cards.len(), 6);
    }

    #[test]
    fn service_rejects_malformed_ids_like_the_tauri_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut service = Service::open(&dir.path().join("gespan.db")).unwrap();
        let bad = "x'; DROP TABLE boards; --";
        assert_eq!(service.get(bad).unwrap_err(), "Invalid board id");
        assert_eq!(service.delete(bad).unwrap_err(), "Invalid board id");
        assert_eq!(
            service.column_move(bad, 0).unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            service.card_create(bad, "Title", "").unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            service.card_move(bad, "column", 0, "user").unwrap_err(),
            "Invalid card id"
        );
        assert_eq!(
            service.card_move("card", bad, 0, "user").unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            service.activity("board", Some(bad), 10).unwrap_err(),
            "Invalid card id"
        );
        assert_eq!(
            service.card_record(bad, "started", None).unwrap_err(),
            "Invalid card id"
        );
    }

    #[test]
    fn two_connections_moving_cards_at_once_lose_no_card_and_keep_columns_contiguous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let mut setup = Service::open(&path).unwrap();
        let board = setup.create("Shared").unwrap();
        let columns = setup.get(&board.id).unwrap().columns;
        let cards: Vec<String> = (0..8)
            .map(|n| {
                setup
                    .card_create(&columns[n % 4].id, &format!("Card {n}"), "")
                    .unwrap()
                    .id
            })
            .collect();
        let column_ids: Vec<String> = columns.iter().map(|c| c.id.clone()).collect();
        let workers: Vec<_> = (0..2u64)
            .map(|worker| {
                let (path, cards, column_ids) = (path.clone(), cards.clone(), column_ids.clone());
                std::thread::spawn(move || {
                    let mut service = Service::open(&path).unwrap();
                    let mut seed = 11 + worker * 7919;
                    for _ in 0..150 {
                        seed = seed
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        let card = &cards[(seed >> 33) as usize % cards.len()];
                        let column = &column_ids[(seed >> 13) as usize % column_ids.len()];
                        let index = (seed >> 7) as usize % 9;
                        service.card_move(card, column, index, "user").unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let detail = setup.get(&board.id).unwrap();
        assert_eq!(detail.cards.len(), cards.len());
        for column in &detail.columns {
            let mut positions: Vec<i64> = detail
                .cards
                .iter()
                .filter(|card| card.column_id == column.id)
                .map(|card| card.position)
                .collect();
            positions.sort();
            let expected: Vec<i64> = (0..positions.len() as i64).collect();
            assert_eq!(positions, expected, "column {} has a gap", column.name);
        }
    }

    #[test]
    fn data_version_changes_after_another_connection_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let mut a = Service::open(&path).unwrap();
        let mut b = Service::open(&path).unwrap();
        let before = a.data_version().unwrap();
        b.create("Elsewhere").unwrap();
        assert_ne!(a.data_version().unwrap(), before);
    }
}
