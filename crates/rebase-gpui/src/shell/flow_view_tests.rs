//! The Flows view inside a real shell: opening it, saving a starter, the
//! JSON editor, the run dialog, approvals, resume and delete.
use super::*;
use crate::flows::FlowsView;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use rebase_core::flow::{
    ApprovalState, Flow, FlowRun, FlowState, FlowVisit, Guard, GuardOp, OutputValue, RunStatus,
    Transition, VisitStatus, starter_flows, validate_flow,
};

fn setup(cx: &mut TestAppContext) -> WindowHandle<Shell> {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, session) = super::tests::window(cx);
    cx.simulate_window_resize(window.into(), gpui::size(px(1500.), px(900.)));
    window
        .update(cx, |shell, window, cx| {
            shell.workspace.sidebar_visible = true;
            shell.workspace.project_rail_open = true;
            shell.workspace.project_rail_width = 220.;
            shell.open(session, window, cx);
            shell.layout_changed(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    visual.refresh().unwrap();
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn view(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> Entity<FlowsView> {
    window
        .read_with(visual, |shell, _| shell.flows.clone().expect("Flows view"))
        .unwrap()
}

/// Runs `f` against the shell's Flow store.
fn with_store<R>(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&rebase_core::flow::FlowStore) -> R,
) -> R {
    window
        .read_with(visual, |shell, _| {
            let store = shell.flows_state.store.as_ref().expect("Flows opened");
            f(&store.lock().unwrap())
        })
        .unwrap()
}

fn fix_an_issue(id: &str, cwd: &str) -> Flow {
    Flow {
        id: id.into(),
        cwd: cwd.into(),
        ..starter_flows().remove(0)
    }
}

/// Saves `flow`, then opens the Flows view on it.
fn open_with(window: WindowHandle<Shell>, visual: &mut VisualTestContext, flow: &Flow) -> Flow {
    let saved = with_store(window, visual, |store| store.upsert(flow).unwrap());
    window
        .update(visual, |shell, window, cx| shell.open_flows(window, cx))
        .unwrap();
    visual.run_until_parked();
    saved
}

fn set_text(input: Entity<crate::input::Input>, text: &str, visual: &mut VisualTestContext) {
    let text = text.to_string();
    input.update(visual, |input, cx| input.set_text(text, cx));
    visual.run_until_parked();
}

#[gpui::test]
fn the_flows_view_opens_from_the_rail(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    click(visual, "sidebar-navigation-Flows");
    window
        .read_with(visual, |shell, _| {
            assert_eq!(shell.active_app_view, Some(app_views::AppView::Flows));
            assert!(shell.flows.is_some());
        })
        .unwrap();
    assert!(visual.debug_bounds("flows-view").is_some());
    assert!(visual.debug_bounds("new-flow").is_some());
}

#[gpui::test]
fn new_flow_from_fix_an_issue_saves_a_valid_flow_in_the_active_project(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    click(visual, "sidebar-navigation-Flows");
    click(visual, "new-flow");
    assert!(visual.debug_bounds("flow-new").is_some());
    click(visual, "flow-starter-0");
    let flows = with_store(window, visual, |store| store.list().unwrap());
    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.name, "Fix an issue");
    assert_eq!(
        flow.cwd, "/work/rebase",
        "the active project is the default"
    );
    assert!(!flow.id.is_empty());
    assert!(flow.created_at > 0 && flow.updated_at > 0);
    assert_eq!(validate_flow(flow).errors, Vec::<String>::new());
    assert_eq!(flow.states, starter_flows()[0].states);
    let view = view(window, visual);
    view.read_with(visual, |view, _| assert_eq!(view.page_name(), "editor"));
    let row: &'static str = format!("flow-row-{}", flow.id).leak();
    assert!(visual.debug_bounds(row).is_some());
}

#[gpui::test]
fn an_unknown_target_in_the_json_shows_the_error_and_disables_save(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let saved = open_with(window, visual, &fix_an_issue("f1", "/work/rebase"));
    click(visual, "flow-edit-f1");
    let view = view(window, visual);
    let input = view.read_with(visual, |view, _| view.editor_input().unwrap());
    let text = input.read_with(visual, |input, _| input.content.to_string());
    assert!(text.contains("\"to\": \"implement\""));

    set_text(
        input.clone(),
        &text.replacen("\"to\": \"implement\"", "\"to\": \"nowhere\"", 1),
        visual,
    );
    view.read_with(visual, |view, _| {
        let errors = view.editor_errors();
        assert!(
            errors.iter().any(|e| e.contains("unknown target")),
            "{errors:?}"
        );
        assert!(!view.editor_can_save());
    });
    assert!(visual.debug_bounds("flow-editor-error-0").is_some());
    click(visual, "flow-editor-save");
    let stored = with_store(window, visual, |store| store.list().unwrap());
    assert_eq!(stored, vec![saved.clone()], "Save did nothing");

    // Fixing the target and renaming enables Save, which writes the Flow.
    set_text(
        input,
        &text.replacen("\"name\": \"Fix an issue\"", "\"name\": \"Fix it\"", 1),
        visual,
    );
    view.read_with(visual, |view, _| {
        assert!(view.editor_errors().is_empty());
        assert!(view.editor_can_save());
    });
    click(visual, "flow-editor-save");
    let stored = with_store(window, visual, |store| store.list().unwrap());
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, "f1");
    assert_eq!(stored[0].name, "Fix it");
}

#[gpui::test]
fn a_fractional_max_visits_is_an_error_that_disables_save(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let saved = open_with(window, visual, &fix_an_issue("f1", "/work/rebase"));
    click(visual, "flow-edit-f1");
    let view = view(window, visual);
    let input = view.read_with(visual, |view, _| view.editor_input().unwrap());
    let text = input.read_with(visual, |input, _| input.content.to_string());
    set_text(
        input,
        &text.replacen("\"maxVisits\": 3", "\"maxVisits\": 2.5", 1),
        visual,
    );
    view.read_with(visual, |view, _| {
        let errors = view.editor_errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("floating point"), "{errors:?}");
        assert!(!view.editor_can_save());
    });
    click(visual, "flow-editor-save");
    assert_eq!(
        with_store(window, visual, |store| store.list().unwrap()),
        vec![saved]
    );
}

#[gpui::test]
fn the_run_dialog_keeps_run_disabled_until_the_required_input_is_filled(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    // The project folder does not exist, so the run's first visit fails
    // without creating a session.
    open_with(window, visual, &fix_an_issue("f1", "/missing/flow-project"));
    click(visual, "flow-run-f1");
    let view = view(window, visual);
    let fields = view.read_with(visual, |view, _| view.dialog_inputs());
    assert_eq!(fields.len(), 2, "one field per input");
    view.read_with(visual, |view, cx| {
        assert_eq!(
            view.dialog_missing(cx),
            Some(vec!["Issue is required".to_string()])
        );
    });
    click(visual, "flow-run-start");
    assert!(
        with_store(window, visual, |store| store.runs("f1", 10).unwrap()).is_empty(),
        "Run is disabled"
    );
    view.read_with(visual, |view, cx| {
        assert!(view.dialog_missing(cx).is_some())
    });

    set_text(fields[0].clone(), "Crash on save", visual);
    view.read_with(visual, |view, cx| {
        assert_eq!(view.dialog_missing(cx), Some(Vec::new()))
    });
    click(visual, "flow-run-start");
    let runs = with_store(window, visual, |store| store.runs("f1", 10).unwrap());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].inputs["issue"], "Crash on save");
    assert_eq!(runs[0].inputs["notes"], "");
    view.read_with(visual, |view, cx| {
        assert_eq!(view.page_name(), "run");
        assert_eq!(view.dialog_missing(cx), None, "the dialog closed");
        assert_eq!(view.open_run().unwrap().0.id, runs[0].id);
    });
}

fn approval_flow() -> Flow {
    Flow {
        id: "gate1".into(),
        name: "Gate".into(),
        description: String::new(),
        cwd: "/work/rebase".into(),
        inputs: Vec::new(),
        initial: "gate".into(),
        states: vec![FlowState::Approval(ApprovalState {
            id: "gate".into(),
            title: "Gate".into(),
            message: "Ship it?".into(),
            transitions: vec![
                Transition {
                    to: "$succeeded".into(),
                    when: Some(Guard::Output {
                        field: "approved".into(),
                        op: GuardOp::Truthy,
                        value: None,
                    }),
                },
                Transition {
                    to: "$failed".into(),
                    when: None,
                },
            ],
            max_visits: 3,
        })],
        created_at: 0,
        updated_at: 0,
    }
}

#[gpui::test]
fn reject_on_a_waiting_approval_resolves_it_with_the_note(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    open_with(window, visual, &approval_flow());
    click(visual, "flow-run-gate1");
    click(visual, "flow-run-start");
    let view = view(window, visual);
    let (run, visits) = view.read_with(visual, |view, _| view.open_run().unwrap());
    assert_eq!(run.status, RunStatus::WaitingApproval);
    assert_eq!(visits.len(), 1);
    assert_eq!(visits[0].status, VisitStatus::WaitingApproval);
    assert!(visual.debug_bounds("flow-reject").is_some());

    let note = view.read_with(visual, |view, _| view.run_note().unwrap());
    set_text(note, "redo", visual);
    click(visual, "flow-reject");

    let visits = with_store(window, visual, |store| store.visits(&run.id).unwrap());
    assert_eq!(visits[0].status, VisitStatus::Completed);
    assert_eq!(visits[0].note.as_deref(), Some("redo"));
    let outputs = visits[0].outputs.clone().unwrap();
    assert_eq!(outputs["approved"], OutputValue::Bool(false));
    assert_eq!(outputs["note"], OutputValue::String("redo".into()));
    let stored = with_store(window, visual, |store| store.run(&run.id).unwrap().unwrap());
    assert_eq!(
        stored.status,
        RunStatus::Failed,
        "a rejection takes $failed"
    );
    // The run view refreshed from the store.
    view.read_with(visual, |view, _| {
        assert_eq!(view.open_run().unwrap().0.status, RunStatus::Failed)
    });
    assert!(visual.debug_bounds("flow-reject").is_none());
}

fn visit(run: &str, seq: u32, state: &str, status: VisitStatus) -> FlowVisit {
    FlowVisit {
        id: format!("{run}-v{seq}"),
        run_id: run.into(),
        seq,
        state_id: state.into(),
        status,
        prompt: Some(format!("{state} prompt")),
        session_id: None,
        outputs: None,
        error: None,
        retried: false,
        note: None,
        started_at: 1_000,
        finished_at: Some(2_000),
    }
}

/// A failed Fix-an-issue run: triage completed, implement failed.
fn seed_failed_run(window: WindowHandle<Shell>, visual: &mut VisualTestContext, flow: &Flow) {
    let run = FlowRun {
        id: "run1".into(),
        flow_id: flow.id.clone(),
        flow: flow.clone(),
        status: RunStatus::Failed,
        inputs: HashMap::from([
            ("issue".to_string(), "42".to_string()),
            ("notes".to_string(), String::new()),
        ]),
        worktree_cwd: None,
        current_state: Some("implement".into()),
        error: Some("Output contract not met: missing summary".into()),
        created_at: 1,
        finished_at: Some(3_000),
    };
    let mut triage = visit("run1", 1, "triage", VisitStatus::Completed);
    triage.outputs = Some(HashMap::from([
        ("plan".to_string(), OutputValue::String("patch it".into())),
        ("files".to_string(), OutputValue::List(vec!["a.rs".into()])),
    ]));
    let mut implement = visit("run1", 2, "implement", VisitStatus::Failed);
    implement.error = Some("Output contract not met: missing summary\nfull reply".into());
    with_store(window, visual, |store| {
        store.save_run(&run).unwrap();
        store.save_visit(&triage).unwrap();
        store.save_visit(&implement).unwrap();
    });
}

#[gpui::test]
fn resume_on_a_failed_run_defaults_to_the_failed_state(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let flow = with_store(window, visual, |store| {
        store
            .upsert(&fix_an_issue("f1", "/missing/flow-project"))
            .unwrap()
    });
    seed_failed_run(window, visual, &flow);
    window
        .update(visual, |shell, window, cx| shell.open_flows(window, cx))
        .unwrap();
    visual.run_until_parked();
    click(visual, "flow-row-f1");
    let view = view(window, visual);
    view.read_with(visual, |view, _| {
        assert_eq!(view.page_name(), "run");
        assert_eq!(view.open_run().unwrap().1.len(), 2);
        assert_eq!(view.resume_choice().as_deref(), Some("implement"));
    });
    assert!(visual.debug_bounds("flow-resume-state-implement").is_some());
    assert!(
        visual.debug_bounds("flow-cancel").is_none(),
        "no cancel when finished"
    );

    click(visual, "flow-resume");
    let visits = with_store(window, visual, |store| store.visits("run1").unwrap());
    assert_eq!(visits.len(), 3, "resume appends a visit");
    assert_eq!(visits[2].state_id, "implement");
    // Its project folder is missing, so the resumed visit fails at launch.
    assert_eq!(
        visits[2].error.as_deref(),
        Some("The Flow's project folder is unavailable")
    );
}

#[gpui::test]
fn picking_another_state_resumes_from_it(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let flow = with_store(window, visual, |store| {
        store
            .upsert(&fix_an_issue("f1", "/missing/flow-project"))
            .unwrap()
    });
    seed_failed_run(window, visual, &flow);
    window
        .update(visual, |shell, window, cx| shell.open_flows(window, cx))
        .unwrap();
    visual.run_until_parked();
    click(visual, "flow-row-f1");
    click(visual, "flow-resume-state-triage");
    click(visual, "flow-resume");
    let visits = with_store(window, visual, |store| store.visits("run1").unwrap());
    assert_eq!(visits.len(), 3);
    assert_eq!(visits[2].state_id, "triage");
}

#[gpui::test]
fn delete_is_disabled_while_a_run_is_active(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let flow = with_store(window, visual, |store| {
        let busy = store.upsert(&fix_an_issue("busy", "/work/rebase")).unwrap();
        store.upsert(&fix_an_issue("idle", "/work/rebase")).unwrap();
        busy
    });
    with_store(window, visual, |store| {
        store
            .save_run(&FlowRun {
                id: "active".into(),
                flow_id: flow.id.clone(),
                flow: flow.clone(),
                status: RunStatus::WaitingApproval,
                inputs: HashMap::new(),
                worktree_cwd: None,
                current_state: Some("review".into()),
                error: None,
                created_at: 1,
                finished_at: None,
            })
            .unwrap()
    });
    window
        .update(visual, |shell, window, cx| shell.open_flows(window, cx))
        .unwrap();
    visual.run_until_parked();

    click(visual, "flow-delete-busy");
    assert!(
        !visual.has_pending_prompt(),
        "a disabled Delete asks nothing"
    );
    assert_eq!(with_store(window, visual, |s| s.list().unwrap()).len(), 2);

    click(visual, "flow-delete-idle");
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Delete");
    visual.run_until_parked();
    let left: Vec<_> = with_store(window, visual, |s| s.list().unwrap())
        .into_iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(left, vec!["busy".to_string()]);
}

#[gpui::test]
fn a_refused_delete_shows_the_stores_error(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let flow = open_with(window, visual, &fix_an_issue("f1", "/work/rebase"));
    // A run starts after the list loaded, so the button is still enabled.
    with_store(window, visual, |store| {
        store
            .save_run(&FlowRun {
                id: "late".into(),
                flow_id: flow.id.clone(),
                flow: flow.clone(),
                status: RunStatus::Running,
                inputs: HashMap::new(),
                worktree_cwd: None,
                current_state: Some("triage".into()),
                error: None,
                created_at: 1,
                finished_at: None,
            })
            .unwrap()
    });
    let view = view(window, visual);
    view.update(visual, |view, cx| view.delete_for_test("f1", cx));
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert_eq!(view.error(), Some("Flow has an active run"))
    });
    assert_eq!(with_store(window, visual, |s| s.list().unwrap()).len(), 1);
}

/// Opens `f1` in the editor and changes its name without saving.
fn dirty_editor(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
) -> (Entity<FlowsView>, Entity<crate::input::Input>) {
    open_with(window, visual, &fix_an_issue("f1", "/missing/flow-project"));
    click(visual, "flow-edit-f1");
    let view = view(window, visual);
    let input = view.read_with(visual, |view, _| view.editor_input().unwrap());
    let text = input.read_with(visual, |input, _| input.content.to_string());
    set_text(
        input.clone(),
        &text.replacen("\"name\": \"Fix an issue\"", "\"name\": \"Unsaved\"", 1),
        visual,
    );
    view.read_with(visual, |view, _| assert!(view.editor_can_save(), "dirty"));
    (view, input)
}

fn press_escape(
    window: WindowHandle<Shell>,
    input: &Entity<crate::input::Input>,
    visual: &mut VisualTestContext,
) {
    window
        .update(visual, |_, window, cx| {
            window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx), cx)
        })
        .unwrap();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
}

#[gpui::test]
fn the_editors_run_waits_until_its_edits_are_saved(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (view, _) = dirty_editor(window, visual);
    click(visual, "flow-editor-run");
    view.read_with(visual, |view, cx| {
        assert_eq!(view.dialog_missing(cx), None, "no run dialog while dirty");
        assert_eq!(view.page_name(), "editor");
    });
    assert!(
        with_store(window, visual, |store| store.runs("f1", 10).unwrap()).is_empty(),
        "nothing ran"
    );

    // The disabled Run's tooltip says why.
    view.read_with(visual, |view, _| {
        assert_eq!(view.editor_run_blocked(), Some("Save your changes first"))
    });

    // Once saved, Run opens the dialog for the saved Flow.
    click(visual, "flow-editor-save");
    view.read_with(visual, |view, _| {
        assert!(!view.editor_can_save(), "saved");
        assert_eq!(view.editor_run_blocked(), None);
    });
    click(visual, "flow-editor-run");
    view.read_with(visual, |view, cx| {
        assert!(view.dialog_missing(cx).is_some(), "the run dialog opened")
    });
}

#[gpui::test]
fn escape_on_a_dirty_editor_asks_before_discarding(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (view, input) = dirty_editor(window, visual);

    press_escape(window, &input, visual);
    assert!(visual.has_pending_prompt(), "Escape asks first");
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert_eq!(view.page_name(), "editor");
        assert!(view.editor_can_save(), "the edits are still there");
    });
    let text = input.read_with(visual, |input, _| input.content.to_string());
    assert!(text.contains("\"name\": \"Unsaved\""));

    press_escape(window, &input, visual);
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Discard");
    visual.run_until_parked();
    view.read_with(visual, |view, _| assert_eq!(view.page_name(), "empty"));
    let stored = with_store(window, visual, |store| store.list().unwrap());
    assert_eq!(stored[0].name, "Fix an issue", "nothing was saved");
}

#[gpui::test]
fn escape_on_a_clean_editor_closes_it_without_asking(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    open_with(window, visual, &fix_an_issue("f1", "/work/rebase"));
    click(visual, "flow-edit-f1");
    let view = view(window, visual);
    let input = view.read_with(visual, |view, _| view.editor_input().unwrap());
    press_escape(window, &input, visual);
    assert!(!visual.has_pending_prompt());
    view.read_with(visual, |view, _| assert_eq!(view.page_name(), "empty"));
}

#[gpui::test]
fn delete_is_refused_while_the_runner_has_a_run_the_store_does_not_show(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    open_with(window, visual, &approval_flow());
    click(visual, "flow-run-gate1");
    click(visual, "flow-run-start");
    let view = view(window, visual);
    let (run, _) = view.read_with(visual, |view, _| view.open_run().unwrap());
    assert_eq!(run.status, RunStatus::WaitingApproval);
    // The store has not caught up: its copy of the run reads as finished.
    with_store(window, visual, |store| {
        let mut stale = store.run(&run.id).unwrap().unwrap();
        stale.status = RunStatus::Failed;
        store.save_run(&stale).unwrap();
        assert!(store.active_runs().unwrap().is_empty());
    });

    view.update(visual, |view, cx| view.confirm_delete_for_test("gate1", cx));
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert_eq!(view.error(), Some("Flow has an active run"))
    });
    assert_eq!(with_store(window, visual, |s| s.list().unwrap()).len(), 1);

    // Once the runner lets the run go, the same delete goes through.
    click(visual, "flow-cancel");
    view.update(visual, |view, cx| view.confirm_delete_for_test("gate1", cx));
    visual.run_until_parked();
    assert!(with_store(window, visual, |s| s.list().unwrap()).is_empty());
}

#[gpui::test]
fn a_running_visit_waiting_on_its_session_shows_the_approval_wait(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let flow = with_store(window, visual, |store| {
        store.upsert(&fix_an_issue("f1", "/work/rebase")).unwrap()
    });
    let run = FlowRun {
        id: "run1".into(),
        flow_id: flow.id.clone(),
        flow: flow.clone(),
        status: RunStatus::Running,
        inputs: HashMap::from([("issue".to_string(), "42".to_string())]),
        worktree_cwd: None,
        current_state: Some("triage".into()),
        error: None,
        created_at: 1,
        finished_at: None,
    };
    let mut triage = visit("run1", 1, "triage", VisitStatus::Running);
    triage.session_id = Some("77".into());
    triage.finished_at = None;
    with_store(window, visual, |store| {
        store.save_run(&run).unwrap();
        store.save_visit(&triage).unwrap();
    });
    window
        .update(visual, |shell, window, cx| shell.open_flows(window, cx))
        .unwrap();
    visual.run_until_parked();
    click(visual, "flow-row-f1");
    let view = view(window, visual);
    view.read_with(visual, |view, _| {
        assert_eq!(view.row_status_of("f1"), "Running");
        assert_eq!(
            view.running_hint_of(1).as_deref(),
            Some("Running in Fix an issue · Triage — open the session if it needs your approval")
        );
    });
    assert!(visual.debug_bounds("flow-visit-hint-1").is_some());

    let waiting = HashMap::from([(
        77,
        crate::flows::Waiting {
            run_id: "run1".into(),
            session_title: "Fix an issue · Triage".into(),
        },
    )]);
    view.update(visual, |view, cx| view.set_waiting(waiting, cx));
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert_eq!(view.row_status_of("f1"), "Waiting for approval");
        assert_eq!(
            view.running_hint_of(1).as_deref(),
            Some("Waiting for your approval in Fix an issue · Triage")
        );
    });

    // The approval was answered: back to the plain hint.
    view.update(visual, |view, cx| view.set_waiting(HashMap::new(), cx));
    view.read_with(visual, |view, _| {
        assert_eq!(view.row_status_of("f1"), "Running");
    });
}

#[gpui::test]
fn resuming_a_run_that_lost_its_worktree_takes_it_from_the_visit_session(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let flow = with_store(window, visual, |store| {
        store
            .upsert(&fix_an_issue("f1", "/missing/flow-project"))
            .unwrap()
    });
    // The interrupted first visit's session moved into a worktree before
    // the restart, but the run never heard of it.
    let session = window
        .update(visual, |shell, _, _| {
            let store = shell.store.lock().unwrap();
            let summary = store.create("claude", "/missing/flow-project").unwrap();
            let mut settings = store
                .agent_settings(summary.id)
                .unwrap()
                .unwrap_or_default();
            settings.worktree_cwd = Some("/missing/flow-project/.worktrees/flow".into());
            store.save_agent_settings(summary.id, &settings).unwrap();
            summary.id
        })
        .unwrap();
    let run = FlowRun {
        id: "run1".into(),
        flow_id: flow.id.clone(),
        flow: flow.clone(),
        status: RunStatus::Failed,
        inputs: HashMap::from([
            ("issue".to_string(), "42".to_string()),
            ("notes".to_string(), String::new()),
        ]),
        worktree_cwd: None,
        current_state: Some("triage".into()),
        error: Some("Interrupted by app restart".into()),
        created_at: 1,
        finished_at: Some(3_000),
    };
    let mut triage = visit("run1", 1, "triage", VisitStatus::Failed);
    triage.session_id = Some(session.to_string());
    triage.error = Some("Interrupted by app restart".into());
    with_store(window, visual, |store| {
        store.save_run(&run).unwrap();
        store.save_visit(&triage).unwrap();
    });

    window
        .update(visual, |shell, _, cx| {
            shell.resume_flow_run("run1".into(), "triage".into(), cx)
        })
        .unwrap()
        .unwrap();
    visual.run_until_parked();
    let stored = with_store(window, visual, |store| store.run("run1").unwrap().unwrap());
    assert_eq!(
        stored.worktree_cwd.as_deref(),
        Some("/missing/flow-project/.worktrees/flow")
    );
    let visits = with_store(window, visual, |store| store.visits("run1").unwrap());
    assert_eq!(visits.len(), 2, "the resume appended a visit");
}
