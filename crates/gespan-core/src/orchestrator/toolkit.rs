//! The `sessions` tools: how a session learns what its sibling sessions in
//! the same repository are doing, and reads one of them. Read-only.
use super::bridge::{dispatch, Action, Scope};
use crate::tool_servers::SESSIONS;
use mcp_hub::Grant;
use serde_json::{json, Map, Value};

const LIST_DESCRIPTION: &str = "List the other sessions working in this repository and its worktrees: what each is doing, its status, where it runs and which files it is changing. scope \"all\" also lists sessions in other repositories.";
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
