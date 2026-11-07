//! Editors' task files: Gespan's own, VS Code's and Zed's.
use super::{env, string, strings, Source};
use crate::project_actions::{jsonc, tasks_json, Ctx, Found};
use serde_json::Value;

/// `.gespan/tasks.json`, the file Gespan writes.
pub struct Gespan;
/// `.vscode/tasks.json`.
pub struct VsCode;
/// `.zed/tasks.json`.
pub struct Zed;

impl Source for Gespan {
    fn id(&self) -> &'static str {
        "gespan"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".gespan/tasks.json"]
    }
    fn parse(&self, ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        tasks_json::parse(ctx, text)
    }
}

impl Source for VsCode {
    fn id(&self) -> &'static str {
        "vscode"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".vscode/tasks.json"]
    }
    fn parse(&self, ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        tasks_json::parse(ctx, text)
    }
}

impl Source for Zed {
    fn id(&self) -> &'static str {
        "zed"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".zed/tasks.json"]
    }
    /// A list of `{ label, command, args, cwd, env, hide }`.
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let value: Value =
            serde_json::from_str(&jsonc::strip(text).json).map_err(|error| error.to_string())?;
        let Value::Array(tasks) = value else {
            return Err("It isn't a list of tasks".into());
        };
        Ok(tasks
            .iter()
            .filter(|task| task.is_object())
            .filter_map(|task| {
                let label = string(task, "label")?;
                let command = string(task, "command")?;
                Some(Found {
                    label,
                    command,
                    args: strings(task, "args"),
                    shell: true,
                    cwd: string(task, "cwd"),
                    env: env(task, "env"),
                    // Zed's other values show the task but keep it out of the spawn list.
                    hide: task.get("hide").and_then(Value::as_str) == Some("always"),
                    ..Found::default()
                })
            })
            .collect())
    }
}
