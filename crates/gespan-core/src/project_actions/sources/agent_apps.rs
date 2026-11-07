//! Agent apps' project configs. Only their run commands count: setup,
//! teardown, archive and install scripts are lifecycle steps, not actions.
use super::{
    env, in_order, json_keys, json_object, string, strings, toml_keys, toml_object, Source,
};
use crate::project_actions::tasks_json::NO_COMMAND;
use crate::project_actions::{Ctx, Found};
use serde_json::Value;

/// Conductor: `[scripts.run.<name>]` tables, or the older `scripts.run` string.
pub struct Conductor;
/// Superset: a `run` list in `.paradev/config.json` or `.superset/config.json`.
pub struct Superset;
/// Cursor's `environment.json`: background `terminals` and a `start` command.
pub struct Cursor;
/// Codex's `environment.toml`: `[[actions]]`.
pub struct Codex;
/// Claude Code's `launch.json`: dev servers, all background.
pub struct ClaudeLaunch;

impl Source for Conductor {
    fn id(&self) -> &'static str {
        "conductor"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".conductor/settings.toml", "conductor.json"]
    }
    fn parse(&self, _ctx: &Ctx, file: &str, text: &str) -> Result<Vec<Found>, String> {
        let toml = file.ends_with(".toml");
        let root = if toml {
            toml_object(text)?
        } else {
            json_object(text)?
        };
        // `run_mode` only says how Conductor starts them, so it is ignored.
        Ok(match root.pointer("/scripts/run") {
            Some(Value::String(command)) => run("Run", command).into_iter().collect(),
            Some(Value::Object(scripts)) => {
                let path = ["scripts", "run"];
                let order = if toml {
                    toml_keys(text, &path)
                } else {
                    json_keys(text, &path)
                };
                in_order(scripts, &order)
                    .into_iter()
                    .filter_map(|(name, script)| {
                        let mut found = entry(name, string(script, "command"))?;
                        found.args = strings(script, "args");
                        found.cwd = script
                            .pointer("/options/cwd")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        found.hide = script.get("hide").and_then(Value::as_bool) == Some(true);
                        Some(found)
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
    }
}

impl Source for Superset {
    fn id(&self) -> &'static str {
        "superset"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".paradev/config.json", ".superset/config.json"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = json_object(text)?;
        let commands: Vec<&str> = match root.get("run") {
            Some(Value::String(command)) => vec![command],
            Some(Value::Array(commands)) => commands.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let cwd = string(&root, "cwd");
        // Numbered by position, so a blank entry doesn't renumber the rest.
        Ok(commands
            .iter()
            .enumerate()
            .filter_map(|(at, command)| {
                let label = if at == 0 {
                    "Run".to_string()
                } else {
                    format!("Run {}", at + 1)
                };
                let mut found = run(&label, command)?;
                found.cwd = cwd.clone();
                Some(found)
            })
            .collect())
    }
}

impl Source for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".cursor/environment.json"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = json_object(text)?;
        let mut found = Vec::new();
        if let Some(Value::Array(terminals)) = root.get("terminals") {
            for terminal in terminals {
                let Some(label) = string(terminal, "name") else {
                    continue;
                };
                found.extend(
                    entry(&label, string(terminal, "command")).map(|found| Found {
                        detail: string(terminal, "description"),
                        background: true,
                        ..found
                    }),
                );
            }
        }
        if let Some(command) = string(&root, "start") {
            found.extend(run("Start", &command).map(|found| Found {
                background: true,
                ..found
            }));
        }
        Ok(found)
    }
}

impl Source for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".codex/environments/environment.toml"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = toml_object(text)?;
        let Some(Value::Array(actions)) = root.get("actions") else {
            return Ok(Vec::new());
        };
        Ok(actions
            .iter()
            .filter_map(|action| {
                let found = entry(&string(action, "name")?, string(action, "command"))?;
                Some(Found {
                    icon: string(action, "icon"),
                    ..found
                })
            })
            .collect())
    }
}

impl Source for ClaudeLaunch {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".claude/launch.json"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = json_object(text)?;
        let Some(Value::Array(configurations)) = root.get("configurations") else {
            return Ok(Vec::new());
        };
        Ok(configurations
            .iter()
            .filter_map(|configuration| {
                let found = entry(
                    &string(configuration, "name")?,
                    string(configuration, "runtimeExecutable"),
                )?;
                Some(Found {
                    // A program and its args, not a line for the shell.
                    shell: false,
                    args: strings(configuration, "runtimeArgs"),
                    cwd: string(configuration, "cwd"),
                    env: env(configuration, "env"),
                    background: true,
                    ..found
                })
            })
            .collect())
    }
}

/// A shell line to run, or nothing when it is blank.
fn run(label: &str, command: &str) -> Option<Found> {
    (!command.trim().is_empty()).then(|| Found {
        label: label.to_string(),
        command: command.to_string(),
        shell: true,
        ..Found::default()
    })
}

/// A named entry. With no command it is listed, blocked, like a `tasks.json`
/// task that has none. A blank name is skipped.
pub(super) fn entry(label: &str, command: Option<String>) -> Option<Found> {
    if label.trim().is_empty() {
        return None;
    }
    Some(match command {
        Some(command) => run(label, &command)?,
        None => Found {
            label: label.to_string(),
            shell: true,
            blocked: Some(NO_COMMAND.into()),
            ..Found::default()
        },
    })
}
