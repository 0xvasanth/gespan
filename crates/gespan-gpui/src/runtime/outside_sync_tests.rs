//! Turns taken in the agent's own CLI between Gespan sends: picked up before
//! the next prompt, behind a "From terminal" marker, and never Gespan's own.
use super::*;
use gespan_core::outside_turns::{Checkpoint, checkpoint_key};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

const THREAD: &str = "sync-thread";
const CLAUDE_SESSION: &str = "sync-session";

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

/// A mock Claude Code (`stream-json`) that writes its session file under
/// `CLAUDE_CONFIG_DIR`, chaining each entry to the last by `parentUuid`.
const CLAUDE_MOCK: &str = r#"#!/usr/bin/env python3
import json, os, sys, uuid
home = os.environ['CLAUDE_CONFIG_DIR']
path = os.path.join(home, 'projects', '-work-app', 'sync-session.jsonl')
def send(value):
    print(json.dumps(value), flush=True)
def last_uuid():
    try:
        lines = [l for l in open(path).read().splitlines() if l.strip()]
        return json.loads(lines[-1]).get('uuid') if lines else None
    except (OSError, ValueError):
        return None
def record(role, content):
    entry = {'type': role, 'uuid': uuid.uuid4().hex, 'parentUuid': last_uuid(), 'isSidechain': False,
             'message': {'role': role, 'content': content}}
    if role == 'assistant':
        entry['message']['id'] = 'msg-' + entry['uuid']
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'a') as out:
        out.write(json.dumps(entry) + '\n')
for line in sys.stdin:
    value = json.loads(line)
    if value.get('type') == 'control_request':
        send({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': value['request_id']}})
    elif value.get('type') == 'user':
        content = value['message']['content']
        if not isinstance(content, str):
            content = ''.join(b.get('text', '') for b in content if b.get('type') == 'text')
        send({'type': 'system', 'subtype': 'init', 'session_id': 'sync-session'})
        record('user', content)
        record('assistant', [{'type': 'text', 'text': 'Done'}])
        send({'type': 'assistant', 'message': {'id': 'm1', 'role': 'assistant', 'content': [{'type': 'text', 'text': 'Done'}]}})
        send({'type': 'result', 'subtype': 'success', 'result': 'Done', 'session_id': 'sync-session'})
        break
"#;

struct Conversation {
    fixture: Fixture,
    kind: Kind,
    store: Database,
    summary: Session,
    account: gespan_core::accounts::Profile,
    binary: PathBuf,
    /// What the conversation view holds, patched the way `TranscriptView` is.
    view: Vec<Row>,
}

impl Conversation {
    fn new(name: &str, shared: bool) -> Self {
        Self::of(Kind::Codex, name, shared)
    }

    fn of(kind: Kind, name: &str, shared: bool) -> Self {
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
        let mock = if kind == Kind::Claude {
            CLAUDE_MOCK
        } else {
            MOCK
        };
        std::fs::write(&binary, mock).unwrap();
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
        let id = book.add(kind, "Work").unwrap().id;
        store.save_provider_accounts(&book).unwrap();
        let account = gespan_core::accounts::Profile::new(&id, &fixture.0);
        let summary = store
            .create(kind.id(), fixture.0.to_str().unwrap())
            .unwrap();
        Self {
            fixture,
            kind,
            store: Arc::new(Mutex::new(store)),
            summary,
            account,
            binary,
            view: Vec::new(),
        }
    }

    fn home(&self) -> PathBuf {
        self.account.home(self.kind).unwrap()
    }

    fn provider_id(&self) -> &'static str {
        if self.kind == Kind::Claude {
            CLAUDE_SESSION
        } else {
            THREAD
        }
    }

    /// The agent's own session file.
    fn rollout(&self) -> PathBuf {
        if self.kind == Kind::Claude {
            self.home()
                .join(format!("projects/-work-app/{CLAUDE_SESSION}.jsonl"))
        } else {
            self.home()
                .join("sessions/2026/11/11/rollout-2026-11-11T00-00-00-sync-thread.jsonl")
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            account: self.account.clone(),
            // The visual bridge is keyed by session id, which parallel
            // fixtures share.
            text_only: true,
            ..Settings::default()
        }
    }

    /// What `codex resume` or `claude --resume` appends when the user
    /// continues in a terminal.
    fn continue_in_terminal(&self, prompt: &str, reply: &str) {
        use std::io::Write;
        if self.kind == Kind::Claude {
            let text = std::fs::read_to_string(self.rollout()).unwrap();
            let last: serde_json::Value =
                serde_json::from_str(text.lines().last().unwrap()).unwrap();
            let parent = last["uuid"].as_str().unwrap().to_owned();
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(self.rollout())
                .unwrap();
            let n = text.lines().count();
            let (user, reply_id) = (format!("t-user-{n}"), format!("t-reply-{n}"));
            let asked = serde_json::json!({"type":"user","uuid":user,"parentUuid":parent,
                "isSidechain":false,"message":{"role":"user","content":prompt}});
            let answered = serde_json::json!({"type":"assistant","uuid":reply_id,"parentUuid":user,
                "isSidechain":false,"message":{"id":format!("t-msg-{n}"),"role":"assistant",
                "content":[{"type":"text","text":reply}]}});
            writeln!(file, "{asked}\n{answered}").unwrap();
            return;
        }
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
        self.summary.provider_id = Some(self.provider_id().into());
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
        outside_sync::Target::of(&self.store, &self.summary, &self.account.id, false)
            .expect("a local session syncs")
    }

    /// The pre-turn sync alone, on the stored rows, with every warm-agent
    /// drop recorded instead of made.
    fn sync_now(&self) -> (outside_sync::Synced, Transcript, Vec<Forgot>) {
        let (synced, transcript, forgot, _) = self.sync_while(false);
        (synced, transcript, forgot)
    }

    /// `sync_now` with Gespan's own agent for the session reported as still
    /// writing (held or settling) or not, and the bindings it was asked about.
    fn sync_while(
        &self,
        still_writing: bool,
    ) -> (outside_sync::Synced, Transcript, Vec<Forgot>, Vec<Forgot>) {
        let mut transcript = locked(&self.store).unwrap().load(self.summary.id).unwrap();
        let turn = outside_sync::Turn {
            session: self.summary.id,
            provider_id: self.provider_id(),
            account: &self.account.id,
            cwd: std::path::Path::new(&self.summary.cwd),
        };
        let mut forgot = Vec::new();
        let mut asked = Vec::new();
        let synced = outside_sync::before_turn_with(
            &self.store,
            &self.target(),
            &turn,
            &mut transcript,
            outside_sync::Effects {
                save_rows: |store: &Database, session, start, rows: &[Row]| {
                    locked(store)?.write_rows(session, start, rows)
                },
                forget: |kind, id: &str, account: &str, cwd: &std::path::Path| {
                    forgot.push((kind, id.to_owned(), account.to_owned(), cwd.to_owned()))
                },
                still_writing: |kind, id: &str, account: &str, cwd: &std::path::Path| {
                    asked.push((kind, id.to_owned(), account.to_owned(), cwd.to_owned()));
                    still_writing
                },
            },
        );
        (synced, transcript, forgot, asked)
    }

    /// Lines appended as the agent writes them, without a send.
    fn agent_writes(&self, entries: &[serde_json::Value]) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.rollout())
            .unwrap();
        for entry in entries {
            writeln!(file, "{entry}").unwrap();
        }
    }
}

/// A recorded `forget_session` call: kind, provider id, account, cwd.
type Forgot = (Kind, String, String, PathBuf);

fn codex_said(kind: &str, message: &str) -> serde_json::Value {
    serde_json::json!({"type":"event_msg","payload":{"type":kind,"message":message}})
}

fn shape(rows: &[Row]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| {
            let text = if row.role == "interjection" {
                serde_json::from_str::<gespan_core::transcript::Interjection>(&row.text)
                    .unwrap()
                    .label()
                    .into_owned()
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
    let warm = (
        Kind::Codex,
        THREAD.to_owned(),
        chat.account.id.clone(),
        PathBuf::from(&chat.summary.cwd),
    );

    let (synced, transcript, forgot) = chat.sync_now();
    assert_eq!(synced, outside_sync::Synced::default(), "nothing outside");
    assert!(forgot.is_empty(), "the warm agent is kept");
    assert_eq!(transcript.rows.len(), 2);

    chat.continue_in_terminal("ran it in the terminal", "ok");
    let (synced, transcript, forgot) = chat.sync_now();
    assert_eq!(
        synced,
        outside_sync::Synced {
            picked: Some(2),
            forgot: true,
            held: false,
        }
    );
    assert_eq!(
        forgot,
        std::slice::from_ref(&warm),
        "the agent keyed by this session goes"
    );
    assert_eq!(transcript.rows.len(), 5);
    assert_eq!(chat.rows(), transcript.rows, "picked-up rows are saved");

    // A terminal turn with no visible text still makes the warm agent stale.
    chat.agent_writes(&[serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[]}})]);
    let (synced, transcript, forgot) = chat.sync_now();
    assert_eq!(
        synced,
        outside_sync::Synced {
            picked: None,
            forgot: true,
            held: false,
        }
    );
    assert_eq!(forgot, [warm]);
    assert_eq!(transcript.rows.len(), 5, "nothing to show");

    let (synced, _, forgot) = chat.sync_now();
    assert_eq!(synced, outside_sync::Synced::default(), "read once");
    assert!(forgot.is_empty());
}

#[test]
fn own_last_turn_flushed_late_imports_nothing_and_keeps_the_warm_agent() {
    let mut chat = Conversation::new("late-flush", true);
    chat.send("Fix the login");
    // The agent wrote its own turn again after reporting it done.
    chat.agent_writes(&[
        codex_said("user_message", "Fix the login"),
        codex_said("agent_message", "Done"),
    ]);
    let (synced, transcript, forgot) = chat.sync_now();
    assert_eq!(synced, outside_sync::Synced::default());
    assert!(forgot.is_empty(), "nothing outside: the warm agent is kept");
    assert_eq!(transcript.rows.len(), 2);
    assert_eq!(
        chat.checkpoint().unwrap().len,
        std::fs::metadata(chat.rollout()).unwrap().len(),
        "the echo is behind the checkpoint now"
    );

    chat.agent_writes(&[codex_said("agent_message", "Done")]);
    assert_eq!(chat.send("Now add a test"), "Ready");
    assert_eq!(
        shape(&chat.rows()),
        pairs(&[
            ("user", "Fix the login"),
            ("assistant", "Done"),
            ("user", "Now add a test"),
            ("assistant", "Done"),
        ])
    );
}

#[test]
fn own_turn_flushed_late_then_a_terminal_turn_imports_only_the_terminal_turn() {
    let mut chat = Conversation::new("late-then-terminal", true);
    chat.send("Fix the login");
    chat.agent_writes(&[
        codex_said("user_message", "Fix the login"),
        codex_said("agent_message", "Done"),
    ]);
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
    assert_eq!(chat.view, rows);
}

/// After Stop the agent finishes its turn in the background, and Hold parks
/// one that may keep working: what they write is Gespan's own.
#[test]
fn writes_by_gespans_own_held_or_stopped_agent_are_not_imported() {
    let mut chat = Conversation::new("still-writing", true);
    chat.send("Fix the login");
    chat.agent_writes(&[
        codex_said("user_message", "<background task finished>"),
        codex_said("agent_message", "The build passed."),
    ]);
    let (synced, transcript, forgot, asked) = chat.sync_while(true);
    let binding = (
        Kind::Codex,
        THREAD.to_owned(),
        chat.account.id.clone(),
        PathBuf::from(&chat.summary.cwd),
    );
    assert_eq!(transcript.rows.len(), 2, "nothing imported");
    assert!(forgot.is_empty(), "the held agent and its work are kept");
    assert_eq!(synced, outside_sync::Synced::default());
    assert_eq!(
        asked,
        std::slice::from_ref(&binding),
        "asked about this agent"
    );
    assert_eq!(chat.rows().len(), 2);
    assert_eq!(
        chat.checkpoint().unwrap().len,
        std::fs::metadata(chat.rollout()).unwrap().len(),
        "its writes are behind the checkpoint"
    );
    // Once it is gone, they are not read back as outside either.
    let (synced, _, forgot, _) = chat.sync_while(false);
    assert_eq!(synced, outside_sync::Synced::default());
    assert!(forgot.is_empty());
}

#[test]
fn a_failed_save_still_drops_the_warm_agent() {
    let mut chat = Conversation::new("failed-save", true);
    chat.send("Fix the login");
    chat.continue_in_terminal("ran it in the terminal", "ok");
    let before = chat.checkpoint();
    let mut transcript = locked(&chat.store).unwrap().load(chat.summary.id).unwrap();
    let turn = outside_sync::Turn {
        session: chat.summary.id,
        provider_id: THREAD,
        account: &chat.account.id,
        cwd: std::path::Path::new(&chat.summary.cwd),
    };
    let mut forgot = 0;
    let synced = outside_sync::before_turn_with(
        &chat.store,
        &chat.target(),
        &turn,
        &mut transcript,
        outside_sync::Effects {
            save_rows: |_: &Database, _, _, _: &[Row]| Err("disk full".into()),
            forget: |_, _: &str, _: &str, _: &std::path::Path| forgot += 1,
            still_writing: |_, _: &str, _: &str, _: &std::path::Path| false,
        },
    );
    assert_eq!(
        synced,
        outside_sync::Synced {
            picked: None,
            forgot: true,
            held: true,
        }
    );
    assert_eq!(forgot, 1, "the file grew from outside either way");
    assert_eq!(transcript.rows.len(), 2, "unsaved rows are not shown");
    assert_eq!(chat.checkpoint(), before);
}

#[test]
fn unsaved_terminal_turns_are_read_again_on_the_next_send() {
    let mut chat = Conversation::new("unsaved", false);
    chat.send("Fix the login");
    chat.continue_in_terminal("ran it in the terminal", "ok");
    let before = chat.checkpoint();
    // The store refuses the marker row, as a full disk would.
    let db = rusqlite::Connection::open(chat.fixture.0.join("sessions.db")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER refuse_marker BEFORE UPDATE OF blocks_json ON sessions
         WHEN NEW.blocks_json LIKE '%interjection%'
         BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .unwrap();

    assert_eq!(chat.send("Now add a test"), "Ready", "the send still goes");
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
    assert_eq!(chat.checkpoint(), before, "neither checkpoint moved");

    db.execute_batch("DROP TRIGGER refuse_marker;").unwrap();
    assert_eq!(chat.send("And the docs"), "Ready");
    let rows = chat.rows();
    // Gespan's own second turn is in the re-read too; it is dropped as an
    // echo, so the terminal turns show after it.
    assert_eq!(
        shape(&rows),
        pairs(&[
            ("user", "Fix the login"),
            ("assistant", "Done"),
            ("user", "Now add a test"),
            ("assistant", "Done"),
            ("interjection", "From terminal"),
            ("user", "ran it in the terminal"),
            ("assistant", "ok"),
            ("user", "And the docs"),
            ("assistant", "Done"),
        ])
    );
    assert_eq!(chat.view, rows);
}

#[test]
fn claude_terminal_turns_show_once_before_the_new_prompt() {
    let mut chat = Conversation::of(Kind::Claude, "claude", true);
    assert_eq!(chat.send("Fix the login"), "Ready");
    assert_eq!(
        chat.checkpoint().map(|cp| cp.path),
        Some(chat.rollout()),
        "the account's own Claude home is read"
    );
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
    assert_eq!(chat.view, rows);

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
    assert!(outside_sync::Target::of(&chat.store, &remote, &chat.account.id, false).is_none());
    let mut cursor = chat.summary.clone();
    cursor.provider = "cursor".into();
    assert!(outside_sync::Target::of(&chat.store, &cursor, &chat.account.id, false).is_none());
    let target = chat.target();
    assert_eq!(target.kind, Kind::Codex);
    assert_eq!(target.home, chat.home());
}

/// A Pilot's Codex keeps its rollouts under `codex-pilots`, not the
/// account's home, so searching the account's home would find nothing on
/// every send.
#[test]
fn pilot_codex_sessions_do_not_sync() {
    let chat = Conversation::new("pilot-codex", true);
    assert!(outside_sync::Target::of(&chat.store, &chat.summary, &chat.account.id, true).is_none());
    let claude = Conversation::of(Kind::Claude, "pilot-claude", true);
    let target = outside_sync::Target::of(&claude.store, &claude.summary, &claude.account.id, true)
        .expect("a Pilot's Claude reads the account's own home");
    assert_eq!(target.home, claude.home());
}
