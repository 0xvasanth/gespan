//! Kiro's V3 engine keeps each session in a folder,
//! `<home>/sessions/<workspace hash>/<session id>/`, holding its
//! `session.json` and a `messages.jsonl` log. The folder is named after the
//! session id, `sess_<uuid>`, or a bare uuid for one moved over from the older
//! `sessions/cli` store. A session never prompted has no log.
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use super::{title_of, Candidate, HEAD_LIMIT};
use crate::outside_turns::{without_injected, OutsideTurn, TurnRole};
use crate::provider::Kind;

const LOG: &str = "messages.jsonl";

pub(super) fn list(home: &Path) -> Vec<Candidate> {
    let Ok(workspaces) = fs::read_dir(home.join("sessions")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for workspace in workspaces.flatten() {
        if !workspace.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(sessions) = fs::read_dir(workspace.path()) else {
            continue;
        };
        for session in sessions.flatten() {
            if !session.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if let Some(candidate) = candidate(&session.path()) {
                found.push(candidate);
            }
        }
    }
    found
}

#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "workspacePaths", default)]
    workspace_paths: Vec<String>,
}

/// The folder's candidate, or `None` when it is not a session's, its
/// `session.json` is unreadable, or its log's head holds no user prompt.
fn candidate(folder: &Path) -> Option<Candidate> {
    let id = folder.file_name()?.to_str()?.to_string();
    let mut meta = Vec::new();
    File::open(folder.join("session.json"))
        .ok()?
        .take(HEAD_LIMIT)
        .read_to_end(&mut meta)
        .ok()?;
    let meta: Meta = serde_json::from_slice(&meta).ok()?;

    let log = folder.join(LOG);
    let updated_at = fs::metadata(&log)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let mut head = Vec::new();
    File::open(&log)
        .ok()?
        .take(HEAD_LIMIT)
        .read_to_end(&mut head)
        .ok()?;
    let title = parse(&head)
        .iter()
        .filter(|t| t.role == TurnRole::User)
        .map(|t| title_of(&t.text))
        .find(|t| !t.is_empty())?;
    Some(Candidate {
        kind: Kind::Kiro,
        provider_id: id,
        cwd: meta.workspace_paths.into_iter().next().unwrap_or_default(),
        title,
        branch: None,
        updated_at,
        messages: None,
        path: folder.to_path_buf(),
    })
}

pub(super) fn read(folder: &Path) -> Result<Vec<OutsideTurn>, String> {
    let log = folder.join(LOG);
    let bytes = fs::read(&log).map_err(|e| format!("read {}: {e}", log.display()))?;
    Ok(parse(&bytes))
}

#[derive(Deserialize)]
struct Line {
    payload: Payload,
    timestamp: Option<String>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "operationType")]
    operation: Option<String>,
    content: Option<String>,
}

/// The user prompts and the assistant's visible text among the log's
/// complete lines. Reasoning, tool calls and results, and Kiro's own notes
/// are left out.
fn parse(log: &[u8]) -> Vec<OutsideTurn> {
    let complete = log.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut turns = Vec::new();
    for line in log[..complete].split(|b| *b == b'\n') {
        let Ok(line) = serde_json::from_slice::<Line>(line) else {
            continue;
        };
        let role = match (
            line.payload.kind.as_str(),
            line.payload.operation.as_deref(),
        ) {
            ("user", _) => TurnRole::User,
            ("assistant", Some("Say")) => TurnRole::Assistant,
            _ => continue,
        };
        let Some(mut text) = line.payload.content else {
            continue;
        };
        if role == TurnRole::User {
            text = without_injected(&text).to_string();
        }
        if text.trim().is_empty() {
            continue;
        }
        turns.push(OutsideTurn {
            role,
            text,
            sent_at: line.timestamp.as_deref().and_then(unix_ms),
        });
    }
    turns
}

/// An RFC 3339 timestamp in Unix milliseconds.
fn unix_ms(timestamp: &str) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}
