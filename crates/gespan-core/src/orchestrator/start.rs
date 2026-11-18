//! What `sessions_start` asks for, checked strictly before anything runs.
use super::{policy::Refusal, tree::Wake};
use crate::{agent::AccessMode, provider::Kind};
use serde_json::{Map, Value};
use std::path::Path;

pub const FIELDS: &[&str] = &[
    "prompt",
    "workspace",
    "harness",
    "model",
    "base",
    "wake",
    "access",
    "title",
    "requestId",
];
const MAX_PROMPT: usize = 32_000;
const MAX_TITLE: usize = 160;
const MAX_SHORT: usize = 200;

/// Where the child works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceChoice {
    /// The project's main checkout.
    Main,
    /// A new worktree of the project, cut from `base` or the default branch.
    Worktree,
    /// A checkout already on disk: an absolute path.
    Existing(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartInput {
    pub prompt: String,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub workspace: WorkspaceChoice,
    pub base: Option<String>,
    pub wake: Wake,
    pub access: Option<AccessMode>,
    pub title: Option<String>,
    pub request_id: Option<String>,
}

impl StartInput {
    pub fn parse(fields: &Map<String, Value>) -> Result<Self, Refusal> {
        if let Some(extra) = fields.keys().find(|key| !FIELDS.contains(&key.as_str())) {
            return Err(Refusal::invalid(format!(
                "sessions_start does not take \"{extra}\". Its fields are: {}",
                FIELDS.join(", ")
            )));
        }
        let prompt = text(fields, "prompt", MAX_PROMPT)?
            .filter(|prompt| !prompt.trim().is_empty())
            .ok_or_else(|| {
                Refusal::invalid("prompt is required: what the new session should do")
            })?;
        let workspace = match text(fields, "workspace", 4_096)?.as_deref() {
            Some("main") => WorkspaceChoice::Main,
            Some("worktree") => WorkspaceChoice::Worktree,
            Some(other) if other.starts_with("existing:") => {
                let path = &other["existing:".len()..];
                if path.is_empty() || !Path::new(path).is_absolute() {
                    return Err(Refusal::invalid(
                        "workspace \"existing:<path>\" needs an absolute path",
                    ));
                }
                WorkspaceChoice::Existing(path.to_owned())
            }
            _ => {
                return Err(Refusal::invalid(
                    "workspace must be \"main\", \"worktree\" or \"existing:<absolute path>\"",
                ))
            }
        };
        let harness = text(fields, "harness", MAX_SHORT)?;
        if let Some(harness) = &harness {
            if Kind::parse(harness).is_none() {
                let known: Vec<&str> = Kind::ALL.iter().map(|kind| kind.id()).collect();
                return Err(Refusal::invalid(format!(
                    "harness \"{harness}\" is unknown; use one of {}",
                    known.join(", ")
                )));
            }
        }
        let base = text(fields, "base", MAX_SHORT)?;
        if base.is_some() && workspace != WorkspaceChoice::Worktree {
            return Err(Refusal::invalid(
                "base only applies to workspace \"worktree\"",
            ));
        }
        let wake = match text(fields, "wake", MAX_SHORT)?.as_deref() {
            None | Some("settled") => Wake::Settled,
            Some("always") => Wake::Always,
            Some("never") => Wake::Never,
            Some(_) => {
                return Err(Refusal::invalid(
                    "wake must be \"always\", \"settled\" or \"never\"",
                ))
            }
        };
        let access = match text(fields, "access", MAX_SHORT)? {
            None => None,
            Some(id) => Some(access_mode(&id).ok_or_else(|| {
                Refusal::invalid(
                    "access must be read-only, supervised, auto-accept-edits, auto or full-access",
                )
            })?),
        };
        let request_id = text(fields, "requestId", MAX_SHORT)?;
        if request_id.as_deref().is_some_and(str::is_empty) {
            return Err(Refusal::invalid("requestId must not be empty"));
        }
        Ok(Self {
            prompt,
            harness,
            model: text(fields, "model", MAX_SHORT)?,
            workspace,
            base,
            wake,
            access,
            title: text(fields, "title", MAX_TITLE)?,
            request_id,
        })
    }
}

/// A mode from its id, refusing ids [`AccessMode::from_id`] would quietly
/// read as supervised.
pub fn access_mode(id: &str) -> Option<AccessMode> {
    let mode = AccessMode::from_id(id);
    (mode.id() == id).then_some(mode)
}

/// An optional string field of at most `max` characters; `null` is absent.
fn text(fields: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, Refusal> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.chars().count() <= max => Ok(Some(value.clone())),
        Some(Value::String(_)) => Err(Refusal::invalid(format!(
            "{key} is longer than {max} characters"
        ))),
        Some(_) => Err(Refusal::invalid(format!("{key} must be a string"))),
    }
}
