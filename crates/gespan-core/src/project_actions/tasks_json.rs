//! VS Code's `tasks.json` (version 2.0.0), which `.gespan/tasks.json` uses and
//! VS Code and Zed read too. Only the fields Gespan can honour are read.
use super::{jsonc, Action, Ctx, Found, ACTION_ICONS};
use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};

const ONLY_DEPENDS_ON: &str = "Only runs other tasks, which Gespan doesn't do yet";
const DEPENDENCIES_NOT_RUN: &str = "Dependencies aren't run";
const OUTSIDE: &str = "Runs outside the project";
pub(crate) const NO_COMMAND: &str = "Has no command Gespan can run";

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
    let fill = |blocked: &mut Option<String>, text: &str| match variables(ctx, text) {
        Ok(pieces) => Some(pieces),
        Err(reason) => {
            blocked.get_or_insert(reason);
            None
        }
    };
    let program = fill(&mut blocked, &found.command);
    let args: Vec<_> = found
        .args
        .iter()
        .map(|arg| fill(&mut blocked, arg))
        .collect();
    let command = match (program, args.into_iter().collect::<Option<Vec<_>>>()) {
        (Some(program), Some(args)) => {
            let mut words = vec![if found.shell {
                Some(shell_line(&program))
            } else {
                quote(&program)
            }];
            words.extend(args.iter().map(|arg| quote(arg)));
            match words.into_iter().collect::<Option<Vec<_>>>() {
                Some(words) => words.join(" "),
                None => {
                    blocked.get_or_insert("Has an argument Gespan can't quote".into());
                    found.command.clone()
                }
            }
        }
        // Already blocked by the variable.
        _ => found.command.clone(),
    };
    let env = found
        .env
        .iter()
        .map(|(name, value)| {
            let value = fill(&mut blocked, value)
                .map_or_else(|| value.clone(), |pieces| shell_line(&pieces));
            (name.clone(), value)
        })
        .collect();
    let cwd = match found
        .cwd
        .as_deref()
        .map(|cwd| (cwd, fill(&mut blocked, cwd)))
    {
        None => None,
        Some((cwd, Some(pieces))) => match pieces.iter().find_map(Piece::env) {
            // A folder isn't read by the shell, so nothing would fill it in.
            Some(name) => {
                blocked.get_or_insert(format!("Uses ${{env:{name}}}, which Gespan can't fill in"));
                Some(cwd.to_string())
            }
            None => Some(shell_line(&pieces)),
        },
        Some((cwd, None)) => Some(cwd.to_string()),
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
    let icon = icon(found.icon.as_deref());
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

/// One of [`ACTION_ICONS`], else `play`.
pub(crate) fn icon(name: Option<&str>) -> &'static str {
    name.and_then(|name| ACTION_ICONS.into_iter().find(|known| *known == name))
        .unwrap_or("play")
}

/// A stretch of a word: plain text, or `${env:NAME}`, which the shell fills in
/// when the action runs so listing it never reads the host's secrets.
#[derive(Debug)]
enum Piece {
    Text(String),
    Env(String),
}

impl Piece {
    fn env(&self) -> Option<&str> {
        match self {
            Piece::Env(name) => Some(name),
            Piece::Text(_) => None,
        }
    }
}

/// `text` with the workspace variables filled in, or why it can't be.
fn variables(ctx: &Ctx, text: &str) -> Result<Vec<Piece>, String> {
    let root = ctx.checkout.display().to_string();
    let mut pieces = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        plain.push_str(&rest[..start]);
        let variable = &rest[start..start + len + 1];
        let name = &variable[2..variable.len() - 1];
        match name {
            "workspaceFolder" => plain.push_str(&root),
            "workspaceFolderBasename" => plain.push_str(
                &ctx.checkout
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default(),
            ),
            _ => match name.strip_prefix("env:") {
                Some(var) if is_env_name(var) => {
                    pieces.push(Piece::Text(std::mem::take(&mut plain)));
                    pieces.push(Piece::Env(var.to_string()));
                }
                Some(_) => {
                    return Err(format!(
                        "Uses {variable} with a name Gespan can't pass to the shell"
                    ))
                }
                None => return Err(format!("Uses {variable}, which Gespan can't fill in")),
            },
        }
        rest = &rest[start + len + 1..];
    }
    plain.push_str(rest);
    pieces.push(Piece::Text(plain));
    for piece in &mut pieces {
        if let Piece::Text(text) = piece {
            *text = text.replace("$ZED_WORKTREE_ROOT", &root);
        }
    }
    pieces.retain(|piece| !matches!(piece, Piece::Text(text) if text.is_empty()));
    Ok(pieces)
}

pub(crate) fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The shell's own reference to `name`: `${NAME}` for `sh`, `%NAME%` for `cmd`.
fn env_ref(name: &str) -> String {
    if cfg!(windows) {
        format!("%{name}%")
    } else {
        format!("${{{name}}}")
    }
}

/// `pieces` as written in a shell line, unquoted.
fn shell_line(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Text(text) => text.clone(),
            Piece::Env(name) => env_ref(name),
        })
        .collect()
}

/// One word for the shell that runs actions: `sh` on Unix, `cmd` on Windows.
/// `cmd` fills in `%NAME%` inside quotes; `sh` needs it outside single quotes.
fn quote(pieces: &[Piece]) -> Option<String> {
    if cfg!(windows) {
        let word = shell_line(pieces);
        let plain = !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:\\@+".contains(c));
        Some(if plain {
            word
        } else {
            format!("\"{}\"", word.replace('"', "\"\""))
        })
    } else if pieces.is_empty() {
        Some("''".into())
    } else {
        pieces
            .iter()
            .map(|piece| match piece {
                Piece::Text(text) => shlex::try_quote(text).ok().map(|text| text.into_owned()),
                Piece::Env(name) => Some(format!("\"{}\"", env_ref(name))),
            })
            .collect()
    }
}

/// `path` with `.` and `..` worked out, without touching the disk.
pub(crate) fn normalize(path: &Path) -> PathBuf {
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
