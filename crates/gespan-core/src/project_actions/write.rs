//! Saves shared actions to `.gespan/tasks.json`.
//!
//! The file is edited as JSON, so other tasks and fields stay. A file with
//! comments is never rewritten, as saving would lose them.
use super::ordered::{Json, Object};
use super::{jsonc, tasks_json, Action};
use std::path::{Component, Path};

const FOLDER: &str = ".gespan";
const FILE: &str = ".gespan/tasks.json";
const VERSION: &str = "2.0.0";

/// What a person types for a shared action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub label: String,
    /// A shell line, run as written.
    pub command: String,
    /// One of [`super::ACTION_ICONS`]; anything else is saved as `play`.
    pub icon: String,
    pub detail: Option<String>,
}

/// A shared action to edit: its label as listed and, when it came from the
/// file, its place there. A repeated label is listed as `Build (2)`, which
/// no task in the file is called, so the place finds it.
#[derive(Clone, Copy, Debug)]
pub struct Shared<'a> {
    pub label: &'a str,
    pub task: Option<usize>,
}

impl<'a> From<&'a str> for Shared<'a> {
    fn from(label: &'a str) -> Self {
        Shared { label, task: None }
    }
}

impl<'a> From<&'a Action> for Shared<'a> {
    fn from(action: &'a Action) -> Self {
        Shared {
            label: &action.label,
            task: action.task,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    /// Rewriting the file would lose its comments. It is left as it was.
    HasComments,
    /// Another shared action already has this label.
    Duplicate(String),
    Io(String),
    /// Something to fix before it can be saved, worded for a person.
    Invalid(String),
    /// The task's command has arguments, per-OS commands or workspace
    /// variables that one shell line can't keep, so it is edited by hand.
    NeedsHandEdit,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, out: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            WriteError::HasComments => {
                out.write_str("This tasks.json has comments, so Gespan won't rewrite it.")
            }
            WriteError::Duplicate(label) => {
                write!(
                    out,
                    "A script named \u{201c}{label}\u{201d} already exists."
                )
            }
            WriteError::NeedsHandEdit => out.write_str(
                "This script uses arguments, per-OS commands or workspace variables; \
                 edit it in tasks.json",
            ),
            WriteError::Io(message) | WriteError::Invalid(message) => out.write_str(message),
        }
    }
}

impl std::error::Error for WriteError {}

/// Adds `draft` as a new shared action.
pub fn add(checkout: &Path, draft: &Draft) -> Result<(), WriteError> {
    let draft = prepare(draft, typed_refs)?;
    edit(checkout, |tasks| {
        taken(tasks, &draft.label, None)?;
        tasks.push(task(&draft, None, false).into());
        Ok(())
    })
}

/// Replaces the shared action `label` with `draft`, in place. Fields Gespan
/// doesn't edit, such as `group`, stay where they are. A task whose command
/// one shell line can't stand for is left alone: see [`WriteError::NeedsHandEdit`].
pub fn update<'a>(
    checkout: &Path,
    shared: impl Into<Shared<'a>>,
    draft: &Draft,
) -> Result<(), WriteError> {
    let shared = shared.into();
    let draft = prepare(draft, typed_refs)?;
    edit(checkout, |tasks| {
        let at = position(tasks, shared)?;
        taken(tasks, &draft.label, Some(at))?;
        let Some(task) = tasks[at].as_object_mut() else {
            return Err(not_found(shared.label));
        };
        if needs_hand_edit(task) {
            return Err(WriteError::NeedsHandEdit);
        }
        // The command is now one shell line, whatever it was before.
        task.remove("script");
        task.insert("label", draft.label.as_str());
        task.insert("type", "shell");
        task.insert("command", draft.command.as_str());
        describe(task, &draft);
        Ok(())
    })
}

/// Changes only the shared action `label`'s name, icon and detail. Its
/// command, arguments, options and per-OS commands stay exactly as written;
/// `draft.command` isn't used.
pub fn relabel<'a>(
    checkout: &Path,
    shared: impl Into<Shared<'a>>,
    draft: &Draft,
) -> Result<(), WriteError> {
    let shared = shared.into();
    if draft.label.trim().is_empty() {
        return Err(WriteError::Invalid("Give the script a name".into()));
    }
    let name = draft.label.trim();
    edit(checkout, |tasks| {
        let at = position(tasks, shared)?;
        taken(tasks, name, Some(at))?;
        let Some(task) = tasks[at].as_object_mut() else {
            return Err(not_found(shared.label));
        };
        task.insert("label", name);
        describe(task, draft);
        Ok(())
    })
}

/// Sets `task`'s detail and icon from `draft`, removing them when it has none.
fn describe(task: &mut Object, draft: &Draft) {
    match detail(draft) {
        Some(detail) => task.insert("detail", detail),
        None => task.remove("detail"),
    }
    match icon(draft) {
        Some(icon) => task.insert("icon", icon),
        None => task.remove("icon"),
    }
}

/// Whether `task` has arguments, a per-OS override or a workspace variable in
/// its command, which a new shell line would lose or bake in.
fn needs_hand_edit(task: &Object) -> bool {
    let command = task.get("command").and_then(Json::as_str).unwrap_or("");
    task.get("args").is_some()
        || ["windows", "osx", "linux"]
            .iter()
            .any(|os| task.get(os).is_some())
        || command.contains("${workspaceFolder}")
        || command.contains("${workspaceFolderBasename}")
        || command.contains("$ZED_WORKTREE_ROOT")
}

/// Removes the shared action `label`.
pub fn remove<'a>(checkout: &Path, shared: impl Into<Shared<'a>>) -> Result<(), WriteError> {
    let shared = shared.into();
    edit(checkout, |tasks| {
        let at = position(tasks, shared)?;
        tasks.remove(at);
        Ok(())
    })
}

/// Saves a detected action to the shared file: its label, command, folder
/// (relative to the checkout), environment, detail, background flag and icon.
/// The checkout's own path is saved as `${workspaceFolder}`.
pub fn save_detected(checkout: &Path, action: &Action) -> Result<(), WriteError> {
    if let Some(reason) = &action.blocked {
        return Err(WriteError::Invalid(format!(
            "\"{}\" can't run: {reason}",
            action.label
        )));
    }
    let draft = prepare(
        &Draft {
            label: action.label.clone(),
            command: action.command.clone(),
            icon: action.icon.to_string(),
            detail: action.detail.clone(),
        },
        file_refs,
    )?;
    let root = tasks_json::normalize(checkout);
    let cwd = tasks_json::normalize(&action.cwd);
    let relative = cwd.strip_prefix(&root).map_err(|_| {
        WriteError::Invalid(format!("\"{}\" runs outside the project", action.label))
    })?;
    let mut options = Object::default();
    if !relative.as_os_str().is_empty() {
        let parts: Vec<_> = relative
            .components()
            .filter_map(|part| match part {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        options.insert("cwd", parts.join("/"));
    }
    if !action.env.is_empty() {
        let mut env = Object::default();
        for (name, value) in &action.env {
            env.insert(name, workspace_folder(checkout, &file_refs(value)?));
        }
        options.insert("env", env);
    }
    let draft = Draft {
        command: workspace_folder(checkout, &draft.command),
        ..draft
    };
    let task = task(&draft, Some(options), action.background);
    edit(checkout, |tasks| {
        taken(tasks, &draft.label, None)?;
        tasks.push(task.into());
        Ok(())
    })
}

/// `line` with this checkout's path written as `${workspaceFolder}`, so the
/// shared file never holds one person's folder, such as their worktree. The
/// path is matched as written and inside the quotes the resolver adds: `sh`'s
/// single quotes and `cmd`'s double quotes keep it as is, and `sh`'s double
/// quotes escape `\`, `"`, `$` and `` ` ``.
fn workspace_folder(checkout: &Path, line: &str) -> String {
    let mut roots: Vec<String> = Vec::new();
    for root in [checkout.to_path_buf(), tasks_json::normalize(checkout)] {
        // A drive or `/` alone would match every path.
        if root.file_name().is_none() {
            continue;
        }
        let root = root.display().to_string();
        if cfg!(not(windows)) {
            let mut escaped = String::with_capacity(root.len());
            for c in root.chars() {
                if matches!(c, '\\' | '"' | '$' | '`') {
                    escaped.push('\\');
                }
                escaped.push(c);
            }
            roots.push(escaped);
        }
        roots.push(root);
    }
    // The longest first, so a shorter spelling never cuts into a longer one.
    roots.sort_by_key(|root| std::cmp::Reverse(root.len()));
    roots.dedup();
    roots
        .iter()
        .fold(line.to_string(), |line, root| replace_path(&line, root))
}

/// `line` with `root` written as `${workspaceFolder}` where it is a whole
/// path or a path's start: followed by the end, a separator, a quote,
/// whitespace or a list separator, never by more of a name like `repo-old`.
fn replace_path(line: &str, root: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(root) {
        let after = &rest[at + root.len()..];
        let whole = after
            .chars()
            .next()
            .is_none_or(|c| matches!(c, '/' | '\\' | '\'' | '"' | ':' | ';') || c.is_whitespace());
        out.push_str(&rest[..at]);
        out.push_str(if whole { "${workspaceFolder}" } else { root });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `draft` as it is saved: its label trimmed and its command spelled as the
/// file spells it by `refs`, or why it can't be.
fn prepare(
    draft: &Draft,
    refs: fn(&str) -> Result<String, WriteError>,
) -> Result<Draft, WriteError> {
    if draft.label.trim().is_empty() {
        return Err(WriteError::Invalid("Give the script a name".into()));
    }
    if draft.command.trim().is_empty() {
        return Err(WriteError::Invalid("Give the script a command".into()));
    }
    Ok(Draft {
        label: draft.label.trim().to_string(),
        command: refs(&draft.command)?,
        ..draft.clone()
    })
}

/// A new task for `draft`, its keys in the order people write them.
fn task(draft: &Draft, options: Option<Object>, background: bool) -> Object {
    let mut task = Object::default();
    task.insert("label", draft.label.as_str());
    task.insert("type", "shell");
    task.insert("command", draft.command.as_str());
    if let Some(options) = options.filter(|options| !options.is_empty()) {
        task.insert("options", options);
    }
    if let Some(detail) = detail(draft) {
        task.insert("detail", detail);
    }
    if background {
        task.insert("isBackground", true);
    }
    if let Some(icon) = icon(draft) {
        task.insert("icon", icon);
    }
    task
}

/// The detail to save, if it isn't blank.
fn detail(draft: &Draft) -> Option<&str> {
    draft.detail.as_deref().filter(|d| !d.trim().is_empty())
}

/// `{ "id": … }`, unless it is the default icon.
fn icon(draft: &Draft) -> Option<Object> {
    let icon = tasks_json::icon(Some(&draft.icon));
    (icon != "play").then(|| {
        let mut object = Object::default();
        object.insert("id", icon);
        object
    })
}

fn not_found(label: &str) -> WriteError {
    WriteError::Invalid(format!("There's no shared script named \"{label}\""))
}

fn label_of(task: &Json) -> Option<&str> {
    match task {
        Json::Object(task) => task.get("label")?.as_str(),
        _ => None,
    }
}

/// Where `shared` is in `tasks`: at its listed place while the task there
/// still goes by that label, or, for `Build (2)`, while it is still the
/// second task called `Build`. Else the task with exactly that label.
fn position(tasks: &[Json], shared: Shared) -> Result<usize, WriteError> {
    let label = shared.label;
    let still_there = |at: usize| {
        let Some(name) = tasks.get(at).and_then(label_of) else {
            return false;
        };
        if name == label {
            return true;
        }
        let n = label
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix(" ("))
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(|n| n.parse::<usize>().ok());
        let before = tasks[..at]
            .iter()
            .filter(|task| label_of(task) == Some(name))
            .count();
        n.is_some_and(|n| before + 1 == n)
    };
    shared
        .task
        .filter(|&at| still_there(at))
        .or_else(|| tasks.iter().position(|task| label_of(task) == Some(label)))
        .ok_or_else(|| not_found(label))
}

/// Fails if a task other than `except` already has `label`.
fn taken(tasks: &[Json], label: &str, except: Option<usize>) -> Result<(), WriteError> {
    let clash = tasks
        .iter()
        .enumerate()
        .any(|(at, task)| Some(at) != except && label_of(task) == Some(label));
    if clash {
        Err(WriteError::Duplicate(label.to_string()))
    } else {
        Ok(())
    }
}

/// Reads the shared file, lets `change` edit its tasks and saves it. Keys
/// keep their order, so only what changed shows in a diff. Nothing is
/// written when anything fails.
fn edit(
    checkout: &Path,
    change: impl FnOnce(&mut Vec<Json>) -> Result<(), WriteError>,
) -> Result<(), WriteError> {
    let folder = checkout.join(FOLDER);
    let path = checkout.join(FILE);
    // A link could point the write anywhere, so only real ones are used.
    let folder_exists = exists(&folder, "isn't a regular folder", |meta| meta.is_dir())?;
    let file_exists =
        folder_exists && exists(&path, "isn't a regular file", |meta| meta.is_file())?;

    let mut root = Json::Object(Object::default());
    if file_exists {
        let text = super::read_capped(&path).map_err(WriteError::Io)?;
        if !text.trim().is_empty() {
            let stripped = jsonc::strip(&text);
            if stripped.had_comments {
                return Err(WriteError::HasComments);
            }
            root = serde_json::from_str(&stripped.json).map_err(|error| {
                WriteError::Invalid(format!("{FILE} isn't valid JSON: {error}"))
            })?;
        }
    }
    let Some(object) = root.as_object_mut() else {
        return Err(WriteError::Invalid(format!("{FILE} isn't a JSON object")));
    };
    object.insert_first("version", VERSION);
    if object.get("tasks").is_none() {
        object.insert("tasks", Json::Array(Vec::new()));
    }
    let Some(tasks) = object.get_mut("tasks").and_then(Json::as_array_mut) else {
        return Err(WriteError::Invalid("\"tasks\" isn't a list".into()));
    };
    change(tasks)?;

    let mut text =
        serde_json::to_string_pretty(&root).map_err(|e| WriteError::Io(e.to_string()))?;
    text.push('\n');
    if !folder_exists {
        std::fs::create_dir(&folder).map_err(io)?;
    }
    save(&folder, &path, &text)?;
    // The file may keep its length and, within a clock tick, its mtime.
    super::forget(&path);
    Ok(())
}

/// Whether `path` is there. It must be a real file or folder as `is_kind`
/// says, never a link.
fn exists(
    path: &Path,
    why: &str,
    is_kind: impl Fn(&std::fs::Metadata) -> bool,
) -> Result<bool, WriteError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if is_kind(&meta) && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(WriteError::Invalid(format!(
            "{} {why}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io(error)),
    }
}

/// Writes `text` to a temp file beside `path`, then renames it over `path`,
/// so a crash never leaves half a file. On Windows the rename replaces the
/// old file too.
fn save(folder: &Path, path: &Path, text: &str) -> Result<(), WriteError> {
    use std::io::Write;
    let mut temp = tempfile::NamedTempFile::new_in(folder).map_err(io)?;
    temp.write_all(text.as_bytes()).map_err(io)?;
    temp.as_file().sync_all().map_err(io)?;
    // A temp file is private; the shared file keeps the mode it had, else 0644.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or(0o644);
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(io)?;
    }
    temp.persist(path).map_err(|error| io(error.error))?;
    Ok(())
}

fn io(error: std::io::Error) -> WriteError {
    WriteError::Io(error.to_string())
}

/// A shell line from [`Action`] as the file spells it: the shell's own
/// `${NAME}` (`%NAME%` on Windows) becomes `${env:NAME}`, which reads back as
/// the same line. Any other `${…}` can't be filled in, so it can't be saved.
fn file_refs(line: &str) -> Result<String, WriteError> {
    // Reading the file fills this in again, so the line would change.
    if line.contains("$ZED_WORKTREE_ROOT") {
        return Err(WriteError::Invalid(
            "Uses $ZED_WORKTREE_ROOT, which Gespan can't fill in".into(),
        ));
    }
    shell_refs(line)
}

/// A shell line a person typed, as the file spells it. The variables the
/// file supports (`${workspaceFolder}`, `${workspaceFolderBasename}`,
/// `${env:NAME}` and `$ZED_WORKTREE_ROOT`) are kept as written; the rest is
/// read as [`file_refs`] reads a resolved line.
fn typed_refs(line: &str) -> Result<String, WriteError> {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let variable = &rest[start..start + len + 1];
        let name = &variable[2..variable.len() - 1];
        out.push_str(&shell_refs(&rest[..start])?);
        let supported = matches!(name, "workspaceFolder" | "workspaceFolderBasename")
            || name
                .strip_prefix("env:")
                .is_some_and(tasks_json::is_env_name);
        if supported {
            out.push_str(variable);
        } else {
            out.push_str(&shell_refs(variable)?);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(&shell_refs(rest)?);
    Ok(out)
}

/// The shell's own references in `line` as `${env:NAME}`, or why one can't be.
fn shell_refs(line: &str) -> Result<String, WriteError> {
    // Windows spells references `%NAME%`, so a `${…}` there is no reference.
    if cfg!(windows) {
        if let Some(start) = line.find("${") {
            if let Some(len) = line[start..].find('}') {
                return Err(WriteError::Invalid(format!(
                    "Uses {}, which Gespan can't fill in",
                    &line[start..start + len + 1]
                )));
            }
        }
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let (open, close) = if cfg!(windows) {
        ("%", "%")
    } else {
        ("${", "}")
    };
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        let Some(len) = after.find(close) else {
            break;
        };
        let name = &after[..len];
        out.push_str(&rest[..start]);
        if tasks_json::is_env_name(name) {
            out.push_str(&format!("${{env:{name}}}"));
        } else if cfg!(windows) {
            // A lone `%` isn't a reference.
            out.push('%');
            rest = after;
            continue;
        } else {
            return Err(WriteError::Invalid(format!(
                "Uses {open}{name}{close}, which Gespan can't fill in"
            )));
        }
        rest = &after[len + close.len()..];
    }
    out.push_str(rest);
    Ok(out)
}
