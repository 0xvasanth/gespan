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

#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    /// Rewriting the file would lose its comments. It is left as it was.
    HasComments,
    /// Another shared action already has this label.
    Duplicate(String),
    Io(String),
    /// Something to fix before it can be saved, worded for a person.
    Invalid(String),
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
                    "An action named \u{201c}{label}\u{201d} already exists."
                )
            }
            WriteError::Io(message) | WriteError::Invalid(message) => out.write_str(message),
        }
    }
}

impl std::error::Error for WriteError {}

/// Adds `draft` as a new shared action.
pub fn add(checkout: &Path, draft: &Draft) -> Result<(), WriteError> {
    let draft = prepare(draft)?;
    edit(checkout, |tasks| {
        taken(tasks, &draft.label, None)?;
        tasks.push(task(&draft, None, false).into());
        Ok(())
    })
}

/// Replaces the shared action `label` with `draft`, in place. Fields Gespan
/// doesn't edit, such as `group`, stay where they are.
pub fn update(checkout: &Path, label: &str, draft: &Draft) -> Result<(), WriteError> {
    let draft = prepare(draft)?;
    edit(checkout, |tasks| {
        let at = position(tasks, label)?;
        taken(tasks, &draft.label, Some(at))?;
        let Some(task) = tasks[at].as_object_mut() else {
            return Err(not_found(label));
        };
        // The command is now one shell line, whatever it was before.
        task.remove("args");
        task.remove("script");
        for os in ["windows", "osx", "linux"] {
            if let Some(over) = task.get_mut(os).and_then(Json::as_object_mut) {
                over.remove("command");
                over.remove("args");
            }
        }
        task.insert("label", draft.label.as_str());
        task.insert("type", "shell");
        task.insert("command", draft.command.as_str());
        match detail(&draft) {
            Some(detail) => task.insert("detail", detail),
            None => task.remove("detail"),
        }
        match icon(&draft) {
            Some(icon) => task.insert("icon", icon),
            None => task.remove("icon"),
        }
        Ok(())
    })
}

/// Removes the shared action `label`.
pub fn remove(checkout: &Path, label: &str) -> Result<(), WriteError> {
    edit(checkout, |tasks| {
        let at = position(tasks, label)?;
        tasks.remove(at);
        Ok(())
    })
}

/// Saves a detected action to the shared file: its label, command, folder
/// (relative to the checkout), environment, detail, background flag and icon.
pub fn save_detected(checkout: &Path, action: &Action) -> Result<(), WriteError> {
    if let Some(reason) = &action.blocked {
        return Err(WriteError::Invalid(format!(
            "\"{}\" can't run: {reason}",
            action.label
        )));
    }
    let draft = prepare(&Draft {
        label: action.label.clone(),
        command: action.command.clone(),
        icon: action.icon.to_string(),
        detail: action.detail.clone(),
    })?;
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
            env.insert(name, file_refs(value)?);
        }
        options.insert("env", env);
    }
    let task = task(&draft, Some(options), action.background);
    edit(checkout, |tasks| {
        taken(tasks, &draft.label, None)?;
        tasks.push(task.into());
        Ok(())
    })
}

/// `draft` as it is saved: its label trimmed and its command spelled as the
/// file spells it, or why it can't be.
fn prepare(draft: &Draft) -> Result<Draft, WriteError> {
    if draft.label.trim().is_empty() {
        return Err(WriteError::Invalid("Give the action a name".into()));
    }
    if draft.command.trim().is_empty() {
        return Err(WriteError::Invalid("Give the action a command".into()));
    }
    Ok(Draft {
        label: draft.label.trim().to_string(),
        command: file_refs(&draft.command)?,
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
    WriteError::Invalid(format!("There's no shared action named \"{label}\""))
}

fn label_of(task: &Json) -> Option<&str> {
    match task {
        Json::Object(task) => task.get("label")?.as_str(),
        _ => None,
    }
}

fn position(tasks: &[Json], label: &str) -> Result<usize, WriteError> {
    tasks
        .iter()
        .position(|task| label_of(task) == Some(label))
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
