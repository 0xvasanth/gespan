//! A turn that ends signed out, then its in-place replay after sign-in.
use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A mock Codex app-server: signed out while `signed-out` exists in its
/// folder, otherwise it answers "Done". Every `turn/start` is logged.
fn fixture(name: &str) -> (Fixture, PathBuf) {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "gespan-sign-in-{name}-{}-{}",
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
import json, os, sys
here = os.path.dirname(sys.argv[0])
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({'id': 1, 'result': {}})
    elif value.get('id') == 2:
        send({'id': 2, 'result': {'thread': {'id': 'sign-in-thread'}}})
    elif value.get('id') == 3:
        with open(os.path.join(here, 'turns'), 'a') as log:
            log.write(json.dumps(value['params'].get('input')) + '\n')
        if os.path.exists(os.path.join(here, 'signed-out')):
            error = {'message': 'Your access token could not be refreshed. Please log out and sign in again.', 'codexErrorInfo': 'unauthorized'}
            send({'method': 'error', 'params': {'error': error, 'willRetry': False}})
        else:
            send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'reply', 'delta': 'Done'}})
            send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#,
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    (fixture, binary)
}

struct Ended {
    signed_out: bool,
    status: String,
    shown: Vec<Row>,
}

fn run(
    store: &Database,
    summary: Session,
    binary: &std::path::Path,
    settings: Settings,
    shown: &mut Vec<Row>,
) -> Ended {
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker_store = store.clone();
    let binary = binary.to_owned();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Fix the login".into(),
                settings,
            },
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    loop {
        let update = receive.recv_blocking().expect("runtime closed early");
        shown.truncate(update.start);
        shown.extend(update.rows);
        if update.finished {
            worker.join().unwrap();
            return Ended {
                signed_out: update.signed_out,
                status: update.status,
                shown: shown.clone(),
            };
        }
    }
}

fn roles(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|row| row.role.as_str()).collect()
}

fn replays_in_place(shared: bool) {
    let (fixture, binary) = fixture(if shared { "shared" } else { "native" });
    std::fs::write(fixture.0.join("signed-out"), "").unwrap();
    let path = fixture.0.join("sessions.db");
    let store = if shared {
        Store::open_gespan(&path)
    } else {
        Store::open(&path)
    }
    .unwrap();
    let summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    let session = summary.id;
    let store: Database = Arc::new(Mutex::new(store));
    let mut shown = Vec::new();

    let failed = run(
        &store,
        summary.clone(),
        &binary,
        Settings::default(),
        &mut shown,
    );
    assert!(failed.signed_out);
    assert!(
        failed
            .status
            .starts_with("Signed out of Codex. Your access token"),
        "{}",
        failed.status
    );
    let saved = locked(&store).unwrap().load(session).unwrap().rows;
    assert_eq!(roles(&saved), ["user", "assistant", "error"]);
    assert_eq!(
        failed_turn(&saved),
        Some(FailedTurn {
            prompt: 0,
            reply: 1
        })
    );

    // Signed in again: the same prompt runs once more, and its reply takes
    // the error's place.
    std::fs::remove_file(fixture.0.join("signed-out")).unwrap();
    let replayed = run(
        &store,
        summary,
        &binary,
        Settings {
            replay_failed_turn: true,
            ..Settings::default()
        },
        &mut shown,
    );
    assert!(!replayed.signed_out);
    assert_eq!(replayed.status, "Ready");
    let saved = locked(&store).unwrap().load(session).unwrap().rows;
    assert_eq!(roles(&saved), ["user", "assistant"]);
    assert_eq!(saved[0].text, "Fix the login");
    assert_eq!(saved[1].text, "Done");
    assert_eq!(
        replayed.shown, saved,
        "the view gets exactly what was saved"
    );
    let turns = std::fs::read_to_string(fixture.0.join("turns")).unwrap();
    assert_eq!(turns.lines().count(), 2, "one failed send, one replay");
    assert!(turns.lines().all(|turn| turn.contains("Fix the login")));
}

#[test]
fn a_signed_out_turn_replays_in_place_in_the_shared_store() {
    replays_in_place(true);
}

#[test]
fn a_signed_out_turn_replays_in_place_in_the_native_store() {
    replays_in_place(false);
}

#[test]
fn a_replay_refuses_a_conversation_that_moved_on() {
    let (fixture, binary) = fixture("moved-on");
    let store = Store::open_gespan(&fixture.0.join("sessions.db")).unwrap();
    let summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    let session = summary.id;
    let store: Database = Arc::new(Mutex::new(store));
    let mut shown = Vec::new();
    // The last turn succeeded, so there is nothing to replay.
    run(
        &store,
        summary.clone(),
        &binary,
        Settings::default(),
        &mut shown,
    );
    let before = locked(&store).unwrap().load(session).unwrap().rows;
    let refused = run(
        &store,
        summary,
        &binary,
        Settings {
            replay_failed_turn: true,
            ..Settings::default()
        },
        &mut shown,
    );
    assert!(
        refused.status.contains("Send it again"),
        "{}",
        refused.status
    );
    assert_eq!(locked(&store).unwrap().load(session).unwrap().rows, before);
    let turns = std::fs::read_to_string(fixture.0.join("turns")).unwrap();
    assert_eq!(turns.lines().count(), 1, "nothing was sent again");
}
