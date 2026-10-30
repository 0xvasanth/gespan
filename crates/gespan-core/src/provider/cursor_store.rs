//! Cursor's own per-session SQLite stores, read-only, ported from
//! src-tauri/src/cursor_store.rs. Cursor's ACP stream omits some tool
//! arguments and all subagent work; both are in the stores it writes under
//! `~/.cursor`, which Gespan opens without ever writing.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

/// Skip huge blobs (reasoning dumps). Everything else is scanned newest-first
/// until every requested id is found — Cursor writes many non-JSON rows that
/// would push real tool-calls out of a small LIMIT window.
const MAX_BLOB_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct CursorToolCall {
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) args: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct CursorSubagentRun {
    pub(crate) tool_call_id: String,
    pub(crate) agent_id: String,
    pub(crate) revision: String,
    pub(crate) agent_type: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) steps: Vec<CursorSubagentStep>,
}

#[derive(Clone, Debug)]
pub(crate) struct CursorSubagentStep {
    pub(crate) id: String,
    pub(crate) kind: &'static str,
    pub(crate) text: String,
    pub(crate) tool_name: Option<String>,
    pub(crate) args: Option<Value>,
    pub(crate) status: Option<&'static str>,
    pub(crate) output: Option<String>,
}

/// `~/.cursor`, where Cursor keeps its stores.
pub(crate) fn cursor_dir() -> Option<PathBuf> {
    crate::accounts::home().map(|home| home.join(".cursor"))
}

/// The child runs that `session_id`'s calls spawned, skipping any whose store
/// has not changed since `known_revisions`. Cursor's ACP transport omits child
/// updates; the child's own store names its parent and spawn call in
/// meta[0].subagentInfo.
pub(crate) fn subagent_runs(
    cursor_dir: &Path,
    session_id: &str,
    tool_call_ids: &[String],
    known_revisions: &HashMap<String, String>,
) -> Vec<CursorSubagentRun> {
    if validate_id(session_id, "session").is_err() || tool_call_ids.is_empty() {
        return Vec::new();
    }
    let ids: Vec<String> = tool_call_ids
        .iter()
        .rev()
        .take(256)
        .filter(|id| validate_id(id, "tool call").is_ok())
        .cloned()
        .collect();
    lookup_subagent_runs(cursor_dir, session_id, &ids, known_revisions)
}

/// The arguments Cursor stored for these calls, keyed by the ids asked for.
pub(crate) fn tool_calls(
    cursor_dir: &Path,
    session_id: &str,
    tool_call_ids: &[String],
) -> Result<Vec<CursorToolCall>, String> {
    validate_id(session_id, "session")?;
    if tool_call_ids.len() > 256 {
        return Err("Too many tool call ids".into());
    }
    for id in tool_call_ids {
        validate_id(id, "tool call")?;
    }
    let store = find_session_store(cursor_dir, session_id)?;
    read_tool_calls(&store, tool_call_ids)
}

fn lookup_subagent_runs(
    cursor_dir: &Path,
    session_id: &str,
    tool_call_ids: &[String],
    known_revisions: &HashMap<String, String>,
) -> Vec<CursorSubagentRun> {
    let mut roots = vec![cursor_dir.join("acp-sessions")];
    if let Ok(projects) = std::fs::read_dir(cursor_dir.join("chats")) {
        roots.extend(projects.flatten().map(|entry| entry.path()));
    }
    let mut runs = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let Ok(sessions) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in sessions.flatten() {
            let path = entry.path().join("store.db");
            if !path.is_file() {
                continue;
            }
            let Ok(connection) = open_cursor_store(&path) else {
                continue;
            };
            let Ok(metadata) = read_cursor_metadata(&connection) else {
                continue;
            };
            let Some(info) = metadata.get("subagentInfo") else {
                continue;
            };
            // Never pair by creation order, description, or a shared workspace.
            if info.get("parentAgentId").and_then(Value::as_str) != Some(session_id) {
                continue;
            }
            let Some(stored_call) = info.get("toolCallId").and_then(Value::as_str) else {
                continue;
            };
            let Some(call_id) = tool_call_ids.iter().find(|id| ids_match(stored_call, id)) else {
                continue;
            };
            let Some(agent_id) = metadata.get("agentId").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(agent_id.to_owned()) {
                continue;
            }
            let Ok(revision) =
                connection.query_row("SELECT COALESCE(MAX(rowid), 0) FROM blobs", [], |row| {
                    row.get::<_, i64>(0)
                })
            else {
                continue;
            };
            let revision = revision.to_string();
            if known_revisions.get(agent_id) == Some(&revision) {
                continue;
            }
            let Ok((prompt, steps, model)) = read_subagent_steps(&connection, agent_id) else {
                continue;
            };
            runs.push(CursorSubagentRun {
                tool_call_id: call_id.clone(),
                agent_id: agent_id.to_owned(),
                revision,
                agent_type: info
                    .get("typeName")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                prompt,
                steps,
                model,
            });
        }
    }
    runs
}

fn read_cursor_metadata(connection: &Connection) -> Result<Value, String> {
    let raw: String = connection
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    if raw.len() > 64 * 1024 {
        return Err("Cursor metadata is too large".into());
    }
    // Current Cursor serializes the metadata JSON as hex; tolerate plain JSON.
    if raw.starts_with('{') {
        return serde_json::from_str(&raw).map_err(|e| e.to_string());
    }
    if !raw.len().is_multiple_of(2) || !raw.is_ascii() {
        return Err("Invalid Cursor metadata".into());
    }
    let bytes = (0..raw.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&raw[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn read_subagent_steps(
    connection: &Connection,
    agent_id: &str,
) -> rusqlite::Result<(Option<String>, Vec<CursorSubagentStep>, Option<String>)> {
    // Filter before reading bytes: stores also contain large binary snapshots.
    let mut statement = connection.prepare(
        "SELECT id, data FROM blobs WHERE length(data) <= ?1 AND substr(data, 1, 1) = x'7b' ORDER BY rowid DESC LIMIT 600",
    )?;
    let rows = statement.query_map([MAX_BLOB_BYTES as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut messages: Vec<(String, Value)> = rows
        .filter_map(Result::ok)
        .filter_map(|(id, data)| {
            serde_json::from_slice(&data)
                .ok()
                .map(|message| (id, message))
        })
        .collect();
    messages.reverse();
    let mut steps: Vec<CursorSubagentStep> = Vec::new();
    let mut tools = HashMap::new();
    let mut prompt = None;
    let mut model = None;
    for (blob_id, message) in messages {
        let role = message.get("role").and_then(Value::as_str);
        let content = message.get("content");
        if role == Some("assistant") {
            if let Some(id) = message
                .pointer("/providerOptions/cursor/systemPromptFingerprint/model")
                .and_then(Value::as_str)
            {
                if !id.trim().is_empty() {
                    model = Some(cap_text(id.trim(), 200));
                }
            }
        }
        if role == Some("user") && prompt.is_none() {
            let text = cursor_content_text(content);
            if let Some((_, query)) = text.split_once("<user_query>") {
                prompt = Some(cap_text(
                    query.split("</user_query>").next().unwrap_or(query).trim(),
                    2_000,
                ));
            }
        }
        let Some(content) = content.and_then(Value::as_array) else {
            continue;
        };
        for (index, part) in content.iter().enumerate() {
            let kind = part.get("type").and_then(Value::as_str);
            if role == Some("assistant") && matches!(kind, Some("text" | "reasoning")) {
                let text = part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if text.is_empty() {
                    continue;
                }
                steps.push(CursorSubagentStep {
                    id: format!("{agent_id}:{blob_id}:{index}"),
                    kind: if kind == Some("reasoning") {
                        "reasoning"
                    } else {
                        "message"
                    },
                    text: cap_text(text, 2_000),
                    tool_name: None,
                    args: None,
                    status: None,
                    output: None,
                });
            } else if role == Some("assistant") && kind == Some("tool-call") {
                let Some(call_id) = part.get("toolCallId").and_then(Value::as_str) else {
                    continue;
                };
                if tools.contains_key(call_id) {
                    continue;
                }
                tools.insert(call_id.to_owned(), steps.len());
                steps.push(CursorSubagentStep {
                    id: format!("{agent_id}:tool:{call_id}"),
                    kind: "tool",
                    text: String::new(),
                    tool_name: part
                        .get("toolName")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    args: part.get("args").cloned(),
                    status: Some("in_progress"),
                    output: None,
                });
            } else if role == Some("tool") && kind == Some("tool-result") {
                let Some(call_id) = part.get("toolCallId").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(index) = tools.get(call_id) {
                    let failed = part.get("isError").and_then(Value::as_bool) == Some(true);
                    steps[*index].status = Some(if failed { "failed" } else { "completed" });
                    let output = cursor_content_text(part.get("result"));
                    if !output.trim().is_empty() {
                        steps[*index].output = Some(cap_text(&output, 8_000));
                    }
                }
            }
        }
    }
    if steps.len() > 300 {
        steps.drain(..steps.len() - 300);
    }
    Ok((prompt, steps, model))
}

fn cursor_content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(part)) => part
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    }
}

fn cap_text(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn validate_id(value: &str, label: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 240 {
        return Err(format!("Invalid {label} id"));
    }
    // Cursor composites ACP ids as `call-…\nfc_…`. Reject path-like junk only.
    if trimmed.bytes().any(|byte| matches!(byte, b'/' | b'\\' | 0)) {
        return Err(format!("Invalid {label} id"));
    }
    Ok(())
}

fn find_session_store(cursor_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    let current = cursor_dir
        .join("acp-sessions")
        .join(session_id)
        .join("store.db");
    if current.is_file() {
        return Ok(current);
    }
    let entries = std::fs::read_dir(cursor_dir.join("chats"))
        .map_err(|_| format!("Cursor session {session_id} was not found"))?;
    entries
        .flatten()
        .map(|entry| entry.path().join(session_id).join("store.db"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("Cursor session {session_id} was not found"))
}

fn read_tool_calls(path: &Path, tool_call_ids: &[String]) -> Result<Vec<CursorToolCall>, String> {
    let connection = open_cursor_store(path)?;
    connection
        .busy_timeout(std::time::Duration::from_millis(100))
        .map_err(|e| e.to_string())?;
    lookup_tool_calls(&connection, tool_call_ids).map_err(|e| e.to_string())
}

fn open_cursor_store(path: &Path) -> Result<Connection, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    if Path::new(&wal).exists() {
        // Live WAL files must stay visible. Never use immutable for a live store.
        return Connection::open_with_flags(path, flags).map_err(|e| e.to_string());
    }
    // A checkpointed WAL-mode database otherwise tries to create a new -shm
    // file even on a read-only connection. No sidecars are needed to read it.
    let name = path
        .to_str()
        .ok_or("Invalid Cursor store path")?
        .replace('%', "%25")
        .replace('?', "%3F")
        .replace('#', "%23");
    // SQLite URIs use forward slashes, and a Windows drive needs `file:///C:/…`.
    let name = name.replace('\\', "/");
    let uri = if name.starts_with('/') {
        format!("file:{name}?immutable=1")
    } else {
        format!("file:///{name}?immutable=1")
    };
    Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI).map_err(|e| e.to_string())
}

fn lookup_tool_calls(
    connection: &Connection,
    tool_call_ids: &[String],
) -> rusqlite::Result<Vec<CursorToolCall>> {
    let wanted: HashSet<&str> = tool_call_ids.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }

    let mut found = Vec::new();
    let mut statement = connection.prepare("SELECT data FROM blobs ORDER BY rowid DESC")?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;

    for row in rows {
        let Ok(data) = row else { continue };
        if data.len() > MAX_BLOB_BYTES {
            continue;
        }
        let Ok(payload) = serde_json::from_slice::<Value>(&data) else {
            continue;
        };
        let Some(content) = payload.get("content").and_then(Value::as_array) else {
            continue;
        };

        for item in content {
            if item.get("type").and_then(Value::as_str) != Some("tool-call") {
                continue;
            }
            let Some(stored_id) = item.get("toolCallId").and_then(Value::as_str) else {
                continue;
            };
            let Some(requested) = wanted.iter().copied().find(|id| ids_match(stored_id, id)) else {
                continue;
            };
            if found
                .iter()
                .any(|call: &CursorToolCall| call.tool_call_id == requested)
            {
                continue;
            }
            let Some(tool_name) = item.get("toolName").and_then(Value::as_str) else {
                continue;
            };
            let Some(args) = item.get("args") else {
                continue;
            };
            found.push(CursorToolCall {
                // Return the ACP id the client asked for so the JS map lookup hits.
                tool_call_id: requested.to_owned(),
                tool_name: tool_name.to_owned(),
                args: args.clone(),
            });
            if found.len() == wanted.len() {
                return Ok(found);
            }
        }
    }

    Ok(found)
}

/// Cursor stores `call-<uuid>-N\nfc_<uuid>_N`. ACP often sends only one half.
fn ids_match(stored: &str, wanted: &str) -> bool {
    if stored == wanted {
        return true;
    }
    let stored_parts = id_parts(stored);
    let wanted_parts = id_parts(wanted);
    stored_parts.contains(&wanted)
        || wanted_parts.contains(&stored)
        || stored_parts.iter().any(|part| wanted_parts.contains(part))
}

fn id_parts(value: &str) -> Vec<&str> {
    value
        .split(|c: char| c.is_whitespace())
        .map(str::trim)
        .filter(|part| part.len() >= 8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    struct TestStore(PathBuf, #[allow(dead_code)] tempfile::TempDir);
    impl TestStore {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            Self(dir.path().to_owned(), dir)
        }

        fn child(&self, agent: &str, parent: &str, call: &str) -> (PathBuf, Connection) {
            let path = self
                .0
                .join("chats")
                .join("project")
                .join(agent)
                .join("store.db");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
            let metadata = serde_json::json!({"agentId": agent, "subagentInfo": {
                "parentAgentId": parent, "rootParentAgentId": parent,
                "toolCallId": call, "typeName": "generalPurpose"
            }})
            .to_string();
            let hex: String = metadata.bytes().map(|byte| format!("{byte:02x}")).collect();
            connection
                .execute("INSERT INTO meta VALUES ('0', ?1)", [hex])
                .unwrap();
            (path, connection)
        }
    }

    fn insert_message(connection: &Connection, id: &str, message: Value) {
        connection
            .execute(
                "INSERT INTO blobs VALUES (?1, ?2)",
                params![id, serde_json::to_vec(&message).unwrap()],
            )
            .unwrap();
    }

    #[test]
    fn finds_only_exact_child_lineage_and_skips_unchanged_revisions() {
        let fixture = TestStore::new();
        let (_, child) = fixture.child("child", "parent", "call-12345678\nfc_abcdefgh");
        insert_message(
            &child,
            "intro",
            serde_json::json!({"role":"assistant","content":[{"type":"text","text":"Reviewing."}]}),
        );
        let (_, unrelated) = fixture.child("unrelated", "other-parent", "call-12345678");
        insert_message(
            &unrelated,
            "intro",
            serde_json::json!({"role":"assistant","content":[{"type":"text","text":"Not this task."}]}),
        );
        let (_, nested) = fixture.child("nested", "child", "call-12345678");
        drop((child, unrelated, nested));
        let ids = vec!["fc_abcdefgh".to_owned()];
        let runs = lookup_subagent_runs(&fixture.0, "parent", &ids, &HashMap::new());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].tool_call_id, "fc_abcdefgh");
        assert_eq!(runs[0].agent_id, "child");
        assert_eq!(runs[0].agent_type.as_deref(), Some("generalPurpose"));
        assert_eq!(runs[0].steps[0].text, "Reviewing.");
        let revisions = HashMap::from([("child".to_owned(), runs[0].revision.clone())]);
        assert!(lookup_subagent_runs(&fixture.0, "parent", &ids, &revisions).is_empty());
        assert!(lookup_subagent_runs(
            &fixture.0,
            "parent",
            &["missing-call".into()],
            &HashMap::new()
        )
        .is_empty());
    }

    #[test]
    fn reads_live_wal_and_checkpointed_stores_without_writing_sidecars() {
        let fixture = TestStore::new();
        let (path, writer) = fixture.child("child", "parent", "spawn");
        insert_message(
            &writer,
            "live",
            serde_json::json!({"role":"assistant","content":[{"type":"text","text":"Live WAL message"}]}),
        );
        assert!(path.with_file_name("store.db-wal").exists());
        let live = open_cursor_store(&path).unwrap();
        assert_eq!(
            read_subagent_steps(&live, "child").unwrap().1[0].text,
            "Live WAL message"
        );
        assert!(live.execute("DELETE FROM blobs", []).is_err());
        drop((live, writer));
        assert!(!path.with_file_name("store.db-wal").exists());
        let bytes = std::fs::read(&path).unwrap();
        let closed = open_cursor_store(&path).unwrap();
        assert_eq!(read_subagent_steps(&closed, "child").unwrap().1.len(), 1);
        assert!(closed.execute("DELETE FROM blobs", []).is_err());
        drop(closed);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!path.with_file_name("store.db-wal").exists());
        assert!(!path.with_file_name("store.db-shm").exists());
    }

    #[test]
    fn reads_native_messages_and_tools_in_order_and_merges_results() {
        let fixture = TestStore::new();
        let (_, connection) = fixture.child("child", "parent", "spawn");
        insert_message(
            &connection,
            "user",
            serde_json::json!({"role":"user","content":[{"type":"text","text":"context\n<user_query>Review ACP routing</user_query>"}]}),
        );
        insert_message(
            &connection,
            "assistant-a",
            serde_json::json!({"role":"assistant","id":"1","providerOptions":{"cursor":{"systemPromptFingerprint":{"model":"grok-4.6"}}},"content":[
                {"type":"text","text":"Inspecting."},
                {"type":"reasoning","text":"","signature":"opaque-signature"},
                {"type":"tool-call","toolCallId":"read","toolName":"Read","args":{"path":"/repo/acp.ts"}}
            ]}),
        );
        insert_message(
            &connection,
            "tool-a",
            serde_json::json!({"role":"tool","content":[{"type":"tool-result","toolCallId":"read","toolName":"Read","result":"file contents"}]}),
        );
        insert_message(
            &connection,
            "assistant-b",
            serde_json::json!({"role":"assistant","id":"1","content":[
                {"type":"reasoning","text":"Checking errors."},
                {"type":"tool-call","toolCallId":"shell","toolName":"Shell","args":{"command":"npm test"}},
                {"type":"tool-call","toolCallId":"read","toolName":"Read","args":{"path":"/repo/acp.ts"}}
            ]}),
        );
        insert_message(
            &connection,
            "tool-b",
            serde_json::json!({"role":"tool","content":[{"type":"tool-result","toolCallId":"shell","isError":true,"result":[{"type":"text","text":"failed test"}]}]}),
        );
        connection
            .execute("INSERT INTO blobs VALUES ('binary', ?1)", [vec![0u8, 255]])
            .unwrap();
        let (prompt, steps, model) = read_subagent_steps(&connection, "child").unwrap();
        assert_eq!(prompt.as_deref(), Some("Review ACP routing"));
        assert_eq!(model.as_deref(), Some("grok-4.6"));
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].id, "child:assistant-a:0");
        assert_eq!(steps[1].id, "child:tool:read");
        assert_eq!(steps[1].status, Some("completed"));
        assert_eq!(steps[1].output.as_deref(), Some("file contents"));
        assert_eq!(steps[2].id, "child:assistant-b:0");
        assert_eq!(steps[3].status, Some("failed"));
        assert_eq!(steps[3].output.as_deref(), Some("failed test"));
    }

    #[test]
    fn bounds_child_history_and_ignores_oversized_messages() {
        let fixture = TestStore::new();
        let (_, connection) = fixture.child("child", "parent", "spawn");
        for index in 0..305 {
            insert_message(
                &connection,
                &format!("m{index}"),
                serde_json::json!({"role":"assistant","content":[{"type":"text","text":format!("Message {index}")}]}),
            );
        }
        insert_message(
            &connection,
            "oversized",
            serde_json::json!({"role":"assistant","content":[{"type":"text","text":"x".repeat(MAX_BLOB_BYTES)}]}),
        );
        let (_, steps, _) = read_subagent_steps(&connection, "child").unwrap();
        assert_eq!(steps.len(), 300);
        assert_eq!(steps[0].text, "Message 5");
        assert_eq!(steps[299].text, "Message 304");
    }

    #[test]
    fn finds_tool_call_and_skips_non_json_blobs() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB)", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                params!["binary", vec![0_u8, 159, 146, 150]],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                params![
                    "assistant",
                    br#"{"role":"assistant","content":[{"type":"tool-call","toolCallId":"tool_123","toolName":"Read","args":{"path":"/tmp/example.ts"}}]}"#
                ],
            )
            .unwrap();

        let ids = vec!["tool_123".to_owned(), "tool_missing".to_owned()];
        let calls = lookup_tool_calls(&connection, &ids).unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.tool_call_id, "tool_123");
        assert_eq!(call.tool_name, "Read");
        assert_eq!(call.args["path"], "/tmp/example.ts");
    }

    #[test]
    fn skips_oversized_blobs_and_prefers_recent_rows() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB)", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                params!["huge", vec![b'x'; MAX_BLOB_BYTES + 1]],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                params![
                    "assistant",
                    br#"{"role":"assistant","content":[{"type":"tool-call","toolCallId":"tool_recent","toolName":"Edit","args":{"path":"/tmp/new.ts"}}]}"#
                ],
            )
            .unwrap();

        let calls = lookup_tool_calls(&connection, &["tool_recent".to_owned()]).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_call_id, "tool_recent");
        assert_eq!(calls[0].args["path"], "/tmp/new.ts");
    }

    #[test]
    fn matches_cursor_composite_ids_by_either_half() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB)", [])
            .unwrap();
        let stored = r#"{"role":"assistant","content":[{"type":"tool-call","toolCallId":"call-29f85c36-8f52-4c01-b890-f2f09fe0648c-0\nfc_946a17a5-3f49-92df-aa58-305a98670f49_0","toolName":"Glob","args":{"glob_pattern":"**/*"}}]}"#;
        connection
            .execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                params!["assistant", stored.as_bytes()],
            )
            .unwrap();

        let by_call = lookup_tool_calls(
            &connection,
            &["call-29f85c36-8f52-4c01-b890-f2f09fe0648c-0".to_owned()],
        )
        .unwrap();
        assert_eq!(by_call.len(), 1);
        assert_eq!(
            by_call[0].tool_call_id,
            "call-29f85c36-8f52-4c01-b890-f2f09fe0648c-0"
        );
        assert_eq!(by_call[0].tool_name, "Glob");
        assert_eq!(by_call[0].args["glob_pattern"], "**/*");

        let by_fc = lookup_tool_calls(
            &connection,
            &["fc_946a17a5-3f49-92df-aa58-305a98670f49_0".to_owned()],
        )
        .unwrap();
        assert_eq!(by_fc.len(), 1);
        assert_eq!(by_fc[0].args["glob_pattern"], "**/*");
    }

    #[test]
    fn accepts_newline_in_requested_id() {
        assert!(validate_id(
            "call-29f85c36-8f52-4c01-b890-f2f09fe0648c-0\nfc_946a17a5-3f49-92df-aa58-305a98670f49_0",
            "tool call"
        )
        .is_ok());
        assert!(validate_id("../etc/passwd", "tool call").is_err());
    }
}
