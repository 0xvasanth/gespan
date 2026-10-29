//! The board, over the same `board*` tables the Tauri app keeps in `gespan.db`.
//! The storage itself (tables, reads, writes, validation) is the shared
//! `gespan-board-store` crate both apps call; this module adds the native
//! app's `Service` handle.
use gespan_board_store::validate_id;
use gespan_board_store::{
    cards_for_link, create_board, create_card, create_column, delete_board, delete_card,
    delete_column, get_board, get_card, link_card, list_activity, list_boards, move_card,
    move_column, record_card, rename_board, update_card, update_column,
};
use rusqlite::Connection;
use std::path::Path;

use crate::tickets::TicketRef;
pub use gespan_board_store::{Activity, Board, BoardDetail, Card, Column};

pub mod auto_move;
pub mod extras;
pub mod linked_turns;

/// The shared board tables, migrated to the current version, plus the
/// pre-v3 native tables kept for one release.
pub(crate) fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    gespan_board_store::ensure_tables(conn)?;
    extras::ensure_legacy_tables(conn)
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

    /// The ticket the card links to; a stored value that no longer parses
    /// reads as no ticket.
    pub fn card_ticket(&mut self, card_id: &str) -> Result<Option<TicketRef>, String> {
        validate_id(card_id, "card")?;
        Ok(gespan_board_store::ticket(&self.conn, card_id)?
            .and_then(|json| serde_json::from_str(&json).ok()))
    }

    /// Links the card to a ticket (replacing any other) or, with `None`,
    /// removes its ticket.
    pub fn set_card_ticket(
        &mut self,
        card_id: &str,
        ticket: Option<&TicketRef>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        let json = ticket
            .map(|ticket| serde_json::to_string(ticket).map_err(|e| e.to_string()))
            .transpose()?;
        gespan_board_store::set_ticket(&self.conn, card_id, json.as_deref()).map(drop)
    }

    /// "New card from ticket": a card in the column titled with the ticket's
    /// key, linked to it and marked as titled from a ticket, in one write.
    pub fn card_create_from_ticket(
        &mut self,
        column_id: &str,
        ticket: &TicketRef,
    ) -> Result<Card, String> {
        validate_id(column_id, "column")?;
        let json = serde_json::to_string(ticket).map_err(|e| e.to_string())?;
        gespan_board_store::create_card_from_ticket(&self.conn, column_id, &json)
    }

    /// Replaces the card's title with the ticket's fetched title while the
    /// card is still titled with the ticket's key and still links it, and
    /// marks it as titled from a ticket. Returns whether the title changed.
    pub fn card_retitle_from_ticket(
        &mut self,
        card_id: &str,
        key: &str,
        title: &str,
    ) -> Result<bool, String> {
        validate_id(card_id, "card")?;
        gespan_board_store::retitle_card_from_ticket(&self.conn, card_id, key, title)
            .map(|(_, changed)| changed)
    }

    /// Every card on the board that links a ticket, by card id.
    pub fn board_tickets(
        &mut self,
        board_id: &str,
    ) -> Result<std::collections::HashMap<String, TicketRef>, String> {
        validate_id(board_id, "board")?;
        Ok(gespan_board_store::board_tickets(&self.conn, board_id)?
            .into_iter()
            .filter_map(|(card, json)| Some((card, serde_json::from_str(&json).ok()?)))
            .collect())
    }

    pub fn card(&mut self, card_id: &str) -> Result<Card, String> {
        validate_id(card_id, "card")?;
        get_card(&self.conn, card_id)
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

    /// Adds a deterministic set of cards (with labels, fields, links, activity
    /// and subtasks) to a board, for scale tests. See `gespan_board_store::seed`.
    #[doc(hidden)]
    pub fn seed(
        &mut self,
        board_id: &str,
        spec: &gespan_board_store::seed::SeedSpec,
    ) -> Result<gespan_board_store::seed::SeedReport, String> {
        gespan_board_store::seed::seed_board(&self.conn, board_id, spec)
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
    fn card_ticket_round_trips_through_the_service() {
        use crate::tickets::Tracker;
        let dir = tempfile::tempdir().unwrap();
        let mut service = Service::open(&dir.path().join("gespan.db")).unwrap();
        let board = service.create("Work").unwrap();
        let column = service.get(&board.id).unwrap().columns[0].id.clone();
        let card = service.card_create(&column, "Card", "").unwrap();
        assert_eq!(service.card_ticket(&card.id).unwrap(), None);
        let ticket = TicketRef {
            tracker: Tracker::Linear,
            key: "ENG-7".into(),
            url: Some("https://linear.app/x/issue/ENG-7".into()),
            title: Some("Fix".into()),
            status: None,
            status_group: None,
            fetched_at: Some(5),
        };
        service.set_card_ticket(&card.id, Some(&ticket)).unwrap();
        assert_eq!(service.card_ticket(&card.id).unwrap(), Some(ticket));
        service.set_card_ticket(&card.id, None).unwrap();
        assert_eq!(service.card_ticket(&card.id).unwrap(), None);
        assert!(service.card_ticket("bad id").is_err());
    }

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
                        // SQLite's write lock is not fair: a writer that commits and
                        // begins again at once keeps it for its whole run, so the other
                        // waits about 150 commits, past busy_timeout where commits
                        // flush slowly (Windows runners). A pause, as people take
                        // between moves, gives the waiter a turn.
                        std::thread::sleep(std::time::Duration::from_millis(1));
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
    fn service_update_settings_and_run_prompt() {
        use crate::board::extras::{CardTool, SettingsPatch};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let mut native = Service::open(&path).unwrap();
        let mut other = Service::open(&path).unwrap();
        let board = native.create("Work").unwrap();
        let defaults = native.settings(&board.id).unwrap();
        assert!(defaults.run_with && defaults.write_back && defaults.ticket_status_changes);

        // A patch changes only what it names, even across connections.
        native
            .update_settings(
                &board.id,
                &SettingsPatch {
                    write_back: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        let after = other
            .update_settings(
                &board.id,
                &SettingsPatch {
                    icon: Some(Some("bug".into())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!after.write_back);
        assert_eq!(after.icon.as_deref(), Some("bug"));
        assert!(after.run_with);
        assert_eq!(native.settings(&board.id).unwrap(), after);
        assert_eq!(native.settings("x'; --").unwrap_err(), "Invalid board id");

        // The prompt: title, description, subtasks, user comments only.
        let columns = native.get(&board.id).unwrap().columns;
        let card = native
            .card_create(&columns[0].id, "Fix login", "It fails on Safari.")
            .unwrap();
        native.add_subtask(&card.id, "Reproduce").unwrap();
        native.add_comment(&card.id, "Check the cookie").unwrap();
        native.add_agent_comment(&card.id, "Agent noise").unwrap();
        let number = native.card(&card.id).unwrap().number;
        assert_eq!(
            native.run_prompt(&card.id).unwrap(),
            format!(
                "Work on board card #{number}: Fix login\n\nIt fails on Safari.\n\n\
                 Subtasks:\n- [ ] Reproduce\n\nComments:\n- Check the cookie"
            )
        );

        // Write-back follows the board switch, remote session keys included.
        native
            .add_link(&card.id, "session", "remote:env-1:s-9", None)
            .unwrap();
        assert!(native
            .write_back("remote:env-1:s-9", "Done.")
            .unwrap()
            .is_empty());
        native
            .update_settings(
                &board.id,
                &SettingsPatch {
                    write_back: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            native.write_back("remote:env-1:s-9", "Done.").unwrap(),
            vec![card.id.clone()]
        );
        let last = native.comments(&card.id).unwrap().pop().unwrap();
        assert_eq!(
            (last.author.as_str(), last.body.as_str()),
            ("agent:remote:env-1:s-9", "Done.")
        );

        // The card tools see only the session's cards.
        let listed = native
            .answer_card("remote:env-1:s-9", &CardTool::Subtasks, false)
            .unwrap();
        assert_eq!(listed["cards"][0]["number"], number);

        // Running links map to cards; blockers come back by card id.
        let running = native
            .running_cards(
                &board.id,
                &[("session".to_string(), "remote:env-1:s-9".to_string())],
            )
            .unwrap();
        assert!(running.contains(&card.id));
        let blocker = native.card_create(&columns[0].id, "Blocker", "").unwrap();
        native
            .add_relation(&blocker.id, &card.id, "blocks")
            .unwrap();
        let blocked = native.blocked(&board.id).unwrap();
        assert_eq!(blocked.get(&card.id), Some(&vec![blocker.number]));
        assert_eq!(blocked.len(), 1);
        assert!(native
            .board_links(&board.id)
            .unwrap()
            .iter()
            .any(|link| link.card_id == card.id && link.kind == "session"));

        // Ticket status permission: board switch, then the session override.
        assert!(native.may_change_ticket_status("remote:env-1:s-9"));
        native
            .update_settings(
                &board.id,
                &SettingsPatch {
                    ticket_status_changes: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!native.may_change_ticket_status("remote:env-1:s-9"));
        native
            .set_session_status_setting("remote:env-1:s-9", Some(true))
            .unwrap();
        assert_eq!(
            native.session_status_setting("remote:env-1:s-9"),
            Ok(Some(true))
        );
        assert!(native.may_change_ticket_status("remote:env-1:s-9"));
        native
            .set_session_status_setting("remote:env-1:s-9", None)
            .unwrap();
        assert_eq!(native.session_status_setting("remote:env-1:s-9"), Ok(None));
        assert!(native.set_session_status_setting(" ", Some(true)).is_err());
    }

    #[test]
    fn an_unreadable_session_status_setting_is_an_error_through_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let mut service = Service::open(&path).unwrap();
        service
            .set_session_status_setting("s1", Some(true))
            .unwrap();
        assert_eq!(service.session_status_setting("s1"), Ok(Some(true)));
        assert_eq!(service.session_status_setting("s2"), Ok(None));

        // Another connection breaks the table under the open service.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE board_session_settings RENAME TO moved")
            .unwrap();
        assert!(service.session_status_setting("s1").is_err());
        assert!(service.session_status_setting("s2").is_err());
        // The permission check still fails closed.
        assert!(!service.may_change_ticket_status("s1"));
        assert!(!service.may_change_ticket_status("s2"));
    }

    #[test]
    fn templates_labels_and_fields_go_through_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let mut service = Service::open(&dir.path().join("gespan.db")).unwrap();
        let ids: Vec<String> = service
            .templates()
            .unwrap()
            .into_iter()
            .map(|template| template.id)
            .collect();
        assert_eq!(
            ids,
            ["blank", "agent-pipeline", "bug-triage", "release-checklist"]
        );
        let board = service.create_from_template("Bugs", "bug-triage").unwrap();
        let labels = service.labels(&board.id).unwrap();
        assert_eq!(labels.len(), 3);
        let fields = service.list_fields(&board.id).unwrap();
        assert_eq!(fields[0].name, "Severity");

        let saved = service.save_as_template(&board.id, "Mine").unwrap();
        assert!(!saved.built_in);
        assert_eq!(service.templates().unwrap().last().unwrap().id, saved.id);
        service.delete_template(&saved.id).unwrap();
        assert_eq!(service.templates().unwrap().len(), 4);
        assert_eq!(
            service.delete_template("blank").unwrap_err(),
            "Built-in templates can't be deleted"
        );

        // Labels: rename, reorder, merge, delete.
        service
            .label_update(&labels[0].id, Some("Defect"), None)
            .unwrap();
        service.label_move(&labels[0].id, 2).unwrap();
        service.labels_merge(&labels[1].id, &labels[2].id).unwrap();
        let names: Vec<String> = service
            .labels(&board.id)
            .unwrap()
            .into_iter()
            .map(|label| label.name)
            .collect();
        assert_eq!(names, ["Needs repro", "Defect"]);
        service.label_delete(&labels[0].id).unwrap();
        assert_eq!(service.labels(&board.id).unwrap().len(), 1);

        // Fields: create, rename an option (values follow), move, delete.
        let column = service.get(&board.id).unwrap().columns[0].id.clone();
        let card = service.card_create(&column, "Crash", "").unwrap();
        let size = service
            .field_create(&board.id, "Size", "select", Some(r#"["S","L"]"#))
            .unwrap();
        service
            .set_field_value(&card.id, &size.id, Some(r#""S""#))
            .unwrap();
        service.field_rename_option(&size.id, "S", "Small").unwrap();
        assert_eq!(
            service.field_values(&card.id).unwrap()[0].value_json,
            r#""Small""#
        );
        service
            .field_update(&size.id, Some("T-shirt"), None)
            .unwrap();
        service.field_move(&size.id, 0).unwrap();
        assert_eq!(service.list_fields(&board.id).unwrap()[0].name, "T-shirt");
        let note = service
            .field_create(&board.id, "Note", "text", None)
            .unwrap();
        service
            .set_field_value(&card.id, &note.id, Some(r#""flaky""#))
            .unwrap();
        assert_eq!(
            service
                .field_distinct_values(&board.id, &note.id, 10)
                .unwrap(),
            [r#""flaky""#]
        );
        service.field_delete(&size.id).unwrap();
        assert!(service
            .list_fields(&board.id)
            .unwrap()
            .iter()
            .all(|field| field.id != size.id));
        assert_eq!(
            service.field_move("bad id", 0).unwrap_err(),
            "Invalid field id"
        );
        assert_eq!(
            service.label_delete("bad id").unwrap_err(),
            "Invalid label id"
        );
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
