//! End-to-end test harness: a real Shell and Conversation in a GPUI
//! `TestAppContext`, each session's turns run through a real provider
//! process. See `docs/testing/e2e.md`.
//!
//! In [`Mode::Fake`] that process is `gespan-fake-agent`, a stand-in `claude`
//! that answers from a script and records every frame it is sent. In
//! [`Mode::Real`] (`GESPAN_E2E_REAL=1`) the same binary passes everything
//! through to the real CLI and still records what Gespan sent, so
//! [`E2e::prompts`] works in both modes.
//!
//! Each session's agent lives in a folder of its own, named in the session's
//! store (see [`agent_binary`]), so tests running in parallel share nothing.
use crate::{
    conversation::Conversation,
    runtime::{Database, locked},
    shell::Shell,
    workspace::Workspace,
};
use gespan_core::{provider::Kind, store::Store, transcript::Row};
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

mod git_polish_tests;
mod hello_tests;
mod message_tests;
mod pilot_live_tests;
mod rail_tests;
mod remote_tests;
mod roster_tests;
mod start_tests;

/// The store setting that points a session's turns at a test agent.
fn binary_key(session: i64) -> String {
    format!("e2e:agent-binary:{session}")
}

/// The store setting naming the folder that holds every session's agent.
const ROOT_KEY: &str = "e2e:agents";

/// The agent an end-to-end test chose for this session, if any. The runtime
/// asks this before resolving the provider's CLI the normal way. A session
/// the test didn't start itself, such as one an agent started through
/// `sessions_start`, gets an agent of its own here, scripted with the next
/// entry of [`E2e::script_next_child`].
pub(crate) fn agent_binary(store: &Database, session: i64) -> Option<PathBuf> {
    let db = locked(store).ok()?;
    if let Some(binary) = db.setting(&binary_key(session)).ok().flatten() {
        return Some(PathBuf::from(binary));
    }
    let root = PathBuf::from(db.setting(ROOT_KEY).ok().flatten()?);
    let harness = Kind::from_id(&db.pilot_session(session).ok()??.provider);
    let binary = provision(&root, session, harness);
    let children = root.join("children.json");
    let mut queue: Vec<Value> = std::fs::read_to_string(&children)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if !queue.is_empty() {
        let script = queue.remove(0);
        std::fs::write(
            root.join(session.to_string()).join("replies.json"),
            serde_json::to_vec(&script).unwrap(),
        )
        .unwrap();
        std::fs::write(&children, serde_json::to_vec(&queue).unwrap()).unwrap();
    }
    db.save_setting(&binary_key(session), &binary.to_string_lossy())
        .ok()?;
    Some(binary)
}

/// The store setting pointing one-off background turns (Polish) at a test
/// agent.
const BACKGROUND_KEY: &str = "e2e:background-agent";

/// The agent an end-to-end test chose for one-off background turns, such as
/// Polish, which run without a session. The app asks this before resolving
/// the provider's CLI the normal way.
pub(crate) fn background_agent(store: &Database) -> Option<PathBuf> {
    let db = locked(store).ok()?;
    db.setting(BACKGROUND_KEY).ok().flatten().map(PathBuf::from)
}

/// A folder of its own for `session`'s agent, holding a copy of the fake
/// agent and, in real mode, the `proxy` file naming the real CLI.
fn provision(root: &Path, session: i64, harness: Kind) -> PathBuf {
    provision_in(root, &session.to_string(), harness)
}
fn provision_in(root: &Path, name: &str, harness: Kind) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let binary = dir.join(format!(
        "{}{}",
        harness.binary_name(),
        std::env::consts::EXE_SUFFIX
    ));
    // Another test process's cargo may be relinking the agent this moment.
    let mut tries = 0;
    while let Err(error) = std::fs::copy(fake_agent(), &binary) {
        tries += 1;
        assert!(tries < 100, "copying the fake agent: {error}");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if let Ok(real) = std::fs::read(root.join("proxy")) {
        std::fs::write(dir.join("proxy"), real).unwrap();
    }
    binary
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The scripted `gespan-fake-agent`. Deterministic; what CI runs.
    Fake,
    /// The real provider CLI, resolved as the app resolves it.
    Real,
}

impl Mode {
    /// `Real` when `GESPAN_E2E_REAL=1`, otherwise `Fake`.
    pub(crate) fn from_env() -> Self {
        match std::env::var("GESPAN_E2E_REAL").as_deref() {
            Ok("1") => Self::Real,
            _ => Self::Fake,
        }
    }
    fn turn_timeout(self) -> Duration {
        match self {
            Self::Fake => Duration::from_secs(120),
            Self::Real => Duration::from_secs(300),
        }
    }
}

/// A window with one project, a temporary git repository with one commit.
pub(crate) struct E2e {
    mode: Mode,
    window: WindowHandle<Shell>,
    repo: tempfile::TempDir,
    agents: tempfile::TempDir,
}

impl E2e {
    pub(crate) fn new(cx: &mut TestAppContext, mode: Mode) -> Self {
        // Turns run on real threads and wake the foreground from there; the
        // test scheduler accepts that only once parking is allowed.
        cx.executor().allow_parking();
        let repo = tempfile::tempdir().unwrap();
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.email", "e2e@example.invalid"],
            &["config", "user.name", "E2E"],
            &["commit", "--allow-empty", "-m", "Initial"],
        ] {
            git(repo.path(), args);
        }
        let project = repo.path().to_string_lossy().into_owned();
        let store: Database = Arc::new(Mutex::new(Store::open(Path::new(":memory:")).unwrap()));
        let mut state = Workspace::default();
        state.add_project(&project);
        let saved = serde_json::to_string(&state).unwrap();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, Vec::new(), project, Some(saved), cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let agents = tempfile::tempdir().unwrap();
        let store = window.update(cx, |shell, _, _| shell.e2e_store()).unwrap();
        locked(&store)
            .unwrap()
            .save_setting(ROOT_KEY, &agents.path().to_string_lossy())
            .unwrap();
        Self {
            mode,
            window,
            repo,
            agents,
        }
    }

    /// Scripts the agent of the next session the app starts without the
    /// test, as `sessions_start` does: one entry per turn, as in
    /// [`E2e::script_frames`].
    pub(crate) fn script_next_child(&self, turns: Value) {
        let path = self.agents.path().join("children.json");
        let mut queue: Vec<Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        queue.push(turns);
        std::fs::write(path, serde_json::to_vec(&queue).unwrap()).unwrap();
    }

    /// Points one-off background turns (Polish) at an agent of their own,
    /// scripted like [`E2e::script_frames`]: one entry per turn.
    pub(crate) fn script_background(&self, turns: Value, cx: &mut TestAppContext) {
        let binary = provision_in(self.agents.path(), "background", Kind::Claude);
        std::fs::write(
            self.agents.path().join("background").join("replies.json"),
            serde_json::to_vec(&turns).unwrap(),
        )
        .unwrap();
        locked(&self.store(cx))
            .unwrap()
            .save_setting(BACKGROUND_KEY, &binary.to_string_lossy())
            .unwrap();
    }
    /// The text of every prompt the background agent received, in order.
    pub(crate) fn background_prompts(&self) -> Vec<String> {
        read_lines(&self.agents.path().join("background").join("received.jsonl"))
            .into_iter()
            .filter(|frame| frame["type"] == "user")
            .map(|frame| prompt_text(&frame["message"]["content"]))
            .collect()
    }

    /// Lets a background turn scripted with `"wait": true` go on.
    pub(crate) fn release_background(&self) {
        let _ = std::fs::write(self.agents.path().join("background").join("release"), b"");
    }
    /// Every start of the background agent: `{"argv","cwd","env"}`.
    pub(crate) fn background_launches(&self) -> Vec<Value> {
        read_lines(&self.agents.path().join("background").join("launches.jsonl"))
    }

    /// Every call the session's agent made to Gespan's own tools, in order:
    /// `{"tool","arguments","result"}` or `{..,"error"}`.
    pub(crate) fn mcp_calls(&self, id: i64) -> Vec<Value> {
        read_lines(&self.agent_dir(id).join("mcp.jsonl"))
    }

    /// Lets a turn scripted with `"wait": true` go on.
    pub(crate) fn release(&self, id: i64) {
        std::fs::write(self.agent_dir(id).join("release"), b"").unwrap();
    }

    /// Runs the app until `done` holds, or fails after the turn timeout.
    pub(crate) fn until(
        &self,
        cx: &mut TestAppContext,
        what: &str,
        mut done: impl FnMut(&Self, &mut TestAppContext) -> bool,
    ) {
        let deadline = Instant::now() + self.mode.turn_timeout();
        loop {
            cx.run_until_parked();
            if done(self, cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The sessions this window has open, in no particular order.
    pub(crate) fn all_sessions(&self, cx: &mut TestAppContext) -> Vec<i64> {
        self.sessions(cx)
    }

    /// Whether `id`'s agent is in a turn.
    pub(crate) fn busy(&self, id: i64, cx: &mut TestAppContext) -> bool {
        self.session(id, cx).read_with(cx, |view, _| view.busy)
    }

    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    /// The project's repository.
    pub(crate) fn repo(&self) -> &Path {
        self.repo.path()
    }

    /// New chat in the project, as the sidebar's New chat starts one, with
    /// its own agent. Returns the session id.
    pub(crate) fn start_session(&self, harness: Kind, cx: &mut TestAppContext) -> i64 {
        let before = self.sessions(cx);
        self.window
            .update(cx, |shell, _, cx| shell.e2e_create(harness, cx))
            .unwrap();
        cx.run_until_parked();
        let id = self
            .sessions(cx)
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("New chat made a session");
        if self.mode == Mode::Real {
            let real = gespan_core::provider::resolve(harness).unwrap_or_else(|| {
                panic!(
                    "GESPAN_E2E_REAL=1 needs the {} CLI; set GESPAN_{}_BINARY",
                    harness.label(),
                    harness.id().to_uppercase()
                )
            });
            std::fs::write(
                self.agents.path().join("proxy"),
                real.to_string_lossy().as_bytes(),
            )
            .unwrap();
        } else {
            assert_eq!(
                harness,
                Kind::Claude,
                "the fake agent speaks only Claude's stream-json"
            );
        }
        let binary = provision(self.agents.path(), id, harness);
        locked(&self.store(cx))
            .unwrap()
            .save_setting(&binary_key(id), &binary.to_string_lossy())
            .unwrap();
        id
    }

    /// Queues the fake agent's answer to this session's next unscripted
    /// turn. Real mode ignores the script.
    pub(crate) fn script_reply(&self, id: i64, text: &str) {
        self.script_frames(id, serde_json::json!(text));
    }

    // Not every helper has a test yet; they are the harness's API.
    #[allow(dead_code)]
    /// Queues a turn with raw stream-json `frames` written before its text,
    /// for tool calls and other events: `{"text": "...", "frames": [...]}`.
    pub(crate) fn script_frames(&self, id: i64, turn: Value) {
        let path = self.agent_dir(id).join("replies.json");
        let mut script: Vec<Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        script.push(turn);
        std::fs::write(path, serde_json::to_vec(&script).unwrap()).unwrap();
    }

    /// Types `text` in the session's composer and presses Enter, then runs
    /// until the turn ends. Returns the assistant's reply to it.
    pub(crate) fn send(&self, id: i64, text: &str, cx: &mut TestAppContext) -> String {
        let view = self.session(id, cx);
        let prompts = count_prompts(&self.transcript(id, cx));
        view.update(cx, |view, cx| {
            view.input
                .update(cx, |input, cx| input.set_text(text.into(), cx));
            view.send_typed(cx);
        });
        let deadline = Instant::now() + self.mode.turn_timeout();
        loop {
            // The turn runs on a thread of its own: give its updates a moment
            // to arrive, then let the foreground apply them.
            cx.run_until_parked();
            let (busy, rows) = view.read_with(cx, |view, cx| {
                (view.busy, view.transcript.read(cx).source_rows().to_vec())
            });
            let started = count_prompts(&rows) > prompts;
            if started && !busy && rows.last().is_some_and(|row| !row.is_prompt()) {
                let from = rows.iter().rposition(Row::is_prompt).unwrap() + 1;
                return rows[from..]
                    .iter()
                    .filter(|row| row.role == "assistant")
                    .map(|row| row.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            assert!(
                Instant::now() < deadline,
                "the turn for {text:?} did not end in {:?}; busy {busy}, rows {rows:?}",
                self.mode.turn_timeout()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The text of every prompt the session's agent received, in order.
    pub(crate) fn prompts(&self, id: i64) -> Vec<String> {
        self.received(id)
            .into_iter()
            .filter(|frame| frame["type"] == "user")
            .map(|frame| prompt_text(&frame["message"]["content"]))
            .collect()
    }

    /// Every stdin frame the session's agent received, in order, across its
    /// processes: control requests (initialize, MCP mounts) and prompts.
    pub(crate) fn received(&self, id: i64) -> Vec<Value> {
        read_lines(&self.agent_dir(id).join("received.jsonl"))
    }

    /// One `{"argv","cwd","env"}` per agent process the session started.
    pub(crate) fn launches(&self, id: i64) -> Vec<Value> {
        read_lines(&self.agent_dir(id).join("launches.jsonl"))
    }

    /// The rows the session's transcript shows.
    pub(crate) fn transcript(&self, id: i64, cx: &mut TestAppContext) -> Vec<Row> {
        self.session(id, cx).read_with(cx, |view, cx| {
            view.transcript.read(cx).source_rows().to_vec()
        })
    }

    pub(crate) fn session(&self, id: i64, cx: &mut TestAppContext) -> Entity<Conversation> {
        self.window
            .update(cx, |shell, _, _| shell.e2e_session(id))
            .unwrap()
            .unwrap_or_else(|| panic!("session {id} is not open"))
    }

    #[allow(dead_code)]
    pub(crate) fn window(&self) -> WindowHandle<Shell> {
        self.window
    }

    pub(crate) fn store(&self, cx: &mut TestAppContext) -> Database {
        self.window
            .update(cx, |shell, _, _| shell.e2e_store())
            .unwrap()
    }

    fn sessions(&self, cx: &mut TestAppContext) -> Vec<i64> {
        self.window
            .update(cx, |shell, _, _| shell.e2e_sessions())
            .unwrap()
    }

    fn agent_dir(&self, id: i64) -> PathBuf {
        self.agents.path().join(id.to_string())
    }
}

fn count_prompts(rows: &[Row]) -> usize {
    rows.iter().filter(|row| row.is_prompt()).count()
}

/// Claude's `message.content`: a string, or blocks whose text is joined.
fn prompt_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn read_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The `gespan-fake-agent` executable: `GESPAN_E2E_FAKE_AGENT` when set,
/// otherwise built once per test process with cargo into a target folder of
/// its own, so it never waits on the build that is running these tests.
fn fake_agent() -> &'static Path {
    static AGENT: OnceLock<PathBuf> = OnceLock::new();
    AGENT.get_or_init(|| {
        if let Some(path) = std::env::var_os("GESPAN_E2E_FAKE_AGENT").filter(|p| !p.is_empty()) {
            return path.into();
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| manifest.join("../../target"))
            .join("e2e-fake-agent");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args(["build", "--quiet", "--manifest-path"])
            .arg(manifest.join("../gespan-fake-agent/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .output()
            .expect("cargo runs");
        assert!(
            output.status.success(),
            "building gespan-fake-agent failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        target
            .join("debug")
            .join(format!("gespan-fake-agent{}", std::env::consts::EXE_SUFFIX))
    })
}
