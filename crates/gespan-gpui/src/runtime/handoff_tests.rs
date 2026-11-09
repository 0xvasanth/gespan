use super::*;
use gespan_core::tool::{ToolCall, ToolState};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn transfer(rows: &[Row]) -> Option<ToolCall> {
    rows.iter()
        .filter(|row| row.role == "tool")
        .filter_map(|row| serde_json::from_str::<ToolCall>(&row.text).ok())
        .find(ToolCall::is_handoff)
}

#[test]
fn a_provider_switch_shows_its_transfer_before_the_new_provider_answers() {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "gespan-runtime-handoff-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&fixture.0).unwrap();
    let binary = fixture.0.join("mock-provider");
    std::fs::write(
        &binary,
        r#"#!/usr/bin/env python3
import json, sys
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    method = value.get('method')
    if method == 'initialize':
        send({'id': value['id'], 'result': {}})
    elif method == 'thread/start':
        send({'id': value['id'], 'result': {'thread': {'id': 'thread'}}})
    elif method == 'turn/start':
        send({'id': value['id'], 'result': {'turn': {'id': 'turn'}}})
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'answer', 'delta': 'Answer'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#,
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open_gespan(&fixture.0.join("sessions.db")).unwrap();
    let summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    let session = summary.id;
    let store = Arc::new(Mutex::new(store));
    let worker_store = store.clone();
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Carry on".into(),
                settings: Settings {
                    text_only: true,
                    pending_provider_switch: Some(gespan_core::agent::ProviderSwitch {
                        from: "claude".into(),
                        model_label: "Opus 5.5".into(),
                        skip_outgoing_recap: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    let mut running_seen = false;
    let final_status = loop {
        let update = receive
            .recv_blocking()
            .expect("Runtime closed before its final update");
        if let Some(tool) = transfer(&update.rows)
            && tool.state == ToolState::Running
        {
            assert_eq!(tool.summary().0, "Transferring from Opus 5.5");
            assert!(
                update.rows.iter().all(|row| row.text != "Answer"),
                "the transfer shows before the new provider answers"
            );
            running_seen = true;
        }
        if update.finished {
            break update.status;
        }
    };
    worker.join().unwrap();
    assert_eq!(final_status, "Ready");
    assert!(running_seen);
    let saved = locked(&store).unwrap().load(session).unwrap();
    let settled = transfer(&saved.rows).unwrap();
    assert_eq!(settled.state, ToolState::Completed);
    assert_eq!(settled.summary().0, "Transferred from Opus 5.5");
    assert!(saved.rows.iter().any(|row| row.text == "Answer"));
}

/// A Codex stand-in that records every request it receives.
fn recording_provider(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    let binary = dir.join("mock-provider");
    let log = dir.join("requests.jsonl");
    std::fs::write(
        &binary,
        format!(
            r#"#!/usr/bin/env python3
import json, sys
log = open({log:?}, 'a')
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    log.write(line)
    log.flush()
    value = json.loads(line)
    method = value.get('method')
    if method == 'initialize':
        send({{'id': value['id'], 'result': {{}}}})
    elif method == 'thread/start':
        send({{'id': value['id'], 'result': {{'thread': {{'id': 'thread'}}}}}})
    elif method == 'turn/start':
        send({{'id': value['id'], 'result': {{'turn': {{'id': 'turn'}}}}}})
        send({{'method': 'item/agentMessage/delta', 'params': {{'itemId': 'answer', 'delta': 'Answer'}}}})
        send({{'method': 'turn/completed', 'params': {{'turn': {{'status': 'completed'}}}}}})
        break
"#,
            log = log.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    (binary, log)
}

fn fixture_dir(name: &str) -> Fixture {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "gespan-runtime-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&fixture.0).unwrap();
    fixture
}

/// Seeds a session the way a fork or handoff does, runs one turn on it and
/// returns the store and the requests the provider received.
fn first_turn_after_transfer(
    fixture: &Fixture,
    settings: Settings,
    rows: &[Row],
) -> (Arc<Mutex<Store>>, i64, Vec<serde_json::Value>) {
    let (binary, log) = recording_provider(&fixture.0);
    let mut store = Store::open_gespan(&fixture.0.join("sessions.db")).unwrap();
    let settings = Settings {
        text_only: true,
        ..settings
    };
    let summary = store
        .create_from(
            "codex",
            fixture.0.to_str().unwrap(),
            "Fork of Footer",
            &settings,
            rows,
        )
        .unwrap();
    let session = summary.id;
    let store = Arc::new(Mutex::new(store));
    let worker_store = store.clone();
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Carry on".into(),
                settings,
            },
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    let status = loop {
        let update = receive
            .recv_blocking()
            .expect("Runtime closed before its final update");
        if update.finished {
            break update.status;
        }
    };
    worker.join().unwrap();
    assert_eq!(status, "Ready");
    let requests = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (store, session, requests)
}

fn request<'a>(requests: &'a [serde_json::Value], method: &str) -> &'a serde_json::Value {
    requests
        .iter()
        .find(|value| value["method"] == method)
        .unwrap_or_else(|| panic!("the provider never received {method}"))
}

fn footer_chat() -> Vec<Row> {
    let row = |role: &str, text: &str| Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    };
    vec![
        row("user", "Plan the footer"),
        row("assistant", "Here is the plan: three columns."),
        row("user", "Use two columns instead"),
        row("assistant", "Switched to two columns."),
    ]
}

#[test]
fn a_fork_on_another_model_sends_the_copied_conversation_with_its_first_turn() {
    let fixture = fixture_dir("fork");
    let source = Settings {
        model: Some("gpt-source".into()),
        ..Default::default()
    };
    let chat = footer_chat();
    // Fork at the first answer, then pick another model before sending.
    let fork = gespan_core::response::fork_at(&chat, 1);
    let mut settings =
        gespan_core::response::carried_settings(&source, &Default::default(), fork.rows.len());
    settings.model = Some("gpt-switched".into());
    let (store, session, requests) = first_turn_after_transfer(&fixture, settings, &fork.rows);
    let turn = request(&requests, "turn/start").to_string();
    assert!(turn.contains("Plan the footer"), "{turn}");
    assert!(turn.contains("Here is the plan: three columns."), "{turn}");
    assert!(turn.contains("Current user message:\\nCarry on"), "{turn}");
    // The fork point drops everything after it.
    assert!(!turn.contains("two columns"), "{turn}");
    assert!(
        requests
            .iter()
            .any(|value| value["params"]["model"] == "gpt-switched"),
        "the fork runs on the model picked for it: {requests:?}"
    );
    let saved = locked(&store).unwrap().load(session).unwrap().rows;
    assert_eq!(saved[..2], chat[..2]);
    assert!(saved.iter().any(|row| row.text == "Answer"));
}

#[test]
fn a_handoff_session_shows_the_chat_and_sends_its_recap_once() {
    let fixture = fixture_dir("handoff");
    let chat = footer_chat();
    let settings = Settings {
        handoff: Some(gespan_core::agent::Handoff {
            from: "claude".into(),
            brief: gespan_core::response::handoff(&chat),
        }),
        ..Default::default()
    };
    let (store, session, requests) = first_turn_after_transfer(&fixture, settings, &chat);
    let turn = request(&requests, "turn/start").to_string();
    assert!(turn.contains("handed off from Claude"), "{turn}");
    assert!(turn.contains("<handoff>"), "{turn}");
    assert!(turn.contains("Use two columns instead"), "{turn}");
    let store = locked(&store).unwrap();
    // The new chat opens on the conversation it continues.
    assert_eq!(store.load(session).unwrap().rows[..chat.len()], chat[..]);
    // A consumed recap is not resent on later turns.
    assert!(
        store
            .agent_settings(session)
            .unwrap()
            .unwrap()
            .handoff
            .is_none()
    );
}

#[test]
fn a_handoff_from_an_empty_chat_tells_the_agent_there_is_no_prior_work() {
    let fixture = fixture_dir("empty-handoff");
    let settings = Settings {
        handoff: Some(gespan_core::agent::Handoff {
            from: "claude".into(),
            brief: gespan_core::response::handoff(&[]),
        }),
        ..Default::default()
    };
    let (_, _, requests) = first_turn_after_transfer(&fixture, settings, &[]);
    let turn = request(&requests, "turn/start").to_string();
    assert!(turn.contains("Do not invent prior work"), "{turn}");
    assert!(!turn.contains("<handoff>"), "{turn}");
}
