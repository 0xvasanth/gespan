//! Pilot Flows: hidden Flows a Pilot owns (`Flow.owner`). A Pilot starts,
//! edits, cancels and resumes only its own runs, and hears each finished
//! step and run of them through `Pilots::flow_event`.
use super::flow_runs::{StoreHandle, store_lock};
use super::*;
use crate::flows::fill_worktree;
use gespan_core::flow::{Flow, FlowRun, FlowVisit, RunStatus, RunTag};

const NOT_READY: &str = "Flows are not ready";
const FLOW_NOT_FOUND: &str = "Flow not found";
const RUN_NOT_FOUND: &str = "Run not found";

/// What a Pilot hears about a run of a Flow it owns.
// Carried by value like the runner's effects; events are short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PilotFlowEvent {
    /// An agent visit ended; `run` is the run as it stands after the step.
    Step {
        run: FlowRun,
        visit: FlowVisit,
    },
    Finished {
        run_id: String,
        status: RunStatus,
    },
}

impl Shell {
    pub(crate) fn pilot_flows_store(&self) -> Result<StoreHandle, String> {
        self.flows_state
            .store
            .clone()
            .ok_or_else(|| NOT_READY.into())
    }

    /// Starts `pilot_id`'s Flow `flow_id`, tagged with the task it serves.
    pub(crate) fn start_pilot_flow(
        &mut self,
        pilot_id: &str,
        flow_id: &str,
        inputs: HashMap<String, String>,
        tag: RunTag,
        cx: &mut Context<Self>,
    ) -> Result<FlowRun, Vec<String>> {
        let store = self.pilot_flows_store().map_err(|e| vec![e])?;
        let flow = store_lock(&store)
            .and_then(|store| store.owned_flow(pilot_id, flow_id))
            .map_err(|e| vec![e])?
            .ok_or_else(|| vec![FLOW_NOT_FOUND.to_string()])?;
        self.with_flow_host(cx, |host, driver| {
            host.start(driver, &flow, &inputs, Some(tag))
        })
        .unwrap_or_else(|| Err(vec![NOT_READY.into()]))
    }

    /// Replaces the snapshot of `pilot_id`'s active run; states already
    /// entered cannot change. The edited run is saved.
    pub(crate) fn edit_pilot_run(
        &mut self,
        pilot_id: &str,
        run_id: &str,
        flow: Flow,
        note: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), Vec<String>> {
        if !self.owns_pilot_run(pilot_id, run_id).map_err(|e| vec![e])? {
            return Err(vec![RUN_NOT_FOUND.into()]);
        }
        self.with_flow_host(cx, |host, driver| host.edit_run(driver, run_id, flow, note))
            .unwrap_or_else(|| Err(vec![NOT_READY.into()]))
    }

    /// Deletes `pilot_id`'s Flow with its runs; refused while a run of it is
    /// active, as for user Flows.
    pub(crate) fn delete_pilot_flow(&self, pilot_id: &str, flow_id: &str) -> Result<(), String> {
        let store = self.pilot_flows_store()?;
        let store = store_lock(&store)?;
        if store.owned_flow(pilot_id, flow_id)?.is_none() {
            return Err(FLOW_NOT_FOUND.into());
        }
        self.refuse_active_flow(flow_id)?;
        store.delete(flow_id)
    }

    /// Cancels `pilot_id`'s run; a run that already ended is left as it is.
    pub(crate) fn cancel_pilot_run(
        &mut self,
        pilot_id: &str,
        run_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.owns_pilot_run(pilot_id, run_id)? {
            return Err(RUN_NOT_FOUND.into());
        }
        self.with_flow_host(cx, |host, driver| host.cancel(driver, run_id))
            .ok_or_else(|| NOT_READY.into())
    }

    /// Re-enters `state_id` of `pilot_id`'s failed or cancelled run.
    pub(crate) fn resume_pilot_run(
        &mut self,
        pilot_id: &str,
        run_id: &str,
        state_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let store = self.pilot_flows_store()?;
        let (mut run, visits) = store_lock(&store)?
            .owned_run(pilot_id, run_id)?
            .ok_or(RUN_NOT_FOUND)?;
        // As for user runs: a restart may have lost the run's worktree.
        if let Ok(sessions) = locked(&self.store) {
            fill_worktree(&mut run, &visits, |session| {
                sessions
                    .agent_settings(session)
                    .ok()
                    .flatten()
                    .and_then(|settings| settings.worktree_cwd)
            });
        }
        match self.with_flow_host(cx, |host, driver| {
            host.resume(driver, run, visits, state_id)
        }) {
            None => Err(NOT_READY.into()),
            Some(false) => Err(format!("This run cannot resume from {state_id}")),
            Some(true) => Ok(()),
        }
    }

    /// An active run is checked against the owner its writes named, since its
    /// first write may not have landed; any other run against the store.
    fn owns_pilot_run(&self, pilot_id: &str, run_id: &str) -> Result<bool, String> {
        if let Some(owner) = self.flows_state.pilot_runs.get(run_id) {
            return Ok(owner == pilot_id);
        }
        let store = self.pilot_flows_store()?;
        Ok(store_lock(&store)?.owned_run(pilot_id, run_id)?.is_some())
    }

    /// Holds every Flow launch from now on instead of opening a session.
    #[cfg(test)]
    pub(crate) fn hold_flow_launches_for_test(&mut self) {
        self.flows_state.held_launches.get_or_insert_default();
    }

    #[cfg(test)]
    pub(crate) fn held_flow_launches_for_test(&self) -> Vec<gespan_core::flow::LaunchRequest> {
        self.flows_state.held_launches.clone().unwrap_or_default()
    }

    /// Queues an event of a Pilot-owned run for its Pilot; user runs have no
    /// owner and are not reported. Delivery is deferred, so the host finishes
    /// applying its effects (the next `Launch` first) and a call made from
    /// inside the Pilots entity never re-enters it.
    pub(super) fn pilot_flow_event(&mut self, event: PilotFlowEvent, cx: &mut Context<Self>) {
        let owner = match &event {
            PilotFlowEvent::Step { run, .. } => run.flow.owner.clone(),
            PilotFlowEvent::Finished { run_id, .. } => self.flows_state.pilot_runs.remove(run_id),
        };
        let Some(pilot_id) = owner else {
            return;
        };
        self.flows_state.pilot_events.push((pilot_id, event));
        if self.flows_state.pilot_events_scheduled {
            return;
        }
        self.flows_state.pilot_events_scheduled = true;
        let shell = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |this, cx| {
                this.flows_state.pilot_events_scheduled = false;
                this.flush_pilot_flow_events(cx);
            });
        });
    }

    /// Hands queued events to the Pilots entity once it exists; it loads its
    /// roster as it is created. Until then (startup recovery can report runs
    /// before it) the events wait here.
    pub(super) fn flush_pilot_flow_events(&mut self, cx: &mut Context<Self>) {
        let Some(pilots) = self.pilots.clone() else {
            return;
        };
        let events = std::mem::take(&mut self.flows_state.pilot_events);
        if events.is_empty() {
            return;
        }
        pilots.update(cx, |pilots, cx| {
            for (pilot_id, event) in events {
                pilots.flow_event(&pilot_id, event, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::SessionDriver;
    use gespan_core::flow::{
        AgentState, FlowInput, FlowState, FlowStore, LaunchRequest, Runner, Transition,
        VisitStatus, starter_flows,
    };
    use gespan_core::store::Store;
    use gpui::{TestAppContext, WindowHandle};

    const MISSING: &str = "/missing/pilot-flow-project";

    /// A two-agent Flow `owner` owns, in a project folder that is missing,
    /// so each launch fails at once without creating a session.
    fn owned(id: &str, owner: Option<&str>) -> Flow {
        let starter = starter_flows()
            .into_iter()
            .find(|f| f.name == "Fix an issue")
            .unwrap();
        let template = starter
            .states
            .iter()
            .find_map(|s| match s {
                FlowState::Agent(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap();
        let agent = |id: &str, to: &str| {
            FlowState::Agent(AgentState {
                id: id.into(),
                title: id.to_uppercase(),
                brief: format!("{id} {{{{inputs.task}}}}"),
                outputs: vec![],
                transitions: vec![Transition {
                    to: to.into(),
                    when: None,
                }],
                ..template.clone()
            })
        };
        Flow {
            id: id.into(),
            name: format!("Pilot Flow {id}"),
            description: String::new(),
            cwd: MISSING.into(),
            inputs: vec![FlowInput {
                name: "task".into(),
                label: "Task".into(),
                required: true,
                default: None,
            }],
            initial: "plan".into(),
            states: vec![agent("plan", "build"), agent("build", "$succeeded")],
            access: starter.access.clone(),
            owner: owner.map(str::to_owned),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn task() -> HashMap<String, String> {
        HashMap::from([("task".to_string(), "ship it".to_string())])
    }

    fn tag() -> RunTag {
        RunTag {
            goal: "g1".into(),
            task: "t1".into(),
        }
    }

    fn shell(store: Store, cx: &mut TestAppContext) -> WindowHandle<Shell> {
        let store = Arc::new(Mutex::new(store));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, vec![], "/work/gespan".into(), None, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
    }

    fn save(shell: &Shell, flows: &[Flow]) {
        let store = shell.pilot_flows_store().unwrap();
        let store = store_lock(&store).unwrap();
        for flow in flows {
            store.upsert(flow).unwrap();
        }
    }

    fn events(shell: &Shell, cx: &App) -> Vec<(String, PilotFlowEvent)> {
        shell.pilot_manager().read(cx).flow_events.clone()
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
        fn visit_finished(&mut self, _: FlowRun, _: FlowVisit) {}
        fn run_finished(&mut self, _: &str, _: RunStatus) {}
    }

    /// Starts `flow` without launching it and saves its records, so the run
    /// stays active with nothing else writing it.
    fn start_quietly(shell: &mut Shell, flow: &Flow) -> FlowRun {
        let mut host = shell.flows_state.host.take().unwrap();
        let mut quiet = Quiet::default();
        let run = host.start(&mut quiet, flow, &task(), Some(tag())).unwrap();
        shell.flows_state.host = Some(host);
        let store = shell.pilot_flows_store().unwrap();
        let store = store_lock(&store).unwrap();
        for (run, visit) in quiet.0 {
            store.save_run(&run).unwrap();
            if let Some(visit) = visit {
                store.save_visit(&visit).unwrap();
            }
        }
        run
    }

    #[gpui::test]
    fn a_pilot_starts_only_its_own_flows(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                save(
                    shell,
                    &[
                        owned("mine", Some("p1")),
                        owned("theirs", Some("p2")),
                        owned("user", None),
                    ],
                );
                let not_found = Err(vec![FLOW_NOT_FOUND.to_string()]);
                for flow_id in ["theirs", "user", "nowhere"] {
                    assert_eq!(
                        shell
                            .start_pilot_flow("p1", flow_id, task(), tag(), cx)
                            .map(|_| ()),
                        not_found,
                        "{flow_id}"
                    );
                }
                let run = shell
                    .start_pilot_flow("p1", "mine", task(), tag(), cx)
                    .unwrap();
                assert_eq!(run.flow_id, "mine");
                assert_eq!(run.tag, Some(tag()));
                assert_eq!(run.flow.owner.as_deref(), Some("p1"));
                assert_eq!(
                    shell
                        .flows_state
                        .pilot_runs
                        .get(&run.id)
                        .map(String::as_str),
                    Some("p1")
                );
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui::test]
    fn starting_before_flows_open_says_they_are_not_ready(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                save(shell, &[owned("mine", Some("p1"))]);
                let host = shell.flows_state.host.take();
                assert_eq!(
                    shell
                        .start_pilot_flow("p1", "mine", task(), tag(), cx)
                        .map(|_| ()),
                    Err(vec![NOT_READY.to_string()])
                );
                shell.flows_state.host = host;
                let store = shell.flows_state.store.take();
                assert_eq!(shell.pilot_flows_store().err().as_deref(), Some(NOT_READY));
                assert_eq!(
                    shell.cancel_pilot_run("p1", "r1", cx),
                    Err(NOT_READY.to_string())
                );
                shell.flows_state.store = store;
            })
            .unwrap();
    }

    #[gpui::test]
    fn owned_step_events_reach_the_pilots_entity_and_user_flow_events_do_not(
        cx: &mut TestAppContext,
    ) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let (run, user_run) = window
            .update(cx, |shell, _, cx| {
                save(shell, &[owned("mine", Some("p1")), owned("user", None)]);
                let run = shell
                    .start_pilot_flow("p1", "mine", task(), tag(), cx)
                    .unwrap();
                let user = owned("user", None);
                let user_run = shell.start_flow(&user, &task(), cx).unwrap();
                (run, user_run)
            })
            .unwrap();
        // Both launches fail: the project folder is missing.
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let events = events(shell, cx);
                assert_eq!(events.len(), 2, "{events:?}");
                assert!(events.iter().all(|(pilot, _)| pilot == "p1"));
                assert_eq!(
                    events[0].1,
                    PilotFlowEvent::Finished {
                        run_id: run.id.clone(),
                        status: RunStatus::Failed,
                    }
                );
                let PilotFlowEvent::Step { run: after, visit } = &events[1].1 else {
                    panic!("a step: {events:?}");
                };
                assert_eq!(after.id, run.id);
                assert_eq!(after.status, RunStatus::Failed);
                assert_eq!(after.tag, Some(tag()));
                assert_eq!(visit.state_id, "plan");
                assert_eq!(visit.status, VisitStatus::Failed);
                assert!(
                    events.iter().all(|(_, e)| match e {
                        PilotFlowEvent::Step { run, .. } => run.id != user_run.id,
                        PilotFlowEvent::Finished { run_id, .. } => *run_id != user_run.id,
                    }),
                    "the user run is not reported"
                );
                // The user run did fail, unreported.
                let store = shell.pilot_flows_store().unwrap();
                let saved = store_lock(&store).unwrap().run(&user_run.id).unwrap();
                assert_eq!(saved.unwrap().status, RunStatus::Failed);
                assert!(shell.flows_state.pilot_runs.is_empty(), "the run ended");
            })
            .unwrap();
    }

    #[gpui::test]
    fn events_before_the_pilots_entity_is_ready_are_delivered_once_it_is(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let (pilots, run) = window
            .update(cx, |shell, _, cx| {
                save(shell, &[owned("mine", Some("p1"))]);
                // As before the Pilots entity and its roster exist.
                let pilots = shell.pilots.take().unwrap();
                let run = shell
                    .start_pilot_flow("p1", "mine", task(), tag(), cx)
                    .unwrap();
                (pilots, run)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(pilots.read(cx).flow_events.is_empty());
                assert_eq!(shell.flows_state.pilot_events.len(), 2, "held");
                shell.pilots = Some(pilots.clone());
                shell.flush_pilot_flow_events(cx);
                assert!(shell.flows_state.pilot_events.is_empty());
                let events = events(shell, cx);
                assert_eq!(events.len(), 2);
                assert!(matches!(
                    &events[0].1,
                    PilotFlowEvent::Finished { run_id, .. } if *run_id == run.id
                ));
                assert!(matches!(
                    &events[1].1,
                    PilotFlowEvent::Step { run: after, .. } if after.id == run.id
                ));
            })
            .unwrap();
    }

    #[gpui::test]
    fn startup_recovery_of_a_pilot_run_reaches_its_pilot(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!(
            "gespan-pilot-flow-recovery-{}",
            gespan_core::automations::new_id()
        ));
        let path = dir.join("gespan.db");
        let store = Store::open(&path).unwrap();
        // A Pilot run the previous app session left mid-visit.
        let run_id = {
            let flows = FlowStore::open(&path).unwrap();
            let flow = flows.upsert(&owned("mine", Some("p1"))).unwrap();
            let mut runner = Runner::new(
                || gespan_core::time::now_ms() as i64,
                gespan_core::automations::new_id,
            );
            let (run, effects) = runner.start(&flow, &task(), Some(tag())).unwrap();
            for effect in effects {
                if let gespan_core::flow::Effect::Persist { run, visit } = effect {
                    flows.save_run(&run).unwrap();
                    if let Some(visit) = visit {
                        flows.save_visit(&visit).unwrap();
                    }
                }
            }
            run.id
        };
        let window = shell(store, cx);
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let events = events(shell, cx);
                assert_eq!(
                    events.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
                    vec!["p1", "p1"]
                );
                assert_eq!(
                    events[0].1,
                    PilotFlowEvent::Finished {
                        run_id: run_id.clone(),
                        status: RunStatus::Failed,
                    }
                );
                let PilotFlowEvent::Step { visit, .. } = &events[1].1 else {
                    panic!("a step: {events:?}");
                };
                assert_eq!(visit.error.as_deref(), Some("Interrupted by app restart"));
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[gpui::test]
    fn a_pilot_run_edit_is_saved_and_only_its_pilot_may_edit_it(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let run = window
            .update(cx, |shell, _, cx| {
                save(shell, &[owned("mine", Some("p1"))]);
                let flow = shell
                    .pilot_flows_store()
                    .map(|s| store_lock(&s).unwrap().owned_flow("p1", "mine").unwrap())
                    .unwrap()
                    .unwrap();
                let run = start_quietly(shell, &flow);
                let mut edited = run.flow.clone();
                edited.name = "Edited by p1".into();
                if let FlowState::Agent(build) = &mut edited.states[1] {
                    build.brief = "build {{inputs.task}} with tests".into();
                }
                assert_eq!(
                    shell.edit_pilot_run("p2", &run.id, edited.clone(), "no", cx),
                    Err(vec![RUN_NOT_FOUND.to_string()])
                );
                assert_eq!(
                    shell.edit_pilot_run("p1", "nowhere", edited.clone(), "no", cx),
                    Err(vec![RUN_NOT_FOUND.to_string()])
                );
                shell
                    .edit_pilot_run("p1", &run.id, edited, "add tests", cx)
                    .unwrap();
                run
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                let store = shell.pilot_flows_store().unwrap();
                let saved = store_lock(&store).unwrap().run(&run.id).unwrap().unwrap();
                assert_eq!(saved.status, RunStatus::Running, "only the edit wrote it");
                assert_eq!(saved.flow.name, "Edited by p1");
                assert_eq!(saved.edits.len(), 1);
                assert_eq!(saved.edits[0].note, "add tests");
                assert_eq!(saved.edits[0].states, vec!["build".to_string()]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn only_its_pilot_cancels_or_resumes_a_run(cx: &mut TestAppContext) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let run = window
            .update(cx, |shell, _, cx| {
                save(shell, &[owned("mine", Some("p1"))]);
                let flow = shell
                    .pilot_flows_store()
                    .map(|s| store_lock(&s).unwrap().owned_flow("p1", "mine").unwrap())
                    .unwrap()
                    .unwrap();
                let run = start_quietly(shell, &flow);
                assert_eq!(
                    shell.cancel_pilot_run("p2", &run.id, cx),
                    Err(RUN_NOT_FOUND.to_string())
                );
                assert_eq!(shell.cancel_pilot_run("p1", &run.id, cx), Ok(()));
                run
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let store = shell.pilot_flows_store().unwrap();
                let saved = store_lock(&store).unwrap().run(&run.id).unwrap().unwrap();
                assert_eq!(saved.status, RunStatus::Cancelled);
                assert_eq!(
                    events(shell, cx),
                    vec![(
                        "p1".to_string(),
                        PilotFlowEvent::Finished {
                            run_id: run.id.clone(),
                            status: RunStatus::Cancelled,
                        }
                    )]
                );
                assert_eq!(
                    shell.resume_pilot_run("p2", &run.id, "build", cx),
                    Err(RUN_NOT_FOUND.to_string())
                );
                assert_eq!(
                    shell.resume_pilot_run("p1", &run.id, "nowhere", cx),
                    Err("This run cannot resume from nowhere".to_string())
                );
                assert_eq!(shell.resume_pilot_run("p1", &run.id, "build", cx), Ok(()));
                assert_eq!(
                    shell
                        .flows_state
                        .pilot_runs
                        .get(&run.id)
                        .map(String::as_str),
                    Some("p1"),
                    "running again"
                );
            })
            .unwrap();
        // The resumed visit's launch fails: the project folder is missing.
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let events = events(shell, cx);
                assert_eq!(events.len(), 3, "{events:?}");
                let PilotFlowEvent::Step { visit, .. } = &events[2].1 else {
                    panic!("a step: {events:?}");
                };
                assert_eq!(visit.state_id, "build");
            })
            .unwrap();
    }
}
