//! Writes approved preferences and project decisions out to each harness's
//! own files, regenerates them when the log changes, and removes everything
//! Gespan wrote when the log is switched off.
//!
//! Where each harness reads them (verified against each harness's docs on
//! 2026-11-12):
//!
//! | Harness | Preferences (global) | Project |
//! |---|---|---|
//! | Claude Code | `<profile home>/CLAUDE.md`, Section, for every profile | `.claude/rules/gespan-decisions.md`, OwnedFile, inline, decisions only |
//! | Codex | `<profile home>/AGENTS.md`, Section, for every profile | none (injected block) |
//! | OpenCode | `$XDG_CONFIG_HOME/opencode/AGENTS.md` (default `~/.config`), Section, only when it exists | none (injected block) |
//! | Cursor | none (User Rules live in its settings, not a file) | `.cursor/rules/gespan-decisions.mdc`, OwnedFile, `alwaysApply: true`, inline |
//! | Kiro | `~/.kiro/steering/gespan-preferences.md`, OwnedFile, when `~/.kiro` exists | `.kiro/steering/gespan-decisions.md`, OwnedFile, `inclusion: always`, inline |
//!
//! Sources and why:
//! - Claude Code, <https://code.claude.com/docs/en/memory>: user memory is
//!   `~/.claude/CLAUDE.md` (the profile's config dir). The brief's
//!   `CLAUDE.local.md` with an `@<shared path>` import is NOT used: (1) a
//!   `CLAUDE.local.md` "counts", so creating one makes Claude stop reading
//!   a project's `AGENTS.md` under the default `claude-md-or-agents-md`
//!   setting; (2) an import that resolves outside the working directory
//!   (the shared file of a linked worktree lives in the main checkout's
//!   `.git`) needs a one-time interactive approval and otherwise does not
//!   load. Project `.claude/rules/*.md` without `paths` load at launch, do
//!   not count against `AGENTS.md`, and need no approval, so Gespan owns
//!   one there with the content inline.
//! - Codex, <https://learn.chatgpt.com/docs/agent-configuration/agents-md>
//!   (was developers.openai.com/codex/guides/agents-md): global
//!   `$CODEX_HOME/AGENTS.md`; per directory "at most one file", with
//!   `AGENTS.override.md` taking the slot of `AGENTS.md`; no additive local
//!   file and no imports are documented. So no project file.
//! - OpenCode, <https://opencode.ai/docs/rules/>: global
//!   `~/.config/opencode/AGENTS.md`, and `~/.claude/CLAUDE.md` only while
//!   that file is missing ("The first matching file wins"). Creating it would
//!   hide the user's Claude rules from OpenCode, so Gespan only adds its
//!   section to one the user already has. Project rules are `AGENTS.md` (or
//!   `CLAUDE.md`) plus `instructions` in `opencode.json`, both tracked user
//!   files, so no project file.
//! - Cursor, <https://cursor.com/docs/context/rules> and
//!   <https://cursor.com/docs/cli/using>: project rules are
//!   `.cursor/rules/*.mdc` with `description`/`globs`/`alwaysApply`
//!   frontmatter, read by the editor and the CLI; User Rules are set in
//!   Customize -> Rules, not a documented file.
//! - Kiro, <https://kiro.dev/docs/steering/>: workspace `.kiro/steering/`,
//!   global `~/.kiro/steering/`; `inclusion: always` (the default) must be
//!   the first content. The CLI reads workspace steering but the docs do not
//!   list global steering for it, so the project file carries preferences
//!   too.
//!
//! Every body comes from `render_block` over active entries only (never
//! pending), so it never holds a line-leading code fence. Cursor's and
//! Kiro's project files carry preferences as well as decisions (Cursor has
//! no global file, Kiro's CLI reads no global steering); Claude's carries
//! decisions only, since its global `CLAUDE.md` has the preferences.
//! With search by meaning on, the shared file and the project files hold
//! pinned decisions only (plus preferences, as above), like the
//! session-start block; the other decisions reach each turn as its related
//! block.
//!
//! Default homes come from a lookup (`default_homes` in the app, temp dirs in
//! tests), so `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `XDG_CONFIG_HOME` are
//! honoured. Project files are refused when they are symlinks, resolve into
//! the git dir, or are tracked by git; files over 256 KB or that are not
//! regular files are never read or written.
use super::entry::{Entry, Kind as EntryKind, Scope};
use super::files::{
    ensure_excluded, remove_excluded, replace_section, user_edited_section, write_atomic,
    write_atomic_global, BEGIN,
};
use super::render::{render_block, START_BUDGET};
use super::scope::{git_common_dir, scope_for, shared_file, TtlCache};
use super::service;
use super::store::DecisionStore;
use crate::accounts::{Book, Profile};
use crate::provider::Kind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

/// One file a harness reads, and how Gespan writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub harness: Kind,
    pub path: PathBuf,
    pub mode: Mode,
    /// The only directory `path` (or a link it follows) may resolve inside.
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Gespan's marked section of a file the user owns.
    Section,
    /// A whole file Gespan owns (still marked, so it can tell it is its own).
    OwnedFile,
}

#[derive(Debug, Default)]
pub struct Report {
    /// Files created, changed or deleted.
    pub written: Vec<PathBuf>,
    /// Files whose Gespan section the user had edited, now replaced.
    pub user_edits_overwritten: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
}

/// Where a harness keeps its default config: the default account's home for
/// Claude Code and Codex, OpenCode's config folder, Kiro's `~/.kiro`.
/// `None` when it has none.
pub type Homes<'a> = &'a dyn Fn(Kind) -> Option<PathBuf>;

/// The real default homes, honouring `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and
/// `XDG_CONFIG_HOME` the way each harness does.
pub fn default_homes(kind: Kind) -> Option<PathBuf> {
    match kind {
        Kind::Claude | Kind::Codex => Profile::default().home(kind).ok(),
        Kind::OpenCode => std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| crate::accounts::home().map(|h| h.join(".config")))
            .map(|config| config.join("opencode")),
        Kind::Kiro => crate::accounts::home().map(|h| h.join(".kiro")),
        _ => None,
    }
}

/// Files larger than this are never read, so never rewritten.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;

// Runs between reading a file and re-reading it just before the write, so
// tests can edit the file in that window.
#[cfg(test)]
pub(crate) type WriteHook = Box<dyn Fn(&Path)>;
#[cfg(test)]
thread_local! {
    pub(crate) static BEFORE_WRITE: std::cell::RefCell<Option<WriteHook>> =
        const { std::cell::RefCell::new(None) };
}

// A scope whose read fails, so tests can make the store fail for one
// workspace.
#[cfg(test)]
thread_local! {
    pub(crate) static FAIL_SCOPE: std::cell::RefCell<Option<Scope>> =
        const { std::cell::RefCell::new(None) };
}

/// The active entries of `scope`.
fn active_in(store: &DecisionStore, scope: &Scope) -> Result<Vec<Entry>, super::entry::Error> {
    #[cfg(test)]
    if FAIL_SCOPE.with(|f| f.borrow().as_ref() == Some(scope)) {
        return Err(super::entry::Error::Db("injected failure".into()));
    }
    store.active(scope)
}

/// One regeneration or removal at a time, so two never race on the files or
/// on the record of what was written.
static WRITING: Mutex<()> = Mutex::new(());

/// Holds the writing lock, so a test can schedule a run and flip the switch
/// before the run gets the lock.
#[cfg(test)]
pub(crate) fn hold_writing() -> std::sync::MutexGuard<'static, ()> {
    WRITING.lock().unwrap_or_else(|e| e.into_inner())
}

const CLAUDE_RULE: [&str; 3] = [".claude", "rules", "gespan-decisions.md"];
const CURSOR_RULE: [&str; 3] = [".cursor", "rules", "gespan-decisions.mdc"];
const KIRO_STEERING: [&str; 3] = [".kiro", "steering", "gespan-decisions.md"];
const CURSOR_HEADER: &str = "---\ndescription: Gespan decision log (generated; edit decisions in Gespan's My tasks)\nalwaysApply: true\n---\n";
const KIRO_HEADER: &str = "---\ninclusion: always\n---\n";

fn join(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |p, part| p.join(part))
}

/// The global files preferences go to, for every installed harness and every
/// account profile Gespan knows. A profile or harness whose home folder does
/// not exist is skipped: Gespan never creates a harness's home.
pub fn preference_targets(book: &Book, data_dir: &Path, homes: Homes) -> Vec<Target> {
    let mut targets = Vec::new();
    for (kind, file) in [(Kind::Claude, "CLAUDE.md"), (Kind::Codex, "AGENTS.md")] {
        for account in book.list(kind) {
            let dir = if account.is_default() {
                homes(kind)
            } else {
                Profile::new(&account.id, data_dir).home(kind).ok()
            };
            if let Some(dir) = dir.filter(|d| d.is_dir()) {
                targets.push(Target {
                    harness: kind,
                    path: dir.join(file),
                    mode: Mode::Section,
                    root: dir,
                });
            }
        }
    }
    // OpenCode reads ~/.claude/CLAUDE.md only while its own file is missing,
    // so Gespan never creates it.
    if let Some(opencode) = homes(Kind::OpenCode).filter(|d| d.join("AGENTS.md").is_file()) {
        targets.push(Target {
            harness: Kind::OpenCode,
            path: opencode.join("AGENTS.md"),
            mode: Mode::Section,
            root: opencode,
        });
    }
    if let Some(kiro) = homes(Kind::Kiro).filter(|d| d.is_dir()) {
        targets.push(Target {
            harness: Kind::Kiro,
            path: kiro.join("steering").join("gespan-preferences.md"),
            mode: Mode::OwnedFile,
            root: kiro,
        });
    }
    targets
}

/// The work tree's toplevel, or the workspace itself outside git;
/// canonical and without Windows' verbatim prefix, like the paths the
/// exclude file is worked out from.
fn toplevel(workspace: &Path) -> PathBuf {
    let top = crate::git::git_probe(workspace, &["rev-parse", "--show-toplevel"])
        .map(|top| PathBuf::from(top.trim()))
        .filter(|top| !top.as_os_str().is_empty())
        .unwrap_or_else(|| workspace.to_path_buf());
    let canonical = fs::canonicalize(&top).unwrap_or(top);
    super::files::without_verbatim(&canonical)
}

/// The files in a workspace that point each harness at the project's
/// decisions. All are Gespan's own, inside the work tree, and excluded from
/// git when written.
pub fn project_targets(workspace: &Path) -> Vec<Target> {
    let top = toplevel(workspace);
    [
        (Kind::Claude, CLAUDE_RULE),
        (Kind::Cursor, CURSOR_RULE),
        (Kind::Kiro, KIRO_STEERING),
    ]
    .into_iter()
    .map(|(harness, parts)| Target {
        harness,
        path: join(&top, &parts),
        mode: Mode::OwnedFile,
        root: top.clone(),
    })
    .collect()
}

/// Whether each (workspace, harness) has Gespan's project file, remembered
/// for [`GIT_CACHE_TTL`](super::scope::GIT_CACHE_TTL) and forgotten after
/// every regeneration or removal.
static READY: LazyLock<TtlCache<(PathBuf, Kind), bool>> = LazyLock::new(TtlCache::new);

/// Whether `kind` will find Gespan's project file in `workspace`: it exists
/// and is Gespan's. When it is not, the session needs the injected block.
/// Checking runs git, so the answer is kept for a while (see [`READY`]).
pub fn project_file_ready(kind: Kind, workspace: &Path) -> bool {
    project_file_ready_with(kind, workspace, Instant::now(), project_file_ready_now)
}

/// [`project_file_ready`] at `now`, checking a missing or stale answer with
/// `check` (tests count its calls).
pub(crate) fn project_file_ready_with(
    kind: Kind,
    workspace: &Path,
    now: Instant,
    check: impl FnOnce(Kind, &Path) -> bool,
) -> bool {
    READY.get_or((workspace.to_path_buf(), kind), now, || {
        check(kind, workspace)
    })
}

/// [`project_file_ready`] without the cache.
fn project_file_ready_now(kind: Kind, workspace: &Path) -> bool {
    project_targets(workspace)
        .into_iter()
        .find(|t| t.harness == kind)
        .is_some_and(|t| {
            check_project_target(&t.path, &t.root).is_ok()
                && matches!(read_capped(&t.path), Ok(Some(text)) if is_ours(&text))
        })
}

fn header(harness: Option<Kind>) -> &'static str {
    match harness {
        Some(Kind::Cursor) => CURSOR_HEADER,
        Some(Kind::Kiro) => KIRO_HEADER,
        _ => "",
    }
}

/// What Gespan remembers about a file it wrote, so it can tell a user's edit
/// from its own text and clean up files no longer in any target list.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    mode: Mode,
    root: PathBuf,
    /// The work tree whose `info/exclude` lists the file.
    workspace: Option<PathBuf>,
    /// Gespan created the file, so it goes when nothing else is left in it.
    created: bool,
    /// The section as last written.
    last: String,
    /// A global, home-level file (see [`Job::global`]).
    #[serde(default)]
    global: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Written {
    #[serde(default)]
    files: BTreeMap<PathBuf, Record>,
    /// Folders Gespan created for its files; only these are ever removed.
    #[serde(default)]
    dirs: BTreeSet<PathBuf>,
}

struct Registry {
    path: PathBuf,
    written: Written,
}

impl Registry {
    fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("decisions").join("written.json");
        let written = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, written }
    }

    fn save(&self, data_dir: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(&self.written).map_err(io::Error::other)?;
        if fs::read_to_string(&self.path).is_ok_and(|now| now == text) {
            return Ok(());
        }
        write_atomic(&self.path, &text, data_dir)
    }
}

/// Whether Gespan recorded writing any harness file under `data_dir` that
/// it has not removed since: at startup with the log off, these are left
/// over from a run that could not clean up.
pub fn has_recorded_files(data_dir: &Path) -> bool {
    !Registry::load(data_dir).written.files.is_empty()
}

/// A file to bring in line with a body (an empty body withdraws it).
struct Job {
    path: PathBuf,
    mode: Mode,
    root: PathBuf,
    header: &'static str,
    /// Set for files inside a work tree: excluded from git, and checked
    /// against symlinks, the git dir and tracked files.
    workspace: Option<PathBuf>,
    /// Whether the body includes preferences.
    prefs: bool,
    /// A global, home-level preference file: a symlink there may lead to
    /// the user's own file elsewhere ([`write_atomic_global`]).
    global: bool,
}

/// Reads a regular file of at most [`MAX_FILE_BYTES`]; `None` when missing.
/// Anything else is an error, so the file is left alone.
pub(crate) fn read_capped(path: &Path) -> io::Result<Option<String>> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file; not touching it",
        ));
    }
    let too_big = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("larger than {MAX_FILE_BYTES} bytes; not touching it"),
        )
    };
    if meta.len() > MAX_FILE_BYTES {
        return Err(too_big());
    }
    let mut text = String::new();
    fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(too_big());
    }
    Ok(Some(text))
}

fn is_ours(text: &str) -> bool {
    text.lines().any(|line| line.trim() == BEGIN)
}

fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.to_owned())
}

/// A project file must not be a symlink, must not resolve into the git dir
/// or the common dir (hooks live there), and must not be tracked by git.
fn check_project_target(path: &Path, workspace: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(refused("is a symlink; not touching it"));
    }
    let existing = path
        .ancestors()
        .skip(1)
        .find(|a| a.exists())
        .and_then(|a| fs::canonicalize(a).ok());
    let git_dir = crate::git::git_probe(workspace, &["rev-parse", "--absolute-git-dir"])
        .map(|d| PathBuf::from(d.trim()));
    let in_git = git_dir.is_some();
    let git_dirs = [git_dir, git_common_dir(workspace)];
    for dir in git_dirs.into_iter().flatten() {
        let dir = fs::canonicalize(&dir).unwrap_or(dir);
        if existing.as_ref().is_some_and(|e| e.starts_with(&dir)) {
            return Err(refused(
                "resolves inside the git directory; not touching it",
            ));
        }
    }
    let Some(path_text) = path.to_str() else {
        return Err(refused("path is not valid UTF-8"));
    };
    if !in_git {
        return Ok(());
    }
    // Exit 1 is git's "not tracked"; 0 is tracked, and anything else means
    // git could not tell, so the file is left alone either way.
    match ls_files_exit(workspace, path_text) {
        Some(1) => Ok(()),
        Some(0) => Err(refused("is tracked by git; not touching it")),
        other => Err(refused(&format!(
            "git could not tell whether it is tracked ({}); not touching it",
            other.map_or_else(|| "it did not run".to_owned(), |c| format!("exit {c}"))
        ))),
    }
}

// The exit code `git ls-files --error-unmatch` gives, when a test sets one.
#[cfg(test)]
thread_local! {
    pub(crate) static LS_FILES_EXIT: std::cell::RefCell<Option<i32>> =
        const { std::cell::RefCell::new(None) };
}

/// The exit code of `git ls-files --error-unmatch -- <path>` in
/// `workspace`; `None` when git did not run or was killed.
fn ls_files_exit(workspace: &Path, path: &str) -> Option<i32> {
    #[cfg(test)]
    if let Some(code) = LS_FILES_EXIT.with(|c| *c.borrow()) {
        return Some(code);
    }
    crate::git::git_answer(
        workspace,
        &["ls-files", "--error-unmatch", "--", path],
        &[],
        None,
    )
    .ok()
    .and_then(|(code, _)| code)
}

/// Writes or withdraws one file, recording errors and changes in `report`.
fn apply(job: &Job, body: &str, registry: &mut Registry, report: &mut Report) {
    if let Err(error) = try_apply(job, body, registry, report) {
        report.errors.push((job.path.clone(), error.to_string()));
    }
}

fn try_apply(
    job: &Job,
    body: &str,
    registry: &mut Registry,
    report: &mut Report,
) -> io::Result<()> {
    if let Some(workspace) = &job.workspace {
        check_project_target(&job.path, workspace)?;
    }
    let record = registry.written.files.get(&job.path).cloned();
    let existing = read_capped(&job.path)?;
    // What the file should become: `None` deletes it.
    let next_for = |current: Option<&str>| -> io::Result<Option<String>> {
        match job.mode {
            Mode::Section => {
                let current = current.unwrap_or("");
                let next = replace_section(current, body);
                let created = record.as_ref().map_or(existing.is_none(), |r| r.created);
                Ok(if next.is_empty() && created {
                    None
                } else {
                    Some(next)
                })
            }
            Mode::OwnedFile => {
                if current.is_some_and(|text| !is_ours(text)) {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "a file Gespan did not write is in the way",
                    ));
                }
                Ok((!body.is_empty())
                    .then(|| format!("{}{}", job.header, replace_section("", body))))
            }
        }
    };
    if existing.is_none() && body.is_empty() {
        if let Some(workspace) = &job.workspace {
            remove_excluded(workspace, &job.path)?;
        }
        registry.written.files.remove(&job.path);
        return Ok(());
    }
    next_for(existing.as_deref())?;
    #[cfg(test)]
    BEFORE_WRITE.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(&job.path);
        }
    });
    // Re-read just before replacing: an edit made meanwhile is kept.
    let current = read_capped(&job.path)?;
    let next = next_for(current.as_deref())?;
    if let (Some(_), Some(workspace)) = (&next, &job.workspace) {
        // Excluded before it exists, so it never shows in `git status`.
        ensure_excluded(workspace, &job.path)?;
    }
    // Only a regeneration replaces an edit; a removal hands the file back.
    if let (false, Some(now), Some(last), Some(next)) = (body.is_empty(), &current, &record, &next)
    {
        if user_edited_section(now, &last.last) && now != next {
            report.user_edits_overwritten.push(job.path.clone());
        }
    }
    match next {
        Some(next) => {
            if current.as_deref() != Some(next.as_str()) {
                let made = missing_dirs(&job.path, &job.root);
                if job.global {
                    write_atomic_global(&job.path, &next, &job.root)?;
                } else {
                    write_atomic(&job.path, &next, &job.root)?;
                }
                registry.written.dirs.extend(made);
                report.written.push(job.path.clone());
            }
            if body.is_empty() {
                registry.written.files.remove(&job.path);
            } else {
                let created = record.as_ref().map_or(current.is_none(), |r| r.created);
                registry.written.files.insert(
                    job.path.clone(),
                    Record {
                        mode: job.mode,
                        root: job.root.clone(),
                        workspace: job.workspace.clone(),
                        created,
                        last: replace_section("", body),
                        global: job.global,
                    },
                );
            }
        }
        None => {
            if current.is_some() {
                fs::remove_file(&job.path)?;
                report.written.push(job.path.clone());
                remove_created_dirs(&job.path, &job.root, &mut registry.written.dirs);
            }
            if let Some(workspace) = &job.workspace {
                remove_excluded(workspace, &job.path)?;
            }
            registry.written.files.remove(&job.path);
        }
    }
    Ok(())
}

/// The folders above `path`, below `root`, that do not exist yet: the ones
/// writing it will create.
fn missing_dirs(path: &Path, root: &Path) -> Vec<PathBuf> {
    path.ancestors()
        .skip(1)
        .take_while(|dir| *dir != root && dir.starts_with(root) && !dir.exists())
        .map(Path::to_path_buf)
        .collect()
}

/// Removes the empty folders above `path` that Gespan created, deepest
/// first, stopping at the first one it did not create or that is not empty.
fn remove_created_dirs(path: &Path, root: &Path, dirs: &mut BTreeSet<PathBuf>) {
    for dir in path.ancestors().skip(1) {
        if dir == root || !dir.starts_with(root) || !dirs.contains(dir) {
            return;
        }
        if fs::remove_dir(dir).is_err() {
            return;
        }
        dirs.remove(dir);
    }
}

fn preference_jobs(book: &Book, data_dir: &Path, homes: Homes) -> Vec<Job> {
    preference_targets(book, data_dir, homes)
        .into_iter()
        .map(|t| Job {
            header: header((t.mode == Mode::OwnedFile).then_some(t.harness)),
            path: t.path,
            mode: t.mode,
            root: t.root,
            workspace: None,
            prefs: true,
            global: true,
        })
        .collect()
}

/// The shared file, then the workspace's own files.
fn project_jobs(workspace: &Path, data_dir: &Path) -> Vec<Job> {
    let shared = shared_file(workspace, data_dir);
    let shared_root = git_common_dir(workspace).unwrap_or_else(|| data_dir.join("decisions"));
    let mut jobs = vec![Job {
        path: shared,
        mode: Mode::OwnedFile,
        root: shared_root,
        header: "",
        workspace: None,
        prefs: true,
        global: false,
    }];
    jobs.extend(project_targets(workspace).into_iter().map(|t| Job {
        header: header(Some(t.harness)),
        workspace: Some(t.root.clone()),
        // Claude Code has its preferences in its global CLAUDE.md.
        prefs: t.harness != Kind::Claude,
        global: false,
        path: t.path,
        mode: t.mode,
        root: t.root,
    }));
    jobs
}

fn body(prefs: &[Entry], decisions: &[Entry]) -> String {
    if prefs.is_empty() && decisions.is_empty() {
        return String::new();
    }
    render_block(prefs, decisions, START_BUDGET).text
}

/// Jobs for files Gespan recorded writing that `jobs` no longer covers (a
/// removed account, a closed project).
fn recorded_leftovers(registry: &Registry, jobs: &[Job]) -> Vec<Job> {
    registry
        .written
        .files
        .iter()
        .filter(|(path, _)| !jobs.iter().any(|job| &job.path == *path))
        .map(|(path, record)| Job {
            path: path.clone(),
            mode: record.mode,
            root: record.root.clone(),
            header: "",
            workspace: record.workspace.clone(),
            prefs: false,
            global: record.global,
        })
        .collect()
}

/// Rewrites every harness file from the active entries: preferences into
/// every profile's global files, and each workspace's decisions into its
/// shared file and the files each harness reads. Never writes pending
/// entries. Files left with nothing to say, and files Gespan wrote for a
/// profile or workspace no longer given, are withdrawn. Does nothing when
/// the log is off by the time it gets to run.
pub fn regenerate(
    store: &DecisionStore,
    book: &Book,
    data_dir: &Path,
    homes: Homes,
    workspaces: &[PathBuf],
) -> Report {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut report = Report::default();
    if !service::enabled() {
        return report;
    }
    if let Err(error) = fs::create_dir_all(data_dir.join("decisions")) {
        report
            .errors
            .push((data_dir.to_path_buf(), error.to_string()));
        return report;
    }
    let mut registry = Registry::load(data_dir);
    let prefs: Vec<Entry> = match store.active(&Scope::Global) {
        Ok(entries) => entries
            .into_iter()
            .filter(|e| e.kind == EntryKind::Preference)
            .collect(),
        Err(error) => {
            report
                .errors
                .push((data_dir.to_path_buf(), error.to_string()));
            return report;
        }
    };
    let by_meaning = store.embedder().is_some();
    let mut done: Vec<Job> = Vec::new();
    let text = body(&prefs, &[]);
    for job in preference_jobs(book, data_dir, homes) {
        apply(&job, &text, &mut registry, &mut report);
        done.push(job);
    }
    for workspace in workspaces {
        let entries = match active_in(store, &scope_for(workspace)) {
            Ok(entries) => entries,
            Err(error) => {
                report.errors.push((workspace.clone(), error.to_string()));
                // Left as they are: counted as done, so the leftovers pass
                // below does not withdraw them for want of a body.
                done.extend(project_jobs(workspace, data_dir));
                continue;
            }
        };
        let (prefs, decisions): (Vec<Entry>, Vec<Entry>) = entries
            .into_iter()
            .partition(|e| e.kind == EntryKind::Preference);
        // With search by meaning on, sessions start with the preferences and
        // the pinned decisions and get the rest per turn; the files say the
        // same as the start block.
        let decisions: Vec<Entry> = decisions
            .into_iter()
            .filter(|e| !by_meaning || e.pinned)
            .collect();
        let full = body(&prefs, &decisions);
        let decisions_only = body(&[], &decisions);
        for job in project_jobs(workspace, data_dir) {
            let text = if job.prefs { &full } else { &decisions_only };
            apply(&job, text, &mut registry, &mut report);
            done.push(job);
        }
    }
    for job in recorded_leftovers(&registry, &done) {
        apply(&job, "", &mut registry, &mut report);
    }
    // Project files may have appeared or gone: sessions look again.
    READY.clear();
    if let Err(error) = registry.save(data_dir) {
        report
            .errors
            .push((registry.path.clone(), error.to_string()));
    }
    report
}

/// The off switch: removes every Gespan section and Gespan-owned file, for
/// the profiles and workspaces given and for any file Gespan recorded
/// writing before (a removed account, a closed project, an old home). The
/// user's text is kept; a file Gespan created is deleted once nothing else
/// is in it. Does nothing when the log is back on by the time it runs.
pub fn remove_all(book: &Book, data_dir: &Path, homes: Homes, workspaces: &[PathBuf]) -> Report {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut report = Report::default();
    if service::enabled() {
        return report;
    }
    let mut registry = Registry::load(data_dir);
    let mut jobs = preference_jobs(book, data_dir, homes);
    for workspace in workspaces {
        jobs.extend(project_jobs(workspace, data_dir));
    }
    let leftovers = recorded_leftovers(&registry, &jobs);
    jobs.extend(leftovers);
    for job in &jobs {
        apply(job, "", &mut registry, &mut report);
    }
    READY.clear();
    if registry.path.exists() {
        if let Err(error) = registry.save(data_dir) {
            report
                .errors
                .push((registry.path.clone(), error.to_string()));
        }
    }
    report
}
