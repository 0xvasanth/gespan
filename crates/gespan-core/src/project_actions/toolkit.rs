//! The `actions` tools: an agent lists the project's actions and runs them in
//! its session's checkout. A read-only session may list but never run.
use super::{detect, runs, Action};
use crate::tool_servers::{project_of, ACTIONS, ACTIONS_READ};
use mcp_hub::Grant;
use serde_json::{json, Map, Value};
use std::time::Duration;

/// How long a run waits before an agent gets it back still running.
const RUN_WAIT: Duration = Duration::from_secs(50);
/// A background action, such as a dev server, never ends, so it returns soon.
const BACKGROUND_WAIT: Duration = Duration::from_secs(5);
const OUTPUT_WAIT: Duration = Duration::from_secs(50);

const READ_ONLY: &str = "Read-only sessions can list actions but not run them.";

/// How long each tool waits on a run. Tests use shorter ones.
pub(crate) struct Waits {
    pub run: Duration,
    pub background: Duration,
    pub output: Duration,
}

const AGENT: Waits = Waits {
    run: RUN_WAIT,
    background: BACKGROUND_WAIT,
    output: OUTPUT_WAIT,
};

/// The action tools, for sessions whose grant names them. `may_run` is false
/// for read-only and plan sessions.
pub(crate) struct Toolkit {
    pub may_run: bool,
}

impl mcp_hub::Toolkit for Toolkit {
    fn name(&self) -> &'static str {
        if self.may_run {
            ACTIONS
        } else {
            ACTIONS_READ
        }
    }

    fn tools(&self, _: &Grant) -> Vec<Value> {
        let mut tools = vec![list_tool()];
        if self.may_run {
            tools.extend(run_tools());
        }
        tools
    }

    fn call(&self, grant: &Grant, tool: &str, input: Value) -> Result<Value, String> {
        self.call_waiting(grant, tool, input, &AGENT)
    }
}

impl Toolkit {
    pub(crate) fn call_waiting(
        &self,
        grant: &Grant,
        tool: &str,
        input: Value,
        waits: &Waits,
    ) -> Result<Value, String> {
        if tool != "actions_list" && !self.may_run {
            return Err(READ_ONLY.into());
        }
        let fields = fields(&input)?;
        match tool {
            "actions_list" => {
                only(&fields, &[])?;
                Ok(list(grant))
            }
            "actions_run" => {
                only(&fields, &["id", "command"])?;
                let id = required(&fields, "id")?;
                run(grant, id, required(&fields, "command")?, waits)
            }
            "actions_output" => {
                only(&fields, &["runId"])?;
                let run_id = required(&fields, "runId")?;
                report(runs::output_within(&grant.workspace, run_id, waits.output)?)
            }
            "actions_stop" => {
                only(&fields, &["runId"])?;
                report(runs::stop_within(
                    &grant.workspace,
                    required(&fields, "runId")?,
                )?)
            }
            _ => Err("Unknown actions tool".into()),
        }
    }
}

fn list_tool() -> Value {
    json!({
        "name": "actions_list",
        "description": "List this project's actions in Gespan: the build, test, dev server and other commands its files define. Prefer running one of these over guessing a command. A blocked action says why it can't run.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    })
}

fn run_tools() -> [Value; 3] {
    let run_id = json!({"runId": {"type": "string", "description": "From actions_run"}});
    [
        json!({
            "name": "actions_run",
            "description": "Run one of the project's actions from actions_list in your checkout, through Gespan, and return its exit code and output. Prefer this over guessing a command. Pass the command actions_list showed; if the project's files have changed it since, nothing runs and you are asked to list again. A run still going after about 50 seconds, or a background action such as a dev server after about 5, returns with status \"running\": read more with actions_output and end it with actions_stop.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The id from actions_list"},
                    "command": {"type": "string", "description": "The action's command exactly as actions_list returned it"},
                },
                "required": ["id", "command"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "actions_output",
            "description": "The output a Gespan action run printed since the last read, once it ends or after about 50 seconds.",
            "inputSchema": {
                "type": "object",
                "properties": run_id,
                "required": ["runId"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "actions_stop",
            "description": "Stop a Gespan action run, with everything it started, and return how it ended.",
            "inputSchema": {
                "type": "object",
                "properties": run_id,
                "required": ["runId"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "openWorldHint": false},
        }),
    ]
}

fn fields(input: &Value) -> Result<Map<String, Value>, String> {
    match input {
        Value::Null => Ok(Map::new()),
        Value::Object(fields) => Ok(fields.clone()),
        _ => Err("Arguments must be an object".into()),
    }
}

fn only(fields: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(extra) => Err(format!("Unknown argument: {extra}")),
        None => Ok(()),
    }
}

fn required<'a>(fields: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    match fields.get(field) {
        None | Some(Value::Null) => Err(format!("{field} is required")),
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err(format!("{field} must be a string")),
    }
}

/// The session's project, or its workspace before Gespan has mounted it.
fn project(grant: &Grant) -> String {
    project_of(grant.session).unwrap_or_else(|| grant.workspace.to_string_lossy().into_owned())
}

fn list(grant: &Grant) -> Value {
    let listing = detect(&project(grant), &grant.workspace);
    let actions: Vec<Value> = listing.actions.iter().map(row).collect();
    json!({"actions": actions, "errors": listing.errors})
}

fn row(action: &Action) -> Value {
    json!({
        "id": action.id,
        "label": action.label,
        "detail": action.detail,
        "source": action.file,
        "command": action.command,
        "cwd": action.cwd.display().to_string(),
        "background": action.background,
        "blocked": action.blocked,
        // Names only: values can be secrets.
        "env": action.env.iter().map(|(name, _)| name).collect::<Vec<_>>(),
    })
}

/// Runs `id` only while its command is still `command`, the one the agent
/// listed and its approval showed: a file changed since then runs nothing.
fn run(grant: &Grant, id: &str, command: &str, waits: &Waits) -> Result<Value, String> {
    let project = project(grant);
    let listing = detect(&project, &grant.workspace);
    let action = listing
        .actions
        .iter()
        .find(|action| action.id == id)
        .ok_or_else(|| format!("Unknown action: {id}. Call actions_list for the ids."))?;
    if let Some(reason) = &action.blocked {
        return Err(format!("{} can't run: {reason}", action.label));
    }
    if action.command != command {
        return Err(format!(
            "The command for {id} is now `{}`; call actions_list again.",
            action.command
        ));
    }
    let wait = if action.background {
        waits.background
    } else {
        waits.run
    };
    report(runs::start(&project, action, wait))
}

fn report(report: runs::Report) -> Result<Value, String> {
    serde_json::to_value(report).map_err(|error| error.to_string())
}
