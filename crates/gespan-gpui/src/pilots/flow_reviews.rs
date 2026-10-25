//! How a Pilot responds to its Flow runs: completed steps get a hidden,
//! read-only side review; failures and review notes worth acting on wait
//! as alerts for the main chat; a goal whose runs have all ended rolls up
//! once. None of it holds a run back: the Flow has already moved on.
use super::*;
use crate::shell::pilot_flows::PilotFlowEvent;
use gespan_core::flow::{FlowRun, FlowVisit, RunStatus, VisitStatus};
use gespan_core::pilot::flow_notes::{self, FlowNote, Goal, MAX_REVIEWS, NoteKind};
use gespan_core::time::now_ms;

/// The only app actions a review session may call.
pub(super) const REVIEW_ACTIONS: [&str; 6] = [
    "flows.read",
    "flows.run",
    "flows.runs",
    "sessions.read",
    "memory.read",
    "memory.search",
];
pub(super) const REVIEW_READ_ONLY: &str = "Reviews are read-only; report your suggestion instead";
const REVIEW_TIMED_OUT: &str = "Review timed out";
const REVIEW_NOT_STARTED: &str = "Review could not start";

/// One hidden review of a finished step.
pub(crate) struct RunningReview {
    pub(super) pilot: String,
    pub(super) view: Entity<Conversation>,
    run_id: String,
    goal: String,
    task: String,
    state_id: String,
    pub(super) started: u64,
    /// Whether the session took the review prompt; until then its changes
    /// say nothing about the review.
    dispatched: bool,
    /// Dropped with the review, so finished reviews stop being watched.
    _changes: Subscription,
}

/// A review waiting for a slot: `(pilot_id, run, visit)`.
pub(crate) type QueuedReview = (String, FlowRun, FlowVisit);

/// What the Pilots entity remembers about its runs between events.
#[derive(Default)]
pub(crate) struct Watch {
    /// `(pilot_id, run_id)` of reviews whose session is being created.
    starting: Vec<(String, String)>,
    /// The latest copy of each run seen this session.
    runs: HashMap<String, FlowRun>,
    /// Runs whose `Finished` arrived, until they are resumed.
    ended: HashMap<String, RunStatus>,
    /// Runs a failed or stopped step already alerted for.
    alerted: HashSet<String>,
    /// Failed runs whose alert the Pilot may still act on, with their Pilot:
    /// no message while the alert waits, then the message it went out in,
    /// until the turn that read it ends. Their goal waits for them.
    recovering: HashMap<String, (String, Option<String>)>,
    /// Runs the Pilot cancelled itself, which need no alert.
    cancelled: HashSet<String>,
    /// `(pilot_id, run_id, status)` of failed runs to check once the
    /// current batch of events is handled.
    failures: Vec<(String, String, RunStatus)>,
    /// Pilots to settle once the current batch of events is handled.
    unsettled: HashSet<String>,
    scheduled: bool,
}

fn goal_and_task(run: &FlowRun) -> (String, String) {
    run.tag
        .as_ref()
        .map(|t| (t.goal.clone(), t.task.clone()))
        .unwrap_or_default()
}

/// A failed run with no recorded visit, as an alert can name it.
fn run_visit(run: &FlowRun, status: RunStatus) -> FlowVisit {
    FlowVisit {
        id: String::new(),
        run_id: run.id.clone(),
        seq: 0,
        state_id: run
            .current_state
            .clone()
            .unwrap_or_else(|| run.flow.initial.clone()),
        status: if status == RunStatus::Cancelled {
            VisitStatus::Cancelled
        } else {
            VisitStatus::Failed
        },
        prompt: None,
        session_id: None,
        outputs: None,
        error: run.error.clone(),
        retried: false,
        note: None,
        started_at: 0,
        finished_at: None,
    }
}

impl Pilots {
    /// A step or the end of a run of a Flow `pilot_id` owns. Reacts without
    /// blocking anything; the main Pilot acts on alerts later.
    pub(crate) fn flow_event(
        &mut self,
        pilot_id: &str,
        event: PilotFlowEvent,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        self.flow_events.push((pilot_id.to_owned(), event.clone()));
        if self.roster.get(pilot_id).is_none() {
            return;
        }
        match event {
            PilotFlowEvent::Step { run, visit } => {
                self.flow_watch.runs.insert(run.id.clone(), run.clone());
                match visit.status {
                    VisitStatus::Completed => {
                        self.review_queue
                            .push_back((pilot_id.to_owned(), run, visit));
                    }
                    // Only a failure that stopped the run: one routed on to
                    // another state leaves nothing to recover.
                    VisitStatus::Failed | VisitStatus::Cancelled
                        if matches!(run.status, RunStatus::Failed | RunStatus::Cancelled) =>
                    {
                        self.flow_watch.alerted.insert(run.id.clone());
                        self.failure_alert(
                            pilot_id,
                            &run.id,
                            flow_notes::failure_alert(&run, &visit),
                        );
                    }
                    VisitStatus::Failed | VisitStatus::Cancelled => {}
                    VisitStatus::Running | VisitStatus::WaitingApproval => {}
                }
            }
            PilotFlowEvent::Finished { run_id, status } => {
                if let Some(run) = self.flow_watch.runs.get_mut(&run_id) {
                    run.status = status;
                }
                self.flow_watch.ended.insert(run_id.clone(), status);
                let failed = matches!(status, RunStatus::Failed | RunStatus::Cancelled);
                if self.flow_watch.cancelled.remove(&run_id) || !failed {
                    self.flow_watch.alerted.remove(&run_id);
                } else {
                    // Its failing step may come after this; check once the
                    // batch is handled.
                    self.flow_watch
                        .failures
                        .push((pilot_id.to_owned(), run_id, status));
                }
            }
        }
        self.settle_flows(pilot_id, cx);
    }
    fn flow_alert(&mut self, pilot_id: &str, alert: String) {
        if let Some(pilot) = self.roster.get_mut(pilot_id) {
            pilot.flow_alerts.push(alert);
        }
    }
    /// Alerts the Pilot that `run_id` stopped; its goal waits for the Pilot.
    fn failure_alert(&mut self, pilot_id: &str, run_id: &str, alert: String) {
        self.flow_alert(pilot_id, alert);
        self.flow_watch
            .recovering
            .insert(run_id.to_owned(), (pilot_id.to_owned(), None));
    }
    /// Whether the Pilot may still be recovering `run_id`: its failure alert
    /// waits, or went out in a turn that has not ended yet.
    fn recovering(&self, run_id: &str, cx: &App) -> bool {
        let Some((pilot, sent)) = self.flow_watch.recovering.get(run_id) else {
            return false;
        };
        let Some(sent) = sent else {
            return self.roster.get(pilot).is_some();
        };
        self.chats.get(pilot).is_some_and(|chat| chat.read(cx).busy)
            || self.inflight.get(pilot).is_some_and(|m| &m.id == sent)
            || self
                .roster
                .get(pilot)
                .is_some_and(|p| p.outbox.iter().any(|m| &m.id == sent))
    }
    /// Whether a failure alert went out to `pilot_id`'s chat and its goal
    /// waits for that turn to end.
    pub(super) fn flow_alert_turns(&self, pilot_id: &str) -> bool {
        self.flow_watch
            .recovering
            .values()
            .any(|(pilot, sent)| pilot == pilot_id && sent.is_some())
    }
    /// Raises failure alerts, starts reviews, rolls up goals and delivers
    /// alerts after the current batch of events, outside any Shell update
    /// (the flow store is read through the Shell).
    pub(super) fn settle_flows(&mut self, pilot_id: &str, cx: &mut Context<Self>) {
        self.flow_watch.unsettled.insert(pilot_id.to_owned());
        if self.flow_watch.scheduled {
            return;
        }
        self.flow_watch.scheduled = true;
        let this = cx.entity().downgrade();
        let app: &mut App = cx;
        app.defer(move |cx| {
            let _ = this.update(cx, |this, cx| {
                this.flow_watch.scheduled = false;
                let failures = std::mem::take(&mut this.flow_watch.failures);
                for (pilot, run, status) in failures {
                    if !this.flow_watch.alerted.remove(&run) {
                        this.alert_failed_run(&pilot, &run, status, cx);
                    }
                }
                this.pump_reviews(cx);
                let pilots = std::mem::take(&mut this.flow_watch.unsettled);
                for pilot in pilots {
                    this.check_goals(&pilot, cx);
                    this.flush_flow_alerts(&pilot, cx);
                }
                this.save(cx);
            });
        });
    }
    /// The flow store, read only outside Shell updates.
    fn flow_store(
        &self,
        cx: &App,
    ) -> Option<std::sync::Arc<std::sync::Mutex<gespan_core::flow::FlowStore>>> {
        self.owner
            .upgrade()
            .and_then(|shell| shell.read(cx).pilot_flows_store().ok())
    }
    /// A run that ended failed or cancelled without a failing step event,
    /// such as a visit that failed before its session launched.
    fn alert_failed_run(
        &mut self,
        pilot_id: &str,
        run_id: &str,
        status: RunStatus,
        cx: &mut Context<Self>,
    ) {
        let stored = self.flow_store(cx).and_then(|store| {
            let store = store.lock().ok()?;
            store.owned_run(pilot_id, run_id).ok().flatten()
        });
        let (run, visits) = match stored {
            Some((run, visits)) => (Some(run), visits),
            None => (self.flow_watch.runs.get(run_id).cloned(), Vec::new()),
        };
        let alert = match run {
            Some(mut run) => {
                run.status = status;
                // Only the latest visit can be this failure; an older failed
                // one belongs to an episode the Pilot already recovered.
                let visit = visits
                    .into_iter()
                    .max_by_key(|v| v.seq)
                    .filter(|v| matches!(v.status, VisitStatus::Failed | VisitStatus::Cancelled))
                    .unwrap_or_else(|| run_visit(&run, status));
                flow_notes::failure_alert(&run, &visit)
            }
            None => format!(
                "Flow run {run_id} {}.\nUse flows.run {run_id} to inspect, then flows.resume, flows.editRun or flows.start; ask the user only if you can't recover.",
                status.as_str()
            ),
        };
        self.failure_alert(pilot_id, run_id, alert);
    }
    fn reviews_of(&self, pilot_id: &str) -> usize {
        self.reviews
            .values()
            .filter(|r| r.pilot == pilot_id)
            .count()
            + self
                .flow_watch
                .starting
                .iter()
                .filter(|(p, _)| p == pilot_id)
                .count()
    }
    /// Starts queued reviews while their Pilot has a free slot.
    pub(super) fn pump_reviews(&mut self, cx: &mut Context<Self>) {
        self.review_queue
            .retain(|(pilot, ..)| self.roster.get(pilot).is_some());
        while let Some(index) = self
            .review_queue
            .iter()
            .position(|(pilot, ..)| self.reviews_of(pilot) < MAX_REVIEWS)
        {
            let Some((pilot, run, visit)) = self.review_queue.remove(index) else {
                break;
            };
            self.start_review(pilot, run, visit, cx);
        }
    }
    fn start_review(
        &mut self,
        pilot_id: String,
        run: FlowRun,
        visit: FlowVisit,
        cx: &mut Context<Self>,
    ) {
        self.flow_watch
            .starting
            .push((pilot_id.clone(), run.id.clone()));
        let store = self.store.clone();
        let session = visit
            .session_id
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok());
        let reply = cx.background_executor().spawn(async move {
            let Some(session) = session else {
                return String::new();
            };
            locked(&store)
                .and_then(|db| db.load(session))
                .map(|history| gespan_core::response::last_assistant_text(&history.rows))
                .unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let reply = reply.await;
            let _ = this.update(cx, |this, cx| {
                this.spawn_review(pilot_id, run, visit, reply, cx)
            });
        })
        .detach();
    }
    fn spawn_review(
        &mut self,
        pilot_id: String,
        run: FlowRun,
        visit: FlowVisit,
        reply: String,
        cx: &mut Context<Self>,
    ) {
        let starting = (pilot_id.clone(), run.id.clone());
        let Some(label) = self.roster.get(&pilot_id).map(|p| p.label()) else {
            self.unstart(&starting);
            return;
        };
        let (goal, task) = goal_and_task(&run);
        let prompt = flow_notes::review_prompt(&run, &visit, &reply);
        let (run_id, state_id) = (run.id.clone(), visit.state_id.clone());
        let pilot = pilot_id.clone();
        let title = format!("{label} · review · {task}");
        let registered = starting.clone();
        let spawned = self.spawn_hidden_run(
            &pilot_id,
            title,
            prompt,
            AccessMode::ReadOnly,
            move |this, session, view, cx| {
                this.unstart(&registered);
                let changes = cx.subscribe(view, move |this, _, _: &SessionChanged, cx| {
                    this.review_changed(session, cx)
                });
                this.reviews.insert(
                    session,
                    RunningReview {
                        pilot,
                        view: view.clone(),
                        run_id,
                        goal,
                        task,
                        state_id,
                        started: now_ms(),
                        dispatched: false,
                        _changes: changes,
                    },
                );
            },
            cx,
        );
        cx.spawn(async move |this, cx| {
            let result = spawned.await;
            let _ = this.update(cx, |this, cx| match result {
                Err(error) => {
                    this.unstart(&starting);
                    this.error = Some(error);
                    this.settle_flows(&starting.0, cx);
                }
                Ok((session, _, true)) => {
                    if let Some(review) = this.reviews.get_mut(&session) {
                        review.dispatched = true;
                    }
                    // Its turn may have ended already.
                    this.review_changed(session, cx);
                }
                // No turn will start: the slot is free at once.
                Ok((session, _, false)) => {
                    if let Some(review) = this.reviews.remove(&session) {
                        this.record_note(
                            review,
                            NoteKind::ReviewProblem,
                            REVIEW_NOT_STARTED.into(),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }
    fn unstart(&mut self, key: &(String, String)) {
        if let Some(i) = self.flow_watch.starting.iter().position(|s| s == key) {
            self.flow_watch.starting.remove(i);
        }
    }
    /// A review's chat changed; once its turn is over its reply becomes a
    /// note, even a turn that failed before writing anything.
    pub(super) fn review_changed(&mut self, session: i64, cx: &mut Context<Self>) {
        let Some(review) = self.reviews.get(&session).filter(|r| r.dispatched) else {
            return;
        };
        let view = review.view.read(cx);
        if view.busy {
            return;
        }
        let (ok, status) = view.external_outcome(cx);
        let reply = gespan_core::response::prose(view.pilot_rows(cx));
        let (kind, text) = if !ok {
            let status = status.lines().next().unwrap_or_default().trim().to_owned();
            (NoteKind::ReviewProblem, format!("Review failed: {status}"))
        } else if reply.trim().is_empty() {
            (
                NoteKind::ReviewProblem,
                "The review returned no reply".to_owned(),
            )
        } else {
            flow_notes::parse_note(&reply)
        };
        if let Some(review) = self.reviews.remove(&session) {
            self.record_note(review, kind, text, cx);
        }
    }
    fn record_note(
        &mut self,
        review: RunningReview,
        kind: NoteKind,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let note = FlowNote {
            run_id: review.run_id,
            goal: review.goal,
            task: review.task,
            state_id: review.state_id,
            kind,
            text,
            at: now_ms().min(i64::MAX as u64) as i64,
        };
        if let Some(pilot) = self.roster.get_mut(&review.pilot) {
            // A review's own problem is no task's: it waits for the roll-up.
            if !matches!(kind, NoteKind::NoChange | NoteKind::ReviewProblem) {
                pilot.flow_alerts.push(flow_notes::note_alert(&note));
            }
            pilot.flow_notes.push(note);
            flow_notes::trim_notes(&mut pilot.flow_notes, &pilot.goals);
        }
        self.settle_flows(&review.pilot, cx);
    }
    /// Reviews still running after a habit's time box end as a review problem.
    pub(super) fn check_review_timeouts(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let late: Vec<i64> = self
            .reviews
            .iter()
            .filter(|(_, r)| now.saturating_sub(r.started) > habits::TIMEOUT_MS)
            .map(|(&id, _)| id)
            .collect();
        for id in late {
            if let Some(review) = self.reviews.remove(&id) {
                review.view.update(cx, |v, cx| v.pilot_stop(cx));
                self.record_note(review, NoteKind::ReviewProblem, REVIEW_TIMED_OUT.into(), cx);
            }
        }
    }
    /// Whether `run_id` has ended, from this session's events, else the store.
    fn run_ended(
        &self,
        run_id: &str,
        stored: &mut dyn FnMut(&str) -> Option<FlowRun>,
    ) -> Option<FlowRun> {
        let mut run = self
            .flow_watch
            .runs
            .get(run_id)
            .cloned()
            .or_else(|| stored(run_id))?;
        if let Some(status) = self.flow_watch.ended.get(run_id) {
            run.status = *status;
        }
        (!run.status.is_active()).then_some(run)
    }
    fn review_pending(&self, run_id: &str) -> bool {
        self.reviews.values().any(|r| r.run_id == run_id)
            || self.review_queue.iter().any(|(_, run, _)| run.id == run_id)
            || self.flow_watch.starting.iter().any(|(_, r)| r == run_id)
    }
    /// Rolls up each goal whose runs have all ended and been reviewed, once,
    /// and not before the Pilot had a turn to recover each failed run.
    fn check_goals(&mut self, pilot_id: &str, cx: &mut Context<Self>) {
        let read: Vec<String> = self
            .flow_watch
            .recovering
            .iter()
            .filter(|(run, (pilot, _))| pilot == pilot_id && !self.recovering(run, cx))
            .map(|(run, _)| run.clone())
            .collect();
        for run in read {
            self.flow_watch.recovering.remove(&run);
        }
        let Some(pilot) = self.roster.get(pilot_id) else {
            return;
        };
        let open: Vec<Goal> = pilot
            .goals
            .iter()
            .filter(|g| !g.reported && !g.runs.is_empty())
            .cloned()
            .collect();
        if open.is_empty() {
            return;
        }
        let store = self.flow_store(cx);
        let mut stored = |run_id: &str| {
            let store = store.as_ref()?.lock().ok()?;
            store
                .run(run_id)
                .ok()
                .flatten()
                .filter(|run| run.flow.owner.as_deref() == Some(pilot_id))
        };
        let mut rollups = Vec::new();
        for goal in open {
            if goal
                .runs
                .iter()
                .any(|id| self.review_pending(id) || self.flow_watch.recovering.contains_key(id))
            {
                continue;
            }
            let runs: Option<Vec<FlowRun>> = goal
                .runs
                .iter()
                .map(|id| self.run_ended(id, &mut stored))
                .collect();
            if let Some(runs) = runs {
                rollups.push((goal, runs));
            }
        }
        let Some(pilot) = self.roster.get_mut(pilot_id) else {
            return;
        };
        for (goal, runs) in rollups {
            let text = flow_notes::goal_rollup(&goal, &runs, &pilot.flow_notes);
            if let Some(g) = pilot.goals.iter_mut().find(|g| g.name == goal.name) {
                g.reported = true;
            }
            pilot.flow_alerts.push(text);
            // Ended runs are in the store from here on.
            for id in &goal.runs {
                self.flow_watch.runs.remove(id);
                self.flow_watch.ended.remove(id);
            }
        }
    }
    /// Delivers waiting alerts as one internal message once the chat is idle.
    pub(super) fn flush_flow_alerts(&mut self, pilot_id: &str, cx: &mut Context<Self>) {
        if self
            .chats
            .get(pilot_id)
            .is_some_and(|chat| chat.read(cx).busy)
        {
            return;
        }
        let Some(alerts) = self
            .roster
            .get_mut(pilot_id)
            .map(|p| std::mem::take(&mut p.flow_alerts))
            .filter(|a| !a.is_empty())
        else {
            return;
        };
        self.ensure_chat(pilot_id, cx);
        let sent = self.enqueue(pilot_id, flow_notes::bundle(&alerts), Vec::new(), true, cx);
        for (pilot, message) in self.flow_watch.recovering.values_mut() {
            if pilot == pilot_id && message.is_none() {
                message.clone_from(&sent);
            }
        }
        self.save(cx);
    }
    /// `flows.start` started `run` for its goal.
    pub(super) fn flow_started(&mut self, pilot_id: &str, run: FlowRun) {
        let (goal, _) = goal_and_task(&run);
        let id = run.id.clone();
        self.flow_watch.runs.insert(id.clone(), run);
        let Some(pilot) = self.roster.get_mut(pilot_id) else {
            return;
        };
        match pilot.goals.iter_mut().find(|g| g.name == goal) {
            Some(g) => {
                if !g.runs.contains(&id) {
                    g.runs.push(id);
                }
                g.reported = false;
            }
            None => pilot.goals.push(Goal {
                name: goal,
                runs: vec![id],
                reported: false,
            }),
        }
    }
    /// `flows.cancel` or `flows.resume` changed `run_id`.
    pub(super) fn flow_changed(&mut self, pilot_id: &str, action: &str, run_id: &str) {
        match action {
            "flows.cancel" => {
                if !self.flow_watch.ended.contains_key(run_id) {
                    self.flow_watch.cancelled.insert(run_id.to_owned());
                }
            }
            "flows.resume" => {
                self.flow_watch.ended.remove(run_id);
                self.flow_watch.cancelled.remove(run_id);
                self.flow_watch.alerted.remove(run_id);
                // Recovered: the goal now waits for the run itself.
                self.flow_watch.recovering.remove(run_id);
                if let Some(run) = self.flow_watch.runs.get_mut(run_id) {
                    run.status = RunStatus::Running;
                }
                if let Some(goal) = self.roster.get_mut(pilot_id).and_then(|p| {
                    p.goals
                        .iter_mut()
                        .find(|g| g.runs.iter().any(|r| r == run_id))
                }) {
                    goal.reported = false;
                }
            }
            _ => {}
        }
    }
    /// Every Pilot gets its waiting alerts and reviews on the clock.
    pub(super) fn tick_flows(&mut self, cx: &mut Context<Self>) {
        self.check_review_timeouts(cx);
        self.pump_reviews(cx);
        let ids: Vec<String> = self.roster.pilots.iter().map(|p| p.id.clone()).collect();
        for id in ids {
            self.flush_flow_alerts(&id, cx);
        }
    }
}
