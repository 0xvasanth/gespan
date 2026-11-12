//! My tasks: what waits on the user, and the recent history shown with it.
use crate::tool::ToolCall;
use crate::transcript::Row;

/// One line of a turn in the read-only history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    You { text: String, at: Option<u64> },
    Agent { text: String, at: Option<u64> },
    /// Back-to-back tool calls, shown as one line that never expands.
    Tools { summaries: Vec<String>, at: Option<u64> },
}

impl Entry {
    /// "1 tool call" or "N tool calls"; `None` for messages.
    pub fn tools_label(&self) -> Option<String> {
        match self {
            Self::Tools { summaries, .. } if summaries.len() == 1 => Some("1 tool call".into()),
            Self::Tools { summaries, .. } => Some(format!("{} tool calls", summaries.len())),
            _ => None,
        }
    }
}

/// One user message and everything after it until the next one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    pub entries: Vec<Entry>,
}

/// The last turns of a session, oldest first, and how many came before them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub turns: Vec<Turn>,
    pub earlier: usize,
}

/// The last `turns` turns of `rows`. Rows the transcript hides (reasoning,
/// Pilot system prompts and cards, interjections, blank replies) are left out.
pub fn history(rows: &[Row], cwd: Option<&str>, turns: usize) -> History {
    let mut all: Vec<Turn> = Vec::new();
    let mut at = 0;
    while at < rows.len() {
        let start = at;
        at += 1;
        while at < rows.len() && !rows[at].first && rows[at].role == rows[start].role {
            at += 1;
        }
        let text: String = rows[start..at].iter().map(|r| r.text.as_str()).collect();
        let sent = rows[start].sent_at;
        let entry = match rows[start].role.as_str() {
            "user" if !text.trim().is_empty() => Entry::You {
                text: text.trim().into(),
                at: sent,
            },
            "assistant" if !text.trim().is_empty() => Entry::Agent {
                text: text.trim().into(),
                at: sent,
            },
            "tool" => Entry::Tools {
                summaries: vec![tool_summary(&text, cwd)],
                at: sent,
            },
            _ => continue,
        };
        if matches!(entry, Entry::You { .. }) || all.is_empty() {
            all.push(Turn::default());
        }
        let turn = &mut all.last_mut().expect("a turn was pushed").entries;
        match (turn.last_mut(), entry) {
            (
                Some(Entry::Tools { summaries, at }),
                Entry::Tools {
                    summaries: more,
                    at: later,
                },
            ) => {
                summaries.extend(more);
                *at = later.or(*at);
            }
            (_, entry) => turn.push(entry),
        }
    }
    let earlier = all.len().saturating_sub(turns);
    History {
        turns: all.split_off(earlier),
        earlier,
    }
}

/// A tool call in one line: the model's own description of it when given,
/// else what it does and to what.
fn tool_summary(text: &str, cwd: Option<&str>) -> String {
    let Ok(call) = serde_json::from_str::<ToolCall>(text) else {
        return "Tool call".into();
    };
    let shown = call.presentation_in(cwd);
    if let Some(description) = shown.description.filter(|d| !d.trim().is_empty()) {
        return description;
    }
    let line = format!("{} {}", shown.action, shown.target);
    match line.trim() {
        "" => "Tool call".into(),
        line => line.into(),
    }
}

#[cfg(test)]
mod tests;
