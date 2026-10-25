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
/// a ticket capability, calls ticket_get the way the stdio server would.
const PROVIDER: &str = r#"#!/usr/bin/env python3
import json, os, socket, sys
def send(value):
    print(json.dumps(value), flush=True)
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
            address, port = os.environ['GESPAN_TICKET_ENDPOINT'].rsplit(':', 1)
            with socket.create_connection((address, int(port)), timeout=10) as connection:
                connection.settimeout(10)
                request = {
                    'session': int(os.environ['GESPAN_TICKET_SESSION']),
                    'token': os.environ['GESPAN_TICKET_TOKEN'],
                    'key': 'ENG-1',
                    'tool': json.loads(os.environ.get('MOCK_TICKET_TOOL', '{"tool": "get"}')),
                }
                connection.sendall((json.dumps(request) + '\n').encode())
                record['reply'] = json.loads(connection.makefile('rb').readline())
        with open('record.json', 'w') as output:
            json.dump(record, output)
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'a', 'delta': 'Done.'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#;

fn run(ticket: Option<TicketRef>, settings: Settings) -> (serde_json::Value, bool, String) {
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
    let store = Store::open_gespan(&fixture.0.join("sessions.db")).unwrap();
    let mut summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    if let Some(ticket) = ticket {
        let item = WorkItem::Ticket(Box::new(ticket));
        store.set_work_item(summary.id, Some(&item)).unwrap();
        summary.work_item = Some(item);
    }
    let store = Arc::new(Mutex::new(store));
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Fix it".into(),
                settings,
            },
            control_rx,
            store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    let mut ticket_changed = false;
    let status = loop {
        let update = receive.recv_blocking().unwrap();
        ticket_changed |= update.ticket.is_some();
        if update.finished {
            break update.status;
        }
    };
    worker.join().unwrap();
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
