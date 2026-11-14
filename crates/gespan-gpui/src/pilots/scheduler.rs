use super::*;
use gespan_core::flow::{FlowRun, RunStatus, RunTag};
use gespan_core::pilot::completions;
use gespan_core::pilot::habits::{Run, Target, Tell};
use gespan_core::time::now_ms;

/// A started hidden run: its session, its view, and whether the session
/// took the prompt.
pub(super) type HiddenRun = (i64, Entity<Conversation>, bool);

/// A Flow run a schedule started, until it ends.
pub(crate) struct ScheduledFlow {
    pub(super) pilot: String,
    pub(super) habit: String,
    pub(super) name: String,
    pub(super) tell: Tell,
    pub(super) launched: u64,
}

/// The prompt of a scheduled instruction. One that tells only when needed
/// may answer NOTHING_TO_REPORT; one that tells every run always reports.
pub(super) fn instruction_prompt(habit: &Habit) -> String {
    let report = match habit.tell {
        Tell::EveryRun => "Return a concise report of what you did and found.",
        Tell::OnlyIfNeeded => {
            "Return a concise report only if something needs the user's attention or decision; otherwise reply exactly NOTHING_TO_REPORT."
        }
    };
    format!(
        "[Scheduled habit: {}]\n{}\n\nDo this scheduled work independently. {report} Any session you start belongs to this habit. Do not change the habit unless asked.",
        habit.name, habit.instructions
    )
}

#[derive(Default)]
pub(super) struct HabitUi {
    pub(super) saving: bool,
    pub(super) revision: u64,
    pub(super) error: Option<String>,
    pub(super) picker_field: Option<&'static str>,
    pub(super) expanded: HashSet<(String, u64)>,
    /// The Pipeline tab's Flows are watched while the form is open.
    pub(super) watching: Option<(String, Subscription)>,
}

pub(super) fn duration(ms: u64) -> String {
    let seconds = (ms.saturating_add(500) / 1000).max(1);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}
/// When a run happened, as Last runs lists it: "Today 02:00", else "Nov 13".
pub(super) fn when(at: u64, now: u64) -> String {
    let Some(at) = chrono::DateTime::from_timestamp_millis(at as i64) else {
        return String::new();
    };
    let at = at.with_timezone(&chrono::Local);
    let today = chrono::DateTime::from_timestamp_millis(now as i64)
        .map(|d| d.with_timezone(&chrono::Local).date_naive());
    if Some(at.date_naive()) == today {
        format!("Today {}", at.format("%H:%M"))
    } else {
        at.format("%b %-d").to_string()
    }
}
fn same_day(a: u64, b: u64) -> bool {
    let day = |at| {
        chrono::DateTime::from_timestamp_millis(at as i64)
            .map(|d| d.with_timezone(&chrono::Local).date_naive())
    };
    day(a).is_some() && day(a) == day(b)
}
/// One row per habit, following HabitsList's today/upcoming/done/later order.
pub(super) fn grouped_habits(
    habits: &[Habit],
    now: u64,
    running: impl Fn(&Habit) -> Option<u64>,
) -> [Vec<usize>; 3] {
    let mut upcoming = Vec::new();
    let mut done = Vec::new();
    let mut later = Vec::new();
    for (ix, h) in habits.iter().enumerate() {
        if let Some(since) = running(h) {
            upcoming.push((0, since, ix));
        } else if h.run_requested || h.enabled && h.next_run_at <= now {
            upcoming.push((1, now, ix));
        } else if h.enabled && same_day(h.next_run_at, now) {
            upcoming.push((2, h.next_run_at, ix));
        } else if let Some(last) = h
            .runs
            .first()
            .map(|r| r.at)
            .or(h.last_run_at)
            .filter(|at| same_day(*at, now))
        {
            done.push((last, ix));
        } else {
            later.push((!h.enabled, h.next_run_at, ix));
        }
    }
    upcoming.sort_by_key(|item| (item.0, item.1));
    done.sort_by_key(|item| item.0);
    later.sort_by_key(|item| (item.0, item.1));
    [
        upcoming.into_iter().map(|item| item.2).collect(),
        done.into_iter().map(|item| item.1).collect(),
        later.into_iter().map(|item| item.2).collect(),
    ]
}
impl Pilots {
    pub(super) fn attention_views(&self, id: &str, cx: &App) -> Vec<Entity<Conversation>> {
        let mut views = Vec::new();
        for (&session, run) in self.running.iter().filter(|(_, r)| r.pilot == id) {
            if run.view.read(cx).needs_approval() {
                views.push(run.view.clone());
            }
            for launch in self
                .roster
                .pilots
                .iter()
                .flat_map(|m| m.launches.iter())
                .filter(|l| {
                    completions::launched_from(l.origin_turn, session)
                        && !l.completed
                        && !l.dismissed
                })
            {
                if let Some(view) = self
                    .owner
                    .read(cx, |owner, _| owner.pilot_session_entity(launch.session_id))
                    .flatten()
                    .filter(|v| v.read(cx).needs_approval())
                {
                    views.push(view);
                }
            }
        }
        views
    }

    pub(super) fn habit_running_since(&self, habit: &Habit) -> Option<u64> {
        self.running
            .values()
            .find(|r| r.habit == habit.id && self.active.as_deref() == Some(r.pilot.as_str()))
            .map(|r| r.launched)
            .or(habit.running_since)
    }
    pub(super) fn habit_busy(&self, habit: &Habit) -> bool {
        self.habit_running_since(habit).is_some()
            || habit.run_requested
            || habit.enabled && habit.next_run_at <= now_ms()
    }
    pub(super) fn clear_habit_fields(&mut self) {
        self.fields.retain(|key, _| !key.starts_with("habit-"));
        self.habit_ui.error = None;
        self.habit_ui.revision = self.habit_ui.revision.wrapping_add(1);
    }
    pub(super) fn habit_toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(pilot) = self.active.clone() else {
            return;
        };
        let root = self.root.clone();
        let target = pilot.clone();
        let id = id.to_string();
        self.file_work(
            pilot,
            move || {
                habits::update(&root, &target, |all| {
                    let h = all
                        .iter_mut()
                        .find(|h| h.id == id)
                        .ok_or("Habit not found")?;
                    h.enabled = !h.enabled;
                    if h.enabled {
                        h.next_run_at = h.schedule.next(now_ms())?;
                    }
                    Ok(())
                })
            },
            cx,
        );
    }
    pub(super) fn request_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .habits
            .iter()
            .find(|h| h.id == id)
            .is_none_or(|h| self.habit_busy(h))
        {
            return;
        }
        let Some(pilot) = self.active.clone() else {
            return;
        };
        let root = self.root.clone();
        let target = pilot.clone();
        let id = id.to_string();
        if let Some(h) = self.habits.iter_mut().find(|h| h.id == id) {
            h.run_requested = true;
        }
        let work = cx.background_executor().spawn(async move {
            habits::update(&root, &target, |all| {
                all.iter_mut()
                    .find(|h| h.id == id)
                    .ok_or("Habit not found")?
                    .run_requested = true;
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.reload_files(&pilot, cx);
                this.check_habits(cx);
                this.changed(cx);
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn remove_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(pilot) = self.active.clone() else {
            return;
        };
        let root = self.root.clone();
        let target = pilot.clone();
        let id = id.to_string();
        let removed = id.clone();
        let work = cx.background_executor().spawn(async move {
            habits::update(&root, &target, |all| {
                all.retain(|h| h.id != id);
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.reload_files(&pilot, cx);
                        if this.active.as_deref() == Some(&pilot)
                            && this.route == Some(Route::Habit(removed.clone()))
                        {
                            this.route = None;
                            this.clear_habit_fields();
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                this.changed(cx);
            });
        })
        .detach();
    }
    /// After a hold: running habits don't count the held time against their
    /// timeouts, and habits that came due while held run once now.
    pub(crate) fn resumed(&mut self, held_ms: u64, since: u64, cx: &mut Context<Self>) {
        for run in self.running.values_mut() {
            run.started += held_ms;
            if let Some(waiting) = run.waiting_since.as_mut() {
                *waiting += held_ms;
            }
        }
        let now = now_ms();
        for id in self
            .roster
            .pilots
            .iter()
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
        {
            if let Err(error) = habits::update(&self.root, &id, |all| {
                habits::catch_up(all, since, now);
                Ok(())
            }) {
                self.error = Some(error);
            }
        }
        cx.notify();
    }
    pub(super) fn tick(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.check_habits(cx);
        self.tick_flows(cx);
        cx.notify();
    }
    pub(super) fn check_habits(&mut self, cx: &mut Context<Self>) {
        // While held no habit starts, times out or reports; Resume moves the clocks on.
        if crate::hold::active(cx) {
            return;
        }
        let now = now_ms();
        if let Some(id) = self.active.clone() {
            self.refresh_work(&id, cx);
        }
        self.refresh_cards(cx);
        let waiting_pilots = self
            .running
            .values()
            .filter(|r| !self.attention_views(&r.pilot, cx).is_empty())
            .map(|r| r.pilot.clone())
            .collect::<HashSet<_>>();
        let timed_out: Vec<_> = self
            .running
            .iter_mut()
            .filter_map(|(&id, run)| {
                let waiting = waiting_pilots.contains(&run.pilot);
                if waiting {
                    let since = *run.waiting_since.get_or_insert(now);
                    (now.saturating_sub(since) > habits::APPROVAL_WAIT_MS).then_some(id)
                } else {
                    if let Some(since) = run.waiting_since.take() {
                        run.started += now.saturating_sub(since);
                    }
                    (now.saturating_sub(run.started) > habits::TIMEOUT_MS).then_some(id)
                }
            })
            .collect();
        for id in timed_out {
            if let Some(run) = self.running.get_mut(&id) {
                run.failure = Some("Habit timed out while waiting for work or an answer".into());
                run.mailbox.clear();
                run.view.update(cx, |v, cx| v.pilot_stop(cx));
            }
            let children = self
                .roster
                .pilots
                .iter_mut()
                .flat_map(|m| m.launches.iter_mut())
                .filter(|l| {
                    completions::launched_from(l.origin_turn, id) && !l.completed && !l.dismissed
                })
                .map(|l| {
                    l.dismissed = true;
                    l.session_id
                })
                .collect::<Vec<_>>();
            for child in children {
                let view = self
                    .owner
                    .read(cx, |owner, _| owner.pilot_session_entity(child))
                    .flatten();
                if let Some(view) = view {
                    view.update(cx, |v, cx| v.stop_if_busy(cx));
                }
            }
            self.habit_changed(id, cx);
        }
        self.check_completions(cx);
        if self.scheduling || self.active_run.is_some() {
            return;
        }
        self.scheduling = true;
        let root = self.root.clone();
        let ids: Vec<_> = self.roster.pilots.iter().map(|m| m.id.clone()).collect();
        let work = cx.background_executor().spawn(async move {
            for id in ids {
                let claimed = habits::update(&root, &id, |all| {
                    // A process restart cannot resume an old provider turn.
                    // Record the interruption instead of leaving it claimed forever.
                    for h in all.iter_mut() {
                        if let Some(start) = h.running_since {
                            h.finish(Run {
                                at: start,
                                duration_ms: now.saturating_sub(start),
                                outcome: "interrupted".into(),
                                report: None,
                                error: Some("The app closed before this run finished".into()),
                            })?;
                        }
                        if h.enabled
                            && h.next_run_at < now
                            && now - h.next_run_at > habits::GRACE_MS
                        {
                            h.next_run_at = h.schedule.next(now)?;
                        }
                    }
                    if let Some(h) = all.iter_mut().find(|h| h.due(now)) {
                        h.running_since = Some(now);
                        let manual = std::mem::take(&mut h.run_requested);
                        if !manual || h.next_run_at <= now {
                            h.next_run_at = h.schedule.next(now)?;
                        }
                        return Ok(Some(h.clone()));
                    }
                    Ok(None)
                })?;
                if let Some(h) = claimed {
                    return Ok::<_, String>(Some((id, h)));
                }
            }
            Ok(None)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.scheduling = false;
                match result {
                    Ok(Some((id, habit))) => this.start_habit(&id, habit, cx),
                    Ok(None) => (),
                    Err(e) => {
                        this.error = Some(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn start_habit(&mut self, id: &str, habit: Habit, cx: &mut Context<Self>) {
        match habit.target.clone() {
            Target::Pipeline { flow_id } => {
                return self.start_habit_flow(id, habit, &flow_id, None, cx);
            }
            Target::Step { flow_id, step_id } => {
                return self.start_habit_flow(id, habit, &flow_id, Some(&step_id), cx);
            }
            Target::Instruction => {}
        }
        self.active_run = Some(-1);
        let Some(pilot) = self.roster.get(id).map(|p| p.label()) else {
            return;
        };
        let target = id.to_string();
        let habit_id = habit.id.clone();
        let tell = habit.tell;
        let prompt = instruction_prompt(&habit);
        let run = self.spawn_hidden_run(
            id,
            format!("{} · {}", pilot, habit.name),
            prompt,
            AccessMode::FullAccess,
            move |this, session, view, cx| {
                this.observations.push(
                    cx.subscribe(view, move |this, _, _: &SessionChanged, cx| {
                        this.habit_changed(session, cx)
                    }),
                );
                this.running.insert(
                    session,
                    RunningHabit {
                        pilot: target.clone(),
                        habit: habit_id,
                        view: view.clone(),
                        started: now_ms(),
                        launched: now_ms(),
                        failure: None,
                        waiting_since: None,
                        cards: Vec::new(),
                        mailbox: Vec::new(),
                        tell,
                    },
                );
                this.active_run = Some(session);
                if let Some(m) = this.roster.get_mut(&target) {
                    m.habit_sessions.push(session);
                }
            },
            cx,
        );
        cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.active_run = None;
                    this.error = Some(e);
                }
                this.changed(cx);
            });
        })
        .detach();
    }
    /// Starts the habit's Flow, or one step of it, through the same path as
    /// `flows.start`. Its goal holds only this run, so the goal's rollup is
    /// this run's report, and the Flow's own alerts carry its failures.
    fn start_habit_flow(
        &mut self,
        id: &str,
        habit: Habit,
        flow_id: &str,
        step: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let launched = now_ms();
        let goal = format!("Scheduled: {}", habit.name)
            .chars()
            .take(300)
            .collect::<String>();
        let tag = RunTag {
            goal: goal.clone(),
            task: match step {
                Some(step) => format!("Step {step}"),
                None => "Whole pipeline".into(),
            },
        };
        let started = match (self.roster.get(id).cloned(), self.flow_store(cx)) {
            (Some(pilot), Some(store)) => {
                self.launch_pilot_flow(&pilot, flow_id, step, HashMap::new(), tag, &store, cx)
            }
            (None, _) => Err("Pilot is no longer available".into()),
            (_, None) => Err("Flows are not ready".into()),
        };
        match started {
            Ok(run) => {
                if let Some(pilot) = self.roster.get_mut(id) {
                    pilot.goals.retain(|g| !(g.name == goal && g.reported));
                }
                self.flow_watch.habits.insert(
                    run.id.clone(),
                    ScheduledFlow {
                        pilot: id.into(),
                        habit: habit.id.clone(),
                        name: habit.name.clone(),
                        tell: habit.tell,
                        launched,
                    },
                );
                self.flow_started(id, run);
                self.flows_changed(id, cx);
                self.save(cx);
            }
            Err(error) => {
                let scheduled = ScheduledFlow {
                    pilot: id.into(),
                    habit: habit.id.clone(),
                    name: habit.name.clone(),
                    tell: habit.tell,
                    launched,
                };
                self.finish_scheduled(scheduled, Err(error), cx);
            }
        }
        self.changed(cx);
    }
    /// A Flow run a schedule started ended; `told` says whether its goal's
    /// rollup went to the chat.
    pub(super) fn scheduled_flow_ended(
        &mut self,
        scheduled: ScheduledFlow,
        run: &FlowRun,
        told: bool,
        cx: &mut Context<Self>,
    ) {
        let result = if run.status == RunStatus::Succeeded {
            Ok(told)
        } else {
            Err(run
                .error
                .clone()
                .unwrap_or_else(|| format!("The run {}", run.status.as_str())))
        };
        // The Flow's own alerts already told the Pilot about a failed run.
        self.record_scheduled(scheduled, result, cx);
    }
    /// A scheduled Flow that never started: recorded and, since it needs
    /// the person, told in the chat whatever the habit's `tell`.
    fn finish_scheduled(
        &mut self,
        scheduled: ScheduledFlow,
        result: Result<bool, String>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = &result {
            if let Some(m) = self.roster.get_mut(&scheduled.pilot) {
                m.pending_reports.push(format!(
                    "**Scheduled run failed**: {}\n\n{error}",
                    scheduled.name
                ));
            }
            let pilot = scheduled.pilot.clone();
            self.ensure_chat(&pilot, cx);
            self.record_scheduled(scheduled, result, cx);
            self.save(cx);
            self.dispatch(&pilot, cx);
        } else {
            self.record_scheduled(scheduled, result, cx);
        }
    }
    /// Files a scheduled Flow's run in its habit's history. `Ok(told)` is a
    /// success that did or did not message the person.
    fn record_scheduled(
        &mut self,
        scheduled: ScheduledFlow,
        result: Result<bool, String>,
        cx: &mut Context<Self>,
    ) {
        let now = now_ms();
        let run = Run {
            at: scheduled.launched,
            duration_ms: now.saturating_sub(scheduled.launched),
            outcome: match result {
                Ok(true) => "posted",
                Ok(false) => "quiet",
                Err(_) => "failed",
            }
            .into(),
            report: None,
            error: result.err(),
        };
        let root = self.root.clone();
        let pilot = scheduled.pilot;
        let target = pilot.clone();
        let habit = scheduled.habit;
        self.file_work(
            pilot,
            move || {
                habits::update(&root, &target, |all| {
                    if let Some(h) = all.iter_mut().find(|h| h.id == habit) {
                        h.finish(run.clone())?;
                    }
                    Ok(())
                })
            },
            cx,
        );
    }
    /// Starts a hidden standalone session for `pilot_id` with its chat's
    /// settings, files it in `roster.hidden_sessions` and sends `prompt` as
    /// an internal message. `register` runs first, so the caller tracks the
    /// session before its turn can report anything. The task's flag says
    /// whether the session took the prompt; if not, no turn will start.
    pub(super) fn spawn_hidden_run(
        &mut self,
        pilot_id: &str,
        title: String,
        prompt: String,
        access: AccessMode,
        register: impl FnOnce(&mut Self, i64, &Entity<Conversation>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<HiddenRun, String>> {
        let Some(pilot) = self.roster.get(pilot_id).cloned() else {
            return gpui::Task::ready(Err("Pilot is no longer available".into()));
        };
        self.ensure_chat(pilot_id, cx);
        let inherited = self
            .chats
            .get(pilot_id)
            .map(|v| v.read(cx).settings.clone())
            .unwrap_or_default();
        let chat_provider = self
            .chats
            .get(pilot_id)
            .map(|v| v.read(cx).summary.provider.clone());
        // The chat is still loading: start where it will.
        let prefs = self.prefs.clone();
        let known = crate::installed_providers::snapshot(cx);
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let provider = chat_provider.unwrap_or_else(|| {
                crate::installed_providers::new_session_choice_resolving(&known, &prefs, None)
                    .0
                    .id()
                    .into()
            });
            let db = locked(&store)?;
            let cwd = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            let mut s = db.create(&provider, &cwd)?;
            s.title = title;
            db.title(s.id, &s.title)?;
            Ok::<_, String>(s)
        });
        cx.spawn(async move |this, cx| {
            let summary = work.await?;
            this.update(cx, |this, cx| {
                let session = summary.id;
                let view = cx.new(|cx| Conversation::new(summary, this.store.clone(), cx));
                let root = this.root.clone();
                let env = this
                    .bridge
                    .as_ref()
                    .map(|b| b.environment(session))
                    .unwrap_or_default();
                let mut settings = inherited;
                settings.access = access;
                settings.internal_message = true;
                settings.pilot = None;
                settings.environment.clear();
                // The main chat's limit is not this session's: like
                // sessions.start, a hidden run gets its own try.
                settings.usage_limit = None;
                view.update(cx, |v, cx| {
                    v.install_transfer(settings, None, cx);
                    v.configure_pilot(pilot, root, env, cx);
                });
                register(this, session, &view, cx);
                this.roster.hidden_sessions.push(session);
                this.save(cx);
                let message = Message {
                    sent_at: Some(gespan_core::time::now_ms()),
                    id: pilot::new_id(),
                    text: prompt,
                    internal: true,
                    error: None,
                    attachments: Vec::new(),
                    report: Vec::new(),
                };
                // Tests never start a provider turn: the prompt is recorded
                // and the session holds it as a turn that is still running.
                #[cfg(test)]
                let held = {
                    this.hidden_prompts.push((session, message.text.clone()));
                    if this.hidden_dispatch_fails {
                        // A session that refuses the prompt before any turn.
                        view.update(cx, |v, _| v.settings.usage_limit = Some(Default::default()));
                    } else {
                        view.update(cx, |v, _| v.hold_history_for_test());
                    }
                    !this.hidden_dispatch_fails
                };
                let dispatched = view.update(cx, |v, cx| v.pilot_dispatch(&message, cx));
                #[cfg(test)]
                let dispatched = if held {
                    view.update(cx, |v, _| v.busy = true);
                    true
                } else {
                    dispatched
                };
                (session, view, dispatched)
            })
            .map_err(|_| "Pilots are no longer available".to_string())
        })
    }
    pub(super) fn habit_changed(&mut self, session: i64, cx: &mut Context<Self>) {
        cx.emit(NotificationChanged);
        let Some(run) = self.running.get(&session) else {
            return;
        };
        if run.view.read(cx).busy {
            self.changed(cx);
            return;
        }
        if let Some(message) = self.running.get_mut(&session).and_then(|r| {
            if r.mailbox.is_empty() {
                None
            } else {
                Some(r.mailbox.remove(0))
            }
        }) {
            let view = self.running[&session].view.clone();
            view.update(cx, |v, cx| v.pilot_dispatch(&message, cx));
            return;
        }
        if self
            .roster
            .pilots
            .iter()
            .flat_map(|m| m.launches.iter())
            .any(|l| {
                completions::launched_from(l.origin_turn, session)
                    && l.notify_on_complete
                    && !l.completed
                    && !l.dismissed
            })
        {
            self.changed(cx);
            return;
        }
        let run = self.running.remove(&session).unwrap();
        self.active_run = None;
        let completion = {
            let view = run.view.read(cx);
            NotificationFinished {
                summary: view.summary.clone(),
                body: view.notification_reply_body(cx),
                habit: true,
                was_cancelled: view.notification_was_cancelled(),
            }
        };
        cx.emit(completion);
        let (mut ok, mut text) = run.view.read(cx).external_outcome(cx);
        if let Some(error) = run.failure {
            ok = false;
            text = error;
        }
        let found = ok
            .then(|| {
                habits::report(&gespan_core::response::prose(
                    run.view.read(cx).pilot_rows(cx),
                ))
            })
            .flatten();
        // Every run reports, even one with nothing to say; one told only when
        // needed stays quiet then. A failure needs the person either way.
        let report = match (ok, run.tell) {
            (true, Tell::EveryRun) => Some(
                found
                    .clone()
                    .unwrap_or_else(|| "Nothing to report.".to_string()),
            ),
            (true, Tell::OnlyIfNeeded) => found.clone(),
            (false, _) => None,
        };
        let title = run.view.read(cx).summary.title.clone();
        let root = self.root.clone();
        let id = run.pilot.clone();
        let habit = run.habit.clone();
        let at = run.launched;
        let duration = now_ms().saturating_sub(at);
        let error = (!ok).then_some(text);
        let failure = error.clone();
        let result = Run {
            at,
            duration_ms: duration,
            outcome: if ok {
                if report.is_some() { "posted" } else { "quiet" }
            } else {
                "failed"
            }
            .into(),
            report: found,
            error,
        };
        self.file_work(
            id.clone(),
            move || {
                habits::update(&root, &id, |all| {
                    if let Some(h) = all.iter_mut().find(|h| h.id == habit) {
                        h.finish(result.clone())?;
                    }
                    Ok(())
                })
            },
            cx,
        );
        let message = report
            .map(|report| format!("**Habit report**\n\n{report}"))
            .or_else(|| {
                failure.map(|error| format!("**Scheduled run failed**: {title}\n\n{error}"))
            });
        if let Some(message) = message {
            if let Some(m) = self.roster.get_mut(&run.pilot) {
                m.pending_reports.push(message);
                for card in &run.cards {
                    m.pending_reports.push(format!("pilot-card:{card}"));
                }
            }
            self.ensure_chat(&run.pilot, cx);
            self.save(cx);
            self.dispatch(&run.pilot, cx);
        }
        self.changed(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use gespan_core::pilot::habits::Schedule;

    #[test]
    fn habits_list_groups_today_with_running_first_and_paused_last() {
        let at = |day, hour| {
            chrono::Local
                .with_ymd_and_hms(2026, 10, day, hour, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis() as u64
        };
        let now = at(12, 12);
        let make = |name: &str, next_run_at| {
            let mut h = Habit::new(
                name.into(),
                "Check updates".into(),
                Schedule::default(),
                now,
            )
            .unwrap();
            h.id = name.into();
            h.next_run_at = next_run_at;
            h
        };
        let mut morning = make("Morning", at(13, 9));
        morning.last_run_at = Some(at(12, 8));
        let mut paused = make("Paused", at(13, 7));
        paused.enabled = false;
        let mut starting = make("Starting", at(13, 9));
        starting.run_requested = true;
        let habits = [
            make("Weekly", at(16, 9)),
            make("Evening", at(12, 18)),
            morning,
            paused,
            starting,
            make("Running", at(13, 9)),
        ];
        assert_eq!(
            grouped_habits(&habits, now, |h| (h.id == "Running").then_some(at(12, 11))),
            [vec![5, 4, 1], vec![2], vec![0, 3]]
        );
    }

    #[test]
    fn a_habit_that_ran_and_is_due_again_today_is_only_upcoming() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 10, 12, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let mut h = Habit::new(
            "Hourly".into(),
            "Check builds".into(),
            Schedule {
                schedule_kind: "hourly".into(),
                ..Default::default()
            },
            now,
        )
        .unwrap();
        h.last_run_at = Some(now - 60_000);
        assert_eq!(
            grouped_habits(&[h], now, |_| None),
            [vec![0], vec![], vec![]]
        );
        assert_eq!(duration(72_000), "1m 12s");
        assert_eq!(duration(60_000), "1m");
    }
}
