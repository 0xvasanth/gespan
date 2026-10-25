//! Where a held turn may stop: never while a foreground command runs.
use super::Event;
use crate::tool::ToolState;
use std::collections::HashSet;

/// How a held turn came to rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldPoint {
    /// Interrupted between steps; nothing was running.
    Interrupted,
    /// Waiting on an approval or question, which stays pending.
    AwaitingInput,
    /// The agent answered and only background tasks remain, left running.
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Send the soft interrupt now.
    Interrupt,
    /// Report the session as held without interrupting it.
    Settle(HoldPoint),
}

/// Watches a turn's events and, once Hold is requested, says when it is safe to stop.
#[derive(Default)]
pub(super) struct Gate {
    requested: bool,
    running: HashSet<String>,
    background: HashSet<String>,
    /// The provider answered and is only waiting on background work.
    yielded: bool,
    settled: Option<HoldPoint>,
}

impl Gate {
    pub(super) fn request(&mut self) {
        self.requested = true;
    }
    pub(super) fn observe(&mut self, event: &Event) {
        match event {
            Event::Tool(tool) if tool.state == ToolState::Running => {
                self.running.insert(tool.id.clone());
                if !self.background.contains(&tool.id) {
                    self.woke();
                }
            }
            Event::Tool(tool) => {
                self.running.remove(&tool.id);
            }
            Event::Background(tasks) => {
                self.background = tasks.iter().filter_map(|t| t.tool_id.clone()).collect();
                self.yielded = !tasks.is_empty();
            }
            Event::Text(_) | Event::Reasoning(_) | Event::BeginAssistant => self.woke(),
            _ => {}
        }
    }
    /// New model output after a quiet rest means the agent is working again.
    fn woke(&mut self) {
        if self.settled == Some(HoldPoint::Background) {
            self.settled = None;
        }
        self.yielded = false;
    }
    fn foreground_running(&self) -> bool {
        self.running.iter().any(|id| !self.background.contains(id))
    }
    pub(super) fn decide(&mut self, awaiting_input: bool) -> Option<Action> {
        if !self.requested || self.settled == Some(HoldPoint::Interrupted) {
            return None;
        }
        let point = if awaiting_input {
            HoldPoint::AwaitingInput
        } else if self.foreground_running() {
            return None;
        } else if self.yielded {
            HoldPoint::Background
        } else {
            self.settled = Some(HoldPoint::Interrupted);
            return Some(Action::Interrupt);
        };
        if self.settled == Some(point) {
            return None;
        }
        self.settled = Some(point);
        Some(Action::Settle(point))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolCall, ToolState};

    fn tool(id: &str, state: ToolState) -> Event {
        let mut call = ToolCall::new(id, "Bash");
        call.state = state;
        Event::Tool(call)
    }
    fn background(tool: &str) -> Event {
        Event::Background(vec![super::super::BackgroundTask {
            id: "b".into(),
            description: "dev server".into(),
            agent: false,
            command: None,
            started_at: 0,
            tool_id: Some(tool.into()),
            latest: None,
            steps: 0,
        }])
    }

    #[test]
    fn nothing_happens_until_hold_is_requested() {
        let mut gate = Gate::default();
        gate.observe(&Event::Text("x".into()));
        assert_eq!(gate.decide(false), None);
    }

    #[test]
    fn streaming_turn_is_interrupted_at_once() {
        let mut gate = Gate::default();
        gate.request();
        assert_eq!(gate.decide(false), Some(Action::Interrupt));
    }

    #[test]
    fn running_command_is_waited_for_then_interrupted() {
        let mut gate = Gate::default();
        gate.observe(&tool("c1", ToolState::Running));
        gate.request();
        assert_eq!(gate.decide(false), None);
        gate.observe(&tool("c1", ToolState::Completed));
        assert_eq!(gate.decide(false), Some(Action::Interrupt));
    }

    #[test]
    fn pending_input_settles_once_without_interrupting() {
        let mut gate = Gate::default();
        gate.request();
        assert_eq!(
            gate.decide(true),
            Some(Action::Settle(HoldPoint::AwaitingInput))
        );
        assert_eq!(gate.decide(true), None);
    }

    #[test]
    fn background_only_settles_and_waking_up_interrupts() {
        let mut gate = Gate::default();
        gate.observe(&tool("t0", ToolState::Running));
        gate.observe(&background("t0"));
        gate.request();
        assert_eq!(
            gate.decide(false),
            Some(Action::Settle(HoldPoint::Background))
        );
        assert_eq!(gate.decide(false), None);
        // The agent woke itself after the background result.
        gate.observe(&Event::Text("Server is up".into()));
        assert_eq!(gate.decide(false), Some(Action::Interrupt));
    }

    #[test]
    fn background_tool_does_not_count_as_a_running_command() {
        let mut gate = Gate::default();
        gate.observe(&background("t0"));
        gate.observe(&tool("t0", ToolState::Running));
        gate.request();
        assert_eq!(
            gate.decide(false),
            Some(Action::Settle(HoldPoint::Background))
        );
    }
}
