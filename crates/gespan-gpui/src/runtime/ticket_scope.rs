//! What a turn's ticket and card tools may reach, read from storage by the
//! app and never taken from the agent: the session's ticket link (session
//! store), its status permission and its write-back cards (board store).
//! Read when the turn starts, again whenever something may have changed, and
//! again for every call the app answers.
use super::{Database, locked};
use gespan_core::{board::Service, ticket_bridge::Scope, tickets::TicketRef, transcript::Row};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// Same text as the bridge's refusal, for a card call answered after the
/// session lost its write-back cards.
pub(super) const NO_CARDS: &str = "Card tools aren't available in this session";

/// The session's tool scope as storage has it now.
pub(super) struct ScopeSource {
    store: Database,
    session: i64,
    /// The board store's key for the session (`board::turns::session_key`).
    key: Option<String>,
    board: Option<Mutex<Service>>,
    read_only: bool,
    /// An orchestration lead or worker: never has a ticket or cards.
    restricted: bool,
}

impl ScopeSource {
    pub(super) fn new(store: Database, session: i64, read_only: bool, restricted: bool) -> Self {
        let key = crate::board::turns::session_key(&store, session);
        let board = locked(&store)
            .ok()
            .map(|store| store.database_path().to_path_buf())
            .and_then(|path| {
                Service::open(&path)
                    .inspect_err(|error| tracing::warn!(%error, "board unavailable to tools"))
                    .ok()
            })
            .map(Mutex::new);
        Self {
            store,
            session,
            key,
            board,
            read_only,
            restricted,
        }
    }

    /// The ticket the session is linked to right now: one session's row,
    /// not the whole session list, since this runs on every scope poll.
    pub(super) fn ticket(&self) -> Result<Option<TicketRef>, String> {
        if self.restricted {
            return Ok(None);
        }
        let item = locked(&self.store)?.work_item(self.session)?;
        Ok(item
            .as_ref()
            .and_then(gespan_core::session_groups::WorkItem::ticket)
            .cloned())
    }

    /// The board side of the scope: (may change status, has write-back
    /// cards). Anything unreadable answers no: a session the board can't
    /// vouch for gets neither.
    pub(super) fn board_scope(&self) -> (bool, bool) {
        if self.restricted {
            return (false, false);
        }
        let (Some(key), Some(board)) = (self.key.as_deref(), self.board.as_ref()) else {
            return (false, false);
        };
        let Ok(mut board) = board.lock() else {
            return (false, false);
        };
        let may_change = board.may_change_ticket_status(key);
        let cards = write_back_cards(&mut board, key).unwrap_or_else(|error| {
            tracing::warn!(%error, "could not read the session's board cards");
            false
        });
        (may_change, cards)
    }

    /// The whole scope; a ticket link that can't be read counts as none.
    pub(super) fn read(&self) -> Scope {
        let (may_change_status, cards) = self.board_scope();
        let ticket = self.ticket().unwrap_or_else(|error| {
            tracing::warn!(%error, "could not read the session's ticket link");
            None
        });
        Scope {
            ticket,
            may_change_status,
            read_only: self.read_only,
            cards,
        }
    }

    /// Answers a card tool on the cards linked to the session now.
    pub(super) fn answer_card(
        &self,
        tool: &gespan_core::board::extras::CardTool,
    ) -> Result<serde_json::Value, String> {
        if self.restricted {
            return Err(NO_CARDS.into());
        }
        let key = self.key.as_deref().ok_or(NO_CARDS)?;
        let board = self.board.as_ref().ok_or(NO_CARDS)?;
        board
            .lock()
            .map_err(|_| "Board unavailable")?
            .answer_card(key, tool, self.read_only)
    }

    /// SQLite's count of other connections' commits, for noticing a change
    /// made by another window, view or app.
    pub(super) fn data_version(&self) -> Option<i64> {
        self.board.as_ref()?.lock().ok()?.data_version().ok()
    }
}

/// Whether a live card on a board with write-back on links the session.
fn write_back_cards(board: &mut Service, key: &str) -> Result<bool, String> {
    let mut boards: Vec<String> = board
        .cards_for_link("session", key)?
        .into_iter()
        .map(|card| card.board_id)
        .collect();
    boards.dedup();
    for id in boards {
        if board.settings(&id)?.write_back {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Two links to the same ticket.
pub(super) fn same_ticket(a: &TicketRef, b: &TicketRef) -> bool {
    a.tracker == b.tracker && a.key.trim().eq_ignore_ascii_case(b.key.trim())
}

type Flags = Mutex<HashMap<i64, Vec<Arc<AtomicBool>>>>;

fn flags() -> &'static Flags {
    static FLAGS: OnceLock<Flags> = OnceLock::new();
    FLAGS.get_or_init(Default::default)
}

/// Tells running turns their scope may have changed: one session's (its
/// ticket link or status override), or every session's (`None`: a card link
/// or a board setting).
pub(crate) fn changed(session: Option<i64>) {
    let Ok(flags) = flags().lock() else {
        return;
    };
    let mark = |list: &Vec<Arc<AtomicBool>>| {
        for flag in list {
            flag.store(true, Ordering::Relaxed);
        }
    };
    match session {
        Some(session) => flags.get(&session).into_iter().for_each(mark),
        None => flags.values().for_each(mark),
    }
}

/// One turn's change flag, registered while the turn's tools exist.
pub(super) struct Watch {
    session: i64,
    flag: Arc<AtomicBool>,
}

impl Watch {
    pub(super) fn new(session: i64) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut flags) = flags().lock() {
            flags.entry(session).or_default().push(flag.clone());
        }
        Self { session, flag }
    }

    /// Whether a change was reported since the last call.
    pub(super) fn take(&self) -> bool {
        self.flag.swap(false, Ordering::Relaxed)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Ok(mut flags) = flags().lock()
            && let Some(list) = flags.get_mut(&self.session)
        {
            list.retain(|flag| !Arc::ptr_eq(flag, &self.flag));
            if list.is_empty() {
                flags.remove(&self.session);
            }
        }
    }
}

/// The reply a completed turn writes back to its cards: the text of the
/// turn's last assistant or plan block. Reasoning, tools, approvals, system
/// and error rows never count. `None` when the turn has no such text.
pub(crate) fn final_reply(rows: &[Row]) -> Option<String> {
    rows.iter().rev().find_map(|row| {
        let text = match row.role.as_str() {
            "assistant" | "plan" => row.text.clone(),
            // A host plan is stored as its document; its text is the plan.
            "remote-plan" => serde_json::from_str::<serde_json::Value>(&row.text)
                .ok()?
                .get("text")?
                .as_str()?
                .to_string(),
            _ => return None,
        };
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }

    #[test]
    fn write_back_posts_the_last_assistant_or_plan_block_and_nothing_else() {
        let tool = serde_json::to_string(&gespan_core::tool::ToolCall::new("t", "Bash")).unwrap();
        let rows = [
            row("user", "Fix it"),
            row("assistant", "Looking."),
            row("tool", &tool),
            row("assistant", "Fixed the login check."),
            row("reasoning", "thinking about it"),
            row("tool", &tool),
            row("error", "late failure"),
        ];
        assert_eq!(
            final_reply(&rows).as_deref(),
            Some("Fixed the login check."),
            "reasoning, tool and error rows after the reply never count"
        );
        // A plan turn posts its plan.
        let plan = serde_json::json!({"id":"p","text":"# Plan\n\n- Step","status":"ready"});
        let rows = [
            row("assistant", "Here is the plan:"),
            row("remote-plan", &plan.to_string()),
        ];
        assert_eq!(final_reply(&rows).as_deref(), Some("# Plan\n\n- Step"));
        let rows = [row("assistant", "Draft"), row("plan", "1. Do it")];
        assert_eq!(final_reply(&rows).as_deref(), Some("1. Do it"));
        // Blank text is skipped; a turn of only tools and reasoning has none.
        let rows = [row("assistant", "Earlier"), row("assistant", "  \n")];
        assert_eq!(final_reply(&rows).as_deref(), Some("Earlier"));
        let rows = [
            row("user", "Go"),
            row("reasoning", "hm"),
            row("tool", &tool),
            row("pilot-system", "system note"),
        ];
        assert_eq!(final_reply(&rows), None);
    }

    #[test]
    fn a_scope_change_marks_only_that_sessions_turns_unless_it_is_everyones() {
        let (a, b) = (Watch::new(-9_001), Watch::new(-9_002));
        changed(Some(-9_001));
        assert!(a.take());
        assert!(!a.take(), "taken once");
        assert!(!b.take());
        changed(None);
        assert!(a.take() && b.take());
        drop(a);
        changed(Some(-9_001));
        assert!(!flags().lock().unwrap().contains_key(&-9_001));
    }

    #[test]
    fn the_scope_reads_the_sessions_own_ticket_link_as_it_is_now() {
        use gespan_core::session_groups::WorkItem;
        let dir = tempfile::tempdir().unwrap();
        let store = gespan_core::store::Store::open(&dir.path().join("gespan.db")).unwrap();
        let session = store.create("codex", "/work").unwrap().id;
        let other = store.create("codex", "/work").unwrap().id;
        let ticket = |key: &str| TicketRef {
            tracker: gespan_core::tickets::Tracker::Linear,
            key: key.into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        };
        let link = |store: &Database, id: i64, key: Option<&str>| {
            let item = key.map(|key| WorkItem::Ticket(Box::new(ticket(key))));
            locked(store)
                .unwrap()
                .set_work_item(id, item.as_ref())
                .unwrap();
        };
        let store: Database = Arc::new(Mutex::new(store));
        let source = ScopeSource::new(store.clone(), session, false, false);
        assert_eq!(source.ticket(), Ok(None));
        // Another session's link is never this session's.
        link(&store, other, Some("ENG-9"));
        assert_eq!(source.ticket(), Ok(None));
        link(&store, session, Some("ENG-1"));
        assert_eq!(source.ticket().unwrap().unwrap().key, "ENG-1");
        assert_eq!(source.read().ticket.unwrap().key, "ENG-1");
        // A relink or unlink mid-turn shows on the next read.
        link(&store, session, Some("ENG-2"));
        assert_eq!(source.ticket().unwrap().unwrap().key, "ENG-2");
        link(&store, session, None);
        assert_eq!(source.ticket(), Ok(None));
        // A GitHub link is not a ticket.
        let pr = gespan_core::session_groups::parse_github_work_item(
            "https://github.com/acme/app/pull/9",
        )
        .unwrap();
        locked(&store)
            .unwrap()
            .set_work_item(session, Some(&WorkItem::Github(pr)))
            .unwrap();
        assert_eq!(source.ticket(), Ok(None));
        // A deleted session is an error, and the scope then has no ticket.
        locked(&store).unwrap().delete(session).unwrap();
        assert!(source.ticket().is_err());
        assert!(source.read().ticket.is_none());
    }

    #[test]
    fn the_status_permission_reads_a_remote_session_by_its_remote_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let store = gespan_core::store::Store::open(&path).unwrap();
        let project = store
            .remember_remote_project(
                "env-1",
                &serde_json::json!({"id":"project","cwd":"/host/repo"}),
            )
            .unwrap();
        let summary = store.create("codex", &project.key).unwrap();
        let mut binding = store.remote_session(summary.id).unwrap().unwrap();
        binding.session_id = Some("s-9".into());
        store.save_remote_session(&binding).unwrap();
        let store: Database = Arc::new(Mutex::new(store));
        let key = crate::board::turns::session_key(&store, summary.id).unwrap();
        assert_eq!(key, "remote:env-1:s-9");
        // A strict board's card links the remote session by that key.
        let mut board = Service::open(&path).unwrap();
        let created = board.create("Strict").unwrap();
        let backlog = board.get(&created.id).unwrap().columns[0].id.clone();
        let card = board.card_create(&backlog, "Remote work", "").unwrap();
        board.add_link(&card.id, "session", &key, None).unwrap();
        let before = ScopeSource::new(store.clone(), summary.id, false, false).board_scope();
        assert_eq!(
            before,
            (true, true),
            "the board allows it; write-back is on"
        );
        board
            .update_settings(
                &created.id,
                &gespan_core::board::extras::SettingsPatch {
                    ticket_status_changes: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            ScopeSource::new(store.clone(), summary.id, false, false).board_scope(),
            (false, true)
        );
        board.set_session_status_setting(&key, Some(true)).unwrap();
        assert_eq!(
            ScopeSource::new(store, summary.id, false, false).board_scope(),
            (true, true),
            "the session's own choice beats the board"
        );
    }

    #[test]
    fn a_restricted_source_reads_no_ticket_and_refuses_card_calls_at_answer_time() {
        use gespan_core::session_groups::WorkItem;
        let dir = tempfile::tempdir().unwrap();
        let store = gespan_core::store::Store::open(&dir.path().join("gespan.db")).unwrap();
        let session = store.create("codex", "/work").unwrap().id;
        let item = WorkItem::Ticket(Box::new(TicketRef {
            tracker: gespan_core::tickets::Tracker::Linear,
            key: "ENG-1".into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        }));
        store.set_work_item(session, Some(&item)).unwrap();
        let store: Database = Arc::new(Mutex::new(store));
        let open = ScopeSource::new(store.clone(), session, false, false);
        assert_eq!(open.read().ticket.unwrap().key, "ENG-1");
        let restricted = ScopeSource::new(store, session, false, true);
        let scope = restricted.read();
        assert!(scope.ticket.is_none() && !scope.cards);
        assert_eq!(restricted.board_scope(), (false, false));
        let tool = gespan_core::board::extras::CardTool::Subtasks;
        assert_eq!(restricted.answer_card(&tool), Err(NO_CARDS.to_string()));
    }
}
