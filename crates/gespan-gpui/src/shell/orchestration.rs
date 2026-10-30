//! Orchestrator's reviewed plans create real native sessions, and the lead
//! supervises them through the `control` CLI. The run itself lives in
//! `gespan_core::orchestration::run`; this applies it to live sessions.
use super::*;
use gespan_core::{
    agent::{Decision, Settings},
    catalog,
    orchestration::{
        self, Choice, Plan,
        run::{self, Effect, Needs, Outcome, Run, RunStatus, Status, Wake},
    },
    pilot::bridge::{Call, Server},
    provider::{self, Kind},
};
use serde_json::Value;

/// A `wait` call parked until the run changes or its time is up.
struct Waiter {
    id: u64,
    /// The view the lead last saw.
    baseline: String,
    reply: std::sync::mpsc::Sender<Result<Value, String>>,
}

/// A worker on its way to a session, with what it needs off the main thread.
struct Launch {
    task: String,
    prompt: String,
    title: String,
    files: Vec<String>,
    planned: orchestration::Task,
    kind: Kind,
    cwd: String,
    lead_provider: String,
    settings: Settings,
}

pub(super) struct Live {
    run: Run,
    /// What workers inherit from the lead.
    settings: Settings,
    /// Blocked workers the lead has already been told about.
    announced: HashSet<String>,
    waiters: Vec<Waiter>,
    next_waiter: u64,
    lead_busy: bool,
    /// The run ended and the lead's card and capability were cleared.
    closed: bool,
}

impl Shell {
    pub(super) fn init_control_bridge(&mut self, cx: &mut Context<Self>) {
        if cx.is_test() {
            return;
        }
        match Server::start() {
            Ok((server, calls)) => {
                self.control_bridge = Some(server);
                cx.spawn(async move |this, cx| {
                    while let Ok(call) = calls.recv().await {
                        if this
                            .update(cx, |this, cx| this.handle_control_call(call, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(error) => self.error = Some(format!("Orchestration control: {error}")),
        }
    }
    pub(super) fn observe_orchestration(
        &mut self,
        view: &Entity<Conversation>,
        id: i64,
        cx: &mut Context<Self>,
    ) {
        use crate::conversation::composer_features::{StartOrchestration, StopOrchestration};
        self.observations.push(cx.subscribe(
            view,
            move |this, _, event: &StartOrchestration, cx| {
                this.start_orchestration(id, event.0.clone(), cx)
            },
        ));
        self.observations.push(
            cx.subscribe(view, move |this, _, _: &StopOrchestration, cx| {
                this.stop_orchestration(id, cx)
            }),
        );
    }
    fn start_orchestration(&mut self, lead: i64, plan: Plan, cx: &mut Context<Self>) {
        if self
            .orchestrations
            .get(&lead)
            .is_some_and(|live| live.run.live())
        {
            return;
        }
        if let Err(error) = orchestration::validate(&plan) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let Some(view) = self.sessions.get(&lead).cloned() else {
            return;
        };
        let conversation = view.read(cx);
        if conversation.busy {
            return;
        }
        let cwd = conversation
            .settings
            .worktree_cwd
            .clone()
            .unwrap_or_else(|| conversation.summary.cwd.clone());
        if self
            .orchestrations
            .values()
            .any(|live| live.run.live() && live.run.cwd == cwd)
        {
            self.error = Some("An orchestration is already using this checkout.".into());
            cx.notify();
            return;
        }
        let mut settings = conversation.settings.clone();
        settings.plan = false;
        settings.orchestrator = false;
        settings.orchestration_worker = true;
        settings.operator = false;
        settings.operator_enabled = false;
        settings.handoff = None;
        settings.turns.clear();
        settings.environment.clear();
        settings.attachments.clear();
        let provider = conversation.summary.provider.clone();
        let request = conversation.last_user_text(cx);
        let profiles = conversation.orchestration_profiles(cx);
        // Plans may name pairs the catalog hasn't confirmed yet.
        let mut allowed: Vec<Choice> = Vec::new();
        for task in &plan.tasks {
            if let (Some(harness), Some(model)) = (&task.harness, &task.model) {
                allowed.push(Choice {
                    harness: harness.clone(),
                    model: model.clone(),
                    name: model.clone(),
                });
            }
        }
        let run = Run::new(plan, cwd, provider, allowed);
        let first_turn = orchestration::approved_prompt(&request, &run);
        if let Some(bridge) = &self.control_bridge {
            let environment = bridge.control_environment(lead);
            view.update(cx, |v, _| v.set_control_environment(environment));
        }
        self.orchestrations.insert(
            lead,
            Live {
                run,
                settings,
                announced: HashSet::new(),
                waiters: Vec::new(),
                next_waiter: 0,
                lead_busy: true,
                closed: false,
            },
        );
        self.schedule_orchestration(lead, cx);
        view.update(cx, |v, cx| v.send_coordinator_report(first_turn, cx));
        let work = cx
            .background_executor()
            .spawn(async move { orchestration::choices(profiles) });
        cx.spawn(async move |this, cx| {
            let catalog = work.await;
            let _ = this.update(cx, |this, _| {
                if let Some(live) = this.orchestrations.get_mut(&lead) {
                    for choice in catalog {
                        match live.run.allowed.iter_mut().find(|known| {
                            known.harness == choice.harness && known.model == choice.model
                        }) {
                            Some(known) => known.name = choice.name,
                            None => live.run.allowed.push(choice),
                        }
                    }
                }
            });
        })
        .detach();
    }
    fn stop_orchestration(&mut self, lead: i64, cx: &mut Context<Self>) {
        if let Some(live) = self.orchestrations.get_mut(&lead) {
            let effects = live.run.stop();
            self.apply_orchestration_effects(lead, effects, cx);
        }
        self.schedule_orchestration(lead, cx);
    }
    fn worker_view(&self, lead: i64, task: &str) -> Option<Entity<Conversation>> {
        let id = self.orchestrations.get(&lead)?.run.task(task)?.session?;
        self.sessions.get(&id).cloned()
    }
    fn apply_orchestration_effects(
        &mut self,
        lead: i64,
        effects: Vec<Effect>,
        cx: &mut Context<Self>,
    ) {
        for effect in effects {
            match effect {
                Effect::StopLead => {
                    if let Some(view) = self.sessions.get(&lead) {
                        view.update(cx, |v, cx| v.stop_if_busy(cx));
                    }
                }
                Effect::Stop(task) => {
                    if let Some(view) = self.worker_view(lead, &task) {
                        view.update(cx, |v, cx| v.stop_if_busy(cx));
                    } else if let Some(live) = self.orchestrations.get_mut(&lead) {
                        // No session yet: nothing will ever report it stopped.
                        live.run.settle(&task, Outcome::Failed, "", None);
                    }
                }
                Effect::Steer { task, text } => {
                    if let Some(view) = self.worker_view(lead, &task) {
                        view.update(cx, |v, cx| v.steer_worker(text, cx));
                    }
                }
                Effect::Approve {
                    task,
                    request,
                    allow,
                } => {
                    if let Some(view) = self.worker_view(lead, &task) {
                        let decision = if allow {
                            Decision::Allow
                        } else {
                            Decision::Deny
                        };
                        view.update(cx, |v, cx| {
                            v.decide_notification_request(&request, decision, cx)
                        });
                    }
                }
                Effect::Answer {
                    task,
                    request,
                    answers,
                } => {
                    if let Some(view) = self.worker_view(lead, &task) {
                        let answers =
                            answers.and_then(|answers| serde_json::from_value(answers).ok());
                        view.update(cx, |v, cx| v.answer_pending_question(&request, answers, cx));
                    }
                }
            }
        }
    }
    fn orchestration_needs(&self, lead: i64, cx: &App) -> Needs {
        let Some(live) = self.orchestrations.get(&lead) else {
            return Needs::new();
        };
        live.run
            .tasks
            .iter()
            .filter(|task| task.status == Status::Running)
            .filter_map(|task| {
                let view = self.sessions.get(&task.session?)?;
                Some((task.id.clone(), view.read(cx).pending_input(cx)?))
            })
            .collect()
    }
    fn handle_control_call(&mut self, call: Call, cx: &mut Context<Self>) {
        let lead = call.session;
        if call.request_id.is_empty()
            || !call
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            let _ = call.reply.send(Err(
                "request ID must use letters, digits, underscores or hyphens".into(),
            ));
            return;
        }
        let needs = self.orchestration_needs(lead, cx);
        let Some(live) = self.orchestrations.get_mut(&lead) else {
            let _ = call
                .reply
                .send(Err("No orchestration run was found for this lead".into()));
            return;
        };
        if call.action == "wait" {
            let seconds = match run::wait_seconds(&call.input) {
                Ok(seconds) => seconds,
                Err(error) => {
                    let _ = call.reply.send(Err(error));
                    return;
                }
            };
            let view = live.run.view(&needs);
            if seconds == 0 || !live.run.should_wait(&needs) {
                let _ = call.reply.send(Ok(view));
                return;
            }
            let id = live.next_waiter;
            live.next_waiter += 1;
            live.waiters.push(Waiter {
                id,
                baseline: view.to_string(),
                reply: call.reply,
            });
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(seconds))
                    .await;
                let _ = this.update(cx, |this, cx| this.finish_orchestration_wait(lead, id, cx));
            })
            .detach();
            return;
        }
        if call.action == "steer"
            && let Some(task) = call.input["taskId"]
                .as_str()
                .and_then(|id| live.run.task(id))
        {
            let kind = Kind::from_id(task.harness.as_deref().unwrap_or(&live.run.provider));
            if !kind.can_steer() {
                let _ = call.reply.send(Err(format!(
                    "{} cannot take guidance mid-turn. Wait for the turn to finish, then use message.",
                    kind.label()
                )));
                return;
            }
        }
        match live
            .run
            .control(&call.request_id, &call.action, &call.input, &needs)
        {
            Ok(reply) => {
                self.apply_orchestration_effects(lead, reply.effects, cx);
                let _ = call.reply.send(Ok(reply.result));
            }
            Err(error) => {
                let _ = call.reply.send(Err(error));
            }
        }
        self.schedule_orchestration(lead, cx);
    }
    fn finish_orchestration_wait(&mut self, lead: i64, id: u64, cx: &mut Context<Self>) {
        let needs = self.orchestration_needs(lead, cx);
        if let Some(live) = self.orchestrations.get_mut(&lead)
            && let Some(ix) = live.waiters.iter().position(|waiter| waiter.id == id)
        {
            let waiter = live.waiters.remove(ix);
            let _ = waiter.reply.send(Ok(live.run.view(&needs)));
        }
    }
    /// Wakes every `wait` whose view changed, including a worker that became
    /// blocked, which changes no run state of its own.
    fn answer_orchestration_waits(&mut self, lead: i64, cx: &App) {
        let needs = self.orchestration_needs(lead, cx);
        let Some(live) = self.orchestrations.get_mut(&lead) else {
            return;
        };
        if live.waiters.is_empty() {
            return;
        }
        let view = live.run.view(&needs);
        let current = view.to_string();
        let (changed, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut live.waiters)
            .into_iter()
            .partition(|waiter| waiter.baseline != current);
        live.waiters = waiting;
        for waiter in changed {
            let _ = waiter.reply.send(Ok(view.clone()));
        }
    }
    pub(super) fn orchestration_session_changed(&mut self, id: i64, cx: &mut Context<Self>) {
        // A held worker hasn't finished; its outcome comes after Resume.
        let held = self.session_held(id, self.sessions.get(&id).map(|v| v.read(cx)), cx);
        let leads: Vec<_> = self
            .orchestrations
            .iter()
            .filter(|(lead, live)| **lead == id || live.run.task_of_session(id).is_some())
            .map(|(lead, _)| *lead)
            .collect();
        for lead in leads {
            let Some(view) = self.sessions.get(&id).cloned() else {
                continue;
            };
            let view = view.read(cx);
            let idle = !view.external_submission_pending();
            let (success, report) = view.external_outcome(cx);
            let Some(live) = self.orchestrations.get_mut(&lead) else {
                continue;
            };
            if id == lead {
                let ended = live.lead_busy && idle && !held;
                live.lead_busy = !idle;
                if ended && !success && live.run.status == RunStatus::Active {
                    let effects = live.run.pause(
                        "The lead was interrupted. Its agents were stopped; review and resume the run.",
                    );
                    self.apply_orchestration_effects(lead, effects, cx);
                }
            } else if idle
                && !held
                && let Some(task) = live.run.task_of_session(id).map(|task| task.id.clone())
            {
                let outcome = if success {
                    Outcome::Completed
                } else {
                    Outcome::Failed
                };
                let error = (!success).then(|| report.clone());
                live.run.settle(&task, outcome, &report, error);
            }
            self.schedule_orchestration(lead, cx);
        }
    }
    fn publish_orchestration(&mut self, lead: i64, cx: &mut Context<Self>) {
        let Some(live) = self.orchestrations.get(&lead) else {
            return;
        };
        let mut summary = live.run.summary();
        // A task with no model of its own runs the lead's.
        for task in &mut summary.tasks {
            if task.model.is_none() && task.provider == live.run.provider {
                task.model = live.settings.model.clone();
            }
        }
        let progress = match &live.run.error {
            Some(error) if live.run.status == RunStatus::Paused => {
                format!("{}\n{error}", live.run.progress())
            }
            _ => live.run.progress(),
        };
        let open = live.run.live();
        if self.orchestration_summaries.get(&lead) != Some(&summary) {
            let store = self.store.clone();
            let saved = summary.clone();
            cx.background_executor()
                .spawn(async move {
                    let _ = locked(&store).and_then(|s| s.save_orchestration_summary(lead, &saved));
                })
                .detach();
            self.orchestration_summaries.insert(lead, summary);
            self.invalidate_chrome(cx);
        }
        if let Some(view) = self.sessions.get(&lead) {
            view.update(cx, |v, cx| {
                v.set_orchestration_progress(open.then_some(progress), cx)
            });
        }
    }
    fn close_orchestration(&mut self, lead: i64, cx: &mut Context<Self>) {
        let Some(live) = self.orchestrations.get_mut(&lead) else {
            return;
        };
        if live.closed || live.run.live() {
            return;
        }
        live.closed = true;
        if let Some(view) = self.sessions.get(&lead) {
            view.update(cx, |v, _| {
                v.set_control_environment(Default::default());
                v.set_orchestration_plan(None);
            });
        }
    }
    /// The idle lead hears finished results and blocked workers.
    fn wake_orchestration_lead(&mut self, lead: i64, cx: &mut Context<Self>) {
        let needs = self.orchestration_needs(lead, cx);
        let Some(view) = self.sessions.get(&lead).cloned() else {
            return;
        };
        if view.read(cx).external_submission_pending() || view.read(cx).has_follow_ups() {
            return;
        }
        let Some(live) = self.orchestrations.get_mut(&lead) else {
            return;
        };
        match live.run.wake(&needs, &mut live.announced) {
            Wake::Nothing => (),
            Wake::Send(body) => {
                live.lead_busy = true;
                view.update(cx, |v, cx| v.send_coordinator_report(body, cx));
            }
            Wake::Paused(effects) => self.apply_orchestration_effects(lead, effects, cx),
        }
    }
    pub(super) fn schedule_orchestration(&mut self, lead: i64, cx: &mut Context<Self>) {
        // Nothing is dispatched while held; Resume schedules every run again.
        // The card still locks: this run already holds the plan it will use.
        if crate::hold::active(cx) {
            if self.orchestrations.contains_key(&lead)
                && let Some(view) = self.sessions.get(&lead)
            {
                view.update(cx, |v, cx| {
                    if v.orchestration_progress.is_none() {
                        v.set_orchestration_progress(Some("Waiting for Resume".into()), cx);
                    }
                });
            }
            return;
        }
        let Some(live) = self.orchestrations.get_mut(&lead) else {
            return;
        };
        let reports = |run: &Run, task: &run::RunTask| {
            task.depends_on
                .iter()
                .filter_map(|id| run.task(id))
                .map(|dependency| format!("{}: {}", dependency.title, dependency.result))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let started = live.run.dispatch();
        let mut followups = Vec::new();
        let mut jobs = Vec::new();
        for id in started {
            let Some(task) = live.run.task(&id) else {
                continue;
            };
            let reports = reports(&live.run, task);
            let text = task.recovery_prompt.as_ref().unwrap_or(&task.prompt);
            let mut prompt = orchestration::worker_prompt(text, &task.files);
            if !reports.is_empty() {
                prompt = format!("{prompt}\n\nDependency reports:\n{reports}");
            }
            if let Some(session) = task.session {
                followups.push((session, prompt));
                continue;
            }
            let planned = orchestration::Task {
                id: task.id.clone(),
                title: task.title.clone(),
                prompt: task.prompt.clone(),
                files: task.files.clone(),
                depends_on: task.depends_on.clone(),
                harness: task.harness.clone(),
                model: task.model.clone(),
                model_settings: task.model_settings.clone(),
            };
            jobs.push(Launch {
                task: task.id.clone(),
                prompt,
                title: task.title.clone(),
                files: task.files.clone(),
                kind: planned.kind(&live.run.provider),
                planned,
                cwd: live.run.cwd.clone(),
                lead_provider: live.run.provider.clone(),
                settings: live.settings.clone(),
            });
        }
        for (session, prompt) in followups {
            if let Some(view) = self.sessions.get(&session) {
                view.update(cx, |v, cx| v.send_worker_turn(prompt, cx));
            }
        }
        for job in jobs {
            self.launch_orchestration_worker(lead, job, cx);
        }
        self.publish_orchestration(lead, cx);
        self.answer_orchestration_waits(lead, cx);
        self.close_orchestration(lead, cx);
        self.wake_orchestration_lead(lead, cx);
    }
    fn launch_orchestration_worker(&mut self, lead: i64, job: Launch, cx: &mut Context<Self>) {
        let Launch {
            task: task_id,
            prompt,
            title,
            files,
            planned,
            kind,
            cwd,
            lead_provider,
            mut settings,
        } = job;
        let store = self.store.clone();
        let project = self.sessions[&lead].read(cx).summary.cwd.clone();
        if kind.id() != lead_provider {
            settings.account = crate::accounts::get(cx)
                .map(|center| center.read(cx).selected(kind, &project))
                .unwrap_or_default();
        }
        let work = cx.background_executor().spawn(async move {
            let root = std::path::Path::new(&cwd)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            // Existing symlink parents must not redirect an assignment outside the checkout.
            for file in &files {
                let mut path = root.join(file);
                while !path.exists() {
                    if !path.pop() {
                        break;
                    }
                }
                if !path
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&root)
                {
                    return Err(format!("Assignment scope escapes the checkout: {file}"));
                }
            }
            if planned.harness.is_some() && provider::resolve(kind).is_none() {
                return Err(orchestration::UNAVAILABLE.to_owned());
            }
            let models = if orchestration::needs_catalog(&planned, &lead_provider) {
                catalog::models_for(kind, &settings.account)?
            } else {
                catalog::cached_models_for(kind, &settings.account).unwrap_or_default()
            };
            let mut settings =
                orchestration::worker_settings(&planned, &lead_provider, settings, kind, &models)?;
            let store = locked(&store)?;
            let mut summary = store.create(kind.id(), &project)?;
            summary.title = format!("Agent · {title}");
            store.title(summary.id, &summary.title)?;
            settings.worktree_cwd = Some(cwd);
            store.save_agent_settings(summary.id, &settings)?;
            Ok::<_, String>((summary, settings))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((summary, settings)) => {
                        let id = summary.id;
                        this.add(summary, cx);
                        let start = this.orchestrations.get(&lead).is_some_and(|live| {
                            live.run.status == RunStatus::Active
                                && live
                                    .run
                                    .task(&task_id)
                                    .is_some_and(|task| task.status == Status::Running)
                        });
                        if let Some(live) = this.orchestrations.get_mut(&lead) {
                            live.run.set_session(&task_id, id);
                        }
                        if start {
                            this.sessions[&id]
                                .update(cx, |v, cx| v.install_transfer(settings, Some(prompt), cx));
                        } else if let Some(live) = this.orchestrations.get_mut(&lead) {
                            // Stopped while it was being created.
                            live.run.settle(&task_id, Outcome::Failed, "", None);
                        }
                    }
                    Err(error) => {
                        if let Some(live) = this.orchestrations.get_mut(&lead) {
                            live.run.settle(&task_id, Outcome::Failed, "", Some(error));
                        }
                    }
                }
                this.invalidate_chrome(cx);
                this.schedule_orchestration(lead, cx);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::orchestration::Task;
    use gpui::TestAppContext;
    use serde_json::json;
    #[gpui::test]
    fn the_sidebar_lists_each_tasks_own_harness_and_model(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&lead].update(cx, |v, cx| {
                    v.install_transfer(
                        Settings {
                            model: Some("lead-model".into()),
                            ..Settings::default()
                        },
                        None,
                        cx,
                    )
                });
                let task = |id: &str, file: &str| Task {
                    id: id.into(),
                    title: id.into(),
                    prompt: "Work".into(),
                    files: vec![file.into()],
                    depends_on: Vec::new(),
                    harness: None,
                    model: None,
                    model_settings: Default::default(),
                };
                let mut own = task("own", "src");
                own.harness = Some("codex".into());
                own.model = Some("gpt-6".into());
                shell.start_orchestration(
                    lead,
                    Plan {
                        title: "Review".into(),
                        summary: String::new(),
                        tasks: vec![own, task("inherits", "docs")],
                        max_workers: orchestration::MAX_WORKERS,
                    },
                    cx,
                );
                let provider = shell.sessions[&lead].read(cx).summary.provider.clone();
                let tasks = &shell.orchestration_summaries[&lead].tasks;
                assert_eq!(tasks[0].provider, "codex");
                assert_eq!(tasks[0].model.as_deref(), Some("gpt-6"));
                assert_eq!(tasks[1].provider, provider);
                assert_eq!(tasks[1].model.as_deref(), Some("lead-model"));
                shell.stop_orchestration(lead, cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    fn control(
        lead: i64,
        request: &str,
        action: &str,
        input: Value,
    ) -> (Call, std::sync::mpsc::Receiver<Result<Value, String>>) {
        let (reply, result) = std::sync::mpsc::channel();
        (
            Call {
                session: lead,
                request_id: request.into(),
                action: action.into(),
                input,
                reply,
            },
            result,
        )
    }
    fn start_one(shell: &mut Shell, lead: i64, cx: &mut Context<Shell>) {
        shell.sessions[&lead].update(cx, |v, cx| {
            v.install_transfer(Settings::default(), None, cx)
        });
        shell.start_orchestration(
            lead,
            Plan {
                title: "Review".into(),
                summary: String::new(),
                tasks: vec![Task {
                    id: "a".into(),
                    title: "A".into(),
                    prompt: "Work".into(),
                    files: vec!["src".into()],
                    depends_on: Vec::new(),
                    harness: None,
                    model: None,
                    model_settings: Default::default(),
                }],
                max_workers: 1,
            },
            cx,
        );
    }
    #[gpui::test]
    fn a_session_without_a_run_has_no_control(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                let (call, result) = control(lead, "r1", "list", json!({}));
                shell.handle_control_call(call, cx);
                let error = result.try_recv().unwrap().unwrap_err();
                assert!(error.contains("No orchestration run"), "{error}");
            })
            .unwrap();
    }
    #[gpui::test]
    fn the_lead_lists_and_cancels_through_control(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                start_one(shell, lead, cx);
                let (call, result) = control(lead, "r1", "list", json!({}));
                shell.handle_control_call(call, cx);
                let listed = result.try_recv().unwrap().unwrap();
                assert_eq!(listed["run"]["tasks"][0]["status"], "running");
                let task = listed["run"]["tasks"][0]["id"].clone();
                let (call, result) = control(lead, "r2", "cancel", json!({"taskId":task}));
                shell.handle_control_call(call, cx);
                assert_eq!(result.try_recv().unwrap().unwrap()["cancelled"], true);
                let (call, result) = control(lead, "bad/id", "list", json!({}));
                shell.handle_control_call(call, cx);
                assert!(result.try_recv().unwrap().is_err());
                shell.stop_orchestration(lead, cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    #[gpui::test]
    fn wait_parks_until_the_run_changes(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        let result = window
            .update(cx, |shell, _, cx| {
                start_one(shell, lead, cx);
                let (call, result) = control(lead, "r1", "wait", json!({"timeoutSeconds":20}));
                shell.handle_control_call(call, cx);
                assert!(result.try_recv().is_err(), "nothing changed yet");
                let needs = shell.orchestration_needs(lead, cx);
                let live = shell.orchestrations.get_mut(&lead).unwrap();
                let task = live.run.view(&needs)["tasks"][0]["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                live.run.settle(&task, Outcome::Completed, "Done", None);
                shell.schedule_orchestration(lead, cx);
                result
            })
            .unwrap();
        let view = result.try_recv().unwrap().unwrap();
        assert_eq!(view["tasks"][0]["status"], "completed");
        window
            .update(cx, |shell, _, cx| {
                // Nothing left to wait for, so the next call returns at once.
                let (call, result) = control(lead, "r2", "wait", json!({"timeoutSeconds":20}));
                shell.handle_control_call(call, cx);
                assert!(result.try_recv().unwrap().is_ok());
                let (call, result) = control(lead, "r3", "wait", json!({"timeoutSeconds":99}));
                shell.handle_control_call(call, cx);
                assert!(result.try_recv().unwrap().is_err());
                shell.stop_orchestration(lead, cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    #[gpui::test]
    fn a_finished_run_clears_the_card_and_answers_parked_waits(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                start_one(shell, lead, cx);
                shell.sessions[&lead].update(cx, |v, _| {
                    v.set_control_environment(HashMap::from([(
                        "GESPAN_CONTROL_TOKEN".to_owned(),
                        "secret".to_owned(),
                    )]))
                });
                let (call, parked) = control(lead, "r1", "wait", json!({"timeoutSeconds":20}));
                shell.handle_control_call(call, cx);
                shell.stop_orchestration(lead, cx);
                assert!(parked.try_recv().unwrap().is_ok());
                let view = shell.sessions[&lead].read(cx);
                assert!(view.orchestration_progress.is_none());
                assert!(!view.has_control_environment());
                assert!(shell.orchestration_summaries[&lead].tasks.len() == 1);
                assert!(!shell.orchestration_summaries[&lead].live);
            })
            .unwrap();
        cx.run_until_parked();
    }
    #[gpui::test]
    fn a_plan_started_on_hold_locks_its_card_and_dispatches_nothing(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&lead].update(cx, |v, cx| {
                    v.install_transfer(Settings::default(), None, cx)
                });
                cx.set_global(crate::hold::Hold(Some(gespan_core::hold::HoldState::new(
                    1,
                ))));
                shell.start_orchestration(
                    lead,
                    Plan {
                        title: "Review".into(),
                        summary: String::new(),
                        tasks: vec![Task {
                            id: "a".into(),
                            title: "A".into(),
                            prompt: "Work".into(),
                            files: vec!["src".into()],
                            depends_on: Vec::new(),
                            harness: None,
                            model: None,
                            model_settings: Default::default(),
                        }],
                        max_workers: 1,
                    },
                    cx,
                );
                let run = &shell.orchestrations.get(&lead).unwrap().run;
                assert!(run.tasks[0].status == Status::Queued);
                assert_eq!(
                    shell.sessions[&lead]
                        .read(cx)
                        .orchestration_progress
                        .as_deref(),
                    Some("Waiting for Resume"),
                    "Edits after this point would not reach the run"
                );
                cx.set_global(crate::hold::Hold(None));
                shell.stop_orchestration(lead, cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    #[gpui::test]
    fn the_worker_count_chosen_on_the_card_limits_dispatch(cx: &mut TestAppContext) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&lead].update(cx, |v, cx| {
                    v.install_transfer(Settings::default(), None, cx)
                });
                let tasks = ["src", "tests", "docs"]
                    .into_iter()
                    .map(|file| Task {
                        id: file.into(),
                        title: file.into(),
                        prompt: "Work".into(),
                        files: vec![file.into()],
                        depends_on: Vec::new(),
                        harness: None,
                        model: None,
                        model_settings: Default::default(),
                    })
                    .collect();
                shell.start_orchestration(
                    lead,
                    Plan {
                        title: "Review".into(),
                        summary: String::new(),
                        tasks,
                        max_workers: 2,
                    },
                    cx,
                );
                let run = &shell.orchestrations.get(&lead).unwrap().run;
                let starting = run
                    .tasks
                    .iter()
                    .filter(|t| t.status == Status::Running)
                    .count();
                assert_eq!(starting, 2);
                shell.stop_orchestration(lead, cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    #[gpui::test]
    fn reviewed_dispatch_caps_workers_serializes_scopes_and_stops_before_launch(
        cx: &mut TestAppContext,
    ) {
        let (window, _, lead) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&lead].update(cx, |v, cx| {
                    v.install_transfer(Settings::default(), None, cx)
                });
                let tasks = ["src", "src/main.rs", "tests", "docs", "examples", "scripts"]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, file)| Task {
                        id: format!("task-{ix}"),
                        title: format!("Assignment {ix}"),
                        prompt: "Inspect and report".into(),
                        files: vec![file.into()],
                        depends_on: Vec::new(),
                        harness: None,
                        model: None,
                        model_settings: Default::default(),
                    })
                    .collect();
                shell.start_orchestration(
                    lead,
                    Plan {
                        title: "Review".into(),
                        summary: String::new(),
                        tasks,
                        max_workers: orchestration::MAX_WORKERS,
                    },
                    cx,
                );
                let run = &shell.orchestrations.get(&lead).unwrap().run;
                assert_eq!(
                    run.tasks
                        .iter()
                        .filter(|t| t.status == Status::Running)
                        .count(),
                    4
                );
                assert!(
                    run.tasks[1].status == Status::Queued,
                    "overlapping src/main.rs waits for src"
                );
                shell.stop_orchestration(lead, cx);
                assert!(!shell.orchestrations[&lead].run.live());
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.sessions.values().all(|v| !v.read(cx).busy))
            })
            .unwrap();
    }
}
