//! A project's Setup and Teardown scripts. They live in Gespan's own data, never
//! in the repository, so opening a repository can't run its code.
//!
//! Setup runs in a new worktree before anything starts in it, and Teardown runs
//! before a worktree is removed. Every step of a repository takes the same
//! [`Turn`], so parallel creation can't race on Git locks or shared caches. The
//! turn is given up while a failure waits for an answer: one person's unanswered
//! question doesn't hold up the repository's other worktrees.
//!
//! The app registers a [`Host`] to show the output and answer failures. Without
//! one, a failure stops the step.
use crate::worktree_copy::Copied;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};

const FILE: &str = "worktree-scripts.json";
const MAX_LINE: usize = 2_000;
/// A script's output after it has exited, read for this long. Something it left
/// running can hold the pipes open forever.
const DRAIN: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scripts {
    pub setup: String,
    pub teardown: String,
}

/// How a step ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Passed,
    Failed(String),
    /// It failed, and the user went on anyway.
    Overridden(String),
    Cancelled,
}

/// What the last Setup did for a worktree, for Settings to show afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    pub copied: usize,
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Setup,
    Teardown,
}

/// One step, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: u64,
    pub stage: Stage,
    pub project: PathBuf,
    pub worktree: PathBuf,
    pub branch: Option<String>,
    pub copied: Copied,
    /// False when there was only copying to report.
    pub scripted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Started(Run),
    Output {
        id: u64,
        line: String,
    },
    Retrying {
        id: u64,
    },
    /// `Decision`s are asked for after this, with `Host::decide`.
    Failed {
        id: u64,
        reason: String,
    },
    Finished {
        id: u64,
        outcome: Outcome,
    },
}

/// The answer to a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Retry,
    /// Start anyway, or remove anyway.
    Continue,
    Cancel,
}

pub trait Host: Send + Sync {
    fn event(&self, event: Event);
    /// Blocks until the failed step `id` has an answer.
    fn decide(&self, id: u64) -> Decision;
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Data {
    projects: HashMap<String, Scripts>,
    worktrees: HashMap<String, Record>,
}

#[derive(Default)]
struct Store {
    /// Unset in tests and anything else without a data folder: it stays in memory.
    path: Option<PathBuf>,
    data: Data,
}
impl Store {
    /// Reads the file. A file that can't be read leaves the store empty and
    /// reports it, so nothing saves over what might still be in it.
    fn load(&mut self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        self.data = Data::default();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("Could not read the scripts: {error}")),
        };
        self.data = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not read the scripts: {error}"))?;
        Ok(())
    }
    fn read(&mut self) {
        if let Err(error) = self.load() {
            tracing::warn!(%error, "could not read a project's worktree scripts");
        }
    }
    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&self.data).map_err(|e| e.to_string())?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let written = std::fs::write(&temporary, bytes)
            .and_then(|()| std::fs::rename(&temporary, path))
            .map_err(|e| format!("Could not save the scripts: {e}"));
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written
    }
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Mutex::default);
static HOST: Mutex<Option<Arc<dyn Host>>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static CANCELS: LazyLock<Mutex<HashMap<u64, Arc<AtomicBool>>>> = LazyLock::new(Mutex::default);
static BUSY: LazyLock<(Mutex<HashSet<String>>, Condvar)> = LazyLock::new(Default::default);

/// Keeps the scripts in `data_dir` from now on. Call once at startup.
pub fn init(data_dir: &Path) {
    if let Ok(mut store) = STORE.lock() {
        store.path = Some(data_dir.join(FILE));
    }
}

/// Registers the app's view of these steps.
pub fn set_host(host: Arc<dyn Host>) {
    if let Ok(mut slot) = HOST.lock() {
        *slot = Some(host);
    }
}

fn host_for(main: &Path) -> Option<Arc<dyn Host>> {
    #[cfg(test)]
    if let Some(host) = test_host::get(main) {
        return Some(host);
    }
    let _ = main;
    HOST.lock().ok()?.clone()
}

/// The same key for every spelling of a path. A remote path is already one.
fn key(path: &Path) -> String {
    if crate::files::remote::is_remote(path) {
        return path.to_string_lossy().into_owned();
    }
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    crate::files::path_key(&real.to_string_lossy())
}

/// The scripts of the project whose main checkout is `main`.
pub fn scripts(main: &Path) -> Scripts {
    let Ok(mut store) = STORE.lock() else {
        return Scripts::default();
    };
    store.read();
    store
        .data
        .projects
        .get(&key(main))
        .cloned()
        .unwrap_or_default()
}

/// [`scripts`], for a step that mustn't go ahead without knowing them.
fn stored(store: &mut Store, main: &Path) -> Result<Scripts, String> {
    store.load()?;
    let scripts = store.data.projects.get(&key(main));
    Ok(scripts.cloned().unwrap_or_default())
}

pub fn save_scripts(main: &Path, scripts: &Scripts) -> Result<(), String> {
    let mut store = STORE.lock().map_err(|_| "The scripts are unavailable")?;
    store.load()?;
    let key = key(main);
    if scripts.setup.trim().is_empty() && scripts.teardown.trim().is_empty() {
        store.data.projects.remove(&key);
    } else {
        store.data.projects.insert(key, scripts.clone());
    }
    store.save()
}

/// What the last Setup did for the worktree at `path`, if it did anything.
pub fn record(path: &Path) -> Option<Record> {
    let mut store = STORE.lock().ok()?;
    store.read();
    store.data.worktrees.get(&key(path)).cloned()
}

/// [`record`] for each of `paths`, reading the file once.
pub fn records(paths: &[&Path]) -> Vec<Option<Record>> {
    let Ok(mut store) = STORE.lock() else {
        return vec![None; paths.len()];
    };
    store.read();
    paths
        .iter()
        .map(|path| store.data.worktrees.get(&key(path)).cloned())
        .collect()
}

fn remember(path: &Path, record: Record) {
    let Ok(mut store) = STORE.lock() else { return };
    let saved = store.load().and_then(|()| {
        store.data.worktrees.insert(key(path), record);
        store.save()
    });
    if let Err(error) = saved {
        tracing::warn!(%error, "could not record a worktree's setup");
    }
}

/// Drops the record of a worktree that has been removed. The path is keyed
/// before the folder goes, in [`tear_down`], so a link in it still resolves.
fn forget(key: &str) {
    let Ok(mut store) = STORE.lock() else { return };
    let cleared = store.load().and_then(|()| {
        if store.data.worktrees.remove(key).is_some() {
            store.save()
        } else {
            Ok(())
        }
    });
    if let Err(error) = cleared {
        tracing::warn!(%error, "could not clear a worktree's setup record");
    }
}

/// Stops the running step `id`, if there is one.
pub fn cancel(id: u64) {
    if let Some(flag) = CANCELS.lock().ok().and_then(|c| c.get(&id).cloned()) {
        flag.store(true, Ordering::SeqCst);
    }
}

/// This repository's right to change its worktrees. Waits for any other holder.
pub(crate) struct Turn {
    key: String,
}
impl Drop for Turn {
    fn drop(&mut self) {
        let (busy, freed) = &*BUSY;
        if let Ok(mut busy) = busy.lock() {
            busy.remove(&self.key);
        }
        freed.notify_all();
    }
}
pub(crate) fn turn(main: &Path) -> Turn {
    let key = key(main);
    let (busy, freed) = &*BUSY;
    let mut held = busy.lock().unwrap_or_else(|e| e.into_inner());
    while held.contains(&key) {
        held = freed.wait(held).unwrap_or_else(|e| e.into_inner());
    }
    held.insert(key.clone());
    Turn { key }
}

/// How a script can end, once it has run. An `Err` from a runner means it
/// couldn't start or be followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Code(Option<i32>),
    Cancelled,
}

/// Where a step's script runs: on this machine, or a remote project's host.
pub(crate) type Runner<'a> =
    dyn Fn(&str, &mut dyn FnMut(String), &AtomicBool) -> Result<Exit, String> + 'a;

fn runner<'a>(main: &'a Path, worktree: &'a Path, branch: Option<&'a str>) -> Box<Runner<'a>> {
    if crate::files::remote::is_remote(worktree) {
        return crate::git::remote_script_runner(worktree, branch);
    }
    Box::new(move |script, line, cancelled| {
        run_local(script, main, worktree, branch, line, cancelled)
    })
}

/// The reason a run didn't pass, or `None` if it was cancelled.
fn failure(exit: Result<Exit, String>) -> Result<(), Option<String>> {
    match exit {
        Ok(Exit::Code(Some(0))) => Ok(()),
        Ok(Exit::Code(Some(code))) => Err(Some(format!("It exited with code {code}."))),
        Ok(Exit::Code(None)) => Err(Some("It was stopped before it finished.".into())),
        Ok(Exit::Cancelled) => Err(None),
        Err(error) => Err(Some(error)),
    }
}

struct Step<'a> {
    run: Run,
    script: String,
    runner: &'a Runner<'a>,
    host: Option<Arc<dyn Host>>,
    cancelled: Arc<AtomicBool>,
}
impl Step<'_> {
    fn emit(&self, event: Event) {
        if let Some(host) = &self.host {
            host.event(event);
        }
    }
    /// Runs the script until it passes or the user answers. `turn` is held only
    /// while the script runs.
    fn run(&self, held: Turn, main: &Path) -> Outcome {
        let mut current = Some(held);
        loop {
            let held = current.take().unwrap_or_else(|| turn(main));
            self.cancelled.store(false, Ordering::SeqCst);
            let host = self.host.clone();
            let id = self.run.id;
            let exit = (self.runner)(
                &self.script,
                &mut |line| {
                    if let Some(host) = &host {
                        host.event(Event::Output { id, line });
                    }
                },
                &self.cancelled,
            );
            drop(held);
            let reason = match failure(exit) {
                Ok(()) => return Outcome::Passed,
                Err(None) => return Outcome::Cancelled,
                Err(Some(reason)) => reason,
            };
            self.emit(Event::Failed {
                id,
                reason: reason.clone(),
            });
            match self.host.as_ref().map(|host| host.decide(id)) {
                Some(Decision::Retry) => self.emit(Event::Retrying { id }),
                Some(Decision::Continue) => return Outcome::Overridden(reason),
                Some(Decision::Cancel) => return Outcome::Cancelled,
                None => return Outcome::Failed(reason),
            }
        }
    }
}

fn step<'a>(
    stage: Stage,
    main: &Path,
    worktree: &Path,
    branch: Option<&str>,
    copied: Copied,
    script: &str,
    runner: &'a Runner<'a>,
) -> Step<'a> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let cancelled = Arc::new(AtomicBool::new(false));
    if let Ok(mut cancels) = CANCELS.lock() {
        cancels.insert(id, cancelled.clone());
    }
    Step {
        run: Run {
            id,
            stage,
            project: main.to_path_buf(),
            worktree: worktree.to_path_buf(),
            branch: branch.map(str::to_owned),
            copied,
            scripted: !script.is_empty(),
        },
        script: script.to_owned(),
        runner,
        host: host_for(main),
        cancelled,
    }
}
fn finish(step: &Step<'_>, outcome: Outcome) {
    if let Ok(mut cancels) = CANCELS.lock() {
        cancels.remove(&step.run.id);
    }
    step.emit(Event::Finished {
        id: step.run.id,
        outcome,
    });
}

/// Runs the project's Setup in a worktree that was just made and filled. Lets
/// the caller take the error to whatever was going to start there: on `Err`
/// nothing should, and the worktree stays where it is.
pub(crate) fn set_up(
    turn: Turn,
    main: &Path,
    worktree: &Path,
    branch: Option<&str>,
    copied: Copied,
) -> Result<(), String> {
    let run = runner(main, worktree, branch);
    let run = &*run;
    let script = scripts(main).setup;
    let script = script.trim();
    let files = copied.files;
    if script.is_empty() && files == 0 && copied.skipped == 0 {
        return Ok(());
    }
    let step = step(Stage::Setup, main, worktree, branch, copied, script, run);
    step.emit(Event::Started(step.run.clone()));
    let outcome = if script.is_empty() {
        Outcome::Passed
    } else {
        step.run(turn, main)
    };
    remember(
        worktree,
        Record {
            copied: files,
            outcome: step.run.scripted.then(|| outcome.clone()),
        },
    );
    finish(&step, outcome.clone());
    let kept = worktree.display();
    match outcome {
        Outcome::Passed | Outcome::Overridden(_) => Ok(()),
        Outcome::Failed(reason) => Err(format!(
            "The Setup script failed, so nothing was started. {reason} The worktree is still at {kept}."
        )),
        Outcome::Cancelled => Err(format!(
            "Setup was cancelled, so nothing was started. The worktree is still at {kept}."
        )),
    }
}

/// Runs the project's Teardown in a worktree about to go. On `Err` it stays.
/// Keep the [`Removal`] until it's gone.
pub(crate) fn tear_down(
    main: &Path,
    worktree: &Path,
    branch: Option<&str>,
) -> Result<Removal, String> {
    let key = key(worktree);
    let mut store = STORE.lock().map_err(|_| "The scripts are unavailable")?;
    let script = stored(&mut store, main)
        .map_err(|error| format!("{error}. The worktree was kept."))?
        .teardown;
    drop(store);
    let script = script.trim();
    let remote = crate::files::remote::is_remote(worktree);
    if script.is_empty() || (!remote && !worktree.is_dir()) {
        return Ok(Removal {
            key,
            ran: false,
            _turn: turn(main),
        });
    }
    let run = runner(main, worktree, branch);
    let step = step(
        Stage::Teardown,
        main,
        worktree,
        branch,
        Copied::default(),
        script,
        &run,
    );
    let held = turn(main);
    step.emit(Event::Started(step.run.clone()));
    let outcome = step.run(held, main);
    finish(&step, outcome.clone());
    match outcome {
        Outcome::Passed | Outcome::Overridden(_) => Ok(Removal {
            key,
            ran: true,
            _turn: turn(main),
        }),
        Outcome::Failed(reason) => Err(format!(
            "The Teardown script failed, so the worktree was kept. {reason}"
        )),
        Outcome::Cancelled => Err("Teardown was cancelled, so the worktree was kept.".into()),
    }
}

/// Clears a worktree that never started for removal: there's nothing to tear
/// down, but it can have a record, and it still takes the repository's turn.
pub(crate) fn abandon(main: &Path, worktree: &Path) -> Removal {
    Removal {
        key: key(worktree),
        ran: false,
        _turn: turn(main),
    }
}

/// A worktree cleared for removal, holding the repository's turn until it's
/// gone. [`Removal::done`] drops its record.
pub(crate) struct Removal {
    key: String,
    /// A Teardown script ran, so it may have changed files that were clean.
    pub(crate) ran: bool,
    _turn: Turn,
}
impl Removal {
    pub(crate) fn done(self) {
        forget(&self.key);
    }
}

#[cfg(unix)]
fn shell(script: &str) -> Result<(Command, Option<tempfile::TempPath>), String> {
    let mut command = Command::new("/bin/sh");
    // Stops at the first failing line, so a later step can't hide it.
    command.args(["-e", "-c", &script.replace("\r\n", "\n")]);
    Ok((command, None))
}
/// cmd has no way to run several lines from an argument, so they go in a file.
#[cfg(windows)]
fn shell(script: &str) -> Result<(Command, Option<tempfile::TempPath>), String> {
    use std::io::Write;
    let mut file = tempfile::Builder::new()
        .prefix("gespan-script-")
        .suffix(".cmd")
        .tempfile()
        .map_err(|e| format!("Could not start the script: {e}"))?;
    // UTF-8, so a path or message outside the system code page survives.
    let lines = script.replace("\r\n", "\n").replace('\n', "\r\n");
    write!(file, "@echo off\r\nchcp 65001 >nul\r\n{lines}\r\n")
        .map_err(|e| format!("Could not start the script: {e}"))?;
    let path = file.into_temp_path();
    let mut command = Command::new("cmd");
    command.arg("/D").arg("/C").arg(&path);
    Ok((command, Some(path)))
}

/// cmd pastes `%GESPAN_BRANCH%` into the line before parsing it, and Git allows
/// `&`, `|` and `%` in a branch name, so a name from a fetched repository could
/// otherwise become a command. `sh` expands variables after parsing.
fn cmd_safe(branch: &str) -> String {
    branch
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '/' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
fn script_branch(branch: &str) -> String {
    if cfg!(windows) {
        cmd_safe(branch)
    } else {
        branch.to_owned()
    }
}

fn run_local(
    script: &str,
    main: &Path,
    worktree: &Path,
    branch: Option<&str>,
    line: &mut dyn FnMut(String),
    cancelled: &AtomicBool,
) -> Result<Exit, String> {
    let (mut command, _file) = shell(script)?;
    command
        .current_dir(worktree)
        .env("GESPAN_PROJECT_ROOT", main)
        .env("GESPAN_WORKTREE_PATH", worktree)
        .env("GESPAN_BRANCH", script_branch(branch.unwrap_or_default()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::scm::shell::apply_gui_env(&mut command);
    crate::process::isolate(&mut command);
    let mut child = crate::process::spawn(&mut command)
        .map_err(|e| format!("Could not start the script: {e}"))?;
    let (sender, lines) = mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        forward(pipe, sender.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        forward(pipe, sender.clone());
    }
    drop(sender);
    loop {
        if cancelled.load(Ordering::SeqCst) {
            crate::process::terminate(&mut child);
            return Ok(Exit::Cancelled);
        }
        match lines.recv_timeout(POLL) {
            Ok(text) => line(text),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // Both pipes closed; only the exit is left.
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("Could not follow the script: {e}"))?
        {
            drain(&lines, line, cancelled, DRAIN);
            return Ok(Exit::Code(status.code()));
        }
    }
}

/// Reads what's left until `limit`. The deadline is checked every line, because
/// something the script left running may never stop writing.
fn drain(
    lines: &mpsc::Receiver<String>,
    line: &mut dyn FnMut(String),
    cancelled: &AtomicBool,
    limit: Duration,
) {
    let deadline = Instant::now() + limit;
    while !cancelled.load(Ordering::SeqCst) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match lines.recv_timeout(left) {
            Ok(text) => line(text),
            Err(_) => return,
        }
    }
}

fn forward(pipe: impl Read + Send + 'static, sender: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        read_lines(pipe, &mut |line| {
            let _ = sender.send(line);
        });
    });
}

/// Splits on line ends and carriage returns, so each frame of a progress bar is
/// a line, and caps the length so output without line ends can't grow forever.
fn read_lines(mut pipe: impl Read, emit: &mut dyn FnMut(String)) {
    let mut pending = Vec::new();
    let mut chunk = [0; 8192];
    let mut flush = |pending: &mut Vec<u8>| {
        let text = String::from_utf8_lossy(pending);
        let text = text.trim_end();
        if !text.is_empty() {
            emit(text.chars().take(MAX_LINE).collect());
        }
        pending.clear();
    };
    while let Ok(read) = pipe.read(&mut chunk) {
        if read == 0 {
            break;
        }
        for byte in &chunk[..read] {
            if matches!(byte, b'\n' | b'\r') {
                flush(&mut pending);
            } else {
                pending.push(*byte);
                if pending.len() >= MAX_LINE * 4 {
                    flush(&mut pending);
                }
            }
        }
    }
    flush(&mut pending);
}

/// The last lines of a step's output, for a view to keep.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tail {
    lines: VecDeque<String>,
}
impl Tail {
    const MAX: usize = 500;
    pub fn push(&mut self, line: String) {
        if self.lines.len() == Self::MAX {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

#[cfg(test)]
pub(crate) mod test_host {
    use super::*;
    use std::sync::Mutex;

    static HOSTS: LazyLock<Mutex<HashMap<String, Arc<dyn Host>>>> = LazyLock::new(Mutex::default);

    pub(crate) fn get(main: &Path) -> Option<Arc<dyn Host>> {
        HOSTS.lock().ok()?.get(&key(main)).cloned()
    }

    pub(crate) fn put(main: &Path, host: Arc<dyn Host>) {
        HOSTS.lock().unwrap().insert(key(main), host);
    }

    /// Answers failures from a list, and keeps everything it's told.
    #[derive(Default)]
    pub(crate) struct Script {
        pub answers: Mutex<VecDeque<Decision>>,
        pub events: Mutex<Vec<Event>>,
    }
    impl Script {
        pub(crate) fn install(main: &Path, answers: &[Decision]) -> Arc<Self> {
            let host = Arc::new(Self {
                answers: Mutex::new(answers.iter().copied().collect()),
                events: Mutex::default(),
            });
            put(main, host.clone());
            host
        }
        pub(crate) fn output(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Output { line, .. } => Some(line.clone()),
                    _ => None,
                })
                .collect()
        }
    }
    impl Host for Script {
        fn event(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
        fn decide(&self, _: u64) -> Decision {
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("an answer for this failure")
        }
    }
}

#[cfg(test)]
#[path = "worktree_scripts/tests.rs"]
mod tests;
