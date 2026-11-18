//! The `sessions` tools: how a session learns what its sibling sessions in
//! the same repository are doing, reads one of them, and starts or cancels
//! sessions of its own.
use super::{
    bridge::{dispatch, Action, Scope},
    policy::Refusal,
    start::StartInput,
};
use crate::tool_servers::SESSIONS;
use mcp_hub::Grant;
use serde_json::{json, Map, Value};

const LIST_DESCRIPTION: &str = "List the other sessions working in this repository and its worktrees: what each is doing, its status, where it runs and which files it is changing. scope \"all\" also lists sessions in other repositories.";
const START_DESCRIPTION: &str = "Start a new session that works on its own, shown under this one in the sidebar, and get its id at once. workspace \"worktree\" gives it a new git worktree of this repository, \"main\" the main checkout, \"existing:<path>\" a checkout already on disk. It runs with this session's permissions or narrower (access). When it finishes, a report arrives here as a new message (wake \"settled\", the default; \"always\" also reports when it waits on the user or is stopped; \"never\" sends none). Pass the same requestId to retry safely: a repeated request returns the first answer. At most 5 started sessions run at once, two levels deep.";
const CANCEL_DESCRIPTION: &str =
    "Stop a session this one started (or one they started), and every session below it.";
const MESSAGE_DESCRIPTION: &str = "Send another session in this repository a message. It is queued and arrives as a new turn the next time that session is idle, labelled with this session's title; nothing interrupts a running turn. Use the same requestId to retry safely. Messages passed back and forth too often are held for the user.";
const WAIT_DESCRIPTION: &str = "Wait until one of these sessions stops working (it is idle, waiting on the user, archived or deleted), for at most timeoutSeconds (25, default 20). Returns which stopped, or timedOut.";
const MAX_MESSAGE: usize = 32_000;
const MAX_WAIT_IDS: usize = 20;
const READ_DESCRIPTION: &str = "Read another session's recent exchanges, newest first. Pass the before cursor from a previous read for older ones.";

pub struct Toolkit;

impl mcp_hub::Toolkit for Toolkit {
    fn name(&self) -> &'static str {
        SESSIONS
    }

    fn tools(&self, _: &Grant) -> Vec<Value> {
        vec![
            json!({
                "name": "sessions_list",
                "description": LIST_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scope": {"enum": ["repo", "all"], "description": "\"repo\" (default) or \"all\""},
                    },
                    "additionalProperties": false,
                },
                "annotations": {"readOnlyHint": true, "openWorldHint": false},
            }),
            json!({
                "name": "sessions_read",
                "description": READ_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "description": "The session's id, from sessions_list"},
                        "before": {"type": "string", "description": "The cursor a previous read returned, for older exchanges"},
                    },
                    "required": ["id"],
                    "additionalProperties": false,
                },
                "annotations": {"readOnlyHint": true, "openWorldHint": false},
            }),
            json!({
                "name": "sessions_start",
                "description": START_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "prompt": {"type": "string", "description": "What the new session should do, as you would ask a colleague"},
                        "workspace": {"type": "string", "description": "\"worktree\", \"main\" or \"existing:<absolute path>\""},
                        "base": {"type": "string", "description": "The branch a new worktree starts from; the default branch when left out"},
                        "harness": {"type": "string", "description": "The agent to run, such as claude or codex; this session's when left out"},
                        "model": {"type": "string", "description": "The model id; the agent's default when left out"},
                        "access": {"enum": ["read-only", "supervised", "auto-accept-edits", "auto", "full-access"], "description": "Its permissions: this session's when left out, never broader"},
                        "wake": {"enum": ["always", "settled", "never"], "description": "When its stop is reported here; \"settled\" by default"},
                        "title": {"type": "string", "description": "Its sidebar title; the start of the prompt when left out"},
                        "requestId": {"type": "string", "description": "A key of your choosing that makes a retry safe"},
                    },
                    "required": ["prompt", "workspace"],
                    "additionalProperties": false,
                },
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
            }),
            json!({
                "name": "sessions_cancel",
                "description": CANCEL_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "description": "The session's id, from sessions_start or sessions_list"},
                    },
                    "required": ["id"],
                    "additionalProperties": false,
                },
                "annotations": {"readOnlyHint": false, "destructiveHint": true, "openWorldHint": false},
            }),
            json!({
                "name": "sessions_message",
                "description": MESSAGE_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "description": "The session's id, from sessions_list"},
                        "text": {"type": "string", "description": "The message"},
                        "requestId": {"type": "string", "description": "A key of your choosing that makes a retry safe"},
                    },
                    "required": ["id", "text"],
                    "additionalProperties": false,
                },
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
            }),
            json!({
                "name": "sessions_wait",
                "description": WAIT_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ids": {"type": "array", "items": {"type": "integer"}, "description": "Session ids, at most 20"},
                        "timeoutSeconds": {"type": "number", "description": "0 to 25; 20 by default"},
                    },
                    "required": ["ids"],
                    "additionalProperties": false,
                },
                "annotations": {"readOnlyHint": true, "openWorldHint": false},
            }),
        ]
    }

    fn call(&self, grant: &Grant, tool: &str, arguments: Value) -> Result<Value, String> {
        let fields = match arguments {
            Value::Null => Map::new(),
            Value::Object(fields) => fields,
            _ => return Err("Arguments must be a JSON object".into()),
        };
        let action = match tool {
            "sessions_list" => {
                only(tool, &fields, &["scope"])?;
                let scope = match fields.get("scope") {
                    None | Some(Value::Null) => Scope::Repo,
                    Some(Value::String(scope)) if scope == "repo" => Scope::Repo,
                    Some(Value::String(scope)) if scope == "all" => Scope::All,
                    Some(_) => return Err("scope must be \"repo\" or \"all\"".into()),
                };
                Action::List { scope }
            }
            "sessions_read" => {
                only(tool, &fields, &["id", "before"])?;
                let id = fields
                    .get("id")
                    .and_then(Value::as_i64)
                    .ok_or("id must be a session id, a whole number")?;
                let before = match fields.get("before") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(before)) => Some(before.clone()),
                    Some(_) => return Err("before must be a string".into()),
                };
                Action::Read { id, before }
            }
            "sessions_start" => Action::Start {
                input: Box::new(StartInput::parse(&fields)?),
            },
            "sessions_cancel" => {
                only(tool, &fields, &["id"]).map_err(|e| Refusal::invalid(e).to_string())?;
                let id = fields
                    .get("id")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Refusal::invalid("id must be a session id, a whole number"))?;
                Action::Cancel { id }
            }
            "sessions_message" => {
                only(tool, &fields, &["id", "text", "requestId"]).map_err(Refusal::invalid)?;
                let id = fields
                    .get("id")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Refusal::invalid("id must be a session id, a whole number"))?;
                let text = match fields.get("text") {
                    Some(Value::String(text))
                        if !text.trim().is_empty() && text.chars().count() <= MAX_MESSAGE =>
                    {
                        text.clone()
                    }
                    _ => {
                        return Err(Refusal::invalid(format!(
                            "text must be the message, 1 to {MAX_MESSAGE} characters"
                        ))
                        .into())
                    }
                };
                let request_id = match fields.get("requestId") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(id)) if !id.is_empty() && id.len() <= 200 => {
                        Some(id.clone())
                    }
                    Some(_) => {
                        return Err(
                            Refusal::invalid("requestId must be a short, non-empty string").into(),
                        )
                    }
                };
                Action::Message {
                    id,
                    text,
                    request_id,
                }
            }
            "sessions_wait" => {
                only(tool, &fields, &["ids", "timeoutSeconds"]).map_err(Refusal::invalid)?;
                let ids: Vec<i64> = fields
                    .get("ids")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.iter().map(Value::as_i64).collect::<Option<Vec<_>>>())
                    .filter(|ids| !ids.is_empty() && ids.len() <= MAX_WAIT_IDS)
                    .ok_or_else(|| {
                        Refusal::invalid(format!(
                            "ids must be 1 to {MAX_WAIT_IDS} session ids, whole numbers"
                        ))
                    })?;
                let max = crate::orchestration::run::MAX_WAIT_SECONDS;
                let timeout_seconds = match fields.get("timeoutSeconds") {
                    None | Some(Value::Null) => 20,
                    Some(value) => value
                        .as_f64()
                        .filter(|seconds| (0.0..=max as f64).contains(seconds))
                        .map(|seconds| seconds.ceil() as u64)
                        .ok_or_else(|| {
                            Refusal::invalid(format!("timeoutSeconds must be 0 to {max}"))
                        })?,
                };
                Action::Wait {
                    ids,
                    timeout_seconds,
                }
            }
            _ => return Err(format!("Unknown sessions tool: {tool}")),
        };
        dispatch(grant.session, action)
    }
}

/// Refuses a field the tool doesn't take, naming it.
fn only(tool: &str, fields: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(extra) => Err(format!(
            "{tool} does not take \"{extra}\". Its fields are: {}",
            allowed.join(", ")
        )),
        None => Ok(()),
    }
}
