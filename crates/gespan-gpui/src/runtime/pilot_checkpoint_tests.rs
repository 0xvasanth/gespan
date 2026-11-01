use super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// The app-wide checkpoint store, in a temporary folder for this run.
fn checkpoints() -> &'static gespan_core::checkpoint::Store {
    if gespan_core::checkpoint::store().is_none() {
        let dir = tempfile::tempdir().unwrap().keep();
        gespan_core::checkpoint::init(dir.join("checkpoints"));
    }
    gespan_core::checkpoint::store().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A mock Codex that reports creating `file` in its folder, then writes it.
fn mock_provider(dir: &Path, file: &Path) -> std::path::PathBuf {
    let binary = dir.join("mock-provider");
    std::fs::write(
        &binary,
        format!(
            r#"#!/usr/bin/env python3
import json, sys, time
path = {path}
item = {{'type': 'fileChange', 'id': 'edit', 'status': 'inProgress', 'changes': [{{'path': path, 'kind': {{'type': 'add'}}, 'diff': 'made by the pilot'}}]}}
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({{'id': 1, 'result': {{}}}})
    elif value.get('id') == 2:
        send({{'id': 2, 'result': {{'thread': {{'id': 'pilot-checkpoint-test'}}}}}})
    elif value.get('id') == 3:
        send({{'method': 'item/started', 'params': {{'item': item}}}})
        time.sleep(0.3)
        open(path, 'w').write('made by the pilot\n')
        send({{'method': 'item/completed', 'params': {{'item': dict(item, status='completed')}}}})
        send({{'method': 'item/agentMessage/delta', 'params': {{'itemId': 'reply', 'delta': 'Done'}}}})
        send({{'method': 'turn/completed', 'params': {{'turn': {{'status': 'completed'}}}}}})
        break
"#,
            path = serde_json::to_string(file.to_str().unwrap()).unwrap(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    binary
}

/// Runs one Pilot turn from `chat`, editing `project`, and returns the files
/// its session recorded.
fn pilot_turn(fixture: &Path, chat: &Path, project: &Path) -> Vec<String> {
    let binary = mock_provider(fixture, &project.join("made.txt"));
    let store = Store::open_gespan(&fixture.join("sessions.db")).unwrap();
    let cwd = chat.to_str().unwrap().to_owned();
    let summary = store.create("codex", &cwd).unwrap();
    let session = summary.id.to_string();
    let checkpoints = checkpoints();
    checkpoints.forget(&session).unwrap();
    let mut pilots = gespan_core::pilot::Roster::default();
    let pilot = pilots
        .create(vec![project.to_str().unwrap().to_owned()])
        .clone();
    let settings = Settings {
        pilot: Some(gespan_core::agent::PilotContext {
            pilot,
            root: fixture.join("pilots"),
            executable: "/bin/true".into(),
        }),
        ..Settings::default()
    };
    let store = Arc::new(Mutex::new(store));
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Make a file".into(),
                settings,
            },
            control_rx,
            store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    loop {
        let update = receive.recv_blocking().expect("runtime closed early");
        if update.finished {
            assert_eq!(update.status, "Ready");
            break;
        }
    }
    worker.join().unwrap();
    let status = checkpoints
        .exclusive(|store| store.status_readonly(&session, &cwd))
        .unwrap();
    checkpoints.forget(&session).unwrap();
    status.files.into_iter().map(|file| file.relative).collect()
}

#[test]
fn a_pilot_turn_in_a_git_project_records_what_it_changed() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["config", "user.email", "demo@example.com"]);
    git(&project, &["config", "user.name", "Demo"]);
    std::fs::write(project.join("kept.txt"), "kept\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "First"]);

    assert_eq!(pilot_turn(fixture.path(), &project, &project), ["made.txt"]);
}

/// A new Pilot's chat runs in the home folder and edits projects inside it.
#[test]
fn a_pilot_chat_in_a_plain_home_records_edits_in_its_projects() {
    let fixture = tempfile::tempdir().unwrap();
    let home = fixture.path().join("home");
    let project = home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("kept.txt"), "kept\n").unwrap();

    assert_eq!(
        pilot_turn(fixture.path(), &home, &project),
        ["project/made.txt"]
    );
}
