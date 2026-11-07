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
                only(&fields, &["id"])?;
                run(grant, required(&fields, "id")?, waits)
            }
            "actions_output" => {
                only(&fields, &["runId"])?;
                report(runs::output(required(&fields, "runId")?, waits.output))
            }
            "actions_stop" => {
                only(&fields, &["runId"])?;
                report(runs::stop(required(&fields, "runId")?))
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
            "description": "Run one of the project's actions from actions_list in your checkout, through Gespan, and return its exit code and output. Prefer this over guessing a command. A run still going after about 50 seconds, or a background action such as a dev server after about 5, returns with status \"running\": read more with actions_output and end it with actions_stop.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string", "description": "The id from actions_list"}},
                "required": ["id"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
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
    })
}

fn run(grant: &Grant, id: &str, waits: &Waits) -> Result<Value, String> {
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
    let wait = if action.background {
        waits.background
    } else {
        waits.run
    };
    report(Ok(runs::start(&project, action, wait)))
}

fn report(report: Result<runs::Report, String>) -> Result<Value, String> {
    serde_json::to_value(report?).map_err(|error| error.to_string())
}
