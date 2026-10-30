//! Orchestrator's reviewed plans create real native sessions. Dependencies and
//! overlapping file scopes gate dispatch; approvals stay in each worker's chat.
use super::*;
use gespan_core::{
    agent::Settings,
    catalog,
    orchestration::{self, Plan, Task},
    provider::{self, Kind},
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Starting,
    Running,
    Done,
    Failed,
    Blocked,
}

/// A worker on its way to a session, with what it needs off the main thread.
struct Launch {
    ix: usize,
    task: Task,
    kind: Kind,
    cwd: String,
    lead_provider: String,
    settings: Settings,
    reports: String,
}

struct Assignment {
    task: Task,
    session: Option<i64>,
    state: State,
    report: String,
}
pub(super) struct Run {
    cwd: String,
    provider: String,
    settings: Settings,
    max_workers: usize,
    tasks: Vec<Assignment>,
    stopped: bool,
}
impl Shell {
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
        if self.orchestrations.contains_key(&lead) {
            return;
        }
        if let Err(error) = orchestration::validate(&plan) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let Some(view) = self.sessions.get(&lead) else {
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
        if self.orchestrations.values().any(|run| run.cwd == cwd) {
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
        let max_workers = plan.max_workers;
        let tasks = plan
            .tasks
            .into_iter()
            .map(|task| Assignment {
                task,
                session: None,
                state: State::Pending,
                report: String::new(),
            })
            .collect();
        self.orchestrations.insert(
            lead,
            Run {
                cwd,
                provider,
                settings,
                max_workers,
                tasks,
                stopped: false,
            },
        );
        self.schedule_orchestration(lead, cx);
    }
    fn stop_orchestration(&mut self, lead: i64, cx: &mut Context<Self>) {
        if let Some(run) = self.orchestrations.get_mut(&lead) {
            run.stopped = true;
            for task in &mut run.tasks {
                if matches!(task.state, State::Pending | State::Starting) {
                    task.state = State::Blocked;
                }
                if task.state == State::Running
                    && let Some(view) = task.session.and_then(|id| self.sessions.get(&id))
                {
                    view.update(cx, |v, cx| v.stop_if_busy(cx));
                }
            }
        }
        self.schedule_orchestration(lead, cx);
    }
    pub(super) fn orchestration_session_changed(&mut self, id: i64, cx: &mut Context<Self>) {
        // A held worker hasn't finished; its outcome comes after Resume.
        let held = self.session_held(id, self.sessions.get(&id).map(|v| v.read(cx)), cx);
        let leads: Vec<_> = self
            .orchestrations
            .iter()
            .filter(|(lead, run)| **lead == id || run.tasks.iter().any(|t| t.session == Some(id)))
            .map(|(lead, _)| *lead)
            .collect();
        for lead in leads {
            if let Some(run) = self.orchestrations.get_mut(&lead) {
                for task in &mut run.tasks {
                    if task.session == Some(id)
                        && task.state == State::Running
                        && let Some(view) = self.sessions.get(&id)
                    {
                        let view = view.read(cx);
                        if !view.busy && !held {
                            let (success, report) = view.external_outcome(cx);
                            task.report = report;
                            task.state = if success { State::Done } else { State::Failed };
                        }
                    }
                }
            }
            self.schedule_orchestration(lead, cx);
        }
    }
    pub(super) fn schedule_orchestration(&mut self, lead: i64, cx: &mut Context<Self>) {
        // Nothing is dispatched while held; Resume schedules every run again.
        if crate::hold::active(cx) {
            return;
        }
        let Some(run) = self.orchestrations.get_mut(&lead) else {
            return;
        };
        // Failed dependencies transitively block their dependants.
        loop {
            let failed: HashSet<_> = run
                .tasks
                .iter()
                .filter(|t| matches!(t.state, State::Failed | State::Blocked))
                .map(|t| t.task.id.clone())
                .collect();
            let mut changed = false;
            for task in &mut run.tasks {
                if task.state == State::Pending
                    && task.task.depends_on.iter().any(|id| failed.contains(id))
                {
                    task.state = State::Blocked;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut occupied: Vec<Vec<String>> = run
            .tasks
            .iter()
            .filter(|t| matches!(t.state, State::Starting | State::Running))
            .map(|t| t.task.files.clone())
            .collect();
        let completed: HashSet<_> = run
            .tasks
            .iter()
            .filter(|t| t.state == State::Done)
            .map(|t| t.task.id.clone())
            .collect();
        let reports = run
            .tasks
            .iter()
            .filter(|t| t.state == State::Done)
            .map(|t| format!("{}: {}", t.task.title, t.report))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut jobs = Vec::new();
        if !run.stopped {
            for (ix, task) in run.tasks.iter_mut().enumerate() {
                if occupied.len() >= run.max_workers {
                    break;
                }
                if task.state != State::Pending
                    || !task.task.depends_on.iter().all(|id| completed.contains(id))
                    || occupied
                        .iter()
                        .any(|files| orchestration::overlaps(files, &task.task.files))
                {
                    continue;
                }
                task.state = State::Starting;
                occupied.push(task.task.files.clone());
                jobs.push(Launch {
                    ix,
                    kind: task.task.kind(&run.provider),
                    task: task.task.clone(),
                    cwd: run.cwd.clone(),
                    lead_provider: run.provider.clone(),
                    settings: run.settings.clone(),
                    reports: reports.clone(),
                });
            }
        }
        let done = run
            .tasks
            .iter()
            .all(|t| matches!(t.state, State::Done | State::Failed | State::Blocked));
        let progress = run
            .tasks
            .iter()
            .map(|t| {
                format!(
                    "{}: {}{}",
                    t.task.title,
                    match t.state {
                        State::Pending => "waiting",
                        State::Starting => "starting",
                        State::Running => "working",
                        State::Done => "done",
                        State::Failed => "failed",
                        State::Blocked => "blocked",
                    },
                    t.session
                        .map(|id| format!(" (session {id})"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The sidebar lists the lead's agents from this, during and after the run.
        let summary = orchestration::Summary {
            live: !done,
            tasks: run
                .tasks
                .iter()
                .map(|t| orchestration::SummaryTask {
                    session: t.session,
                    title: t.task.title.clone(),
                    provider: t.task.kind(&run.provider).id().to_owned(),
                    model: t.task.model.clone().or_else(|| {
                        (t.task.kind(&run.provider).id() == run.provider)
                            .then(|| run.settings.model.clone())
                            .flatten()
                    }),
                    status: match t.state {
                        State::Pending | State::Starting => "queued",
                        State::Running => "running",
                        State::Done => "completed",
                        State::Failed => "failed",
                        State::Blocked => "blocked",
                    }
                    .into(),
                })
                .collect(),
        };
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
        let Some(run) = self.orchestrations.get_mut(&lead) else {
            return;
        };
        if let Some(view) = self.sessions.get(&lead) {
            view.update(cx, |v, cx| v.set_orchestration_progress(Some(progress), cx));
        }
        if done {
            let stopped = run.stopped;
            let reports = run
                .tasks
                .iter()
                .map(|t| {
                    format!(
                        "{} [{}]\n{}",
                        t.task.title,
                        if t.state == State::Done {
                            "completed"
                        } else {
                            "incomplete"
                        },
                        t.report
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let ready = self
                .sessions
                .get(&lead)
                .is_some_and(|v| v.read(cx).ready_for_external(cx));
            if stopped || ready {
                self.orchestrations.remove(&lead);
                if let Some(view) = self.sessions.get(&lead) {
                    view.update(cx, |v, cx| {
                        v.orchestration_plan = None;
                        v.set_orchestration_progress(None, cx);
                        if !stopped {
                            v.send_coordinator_report(format!("The orchestration workers have finished. Summarize and verify the combined results, identify incomplete work and report the outcome to the user. Preserve their changes.\n\n{reports}"), cx);
                        }
                    });
                }
            }
        }
        for job in jobs {
            let Launch {
                ix,
                task,
                kind,
                cwd,
                lead_provider,
                mut settings,
                reports,
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
                for file in &task.files {
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
                if task.harness.is_some() && provider::resolve(kind).is_none() {
                    return Err(orchestration::UNAVAILABLE.to_owned());
                }
                let models = if orchestration::needs_catalog(&task, &lead_provider) {
                    catalog::models_for(kind, &settings.account)?
                } else {
                    catalog::cached_models_for(kind, &settings.account).unwrap_or_default()
                };
                let mut settings =
                    orchestration::worker_settings(&task, &lead_provider, settings, kind, &models)?;
                let store = locked(&store)?;
                let mut summary = store.create(kind.id(), &project)?;
                summary.title = format!("Agent · {}", task.title);
                store.title(summary.id, &summary.title)?;
                settings.worktree_cwd = Some(cwd);
                store.save_agent_settings(summary.id, &settings)?;
                Ok::<_, String>((
                    summary,
                    settings,
                    orchestration::worker_prompt(&task, &reports),
                ))
            });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok((summary, settings, prompt)) => {
                            let id = summary.id;
                            this.add(summary, cx);
                            let start = this.orchestrations.get(&lead).is_some_and(|run| {
                                !run.stopped && run.tasks[ix].state == State::Starting
                            });
                            if start {
                                if let Some(run) = this.orchestrations.get_mut(&lead) {
                                    run.tasks[ix].session = Some(id);
                                    run.tasks[ix].state = State::Running;
                                }
                                this.sessions[&id].update(cx, |v, cx| {
                                    v.install_transfer(settings, Some(prompt), cx)
                                });
                            }
                        }
                        Err(error) => {
                            if let Some(run) = this.orchestrations.get_mut(&lead) {
                                run.tasks[ix].state = State::Failed;
                                run.tasks[ix].report = error;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
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
                let run = shell.orchestrations.get(&lead).unwrap();
                let starting = run
                    .tasks
                    .iter()
                    .filter(|t| t.state == State::Starting)
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
                let run = shell.orchestrations.get(&lead).unwrap();
                assert_eq!(
                    run.tasks
                        .iter()
                        .filter(|t| t.state == State::Starting)
                        .count(),
                    4
                );
                assert!(
                    run.tasks[1].state == State::Pending,
                    "overlapping src/main.rs waits for src"
                );
                shell.stop_orchestration(lead, cx);
                assert!(!shell.orchestrations.contains_key(&lead));
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
