//! Turns taken in an agent's own CLI, read back from its session file so
//! Gespan's chat can show them.
use std::collections::{HashMap, HashSet};

use serde_json::Value;

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
}
