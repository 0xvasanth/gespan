//! Turns taken in the agent's own CLI between Gespan sends: picked up before
//! the next prompt, behind a "From terminal" marker, and never Gespan's own.
use super::*;
use gespan_core::outside_turns::{Checkpoint, checkpoint_key};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

const THREAD: &str = "sync-thread";

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A mock Codex app-server that writes its rollout under `CODEX_HOME` the way
/// Codex does, and fails a turn while `fail` exists in its folder.
const MOCK: &str = r#"#!/usr/bin/env python3
import json, os, sys
here = os.path.dirname(sys.argv[0])
home = os.environ['CODEX_HOME']
rollout = os.path.join(home, 'sessions', '2026', '11', '11', 'rollout-2026-11-11T00-00-00-sync-thread.jsonl')
def send(value):
    print(json.dumps(value), flush=True)
def record(kind, message):
    try:
        os.makedirs(os.path.dirname(rollout), exist_ok=True)
        with open(rollout, 'a') as out:
            out.write(json.dumps({'type': 'event_msg', 'payload': {'type': kind, 'message': message}}) + '\n')
    except OSError:
        pass
for line in sys.stdin:
    value = json.loads(line)
    method = value.get('method')
    if method == 'initialize':
        send({'id': value['id'], 'result': {}})
    elif method in ('thread/start', 'thread/resume'):
        send({'id': value['id'], 'result': {'thread': {'id': 'sync-thread'}}})
    elif method == 'turn/start':
        if os.path.exists(os.path.join(here, 'fail')):
            send({'method': 'error', 'params': {'error': {'message': 'The model is overloaded.'}, 'willRetry': False}})
            break
        text = ''.join(item.get('text', '') for item in value['params'].get('input', []))
        record('user_message', text)
        record('agent_message', 'Done')
        send({'id': value['id'], 'result': {'turn': {'id': 'turn'}}})
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'reply', 'delta': 'Done'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#;

struct Conversation {
    fixture: Fixture,
    store: Database,
    summary: Session,
    account: gespan_core::accounts::Profile,
    binary: PathBuf,
    /// What the conversation view holds, patched the way `TranscriptView` is.
    view: Vec<Row>,
}

impl Conversation {
    fn new(name: &str, shared: bool) -> Self {
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "gespan-outside-sync-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
        std::fs::create_dir_all(&fixture.0).unwrap();
        let binary = fixture.0.join("mock-provider");
        std::fs::write(&binary, MOCK).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = fixture.0.join("sessions.db");
        let store = if shared {
            Store::open_gespan(&path)
        } else {
            Store::open(&path)
        }
        .unwrap();
        // A non-default account keeps the agent's home inside the fixture.
        let mut book = store.provider_accounts().unwrap();
        let id = book.add(Kind::Codex, "Work").unwrap().id;
        store.save_provider_accounts(&book).unwrap();
        let account = gespan_core::accounts::Profile::new(&id, &fixture.0);
        let summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
        Self {
            fixture,
            store: Arc::new(Mutex::new(store)),
            summary,
            account,
            binary,
            view: Vec::new(),
        }
    }

    fn home(&self) -> PathBuf {
        self.account.home(Kind::Codex).unwrap()
    }

    fn rollout(&self) -> PathBuf {
        self.home()
            .join("sessions/2026/11/11/rollout-2026-11-11T00-00-00-sync-thread.jsonl")
    }

    fn settings(&self) -> Settings {
        Settings {
            account: self.account.clone(),
            ..Settings::default()
        }
    }

    /// What `codex resume` appends when the user continues in a terminal.
    fn continue_in_terminal(&self, prompt: &str, reply: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.rollout())
            .unwrap();
        for (kind, message) in [("user_message", prompt), ("agent_message", reply)] {
            let entry =
                serde_json::json!({"type":"event_msg","payload":{"type":kind,"message":message}});
            writeln!(file, "{entry}").unwrap();
        }
    }

    fn send(&mut self, prompt: &str) -> String {
        let settings = self.settings();
        self.send_with(prompt, settings)
    }

    /// One turn as the UI runs it: the prompt is painted locally at the old
    /// end first, then every update is patched in.
    fn send_with(&mut self, prompt: &str, settings: Settings) -> String {
        if settings.replay_failed_turn {
            // The reply is written again where the failed one was.
            if let Some(failed) = failed_turn(&self.view) {
                self.view.truncate(failed.reply);
            }
        } else {
            self.view.push(Row {
                role: "user".into(),
                text: prompt.into(),
                first: true,
                sent_at: None,
            });
        }
        let (updates, receive) = async_channel::bounded(64);
        let (_controls, control_rx) = std::sync::mpsc::channel();
        let store = self.store.clone();
        let binary = self.binary.clone();
        let summary = self.summary.clone();
        let prompt = prompt.to_owned();
        let worker = std::thread::spawn(move || {
            run_turn_using(
                TurnInput {
                    summary,
                    prompt,
                    settings,
                },
                control_rx,
                store,
                Arc::new(AtomicBool::new(false)),
                updates,
                |_| Some(binary),
            );
        });
        let status = loop {
            let update = receive.recv_blocking().expect("runtime closed early");
            assert!(
                update.start <= self.view.len(),
                "transcript patches must be contiguous: start {} past {} rows",
                update.start,
                self.view.len()
            );
            for (ix, row) in update.rows.into_iter().enumerate() {
                match self.view.get_mut(update.start + ix) {
                    Some(old) => *old = row,
                    None => self.view.push(row),
                }
            }
            if update.finished {
                break update.status;
            }
        };
        worker.join().unwrap();
        // The next send resumes the thread, as the reloaded session would.
        self.summary.provider_id = Some(THREAD.into());
        status
    }

    fn rows(&self) -> Vec<Row> {
        locked(&self.store)
            .unwrap()
            .load(self.summary.id)
            .unwrap()
            .rows
    }

    fn checkpoint(&self) -> Option<Checkpoint> {
        locked(&self.store)
            .unwrap()
            .setting(&checkpoint_key(self.summary.id))
            .unwrap()
            .map(|value| serde_json::from_str(&value).unwrap())
    }

    fn target(&self) -> outside_sync::Target {
        outside_sync::Target::of(&self.store, &self.summary, &self.account.id)
            .expect("a local Codex session syncs")
    }

    /// The pre-turn sync alone, on the stored rows.
    fn sync_now(&self) -> (outside_sync::Synced, Transcript) {
        let mut transcript = locked(&self.store).unwrap().load(self.summary.id).unwrap();
        let turn = outside_sync::Turn {
            session: self.summary.id,
            provider_id: THREAD,
            account: &self.account.id,
            cwd: std::path::Path::new(&self.summary.cwd),
        };
        let synced = outside_sync::before_turn(&self.store, &self.target(), &turn, &mut transcript);
        (synced, transcript)
    }
}

fn shape(rows: &[Row]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| {
            let text = if row.role == "interjection" {
                serde_json::from_str::<gespan_core::transcript::Interjection>(&row.text)
                    .unwrap()
                    .label()
                    .to_owned()
            } else {
                row.text.clone()
            };
            (row.role.clone(), text)
        })
        .collect()
}

fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(role, text)| (role.to_string(), text.to_string()))
        .collect()
}

fn terminal_turns_show_before_the_new_prompt(shared: bool) {
    let mut chat = Conversation::new(if shared { "shown-shared" } else { "shown" }, shared);
    assert_eq!(chat.send("Fix the login"), "Ready");
    chat.continue_in_terminal("ran it in the terminal", "ok");

    assert_eq!(chat.send("Now add a test"), "Ready");
    let rows = chat.rows();
    assert_eq!(
        shape(&rows),
        pairs(&[
            ("user", "Fix the login"),
            ("assistant", "Done"),
            ("interjection", "From terminal"),
            ("user", "ran it in the terminal"),
            ("assistant", "ok"),
            ("user", "Now add a test"),
            ("assistant", "Done"),
        ])
    );
    assert_eq!(chat.view, rows, "the view shows exactly what was saved");

    // Picked up once: the next send adds only its own turn.
    assert_eq!(chat.send("And the docs"), "Ready");
    let rows = chat.rows();
    assert_eq!(rows.len(), 9);
    assert_eq!(
        rows.iter().filter(|row| row.role == "interjection").count(),
        1
    );
    assert_eq!(chat.view, rows);
}

#[test]
fn terminal_turns_show_before_the_new_prompt_with_the_marker() {
    terminal_turns_show_before_the_new_prompt(false);
}

#[test]
fn terminal_turns_show_before_the_new_prompt_in_the_shared_store() {
    terminal_turns_show_before_the_new_prompt(true);
}

#[test]
fn own_turn_is_never_imported_after_it_completes() {
    let mut chat = Conversation::new("own", true);
    chat.send("Fix the login");
    let first = chat
        .checkpoint()
        .expect("a finished turn records a checkpoint");
    assert_eq!(first.path, chat.rollout());
    assert_eq!(first.len, std::fs::metadata(chat.rollout()).unwrap().len());

    chat.send("Now add a test");
    let rows = chat.rows();
    assert_eq!(
        shape(&rows),
        pairs(&[
            ("user", "Fix the login"),
            ("assistant", "Done"),
            ("user", "Now add a test"),
            ("assistant", "Done"),
        ])
    );
    assert_eq!(chat.view, rows);
    assert_eq!(
        chat.checkpoint().unwrap().len,
        std::fs::metadata(chat.rollout()).unwrap().len(),
        "the second turn's own entries are behind the checkpoint too"
    );
}

#[test]
fn growth_drops_the_warm_agent_and_none_keeps_it() {
    let mut chat = Conversation::new("growth", true);
    chat.send("Fix the login");

    let (synced, transcript) = chat.sync_now();
    assert_eq!(synced, outside_sync::Synced::default(), "nothing outside");
    assert_eq!(transcript.rows.len(), 2);

    chat.continue_in_terminal("ran it in the terminal", "ok");
    let (synced, transcript) = chat.sync_now();
    assert_eq!(
        synced,
        outside_sync::Synced {
            picked: Some(2),
            forgot: true
        }
    );
    assert_eq!(transcript.rows.len(), 5);
    assert_eq!(chat.rows(), transcript.rows, "picked-up rows are saved");

    // A terminal turn with no visible text still makes the warm agent stale.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(chat.rollout())
        .unwrap();
    let tool_only = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[]}});
    writeln!(file, "{tool_only}").unwrap();
    drop(file);
    let (synced, transcript) = chat.sync_now();
    assert_eq!(
        synced,
        outside_sync::Synced {
            picked: None,
            forgot: true
        }
    );
    assert_eq!(transcript.rows.len(), 5, "nothing to show");

    let (synced, _) = chat.sync_now();
    assert_eq!(synced, outside_sync::Synced::default(), "read once");
}

#[test]
fn replay_does_not_sync() {
    let mut chat = Conversation::new("replay", true);
    chat.send("Fix the login");
    std::fs::write(chat.fixture.0.join("fail"), "").unwrap();
    let failed = chat.send("Now add a test");
    assert_ne!(failed, "Ready");
    assert_eq!(
        chat.rows()
            .iter()
            .map(|row| row.role.as_str())
            .collect::<Vec<_>>(),
        ["user", "assistant", "user", "assistant", "error"]
    );
    chat.continue_in_terminal("ran it in the terminal", "ok");
    std::fs::remove_file(chat.fixture.0.join("fail")).unwrap();

    let settings = Settings {
        replay_failed_turn: true,
        ..chat.settings()
    };
    assert_eq!(chat.send_with("Now add a test", settings), "Ready");
    let rows = chat.rows();
    assert_eq!(
        shape(&rows),
        pairs(&[
            ("user", "Fix the login"),
            ("assistant", "Done"),
            ("user", "Now add a test"),
            ("assistant", "Done"),
        ])
    );
    assert_eq!(chat.view, rows);
    assert_eq!(
        chat.checkpoint().unwrap().len,
        std::fs::metadata(chat.rollout()).unwrap().len(),
        "the replay still moves the checkpoint past its own turn"
    );
}

#[test]
fn a_broken_session_file_never_fails_the_send() {
    use std::io::Write;
    let mut chat = Conversation::new("broken", true);
    chat.send("Fix the login");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(chat.rollout())
        .unwrap();
    file.write_all(b"not json at all\n{\"type\":\"event_msg\",\"payload\":\n")
        .unwrap();
    drop(file);
    assert_eq!(chat.send("Now add a test"), "Ready");
    assert_eq!(chat.rows().len(), 4, "garbage imports nothing");

    // An unreadable file: the send goes through and the checkpoint is kept.
    let before = chat.checkpoint();
    std::fs::set_permissions(chat.rollout(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::File::open(chat.rollout()).is_ok();
    let status = chat.send("And the docs");
    std::fs::set_permissions(chat.rollout(), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(status, "Ready");
    assert_eq!(chat.rows().len(), 6);
    assert_eq!(chat.view, chat.rows());
    if !readable {
        // Root reads anything; elsewhere the failed read keeps what we had.
        assert_eq!(chat.checkpoint(), before);
    }
}

#[test]
fn a_session_without_a_checkpoint_places_itself_by_its_last_prompt() {
    let mut chat = Conversation::new("no-checkpoint", true);
    chat.send("Fix the login");
    chat.continue_in_terminal("ran it in the terminal", "ok");
    // As if the session ran before checkpoints existed.
    locked(&chat.store)
        .unwrap()
        .save_setting(&checkpoint_key(chat.summary.id), "not a checkpoint")
        .unwrap();
    assert_eq!(chat.send("Now add a test"), "Ready");
    assert_eq!(
        shape(&chat.rows()),
        pairs(&[
            ("user", "Fix the login"),
            ("assistant", "Done"),
            ("interjection", "From terminal"),
            ("user", "ran it in the terminal"),
            ("assistant", "ok"),
            ("user", "Now add a test"),
            ("assistant", "Done"),
        ])
    );
}

#[test]
fn remote_and_unsupported_sessions_never_sync() {
    let chat = Conversation::new("targets", true);
    let mut remote = chat.summary.clone();
    remote.cwd = "remote://machine/work/gespan".into();
    assert!(outside_sync::Target::of(&chat.store, &remote, &chat.account.id).is_none());
    let mut cursor = chat.summary.clone();
    cursor.provider = "cursor".into();
    assert!(outside_sync::Target::of(&chat.store, &cursor, &chat.account.id).is_none());
    let target = chat.target();
    assert_eq!(target.kind, Kind::Codex);
    assert_eq!(target.home, chat.home());
}
