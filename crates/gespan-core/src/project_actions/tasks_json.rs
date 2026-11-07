//! VS Code's `tasks.json` (version 2.0.0), which `.gespan/tasks.json` uses and
//! VS Code and Zed read too. Only the fields Gespan can honour are read.
use super::{jsonc, Action, Ctx, Found, ACTION_ICONS};
use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};

const ONLY_DEPENDS_ON: &str = "Only runs other tasks, which Gespan doesn't do yet";
const DEPENDENCIES_NOT_RUN: &str = "Dependencies aren't run";
const OUTSIDE: &str = "Runs outside the project";
const NO_COMMAND: &str = "Has no command Gespan can run";

/// The tasks in `text`, with this OS's `windows`/`osx`/`linux` overrides.
pub(crate) fn parse(ctx: &Ctx, text: &str) -> Result<Vec<Found>, String> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    parse_for(os, ctx, text)
}

/// [`parse`] as if on `os`: `windows`, `osx` or `linux`.
pub(crate) fn parse_for(os: &str, ctx: &Ctx, text: &str) -> Result<Vec<Found>, String> {
    let value: Value =
        serde_json::from_str(&jsonc::strip(text).json).map_err(|error| error.to_string())?;
    let Some(root) = value.as_object() else {
        return Err("It isn't a JSON object".into());
    };
    match root.get("tasks") {
        None => Ok(Vec::new()),
        Some(Value::Array(tasks)) => Ok(tasks
            .iter()
            .filter_map(|task| found(os, ctx, task.as_object()?))
            .collect()),
        Some(_) => Err("\"tasks\" isn't a list".into()),
    }
}

fn found(os: &str, ctx: &Ctx, task: &Map<String, Value>) -> Option<Found> {
    let over = task.get(os).and_then(Value::as_object);
    let field = |key: &str| {
        over.and_then(|over| over.get(key))
            .or_else(|| task.get(key))
    };
    let option = |key: &str| {
        over.and_then(|over| options(over, key))
            .or_else(|| options(task, key))
    };
    let mut found = Found {
        shell: true,
        ..Found::default()
    };
    match task.get("type").and_then(Value::as_str) {
        Some("npm") => {
            let script = task.get("script").and_then(Value::as_str)?;
            found.command = format!("{} run {script}", ctx.package_manager);
            found.label = format!("npm: {script}");
        }
        kind => {
            found.shell = kind != Some("process");
            found.command = field("command").and_then(word).unwrap_or_default();
            found.args = match field("args") {
                Some(Value::Array(args)) => args.iter().filter_map(word).collect(),
                _ => Vec::new(),
            };
        }
    }
    if let Some(label) = task.get("label").and_then(Value::as_str) {
        found.label = label.to_string();
    }
    if found.label.is_empty() {
        return None;
    }
    found.cwd = option("cwd").and_then(Value::as_str).map(str::to_string);
    if let Some(Value::Object(env)) = option("env") {
        found.env = env
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_string())))
            .collect();
    }
    let text = |key: &str| task.get(key).and_then(Value::as_str).map(str::to_string);
    let flag = |key: &str| task.get(key).and_then(Value::as_bool).unwrap_or(false);
    found.detail = text("detail");
    found.background = flag("isBackground");
    found.hide = flag("hide");
    found.icon = task
        .get("icon")
        .and_then(|icon| icon.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if task
        .get("dependsOn")
        .is_some_and(|depends| !depends.is_null())
    {
        if found.command.trim().is_empty() {
            found.blocked = Some(ONLY_DEPENDS_ON.into());
        } else {
            found.note = Some(DEPENDENCIES_NOT_RUN.into());
        }
    }
    // Such as a `gulp` or `typescript` task, which only VS Code knows how to run.
    if found.command.trim().is_empty() && found.blocked.is_none() {
        found.blocked = Some(NO_COMMAND.into());
    }
    Some(found)
}

fn options<'a>(task: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    task.get("options")?.get(key)
}

/// A command or an arg: a string, or VS Code's `{ "value": ... }` form.
fn word(value: &Value) -> Option<String> {
    match value {
        Value::String(word) => Some(word.clone()),
        Value::Object(word) => word.get("value")?.as_str().map(str::to_string),
        _ => None,
    }
}

/// Fills in variables, quotes and joins the command, and settles `cwd`.
pub(crate) fn resolve(ctx: &Ctx, source: &'static str, file: &str, found: Found) -> Action {
    let mut blocked = found.blocked;
    let mut fill = |text: &str| match variables(ctx, text) {
        Ok(filled) => filled,
        Err(variable) => {
            blocked.get_or_insert(format!("Uses {variable}, which Gespan can't fill in"));
            text.to_string()
        }
    };
    let program = fill(&found.command);
    let args: Vec<String> = found.args.iter().map(|arg| fill(arg)).collect();
    let env = found
        .env
        .iter()
        .map(|(name, value)| (name.clone(), fill(value)))
        .collect();
    let cwd = found.cwd.as_deref().map(&mut fill);

    let mut words = vec![if found.shell {
        Some(program)
    } else {
        quote(&program)
    }];
    words.extend(args.iter().map(|arg| quote(arg)));
    let command = match words.into_iter().collect::<Option<Vec<_>>>() {
        Some(words) => words.join(" "),
        None => {
            blocked.get_or_insert("Has an argument Gespan can't quote".into());
            found.command.clone()
        }
    };

    let checkout = normalize(ctx.checkout);
    let cwd = match cwd {
        None => checkout,
        Some(cwd) => {
            let cwd = normalize(&checkout.join(cwd));
            if cwd.starts_with(&checkout) {
                cwd
            } else {
                blocked.get_or_insert(OUTSIDE.into());
                checkout
            }
        }
    };
    let icon = found
        .icon
        .and_then(|icon| ACTION_ICONS.into_iter().find(|known| *known == icon))
        .unwrap_or("play");
    Action {
        id: format!("{source}:{}", found.label),
        label: found.label,
        detail: found.detail,
        source,
        file: file.to_string(),
        command,
        cwd,
        env,
        background: found.background,
        icon,
        blocked,
        note: found.note,
    }
}

/// `text` with the supported variables filled in, or the first `${…}` that
/// isn't supported.
fn variables(ctx: &Ctx, text: &str) -> Result<String, String> {
    let root = ctx.checkout.display().to_string();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        let variable = &rest[start..start + len + 1];
        let name = &variable[2..variable.len() - 1];
        match name {
            "workspaceFolder" => out.push_str(&root),
            "workspaceFolderBasename" => out.push_str(
                &ctx.checkout
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default(),
            ),
            _ => match name.strip_prefix("env:") {
                Some(var) => out.push_str(&std::env::var(var).unwrap_or_default()),
                None => return Err(variable.to_string()),
            },
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    Ok(out.replace("$ZED_WORKTREE_ROOT", &root))
}

/// One word for the shell that runs actions: `sh` on Unix, `cmd` on Windows.
fn quote(word: &str) -> Option<String> {
    if cfg!(windows) {
        let plain = !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:\\@+".contains(c));
        Some(if plain {
            word.to_string()
        } else {
            format!("\"{}\"", word.replace('"', "\"\""))
        })
    } else {
        shlex::try_quote(word).ok().map(|word| word.into_owned())
    }
}

/// `path` with `.` and `..` worked out, without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
