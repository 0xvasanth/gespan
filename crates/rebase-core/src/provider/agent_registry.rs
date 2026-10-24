//! Claude agent runs outlive the turn that dispatched them. A sub-agent resumed
//! with `SendMessage` reports its work under the original Agent call's id, so
//! the session must remember which call dispatched which agent, and which
//! block is currently showing that agent's resumed work.
use crate::tool::{tasks::Panel, ToolCall};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
struct Dispatch {
    agent_id: Option<String>,
    name: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct AgentRegistry {
    /// Agent tool_use id -> what it dispatched.
    dispatches: HashMap<String, Dispatch>,
    /// Agent id -> the Agent tool_use id that dispatched it.
    agents: HashMap<String, String>,
    /// Resumption or continuation block id -> the agent id it shows.
    resumes: HashMap<String, String>,
    /// Agent id -> the block receiving its live resumed work.
    active: HashMap<String, String>,
}

impl AgentRegistry {
    pub fn note_dispatch(&mut self, tool_id: &str, agent_id: Option<&str>, name: &str) {
        if self.resumes.contains_key(tool_id) {
            return;
        }
        let dispatch = self.dispatches.entry(tool_id.into()).or_default();
        if !name.trim().is_empty() {
            dispatch.name = name.trim().into();
        }
        if let Some(agent_id) = agent_id {
            self.note_agent_id(tool_id, agent_id);
        }
    }
    pub fn note_agent_id(&mut self, tool_id: &str, agent_id: &str) {
        if self.resumes.contains_key(tool_id) || agent_id.trim().is_empty() {
            return;
        }
        self.dispatches.entry(tool_id.into()).or_default().agent_id = Some(agent_id.into());
        self.agents.insert(agent_id.into(), tool_id.into());
    }
    /// `send_tool_id` now shows `agent_id`'s resumed work; the latest resume wins.
    pub fn note_resume(&mut self, send_tool_id: &str, agent_id: &str) {
        self.dispatches.remove(send_tool_id);
        self.resumes.insert(send_tool_id.into(), agent_id.into());
        self.active.insert(agent_id.into(), send_tool_id.into());
    }
    /// The agent's resumed run ended; later stray frames get a continuation.
    pub fn note_finished(&mut self, agent_id: &str) {
        self.active.remove(agent_id);
    }
    /// Routing rule 2: the active resumption block of the agent `parent` dispatched.
    pub fn target_for_parent(&self, parent: &str) -> Option<String> {
        let agent = self.dispatches.get(parent)?.agent_id.as_deref()?;
        self.active.get(agent).cloned()
    }
    /// The active resumption block for an agent id reported on a child frame.
    pub fn target_for_agent(&self, agent_id: &str) -> Option<String> {
        self.active.get(agent_id).cloned()
    }
    /// Task events are keyed to the call that started the task (an Agent or a
    /// resuming SendMessage), and otherwise to the agent's task id.
    pub fn target_for_task(
        &self,
        tool_use_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Option<String> {
        if let Some(id) = tool_use_id {
            if self.resumes.contains_key(id) || self.dispatches.contains_key(id) {
                return Some(id.into());
            }
        }
        let task = task_id?;
        self.active
            .get(task)
            .or_else(|| self.agents.get(task))
            .cloned()
    }
    pub fn is_resume(&self, tool_id: &str) -> bool {
        self.resumes.contains_key(tool_id)
    }
    pub fn resumed_agent(&self, tool_id: &str) -> Option<&str> {
        self.resumes.get(tool_id).map(String::as_str)
    }
    /// The dispatch's display name, when `tool_id` is a known Agent call.
    pub fn dispatch_name(&self, tool_id: &str) -> Option<&str> {
        self.dispatches.get(tool_id).map(|d| d.name.as_str())
    }
    pub fn dispatch_agent(&self, tool_id: &str) -> Option<&str> {
        self.dispatches.get(tool_id)?.agent_id.as_deref()
    }
    /// The Agent call that dispatched `agent_id`, with its name.
    pub fn dispatch_of(&self, agent_id: &str) -> Option<(&str, &str)> {
        let tool_id = self.agents.get(agent_id)?;
        let dispatch = self.dispatches.get(tool_id)?;
        Some((tool_id.as_str(), dispatch.name.as_str()))
    }
    /// Rebuild from a conversation's persisted agent-run blocks. A new process
    /// cannot continue an old resumption's stream, so nothing is marked active.
    pub fn seed_from_blocks(&mut self, blocks: &[ToolCall]) {
        for block in blocks {
            if block.id.starts_with(super::tasks::FALLBACK_PREFIX) {
                continue;
            }
            let Some(Panel::Agent(run)) = block.panel() else {
                continue;
            };
            let agent = run.agent_id.clone().or_else(|| run.ids.first().cloned());
            if let Some(origin) = run.resume_of.as_deref() {
                if let Some(agent) = &agent {
                    self.resumes.insert(block.id.clone(), agent.clone());
                    if self.dispatch_agent(origin).is_none() {
                        self.note_agent_id(origin, agent);
                    }
                }
                continue;
            }
            self.note_dispatch(&block.id, agent.as_deref(), &run.display_name());
        }
    }
}
