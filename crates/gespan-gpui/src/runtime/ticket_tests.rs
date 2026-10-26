use super::*;
use gespan_core::session_groups::WorkItem;
use gespan_core::tickets::{TicketRef, Tracker};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A Codex stand-in that records what it was launched with and, when it has
/// a ticket capability, asks the bridge for its scope and makes the calls
/// the stdio server would: `MOCK_TICKET_CALLS` (a JSON list of
/// `{"key"?, "tool"}`), else one `ticket_get` on ENG-1. With `MOCK_PAUSE`
/// set it writes `ready` and waits for `go` before calling, and with
/// `MOCK_EXPECT_KEY` it first waits until the bridge's scope names that key.
const PROVIDER: &str = r#"#!/usr/bin/env python3
import json, os, socket, sys, time
def send(value):
    print(json.dumps(value), flush=True)
def bridge(key, tool):
    address, port = os.environ['GESPAN_TICKET_ENDPOINT'].rsplit(':', 1)
    with socket.create_connection((address, int(port)), timeout=10) as connection:
        connection.settimeout(10)
        request = {'session': int(os.environ['GESPAN_TICKET_SESSION']), 'token': os.environ['GESPAN_TICKET_TOKEN']}
        if key is not None:
            request['key'] = key
        if tool is not None:
            request['tool'] = tool
        connection.sendall((json.dumps(request) + '\n').encode())
        return json.loads(connection.makefile('rb').readline())
record = {'argv': sys.argv[1:], 'env': sorted(k for k in os.environ if k.startswith('GESPAN_TICKET_'))}
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({'id': 1, 'result': {}})
    elif value.get('id') == 2:
        send({'id': 2, 'result': {'thread': {'id': 'native-ticket-test'}}})
    elif value.get('id') == 3:
        record['prompt'] = value['params']['input'][0]['text']
        if 'GESPAN_TICKET_ENDPOINT' in os.environ:
            record['scope'] = bridge(None, None)
            if 'MOCK_PAUSE' in os.environ:
                open('ready', 'w').close()
                deadline = time.time() + 20
                while not os.path.exists('go') and time.time() < deadline:
                    time.sleep(0.02)
            if 'MOCK_EXPECT_KEY' in os.environ:
                expected = json.loads(os.environ['MOCK_EXPECT_KEY'])
                deadline = time.time() + 20
                while time.time() < deadline:
                    scope = bridge(None, None)
                    if scope.get('ok') and scope['result']['key'] == expected:
                        break
                    time.sleep(0.02)
                record['scope_after'] = scope
            calls = json.loads(os.environ.get('MOCK_TICKET_CALLS', 'null')) or [
                {'key': 'ENG-1', 'tool': json.loads(os.environ.get('MOCK_TICKET_TOOL', '{"tool": "get"}'))}
            ]
            record['replies'] = [bridge(call.get('key'), call['tool']) for call in calls]
            record['reply'] = record['replies'][0]
        with open('record.json', 'w') as output:
            json.dump(record, output)
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'a', 'delta': 'Done.'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#;

/// What a test sets up in storage before the turn: the session, its store
/// and the board database (the same file).
struct Prepared<'a> {
    store: &'a Database,
    session: i64,
    key: String,
    board: gespan_core::board::Service,
}

thread_local! {
    /// This test's last turn's final reply, as the final update carried it.
    static LAST_REPLY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn run(ticket: Option<TicketRef>, settings: Settings) -> (serde_json::Value, bool, String) {
    run_with(ticket, settings, |_| {}, |_, _| {})
}

fn run_with(
    ticket: Option<TicketRef>,
    settings: Settings,
    prepare: impl FnOnce(&mut Prepared),
    pause: impl FnOnce(&mut Prepared, &std::path::Path),
) -> (serde_json::Value, bool, String) {
    run_full(ticket, settings, |p, _| prepare(p), pause, |_| {})
}

/// `prepare` runs before the turn and may add to its settings; `pause` runs
/// while the provider waits mid-turn (when `MOCK_PAUSE` is set), with the
/// provider's folder; `after` runs once the turn finished, before the
/// storage is removed.
fn run_full(
    ticket: Option<TicketRef>,
    mut settings: Settings,
    prepare: impl FnOnce(&mut Prepared, &mut Settings),
    pause: impl FnOnce(&mut Prepared, &std::path::Path),
    after: impl FnOnce(&mut Prepared),
) -> (serde_json::Value, bool, String) {
    // Every test's fresh store numbers its session 1, and the bridge's hub
    // and the scope-change flags are app-wide by session id: one turn at a
    // time.
    static ONE_TURN: Mutex<()> = Mutex::new(());
    let _turn = ONE_TURN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    gespan_tracker_secrets::testing::use_memory_keychain();
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "gespan-runtime-ticket-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&fixture.0).unwrap();
    let binary = fixture.0.join("mock-provider");
    std::fs::write(&binary, PROVIDER).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let db = fixture.0.join("sessions.db");
    let store = Store::open_gespan(&db).unwrap();
    let mut summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    if let Some(ticket) = ticket {
        let item = WorkItem::Ticket(Box::new(ticket));
        store.set_work_item(summary.id, Some(&item)).unwrap();
        summary.work_item = Some(item);
    }
    let key = store.persisted_session_id(summary.id).unwrap();
    let store: Database = Arc::new(Mutex::new(store));
    let mut prepared = Prepared {
        store: &store,
        session: summary.id,
        key,
        board: gespan_core::board::Service::open(&db).unwrap(),
    };
    prepare(&mut prepared, &mut settings);
    let paused = settings.environment.contains_key("MOCK_PAUSE");
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Fix it".into(),
                settings,
            },
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    if paused {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !fixture.0.join("ready").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the provider never paused"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        pause(&mut prepared, &fixture.0);
        std::fs::write(fixture.0.join("go"), "").unwrap();
    }
    let mut ticket_changed = false;
    let status = loop {
        let update = receive.recv_blocking().unwrap();
        ticket_changed |= update.ticket.is_some();
        if update.finished {
            LAST_REPLY.set(update.reply.clone());
            break update.status;
        }
    };
    worker.join().unwrap();
    after(&mut prepared);
    let record =
        serde_json::from_slice(&std::fs::read(fixture.0.join("record.json")).unwrap()).unwrap();
    (record, ticket_changed, status)
}

fn linked() -> Option<TicketRef> {
    Some(TicketRef {
        tracker: Tracker::Linear,
        key: "ENG-1".into(),
        url: None,
        title: Some("Fix login".into()),
        status: None,
        status_group: None,
        fetched_at: None,
    })
}

#[test]
fn a_linked_session_mounts_ticket_tools_and_answers_calls() {
    let (record, ticket_changed, status) = run(linked(), Settings::default());
    assert_eq!(status, "Ready");
    let argv = record["argv"].to_string();
    assert!(
        argv.contains(r#"mcp_servers.gespan_ticket.args=[\"ticket-mcp\"]"#),
        "{argv}"
    );
    assert_eq!(record["env"].as_array().unwrap().len(), 4);
    assert!(record["prompt"].as_str().unwrap().starts_with(
        "This session is linked to Linear ticket ENG-1. Use the gespan_ticket tools to read it, comment on it, and change its status.\n\n"
    ));
    // No Linear credential is saved, so the app's answer is the tracker's
    // user-facing error, delivered while the provider waited on the call.
    assert_eq!(
        record["reply"],
        serde_json::json!({"ok":false,"error":"Connect Linear in Settings"})
    );
    assert!(
        !ticket_changed,
        "A failed call changes nothing, so nothing is refetched"
    );
}

#[test]
fn a_read_only_session_is_told_it_may_only_read_and_cannot_comment() {
    for settings in [
        Settings {
            access: gespan_core::agent::AccessMode::ReadOnly,
            ..Settings::default()
        },
        Settings {
            plan: true,
            ..Settings::default()
        },
    ] {
        let mut settings = settings;
        settings.environment.insert(
            "MOCK_TICKET_TOOL".into(),
            r#"{"tool":"comment","markdown":"Done"}"#.into(),
        );
        let (record, ticket_changed, status) = run(linked(), settings);
        assert_eq!(status, "Ready");
        assert!(record["prompt"].as_str().unwrap().starts_with(
            "This session is linked to Linear ticket ENG-1. Use the gespan_ticket tools to read it.\n\n"
        ));
        assert_eq!(
            record["reply"],
            serde_json::json!({"ok":false,"error":"This session is read-only"})
        );
        assert!(!ticket_changed);
    }
}

#[test]
fn a_session_without_a_ticket_gets_no_ticket_tools() {
    let (record, ticket_changed, status) = run(None, Settings::default());
    assert_eq!(status, "Ready");
    assert!(!record["argv"].to_string().contains("gespan_ticket"));
    assert_eq!(record["env"], serde_json::json!([]));
    assert!(!record["prompt"].as_str().unwrap().contains("ticket"));
    assert!(!ticket_changed);
}

const STATUS_OFF: &str = "Changing ticket status is turned off for this session";
const NOT_CONNECTED: &str = "Connect Linear in Settings";

fn with_calls(mut settings: Settings, calls: serde_json::Value) -> Settings {
    settings
        .environment
        .insert("MOCK_TICKET_CALLS".into(), calls.to_string());
    settings
}

/// A board whose first card links the session, with these settings.
fn linked_board(
    p: &mut Prepared,
    name: &str,
    patch: gespan_core::board::extras::SettingsPatch,
) -> String {
    let board = p.board.create(name).unwrap();
    let backlog = p.board.get(&board.id).unwrap().columns[0].id.clone();
    let card = p.board.card_create(&backlog, name, "").unwrap();
    p.board.add_link(&card.id, "session", &p.key, None).unwrap();
    p.board.update_settings(&board.id, &patch).unwrap();
    card.id
}

fn status_off() -> gespan_core::board::extras::SettingsPatch {
    gespan_core::board::extras::SettingsPatch {
        ticket_status_changes: Some(false),
        ..Default::default()
    }
}

fn errors(record: &serde_json::Value) -> Vec<String> {
    record["replies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reply| reply["error"].as_str().unwrap_or("ok").to_string())
        .collect()
}

fn status_calls() -> serde_json::Value {
    serde_json::json!([
        {"tool": {"tool": "statuses"}},
        {"tool": {"tool": "set_status", "target": "Done"}},
    ])
}

#[test]
fn board_status_off_makes_ticket_status_list_only() {
    let settings = with_calls(Settings::default(), status_calls());
    let (record, _, status) = run_with(
        linked(),
        settings,
        |p| {
            linked_board(p, "Strict", status_off());
        },
        |_, _| {},
    );
    assert_eq!(status, "Ready");
    assert_eq!(record["scope"]["result"]["mayChangeStatus"], false);
    assert!(record["prompt"].as_str().unwrap().starts_with(
        "This session is linked to Linear ticket ENG-1. Use the gespan_ticket tools to read it, comment on it.\n\n"
    ));
    // Listing statuses reaches the tracker; moving is refused.
    assert_eq!(errors(&record), [NOT_CONNECTED, STATUS_OFF]);
}

#[test]
fn session_toggle_overrides_board_setting() {
    // The board says no, the session says yes: the move reaches the tracker.
    let settings = with_calls(Settings::default(), status_calls());
    let (record, _, _) = run_with(
        linked(),
        settings.clone(),
        |p| {
            linked_board(p, "Strict", status_off());
            p.board
                .set_session_status_setting(&p.key, Some(true))
                .unwrap();
        },
        |_, _| {},
    );
    assert_eq!(record["scope"]["result"]["mayChangeStatus"], true);
    assert_eq!(errors(&record), [NOT_CONNECTED, NOT_CONNECTED]);
    // The board says yes, the session says no: refused.
    let (record, _, _) = run_with(
        linked(),
        settings,
        |p| {
            linked_board(p, "Open", Default::default());
            p.board
                .set_session_status_setting(&p.key, Some(false))
                .unwrap();
        },
        |_, _| {},
    );
    assert_eq!(record["scope"]["result"]["mayChangeStatus"], false);
    assert_eq!(errors(&record), [NOT_CONNECTED, STATUS_OFF]);
}

#[test]
fn the_stricter_board_wins_for_a_session_linked_to_cards_on_two_boards() {
    let settings = with_calls(Settings::default(), status_calls());
    let (record, _, _) = run_with(
        linked(),
        settings,
        |p| {
            linked_board(p, "Open", Default::default());
            linked_board(p, "Strict", status_off());
        },
        |_, _| {},
    );
    assert_eq!(record["scope"]["result"]["mayChangeStatus"], false);
    assert_eq!(errors(&record), [NOT_CONNECTED, STATUS_OFF]);
}

#[test]
fn a_status_override_changed_mid_turn_applies_to_the_next_call() {
    let mut settings = with_calls(
        Settings::default(),
        serde_json::json!([{"tool": {"tool": "set_status", "target": "Done"}}]),
    );
    settings.environment.insert("MOCK_PAUSE".into(), "1".into());
    let (record, _, _) = run_with(
        linked(),
        settings,
        |_| {},
        |p, _| {
            // Started allowed (no boards); the user turns it off mid-turn.
            p.board
                .set_session_status_setting(&p.key, Some(false))
                .unwrap();
        },
    );
    assert_eq!(record["scope"]["result"]["mayChangeStatus"], true);
    assert_eq!(errors(&record), [STATUS_OFF]);
}

fn subtask_calls(subtask: &str) -> serde_json::Value {
    serde_json::json!([
        {"tool": {"tool": "card_subtasks"}},
        {"tool": {"tool": "card_subtask_done", "subtaskId": subtask, "done": true}},
    ])
}

#[test]
fn card_tools_offered_only_when_a_write_back_card_links_the_session() {
    // No ticket and no cards: no tools at all.
    let (record, _, _) = run(None, Settings::default());
    assert_eq!(record["env"], serde_json::json!([]));
    // A card on a board with write-back off: still no tools.
    let (record, _, _) = run_with(
        None,
        Settings::default(),
        |p| {
            linked_board(
                p,
                "Quiet",
                gespan_core::board::extras::SettingsPatch {
                    write_back: Some(false),
                    ..Default::default()
                },
            );
        },
        |_, _| {},
    );
    assert_eq!(record["env"], serde_json::json!([]));
    // A card on a write-back board: card tools, without ticket tools.
    let (record, _, _) = run_with(
        None,
        with_calls(
            Settings::default(),
            serde_json::json!([{"tool": {"tool": "card_subtasks"}}]),
        ),
        |p| {
            linked_board(p, "Loud", Default::default());
        },
        |_, _| {},
    );
    assert_eq!(record["env"].as_array().unwrap().len(), 4);
    assert_eq!(record["scope"]["result"]["cards"], true);
    assert_eq!(record["scope"]["result"]["key"], serde_json::Value::Null);
    assert_eq!(record["replies"][0]["ok"], true, "{record}");
    assert!(!record["prompt"].as_str().unwrap().contains("ticket"));
}

#[test]
fn agent_ticks_a_subtask_through_card_subtask_done() {
    let card = std::cell::RefCell::new(String::new());
    let (record, _, status) = run_full(
        None,
        Settings::default(),
        |p, settings| {
            *card.borrow_mut() = linked_board(p, "Work", Default::default());
            let subtask = p
                .board
                .add_subtask(&card.borrow(), "Write the test")
                .unwrap();
            settings.environment.insert(
                "MOCK_TICKET_CALLS".into(),
                subtask_calls(&subtask.id).to_string(),
            );
        },
        |_, _| {},
        |p| {
            let card = card.borrow();
            let subtask = &p.board.subtasks(&card).unwrap()[0];
            assert!(subtask.done, "the agent ticked it");
            let board = p.board.card(&card).unwrap().board_id;
            let actors: Vec<String> = p
                .board
                .activity(&board, Some(&card), 20)
                .unwrap()
                .into_iter()
                .map(|row| row.actor)
                .collect();
            assert!(actors.contains(&format!("agent:{}", p.key)), "{actors:?}");
        },
    );
    assert_eq!(status, "Ready");
    assert_eq!(errors(&record), ["ok", "ok"], "{record}");
    let listed = &record["replies"][0]["result"]["cards"][0];
    assert_eq!(listed["subtasks"][0]["title"], "Write the test");
    assert_eq!(listed["subtasks"][0]["done"], false);
    assert_eq!(record["replies"][1]["result"]["subtask"]["done"], true);
}

#[test]
fn plan_mode_session_cannot_tick_subtasks_or_comment() {
    for read_only in [
        Settings {
            plan: true,
            ..Settings::default()
        },
        Settings {
            access: gespan_core::agent::AccessMode::ReadOnly,
            ..Settings::default()
        },
    ] {
        let card = std::cell::RefCell::new(String::new());
        let (record, ticket_changed, _) = run_full(
            linked(),
            read_only,
            |p, settings| {
                *card.borrow_mut() = linked_board(p, "Work", Default::default());
                let subtask = p
                    .board
                    .add_subtask(&card.borrow(), "Write the test")
                    .unwrap();
                let mut calls = subtask_calls(&subtask.id);
                calls.as_array_mut().unwrap().push(serde_json::json!(
                    {"key": "ENG-1", "tool": {"tool": "comment", "markdown": "Done"}}
                ));
                settings
                    .environment
                    .insert("MOCK_TICKET_CALLS".into(), calls.to_string());
            },
            |_, _| {},
            |p| assert!(!p.board.subtasks(&card.borrow()).unwrap()[0].done),
        );
        assert_eq!(record["scope"]["result"]["readOnly"], true);
        // Listing works; ticking and commenting are refused.
        assert_eq!(
            errors(&record),
            [
                "ok",
                "This session is read-only",
                "This session is read-only"
            ]
        );
        assert!(!ticket_changed);
    }
}

#[test]
fn card_tools_keep_rejecting_unknown_keys() {
    let (record, _, _) = run_full(
        None,
        Settings::default(),
        |p, settings| {
            let card = linked_board(p, "Work", Default::default());
            let subtask = p.board.add_subtask(&card, "One").unwrap();
            let calls = serde_json::json!([
                {"tool": {"tool": "card_subtasks", "cardId": "other"}},
                {"tool": {"tool": "card_subtask_done", "subtaskId": subtask.id, "done": true, "session": "x"}},
            ]);
            settings
                .environment
                .insert("MOCK_TICKET_CALLS".into(), calls.to_string());
        },
        |_, _| {},
        |_| {},
    );
    assert_eq!(
        errors(&record),
        ["Invalid ticket request", "Invalid ticket request"]
    );
}

fn ticket(key: &str) -> TicketRef {
    TicketRef {
        key: key.into(),
        ..linked().unwrap()
    }
}

fn relink(p: &mut Prepared, key: Option<&str>) {
    let item = key.map(|key| WorkItem::Ticket(Box::new(ticket(key))));
    locked(p.store)
        .unwrap()
        .set_work_item(p.session, item.as_ref())
        .unwrap();
}

#[test]
fn a_relink_mid_turn_refuses_the_old_key_at_answer_time() {
    // The bridge still holds ENG-1 when the call arrives (nothing reported
    // the change); the app reads ENG-2 from the store and refuses.
    let mut settings = with_calls(
        Settings::default(),
        serde_json::json!([{"key": "ENG-1", "tool": {"tool": "get"}}]),
    );
    settings.environment.insert("MOCK_PAUSE".into(), "1".into());
    let (record, _, _) = run_with(linked(), settings, |_| {}, |p, _| relink(p, Some("ENG-2")));
    assert_eq!(errors(&record), ["This session is linked to ENG-2"]);
}

#[test]
fn an_unlink_mid_turn_refuses_ticket_calls() {
    let mut settings = with_calls(
        Settings::default(),
        serde_json::json!([{"tool": {"tool": "get"}}]),
    );
    settings.environment.insert("MOCK_PAUSE".into(), "1".into());
    let (record, _, _) = run_with(linked(), settings, |_| {}, |p, _| relink(p, None));
    assert_eq!(errors(&record), ["This session isn't linked to a ticket"]);
}

#[test]
fn a_reported_relink_mid_turn_moves_the_next_call_to_the_new_ticket() {
    let mut settings = with_calls(
        Settings::default(),
        serde_json::json!([
            {"key": "ENG-2", "tool": {"tool": "get"}},
            {"key": "ENG-1", "tool": {"tool": "get"}},
        ]),
    );
    settings.environment.insert("MOCK_PAUSE".into(), "1".into());
    settings
        .environment
        .insert("MOCK_EXPECT_KEY".into(), "\"ENG-2\"".into());
    let (record, _, _) = run_with(
        linked(),
        settings,
        |_| {},
        |p, _| {
            relink(p, Some("ENG-2"));
            crate::runtime::ticket_scope::changed(Some(p.session));
        },
    );
    assert_eq!(record["scope"]["result"]["key"], "ENG-1");
    assert_eq!(record["scope_after"]["result"]["key"], "ENG-2");
    // The new ticket's call reaches the tracker; the old key is refused.
    assert_eq!(
        errors(&record),
        [NOT_CONNECTED, "This session is linked to ENG-2"]
    );
}

#[test]
fn a_completed_turn_hands_its_final_reply_to_write_back() {
    let (_, _, status) = run(None, Settings::default());
    assert_eq!(status, "Ready");
    assert_eq!(LAST_REPLY.take().as_deref(), Some("Done."));
}
