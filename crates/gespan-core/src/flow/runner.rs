//! The Flow state machine. Step-driven: every method returns the `Effect`s the
//! host must apply, and the host reports session results back.
use super::contract::{
    branch_runs, contract_footer, parse_outputs, retry_prompt, select_transition,
};
use super::definition::{
    outputs_of, validate_flow, Flow, FlowState, OutputValue, ParallelState, VisitResult, FAILED,
    SUCCEEDED,
};
use super::run::{FlowRun, FlowVisit, RunEdit, RunStatus, RunTag, VisitStatus};
use super::template::{render_template, resolve_inputs, BindingContext};
use std::collections::{HashMap, HashSet};

pub const MAX_RUN_STEPS: u32 = 50;
/// The error `recover` gives a visit (and its run) cut off by a restart.
pub const INTERRUPTED: &str = "Interrupted by app restart";
const IDENTITY_EDIT: &str =
    "A run edit cannot change the Flow's id, owner, initial state, access, folder or inputs";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub run_id: String,
    pub visit_id: String,
    pub harness: String,
    pub model: String,
    pub cwd: String,
    pub worktree_cwd: Option<String>,
    /// The run snapshot's `AccessMode` id; every visit of a run shares it.
    pub access: String,
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
/// launch, follow-up, cancel or approval it records, and `Finished` follows
/// them. `VisitFinished` reports come after everything else, so the next
/// `Launch` never waits on whoever listens for them.
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
    /// An agent visit ended (completed, failed or cancelled by its session),
    /// or recovery failed it. `run` is the run as it stands after the step.
    /// Not emitted for `Runner::cancel` or a contract retry.
    VisitFinished {
        run: FlowRun,
        visit: FlowVisit,
    },
}

/// A run the runner is driving: its record and every visit, in seq order.
struct ActiveRun {
    run: FlowRun,
    visits: Vec<FlowVisit>,
}

impl ActiveRun {
    /// Visits of `state_id` that count against its `maxVisits`: every one,
    /// or for a parallel step's branch, those since its step last started,
    /// so each run of the step gives a branch its full tries.
    fn visits_of(&self, state_id: &str) -> usize {
        let since = match self.run.flow.parent_of(state_id) {
            Some(parent) => self
                .visits
                .iter()
                .rev()
                .find(|v| v.state_id == parent.id)
                .map_or(0, |v| v.seq),
            None => 0,
        };
        self.visits
            .iter()
            .filter(|v| v.state_id == state_id && v.seq > since)
            .count()
    }

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

    /// Persists the run with visit `visit_id`.
    fn persist_visit(&self, visit_id: &str) -> Effect {
        Effect::Persist {
            run: self.run.clone(),
            visit: self.visits.iter().find(|v| v.id == visit_id).cloned(),
        }
    }

    /// Visit `visit_id`, if it still has `status`. Several visits are in
    /// flight while a parallel step's branches run.
    fn visit_mut(&mut self, visit_id: &str, status: VisitStatus) -> Option<&mut FlowVisit> {
        self.visits
            .iter_mut()
            .rev()
            .find(|v| v.id == visit_id && v.status == status)
    }

    fn visit(&mut self, visit_id: &str) -> &mut FlowVisit {
        self.visits
            .iter_mut()
            .rev()
            .find(|v| v.id == visit_id)
            .expect("visit of this run")
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
        tag: Option<RunTag>,
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
            tag,
            edits: vec![],
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
            let Some(visit) = active.visit_mut(visit_id, VisitStatus::Running) else {
                return vec![];
            };
            if visit.session_id.as_deref() == Some(session_id) {
                return vec![];
            }
            visit.session_id = Some(session_id.to_owned());
            vec![active.persist_visit(visit_id)]
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
            if active.visit_mut(visit_id, VisitStatus::Running).is_none() {
                return vec![];
            }
            let mut effects = runner.settle(active, visit_id, outcome);
            // The visit ended unless a contract retry keeps it running.
            if let Some(visit) = active
                .visits
                .iter()
                .find(|v| v.id == visit_id && v.status != VisitStatus::Running)
            {
                effects.push(Effect::VisitFinished {
                    run: active.run.clone(),
                    visit: visit.clone(),
                });
            }
            effects
        })
    }

    /// Records the outcome on running visit `visit_id` and follows its
    /// transition, or, for a parallel step's branch, joins it.
    fn settle(
        &mut self,
        active: &mut ActiveRun,
        visit_id: &str,
        outcome: AgentOutcome,
    ) -> Vec<Effect> {
        let now = (self.clock)();
        let Some(visit) = active.visit_mut(visit_id, VisitStatus::Running) else {
            return vec![];
        };
        let state_id = visit.state_id.clone();
        match outcome.status {
            AgentStatus::Cancelled => {
                visit.status = VisitStatus::Cancelled;
                visit.finished_at = Some(now);
                self.finish(active, RunStatus::Cancelled, None, Some(visit_id))
            }
            AgentStatus::Failed => {
                let error = outcome.error.unwrap_or_else(|| "Agent failed".into());
                visit.status = VisitStatus::Failed;
                visit.error = Some(error.clone());
                visit.finished_at = Some(now);
                self.after_visit(
                    active,
                    visit_id,
                    VisitResult::Failed,
                    &HashMap::new(),
                    error,
                )
            }
            AgentStatus::Completed => {
                let Some(state) = active.state(&state_id) else {
                    let error = format!("Unknown state {state_id}");
                    let visit = active.visit(visit_id);
                    visit.status = VisitStatus::Failed;
                    visit.error = Some(error.clone());
                    visit.finished_at = Some(now);
                    return self.finish(active, RunStatus::Failed, Some(error), Some(visit_id));
                };
                let declared = outputs_of(&state);
                let parsed = if declared.is_empty() {
                    Ok(HashMap::new())
                } else {
                    parse_outputs(&outcome.text, &declared)
                };
                let visit = active.visit(visit_id);
                match parsed {
                    Ok(values) => {
                        visit.status = VisitStatus::Completed;
                        visit.outputs = Some(values.clone());
                        visit.finished_at = Some(now);
                        let error = format!("No transition matched from {state_id}");
                        self.after_visit(active, visit_id, VisitResult::Completed, &values, error)
                    }
                    Err(errors) => match visit.session_id.clone() {
                        Some(session_id) if !visit.retried => {
                            visit.retried = true;
                            vec![
                                active.persist_visit(visit_id),
                                Effect::FollowUp {
                                    session_id,
                                    prompt: retry_prompt(&errors),
                                },
                            ]
                        }
                        _ => {
                            let error = format!("Output contract not met: {}", errors.join("; "));
                            visit.status = VisitStatus::Failed;
                            visit.error = Some(error.clone());
                            visit.finished_at = Some(now);
                            self.after_visit(
                                active,
                                visit_id,
                                VisitResult::Failed,
                                &HashMap::new(),
                                error,
                            )
                        }
                    },
                }
            }
        }
    }

    /// Visit `visit_id` ended: a branch joins its parallel step; any other
    /// step follows its transition.
    fn after_visit(
        &mut self,
        active: &mut ActiveRun,
        visit_id: &str,
        result: VisitResult,
        values: &HashMap<String, OutputValue>,
        error: String,
    ) -> Vec<Effect> {
        let state_id = active.visit(visit_id).state_id.clone();
        match active.run.flow.parent_of(&state_id).cloned() {
            Some(parent) => self.join(active, &parent, visit_id, result, error),
            None => self.advance(active, &state_id, visit_id, result, values, error),
        }
    }

    /// Branch visit `visit_id` of `parent` ended. A failed branch with
    /// visits left runs again; one without fails the parallel step and stops
    /// the other branches. Once every branch has completed, the parallel step
    /// completes and follows its transition.
    fn join(
        &mut self,
        active: &mut ActiveRun,
        parent: &ParallelState,
        visit_id: &str,
        result: VisitResult,
        error: String,
    ) -> Vec<Effect> {
        let mut effects = vec![active.persist_visit(visit_id)];
        let Some(step) = active
            .visits
            .iter()
            .rposition(|v| v.state_id == parent.id && v.status == VisitStatus::Running)
        else {
            return effects;
        };
        let step_id = active.visits[step].id.clone();
        let step_seq = active.visits[step].seq;
        let branch = active.visit(visit_id).state_id.clone();
        if result == VisitResult::Failed {
            let used = active.visits_of(&branch);
            let max = active.state(&branch).map_or(0, |s| s.max_visits());
            if used < usize::from(max) {
                effects.extend(self.enter(active, &branch, Some(&error)));
                if active.run.status.is_active() {
                    active.run.current_state = Some(parent.id.clone());
                }
                return effects;
            }
            let title = active
                .state(&branch)
                .map_or(branch.clone(), |s| s.title().to_owned());
            let failure = format!("{title} failed: {error}");
            let now = (self.clock)();
            let step = active.visit(&step_id);
            step.status = VisitStatus::Failed;
            step.error = Some(failure.clone());
            step.finished_at = Some(now);
            effects.extend(self.stop_others(active, step_seq, None));
            effects.extend(self.advance(
                active,
                &parent.id,
                &step_id,
                VisitResult::Failed,
                &HashMap::new(),
                failure,
            ));
            return effects;
        }
        let waiting = active
            .visits
            .iter()
            .any(|v| v.seq > step_seq && v.status == VisitStatus::Running);
        if waiting {
            return effects;
        }
        let now = (self.clock)();
        let step = active.visit(&step_id);
        step.status = VisitStatus::Completed;
        step.finished_at = Some(now);
        let error = format!("No transition matched from {}", parent.id);
        effects.extend(self.advance(
            active,
            &parent.id,
            &step_id,
            VisitResult::Completed,
            &HashMap::new(),
            error,
        ));
        effects
    }

    /// Cancels every visit still in flight after `after_seq`, except
    /// `keep`: the other branches of a parallel step that failed or of a run
    /// that ended. Persists each, then stops its session.
    fn stop_others(
        &mut self,
        active: &mut ActiveRun,
        after_seq: u32,
        keep: Option<&str>,
    ) -> Vec<Effect> {
        let now = (self.clock)();
        let mut stopped = Vec::new();
        let mut sessions = Vec::new();
        for v in active.visits.iter_mut().filter(|v| v.seq > after_seq) {
            if Some(v.id.as_str()) == keep
                || !matches!(
                    v.status,
                    VisitStatus::Running | VisitStatus::WaitingApproval
                )
            {
                continue;
            }
            let agent = active
                .run
                .flow
                .state(&v.state_id)
                .is_some_and(|s| matches!(s, FlowState::Agent(_)));
            if v.status == VisitStatus::Running && agent {
                match &v.session_id {
                    Some(session) => sessions.push(session.clone()),
                    None => {
                        self.cancelled_unreported
                            .insert((active.run.id.clone(), v.id.clone()));
                    }
                }
            }
            v.status = VisitStatus::Cancelled;
            v.finished_at = Some(now);
            stopped.push(v.id.clone());
        }
        let mut effects: Vec<Effect> = stopped.iter().map(|id| active.persist_visit(id)).collect();
        effects.extend(
            sessions
                .into_iter()
                .map(|session_id| Effect::Cancel { session_id }),
        );
        effects
    }

    /// Decides the run's pending approval; the note is the visit's `note` output.
    pub fn resolve_approval(&mut self, run_id: &str, approved: bool, note: &str) -> Vec<Effect> {
        self.with_run(run_id, |runner, active| {
            let now = (runner.clock)();
            if active.run.status != RunStatus::WaitingApproval {
                return vec![];
            }
            let Some(visit) = active
                .visits
                .iter_mut()
                .rev()
                .find(|v| v.status == VisitStatus::WaitingApproval)
            else {
                return vec![];
            };
            let values = HashMap::from([
                ("approved".to_string(), OutputValue::Bool(approved)),
                ("note".to_string(), OutputValue::String(note.to_owned())),
            ]);
            visit.status = VisitStatus::Completed;
            visit.outputs = Some(values.clone());
            visit.note = Some(note.to_owned());
            visit.finished_at = Some(now);
            let error = format!("No transition matched from {}", visit.state_id);
            let (state_id, visit_id) = (visit.state_id.clone(), visit.id.clone());
            active.run.status = RunStatus::Running;
            runner.advance(
                active,
                &state_id,
                &visit_id,
                VisitResult::Completed,
                &values,
                error,
            )
        })
    }

    /// Replaces an active run's snapshot with `flow`. Only the name, the
    /// description and states that no visit has entered may change, so a
    /// running session is never edited.
    pub fn edit_run(
        &mut self,
        run_id: &str,
        flow: Flow,
        note: &str,
    ) -> Result<Vec<Effect>, Vec<String>> {
        let Some(active) = self.runs.get_mut(run_id) else {
            return Err(vec!["Run not found".into()]);
        };
        let old = &active.run.flow;
        // Launches take `cwd` and `access` from the snapshot, and the run's
        // inputs were resolved at start: none of these may move mid-run.
        if flow.id != old.id
            || flow.owner != old.owner
            || flow.initial != old.initial
            || flow.access != old.access
            || flow.cwd != old.cwd
            || flow.inputs != old.inputs
        {
            return Err(vec![IDENTITY_EDIT.into()]);
        }
        let validation = validate_flow(&flow);
        if !validation.errors.is_empty() {
            return Err(validation.errors);
        }
        let find = |f: &Flow, id: &str| f.states.iter().find(|s| s.id() == id).cloned();
        let mut visited: Vec<&str> = Vec::new();
        for v in &active.visits {
            if !visited.contains(&v.state_id.as_str()) {
                visited.push(&v.state_id);
            }
        }
        let errors: Vec<String> = visited
            .into_iter()
            .filter(|id| find(&flow, id) != find(old, id))
            .map(|id| format!("State \"{id}\" has already run and cannot be changed"))
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        let states = flow
            .states
            .iter()
            .filter(|s| find(old, s.id()).as_ref() != Some(*s))
            .map(|s| s.id().to_owned())
            .collect();
        active.run.flow = flow;
        active.run.edits.push(RunEdit {
            at: (self.clock)(),
            note: note.to_owned(),
            states,
        });
        Ok(vec![active.persist(false)])
    }

    pub fn cancel(&mut self, run_id: &str) -> Vec<Effect> {
        let Some(mut active) = self.runs.remove(run_id) else {
            return vec![];
        };
        let now = (self.clock)();
        let stopped = self.stop_others(&mut active, 0, None);
        active.run.status = RunStatus::Cancelled;
        active.run.finished_at = Some(now);
        // The run as it ends, with each visit it stopped, then the sessions.
        let mut effects = Vec::new();
        let (persists, cancels): (Vec<_>, Vec<_>) = stopped
            .into_iter()
            .partition(|e| matches!(e, Effect::Persist { .. }));
        if persists.is_empty() {
            effects.push(active.persist(false));
        }
        for effect in persists {
            if let Effect::Persist {
                visit: Some(visit), ..
            } = effect
            {
                effects.push(Effect::Persist {
                    run: active.run.clone(),
                    visit: active.visits.iter().find(|v| v.id == visit.id).cloned(),
                });
            }
        }
        effects.extend(cancels);
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
        // A branch runs again through its parallel step.
        let state_id = run
            .flow
            .parent_of(state_id)
            .map_or(state_id.to_owned(), |p| p.id.clone());
        let state_id = state_id.as_str();
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
            for visit in &interrupted {
                effects.push(Effect::Persist {
                    run: active.run.clone(),
                    visit: Some(visit.clone()),
                });
            }
            effects.push(Effect::Finished {
                run_id: active.run.id.clone(),
                status: RunStatus::Failed,
            });
            effects.extend(interrupted.into_iter().map(|visit| Effect::VisitFinished {
                run: active.run.clone(),
                visit,
            }));
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

    /// Ends the run, persisting visit `visit_id` alongside it. Any other
    /// visit still in flight (a parallel step's branch) is stopped.
    fn finish(
        &mut self,
        active: &mut ActiveRun,
        status: RunStatus,
        error: Option<String>,
        visit_id: Option<&str>,
    ) -> Vec<Effect> {
        let stopped = self.stop_others(active, 0, visit_id);
        active.run.status = status;
        active.run.error = error;
        active.run.finished_at = Some((self.clock)());
        let mut effects = vec![match visit_id {
            Some(id) => active.persist_visit(id),
            None => active.persist(false),
        }];
        // Each stopped visit is persisted as the run now stands.
        for effect in stopped {
            effects.push(match effect {
                Effect::Persist {
                    visit: Some(visit), ..
                } => active.persist_visit(&visit.id),
                other => other,
            });
        }
        effects.push(Effect::Finished {
            run_id: active.run.id.clone(),
            status,
        });
        effects
    }

    /// Follows the transition of `state_id` chosen for its just-ended visit
    /// `visit_id`. `unmatched` is the run's error when no transition matches.
    fn advance(
        &mut self,
        active: &mut ActiveRun,
        state_id: &str,
        visit_id: &str,
        result: VisitResult,
        values: &HashMap<String, OutputValue>,
        unmatched: String,
    ) -> Vec<Effect> {
        let target = active
            .state(state_id)
            .and_then(|s| select_transition(&s, result, values).map(|t| t.to.clone()));
        match target.as_deref() {
            None => self.finish(active, RunStatus::Failed, Some(unmatched), Some(visit_id)),
            Some(SUCCEEDED) => self.finish(active, RunStatus::Succeeded, None, Some(visit_id)),
            Some(FAILED) => {
                let error = format!("Reached {FAILED} from {state_id}");
                self.finish(active, RunStatus::Failed, Some(error), Some(visit_id))
            }
            Some(next) => {
                let mut effects = vec![active.persist_visit(visit_id)];
                effects.extend(self.enter_state(active, next));
                effects
            }
        }
    }

    fn enter_state(&mut self, active: &mut ActiveRun, state_id: &str) -> Vec<Effect> {
        self.enter(active, state_id, None)
    }

    /// Creates the next visit of `state_id`, after the visit and step caps.
    /// `failed` is the error of the branch visit this one retries.
    fn enter(
        &mut self,
        active: &mut ActiveRun,
        state_id: &str,
        failed: Option<&str>,
    ) -> Vec<Effect> {
        let Some(state) = active.state(state_id) else {
            let error = format!("Unknown state {state_id}");
            return self.finish(active, RunStatus::Failed, Some(error), None);
        };
        let max = state.max_visits();
        if active.visits_of(state_id) >= usize::from(max) {
            let error = format!("State {state_id} exceeded {max} visits");
            return self.finish(active, RunStatus::Failed, Some(error), None);
        }
        if active.visits.len() >= MAX_RUN_STEPS as usize {
            let error = format!("Run exceeded {MAX_RUN_STEPS} steps");
            return self.finish(active, RunStatus::Failed, Some(error), None);
        }

        let template = match &state {
            FlowState::Agent(s) => s.brief.as_str(),
            FlowState::Approval(s) => s.message.as_str(),
            FlowState::Parallel(_) => "",
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
                let id = visit.id.clone();
                active.visits.push(visit);
                return self.finish(active, RunStatus::Failed, Some(error), Some(&id));
            }
        };

        match &state {
            FlowState::Agent(agent) => {
                let text = match failed {
                    Some(error) => format!(
                        "{text}\n\nThe last attempt at this step failed: {error}\nFix what failed, then do the step again."
                    ),
                    None => text,
                };
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
                    access: active.run.flow.access.clone(),
                    // A Pilot run's sessions are recognised by task.
                    title: format!(
                        "{} · {}",
                        active
                            .run
                            .tag
                            .as_ref()
                            .map_or(active.run.flow.name.as_str(), |tag| tag.task.as_str()),
                        agent.title
                    ),
                    prompt,
                };
                let id = visit.id.clone();
                active.visits.push(visit);
                vec![active.persist_visit(&id), Effect::Launch(launch)]
            }
            FlowState::Approval(_) => {
                visit.status = VisitStatus::WaitingApproval;
                visit.prompt = Some(text);
                active.run.status = RunStatus::WaitingApproval;
                let needs = Effect::NeedsApproval {
                    run_id: active.run.id.clone(),
                    visit_id: visit.id.clone(),
                };
                let id = visit.id.clone();
                active.visits.push(visit);
                vec![active.persist_visit(&id), needs]
            }
            FlowState::Parallel(parallel) => self.enter_parallel(active, parallel, visit),
        }
    }

    /// Starts parallel step `visit` and launches each branch whose `when`
    /// matches the outputs of the step that led here. With none to run, the
    /// step completes at once.
    fn enter_parallel(
        &mut self,
        active: &mut ActiveRun,
        parallel: &ParallelState,
        visit: FlowVisit,
    ) -> Vec<Effect> {
        let entry = active
            .visits
            .iter()
            .rev()
            .find(|v| v.status == VisitStatus::Completed)
            .and_then(|v| v.outputs.clone())
            .unwrap_or_default();
        active.run.status = RunStatus::Running;
        let step_id = visit.id.clone();
        active.visits.push(visit);
        let mut effects = vec![active.persist_visit(&step_id)];
        let mut ran = false;
        for branch in &parallel.branches {
            let Some(FlowState::Agent(agent)) = active.state(branch) else {
                continue;
            };
            if !branch_runs(&agent.when, &entry) {
                continue;
            }
            ran = true;
            effects.extend(self.enter(active, branch, None));
            if !active.run.status.is_active() {
                return effects;
            }
        }
        active.run.current_state = Some(parallel.id.clone());
        if !ran {
            let now = (self.clock)();
            let step = active.visit(&step_id);
            step.status = VisitStatus::Completed;
            step.finished_at = Some(now);
            let error = format!("No transition matched from {}", parallel.id);
            effects.extend(self.advance(
                active,
                &parallel.id,
                &step_id,
                VisitResult::Completed,
                &HashMap::new(),
                error,
            ));
        }
        effects
    }
}

#[cfg(test)]
mod tests;
