//! Actions an agent runs, captured so it can read their output and stop them.
//!
//! A run keeps its last [`MAX_LINES`] lines. [`output`] returns the lines since
//! the last read; lines that scroll out before they are read are gone. Finished
//! runs stay readable until [`MAX_FINISHED`] newer ones have finished.
//!
//! The app registers a [`Mirror`] to show runs in dock tabs. Without one, runs
//! are only captured.
use super::Action;
use crate::process::Group;
use serde::Serialize;
use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, LazyLock, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};

const MAX_LINES: usize = 200;
const MAX_LINE: usize = 2_000;
const MAX_FINISHED: usize = 32;
/// How often a run's exit is checked.
const POLL: Duration = Duration::from_millis(20);
/// Output after the exit is read for this long while something the action
/// left running is still alive: it can hold the pipes open forever.
const DRAIN: Duration = Duration::from_millis(500);
/// With nothing left running, the pipes close once the readers catch up, which
/// a busy machine can delay. This only bounds a pipe held outside the group.
const CLOSED_DRAIN: Duration = Duration::from_secs(10);
/// How long `stop` waits for the run to end.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// Output bytes a slow mirror may fall behind by. Past it, chunks are dropped
/// for the mirror only; the run and its captured lines are unaffected.
const MIRROR_BACKLOG: usize = 4 * 1024 * 1024;

/// A run as an agent sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub run_id: String,
    pub status: Status,
    pub exit_code: Option<i32>,
    /// Lines since the last read.
    pub output: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Running,
    Exited,
    Stopped,
}

/// What a [`Mirror`] is told, in order: `Started`, `Output`s, `Finished`.
pub enum RunEvent {
    Started {
        run_id: String,
        project: String,
        /// The checkout the action runs in: the nearest folder holding `.git`.
        checkout: PathBuf,
        label: String,
        command: String,
    },
    /// Raw output, stdout and stderr as they arrive.
    Output { run_id: String, bytes: Vec<u8> },
    Finished {
        run_id: String,
        status: Status,
        exit_code: Option<i32>,
    },
}

/// Shows runs in the app. Events come on a thread of their own, so a slow
/// mirror never holds up a run.
pub trait Mirror: Send + Sync {
    fn event(&self, event: RunEvent);
}

static MIRROR: Mutex<Option<mpsc::Sender<RunEvent>>> = Mutex::new(None);
static BACKLOG: AtomicUsize = AtomicUsize::new(0);
static RUNS: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

/// Sends every later run's events to `mirror`, in place of any earlier one.
pub fn set_mirror(mirror: Arc<dyn Mirror>) {
    let (sender, events) = mpsc::channel::<RunEvent>();
    let spawned = std::thread::Builder::new()
        .name("gespan-action-mirror".into())
        .spawn(move || {
            for event in events {
                if let RunEvent::Output { bytes, .. } = &event {
                    BACKLOG.fetch_sub(bytes.len(), Ordering::Relaxed);
                }
                mirror.event(event);
            }
        });
    if spawned.is_ok() {
        *lock(&MIRROR) = Some(sender);
    }
}

fn emit(event: RunEvent) {
    let mirror = lock(&MIRROR);
    let Some(sender) = mirror.as_ref() else {
        return;
    };
    if let RunEvent::Output { bytes, .. } = &event {
        if BACKLOG.fetch_add(bytes.len(), Ordering::Relaxed) > MIRROR_BACKLOG {
            BACKLOG.fetch_sub(bytes.len(), Ordering::Relaxed);
            return;
        }
    }
    let _ = sender.send(event);
}

#[derive(Default)]
struct Registry {
    /// Oldest first.
    runs: Vec<Arc<Run>>,
}

impl Registry {
    /// The run, if it runs in `root` or below when one is given. A run outside
    /// it gets the same error as an id that doesn't exist, so nothing leaks.
    fn find(&self, root: Option<&Path>, run_id: &str) -> Result<Arc<Run>, String> {
        let root = root.map(super::tasks_json::normalize);
        self.runs
            .iter()
            .find(|run| {
                run.id == run_id && root.as_ref().is_none_or(|root| run.cwd.starts_with(root))
            })
            .cloned()
            .ok_or_else(|| format!("No run {run_id}. Finished runs are kept only for a while."))
    }

    /// Random, so one session can't guess another's.
    fn new_id(&self) -> String {
        loop {
            let id = format!("run-{:016x}", rand::random::<u64>());
            if !self.runs.iter().any(|run| run.id == id) {
                return id;
            }
        }
    }

    /// Forgets the oldest finished runs past [`MAX_FINISHED`].
    fn evict(&mut self) {
        let finished = self.runs.iter().filter(|run| !run.running()).count();
        let mut excess = finished.saturating_sub(MAX_FINISHED);
        self.runs.retain(|run| {
            if excess > 0 && !run.running() {
                excess -= 1;
                return false;
            }
            true
        });
    }
}

struct Run {
    id: String,
    action: String,
    cwd: PathBuf,
    state: Mutex<State>,
    changed: Condvar,
    /// Set once spawned. The group outlives the shell, for what it left running.
    process: Mutex<Option<(Child, Group)>>,
    stopping: AtomicBool,
    /// Cleared when the mirror is told the run finished; nothing follows it.
    mirrored: Mutex<bool>,
}

struct State {
    lines: VecDeque<String>,
    /// Lines ever kept, so `pushed - lines.len()` numbers the oldest.
    pushed: u64,
    /// Lines handed out so far.
    read: u64,
    status: Status,
    exit_code: Option<i32>,
    open_pipes: usize,
}

impl Run {
    fn new(id: String, action: &Action) -> Self {
        Run {
            id,
            action: action.id.clone(),
            cwd: action.cwd.clone(),
            state: Mutex::new(State {
                lines: VecDeque::new(),
                pushed: 0,
                read: 0,
                status: Status::Running,
                exit_code: None,
                open_pipes: 0,
            }),
            changed: Condvar::new(),
            process: Mutex::new(None),
            stopping: AtomicBool::new(false),
            mirrored: Mutex::new(true),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn running(&self) -> bool {
        self.state().status == Status::Running
    }

    fn push(&self, line: String) {
        let line = match line.char_indices().nth(MAX_LINE) {
            Some((end, _)) => line[..end].to_owned(),
            None => line,
        };
        let mut state = self.state();
        if state.lines.len() == MAX_LINES {
            state.lines.pop_front();
        }
        state.lines.push_back(line);
        state.pushed += 1;
    }

    fn mirror_output(&self, bytes: &[u8]) {
        let mirrored = lock(&self.mirrored);
        if *mirrored {
            emit(RunEvent::Output {
                run_id: self.id.clone(),
                bytes: bytes.to_vec(),
            });
        }
    }

    /// Waits until the run ends or `wait` passes.
    fn wait(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut state = self.state();
        while state.status == Status::Running {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// The run with the lines since the last read, or with every kept line.
    fn report(&self, every_line: bool) -> Report {
        let mut state = self.state();
        let oldest = state.pushed - state.lines.len() as u64;
        let from = if every_line {
            oldest
        } else {
            state.read.max(oldest)
        };
        let output = state
            .lines
            .iter()
            .skip((from - oldest) as usize)
            .cloned()
            .collect();
        state.read = state.pushed;
        Report {
            run_id: self.id.clone(),
            status: state.status,
            exit_code: state.exit_code,
            output,
        }
    }

    fn finish(&self, status: Status, exit_code: Option<i32>) {
        let mut state = self.state();
        state.status = status;
        state.exit_code = exit_code;
        drop(state);
        let mut mirrored = lock(&self.mirrored);
        *mirrored = false;
        emit(RunEvent::Finished {
            run_id: self.id.clone(),
            status,
            exit_code,
        });
        drop(mirrored);
        self.changed.notify_all();
    }

    /// Kills the run's whole group, even after the shell itself has exited:
    /// what it left running still belongs to the run. A run whose group is
    /// gone is left as it ended. One stopped after it finished reports
    /// `Stopped` from then on; the mirror has already been told it finished.
    fn terminate(&self) {
        let mut process = lock(&self.process);
        let Some((child, group)) = process.as_mut() else {
            // Not spawned yet: `launch` stops it as soon as it is.
            self.stopping.store(true, Ordering::SeqCst);
            return;
        };
        let running = self.running();
        if !running && !group.alive() {
            return;
        }
        self.stopping.store(true, Ordering::SeqCst);
        group.kill();
        if running {
            crate::process::terminate(child);
            return;
        }
        drop(process);
        self.state().status = Status::Stopped;
        self.changed.notify_all();
    }
}

/// Starts `action` and waits until it exits or `wait` passes. If the same
/// action is already running in the same folder, that run is returned at once,
/// ignoring `wait`, with every line it has kept.
pub fn start(project: &str, action: &Action, wait: Duration) -> Report {
    // Built before the lock: the first build reads the login shell's PATH.
    let command = command(action);
    let run = {
        let mut registry = lock(&RUNS);
        let live = registry
            .runs
            .iter()
            .find(|run| run.action == action.id && run.cwd == action.cwd && run.running())
            .cloned();
        if let Some(run) = live {
            drop(registry);
            return run.report(true);
        }
        // Reserved under the lock, so a second start of the same action finds
        // this run; it is spawned after.
        let run = Arc::new(Run::new(registry.new_id(), action));
        registry.runs.push(run.clone());
        registry.evict();
        run
    };
    launch(&run, project, action, command);
    run.wait(wait);
    run.report(false)
}

/// The lines since the last read, once the run ends or `wait` passes.
pub fn output(run_id: &str, wait: Duration) -> Result<Report, String> {
    read(None, run_id, wait)
}

/// Kills the run with everything it started, even what it left running after
/// it exited, and reports how it ended.
pub fn stop(run_id: &str) -> Result<Report, String> {
    end(None, run_id)
}

/// [`output`], for a caller whose checkout is `root`: a run elsewhere is
/// reported as unknown.
pub fn output_within(root: &Path, run_id: &str, wait: Duration) -> Result<Report, String> {
    read(Some(root), run_id, wait)
}

/// [`stop`], for a caller whose checkout is `root`: a run elsewhere is
/// reported as unknown and left alone.
pub fn stop_within(root: &Path, run_id: &str) -> Result<Report, String> {
    end(Some(root), run_id)
}

fn read(root: Option<&Path>, run_id: &str, wait: Duration) -> Result<Report, String> {
    let run = lock(&RUNS).find(root, run_id)?;
    run.wait(wait);
    Ok(run.report(false))
}

fn end(root: Option<&Path>, run_id: &str) -> Result<Report, String> {
    let run = lock(&RUNS).find(root, run_id)?;
    run.terminate();
    run.wait(STOP_WAIT);
    Ok(run.report(false))
}

/// Kills every run and what it left running, for when Gespan quits.
pub fn stop_all() {
    let runs: Vec<_> = lock(&RUNS).runs.clone();
    for run in runs {
        run.terminate();
    }
}

fn command(action: &Action) -> Command {
    let mut command = shell(&action.command);
    command
        .current_dir(&action.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::scm::shell::apply_gui_env(&mut command);
    for (name, value) in &action.env {
        command.env(
            name,
            expand_env(value, cfg!(windows), |name| std::env::var(name).ok()),
        );
    }
    crate::process::isolate(&mut command);
    command
}

fn launch(run: &Arc<Run>, project: &str, action: &Action, mut command: Command) {
    emit(RunEvent::Started {
        run_id: run.id.clone(),
        project: project.to_owned(),
        checkout: checkout(&action.cwd),
        label: action.label.clone(),
        command: action.command.clone(),
    });
    let (mut child, group) = match crate::process::spawn_group(&mut command) {
        Ok(spawned) => spawned,
        Err(error) => {
            let message = format!("Couldn't start it: {error}");
            run.mirror_output(format!("{message}\r\n").as_bytes());
            run.push(message);
            run.finish(Status::Exited, None);
            return;
        }
    };
    let pipes: Vec<Box<dyn Read + Send>> = [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    run.state().open_pipes = pipes.len();
    *lock(&run.process) = Some((child, group));
    for pipe in pipes {
        let run = run.clone();
        std::thread::spawn(move || {
            let tee = Tee {
                pipe,
                run: run.clone(),
            };
            crate::worktree_scripts::read_lines(tee, &mut |line| run.push(line));
            run.state().open_pipes -= 1;
            run.changed.notify_all();
        });
    }
    let waiter = run.clone();
    std::thread::spawn(move || {
        let run = waiter;
        let exit_code = loop {
            if let Some((child, _)) = lock(&run.process).as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => break status.code(),
                    Ok(None) => {}
                    Err(_) => break None,
                }
            }
            std::thread::sleep(POLL);
        };
        let left_running = lock(&run.process)
            .as_ref()
            .is_some_and(|(_, group)| group.alive());
        let deadline = Instant::now() + if left_running { DRAIN } else { CLOSED_DRAIN };
        let mut state = run.state();
        while state.open_pipes > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = run
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(state);
        let status = if run.stopping.load(Ordering::SeqCst) {
            Status::Stopped
        } else {
            Status::Exited
        };
        run.finish(status, exit_code);
    });
    // Stopped while it was being spawned.
    if run.stopping.load(Ordering::SeqCst) {
        run.terminate();
    }
}

/// Passes output through to the mirror as it is read.
struct Tee {
    pipe: Box<dyn Read + Send>,
    run: Arc<Run>,
}

impl Read for Tee {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.pipe.read(buf)?;
        if read > 0 {
            self.run.mirror_output(&buf[..read]);
        }
        Ok(read)
    }
}

/// The nearest folder at or above `cwd` holding `.git`, else `cwd`.
fn checkout(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// The command line runs as written, so the shell expands its own references.
fn shell(line: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let comspec = std::env::var("COMSPEC")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "cmd.exe".into());
        let mut command = Command::new(comspec);
        // `/S` with the line in quotes: cmd drops just those, so a quoted
        // program path keeps its quotes. Raw, so Rust's quoting stays out.
        command
            .args(["/D", "/S", "/C"])
            .raw_arg(format!("\"{line}\""));
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(line);
        command
    }
}

/// Fills in the shell references in an `env` value: `${NAME}` for `sh`,
/// `%NAME%` for `cmd`. A variable that isn't set is empty. Anything else stays.
pub(crate) fn expand_env(
    value: &str,
    windows: bool,
    lookup: impl Fn(&str) -> Option<String>,
) -> String {
    let (open, close) = if windows { ("%", "%") } else { ("${", "}") };
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        let Some(len) = after.find(close) else {
            break;
        };
        let name = &after[..len];
        out.push_str(&rest[..start]);
        if super::tasks_json::is_env_name(name) {
            out.push_str(&lookup(name).unwrap_or_default());
            rest = &after[len + close.len()..];
        } else {
            // Not a reference: keep the opening and look again after it.
            out.push_str(open);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
