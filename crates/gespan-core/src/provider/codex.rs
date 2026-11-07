//! Codex's `app-server` JSON-RPC: launch arguments, per-turn state and the
//! frames only Codex sends.
use super::*;

#[derive(Default)]
pub(super) struct State {
    /// The last `tokenUsage.total`, so a repeated report is not counted twice.
    pub(super) usage_total: u64,
    /// Image items already reported; their completion can repeat.
    pub(super) generated_images: std::collections::HashSet<String>,
}

pub(super) fn configure(command: &mut Command, request: &Request) -> Result<(), String> {
    command.arg("app-server");
    if let Some(pilot) = &request.settings.pilot {
        if let Some(home) = crate::pilot::codex_home(&pilot.root, &request.settings.account.id)? {
            let sqlite_home = serde_json::to_string(&home).map_err(|e| e.to_string())?;
            command
                .env("CODEX_HOME", &home)
                .env("CODEX_SQLITE_HOME", &home)
                .args(["-c", &format!("sqlite_home={sqlite_home}")]);
        }
    }
    Ok(())
}

impl Protocol {
    /// Frames Codex answers or drops before any shared handling sees them.
    pub(super) fn codex_screen(
        &mut self,
        v: &Value,
        stdin: &mut impl Write,
    ) -> Result<Option<Vec<Event>>, String> {
        // Codex requests its clock before thread/start may have replied.
        if v["method"] == "currentTime/read" && v.get("id").is_some() {
            write_json(
                stdin,
                json!({"id":v["id"],"result":{"currentTimeAt":crate::time::now_ms()/1000}}),
            )?;
            return Ok(Some(Vec::new()));
        }
        let turn = v["params"]["turnId"]
            .as_str()
            .or_else(|| v["params"]["turn"]["id"].as_str());
        if turn.is_some() && turn == self.previous_turn.as_deref() {
            return Ok(Some(Vec::new()));
        }
        if let Some(id) = v["id"].as_str() {
            if let Some((wire, kind, request, question)) = self.async_answers.remove(id) {
                if let Some(error) = v.get("error").filter(|e| !e.is_null()) {
                    let message = codex_error(error, "Could not send your answer to Codex");
                    self.questions
                        .insert(question.id.clone(), (wire, kind, request, question.clone()));
                    self.visible_question = Some(question.id.clone());
                    return Ok(Some(vec![
                        Event::Status(message),
                        Event::Question(question),
                    ]));
                }
                return Ok(Some(vec![Event::QuestionResolved(question.id)]));
            }
            if id.starts_with("native-answer-") || id.starts_with("native-interrupt-") {
                return Ok(Some(Vec::new()));
            }
        }
        let method = v["method"].as_str().unwrap_or("");
        if self.previous_turn.is_some()
            && self.active_turn.is_none()
            && method != "turn/started"
            && (method.starts_with("item/") || method.starts_with("turn/") || method == "error")
        {
            return Ok(Some(Vec::new()));
        }
        Ok(None)
    }
    /// The turn a `turn/started` notification or the `turn/start` reply names,
    /// when it is new.
    pub(super) fn codex_turn_started(&mut self, v: &Value) -> Option<String> {
        let mut started_turn = None;
        if v["id"] == 2 {
            if let Some(id) = v["result"]["thread"]["id"].as_str() {
                self.request.provider_id = Some(id.into());
            }
        }
        if v["method"] == "turn/started" || v["id"] == 3 {
            let turn = if v["id"] == 3 {
                &v["result"]["turn"]
            } else {
                &v["params"]["turn"]
            };
            if let Some(id) = turn["id"].as_str() {
                if self.active_turn.as_deref() != Some(id) {
                    started_turn = Some(id.to_owned());
                }
                self.active_turn = Some(id.into());
            }
        }
        started_turn
    }

    pub(super) fn codex_frame(
        &mut self,
        v: Value,
        stdin: &mut impl Write,
        mut events: Vec<Event>,
    ) -> Result<Vec<Event>, String> {
        if let Some(error) = v.get("error").filter(|e| !e.is_null()) {
            let message = codex_error(error, "RPC request failed");
            if v["id"] == 2
                && self.request.provider_id.is_some()
                && lifecycle::recoverable_codex_resume(&message)
            {
                if self.request.settings.pilot.is_some() {
                    return Err("The saved Pilot Codex context could not be resumed. Retry to keep its saved context.".into());
                }
                self.request.provider_id = None;
                return self.frame(json!({"id":1,"result":{}}), stdin);
            }
            return Err(message);
        }
        match v["id"].as_u64() {
            Some(1) if v.get("result").is_some() => {
                if !self.initialized {
                    write_json(stdin, json!({"method":"initialized"}))?;
                    self.initialized = true;
                }
                let (policy, sandbox, sandbox_policy, reviewer) = self.codex_access();
                let mut params = json!({"cwd":self.request.cwd,"approvalPolicy":policy,"sandbox":sandbox,"sandboxPolicy":sandbox_policy,"approvalsReviewer":reviewer,
                        "developerInstructions":format!("You are the coding agent in Gespan. Use the available tools to inspect and work in the supplied cwd. Do not ask the user to paste files or git output you can inspect yourself. Respect the selected access policy and request approval when required. {}", if self.request.settings.plan { "This is a planning turn: inspect and propose a plan without changing files." } else { "" })});
                if self.request.settings.text_only {
                    params["ephemeral"] = true.into();
                    params["developerInstructions"] = "Answer the supplied text request directly. Do not call tools or inspect the workspace.".into();
                }
                // Over stdin, so the token reaches neither argv nor env.
                if let (Some(access), Some(mount)) = (&self.access, &self.request.settings.mount) {
                    params["config"] = access.codex_config(mount);
                }
                if let Some(model) = &self.request.model {
                    params["model"] = model.clone().into();
                }
                if let Some(tier) = self
                    .request
                    .settings
                    .service_tier
                    .as_deref()
                    .filter(|tier| *tier != "default")
                {
                    params["serviceTier"] = tier.into();
                }
                let method = if let Some(id) = &self.request.provider_id {
                    params["threadId"] = id.clone().into();
                    // The app already owns its transcript. Resume the
                    // provider context without serializing its history.
                    params["excludeTurns"] = true.into();
                    "thread/resume"
                } else {
                    "thread/start"
                };
                write_json(stdin, json!({"id":2,"method":method,"params":params}))?;
            }
            Some(2) if v.get("result").is_some() => {
                let id = v["result"]["thread"]["id"]
                    .as_str()
                    .ok_or("Codex returned no thread id")?;
                events.push(Event::SessionId(id.into()));
                if let Some(model) = v["result"]["model"].as_str() {
                    events.push(Event::Model(model.into()));
                }
                if self.compact {
                    write_json(
                        stdin,
                        json!({"id":3,"method":"thread/compact/start","params":{"threadId":id}}),
                    )?;
                    return Ok(events);
                }
                let (policy, _, sandbox, reviewer) = self.codex_access();
                let mut params = json!({"threadId":id,"input":self.request.codex_input(),
                        "approvalPolicy":policy,"sandboxPolicy":sandbox,"approvalsReviewer":reviewer});
                if let Some(effort) = &self.request.settings.effort {
                    params["effort"] = effort.clone().into();
                }
                if self.request.settings.fast {
                    params["serviceTier"] = "fast".into();
                }
                if let Some(tier) = self
                    .request
                    .settings
                    .service_tier
                    .as_deref()
                    .filter(|tier| *tier != "default")
                {
                    params["serviceTier"] = tier.into();
                }
                if self.request.settings.plan {
                    params["sandboxPolicy"] = json!({"type":"readOnly"});
                    if self.request.settings.operator
                        || self.request.settings.operator_enabled
                        || self.request.settings.orchestrator
                    {
                        params["sandboxPolicy"]["networkAccess"] = true.into();
                    }
                    params["approvalPolicy"] = "never".into();
                }
                if let Some(model) = self
                    .request
                    .model
                    .as_deref()
                    .or_else(|| v["result"]["model"].as_str())
                {
                    params["collaborationMode"] = json!({"mode": if self.request.settings.plan { "plan" } else { "default" }, "settings":{"model":model,"reasoning_effort":self.request.settings.effort,"developer_instructions":null}});
                }
                if let Some(model) = &self.request.model {
                    params["model"] = model.clone().into();
                }
                let frame = json!({"id":3,"method":"turn/start","params":params});
                if self.baseline_pending() {
                    self.held_prompt = Some(super::prompt_gate::HeldPrompt::Codex(frame));
                } else {
                    write_json(stdin, frame)?;
                }
            }
            _ => {}
        }
        let p = &v["params"];
        match v["method"].as_str().unwrap_or("") {
            "account/rateLimits/updated" => {
                let windows = crate::usage::codex_live_windows(p);
                if !windows.is_empty() {
                    events.push(Event::UsageWindows(windows));
                }
            }
            "thread/tokenUsage/updated" => {
                let usage = &p["tokenUsage"];
                events.push(Event::Context {
                    used: usage["last"]["totalTokens"].as_u64(),
                    window: usage["modelContextWindow"].as_u64().filter(|w| *w > 0),
                });
                let total = usage["total"]["totalTokens"].as_u64().unwrap_or(0);
                if usage["last"].is_object() && (total == 0 || total != self.codex.usage_total) {
                    self.codex.usage_total = total;
                    let last = &usage["last"];
                    self.metrics.input_tokens += last["inputTokens"].as_u64().unwrap_or(0);
                    self.metrics.output_tokens += last["outputTokens"].as_u64().unwrap_or(0);
                    self.metrics.cache_read_tokens +=
                        last["cachedInputTokens"].as_u64().unwrap_or(0);
                    self.metrics.cache_write_tokens +=
                        last["cacheWriteInputTokens"].as_u64().unwrap_or(0);
                    if (last.get("cachedInputTokens").is_some()
                        || last.get("cacheWriteInputTokens").is_some())
                        && self.metrics.input_tokens > 0
                    {
                        self.metrics.cache_hit_percent = Some(
                            self.metrics.cache_read_tokens as f64
                                / (self.metrics.input_tokens + self.metrics.cache_write_tokens)
                                    as f64
                                * 100.,
                        );
                    }
                    if self.metrics.reported() {
                        events.push(Event::Metrics(self.metrics));
                    }
                }
            }
            "item/agentMessage/delta" => {
                self.text(
                    p["itemId"].as_str().unwrap_or("assistant"),
                    p["delta"].as_str().unwrap_or(""),
                    false,
                    &mut events,
                );
            }
            "item/completed" if p["item"]["type"] == "agentMessage" => {
                self.text(
                    p["item"]["id"].as_str().unwrap_or("assistant"),
                    p["item"]["text"].as_str().unwrap_or(""),
                    true,
                    &mut events,
                );
            }
            "item/started" if p["item"]["type"] == "agentMessage" => {}
            "item/started" if p["item"]["type"] == "plan" => {}
            "item/completed" if p["item"]["type"] == "plan" => {
                if let Some(plan) = p["item"]["text"].as_str().filter(|t| !t.is_empty()) {
                    events.push(Event::Plan(plan.into()));
                }
            }
            "item/started" if p["item"]["type"] == "imageGeneration" => {}
            "item/completed" if p["item"]["type"] == "imageGeneration" => {
                let item = &p["item"];
                let id = item["id"].as_str().unwrap_or("image").to_owned();
                let data = item["result"].as_str().unwrap_or("").trim();
                if !data.is_empty() && self.codex.generated_images.insert(id.clone()) {
                    events.push(Event::GeneratedImage {
                        id,
                        data: data.to_owned(),
                        alt: item["revisedPrompt"].as_str().map(str::to_owned),
                    });
                }
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                events.push(Event::Reasoning(p["delta"].as_str().unwrap_or("").into()));
                events.push(Event::Status("Thinking…".into()));
            }
            "item/started" | "item/completed" => {
                self.codex_tool(&p["item"], v["method"] == "item/completed", &mut events)
            }
            "thread/compacted" if self.active_compaction.is_some() => {
                self.compaction(None, ToolState::Completed, &mut events);
            }
            "item/commandExecution/outputDelta" => {
                let id = p["itemId"].as_str().unwrap_or("");
                let tool = self
                    .tools
                    .entry(id.into())
                    .or_insert_with(|| ToolCall::new(id, "commandExecution"));
                tool.append_output(p["delta"].as_str().unwrap_or(""));
                events.push(Event::Tool(tool.clone()));
            }
            "turn/completed" | "turn/aborted" => {
                let status = p["turn"]["status"].as_str().unwrap_or("completed");
                if status == "failed" {
                    return Err(self.codex_failure(&p["turn"]["error"], "Codex turn failed"));
                }
                let interrupted = matches!(status, "interrupted" | "cancelled" | "canceled")
                    || v["method"] == "turn/aborted";
                for item in p["turn"]["items"].as_array().into_iter().flatten() {
                    if interrupted && item["type"] == "contextCompaction" {
                        // An aborted turn can include its unfinished
                        // compaction item in the final snapshot.
                        continue;
                    }
                    self.codex_tool(item, true, &mut events);
                }
                if interrupted {
                    for tool in self
                        .tools
                        .values_mut()
                        .filter(|tool| tool.state == ToolState::Running)
                    {
                        tool.state = ToolState::Cancelled;
                        events.push(Event::Tool(tool.clone()));
                    }
                }
                events.push(Event::Done);
                self.done = true;
            }
            "error" if p["willRetry"] != true => {
                return Err(self.codex_failure(&p["error"], "Codex stream error"));
            }
            "error" => {
                events.push(Event::Status(codex_error(
                    &p["error"],
                    "Codex is reconnecting…",
                )));
            }
            "mcpServer/elicitation/request" => {
                self.codex_elicitation(v["id"].clone(), p.clone(), &mut events, stdin)?
            }
            "item/tool/requestUserInput" => {
                self.question(v["id"].clone(), "codex", p.clone(), &mut events, stdin)?
            }
            method
                if v.get("id").is_some()
                    && (method.ends_with("/requestApproval")
                        || matches!(method, "execCommandApproval" | "applyPatchApproval")) =>
            {
                let detail = p["command"]
                    .as_str()
                    .or(p["reason"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_default();
                self.approval(
                    v["id"].clone(),
                    method,
                    p.clone(),
                    detail,
                    stdin,
                    &mut events,
                )?;
            }
            method if v.get("id").is_some() && !method.is_empty() => {
                // No approval UX in this gate: resolve every server request promptly.
                write_json(
                    stdin,
                    json!({"id":v["id"],"error":{"code":-32601,"message":format!("Unsupported method: {method}")}}),
                )?;
                events.push(Event::Status(format!(
                    "Unsupported Codex request: {method}"
                )));
            }
            _ => {}
        }
        Ok(events)
    }
}
