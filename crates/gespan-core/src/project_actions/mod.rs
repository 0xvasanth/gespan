//! A project's everyday commands, read from the files that already list them:
//! Gespan's own `.gespan/tasks.json`, other tools' configs and task runners.
//!
//! Reading a file never runs anything. An action runs only when a person or an
//! agent asks for it.
use serde::Serialize;
use sources::agent_apps::{ClaudeLaunch, Codex, Conductor, Cursor, Superset};
use sources::editors::{Gespan, VsCode, Zed};
use sources::runners::{
    package_manager, CargoAlias, Deno, Just, Makefile, Mise, PackageJson, Procfile,
};
use sources::Source;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::SystemTime,
};

pub(crate) mod jsonc;
pub mod sources;
pub(crate) mod tasks_json;

/// Every source, highest priority first. Gespan's own file leads, then agent
/// apps, editors and task runners. Personal actions list right after Gespan's.
pub static SOURCES: &[&dyn Source] = &[
    &Gespan,
    &Conductor,
    &Superset,
    &Cursor,
    &Codex,
    &ClaudeLaunch,
    &VsCode,
    &Zed,
    &PackageJson,
    &Deno,
    &Mise,
    &Just,
    &CargoAlias,
    &Procfile,
    &Makefile,
];

const PERSONAL: &str = "personal";
const PERSONAL_FILE: &str = "Personal";

/// The icons an action can show. Anything else shows as `play`.
pub const ACTION_ICONS: [&str; 6] = ["play", "test-tube", "hammer", "rocket", "bug", "package"];

/// An action ready to list and run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// `<source id>:<label>`, such as `gespan:Test` or `package.json:dev`.
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub source: &'static str,
    /// The file it came from, as shown, such as `.vscode/tasks.json`.
    pub file: String,
    /// The shell line to run.
    pub command: String,
    /// Absolute, inside the checkout.
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// It keeps running, such as a dev server.
    pub background: bool,
    /// One of [`ACTION_ICONS`].
    pub icon: &'static str,
    /// Why it can't run. It is still listed, greyed out.
    pub blocked: Option<String>,
    /// Shown beside it, such as "Dependencies aren't run".
    pub note: Option<String>,
}

/// What a source parser yields, before variables, quoting and `cwd` resolve.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    /// False for a `process` task, whose program is quoted like its args.
    pub shell: bool,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub detail: Option<String>,
    pub background: bool,
    pub icon: Option<String>,
    pub hide: bool,
    pub blocked: Option<String>,
    pub note: Option<String>,
}

/// What parsing and resolving need to know about the project.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    /// The active checkout: the session's worktree, else the project root.
    pub checkout: &'a Path,
    /// `bun`, `pnpm`, `yarn` or `npm`, from the lockfile.
    pub package_manager: &'static str,
}

/// A project's actions, and the files that couldn't be read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub actions: Vec<Action>,
    pub errors: Vec<SourceError>,
}

/// A file that couldn't be read or parsed. Its source lists nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceError {
    /// As shown, such as `.vscode/tasks.json`.
    pub file: String,
    pub reason: String,
}

/// An action a person saved for a project in Gespan, outside the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Personal {
    pub id: String,
    pub name: String,
    /// A shell line, run as written.
    pub command: String,
    pub icon: String,
}

static PERSONAL_ACTIONS: LazyLock<Mutex<HashMap<String, Vec<Personal>>>> =
    LazyLock::new(Default::default);

/// A parsed file, kept until its mtime or length changes.
struct Cached {
    modified: SystemTime,
    len: u64,
    /// `package.json` scripts and npm tasks run with it, so a new lockfile
    /// parses the file again.
    package_manager: &'static str,
    found: Vec<Found>,
}

/// Keyed by absolute file path.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Cached>>> = LazyLock::new(Default::default);

#[cfg(test)]
thread_local! {
    static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many files this thread has read, so tests can see the cache work.
#[cfg(test)]
pub(crate) fn reads() -> usize {
    READS.with(std::cell::Cell::get)
}

/// Replaces `project`'s personal actions.
pub fn set_personal(project: &str, actions: Vec<Personal>) {
    let mut all = PERSONAL_ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
    if actions.is_empty() {
        all.remove(project);
    } else {
        all.insert(project.to_string(), actions);
    }
}

/// `project`'s actions as found in `checkout`, its active worktree or root.
/// Only reads files; a file that hasn't changed since last time isn't read.
pub fn detect(project: &str, checkout: &Path) -> Listing {
    let ctx = Ctx {
        checkout,
        package_manager: package_manager(checkout),
    };
    let mut listing = Listing::default();
    let mut found: Vec<Action> = Vec::new();
    for (at, source) in SOURCES.iter().enumerate() {
        match read(&ctx, *source) {
            None => {}
            Some((file, Ok(entries))) => found.extend(
                entries
                    .into_iter()
                    .filter(|entry| !entry.hide)
                    .map(|entry| tasks_json::resolve(&ctx, source.id(), file, entry)),
            ),
            Some((file, Err(reason))) => listing.errors.push(SourceError {
                file: file.to_string(),
                reason,
            }),
        }
        if at == 0 {
            found.extend(personal(project, checkout));
        }
    }

    let gespan = Gespan.id();
    let gespan_labels: HashSet<String> = found
        .iter()
        .filter(|action| action.source == gespan)
        .map(|action| action.label.clone())
        .collect();
    let mut commands: HashSet<(String, PathBuf)> = HashSet::new();
    let mut ids: HashSet<String> = HashSet::new();
    for mut action in found {
        if action.source != gespan && gespan_labels.contains(&action.label) {
            continue;
        }
        // A blocked row with no command is never the same as another.
        if !action.command.trim().is_empty()
            && !commands.insert((action.command.clone(), action.cwd.clone()))
        {
            continue;
        }
        if ids.contains(&action.id) {
            let (label, id) = (action.label.clone(), action.id.clone());
            let mut n = 2;
            while ids.contains(&format!("{id} ({n})")) {
                n += 1;
            }
            action.label = format!("{label} ({n})");
            action.id = format!("{id} ({n})");
        }
        ids.insert(action.id.clone());
        listing.actions.push(action);
    }
    listing
}

fn personal(project: &str, checkout: &Path) -> Vec<Action> {
    let all = PERSONAL_ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let cwd = tasks_json::normalize(checkout);
    all.get(project)
        .into_iter()
        .flatten()
        .map(|personal| Action {
            id: format!("{PERSONAL}:{}", personal.id),
            label: personal.name.clone(),
            detail: None,
            source: PERSONAL,
            file: PERSONAL_FILE.to_string(),
            command: personal.command.clone(),
            cwd: cwd.clone(),
            env: Vec::new(),
            background: false,
            icon: tasks_json::icon(Some(&personal.icon)),
            blocked: None,
            note: None,
        })
        .collect()
}

/// The first of `source`'s files that exists, and what it lists. A file that
/// is gone drops its cache entry.
fn read(ctx: &Ctx, source: &dyn Source) -> Option<(&'static str, Result<Vec<Found>, String>)> {
    for file in source.files() {
        let path = ctx.checkout.join(file);
        let key = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        let meta = match std::fs::metadata(&path) {
            Ok(meta) if !meta.is_dir() => meta,
            Ok(_) => {
                cache().remove(&key);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                cache().remove(&key);
                continue;
            }
            Err(error) => {
                cache().remove(&key);
                return Some((file, Err(error.to_string())));
            }
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let len = meta.len();
        if let Some(cached) = cache().get(&key).filter(|cached| {
            cached.modified == modified
                && cached.len == len
                && cached.package_manager == ctx.package_manager
        }) {
            return Some((file, Ok(cached.found.clone())));
        }
        #[cfg(test)]
        READS.with(|reads| reads.set(reads.get() + 1));
        let parsed = std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| source.parse(ctx, file, &text));
        match &parsed {
            Ok(found) => {
                let cached = Cached {
                    modified,
                    len,
                    package_manager: ctx.package_manager,
                    found: found.clone(),
                };
                cache().insert(key, cached);
            }
            Err(_) => {
                cache().remove(&key);
            }
        }
        return Some((file, parsed));
    }
    None
}

fn cache() -> std::sync::MutexGuard<'static, HashMap<PathBuf, Cached>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests;
