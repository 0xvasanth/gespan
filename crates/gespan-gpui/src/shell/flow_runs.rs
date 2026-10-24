//! Flow visits run as background sessions, launched like automation runs.
//! `crate::flows::Host` owns the routing; this is its session driver.
use super::automation_runs::{WorktreeChoice, launch_available, managed_settings};
use super::*;
use crate::flows::{Host, Probe, SessionDriver, Settled, Waiting, fill_worktree};
use gespan_core::flow::{Flow, FlowRun, FlowStore, FlowVisit, LaunchRequest, Runner};

pub(super) type StoreHandle = Arc<Mutex<FlowStore>>;

const STILL_OPENING: &str = "Flows are still opening";
/// `FlowStore::delete`'s refusal, for a run the store has not seen yet.
const FLOW_ACTIVE: &str = "Flow has an active run";

#[derive(Default)]
pub(super) struct State {
    pub store: Option<StoreHandle>,
    /// Taken while it applies effects, so the driver can borrow the shell.
    host: Option<Host>,
    opening: bool,
    writes: Option<gpui::Task<()>>,
    /// Flow sessions whose running turn is waiting for the user's approval.
    pub waiting: HashMap<i64, Waiting>,
}

impl State {
    /// A Flow turn still in flight; its history must stay loaded to settle.
    pub(super) fn awaits(&self, session: i64) -> bool {
        self.host.as_ref().is_some_and(|host| host.awaits(session))
    }
}

fn now() -> i64 {
    gespan_core::time::now_ms().min(i64::MAX as u64) as i64
}

fn store_lock(store: &StoreHandle) -> Result<std::sync::MutexGuard<'_, FlowStore>, String> {
    store.lock().map_err(|_| "Flows are unavailable".into())
}

struct Driver<'a, 'c> {
    shell: &'a mut Shell,
    cx: &'a mut Context<'c, Shell>,
}

impl SessionDriver for Driver<'_, '_> {
    fn launch(&mut self, request: LaunchRequest) {
        self.shell.launch_flow_visit(request, self.cx);
    }
    fn follow_up(&mut self, session: i64, prompt: String) -> Option<u64> {
        let view = self.shell.sessions.get(&session)?.clone();
        let (before, settings) = {
            let view = view.read(self.cx);
            (view.submission_generation, view.settings.clone())
        };
        view.update(self.cx, |view, cx| view.quick_submit(prompt, settings, cx));
        Some(before)
    }
    fn stop(&mut self, session: i64) {
        if let Some(view) = self.shell.sessions.get(&session) {
            view.update(self.cx, |view, cx| view.stop_if_busy(cx));
        }
    }
    fn persist(&mut self, run: FlowRun, visit: Option<FlowVisit>) {
        self.shell.persist_flow_run(run, visit, self.cx);
    }
    fn changed(&mut self, _run_id: &str) {
        self.shell.refresh_flows_view(self.cx);
        self.cx.notify();
    }
}

impl Shell {
    pub(super) fn initialize_flows(&mut self, cx: &mut Context<Self>) {
        if self.flows_state.opening || self.flows_state.store.is_some() {
            return;
        }
        self.flows_state.opening = true;
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let path = locked(&store)?.database_path().to_path_buf();
            let flows = FlowStore::open(&path)?;
            let active = flows
                .active_runs()?
                .into_iter()
                .map(|run| flows.visits(&run.id).map(|visits| (run, visits)))
                .collect::<Result<Vec<_>, String>>()?;
            Ok::<_, String>((Arc::new(Mutex::new(flows)), active))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.flows_state.opening = false;
                match result {
                    Ok((store, active)) => {
                        if let Some(view) = &this.flows {
                            view.update(cx, |view, cx| view.set_store(store.clone(), cx));
                        }
                        this.flows_state.store = Some(store);
                        this.flows_state.host = Some(Host::new(Runner::new(
                            now,
                            gespan_core::automations::new_id,
                        )));
                        this.with_flow_host(cx, |host, driver| host.recover(driver, active));
                    }
                    Err(error) => {
                        this.error = Some(format!("Could not open Flows: {error}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Reloads the Flows view after a run changed or a write landed.
    pub(super) fn refresh_flows_view(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.flows {
            view.update(cx, |view, cx| view.refresh(cx));
        }
    }

    /// Starts a saved Flow. Errors are the definition's or the inputs'.
    pub(crate) fn start_flow(
        &mut self,
        flow: &Flow,
        inputs: &HashMap<String, String>,
        cx: &mut Context<Self>,
    ) -> Result<FlowRun, Vec<String>> {
        self.with_flow_host(cx, |host, driver| host.start(driver, flow, inputs))
            .unwrap_or_else(|| Err(vec![STILL_OPENING.into()]))
    }

    pub(crate) fn resolve_flow_approval(
        &mut self,
        run_id: &str,
        approved: bool,
        note: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.with_flow_host(cx, |host, driver| {
            host.resolve_approval(driver, run_id, approved, note)
        })
        .ok_or_else(|| STILL_OPENING.into())
    }

    pub(crate) fn cancel_flow_run(
        &mut self,
        run_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.with_flow_host(cx, |host, driver| host.cancel(driver, run_id))
            .ok_or_else(|| STILL_OPENING.into())
    }

    /// Re-enters `state_id` of a failed or cancelled run, in its worktree.
    /// A refusal found after loading the run is reported to the Flows view.
    pub(crate) fn resume_flow_run(
        &mut self,
        run_id: String,
        state_id: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(store) = self
            .flows_state
            .store
            .clone()
            .filter(|_| self.flows_state.host.is_some())
        else {
            return Err(STILL_OPENING.into());
        };
        let sessions = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let (mut run, visits) = {
                let store = store_lock(&store)?;
                let run = store.run(&run_id)?.ok_or("The Flow run no longer exists")?;
                (run, store.visits(&run_id)?)
            };
            // A restart before the first turn settled lost the worktree the
            // run made; its session's saved settings still have it.
            let sessions = locked(&sessions)?;
            fill_worktree(&mut run, &visits, |session| {
                sessions
                    .agent_settings(session)
                    .ok()
                    .flatten()
                    .and_then(|settings| settings.worktree_cwd)
            });
            Ok::<_, String>((run, visits))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                let resumed = result.and_then(|(run, visits)| {
                    match this.with_flow_host(cx, |host, driver| {
                        host.resume(driver, run, visits, &state_id)
                    }) {
                        None => Err(STILL_OPENING.to_string()),
                        Some(false) => Err(format!("This run cannot resume from {state_id}")),
                        Some(true) => Ok(()),
                    }
                });
                if let Err(error) = resumed {
                    this.flow_error(format!("Could not resume Flow: {error}"), cx);
                }
            });
        })
        .detach();
        Ok(())
    }

    /// Shows a Flow error in the Flows view, or the shell's banner without it.
    fn flow_error(&mut self, error: String, cx: &mut Context<Self>) {
        match &self.flows {
            Some(view) => view.update(cx, |view, cx| view.set_error(error, cx)),
            None => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    /// Deletes a Flow unless a run of it is in flight. The runner knows of a
    /// run before its records are written, so it is asked as well as the store.
    pub(crate) fn delete_flow(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), String> {
        if self
            .flows_state
            .host
            .as_ref()
            .is_some_and(|host| host.has_active(id))
        {
            return Err(FLOW_ACTIVE.into());
        }
        if let Some(view) = &self.flows {
            view.update(cx, |view, cx| view.delete(id.to_owned(), cx));
        }
        Ok(())
    }

    /// The user deleted session `id`; a Flow visit running in it fails.
    pub(super) fn flow_session_closed(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.flows_state.awaits(id) {
            self.with_flow_host(cx, |host, driver| host.session_closed(driver, id));
        }
        self.sync_flow_waiting(id, cx);
    }

    /// The `SessionChanged` hook: records a Flow visit's worktree as soon as
    /// its session has one, settles its finished turn, and tracks whether it
    /// waits for the user's approval.
    pub(super) fn flow_session_changed(&mut self, id: i64, cx: &mut Context<Self>) {
        self.settle_flow_session(id, cx);
        self.sync_flow_waiting(id, cx);
    }

    fn settle_flow_session(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.flows_state.awaits(id) {
            return;
        }
        let Some(view) = self.sessions.get(&id).cloned() else {
            return;
        };
        // Before the turn ends: a cancel or restart must not lose it.
        if let Some(path) = view.read(cx).settings.worktree_cwd.clone() {
            self.with_flow_host(cx, |host, driver| host.worktree(driver, id, &path));
        }
        let view = view.read(cx);
        let probe = Probe {
            pending: view.external_submission_pending(),
            generation: view.submission_generation,
        };
        if !self
            .flows_state
            .host
            .as_ref()
            .is_some_and(|host| host.ready(id, probe))
        {
            return;
        }
        let settled = Settled::read(
            view.transcript.read(cx).source_rows(),
            view.external_outcome(cx),
            view.notification_was_cancelled(),
            view.settings.worktree_cwd.clone(),
        );
        self.with_flow_host(cx, |host, driver| host.settle(driver, id, probe, settled));
    }

    /// Shows or clears session `id`'s approval wait in the Flows view.
    fn sync_flow_waiting(&mut self, id: i64, cx: &mut Context<Self>) {
        let waiting = self
            .flows_state
            .host
            .as_ref()
            .and_then(|host| host.run_of(id))
            .zip(self.sessions.get(&id))
            .and_then(|(run_id, view)| {
                let view = view.read(cx);
                view.needs_approval().then(|| Waiting {
                    run_id: run_id.to_owned(),
                    session_title: view.summary.title.clone(),
                })
            });
        let changed = match waiting {
            Some(waiting) => self.flows_state.waiting.insert(id, waiting.clone()) != Some(waiting),
            None => self.flows_state.waiting.remove(&id).is_some(),
        };
        if changed && let Some(view) = &self.flows {
            let waiting = self.flows_state.waiting.clone();
            view.update(cx, |view, cx| view.set_waiting(waiting, cx));
        }
    }

    /// Runs `f` with the host and a driver over this shell, then probes the
    /// sessions it re-armed. `None` until the Flow store has opened.
    fn with_flow_host<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Host, &mut dyn SessionDriver) -> R,
    ) -> Option<R> {
        let mut host = self.flows_state.host.take()?;
        let result = f(&mut host, &mut Driver { shell: self, cx });
        let recheck = host.take_rechecks();
        self.flows_state.host = Some(host);
        for id in recheck {
            self.flow_session_changed(id, cx);
        }
        Some(result)
    }

    fn launch_flow_visit(&mut self, request: LaunchRequest, cx: &mut Context<Self>) {
        let kind = Kind::from_id(&request.harness);
        let mut settings =
            managed_settings(Default::default(), kind, &request.cwd, &request.model, cx);
        // The run snapshot's mode, so a resume launches as the run started.
        settings.access = gespan_core::agent::AccessMode::from_id(&request.access);
        let store = self.store.clone();
        let launch = request.clone();
        let work = cx.background_executor().spawn(async move {
            launch_available(
                "Flow",
                &launch.harness,
                &launch.cwd,
                launch.worktree_cwd.as_deref(),
            )?;
            // Later visits reuse the first visit's worktree; the first makes
            // one when the project is a Git repository.
            let worktree = match launch.worktree_cwd.clone() {
                Some(path) => WorktreeChoice::Reuse(path),
                None if gespan_core::git::worktrees(std::path::Path::new(&launch.cwd)).is_ok() => {
                    WorktreeChoice::New
                }
                None => WorktreeChoice::Current,
            };
            worktree.apply(&mut settings);
            let store = locked(&store)?;
            let mut summary = store.create(&launch.harness, &launch.cwd)?;
            summary.title = launch.title.clone();
            store.title(summary.id, &summary.title)?;
            store.save_agent_settings(summary.id, &settings)?;
            Ok::<_, String>((summary, settings))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((summary, settings)) => {
                    let id = summary.id;
                    this.add(summary, cx);
                    let before = this.sessions[&id].read(cx).submission_generation;
                    let submit = this
                        .with_flow_host(cx, |host, driver| {
                            host.launched(driver, &request.run_id, &request.visit_id, id, before)
                        })
                        .unwrap_or(false);
                    if submit {
                        this.submit_in_background(id, request.prompt, settings, cx);
                        if let Some(host) = &mut this.flows_state.host {
                            host.submitted(id);
                        }
                        this.flow_session_changed(id, cx);
                    }
                    this.persist(cx);
                }
                Err(error) => {
                    this.with_flow_host(cx, |host, driver| {
                        host.launch_failed(driver, &request.run_id, &request.visit_id, error)
                    });
                }
            });
        })
        .detach();
    }

    /// Writes run in order, one after another, like automation results.
    fn persist_flow_run(&mut self, run: FlowRun, visit: Option<FlowVisit>, cx: &mut Context<Self>) {
        let Some(store) = self.flows_state.store.clone() else {
            return;
        };
        let previous = self.flows_state.writes.take();
        self.flows_state.writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                let store = store_lock(&store)?;
                store.save_run(&run)?;
                if let Some(visit) = visit {
                    store.save_visit(&visit)?;
                }
                Ok::<_, String>(())
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.refresh_flows_view(cx),
                Err(error) => {
                    this.error = Some(format!("Could not save Flow run: {error}"));
                    cx.notify();
                }
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::flow::{Effect, RunStatus, VisitStatus, starter_flows};
    use gespan_core::store::Store;
    use gpui::{TestAppContext, WindowHandle};

    fn fix_an_issue(cwd: &str) -> Flow {
        let mut flow = starter_flows()
            .into_iter()
            .find(|f| f.name == "Fix an issue")
            .unwrap();
        flow.id = "flow1".into();
        flow.cwd = cwd.into();
        flow
    }

    fn inputs(flow: &Flow) -> HashMap<String, String> {
        flow.inputs
            .iter()
            .map(|input| (input.name.clone(), "42".to_string()))
            .collect()
    }

    /// Keeps a host's writes and drops its launches.
    #[derive(Default)]
    struct Quiet(Vec<(FlowRun, Option<FlowVisit>)>);
    impl SessionDriver for Quiet {
        fn launch(&mut self, _: LaunchRequest) {}
        fn follow_up(&mut self, _: i64, _: String) -> Option<u64> {
            None
        }
        fn stop(&mut self, _: i64) {}
        fn persist(&mut self, run: FlowRun, visit: Option<FlowVisit>) {
            self.0.push((run, visit));
        }
        fn changed(&mut self, _: &str) {}
    }

    fn shell(store: Store, cwd: &str, cx: &mut TestAppContext) -> WindowHandle<Shell> {
        let store = Arc::new(Mutex::new(store));
        let cwd = cwd.to_string();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, vec![], cwd, None, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
    }

    #[gpui::test]
    fn startup_recovery_marks_a_persisted_running_visit_interrupted(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!(
            "gespan-flow-recovery-{}",
            gespan_core::automations::new_id()
        ));
        let path = dir.join("gespan.db");
        let store = Store::open(&path).unwrap();
        // A run the previous app session left mid-visit.
        let (run_id, visit_id) = {
            let flows = FlowStore::open(&path).unwrap();
            let flow = flows.upsert(&fix_an_issue("/repo")).unwrap();
            let mut runner = Runner::new(now, gespan_core::automations::new_id);
            let (run, effects) = runner.start(&flow, &inputs(&flow)).unwrap();
            let mut visit_id = None;
            for effect in effects {
                if let Effect::Persist { run, visit } = effect {
                    flows.save_run(&run).unwrap();
                    if let Some(visit) = visit {
                        visit_id = Some(visit.id.clone());
                        flows.save_visit(&visit).unwrap();
                    }
                }
            }
            (run.id, visit_id.expect("a running visit"))
        };

        let window = shell(store, "/repo", cx);
        cx.run_until_parked();

        let flows = FlowStore::open(&path).unwrap();
        let run = flows.run(&run_id).unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some("Interrupted by app restart"));
        let visits = flows.visits(&run_id).unwrap();
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].id, visit_id);
        assert_eq!(visits[0].status, VisitStatus::Failed);
        assert_eq!(
            visits[0].error.as_deref(),
            Some("Interrupted by app restart")
        );
        assert!(flows.active_runs().unwrap().is_empty());
        window
            .update(cx, |shell, _, _| {
                assert!(shell.flows_state.host.is_some());
                assert_eq!(shell.error, None);
            })
            .unwrap();
        drop(flows);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[gpui::test]
    fn a_worktree_the_session_reports_mid_turn_is_kept_when_the_run_is_cancelled(
        cx: &mut TestAppContext,
    ) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, "/work/gespan", cx);
        let run_id = window
            .update(cx, |shell, _, cx| {
                // Known ids: the run is "id1", its first visit "id2".
                let mut n = 0;
                shell.flows_state.host = Some(Host::new(Runner::new(now, move || {
                    n += 1;
                    format!("id{n}")
                })));
                let flow = store_lock(shell.flows_state.store.as_ref().unwrap())
                    .unwrap()
                    .upsert(&fix_an_issue("/missing/flow-project"))
                    .unwrap();
                let run = shell.start_flow(&flow, &inputs(&flow), cx).unwrap();
                assert_eq!(run.id, "id1");
                // The visit's session, as `launch_flow_visit` reports it.
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("claude", "/missing/flow-project")
                    .unwrap();
                let id = summary.id;
                shell.add(summary, cx);
                let before = shell.sessions[&id].read(cx).submission_generation;
                assert!(
                    shell
                        .with_flow_host(cx, |host, driver| {
                            host.launched(driver, "id1", "id2", id, before)
                        })
                        .unwrap()
                );
                // Mid-turn, the conversation moves into its new worktree.
                shell.sessions[&id].update(cx, |view, _| {
                    view.settings.worktree_cwd = Some("/missing/flow-project/.wt".into())
                });
                shell.flow_session_changed(id, cx);
                assert!(shell.flows_state.awaits(id), "the turn has not settled");
                shell.cancel_flow_run(&run.id, cx).unwrap();
                run.id
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                let store = store_lock(shell.flows_state.store.as_ref().unwrap()).unwrap();
                let saved = store.run(&run_id).unwrap().unwrap();
                assert_eq!(saved.status, RunStatus::Cancelled);
                assert_eq!(
                    saved.worktree_cwd.as_deref(),
                    Some("/missing/flow-project/.wt"),
                    "resume will reuse it"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_flow_session_waiting_for_approval_shows_in_the_flows_view(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, "/work/gespan", cx);
        let (flow_id, id) = window
            .update(cx, |shell, window, cx| {
                let mut n = 0;
                shell.flows_state.host = Some(Host::new(Runner::new(now, move || {
                    n += 1;
                    format!("id{n}")
                })));
                let flows = shell.flows_state.store.clone().unwrap();
                let flow = store_lock(&flows)
                    .unwrap()
                    .upsert(&fix_an_issue("/missing/flow-project"))
                    .unwrap();
                // Start without launching, so no background launch fails it.
                let mut host = shell.flows_state.host.take().unwrap();
                let mut quiet = Quiet::default();
                host.start(&mut quiet, &flow, &inputs(&flow)).unwrap();
                shell.flows_state.host = Some(host);
                for (run, visit) in quiet.0 {
                    let flows = store_lock(&flows).unwrap();
                    flows.save_run(&run).unwrap();
                    if let Some(visit) = visit {
                        flows.save_visit(&visit).unwrap();
                    }
                }
                let mut summary = locked(&shell.store)
                    .unwrap()
                    .create("claude", "/missing/flow-project")
                    .unwrap();
                summary.title = "Fix an issue · Triage".into();
                let id = summary.id;
                shell.add(summary, cx);
                let before = shell.sessions[&id].read(cx).submission_generation;
                shell.with_flow_host(cx, |host, driver| {
                    host.launched(driver, "id1", "id2", id, before)
                });
                shell.open_flows(window, cx);
                shell.sessions[&id].update(cx, |view, _| {
                    view.set_approvals_for_test(vec![gespan_core::agent::Approval {
                        id: "a1".into(),
                        title: "Run a command?".into(),
                        detail: "git status".into(),
                        can_remember: false,
                    }])
                });
                shell.flow_session_changed(id, cx);
                assert_eq!(
                    shell.flows_state.waiting.get(&id),
                    Some(&Waiting {
                        run_id: "id1".into(),
                        session_title: "Fix an issue · Triage".into(),
                    })
                );
                (flow.id, id)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let view = shell.flows.clone().unwrap();
                assert_eq!(
                    view.read(cx).row_status_of(&flow_id),
                    "Waiting for approval"
                );
                // Answered: the wait clears from the shell and the view.
                shell.sessions[&id].update(cx, |view, _| view.set_approvals_for_test(vec![]));
                shell.flow_session_changed(id, cx);
                assert!(shell.flows_state.waiting.is_empty());
                assert_eq!(view.read(cx).row_status_of(&flow_id), "Running");

                // A session that is not running a visit never waits here.
                shell.sessions[&id].update(cx, |view, _| {
                    view.set_approvals_for_test(vec![gespan_core::agent::Approval {
                        id: "a2".into(),
                        title: "Run a command?".into(),
                        detail: "ls".into(),
                        can_remember: false,
                    }])
                });
                shell.flow_session_closed(id, cx);
                assert!(shell.flows_state.waiting.is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_visit_session_is_created_with_the_run_snapshots_access_mode(cx: &mut TestAppContext) {
        let project = std::env::temp_dir().join(format!(
            "gespan-flow-access-{}",
            gespan_core::automations::new_id()
        ));
        std::fs::create_dir_all(&project).unwrap();
        let cwd = project.to_string_lossy().to_string();
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, "/work/gespan", cx);
        let sessions = window
            .update(cx, |shell, _, cx| {
                let mut flow = fix_an_issue(&cwd);
                flow.access = "full-access".into();
                let flow = store_lock(shell.flows_state.store.as_ref().unwrap())
                    .unwrap()
                    .upsert(&flow)
                    .unwrap();
                let run = shell.start_flow(&flow, &inputs(&flow), cx).unwrap();
                // Cancelled while its session is created: it gets no prompt.
                shell.cancel_flow_run(&run.id, cx).unwrap();
                shell.sessions.keys().copied().collect::<Vec<_>>()
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                let created: Vec<i64> = shell
                    .sessions
                    .keys()
                    .copied()
                    .filter(|id| !sessions.contains(id))
                    .collect();
                assert_eq!(created.len(), 1, "the visit's session");
                let settings = locked(&shell.store)
                    .unwrap()
                    .agent_settings(created[0])
                    .unwrap()
                    .unwrap();
                assert_eq!(settings.access, gespan_core::agent::AccessMode::FullAccess);
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(project);
    }

    #[gpui::test]
    fn run_actions_report_that_flows_are_still_opening(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, "/work/gespan", cx);
        window
            .update(cx, |shell, _, cx| {
                let host = shell.flows_state.host.take();
                assert!(host.is_some());
                let opening = Err(STILL_OPENING.to_string());
                assert_eq!(
                    shell.resolve_flow_approval("r1", false, "redo", cx),
                    opening
                );
                assert_eq!(shell.cancel_flow_run("r1", cx), opening);
                assert_eq!(shell.resume_flow_run("r1".into(), "s".into(), cx), opening);
                let flow = fix_an_issue("/repo");
                assert_eq!(
                    shell.start_flow(&flow, &inputs(&flow), cx).map(|_| ()),
                    Err(vec![STILL_OPENING.to_string()])
                );
                shell.flows_state.host = host;
            })
            .unwrap();
    }

    #[gpui::test]
    fn resuming_an_unknown_state_says_so(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, "/work/gespan", cx);
        window
            .update(cx, |shell, _, cx| {
                let flows = store_lock(shell.flows_state.store.as_ref().unwrap()).unwrap();
                let flow = flows.upsert(&fix_an_issue("/missing")).unwrap();
                let mut runner = Runner::new(now, gespan_core::automations::new_id);
                let (mut run, _) = runner.start(&flow, &inputs(&flow)).unwrap();
                run.status = RunStatus::Failed;
                flows.save_run(&run).unwrap();
                drop(flows);
                assert_eq!(
                    shell.resume_flow_run(run.id.clone(), "nowhere".into(), cx),
                    Ok(())
                );
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.flows.is_none());
                assert_eq!(
                    shell.error.as_deref(),
                    Some("Could not resume Flow: This run cannot resume from nowhere")
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_visit_whose_project_folder_is_missing_fails_without_a_session(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, "/work/gespan", cx);
        let flow = fix_an_issue("/missing/flow-project");
        let (run, sessions) = window
            .update(cx, |shell, _, cx| {
                let saved = store_lock(shell.flows_state.store.as_ref().unwrap())
                    .unwrap()
                    .upsert(&flow)
                    .unwrap();
                let run = shell.start_flow(&saved, &inputs(&saved), cx).unwrap();
                (run, shell.sessions.len())
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.sessions.len(), sessions, "no session was created");
                let store = store_lock(shell.flows_state.store.as_ref().unwrap()).unwrap();
                let saved = store.run(&run.id).unwrap().unwrap();
                assert_eq!(saved.status, RunStatus::Failed);
                let visits = store.visits(&run.id).unwrap();
                assert_eq!(visits[0].status, VisitStatus::Failed);
                assert_eq!(visits[0].session_id, None);
                assert_eq!(
                    visits[0].error.as_deref(),
                    Some("The Flow's project folder is unavailable")
                );
            })
            .unwrap();
    }
}
