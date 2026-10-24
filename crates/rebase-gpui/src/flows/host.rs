//! The GPUI-free half of the Flows host. It applies the runner's effects
//! through a `SessionDriver`, routes session results back to the runner, and
//! keeps the host contract: `on_session` before any settle, the first visit's
//! worktree before its settle, and exactly one settle per agent turn.
use rebase_core::flow::{
    AgentOutcome, AgentStatus, Effect, Flow, FlowRun, FlowVisit, LaunchRequest, Runner,
};
use rebase_core::transcript::Row;
use std::collections::HashMap;

/// What the shell does for the host. Calls never re-enter the host: launches
/// report back later through `Host::launched` or `Host::launch_failed`.
pub(crate) trait SessionDriver {
    /// Creates a background session for `request` without revealing it.
    fn launch(&mut self, request: LaunchRequest);
    /// Submits `prompt` to an existing session and returns its submission
    /// generation from before the submit, or `None` if the session is gone.
    fn follow_up(&mut self, session: i64, prompt: String) -> Option<u64>;
    /// Stops the session's turn through the conversation's stop path.
    fn stop(&mut self, session: i64);
    fn persist(&mut self, run: FlowRun, visit: Option<FlowVisit>);
    /// A run needs approval or finished.
    fn changed(&mut self, run_id: &str);
}

/// A session's turn state, read on every `SessionChanged`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Probe {
    /// `external_submission_pending()`: the turn has not finished.
    pub pending: bool,
    pub generation: u64,
}

/// A finished turn, read only once the host says the session is ready.
#[derive(Clone, Debug, Default)]
pub(crate) struct Settled {
    pub cancelled: bool,
    /// `external_outcome`: whether the turn ended `Ready`, and its message.
    pub ok: bool,
    pub message: String,
    /// The untruncated reply the output contract is parsed from.
    pub text: String,
    pub worktree: Option<String>,
}

impl Settled {
    /// `outcome` is the conversation's `external_outcome`; the contract text
    /// comes from the full rows, not that message's 12,000-char excerpt.
    pub(crate) fn read(
        rows: &[Row],
        outcome: (bool, String),
        cancelled: bool,
        worktree: Option<String>,
    ) -> Self {
        Self {
            cancelled,
            ok: outcome.0,
            message: outcome.1,
            text: rebase_core::response::last_assistant_text(rows),
            worktree,
        }
    }
}

/// A submitted turn that has not been reported to the runner yet.
struct Turn {
    generation_before: u64,
    submitting: bool,
}

pub(crate) const NOT_STARTED: &str = "The Flow's session could not start this step.";
pub(crate) const SESSION_GONE: &str = "The Flow's session is no longer open.";
pub(crate) const SESSION_CLOSED: &str = "The Flow session was closed";

pub(crate) struct Host {
    runner: Runner,
    /// `(run_id, visit_id)` of every visit session, to route follow-ups,
    /// cancels and settles; dropped when the run finishes.
    sessions: HashMap<i64, (String, String)>,
    awaiting: HashMap<i64, Turn>,
    /// Sessions to probe after this call: a follow-up may not change them.
    recheck: Vec<i64>,
}

impl Host {
    pub(crate) fn new(runner: Runner) -> Self {
        Self {
            runner,
            sessions: HashMap::new(),
            awaiting: HashMap::new(),
            recheck: Vec::new(),
        }
    }

    pub(crate) fn start(
        &mut self,
        driver: &mut dyn SessionDriver,
        flow: &Flow,
        inputs: &HashMap<String, String>,
    ) -> Result<FlowRun, Vec<String>> {
        let (run, effects) = self.runner.start(flow, inputs)?;
        self.apply(driver, effects);
        Ok(run)
    }

    pub(crate) fn resolve_approval(
        &mut self,
        driver: &mut dyn SessionDriver,
        run_id: &str,
        approved: bool,
        note: &str,
    ) {
        let effects = self.runner.resolve_approval(run_id, approved, note);
        self.apply(driver, effects);
    }

    pub(crate) fn cancel(&mut self, driver: &mut dyn SessionDriver, run_id: &str) {
        let effects = self.runner.cancel(run_id);
        self.apply(driver, effects);
    }

    /// False when the runner refused: the run is active or the state unknown.
    pub(crate) fn resume(
        &mut self,
        driver: &mut dyn SessionDriver,
        run: FlowRun,
        visits: Vec<FlowVisit>,
        state_id: &str,
    ) -> bool {
        let effects = self.runner.resume(run, visits, state_id);
        let resumed = !effects.is_empty();
        self.apply(driver, effects);
        resumed
    }

    pub(crate) fn recover(
        &mut self,
        driver: &mut dyn SessionDriver,
        active: Vec<(FlowRun, Vec<FlowVisit>)>,
    ) {
        let effects = self.runner.recover(active);
        self.apply(driver, effects);
    }

    /// A `Launch` created `session`. Returns whether to submit its prompt: a
    /// visit cancelled while its session was being created gets none.
    pub(crate) fn launched(
        &mut self,
        driver: &mut dyn SessionDriver,
        run_id: &str,
        visit_id: &str,
        session: i64,
        generation_before: u64,
    ) -> bool {
        let key = session.to_string();
        let effects = self.runner.on_session(run_id, visit_id, &key);
        let stopped = effects
            .iter()
            .any(|e| matches!(e, Effect::Cancel { session_id } if *session_id == key));
        if !stopped {
            self.sessions
                .insert(session, (run_id.to_owned(), visit_id.to_owned()));
            self.awaiting.insert(
                session,
                Turn {
                    generation_before,
                    submitting: true,
                },
            );
        }
        // The cancelled session has no turn to stop yet.
        let effects = effects
            .into_iter()
            .filter(|e| !matches!(e, Effect::Cancel { session_id } if *session_id == key))
            .collect();
        self.apply(driver, effects);
        !stopped
    }

    /// The launched session's prompt was submitted; its turn can settle.
    pub(crate) fn submitted(&mut self, session: i64) {
        if let Some(turn) = self.awaiting.get_mut(&session) {
            turn.submitting = false;
            self.recheck.push(session);
        }
    }

    /// Creating the visit's session failed: the visit fails with `error`.
    pub(crate) fn launch_failed(
        &mut self,
        driver: &mut dyn SessionDriver,
        run_id: &str,
        visit_id: &str,
        error: String,
    ) {
        let effects = self.runner.on_agent_settled(
            run_id,
            visit_id,
            AgentOutcome {
                status: AgentStatus::Failed,
                text: String::new(),
                error: Some(error),
            },
        );
        self.apply(driver, effects);
    }

    /// Whether `session`'s turn has ended and should be read and settled.
    pub(crate) fn ready(&self, session: i64, probe: Probe) -> bool {
        self.awaiting
            .get(&session)
            .is_some_and(|turn| !turn.submitting && !probe.pending)
    }

    /// Reports `session`'s finished turn once. A stopped turn is cancelled; a
    /// turn that never started (the generation did not move) fails rather
    /// than waiting forever.
    pub(crate) fn settle(
        &mut self,
        driver: &mut dyn SessionDriver,
        session: i64,
        probe: Probe,
        settled: Settled,
    ) {
        if !self.ready(session, probe) {
            return;
        }
        let Some(turn) = self.awaiting.remove(&session) else {
            return;
        };
        let Some((run_id, visit_id)) = self.sessions.get(&session).cloned() else {
            return;
        };
        // A stop wins even before the turn started, as for automations.
        let outcome = if settled.cancelled {
            AgentOutcome {
                status: AgentStatus::Cancelled,
                text: settled.text,
                error: None,
            }
        } else if probe.generation == turn.generation_before {
            failed(NOT_STARTED.into())
        } else if !settled.ok {
            AgentOutcome {
                error: Some(settled.message),
                ..failed(String::new())
            }
        } else {
            AgentOutcome {
                status: AgentStatus::Completed,
                text: settled.text,
                error: None,
            }
        };
        if let Some(path) = settled.worktree {
            let effects = self.runner.on_worktree(&run_id, &path);
            self.apply(driver, effects);
        }
        let effects = self.runner.on_agent_settled(&run_id, &visit_id, outcome);
        self.apply(driver, effects);
    }

    /// `session` is in worktree `path`, which a conversation reports
    /// mid-turn. Reported as soon as it is seen, so a cancel or restart
    /// before the turn settles keeps it; the runner keeps only the first.
    pub(crate) fn worktree(&mut self, driver: &mut dyn SessionDriver, session: i64, path: &str) {
        let Some((run_id, _)) = self.sessions.get(&session).cloned() else {
            return;
        };
        let effects = self.runner.on_worktree(&run_id, path);
        self.apply(driver, effects);
    }

    /// Whether a run of `flow_id` is in flight, written to the store or not.
    pub(crate) fn has_active(&self, flow_id: &str) -> bool {
        self.runner.has_active(flow_id)
    }

    /// The run whose visit has a turn in flight in `session`.
    pub(crate) fn run_of(&self, session: i64) -> Option<&str> {
        if !self.awaiting.contains_key(&session) {
            return None;
        }
        self.sessions.get(&session).map(|(run, _)| run.as_str())
    }

    /// The user deleted `session`. A visit still running in it fails; a
    /// later report from its stopped turn is ignored.
    pub(crate) fn session_closed(&mut self, driver: &mut dyn SessionDriver, session: i64) {
        if self.awaiting.remove(&session).is_none() {
            return;
        }
        let Some((run_id, visit_id)) = self.sessions.remove(&session) else {
            return;
        };
        let effects =
            self.runner
                .on_agent_settled(&run_id, &visit_id, failed(SESSION_CLOSED.into()));
        self.apply(driver, effects);
    }

    pub(crate) fn awaits(&self, session: i64) -> bool {
        self.awaiting.contains_key(&session)
    }

    pub(crate) fn take_rechecks(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.recheck)
    }

    fn apply(&mut self, driver: &mut dyn SessionDriver, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Launch(request) => driver.launch(request),
                Effect::FollowUp { session_id, prompt } => {
                    let Some(session) = self.routed(&session_id) else {
                        continue;
                    };
                    self.follow_up(driver, session, prompt);
                }
                Effect::Cancel { session_id } => {
                    if let Ok(session) = session_id.parse() {
                        self.awaiting.remove(&session);
                        driver.stop(session);
                    }
                }
                Effect::Persist { run, visit } => driver.persist(run, visit),
                Effect::NeedsApproval { run_id, .. } => driver.changed(&run_id),
                Effect::Finished { run_id, .. } => {
                    let done: Vec<i64> = self
                        .sessions
                        .iter()
                        .filter(|(_, (run, _))| *run == run_id)
                        .map(|(&session, _)| session)
                        .collect();
                    for session in done {
                        self.sessions.remove(&session);
                        self.awaiting.remove(&session);
                    }
                    driver.changed(&run_id);
                }
            }
        }
    }

    /// The contract retry: a second turn in the same session, settled once.
    fn follow_up(&mut self, driver: &mut dyn SessionDriver, session: i64, prompt: String) {
        let Some((run_id, visit_id)) = self.sessions.get(&session).cloned() else {
            return;
        };
        match driver.follow_up(session, prompt) {
            Some(generation_before) => {
                self.awaiting.insert(
                    session,
                    Turn {
                        generation_before,
                        submitting: false,
                    },
                );
                self.recheck.push(session);
            }
            None => self.launch_failed(driver, &run_id, &visit_id, SESSION_GONE.into()),
        }
    }

    fn routed(&self, session_id: &str) -> Option<i64> {
        session_id
            .parse()
            .ok()
            .filter(|session| self.sessions.contains_key(session))
    }
}

/// A run recovered without its worktree (the restart came before the turn
/// settled) takes it from the newest visit session whose saved settings name
/// one. `saved` reads a session's saved `worktree_cwd`.
pub(crate) fn fill_worktree(
    run: &mut FlowRun,
    visits: &[FlowVisit],
    mut saved: impl FnMut(i64) -> Option<String>,
) {
    if run.worktree_cwd.is_some() {
        return;
    }
    let mut sessions: Vec<_> = visits
        .iter()
        .filter_map(|v| Some((v.seq, v.session_id.as_deref()?.parse::<i64>().ok()?)))
        .collect();
    sessions.sort_by_key(|(seq, _)| std::cmp::Reverse(*seq));
    run.worktree_cwd = sessions.into_iter().find_map(|(_, session)| saved(session));
}

fn failed(error: String) -> AgentOutcome {
    AgentOutcome {
        status: AgentStatus::Failed,
        text: String::new(),
        error: Some(error),
    }
}

#[cfg(test)]
mod tests;
