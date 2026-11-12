//! Writes approved preferences and project decisions out to each harness's
//! own files, regenerates them when the log changes, and removes everything
//! Gespan wrote when the log is switched off.
//!
//! Where each harness reads them (verified against each harness's docs on
//! 2026-11-12):
//!
//! | Harness | Preferences (global) | Project |
//! |---|---|---|
//! | Claude Code | `<profile home>/CLAUDE.md`, Section, for every profile | `.claude/rules/gespan-decisions.md`, OwnedFile, inline |
//! | Codex | `<profile home>/AGENTS.md`, Section, for every profile | none (injected block) |
//! | OpenCode | `~/.config/opencode/AGENTS.md`, Section, only when it exists | none (injected block) |
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
//! pending), so it never holds a line-leading code fence. Project files
//! carry preferences as well as decisions: Cursor has no global file and
//! Kiro's CLI reads no global steering.
use super::files::{
    ensure_excluded, remove_excluded, replace_section, user_edited_section, write_atomic, BEGIN,
};
use super::scope::{git_common_dir, scope_for, shared_file};
use crate::accounts::{Book, Profile};
use crate::provider::Kind;
use gespan_memory::{render_block, Entry, Kind as EntryKind, Scope, Store, START_BUDGET};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

// Runs between reading a file and re-reading it just before the write, so
// tests can edit the file in that window.
#[cfg(test)]
pub(crate) type WriteHook = Box<dyn Fn(&Path)>;
#[cfg(test)]
thread_local! {
    pub(crate) static BEFORE_WRITE: std::cell::RefCell<Option<WriteHook>> =
        const { std::cell::RefCell::new(None) };
}

/// One regeneration or removal at a time, so two never race on the files or
/// on the record of what was written.
static WRITING: Mutex<()> = Mutex::new(());

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

/// The default account's config home, under `home`, for a provider with
/// per-account homes.
fn default_home(kind: Kind, home: &Path) -> Option<PathBuf> {
    let profile = kind.spec().accounts?.profile?;
    Some(home.join(profile.default_dir))
}

/// The global files preferences go to, for every installed harness and every
/// account profile Gespan knows. A profile or harness whose home folder does
/// not exist is skipped: Gespan never creates a harness's home.
pub fn preference_targets(book: &Book, data_dir: &Path, home: &Path) -> Vec<Target> {
    let mut targets = Vec::new();
    for (kind, file) in [(Kind::Claude, "CLAUDE.md"), (Kind::Codex, "AGENTS.md")] {
        for account in book.list(kind) {
            let dir = if account.is_default() {
                default_home(kind, home)
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
    let opencode = home.join(".config").join("opencode");
    if opencode.join("AGENTS.md").is_file() {
        targets.push(Target {
            harness: Kind::OpenCode,
            path: opencode.join("AGENTS.md"),
            mode: Mode::Section,
            root: opencode,
        });
    }
    let kiro = home.join(".kiro");
    if kiro.is_dir() {
        targets.push(Target {
            harness: Kind::Kiro,
            path: kiro.join("steering").join("gespan-preferences.md"),
            mode: Mode::OwnedFile,
            root: kiro,
        });
    }
    targets
}

/// The work tree's toplevel, or the workspace itself outside git.
fn toplevel(workspace: &Path) -> PathBuf {
    crate::git::git_probe(workspace, &["rev-parse", "--show-toplevel"])
        .map(|top| PathBuf::from(top.trim()))
        .filter(|top| !top.as_os_str().is_empty())
        .unwrap_or_else(|| workspace.to_path_buf())
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
}

struct Registry {
    path: PathBuf,
    records: BTreeMap<PathBuf, Record>,
}

impl Registry {
    fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("decisions").join("written.json");
        let records = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, records }
    }

    fn save(&self, data_dir: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(&self.records).map_err(io::Error::other)?;
        if fs::read_to_string(&self.path).is_ok_and(|now| now == text) {
            return Ok(());
        }
        write_atomic(&self.path, &text, data_dir)
    }
}

/// A file to bring in line with `body` (an empty body withdraws it).
struct Job {
    path: PathBuf,
    mode: Mode,
    root: PathBuf,
    header: &'static str,
    workspace: Option<PathBuf>,
}

fn read_existing(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn is_ours(text: &str) -> bool {
    text.lines().any(|line| line.trim() == BEGIN)
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
    let record = registry.records.get(&job.path).cloned();
    let existing = read_existing(&job.path)?;
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
        registry.records.remove(&job.path);
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
    let current = read_existing(&job.path)?;
    let next = next_for(current.as_deref())?;
    if let (Some(_), Some(workspace)) = (&next, &job.workspace) {
        // Excluded before it exists, so it never shows in `git status`.
        ensure_excluded(workspace, &job.path)?;
    }
    if let (Some(now), Some(last), Some(next)) = (&current, &record, &next) {
        if user_edited_section(now, &last.last) && now != next {
            report.user_edits_overwritten.push(job.path.clone());
        }
    }
    match next {
        Some(next) => {
            if current.as_deref() != Some(next.as_str()) {
                write_atomic(&job.path, &next, &job.root)?;
                report.written.push(job.path.clone());
            }
            if body.is_empty() {
                registry.records.remove(&job.path);
            } else {
                let created = record.as_ref().map_or(current.is_none(), |r| r.created);
                registry.records.insert(
                    job.path.clone(),
                    Record {
                        mode: job.mode,
                        root: job.root.clone(),
                        workspace: job.workspace.clone(),
                        created,
                        last: replace_section("", body),
                    },
                );
            }
        }
        None => {
            if current.is_some() {
                fs::remove_file(&job.path)?;
                report.written.push(job.path.clone());
                if job.mode == Mode::OwnedFile {
                    remove_empty_dirs(&job.path, &job.root);
                }
            }
            if let Some(workspace) = &job.workspace {
                remove_excluded(workspace, &job.path)?;
            }
            registry.records.remove(&job.path);
        }
    }
    Ok(())
}

/// Removes the folders above `path` that are now empty, up to `root`.
fn remove_empty_dirs(path: &Path, root: &Path) {
    for dir in path.ancestors().skip(1) {
        if dir == root || !dir.starts_with(root) || fs::remove_dir(dir).is_err() {
            return;
        }
    }
}

fn preference_jobs(book: &Book, data_dir: &Path, home: &Path) -> Vec<Job> {
    preference_targets(book, data_dir, home)
        .into_iter()
        .map(|t| Job {
            header: header((t.mode == Mode::OwnedFile).then_some(t.harness)),
            path: t.path,
            mode: t.mode,
            root: t.root,
            workspace: None,
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
    }];
    jobs.extend(project_targets(workspace).into_iter().map(|t| Job {
        header: header(Some(t.harness)),
        workspace: Some(t.root.clone()),
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

/// Rewrites every harness file from the active entries: preferences into
/// every profile's global files, and each workspace's decisions into its
/// shared file and the files each harness reads. Never writes pending
/// entries. Files left with nothing to say are withdrawn.
pub fn regenerate(
    store: &Store,
    book: &Book,
    data_dir: &Path,
    home: &Path,
    workspaces: &[PathBuf],
) -> Report {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut report = Report::default();
    if let Err(error) = fs::create_dir_all(data_dir.join("decisions")) {
        report
            .errors
            .push((data_dir.to_path_buf(), error.to_string()));
        return report;
    }
    let mut registry = Registry::load(data_dir);
    match store.active(&Scope::Global) {
        Ok(entries) => {
            let prefs: Vec<Entry> = entries
                .into_iter()
                .filter(|e| e.kind == EntryKind::Preference)
                .collect();
            let text = body(&prefs, &[]);
            for job in preference_jobs(book, data_dir, home) {
                apply(&job, &text, &mut registry, &mut report);
            }
        }
        Err(error) => report
            .errors
            .push((data_dir.to_path_buf(), error.to_string())),
    }
    for workspace in workspaces {
        let entries = match store.active(&scope_for(workspace)) {
            Ok(entries) => entries,
            Err(error) => {
                report.errors.push((workspace.clone(), error.to_string()));
                continue;
            }
        };
        let (prefs, decisions): (Vec<Entry>, Vec<Entry>) = entries
            .into_iter()
            .partition(|e| e.kind == EntryKind::Preference);
        let text = body(&prefs, &decisions);
        for job in project_jobs(workspace, data_dir) {
            apply(&job, &text, &mut registry, &mut report);
        }
    }
    if let Err(error) = registry.save(data_dir) {
        report
            .errors
            .push((registry.path.clone(), error.to_string()));
    }
    report
}

/// The off switch: removes every Gespan section and Gespan-owned file, for
/// the profiles and workspaces given and for any file Gespan recorded
/// writing before (a removed account, a closed project). The user's text is
/// kept; a file Gespan created is deleted once nothing else is in it.
pub fn remove_all(book: &Book, data_dir: &Path, home: &Path, workspaces: &[PathBuf]) -> Report {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut report = Report::default();
    let mut registry = Registry::load(data_dir);
    let mut jobs = preference_jobs(book, data_dir, home);
    for workspace in workspaces {
        jobs.extend(project_jobs(workspace, data_dir));
    }
    for (path, record) in &registry.records {
        if !jobs.iter().any(|job| &job.path == path) {
            jobs.push(Job {
                path: path.clone(),
                mode: record.mode,
                root: record.root.clone(),
                header: "",
                workspace: record.workspace.clone(),
            });
        }
    }
    for job in &jobs {
        apply(job, "", &mut registry, &mut report);
    }
    if registry.path.exists() || !registry.records.is_empty() {
        if let Err(error) = registry.save(data_dir) {
            report
                .errors
                .push((registry.path.clone(), error.to_string()));
        }
    }
    report
}
