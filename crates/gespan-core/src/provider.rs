//! Native provider transports, ported from Gespan's TS adapters.
//! Blocking work belongs to a worker; never call `run` from a render/input handler.
use crate::agent::{AccessMode, Approval, Control, Decision, Settings, TurnMetrics};
use crate::review_policy::{self, Verdict};
use crate::tool::{ToolCall, ToolState};
use crate::{process, transcript::snapshot_remainder};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

mod acp;
mod agent_registry;
pub mod api;
mod commands;
pub use commands::{
    compact, discover_commands, is_raw_command, native_invocation, native_prompt, prepare_prompt,
    supports_compact, NativeCommand, NativeSubcommand,
};
mod attachments;
#[cfg(all(test, unix))]
mod baseline_tests;
mod batch;
mod compaction;
mod cursor_live;
pub(crate) mod cursor_store;
pub(crate) mod devin;
mod discovery;
mod follow_up;
mod hold;
pub use hold::HoldPoint;
use lifecycle::Interrupt;
pub(crate) mod claude;
mod codex;
mod error;
#[cfg(all(test, unix))]
mod hold_tests;
mod kind;

/// The call a Claude or Codex approval is about, for a readable card. Claude
/// sends the tool's name and input; Codex names an item it already announced,
/// or sends the command and its directory with the request.
fn approval_tool(
    kind: &str,
    request: &Value,
    tools: &HashMap<String, ToolCall>,
) -> Option<ToolCall> {
    if kind == "claude" {
        let name = request["tool_name"].as_str()?;
        let mut call = ToolCall::new(request["tool_use_id"].as_str().unwrap_or(""), name);
        call.input = serde_json::to_string(&request["input"]).unwrap_or_default();
        return Some(call);
    }
    if let Some(known) = request["itemId"].as_str().and_then(|id| tools.get(id)) {
        return Some(known.clone());
    }
    let command = request
        .get("command")
        .filter(|c| c.is_string() || c.is_array())?;
    let mut call = ToolCall::new(request["itemId"].as_str().unwrap_or(""), "commandExecution");
    let mut input = json!({ "command": command });
    if let Some(cwd) = request.get("cwd").filter(|cwd| cwd.is_string()) {
        input["cwd"] = cwd.clone();
    }
    call.input = input.to_string();
    Some(call)
}

#[cfg(test)]
mod launch_argv_tests;
mod lifecycle;
mod limits;
pub mod manifest;
mod opencode;
mod parked;
mod pi;
mod prompt_gate;
mod spec;
pub use error::ProviderError;
pub fn configure_idle_agents(limit: usize) {
    parked::configure(limit);
    opencode::configure(limit);
}
pub fn keep_idle_agents(ids: Vec<i64>) {
    parked::keep(ids);
}
/// Hold keeps these sessions' agents and their background work alive.
pub fn pin_idle_agents(ids: impl IntoIterator<Item = i64>) {
    parked::pin_idle_agents(ids);
}
pub fn idle_agent_pinned(id: i64) -> bool {
    parked::idle_agent_pinned(id)
}
/// Resume gives these sessions' held agents the usual idle period from now.
pub fn unpin_idle_agents(ids: impl IntoIterator<Item = i64>) {
    parked::unpin_idle_agents(ids);
}
pub fn discard_worktree(path: &Path) {
    parked::discard_worktree(path);
    opencode::discard_worktree(path);
}
/// Whether an agent Gespan ran for this session may still be writing to
/// the session's file: Hold parked it with background work, or Stop left it
/// finishing its turn.
pub fn agent_still_writing(kind: Kind, provider_id: &str, account_id: &str, cwd: &Path) -> bool {
    parked::has_live(kind, provider_id, account_id, cwd)
}
pub fn forget_session(kind: Kind, provider_id: &str, account_id: &str, cwd: &Path) {
    parked::forget_session(kind, provider_id, account_id, cwd);
    if kind == Kind::OpenCode {
        opencode::forget_session(provider_id, account_id, cwd);
    }
}
mod questions;
mod rewind;
pub mod signed_out;
mod tasks;
pub use cursor_live::recover_cursor_subagents;
pub use rewind::{rewind_last_turn, supports_rewind, RewindError};
#[cfg(all(test, unix))]
mod text_tests;
pub use kind::{pick_installed, Kind};
pub use spec::{Accounts, Family, Features, Login, ProfileHome, Spec};

const BATCH_BYTES: usize = 64 * 1024;
const TICK: Duration = Duration::from_millis(16);
const HELD_POLL: Duration = Duration::from_millis(2);

#[derive(Clone, Debug)]
pub struct Request {
    pub kind: Kind,
    pub binary: PathBuf,
    pub cwd: PathBuf,
    pub prompt: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub settings: Settings,
}

/// Claude's ExitPlanMode answer: the card is the review, so it stops here.
const PLAN_CAPTURED: &str = "The client captured your proposed plan. Stop here and wait for the user's feedback or implementation request in a later turn.";

// Events are short-lived batches; boxing every tool snapshot would only move
// the same bytes to the heap and touch every consumer's pattern.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Ready,
    Commands(Vec<NativeCommand>),
    FollowUp(crate::follow_up::Message),
    FollowUpFailed {
        id: String,
        error: String,
    },
    SessionId(String),
    TurnId(String),
    Model(String),
    Context {
        used: Option<u64>,
        window: Option<u64>,
    },
    /// The share of the context window in use, from providers that report
    /// no token counts.
    ContextPercent(f64),
    Metrics(TurnMetrics),
    /// Plan limits the provider reported mid-turn, fresher than the last probe.
    UsageWindows(Vec<crate::usage::Window>),
    Approval(Approval),
    ApprovalResolved(String),
    Question(crate::agent::Question),
    QuestionResolved(String),
    BeginAssistant,
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
    Status(String),
    /// What the turn still waits on after the provider yielded; empty clears it.
    Background(Vec<BackgroundTask>),
    UsageLimited(crate::usage::Limit),
    /// The turn failed because the provider CLI is signed out or its sign-in
    /// expired.
    SignedOut,
    Interjection {
        text: String,
        custom_type: String,
        severity: Option<String>,
    },
    /// A plan the provider hands over for review.
    Plan(String),
    /// A finished image as base64 PNG data, which the runtime saves.
    GeneratedImage {
        id: String,
        data: String,
        alt: Option<String>,
    },
    ConfigChanged {
        model: Option<String>,
        model_settings: HashMap<String, String>,
    },
    /// A held turn reached its safe point.
    HoldSettled(HoldPoint),
    Done,
}

/// A command or subagent left running after the provider yielded its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundTask {
    pub id: String,
    pub description: String,
    pub agent: bool,
    /// The shell command behind a backgrounded Bash call, when known.
    pub command: Option<String>,
    /// Unix milliseconds.
    pub started_at: u64,
    /// The transcript block (tool call id) that shows this task's work.
    pub tool_id: Option<String>,
    /// The task's latest progress description.
    pub latest: Option<String>,
    /// Tool steps recorded on that block so far.
    pub steps: u32,
}

impl Protocol {
    /// A terminal Codex error, marking the turn signed out when Codex says so.
    fn codex_failure(&mut self, error: &Value, fallback: &str) -> String {
        let message = codex_error(error, fallback);
        if signed_out::codex(error) {
            self.signed_out = true;
            return signed_out::message(self.request.kind, &message);
        }
        message
    }
}

fn codex_error(error: &Value, fallback: &str) -> String {
    let message = error["message"].as_str().unwrap_or(fallback);
    if error["codexErrorInfo"] == "usageLimitExceeded" {
        format!("Usage limit reached: {message}")
    } else {
        message.into()
    }
}

/// GUI apps may not inherit the login shell PATH. Try the same common user and
/// system install locations as harness.rs, plus Claude's versioned native install.
/// An explicit override wins and errors at launch rather than silently falling back.
pub fn resolve(kind: Kind) -> Option<PathBuf> {
    if let Some(path) = crate::cli::configured(kind) {
        return Some(path);
    }
    resolve_auto(kind)
}
pub fn resolve_auto(kind: Kind) -> Option<PathBuf> {
    let key = format!("GESPAN_{}_BINARY", kind.id().to_uppercase());
    if let Some(path) = std::env::var_os(key).filter(|s| !s.is_empty()) {
        return Some(path.into());
    }
    let mut candidates: Vec<PathBuf> = Some(std::ffi::OsString::from(
        crate::scm::shell::gui_search_path(),
    ))
    .into_iter()
    .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
    .filter(|p| !p.as_os_str().is_empty())
    .map(|p| p.join(kind.binary_name()))
    .collect();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        for dir in kind.install_dirs().iter().copied().chain([
            ".local/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".cargo/bin",
            "n/bin",
        ]) {
            candidates.push(home.join(dir).join(kind.binary_name()));
        }
        if kind == Kind::Claude {
            candidates.push(home.join(".claude/local/claude"));
            let dir = home.join(".local/share/claude/versions");
            let mut versions: Vec<_> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .collect();
            versions.sort_by_key(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .split('.')
                    .map(|s| s.parse::<u32>().unwrap_or(0))
                    .collect::<Vec<_>>()
            });
            candidates.extend(versions.into_iter().rev());
        }
        #[cfg(windows)]
        candidates.push(home.join("AppData/Roaming/npm").join(kind.binary_name()));
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
    ] {
        candidates.push(Path::new(dir).join(kind.binary_name()));
    }
    if matches!(kind, Kind::Cursor | Kind::Pi) {
        let alias = if kind == Kind::Cursor {
            "agent"
        } else {
            "pi-coding-agent"
        };
        let paths: Vec<_> = candidates
            .iter()
            .filter_map(|p| p.parent().map(|dir| dir.join(alias)))
            .collect();
        candidates.extend(paths);
    }
    candidates.into_iter().find_map(|p| {
        #[cfg(windows)]
        for ext in ["exe", "cmd", "bat", "com"] {
            let candidate = p.with_extension(ext);
            if executable(&candidate) && discovery::matches(kind, &candidate) {
                return Some(candidate);
            }
        }
        (executable(&p) && discovery::matches(kind, &p)).then_some(p)
    })
}
fn executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// On resume, Claude Code replays a background-task notification from the
/// earlier session and answers it with an empty result before the real turn.
fn claude_turn_result(value: &Value) -> bool {
    value["type"] == "result"
        && !(value["origin"]["kind"] == "task-notification" && value["num_turns"] == 0)
}

fn write_json(out: &mut impl Write, value: Value) -> Result<(), String> {
    let mut frame = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    frame.push(b'\n');
    out.write_all(&frame)
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}

/// State machine is deliberately testable without spawning a provider.
struct Protocol {
    initialized: bool,
    compact: bool,
    command_probe: bool,
    compact_confirmed: bool,
    active_turn: Option<String>,
    previous_turn: Option<String>,
    interrupt_id: Option<String>,
    /// Hold was requested: pending questions wait instead of auto-resolving.
    holding: bool,
    /// The terminal error was the provider's own signed-out signal.
    signed_out: bool,
    follow_up_ready: bool,
    steering: HashMap<String, (crate::follow_up::Message, Instant)>,
    request: Request,
    emitted: HashMap<String, String>,
    begun: bool,
    done: bool,
    tools: HashMap<String, ToolCall>,
    active_compaction: Option<String>,
    activities: tasks::Activities,
    text_key: Option<String>,
    approvals: HashMap<String, (Value, String, Value)>,
    metrics: TurnMetrics,
    claude: claude::State,
    codex: codex::State,
    acp: acp::State,
    pi: pi::State,
    probe: bool,
    catalog_result: Option<Value>,
    questions: HashMap<String, (Value, String, Value, crate::agent::Question)>,
    question_order: std::collections::VecDeque<String>,
    visible_question: Option<String>,
    async_question_items: std::collections::HashSet<String>,
    async_answers: HashMap<String, (Value, String, Value, crate::agent::Question)>,
    /// This new process's tool credential. A warm process already holds one.
    access: Option<crate::tool_servers::Access>,
    /// The prompt step waiting for the turn's baseline (`prompt_gate`).
    held_prompt: Option<prompt_gate::HeldPrompt>,
}
impl Protocol {
    fn new(request: Request) -> Self {
        Self {
            request,
            initialized: false,
            compact: false,
            command_probe: false,
            compact_confirmed: false,
            active_turn: None,
            previous_turn: None,
            interrupt_id: None,
            holding: false,
            signed_out: false,
            follow_up_ready: false,
            steering: HashMap::new(),
            emitted: HashMap::new(),
            begun: false,
            done: false,
            tools: HashMap::new(),
            active_compaction: None,
            activities: tasks::Activities::default(),
            text_key: None,
            approvals: HashMap::new(),
            metrics: TurnMetrics::default(),
            claude: Default::default(),
            codex: Default::default(),
            acp: Default::default(),
            pi: Default::default(),
            probe: false,
            catalog_result: None,
            questions: HashMap::new(),
            question_order: Default::default(),
            visible_question: None,
            async_question_items: Default::default(),
            async_answers: HashMap::new(),
            access: None,
            held_prompt: None,
        }
    }
    fn text(&mut self, key: &str, text: &str, snapshot: bool, events: &mut Vec<Event>) {
        let already = self.emitted.entry(key.to_owned()).or_default();
        let text = if snapshot {
            snapshot_remainder(already, text)
        } else {
            text
        };
        if text.is_empty() {
            return;
        }
        already.push_str(text);
        let text = text.to_owned();
        if self.text_key.as_deref() != Some(key) {
            events.push(Event::BeginAssistant);
            self.begun = true;
            self.text_key = Some(key.to_owned());
        }
        events.push(Event::Text(text));
    }
    fn frame(&mut self, v: Value, stdin: &mut impl Write) -> Result<Vec<Event>, String> {
        if self.request.kind.family() == Family::Codex {
            if let Some(events) = self.codex_screen(&v, stdin)? {
                return Ok(events);
            }
        }
        let mut activity_events = Vec::new();
        if self.request.kind.family() == Family::Claude {
            if let Some(events) = self.claude_screen(&v, stdin, &mut activity_events)? {
                return Ok(events);
            }
        }
        if self.activity(&v, &mut activity_events) {
            return Ok(activity_events);
        }
        if let Some(events) = self.steer_response(&v) {
            return Ok(events);
        }
        let mut started_turn = None;
        if self.request.kind.family() == Family::Codex {
            started_turn = self.codex_turn_started(&v);
        }
        if match self.request.kind.family() {
            Family::Claude => v["type"] == "system" && v["subtype"] == "init",
            Family::Pi => v["type"] == "agent_start",
            _ => false,
        } {
            self.follow_up_ready = true;
        }
        let mut events = activity_events;
        if let Some(id) = started_turn {
            events.push(Event::TurnId(id));
        }
        if self.request.kind.family() == Family::Claude {
            self.claude_compaction(&v, &mut events);
        }
        if let Some(commands) = self.command_updates(&v) {
            events.push(Event::Commands(commands));
        }
        if self.request.kind.family() == Family::Codex {
            self.codex_async_questions(&v, &mut events, stdin)?;
        }
        match self.request.kind.family() {
            Family::Codex => self.codex_frame(v, stdin, events),
            Family::Claude => self.claude_frame(v, stdin, events),
            Family::Pi => {
                events.extend(self.pi_frame(v, stdin)?);
                Ok(events)
            }
            Family::Acp | Family::OpenCode => {
                events.extend(self.acp_frame(v, stdin)?);
                Ok(events)
            }
        }
    }
    /// Persisted as an interjection row/block (`customType: "task-notification"`),
    /// which both apps already store and load.
    fn wake_marker(summary: String) -> Event {
        Event::Interjection {
            text: summary,
            custom_type: "task-notification".into(),
            severity: None,
        }
    }
    /// The review policy's answer for this permission request, or `None` when
    /// no policy applies. Claude's Bash tool and every Codex approval request
    /// are covered; a Codex request that is not a command (a file change, extra
    /// permissions) is always denied, since a review never edits anything.
    fn review_verdict(&self, kind: &str, request: &Value) -> Option<Verdict> {
        let policy = self.request.settings.review.as_ref()?;
        if kind == "claude" && request["tool_name"] == "Bash" {
            let command = request["input"]["command"].as_str()?;
            return Some(review_policy::verdict(policy, command));
        }
        if self.request.kind.family() == Family::Codex && kind != "codex-mcp" {
            let command = match &request["command"] {
                Value::String(command) => Some(command.clone()),
                Value::Array(argv) => argv
                    .iter()
                    .map(|a| a.as_str())
                    .collect::<Option<Vec<_>>>()
                    .and_then(|argv| shlex::try_join(argv).ok()),
                _ => None,
            };
            let is_command = kind == "execCommandApproval" || kind.contains("ommandExecution");
            // A command asking for more than the sandbox gives is never routine.
            let extra =
                ["additionalPermissions", "permissions"]
                    .iter()
                    .any(|key| match &request[*key] {
                        Value::Null => false,
                        Value::Object(o) => !o.is_empty(),
                        Value::Array(a) => !a.is_empty(),
                        _ => true,
                    });
            return Some(match command {
                Some(command) if is_command && !extra => review_policy::verdict(policy, &command),
                _ => Verdict::Deny,
            });
        }
        None
    }
    fn approval(
        &mut self,
        wire: Value,
        kind: &str,
        request: Value,
        detail: String,
        stdin: &mut impl Write,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let id = wire.to_string();
        let title = if kind == "claude" {
            format!("Allow {}?", request["tool_name"].as_str().unwrap_or("tool"))
        } else if kind.contains("fileChange") || kind == "applyPatchApproval" {
            "Allow file changes?".into()
        } else if kind.contains("permissions") {
            "Allow additional permissions?".into()
        } else {
            "Allow this command?".into()
        };
        let tool = approval_tool(kind, &request, &self.tools);
        let mut detail =
            if let Some(tool) = request["itemId"].as_str().and_then(|id| self.tools.get(id)) {
                let detail = if detail.is_empty() {
                    tool.summary().1
                } else {
                    detail
                };
                if tool.output.is_empty() {
                    detail
                } else {
                    format!("{detail}\n\n{}", tool.output)
                }
            } else {
                detail
            };
        if detail.is_empty() {
            detail = request
                .get("permissions")
                .and_then(|p| serde_json::to_string_pretty(p).ok())
                .unwrap_or_else(|| "The agent is requesting permission to continue.".into());
        }
        let access = self.request.settings.effective_access();
        let read_only = access == AccessMode::ReadOnly;
        let read_only_query = read_only
            && kind == "claude"
            && request["tool_name"] == "Bash"
            && request["input"]["command"]
                .as_str()
                .is_some_and(claude::read_only::allowed);
        let review = self.review_verdict(kind, &request);
        self.approvals
            .insert(id.clone(), (wire, kind.into(), request));
        match review {
            Some(Verdict::Allow) => {
                return self.respond(
                    Control {
                        id,
                        decision: Decision::Allow,
                    },
                    stdin,
                );
            }
            Some(Verdict::Deny) => {
                return self.respond(
                    Control {
                        id,
                        decision: Decision::Deny,
                    },
                    stdin,
                );
            }
            _ => {}
        }
        if read_only_query && review.is_none() {
            return self.respond(
                Control {
                    id,
                    decision: Decision::Allow,
                },
                stdin,
            );
        }
        if self.request.settings.plan || (read_only && review != Some(Verdict::Ask)) {
            return self.respond(
                Control {
                    id,
                    decision: Decision::Deny,
                },
                stdin,
            );
        }
        let file_change = kind.contains("fileChange") || kind == "applyPatchApproval";
        let permissions = kind.contains("permissions");
        let ask = review == Some(Verdict::Ask);
        if !ask
            && (access == AccessMode::FullAccess
                || (self.request.kind == Kind::Codex
                    && access == AccessMode::AutoAcceptEdits
                    && file_change)
                || (self.request.kind == Kind::Codex
                    && permissions
                    && matches!(access, AccessMode::Auto | AccessMode::AutoAcceptEdits)))
        {
            return self.respond(
                Control {
                    id,
                    decision: Decision::Allow,
                },
                stdin,
            );
        }
        events.push(Event::Approval(Approval {
            id,
            title,
            detail,
            can_remember: self.request.kind != Kind::Codex && !file_change && !ask,
            tool,
        }));
        Ok(())
    }
    fn respond(&mut self, control: Control, stdin: &mut impl Write) -> Result<(), String> {
        if matches!(control.decision, Decision::KeepOpen) {
            if let Some((_, _, _, question)) = self.questions.get_mut(&control.id) {
                question.auto_resolve_at = None;
            }
            return Ok(());
        }
        if let Some((wire, kind, request, mut question)) = self.questions.remove(&control.id) {
            if self.visible_question.as_deref() == Some(control.id.as_str()) {
                self.visible_question = None;
            }
            if kind == "codex-async" {
                if let Decision::Answer(answers) = &control.decision {
                    let text = question
                        .items
                        .iter()
                        .zip(answers)
                        .filter(|(_, a)| !a.is_empty())
                        .map(|(q, a)| format!("{}\n{}", q.prompt, a.join("\n")))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    if !text.is_empty() && request["turnId"].as_str() == self.active_turn.as_deref()
                    {
                        question.auto_resolve_at = None;
                        let id = format!("native-answer-{}-{}", question.id, crate::time::now_ms());
                        write_json(
                            stdin,
                            json!({"id":id,"method":"turn/steer","params":{"threadId":request["threadId"],"expectedTurnId":request["turnId"],"input":[{"type":"text","text":text}]}}),
                        )?;
                        self.async_answers
                            .insert(id, (wire, kind, request, question));
                    }
                }
                return Ok(());
            }
            return questions::respond(stdin, wire, &kind, &request, &question, control.decision);
        }
        let Some((wire, kind, request)) = self.approvals.remove(&control.id) else {
            return Ok(());
        };
        let allow = matches!(control.decision, Decision::Allow | Decision::Session);
        let session = matches!(control.decision, Decision::Session);
        if kind == "acp" {
            // A review's post is approved once, never for the session.
            let decision = match control.decision {
                Decision::Session if self.request.settings.review.is_some() => Decision::Allow,
                other => other,
            };
            return acp::respond(stdin, wire, &request, decision);
        }
        if kind == "codex-mcp" {
            return write_json(
                stdin,
                json!({"id":wire,"result":{"action":if allow {"accept"} else {"decline"},"content":if allow {request["nativeContent"].clone()} else {Value::Null},"_meta":null}}),
            );
        }
        if kind == "pi" {
            return write_json(
                stdin,
                json!({"type":"extension_ui_response","id":wire,"confirmed":allow,"cancelled":!allow}),
            );
        }
        if kind == "claude" {
            let policy_denied = self.review_verdict(&kind, &request) == Some(Verdict::Deny);
            let mut response = if allow {
                json!({"behavior":"allow","updatedInput":request["input"]})
            } else if policy_denied {
                json!({"behavior":"deny","message":"Not allowed for a pull request review"})
            } else {
                json!({"behavior":"deny","message":"Denied by the user"})
            };
            // A review never installs a CLI-side rule: every post must come back here.
            if session && allow && self.request.settings.review.is_none() {
                if let Some(suggestions) = request.get("permission_suggestions") {
                    response["updatedPermissions"] = suggestions.clone();
                }
            }
            write_json(
                stdin,
                json!({"type":"control_response","response":{"subtype":"success","request_id":wire,"response":response}}),
            )
        } else {
            // A review's post is approved once, never for the session.
            let session = session && self.request.settings.review.is_none();
            let result = if kind.contains("permissions") {
                json!({"permissions": if allow { request["permissions"].clone() } else { json!({}) }, "scope":if self.request.settings.effective_access() == AccessMode::FullAccess { "session" } else { "turn" }})
            } else if kind == "execCommandApproval" || kind == "applyPatchApproval" {
                json!({"decision":if !allow { "denied" } else if session { "approved_for_session" } else { "approved" }})
            } else {
                json!({"decision":if !allow { "decline" } else { "accept" }})
            };
            write_json(stdin, json!({"id":wire,"result":result}))
        }
    }
    fn codex_tool(&mut self, item: &Value, completed: bool, events: &mut Vec<Event>) {
        let name = item["type"].as_str().unwrap_or("");
        if name == "contextCompaction" {
            self.compaction(
                item["id"].as_str(),
                if completed {
                    ToolState::Completed
                } else {
                    ToolState::Running
                },
                events,
            );
            return;
        }
        if !matches!(
            name,
            "commandExecution" | "fileChange" | "mcpToolCall" | "webSearch" | "dynamicToolCall"
        ) {
            return;
        }
        let id = item["id"].as_str().unwrap_or("");
        let tool = self
            .tools
            .entry(id.into())
            .or_insert_with(|| ToolCall::new(id, name));
        tool.name = item["tool"].as_str().unwrap_or(name).to_owned();
        if name == "mcpToolCall" {
            tool.name = format!("{}:{}", item["server"].as_str().unwrap_or("mcp"), tool.name);
        }
        let previous_input = tool.input.clone();
        tool.input = item["command"]
            .as_str()
            .or_else(|| item["query"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                serde_json::to_string_pretty(
                    item.get("arguments")
                        .or_else(|| item.get("changes"))
                        .unwrap_or(&Value::Null),
                )
                .unwrap_or_default()
            });
        if name == "commandExecution" {
            // Keep argv and parsed provider metadata intact. Presentation unwraps
            // recognized launchers without changing the recorded execution.
            let previous = serde_json::from_str::<Value>(&previous_input).ok();
            let mut input = previous
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            for field in [
                "command",
                "commandActions",
                "parsed_cmd",
                "parsedCmd",
                "cwd",
                "exitCode",
                "durationMs",
            ] {
                if let Some(value) = item.get(field) {
                    input[field] = value.clone();
                }
            }
            tool.input = input.to_string();
        }
        if let Some(changes) = item["changes"].as_array().filter(|_| name == "fileChange") {
            tool.input = json!({
                "path": changes.first().and_then(|c| c["path"].as_str()).unwrap_or("Files"),
                "paths": changes.iter().filter_map(|c| c["path"].as_str()).collect::<Vec<_>>(),
            })
            .to_string();
            tool.output = changes
                .iter()
                .map(|c| {
                    format!(
                        "{}\n{}",
                        c["path"].as_str().unwrap_or(""),
                        c["diff"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(first) = changes.first() {
                let diff = first["diff"].as_str().unwrap_or("");
                let added = first["kind"]["type"] == "add";
                let built =
                    crate::tool::edit_preview::EditPreview::from_patch(diff).or_else(|| {
                        added
                            .then(|| crate::tool::edit_preview::EditPreview::from_texts(None, diff))
                    });
                if let Some(built) = built {
                    tool.display.get_or_insert_with(|| json!({}))
                        [crate::tool::edit_preview::DISPLAY_KEY] = json!(built);
                }
            }
        }
        if let Some(output) = item["aggregatedOutput"]
            .as_str()
            .or_else(|| item["output"].as_str())
        {
            tool.output = output.to_owned();
        } else if let Some(result) = item.get("result") {
            tool.output = serde_json::to_string_pretty(result).unwrap_or_default();
        }
        tool.bound_output();
        tool.state = match item["status"].as_str().unwrap_or(if completed {
            "completed"
        } else {
            "inProgress"
        }) {
            "completed" => ToolState::Completed,
            "failed" | "declined" => ToolState::Failed,
            "cancelled" | "canceled" | "interrupted" | "aborted" => ToolState::Cancelled,
            _ => ToolState::Running,
        };
        if item["exitCode"].as_i64().is_some_and(|code| code != 0) {
            tool.state = ToolState::Failed;
        }
        events.push(Event::Tool(tool.clone()));
    }
}

/// Stream at bounded memory cost. `emit` implements backpressure and must observe
/// cancellation while waiting for the viewport. Errors are returned after reaping.
pub fn run(
    request: Request,
    cancel: Arc<AtomicBool>,
    emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<(), String> {
    run_with_controls(request, cancel, None, emit)
}
pub fn run_with_controls(
    request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<(), String> {
    run_process(request, cancel, controls, false, emit).map(|_| ())
}

/// A bounded, separate conversation for titles and other background text work.
/// Never resume the user's conversation or inherit its app capabilities/tools.
pub fn run_text(mut request: Request, cancel: Arc<AtomicBool>) -> Result<String, String> {
    request.provider_id = None;
    request.settings = Settings {
        account: request.settings.account.clone(),
        text_only: true,
        access: AccessMode::ReadOnly,
        effort: (request.kind == Kind::Codex).then(|| "low".into()),
        ..Default::default()
    };
    let mut output = String::new();
    run(request, cancel, |events| {
        for event in events {
            match event {
                Event::Text(text) if output.len() + text.len() <= 64 * 1024 => {
                    output.push_str(&text)
                }
                Event::Text(_) => return Err("Background text response exceeded its limit".into()),
                Event::Tool(tool) if tool.is_compaction() => {}
                Event::Tool(_) | Event::Approval(_) | Event::Question(_) => {
                    return Err(
                        "Background text request attempted to use tools or request input".into(),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok(output)
}

pub(crate) fn catalog_for(kind: Kind, account: crate::accounts::Profile) -> Result<Value, String> {
    let binary = resolve(kind).ok_or_else(|| kind.unavailable_hint())?;
    let cwd = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    run_process(
        Request {
            kind,
            binary,
            cwd,
            prompt: String::new(),
            provider_id: None,
            model: None,
            settings: Settings {
                account,
                access: AccessMode::ReadOnly,
                ..Default::default()
            },
        },
        Arc::new(AtomicBool::new(false)),
        None,
        true,
        |_| Ok(()),
    )?
    .ok_or_else(|| format!("{} returned no model catalog", kind.label()))
}

/// Waits for the turn's workspace snapshot before an OpenCode turn starts: its
/// server takes the prompt with the session, so nothing can be held there.
/// Other transports hold only their prompt step (`prompt_gate`).
fn baseline_recorded(request: &Request, cancel: &AtomicBool) -> Result<(), String> {
    match &request.settings.baseline {
        Some(baseline) if !baseline.wait(cancel) => Err(ProviderError::Stopped.into()),
        _ => Ok(()),
    }
}

fn kiro_auth_error(detail: &str) -> String {
    signed_out::message(Kind::Kiro, &format!("{detail} {}", acp::KIRO_AUTH_HELP))
}

/// Kiro's sign-in failure from stderr. An exited process gets a moment for
/// its last stderr lines to be read.
fn kiro_stderr_failure(wire: &mut parked::Wire) -> Option<String> {
    let deadline = Instant::now() + Duration::from_millis(300);
    loop {
        if let Some(detail) = wire.auth.lock().unwrap().as_deref() {
            return Some(kiro_auth_error(detail));
        }
        if wire.stderr_closed()
            || Instant::now() >= deadline
            || !matches!(wire.child.try_wait(), Ok(Some(_)))
        {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether the child exits soon after its output ended.
fn exited(child: &mut std::process::Child) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => return false,
        }
    }
}

fn run_process(
    request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    probe: bool,
    emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    run_operation(request, cancel, controls, probe, false, false, emit)
}

fn run_operation(
    request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    probe: bool,
    compact: bool,
    command_probe: bool,
    mut emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ProviderError::Stopped.into());
    }
    if api::ProviderRegistry::builtin()
        .get(request.kind)
        .transport()
        == api::Transport::Http
    {
        baseline_recorded(&request, &cancel)?;
        return opencode::run(request, cancel, controls, probe, compact, emit);
    }
    let controls = controls.as_ref();
    let mut fallback = false;
    loop {
        let mut relaunch = false;
        let result = launch(
            request.clone(),
            &cancel,
            controls,
            (probe, compact, command_probe),
            fallback,
            &mut relaunch,
            &mut emit,
        );
        if !relaunch {
            return result;
        }
        // Stop can arrive while waiting for the first attempt to exit.
        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Stopped.into());
        }
        fallback = true;
    }
}

/// A family's launch arguments. `probe` asks Pi for a throwaway session,
/// `fallback` is Kiro's retry without the V3 engine flag, and
/// `claude_new_session` is the id a new Claude conversation starts with.
pub(super) fn configure_by_family(
    command: &mut Command,
    request: &Request,
    probe: bool,
    fallback: bool,
    claude_new_session: Option<&str>,
) -> Result<(), String> {
    match request.kind.family() {
        Family::Codex => codex::configure(command, request)?,
        Family::Claude => claude::configure(command, request, claude_new_session)?,
        Family::Pi => pi::configure(command, request, probe),
        Family::Acp | Family::OpenCode => acp::configure(command, request, fallback)?,
    }
    Ok(())
}

/// One provider process. `relaunch` is set, instead of reporting a failure,
/// when Kiro exits before answering `initialize` and `fallback` was not tried.
fn launch(
    request: Request,
    cancel: &Arc<AtomicBool>,
    controls: Option<&mpsc::Receiver<Control>>,
    (probe, compact, command_probe): (bool, bool, bool),
    fallback: bool,
    relaunch: &mut bool,
    emit: &mut impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    let provider = api::ProviderRegistry::builtin().get(request.kind);
    let claude_new_session = provider.new_session_id(&request);
    #[cfg(unix)]
    if tracing::enabled!(tracing::Level::DEBUG) {
        unsafe {
            let mut signals = std::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut signals);
            tracing::debug!(
                sigchld_blocked = libc::sigismember(&signals, libc::SIGCHLD) == 1,
                "provider launch"
            );
        }
    }
    let mut command = Command::new(&request.binary);
    crate::scm::shell::apply_gui_env(&mut command);
    command.envs(&request.settings.environment);
    request.settings.account.apply(request.kind, &mut command)?;
    provider.configure(
        &mut command,
        &request,
        api::Attempt {
            probe: probe || command_probe,
            fallback,
        },
        claude_new_session.as_deref(),
    )?;
    command
        .current_dir(provider.working_dir(&request))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    if cancel.load(Ordering::Relaxed) {
        return Err(ProviderError::Stopped.into());
    }
    let eligible = !probe && !command_probe && !request.settings.text_only;
    let cached = eligible
        .then(|| parked::take(&parked::key(&request)))
        .flatten();
    let (mut wire, mut protocol, warm) = if let Some(cached) = cached {
        let mut protocol = Protocol::new(request);
        protocol.acp = cached.protocol.acp;
        protocol.pi = cached.protocol.pi;
        protocol.previous_turn = cached.protocol.active_turn;
        protocol.activities.registry = cached.protocol.activities.registry;
        (cached.wire, protocol, true)
    } else {
        if eligible {
            // A new process supersedes older warm launch configurations and
            // any cancelled process still draining for this provider binding.
            parked::discard_session(&request);
        }
        // The fallback relaunch is part of the start already counted.
        let wire = parked::Wire::spawn(&mut command, &request, eligible && !fallback)?;
        let mut protocol = Protocol::new(request);
        protocol.access = protocol
            .request
            .settings
            .mount
            .as_ref()
            .filter(|_| {
                crate::tool_servers::delivery(
                    protocol.request.kind,
                    protocol.request.provider_id.as_deref(),
                ) != crate::tool_servers::Delivery::Environment
            })
            .and_then(crate::tool_servers::Access::open);
        (wire, protocol, false)
    };
    tracing::debug!(target: "gespan::send", stage = "provider.spawned", warm);
    // Fresh, warm or `--resume`d: rebuild which call dispatched which agent.
    protocol.seed_agent_runs();
    protocol.seed_claude_tasks();
    let rx = &wire.rx;
    let tail = &wire.tail;
    let auth = &wire.auth;
    let mut stdin = &mut wire.stdin;
    let mut pending = Vec::new();
    if let Some(id) = claude_new_session {
        protocol.request.provider_id = Some(id.clone());
        pending.push(Event::SessionId(id));
    }
    let mut stop_confirmed = false;
    let mut held = false;
    let result = (|| {
        protocol.probe = probe;
        protocol.compact = compact;
        protocol.command_probe = command_probe;
        if warm {
            if protocol.request.kind.is_acp() {
                protocol.acp_resume_warm(&mut stdin)?;
            } else if protocol.request.kind.is_pi() {
                protocol.pi_start(&mut stdin)?;
            } else if protocol.request.kind == Kind::Codex {
                protocol.initialized = true;
                protocol.frame(json!({"id":1,"result":{}}), &mut stdin)?;
            } else {
                protocol.follow_up_ready = true;
                protocol.frame(
                    json!({"type":"control_response","response":{"request_id":"native-init"}}),
                    &mut stdin,
                )?;
            }
        } else if protocol.request.kind.is_acp() {
            protocol.acp_start(&mut stdin)?;
        } else if protocol.request.kind.is_pi() {
            protocol.pi_start(&mut stdin)?;
        } else if protocol.request.kind == Kind::Codex {
            write_json(
                &mut stdin,
                json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"gespan_native","title":"Gespan","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}}),
            )?;
        } else {
            write_json(
                &mut stdin,
                json!({"type":"control_request","request_id":"native-init","request":{"subtype":"initialize"}}),
            )?;
        }
        let mut text_seen = false;
        let mut batch = batch::Batch::new(Instant::now());
        let started = Instant::now();
        let mut ready_emitted = false;
        let mut interrupted = false;
        let mut gate = hold::Gate::default();
        loop {
            if cancel.load(Ordering::Relaxed) && !interrupted {
                // The provider never got the prompt: there is no turn to stop.
                if protocol.prompt_held() {
                    return Err(ProviderError::Stopped.into());
                }
                interrupted = true;
                if !protocol.interrupt(&mut stdin, Interrupt::Stop)? {
                    return Err(ProviderError::Stopped.into());
                }
                if protocol.request.kind == Kind::Claude {
                    return Err(ProviderError::Stopped.into());
                }
            }
            if !interrupted {
                protocol.release_prompt(&mut stdin)?;
                // Steers and other controls wait with the prompt for the
                // turn's baseline, which nothing may get ahead of. A prompt
                // still held (the baseline may have finished just now) goes
                // out first, on the next pass.
                let gated = protocol.baseline_pending() || protocol.prompt_held();
                if let Some(controls) = controls.filter(|_| !gated) {
                    while let Ok(control) = controls.try_recv() {
                        match control.decision {
                            Decision::Steer(message) => {
                                let id = message.id.clone();
                                match protocol.steer(message, &mut stdin) {
                                    Ok(events) => pending.extend(events),
                                    Err(error) => pending.push(Event::FollowUpFailed { id, error }),
                                }
                            }
                            Decision::Hold => {
                                gate.request();
                                protocol.holding = true;
                            }
                            Decision::Release => {
                                gate.release();
                                protocol.holding = false;
                            }
                            _ => protocol.respond(control, &mut stdin)?,
                        }
                    }
                }
                pending.extend(protocol.expire_steering());
                let tick = protocol.lifecycle_tick(started, &mut stdin)?;
                for event in &tick {
                    gate.observe(event);
                }
                pending.extend(tick);
                if protocol.ready() && !ready_emitted {
                    ready_emitted = true;
                    pending.push(Event::Ready);
                }
                let awaiting = !protocol.approvals.is_empty() || !protocol.questions.is_empty();
                let decision = (awaiting || protocol.prompt_delivered())
                    .then(|| gate.decide(awaiting))
                    .flatten();
                match decision {
                    Some(hold::Action::Interrupt) => {
                        interrupted = true;
                        held = true;
                        pending.push(Event::HoldSettled(HoldPoint::Interrupted));
                        if !protocol.interrupt(&mut stdin, Interrupt::Hold)? {
                            return Err(ProviderError::Stopped.into());
                        }
                    }
                    Some(hold::Action::Settle(point)) => pending.push(Event::HoldSettled(point)),
                    None => {}
                }
            }
            // Kiro can wait forever at startup once signed out; its stderr
            // says why. During a prompt, tool and MCP stderr can look the
            // same, so the line only explains a failure that happens anyway.
            if !interrupted && protocol.acp_starting() {
                if let Some(detail) = auth.lock().unwrap().as_deref() {
                    protocol.signed_out = true;
                    return Err(kiro_auth_error(detail));
                }
            }
            if interrupted && protocol.request.kind.is_pi() {
                let _ = protocol.pi_tick(&mut stdin);
            }
            if interrupted && protocol.done {
                stop_confirmed = true;
                return Err(ProviderError::Stopped.into());
            }
            if protocol.request.settings.text_only && started.elapsed() >= Duration::from_secs(45) {
                return Err("Background text request timed out".into());
            }
            // A held prompt goes out within a couple of milliseconds of the
            // baseline being recorded.
            match rx.recv_timeout(if protocol.prompt_held() {
                HELD_POLL
            } else {
                TICK
            }) {
                Ok(Ok(line)) => {
                    let value: Value = match serde_json::from_slice(&line) {
                        Ok(value) => value,
                        // Some CLIs print startup banners before their NDJSON stream.
                        Err(_)
                            if line
                                .iter()
                                .copied()
                                .find(|b| !b.is_ascii_whitespace())
                                .is_none_or(|b| b != b'{') =>
                        {
                            continue;
                        }
                        Err(e) => return Err(format!("Invalid provider JSON: {e}")),
                    };
                    if tracing::enabled!(tracing::Level::DEBUG) {
                        let method = value["method"].as_str().unwrap_or("");
                        let item = &value["params"]["item"];
                        if !method.is_empty()
                            && !method.ends_with("Delta")
                            && !method.ends_with("/delta")
                        {
                            tracing::debug!(
                                method,
                                item_type = %item["type"],
                                status = %item["status"],
                                id = %item["id"],
                                output_bytes =
                                    item["aggregatedOutput"].as_str().map_or(0, str::len),
                                keys = ?value["params"]
                                    .as_object()
                                    .map(|o| o.keys().collect::<Vec<_>>()),
                                legacy = %value["params"]["msg"]["type"],
                                "provider frame"
                            );
                        }
                    }
                    if interrupted {
                        let reply_id = value["id"].as_str();
                        let turn_id = value["params"]["turnId"]
                            .as_str()
                            .or_else(|| value["params"]["turn"]["id"].as_str());
                        let terminal = matches!(
                            value["method"].as_str(),
                            Some("turn/completed" | "turn/aborted")
                        ) && (turn_id.is_some()
                            && turn_id == protocol.active_turn.as_deref()
                            || turn_id.is_none() && protocol.previous_turn.is_none());
                        let codex_ack = protocol.request.kind == Kind::Codex
                            && (reply_id.is_some() && reply_id == protocol.interrupt_id.as_deref()
                                || terminal);
                        let claude_end =
                            protocol.request.kind == Kind::Claude && claude_turn_result(&value);
                        if codex_ack || claude_end {
                            stop_confirmed = value.get("error").is_none_or(Value::is_null);
                            if terminal || claude_end {
                                protocol.done = true;
                            }
                            return Err(ProviderError::Stopped.into());
                        }
                    }
                    let events = match protocol.frame(value, &mut stdin) {
                        Ok(events) => events,
                        Err(_) if interrupted => return Err(ProviderError::Stopped.into()),
                        Err(error) => return Err(error),
                    };
                    for event in events {
                        gate.observe(&event);
                        if controls.is_none() || interrupted {
                            if let Event::Approval(approval) = &event {
                                protocol.respond(
                                    Control {
                                        id: approval.id.clone(),
                                        decision: Decision::Deny,
                                    },
                                    &mut stdin,
                                )?;
                            }
                            if let Event::Question(question) = &event {
                                protocol.respond(
                                    Control {
                                        id: question.id.clone(),
                                        decision: Decision::Deny,
                                    },
                                    &mut stdin,
                                )?;
                            }
                        }
                        if interrupted {
                            continue;
                        }
                        batch.add(&event);
                        if let Event::SessionId(id) = &event {
                            protocol.request.provider_id = Some(id.clone());
                        }
                        if !text_seen && matches!(event, Event::Text(_)) {
                            text_seen = true;
                            tracing::debug!(target: "gespan::send", stage = "provider.first_text");
                        }
                        pending.push(event);
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if interrupted {
                        return Err(ProviderError::Stopped.into());
                    }
                    return Err(format!(
                        "Provider exited before completing the turn. {}",
                        String::from_utf8_lossy(&tail.lock().unwrap())
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if !pending.is_empty() && batch.due(Instant::now(), protocol.done) {
                emit(std::mem::take(&mut pending))?;
                batch.emitted(Instant::now());
            }
            // A visual publication or ticket call may arrive while the provider
            // is waiting for its tool result and therefore emits no stream events.
            if protocol.request.settings.mount.is_some() {
                emit(Vec::new())?;
            }
            if protocol.done {
                if interrupted {
                    stop_confirmed = true;
                    return Err(ProviderError::Stopped.into());
                }
                return Ok(protocol.catalog_result.take());
            }
            if command_probe && started.elapsed() > Duration::from_secs(45) {
                return Err("Command discovery timed out".into());
            }
            let discovery_timeout = protocol.request.kind.spec().discovery_timeout;
            if probe
                && discovery_timeout
                    .is_some_and(|seconds| started.elapsed() > Duration::from_secs(seconds))
            {
                return Err("Model discovery timed out".into());
            }
        }
    })();
    // A Hold interrupt ends the turn like Stop, but the session is held, not stopped.
    let result = match result {
        Err(error) if held && ProviderError::is_stopped(&error) => Err(ProviderError::Held.into()),
        result => result,
    };
    let stopped = held || cancel.load(Ordering::Relaxed);
    if provider.retry_policy() == api::RetryPolicy::FallbackOnce && result.is_err() && !stopped {
        // A CLI without the V3 engine rejects the flag and exits; a live
        // process that timed out or answered with an error is not retried.
        if !fallback && !warm && protocol.acp_initializing() && exited(&mut wire.child) {
            // A signed-out CLI exits too; the plain launch would only repeat
            // it, so the turn fails below with the login help instead.
            if kiro_stderr_failure(&mut wire).is_none() {
                *relaunch = true;
                return result;
            }
        }
    }
    let result = match result {
        Err(error) if protocol.request.kind == Kind::Kiro && !stopped => {
            match kiro_stderr_failure(&mut wire) {
                Some(failure) => {
                    protocol.signed_out = true;
                    Err(failure)
                }
                None => Err(error),
            }
        }
        result => result,
    };
    // Cancellation can arrive while publishing an event batch. Its callback
    // then returns Stopped before the next receive-loop iteration can interrupt.
    if cancel.load(Ordering::Relaxed) && protocol.interrupt_id.is_none() {
        let _ = protocol.interrupt(&mut wire.stdin, Interrupt::Stop);
    }
    if cancel.load(Ordering::Relaxed) && protocol.done {
        stop_confirmed = true;
    }
    if let (Err(error), false) = (&result, cancel.load(Ordering::Relaxed) || held) {
        // Conversation turns only: not probes or background text work.
        if eligible {
            crate::diagnostics::metrics::failure(
                "agent.turn_error",
                crate::diagnostics::metrics::ErrorKind::of_message(error),
            );
        }
        if let Some(limit) = protocol.claude.usage_limit.take() {
            pending.push(Event::UsageLimited(limit));
        }
        if protocol.signed_out {
            pending.push(Event::SignedOut);
        }
    }
    if !pending.is_empty() {
        let _ = emit(std::mem::take(&mut pending));
    }
    if eligible
        && cancel.load(Ordering::Relaxed)
        && !protocol.done
        && protocol.request.kind != Kind::Antigravity
        && protocol.interrupt_id.is_some()
    {
        parked::settle_cancelled(wire, protocol);
    } else if eligible
        && ((result.is_ok() && !cancel.load(Ordering::Relaxed))
            || (stop_confirmed && protocol.request.kind != Kind::Antigravity))
    {
        if held {
            parked::put_held(wire, protocol);
        } else {
            parked::put(wire, protocol);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // The agents here are shell scripts.
    #[cfg(unix)]
    mod counters {
        use super::*;
        use crate::diagnostics::metrics::{capture, ErrorKind};

        fn agent(dir: &Path, kind: Kind, script: &str) -> Request {
            let binary = dir.join("agent");
            std::fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
            crate::test_exec::make_executable(&binary, 0o755);
            Request {
                kind,
                binary,
                cwd: dir.to_owned(),
                prompt: "Hello".into(),
                provider_id: None,
                model: None,
                settings: Default::default(),
            }
        }
        fn counted(
            counts: Vec<crate::diagnostics::metrics::Count>,
        ) -> Vec<(String, Option<ErrorKind>)> {
            counts.into_iter().map(|c| (c.name, c.kind)).collect()
        }

        #[test]
        fn a_resumed_claude_turn_keeps_its_reply_after_a_replayed_task_result() {
            // Claude Code answers a replayed background-task notification with
            // an empty result before the real turn starts.
            let dir = tempfile::tempdir().unwrap();
            let request = agent(
                dir.path(),
                Kind::Claude,
                r#"read init
echo '{"type":"control_response","response":{"subtype":"success","request_id":"native-init"}}'
read prompt
echo '{"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","origin":{"kind":"task-notification"}}'
echo '{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"The real reply"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The real reply"}'
cat >/dev/null"#,
            );
            let mut events = Vec::new();
            run(request, Arc::new(AtomicBool::new(false)), |batch| {
                events.extend(batch);
                Ok(())
            })
            .unwrap();
            let text: String = events
                .iter()
                .filter_map(|e| match e {
                    Event::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(text.contains("The real reply"), "{events:?}");
            assert_eq!(events.iter().filter(|e| **e == Event::Done).count(), 1);
        }
        #[test]
        fn a_failed_turn_counts_one_turn_error_after_a_start() {
            let dir = tempfile::tempdir().unwrap();
            let request = agent(dir.path(), Kind::Claude, "exit 3");
            let (result, counts) =
                capture(|| run(request, Arc::new(AtomicBool::new(false)), |_| Ok(())));
            assert!(result.is_err());
            let counts = counted(counts);
            assert!(
                counts.contains(&("agent.start.claude".into(), None)),
                "{counts:?}"
            );
            assert_eq!(
                counts
                    .iter()
                    .filter(|(name, _)| name == "agent.turn_error")
                    .count(),
                1,
                "{counts:?}"
            );
        }

        #[test]
        fn a_cancelled_turn_is_not_an_error() {
            let dir = tempfile::tempdir().unwrap();
            let request = agent(
                dir.path(),
                Kind::Claude,
                "touch \"$0.started\"; exec sleep 5",
            );
            let marker = request.binary.with_extension("started");
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let stopper = std::thread::spawn(move || {
                while !marker.exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                stop.store(true, Ordering::Relaxed);
            });
            let (result, counts) = capture(|| run(request, cancel, |_| Ok(())));
            stopper.join().unwrap();
            assert!(result.is_err());
            let counts = counted(counts);
            assert!(
                counts.contains(&("agent.start.claude".into(), None)),
                "{counts:?}"
            );
            assert!(
                !counts.iter().any(|(name, _)| name == "agent.turn_error"),
                "{counts:?}"
            );
        }

        #[test]
        fn model_catalog_probes_are_not_agent_starts() {
            let dir = tempfile::tempdir().unwrap();
            let request = agent(dir.path(), Kind::Cursor, "exit 3");
            let (result, counts) = capture(|| {
                run_process(
                    request,
                    Arc::new(AtomicBool::new(false)),
                    None,
                    true,
                    |_| Ok(()),
                )
            });
            assert!(result.is_err());
            assert!(counts.is_empty(), "{:?}", counted(counts));
        }

        #[test]
        fn background_text_requests_are_not_starts_or_turn_errors() {
            let dir = tempfile::tempdir().unwrap();
            let request = agent(dir.path(), Kind::Claude, "exit 3");
            let (result, counts) = capture(|| run_text(request, Arc::new(AtomicBool::new(false))));
            assert!(result.is_err());
            assert!(counts.is_empty(), "{:?}", counted(counts));
        }

        #[test]
        fn a_stopped_opencode_start_is_not_a_failure() {
            let dir = tempfile::tempdir().unwrap();
            let request = agent(
                dir.path(),
                Kind::OpenCode,
                "touch \"$0.started\"; exec sleep 5",
            );
            let marker = request.binary.with_extension("started");
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let stopper = std::thread::spawn(move || {
                while !marker.exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                stop.store(true, Ordering::Relaxed);
            });
            let (result, counts) = capture(|| run(request, cancel, |_| Ok(())));
            stopper.join().unwrap();
            assert_eq!(result, Err("Stopped".into()));
            assert!(counts.is_empty(), "{:?}", counted(counts));
        }

        #[test]
        fn a_missing_opencode_counts_a_start_failure() {
            let dir = tempfile::tempdir().unwrap();
            let mut request = agent(dir.path(), Kind::OpenCode, "exit 0");
            request.binary = dir.path().join("missing-opencode");
            let (result, counts) =
                capture(|| run(request, Arc::new(AtomicBool::new(false)), |_| Ok(())));
            assert!(result.is_err());
            assert_eq!(
                counted(counts),
                [("agent.start.opencode".into(), Some(ErrorKind::NotFound))]
            );
        }
    }

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
    #[test]
    fn codex_file_changes_retain_every_edited_path() {
        let mut protocol = protocol(Kind::Codex);
        let mut events = Vec::new();
        protocol.codex_tool(&json!({
            "type":"fileChange", "id":"edit", "status":"completed",
            "changes":[{"path":"/repo/a.rs", "diff":"first"}, {"path":"/repo/b.rs", "diff":"second"}]
        }), true, &mut events);
        let Event::Tool(tool) = &events[0] else {
            panic!("missing edit tool")
        };
        let input: Value = serde_json::from_str(&tool.input).unwrap();
        assert_eq!(tool.state, ToolState::Completed);
        assert_eq!(input["path"], "/repo/a.rs");
        assert_eq!(input["paths"], json!(["/repo/a.rs", "/repo/b.rs"]));
    }
    #[test]
    fn a_codex_file_change_previews_its_first_patch_or_new_file() {
        let mut protocol = protocol(Kind::Codex);
        let mut events = Vec::new();
        protocol.codex_tool(&json!({
            "type":"fileChange", "id":"edit", "status":"inProgress",
            "changes":[
                {"path":"/repo/a.rs", "kind":{"type":"update"}, "diff":"@@ -3,2 +3,2 @@\n fn a() {}\n-let x = 1;\n+let x = 2;\n"},
                {"path":"/repo/b.rs", "kind":{"type":"add"}, "diff":"new"}
            ]
        }), false, &mut events);
        let preview = protocol.tools["edit"].edit_preview().unwrap();
        assert_eq!((preview.additions, preview.deletions), (1, 1));
        assert_eq!(preview.lines[0].number, Some(3));
        assert_eq!(
            preview.lines[1].kind,
            crate::tool::edit_preview::LineKind::Del
        );
        assert_eq!(preview.lines[2].text, "let x = 2;");
        protocol.codex_tool(
            &json!({
                "type":"fileChange", "id":"add", "status":"completed",
                "changes":[{"path":"/repo/new.rs", "kind":{"type":"add"}, "diff":"fn main() {}\n"}]
            }),
            true,
            &mut events,
        );
        let added = protocol.tools["add"].edit_preview().unwrap();
        assert_eq!((added.additions, added.deletions), (1, 0));
        assert_eq!(added.lines[0].text, "fn main() {}");
        protocol.codex_tool(
            &json!({
                "type":"fileChange", "id":"odd", "status":"completed",
                "changes":[{"path":"/repo/c.rs", "kind":{"type":"update"}, "diff":"not a patch"}]
            }),
            true,
            &mut events,
        );
        assert_eq!(protocol.tools["odd"].edit_preview(), None);
    }
    #[test]
    fn operator_thread_access_keeps_the_codex_app_bridge_reachable_on_later_turns() {
        for plan in [false, true] {
            let mut p = protocol(Kind::Codex);
            p.request.settings.operator_enabled = true;
            p.request.settings.plan = plan;
            assert!(!p.request.settings.operator);
            assert_eq!(p.codex_access().2["networkAccess"], true);
            let mut out = Vec::new();
            p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
            p.frame(json!({"id":2,"result":{"thread":{"id":"saved"}}}), &mut out)
                .unwrap();
            let started: Value = String::from_utf8_lossy(&out)
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|message| message["method"] == "turn/start")
                .unwrap();
            assert_eq!(started["params"]["sandboxPolicy"]["networkAccess"], true);
            if plan {
                assert_eq!(started["params"]["sandboxPolicy"]["type"], "readOnly");
            }
        }
    }
    fn codex_review_protocol(results: crate::review_policy::ReviewResults) -> Protocol {
        let mut p = review_protocol(results, AccessMode::ReadOnly);
        p.request.kind = Kind::Codex;
        p
    }
    fn codex_request(p: &mut Protocol, method: &str, params: Value) -> (Vec<Event>, Value) {
        let mut out = Vec::new();
        let events = p
            .frame(json!({"id":9,"method":method,"params":params}), &mut out)
            .unwrap();
        let response = String::from_utf8_lossy(&out)
            .lines()
            .next()
            .map_or(Value::Null, |l| serde_json::from_str(l).unwrap());
        (events, response)
    }
    fn codex_starts(p: &mut Protocol) -> Vec<Value> {
        let mut out = Vec::new();
        p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
        p.frame(json!({"id":2,"result":{"thread":{"id":"t"}}}), &mut out)
            .unwrap();
        String::from_utf8_lossy(&out)
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|m| m["method"] == "thread/start" || m["method"] == "turn/start")
            .collect()
    }
    #[test]
    fn codex_review_runs_ask_gespan_with_network() {
        use crate::review_policy::ReviewResults;
        let mut p = codex_review_protocol(ReviewResults::Draft);
        let starts = codex_starts(&mut p);
        assert_eq!(starts.len(), 2);
        for m in &starts {
            assert_eq!(m["params"]["approvalPolicy"], "untrusted", "{m}");
            assert_eq!(m["params"]["sandboxPolicy"]["type"], "readOnly", "{m}");
            assert_eq!(m["params"]["sandboxPolicy"]["networkAccess"], true, "{m}");
        }
        assert_eq!(starts[0]["params"]["sandbox"], "read-only");
        // Without a review: exactly today's.
        let mut plain = protocol(Kind::Codex);
        plain.request.settings.access = AccessMode::ReadOnly;
        for m in codex_starts(&mut plain) {
            assert_eq!(m["params"]["approvalPolicy"], "never");
            assert!(m["params"]["sandboxPolicy"].get("networkAccess").is_none());
        }
    }
    #[test]
    fn codex_review_policy_decides_commands_in_both_wire_shapes() {
        use crate::review_policy::ReviewResults;
        let mut p = codex_review_protocol(ReviewResults::Draft);
        let (events, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["gh","pr","diff","7","-R","acme/app"]}),
        );
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(r["result"]["decision"], "approved");
        let (_, r) = codex_request(
            &mut p,
            "item/commandExecution/requestApproval",
            json!({"command":"gh pr view 7 -R acme/app"}),
        );
        assert_eq!(r["result"]["decision"], "accept");
        // A read that omits the target repo is not the triggering PR.
        let (_, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["gh","pr","diff","7"]}),
        );
        assert_eq!(r["result"]["decision"], "denied");
        let (_, r) = codex_request(&mut p, "execCommandApproval", json!({"command":["rm","x"]}));
        assert_eq!(r["result"]["decision"], "denied");
        let (_, r) = codex_request(
            &mut p,
            "item/commandExecution/requestApproval",
            json!({"command":"rm x"}),
        );
        assert_eq!(r["result"]["decision"], "decline");
        assert!(p.approvals.is_empty());
    }
    #[test]
    fn codex_review_draft_post_asks_the_user_and_never_remembers() {
        use crate::review_policy::ReviewResults;
        let mut p = codex_review_protocol(ReviewResults::Draft);
        let (events, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["gh","pr","review","7","-R","acme/app","--comment","-b","x"]}),
        );
        assert_eq!(r, Value::Null);
        let [Event::Approval(a)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert!(!a.can_remember);
        assert!(
            a.detail.contains("gh pr review 7 -R acme/app"),
            "{}",
            a.detail
        );
        // "Allow for this session" must still be a one-off approval.
        let mut out = Vec::new();
        p.respond(
            Control {
                id: "9".into(),
                decision: Decision::Session,
            },
            &mut out,
        )
        .unwrap();
        let r: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(r["result"]["decision"], "approved");
        // Post is allowed outright.
        let mut p = codex_review_protocol(ReviewResults::Post);
        let (events, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["gh","pr","review","7","-R","acme/app","--comment","-b","x"]}),
        );
        assert!(events.is_empty());
        assert_eq!(r["result"]["decision"], "approved");
    }
    #[test]
    fn codex_review_denies_file_changes_and_permission_requests() {
        use crate::review_policy::ReviewResults;
        for results in [ReviewResults::Draft, ReviewResults::Post] {
            let mut p = codex_review_protocol(results);
            for (method, key, denied) in [
                ("applyPatchApproval", "decision", "denied"),
                ("item/fileChange/requestApproval", "decision", "decline"),
                ("item/permissions/requestApproval", "scope", "turn"),
            ] {
                let (events, r) = codex_request(
                    &mut p,
                    method,
                    json!({"permissions":{"network":true},"reason":"edit"}),
                );
                assert!(events.is_empty(), "{method}: {events:?}");
                if key == "decision" {
                    assert_eq!(r["result"][key], denied, "{method}");
                } else {
                    assert_eq!(r["result"]["permissions"], json!({}), "{method}");
                }
            }
        }
    }
    #[test]
    fn codex_review_unwraps_the_shell_launcher_in_both_wire_shapes() {
        use crate::review_policy::ReviewResults;
        let mut p = codex_review_protocol(ReviewResults::Draft);
        let (events, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["/bin/zsh","-lc","gh pr diff 7 -R acme/app"]}),
        );
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(r["result"]["decision"], "approved");
        let (events, r) = codex_request(
            &mut p,
            "item/commandExecution/requestApproval",
            json!({"command":"/bin/zsh -lc 'gh pr diff 7 -R acme/app'"}),
        );
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(r["result"]["decision"], "accept");
        // A wrapped post still follows Results (Draft asks).
        let (events, _) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["/bin/zsh","-lc","gh pr review 7 -R acme/app --comment -b x"]}),
        );
        assert!(
            matches!(events.as_slice(), [Event::Approval(_)]),
            "{events:?}"
        );
        let (_, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["/bin/zsh","-lc","gh pr diff 7 -R acme/app; rm x"]}),
        );
        assert_eq!(r["result"]["decision"], "denied");
    }
    #[test]
    fn codex_review_denies_a_launcher_that_is_not_a_known_shell() {
        use crate::review_policy::ReviewResults;
        let mut p = codex_review_protocol(ReviewResults::Post);
        let (_, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["/abs/checkout/tools/zsh","-lc","gh pr diff 7 -R acme/app"]}),
        );
        assert_eq!(r["result"]["decision"], "denied");
        let (_, r) = codex_request(
            &mut p,
            "execCommandApproval",
            json!({"command":["/usr/bin/bash","-c","gh pr diff 7 -R acme/app"]}),
        );
        assert_eq!(r["result"]["decision"], "approved");
    }
    #[test]
    fn codex_review_access_ignores_the_access_mode() {
        use crate::review_policy::ReviewResults;
        for access in [
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::AutoAcceptEdits,
            AccessMode::Auto,
            AccessMode::FullAccess,
        ] {
            for plan in [false, true] {
                let mut p = codex_review_protocol(ReviewResults::Draft);
                p.request.settings.access = access;
                p.request.settings.plan = plan;
                let starts = codex_starts(&mut p);
                assert_eq!(starts.len(), 2, "{access:?}");
                for m in &starts {
                    let params = &m["params"];
                    assert_eq!(params["approvalPolicy"], "untrusted", "{access:?} {m}");
                    assert_eq!(params["approvalsReviewer"], "user", "{access:?} {m}");
                    assert_eq!(
                        params["sandboxPolicy"]["type"], "readOnly",
                        "{access:?} {m}"
                    );
                    assert_eq!(params["sandboxPolicy"]["networkAccess"], true, "{access:?}");
                }
                assert_eq!(starts[0]["params"]["sandbox"], "read-only", "{access:?}");
            }
        }
    }
    #[test]
    fn codex_review_denies_a_command_that_asks_for_extra_permissions() {
        use crate::review_policy::ReviewResults;
        let mut p = codex_review_protocol(ReviewResults::Post);
        for extra in [
            json!({"additionalPermissions":{"network":{"enabled":true}}}),
            json!({"permissions":{"fileSystem":{"write":["/"]}}}),
        ] {
            let mut params = json!({"command":"gh pr diff 7 -R acme/app"});
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let (events, r) =
                codex_request(&mut p, "item/commandExecution/requestApproval", params);
            assert!(events.is_empty(), "{events:?}");
            assert_eq!(r["result"]["decision"], "decline", "{extra}");
        }
        // Empty or null extras do not make a plain read suspicious.
        let (_, r) = codex_request(
            &mut p,
            "item/commandExecution/requestApproval",
            json!({"command":"gh pr diff 7 -R acme/app","additionalPermissions":null}),
        );
        assert_eq!(r["result"]["decision"], "accept");
    }
    #[test]
    fn codex_review_declines_mcp_elicitations_without_asking() {
        use crate::review_policy::ReviewResults;
        let elicitation = json!({"id":5,"method":"mcpServer/elicitation/request","params":{
            "mode":"form","message":"Run it?","requestedSchema":{"type":"object","properties":{}}}});
        for access in [
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::FullAccess,
        ] {
            let mut p = codex_review_protocol(ReviewResults::Post);
            p.request.settings.access = access;
            let mut out = Vec::new();
            let events = p.frame(elicitation.clone(), &mut out).unwrap();
            assert!(events.is_empty(), "{access:?}: {events:?}");
            let reply: Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(reply["id"], 5);
            assert_eq!(reply["result"]["action"], "decline", "{access:?}");
            assert!(p.approvals.is_empty());
        }
        // Without a review the user is still asked.
        let mut plain = protocol(Kind::Codex);
        plain.request.settings.access = AccessMode::ReadOnly;
        let mut out = Vec::new();
        let events = plain.frame(elicitation, &mut out).unwrap();
        assert!(
            matches!(events.as_slice(), [Event::Approval(_)]),
            "{events:?}"
        );
        assert!(out.is_empty());
    }
    /// Every reply and event a run gives to a fixed set of requests.
    fn transcript(p: &mut Protocol, frames: &[Value]) -> Vec<String> {
        frames
            .iter()
            .map(|frame| {
                let mut out = Vec::new();
                let events = p.frame(frame.clone(), &mut out).unwrap();
                format!("{events:?} => {}", String::from_utf8_lossy(&out))
            })
            .collect()
    }
    #[test]
    fn a_review_answers_the_same_whatever_the_access_chip_says() {
        use crate::review_policy::ReviewResults;
        let claude = |tool: &str, input: Value| json!({"type":"control_request","request_id":format!("r-{tool}-{input}"),"request":{"subtype":"can_use_tool","tool_name":tool,"input":input}});
        let claude_frames = [
            claude("Bash", json!({"command":"git status"})),
            claude("Bash", json!({"command":REVIEW_POST})),
            claude("Bash", json!({"command":"gh pr merge 7 -R acme/app"})),
            claude("Bash", json!({"command":"rm -rf src"})),
            claude("Write", json!({"file_path":"x","content":"y"})),
            claude("Edit", json!({"file_path":"x"})),
            claude("mcp__github__create_review", json!({})),
        ];
        let codex_frames = [
            json!({"id":11,"method":"item/commandExecution/requestApproval","params":{"command":"gh pr view 7 -R acme/app"}}),
            json!({"id":12,"method":"item/commandExecution/requestApproval","params":{"command":REVIEW_POST}}),
            json!({"id":13,"method":"item/commandExecution/requestApproval","params":{"command":"rm -rf src"}}),
            json!({"id":14,"method":"item/fileChange/requestApproval","params":{"reason":"edit"}}),
            json!({"id":15,"method":"applyPatchApproval","params":{"reason":"edit"}}),
            json!({"id":16,"method":"item/permissions/requestApproval","params":{"permissions":{"network":true}}}),
            json!({"id":17,"method":"mcpServer/elicitation/request","params":{"mode":"form","message":"Run it?","requestedSchema":{"type":"object","properties":{}}}}),
        ];
        for results in [ReviewResults::Draft, ReviewResults::Post] {
            let reference_claude = transcript(
                &mut review_protocol(results, AccessMode::ReadOnly),
                &claude_frames,
            );
            let mut codex = codex_review_protocol(results);
            codex.request.settings.access = AccessMode::ReadOnly;
            let reference_codex = transcript(&mut codex, &codex_frames);
            for access in [
                AccessMode::Supervised,
                AccessMode::AutoAcceptEdits,
                AccessMode::Auto,
                AccessMode::FullAccess,
            ] {
                assert_eq!(
                    transcript(&mut review_protocol(results, access), &claude_frames),
                    reference_claude,
                    "claude {results:?} {access:?}"
                );
                let mut codex = codex_review_protocol(results);
                codex.request.settings.access = access;
                assert_eq!(
                    transcript(&mut codex, &codex_frames),
                    reference_codex,
                    "codex {results:?} {access:?}"
                );
            }
        }
    }
    fn replay(kind: Kind, raw: &str) -> Vec<Event> {
        let mut protocol = protocol(kind);
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .flat_map(|line| {
                protocol
                    .frame(serde_json::from_str(line).unwrap(), &mut Vec::new())
                    .unwrap()
            })
            .collect()
    }
    fn turn_metrics(events: &[Event]) -> Vec<TurnMetrics> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Metrics(m) => Some(*m),
                _ => None,
            })
            .collect()
    }
    fn live_windows(events: &[Event]) -> Vec<Vec<crate::usage::Window>> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::UsageWindows(w) => Some(w.clone()),
                _ => None,
            })
            .collect()
    }
    #[test]
    fn real_claude_turns_report_tokens_cost_context_and_live_limits() {
        for (raw, cached, written, cost) in [
            (
                include_str!("usage/fixtures/claude-stream.jsonl"),
                23_608,
                21_991,
                0.00463648,
            ),
            (
                include_str!("usage/fixtures/claude-stream-turn2.jsonl"),
                27_635,
                26_617,
                0.01023843,
            ),
        ] {
            let events = replay(Kind::Claude, raw);
            let [metrics] = turn_metrics(&events)[..] else {
                panic!("one reading per turn: {events:?}")
            };
            assert_eq!((metrics.input_tokens, metrics.output_tokens), (2, 4));
            assert_eq!(
                (metrics.cache_read_tokens, metrics.cache_write_tokens),
                (cached, written)
            );
            assert_eq!(metrics.cost_usd, Some(cost));
            assert_eq!(metrics.credits, None);
            let mut context = crate::context::Usage::default();
            for event in &events {
                if let Event::Context { used, window } = event {
                    context.merge(*used, *window);
                }
            }
            assert_eq!(context.used, 2 + 4 + cached + written);
            assert_eq!(context.window, Some(1_000_000));
            assert_eq!(
                live_windows(&events),
                vec![vec![crate::usage::Window::new(
                    2.,
                    10_080,
                    Some(1_794_272_400_000)
                )]]
            );
        }
    }
    #[test]
    fn claude_replayed_task_notification_result_does_not_end_the_turn() {
        let mut claude = protocol(Kind::Claude);
        let replayed = json!({"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","origin":{"kind":"task-notification"}});
        assert!(!claude_turn_result(&replayed));
        let events = claude.frame(replayed, &mut Vec::new()).unwrap();
        assert!(!events.contains(&Event::Done));
        assert!(!claude.done);
        // A task that finishes during the session wakes a real turn, which ends.
        let woken = json!({"type":"result","num_turns":1,"result":"Done.","origin":{"kind":"task-notification"}});
        assert!(claude
            .frame(woken, &mut Vec::new())
            .unwrap()
            .contains(&Event::Done));
    }
    #[test]
    fn claude_replayed_task_notification_result_drops_its_usage_limit() {
        let mut claude = protocol(Kind::Claude);
        claude.frame(json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1}}), &mut Vec::new()).unwrap();
        assert!(claude.claude.usage_limit.is_some());
        claude.frame(json!({"type":"result","num_turns":0,"result":"","origin":{"kind":"task-notification"}}), &mut Vec::new()).unwrap();
        assert!(claude.claude.usage_limit.is_none());
        let error = claude.frame(json!({"type":"result","is_error":true,"num_turns":1,"result":"Something else went wrong"}), &mut Vec::new()).unwrap_err();
        assert_eq!(error, "Something else went wrong");
    }
    #[test]
    fn a_plain_claude_result_without_num_turns_still_ends_the_turn() {
        // Older CLIs omit num_turns and origin; their results must still end.
        assert!(claude_turn_result(&json!({"type":"result","result":"ok"})));
        assert!(claude_turn_result(
            &json!({"type":"result","num_turns":0,"result":""})
        ));
        assert!(!claude_turn_result(&json!({"type":"assistant"})));
    }
    #[test]
    fn claude_turn_without_cost_or_limits_still_reports_tokens() {
        let events = replay(
            Kind::Claude,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794272400}}
{"type":"result","usage":{"input_tokens":5,"output_tokens":1}}"#,
        );
        assert!(live_windows(&events).is_empty());
        let [metrics] = turn_metrics(&events)[..] else {
            panic!()
        };
        assert_eq!((metrics.input_tokens, metrics.cost_usd), (5, None));
        // A subagent's limits are not the account's.
        assert!(live_windows(&replay(
            Kind::Claude,
            r#"{"type":"rate_limit_event","parent_tool_use_id":"x","rate_limit_info":{"unifiedWindows":{"seven_day":{"utilization":0.9}}}}"#
        ))
        .is_empty());
    }
    #[test]
    fn real_codex_turns_count_each_request_once_and_push_live_limits() {
        let raw = include_str!("usage/fixtures/codex-app-server.jsonl");
        let events = replay(Kind::Codex, raw);
        let metrics = turn_metrics(&events);
        assert_eq!(metrics.len(), 2);
        // Turn one: its own request.
        assert_eq!(
            (
                metrics[0].input_tokens,
                metrics[0].output_tokens,
                metrics[0].cache_read_tokens
            ),
            (20_129, 5, 6_656)
        );
        // A process that carries on adds only the new request, never the
        // running total again.
        assert_eq!(
            (
                metrics[1].input_tokens,
                metrics[1].output_tokens,
                metrics[1].cache_read_tokens
            ),
            (40_275, 11, 26_624)
        );
        assert!(metrics.iter().all(|m| m.cost_usd.is_none()));
        assert!(events.contains(&Event::Context {
            used: Some(20_152),
            window: Some(258_400)
        }));
        let live = live_windows(&events);
        assert_eq!(live.len(), 2);
        assert_eq!(
            live[1],
            vec![crate::usage::Window::new(
                19.,
                43_200,
                Some(1_795_418_928_000)
            )]
        );
        // A notification with no readable window changes nothing.
        assert!(live_windows(&replay(
            Kind::Codex,
            r#"{"method":"account/rateLimits/updated","params":{"rateLimits":{"primary":null,"secondary":null}}}"#
        ))
        .is_empty());
    }
    #[test]
    fn context_reports_use_the_latest_request_and_include_cached_tokens() {
        let mut codex = protocol(Kind::Codex);
        let events = codex.frame(json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":4200},"total":{"totalTokens":99000},"modelContextWindow":272000}}}),&mut Vec::new()).unwrap();
        assert_eq!(
            events,
            vec![Event::Context {
                used: Some(4200),
                window: Some(272000)
            }]
        );
        let mut claude = protocol(Kind::Claude);
        let events = claude.frame(json!({"type":"assistant","message":{"content":[],"usage":{"input_tokens":100,"cache_read_input_tokens":9000,"cache_creation_input_tokens":500,"output_tokens":25}}}),&mut Vec::new()).unwrap();
        assert_eq!(
            events,
            vec![Event::Context {
                used: Some(9625),
                window: None
            }]
        );
        assert!(claude.frame(json!({"type":"assistant","parent_tool_use_id":"subagent","message":{"content":[],"usage":{"input_tokens":999999}}}),&mut Vec::new()).unwrap().is_empty());
        let events = claude.frame(json!({"type":"result","usage":{"input_tokens":90000,"iterations":[{"input_tokens":1000},{"input_tokens":2000,"output_tokens":50}]},"modelUsage":{"model":{"contextWindow":200000}}}),&mut Vec::new()).unwrap();
        assert!(events.contains(&Event::Context {
            used: Some(2050),
            window: Some(200000)
        }));
    }
    #[test]
    fn claude_routed_turn_with_empty_iterations_keeps_the_assistant_level() {
        let mut claude = protocol(Kind::Claude);
        let mut context = crate::context::Usage::default();
        let mut feed = |frame: Value, context: &mut crate::context::Usage| {
            for event in claude.frame(frame, &mut Vec::new()).unwrap() {
                if let Event::Context { used, window } = event {
                    context.merge(used, window);
                }
            }
        };
        feed(
            json!({"type":"assistant","message":{"content":[],"usage":{"input_tokens":40000,"output_tokens":2000}}}),
            &mut context,
        );
        assert_eq!(context.used, 42000);
        // A routed provider: no iterations, and a usage summed over the turn
        // that is bigger than the window.
        feed(
            json!({"type":"result","usage":{"input_tokens":6058017,"output_tokens":24770,"iterations":[]},"modelUsage":{"deepseek-v4.1-flash":{"contextWindow":1048576}}}),
            &mut context,
        );
        assert_eq!(context.used, 42000, "the summed turn is not the level");
        assert_eq!(context.window, Some(1048576));
        assert!(context.ratio().unwrap() < 0.05, "the meter is not full");
        // Without the key at all, older CLIs' usage is still the level.
        feed(
            json!({"type":"result","usage":{"input_tokens":50000,"output_tokens":100},"modelUsage":{"m":{"contextWindow":200000}}}),
            &mut context,
        );
        assert_eq!(context.used, 50100);
        assert_eq!(context.window, Some(200000));
    }
    #[test]
    fn a_codex_image_is_reported_once_with_its_prompt() {
        let mut p = protocol(Kind::Codex);
        let mut out = Vec::new();
        let item = |result: &str| json!({"method":"item/completed","params":{"item":{"type":"imageGeneration","id":"img-1","status":"completed","revisedPrompt":"A red fox","result":result}}});
        assert_eq!(
            p.frame(json!({"method":"item/started","params":{"item":{"type":"imageGeneration","id":"img-1"}}}), &mut out).unwrap(),
            Vec::<Event>::new(),
            "nothing to show until the image is done"
        );
        assert_eq!(
            p.frame(item(" iVBORw0KGgo= "), &mut out).unwrap(),
            vec![Event::GeneratedImage {
                id: "img-1".into(),
                data: "iVBORw0KGgo=".into(),
                alt: Some("A red fox".into()),
            }]
        );
        assert_eq!(
            p.frame(item("iVBORw0KGgo="), &mut out).unwrap(),
            Vec::<Event>::new(),
            "a repeated completion adds nothing"
        );
        let empty = json!({"method":"item/completed","params":{"item":{"type":"imageGeneration","id":"img-2","result":"  "}}});
        assert_eq!(
            p.frame(empty, &mut out).unwrap(),
            Vec::<Event>::new(),
            "a failed image has no row"
        );
    }
    #[test]
    fn codex_handshake_stream_snapshot_and_resume() {
        let mut p = protocol(Kind::Codex);
        p.request.provider_id = Some("saved".into());
        let mut out = Vec::new();
        p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("thread/resume"));
        let resumed: Value = String::from_utf8_lossy(&out)
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|message| message["method"] == "thread/resume")
            .unwrap();
        assert_eq!(resumed["params"]["threadId"], "saved");
        assert_eq!(resumed["params"]["excludeTurns"], true);
        p.frame(json!({"id":2,"result":{"thread":{"id":"saved"}}}), &mut out)
            .unwrap();
        let e = p
            .frame(
                json!({"method":"item/agentMessage/delta","params":{"itemId":"a","delta":"Hi\n"}}),
                &mut out,
            )
            .unwrap();
        assert_eq!(e, vec![Event::BeginAssistant, Event::Text("Hi\n".into())]);
        assert_eq!(p.frame(json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"a","text":"Hi\n\n"}}}), &mut out).unwrap(),vec![Event::Text("\n".into())]);
        assert_eq!(
            p.frame(
                json!({"method":"turn/completed","params":{"turn":{"status":"completed"}}}),
                &mut out
            )
            .unwrap(),
            vec![Event::Done]
        );
    }
    #[test]
    fn claude_stream_and_completed_snapshot_do_not_duplicate() {
        let mut p = protocol(Kind::Claude);
        let mut out = Vec::new();
        p.frame(
            json!({"type":"stream_event","event":{"type":"message_start"}}),
            &mut out,
        )
        .unwrap();
        p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello 🙂"}}}), &mut out).unwrap();
        assert!(p.frame(json!({"type":"assistant","message":{"content":[{"type":"text","text":"Hello 🙂"}]}}), &mut out).unwrap().is_empty());
        assert!(p
            .frame(
                json!({"type":"result","is_error":true,"result":"Login required"}),
                &mut out
            )
            .unwrap_err()
            .contains("Login"));
    }
    #[test]
    fn unknown_server_requests_are_resolved_and_errors_surface() {
        let mut p = protocol(Kind::Codex);
        let mut out = Vec::new();
        p.frame(
            json!({"id":"req-1","method":"unsupported/request","params":{}}),
            &mut out,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("req-1"));
        assert!(p
            .frame(
                json!({"id":2,"error":{"message":"Resume failed"}}),
                &mut out
            )
            .unwrap_err()
            .contains("Resume failed"));
    }
    #[test]
    fn read_only_claude_allows_no_bash_pattern_at_launch() {
        let mut request = protocol(Kind::Claude).request;
        request.settings.access = AccessMode::ReadOnly;
        let mut command = Command::new("claude");
        claude::configure(&mut command, &request, None).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let at = |flag: &str| &args[args.iter().position(|arg| arg == flag).unwrap() + 1];
        assert_eq!(at("--allowedTools"), "Read,Glob,Grep");
        assert_eq!(at("--tools"), "Read,Glob,Grep,Bash");
        assert!(
            !args.iter().any(|arg| arg.contains("Bash(")),
            "a Bash pattern approves any arguments: {args:?}"
        );
    }
    fn claude_argv(request: &Request) -> Vec<String> {
        let mut command = Command::new("claude");
        claude::configure(&mut command, request, None).unwrap();
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
    fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|arg| arg == flag)
            .map(|at| args[at + 1].as_str())
    }
    #[test]
    fn claude_review_launch_overrides_user_rules_and_runs_in_default_mode() {
        use crate::review_policy::ReviewResults;
        for access in [
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::FullAccess,
        ] {
            let p = review_protocol(ReviewResults::Draft, access);
            let args = claude_argv(&p.request);
            // Default, not plan: plan tells the model not to post. Every Bash
            // call still reaches the prompt tool through the ask rule.
            assert_eq!(
                flag(&args, "--permission-mode"),
                Some("default"),
                "{access:?}"
            );
            assert_eq!(flag(&args, "--tools"), Some("Read,Glob,Grep,Bash"));
            assert_eq!(flag(&args, "--allowedTools"), Some("Read,Glob,Grep"));
            assert!(!args
                .iter()
                .any(|a| a == "--allow-dangerously-skip-permissions"));
            assert_eq!(args.iter().filter(|a| *a == "--settings").count(), 1);
            let settings: Value = serde_json::from_str(flag(&args, "--settings").unwrap()).unwrap();
            // MCP calls reach the prompt tool too, and no user or project MCP
            // server is loaded at all.
            assert_eq!(settings["permissions"]["ask"], json!(["Bash", "mcp__*"]));
            assert_eq!(
                args.iter().filter(|a| *a == "--strict-mcp-config").count(),
                1,
                "{args:?}"
            );
            assert_eq!(flag(&args, "--mcp-config"), None);
            assert_eq!(
                settings["permissions"]["deny"],
                json!(["Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch"])
            );
            assert_eq!(settings["disableAllHooks"], true);
            assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], false);
            // The repo under review supplies no settings.
            assert!(
                args.iter().any(|a| a == "--setting-sources=user"),
                "{args:?}"
            );
            assert!(!args.iter().any(|a| a.contains("project")), "{args:?}");
        }
        // A review with any access chip launches exactly like ReadOnly.
        let read_only =
            claude_argv(&review_protocol(ReviewResults::Post, AccessMode::ReadOnly).request);
        let full =
            claude_argv(&review_protocol(ReviewResults::Post, AccessMode::FullAccess).request);
        assert_eq!(read_only, full);
    }
    #[test]
    fn claude_non_review_launch_is_unchanged() {
        let mut request = protocol(Kind::Claude).request;
        request.settings.access = AccessMode::ReadOnly;
        let args = claude_argv(&request);
        assert_eq!(flag(&args, "--permission-mode"), Some("plan"));
        let settings: Value = serde_json::from_str(flag(&args, "--settings").unwrap()).unwrap();
        assert_eq!(settings, json!({}));
        assert!(args
            .iter()
            .any(|a| a == "--setting-sources=user,project,local"));
        assert!(!args.iter().any(|a| a == "--strict-mcp-config"));
        request.settings.access = AccessMode::FullAccess;
        let args = claude_argv(&request);
        assert_eq!(flag(&args, "--permission-mode"), Some("bypassPermissions"));
        assert!(args
            .iter()
            .any(|a| a == "--allow-dangerously-skip-permissions"));
        assert_eq!(flag(&args, "--tools"), None);
    }
    fn claude_bash(p: &mut Protocol, command: &str) -> (Vec<Event>, Value) {
        let mut out = Vec::new();
        let events = p
            .frame(json!({"type":"control_request","request_id":"r","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":command}}}), &mut out)
            .unwrap();
        let response = if out.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice::<Value>(&out).unwrap()["response"]["response"].clone()
        };
        (events, response)
    }
    #[test]
    fn read_only_claude_runs_safe_queries_and_denies_flags_that_execute() {
        let mut p = protocol(Kind::Claude);
        p.request.settings.access = AccessMode::ReadOnly;
        for command in [
            "git status -sb",
            "git diff --stat HEAD~1",
            "git log --oneline -5",
            "ls -la",
        ] {
            let (events, response) = claude_bash(&mut p, command);
            assert!(events.is_empty(), "{command}: {events:?}");
            assert_eq!(response["behavior"], "allow", "{command}");
            assert_eq!(response["updatedInput"]["command"], command);
        }
        for command in [
            "rg --pre ./evil x",
            "git diff --ext-diff",
            "git log -p --textconv",
            "git -c core.pager=./evil log",
            "git log --output=/tmp/owned",
            "git status && touch owned",
        ] {
            let (events, response) = claude_bash(&mut p, command);
            assert!(events.is_empty(), "{command}: {events:?}");
            assert_eq!(response["behavior"], "deny", "{command}");
        }
        assert!(
            p.approvals.is_empty(),
            "every answered request is forgotten"
        );
    }
    #[test]
    fn claude_review_policy_decides_bash() {
        use crate::review_policy::{ReviewPolicy, ReviewResults, ReviewTarget};
        let with = |results, review: bool| {
            let mut p = protocol(Kind::Claude);
            p.request.settings.access = AccessMode::ReadOnly;
            p.request.settings.review = review.then(|| ReviewPolicy {
                results,
                target: Some(ReviewTarget {
                    forge: crate::review_policy::Forge::GitHub,
                    repo: "acme/app".into(),
                    number: 7,
                }),
            });
            p
        };
        let read = "gh pr diff 7 -R acme/app";
        let post = "gh pr review 7 -R acme/app --comment -b \"x\"";
        let merge = "gh pr merge 7 -R acme/app";
        for results in [
            ReviewResults::Gespan,
            ReviewResults::Draft,
            ReviewResults::Post,
        ] {
            let mut p = with(results, true);
            let (events, response) = claude_bash(&mut p, read);
            assert!(events.is_empty(), "{results:?}: {events:?}");
            assert_eq!(response["behavior"], "allow", "{results:?}");
            let (events, response) = claude_bash(&mut p, merge);
            assert!(events.is_empty(), "{results:?}: {events:?}");
            assert_eq!(response["behavior"], "deny", "{results:?}");
        }
        let mut p = with(ReviewResults::Draft, true);
        let (events, response) = claude_bash(&mut p, post);
        assert_eq!(response, Value::Null, "draft waits for the user");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Approval(_)))
                .count(),
            1
        );
        let mut p = with(ReviewResults::Post, true);
        let (events, response) = claude_bash(&mut p, post);
        assert!(events.is_empty());
        assert_eq!(response["behavior"], "allow");
        let mut p = with(ReviewResults::Gespan, true);
        let (events, response) = claude_bash(&mut p, post);
        assert!(events.is_empty());
        assert_eq!(response["behavior"], "deny");
        let mut p = with(ReviewResults::Gespan, false);
        let (events, response) = claude_bash(&mut p, read);
        assert!(events.is_empty());
        assert_eq!(
            response["behavior"], "deny",
            "no policy: read-only denies gh"
        );
    }
    fn review_protocol(
        results: crate::review_policy::ReviewResults,
        access: AccessMode,
    ) -> Protocol {
        use crate::review_policy::{ReviewPolicy, ReviewTarget};
        let mut p = protocol(Kind::Claude);
        p.request.settings.access = access;
        p.request.settings.review = Some(ReviewPolicy {
            results,
            target: Some(ReviewTarget {
                forge: crate::review_policy::Forge::GitHub,
                repo: "acme/app".into(),
                number: 7,
            }),
        });
        p
    }
    const REVIEW_POST: &str = "gh pr review 7 -R acme/app --comment -b \"x\"";
    #[test]
    fn claude_review_policy_ask_is_not_rememberable_and_session_installs_no_rule() {
        use crate::review_policy::ReviewResults;
        let mut p = review_protocol(ReviewResults::Draft, AccessMode::Supervised);
        let mut out = Vec::new();
        let events = p
            .frame(json!({"type":"control_request","request_id":"r","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":REVIEW_POST},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"gh pr review:*"}]}]}}), &mut out)
            .unwrap();
        let approvals: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Approval(a) => Some(a),
                _ => None,
            })
            .collect();
        assert_eq!(approvals.len(), 1);
        assert!(!approvals[0].can_remember);
        assert!(
            approvals[0]
                .detail
                .contains("gh pr review 7 -R acme/app --comment"),
            "{}",
            approvals[0].detail
        );
        let id = approvals[0].id.clone();
        let mut out = Vec::new();
        p.respond(
            Control {
                id,
                decision: Decision::Session,
            },
            &mut out,
        )
        .unwrap();
        let response =
            serde_json::from_slice::<Value>(&out).unwrap()["response"]["response"].clone();
        assert_eq!(response["behavior"], "allow");
        assert!(response.get("updatedPermissions").is_none(), "{response}");
    }
    #[test]
    fn claude_review_policy_ask_reaches_the_user_even_in_full_access() {
        use crate::review_policy::ReviewResults;
        let mut p = review_protocol(ReviewResults::Draft, AccessMode::FullAccess);
        let (events, response) = claude_bash(&mut p, REVIEW_POST);
        assert_eq!(response, Value::Null);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Approval(_)))
                .count(),
            1
        );
    }
    #[test]
    fn claude_review_policy_deny_says_it_is_policy_and_other_tools_are_denied() {
        use crate::review_policy::ReviewResults;
        for results in [ReviewResults::Draft, ReviewResults::Post] {
            let mut p = review_protocol(results, AccessMode::ReadOnly);
            let (_, response) = claude_bash(&mut p, "gh pr merge 7 -R acme/app");
            assert_eq!(response["behavior"], "deny");
            assert_eq!(response["message"], "Not allowed for a pull request review");
            for tool in ["Write", "mcp__github__create_review"] {
                let mut out = Vec::new();
                let events = p
                    .frame(json!({"type":"control_request","request_id":"r","request":{"subtype":"can_use_tool","tool_name":tool,"input":{}}}), &mut out)
                    .unwrap();
                assert!(events.is_empty(), "{tool}: {events:?}");
                let response =
                    serde_json::from_slice::<Value>(&out).unwrap()["response"]["response"].clone();
                assert_eq!(response["behavior"], "deny", "{tool} under {results:?}");
            }
        }
    }
    #[test]
    fn safe_bash_still_asks_outside_read_only() {
        for access in [AccessMode::Supervised, AccessMode::AutoAcceptEdits] {
            let mut p = protocol(Kind::Claude);
            p.request.settings.access = access;
            let (events, response) = claude_bash(&mut p, "git status");
            assert!(
                matches!(events.as_slice(), [Event::Approval(_)]),
                "{access:?}: {events:?}"
            );
            assert_eq!(response, Value::Null, "no answer before the user decides");
        }
        let mut p = protocol(Kind::Claude);
        p.request.settings.plan = true;
        let (_, response) = claude_bash(&mut p, "git status");
        assert_eq!(
            response["behavior"], "deny",
            "a plan turn keeps denying Bash"
        );
    }
    #[test]
    fn approvals_carry_the_call_they_ask_about() {
        let mut claude = protocol(Kind::Claude);
        let mut out = Vec::new();
        let input = json!({"command":"cd /tmp && ls","description":"List files"});
        let events = claude.frame(json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","tool_use_id":"tu1","input":input}}), &mut out).unwrap();
        let Event::Approval(approval) = &events[0] else {
            panic!("expected an approval")
        };
        let tool = approval.tool.as_ref().expect("Claude names its tool");
        assert_eq!((tool.id.as_str(), tool.name.as_str()), ("tu1", "Bash"));
        assert_eq!(serde_json::from_str::<Value>(&tool.input).unwrap(), input);
        assert_eq!(
            serde_json::from_str::<Value>(&approval.detail).unwrap(),
            input,
            "the raw detail is kept for the notification card"
        );
        let events = claude.frame(json!({"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"mcp__github__create_issue","input":{"title":"Bug"}}}), &mut out).unwrap();
        let Event::Approval(approval) = &events[0] else {
            panic!("expected an approval")
        };
        assert_eq!(
            approval.tool.as_ref().unwrap().name,
            "mcp__github__create_issue"
        );

        // Codex: a command it announced as an item, then one it didn't.
        let mut codex = protocol(Kind::Codex);
        codex.frame(json!({"method":"item/started","params":{"item":{"id":"c1","type":"commandExecution","command":"/bin/zsh -lc 'cargo build'","cwd":"/work"}}}), &mut out).unwrap();
        let events = codex.frame(json!({"id":"p1","method":"item/commandExecution/requestApproval","params":{"itemId":"c1","reason":"Build it"}}), &mut out).unwrap();
        let Event::Approval(approval) = events
            .iter()
            .find(|e| matches!(e, Event::Approval(_)))
            .unwrap()
        else {
            unreachable!()
        };
        let tool = approval.tool.as_ref().expect("the announced item");
        assert_eq!(tool.id, "c1");
        assert_eq!(tool.presentation().command.as_deref(), Some("cargo build"));
        let events = codex.frame(json!({"id":"p2","method":"execCommandApproval","params":{"command":["git","push"],"cwd":"/work/app"}}), &mut out).unwrap();
        let Event::Approval(approval) = &events[0] else {
            panic!("expected an approval")
        };
        let tool = approval.tool.as_ref().expect("built from the request");
        assert_eq!(tool.name, "commandExecution");
        assert_eq!(
            serde_json::from_str::<Value>(&tool.input).unwrap(),
            json!({"command":["git","push"],"cwd":"/work/app"})
        );
        assert_eq!(tool.presentation().command.as_deref(), Some("git push"));
        let events = codex.frame(json!({"id":"p3","method":"item/fileChange/requestApproval","params":{"itemId":"unknown","reason":"Edit"}}), &mut out).unwrap();
        let Event::Approval(approval) = &events[0] else {
            panic!("expected an approval")
        };
        assert_eq!(approval.tool, None, "nothing to show but the reason");
    }
    #[test]
    fn approvals_wait_for_a_decision_and_keep_the_requested_input() {
        let mut codex = protocol(Kind::Codex);
        let mut out = Vec::new();
        let events = codex.frame(json!({"id":"permission-1","method":"item/commandExecution/requestApproval","params":{"command":"cargo build","itemId":"c1"}}), &mut out).unwrap();
        let Event::Approval(approval) = &events[0] else {
            panic!("expected an approval")
        };
        assert!(
            out.is_empty(),
            "never answer an approval before a native decision"
        );
        codex
            .respond(
                Control {
                    id: approval.id.clone(),
                    decision: Decision::Deny,
                },
                &mut out,
            )
            .unwrap();
        let response: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(response["id"], "permission-1");
        assert_eq!(response["result"]["decision"], "decline");
        let mut claude = protocol(Kind::Claude);
        out.clear();
        let input = json!({"file_path":"/work/example.rs","content":"approved"});
        let events = claude.frame(json!({"type":"control_request","request_id":"permission-2","request":{"subtype":"can_use_tool","tool_name":"Write","input":input}}), &mut out).unwrap();
        let Event::Approval(approval) = &events[0] else {
            panic!("expected an approval")
        };
        assert!(out.is_empty());
        claude
            .respond(
                Control {
                    id: approval.id.clone(),
                    decision: Decision::Allow,
                },
                &mut out,
            )
            .unwrap();
        let response: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(response["response"]["request_id"], "permission-2");
        assert_eq!(response["response"]["response"]["behavior"], "allow");
        assert_eq!(response["response"]["response"]["updatedInput"], input);
    }
    #[test]
    fn selected_model_effort_and_access_reach_the_codex_turn() {
        let mut p = protocol(Kind::Codex);
        p.request.model = Some("selected-model".into());
        p.request.settings.effort = Some("high".into());
        p.request.settings.access = AccessMode::AutoAcceptEdits;
        let mut out = Vec::new();
        p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
        let rows: Vec<Value> = String::from_utf8_lossy(&out)
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows[1]["params"]["sandbox"], "workspace-write");
        out.clear();
        p.frame(
            json!({"id":2,"result":{"thread":{"id":"thread"},"model":"selected-model"}}),
            &mut out,
        )
        .unwrap();
        let row: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(row["params"]["model"], "selected-model");
        assert_eq!(row["params"]["effort"], "high");
        assert_eq!(row["params"]["sandboxPolicy"]["type"], "workspaceWrite");
        assert_eq!(row["params"]["approvalsReviewer"], "user");
    }
    #[test]
    fn codex_tool_deltas_settle_to_one_completed_snapshot() {
        let mut p = protocol(Kind::Codex);
        let mut out = Vec::new();
        let started = p.frame(json!({"method":"item/started","params":{"item":{"id":"cmd-1","type":"commandExecution","command":"cargo check","status":"inProgress"}}}), &mut out).unwrap();
        assert!(
            matches!(&started[0], Event::Tool(t) if t.state == ToolState::Running && t.summary() == ("Run".into(), "cargo check".into()))
        );
        p.frame(json!({"method":"item/commandExecution/outputDelta","params":{"itemId":"cmd-1","delta":"Checking 🙂"}}), &mut out).unwrap();
        let completed = p.frame(json!({"method":"item/completed","params":{"item":{"id":"cmd-1","type":"commandExecution","command":"cargo check","status":"completed","aggregatedOutput":"Checking 🙂\nFinished"}}}), &mut out).unwrap();
        assert!(
            matches!(&completed[0], Event::Tool(t) if t.id == "cmd-1" && t.state == ToolState::Completed && t.output == "Checking 🙂\nFinished")
        );
        assert_eq!(p.tools.len(), 1);
    }
    #[test]
    fn codex_live_tool_preserves_argv_and_semantic_rollout_metadata() {
        let mut protocol = protocol(Kind::Codex);
        let mut out = Vec::new();
        let command = json!(["/bin/zsh", "-lc", "sed -n '8,20p' 'src/my file.rs'"]);
        let events = protocol.frame(json!({"method":"item/started","params":{"item":{"id":"read","type":"commandExecution","command":command,"parsed_cmd":[{"type":"read","path":"/repo/src/my file.rs"}],"cwd":"/repo","status":"inProgress"}}}), &mut out).unwrap();
        let Event::Tool(call) = &events[0] else {
            panic!()
        };
        assert_eq!(call.summary(), ("Read".into(), "src/my file.rs".into()));
        assert_eq!(
            call.presentation().command.as_deref(),
            Some("sed -n '8,20p' 'src/my file.rs'")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&call.input).unwrap()["command"],
            command
        );
        let events = protocol.frame(json!({"method":"item/completed","params":{"item":{"id":"read","type":"commandExecution","status":"failed","aggregatedOutput":"permission denied","exitCode":1}}}), &mut out).unwrap();
        let Event::Tool(call) = &events[0] else {
            panic!()
        };
        assert_eq!(call.summary().0, "Read");
        assert_eq!(call.output, "permission denied");
        assert_eq!(call.state, ToolState::Failed);
        assert_eq!(
            serde_json::from_str::<Value>(&call.input).unwrap()["command"],
            command
        );
    }
    #[test]
    fn claude_tool_input_and_result_keep_identity_and_failure() {
        let mut p = protocol(Kind::Claude);
        let mut out = Vec::new();
        p.frame(json!({"type":"stream_event","event":{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"read-1","name":"Read"}}}), &mut out).unwrap();
        p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"src/main.rs\"}"}}}), &mut out).unwrap();
        let input = p.tools["read-1"].summary();
        assert_eq!(input, ("Read".into(), "src/main.rs".into()));
        p.frame(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"src/main.rs"}}]}}), &mut out).unwrap();
        let events = p.frame(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"read-1","is_error":true,"content":[{"type":"text","text":"File missing"}]}]}}), &mut out).unwrap();
        assert!(
            matches!(&events[0], Event::Tool(t) if t.id == "read-1" && t.state == ToolState::Failed && t.output == "File missing")
        );
        assert_eq!(p.tools.len(), 1);
    }
    #[test]
    fn claude_partial_tool_input_waits_until_it_parses() {
        let mut p = protocol(Kind::Claude);
        let mut out = Vec::new();
        p.frame(json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"sh","name":"Bash"}}}), &mut out).unwrap();
        let events = p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"git status"}}}), &mut out).unwrap();
        assert!(events.is_empty());
        assert_eq!(p.tools["sh"].input, "");
        let events = p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":" --short\"}"}}}), &mut out).unwrap();
        let Event::Tool(call) = &events[0] else {
            panic!()
        };
        assert_eq!(call.input, r#"{"command": "git status --short"}"#);
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    fn fixture(script: &str) -> (tempfile::TempDir, Request) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("provider");
        std::fs::write(&path, script).unwrap();
        crate::test_exec::make_executable(&path, 0o700);
        let request = Request {
            kind: Kind::Codex,
            binary: path,
            cwd: tmp.path().into(),
            prompt: "hello".into(),
            provider_id: None,
            model: None,
            settings: Default::default(),
        };
        (tmp, request)
    }
    #[test]
    fn cancellation_reaps_child_and_descendant_promptly() {
        for kind in Kind::ALL {
            let (_tmp, mut request) =
                fixture("#!/bin/sh\nsleep 30 &\nprintf ready > ready\nwait\n");
            request.kind = kind;
            request.settings.access = AccessMode::ReadOnly;
            let ready = request.cwd.join("ready");
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let timer = thread::spawn(move || {
                // Wait until the descendant exists, then measure cancellation
                // independently of provider startup and test-runner load.
                let deadline = Instant::now() + Duration::from_secs(10);
                while !ready.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                let started = ready.exists();
                let stopped_at = Instant::now();
                stop.store(true, Ordering::Relaxed);
                (started, stopped_at)
            });
            assert_eq!(run(request, cancel, |_| Ok(())).unwrap_err(), "Stopped");
            let (started, stopped_at) = timer.join().unwrap();
            assert!(started, "{} fixture did not start", kind.label());
            assert!(
                stopped_at.elapsed() < Duration::from_secs(2),
                "{} cancellation must not wait on provider output",
                kind.label()
            );
        }
    }
    #[test]
    fn crash_preserves_coalesced_partial_text() {
        // Read initialization before exiting so startup cannot race a closed stdin.
        let (_tmp, request) = fixture(
            "#!/bin/sh\nIFS= read -r init\nprintf '%s\\n' '{\"method\":\"item/agentMessage/delta\",\"params\":{\"itemId\":\"a\",\"delta\":\"partial\"}}'\n",
        );
        let mut text = String::new();
        let result = run(request, Arc::new(AtomicBool::new(false)), |events| {
            for event in events {
                if let Event::Text(t) = event {
                    text.push_str(&t);
                }
            }
            Ok(())
        });
        assert!(result.unwrap_err().contains("exited"));
        assert_eq!(text, "partial");
    }
    #[test]
    fn large_history_and_tool_frames_complete_the_turn() {
        let (_tmp, mut request) = fixture(
            r#"#!/usr/bin/env python3
import json, sys
payload = 'x' * (3 * 1024 * 1024) + 'end'
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({'id': 1, 'result': {}})
    elif value.get('id') == 2:
        assert value['method'] == 'thread/resume'
        send({'id': 2, 'result': {'thread': {'id': 'large-thread', 'turns': [
            {'items': [{'id': 'history', 'type': 'agentMessage', 'text': payload}]}
        ]}}})
    elif value.get('id') == 3:
        send({'method': 'item/started', 'params': {'item': {
            'id': 'large-tool', 'type': 'commandExecution', 'command': 'large output', 'status': 'inProgress'
        }}})
        send({'method': 'item/completed', 'params': {'item': {
            'id': 'large-tool', 'type': 'commandExecution', 'status': 'completed', 'aggregatedOutput': payload
        }}})
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'answer', 'delta': 'Complete'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
"#,
        );
        request.provider_id = Some("large-thread".into());
        let mut text = String::new();
        let mut output = String::new();
        let mut done = false;
        run(request, Arc::new(AtomicBool::new(false)), |events| {
            for event in events {
                match event {
                    Event::Text(delta) => text.push_str(&delta),
                    Event::Tool(tool)
                        if tool.id == "large-tool" && tool.state == ToolState::Completed =>
                    {
                        output = tool.output;
                    }
                    Event::Done => done = true,
                    _ => {}
                }
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(text, "Complete");
        assert!(output.ends_with("end"));
        assert!(done);
    }

    /// A fake `kiro-cli` in Python. `launch` runs first with `args`, the
    /// arguments it was started with; `turn` answers each ACP request `m`.
    fn kiro(launch: &str, turn: &str) -> (tempfile::TempDir, Request) {
        let indent = |code: &str| {
            code.lines()
                .map(|line| format!("        {line}\n"))
                .collect::<String>()
        };
        let (tmp, mut request) = fixture(&format!(
            r#"#!/usr/bin/env python3
import json, os, sys, time
args = sys.argv[1:]
with open(os.path.join(os.path.dirname(sys.argv[0]), 'launches'), 'a') as f:
    f.write(' '.join(args) + '\n')
def send(value):
    print(json.dumps(value), flush=True)
def reply(m, result):
    send({{'jsonrpc': '2.0', 'id': m['id'], 'result': result}})
def say(text):
    send({{'jsonrpc': '2.0', 'method': 'session/update', 'params': {{'sessionId': 'k1', 'update': {{'sessionUpdate': 'agent_message_chunk', 'content': {{'type': 'text', 'text': text}}}}}}}})
def log(line):
    sys.stderr.write(line + '\n')
    sys.stderr.flush()
{launch}
for line in sys.stdin:
    m = json.loads(line)
    method = m.get('method')
    if True:
{turn}"#,
            turn = indent(turn)
        ));
        request.kind = Kind::Kiro;
        (tmp, request)
    }
    const KIRO_TURN: &str = "if method == 'initialize':
    reply(m, {})
elif method == 'session/new':
    reply(m, {'sessionId': 'k1'})
elif method == 'session/prompt':
    say('Hi')
    reply(m, {'stopReason': 'end_turn'})";
    fn launches(request: &Request) -> Vec<String> {
        std::fs::read_to_string(request.cwd.join("launches"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn kiro_run(request: Request) -> (Result<(), String>, String) {
        let mut text = String::new();
        let result = run(request, Arc::new(AtomicBool::new(false)), |events| {
            for event in events {
                if let Event::Text(t) = event {
                    text.push_str(&t);
                }
            }
            Ok(())
        });
        (result, text)
    }
    #[test]
    fn a_new_claude_conversation_launches_with_the_id_gespan_picked() {
        let launched_args = |provider_id: Option<&str>| {
            let (tmp, mut request) =
                fixture("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$GESPAN_TEST_ARGS\"\n");
            let args = tmp.path().join("args");
            request.kind = Kind::Claude;
            request.provider_id = provider_id.map(Into::into);
            request
                .settings
                .environment
                .insert("GESPAN_TEST_ARGS".into(), args.display().to_string());
            let _ = run(request, Arc::new(AtomicBool::new(false)), |_| Ok(()));
            let args = std::fs::read_to_string(args).unwrap();
            args.lines().map(str::to_owned).collect::<Vec<_>>()
        };
        let value_after = |args: &[String], flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };

        let new = launched_args(None);
        let id = value_after(&new, "--session-id").expect("a new conversation names its id");
        assert_eq!(id.len(), 36, "{id}");
        assert_eq!(value_after(&new, "--resume"), None);

        let resumed = launched_args(Some("previous-session"));
        assert_eq!(value_after(&resumed, "--session-id"), None);
        assert_eq!(
            value_after(&resumed, "--resume").as_deref(),
            Some("previous-session")
        );
    }
    #[test]
    fn a_provider_process_starts_in_the_folder_its_provider_names() {
        for (kind, in_install_folder) in [
            (Kind::Antigravity, true),
            (Kind::Codex, false),
            (Kind::Cursor, false),
            (Kind::Kiro, false),
        ] {
            let (tmp, mut request) = fixture("#!/bin/sh\npwd -P > \"$GESPAN_TEST_PWD\"\n");
            let project = tmp.path().join("project");
            std::fs::create_dir(&project).unwrap();
            let mark = tmp.path().join("started-in");
            request.kind = kind;
            request.cwd = project.clone();
            request
                .settings
                .environment
                .insert("GESPAN_TEST_PWD".into(), mark.display().to_string());
            // The stand-in exits at once, so the turn fails; only where it ran matters.
            let _ = run(request, Arc::new(AtomicBool::new(false)), |_| Ok(()));
            let started = std::fs::read_to_string(&mark).unwrap();
            let expected = if in_install_folder {
                tmp.path().canonicalize().unwrap()
            } else {
                project.canonicalize().unwrap()
            };
            assert_eq!(PathBuf::from(started.trim()), expected, "{kind:?}");
        }
    }
    #[test]
    fn kiro_launches_v3_and_falls_back_to_plain_acp_when_v3_exits() {
        // A CLI without the V3 engine rejects the flag and exits.
        let (_tmp, request) = kiro(
            "if '--agent-engine=v3' in args:\n    log('error: unexpected argument --agent-engine')\n    sys.exit(2)",
            KIRO_TURN,
        );
        let (result, text) = kiro_run(request.clone());
        assert_eq!(result, Ok(()));
        assert_eq!(text, "Hi");
        assert_eq!(launches(&request), ["acp --agent-engine=v3", "acp"]);

        // A V3 CLI is launched once.
        let (_tmp, request) = kiro("", KIRO_TURN);
        let (result, text) = kiro_run(request.clone());
        assert_eq!(result, Ok(()));
        assert_eq!(text, "Hi");
        assert_eq!(launches(&request), ["acp --agent-engine=v3"]);

        // The fallback is tried once, even when it exits as well.
        let (_tmp, request) = kiro("sys.exit(2)", KIRO_TURN);
        let (result, _) = kiro_run(request.clone());
        assert!(result.unwrap_err().contains("exited"));
        assert_eq!(launches(&request), ["acp --agent-engine=v3", "acp"]);

        // Exiting after `initialize` succeeded is a failed turn, not a fallback.
        let (_tmp, request) = kiro(
            "",
            "if method == 'initialize':\n    reply(m, {})\nelse:\n    sys.exit(1)",
        );
        let (result, _) = kiro_run(request.clone());
        assert!(result.unwrap_err().contains("exited"));
        assert_eq!(launches(&request), ["acp --agent-engine=v3"]);
    }
    #[test]
    fn kiro_does_not_retry_a_live_process_that_times_out() {
        let help = "Run `kiro-cli login` (check with `kiro-cli whoami`).";
        acp::DEADLINE_OVERRIDE.with(|d| d.set(Some(Duration::from_secs(2))));
        let (_tmp, request) = kiro("", "pass");
        let (result, _) = kiro_run(request.clone());
        acp::DEADLINE_OVERRIDE.with(|d| d.set(None));
        assert_eq!(result, Err(format!("Kiro CLI did not start. {help}")));
        assert_eq!(launches(&request), ["acp --agent-engine=v3"]);

        // An error reply to `initialize` is an answer, not an exit.
        let (_tmp, request) = kiro(
            "",
            "send({'jsonrpc': '2.0', 'id': m['id'], 'error': {'code': -32600, 'message': 'Unsupported protocol version'}})",
        );
        let (result, _) = kiro_run(request.clone());
        assert_eq!(result, Err("Kiro CLI: Unsupported protocol version".into()));
        assert_eq!(launches(&request), ["acp --agent-engine=v3"]);
    }
    #[test]
    fn kiro_signed_out_errors_end_with_the_login_help() {
        let help = "Run `kiro-cli login` (check with `kiro-cli whoami`).";
        // Kiro logs progress, debug lines included, on stderr: a turn succeeds.
        let (_tmp, request) = kiro(
            "log('[DEBUG] token expired, refreshing')\nlog('INFO auth failed for cached profile; retrying')\nlog('Starting login service')",
            KIRO_TURN,
        );
        let (result, text) = kiro_run(request);
        assert_eq!(result, Ok(()));
        assert_eq!(text, "Hi");

        // An explicit sign-in failure ends a turn that would otherwise hang.
        let (_tmp, request) = kiro(
            "",
            "if method == 'initialize':\n    reply(m, {})\nelif method == 'session/new':\n    log('Error: not logged in')",
        );
        let started = Instant::now();
        let (result, _) = kiro_run(request);
        assert_eq!(
            result,
            Err(format!(
                "Signed out of Kiro CLI. Error: not logged in {help}"
            ))
        );
        assert!(started.elapsed() < Duration::from_secs(10));

        // The sign-in failure wins over the generic reply that follows it.
        let (_tmp, request) = kiro(
            "",
            "if method == 'initialize':\n    reply(m, {})\nelif method == 'session/new':\n    log('authentication required')\n    time.sleep(0.3)\n    send({'jsonrpc': '2.0', 'id': m['id'], 'error': {'code': -32603, 'message': 'Internal error'}})",
        );
        let (result, _) = kiro_run(request);
        assert_eq!(
            result,
            Err(format!(
                "Signed out of Kiro CLI. authentication required {help}"
            ))
        );
    }
    #[test]
    fn kiro_signed_out_v3_exit_does_not_fall_back() {
        let help = "Run `kiro-cli login` (check with `kiro-cli whoami`).";
        let (_tmp, request) = kiro("log('Error: not logged in')\nsys.exit(1)", KIRO_TURN);
        let (result, _) = kiro_run(request.clone());
        assert_eq!(
            result,
            Err(format!(
                "Signed out of Kiro CLI. Error: not logged in {help}"
            ))
        );
        assert_eq!(launches(&request), ["acp --agent-engine=v3"]);
    }
    #[test]
    fn kiro_stderr_auth_noise_mid_prompt_does_not_end_the_turn() {
        // An MCP server or tool logs an auth-like line while the prompt runs.
        let (_tmp, request) = kiro(
            "",
            "if method == 'initialize':\n    reply(m, {})\nelif method == 'session/new':\n    reply(m, {'sessionId': 'k1'})\nelif method == 'session/prompt':\n    log('mcp github: 401 Unauthorized')\n    log('2026-10-25T10:00:00Z  WARN token expired, refreshing')\n    time.sleep(0.3)\n    say('Hi')\n    reply(m, {'stopReason': 'end_turn'})",
        );
        let (result, text) = kiro_run(request);
        assert_eq!(result, Ok(()));
        assert_eq!(text, "Hi");
    }
    #[test]
    fn kiro_stop_while_v3_exits_does_not_launch_the_fallback() {
        // The v3 attempt closes stdout, then takes a while to exit; Stop
        // arrives while the launcher waits to see it exit.
        let (_tmp, request) = kiro(
            "if '--agent-engine=v3' in args:\n    os.close(1)\n    open(os.path.join(os.path.dirname(sys.argv[0]), 'closed'), 'w').close()\n    time.sleep(1)\n    sys.exit(2)",
            KIRO_TURN,
        );
        let closed = request.cwd.join("closed");
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let stopper = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !closed.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            thread::sleep(Duration::from_millis(200));
            stop.store(true, Ordering::Relaxed);
        });
        let result = run(request.clone(), cancel, |_| Ok(()));
        stopper.join().unwrap();
        assert_eq!(result, Err("Stopped".into()));
        assert_eq!(launches(&request), ["acp --agent-engine=v3"]);
    }
    #[test]
    fn malformed_provider_json_fails() {
        // Exercise JSON parsing after startup, rather than a broken initialization pipe.
        let (_tmp, request) = fixture("#!/bin/sh\nIFS= read -r init\nprintf '{invalid json\\n'\n");
        let error = run(request, Arc::new(AtomicBool::new(false)), |_| Ok(())).unwrap_err();
        assert!(error.starts_with("Invalid provider JSON:"), "{error}");
    }
}
