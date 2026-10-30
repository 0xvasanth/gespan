//! cursor.ts's live store reads: while a Cursor turn runs, fill in the tool
//! arguments its ACP events leave out, and mirror each subagent's own work
//! into its Agent card (cursorSubagents.ts). A last read runs as the turn ends.
use super::cursor_store::{self, CursorSubagentRun, CursorToolCall};
use super::{Event, Protocol};
use crate::tool::tasks::{AgentStep, Panel};
use crate::tool::{ToolCall, ToolState};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Arguments land in the store just before the call's result; ask soon.
const ARGS_POLL: Duration = Duration::from_millis(250);
const AGENTS_POLL: Duration = Duration::from_secs(1);
/// A call whose arguments never appear stops being asked about.
const ARGS_ATTEMPTS: u32 = 20;

#[derive(Default)]
pub(super) struct Live {
    /// `~/.cursor`, or a test's folder.
    pub(super) dir: Option<PathBuf>,
    pending_args: HashMap<String, u32>,
    /// Agent calls in the order they started.
    agents: Vec<String>,
    /// Each child store's last read revision, so unchanged runs are skipped.
    revisions: HashMap<String, String>,
    next: Option<Instant>,
}

/// cursorSubagents.ts's kindFromCursorToolName.
pub(super) fn kind_from_tool_name(name: &str) -> Option<&'static str> {
    let key = name.to_lowercase();
    Some(match key.as_str() {
        "grep" | "glob" | "rg" => "search",
        _ if key.contains("search") => "search",
        "read" => "read",
        "edit" | "write" | "strreplace" | "applypatch" => "edit",
        "shell" | "bash" => "execute",
        "skill" | "skills" => "skill",
        "agent" | "task" | "subagent" => "agent",
        "todowrite" | "updatetodos" | "writetodos" => "tasks",
        _ => return None,
    })
}

/// A caption worth keeping as the agent's name, without "Agent:" or a
/// placeholder such as "Subagent task".
fn agent_label(value: &str) -> Option<String> {
    let mut label = value
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, ':' | '·' | '-') || ch.is_whitespace());
    for prefix in ["agent", "task"] {
        if label.len() > prefix.len() && label[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let rest = label[prefix.len()..].trim_start();
            if let Some(rest) = rest.strip_prefix([':', '·', '-']) {
                label = rest.trim();
            }
        }
    }
    let placeholder = ["subagent", "subagent task", "task", "agent", "unspecified"]
        .iter()
        .any(|word| label.eq_ignore_ascii_case(word));
    (!label.is_empty() && !placeholder).then(|| label.to_owned())
}

/// The prompt's first line, as "Review X" for review requests, without a path.
fn prompt_label(prompt: &str) -> Option<String> {
    let line = prompt.trim().lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let lower = line.to_lowercase();
    let mut label = line.to_owned();
    if ["perform", "conduct", "do"]
        .iter()
        .any(|verb| lower.starts_with(verb))
    {
        if let Some(at) = lower.find(" review of ") {
            label = format!("Review {}", &line[at + " review of ".len()..]);
        }
    }
    if let Some(at) = label.find(" in /").or_else(|| {
        label
            .match_indices(" in ")
            .find(|(at, _)| {
                let rest = &label.as_bytes()[at + 4..];
                rest.len() > 2 && rest[0].is_ascii_alphabetic() && rest[1] == b':'
            })
            .map(|(at, _)| at)
    }) {
        label.truncate(at);
    }
    let label = label.trim_end_matches(['.', ' ']).to_owned();
    (!label.is_empty()).then_some(label)
}

/// preview.ts's formatAgentType: separators to spaces, words capitalized.
fn format_agent_type(value: &str) -> String {
    let text = value.trim().replace(['_', '-'], " ");
    if text.is_empty() {
        return "Subagent".into();
    }
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for ch in text.chars() {
        out.extend(if start {
            ch.to_uppercase().collect::<Vec<_>>()
        } else {
            vec![ch]
        });
        start = !ch.is_alphanumeric();
    }
    out
}

fn empty_input(input: &str) -> bool {
    matches!(input.trim(), "" | "{}" | "null")
}

fn is_agent(call: &ToolCall) -> bool {
    matches!(call.panel(), Some(Panel::Agent(_)))
}

impl Protocol {
    /// Note a Cursor call that may need its arguments or child run read.
    pub(super) fn cursor_track(&mut self, id: &str) {
        let Some(call) = self.tools.get(id) else {
            return;
        };
        let live = &mut self.acp.cursor;
        if empty_input(&call.input) {
            live.pending_args.entry(id.to_owned()).or_insert(0);
        }
        if is_agent(call) && !live.agents.iter().any(|known| known == id) {
            live.agents.push(id.to_owned());
        }
        if live.next.is_none() && (!live.pending_args.is_empty() || !live.agents.is_empty()) {
            live.next = Some(Instant::now());
        }
    }

    /// Read the store when due, or now and for the last time when `finished`.
    pub(super) fn cursor_tick(&mut self, finished: bool) -> Vec<Event> {
        let due = self.acp.cursor.next.is_some_and(|at| Instant::now() >= at);
        if !(due || finished && self.acp.cursor.next.is_some()) {
            return Vec::new();
        }
        let Some(dir) = self
            .acp
            .cursor
            .dir
            .clone()
            .or_else(cursor_store::cursor_dir)
        else {
            self.acp.cursor.next = None;
            return Vec::new();
        };
        let session = self.acp.session.clone();
        let mut events = Vec::new();

        let wanted: Vec<String> = self.acp.cursor.pending_args.keys().cloned().collect();
        if !wanted.is_empty() {
            for stored in cursor_store::tool_calls(&dir, &session, &wanted).unwrap_or_default() {
                self.acp.cursor.pending_args.remove(&stored.tool_call_id);
                if let Some(event) = self.cursor_apply_args(stored) {
                    events.push(event);
                }
            }
            self.acp.cursor.pending_args.retain(|_, attempts| {
                *attempts += 1;
                *attempts < ARGS_ATTEMPTS
            });
        }

        let runs = cursor_store::subagent_runs(
            &dir,
            &session,
            &self.acp.cursor.agents,
            &self.acp.cursor.revisions,
        );
        for run in runs {
            self.acp
                .cursor
                .revisions
                .insert(run.agent_id.clone(), run.revision.clone());
            if let Some(event) = self.cursor_apply_run(run) {
                events.push(event);
            }
        }

        let live = &mut self.acp.cursor;
        live.next = if finished {
            None
        } else if !live.pending_args.is_empty() {
            Some(Instant::now() + ARGS_POLL)
        } else if !live.agents.is_empty() {
            Some(Instant::now() + AGENTS_POLL)
        } else {
            None
        };
        events
    }

    fn cursor_apply_args(&mut self, stored: CursorToolCall) -> Option<Event> {
        let call = self.tools.get_mut(&stored.tool_call_id)?;
        if !empty_input(&call.input) {
            return None;
        }
        call.input = serde_json::to_string_pretty(&stored.args).unwrap_or_default();
        let display = call.display.get_or_insert_with(|| json!({}));
        let known = display["kind"]
            .as_str()
            .is_some_and(|kind| !matches!(kind, "" | "other"));
        if !known {
            if let Some(kind) = kind_from_tool_name(&stored.tool_name) {
                display["kind"] = json!(kind);
            }
        }
        call.refresh_agent_input();
        let event = Event::Tool(call.clone());
        self.cursor_track(&stored.tool_call_id);
        Some(event)
    }

    fn cursor_apply_run(&mut self, run: CursorSubagentRun) -> Option<Event> {
        let call = self.tools.get_mut(&run.tool_call_id)?;
        let title = call
            .display
            .as_ref()
            .and_then(|display| display["title"].as_str())
            .unwrap_or(&call.name)
            .to_owned();
        let type_label = run.agent_type.as_deref().map_or_else(
            || "Subagent".to_owned(),
            |kind| format!("{} subagent", format_agent_type(kind)),
        );
        let name = agent_label(&title)
            .filter(|label| *label != type_label)
            .or_else(|| run.prompt.as_deref().and_then(prompt_label))
            .unwrap_or(type_label);
        let agent = call.agent_mut()?;
        agent.name = Some(name);
        if run.model.is_some() {
            agent.model = run.model.clone();
        }
        if run.agent_type.is_some() {
            agent.agent_type = run.agent_type.clone();
        }
        if agent.brief.is_empty() {
            agent.brief = run.prompt.clone().unwrap_or_default();
        }
        for step in run.steps {
            agent.upsert(if step.kind == "tool" {
                let tool_name = step.tool_name.as_deref().unwrap_or("Tool");
                let mut child = ToolCall::new(&step.id, tool_name);
                child.input = step
                    .args
                    .as_ref()
                    .filter(|args| args.is_object())
                    .map_or_else(|| "{}".into(), Value::to_string);
                child.output = step.output.unwrap_or_default();
                child.state = match step.status {
                    Some("completed") => ToolState::Completed,
                    Some("failed") => ToolState::Failed,
                    _ => ToolState::Running,
                };
                if let Some(kind) = kind_from_tool_name(tool_name) {
                    child.display = Some(json!({ "kind": kind }));
                }
                AgentStep::from_tool_call(&child)
            } else {
                AgentStep {
                    id: step.id,
                    kind: step.kind.into(),
                    text: step.text,
                    ..AgentStep::default()
                }
            });
        }
        Some(Event::Tool(call.clone()))
    }
}

#[cfg(test)]
mod tests;
