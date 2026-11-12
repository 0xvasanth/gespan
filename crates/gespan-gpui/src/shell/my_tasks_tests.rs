//! My tasks inside a real shell: the sidebar entry, its count, and the page.
use super::*;
use crate::my_tasks::{Body, MyTasks, Source, TaskItem};
use gespan_core::tasks::{History, Kind};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

pub(super) fn setup(cx: &mut TestAppContext) -> WindowHandle<Shell> {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, session) = super::tests::window(cx);
    cx.simulate_window_resize(window.into(), gpui::size(px(1500.), px(900.)));
    window
        .update(cx, |shell, window, cx| {
            shell.workspace.sidebar_visible = true;
            shell.open(session, window, cx);
            shell.layout_changed(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
}

pub(super) fn click(visual: &mut VisualTestContext, selector: &str) {
    visual.refresh().unwrap();
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

/// A Flow approval task: the simplest kind to build by hand.
pub(super) fn flow_task(key: &str, project: &str, since_ms: u64) -> TaskItem {
    TaskItem {
        key: key.into(),
        kind: Kind::FlowApproval,
        project: project.into(),
        headline: format!("Approve {key}?"),
        who: "Release Flow".into(),
        source_label: format!("Release · {key}"),
        source: Source::FlowRun(key.into()),
        body: Body::Flow {
            prompt: format!("Publish {key}?"),
        },
        since_ms,
        history: History::default(),
    }
}

pub(super) fn set_tasks(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    tasks: Vec<TaskItem>,
) {
    window
        .update(visual, |shell, _, cx| {
            shell.show_my_tasks_for_test(tasks, cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn page(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> Entity<MyTasks> {
    window
        .read_with(visual, |shell, _| {
            shell.my_tasks.clone().expect("My tasks view")
        })
        .unwrap()
}

#[gpui::test]
fn my_tasks_is_the_first_sidebar_entry_and_shows_the_open_count(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    let entry = visual
        .debug_bounds("sidebar-navigation-My tasks")
        .expect("My tasks is in the sidebar");
    let inbox = visual
        .debug_bounds("sidebar-navigation-Inbox")
        .expect("Inbox is in the sidebar");
    assert!(entry.origin.y < inbox.origin.y, "My tasks sits above Inbox");
    assert!(
        visual
            .debug_bounds("sidebar-navigation-count-My tasks")
            .is_none()
    );

    set_tasks(
        window,
        visual,
        vec![flow_task("a", "gespan", 1), flow_task("b", "gespan", 2)],
    );
    assert!(
        visual
            .debug_bounds("sidebar-navigation-count-My tasks")
            .is_some()
    );
    window
        .read_with(visual, |shell, cx| assert_eq!(shell.my_task_count(cx), 2))
        .unwrap();
}

#[gpui::test]
fn clicking_my_tasks_opens_the_page(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    click(visual, "sidebar-navigation-My tasks");
    window
        .read_with(visual, |shell, _| {
            assert_eq!(shell.active_app_view, Some(app_views::AppView::MyTasks));
        })
        .unwrap();
    assert!(visual.debug_bounds("my-tasks").is_some());
}

#[gpui::test]
fn set_tasks_keeps_the_selection_and_moves_on_when_it_goes(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_tasks(
        window,
        visual,
        vec![
            flow_task("a", "gespan", 1),
            flow_task("b", "gespan", 2),
            flow_task("c", "gespan", 3),
        ],
    );
    let view = page(window, visual);
    view.update(visual, |view, cx| view.select("b", cx));
    // A newer refresh with the same tasks keeps B.
    set_tasks(
        window,
        visual,
        vec![
            flow_task("a", "gespan", 1),
            flow_task("b", "gespan", 2),
            flow_task("c", "gespan", 3),
        ],
    );
    view.read_with(visual, |view, _| {
        assert_eq!(view.selected().map(|t| t.key.as_str()), Some("b"));
    });
    // B is answered elsewhere: the one after it takes its place.
    set_tasks(
        window,
        visual,
        vec![flow_task("a", "gespan", 1), flow_task("c", "gespan", 3)],
    );
    view.read_with(visual, |view, _| {
        assert_eq!(view.selected().map(|t| t.key.as_str()), Some("c"));
    });
    // The last one goes: nothing is left selected.
    set_tasks(window, visual, Vec::new());
    view.read_with(visual, |view, _| assert!(view.selected().is_none()));
}

fn approval(id: &str) -> gespan_core::agent::Approval {
    gespan_core::agent::Approval {
        id: id.into(),
        title: "Allow Bash?".into(),
        detail: "git status".into(),
        can_remember: true,
        tool: None,
    }
}

fn question(id: &str, prompt: &str) -> gespan_core::agent::Question {
    gespan_core::agent::Question {
        id: id.into(),
        title: "A question".into(),
        auto_resolve_at: None,
        items: vec![gespan_core::agent::QuestionItem {
            id: "only".into(),
            prompt: prompt.into(),
            multiple: false,
            allow_custom: false,
            options: vec![gespan_core::agent::QuestionOption {
                id: "a".into(),
                label: "A".into(),
                description: String::new(),
            }],
        }],
    }
}

/// The session the window is not showing, and the shell's tasks after a refresh.
fn other_session(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> i64 {
    window
        .read_with(visual, |shell, _| {
            *shell
                .sessions
                .keys()
                .find(|id| Some(**id) != shell.workspace.active)
                .expect("a second session")
        })
        .unwrap()
}

fn tasks(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> Vec<(Kind, String)> {
    window
        .update(visual, |shell, _, cx| {
            shell.refresh_my_tasks(cx);
            shell
                .my_tasks
                .as_ref()
                .map(|view| {
                    view.read(cx)
                        .tasks()
                        .iter()
                        .map(|t| (t.kind, t.headline.clone()))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap()
}

#[gpui::test]
fn an_approval_from_a_hidden_session_is_a_task(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let hidden = other_session(window, visual);
    window
        .update(visual, |shell, _, cx| {
            // A Flow step or Pilot-launched session: never listed in the sidebar.
            shell.sidebar_hidden_sessions.insert(hidden);
            shell.sessions[&hidden].update(cx, |view, _| {
                view.set_approvals_for_test(vec![approval("a1")])
            });
        })
        .unwrap();
    let found = tasks(window, visual);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, Kind::Approval);
    window
        .read_with(visual, |shell, cx| {
            let view = shell.my_tasks.as_ref().unwrap().read(cx);
            let task = &view.tasks()[0];
            assert_eq!(task.source, Source::Session(hidden));
            assert_eq!(task.project, shell.sessions[&hidden].read(cx).summary.cwd);
        })
        .unwrap();
}

#[gpui::test]
fn a_question_is_a_task_until_it_is_answered_and_only_the_front_one_counts(
    cx: &mut TestAppContext,
) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.add_question(question("q1", "How should fsidCount be kept?"), cx);
                view.add_question(question("q2", "And the tag count?"), cx);
            });
        })
        .unwrap();
    let found = tasks(window, visual);
    assert_eq!(
        found,
        [(Kind::Question, "How should fsidCount be kept?".to_string())]
    );
    // Resolved elsewhere (the chat, the phone, the turn ending).
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, _| view.drop_questions_for_test());
        })
        .unwrap();
    assert!(tasks(window, visual).is_empty());
}

#[gpui::test]
fn a_task_keeps_its_first_seen_time_across_refreshes(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, _| {
                view.set_approvals_for_test(vec![approval("a1")])
            });
        })
        .unwrap();
    tasks(window, visual);
    let first = window
        .read_with(visual, |shell, cx| {
            shell.my_tasks.as_ref().unwrap().read(cx).tasks()[0].since_ms
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    tasks(window, visual);
    let again = window
        .read_with(visual, |shell, cx| {
            shell.my_tasks.as_ref().unwrap().read(cx).tasks()[0].since_ms
        })
        .unwrap();
    assert_eq!(first, again);
}

#[gpui::test]
fn a_waiting_flow_run_is_a_task_even_with_flows_hidden(cx: &mut TestAppContext) {
    use gespan_core::flow::{FlowRun, FlowVisit, RunStatus, VisitStatus, starter_flows};
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    window
        .read_with(visual, |shell, _| {
            let store = shell
                .flows_state
                .store
                .as_ref()
                .expect("the Flow store opens");
            let store = store.lock().unwrap();
            let flow = store
                .upsert(&gespan_core::flow::Flow {
                    id: "release".into(),
                    cwd: "/work/gespan".into(),
                    ..starter_flows().remove(0)
                })
                .unwrap();
            store
                .save_run(&FlowRun {
                    id: "run1".into(),
                    flow_id: flow.id.clone(),
                    flow,
                    status: RunStatus::WaitingApproval,
                    inputs: Default::default(),
                    worktree_cwd: None,
                    current_state: Some("publish".into()),
                    error: None,
                    created_at: 1,
                    finished_at: None,
                    tag: None,
                    edits: vec![],
                })
                .unwrap();
            store
                .save_visit(&FlowVisit {
                    id: "v1".into(),
                    run_id: "run1".into(),
                    seq: 1,
                    state_id: "publish".into(),
                    status: VisitStatus::WaitingApproval,
                    prompt: Some("Publish 0.1.3?".into()),
                    session_id: None,
                    outputs: None,
                    error: None,
                    retried: false,
                    note: None,
                    started_at: 1,
                    finished_at: None,
                })
                .unwrap();
        })
        .unwrap();
    let found = tasks(window, visual);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, Kind::FlowApproval);
    window
        .read_with(visual, |shell, cx| {
            let task = &shell.my_tasks.as_ref().unwrap().read(cx).tasks()[0];
            assert_eq!(task.source, Source::FlowRun("run1".into()));
            assert_eq!(task.project, "/work/gespan");
            assert!(matches!(&task.body, Body::Flow { prompt } if prompt == "Publish 0.1.3?"));
        })
        .unwrap();
}
