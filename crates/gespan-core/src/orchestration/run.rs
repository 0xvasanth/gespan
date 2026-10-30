//! One orchestration run under its lead: orchestration.ts's state machine. The
//! app applies the returned effects to live sessions, so everything here runs
//! without one.
use super::{overlaps, worktree::Workspace, Choice, Plan, Summary, SummaryTask};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_TASKS: usize = 40;
const MAX_REQUESTS: usize = 512;
const MAX_CONTINUATIONS: u32 = 20;
const MAX_RESULT: usize = 20_000;
pub const MAX_WAIT_SECONDS: u64 = 25;

/// A mistyped field must fail loudly rather than silently change the task.
const FIELDS: [(&str, &[&str]); 12] = [
    ("list", &[]),
    (
        "delegate",
        &["title", "harness", "model", "prompt", "files", "dependsOn"],
    ),
    ("get", &["taskId"]),
    ("message", &["taskId", "text"]),
    ("retry", &["taskId", "text", "files"]),
    ("cancel", &["taskId"]),
    ("wait", &["timeoutSeconds"]),
    ("review", &["taskId"]),
    ("finish", &[]),
    ("steer", &["taskId", "text"]),
    ("respond", &["taskId", "requestId", "decision"]),
    ("answer", &["taskId", "requestId", "answers", "skip"]),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Blocked,
    Interrupted,
    Cancelled,
}
impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
    fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Active,
    Paused,
    Stopped,
    Finished,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTask {
    pub id: String,
    /// The worker's session, once it has one.
    #[serde(default)]
    pub session: Option<i64>,
    pub title: String,
    pub prompt: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_settings: BTreeMap<String, String>,
    pub status: Status,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub error: Option<String>,
    /// A retained worker can continue safely with this recovery turn.
    #[serde(default)]
    pub recovery_prompt: Option<String>,
    /// The lead has this result, or there is none for it yet.
    #[serde(default = "yes")]
    pub delivered: bool,
    /// This worker's own checkout, while it has one. A retry reuses it so
    /// partial work is never orphaned.
    #[serde(default)]
    pub workspace: Option<Workspace>,
    /// Its unchanged checkout was already looked at for removal.
    #[serde(default)]
    pub swept: bool,
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub signature: String,
    pub result: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub cwd: String,
    /// The lead's harness: what a task with none of its own runs on.
    pub provider: String,
    pub status: RunStatus,
    /// The pairs delegate may choose from.
    #[serde(default)]
    pub allowed: Vec<Choice>,
    pub max_workers: usize,
    pub tasks: Vec<RunTask>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub continuations: u32,
    #[serde(default)]
    pub last_pause_reason: Option<String>,
    #[serde(default)]
    pub requests: HashMap<String, Receipt>,
}

/// What a worker is blocked on, as the lead reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsInput {
    /// approval or question.
    pub kind: &'static str,
    pub request_id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub questions: Option<Value>,
}
/// Blocked workers by task ID, read from their sessions for each call.
pub type Needs = HashMap<String, NeedsInput>;

/// What the app must do to live sessions after a control call.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Stop this task's current turn.
    Stop(String),
    /// Stop this session's turn: the lead's, when a run stops.
    StopLead,
    Steer {
        task: String,
        text: String,
    },
    Approve {
        task: String,
        request: String,
        allow: bool,
    },
    Answer {
        task: String,
        request: String,
        /// Answers by question, in order; `None` skips.
        answers: Option<Value>,
    },
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    pub result: Value,
    pub effects: Vec<Effect>,
}
impl Reply {
    fn plain(result: Value) -> Self {
        Self {
            result,
            effects: Vec::new(),
        }
    }
}

/// How a worker's turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed,
}

/// What to do about the lead now that it is idle.
#[derive(Debug, PartialEq)]
pub enum Wake {
    Nothing,
    Send(String),
    /// The continuation limit was hit and the run paused.
    Paused(Vec<Effect>),
}

fn text(input: &Value, key: &str, max: usize) -> Result<String, String> {
    match input.get(key).and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() && value.chars().count() <= max => {
            Ok(value.trim().to_owned())
        }
        Some(value) if !value.trim().is_empty() => {
            Err(format!("Invalid {key}: keep it under {max} characters"))
        }
        _ => Err(format!("Invalid {key}: provide a non-empty string")),
    }
}
fn strings(input: &Value, key: &str, max: usize) -> Result<Vec<String>, String> {
    let Some(values) = input.get(key).and_then(Value::as_array) else {
        return Err(format!("Invalid {key}: provide an array of strings"));
    };
    if values.len() > max {
        return Err(format!("Invalid {key}: at most {max} entries"));
    }
    let mut seen = Vec::new();
    for value in values {
        let value = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 512)
            .ok_or_else(|| format!("Invalid {key}: provide an array of strings"))?;
        if !seen.iter().any(|item| item == value) {
            seen.push(value.to_owned());
        }
    }
    Ok(seen)
}
fn listed(values: &[String]) -> String {
    const MAX: usize = 12;
    if values.len() > MAX {
        format!(
            "{} (+{} more)",
            values[..MAX].join(", "),
            values.len() - MAX
        )
    } else {
        values.join(", ")
    }
}
fn check_fields(action: &str, input: &Value) -> Result<(), String> {
    let Some((_, allowed)) = FIELDS.iter().find(|(name, _)| *name == action) else {
        let names: Vec<_> = FIELDS.iter().map(|(name, _)| *name).collect();
        return Err(format!(
            "Unknown action \"{action}\". Use one of: {}. Run the control CLI with --help.",
            names.join(", ")
        ));
    };
    let unknown: Vec<_> = input
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Unknown {action} field{}: {}. {}",
        if unknown.len() > 1 { "s" } else { "" },
        unknown.join(", "),
        if allowed.is_empty() {
            format!("{action} takes no input.")
        } else {
            format!("{action} accepts: {}.", allowed.join(", "))
        }
    ))
}
fn tail(value: &str) -> String {
    let skip = value.chars().count().saturating_sub(MAX_RESULT);
    value.chars().skip(skip).collect()
}
fn recovery_turn(reason: &str) -> String {
    format!(
        "Continue the existing assignment from its retained worker checkout. The previous turn was stopped because the orchestration run was interrupted: {reason}\n\nInspect the current files and prior conversation before acting. Preserve completed work, do not repeat destructive or external operations, remain inside the assigned write scope, run the remaining focused checks, and report what was already done versus what you completed now."
    )
}
/// How long `wait` may block, from its input.
pub fn wait_seconds(input: &Value) -> Result<u64, String> {
    check_fields("wait", input)?;
    match input.get("timeoutSeconds") {
        None => Ok(20),
        Some(value) => value
            .as_f64()
            .filter(|seconds| (0.0..=MAX_WAIT_SECONDS as f64).contains(seconds))
            .map(|seconds| seconds.ceil() as u64)
            .ok_or_else(|| "timeoutSeconds must be 0 to 25".to_owned()),
    }
}

impl Run {
    pub fn new(plan: Plan, cwd: String, provider: String, allowed: Vec<Choice>) -> Self {
        let ids: HashMap<_, _> = plan
            .tasks
            .iter()
            .map(|task| (task.id.clone(), crate::pilot::new_id()))
            .collect();
        Self {
            cwd,
            provider,
            status: RunStatus::Active,
            allowed,
            max_workers: plan.max_workers,
            tasks: plan
                .tasks
                .into_iter()
                .map(|task| RunTask {
                    id: ids[&task.id].clone(),
                    session: None,
                    title: task.title,
                    prompt: task.prompt,
                    files: task.files,
                    depends_on: task.depends_on.iter().map(|id| ids[id].clone()).collect(),
                    harness: task.harness,
                    model: task.model,
                    model_settings: task.model_settings,
                    status: Status::Queued,
                    accepted: false,
                    result: String::new(),
                    error: None,
                    recovery_prompt: None,
                    delivered: true,
                    workspace: None,
                    swept: false,
                })
                .collect(),
            error: None,
            continuations: 0,
            last_pause_reason: None,
            requests: HashMap::new(),
        }
    }
    pub fn live(&self) -> bool {
        matches!(self.status, RunStatus::Active | RunStatus::Paused)
            || self.tasks.iter().any(|task| task.status.active())
    }
    pub fn task(&self, id: &str) -> Option<&RunTask> {
        self.tasks.iter().find(|task| task.id == id)
    }
    fn task_mut(&mut self, id: &str) -> Option<&mut RunTask> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }
    pub fn task_of_session(&self, session: i64) -> Option<&RunTask> {
        self.tasks.iter().find(|task| task.session == Some(session))
    }
    pub fn set_session(&mut self, task: &str, session: i64) {
        if let Some(task) = self.task_mut(task) {
            task.session = Some(session);
        }
    }
    pub fn set_workspace(&mut self, task: &str, workspace: Option<Workspace>) {
        if let Some(task) = self.task_mut(task) {
            task.workspace = workspace;
        }
    }
    /// Checkouts of work that ended without a result worth keeping: each is
    /// removed if it never changed, and otherwise stays for review.
    pub fn to_sweep(&mut self) -> Vec<(String, Workspace)> {
        let ended = !self.live();
        self.tasks
            .iter_mut()
            .filter(|task| {
                !task.swept
                    && !task.accepted
                    && (ended || matches!(task.status, Status::Failed | Status::Cancelled))
            })
            .filter_map(|task| {
                task.swept = true;
                Some((task.id.clone(), task.workspace.clone()?))
            })
            .collect()
    }
    /// Why this task can't be accepted, before any file moves.
    pub fn check_review(&self, input: &Value) -> Result<RunTask, String> {
        if self.status != RunStatus::Active {
            return Err(self.inactive_reason());
        }
        let target = self.target(input)?;
        if target.status != Status::Completed {
            return Err(format!(
                "Only a completed result can be accepted; {} is {}. {}",
                target.title,
                target.status.name(),
                if matches!(
                    target.status,
                    Status::Failed | Status::Blocked | Status::Interrupted
                ) {
                    "Send it another turn with message, retry it with corrected scope when necessary, or drop it with cancel."
                } else {
                    "Wait for it to finish, or cancel it."
                }
            ));
        }
        Ok(target)
    }
    /// This request ID already has its answer.
    pub fn answered(&self, request: &str) -> bool {
        self.requests.contains_key(request)
    }
    /// orchestrationSummary.ts: what the sidebar shows, without any prompt.
    pub fn summary(&self) -> Summary {
        Summary {
            live: self.live(),
            tasks: self
                .tasks
                .iter()
                .map(|task| SummaryTask {
                    session: task.session,
                    title: task.title.clone(),
                    provider: task
                        .harness
                        .clone()
                        .unwrap_or_else(|| self.provider.clone()),
                    model: task.model.clone(),
                    status: task.status.name().to_owned(),
                })
                .collect(),
        }
    }
    /// One line per task for the lead's card.
    pub fn progress(&self) -> String {
        self.tasks
            .iter()
            .map(|task| {
                let state = match task.status {
                    Status::Completed if task.accepted => "accepted",
                    Status::Completed => "ready for review",
                    Status::Queued => "waiting",
                    Status::Running => "working",
                    Status::Cancelling => "stopping",
                    status => status.name(),
                };
                format!("{}: {state}", task.title)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn waiting_for(&self, task: &RunTask) -> Option<String> {
        if task.status != Status::Queued {
            return None;
        }
        if let Some(dependency) = self
            .tasks
            .iter()
            .find(|entry| task.depends_on.contains(&entry.id) && !entry.accepted)
        {
            return Some(format!("Waiting for review: {}", dependency.title));
        }
        if let Some(owner) = self
            .tasks
            .iter()
            .find(|entry| entry.status.active() && overlaps(&entry.files, &task.files))
        {
            return Some(format!("Waiting for files: {}", owner.title));
        }
        Some("Waiting for a worker slot".into())
    }
    fn inactive_reason(&self) -> String {
        if self.status == RunStatus::Paused {
            format!(
                "This run is paused. {} list, get and wait remain available for inspection. Do not retry mutations or keep polling: explain the pause and ask the user to click Resume in Gespan. Resume will continue interrupted tasks from their retained worker checkouts; policy-blocked tasks remain stopped for an explicit retry or cancellation.",
                self.error.as_deref().unwrap_or("Work was interrupted.")
            )
        } else {
            format!(
                "This run is {}. Inspect results with list or get; do not keep retrying commands for this run.",
                match self.status {
                    RunStatus::Stopped => "stopped",
                    RunStatus::Finished => "finished",
                    _ => "active",
                }
            )
        }
    }
    fn task_view(&self, task: &RunTask, needs: &Needs) -> Value {
        let mut value = serde_json::to_value(task).unwrap_or_default();
        value["waitingFor"] = json!(self.waiting_for(task));
        value["needsInput"] = json!(needs.get(&task.id));
        value
    }
    /// The run as the lead reads it, without credentials or receipts.
    pub fn view(&self, needs: &Needs) -> Value {
        let tasks: Vec<_> = self
            .tasks
            .iter()
            .map(|task| {
                let mut value = self.task_view(task, needs);
                value.as_object_mut().map(|task| task.remove("prompt"));
                value
            })
            .collect();
        json!({
            "status": self.status,
            "maxWorkers": self.max_workers,
            "tasks": tasks,
            "error": self.error,
            "recovery": (self.status != RunStatus::Active).then(|| self.inactive_reason()),
        })
    }
    /// Input that arrived before `wait` is already actionable: only long-poll
    /// while every worker can still make progress without the lead.
    pub fn should_wait(&self, needs: &Needs) -> bool {
        self.status == RunStatus::Active
            && needs.is_empty()
            && self
                .tasks
                .iter()
                .any(|task| task.status.active() || task.status == Status::Queued)
    }

    /// Every task that can start now, marked running.
    pub fn dispatch(&mut self) -> Vec<String> {
        let mut started = Vec::new();
        if self.status != RunStatus::Active {
            return started;
        }
        for ix in 0..self.tasks.len() {
            if self.tasks.iter().filter(|t| t.status.active()).count() >= self.max_workers {
                break;
            }
            let task = &self.tasks[ix];
            let ready = task.status == Status::Queued
                && task
                    .depends_on
                    .iter()
                    .all(|id| self.task(id).is_some_and(|dependency| dependency.accepted))
                && !self
                    .tasks
                    .iter()
                    .any(|entry| entry.status.active() && overlaps(&entry.files, &task.files));
            if ready {
                started.push(task.id.clone());
                self.tasks[ix].status = Status::Running;
            }
        }
        started
    }
    /// A worker's turn ended. Anything but a running task ignores it.
    pub fn settle(&mut self, id: &str, outcome: Outcome, text: &str, error: Option<String>) {
        if let Some(task) = self.task_mut(id) {
            match task.status {
                Status::Running => {
                    task.status = match outcome {
                        Outcome::Completed => Status::Completed,
                        Outcome::Failed => Status::Failed,
                    };
                    task.result = tail(text);
                    task.error = error;
                    task.recovery_prompt = None;
                    task.delivered = false;
                    task.accepted = false;
                }
                Status::Cancelling => {
                    task.status = Status::Cancelled;
                    task.accepted = false;
                    task.delivered = false;
                    task.recovery_prompt = None;
                    if !text.is_empty() {
                        task.result = tail(text);
                    }
                }
                _ => (),
            }
        }
    }
    /// A paused run has no supervisor, so its agents stop with it. Queued work
    /// stays queued, and nothing dispatches until Resume.
    pub fn pause(&mut self, reason: &str) -> Vec<Effect> {
        if self.status != RunStatus::Active {
            return Vec::new();
        }
        self.status = RunStatus::Paused;
        self.error = Some(reason.to_owned());
        self.last_pause_reason = Some(reason.to_owned());
        let mut effects = Vec::new();
        for task in &mut self.tasks {
            if task.status == Status::Running {
                task.status = Status::Interrupted;
                task.accepted = false;
                task.delivered = true;
                task.error = Some(reason.to_owned());
                task.recovery_prompt = Some(recovery_turn(reason));
                effects.push(Effect::Stop(task.id.clone()));
            }
        }
        effects
    }
    pub fn stop(&mut self) -> Vec<Effect> {
        self.status = RunStatus::Stopped;
        let mut effects = vec![Effect::StopLead];
        for task in &mut self.tasks {
            if task.status.active() {
                effects.push(Effect::Stop(task.id.clone()));
            }
            if task.status.active() || task.status == Status::Queued {
                task.status = Status::Cancelled;
                task.accepted = false;
                task.delivered = true;
            }
        }
        effects
    }
    /// What the idle lead should hear: finished results, and blocked workers
    /// it hasn't been told about. `announced` remembers the latter.
    pub fn wake(&mut self, needs: &Needs, announced: &mut HashSet<String>) -> Wake {
        if self.status != RunStatus::Active {
            return Wake::Nothing;
        }
        let ready: Vec<usize> = (0..self.tasks.len())
            .filter(|ix| {
                let task = &self.tasks[*ix];
                !task.delivered && !task.status.active() && task.status != Status::Queued
            })
            .collect();
        let blocked: Vec<(&RunTask, &NeedsInput, String)> = self
            .tasks
            .iter()
            .filter_map(|task| {
                let pending = needs.get(&task.id)?;
                let key = format!("{}:{}", task.id, pending.request_id);
                (!announced.contains(&key)).then_some((task, pending, key))
            })
            .collect();
        if ready.is_empty() && blocked.is_empty() {
            return Wake::Nothing;
        }
        if self.continuations >= MAX_CONTINUATIONS {
            return Wake::Paused(self.pause(
                "Automatic continuation limit reached. Its agents were stopped; review and resume the run.",
            ));
        }
        let summary: Vec<_> = ready
            .iter()
            .map(|ix| {
                let task = &self.tasks[*ix];
                let result: String = {
                    let skip = task.result.chars().count().saturating_sub(4_000);
                    task.result.chars().skip(skip).collect()
                };
                format!(
                    "{} — {}: {}\n{}\n{result}",
                    task.id,
                    task.title,
                    task.status.name(),
                    task.error.as_deref().unwrap_or("")
                )
            })
            .collect();
        let asks: Vec<_> = blocked
            .iter()
            .map(|(task, pending, _)| {
                format!(
                    "{} — {} needs {} (requestId {}): {}\n{}{}",
                    task.id,
                    task.title,
                    if pending.kind == "approval" {
                        "an approval"
                    } else {
                        "an answer"
                    },
                    pending.request_id,
                    pending.label,
                    pending
                        .detail
                        .as_deref()
                        .map(|detail| detail.chars().take(2_000).collect::<String>())
                        .unwrap_or_default(),
                    pending
                        .questions
                        .as_ref()
                        .map(|questions| format!("\n{questions}"))
                        .unwrap_or_default()
                )
            })
            .collect();
        let mut body = Vec::new();
        if let Some(reason) = &self.last_pause_reason {
            body.push(format!("Previous interruption: {reason}\nInterrupted workers were continued from their retained checkouts. Inspect their results normally before review. Policy-blocked tasks were not restarted; correct or explicitly re-scope those tasks before retrying them. Dependent validation remains queued until its prerequisites pass review."));
        }
        if !summary.is_empty() {
            body.push(format!("Worker results are ready. Review the work, request corrections through the CLI when needed, and finish the original task.\n\n{}", summary.join("\n\n")));
        }
        if !asks.is_empty() {
            body.push(format!("These agents are blocked waiting on you. Decide each one with respond or answer; they stay stopped until you do. Judge it against the task you assigned, and ask the user in this conversation only when the call is genuinely theirs to make.\n\n{}", asks.join("\n\n")));
        }
        let keys: Vec<_> = blocked.iter().map(|(_, _, key)| key.clone()).collect();
        announced.extend(keys);
        self.continuations += 1;
        self.last_pause_reason = None;
        for ix in ready {
            self.tasks[ix].delivered = true;
        }
        Wake::Send(body.join("\n\n"))
    }

    /// One control call. Mutations and their retry receipts land together, so
    /// a request ID applies at most once.
    pub fn control(
        &mut self,
        request: &str,
        action: &str,
        input: &Value,
        needs: &Needs,
    ) -> Result<Reply, String> {
        check_fields(action, input)?;
        let signature = format!("{action}:{input}");
        if let Some(previous) = self.requests.get(request) {
            if previous.signature != signature {
                return Err("Request ID was already used with different input".into());
            }
            return Ok(Reply::plain(previous.result.clone()));
        }
        let read_only = matches!(action, "list" | "get");
        if !read_only && self.status != RunStatus::Active {
            return Err(self.inactive_reason());
        }
        if !read_only && self.requests.len() >= MAX_REQUESTS {
            return Err(
                "Run command limit reached. Stop the run and start another after reviewing the files."
                    .into(),
            );
        }
        let reply = self.apply(action, input, needs)?;
        if !read_only {
            self.requests.insert(
                request.to_owned(),
                Receipt {
                    signature,
                    result: reply.result.clone(),
                },
            );
        }
        Ok(reply)
    }
    fn target(&self, input: &Value) -> Result<RunTask, String> {
        let id = text(input, "taskId", 128)?;
        self.task(&id).cloned().ok_or_else(|| {
            "Task does not belong to this run. Use a taskId returned by delegate or list.".into()
        })
    }
    fn dependant_started(&self, id: &str) -> bool {
        self.tasks.iter().any(|entry| {
            entry.depends_on.iter().any(|dependency| dependency == id)
                && !matches!(entry.status, Status::Queued | Status::Cancelled)
        })
    }
    fn requeue(&mut self, id: &str, prompt: String) {
        if let Some(task) = self.task_mut(id) {
            task.prompt = prompt;
            task.status = Status::Queued;
            task.accepted = false;
            task.result = String::new();
            task.error = None;
            task.recovery_prompt = None;
            task.delivered = true;
            task.swept = false;
        }
    }
    fn correctable(&self, target: &RunTask, verb: &str) -> Result<(), String> {
        if target.status.active() || target.status == Status::Queued {
            return Err(format!(
                "Wait for this worker or cancel it before {verb}; {} is {}.",
                target.title,
                target.status.name()
            ));
        }
        if self.dependant_started(&target.id) {
            return Err(
                "A dependent task has already started; create a separate correction task after it finishes"
                    .into(),
            );
        }
        Ok(())
    }
    fn pending(
        &self,
        target: &RunTask,
        input: &Value,
        kind: &str,
        needs: &Needs,
    ) -> Result<NeedsInput, String> {
        let pending = needs.get(&target.id).filter(|pending| pending.kind == kind);
        let Some(pending) = pending else {
            let noun = if kind == "approval" {
                ("an approval", "responding")
            } else {
                ("a question", "answering")
            };
            return Err(format!(
                "{} is not waiting on {}. Read needsInput from list or wait before {}.",
                target.title, noun.0, noun.1
            ));
        };
        let given = match input.get("requestId") {
            Some(Value::String(id)) => id.clone(),
            Some(value) => value.to_string(),
            None => String::new(),
        };
        if given != pending.request_id {
            return Err(format!(
                "Stale requestId. {} is waiting on {}.",
                target.title, pending.request_id
            ));
        }
        Ok(pending.clone())
    }
    fn apply(&mut self, action: &str, input: &Value, needs: &Needs) -> Result<Reply, String> {
        match action {
            "list" => Ok(Reply::plain(
                json!({"run": self.view(needs), "harnesses": self.harnesses()}),
            )),
            "get" => {
                let target = self.target(input)?;
                let mut value = self.task_view(&target, needs);
                value["runStatus"] = json!(self.status);
                value["recovery"] =
                    json!((self.status != RunStatus::Active).then(|| self.inactive_reason()));
                Ok(Reply::plain(value))
            }
            "delegate" => self.delegate(input).map(Reply::plain),
            "message" => {
                let target = self.target(input)?;
                self.correctable(&target, "sending a new turn")?;
                let prompt = text(input, "text", 30_000)?;
                self.requeue(&target.id, prompt);
                Ok(Reply::plain(json!({"taskId":target.id,"status":"queued"})))
            }
            "retry" => {
                let target = self.target(input)?;
                self.correctable(&target, "retrying")?;
                let files = strings(input, "files", 64)?;
                if files.is_empty() {
                    return Err("Declare at least one corrected project-relative file/directory scope, or '.' for the whole checkout".into());
                }
                super::validate_scopes(&files)?;
                let prompt = text(input, "text", 30_000)?;
                self.requeue(&target.id, prompt);
                if let Some(task) = self.task_mut(&target.id) {
                    task.files = files.clone();
                }
                Ok(Reply::plain(
                    json!({"taskId":target.id,"status":"queued","files":files}),
                ))
            }
            "steer" => {
                let target = self.target(input)?;
                if target.status != Status::Running {
                    return Err(format!(
                        "Only a running agent can be steered; {} is {}. {}",
                        target.title,
                        target.status.name(),
                        if target.status == Status::Queued {
                            "It has not started yet, so edit it with message instead."
                        } else {
                            "Send it a fresh turn with message."
                        }
                    ));
                }
                Ok(Reply {
                    result: json!({"taskId":target.id,"steered":true}),
                    effects: vec![Effect::Steer {
                        task: target.id,
                        text: text(input, "text", 30_000)?,
                    }],
                })
            }
            "cancel" => {
                let target = self.target(input)?;
                let mut effects = Vec::new();
                if let Some(task) = self.task_mut(&target.id) {
                    if task.status.active() {
                        task.status = Status::Cancelling;
                        effects.push(Effect::Stop(target.id.clone()));
                    } else if task.status != Status::Cancelled {
                        task.status = Status::Cancelled;
                        task.accepted = false;
                        task.delivered = false;
                        task.recovery_prompt = None;
                    }
                }
                Ok(Reply {
                    result: json!({"cancelled":true}),
                    effects,
                })
            }
            "respond" => {
                let target = self.target(input)?;
                let pending = self.pending(&target, input, "approval", needs)?;
                let decision = text(input, "decision", 16)?;
                if decision != "allow" && decision != "deny" {
                    return Err("decision must be \"allow\" or \"deny\"".into());
                }
                Ok(Reply {
                    result: json!({"taskId":target.id,"decision":decision}),
                    effects: vec![Effect::Approve {
                        task: target.id,
                        request: pending.request_id,
                        allow: decision == "allow",
                    }],
                })
            }
            "answer" => {
                let target = self.target(input)?;
                let pending = self.pending(&target, input, "question", needs)?;
                let answers = if input["skip"] == true {
                    None
                } else {
                    Some(question_answers(input.get("answers"), &pending)?)
                };
                Ok(Reply {
                    result: json!({"taskId":target.id,"answered":answers.is_some()}),
                    effects: vec![Effect::Answer {
                        task: target.id,
                        request: pending.request_id,
                        answers,
                    }],
                })
            }
            "review" => {
                let target = self.check_review(input)?;
                if let Some(task) = self.task_mut(&target.id) {
                    task.accepted = true;
                }
                Ok(Reply::plain(json!({"accepted":true})))
            }
            "finish" => {
                let outstanding: Vec<_> = self
                    .tasks
                    .iter()
                    .filter(|task| task.status != Status::Cancelled && !task.accepted)
                    .map(|task| format!("{} ({})", task.title, task.status.name()))
                    .collect();
                if !outstanding.is_empty() {
                    return Err(format!(
                        "Review all remaining tasks before finishing. Outstanding: {}. Accept a completed task with review, correct a failed or blocked task with message/retry, or drop it with cancel.",
                        listed(&outstanding)
                    ));
                }
                self.status = RunStatus::Finished;
                Ok(Reply::plain(json!({"finished":true})))
            }
            _ => Err("Unknown action. Run control --help.".into()),
        }
    }
    fn harnesses(&self) -> Vec<Value> {
        let mut harnesses: Vec<(String, Vec<Value>)> = Vec::new();
        for choice in &self.allowed {
            let model = json!({"id":choice.model,"name":choice.name});
            match harnesses.iter_mut().find(|(id, _)| *id == choice.harness) {
                Some((_, models)) => models.push(model),
                None => harnesses.push((choice.harness.clone(), vec![model])),
            }
        }
        harnesses
            .into_iter()
            .map(|(harness, models)| json!({"harness":harness,"models":models}))
            .collect()
    }
    fn delegate(&mut self, input: &Value) -> Result<Value, String> {
        if self.tasks.len() >= MAX_TASKS {
            return Err("This run has reached its 40-task limit".into());
        }
        let harness = text(input, "harness", 32)?;
        let permitted: Vec<_> = self
            .allowed
            .iter()
            .filter(|choice| choice.harness == harness)
            .collect();
        if permitted.is_empty() {
            let mut allowed: Vec<_> = self.allowed.iter().map(|c| c.harness.clone()).collect();
            allowed.dedup();
            return Err(format!(
                "Harness \"{harness}\" is not allowed in this run. Allowed: {}.",
                listed(&allowed)
            ));
        }
        let model = match input.get("model") {
            None | Some(Value::Null) => permitted[0].model.clone(),
            Some(_) => text(input, "model", 256)?,
        };
        if !permitted.iter().any(|choice| choice.model == model) {
            let ids: Vec<_> = permitted.iter().map(|c| c.model.clone()).collect();
            return Err(format!(
                "Choose a model ID returned by list for {harness}: {}.",
                listed(&ids)
            ));
        }
        let title = text(input, "title", 160)?;
        let prompt = text(input, "prompt", 30_000)?;
        let files = strings(input, "files", 64)?;
        if files.is_empty() {
            return Err("Declare at least one file/directory scope in files, or '.' for exclusive checkout access".into());
        }
        super::validate_scopes(&files)?;
        let depends_on = if input.get("dependsOn").is_some() {
            strings(input, "dependsOn", MAX_TASKS)?
        } else {
            Vec::new()
        };
        let missing: Vec<_> = depends_on
            .iter()
            .filter(|id| {
                !self
                    .tasks
                    .iter()
                    .any(|task| task.id == **id && task.status != Status::Cancelled)
            })
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "dependsOn must hold taskIds from this run; unknown or cancelled: {}.",
                listed(&missing)
            ));
        }
        let id = crate::pilot::new_id();
        self.tasks.push(RunTask {
            id: id.clone(),
            session: None,
            title,
            prompt,
            files,
            depends_on,
            harness: Some(harness),
            model: Some(model),
            model_settings: BTreeMap::new(),
            status: Status::Queued,
            accepted: false,
            result: String::new(),
            error: None,
            recovery_prompt: None,
            delivered: true,
            workspace: None,
            swept: false,
        });
        Ok(json!({"taskId":id,"status":"queued"}))
    }
}

/// The lead's option IDs as the worker's question takes them: for each
/// question in order, the chosen options' labels.
fn question_answers(value: Option<&Value>, pending: &NeedsInput) -> Result<Value, String> {
    let bad = || "answers must be an object of questionId -> [optionId], or pass {\"skip\":true}";
    let answers = value.and_then(Value::as_object).ok_or_else(bad)?;
    let questions = pending
        .questions
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(unknown) = answers
        .keys()
        .find(|id| !questions.iter().any(|question| question["id"] == **id))
    {
        let ids: Vec<_> = questions
            .iter()
            .filter_map(|question| question["id"].as_str().map(str::to_owned))
            .collect();
        return Err(format!(
            "Unknown question \"{unknown}\". Ask for: {}.",
            listed(&ids)
        ));
    }
    if answers.is_empty() {
        return Err("Answer at least one question, or pass {\"skip\":true}".into());
    }
    let mut chosen = Vec::new();
    for question in &questions {
        let id = question["id"].as_str().unwrap_or_default();
        let picked = answers.get(id).map_or(Ok(Vec::new()), |picked| {
            strings(&json!({ id: picked }), id, 16)
        })?;
        let options = question["options"].as_array().cloned().unwrap_or_default();
        let mut labels = Vec::new();
        for option in picked {
            match options.iter().find(|entry| entry["id"] == *option.as_str()) {
                Some(entry) => labels.push(entry["label"].clone()),
                None if question["allowCustom"] == true => labels.push(json!(option)),
                None => {
                    let ids: Vec<_> = options
                        .iter()
                        .filter_map(|entry| entry["id"].as_str().map(str::to_owned))
                        .collect();
                    return Err(format!(
                        "Unknown option for \"{id}\": {option}. Choose from: {}.",
                        listed(&ids)
                    ));
                }
            }
        }
        chosen.push(labels);
    }
    Ok(json!(chosen))
}

#[cfg(test)]
mod tests;
