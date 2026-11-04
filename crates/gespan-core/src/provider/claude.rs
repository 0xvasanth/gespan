//! Claude Code's stream-json: launch arguments, per-turn state and the
//! frames only Claude sends.
use super::*;

pub(super) mod read_only;

#[derive(Default)]
pub(super) struct State {
    pub(super) prompt_sent: bool,
    pub(super) result_seen: bool,
    pub(super) usage_limit: Option<crate::usage::Limit>,
    pub(super) resume_at: Option<Instant>,
    pub(super) message: usize,
    /// Streaming tool id and its partial input JSON, by content block index.
    pub(super) tools: HashMap<u64, (String, String)>,
    pub(super) mcp: lifecycle::ClaudeMcp,
}

pub(super) fn configure(
    command: &mut Command,
    request: &Request,
    claude_new_session: Option<&str>,
) -> Result<(), String> {
    command.args([
        "--verbose",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
    ]);
    let settings = &request.settings;
    let mode = if settings.plan {
        "plan"
    } else {
        settings.access.claude()
    };
    command.args(["--permission-mode", mode]);
    if settings.text_only {
        command.args([
            "--tools",
            "",
            "--max-turns",
            "1",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
        ]);
    } else if settings.access == AccessMode::ReadOnly {
        // No Bash pattern here: a pattern approves any arguments. Each Bash
        // call comes to the approval path, which allows only `read_only`.
        command.args([
            "--tools",
            "Read,Glob,Grep,Bash",
            "--allowedTools",
            "Read,Glob,Grep",
        ]);
    }
    if settings.mount.as_ref().is_some_and(|mount| mount.exclusive) {
        command.arg("--strict-mcp-config");
    }
    if mode == "bypassPermissions" {
        command.arg("--allow-dangerously-skip-permissions");
    }
    let mut config = json!({});
    if settings.disable_claude_hooks || settings.text_only {
        config["disableAllHooks"] = true.into();
    }
    if settings.fast {
        config["fastMode"] = true.into();
    }
    if settings.thinking == Some(true) {
        config["alwaysThinkingEnabled"] = true.into();
    }
    if let Some(effort) = settings.effort.as_deref().filter(|e| *e != "ultrathink") {
        let effort = if effort == "ultracode" {
            config["ultracode"] = true.into();
            "xhigh"
        } else if effort == "xhigh"
            && request.model.as_deref().is_some_and(|m| {
                matches!(
                    m,
                    "claude-opus-4-6"
                        | "claude-sonnet-4-6"
                        | "claude-opus-4-5"
                        | "claude-haiku-4-5"
                        | "claude-opus-4-1"
                        | "claude-opus-4-0"
                        | "claude-sonnet-4-0"
                        | "claude-sonnet-4-5"
                )
            })
        {
            "max"
        } else if effort == "max" && request.model.as_deref() == Some("claude-sonnet-4-6") {
            "high"
        } else {
            effort
        };
        command.args(["--effort", effort]);
    }
    command.args(["--settings", &config.to_string()]);
    if !settings.text_only {
        command.arg("--setting-sources=user,project,local");
    }
    if let Some(id) = &request.provider_id {
        command.args(["--resume", id]);
    } else if let Some(id) = claude_new_session {
        command.args(["--session-id", id]);
    }
    if let Some(model) = &request.model {
        let model = if settings
            .model_settings
            .get("context")
            .is_some_and(|value| value == "1m")
            && !model.ends_with("[1m]")
        {
            format!("{model}[1m]")
        } else {
            model.clone()
        };
        command.args(["--model", &model]);
    }
    Ok(())
}

impl Protocol {
    /// Frames Claude answers before any shared handling sees them, and the
    /// wake bookkeeping a new message starts.
    pub(super) fn claude_screen(
        &mut self,
        v: &Value,
        stdin: &mut impl Write,
        activity_events: &mut Vec<Event>,
    ) -> Result<Option<Vec<Event>>, String> {
        if v["type"] == "control_cancel_request" {
            let id = v["request_id"].to_string();
            let mut events = Vec::new();
            if self.approvals.remove(&id).is_some() {
                events.push(Event::ApprovalResolved(id.clone()));
            }
            if self.questions.remove(&id).is_some() {
                if self.visible_question.as_deref() == Some(id.as_str()) {
                    self.visible_question = None;
                }
                events.push(Event::QuestionResolved(id));
                events.extend(self.show_next_question());
            }
            return Ok(Some(events));
        }
        if v["type"] == "control_response"
            || (v["type"] == "system"
                && matches!(v["subtype"].as_str(), Some("init" | "initialized")))
        {
            self.claude_prompt(stdin)?;
        }
        let starts_message = v["parent_tool_use_id"].is_null()
            && (v["type"] == "assistant"
                || (v["type"] == "stream_event" && v["event"]["type"] == "message_start")
                || (v["type"] == "system" && v["subtype"] == "init"));
        if starts_message && self.claude.result_seen {
            // Claude woke itself after a background task's notification:
            // lead its follow-up with that notification, so the answer is
            // not read as a reply to the user. Task 3 renders the marker.
            // A turn the user started gets no marker; the summary stays for
            // the result-origin fallback should Claude still wake itself.
            let mut woke = false;
            // Without the window the summary stays queued: the wake's result
            // then shows it, so it is never dropped unseen.
            if !std::mem::take(&mut self.activities.user_turn_pending)
                && self.claude.resume_at.is_some()
            {
                if let Some(summary) = self.activities.wake_summaries.pop_front() {
                    woke = true;
                    activity_events.push(Self::wake_marker(summary));
                }
            }
            self.activities.wake_handled = woke;
            self.activities.steered_in_turn = false;
            self.claude.result_seen = false;
            self.claude.resume_at = None;
            self.text_key = None;
        }
        Ok(None)
    }
    /// Compaction progress from `system` frames.
    pub(super) fn claude_compaction(&mut self, v: &Value, events: &mut Vec<Event>) {
        if v["type"] != "system" {
            return;
        }
        if v["status"] == "compacting" {
            self.compaction(None, ToolState::Running, events);
        } else if v["subtype"]
            .as_str()
            .is_some_and(|subtype| subtype.starts_with("compact"))
        {
            self.compaction(None, ToolState::Completed, events);
            if self.compact {
                self.compact_confirmed = true;
            }
        }
    }
    pub(super) fn claude_frame(
        &mut self,
        v: Value,
        stdin: &mut impl Write,
        mut events: Vec<Event>,
    ) -> Result<Vec<Event>, String> {
        if let Some(id) = v["session_id"].as_str() {
            events.push(Event::SessionId(id.into()));
        }
        if let Some(model) = v["model"].as_str().filter(|_| v["type"] == "system") {
            events.push(Event::Model(model.into()));
        }
        match v["type"].as_str().unwrap_or("") {
            "rate_limit_event" if v["parent_tool_use_id"].is_null() => {
                self.claude.usage_limit =
                    crate::usage::Limit::from_claude_info(&v["rate_limit_info"]);
                let windows = crate::usage::claude_live_windows(&v["rate_limit_info"]);
                if !windows.is_empty() {
                    events.push(Event::UsageWindows(windows));
                }
            }
            "control_response" if v["response"]["request_id"] == "native-init" => {
                self.claude_prompt(stdin)?;
            }
            "control_response" if v["response"]["request_id"] == lifecycle::CLAUDE_MCP => {
                self.claude_mcp_answered(&v["response"], stdin)?;
            }
            "stream_event" => {
                let e = &v["event"];
                if e["type"] == "message_start" {
                    self.claude.message += 1;
                    self.claude.tools.clear();
                }
                if e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use" {
                    let block = &e["content_block"];
                    let id = block["id"].as_str().unwrap_or("");
                    let tool = ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"));
                    self.claude
                        .tools
                        .insert(e["index"].as_u64().unwrap_or(0), (id.into(), String::new()));
                    self.tools.insert(id.into(), tool.clone());
                    events.push(Event::Tool(tool));
                }
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "input_json_delta" {
                    // Like the desktop adapter, keep the last input that
                    // parses; a half-streamed object is not shown.
                    if let Some((id, partial)) =
                        self.claude.tools.get_mut(&e["index"].as_u64().unwrap_or(0))
                    {
                        partial.push_str(e["delta"]["partial_json"].as_str().unwrap_or(""));
                        let parsed = serde_json::from_str::<Value>(partial)
                            .is_ok_and(|value| value.is_object());
                        if let Some(tool) = self.tools.get_mut(id.as_str()).filter(|_| parsed) {
                            tool.input = partial.clone();
                            events.push(Event::Tool(tool.clone()));
                        }
                    }
                }
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "thinking_delta" {
                    events.push(Event::Reasoning(
                        e["delta"]["thinking"].as_str().unwrap_or("").into(),
                    ));
                }
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                    self.text(
                        &self.claude.message.to_string(),
                        e["delta"]["text"].as_str().unwrap_or(""),
                        false,
                        &mut events,
                    );
                }
            }
            "assistant" => {
                if let Some(error) = v["error"].as_str() {
                    return Err(self.claude_error(error, &v));
                }
                if v["parent_tool_use_id"].is_null() {
                    if let Some(used) = crate::context::claude_used(&v["message"]["usage"]) {
                        events.push(Event::Context {
                            used: Some(used),
                            window: None,
                        });
                    }
                }
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        let id = block["id"].as_str().unwrap_or("");
                        let tool = self.tools.entry(id.into()).or_insert_with(|| {
                            ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"))
                        });
                        tool.input =
                            serde_json::to_string_pretty(&block["input"]).unwrap_or_default();
                        tool.refresh_agent_input();
                        events.push(Event::Tool(tool.clone()));
                        if v["parent_tool_use_id"].is_null() {
                            self.claude_note_dispatch(id);
                        }
                    }
                }
                let text = v["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<String>();
                self.text(&self.claude.message.to_string(), &text, true, &mut events);
            }
            "user" => {
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result" {
                        // A SendMessage that resumed an agent returns a receipt;
                        // its block shows the resumed work instead.
                        if self.claude_tool_result(block, &v, &mut events) {
                            continue;
                        }
                        // A background Agent's launch receipt is not its final report.
                        if block["is_error"] != true
                            && block["tool_use_id"]
                                .as_str()
                                .is_some_and(|id| self.activities.background_calls.contains(id))
                        {
                            continue;
                        }
                        if let Some(tool) = block["tool_use_id"]
                            .as_str()
                            .and_then(|id| self.tools.get_mut(id))
                        {
                            tool.output = block["content"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| {
                                    block["content"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter_map(|b| b["text"].as_str())
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                });
                            tool.bound_output();
                            tool.state = if block["is_error"] == true {
                                ToolState::Failed
                            } else {
                                ToolState::Completed
                            };
                            events.push(Event::Tool(tool.clone()));
                        }
                        self.claude_task_result(block, &v, &mut events);
                        self.claude_settle_inline_agent(block);
                    }
                }
            }
            "result" => {
                if self.compact && !self.compact_confirmed && v["is_error"] != true {
                    return Err("Claude Code did not confirm context compaction".into());
                }
                let usage = &v["usage"];
                let input = usage["input_tokens"].as_u64().unwrap_or(0);
                let output = usage["output_tokens"].as_u64().unwrap_or(0);
                let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                let metrics = TurnMetrics {
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: read,
                    cache_write_tokens: write,
                    cache_hit_percent: ((usage.get("cache_read_input_tokens").is_some()
                        || usage.get("cache_creation_input_tokens").is_some())
                        && input + read + write > 0)
                        .then(|| read as f64 / (input + read + write) as f64 * 100.),
                    cost_usd: v["total_cost_usd"]
                        .as_f64()
                        .filter(|cost| cost.is_finite() && *cost >= 0.),
                    credits: None,
                };
                if metrics.reported() {
                    events.push(Event::Metrics(metrics));
                }
                // The last iteration is what sits in the window. Routed
                // providers send `iterations: []` while `usage` sums the
                // whole turn, so report no level then and keep the last
                // assistant reading.
                let used = match usage["iterations"].as_array() {
                    Some(iterations) => iterations.last().and_then(crate::context::claude_used),
                    None => crate::context::claude_used(usage),
                };
                let window = v["modelUsage"]
                    .as_object()
                    .into_iter()
                    .flat_map(|models| models.values())
                    .filter_map(|model| model["contextWindow"].as_u64())
                    .filter(|w| *w > 0)
                    .max();
                if used.is_some() || window.is_some() {
                    events.push(Event::Context { used, window });
                }
                if v["is_error"] == true {
                    let error = v["result"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v["errors"].to_string());
                    return Err(self.claude_error(&error, &v));
                }
                // A self-started turn whose start went unseen still gets its
                // marker. When a user follow-up wins the race, the marker is
                // emitted from the result, so it trails the wake's text
                // rather than leading it.
                let woke_at_start = self.activities.wake_handled;
                if v["origin"]["kind"] == "task-notification" && !woke_at_start {
                    if let Some(summary) = self.activities.wake_summaries.pop_front() {
                        events.push(Self::wake_marker(summary));
                    }
                }
                self.activities.wake_handled = false;
                // Another finished task is queued: its own wake may follow.
                // The window opens at this result, not the wake's start, since
                // a wake can run longer than the grace. A user write into the
                // wake leaves the next turn theirs to answer.
                if woke_at_start
                    && !self.activities.wake_summaries.is_empty()
                    && !self.activities.steered_in_turn
                {
                    self.claude.resume_at = Some(Instant::now() + Duration::from_secs(15));
                }
                // Some versions omit partial/assistant frames in simple turns.
                if !self.begun {
                    self.text(
                        &self.claude.message.to_string(),
                        v["result"].as_str().unwrap_or(""),
                        true,
                        &mut events,
                    );
                }
                self.claude.result_seen = true;
                if self.activities.claude_background.is_empty()
                    && self.activities.claude_pending.is_empty()
                    && self.claude.resume_at.is_none()
                {
                    events.push(Event::Done);
                    self.done = true;
                } else {
                    events.push(Event::Status("Waiting for background tasks…".into()));
                }
            }
            "control_request" => {
                let request = &v["request"];
                if request["subtype"] == "can_use_tool" {
                    if request["tool_name"] == "AskUserQuestion" {
                        self.question(
                            v["request_id"].clone(),
                            "claude",
                            request.clone(),
                            &mut events,
                            stdin,
                        )?;
                        return Ok(events);
                    }
                    if request["tool_name"] == "ExitPlanMode" {
                        if let Some(plan) = request["input"]["plan"].as_str() {
                            events.push(Event::Plan(plan.into()));
                        }
                        write_json(
                            stdin,
                            json!({"type":"control_response","response":{"subtype":"success","request_id":v["request_id"],"response":{"behavior":"deny","message":PLAN_CAPTURED}}}),
                        )?;
                        return Ok(events);
                    }
                    let detail =
                        serde_json::to_string_pretty(&request["input"]).unwrap_or_default();
                    self.approval(
                        v["request_id"].clone(),
                        "claude",
                        request.clone(),
                        detail,
                        stdin,
                        &mut events,
                    )?;
                } else {
                    write_json(
                        stdin,
                        json!({"type":"control_response","response":{"subtype":"error","request_id":v["request_id"],"error":"Unsupported host request"}}),
                    )?;
                }
            }
            _ => {}
        }
        Ok(events)
    }
}
