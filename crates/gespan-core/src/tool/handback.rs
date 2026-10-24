//! Splits a Claude sub-agent hand-back tool result into a clean report and metadata.
use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Handback {
    pub report: String,
    pub agent_id: Option<String>,
    pub subagent_tokens: Option<u64>,
    pub tool_uses: Option<u64>,
    pub duration_ms: Option<u64>,
}

const PREAMBLE_LEAD: &str = "[Subagent hand-back]";
const PREAMBLE_END: &str = "The report follows:";

/// Splits `text` into the agent's report and the trailer metadata.
///
/// The trailer (`agentId: X ... subagent_tokens: N tool_uses: N duration_ms: N`, optionally
/// wrapped in `<usage>`) must sit at the very end and start at a line start; an `agentId:` line
/// inside the report is left alone. The `[Subagent hand-back] ... The report follows:` preamble
/// is removed and the harness's per-line indentation stripped. Anything else is returned as is.
pub fn parse_handback(text: &str) -> Handback {
    static TAIL: OnceLock<Regex> = OnceLock::new();
    static ID: OnceLock<Regex> = OnceLock::new();
    let tail = TAIL.get_or_init(|| {
        Regex::new(
            r"\Asubagent_tokens:\s*(\d+)\s*tool_uses:\s*(\d+)\s*duration_ms:\s*(\d+)\s*(?:</usage>)?\z",
        )
        .expect("tail regex")
    });
    let id = ID.get_or_init(|| Regex::new(r"\AagentId:\s*(\S+)").expect("id regex"));

    // Linear: trim the end once, anchor the usage tail at the last `subagent_tokens:`, then walk
    // back to the nearest line-start `agentId:` before it.
    let trimmed = text.trim_end();
    let mut body = text;
    let mut meta = None;
    if let Some(tail_start) = trimmed.rfind("subagent_tokens:") {
        if let Some(usage) = tail.captures(&trimmed[tail_start..]) {
            let mut end = tail_start;
            while let Some(at) = trimmed[..end].rfind("agentId:") {
                if at == 0 || trimmed.as_bytes()[at - 1] == b'\n' {
                    if let Some(found) = id.captures(&trimmed[at..]) {
                        meta = Some((
                            found[1].to_string(),
                            usage[1].parse::<u64>().ok(),
                            usage[2].parse::<u64>().ok(),
                            usage[3].parse::<u64>().ok(),
                        ));
                        body = &text[..at];
                        break;
                    }
                }
                end = at;
            }
        }
    }

    let mut stripped = false;
    if body.starts_with(PREAMBLE_LEAD) {
        if let Some(at) = body.find(PREAMBLE_END) {
            body = body[at + PREAMBLE_END.len()..].trim_start_matches(['\r', '\n']);
            stripped = true;
        }
    }

    if meta.is_none() && !stripped {
        return Handback {
            report: text.to_string(),
            ..Handback::default()
        };
    }
    let report = if stripped {
        dedent(body)
    } else {
        body.to_string()
    };
    let (agent_id, subagent_tokens, tool_uses, duration_ms) = match meta {
        Some((id, tokens, uses, ms)) => (Some(id), tokens, uses, ms),
        None => (None, None, None, None),
    };
    Handback {
        report: report.trim_end().to_string(),
        agent_id,
        subagent_tokens,
        tool_uses,
        duration_ms,
    }
}

/// Removes the leading whitespace shared by every non-blank line; blank lines become empty.
fn dedent(text: &str) -> String {
    let indent = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                l.chars().skip(indent).collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "../../tests/fixtures/claude/resumed-background-agent.jsonl";

    /// The text of the Agent tool_result that carries the hand-back (found by content).
    fn fixture_handback_text() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        let raw = std::fs::read_to_string(path).expect("fixture");
        for line in raw.lines() {
            let frame: serde_json::Value = serde_json::from_str(line).expect("frame json");
            let Some(blocks) = frame["message"]["content"].as_array() else {
                continue;
            };
            for block in blocks {
                if block["type"] != "tool_result" {
                    continue;
                }
                let text = match &block["content"] {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(parts) => parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(""),
                    _ => continue,
                };
                if text.contains("[Subagent hand-back]") {
                    return text;
                }
            }
        }
        panic!("no hand-back tool_result in fixture");
    }

    #[test]
    fn parses_the_fixture_hand_back() {
        let parsed = parse_handback(&fixture_handback_text());
        assert!(
            parsed.report.starts_with(
                "Here are the outputs of the two echo commands:\n\n1. `echo step-one`"
            ),
            "report starts at the agent's first line: {:?}",
            parsed.report
        );
        assert!(!parsed.report.contains("[Subagent hand-back]"));
        assert!(!parsed.report.contains("The report follows:"));
        assert!(!parsed.report.contains("agentId:"));
        assert!(!parsed.report.contains("<usage>"));
        assert!(parsed.report.ends_with("run `echo step-two` now."));
        assert!(parsed.report.lines().all(|l| !l.starts_with(' ')));
        assert_eq!(parsed.agent_id.as_deref(), Some("aba491c492e906740"));
        assert_eq!(parsed.subagent_tokens, Some(269110));
        assert_eq!(parsed.tool_uses, Some(2));
        assert_eq!(parsed.duration_ms, Some(9517));
    }

    #[test]
    fn plain_text_is_unchanged() {
        let text = "Just a normal tool result.\n  with indentation\n";
        assert_eq!(
            parse_handback(text),
            Handback {
                report: text.to_string(),
                ..Handback::default()
            }
        );
    }

    #[test]
    fn mid_text_agent_id_line_is_not_the_trailer() {
        let text = "Findings:\nagentId: fake\nmore text\nagentId: real1 (use SendMessage with to: 'real1') subagent_tokens: 10 tool_uses: 3 duration_ms: 400";
        let parsed = parse_handback(text);
        assert_eq!(parsed.agent_id.as_deref(), Some("real1"));
        assert_eq!(parsed.report, "Findings:\nagentId: fake\nmore text");
        assert_eq!(parsed.subagent_tokens, Some(10));
        assert_eq!(parsed.tool_uses, Some(3));
        assert_eq!(parsed.duration_ms, Some(400));
    }

    #[test]
    fn trailer_without_preamble_still_extracts_fields() {
        let text = "Done.\nagentId: abc123 (use SendMessage with to: 'abc123', summary: '<5-10 word recap>' to continue this agent)\n<usage>subagent_tokens: 5\ntool_uses: 1\nduration_ms: 20</usage>";
        let parsed = parse_handback(text);
        assert_eq!(parsed.report, "Done.");
        assert_eq!(parsed.agent_id.as_deref(), Some("abc123"));
        assert_eq!(parsed.subagent_tokens, Some(5));
        assert_eq!(parsed.tool_uses, Some(1));
        assert_eq!(parsed.duration_ms, Some(20));
    }

    #[test]
    fn dedent_keeps_nested_indentation_and_fenced_code() {
        let text = "[Subagent hand-back] x The report follows:\n  Plan:\n  - one\n    - nested\n  \n    ```rust\n    let a = 1;\n    ```\nagentId: z9 (use SendMessage) subagent_tokens: 1 tool_uses: 2 duration_ms: 3";
        let parsed = parse_handback(text);
        assert_eq!(
            parsed.report,
            "Plan:\n- one\n  - nested\n\n  ```rust\n  let a = 1;\n  ```"
        );
        assert_eq!(parsed.agent_id.as_deref(), Some("z9"));
    }

    #[test]
    fn crlf_input_is_parsed() {
        let text = "[Subagent hand-back] x The report follows:\r\n  Line one\r\n  Line two\r\nagentId: c1 (use SendMessage)\r\n<usage>subagent_tokens: 7\r\ntool_uses: 8\r\nduration_ms: 9</usage>\r\n";
        let parsed = parse_handback(text);
        assert_eq!(parsed.report, "Line one\nLine two");
        assert_eq!(parsed.agent_id.as_deref(), Some("c1"));
        assert_eq!(parsed.subagent_tokens, Some(7));
        assert_eq!(parsed.tool_uses, Some(8));
        assert_eq!(parsed.duration_ms, Some(9));
    }

    #[test]
    fn trailer_without_usage_wrapper_is_parsed() {
        let parsed = parse_handback(
            "Body\nagentId: w1 (hint)\nsubagent_tokens: 11\ntool_uses: 12\nduration_ms: 13\n",
        );
        assert_eq!(parsed.report, "Body");
        assert_eq!(parsed.agent_id.as_deref(), Some("w1"));
        assert_eq!(parsed.duration_ms, Some(13));
    }

    #[test]
    fn trailer_without_usage_numbers_is_plain_text() {
        let text = "Body\nagentId: only1 (use SendMessage with to: 'only1')";
        assert_eq!(
            parse_handback(text),
            Handback {
                report: text.to_string(),
                ..Handback::default()
            }
        );
    }

    #[test]
    fn empty_string_gives_empty_report() {
        assert_eq!(parse_handback(""), Handback::default());
    }
}
