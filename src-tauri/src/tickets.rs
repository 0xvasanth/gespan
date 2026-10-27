//! Ticket links for sessions: resolve what the user typed, refresh a linked
//! ticket, and the Plane connection. The logic lives in `gespan-tracker-client`;
//! these commands only run it off the main thread (credentials come from the
//! keychain, and fetches go over the network).
use std::path::PathBuf;

use gespan_board_store::{check_session_key, may_change_ticket_status};
use gespan_tracker_client::credentials::plane::{self, PlaneStatus};
use gespan_tracker_client::tickets::{parse_ticket, Resolved, TicketRef, Tickets, Tracker};
use gespan_tracker_client::tools::{answer, refusal, Effect, TicketTool, NO_TICKET};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::session_store::SessionStore;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolveResult {
    One {
        ticket: TicketRef,
    },
    /// A bare key that more than one connected tracker could own; the caller
    /// picks one and builds the ref from `key`.
    Ambiguous {
        key: String,
        trackers: Vec<Tracker>,
    },
    Error {
        message: String,
    },
}

impl ResolveResult {
    fn new(input: &str, resolved: Resolved) -> Self {
        match resolved {
            Resolved::One(ticket) => Self::One { ticket },
            Resolved::Ambiguous(trackers) => Self::Ambiguous {
                key: parse_ticket(input).map(|p| p.key).unwrap_or_default(),
                trackers,
            },
            other => Self::Error {
                message: other
                    .message()
                    .unwrap_or_else(|| "Not a ticket key or link".into()),
            },
        }
    }
}

fn resolve_with(tickets: &Tickets, input: &str) -> ResolveResult {
    ResolveResult::new(input, tickets.resolve(input))
}

fn fetch_with(tickets: &Tickets, ticket: &TicketRef) -> Result<TicketRef, String> {
    tickets.fetch(ticket).map_err(|error| error.to_string())
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|error| error.to_string())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn ticket_resolve(app: AppHandle, input: String) -> Result<ResolveResult, String> {
    let dir = data_dir(&app)?;
    blocking(move || Ok(resolve_with(&Tickets::new(dir), &input))).await
}

#[tauri::command]
pub async fn ticket_fetch(app: AppHandle, ticket: TicketRef) -> Result<TicketRef, String> {
    let dir = data_dir(&app)?;
    blocking(move || fetch_with(&Tickets::new(dir), &ticket)).await
}

/// Trackers with saved credentials, for the stale marker on ticket chips.
#[tauri::command]
pub async fn ticket_connected(app: AppHandle) -> Result<Vec<Tracker>, String> {
    let dir = data_dir(&app)?;
    blocking(move || Ok(Tickets::new(dir).connected())).await
}

#[tauri::command]
pub async fn plane_status(app: AppHandle) -> Result<PlaneStatus, String> {
    let dir = data_dir(&app)?;
    blocking(move || plane::plane_status(dir)).await
}

/// An empty key disconnects.
#[tauri::command]
pub async fn plane_set_config(
    app: AppHandle,
    base_url: String,
    workspace: String,
    api_key: String,
) -> Result<PlaneStatus, String> {
    let dir = data_dir(&app)?;
    blocking(move || plane::plane_set_config(dir, base_url, workspace, api_key)).await
}

/// The ticket the session is linked to right now, read from the session
/// store. Agents never name the ticket a tool acts on; this read does.
fn linked_ticket(conn: &Connection, session_id: &str) -> Result<Option<TicketRef>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT linked_work_item_json FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    let Some(item) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) else {
        return Ok(None);
    };
    if item.get("kind").and_then(Value::as_str) != Some("ticket") {
        return Ok(None);
    }
    Ok(item
        .get("ticket")
        .cloned()
        .and_then(|ticket| serde_json::from_value(ticket).ok()))
}

fn same_ticket(a: &TicketRef, b: &TicketRef) -> bool {
    a.tracker == b.tracker && a.key.trim().eq_ignore_ascii_case(b.key.trim())
}

/// The session's current ticket for this call, or why the call is refused:
/// no ticket, a key naming another ticket, a read-only turn, or status
/// changes turned off for the session.
fn ticket_for_call(
    conn: &Connection,
    session_id: &str,
    read_only: bool,
    tool: &TicketTool,
    key: Option<&str>,
) -> Result<TicketRef, String> {
    check_session_key(session_id)?;
    let linked = linked_ticket(conn, session_id)?;
    if let Some(key) = key {
        match &linked {
            Some(ticket) if ticket.key.trim().eq_ignore_ascii_case(key.trim()) => {}
            Some(ticket) => return Err(format!("This session is linked to {}", ticket.key)),
            None => return Err(NO_TICKET.into()),
        }
    }
    let ticket = linked.ok_or(NO_TICKET)?;
    if let Some(refused) = refusal(tool, may_change_ticket_status(conn, session_id), read_only) {
        return Err(refused.into());
    }
    Ok(ticket)
}

/// Stores a moved ticket's snapshot on the session, unless the session was
/// unlinked or linked to another ticket while the move was out. Other fields
/// of the stored link are kept.
fn store_moved(conn: &Connection, session_id: &str, moved: &TicketRef) -> Result<bool, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    if !linked_ticket(&tx, session_id)?.is_some_and(|ticket| same_ticket(&ticket, moved)) {
        return Ok(false);
    }
    let raw: String = tx
        .query_row(
            "SELECT linked_work_item_json FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut item: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    item["ticket"] = serde_json::to_value(moved).map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE sessions SET linked_work_item_json = ?1 WHERE id = ?2",
        params![item.to_string(), session_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(true)
}

/// One agent ticket call. The store lock is held only to read the scope and
/// to store a move, never across the tracker request. Answers
/// `{result, ticket}`: `result` goes to the agent; `ticket` is the moved
/// snapshot the app stored, for the ticket chip.
fn ticket_tool_with(
    store: &SessionStore,
    tickets: &Tickets,
    session_id: &str,
    read_only: bool,
    tool: &TicketTool,
    key: Option<&str>,
) -> Result<Value, String> {
    let ticket = {
        let conn = store.lock_conn()?;
        ticket_for_call(&conn, session_id, read_only, tool, key)?
    };
    let answered = answer(tickets, &ticket, tool)?;
    let stored = match answered.effect {
        Effect::Moved(moved) => {
            let conn = store.lock_conn()?;
            store_moved(&conn, session_id, &moved)?.then_some(moved)
        }
        Effect::None | Effect::Commented => None,
    };
    Ok(json!({ "result": answered.value, "ticket": stored }))
}

/// A ticket tool an agent called through the app CLI (`ticket.*`), for the
/// session the control host granted. `read_only` is set by the app for a
/// plan turn. The ticket is the session's link at call time.
#[tauri::command]
pub async fn ticket_tool(
    app: AppHandle,
    session_id: String,
    read_only: bool,
    tool: TicketTool,
    key: Option<String>,
) -> Result<Value, String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        let store = app.state::<SessionStore>();
        ticket_tool_with(
            &store,
            &Tickets::new(dir),
            &session_id,
            read_only,
            &tool,
            key.as_deref(),
        )
    })
    .await
}

/// Whether the agent in this session may change its ticket's status now, for
/// the introduction the app gives it. The tools check again on every call.
#[tauri::command(async)]
pub fn ticket_may_change_status(
    store: tauri::State<'_, SessionStore>,
    session_id: String,
) -> Result<bool, String> {
    check_session_key(&session_id)?;
    Ok(may_change_ticket_status(&*store.lock_conn()?, &session_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_client::provider_boards::fake::FakeProvider;
    use gespan_tracker_client::provider_boards::{
        BoardProvider, RemoteIssue, StateGroup, WorkflowState,
    };
    use gespan_tracker_client::tickets::Connection;
    use serde_json::json;
    use std::sync::Arc;

    struct Conn(FakeProvider);

    impl Connection for Conn {
        fn board(&self) -> &dyn BoardProvider {
            &self.0
        }
    }

    fn issue(key: &str) -> RemoteIssue {
        RemoteIssue {
            id: key.into(),
            key: key.into(),
            title: "Fix login".into(),
            url: format!("https://tracker.test/{key}"),
            container_id: "C1".into(),
            updated_at: "2026-03-01T10:00:00Z".into(),
            state: WorkflowState {
                id: "todo".into(),
                name: "Todo".into(),
                group: StateGroup::Started,
            },
            ..Default::default()
        }
    }

    fn tickets(trackers: &[Tracker], fake: impl Fn() -> FakeProvider) -> Tickets {
        gespan_tracker_secrets::testing::use_memory_keychain();
        let providers = trackers
            .iter()
            .map(|tracker| (*tracker, Arc::new(Conn(fake())) as Arc<dyn Connection>))
            .collect();
        Tickets::with_providers(PathBuf::from("/unused"), providers)
    }

    #[test]
    fn a_bare_key_with_two_trackers_connected_asks_which_one() {
        let t = tickets(&[Tracker::Jira, Tracker::Linear], FakeProvider::new);
        let result = resolve_with(&t, "ENG-12");
        assert_eq!(
            result,
            ResolveResult::Ambiguous {
                key: "ENG-12".into(),
                trackers: vec![Tracker::Jira, Tracker::Linear]
            }
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({ "kind": "ambiguous", "key": "ENG-12", "trackers": ["jira", "linear"] })
        );
    }

    #[test]
    fn a_linear_url_resolves_to_one_ticket_without_a_fetch() {
        let t = tickets(&[Tracker::Linear], FakeProvider::new);
        let result = resolve_with(&t, "https://linear.app/acme/issue/ENG-7/fix-login");
        let ResolveResult::One { ticket } = &result else {
            panic!("expected one ticket, got {result:?}");
        };
        assert_eq!(
            (ticket.tracker, ticket.key.as_str()),
            (Tracker::Linear, "ENG-7")
        );
        assert_eq!(ticket.title, None);
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["kind"], "one");
        assert_eq!(value["ticket"]["key"], "ENG-7");
    }

    #[test]
    fn a_url_for_a_tracker_that_is_not_connected_is_an_error_message() {
        let t = tickets(&[Tracker::Jira], FakeProvider::new);
        let result = resolve_with(&t, "https://linear.app/acme/issue/ENG-7/fix-login");
        assert_eq!(
            result,
            ResolveResult::Error {
                message: "Connect Linear in Settings".into()
            }
        );
    }

    #[test]
    fn garbage_and_a_bare_key_with_nothing_connected_are_errors() {
        let t = tickets(&[], FakeProvider::new);
        for input in ["hello there", "ENG-12", ""] {
            assert_eq!(
                resolve_with(&t, input),
                ResolveResult::Error {
                    message: "Not a ticket key or link".into()
                },
                "{input:?}"
            );
        }
    }

    #[test]
    fn fetch_fills_the_snapshot_and_errors_are_user_facing_text() {
        let t = tickets(&[Tracker::Linear], || {
            FakeProvider::new().with_issue(issue("ENG-7"))
        });
        let linked = TicketRef {
            tracker: Tracker::Linear,
            key: "ENG-7".into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        };
        let filled = fetch_with(&t, &linked).unwrap();
        assert_eq!(filled.title.as_deref(), Some("Fix login"));
        assert_eq!(filled.status.as_deref(), Some("Todo"));
        assert_eq!(filled.status_group, Some(StateGroup::Started));
        assert!(filled.fetched_at.is_some());

        let jira = TicketRef {
            tracker: Tracker::Jira,
            ..linked
        };
        assert_eq!(
            fetch_with(&t, &jira).unwrap_err(),
            "Connect Jira in Settings"
        );
    }

    // Agent ticket calls (`ticket.*` app actions).

    struct Shared(Arc<FakeProvider>);

    impl Connection for Shared {
        fn board(&self) -> &dyn BoardProvider {
            &*self.0
        }
    }

    fn shared_tickets(fake: Arc<FakeProvider>) -> Tickets {
        gespan_tracker_secrets::testing::use_memory_keychain();
        Tickets::with_providers(
            PathBuf::from("/unused"),
            vec![(
                Tracker::Linear,
                Arc::new(Shared(fake)) as Arc<dyn Connection>,
            )],
        )
    }

    fn linear(key: &str) -> TicketRef {
        TicketRef {
            tracker: Tracker::Linear,
            key: key.into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        }
    }

    fn session_store(id: &str, ticket: Option<&str>) -> SessionStore {
        let store = SessionStore::open_in_memory().unwrap();
        crate::session_store::upsert_session(
            &store.lock_conn().unwrap(),
            &crate::session_store::SessionUpsert {
                sidebar_hidden: false,
                id: id.into(),
                cwd: "/repo".into(),
                harness: "codex".into(),
                model: "gpt-5".into(),
                model_settings: json!({}),
                runtime_mode: "supervised".into(),
                title: "Session".into(),
                provider_session_id: None,
                provider_account_id: None,
                blocks: json!([]),
                queued_messages: vec![],
                queue_status: None,
                context_used: None,
                context_window: None,
                branch: None,
                worktree_cwd: None,
                worktree_removed: false,
                linked_work_item: None,
                automation_id: None,
            },
        )
        .unwrap();
        link(&store, id, ticket);
        store
    }

    /// Relinks (or unlinks) the session the way the app's ticket chip does.
    fn link(store: &SessionStore, id: &str, ticket: Option<&str>) {
        let item = ticket.map(|key| json!({"kind":"ticket","ticket":linear(key)}).to_string());
        store
            .lock_conn()
            .unwrap()
            .execute(
                "UPDATE sessions SET linked_work_item_json = ?1 WHERE id = ?2",
                rusqlite::params![item, id],
            )
            .unwrap();
    }

    fn fake_with(keys: &[&str]) -> Arc<FakeProvider> {
        Arc::new(
            keys.iter()
                .fold(FakeProvider::new(), |fake, key| fake.with_issue(issue(key))),
        )
    }

    fn comment(text: &str) -> TicketTool {
        TicketTool::Comment {
            markdown: text.into(),
        }
    }

    fn set_status(target: &str) -> TicketTool {
        TicketTool::SetStatus {
            target: target.into(),
            fields: Default::default(),
        }
    }

    fn call(
        store: &SessionStore,
        tickets: &Tickets,
        read_only: bool,
        tool: TicketTool,
        key: Option<&str>,
    ) -> Result<Value, String> {
        ticket_tool_with(store, tickets, "s1", read_only, &tool, key)
    }

    /// The `ticket_tool` command as Tauri runs it: each argument is
    /// deserialized from its camelCase key of the invoke payload (the object
    /// `runTicketTool` sends), then the command body runs.
    fn invoke_ticket_tool(
        store: &SessionStore,
        tickets: &Tickets,
        payload: Value,
    ) -> Result<Value, String> {
        fn arg<T: serde::de::DeserializeOwned>(payload: &Value, key: &str) -> Result<T, String> {
            serde_json::from_value(payload.get(key).cloned().unwrap_or(Value::Null))
                .map_err(|error| format!("invalid args `{key}` for command `ticket_tool`: {error}"))
        }
        let session_id: String = arg(&payload, "sessionId")?;
        let read_only: bool = arg(&payload, "readOnly")?;
        let tool: TicketTool = arg(&payload, "tool")?;
        let key: Option<String> = arg(&payload, "key")?;
        ticket_tool_with(
            store,
            tickets,
            &session_id,
            read_only,
            &tool,
            key.as_deref(),
        )
    }

    #[test]
    fn ticket_tool_command_refuses_an_unknown_key_in_the_tool() {
        let fake = fake_with(&["ENG-1", "ENG-2"]);
        let tickets = shared_tickets(fake.clone());
        let store = session_store("s1", Some("ENG-1"));
        let payload =
            |tool: Value| json!({"sessionId": "s1", "readOnly": false, "tool": tool, "key": null});
        // The payloads the app really sends run.
        let read = invoke_ticket_tool(&store, &tickets, payload(json!({"tool": "get"}))).unwrap();
        assert_eq!(read["result"]["key"], "ENG-1");
        assert!(invoke_ticket_tool(&store, &tickets, payload(json!({"tool": "statuses"}))).is_ok());
        invoke_ticket_tool(
            &store,
            &tickets,
            payload(json!({"tool": "comment", "markdown": "From the agent"})),
        )
        .unwrap();
        assert_eq!(fake.comments("ENG-1").len(), 1);
        // An extra key, even one naming another ticket, is refused before
        // anything runs.
        for tool in [
            json!({"tool": "get", "key": "ENG-2"}),
            json!({"tool": "statuses", "ticket": "ENG-2"}),
            json!({"tool": "comment", "markdown": "x", "key": "ENG-2"}),
            json!({"tool": "set_status", "target": "Done", "fields": {}, "sessionId": "s2"}),
        ] {
            let error = invoke_ticket_tool(&store, &tickets, payload(tool.clone())).unwrap_err();
            assert!(error.contains("unknown field"), "{tool}: {error}");
        }
        assert!(fake.comments("ENG-2").is_empty());
        assert_eq!(fake.comments("ENG-1").len(), 1);
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
    }

    #[test]
    fn ticket_tool_refuses_another_key() {
        let fake = fake_with(&["ENG-1", "ENG-2"]);
        let tickets = shared_tickets(fake.clone());
        let store = session_store("s1", Some("ENG-1"));
        for tool in [
            TicketTool::Get,
            comment("Hi"),
            TicketTool::Statuses,
            set_status("Doing"),
        ] {
            assert_eq!(
                call(&store, &tickets, false, tool, Some("ENG-2")).unwrap_err(),
                "This session is linked to ENG-1"
            );
        }
        assert!(fake.comments("ENG-2").is_empty());
        assert_eq!(fake.get_issue("ENG-2").unwrap().state.name, "Todo");
        // Its own key, in any case, is fine; so is no key.
        let read = call(&store, &tickets, false, TicketTool::Get, Some(" eng-1 ")).unwrap();
        assert_eq!(read["result"]["key"], "ENG-1");
        assert_eq!(read["ticket"], Value::Null);
        assert_eq!(
            call(&store, &tickets, false, TicketTool::Get, None).unwrap()["result"]["key"],
            "ENG-1"
        );
        // Unlinked sessions are refused whatever key is named.
        let unlinked = session_store("s1", None);
        for key in [None, Some("ENG-1")] {
            assert_eq!(
                call(&unlinked, &tickets, false, TicketTool::Get, key).unwrap_err(),
                NO_TICKET
            );
        }
        // A session id that isn't a valid key never reaches the store.
        assert_eq!(
            ticket_tool_with(&store, &tickets, " ", false, &TicketTool::Get, None).unwrap_err(),
            "Invalid session key"
        );
    }

    #[test]
    fn ticket_tool_uses_the_link_at_call_time() {
        let fake = fake_with(&["ENG-1", "ENG-2"]);
        let tickets = shared_tickets(fake.clone());
        let store = session_store("s1", Some("ENG-1"));
        call(&store, &tickets, false, comment("First"), None).unwrap();
        assert_eq!(fake.comments("ENG-1").len(), 1);

        // Relinked mid-turn: the next call acts on the new ticket, and the
        // old key the agent still remembers is refused.
        link(&store, "s1", Some("ENG-2"));
        assert_eq!(
            call(&store, &tickets, false, comment("Old"), Some("ENG-1")).unwrap_err(),
            "This session is linked to ENG-2"
        );
        call(&store, &tickets, false, comment("Second"), None).unwrap();
        assert_eq!(fake.comments("ENG-1").len(), 1);
        assert_eq!(fake.comments("ENG-2")[0].1, "Second");

        // Unlinked mid-turn: every ticket call is refused.
        link(&store, "s1", None);
        for tool in [
            TicketTool::Get,
            comment("Late"),
            TicketTool::Statuses,
            set_status("Done"),
        ] {
            assert_eq!(
                call(&store, &tickets, false, tool, None).unwrap_err(),
                NO_TICKET
            );
        }
        assert_eq!(fake.comments("ENG-2").len(), 1);
        assert_eq!(fake.get_issue("ENG-2").unwrap().state.name, "Todo");
    }

    #[test]
    fn ticket_tool_read_only_refuses_comment_and_move() {
        let fake = fake_with(&["ENG-1"]);
        let tickets = shared_tickets(fake.clone());
        let store = session_store("s1", Some("ENG-1"));
        for tool in [comment("Plan says hi"), set_status("Done")] {
            assert_eq!(
                call(&store, &tickets, true, tool, None).unwrap_err(),
                "This session is read-only"
            );
        }
        assert!(fake.comments("ENG-1").is_empty());
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
        // Reading and listing still work in a plan turn.
        assert_eq!(
            call(&store, &tickets, true, TicketTool::Get, None).unwrap()["result"]["title"],
            "Fix login"
        );
        assert!(
            call(&store, &tickets, true, TicketTool::Statuses, None).unwrap()["result"]["statuses"]
                .as_array()
                .is_some_and(|statuses| !statuses.is_empty())
        );
    }

    #[test]
    fn ticket_tool_status_off_lists_but_never_moves() {
        let fake = fake_with(&["ENG-1"]);
        let tickets = shared_tickets(fake.clone());
        let store = session_store("s1", Some("ENG-1"));
        gespan_board_store::set_session_status_setting(
            &store.lock_conn().unwrap(),
            "s1",
            Some(false),
        )
        .unwrap();
        assert_eq!(
            call(&store, &tickets, false, set_status("Done"), None).unwrap_err(),
            "Changing ticket status is turned off for this session"
        );
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
        let listing = call(&store, &tickets, false, TicketTool::Statuses, None).unwrap();
        assert_eq!(listing["result"]["key"], "ENG-1");
        assert!(listing["result"]["statuses"]
            .as_array()
            .is_some_and(|statuses| !statuses.is_empty()));
        // Comments are not status changes.
        call(&store, &tickets, false, comment("Still here"), None).unwrap();
        assert_eq!(fake.comments("ENG-1").len(), 1);
        // Turned back on, the same move goes through.
        gespan_board_store::set_session_status_setting(
            &store.lock_conn().unwrap(),
            "s1",
            Some(true),
        )
        .unwrap();
        call(&store, &tickets, false, set_status("Done"), None).unwrap();
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Done");
    }

    #[test]
    fn moved_ticket_is_stored_on_the_session() {
        let fake = fake_with(&["ENG-1"]);
        let tickets = shared_tickets(fake.clone());
        let store = session_store("s1", Some("ENG-1"));
        let moved = call(&store, &tickets, false, set_status("Doing"), None).unwrap();
        assert_eq!(moved["result"]["status"], "Doing");
        assert_eq!(moved["ticket"]["key"], "ENG-1");
        assert_eq!(moved["ticket"]["statusGroup"], "started");
        let stored = linked_ticket(&store.lock_conn().unwrap(), "s1")
            .unwrap()
            .unwrap();
        assert_eq!(stored.status.as_deref(), Some("Doing"));
        assert_eq!(stored.status_group, Some(StateGroup::Started));
        assert!(stored.fetched_at.is_some());
        // Only the snapshot changes: the link's other fields stay.
        let raw: String = store
            .lock_conn()
            .unwrap()
            .query_row(
                "SELECT linked_work_item_json FROM sessions WHERE id = 's1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap()["kind"],
            "ticket"
        );

        // A move that lands after the user relinked never restores the old
        // ticket over the new link.
        link(&store, "s1", Some("ENG-2"));
        assert!(!store_moved(&store.lock_conn().unwrap(), "s1", &stored).unwrap());
        assert_eq!(
            linked_ticket(&store.lock_conn().unwrap(), "s1")
                .unwrap()
                .unwrap()
                .key,
            "ENG-2"
        );
        link(&store, "s1", None);
        assert!(!store_moved(&store.lock_conn().unwrap(), "s1", &stored).unwrap());
        assert_eq!(
            linked_ticket(&store.lock_conn().unwrap(), "s1").unwrap(),
            None
        );
    }
}
