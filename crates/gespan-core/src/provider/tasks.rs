//! Keep delegated traffic under its real parent, rather than merging it into
//! the assistant's answer or allowing a child completion to finish the turn.
use super::*;
use crate::tool::tasks::{AgentRun, AgentStep, Panel, TaskList};

#[derive(Default)]
pub(super) struct Activities {
    main_thread: Option<String>,
    owners: HashMap<String, String>,
    pending: Vec<Value>,
    reported_states: HashMap<String, ToolState>,
    pub(super) background_calls: std::collections::HashSet<String>,
    pub(super) claude_background: HashMap<String, String>,
    pub(super) claude_pending: std::collections::HashSet<String>,
    /// Every running Claude task in start order, as claude.ts's backgroundTasks.
    pub(super) claude_tasks: Vec<BackgroundTask>,
    /// What the UI was last told the yielded turn is waiting on.
    pub(super) published: Vec<BackgroundTask>,
    /// Which Agent call dispatched which sub-agent, across turns.
    pub(super) registry: super::agent_registry::AgentRegistry,
    /// Claude `system/init` frames seen; one per turn, user- or self-started.
    turn: u64,
    /// Unique per provider operation, so synthetic block ids never repeat in
    /// a session even though `turn` restarts with every `Protocol`.
    scope: Option<String>,
    /// The user wrote a follow-up after Claude yielded: the next turn answers
    /// them, so it must not lead with a background-wake marker.
    pub(super) user_turn_pending: bool,
    /// Summaries of the background tasks whose notifications may wake Claude,
    /// oldest first: one wake takes one.
    pub(super) wake_summaries: std::collections::VecDeque<String>,
    /// The running turn already took its wake summary when it started.
    pub(super) wake_handled: bool,
    /// The user wrote into the running turn, so its reply may not be a wake.
    pub(super) steered_in_turn: bool,
    /// Claude's TaskCreate/TaskUpdate list, seeded from saved cards.
    pub(super) claude_task_list: crate::tool::tasks::ClaudeTasks,
    /// This turn's task card, once a task call changed the list.
    claude_task_card: Option<String>,
}
/// Synthetic block ids for work whose parent this session cannot place.
pub(super) const FALLBACK_PREFIX: &str = crate::tool::tasks::SYNTHETIC_BLOCK_PREFIX;
const CONTINUED_PREFIX: &str = "continued:";
fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.trim().is_empty())
}
fn state(status: &str) -> Option<ToolState> {
    match status.to_lowercase().as_str() {
        "completed" | "complete" | "done" | "finished" | "closed" | "stopped" => {
            Some(ToolState::Completed)
        }
        "failed" | "error" | "errored" | "notfound" | "not_found" => Some(ToolState::Failed),
        "interrupted" | "cancelled" | "canceled" | "killed" | "aborted" => {
            Some(ToolState::Cancelled)
        }
        "running" | "inprogress" | "in_progress" | "pending" => Some(ToolState::Running),
        _ => None,
    }
}
impl Protocol {
    /// Seed the agent registry from the conversation's persisted agent runs.
    /// Called wherever a Claude process is (re)started for a session.
    pub(super) fn seed_agent_runs(&mut self) {
        let runs = std::mem::take(&mut self.request.settings.agent_runs);
        if self.request.kind == Kind::Claude {
            self.activities.registry.seed_from_blocks(&runs);
        }
    }
    /// Seed Claude's task list from the latest saved card of the conversation
    /// this turn resumes. A new conversation's task ids start over.
    pub(super) fn seed_claude_tasks(&mut self) {
        let lists = std::mem::take(&mut self.request.settings.claude_task_lists);
        if self.request.kind != Kind::Claude {
            return;
        }
        let Some(conversation) = self.request.provider_id.as_deref() else {
            return;
        };
        if let Some(list) = lists
            .into_iter()
            .rev()
            .find(|list| list.conversation == conversation)
        {
            self.activities.claude_task_list = list;
        }
    }
    /// A main-thread tool_result for TaskCreate or TaskUpdate: fold it into
    /// the task list and show the list as this turn's task card.
    pub(super) fn claude_task_result(
        &mut self,
        block: &Value,
        frame: &Value,
        events: &mut Vec<Event>,
    ) {
        if block["is_error"] == true {
            return;
        }
        let Some(id) = text(&block["tool_use_id"]) else {
            return;
        };
        let Some(tool) = self.tools.get(id) else {
            return;
        };
        if !matches!(tool.name.trim(), "TaskCreate" | "TaskUpdate") {
            return;
        }
        let input: Value = serde_json::from_str(&tool.input).unwrap_or_default();
        let conversation = text(&frame["session_id"])
            .or(self.request.provider_id.as_deref())
            .unwrap_or("")
            .to_owned();
        let list = &mut self.activities.claude_task_list;
        if list.conversation != conversation {
            // Task ids from another conversation mean nothing in this one.
            *list = crate::tool::tasks::ClaudeTasks {
                conversation,
                items: Vec::new(),
            };
        }
        let content = text_content(&block["content"]);
        if !list.apply(&tool.name, &input, &content, &frame["tool_use_result"]) {
            return;
        }
        let card = self
            .activities
            .claude_task_card
            .get_or_insert_with(|| format!("{}:{id}", crate::tool::tasks::CLAUDE_TASKS_KEY));
        events.push(Event::Tool(self.activities.claude_task_list.card(card)));
    }
    pub(super) fn activity(&mut self, value: &Value, events: &mut Vec<Event>) -> bool {
        match self.request.kind {
            Kind::Codex => self.codex_activity(value, events),
            Kind::Claude => self.claude_activity(value, events),
            _ => false,
        }
    }
    fn agent_step(
        &mut self,
        parent: &str,
        step: AgentStep,
        model: Option<&str>,
        events: &mut Vec<Event>,
    ) {
        let Some(tool) = self.tools.get_mut(parent) else {
            return;
        };
        let Some(run) = tool.agent_mut() else { return };
        if let Some(model) = model {
            run.model = Some(model.into());
        }
        run.upsert(step);
        let steps = run.tool_step_count() as u32;
        for task in &mut self.activities.claude_tasks {
            if task.tool_id.as_deref() == Some(parent) {
                task.steps = steps;
            }
        }
        events.push(Event::Tool(tool.clone()));
    }
    fn codex_activity(&mut self, value: &Value, events: &mut Vec<Event>) -> bool {
        if value["id"] == 2 {
            if let Some(id) = text(&value["result"]["thread"]["id"]) {
                self.activities.main_thread = Some(id.into());
            }
        }
        let p = &value["params"];
        let method = value["method"].as_str().unwrap_or("");
        // Approval/question requests still go through the shared approval UI.
        if value.get("id").is_some()
            && (method.ends_with("/requestApproval") || method == "item/tool/requestUserInput")
        {
            return false;
        }
        let thread = text(&p["threadId"])
            .or_else(|| text(&p["thread_id"]))
            .or_else(|| text(&p["thread"]["id"]));
        if let Some(parent) = thread
            .and_then(|id| self.activities.owners.get(id))
            .cloned()
        {
            self.codex_child(&parent, method, p, events);
            return true;
        }
        if let (Some(thread), Some(main)) = (thread, self.activities.main_thread.as_deref()) {
            if thread != main {
                // A child can speak before its spawn snapshot identifies it.
                if matches!(
                    method,
                    "item/started"
                        | "item/completed"
                        | "turn/completed"
                        | "turn/aborted"
                        | "thread/started"
                ) && self.activities.pending.len() < 64
                    && value.to_string().len() <= 16 * 1024
                {
                    self.activities.pending.push(value.clone());
                }
                return true;
            }
        }
        if method == "turn/completed" {
            for item in p["turn"]["items"].as_array().into_iter().flatten() {
                if matches!(
                    item["type"].as_str(),
                    Some("collabAgentToolCall" | "subAgentActivity")
                ) {
                    self.codex_activity(&json!({"method":"item/completed","params":{"threadId":thread,"item":item}}), events);
                }
            }
        }
        if method == "turn/plan/updated" {
            let Some(list) = TaskList::from_value(p) else {
                return true;
            };
            let Some(turn) = text(&p["turnId"]).or(self.active_turn.as_deref()) else {
                return true;
            };
            let id = format!("plan:{turn}");
            let tool = self
                .tools
                .entry(id.clone())
                .or_insert_with(|| ToolCall::new(&id, "update_plan"));
            tool.input = p.to_string();
            tool.panel = Some(Panel::Tasks(list));
            tool.state = ToolState::Completed; // A received snapshot, not a claim its tasks finished.
            events.push(Event::Tool(tool.clone()));
            return true;
        }
        if !matches!(method, "item/started" | "item/completed") {
            return false;
        }
        let item = &p["item"];
        if !matches!(
            item["type"].as_str(),
            Some("collabAgentToolCall" | "subAgentActivity")
        ) {
            return false;
        }
        let Some(id) = text(&item["id"]) else {
            return true;
        };
        let action = item["tool"].as_str().unwrap_or("");
        let mut ids: Vec<String> = ["agentThreadId", "agent_thread_id"]
            .iter()
            .filter_map(|key| text(&item[*key]).map(str::to_owned))
            .collect();
        for receiver in item
            .get("receiverThreadIds")
            .or_else(|| item.get("receiver_thread_ids"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(receiver) = text(receiver) {
                ids.push(receiver.into());
            }
        }
        let states = item
            .get("agentsStates")
            .or_else(|| item.get("agents_states"))
            .and_then(Value::as_object);
        if let Some(states) = states {
            ids.extend(states.keys().cloned());
        }
        ids.sort();
        ids.dedup();
        let spawn =
            matches!(action, "spawnAgent" | "resumeAgent") || item["type"] == "subAgentActivity";
        if spawn {
            let existing = ids
                .iter()
                .find_map(|id| self.activities.owners.get(id))
                .cloned();
            let owner = existing.as_deref().unwrap_or(id);
            let tool = self
                .tools
                .entry(owner.into())
                .or_insert_with(|| ToolCall::new(owner, "collabAgentToolCall"));
            if tool.panel.is_none() {
                tool.panel = Some(Panel::Agent(AgentRun::default()));
            }
            tool.input = item.to_string();
            if let Some(run) = tool.agent_mut() {
                for id in &ids {
                    if !run.ids.contains(id) {
                        run.ids.push(id.clone());
                    }
                    self.activities
                        .owners
                        .entry(id.clone())
                        .or_insert_with(|| owner.into());
                }
                // A second item may describe an already-named child. Keep the
                // original task title, but let it fill a missing path label.
                if owner == id || (run.name.is_none() && run.brief.is_empty()) {
                    if let Some(name) = AgentRun::from_input(item).name {
                        run.name = Some(name);
                    }
                }
                if let Some(model) = text(&item["model"]) {
                    run.model = Some(model.into());
                }
                if let Some(brief) = text(&item["prompt"]).or_else(|| text(&item["description"])) {
                    run.brief = brief.into();
                }
            }
            // The spawn RPC returning does not mean the agent has completed.
            if item["status"] == "failed" || item["kind"] == "interrupted" {
                tool.state = ToolState::Failed;
                if let Some(message) =
                    text(&item["error"]["message"]).or_else(|| text(&item["message"]))
                {
                    tool.output = message.into();
                    tool.bound_output();
                }
            }
            events.push(Event::Tool(tool.clone()));
        }
        if let Some(states) = states {
            for (thread, reported) in states {
                if let Some(status) = reported["status"].as_str().and_then(state) {
                    self.settle_child(thread, status, text(&reported["message"]), events);
                }
            }
        }
        let pending = std::mem::take(&mut self.activities.pending);
        for frame in pending {
            let thread =
                text(&frame["params"]["threadId"]).or_else(|| text(&frame["params"]["thread_id"]));
            if let Some(owner) = thread
                .and_then(|id| self.activities.owners.get(id))
                .cloned()
            {
                self.codex_child(
                    &owner,
                    frame["method"].as_str().unwrap_or(""),
                    &frame["params"],
                    events,
                );
            } else {
                self.activities.pending.push(frame);
            }
        }
        true
    }
    fn codex_child(&mut self, parent: &str, method: &str, params: &Value, events: &mut Vec<Event>) {
        if method == "thread/started" {
            if let Some(model) = text(&params["thread"]["model"]) {
                if let Some(tool) = self.tools.get_mut(parent) {
                    if let Some(run) = tool.agent_mut() {
                        run.model = Some(model.into());
                    }
                    events.push(Event::Tool(tool.clone()));
                }
            }
        } else if matches!(method, "item/started" | "item/completed") {
            let item = &params["item"];
            let Some(id) = text(&item["id"]) else { return };
            let kind = item["type"].as_str().unwrap_or("");
            if matches!(kind, "agentMessage" | "reasoning") {
                if method != "item/completed" {
                    return;
                }
                let narration = text(&item["text"]).map(str::to_owned).unwrap_or_else(|| {
                    item["summary"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| text(&part["text"]))
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                if !narration.is_empty() {
                    self.agent_step(
                        parent,
                        AgentStep {
                            id: id.into(),
                            kind: if kind == "reasoning" {
                                "reasoning"
                            } else {
                                "message"
                            }
                            .into(),
                            text: narration,
                            state: None,
                            detail: String::new(),
                            ..AgentStep::default()
                        },
                        None,
                        events,
                    );
                }
            } else {
                let mut child_events = Vec::new();
                self.codex_tool(item, method == "item/completed", &mut child_events);
                for child in child_events {
                    if let Event::Tool(child) = child {
                        self.agent_step(parent, AgentStep::from_tool_call(&child), None, events);
                    }
                }
            }
        } else if matches!(method, "turn/completed" | "turn/aborted") {
            if let (Some(thread), Some(status)) = (
                text(&params["threadId"]).or_else(|| text(&params["thread_id"])),
                params["turn"]["status"]
                    .as_str()
                    .and_then(state)
                    .or_else(|| (method == "turn/aborted").then_some(ToolState::Cancelled)),
            ) {
                self.settle_child(
                    thread,
                    status,
                    text(&params["turn"]["error"]["message"]),
                    events,
                );
            }
        }
    }
    fn settle_child(
        &mut self,
        thread: &str,
        status: ToolState,
        message: Option<&str>,
        events: &mut Vec<Event>,
    ) {
        self.activities
            .reported_states
            .insert(thread.into(), status);
        let Some(owner) = self.activities.owners.get(thread) else {
            return;
        };
        let Some(tool) = self.tools.get_mut(owner) else {
            return;
        };
        if let Some(run) = tool.agent_mut() {
            let states = &self.activities.reported_states;
            tool.state = if run
                .ids
                .iter()
                .any(|id| states.get(id) == Some(&ToolState::Failed))
            {
                ToolState::Failed
            } else if !run.ids.is_empty()
                && run.ids.iter().all(|id| {
                    matches!(
                        states.get(id),
                        Some(ToolState::Completed | ToolState::Cancelled)
                    )
                })
                && run
                    .ids
                    .iter()
                    .any(|id| states.get(id) == Some(&ToolState::Cancelled))
            {
                ToolState::Cancelled
            } else if !run.ids.is_empty()
                && run
                    .ids
                    .iter()
                    .all(|id| states.get(id) == Some(&ToolState::Completed))
            {
                ToolState::Completed
            } else {
                ToolState::Running
            };
        }
        if let Some(message) = message {
            tool.output = message.into();
            tool.bound_output();
        }
        events.push(Event::Tool(tool.clone()));
    }
    /// A heartbeat is the tool call itself still running, never a step: it
    /// must not reach routing, or it becomes a never-ending child step.
    fn claude_heartbeat(&mut self, value: &Value, events: &mut Vec<Event>) {
        let Some((parent, elapsed)) = crate::tool::tasks::heartbeat_progress(value) else {
            return;
        };
        let Some(tool) = self.tools.get_mut(parent) else {
            return;
        };
        if tool.state == ToolState::Running && tool.live_elapsed_secs != Some(elapsed) {
            tool.live_elapsed_secs = Some(elapsed);
            events.push(Event::Tool(tool.clone()));
        }
    }
    fn claude_activity(&mut self, value: &Value, events: &mut Vec<Event>) -> bool {
        if crate::tool::tasks::is_heartbeat_frame(value) {
            self.claude_heartbeat(value, events);
            return true;
        }
        self.claude_background_list(value);
        if value["type"] == "system" && value["subtype"] == "init" {
            self.activities.turn += 1;
        }
        let parent = text(&value["parent_tool_use_id"]);
        if let Some(parent) = parent {
            let hint = text(&value["agent_id"]);
            match value["type"].as_str() {
                Some("assistant") => {
                    let model = text(&value["message"]["model"]);
                    let message_id = text(&value["message"]["id"]);
                    for (index, block) in value["message"]["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        match block["type"].as_str() {
                            Some("tool_use") => {
                                let Some(id) = text(&block["id"]) else {
                                    continue;
                                };
                                let mut child =
                                    ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"));
                                child.input = block["input"].to_string();
                                let target = self.claude_route(parent, hint);
                                self.agent_step(
                                    &target,
                                    AgentStep::from_tool_call(&child),
                                    model,
                                    events,
                                );
                            }
                            Some("text" | "thinking") => {
                                let Some(message_id) = message_id else {
                                    continue;
                                };
                                let thinking = block["type"] == "thinking";
                                let narration = block[if thinking { "thinking" } else { "text" }]
                                    .as_str()
                                    .unwrap_or("");
                                if narration.is_empty() {
                                    continue;
                                }
                                let target = self.claude_route(parent, hint);
                                self.agent_step(
                                    &target,
                                    AgentStep {
                                        id: format!("{message_id}:{index}"),
                                        kind: if thinking { "reasoning" } else { "message" }.into(),
                                        text: narration.into(),
                                        state: None,
                                        detail: String::new(),
                                        ..AgentStep::default()
                                    },
                                    model,
                                    events,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Some("user") => {
                    for block in value["message"]["content"].as_array().into_iter().flatten() {
                        if block["type"] != "tool_result" {
                            continue;
                        }
                        let Some(id) = text(&block["tool_use_id"]) else {
                            continue;
                        };
                        self.claude_child_result(parent, hint, id, block, events);
                    }
                }
                Some("tool_progress") => self.claude_progress(value, parent, events),
                _ => {}
            }
            // Every child step and result above is attached somewhere, at worst
            // to a visible "Background agent" block. What remains is consumed on
            // purpose: nested stream deltas (the completed assistant snapshot
            // supplies stable ids and avoids duplicate narration), the child's
            // prompt echo, and child rate-limit or status frames.
            return true;
        }
        if value["type"] == "system"
            && matches!(
                value["subtype"].as_str(),
                Some("task_started" | "task_progress" | "task_updated" | "task_notification")
            )
            && value["ambient"] != true
        {
            let subtype = value["subtype"].as_str().unwrap_or("");
            let patch = if subtype == "task_updated" {
                &value["patch"]
            } else {
                value
            };
            let task_id = text(&value["task_id"]);
            let tool_use_id = text(&value["tool_use_id"]);
            if subtype == "task_started" {
                self.claude_task_started(value, tool_use_id, task_id, events);
            }
            let owner = self
                .activities
                .registry
                .target_for_task(tool_use_id, task_id)
                .filter(|id| self.tools.contains_key(id))
                .or_else(|| tool_use_id.map(str::to_owned))
                .or_else(|| task_id.and_then(|id| self.activities.owners.get(id).cloned()));
            if let Some(id) = task_id {
                if patch["is_backgrounded"] == true {
                    self.activities.claude_background.insert(
                        id.into(),
                        text(&patch["description"])
                            .unwrap_or("Background task")
                            .into(),
                    );
                } else if patch["is_backgrounded"] == false {
                    self.activities.claude_background.remove(id);
                }
                if subtype == "task_started" {
                    self.activities.claude_pending.insert(id.into());
                }
                let card = owner
                    .as_deref()
                    .filter(|owner| self.tools.get(*owner).is_some_and(|t| t.panel.is_some()));
                self.track_claude_task(id, subtype, value, patch, card);
                if subtype == "task_notification" {
                    let background = self.activities.claude_background.remove(id).is_some();
                    self.activities.claude_pending.remove(id);
                    if background && self.claude_result_seen {
                        // Claude may wake itself after a background result. The
                        // desktop adapter leaves a 15s window for that follow-up.
                        self.claude_resume_at = Some(Instant::now() + Duration::from_secs(15));
                        if let Some(summary) =
                            text(&value["summary"]).or_else(|| text(&value["description"]))
                        {
                            self.activities.wake_summaries.push_back(summary.to_owned());
                        }
                    }
                }
                let terminal = subtype == "task_notification"
                    || patch["status"]
                        .as_str()
                        .and_then(state)
                        .is_some_and(|status| status != ToolState::Running);
                if terminal
                    && owner
                        .as_deref()
                        .is_some_and(|owner| self.activities.registry.is_resume(owner))
                {
                    self.activities.registry.note_finished(id);
                }
            }
            if let Some(tool) = owner.as_deref().and_then(|owner| self.tools.get_mut(owner)) {
                if tool.agent_mut().is_some() {
                    if let Some(id) = task_id {
                        if let Some(run) = tool.agent_mut() {
                            if !run.ids.iter().any(|old| old == id) {
                                run.ids.push(id.into());
                            }
                        }
                        self.activities.owners.insert(id.into(), tool.id.clone());
                    }
                    if patch["is_backgrounded"] == true {
                        self.activities.background_calls.insert(tool.id.clone());
                    } else if patch["is_backgrounded"] == false || subtype == "task_notification" {
                        self.activities.background_calls.remove(&tool.id);
                    }
                    if let Some(status) = patch["status"].as_str().and_then(state) {
                        tool.state = status;
                    } else if subtype == "task_started" {
                        tool.state = ToolState::Running;
                    }
                    if let Some(run) = tool.agent_mut() {
                        if subtype == "task_progress" {
                            // Progress is what the agent is doing now, never its title.
                            if let Some(description) = text(&value["description"]) {
                                run.activity = Some(description.into());
                            }
                        } else if run.steps.is_empty() {
                            // Match React: keep the run's name once its trail starts.
                            if let Some(description) = text(&patch["description"]) {
                                run.brief = description.into();
                            }
                        }
                        if let Some(summary) =
                            text(&value["summary"]).filter(|_| subtype == "task_progress")
                        {
                            if let Some(id) = task_id {
                                run.upsert(AgentStep {
                                    id: format!("task:{id}"),
                                    kind: "message".into(),
                                    text: summary.into(),
                                    state: None,
                                    detail: String::new(),
                                    ..AgentStep::default()
                                });
                            }
                        }
                    }
                    if subtype == "task_notification" {
                        if let Some(summary) = text(&value["summary"]) {
                            tool.output = summary.into();
                            tool.bound_output();
                        }
                    } else if let Some(error) = text(&patch["error"]) {
                        tool.output = error.into();
                        tool.bound_output();
                    }
                    events.push(Event::Tool(tool.clone()));
                }
            }
            return true;
        }
        false
    }
    /// A task_started names the call behind an agent task: an Agent dispatch
    /// learns its agent id; a SendMessage becomes that agent's resumption.
    fn claude_task_started(
        &mut self,
        value: &Value,
        tool_use_id: Option<&str>,
        task_id: Option<&str>,
        events: &mut Vec<Event>,
    ) {
        let (Some(call), Some(task)) = (tool_use_id, task_id) else {
            return;
        };
        let agent_task = is_agent_task(value["task_type"].as_str());
        let Some(name) = self.tools.get(call).map(|tool| tool.name.clone()) else {
            return;
        };
        if name == "SendMessage" {
            if agent_task || self.activities.registry.dispatch_of(task).is_some() {
                self.claude_resume(call, task, events);
            }
        } else if agent_task && !self.activities.registry.is_resume(call) {
            let Some(tool) = self.tools.get_mut(call) else {
                return;
            };
            if let Some(run) = tool.agent_mut() {
                run.agent_id.get_or_insert_with(|| task.into());
                let name = run.display_name();
                self.activities
                    .registry
                    .note_dispatch(call, Some(task), &name);
            }
        }
    }
    /// A main-thread Agent call: remember it so later turns can place its agent.
    pub(super) fn claude_note_dispatch(&mut self, id: &str) {
        if self.activities.registry.is_resume(id) {
            return;
        }
        let Some(tool) = self.tools.get(id).filter(|tool| tool.name != "SendMessage") else {
            return;
        };
        // Read-only: caching a panel here would freeze a still-streaming input.
        if let Some(Panel::Agent(run)) = tool.panel() {
            let (agent, name) = (run.agent_id.clone(), run.display_name());
            self.activities
                .registry
                .note_dispatch(id, agent.as_deref(), &name);
        }
    }
    /// A main-thread tool_result for an Agent or SendMessage call. Returns true
    /// when the result is a resumption receipt rather than a report.
    pub(super) fn claude_tool_result(
        &mut self,
        block: &Value,
        frame: &Value,
        events: &mut Vec<Event>,
    ) -> bool {
        let Some(id) = text(&block["tool_use_id"]) else {
            return false;
        };
        let Some(tool) = self.tools.get_mut(id) else {
            return false;
        };
        let content = text_content(&block["content"]);
        if tool.name == "SendMessage" {
            let resumed = text(&frame["tool_use_result"]["resumedAgentId"])
                .map(str::to_owned)
                .or_else(|| {
                    serde_json::from_str::<Value>(&content)
                        .ok()
                        .and_then(|receipt| text(&receipt["resumedAgentId"]).map(str::to_owned))
                });
            if let Some(agent) = resumed {
                self.claude_resume(id, &agent, events);
            }
            return self.activities.registry.is_resume(id) && block["is_error"] != true;
        }
        if self.activities.registry.is_resume(id) || !matches!(tool.panel(), Some(Panel::Agent(_)))
        {
            return false;
        }
        let Some(run) = tool.agent_mut() else {
            return false;
        };
        let agent = crate::tool::handback::parse_handback(&content)
            .agent_id
            .or_else(|| text(&frame["tool_use_result"]["agentId"]).map(str::to_owned));
        if let Some(agent) = agent {
            run.agent_id.get_or_insert_with(|| agent.clone());
            let name = run.display_name();
            self.activities
                .registry
                .note_dispatch(id, Some(&agent), &name);
        }
        false
    }
    /// The SendMessage block `send` now shows `agent`'s resumed work.
    fn claude_resume(&mut self, send: &str, agent: &str, events: &mut Vec<Event>) {
        let registry = &mut self.activities.registry;
        let known = registry.resumed_agent(send) == Some(agent);
        registry.note_resume(send, agent);
        let origin = registry
            .dispatch_of(agent)
            .map(|(id, name)| (id.to_owned(), name.to_owned()));
        let Some(tool) = self.tools.get_mut(send) else {
            return;
        };
        if known && matches!(&tool.panel, Some(Panel::Agent(run)) if run.agent_id.is_some()) {
            return;
        }
        let input: Value = serde_json::from_str(&tool.input).unwrap_or_default();
        let name = origin
            .as_ref()
            .map(|(_, name)| name.as_str())
            .filter(|name| !name.is_empty())
            .or_else(|| text(&input["to"]))
            .unwrap_or(agent);
        tool.panel = Some(Panel::Agent(AgentRun {
            name: Some(format!("Resumed · {name}")),
            brief: text(&input["message"]).unwrap_or_default().into(),
            agent_id: Some(agent.into()),
            resume_of: origin.map(|(id, _)| id),
            ..AgentRun::default()
        }));
        // The receipt is not the agent's report; the notification brings that.
        tool.state = ToolState::Running;
        tool.output.clear();
        events.push(Event::Tool(tool.clone()));
    }
    /// Where a child frame of `parent` goes, in the documented order:
    /// 1. `parent` is a live agent call in this turn;
    /// 2. the active resumption of the agent `parent` dispatched;
    /// 3. a per-turn continuation block for a known but finished dispatch;
    /// 4. a visible "Background agent" block. Never nowhere.
    fn claude_route(&mut self, parent: &str, agent_hint: Option<&str>) -> String {
        if let Some(target) = self.claude_placed(parent, agent_hint) {
            return target;
        }
        let scope = self.synthetic_scope();
        let registry = &mut self.activities.registry;
        if let Some(name) = registry.dispatch_name(parent).map(str::to_owned) {
            let id = format!("{CONTINUED_PREFIX}{parent}:{scope}");
            if !self.tools.contains_key(&id) {
                let agent = registry
                    .dispatch_agent(parent)
                    .or(agent_hint)
                    .map(str::to_owned);
                if let Some(agent) = &agent {
                    registry.note_resume(&id, agent);
                }
                let mut tool = ToolCall::new(&id, "Agent");
                tool.panel = Some(Panel::Agent(AgentRun {
                    name: Some(format!(
                        "Continued · {}",
                        if name.is_empty() { "agent" } else { &name }
                    )),
                    agent_id: agent,
                    resume_of: Some(parent.into()),
                    continuation: true,
                    ..AgentRun::default()
                }));
                // Like the fallback block: its steps carry their own state, and
                // a run that never reports again must not be settled as failed.
                tool.state = ToolState::Completed;
                self.tools.insert(id.clone(), tool);
            }
            return id;
        }
        let id = format!("{FALLBACK_PREFIX}{parent}:{scope}");
        self.tools.entry(id.clone()).or_insert_with(|| {
            let mut tool = ToolCall::new(&id, "Agent");
            tool.panel = Some(Panel::Agent(AgentRun {
                name: Some("Background agent".into()),
                agent_id: agent_hint.map(str::to_owned),
                ..AgentRun::default()
            }));
            // A container of steps that each carry their own state; nothing
            // reports when unplaced work ends, so it never spins forever.
            tool.state = ToolState::Completed;
            tool
        });
        id
    }
    /// `<operation>-<turn>`: one synthetic block per parent and turn, unique
    /// across the session's processes and warm reuses.
    fn synthetic_scope(&mut self) -> String {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let operation = self.activities.scope.get_or_insert_with(|| {
            format!(
                "{:x}{:x}",
                crate::time::now_ms(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )
        });
        format!("{operation}-{}", self.activities.turn)
    }
    /// Rules 1 and 2 only: an existing block, never a new one.
    fn claude_placed(&mut self, parent: &str, agent_hint: Option<&str>) -> Option<String> {
        if let Some(tool) = self.tools.get(parent) {
            if tool.state == ToolState::Running && matches!(tool.panel(), Some(Panel::Agent(_))) {
                return Some(parent.into());
            }
        }
        // A continuation belongs to one (parent, turn); a later turn gets its own.
        let this_turn = format!(":{}", self.synthetic_scope());
        let registry = &self.activities.registry;
        registry
            .target_for_parent(parent)
            .or_else(|| agent_hint.and_then(|agent| registry.target_for_agent(agent)))
            .filter(|target| !target.starts_with(CONTINUED_PREFIX) || target.ends_with(&this_turn))
            .filter(|target| self.tools.contains_key(target))
    }
    /// A child tool_result completes the step it answers, wherever that step lives.
    fn claude_child_result(
        &mut self,
        parent: &str,
        hint: Option<&str>,
        id: &str,
        block: &Value,
        events: &mut Vec<Event>,
    ) {
        let has_step = |tool: &ToolCall| matches!(&tool.panel, Some(Panel::Agent(run)) if run.steps.iter().any(|step| step.id == id));
        let owner = self
            .claude_placed(parent, hint)
            .filter(|target| self.tools.get(target).is_some_and(has_step))
            .or_else(|| {
                self.tools
                    .values()
                    .find(|tool| has_step(tool))
                    .map(|tool| tool.id.clone())
            });
        let state = Some(if block["is_error"] == true {
            ToolState::Failed
        } else {
            ToolState::Completed
        });
        let mut detail = text_content(&block["content"]);
        crate::tool::tasks::bound(&mut detail, 16 * 1024);
        if let Some(tool) = owner.as_deref().and_then(|owner| self.tools.get_mut(owner)) {
            if let Some(step) = tool
                .agent_mut()
                .and_then(|run| run.steps.iter_mut().find(|step| step.id == id))
            {
                step.state = state;
                step.detail = detail;
                events.push(Event::Tool(tool.clone()));
            }
            return;
        }
        // A result for a call this session never saw still shows up.
        let target = self.claude_route(parent, hint);
        self.agent_step(
            &target,
            AgentStep {
                id: id.into(),
                kind: "tool".into(),
                text: "Tool result".into(),
                state,
                detail,
                ..AgentStep::default()
            },
            None,
            events,
        );
    }
    /// Mirrors claude.ts's backgroundTasks: a task runs from task_started
    /// until a terminal update, its notification, or a background_tasks_changed
    /// list that no longer has it.
    fn track_claude_task(
        &mut self,
        id: &str,
        subtype: &str,
        value: &Value,
        patch: &Value,
        block: Option<&str>,
    ) {
        let steps = block
            .and_then(|block| self.tools.get(block))
            .and_then(|tool| match &tool.panel {
                Some(Panel::Agent(run)) => Some(run.tool_step_count() as u32),
                _ => None,
            })
            .unwrap_or(0);
        let tasks = &mut self.activities.claude_tasks;
        match subtype {
            "task_started" => {
                if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
                    if block.is_some() {
                        task.tool_id = block.map(str::to_owned);
                        task.steps = steps;
                    }
                    return;
                }
                let source = text(&value["tool_use_id"]).and_then(|id| self.tools.get(id));
                let command = source
                    .filter(|tool| tool.name == "Bash")
                    .and_then(|tool| serde_json::from_str::<Value>(&tool.input).ok())
                    .and_then(|input| text(&input["command"]).map(str::to_owned));
                tasks.push(BackgroundTask {
                    id: id.into(),
                    description: text(&value["description"])
                        .unwrap_or(if command.is_some() {
                            "Background command"
                        } else {
                            "Background task"
                        })
                        .into(),
                    agent: is_agent_task(value["task_type"].as_str()),
                    command,
                    started_at: crate::time::now_ms(),
                    tool_id: block.map(str::to_owned),
                    latest: None,
                    steps,
                });
            }
            "task_progress" => {
                if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
                    if let Some(description) = text(&value["description"]) {
                        task.latest = Some(description.into());
                    }
                    if block.is_some() {
                        task.tool_id = block.map(str::to_owned);
                        task.steps = steps;
                    }
                }
            }
            "task_updated" => {
                if let Some(description) = text(&patch["description"]) {
                    if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
                        task.description = description.into();
                    }
                }
                let status = patch["status"].as_str().unwrap_or("").to_lowercase();
                if matches!(
                    status.as_str(),
                    "completed" | "failed" | "killed" | "stopped"
                ) {
                    tasks.retain(|task| task.id != id);
                }
            }
            "task_notification" => tasks.retain(|task| task.id != id),
            _ => {}
        }
    }
    /// Claude's full list of running tasks replaces the tracked one.
    pub(super) fn claude_background_list(&mut self, value: &Value) {
        if value["type"] != "system" || value["subtype"] != "background_tasks_changed" {
            return;
        }
        let rows: Vec<&Value> = value["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row["ambient"] != true)
            .collect();
        let tasks = &mut self.activities.claude_tasks;
        tasks.retain(|task| rows.iter().any(|row| row["task_id"] == task.id.as_str()));
        for row in rows {
            let Some(id) = text(&row["task_id"]) else {
                continue;
            };
            if !tasks.iter().any(|task| task.id == id) {
                tasks.push(BackgroundTask {
                    id: id.into(),
                    description: text(&row["description"])
                        .unwrap_or("Background task")
                        .into(),
                    agent: is_agent_task(row["task_type"].as_str()),
                    command: None,
                    started_at: crate::time::now_ms(),
                    tool_id: None,
                    latest: None,
                    steps: 0,
                });
            }
        }
    }
    /// Once Claude has yielded, tell the UI which tasks the turn still waits
    /// on, and clear the list when Claude picks the thread back up.
    pub(super) fn sync_background(&mut self, events: &mut Vec<Event>) {
        let waiting = if self.claude_result_seen && !self.done {
            self.activities.claude_tasks.clone()
        } else {
            Vec::new()
        };
        if waiting != self.activities.published {
            self.activities.published = waiting.clone();
            events.push(Event::Background(waiting));
        }
    }
    fn claude_progress(&mut self, value: &Value, parent: &str, events: &mut Vec<Event>) {
        let (Some(id), Some(name)) = (text(&value["tool_use_id"]), text(&value["tool_name"]))
        else {
            return;
        };
        // Progress is a child's step, never the parent Agent's final report.
        let target = self.claude_route(parent, text(&value["agent_id"]));
        self.agent_step(
            &target,
            AgentStep {
                id: id.into(),
                kind: "tool".into(),
                text: name.into(),
                state: Some(ToolState::Running),
                detail: String::new(),
                tool_kind: Some(name.into()),
                ..AgentStep::default()
            },
            None,
            events,
        );
    }
}
fn is_agent_task(task_type: Option<&str>) -> bool {
    matches!(
        task_type.map(str::to_lowercase).as_deref(),
        Some("local_agent" | "remote_agent")
    )
}
fn text_content(value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn protocol(kind: Kind) -> Protocol {
        Protocol::new(Request {
            kind,
            binary: "unused".into(),
            cwd: "/tmp".into(),
            prompt: "Hello".into(),
            provider_id: None,
            model: None,
            settings: Default::default(),
        })
    }
    fn tool(events: &[Event]) -> &ToolCall {
        events
            .iter()
            .rev()
            .find_map(|e| {
                if let Event::Tool(tool) = e {
                    Some(tool)
                } else {
                    None
                }
            })
            .unwrap()
    }
    fn spawn(p: &mut Protocol) {
        p.frame(json!({"method":"item/completed","params":{"threadId":"main","item":{"id":"spawn-id","type":"collabAgentToolCall","tool":"spawnAgent","status":"completed","receiverThreadIds":["child"],"prompt":"Audit correctness","model":"child-model"}}}), &mut Vec::new()).unwrap();
    }
    #[test]
    fn codex_path_titles_match_tauri_for_both_item_formats() {
        for kind in ["collabAgentToolCall", "subAgentActivity"] {
            for (key, path, expected) in [
                ("agentPath", "/root/explore-auth", "Explore Auth subagent"),
                (
                    "agent_path",
                    "\\root\\pairing__dialog-parity\\",
                    "Pairing Dialog Parity subagent",
                ),
            ] {
                let mut p = protocol(Kind::Codex);
                let mut item = json!({
                    "id":"spawn-id", "type":kind, "tool":"spawnAgent", "kind":"started",
                    "receiverThreadIds":["child"], "agentThreadId":"child",
                    "prompt":"Full instructions for the delegated task.", "model":"child-model"
                });
                item[key] = json!(path);
                let events = p
                    .frame(
                        json!({"method":"item/completed","params":{"threadId":"main","item":item}}),
                        &mut Vec::new(),
                    )
                    .unwrap();
                let Some(Panel::Agent(run)) = tool(&events).panel() else {
                    panic!()
                };
                assert_eq!(run.display_name(), expected);
                assert_eq!(run.display_brief(), expected);
                assert_eq!(run.model.as_deref(), Some("child-model"));
                assert_eq!(tool(&events).state, ToolState::Running);

                // Sparse completion and child work must retain the title.
                let events = p
                    .frame(
                        json!({"method":"item/completed","params":{"threadId":"main","item":{
                            "id":"spawn-id", "type":kind, "tool":"spawnAgent", "kind":"interacted",
                            "receiverThreadIds":["child"], "agentThreadId":"child"
                        }}}),
                        &mut Vec::new(),
                    )
                    .unwrap();
                let Some(Panel::Agent(run)) = tool(&events).panel() else {
                    panic!()
                };
                assert_eq!(run.display_name(), expected);
                let events = p.frame(
                    json!({"method":"item/completed","params":{"threadId":"child","item":{
                        "id":"read", "type":"commandExecution", "command":"cat source.rs", "status":"completed"
                    }}}), &mut Vec::new(),
                ).unwrap();
                let Some(Panel::Agent(run)) = tool(&events).panel() else {
                    panic!()
                };
                assert_eq!(run.display_name(), expected);
                assert_eq!(run.steps.len(), 1);
                // Desktop persistence carries the same label back to native.
                let restored = crate::tool::tasks::from_desktop(&json!({
                    "role":"tool", "agentRun":crate::tool::tasks::desktop_agent(&run)
                }))
                .unwrap();
                assert_eq!(restored, Panel::Agent(run));
            }
        }
    }
    #[test]
    fn codex_duplicate_path_items_keep_the_original_task_title() {
        let mut p = protocol(Kind::Codex);
        spawn(&mut p);
        let events = p
            .frame(
                json!({"method":"item/completed","params":{"threadId":"main","item":{
                    "id":"activity-id", "type":"subAgentActivity", "kind":"started",
                    "agentPath":"/root/explore-auth", "agentThreadId":"child"
                }}}),
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(tool(&events).id, "spawn-id");
        let Some(Panel::Agent(run)) = tool(&events).panel() else {
            panic!()
        };
        assert_eq!(run.display_name(), "Audit correctness");
        assert!(!p.tools.contains_key("activity-id"));
    }
    #[test]
    fn codex_completed_spawn_wait_and_child_lifecycle_keep_actual_owner() {
        let mut p = protocol(Kind::Codex);
        spawn(&mut p);
        assert_eq!(p.tools["spawn-id"].state, ToolState::Running);
        p.active_turn = Some("parent-turn".into());
        let child = p.frame(json!({"method":"item/completed","params":{"threadId":"child","item":{"id":"child-answer","type":"agentMessage","text":"Only the child said this"}}}), &mut Vec::new()).unwrap();
        assert_eq!(child.len(), 1);
        assert_eq!(tool(&child).id, "spawn-id");
        let Some(Panel::Agent(run)) = tool(&child).panel() else {
            panic!()
        };
        assert_eq!(run.ids, vec!["child"]);
        assert_eq!(run.display_name(), "Audit correctness");
        assert_eq!(run.model.as_deref(), Some("child-model"));
        assert_eq!(run.steps[0].id, "child-answer");
        assert_eq!(run.steps[0].text, "Only the child said this");
        let completed = p.frame(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"child-turn","status":"completed"}}}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&completed).state, ToolState::Completed);
        assert_eq!(p.active_turn.as_deref(), Some("parent-turn"));
        assert!(!p.done);
        let settled = p.frame(json!({"method":"item/completed","params":{"threadId":"main","item":{"id":"wait-id","type":"collabAgentToolCall","tool":"wait","agentsStates":{"child":{"status":"failed","message":"Provider failure"}}}}}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&settled).id, "spawn-id");
        assert_eq!(tool(&settled).state, ToolState::Failed);
        assert_eq!(tool(&settled).output, "Provider failure");
        assert!(!p.tools.contains_key("wait-id"));
    }
    #[test]
    fn codex_early_child_snapshots_replay_and_do_not_finish_main_turn() {
        let mut p = protocol(Kind::Codex);
        p.activities.main_thread = Some("main".into());
        let events = p.frame(json!({"method":"item/completed","params":{"threadId":"child","item":{"id":"early","type":"agentMessage","text":"Early snapshot"}}}), &mut Vec::new()).unwrap();
        assert!(events.is_empty());
        spawn(&mut p);
        let Some(Panel::Agent(run)) = p.tools["spawn-id"].panel() else {
            panic!()
        };
        assert_eq!(run.steps[0].id, "early");
        assert!(p.activities.pending.is_empty());
        let approval = p.frame(json!({"id":88,"method":"item/commandExecution/requestApproval","params":{"threadId":"child","command":"write"}}), &mut Vec::new()).unwrap();
        assert!(approval
            .iter()
            .any(|event| matches!(event, Event::Approval(_))));
    }
    #[test]
    fn codex_plan_snapshots_update_one_real_turn_and_clear_explicitly() {
        let mut p = protocol(Kind::Codex);
        let a = p.frame(json!({"method":"turn/plan/updated","params":{"turnId":"real-turn","explanation":"Inspect first","plan":[{"step":"Inspect","status":"in_progress"}]}}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&a).id, "plan:real-turn");
        let b = p
            .frame(
                json!({"method":"turn/plan/updated","params":{"turnId":"real-turn","plan":[]}}),
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(tool(&a).id, tool(&b).id);
        let Some(Panel::Tasks(list)) = tool(&b).panel() else {
            panic!()
        };
        assert!(list.items.is_empty());
    }
    #[test]
    fn claude_child_work_never_leaks_into_parent_answer_or_metrics() {
        let mut p = protocol(Kind::Claude);
        p.frame(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"agent-call","name":"Agent","input":{"description":"Correctness review","model":"requested"}}]}}), &mut Vec::new()).unwrap();
        let child = json!({"type":"assistant","parent_tool_use_id":"agent-call","message":{"id":"child-message","model":"actual-child-model","usage":{"input_tokens":999999},"content":[{"type":"text","text":"Child narration"},{"type":"tool_use","id":"child-tool","name":"Read","input":{"file_path":"source.rs"}}]}});
        let events = p.frame(child.clone(), &mut Vec::new()).unwrap();
        assert!(events.iter().all(|event| matches!(event, Event::Tool(_))));
        let Some(Panel::Agent(run)) = tool(&events).panel() else {
            panic!()
        };
        assert_eq!(run.model.as_deref(), Some("actual-child-model"));
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[0].id, "child-message:0");
        p.frame(json!({"type":"user","parent_tool_use_id":"agent-call","message":{"content":[{"type":"tool_result","tool_use_id":"child-tool","content":"Read result"}]}}), &mut Vec::new()).unwrap();
        p.frame(child, &mut Vec::new()).unwrap();
        let Some(Panel::Agent(run)) = p.tools["agent-call"].panel() else {
            panic!()
        };
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[1].state, Some(ToolState::Completed));
        assert_eq!(run.steps[1].detail, "Read result");
        assert!(p.tools["agent-call"].output.is_empty());
    }
    #[test]
    fn claude_task_descriptions_keep_the_name_after_child_work_starts() {
        let mut p = protocol(Kind::Claude);
        p.frame(
            json!({"type":"assistant","message":{"content":[{
                "type":"tool_use","id":"agent-call","name":"Agent",
                "input":{"description":"Review implementation"}
            }]}}),
            &mut Vec::new(),
        )
        .unwrap();
        // Task metadata can refine the name before the child starts working.
        p.frame(
            json!({"type":"system","subtype":"task_started","task_id":"review-task",
                "tool_use_id":"agent-call","description":"Correctness review"}),
            &mut Vec::new(),
        )
        .unwrap();
        p.frame(
            json!({"type":"assistant","parent_tool_use_id":"agent-call",
            "message":{"id":"child-message","content":[{
                "type":"tool_use","id":"child-read","name":"Read",
                "input":{"file_path":"source.rs"}
            }]}}),
            &mut Vec::new(),
        )
        .unwrap();

        for update in [
            json!({"type":"system","subtype":"task_progress","task_id":"review-task",
                "description":"Running cd /repo && cargo check",
                "summary":"Inspecting files"}),
            json!({"type":"system","subtype":"task_updated","task_id":"review-task",
                "patch":{"description":"Running Read phases."}}),
            json!({"type":"system","subtype":"task_notification","task_id":"review-task",
                "description":"Finished reviewing source.rs","status":"completed",
                "summary":"No regressions found."}),
        ] {
            let events = p.frame(update, &mut Vec::new()).unwrap();
            let call = tool(&events);
            let Some(Panel::Agent(run)) = call.panel() else {
                panic!()
            };
            assert_eq!(run.display_name(), "Correctness review");
            assert_eq!(run.display_brief(), "Correctness review");
            assert_eq!(run.status_line(&call.state), "1 step");
            assert!(run.steps.iter().any(|step| step.text == "Inspecting files"));
        }
        assert_eq!(p.tools["agent-call"].state, ToolState::Completed);
        assert_eq!(p.tools["agent-call"].output, "No regressions found.");
    }
    #[test]
    fn claude_task_ids_progress_and_reports_come_from_task_events() {
        let mut p = protocol(Kind::Claude);
        p.tools
            .insert("agent".into(), ToolCall::new("agent", "Agent"));
        p.frame(json!({"type":"system","subtype":"task_started","task_id":"actual-task","tool_use_id":"agent","description":"Review","is_backgrounded":true}), &mut Vec::new()).unwrap();
        p.frame(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent","content":"Background launch receipt"}]}}), &mut Vec::new()).unwrap();
        assert_eq!(p.tools["agent"].state, ToolState::Running);
        assert!(p.tools["agent"].output.is_empty());
        let progress = p.frame(json!({"type":"system","subtype":"task_progress","task_id":"actual-task","summary":"Inspecting files"}), &mut Vec::new()).unwrap();
        let Some(Panel::Agent(run)) = tool(&progress).panel() else {
            panic!()
        };
        assert_eq!(run.ids, vec!["actual-task"]);
        assert_eq!(run.steps[0].text, "Inspecting files");
        assert!(tool(&progress).output.is_empty());
        let done = p.frame(json!({"type":"system","subtype":"task_notification","task_id":"actual-task","status":"failed","summary":"Provider reported failure"}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&done).state, ToolState::Failed);
        assert_eq!(tool(&done).output, "Provider reported failure");
        // Work this session cannot place is shown, never merged into the run.
        let orphan = p.frame(json!({"type":"assistant","parent_tool_use_id":"unknown","message":{"id":"x","content":[{"type":"text","text":"Unattributed child"}]}}), &mut Vec::new()).unwrap();
        assert!(tool(&orphan).id.starts_with("background:unknown:"));
        let Some(Panel::Agent(run)) = p.tools["agent"].panel() else {
            panic!()
        };
        assert!(!run
            .steps
            .iter()
            .any(|step| step.text == "Unattributed child"));
    }
    #[test]
    fn claude_background_tasks_show_once_the_answer_yields_and_clear_as_they_finish() {
        let mut p = protocol(Kind::Claude);
        let tick = |p: &mut Protocol| {
            p.lifecycle_tick(Instant::now(), &mut Vec::new())
                .unwrap()
                .into_iter()
                .find_map(|event| match event {
                    Event::Background(tasks) => Some(tasks),
                    _ => None,
                })
        };
        p.frame(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"dev-call","name":"Bash","input":{"command":"npm run dev","run_in_background":true}}]}}), &mut Vec::new()).unwrap();
        p.frame(json!({"type":"system","subtype":"task_started","task_id":"dev","tool_use_id":"dev-call","description":"Start dev server","task_type":"local_bash","is_backgrounded":true}), &mut Vec::new()).unwrap();
        p.frame(json!({"type":"system","subtype":"task_started","task_id":"review","description":"Review","task_type":"local_agent"}), &mut Vec::new()).unwrap();
        // Work the agent is still doing in the foreground is not background.
        assert_eq!(tick(&mut p), None);

        p.frame(
            json!({"type":"result","result":"Started it."}),
            &mut Vec::new(),
        )
        .unwrap();
        let tasks = tick(&mut p).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].description, "Start dev server");
        assert_eq!(tasks[0].command.as_deref(), Some("npm run dev"));
        assert!(!tasks[0].agent && tasks[1].agent);
        assert_eq!(tick(&mut p), None, "an unchanged list is not sent again");

        p.frame(json!({"type":"system","subtype":"task_updated","task_id":"review","patch":{"status":"completed"}}), &mut Vec::new()).unwrap();
        let tasks = tick(&mut p).unwrap();
        assert_eq!(
            tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["dev"]
        );

        p.frame(
            json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(tick(&mut p), Some(Vec::new()));
    }
}

#[cfg(test)]
#[path = "tasks/claude_task_tests.rs"]
mod claude_task_tests;
#[cfg(test)]
#[path = "tasks/heartbeat_tests.rs"]
mod heartbeat_tests;
#[cfg(test)]
#[path = "tasks/resume_tests.rs"]
mod resume_tests;
