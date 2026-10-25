//! Task fields beyond title and body: priority, due date, projects, labels,
//! subtasks and comments. The storage is the shared board-store's v3 tables;
//! these `Service` methods are the native app's handle on them.
use super::{validate_id, Board, Service};
use gespan_board_store as store;
use rusqlite::Connection;
use std::collections::HashMap;

pub use gespan_board_store::{CardExtras, Comment, Label, Subtask, PRIORITIES};

/// The pre-v3 native tables. Nothing reads them now that board-store has
/// copied them into its v3 tables; they are still created, and never dropped,
/// for one release so a build from before v3 opening this file finds what it
/// expects.
pub(crate) fn ensure_legacy_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS board_native_meta (
           card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE,
           priority INTEGER NOT NULL DEFAULT 0,
           due_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS board_native_projects (
           card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
           project TEXT NOT NULL,
           PRIMARY KEY (card_id, project)
         );
         CREATE TABLE IF NOT EXISTS board_native_labels (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           name TEXT NOT NULL,
           color TEXT NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS board_native_labels_name_idx
           ON board_native_labels (board_id, lower(name));
         CREATE TABLE IF NOT EXISTS board_native_card_labels (
           card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
           label_id TEXT NOT NULL REFERENCES board_native_labels(id) ON DELETE CASCADE,
           PRIMARY KEY (card_id, label_id)
         );
         CREATE TABLE IF NOT EXISTS board_native_subtasks (
           id TEXT PRIMARY KEY,
           card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
           title TEXT NOT NULL,
           done INTEGER NOT NULL DEFAULT 0,
           position INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS board_native_subtasks_card_idx
           ON board_native_subtasks (card_id, position);
         CREATE TABLE IF NOT EXISTS board_native_comments (
           id TEXT PRIMARY KEY,
           card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
           body TEXT NOT NULL,
           created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS board_native_comments_card_idx
           ON board_native_comments (card_id, created_at);",
    )
}

impl Service {
    /// The one board the native app shows: the oldest, made on first use.
    pub fn default_board(&mut self) -> Result<Board, String> {
        match self.list()?.into_iter().next() {
            Some(board) => Ok(board),
            None => self.create("Board"),
        }
    }

    /// Every card's extras on a board, keyed by card id. Cards with none are absent.
    pub fn extras(&mut self, board_id: &str) -> Result<HashMap<String, CardExtras>, String> {
        validate_id(board_id, "board")?;
        store::card_extras(&self.conn, board_id)
    }

    pub fn set_priority(&mut self, card_id: &str, priority: u8) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::set_priority(&self.conn, card_id, priority).map(drop)
    }

    pub fn set_due(&mut self, card_id: &str, due_at: Option<i64>) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::set_due(&self.conn, card_id, due_at).map(drop)
    }

    pub fn set_projects(&mut self, card_id: &str, projects: &[String]) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::set_projects(&self.conn, card_id, projects).map(drop)
    }

    pub fn labels(&mut self, board_id: &str) -> Result<Vec<Label>, String> {
        validate_id(board_id, "board")?;
        store::list_labels(&self.conn, board_id)
    }

    pub fn create_label(
        &mut self,
        board_id: &str,
        name: &str,
        color: &str,
    ) -> Result<Label, String> {
        validate_id(board_id, "board")?;
        store::create_label(&self.conn, board_id, name, color)
    }

    pub fn set_labels(&mut self, card_id: &str, label_ids: &[String]) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::set_card_labels(&self.conn, card_id, label_ids).map(drop)
    }

    pub fn subtasks(&mut self, card_id: &str) -> Result<Vec<Subtask>, String> {
        validate_id(card_id, "card")?;
        store::list_subtasks(&self.conn, card_id)
    }

    pub fn add_subtask(&mut self, card_id: &str, title: &str) -> Result<Subtask, String> {
        validate_id(card_id, "card")?;
        store::add_subtask(&self.conn, card_id, title)
    }

    pub fn toggle_subtask(&mut self, subtask_id: &str, done: bool) -> Result<(), String> {
        validate_id(subtask_id, "subtask")?;
        store::toggle_subtask(&self.conn, subtask_id, done).map(drop)
    }

    pub fn delete_subtask(&mut self, subtask_id: &str) -> Result<(), String> {
        validate_id(subtask_id, "subtask")?;
        store::delete_subtask(&self.conn, subtask_id).map(drop)
    }

    pub fn comments(&mut self, card_id: &str) -> Result<Vec<Comment>, String> {
        validate_id(card_id, "card")?;
        store::list_comments(&self.conn, card_id)
    }

    pub fn add_comment(&mut self, card_id: &str, body: &str) -> Result<Comment, String> {
        validate_id(card_id, "card")?;
        store::add_comment(&self.conn, card_id, body, "user")
    }
}

#[cfg(test)]
mod tests {
    use crate::board::Service;

    fn service() -> (tempfile::TempDir, Service, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let service = Service::open(&path).unwrap();
        (dir, service, path)
    }

    fn card(service: &mut Service) -> (String, String) {
        let board = service.default_board().unwrap();
        let backlog = service.get(&board.id).unwrap().columns[0].id.clone();
        let card = service.card_create(&backlog, "Task", "").unwrap();
        (board.id, card.id)
    }

    fn last_detail(service: &mut Service, board: &str, card: &str) -> Option<String> {
        service.activity(board, Some(card), 1).unwrap()[0]
            .detail
            .clone()
    }

    #[test]
    fn default_board_creates_one_board_once() {
        let (_dir, mut service, _) = service();
        let first = service.default_board().unwrap();
        let again = service.default_board().unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(service.list().unwrap().len(), 1);
        assert_eq!(service.get(&first.id).unwrap().columns.len(), 4);
    }

    #[test]
    fn default_board_is_the_oldest_of_several() {
        let (_dir, mut service, _) = service();
        // Alphabetical order differs from creation order.
        let oldest = service.create("Zeta").unwrap();
        service.create("Alpha").unwrap();
        service.create("Mid").unwrap();
        assert_eq!(service.default_board().unwrap().id, oldest.id);
        assert_eq!(service.list().unwrap().len(), 3);
    }

    #[test]
    fn priority_and_due_round_trip_and_log_an_edit() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        service.set_priority(&card, 3).unwrap();
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("priority: High")
        );
        service.set_due(&card, Some(1_793_404_800_000)).unwrap();
        let extras = service.extras(&board).unwrap();
        assert_eq!(extras[&card].priority, 3);
        assert_eq!(extras[&card].due_at, Some(1_793_404_800_000));
        service.set_due(&card, None).unwrap();
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("due: none")
        );
        assert_eq!(service.extras(&board).unwrap()[&card].due_at, None);
    }

    #[test]
    fn priority_above_urgent_is_rejected() {
        let (_dir, mut service, _) = service();
        let (_, card) = card(&mut service);
        assert!(service.set_priority(&card, 5).is_err());
    }

    #[test]
    fn set_projects_replaces_the_set_and_collapses_duplicates() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        service
            .set_projects(&card, &["/a".into(), "/b".into(), "/a".into()])
            .unwrap();
        let mut projects = service.extras(&board).unwrap()[&card].projects.clone();
        projects.sort();
        assert_eq!(projects, ["/a", "/b"]);
        service.set_projects(&card, &["/c".into()]).unwrap();
        assert_eq!(service.extras(&board).unwrap()[&card].projects, ["/c"]);
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("projects: 1")
        );
    }

    #[test]
    fn labels_are_unique_per_board_ignoring_case_and_attach_to_cards() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        assert!(service.create_label(&board, "  bug ", "4da3f5").is_err());
        let docs = service.create_label(&board, "Docs", "4da3f5").unwrap();
        // Case is folded beyond ASCII, and colours are plain hex.
        service.create_label(&board, "Ärger", "e0823d").unwrap();
        assert!(service.create_label(&board, "ärger", "e0823d").is_err());
        assert!(service.create_label(&board, "Red", "+12345").is_err());
        assert!(service.create_label(&board, "Red", "red").is_err());
        assert_eq!(service.labels(&board).unwrap().len(), 3);
        service
            .set_labels(&card, &[bug.id.clone(), docs.id.clone()])
            .unwrap();
        let mut labels = service.extras(&board).unwrap()[&card].labels.clone();
        labels.sort();
        let mut expected = vec![bug.id, docs.id];
        expected.sort();
        assert_eq!(labels, expected);
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("labels: Bug, Docs")
        );
    }

    #[test]
    fn subtasks_keep_order_toggle_and_count() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        let one = service.add_subtask(&card, " One ").unwrap();
        service.add_subtask(&card, "Two").unwrap();
        service.add_subtask(&card, "Three").unwrap();
        assert!(service.add_subtask(&card, "   ").is_err());
        service.toggle_subtask(&one.id, true).unwrap();
        let titles: Vec<String> = service
            .subtasks(&card)
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(titles, ["One", "Two", "Three"]);
        let extras = &service.extras(&board).unwrap()[&card];
        assert_eq!((extras.subtasks_done, extras.subtasks_total), (1, 3));
        service.delete_subtask(&one.id).unwrap();
        let extras = &service.extras(&board).unwrap()[&card];
        assert_eq!((extras.subtasks_done, extras.subtasks_total), (0, 2));
    }

    fn set_updated_at(path: &std::path::Path, card: &str, at: i64) {
        rusqlite::Connection::open(path)
            .unwrap()
            .execute(
                "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![at, card],
            )
            .unwrap();
    }

    fn updated_at(service: &mut Service, board: &str, card: &str) -> i64 {
        let detail = service.get(board).unwrap();
        detail
            .cards
            .iter()
            .find(|c| c.id == card)
            .unwrap()
            .updated_at
    }

    #[test]
    fn toggling_or_deleting_a_subtask_counts_as_a_change_to_its_card() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        let one = service.add_subtask(&card, "One").unwrap();
        set_updated_at(&path, &card, 1);
        service.toggle_subtask(&one.id, true).unwrap();
        assert!(updated_at(&mut service, &board, &card) > 1);
        set_updated_at(&path, &card, 1);
        service.delete_subtask(&one.id).unwrap();
        assert!(updated_at(&mut service, &board, &card) > 1);
        // Toggling to the state it already has is not a change.
        let two = service.add_subtask(&card, "Two").unwrap();
        service.toggle_subtask(&two.id, true).unwrap();
        set_updated_at(&path, &card, 1);
        service.toggle_subtask(&two.id, true).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
        // A subtask already gone is still not found for a toggle, and a
        // repeated delete stays a quiet no-op that touches nothing.
        set_updated_at(&path, &card, 1);
        assert_eq!(
            service.toggle_subtask(&one.id, false).unwrap_err(),
            "Subtask not found"
        );
        service.delete_subtask(&one.id).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
    }

    #[test]
    fn setting_a_field_to_the_value_it_already_has_writes_nothing() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        // A fresh card already has no priority, due date, projects or labels.
        let before = service.activity(&board, Some(&card), 100).unwrap().len();
        set_updated_at(&path, &card, 1);
        service.set_priority(&card, 0).unwrap();
        service.set_due(&card, None).unwrap();
        service.set_projects(&card, &[]).unwrap();
        service.set_labels(&card, &[]).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
        assert_eq!(
            service.activity(&board, Some(&card), 100).unwrap().len(),
            before
        );
        service.set_priority(&card, 2).unwrap();
        service.set_due(&card, Some(1_793_404_800_000)).unwrap();
        service
            .set_projects(&card, &["/b".into(), "/a".into()])
            .unwrap();
        service
            .set_labels(&card, std::slice::from_ref(&bug.id))
            .unwrap();
        let after = service.activity(&board, Some(&card), 100).unwrap().len();
        assert_eq!(after, before + 4);
        set_updated_at(&path, &card, 1);
        service.set_priority(&card, 2).unwrap();
        service.set_due(&card, Some(1_793_404_800_000)).unwrap();
        service
            .set_projects(&card, &["/a".into(), " /b ".into(), "/a".into()])
            .unwrap();
        service.set_labels(&card, &[bug.id]).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
        assert_eq!(
            service.activity(&board, Some(&card), 100).unwrap().len(),
            after
        );
        // A real change after the no-ops still writes.
        service.set_priority(&card, 3).unwrap();
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("priority: High")
        );
    }

    #[test]
    fn a_database_error_while_attaching_a_label_is_not_reported_as_a_missing_label() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        assert_eq!(
            service
                .set_labels(&card, &["not-a-label".into()])
                .unwrap_err(),
            "Label not on this board"
        );
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE board_labels RENAME COLUMN name TO title;")
            .unwrap();
        let error = service.set_labels(&card, &[bug.id]).unwrap_err();
        assert_ne!(error, "Label not on this board");
        assert!(error.contains("no such column"), "{error}");
    }

    #[test]
    fn comments_come_back_oldest_first_and_count() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        service.add_comment(&card, "first").unwrap();
        service.add_comment(&card, "second").unwrap();
        assert!(service.add_comment(&card, "  ").is_err());
        let bodies: Vec<String> = service
            .comments(&card)
            .unwrap()
            .into_iter()
            .map(|c| c.body)
            .collect();
        assert_eq!(bodies, ["first", "second"]);
        assert_eq!(service.extras(&board).unwrap()[&card].comments, 2);
    }

    fn extra_rows(path: &std::path::Path, card: &str) -> Vec<(&'static str, i64)> {
        let conn = rusqlite::Connection::open(path).unwrap();
        [
            "board_card_meta",
            "board_card_projects",
            "board_card_labels",
            "board_subtasks",
            "board_comments",
        ]
        .into_iter()
        .map(|table| {
            let count = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE card_id = ?1"),
                    [card],
                    |row| row.get(0),
                )
                .unwrap();
            (table, count)
        })
        .collect()
    }

    fn with_every_extra(service: &mut Service, board: &str, card: &str) {
        service.set_priority(card, 2).unwrap();
        service.set_projects(card, &["/a".into()]).unwrap();
        let label = service.create_label(board, "Bug", "e5534b").unwrap();
        service.set_labels(card, &[label.id]).unwrap();
        service.add_subtask(card, "One").unwrap();
        service.add_comment(card, "Hi").unwrap();
    }

    #[test]
    fn deleting_a_card_through_the_service_removes_its_extra_rows() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        with_every_extra(&mut service, &board, &card);
        assert!(extra_rows(&path, &card).iter().all(|(_, n)| *n == 1));
        service.card_delete(&card).unwrap();
        for (table, left) in extra_rows(&path, &card) {
            assert_eq!(left, 0, "{table}");
        }
    }

    #[test]
    fn deleting_a_card_with_a_raw_delete_removes_its_extra_rows() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        service.set_priority(&card, 2).unwrap();
        service.set_projects(&card, &["/a".into()]).unwrap();
        let label = service.create_label(&board, "Bug", "e5534b").unwrap();
        service.set_labels(&card, &[label.id]).unwrap();
        service.add_subtask(&card, "One").unwrap();
        service.add_comment(&card, "Hi").unwrap();
        // The Tauri app deletes cards with a plain DELETE on its own connection.
        let tauri = rusqlite::Connection::open(&path).unwrap();
        tauri.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        tauri
            .execute("DELETE FROM board_cards WHERE id = ?1", [&card])
            .unwrap();
        for table in [
            "board_card_meta",
            "board_card_projects",
            "board_card_labels",
            "board_subtasks",
            "board_comments",
        ] {
            let left: i64 = tauri
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE card_id = ?1"),
                    [&card],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(left, 0, "{table}");
        }
    }
}
