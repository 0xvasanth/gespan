//! My tasks: what waits on the user, and the recent history shown with it.
use crate::flow::{FlowVisit, VisitStatus};
use crate::tool::ToolCall;
use crate::transcript::Row;
use std::collections::HashSet;

/// One line of a turn in the read-only history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    You {
        text: String,
        at: Option<u64>,
    },
    Agent {
        text: String,
        at: Option<u64>,
    },
    /// Back-to-back tool calls, shown as one line that never expands.
    Tools {
        summaries: Vec<String>,
        at: Option<u64>,
    },
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
    // Find where each turn starts without reading what the rows say, so a
    // long session only parses the turns it shows.
    let mut user_turns = Vec::new();
    let mut leading = false;
    let mut at = 0;
    while at < rows.len() {
        let start = at;
        at += 1;
        while at < rows.len() && !rows[at].first && rows[at].role == rows[start].role {
            at += 1;
        }
        let said = || rows[start..at].iter().any(|r| !r.text.trim().is_empty());
        match rows[start].role.as_str() {
            "user" if said() => user_turns.push(start),
            "assistant" if user_turns.is_empty() && said() => leading = true,
            "tool" if user_turns.is_empty() => leading = true,
            _ => {}
        }
    }
    let total = user_turns.len() + usize::from(leading);
    let earlier = total.saturating_sub(turns);
    let from = match earlier {
        0 => 0,
        n => user_turns[n - usize::from(leading)],
    };
    History {
        turns: build_turns(&rows[from..], cwd),
        earlier,
    }
}

/// Every turn of `rows`, oldest first.
fn build_turns(rows: &[Row], cwd: Option<&str>) -> Vec<Turn> {
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
    all
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

/// The latest `choices` card a Pilot posted, while no user message follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenChoice {
    pub row: usize,
    pub options: Vec<String>,
    pub at: Option<u64>,
}

/// The Pilot chat's open choice, if its last card asks one and the user has
/// not written since.
pub fn open_choice(rows: &[Row]) -> Option<OpenChoice> {
    let at = rows
        .iter()
        .rposition(|r| r.first && (r.role == "pilot-card" || r.role == "user"))?;
    let row = &rows[at];
    if row.role != "pilot-card" {
        return None;
    }
    let card: serde_json::Value = serde_json::from_str(&row.text).ok()?;
    if card["type"] != "choices" {
        return None;
    }
    let options: Vec<String> = card["options"]
        .as_array()?
        .iter()
        .filter_map(|o| o.as_str().map(str::to_owned))
        .collect();
    (!options.is_empty()).then_some(OpenChoice {
        row: at,
        options,
        at: row.sent_at,
    })
}

/// What the user wrote to the Pilot after its choices card at `row`: the
/// answer when they replied in the chat instead of picking on My tasks.
pub fn choice_reply(rows: &[Row], row: usize) -> Option<String> {
    rows.get(row + 1..)?
        .iter()
        .find(|r| r.first && r.role == "user")
        .map(|r| r.text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// The step of a Flow run that waits for approval.
pub fn waiting_visit(visits: &[FlowVisit]) -> Option<&FlowVisit> {
    visits
        .iter()
        .rev()
        .find(|v| v.status == VisitStatus::WaitingApproval)
}

/// A Flow run as one turn: its earlier steps on one line, then what the
/// waiting step asks.
pub fn flow_history(visits: &[FlowVisit]) -> History {
    let mut entries = Vec::new();
    let steps: Vec<String> = visits
        .iter()
        .filter(|v| v.status != VisitStatus::WaitingApproval)
        .map(|v| {
            let status = match v.status {
                VisitStatus::Completed => "passed",
                status => status.as_str(),
            };
            format!("Step {} · {} · {status}", v.seq, v.state_id)
        })
        .collect();
    if !steps.is_empty() {
        let at = visits
            .iter()
            .map(|v| v.finished_at.unwrap_or(v.started_at))
            .max();
        entries.push(Entry::Tools {
            summaries: steps,
            at: at.and_then(|ms| u64::try_from(ms).ok()),
        });
    }
    let waiting = waiting_visit(visits);
    let prompt = waiting.and_then(|w| w.prompt.as_deref().filter(|p| !p.trim().is_empty()));
    if let (Some(waiting), Some(prompt)) = (waiting, prompt) {
        entries.push(Entry::Agent {
            text: prompt.trim().into(),
            at: u64::try_from(waiting.started_at).ok(),
        });
    }
    History {
        turns: if entries.is_empty() {
            Vec::new()
        } else {
            vec![Turn { entries }]
        },
        earlier: 0,
    }
}

/// What a task asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Question,
    Approval,
    FlowApproval,
    PilotChoice,
    /// A draft review Gespan left on a pull or merge request: submit it or
    /// keep it as a draft.
    DraftReview,
    /// A decision-log proposal to approve or reject, or an agent's flag on
    /// an approved entry.
    Decision,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Question,
        Kind::Approval,
        Kind::FlowApproval,
        Kind::PilotChoice,
        Kind::DraftReview,
        Kind::Decision,
    ];
    /// The filter menu's name for this kind.
    pub fn label(self) -> &'static str {
        match self {
            Self::Question => "Questions",
            Self::Approval => "Tool approvals",
            Self::FlowApproval => "Flow approvals",
            Self::PilotChoice => "Pilot choices",
            Self::DraftReview => "Draft reviews",
            Self::Decision => "Decision to review",
        }
    }
}

/// The WAITING filter: how long a task must have waited to be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MinWait {
    #[default]
    Any,
    Hour,
    EightHours,
}

impl MinWait {
    pub fn minutes(self) -> u64 {
        match self {
            Self::Any => 0,
            Self::Hour => 60,
            Self::EightHours => 480,
        }
    }
}

/// What the list shows. Everything is shown by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub hidden_projects: HashSet<String>,
    pub hidden_kinds: HashSet<Kind>,
    pub min_wait: MinWait,
    pub query: String,
}

impl Filter {
    /// Whether the filter menu hides anything; the search field is apart.
    pub fn is_active(&self) -> bool {
        !self.hidden_projects.is_empty()
            || !self.hidden_kinds.is_empty()
            || self.min_wait != MinWait::Any
    }
    pub fn keeps(&self, project: &str, kind: Kind, waited_ms: u64, text: &str) -> bool {
        let query = self.query.trim().to_lowercase();
        !self.hidden_projects.contains(project)
            && !self.hidden_kinds.contains(&kind)
            && waited_ms >= self.min_wait.minutes() * 60_000
            && (query.is_empty() || text.to_lowercase().contains(&query))
    }
}

/// Longest wait first: the earliest `since` leads; ties keep their order.
pub fn order<T>(items: &mut [T], since: impl Fn(&T) -> u64) {
    items.sort_by_key(since);
}

mod log;
pub use log::{SavedDecision, Status, TaskRecord};

#[cfg(test)]
mod tests;
