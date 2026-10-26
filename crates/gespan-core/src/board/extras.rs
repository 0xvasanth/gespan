//! Task fields beyond title and body: priority, due date, projects, labels,
//! subtasks and comments. The storage is the shared board-store's v3 tables;
//! these `Service` methods are the native app's handle on them.
use super::{validate_id, Board, Card, Service};
use gespan_board_store as store;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

pub use gespan_board_store::{
    auto_move_rules, display_options, priority_scheme, AutoMoveRules, BoardSettings, CardExtras,
    CardFace, CardIndexRow, CardLink, CardMeta, CardTool, Comment, Density, Dir, DisplayOptions,
    Field, FieldValue, FilterRow, Label, Op, PriorityScheme, Relation, RuleTarget, SettingsPatch,
    SortKey, SortSpec, Subtask, TemplateInfo, View, ViewSpec, BOARD_ICONS, BUILT_IN_TEMPLATES,
    FIELD_KINDS, PRIORITIES,
};
use std::collections::BTreeMap;

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

    pub fn archive(&mut self, card_id: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::archive_card(&self.conn, card_id).map(drop)
    }

    pub fn archive_many(&mut self, card_ids: &[String]) -> Result<(), String> {
        for id in card_ids {
            validate_id(id, "card")?;
        }
        store::archive_cards(&self.conn, card_ids).map(drop)
    }

    pub fn unarchive(&mut self, card_id: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::unarchive_card(&self.conn, card_id).map(drop)
    }

    /// Unarchives the card and places it at `index` in `column_id`.
    pub fn restore(&mut self, card_id: &str, column_id: &str, index: usize) -> Result<(), String> {
        validate_id(card_id, "card")?;
        validate_id(column_id, "column")?;
        store::restore_card(&self.conn, card_id, column_id, index).map(drop)
    }

    /// Deletes the card only while it is still archived, in one write. Returns
    /// the board id when it deleted, `None` when the card is live or gone.
    pub fn delete_archived(&mut self, card_id: &str) -> Result<Option<String>, String> {
        validate_id(card_id, "card")?;
        store::delete_archived_card(&self.conn, card_id)
    }

    pub fn archived(&mut self, board_id: &str) -> Result<Vec<super::Card>, String> {
        validate_id(board_id, "board")?;
        store::archived_cards(&self.conn, board_id)
    }

    pub fn move_many(
        &mut self,
        card_ids: &[String],
        column_id: &str,
        index: usize,
        actor: &str,
    ) -> Result<(), String> {
        for id in card_ids {
            validate_id(id, "card")?;
        }
        validate_id(column_id, "column")?;
        store::move_cards(&self.conn, card_ids, column_id, index, actor).map(drop)
    }

    /// Overwrites colour, WIP limit and collapsed together; pass the current
    /// colour and limit back to change only `collapsed`.
    pub fn set_column_display(
        &mut self,
        column_id: &str,
        color: Option<&str>,
        wip_limit: Option<i64>,
        collapsed: bool,
    ) -> Result<(), String> {
        validate_id(column_id, "column")?;
        store::set_column_display(&self.conn, column_id, color, wip_limit, collapsed).map(drop)
    }

    /// `(live, archived)` card counts of a column.
    pub fn column_card_count(&mut self, column_id: &str) -> Result<(usize, usize), String> {
        validate_id(column_id, "column")?;
        store::column_card_count(&self.conn, column_id)
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

    /// Card ids matching `spec`, in display order. `running` holds the cards
    /// whose linked session or Pilot runs now. The clock and the local day
    /// are read here.
    pub fn query(
        &mut self,
        board_id: &str,
        spec: &ViewSpec,
        running: &HashSet<String>,
    ) -> Result<Vec<String>, String> {
        validate_id(board_id, "board")?;
        store::query_cards(
            &self.conn,
            board_id,
            spec,
            store::now_millis(),
            &store::local_day_chrono,
            running,
        )
    }

    pub fn card_index(&mut self, board_id: &str) -> Result<Vec<CardIndexRow>, String> {
        validate_id(board_id, "board")?;
        store::card_index(&self.conn, board_id)
    }

    pub fn views(&mut self, board_id: &str) -> Result<Vec<View>, String> {
        validate_id(board_id, "board")?;
        store::list_views(&self.conn, board_id)
    }

    pub fn view_create(
        &mut self,
        board_id: &str,
        name: &str,
        spec_json: &str,
        layout_json: &str,
    ) -> Result<View, String> {
        validate_id(board_id, "board")?;
        store::create_view(&self.conn, board_id, name, spec_json, layout_json)
    }

    pub fn view_update(
        &mut self,
        view_id: &str,
        name: Option<&str>,
        spec_json: Option<&str>,
        layout_json: Option<&str>,
    ) -> Result<(), String> {
        validate_id(view_id, "view")?;
        store::update_view(&self.conn, view_id, name, spec_json, layout_json).map(drop)
    }

    pub fn view_delete(&mut self, view_id: &str) -> Result<(), String> {
        validate_id(view_id, "view")?;
        store::delete_view(&self.conn, view_id).map(drop)
    }

    pub fn view_move(&mut self, view_id: &str, index: usize) -> Result<(), String> {
        validate_id(view_id, "view")?;
        store::move_view(&self.conn, view_id, index).map(drop)
    }

    pub fn set_priority_many(&mut self, card_ids: &[String], priority: u8) -> Result<(), String> {
        for id in card_ids {
            validate_id(id, "card")?;
        }
        store::set_priority_many(&self.conn, card_ids, priority).map(drop)
    }

    pub fn set_label_many(
        &mut self,
        card_ids: &[String],
        label_id: &str,
        on: bool,
    ) -> Result<(), String> {
        for id in card_ids {
            validate_id(id, "card")?;
        }
        validate_id(label_id, "label")?;
        store::set_label_many(&self.conn, card_ids, label_id, on).map(drop)
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

    /// A comment written by an agent (write-back); the author is "agent".
    pub fn add_agent_comment(&mut self, card_id: &str, body: &str) -> Result<Comment, String> {
        validate_id(card_id, "card")?;
        store::add_comment(&self.conn, card_id, body, "agent")
    }

    pub fn edit_comment(&mut self, comment_id: &str, body: &str) -> Result<(), String> {
        validate_id(comment_id, "comment")?;
        store::edit_comment(&self.conn, comment_id, body).map(drop)
    }

    pub fn card_meta(&mut self, card_id: &str) -> Result<CardMeta, String> {
        validate_id(card_id, "card")?;
        store::card_meta(&self.conn, card_id)
    }

    pub fn set_start(&mut self, card_id: &str, start_at: Option<i64>) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::set_start(&self.conn, card_id, start_at).map(drop)
    }

    pub fn set_estimate(&mut self, card_id: &str, minutes: Option<i64>) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::set_estimate(&self.conn, card_id, minutes).map(drop)
    }

    pub fn add_relation(&mut self, card_id: &str, related: &str, kind: &str) -> Result<(), String> {
        validate_id(card_id, "card")?;
        validate_id(related, "card")?;
        store::add_relation(&self.conn, card_id, related, kind).map(drop)
    }

    pub fn remove_relation(
        &mut self,
        card_id: &str,
        related: &str,
        kind: &str,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        validate_id(related, "card")?;
        store::remove_relation(&self.conn, card_id, related, kind).map(drop)
    }

    pub fn card_relations(&mut self, card_id: &str) -> Result<Vec<Relation>, String> {
        validate_id(card_id, "card")?;
        store::card_relations(&self.conn, card_id)
    }

    pub fn list_fields(&mut self, board_id: &str) -> Result<Vec<Field>, String> {
        validate_id(board_id, "board")?;
        store::list_fields(&self.conn, board_id)
    }

    pub fn set_field_value(
        &mut self,
        card_id: &str,
        field_id: &str,
        value_json: Option<&str>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        validate_id(field_id, "field")?;
        store::set_field_value(&self.conn, card_id, field_id, value_json).map(drop)
    }

    pub fn field_values(&mut self, card_id: &str) -> Result<Vec<FieldValue>, String> {
        validate_id(card_id, "card")?;
        store::field_values(&self.conn, card_id)
    }

    pub fn add_link(
        &mut self,
        card_id: &str,
        kind: &str,
        reference: &str,
        role: Option<&str>,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::add_link(&self.conn, card_id, kind, reference, role).map(drop)
    }

    pub fn remove_link(
        &mut self,
        card_id: &str,
        kind: &str,
        reference: &str,
    ) -> Result<(), String> {
        validate_id(card_id, "card")?;
        store::remove_link(&self.conn, card_id, kind, reference).map(drop)
    }

    pub fn card_links(&mut self, card_id: &str) -> Result<Vec<CardLink>, String> {
        validate_id(card_id, "card")?;
        store::card_links(&self.conn, card_id)
    }

    pub fn move_subtask(&mut self, subtask_id: &str, index: usize) -> Result<(), String> {
        validate_id(subtask_id, "subtask")?;
        store::move_subtask(&self.conn, subtask_id, index).map(drop)
    }

    pub fn convert_subtask(&mut self, subtask_id: &str) -> Result<Card, String> {
        validate_id(subtask_id, "subtask")?;
        store::convert_subtask(&self.conn, subtask_id)
    }

    pub fn duplicate_card(&mut self, card_id: &str) -> Result<Card, String> {
        validate_id(card_id, "card")?;
        store::duplicate_card(&self.conn, card_id)
    }

    pub fn move_card_to_board(&mut self, card_id: &str, board_id: &str) -> Result<Card, String> {
        validate_id(card_id, "card")?;
        validate_id(board_id, "board")?;
        store::move_card_to_board(&self.conn, card_id, board_id)
    }
}

/// Settings, templates, label and field management, and the agent side of
/// the board (auto-move events, write-back, blockers, link and run state).
impl Service {
    /// The board's settings; a board without a settings row reads as the
    /// defaults.
    pub fn settings(&mut self, board_id: &str) -> Result<BoardSettings, String> {
        validate_id(board_id, "board")?;
        store::board_settings(&self.conn, board_id)
    }

    /// Changes only the fields `patch` names, so a toggle the other app set
    /// a moment ago survives. Returns the settings after the change.
    pub fn update_settings(
        &mut self,
        board_id: &str,
        patch: &SettingsPatch,
    ) -> Result<BoardSettings, String> {
        validate_id(board_id, "board")?;
        store::update_board_settings(&self.conn, board_id, patch)
    }

    /// Built-in templates first, then saved ones by name.
    pub fn templates(&mut self) -> Result<Vec<TemplateInfo>, String> {
        store::list_templates(&self.conn)
    }

    pub fn create_from_template(&mut self, name: &str, template_id: &str) -> Result<Board, String> {
        validate_id(template_id, "template")?;
        store::create_board_from_template(&self.conn, name, template_id)
    }

    pub fn save_as_template(&mut self, board_id: &str, name: &str) -> Result<TemplateInfo, String> {
        validate_id(board_id, "board")?;
        store::save_board_as_template(&self.conn, board_id, name)
    }

    pub fn delete_template(&mut self, template_id: &str) -> Result<(), String> {
        validate_id(template_id, "template")?;
        store::delete_template(&self.conn, template_id)
    }

    /// Renames and/or recolours a label; `None` leaves that part alone.
    pub fn label_update(
        &mut self,
        label_id: &str,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Result<(), String> {
        validate_id(label_id, "label")?;
        store::update_label(&self.conn, label_id, name, color).map(drop)
    }

    pub fn label_move(&mut self, label_id: &str, index: usize) -> Result<(), String> {
        validate_id(label_id, "label")?;
        store::move_label(&self.conn, label_id, index).map(drop)
    }

    pub fn label_delete(&mut self, label_id: &str) -> Result<(), String> {
        validate_id(label_id, "label")?;
        store::delete_label(&self.conn, label_id).map(drop)
    }

    /// Moves every card from `from_id` to `into_id`, then deletes `from_id`.
    pub fn labels_merge(&mut self, from_id: &str, into_id: &str) -> Result<(), String> {
        validate_id(from_id, "label")?;
        validate_id(into_id, "label")?;
        store::merge_labels(&self.conn, from_id, into_id).map(drop)
    }

    pub fn field_create(
        &mut self,
        board_id: &str,
        name: &str,
        kind: &str,
        options_json: Option<&str>,
    ) -> Result<Field, String> {
        validate_id(board_id, "board")?;
        store::create_field(&self.conn, board_id, name, kind, options_json)
    }

    /// Renames a field and/or replaces its options; `None` leaves that part
    /// alone. A field's kind never changes.
    pub fn field_update(
        &mut self,
        field_id: &str,
        name: Option<&str>,
        options_json: Option<&str>,
    ) -> Result<(), String> {
        validate_id(field_id, "field")?;
        store::update_field(&self.conn, field_id, name, options_json).map(drop)
    }

    pub fn field_rename_option(
        &mut self,
        field_id: &str,
        from: &str,
        to: &str,
    ) -> Result<(), String> {
        validate_id(field_id, "field")?;
        store::rename_field_option(&self.conn, field_id, from, to).map(drop)
    }

    pub fn field_move(&mut self, field_id: &str, index: usize) -> Result<(), String> {
        validate_id(field_id, "field")?;
        store::move_field(&self.conn, field_id, index).map(drop)
    }

    pub fn field_delete(&mut self, field_id: &str) -> Result<(), String> {
        validate_id(field_id, "field")?;
        store::delete_field(&self.conn, field_id).map(drop)
    }

    /// The values a text, number or url field holds on the board, for filter
    /// pickers.
    pub fn field_distinct_values(
        &mut self,
        board_id: &str,
        field_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, String> {
        validate_id(board_id, "board")?;
        validate_id(field_id, "field")?;
        store::field_distinct_values(&self.conn, board_id, field_id, limit)
    }

    /// Applies the auto-move rules for `event` to every card linked to
    /// `kind`/`id`. Returns the ids of the boards written.
    pub fn linked_event(
        &mut self,
        kind: &str,
        id: &str,
        event: store::LinkedEvent,
    ) -> Result<Vec<String>, String> {
        store::on_linked_work(&self.conn, kind, id, event)
    }

    /// Comments a finished turn's reply on every card linked to the session,
    /// on boards with write-back on. Returns the card ids written.
    pub fn write_back(&mut self, session_key: &str, summary: &str) -> Result<Vec<String>, String> {
        store::write_back(&self.conn, session_key, summary)
    }

    /// Blocked cards on the board, by card id, to the numbers of their open
    /// blockers.
    pub fn blocked(&mut self, board_id: &str) -> Result<BTreeMap<String, Vec<i64>>, String> {
        validate_id(board_id, "board")?;
        store::blocked_cards(&self.conn, board_id)
    }

    /// Card id to the kind of its newest run event (`started`,
    /// `needs_input`, `completed` or `failed`), in one read-only windowed
    /// query, so newer comments or moves never hide a failure.
    pub fn latest_run_events(&mut self, board_id: &str) -> Result<HashMap<String, String>, String> {
        validate_id(board_id, "board")?;
        store::latest_run_events(&self.conn, board_id)
    }

    /// Every link on every card of the board.
    pub fn board_links(&mut self, board_id: &str) -> Result<Vec<CardLink>, String> {
        validate_id(board_id, "board")?;
        store::board_links(&self.conn, board_id)
    }

    /// The board's cards linked to one of `running` (`("session", key)` or
    /// `("pilot", id)`, see `LinkedTurns::open_links`).
    pub fn running_cards(
        &mut self,
        board_id: &str,
        running: &[(String, String)],
    ) -> Result<HashSet<String>, String> {
        validate_id(board_id, "board")?;
        store::running_card_ids(&self.conn, board_id, running)
    }

    /// The prompt "Run with..." starts a session with.
    pub fn run_prompt(&mut self, card_id: &str) -> Result<String, String> {
        validate_id(card_id, "card")?;
        store::run_prompt(&self.conn, card_id)
    }

    /// The session's own "may change ticket status" override, if set
    /// (`Ok(None)`: it follows its boards). A failed read is an error, so it
    /// never shows as following the boards.
    pub fn session_status_setting(&mut self, session_key: &str) -> Result<Option<bool>, String> {
        store::session_status_setting(&self.conn, session_key)
    }

    /// Sets the session's override; `None` follows the boards again.
    pub fn set_session_status_setting(
        &mut self,
        session_key: &str,
        may_change: Option<bool>,
    ) -> Result<(), String> {
        store::set_session_status_setting(&self.conn, session_key, may_change)
    }

    /// Whether an agent in this session may move its ticket's status: the
    /// session override, else the strictest board linking the session, else
    /// yes. An unreadable override fails closed.
    pub fn may_change_ticket_status(&mut self, session_key: &str) -> bool {
        store::may_change_ticket_status(&self.conn, session_key)
    }

    /// Runs a card tool for an agent in `session_key`; the cards it may touch
    /// are the ones linked to the session now.
    pub fn answer_card(
        &mut self,
        session_key: &str,
        tool: &CardTool,
        read_only: bool,
    ) -> Result<serde_json::Value, String> {
        store::answer_card(&self.conn, session_key, tool, read_only)
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
    fn latest_run_events_go_through_the_service() {
        let (_dir, mut service, _) = service();
        let (board, failed) = card(&mut service);
        let (_, quiet) = card(&mut service);
        service.card_record(&failed, "failed", None).unwrap();
        for n in 0..60 {
            service.add_comment(&failed, &format!("note {n}")).unwrap();
        }
        let events = service.latest_run_events(&board).unwrap();
        assert_eq!(events.get(&failed).map(String::as_str), Some("failed"));
        assert!(!events.contains_key(&quiet));
        service.card_record(&failed, "started", None).unwrap();
        assert_eq!(
            service.latest_run_events(&board).unwrap()[&failed],
            "started"
        );
        assert!(service.latest_run_events(" ").is_err());
    }

    #[test]
    fn query_views_and_bulk_edits_go_through_the_service() {
        use super::{Dir, FilterRow, Op, SortKey, SortSpec, ViewSpec};
        use std::collections::HashSet;
        let (_dir, mut service, _) = service();
        let board = service.default_board().unwrap();
        let backlog = service.get(&board.id).unwrap().columns[0].id.clone();
        let ids: Vec<String> = ["Fix login", "Write docs", "Ship"]
            .iter()
            .map(|t| service.card_create(&backlog, t, "").unwrap().id)
            .collect();
        service.set_priority_many(&ids[1..], 3).unwrap();
        let label = service.create_label(&board.id, "Bug", "e5534b").unwrap();
        service.set_label_many(&ids[..2], &label.id, true).unwrap();
        service.set_label_many(&ids[..1], &label.id, false).unwrap();

        let spec = ViewSpec {
            query: String::new(),
            rows: vec![FilterRow {
                field: "priority".into(),
                op: Op::Is,
                values: vec!["3".into()],
            }],
            sort: SortSpec {
                key: SortKey::Title,
                dir: Dir::Desc,
            },
        };
        let none = HashSet::new();
        assert_eq!(
            service.query(&board.id, &spec, &none).unwrap(),
            vec![ids[1].clone(), ids[2].clone()]
        );
        let running: HashSet<String> = [ids[0].clone()].into();
        let linked = ViewSpec {
            rows: vec![FilterRow {
                field: "linked".into(),
                op: Op::Is,
                values: vec!["running".into()],
            }],
            ..ViewSpec::default()
        };
        assert_eq!(
            service.query(&board.id, &linked, &running).unwrap(),
            vec![ids[0].clone()]
        );

        let index = service.card_index(&board.id).unwrap();
        let row = |id: &String| index.iter().find(|r| &r.card_id == id).unwrap();
        assert_eq!(row(&ids[1]).labels, vec![label.id.clone()]);
        assert!(row(&ids[0]).labels.is_empty());
        assert_eq!(row(&ids[2]).priority, 3);

        let view = service
            .view_create(&board.id, "High", r#"{"query":"x"}"#, "{}")
            .unwrap();
        service
            .view_update(&view.id, Some("Renamed"), None, None)
            .unwrap();
        assert_eq!(service.views(&board.id).unwrap()[0].name, "Renamed");
        assert!(service
            .view_update(&view.id, None, Some("not json"), None)
            .is_err());
        service.view_move(&view.id, 0).unwrap();
        service.view_delete(&view.id).unwrap();
        assert!(service.views(&board.id).unwrap().is_empty());
        assert!(service.set_priority_many(&ids, 9).is_err());
        assert!(service.query("bad id!", &spec, &none).is_err());
    }

    #[test]
    fn archive_restore_and_bulk_operations_go_through_the_service() {
        let (_dir, mut service, _) = service();
        let board = service.default_board().unwrap();
        let columns = service.get(&board.id).unwrap().columns;
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|t| service.card_create(&columns[0].id, t, "").unwrap().id)
            .collect();

        service.archive_many(&ids[..2]).unwrap();
        assert_eq!(service.column_card_count(&columns[0].id).unwrap(), (1, 2));
        assert_eq!(service.archived(&board.id).unwrap().len(), 2);
        assert!(service.column_card_count(&columns[0].id).is_ok());

        service.restore(&ids[1], &columns[1].id, 0).unwrap();
        service.unarchive(&ids[0]).unwrap();
        service.archive(&ids[2]).unwrap();
        assert_eq!(service.archived(&board.id).unwrap().len(), 1);

        service
            .move_many(&ids[..2], &columns[1].id, 0, "user")
            .unwrap();
        let detail = service.get(&board.id).unwrap();
        let in_doing: Vec<&str> = detail
            .cards
            .iter()
            .filter(|c| c.column_id == columns[1].id)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(in_doing, [ids[0].as_str(), ids[1].as_str()]);

        service
            .set_column_display(&columns[1].id, Some("ff0000"), Some(3), true)
            .unwrap();
        let col = &service.get(&board.id).unwrap().columns[1];
        assert_eq!(
            (col.color.as_deref(), col.wip_limit, col.collapsed),
            (Some("ff0000"), Some(3), true)
        );

        assert!(service.archive("bad id!").is_err());
        assert!(service.archive_many(&["bad id!".to_string()]).is_err());
        assert!(service.restore(&ids[2], "bad id!", 0).is_err());
    }

    #[test]
    fn card_detail_operations_go_through_the_service() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        let backlog = service.get(&board).unwrap().columns[0].id.clone();
        let other = service.card_create(&backlog, "Other", "").unwrap();

        service.set_start(&card, Some(1_000)).unwrap();
        service.set_estimate(&card, Some(90)).unwrap();
        let meta = service.card_meta(&card).unwrap();
        assert_eq!(
            (meta.start_at, meta.estimate_minutes),
            (Some(1_000), Some(90))
        );
        assert!(service.set_estimate(&card, Some(0)).is_err());

        service.add_relation(&card, &other.id, "blocks").unwrap();
        assert_eq!(service.card_relations(&card).unwrap().len(), 1);
        assert!(service.add_relation(&other.id, &card, "blocks").is_err());
        service.remove_relation(&card, &other.id, "blocks").unwrap();
        assert!(service.card_relations(&card).unwrap().is_empty());

        assert!(service.list_fields(&board).unwrap().is_empty());
        assert!(service.field_values(&card).unwrap().is_empty());
        assert!(service.set_field_value(&card, "nope", Some("1")).is_err());

        service
            .add_link(&card, "pr", "https://x/pr/1", None)
            .unwrap();
        assert_eq!(
            service.card_links(&card).unwrap()[0].reference,
            "https://x/pr/1"
        );
        service.remove_link(&card, "pr", "https://x/pr/1").unwrap();
        assert!(service.card_links(&card).unwrap().is_empty());

        let first = service.add_subtask(&card, "one").unwrap();
        let second = service.add_subtask(&card, "two").unwrap();
        service.move_subtask(&second.id, 0).unwrap();
        assert_eq!(service.subtasks(&card).unwrap()[0].id, second.id);
        let converted = service.convert_subtask(&first.id).unwrap();
        assert_eq!(converted.title, "one");
        assert_eq!(service.subtasks(&card).unwrap().len(), 1);

        let copy = service.duplicate_card(&card).unwrap();
        assert_eq!(copy.title, "Task (copy)");

        let target = service.create("Target").unwrap();
        let moved = service.move_card_to_board(&card, &target.id).unwrap();
        assert_eq!(moved.board_id, target.id);
        assert!(service.move_card_to_board("bad id!", &target.id).is_err());
    }

    #[test]
    fn comments_edit_and_agent_author() {
        let (_dir, mut service, _) = service();
        let (_, card) = card(&mut service);
        let user = service.add_comment(&card, "hi").unwrap();
        let agent = service.add_agent_comment(&card, "done").unwrap();
        assert_eq!(
            (user.author.as_str(), agent.author.as_str()),
            ("user", "agent")
        );
        service.edit_comment(&user.id, "hello").unwrap();
        let comments = service.comments(&card).unwrap();
        assert_eq!(comments[0].body, "hello");
        assert!(comments[0].edited_at.is_some());
        assert!(service.edit_comment("bad id!", "x").is_err());
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
