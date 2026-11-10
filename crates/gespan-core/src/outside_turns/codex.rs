//! Turns from a region of a Codex rollout file.
use super::raw::CodexLine;
use super::{complete_len, visible_text, OutsideTurn, Parsed, TurnRole};

/// Reads turns from a region of a Codex rollout file. The `event_msg` entries
/// are preferred; `response_item` messages are the fallback when the region
/// has none.
pub fn parse_codex(region: &[u8]) -> Parsed {
    let consumed = complete_len(region);
    let mut events: Vec<OutsideTurn> = Vec::new();
    let mut items: Vec<OutsideTurn> = Vec::new();
    let mut saw_event = false;
    let mut grew = false;

    for line in region[..consumed].split(|b| *b == b'\n') {
        let Ok(line) = serde_json::from_slice::<CodexLine>(line) else {
            continue;
        };
        let Some(payload) = &line.payload else {
            continue;
        };
        match (line.kind.get(), payload.kind.get()) {
            (Some("event_msg"), Some(kind @ ("user_message" | "agent_message"))) => {
                grew = true;
                saw_event = true;
                let role = if kind == "user_message" {
                    TurnRole::User
                } else {
                    TurnRole::Assistant
                };
                let text = payload.message.get().map(str::trim).unwrap_or_default();
                push(&mut events, role, text);
            }
            (Some("event_msg"), Some("thread_rolled_back")) => {
                let n = payload.num_turns.as_ref().and_then(|n| n.0).unwrap_or(0);
                roll_back(&mut events, n);
                roll_back(&mut items, n);
            }
            (Some("response_item"), Some("message")) => {
                let (role, block) = match payload.role.get() {
                    Some("user") => (TurnRole::User, "input_text"),
                    Some("assistant") => (TurnRole::Assistant, "output_text"),
                    _ => continue,
                };
                grew = true;
                // Codex's message content is always a list of blocks.
                let text = match &payload.content {
                    super::raw::Content::Blocks(_) => visible_text(&payload.content, block),
                    _ => String::new(),
                };
                if role == TurnRole::User && is_injected(&text) {
                    continue;
                }
                push(&mut items, role, &text);
            }
            _ => {}
        }
    }

    Parsed {
        turns: if saw_event { events } else { items },
        consumed: consumed as u64,
        grew,
    }
}

fn push(turns: &mut Vec<OutsideTurn>, role: TurnRole, text: &str) {
    if !text.is_empty() {
        turns.push(OutsideTurn {
            role,
            text: text.to_string(),
        });
    }
}

/// Drops the last `n` turns, each being a user prompt and the replies after it.
fn roll_back(turns: &mut Vec<OutsideTurn>, n: u64) {
    for _ in 0..n {
        let Some(start) = turns.iter().rposition(|t| t.role == TurnRole::User) else {
            turns.clear();
            return;
        };
        turns.truncate(start);
    }
}

/// Context Codex injects as a user message: environment blocks and AGENTS.md.
fn is_injected(text: &str) -> bool {
    text.starts_with('<') || text.starts_with("# AGENTS.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn jsonl(entries: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in entries {
            out.extend_from_slice(e.to_string().as_bytes());
            out.push(b'\n');
        }
        out
    }

    fn event(kind: &str, message: &str) -> Value {
        json!({"type":"event_msg","payload":{"type":kind,"message":message}})
    }

    fn item(role: &str, block: &str, text: &str) -> Value {
        json!({"type":"response_item","payload":{"type":"message","role":role,
            "content":[{"type":block,"text":text}]}})
    }

    fn rollback(n: u64) -> Value {
        json!({"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":n}})
    }

    fn texts(parsed: &Parsed) -> Vec<(TurnRole, &str)> {
        parsed
            .turns
            .iter()
            .map(|t| (t.role, t.text.as_str()))
            .collect()
    }

    #[test]
    fn event_messages_become_turns() {
        let region = jsonl(&[
            json!({"type":"session_meta","payload":{"id":"x"}}),
            event("user_message", "fix it"),
            event("token_count", ""),
            event("agent_message", "Fixed."),
        ]);
        let parsed = parse_codex(&region);
        assert_eq!(
            texts(&parsed),
            vec![(TurnRole::User, "fix it"), (TurnRole::Assistant, "Fixed.")]
        );
        assert!(parsed.grew);
        assert_eq!(parsed.consumed, region.len() as u64);
    }

    #[test]
    fn injected_context_and_developer_messages_are_skipped() {
        let region = jsonl(&[
            item("developer", "input_text", "be careful"),
            item(
                "user",
                "input_text",
                "<environment_context>cwd</environment_context>",
            ),
            item("user", "input_text", "# AGENTS.md instructions for /repo"),
        ]);
        let parsed = parse_codex(&region);
        assert!(parsed.turns.is_empty());
        assert!(parsed.grew);
    }

    #[test]
    fn response_item_messages_are_used_when_no_event_messages() {
        let region = jsonl(&[
            item("developer", "input_text", "rules"),
            item("user", "input_text", "fix it"),
            item("assistant", "output_text", "Fixed."),
        ]);
        let parsed = parse_codex(&region);
        assert_eq!(
            texts(&parsed),
            vec![(TurnRole::User, "fix it"), (TurnRole::Assistant, "Fixed.")]
        );
    }

    #[test]
    fn event_messages_win_over_duplicate_response_items() {
        let region = jsonl(&[
            item("user", "input_text", "fix it"),
            event("user_message", "fix it"),
            item("assistant", "output_text", "Fixed."),
            event("agent_message", "Fixed."),
        ]);
        assert_eq!(parse_codex(&region).turns.len(), 2);
    }

    #[test]
    fn rolled_back_turns_are_dropped() {
        let region = jsonl(&[
            event("user_message", "one"),
            event("agent_message", "reply one"),
            event("user_message", "two"),
            event("agent_message", "reply two"),
            rollback(1),
        ]);
        assert_eq!(
            texts(&parse_codex(&region)),
            vec![(TurnRole::User, "one"), (TurnRole::Assistant, "reply one")]
        );
    }

    #[test]
    fn rollback_past_the_region_start_empties_it_and_works_for_response_items() {
        let region = jsonl(&[
            item("user", "input_text", "one"),
            item("assistant", "output_text", "reply"),
            rollback(5),
        ]);
        assert!(parse_codex(&region).turns.is_empty());
    }

    #[test]
    fn fields_of_an_unexpected_type_are_ignored_not_fatal() {
        let region = jsonl(&[
            event("user_message", "one"),
            json!({"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":"x"}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":{"text":"no"}}}),
            event("agent_message", "reply"),
        ]);
        let parsed = parse_codex(&region);
        assert_eq!(
            texts(&parsed),
            vec![(TurnRole::User, "one"), (TurnRole::Assistant, "reply")]
        );
    }

    #[test]
    fn partial_trailing_line_is_left_for_next_time() {
        let mut region = jsonl(&[event("user_message", "fix it")]);
        let complete = region.len();
        region.extend_from_slice(br#"{"type":"event_msg","payload":{"type":"agent_mess"#);
        let parsed = parse_codex(&region);
        assert_eq!(parsed.consumed, complete as u64);
        assert_eq!(texts(&parsed), vec![(TurnRole::User, "fix it")]);
    }

    #[test]
    fn malformed_lines_are_skipped_and_empty_messages_still_count_as_growth() {
        let mut region = b"not json\n".to_vec();
        region.extend(jsonl(&[event("agent_message", "  ")]));
        let parsed = parse_codex(&region);
        assert!(parsed.turns.is_empty());
        assert!(parsed.grew);
        assert_eq!(parsed.consumed, region.len() as u64);
        assert!(!parse_codex(b"{\"type\":\"turn_context\",\"payload\":{}}\n").grew);
    }
}
