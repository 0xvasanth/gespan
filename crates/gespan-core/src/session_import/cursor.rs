//! Cursor keeps each session `cursor-agent acp` runs in a folder,
//! `<home>/acp-sessions/<session id>/`, holding a `meta.json` (`cwd`, and
//! Cursor's own `title`) and a `store.db`. A session opened and never
//! prompted has no store.
//!
//! The store is SQLite with two tables:
//! - `meta`: row `0` is hex-encoded JSON naming the session (`agentId`) and
//!   its latest root blob (`latestRootBlobId`);
//! - `blobs`: content-addressed rows, `id` being the SHA-256 of `data` in hex.
//!
//! The root blob is protobuf. Its field 1 repeats the 32-byte ids of the
//! conversation's messages, oldest first; its field 21, when the folder is a
//! git checkout, holds the workspace path (field 1) and branch (field 2).
//! Each message blob is JSON `{role, content, providerOptions}`:
//! - `system`: the system prompt;
//! - `user`: either Cursor's context, a string of `<user_info>`, `<rules>`
//!   and the like, or a prompt: `text` parts, one of which wraps the typed
//!   words in `<user_query>` after a `<timestamp>`;
//! - `assistant`: `text`, `reasoning` and `tool-call` parts;
//! - `tool`: `tool-result` parts.
//!
//! Every other blob (earlier roots, ACP turn records in protobuf, messages a
//! later root dropped) is not part of the conversation.
//!
//! `cursor-agent`'s terminal chats, `<home>/chats/<workspace hash>/<id>/`,
//! use the same store, but ACP's `session/load` answers "not found" for
//! their ids, so they are not offered.
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::TimeZone;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;

use super::{title_of, Candidate, HEAD_LIMIT};
use crate::outside_turns::{OutsideTurn, TurnRole};
use crate::provider::cursor_store::{open_cursor_store, read_cursor_metadata};
use crate::provider::Kind;

const STORE: &str = "store.db";

pub(super) fn list(home: &Path) -> Vec<Candidate> {
    let Ok(sessions) = fs::read_dir(home.join("acp-sessions")) else {
        return Vec::new();
    };
    sessions
        .flatten()
        .filter(|session| session.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|session| candidate(&session.path()))
        .collect()
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    cwd: String,
}

/// The folder's candidate, or `None` when it has no readable store or the
/// store holds no prompt.
fn candidate(folder: &Path) -> Option<Candidate> {
    let id = folder.file_name()?.to_str()?.to_string();
    let mut meta = Vec::new();
    File::open(folder.join("meta.json"))
        .ok()?
        .take(HEAD_LIMIT)
        .read_to_end(&mut meta)
        .ok()?;
    let meta: Meta = serde_json::from_slice(&meta).ok()?;
    let updated_at = modified_ms(folder)?;

    let store = open(folder).ok()?;
    let root = root(&store).ok()?;
    let title = root
        .messages
        .iter()
        .filter_map(|id| turn(&message(&store, id)?))
        .filter(|turn| turn.role == TurnRole::User)
        .map(|turn| title_of(&turn.text))
        .find(|title| !title.is_empty())?;
    Some(Candidate {
        kind: Kind::Cursor,
        provider_id: id,
        cwd: meta.cwd,
        title,
        branch: root.branch,
        updated_at,
        messages: None,
        path: folder.to_path_buf(),
    })
}

pub(super) fn read(folder: &Path) -> Result<Vec<OutsideTurn>, String> {
    let store = open(folder)?;
    let root = root(&store)?;
    Ok(root
        .messages
        .iter()
        .filter_map(|id| turn(&message(&store, id)?))
        .collect())
}

fn open(folder: &Path) -> Result<Connection, String> {
    let store = open_cursor_store(&folder.join(STORE))?;
    store
        .busy_timeout(std::time::Duration::from_millis(100))
        .map_err(|e| e.to_string())?;
    Ok(store)
}

/// When the store or its write-ahead log last changed, in Unix milliseconds.
fn modified_ms(folder: &Path) -> Option<u64> {
    let modified = [STORE, "store.db-wal"]
        .iter()
        .filter_map(|name| fs::metadata(folder.join(name)).ok()?.modified().ok())
        .max()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

#[derive(Default)]
struct Root {
    /// Message blob ids, oldest first.
    messages: Vec<String>,
    branch: Option<String>,
}

/// The root blob `meta` names.
fn root(store: &Connection) -> Result<Root, String> {
    let meta = read_cursor_metadata(store)?;
    let id = meta
        .get("latestRootBlobId")
        .and_then(Value::as_str)
        .ok_or("Cursor store names no root")?;
    let data: Vec<u8> = store
        .query_row("SELECT data FROM blobs WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    let mut root = Root::default();
    let mut fields = &data[..];
    while !fields.is_empty() {
        match next_field(&mut fields).ok_or("Cursor root is not protobuf")? {
            (1, Some(id)) if id.len() == 32 => {
                root.messages
                    .push(id.iter().map(|b| format!("{b:02x}")).collect());
            }
            (21, Some(mut workspace)) => {
                while let Some((field, value)) = next_field(&mut workspace) {
                    if let (2, Some(branch)) = (field, value) {
                        root.branch = std::str::from_utf8(branch)
                            .ok()
                            .filter(|b| !b.is_empty())
                            .map(str::to_string);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(root)
}

/// The next protobuf field's number, with its bytes when it is
/// length-delimited; `None` when the data is cut short or malformed.
fn next_field<'a>(data: &mut &'a [u8]) -> Option<(u64, Option<&'a [u8]>)> {
    let key = varint(data)?;
    let bytes = match key & 7 {
        0 => {
            varint(data)?;
            None
        }
        1 => {
            *data = data.get(8..)?;
            None
        }
        2 => {
            let len = usize::try_from(varint(data)?).ok()?;
            let bytes = data.get(..len)?;
            *data = &data[len..];
            Some(bytes)
        }
        5 => {
            *data = data.get(4..)?;
            None
        }
        _ => return None,
    };
    Some((key >> 3, bytes))
}

fn varint(data: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = data.split_first()?;
        *data = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

/// The message blob `id`, or `None` when it is missing or not JSON.
fn message(store: &Connection, id: &str) -> Option<Value> {
    let data: Vec<u8> = store
        .query_row("SELECT data FROM blobs WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()
        .ok()??;
    serde_json::from_slice(&data).ok()
}

/// A prompt's typed words or the assistant's visible text. Context,
/// reasoning, tool calls and results are left out.
fn turn(message: &Value) -> Option<OutsideTurn> {
    let text = text_parts(message.get("content")?);
    let (role, text, sent_at) = match message.get("role")?.as_str()? {
        "user" => {
            let queries: Vec<&str> = tagged(&text, "user_query").map(str::trim).collect();
            let sent_at = tagged(&text, "timestamp").next().and_then(unix_ms);
            (TurnRole::User, queries.join("\n\n"), sent_at)
        }
        "assistant" => (TurnRole::Assistant, text, None),
        _ => return None,
    };
    if text.trim().is_empty() {
        return None;
    }
    Some(OutsideTurn {
        role,
        text,
        sent_at,
    })
}

/// The message's `text` parts, or its content when that is a plain string.
fn text_parts(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// Each `<tag>…</tag>` section of `text`.
fn tagged<'a>(text: &'a str, tag: &str) -> impl Iterator<Item = &'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut rest = text;
    std::iter::from_fn(move || {
        let (_, after) = rest.split_once(open.as_str())?;
        let (inside, tail) = after.split_once(close.as_str())?;
        rest = tail;
        Some(inside)
    })
}

/// Cursor's `<timestamp>`, such as `Wednesday, Nov 11, 2026, 7:32 AM
/// (UTC+5:30)`, in Unix milliseconds.
fn unix_ms(stamp: &str) -> Option<u64> {
    let (_weekday, rest) = stamp.trim().split_once(", ")?;
    let (local, zone) = rest.split_once(" (UTC")?;
    let zone = zone.strip_suffix(')')?;
    let offset = match zone.split_at_checked(1) {
        None => 0,
        Some((sign, hours_minutes)) => {
            let sign = match sign {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let (hours, minutes) = hours_minutes
                .split_once(':')
                .unwrap_or((hours_minutes, "0"));
            sign * (hours.parse::<i32>().ok()? * 3600 + minutes.parse::<i32>().ok()? * 60)
        }
    };
    let local = chrono::NaiveDateTime::parse_from_str(local, "%b %d, %Y, %I:%M %p").ok()?;
    let at = chrono::FixedOffset::east_opt(offset)?
        .from_local_datetime(&local)
        .single()?;
    u64::try_from(at.timestamp_millis()).ok()
}
