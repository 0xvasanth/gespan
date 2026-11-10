//! Turns taken in an agent's own CLI, read back from its session file so
//! Gespan's chat can show them.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::Kind;

mod codex;
pub use codex::parse_codex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRole {
    User,
    Assistant,
}

/// One visible message: the prompt a user typed or the text an assistant
/// replied with. Tool inputs and outputs never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutsideTurn {
    pub role: TurnRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub turns: Vec<OutsideTurn>,
    /// Bytes up to and including the last complete line of the region.
    pub consumed: u64,
    /// A user or assistant entry was seen, even one that produced no text.
    pub grew: bool,
}

struct Entry {
    uuid: Option<String>,
    role: TurnRole,
    message_id: Option<String>,
    text: String,
}

/// Reads turns from a region of a Claude Code session file. Only the live
/// branch is returned, and only entries after `after_uuid` when given.
pub fn parse_claude(region: &[u8], after_uuid: Option<&str>) -> Parsed {
    let consumed = complete_len(region);

    let mut parents: HashMap<String, String> = HashMap::new();
    let mut entries: Vec<Entry> = Vec::new();
    for line in region[..consumed].split(|b| *b == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let uuid = str_field(&value, "uuid");
        if let Some(uuid) = &uuid {
            // A compact boundary has no parent but points back at the last
            // entry before the compaction.
            let parent =
                str_field(&value, "parentUuid").or_else(|| str_field(&value, "logicalParentUuid"));
            if let Some(parent) = parent {
                parents.insert(uuid.clone(), parent);
            }
        }
        if let Some(entry) = turn_entry(&value, uuid) {
            entries.push(entry);
        }
    }

    let grew = !entries.is_empty();
    let by_uuid: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.uuid.as_deref().map(|u| (u, i)))
        .collect();

    // Walk back from the newest entry through every parent, including
    // bookkeeping entries that sit between two turns.
    let mut live: Vec<usize> = Vec::new();
    if let Some(last) = entries.last() {
        match &last.uuid {
            Some(uuid) => {
                let mut cursor = Some(uuid.as_str());
                let mut visited: HashSet<&str> = HashSet::new();
                while let Some(uuid) = cursor {
                    if after_uuid == Some(uuid) || !visited.insert(uuid) {
                        break;
                    }
                    if let Some(&i) = by_uuid.get(uuid) {
                        live.push(i);
                    }
                    cursor = parents.get(uuid).map(String::as_str);
                }
                live.reverse();
            }
            None => live.push(entries.len() - 1),
        }
    }

    let mut turns: Vec<OutsideTurn> = Vec::new();
    let mut last_id: Option<&str> = None;
    for i in live {
        let entry = &entries[i];
        if entry.text.is_empty() {
            continue;
        }
        let same_reply = entry.role == TurnRole::Assistant
            && entry.message_id.is_some()
            && entry.message_id.as_deref() == last_id
            && turns.last().is_some_and(|t| t.role == TurnRole::Assistant);
        if same_reply {
            if let Some(turn) = turns.last_mut() {
                turn.text.push_str("\n\n");
                turn.text.push_str(&entry.text);
            }
        } else {
            turns.push(OutsideTurn {
                role: entry.role,
                text: entry.text.clone(),
            });
        }
        last_id = entry.message_id.as_deref();
    }

    Parsed {
        turns,
        consumed: consumed as u64,
        grew,
    }
}

/// Length of the region up to and including its last newline; anything after
/// it is a line still being written.
fn complete_len(region: &[u8]) -> usize {
    region
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |i| i + 1)
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn turn_entry(value: &Value, uuid: Option<String>) -> Option<Entry> {
    let role = match value.get("type")?.as_str()? {
        "user" => TurnRole::User,
        "assistant" => TurnRole::Assistant,
        _ => return None,
    };
    if flag(value, "isSidechain") || flag(value, "isMeta") || flag(value, "isCompactSummary") {
        return None;
    }
    let message = value.get("message")?;
    Some(Entry {
        uuid,
        role,
        message_id: str_field(message, "id"),
        text: visible_text(message.get("content")),
    })
}

/// The text blocks of a message, ignoring thinking, tool calls and tool results.
fn visible_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// Where the last sync stopped in an agent's session file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Checkpoint {
    pub path: PathBuf,
    /// Bytes already read; always the end of a complete line.
    pub len: u64,
    /// Claude only: the last entry's uuid, where the next read picks up.
    pub last_uuid: Option<String>,
}

/// What a sync found. `checkpoint` is the new end of the file, or `None`
/// when there is no file to read or reading it failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Sync {
    pub turns: Vec<OutsideTurn>,
    pub grew: bool,
    pub checkpoint: Option<Checkpoint>,
}

impl Sync {
    fn nothing(checkpoint: Option<Checkpoint>) -> Self {
        Self {
            turns: Vec::new(),
            grew: false,
            checkpoint,
        }
    }
}

pub fn checkpoint_key(session_id: i64) -> String {
    format!("outside-sync:{session_id}")
}

/// The agent's own file for a session, or `None` for agents without one we
/// can read.
pub fn session_file(kind: Kind, home: &Path, provider_id: &str) -> Option<PathBuf> {
    match kind {
        Kind::Claude => crate::store::shared::claude_shell::transcript(home, provider_id),
        Kind::Codex => codex_file(home, provider_id),
        _ => None,
    }
}

fn codex_file(home: &Path, id: &str) -> Option<PathBuf> {
    use crate::accounts::transfer::codex::{find_thread, validate_thread_id};
    validate_thread_id(id).ok()?;
    for folder in ["sessions", "archived_sessions"] {
        let dir = home.join(folder);
        if !dir.is_dir() {
            continue;
        }
        match find_thread(&dir, id) {
            Ok(Some(path)) => return Some(path),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, dir = %dir.display(), "could not search Codex sessions")
            }
        }
    }
    None
}

/// The checkpoint to store once a turn Gespan ran has finished.
pub fn checkpoint_at_end(kind: Kind, home: &Path, provider_id: &str) -> Option<Checkpoint> {
    let path = session_file(kind, home, provider_id)?;
    match end_checkpoint(kind, &path) {
        Ok(checkpoint) => Some(checkpoint),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not checkpoint session file");
            None
        }
    }
}

/// Reads what the agent's own CLI added to a session since `checkpoint`.
///
/// Without a checkpoint, the turn typed as `last_prompt` is searched for and
/// only what follows its reply is imported; no match, or more than one, means
/// nothing is imported. Best effort: any failure imports nothing.
pub fn sync(
    kind: Kind,
    home: &Path,
    provider_id: &str,
    checkpoint: Option<&Checkpoint>,
    last_prompt: Option<&str>,
) -> Sync {
    let Some(path) = session_file(kind, home, provider_id) else {
        return Sync::nothing(None);
    };
    match read_sync(kind, &path, checkpoint, last_prompt) {
        Ok(sync) => sync,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not sync session file");
            Sync::nothing(None)
        }
    }
}

fn read_sync(
    kind: Kind,
    path: &Path,
    checkpoint: Option<&Checkpoint>,
    last_prompt: Option<&str>,
) -> io::Result<Sync> {
    // Compressed rollouts are archived and complete.
    if path.extension().is_some_and(|ext| ext == "zst") {
        let len = std::fs::metadata(path)?.len();
        let end = Checkpoint {
            path: path.to_path_buf(),
            len,
            last_uuid: None,
        };
        return Ok(Sync::nothing(Some(end)));
    }
    let size = std::fs::metadata(path)?.len();
    match checkpoint {
        Some(cp) if cp.path == path && cp.len <= size => {
            let region = read_from(path, cp.len)?;
            let parsed = parse(kind, &region, cp.last_uuid.as_deref());
            let consumed = parsed.consumed as usize;
            let last_uuid =
                last_uuid_in(kind, &region[..consumed]).or_else(|| cp.last_uuid.clone());
            Ok(Sync {
                turns: parsed.turns,
                grew: parsed.grew,
                checkpoint: Some(Checkpoint {
                    path: path.to_path_buf(),
                    len: cp.len + parsed.consumed,
                    last_uuid,
                }),
            })
        }
        // Another file, or one that shrank: what we remember no longer
        // describes it, so start over from its end.
        Some(cp) => {
            let end = end_checkpoint(kind, path)?;
            if cp.path != path {
                tracing::warn!(
                    old_path = %cp.path.display(),
                    new_path = %path.display(),
                    new_len = end.len,
                    "session file changed since the last sync; importing nothing"
                );
            } else {
                tracing::warn!(
                    path = %path.display(),
                    old_len = cp.len,
                    new_len = size,
                    "session file shrank since the last sync; importing nothing"
                );
            }
            Ok(Sync::nothing(Some(end)))
        }
        None => {
            let region = read_from(path, 0)?;
            let parsed = parse(kind, &region, None);
            let consumed = parsed.consumed as usize;
            let turns = after_last_prompt(parsed.turns, last_prompt);
            Ok(Sync {
                grew: !turns.is_empty(),
                turns,
                checkpoint: Some(Checkpoint {
                    path: path.to_path_buf(),
                    len: parsed.consumed,
                    last_uuid: last_uuid_in(kind, &region[..consumed]),
                }),
            })
        }
    }
}

fn parse(kind: Kind, region: &[u8], after_uuid: Option<&str>) -> Parsed {
    match kind {
        Kind::Claude => parse_claude(region, after_uuid),
        _ => parse_codex(region),
    }
}

fn read_from(path: &Path, from: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The turns after the reply to the one user turn that reads `prompt`.
fn after_last_prompt(turns: Vec<OutsideTurn>, prompt: Option<&str>) -> Vec<OutsideTurn> {
    let Some(prompt) = prompt.map(str::trim).filter(|p| !p.is_empty()) else {
        return Vec::new();
    };
    let mut matches = turns
        .iter()
        .enumerate()
        .filter(|(_, t)| t.role == TurnRole::User && t.text.trim() == prompt)
        .map(|(i, _)| i);
    let (Some(at), None) = (matches.next(), matches.next()) else {
        return Vec::new();
    };
    let replies = turns[at + 1..]
        .iter()
        .take_while(|t| t.role == TurnRole::Assistant)
        .count();
    turns.into_iter().skip(at + 1 + replies).collect()
}

fn last_uuid_in(kind: Kind, complete_lines: &[u8]) -> Option<String> {
    if kind != Kind::Claude {
        return None;
    }
    complete_lines
        .rsplit(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .find_map(|line| str_field(&serde_json::from_slice::<Value>(line).ok()?, "uuid"))
}

/// A checkpoint at the end of the file's last complete line, reading only as
/// much of its tail as needed to find the last uuid.
fn end_checkpoint(kind: Kind, path: &Path) -> io::Result<Checkpoint> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut window: u64 = 64 * 1024;
    loop {
        let start = size.saturating_sub(window);
        file.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::new();
        (&mut file).take(size - start).read_to_end(&mut buf)?;
        let whole = start == 0;
        let end = complete_len(&buf);
        let found = |last_uuid: Option<String>| Checkpoint {
            path: path.to_path_buf(),
            len: start + end as u64,
            last_uuid,
        };
        if end == 0 && whole {
            return Ok(found(None));
        }
        if end > 0 && kind != Kind::Claude {
            return Ok(found(None));
        }
        if end > 0 {
            // A window that starts mid-file begins inside a line; skip it.
            let body = if whole {
                Some(0)
            } else {
                buf.iter().position(|b| *b == b'\n').map(|i| i + 1)
            };
            if let Some(body) = body.filter(|b| *b < end) {
                if let Some(uuid) = last_uuid_in(kind, &buf[body..end]) {
                    return Ok(found(Some(uuid)));
                }
            }
            if whole {
                return Ok(found(None));
            }
        }
        window *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jsonl(entries: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in entries {
            out.extend_from_slice(e.to_string().as_bytes());
            out.push(b'\n');
        }
        out
    }

    fn user(uuid: &str, parent: Option<&str>, content: Value) -> Value {
        json!({"type":"user","uuid":uuid,"parentUuid":parent,"isSidechain":false,
            "message":{"role":"user","content":content}})
    }

    fn assistant(uuid: &str, parent: Option<&str>, msg_id: &str, content: Value) -> Value {
        json!({"type":"assistant","uuid":uuid,"parentUuid":parent,"isSidechain":false,
            "message":{"id":msg_id,"role":"assistant","content":content}})
    }

    fn texts(parsed: &Parsed) -> Vec<(TurnRole, &str)> {
        parsed
            .turns
            .iter()
            .map(|t| (t.role, t.text.as_str()))
            .collect()
    }

    #[test]
    fn user_and_assistant_text_become_turns() {
        let region = jsonl(&[
            user("u1", None, json!("hi")),
            assistant(
                "a1",
                Some("u1"),
                "m1",
                json!([{"type":"text","text":"Hello!"}]),
            ),
        ]);
        let parsed = parse_claude(&region, None);
        assert_eq!(
            texts(&parsed),
            vec![(TurnRole::User, "hi"), (TurnRole::Assistant, "Hello!")]
        );
        assert!(parsed.grew);
        assert_eq!(parsed.consumed, region.len() as u64);
    }

    #[test]
    fn bookkeeping_sidechain_meta_and_tool_results_are_skipped() {
        let mut sidechain = user("s1", Some("u1"), json!("side"));
        sidechain["isSidechain"] = json!(true);
        let mut meta = user("m1", Some("u1"), json!("meta"));
        meta["isMeta"] = json!(true);
        let region = jsonl(&[
            json!({"type":"attachment","uuid":"x1","parentUuid":null}),
            json!({"type":"system","uuid":"x2","parentUuid":"x1","content":"note"}),
            json!({"type":"last-prompt","lastPrompt":"p"}),
            json!({"type":"file-history-snapshot","messageId":"z","snapshot":{}}),
            sidechain,
            meta,
            user(
                "t1",
                Some("x2"),
                json!([{"type":"tool_result","tool_use_id":"c","content":"secret output"}]),
            ),
        ]);
        let parsed = parse_claude(&region, None);
        assert!(parsed.turns.is_empty());
    }

    #[test]
    fn assistant_tool_use_and_thinking_are_dropped_text_kept() {
        let region = jsonl(&[assistant(
            "a1",
            None,
            "m1",
            json!([
                {"type":"thinking","thinking":"hmm"},
                {"type":"text","text":"Done."},
                {"type":"tool_use","id":"c","name":"Bash","input":{"command":"rm -rf x"}}
            ]),
        )]);
        let parsed = parse_claude(&region, None);
        assert_eq!(texts(&parsed), vec![(TurnRole::Assistant, "Done.")]);
    }

    #[test]
    fn tool_only_turn_yields_no_rows_but_counts_as_growth() {
        let region = jsonl(&[assistant(
            "a1",
            None,
            "m1",
            json!([{"type":"tool_use","id":"c","name":"Read","input":{}}]),
        )]);
        let parsed = parse_claude(&region, None);
        assert!(parsed.turns.is_empty());
        assert!(parsed.grew);
    }

    #[test]
    fn only_the_live_branch_is_imported() {
        let region = jsonl(&[
            user("u1", None, json!("one")),
            assistant("a1", Some("u1"), "m1", json!([{"type":"text","text":"A1"}])),
            user("u2", Some("a1"), json!("two")),
            assistant("a2", Some("u2"), "m2", json!([{"type":"text","text":"A2"}])),
            user("u3", Some("a1"), json!("three")),
            assistant("a3", Some("u3"), "m3", json!([{"type":"text","text":"A3"}])),
        ]);
        let parsed = parse_claude(&region, None);
        assert_eq!(
            texts(&parsed),
            vec![
                (TurnRole::User, "one"),
                (TurnRole::Assistant, "A1"),
                (TurnRole::User, "three"),
                (TurnRole::Assistant, "A3"),
            ]
        );
    }

    #[test]
    fn partial_trailing_line_is_left_for_next_time() {
        let mut region = jsonl(&[user("u1", None, json!("hi"))]);
        let complete = region.len();
        region.extend_from_slice(br#"{"type":"user","uuid":"u2","parentUuid":"u1","mess"#);
        let parsed = parse_claude(&region, None);
        assert_eq!(parsed.consumed, complete as u64);
        assert_eq!(texts(&parsed), vec![(TurnRole::User, "hi")]);
    }

    #[test]
    fn no_complete_line_consumes_nothing() {
        let parsed = parse_claude(br#"{"type":"user""#, None);
        assert_eq!(parsed.consumed, 0);
        assert!(parsed.turns.is_empty());
        assert!(!parsed.grew);
    }

    #[test]
    fn after_uuid_starts_after_a_known_entry() {
        let region = jsonl(&[
            user("u1", None, json!("one")),
            assistant("a1", Some("u1"), "m1", json!([{"type":"text","text":"A1"}])),
            user("u2", Some("a1"), json!("two")),
            assistant("a2", Some("u2"), "m2", json!([{"type":"text","text":"A2"}])),
        ]);
        let parsed = parse_claude(&region, Some("a1"));
        assert_eq!(
            texts(&parsed),
            vec![(TurnRole::User, "two"), (TurnRole::Assistant, "A2")]
        );
    }

    #[test]
    fn compact_summaries_are_skipped() {
        let mut summary = user(
            "u2",
            Some("a1"),
            json!("This session is being continued..."),
        );
        summary["isCompactSummary"] = json!(true);
        let region = jsonl(&[
            user("u1", None, json!("one")),
            assistant("a1", Some("u1"), "m1", json!([{"type":"text","text":"A1"}])),
            summary,
        ]);
        let parsed = parse_claude(&region, None);
        assert_eq!(
            texts(&parsed),
            vec![(TurnRole::User, "one"), (TurnRole::Assistant, "A1")]
        );
    }

    #[test]
    fn one_reply_split_across_lines_is_merged() {
        let region = jsonl(&[
            user("u1", None, json!("go")),
            assistant(
                "a1",
                Some("u1"),
                "m1",
                json!([{"type":"text","text":"First."}]),
            ),
            assistant(
                "a2",
                Some("a1"),
                "m1",
                json!([{"type":"tool_use","id":"c","name":"Bash","input":{}}]),
            ),
            user(
                "t1",
                Some("a2"),
                json!([{"type":"tool_result","tool_use_id":"c","content":"out"}]),
            ),
            assistant(
                "a3",
                Some("t1"),
                "m1",
                json!([{"type":"text","text":"Second."}]),
            ),
            assistant(
                "a4",
                Some("a3"),
                "m2",
                json!([{"type":"text","text":"Other."}]),
            ),
        ]);
        let parsed = parse_claude(&region, None);
        assert_eq!(
            texts(&parsed),
            vec![
                (TurnRole::User, "go"),
                (TurnRole::Assistant, "First.\n\nSecond."),
                (TurnRole::Assistant, "Other."),
            ]
        );
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let mut region = b"not json\n{\"type\":\n".to_vec();
        region.extend(jsonl(&[user("u1", None, json!("hi"))]));
        let parsed = parse_claude(&region, None);
        assert_eq!(texts(&parsed), vec![(TurnRole::User, "hi")]);
    }

    #[test]
    fn turns_before_a_compact_boundary_are_kept() {
        let mut summary = user("c2", Some("c1"), json!("Summary of the session so far"));
        summary["isCompactSummary"] = json!(true);
        let region = jsonl(&[
            user("u1", None, json!("one")),
            assistant("a1", Some("u1"), "m1", json!([{"type":"text","text":"A1"}])),
            json!({"type":"system","subtype":"compact_boundary","uuid":"c1",
                "parentUuid":null,"logicalParentUuid":"a1"}),
            summary,
            user("u2", Some("c2"), json!("two")),
            assistant("a2", Some("u2"), "m2", json!([{"type":"text","text":"A2"}])),
        ]);
        let parsed = parse_claude(&region, None);
        assert_eq!(
            texts(&parsed),
            vec![
                (TurnRole::User, "one"),
                (TurnRole::Assistant, "A1"),
                (TurnRole::User, "two"),
                (TurnRole::Assistant, "A2"),
            ]
        );
    }

    #[test]
    fn a_very_long_chain_is_walked_in_linear_time() {
        let mut entries = Vec::new();
        let mut parent: Option<String> = None;
        for i in 0..20_000 {
            let uuid = format!("e{i}");
            entries.push(if i % 2 == 0 {
                user(&uuid, parent.as_deref(), json!(format!("q{i}")))
            } else {
                assistant(
                    &uuid,
                    parent.as_deref(),
                    &format!("m{i}"),
                    json!([{"type":"text","text":format!("r{i}")}]),
                )
            });
            parent = Some(uuid);
        }
        let parsed = parse_claude(&jsonl(&entries), None);
        assert_eq!(parsed.turns.len(), 20_000);
        assert_eq!(parsed.turns[0].text, "q0");
        assert_eq!(parsed.turns[19_999].text, "r19999");
    }

    // ---- sync -----------------------------------------------------------

    use crate::provider::Kind;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    const SID: &str = "11111111-2222-3333-4444-555555555555";

    fn claude_path(home: &Path) -> PathBuf {
        let dir = home.join("projects").join("-work-app");
        fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{SID}.jsonl"))
    }

    fn codex_path(home: &Path, folder: &str) -> PathBuf {
        let dir = home.join(folder).join("2026/11/11");
        fs::create_dir_all(&dir).unwrap();
        dir.join(format!("rollout-x-{SID}.jsonl"))
    }

    fn append(path: &Path, bytes: &[u8]) {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(bytes).unwrap();
    }

    fn reply(uuid: &str, parent: &str, id: &str, text: &str) -> Value {
        assistant(uuid, Some(parent), id, json!([{"type":"text","text":text}]))
    }

    fn codex_user(text: &str) -> Value {
        json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
    }

    fn codex_agent(text: &str) -> Value {
        json!({"type":"event_msg","payload":{"type":"agent_message","message":text}})
    }

    fn pairs(sync: &Sync) -> Vec<(TurnRole, &str)> {
        sync.turns
            .iter()
            .map(|t| (t.role, t.text.as_str()))
            .collect()
    }

    #[test]
    fn checkpoint_key_names_the_session() {
        assert_eq!(checkpoint_key(42), "outside-sync:42");
    }

    #[test]
    fn reads_only_past_the_checkpoint() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        append(&path, b"not json at all\n\xff\xfe invalid utf-8\n");
        append(
            &path,
            &jsonl(&[
                user("u1", None, json!("old")),
                reply("a1", "u1", "m1", "old reply"),
            ]),
        );
        let cp = checkpoint_at_end(Kind::Claude, home.path(), SID).unwrap();
        assert_eq!(cp.last_uuid.as_deref(), Some("a1"));
        assert_eq!(cp.len, fs::metadata(&path).unwrap().len());

        append(
            &path,
            &jsonl(&[
                user("u2", Some("a1"), json!("new")),
                reply("a2", "u2", "m2", "new reply"),
            ]),
        );
        // Without the uuid to stop at, only the byte offset keeps the old
        // turns out, so a re-read from the start would show them.
        let blind = Checkpoint {
            last_uuid: None,
            ..cp.clone()
        };
        let from_offset = sync(Kind::Claude, home.path(), SID, Some(&blind), None);
        assert_eq!(
            pairs(&from_offset),
            vec![(TurnRole::User, "new"), (TurnRole::Assistant, "new reply")]
        );
        let sync = sync(Kind::Claude, home.path(), SID, Some(&cp), None);
        assert_eq!(
            pairs(&sync),
            vec![(TurnRole::User, "new"), (TurnRole::Assistant, "new reply")]
        );
        assert!(sync.grew);
        let next = sync.checkpoint.unwrap();
        assert_eq!(next.len, fs::metadata(&path).unwrap().len());
        assert_eq!(next.last_uuid.as_deref(), Some("a2"));
    }

    #[test]
    fn codex_is_read_from_the_checkpoint_not_from_the_start() {
        let home = tempfile::tempdir().unwrap();
        let path = codex_path(home.path(), "sessions");
        append(
            &path,
            &jsonl(&[codex_user("old"), codex_agent("old reply")]),
        );
        let cp = checkpoint_at_end(Kind::Codex, home.path(), SID).unwrap();
        append(
            &path,
            &jsonl(&[codex_user("new"), codex_agent("new reply")]),
        );
        let sync = sync(Kind::Codex, home.path(), SID, Some(&cp), None);
        assert_eq!(
            pairs(&sync),
            vec![(TurnRole::User, "new"), (TurnRole::Assistant, "new reply")]
        );
    }

    #[test]
    fn a_partial_trailing_line_stays_behind_the_checkpoint() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        append(&path, &jsonl(&[user("u1", None, json!("one"))]));
        let cp = checkpoint_at_end(Kind::Claude, home.path(), SID).unwrap();
        let mut line = jsonl(&[reply("a1", "u1", "m1", "two")]);
        let tail = line.split_off(20);
        append(&path, &line);
        let first = sync(Kind::Claude, home.path(), SID, Some(&cp), None);
        assert!(first.turns.is_empty());
        let mid = first.checkpoint.unwrap();
        assert_eq!(mid.len, cp.len);
        append(&path, &tail);
        let second = sync(Kind::Claude, home.path(), SID, Some(&mid), None);
        assert_eq!(pairs(&second), vec![(TurnRole::Assistant, "two")]);
    }

    #[test]
    fn no_new_bytes_means_no_growth() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        append(&path, &jsonl(&[user("u1", None, json!("hi"))]));
        let cp = checkpoint_at_end(Kind::Claude, home.path(), SID).unwrap();
        let sync = sync(Kind::Claude, home.path(), SID, Some(&cp), None);
        assert!(sync.turns.is_empty());
        assert!(!sync.grew);
        assert_eq!(sync.checkpoint, Some(cp));
    }

    #[test]
    fn checkpoint_for_another_file_imports_nothing() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        append(
            &path,
            &jsonl(&[user("u1", None, json!("hi")), reply("a1", "u1", "m1", "yo")]),
        );
        let stale = Checkpoint {
            path: home.path().join("elsewhere.jsonl"),
            len: 0,
            last_uuid: None,
        };
        let sync = sync(Kind::Claude, home.path(), SID, Some(&stale), None);
        assert!(sync.turns.is_empty());
        assert!(!sync.grew);
        let next = sync.checkpoint.unwrap();
        assert_eq!(next.path, path);
        assert_eq!(next.len, fs::metadata(&path).unwrap().len());
        assert_eq!(next.last_uuid.as_deref(), Some("a1"));
    }

    #[test]
    fn shrunk_file_imports_nothing() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        append(&path, &jsonl(&[user("u1", None, json!("hi"))]));
        let size = fs::metadata(&path).unwrap().len();
        let cp = Checkpoint {
            path: path.clone(),
            len: size + 500,
            last_uuid: Some("gone".into()),
        };
        let sync = sync(Kind::Claude, home.path(), SID, Some(&cp), None);
        assert!(sync.turns.is_empty());
        assert!(!sync.grew);
        assert_eq!(sync.checkpoint.unwrap().len, size);
    }

    fn two_prompts(home: &Path) -> PathBuf {
        let path = claude_path(home);
        append(
            &path,
            &jsonl(&[
                user("u1", None, json!("first")),
                reply("a1", "u1", "m1", "R1"),
                user("u2", Some("a1"), json!("second")),
                reply("a2", "u2", "m2", "R2"),
                user("u3", Some("a2"), json!("typed outside")),
                reply("a3", "u3", "m3", "R3"),
            ]),
        );
        path
    }

    #[test]
    fn unique_last_prompt_match_imports_what_followed() {
        let home = tempfile::tempdir().unwrap();
        let path = two_prompts(home.path());
        let sync = sync(Kind::Claude, home.path(), SID, None, Some("  second\n"));
        assert_eq!(
            pairs(&sync),
            vec![
                (TurnRole::User, "typed outside"),
                (TurnRole::Assistant, "R3")
            ]
        );
        assert!(sync.grew);
        let cp = sync.checkpoint.unwrap();
        assert_eq!(cp.len, fs::metadata(&path).unwrap().len());
        assert_eq!(cp.last_uuid.as_deref(), Some("a3"));
    }

    #[test]
    fn last_prompt_with_nothing_after_it_imports_nothing_but_checkpoints() {
        let home = tempfile::tempdir().unwrap();
        two_prompts(home.path());
        let sync = sync(Kind::Claude, home.path(), SID, None, Some("typed outside"));
        assert!(sync.turns.is_empty());
        assert!(!sync.grew);
        assert!(sync.checkpoint.is_some());
    }

    #[test]
    fn codex_last_prompt_match_imports_what_followed() {
        let home = tempfile::tempdir().unwrap();
        let path = codex_path(home.path(), "sessions");
        append(
            &path,
            &jsonl(&[
                codex_user("first"),
                codex_agent("R1"),
                codex_user("outside"),
                codex_agent("R2"),
            ]),
        );
        let sync = sync(Kind::Codex, home.path(), SID, None, Some("first"));
        assert_eq!(
            pairs(&sync),
            vec![(TurnRole::User, "outside"), (TurnRole::Assistant, "R2")]
        );
    }

    #[test]
    fn ambiguous_or_missing_last_prompt_imports_nothing() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        append(
            &path,
            &jsonl(&[
                user("u1", None, json!("again")),
                reply("a1", "u1", "m1", "R1"),
                user("u2", Some("a1"), json!("again")),
                reply("a2", "u2", "m2", "R2"),
            ]),
        );
        let size = fs::metadata(&path).unwrap().len();
        for prompt in [Some("again"), Some("never typed"), None] {
            let sync = sync(Kind::Claude, home.path(), SID, None, prompt);
            assert!(sync.turns.is_empty(), "{prompt:?}");
            assert!(!sync.grew, "{prompt:?}");
            assert_eq!(sync.checkpoint.unwrap().len, size, "{prompt:?}");
        }
    }

    #[test]
    fn unsupported_kind_and_missing_file_import_nothing() {
        let home = tempfile::tempdir().unwrap();
        claude_path(home.path());
        let none = sync(Kind::Cursor, home.path(), SID, None, Some("x"));
        assert!(none.turns.is_empty() && !none.grew && none.checkpoint.is_none());
        assert!(session_file(Kind::Cursor, home.path(), SID).is_none());
        assert!(checkpoint_at_end(Kind::Cursor, home.path(), SID).is_none());

        let missing = sync(Kind::Claude, home.path(), SID, None, Some("x"));
        assert!(missing.turns.is_empty() && !missing.grew && missing.checkpoint.is_none());
        assert!(checkpoint_at_end(Kind::Claude, home.path(), SID).is_none());
        let none = sync(Kind::Codex, home.path(), SID, None, None);
        assert!(none.checkpoint.is_none());
    }

    #[test]
    fn unsafe_provider_ids_never_locate_a_file() {
        let home = tempfile::tempdir().unwrap();
        claude_path(home.path());
        assert!(session_file(Kind::Claude, home.path(), "../x").is_none());
        assert!(session_file(Kind::Codex, home.path(), "").is_none());
    }

    #[test]
    fn codex_session_is_found_in_archived_sessions_too() {
        let home = tempfile::tempdir().unwrap();
        let path = codex_path(home.path(), "archived_sessions");
        append(&path, &jsonl(&[codex_user("hi"), codex_agent("yo")]));
        assert_eq!(session_file(Kind::Codex, home.path(), SID), Some(path));
        let live = codex_path(home.path(), "sessions");
        append(&live, &jsonl(&[codex_user("hi")]));
        assert_eq!(session_file(Kind::Codex, home.path(), SID), Some(live));
    }

    #[test]
    fn compressed_codex_files_are_never_imported() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("archived_sessions");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-x-{SID}.jsonl.zst"));
        fs::write(&path, b"\x28\xb5\x2f\xfd not really").unwrap();
        let sync = sync(Kind::Codex, home.path(), SID, None, Some("x"));
        assert!(sync.turns.is_empty() && !sync.grew);
        let cp = sync.checkpoint.unwrap();
        assert_eq!(cp.path, path);
        assert_eq!(cp.len, fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn checkpoint_at_end_of_a_big_file_finds_the_last_uuid() {
        let home = tempfile::tempdir().unwrap();
        let path = claude_path(home.path());
        let filler = "x".repeat(200_000);
        append(
            &path,
            &jsonl(&[
                user("u1", None, json!("hi")),
                reply("a1", "u1", "m1", &filler),
                json!({"type":"last-prompt","lastPrompt":"hi"}),
            ]),
        );
        let cp = checkpoint_at_end(Kind::Claude, home.path(), SID).unwrap();
        assert_eq!(cp.last_uuid.as_deref(), Some("a1"));
        assert_eq!(cp.len, fs::metadata(&path).unwrap().len());
    }
}
