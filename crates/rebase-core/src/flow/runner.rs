//! The Flow state machine. Step-driven: every method returns the `Effect`s the
//! host must apply, and the host reports session results back.
use super::contract::{contract_footer, parse_outputs, retry_prompt, select_transition};
use super::definition::{
    outputs_of, validate_flow, Flow, FlowState, OutputValue, VisitResult, FAILED, SUCCEEDED,
};
use super::run::{FlowRun, FlowVisit, RunStatus, VisitStatus};
use super::template::{render_template, resolve_inputs, BindingContext};
use std::collections::{HashMap, HashSet};

pub const MAX_RUN_STEPS: u32 = 50;
const INTERRUPTED: &str = "Interrupted by app restart";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub run_id: String,
    pub visit_id: String,
    pub harness: String,
    pub model: String,
    pub cwd: String,
    pub worktree_cwd: Option<String>,
    pub title: String,
    pub prompt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentOutcome {
    pub status: AgentStatus,
    pub text: String,
    pub error: Option<String>,
}

/// What the host must do. Apply in order: `Persist` always precedes the
/// launch, follow-up, cancel or approval it records, and `Finished` is last.
// `Persist` carries the run by value, as the host contract specifies; effects
// are short-lived, so the variant size gap does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Launch(LaunchRequest),
    FollowUp {
        session_id: String,
        prompt: String,
    },
    Cancel {
        session_id: String,
    },
    Persist {
        run: FlowRun,
        visit: Option<FlowVisit>,
    },
    NeedsApproval {
        run_id: String,
        visit_id: String,
    },
    Finished {
        run_id: String,
        status: RunStatus,
    },
}

/// A run the runner is driving: its record and every visit, in seq order.
struct ActiveRun {
    run: FlowRun,
    visits: Vec<FlowVisit>,
}

impl ActiveRun {
    fn persist(&self, with_visit: bool) -> Effect {
        Effect::Persist {
            run: self.run.clone(),
            visit: if with_visit {
                self.visits.last().cloned()
            } else {
                None
            },
        }
    }

    fn state(&self, id: &str) -> Option<FlowState> {
        self.run.flow.states.iter().find(|s| s.id() == id).cloned()
    }

    /// The current visit, if it is `visit_id` and still has `status`.
    fn current_mut(&mut self, visit_id: &str, status: VisitStatus) -> Option<&mut FlowVisit> {
        self.visits
            .last_mut()
            .filter(|v| v.id == visit_id && v.status == status)
    }

    /// Outputs of the latest completed visit of each state.
    fn latest_outputs(&self) -> HashMap<String, HashMap<String, OutputValue>> {
        let mut latest = HashMap::new();
        for v in &self.visits {
            if v.status == VisitStatus::Completed {
                if let Some(outputs) = &v.outputs {
                    latest.insert(v.state_id.clone(), outputs.clone());
                }
            }
        }
        latest
    }
}

/// Owns every state transition of every active run. The host only applies
/// the returned effects and reports session events back.
///
/// Host contract: call `on_session` before `on_agent_settled` for a visit;
/// call `on_worktree` before `on_agent_settled` for the visit that created
/// the worktree; and deliver exactly one `on_agent_settled` per agent turn
/// (the initial turn, and the contract-retry turn if there is one).
pub struct Runner {
    clock: Box<dyn Fn() -> i64>,
    ids: Box<dyn FnMut() -> String>,
    runs: HashMap<String, ActiveRun>,
    /// `(run_id, visit_id)` of visits cancelled before their session was
    /// reported; a late `on_session` for one stops that session.
    cancelled_unreported: HashSet<(String, String)>,
}

impl Runner {
    pub fn new(clock: impl Fn() -> i64 + 'static, ids: impl FnMut() -> String + 'static) -> Self {
        Self {
            clock: Box::new(clock),
            ids: Box::new(ids),
            runs: HashMap::new(),
            cancelled_unreported: HashSet::new(),
        }
    }

    /// Whether a run of `flow_id` is running or waiting for approval here,
    /// including one whose records have not been written yet.
    pub fn has_active(&self, flow_id: &str) -> bool {
        self.runs
            .values()
            .any(|active| active.run.flow_id == flow_id)
    }

    /// Validates `flow`, resolves its inputs and enters the initial state.
    /// The returned run is the run as it stands after these effects.
    pub fn start(
        &mut self,
        flow: &Flow,
        given: &HashMap<String, String>,
    ) -> Result<(FlowRun, Vec<Effect>), Vec<String>> {
        let validation = validate_flow(flow);
        if !validation.errors.is_empty() {
            return Err(validation.errors);
        }
        let inputs = resolve_inputs(flow, given)?;
        let run = FlowRun {
            id: (self.ids)(),
            flow_id: flow.id.clone(),
            flow: flow.clone(),
            status: RunStatus::Running,
            inputs,
            worktree_cwd: None,
            current_state: Some(flow.initial.clone()),
            error: None,
            created_at: (self.clock)(),
            finished_at: None,
        };
        let mut active = ActiveRun {
            run,
            visits: Vec::new(),
        };
        let mut effects = vec![active.persist(false)];
        effects.extend(self.enter_state(&mut active, &flow.initial));
        let run = active.run.clone();
        self.keep(active);
        Ok((run, effects))
    }

    /// The host launched the session for `visit_id`.
    pub fn on_session(&mut self, run_id: &str, visit_id: &str, session_id: &str) -> Vec<Effect> {
        let key = (run_id.to_owned(), visit_id.to_owned());
        if self.cancelled_unreported.remove(&key) {
            return vec![Effect::Cancel {
                session_id: session_id.to_owned(),
            }];
        }
        self.with_run(run_id, |_, active| {
            let Some(visit) = active.current_mut(visit_id, VisitStatus::Running) else {
                return vec![];
            };
            if visit.session_id.as_deref() == Some(session_id) {
                return vec![];
            }
            visit.session_id = Some(session_id.to_owned());
            vec![active.persist(true)]
        })
    }

    /// The first visit created the run's worktree. Later reports are ignored.
    pub fn on_worktree(&mut self, run_id: &str, path: &str) -> Vec<Effect> {
        self.with_run(run_id, |_, active| {
            if active.run.worktree_cwd.is_some() {
                return vec![];
            }
            active.run.worktree_cwd = Some(path.to_owned());
            vec![active.persist(false)]
        })
    }

    pub fn on_agent_settled(
        &mut self,
        run_id: &str,
        visit_id: &str,
        outcome: AgentOutcome,
    ) -> Vec<Effect> {
        self.with_run(run_id, |runner, active| {
            let now = (runner.clock)();
            let Some(visit) = active.current_mut(visit_id, VisitStatus::Running) else {
                return vec![];
            };
            let state_id = visit.state_id.clone();
            match outcome.status {
                AgentStatus::Cancelled => {
                    visit.status = VisitStatus::Cancelled;
                    visit.finished_at = Some(now);
                    runner.finish(active, RunStatus::Cancelled, None, true)
                }
                AgentStatus::Failed => {
                    let error = outcome.error.unwrap_or_else(|| "Agent failed".into());
                    visit.status = VisitStatus::Failed;
                    visit.error = Some(error.clone());
                    visit.finished_at = Some(now);
                    runner.advance(active, VisitResult::Failed, &HashMap::new(), error)
                }
                AgentStatus::Completed => {
                    let Some(state) = active.state(&state_id) else {
                        let error = format!("Unknown state {state_id}");
                        let visit = active.visits.last_mut().expect("current visit");
                        visit.status = VisitStatus::Failed;
                        visit.error = Some(error.clone());
                        visit.finished_at = Some(now);
                        return runner.finish(active, RunStatus::Failed, Some(error), true);
                    };
                    let declared = outputs_of(&state);
                    let parsed = if declared.is_empty() {
                        Ok(HashMap::new())
                    } else {
                        parse_outputs(&outcome.text, &declared)
                    };
                    let visit = active.visits.last_mut().expect("current visit");
                    match parsed {
                        Ok(values) => {
                            visit.status = VisitStatus::Completed;
                            visit.outputs = Some(values.clone());
                            visit.finished_at = Some(now);
                            let error = format!("No transition matched from {state_id}");
                            runner.advance(active, VisitResult::Completed, &values, error)
                        }
                        Err(errors) => match visit.session_id.clone() {
                            Some(session_id) if !visit.retried => {
                                visit.retried = true;
                                vec![
                                    active.persist(true),
                                    Effect::FollowUp {
                                        session_id,
                                        prompt: retry_prompt(&errors),
                                    },
                                ]
                            }
                            _ => {
                                let error =
                                    format!("Output contract not met: {}", errors.join("; "));
                                visit.status = VisitStatus::Failed;
                                visit.error = Some(error.clone());
                                visit.finished_at = Some(now);
                                runner.advance(active, VisitResult::Failed, &HashMap::new(), error)
                            }
                        },
                    }
                }
            }
        })
    }

    /// Decides the run's pending approval; the note is the visit's `note` output.
    pub fn resolve_approval(&mut self, run_id: &str, approved: bool, note: &str) -> Vec<Effect> {
        self.with_run(run_id, |runner, active| {
            let now = (runner.clock)();
            if active.run.status != RunStatus::WaitingApproval {
                return vec![];
            }
            let Some(visit) = active.visits.last_mut() else {
                return vec![];
            };
            if visit.status != VisitStatus::WaitingApproval {
                return vec![];
            }
            let values = HashMap::from([
                ("approved".to_string(), OutputValue::Bool(approved)),
                ("note".to_string(), OutputValue::String(note.to_owned())),
            ]);
            visit.status = VisitStatus::Completed;
            visit.outputs = Some(values.clone());
            visit.note = Some(note.to_owned());
            visit.finished_at = Some(now);
            let error = format!("No transition matched from {}", visit.state_id);
            active.run.status = RunStatus::Running;
            runner.advance(active, VisitResult::Completed, &values, error)
        })
    }

    pub fn cancel(&mut self, run_id: &str) -> Vec<Effect> {
        let Some(mut active) = self.runs.remove(run_id) else {
            return vec![];
        };
        let now = (self.clock)();
        let mut session = None;
        let mut visit_changed = false;
        if let Some(visit) = active.visits.last_mut() {
            if matches!(
                visit.status,
                VisitStatus::Running | VisitStatus::WaitingApproval
            ) {
                if visit.status == VisitStatus::Running {
                    session = visit.session_id.clone();
                    if session.is_none() {
                        self.cancelled_unreported
                            .insert((active.run.id.clone(), visit.id.clone()));
                    }
                }
                visit.status = VisitStatus::Cancelled;
                visit.finished_at = Some(now);
                visit_changed = true;
            }
        }
        active.run.status = RunStatus::Cancelled;
        active.run.finished_at = Some(now);
        let mut effects = vec![active.persist(visit_changed)];
        if let Some(session_id) = session {
            effects.push(Effect::Cancel { session_id });
        }
        effects.push(Effect::Finished {
            run_id: active.run.id.clone(),
            status: RunStatus::Cancelled,
        });
        effects
    }

    /// Re-enters `state_id` of a failed or cancelled run, appending a visit.
    pub fn resume(
        &mut self,
        mut run: FlowRun,
        mut visits: Vec<FlowVisit>,
        state_id: &str,
    ) -> Vec<Effect> {
        if !matches!(run.status, RunStatus::Failed | RunStatus::Cancelled)
            || self.runs.contains_key(&run.id)
            || !run.flow.states.iter().any(|s| s.id() == state_id)
        {
            return vec![];
        }
        run.status = RunStatus::Running;
        run.error = None;
        run.finished_at = None;
        run.current_state = Some(state_id.to_owned());
        visits.sort_by_key(|v| v.seq);
        let mut active = ActiveRun { run, visits };
        let mut effects = vec![active.persist(false)];
        effects.extend(self.enter_state(&mut active, state_id));
        self.keep(active);
        effects
    }

    /// Loads the active runs found at startup. Running visits were cut off by
    /// the restart, so they and their runs fail (Persist, then Finished);
    /// pending approvals resume.
    pub fn recover(&mut self, active: Vec<(FlowRun, Vec<FlowVisit>)>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (run, mut visits) in active {
            if !run.status.is_active() {
                continue;
            }
            visits.sort_by_key(|v| v.seq);
            let mut active = ActiveRun { run, visits };
            let now = (self.clock)();
            let waiting = active
                .visits
                .last()
                .is_some_and(|v| v.status == VisitStatus::WaitingApproval);
            let mut interrupted = Vec::new();
            for v in &mut active.visits {
                if v.status == VisitStatus::Running {
                    v.status = VisitStatus::Failed;
                    v.error = Some(INTERRUPTED.into());
                    v.finished_at = Some(now);
                    interrupted.push(v.clone());
                }
            }
            if waiting && interrupted.is_empty() {
                if active.run.status != RunStatus::WaitingApproval {
                    active.run.status = RunStatus::WaitingApproval;
                    effects.push(active.persist(false));
                }
                self.keep(active);
                continue;
            }
            // Nothing can settle this run any more: fail it.
            active.run.status = RunStatus::Failed;
            active.run.error = Some(INTERRUPTED.into());
            active.run.finished_at = Some(now);
            if interrupted.is_empty() {
                effects.push(active.persist(false));
            }
            for visit in interrupted {
                effects.push(Effect::Persist {
                    run: active.run.clone(),
                    visit: Some(visit),
                });
            }
            effects.push(Effect::Finished {
                run_id: active.run.id.clone(),
                status: RunStatus::Failed,
            });
        }
        effects
    }

    // ---- internals -----------------------------------------------------

    /// Runs `f` on an in-memory run; finished runs are dropped afterwards.
    fn with_run(
        &mut self,
        run_id: &str,
        f: impl FnOnce(&mut Self, &mut ActiveRun) -> Vec<Effect>,
    ) -> Vec<Effect> {
        let Some(mut active) = self.runs.remove(run_id) else {
            return vec![];
        };
        let effects = f(self, &mut active);
        self.keep(active);
        effects
    }

    fn keep(&mut self, active: ActiveRun) {
        if active.run.status.is_active() {
            self.runs.insert(active.run.id.clone(), active);
        }
    }

    /// Ends the run. `with_visit` persists the current visit alongside it.
    fn finish(
        &mut self,
        active: &mut ActiveRun,
        status: RunStatus,
        error: Option<String>,
        with_visit: bool,
    ) -> Vec<Effect> {
        active.run.status = status;
        active.run.error = error;
        active.run.finished_at = Some((self.clock)());
        vec![
            active.persist(with_visit),
            Effect::Finished {
                run_id: active.run.id.clone(),
                status,
            },
        ]
    }

    /// Follows the transition chosen for the just-settled current visit.
    /// `unmatched` is the run's error when no transition matches.
    fn advance(
        &mut self,
        active: &mut ActiveRun,
        result: VisitResult,
        values: &HashMap<String, OutputValue>,
        unmatched: String,
    ) -> Vec<Effect> {
        let state_id = active
            .visits
            .last()
            .map(|v| v.state_id.clone())
            .unwrap_or_default();
        let target = active
            .state(&state_id)
            .and_then(|s| select_transition(&s, result, values).map(|t| t.to.clone()));
        match target.as_deref() {
            None => self.finish(active, RunStatus::Failed, Some(unmatched), true),
            Some(SUCCEEDED) => self.finish(active, RunStatus::Succeeded, None, true),
            Some(FAILED) => {
                let error = format!("Reached {FAILED} from {state_id}");
                self.finish(active, RunStatus::Failed, Some(error), true)
            }
            Some(next) => {
                let mut effects = vec![active.persist(true)];
                effects.extend(self.enter_state(active, next));
                effects
            }
        }
    }

    /// Creates the next visit of `state_id`, after the visit and step caps.
    fn enter_state(&mut self, active: &mut ActiveRun, state_id: &str) -> Vec<Effect> {
        let Some(state) = active.state(state_id) else {
            let error = format!("Unknown state {state_id}");
            return self.finish(active, RunStatus::Failed, Some(error), false);
        };
        let max = state.max_visits();
        let visited = active
            .visits
            .iter()
            .filter(|v| v.state_id == state_id)
            .count();
        if visited >= usize::from(max) {
            let error = format!("State {state_id} exceeded {max} visits");
            return self.finish(active, RunStatus::Failed, Some(error), false);
        }
        if active.visits.len() >= MAX_RUN_STEPS as usize {
            let error = format!("Run exceeded {MAX_RUN_STEPS} steps");
            return self.finish(active, RunStatus::Failed, Some(error), false);
        }

        let template = match &state {
            FlowState::Agent(s) => &s.brief,
            FlowState::Approval(s) => &s.message,
        };
        let outputs = active.latest_outputs();
        let rendered = render_template(
            template,
            &BindingContext {
                inputs: &active.run.inputs,
                outputs: &outputs,
            },
        );
        let now = (self.clock)();
        let seq = active.visits.iter().map(|v| v.seq).max().unwrap_or(0) + 1;
        let mut visit = FlowVisit {
            id: (self.ids)(),
            run_id: active.run.id.clone(),
            seq,
            state_id: state_id.to_owned(),
            status: VisitStatus::Running,
            prompt: None,
            session_id: None,
            outputs: None,
            error: None,
            retried: false,
            note: None,
            started_at: now,
            finished_at: None,
        };
        active.run.current_state = Some(state_id.to_owned());

        let text = match rendered {
            Ok(text) => text,
            Err(errors) => {
                let error = errors.join("; ");
                visit.status = VisitStatus::Failed;
                visit.error = Some(error.clone());
                visit.finished_at = Some(now);
                active.visits.push(visit);
                return self.finish(active, RunStatus::Failed, Some(error), true);
            }
        };

        match &state {
            FlowState::Agent(agent) => {
                let footer = contract_footer(&agent.outputs);
                let prompt = if footer.is_empty() {
                    text
                } else {
                    format!("{text}\n\n{footer}")
                };
                visit.prompt = Some(prompt.clone());
                active.run.status = RunStatus::Running;
                let launch = LaunchRequest {
                    run_id: active.run.id.clone(),
                    visit_id: visit.id.clone(),
                    harness: agent.harness.clone(),
                    model: agent.model.clone(),
                    cwd: active.run.flow.cwd.clone(),
                    worktree_cwd: active.run.worktree_cwd.clone(),
                    title: format!("{} · {}", active.run.flow.name, agent.title),
                    prompt,
                };
                active.visits.push(visit);
                vec![active.persist(true), Effect::Launch(launch)]
            }
            FlowState::Approval(_) => {
                visit.status = VisitStatus::WaitingApproval;
                visit.prompt = Some(text);
                active.run.status = RunStatus::WaitingApproval;
                let needs = Effect::NeedsApproval {
                    run_id: active.run.id.clone(),
                    visit_id: visit.id.clone(),
                };
                active.visits.push(visit);
                vec![active.persist(true), needs]
            }
        }
    }
}

#[cfg(test)]
mod tests;
