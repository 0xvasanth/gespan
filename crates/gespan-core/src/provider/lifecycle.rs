//! Lifecycle rules from the desktop adapters. An idle stream is not a failed turn.
use super::*;
static CONTROL_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The control request that mounts the tool endpoint.
pub(super) const CLAUDE_MCP: &str = "native-mcp";
const CLAUDE_MCP_WAIT: Duration = Duration::from_secs(15);

/// Where a new Claude process stands with the tool endpoint.
#[derive(Default)]
pub(super) enum ClaudeMcp {
    #[default]
    Settled,
    Waiting(Instant),
}

#[cfg(test)]
mod tool_tests {
    use super::*;
    use crate::tool_servers::{Access, Mount, VISUAL};

    fn protocol(kind: Kind, session: i64, provider_id: Option<&str>) -> Protocol {
        let mount = Mount {
            session,
            worktree: "/tmp".into(),
            toolkits: [VISUAL.to_owned()].into(),
            ..Default::default()
        };
        let mut protocol = Protocol::new(Request {
            kind,
            binary: "unused".into(),
            cwd: "/tmp".into(),
            prompt: "Hello".into(),
            provider_id: provider_id.map(str::to_owned),
            model: None,
            settings: crate::agent::Settings {
                mount: Some(mount.clone()),
                ..Default::default()
            },
        });
        protocol.access = Access::open(&mount);
        protocol
    }

    fn sent(out: &mut Vec<u8>) -> Vec<Value> {
        let text = String::from_utf8(std::mem::take(out)).unwrap();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn mounted_first(session: i64) -> (Protocol, Vec<u8>) {
        let mut protocol = protocol(Kind::Claude, session, None);
        let mut out = Vec::new();
        protocol.claude_prompt(&mut out).unwrap();
        let frames = sent(&mut out);
        assert_eq!(frames.len(), 1, "the prompt waits for the endpoint");
        assert_eq!(frames[0]["request"]["subtype"], "mcp_set_servers");
        protocol.claude_prompt(&mut out).unwrap();
        assert!(out.is_empty(), "still waiting: {:?}", sent(&mut out));
        (protocol, out)
    }

    fn prompt_followed(protocol: &Protocol, out: &mut Vec<u8>) {
        let frames = sent(out);
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["type"], "user");
        assert!(protocol.claude.prompt_sent);
    }

    #[test]
    fn claude_gets_its_prompt_once_the_endpoint_is_mounted() {
        let (mut protocol, mut out) = mounted_first(5201);
        let answer = json!({"type":"control_response","response":{
            "subtype":"success","request_id":CLAUDE_MCP,
        }});
        protocol.frame(answer, &mut out).unwrap();
        prompt_followed(&protocol, &mut out);
    }

    #[test]
    fn claude_runs_without_tools_when_it_refuses_the_endpoint() {
        let (mut protocol, mut out) = mounted_first(5202);
        let answer = json!({"type":"control_response","response":{
            "subtype":"error","request_id":CLAUDE_MCP,"error":"no",
        }});
        protocol.frame(answer, &mut out).unwrap();
        prompt_followed(&protocol, &mut out);
    }

    #[test]
    fn a_claude_that_never_answers_still_gets_its_prompt() {
        let (mut protocol, mut out) = mounted_first(5203);
        protocol.claude_mcp_tick(&mut out).unwrap();
        assert!(out.is_empty(), "too soon to give up");
        protocol.claude.mcp = ClaudeMcp::Waiting(Instant::now() - CLAUDE_MCP_WAIT);
        protocol.claude_mcp_tick(&mut out).unwrap();
        prompt_followed(&protocol, &mut out);
    }

    #[test]
    fn a_warm_claude_holds_its_credential_and_goes_straight_to_the_prompt() {
        let mut protocol = protocol(Kind::Claude, 5204, None);
        protocol.access = None;
        let mut out = Vec::new();
        protocol.claude_prompt(&mut out).unwrap();
        prompt_followed(&protocol, &mut out);
    }

    #[test]
    fn codex_gets_the_endpoint_on_a_new_thread_and_on_a_resumed_one() {
        for (provider_id, method) in [(None, "thread/start"), (Some("t"), "thread/resume")] {
            let mut protocol = protocol(Kind::Codex, 5205, provider_id);
            let mut out = Vec::new();
            protocol
                .frame(json!({"id":1,"result":{}}), &mut out)
                .unwrap();
            let frames = sent(&mut out);
            let request = frames.iter().find(|frame| frame["method"] == method);
            let server = &request.unwrap()["params"]["config"]["mcp_servers"]["gespan"];
            assert!(server["url"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1"));
        }
    }

    #[test]
    fn codex_with_no_credential_starts_with_no_config() {
        let mut protocol = protocol(Kind::Codex, 5206, None);
        protocol.access = None;
        let mut out = Vec::new();
        protocol
            .frame(json!({"id":1,"result":{}}), &mut out)
            .unwrap();
        let frames = sent(&mut out);
        let request = frames
            .iter()
            .find(|frame| frame["method"] == "thread/start");
        assert!(request.unwrap()["params"].get("config").is_none());
    }
}

impl Protocol {
    pub(super) fn codex_elicitation(
        &mut self,
        wire: Value,
        mut request: Value,
        events: &mut Vec<Event>,
        stdin: &mut impl Write,
    ) -> Result<(), String> {
        let confirmation = mcp_confirmation(&request);
        let Some((title, content)) = confirmation else {
            events.push(Event::Status("This MCP server requested a form or browser sign-in that Gespan does not support yet. Complete it in the server's own interface.".into()));
            return write_json(
                stdin,
                json!({"id":wire,"result":{"action":"cancel","content":null,"_meta":null}}),
            );
        };
        if !self.request.settings.plan && self.request.settings.access == AccessMode::FullAccess {
            return write_json(
                stdin,
                json!({"id":wire,"result":{"action":"accept","content":content,"_meta":null}}),
            );
        }
        request["nativeContent"] = content;
        let id = wire.to_string();
        self.approvals
            .insert(id.clone(), (wire, "codex-mcp".into(), request));
        events.push(Event::Approval(Approval {
            id,
            title,
            detail: String::new(),
            can_remember: false,
            tool: None,
        }));
        Ok(())
    }

    /// Mounts the tool endpoint first, so the prompt's turn can use it. Over
    /// the control channel, the token reaches neither argv nor env.
    pub(super) fn claude_prompt(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        if self.claude.prompt_sent {
            return Ok(());
        }
        if let Some(access) = self.access.take() {
            write_json(stdin, access.claude_control(CLAUDE_MCP))?;
            self.claude.mcp = ClaudeMcp::Waiting(Instant::now());
            return Ok(());
        }
        if matches!(self.claude.mcp, ClaudeMcp::Waiting(_)) {
            return Ok(());
        }
        if self.hold_prompt(super::prompt_gate::HeldPrompt::Claude) {
            return Ok(());
        }
        let prompt = if self.request.settings.effort.as_deref() == Some("ultrathink") {
            if self.request.prompt.is_empty() {
                "Ultrathink:".into()
            } else {
                format!("Ultrathink:\n{}", self.request.prompt)
            }
        } else {
            self.request.prompt.clone()
        };
        tracing::debug!(target: "gespan::send", stage = "claude.prompt_written");
        write_json(
            stdin,
            json!({"type":"user","session_id":"","parent_tool_use_id":null,
                "message":{"role":"user","content":self.request.claude_content(&prompt)?}}),
        )?;
        self.claude.prompt_sent = true;
        self.follow_up_ready = true;
        Ok(())
    }

    pub(super) fn claude_mcp_answered(
        &mut self,
        response: &Value,
        stdin: &mut impl Write,
    ) -> Result<(), String> {
        // The error stays out of the log: it may quote the token back.
        if response["subtype"] != "success" {
            tracing::warn!("Claude refused the tool endpoint");
        }
        tracing::debug!(target: "gespan::send", stage = "claude.mcp_answered");
        self.claude.mcp = ClaudeMcp::Settled;
        self.claude_prompt(stdin)
    }

    /// A CLI that never answers still gets its prompt, with no tools.
    fn claude_mcp_tick(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        if let ClaudeMcp::Waiting(since) = self.claude.mcp {
            if since.elapsed() >= CLAUDE_MCP_WAIT {
                tracing::warn!("Claude did not answer the tool endpoint request");
                self.claude.mcp = ClaudeMcp::Settled;
                self.claude_prompt(stdin)?;
            }
        }
        Ok(())
    }

    pub(super) fn lifecycle_tick(
        &mut self,
        started: Instant,
        stdin: &mut impl Write,
    ) -> Result<Vec<Event>, String> {
        self.claude_mcp_tick(stdin)?;
        let mut events = self.expire_questions(stdin)?;
        if self.request.kind.is_acp() {
            events.extend(self.acp_tick(stdin)?);
        } else if self.request.kind.is_pi() {
            events.extend(self.pi_tick(stdin)?);
        } else if self.request.kind == Kind::Claude {
            // The desktop adapter proceeds after eight seconds even when older
            // CLIs never acknowledge the initialize control request.
            if !self.claude.prompt_sent && started.elapsed() >= Duration::from_secs(8) {
                self.claude_prompt(stdin)?;
            }
            self.sync_background(&mut events);
            if self.claude.result_seen
                && self.activities.claude_background.is_empty()
                && self.activities.claude_pending.is_empty()
                && self.claude.resume_at.is_none_or(|at| Instant::now() >= at)
            {
                self.done = true;
                events.push(Event::Done);
            }
        }
        Ok(events)
    }

    /// A user Stop uses the provider's cancellation API before transport teardown.
    /// Returns whether there is an active request whose acknowledgement can settle.
    pub(super) fn interrupt(
        &mut self,
        stdin: &mut impl Write,
        kind: Interrupt,
    ) -> Result<bool, String> {
        let interrupt_id = format!(
            "native-interrupt-{}",
            CONTROL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        self.interrupt_id = Some(interrupt_id.clone());
        match self.request.kind {
            Kind::Codex => {
                self.approvals.clear();
                self.questions.clear();
                if let (Some(thread), Some(turn)) = (&self.request.provider_id, &self.active_turn) {
                    write_json(
                        stdin,
                        json!({"id":interrupt_id,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}}),
                    )?;
                    return Ok(true);
                }
                Ok(false)
            }
            Kind::Claude if kind == Interrupt::Hold => {
                // Hold only interrupts between steps, so nothing is pending, and it
                // leaves background tasks running.
                write_json(
                    stdin,
                    json!({"type":"control_request","request_id":interrupt_id,"request":{"subtype":"interrupt"}}),
                )?;
                Ok(self.claude.prompt_sent)
            }
            Kind::Claude => {
                // Deny outstanding permissions and skip questions before interrupt,
                // exactly as cancelClaudeTurn does. Stop includes background tasks.
                for id in self.approvals.keys().cloned().collect::<Vec<_>>() {
                    self.respond(
                        Control {
                            id,
                            decision: Decision::Deny,
                        },
                        stdin,
                    )?;
                }
                for id in self.questions.keys().cloned().collect::<Vec<_>>() {
                    self.respond(
                        Control {
                            id,
                            decision: Decision::Deny,
                        },
                        stdin,
                    )?;
                }
                let tasks: Vec<_> = self.activities.claude_background.keys().cloned().collect();
                for (ix, task) in tasks.iter().enumerate() {
                    write_json(
                        stdin,
                        json!({"type":"control_request","request_id":format!("native-stop-task-{ix}"),"request":{"subtype":"stop_task","task_id":task}}),
                    )?;
                }
                write_json(
                    stdin,
                    json!({"type":"control_request","request_id":interrupt_id,"request":{"subtype":"interrupt"}}),
                )?;
                Ok(self.claude.prompt_sent)
            }
            kind if kind.is_pi() => {
                self.pi_cancel(stdin)?;
                Ok(self.follow_up_ready)
            }
            _ => {
                let active = self.acp_active_session().is_some();
                self.acp_cancel(stdin)?;
                Ok(active)
            }
        }
    }
}

/// Stop ends the turn and everything it started; Hold only ends the turn.
/// Checked against Claude Code 2.1.310: a `run_in_background` Bash command kept
/// running after an `interrupt` sent without `stop_task`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Interrupt {
    Stop,
    Hold,
}

impl Protocol {
    pub(super) fn codex_access(&self) -> (Value, &'static str, Value, &'static str) {
        let (policy, sandbox, mut sandbox_policy, reviewer) = self.request.settings.access.codex();
        if (self.request.settings.operator
            || self.request.settings.operator_enabled
            || self.request.settings.orchestrator)
            && sandbox_policy["type"] != "dangerFullAccess"
        {
            sandbox_policy["networkAccess"] = true.into();
        }
        (policy, sandbox, sandbox_policy, reviewer)
    }
}

fn mcp_confirmation(request: &Value) -> Option<(String, Value)> {
    if !matches!(
        request["mode"].as_str(),
        Some("form" | "openai/form" | "openaiForm")
    ) {
        return None;
    }
    let schema = request["requestedSchema"].as_object()?;
    if schema.get("type")?.as_str()? != "object"
        || schema.keys().any(|key| {
            ![
                "type",
                "properties",
                "required",
                "title",
                "description",
                "$schema",
                "additionalProperties",
            ]
            .contains(&key.as_str())
        })
    {
        return None;
    }
    let properties = schema.get("properties")?.as_object()?;
    if properties.len() > 1 {
        return None;
    }
    if let Some(required) = schema.get("required") {
        if required
            .as_array()?
            .iter()
            .any(|key| key.as_str().is_none_or(|key| !properties.contains_key(key)))
        {
            return None;
        }
    }
    let mut content = serde_json::Map::new();
    let mut detail = String::new();
    if let Some((key, field)) = properties.iter().next() {
        let field = field.as_object()?;
        if field.get("type")?.as_str()? != "boolean"
            || field
                .keys()
                .any(|key| !["type", "title", "description", "default"].contains(&key.as_str()))
        {
            return None;
        }
        detail = field
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_owned();
        if let Some(description) = field
            .get("description")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            detail.push_str(" — ");
            detail.push_str(description);
        }
        content.insert(key.clone(), true.into());
    }
    let mut title = format!(
        "{}: {}",
        request["serverName"].as_str().unwrap_or("MCP"),
        request["message"].as_str().unwrap_or("Approve request")
    );
    if !detail.is_empty() {
        title.push_str(" — ");
        title.push_str(&detail);
    }
    Some((title, content.into()))
}

pub(super) fn recoverable_codex_resume(error: &str) -> bool {
    let message = error.to_lowercase();
    message.contains("thread")
        && [
            "not found",
            "unknown thread",
            "no such thread",
            "does not exist",
            "missing thread",
            "thread id",
        ]
        .iter()
        .any(|part| message.contains(part))
}

pub(super) fn claude_session_id() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
