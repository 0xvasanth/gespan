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
        agent: None,
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

/// Opens My tasks on the other session's requests, with a control channel the
/// test reads.
fn waiting_page(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    approvals: Vec<gespan_core::agent::Approval>,
) -> (i64, std::sync::mpsc::Receiver<gespan_core::agent::Control>) {
    let id = other_session(window, visual);
    let (send, receive) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, window, cx| {
            shell.sessions[&id].update(cx, |view, _| {
                view.controls = Some(send);
                view.set_approvals_for_test(approvals);
            });
            shell.open_my_tasks(window, cx);
            shell.refresh_my_tasks(cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    (id, receive)
}

fn selected_key(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> Option<String> {
    window
        .read_with(visual, |shell, cx| {
            shell
                .my_tasks
                .as_ref()?
                .read(cx)
                .selected()
                .map(|t| t.key.clone())
        })
        .unwrap()
}

#[gpui::test]
fn allow_once_decides_that_exact_request_and_moves_to_the_next(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, controls) = waiting_page(window, visual, vec![approval("a1"), approval("a2")]);
    assert_eq!(
        selected_key(window, visual),
        Some(format!("approval:{id}:a1"))
    );
    assert!(visual.debug_bounds("my-tasks-card").is_some());
    click(visual, "my-tasks-allow");
    let sent = controls.try_recv().expect("a decision was sent");
    assert_eq!(sent.id, "a1");
    assert!(matches!(sent.decision, gespan_core::agent::Decision::Allow));
    window
        .update(visual, |shell, _, cx| shell.refresh_my_tasks(cx))
        .unwrap();
    assert_eq!(
        selected_key(window, visual),
        Some(format!("approval:{id}:a2"))
    );
    click(visual, "my-tasks-deny");
    let sent = controls.try_recv().expect("a decision was sent");
    assert_eq!(sent.id, "a2");
    assert!(matches!(sent.decision, gespan_core::agent::Decision::Deny));
}

#[gpui::test]
fn allow_for_session_is_offered_only_when_it_can_be_remembered(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let once = gespan_core::agent::Approval {
        can_remember: false,
        ..approval("a1")
    };
    let (_, controls) = waiting_page(window, visual, vec![once, approval("a2")]);
    assert!(visual.debug_bounds("my-tasks-allow-session").is_none());
    click(visual, "my-tasks-next");
    click(visual, "my-tasks-allow-session");
    let sent = controls.try_recv().expect("a decision was sent");
    assert_eq!(sent.id, "a2");
    assert!(matches!(
        sent.decision,
        gespan_core::agent::Decision::Session
    ));
}

#[gpui::test]
fn next_and_previous_wrap_and_show_the_position(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, _controls) = waiting_page(
        window,
        visual,
        vec![approval("a1"), approval("a2"), approval("a3")],
    );
    let position = |window: WindowHandle<Shell>, visual: &mut VisualTestContext| {
        window
            .read_with(visual, |shell, cx| {
                shell.my_tasks.as_ref().unwrap().read(cx).position(cx)
            })
            .unwrap()
    };
    assert_eq!(position(window, visual), "1 of 3");
    assert!(visual.debug_bounds("my-tasks-position").is_some());
    click(visual, "my-tasks-next");
    assert_eq!(position(window, visual), "2 of 3");
    click(visual, "my-tasks-prev");
    click(visual, "my-tasks-prev");
    assert_eq!(position(window, visual), "3 of 3");
    assert_eq!(
        selected_key(window, visual),
        Some(format!("approval:{id}:a3"))
    );
}

#[gpui::test]
fn a_question_is_answered_from_the_page_and_reaches_its_session(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    let (send, controls) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, window, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.controls = Some(send);
                view.add_question(question("q1", "How should fsidCount be kept?"), cx);
            });
            shell.open_my_tasks(window, cx);
            shell.refresh_my_tasks(cx);
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "question-option-0-0");
    if visual.debug_bounds("question-continue").is_some() {
        click(visual, "question-continue");
    }
    let sent = controls.try_recv().expect("the answer reached the session");
    assert_eq!(sent.id, "q1");
    assert!(matches!(
        sent.decision,
        gespan_core::agent::Decision::Answer(_)
    ));
}

#[gpui::test]
fn history_shows_four_turns_and_loads_four_more(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                for n in 1..=8 {
                    view.push_history_for_test("user", &format!("ask {n}"), cx);
                    view.push_history_for_test("assistant", &format!("reply {n}"), cx);
                }
            });
        })
        .unwrap();
    let (_, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    assert!(visual.debug_bounds("my-tasks-history").is_some());
    assert!(visual.debug_bounds("my-tasks-turn-3").is_some());
    assert!(visual.debug_bounds("my-tasks-turn-4").is_none());
    assert!(visual.debug_bounds("my-tasks-load-earlier").is_some());
    // Scrolled to the top of the history, scrolling further up loads more.
    let history = visual.debug_bounds("my-tasks-history").unwrap();
    for _ in 0..40 {
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: history.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(400.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
    }
    visual.run_until_parked();
    visual.refresh().unwrap();
    let turns = window
        .read_with(visual, |shell, cx| {
            let view = shell.my_tasks.as_ref().unwrap().read(cx);
            let history = &view.selected().unwrap().history;
            (history.turns.len(), history.earlier)
        })
        .unwrap();
    assert_eq!(turns, (8, 0));
    // The older turns open above the ones already read.
    assert!(visual.debug_bounds("my-tasks-turn-0").is_some());
    assert!(visual.debug_bounds("my-tasks-load-earlier").is_none());
}

/// A session whose last turn ran `calls` tool calls back to back.
fn tool_run(window: WindowHandle<Shell>, visual: &mut VisualTestContext, calls: usize) {
    let id = other_session(window, visual);
    let tool = |id: String| {
        let mut call = gespan_core::tool::ToolCall::new(&id, "Bash");
        call.input = serde_json::json!({ "command": "cargo test" }).to_string();
        serde_json::to_string(&call).unwrap()
    };
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.push_history_for_test("user", "Run the tests", cx);
                for n in 0..calls {
                    view.push_history_for_test("tool", &tool(format!("t{n}")), cx);
                }
            });
        })
        .unwrap();
}

#[gpui::test]
fn each_tool_call_is_its_own_pill_on_one_row(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    tool_run(window, visual, 2);
    let (_, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    // One row for the run, counting both calls, with a pill for each.
    assert!(visual.debug_bounds("my-tasks-tools-0-2").is_some());
    assert!(
        visual.debug_bounds("my-tasks-tools-0-1").is_none(),
        "one row, not one each"
    );
    let first = visual
        .debug_bounds("my-tasks-tool-0-0")
        .expect("first pill");
    let second = visual
        .debug_bounds("my-tasks-tool-0-1")
        .expect("second pill");
    assert_eq!(first.origin.y, second.origin.y, "side by side");
    assert!(second.origin.x > first.origin.x + first.size.width);
    assert!(visual.debug_bounds("my-tasks-tools-more-0").is_none());
}

#[gpui::test]
fn a_long_run_of_tool_calls_folds_into_more(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    tool_run(window, visual, 5);
    let (_, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    assert!(visual.debug_bounds("my-tasks-tools-0-5").is_some());
    assert!(visual.debug_bounds("my-tasks-tool-0-2").is_some());
    assert!(visual.debug_bounds("my-tasks-tool-0-3").is_none());
    assert!(visual.debug_bounds("my-tasks-tools-more-0").is_some());
}

/// A Bash approval for `command`, as Claude Code sends one.
fn bash(id: &str, command: &str) -> gespan_core::agent::Approval {
    let mut call = gespan_core::tool::ToolCall::new(&format!("call-{id}"), "Bash");
    call.input = serde_json::json!({ "command": command }).to_string();
    gespan_core::agent::Approval {
        id: id.into(),
        title: "Allow Bash?".into(),
        detail: command.into(),
        can_remember: true,
        tool: Some(call),
    }
}

#[gpui::test]
fn a_shell_command_sits_in_one_unwrapped_line_with_the_answer_hint(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    // Far wider than the card: it must stay on one line, not wrap.
    let long = format!(
        "gh pr create --base main --title \"{}\"",
        "keep the version ".repeat(12)
    );
    let (_, _controls) = waiting_page(window, visual, vec![bash("a1", &long)]);
    let well = visual
        .debug_bounds("my-tasks-command")
        .expect("the command well");
    let card = visual.debug_bounds("my-tasks-card").unwrap();
    assert!(
        well.size.height < px(48.),
        "one 19px line plus padding: {well:?}"
    );
    assert!(well.size.width <= card.size.width);
    assert!(visual.debug_bounds("my-tasks-card-hint").is_some());
    assert!(
        visual.debug_bounds("approval-detail").is_none(),
        "no second copy"
    );
}

#[gpui::test]
fn a_multi_line_command_shows_every_line(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (_, _controls) = waiting_page(
        window,
        visual,
        vec![bash("a1", "cd web\nbun install\nbun run build")],
    );
    let well = visual
        .debug_bounds("my-tasks-command")
        .expect("the command well");
    assert!(well.size.height >= px(57.), "three 19px lines: {well:?}");
}

#[gpui::test]
fn a_request_with_no_command_keeps_the_full_detail(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (_, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    assert!(visual.debug_bounds("my-tasks-command").is_none());
    assert!(visual.debug_bounds("approval-detail").is_some());
}

#[gpui::test]
fn enter_on_the_page_allows_the_selected_request_once(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (_, controls) = waiting_page(
        window,
        visual,
        vec![bash("a1", "git push"), bash("a2", "ls")],
    );
    let view = page(window, visual);
    window
        .update(visual, |_, window, cx| {
            view.focus_handle(cx).focus(window, cx)
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    let sent = controls.try_recv().expect("Enter answered");
    assert_eq!(sent.id, "a1");
    assert!(matches!(sent.decision, gespan_core::agent::Decision::Allow));
    assert!(controls.try_recv().is_err(), "one answer for one press");
}

#[gpui::test]
fn enter_while_typing_or_with_a_modifier_answers_nothing(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (_, controls) = waiting_page(window, visual, vec![bash("a1", "git push")]);
    let view = page(window, visual);
    window
        .update(visual, |_, window, cx| {
            let search = view.read(cx).search.focus_handle(cx);
            search.focus(window, cx);
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    assert!(
        controls.try_recv().is_err(),
        "Enter in the search allowed a command"
    );
    window
        .update(visual, |_, window, cx| {
            view.focus_handle(cx).focus(window, cx)
        })
        .unwrap();
    visual.simulate_keystrokes("cmd-enter shift-enter");
    assert!(
        controls.try_recv().is_err(),
        "a modified Enter allowed a command"
    );
}

#[gpui::test]
fn open_session_opens_the_tasks_session(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    click(visual, "my-tasks-open-source");
    window
        .read_with(visual, |shell, _| {
            assert_eq!(shell.active_app_view, None);
            assert_eq!(shell.workspace.active, Some(id));
        })
        .unwrap();
}

/// The task under `key` as the database keeps it.
fn kept(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    key: &str,
) -> Option<gespan_core::tasks::TaskRecord> {
    window
        .read_with(visual, |shell, _| {
            crate::runtime::locked(&shell.store)
                .unwrap()
                .task(key)
                .unwrap()
        })
        .unwrap()
}

#[gpui::test]
fn an_approval_answered_on_my_tasks_is_kept_with_its_request_and_answer(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    let key = format!("approval:{id}:a1");
    let open = kept(window, visual, &key).expect("kept once it was seen waiting");
    assert_eq!(open.status, gespan_core::tasks::Status::Open);
    assert_eq!(open.request["detail"], "git status");
    click(visual, "my-tasks-allow");
    let answered = kept(window, visual, &key).unwrap();
    assert_eq!(answered.status, gespan_core::tasks::Status::Answered);
    assert_eq!(answered.answer.as_deref(), Some("Allowed once"));
    assert_eq!(answered.answer_value, Some(serde_json::json!("allow")));
    assert_eq!(answered.asked_at, open.asked_at, "the wait start is kept");
    assert_eq!(answered.origin, format!("session:{id}"));
}

#[gpui::test]
fn an_approval_answered_in_its_chat_is_kept_even_before_my_tasks_saw_it(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    let (send, _controls) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.controls = Some(send);
                view.set_approvals_for_test(vec![approval("a1")]);
                // The chat's shelf and the phone both decide through here.
                view.decide_notification_request("a1", gespan_core::agent::Decision::Session, cx);
            });
        })
        .unwrap();
    let answered = kept(window, visual, &format!("approval:{id}:a1")).expect("kept");
    assert_eq!(answered.answer.as_deref(), Some("Allowed for this session"));
    assert_eq!(answered.headline, "Allow Bash?");
}

#[gpui::test]
fn a_question_answer_is_kept_with_every_option_offered(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    let (send, _controls) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, window, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.controls = Some(send);
                view.add_question(question("q1", "How should fsidCount be kept?"), cx);
            });
            shell.open_my_tasks(window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "question-option-0-0");
    if visual.debug_bounds("question-continue").is_some() {
        click(visual, "question-continue");
    }
    let answered = kept(window, visual, &format!("question:{id}:q1")).expect("kept");
    assert_eq!(answered.status, gespan_core::tasks::Status::Answered);
    assert_eq!(answered.answer.as_deref(), Some("A"));
    assert_eq!(answered.headline, "How should fsidCount be kept?");
    assert_eq!(answered.request["items"][0]["options"][0]["label"], "A");
    assert!(
        answered.can_remember(),
        "a question's answer can become a decision"
    );
}

#[gpui::test]
fn a_request_that_stops_waiting_unanswered_is_kept_as_withdrawn(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    window
        .update(visual, |shell, _, cx| {
            // The turn was stopped: the request is gone with no answer.
            shell.sessions[&id].update(cx, |view, _| view.set_approvals_for_test(vec![]));
            shell.refresh_my_tasks(cx);
        })
        .unwrap();
    let withdrawn = kept(window, visual, &format!("approval:{id}:a1")).unwrap();
    assert_eq!(withdrawn.status, gespan_core::tasks::Status::Withdrawn);
    assert_eq!(withdrawn.answer, None);
}

/// Answers a question on the page, the way the user does.
fn answer_question_on_page(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    prompt: &str,
) -> i64 {
    let id = other_session(window, visual);
    let (send, controls) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, window, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.controls = Some(send);
                view.add_question(question("q1", prompt), cx);
            });
            shell.open_my_tasks(window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "question-option-0-0");
    if visual.debug_bounds("question-continue").is_some() {
        click(visual, "question-continue");
    }
    controls.try_recv().expect("the answer reached the session");
    window
        .update(visual, |shell, _, cx| shell.refresh_my_tasks(cx))
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    id
}

#[gpui::test]
fn the_page_opens_on_waiting_and_answered_lists_what_you_chose(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, _controls) = waiting_page(window, visual, vec![approval("a1"), approval("a2")]);
    assert_eq!(
        page(window, visual).read_with(visual, |p, _| p.tab),
        crate::my_tasks::Tab::Waiting
    );
    click(visual, "my-tasks-allow");
    window
        .update(visual, |shell, _, cx| {
            // a2 stops waiting with no answer.
            shell.sessions[&id].update(cx, |view, _| view.set_approvals_for_test(vec![]));
            shell.refresh_my_tasks(cx);
        })
        .unwrap();
    click(visual, "my-tasks-tab-answered");
    let first = format!("my-tasks-past-approval:{id}:a1");
    let second = format!("my-tasks-past-approval:{id}:a2");
    assert!(
        visual
            .debug_bounds(Box::leak(first.clone().into_boxed_str()))
            .is_some()
    );
    assert!(
        visual
            .debug_bounds(Box::leak(second.clone().into_boxed_str()))
            .is_some()
    );
    let shown = |visual: &mut VisualTestContext| {
        page(window, visual).read_with(visual, |p, cx| {
            p.visible_past(cx)
                .iter()
                .map(|t| (t.key.clone(), t.answer.clone()))
                .collect::<Vec<_>>()
        })
    };
    let past = shown(visual);
    assert_eq!(past.len(), 2, "{past:?}");
    assert!(past.contains(&(format!("approval:{id}:a1"), Some("Allowed once".into()))));
    assert!(past.contains(&(format!("approval:{id}:a2"), None)));
    // The detail says what was chosen; an approval is not a decision.
    click(visual, &first);
    assert!(visual.debug_bounds("my-tasks-past-answer").is_some());
    assert!(visual.debug_bounds("my-tasks-remember").is_none());
    // The Answered segment counts what its list holds.
    assert!(visual.debug_bounds("my-tasks-count-answered-2").is_some());
    // Waiting has nothing left, and its count says so.
    click(visual, "my-tasks-tab-waiting");
    assert_eq!(
        page(window, visual).read_with(visual, |p, _| p.open_count()),
        0
    );
}

#[gpui::test]
fn a_question_answered_on_the_page_is_offered_and_saved_as_a_decision(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = answer_question_on_page(window, visual, "How should fsidCount be kept?");
    assert!(
        visual.debug_bounds("my-tasks-remember").is_some(),
        "the answer is offered as a decision"
    );
    let statement =
        page(window, visual).read_with(visual, |p, cx| p.statement.read(cx).content.to_string());
    assert_eq!(statement, "How should fsidCount be kept: A");
    let field = page(window, visual).read_with(visual, |p, _| p.statement.clone());
    field.update(visual, |input, cx| {
        input.set_text("Recount fsidCount at read time.".into(), cx)
    });
    click(visual, "my-tasks-save-decision");
    assert!(
        visual.debug_bounds("my-tasks-remember").is_none(),
        "offered once"
    );
    let decisions = window
        .read_with(visual, |shell, _| {
            crate::runtime::locked(&shell.store)
                .unwrap()
                .decisions()
                .unwrap()
        })
        .unwrap();
    let [decision] = &decisions[..] else {
        panic!("one decision: {decisions:?}");
    };
    assert_eq!(decision.statement, "Recount fsidCount at read time.");
    assert_eq!(decision.sources[0].key, format!("question:{id}:q1"));
    click(visual, "my-tasks-tab-decisions");
    assert!(
        visual
            .debug_bounds(Box::leak(
                format!("my-tasks-decision-{}", decision.id).into_boxed_str()
            ))
            .is_some()
    );
    assert!(visual.debug_bounds("my-tasks-decision-statement").is_some());
    // On Answered, the answer links to its decision instead of offering again.
    click(visual, "my-tasks-tab-answered");
    assert!(visual.debug_bounds("my-tasks-view-decision").is_some());
    assert!(visual.debug_bounds("my-tasks-remember").is_none());
}

#[gpui::test]
fn just_this_once_keeps_the_answer_without_a_decision(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    answer_question_on_page(window, visual, "Which counter?");
    click(visual, "my-tasks-just-once");
    assert!(visual.debug_bounds("my-tasks-remember").is_none());
    let (past, decisions) = window
        .read_with(visual, |shell, _| {
            let store = crate::runtime::locked(&shell.store).unwrap();
            (store.past_tasks().unwrap(), store.decisions().unwrap())
        })
        .unwrap();
    assert_eq!(past.len(), 1);
    assert!(decisions.is_empty());
    // It can still be saved later from Answered.
    click(visual, "my-tasks-tab-answered");
    assert!(visual.debug_bounds("my-tasks-remember-start").is_some());
}

#[gpui::test]
fn a_decision_is_edited_and_archived_on_the_decisions_tab(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let decision = window
        .update(visual, |shell, window, cx| {
            let id = crate::runtime::locked(&shell.store)
                .unwrap()
                .save_decision(
                    "/work/gespan",
                    "Review only my own merge requests.",
                    "",
                    &[],
                    1,
                )
                .unwrap();
            shell.open_my_tasks(window, cx);
            id
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "my-tasks-tab-decisions");
    click(visual, "my-tasks-edit-decision");
    let field = page(window, visual).read_with(visual, |p, _| p.statement.clone());
    assert_eq!(
        field.read_with(visual, |input, _| input.content.to_string()),
        "Review only my own merge requests."
    );
    field.update(visual, |input, cx| {
        input.set_text("Review merge requests from my team.".into(), cx)
    });
    click(visual, "my-tasks-edit-save");
    let saved = |visual: &mut VisualTestContext| {
        window
            .read_with(visual, |shell, _| {
                crate::runtime::locked(&shell.store)
                    .unwrap()
                    .decisions()
                    .unwrap()
            })
            .unwrap()
    };
    assert_eq!(
        saved(visual)[0].statement,
        "Review merge requests from my team."
    );
    assert_eq!(saved(visual)[0].id, decision);
    click(visual, "my-tasks-archive-decision");
    assert!(saved(visual).is_empty());
    assert!(
        visual
            .debug_bounds(Box::leak(
                format!("my-tasks-decision-{decision}").into_boxed_str()
            ))
            .is_none()
    );
}

#[gpui::test]
fn the_answered_filter_has_no_waiting_section_and_hides_projects(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (id, _controls) = waiting_page(window, visual, vec![approval("a1")]);
    click(visual, "my-tasks-allow");
    click(visual, "my-tasks-tab-answered");
    click(visual, "my-tasks-filter");
    assert!(visual.debug_bounds("my-tasks-filter-wait-any").is_none());
    let project = window
        .read_with(visual, |shell, cx| {
            shell.sessions[&id].read(cx).summary.cwd.clone()
        })
        .unwrap();
    click(visual, &format!("my-tasks-filter-project-{project}"));
    let shown = page(window, visual).read_with(visual, |p, cx| p.visible_past(cx).len());
    assert_eq!(shown, 0, "the hidden project's answers are filtered out");
}

/// The decision log, on, on a gespan.db in a temporary directory this
/// process's tests share (nextest gives each test its own process). Each test
/// uses its own project and finds its tasks by key, so a shared store never
/// mixes them. Call it after `setup`: the shell applies the user's
/// preference, off by default, as it starts.
fn decision_log() {
    static DIR: std::sync::LazyLock<tempfile::TempDir> =
        std::sync::LazyLock::new(|| tempfile::tempdir().unwrap());
    gespan_core::decisions::init(&DIR.path().join("gespan.db")).unwrap();
    gespan_core::decisions::set_enabled(true);
}

/// Held for a whole test that switches the decision log on or off, opens
/// it, or relies on its hooks: they are process-wide, and `cargo test` runs
/// tests on threads of one process.
fn log_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOG.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_log<T>(f: impl FnOnce(&gespan_core::decisions::DecisionStore) -> T) -> T {
    gespan_core::decisions::service::with(f).unwrap()
}

/// A pending proposal in `project` that the user answered in `session`.
fn propose_decision(
    project: &str,
    session: i64,
    title: &str,
    text: &str,
) -> gespan_core::decisions::Entry {
    use gespan_core::decisions::{Kind as EntryKind, NewEntry, Scope, Source};
    with_log(|log| {
        log.propose(
            NewEntry {
                kind: EntryKind::Decision,
                scope: Scope::Project(project.into()),
                title: title.into(),
                text: text.into(),
                why: "panics hid failures in A".into(),
                user_answer: Some("Use Result<T, MigrationError>, never panics".into()),
                source: Source {
                    session: Some(session),
                    question_ref: Some("q1".into()),
                    ..Source::default()
                },
            },
            1,
        )
        .unwrap()
    })
}

fn active_decision(project: &str, title: &str, text: &str) -> gespan_core::decisions::Entry {
    let entry = propose_decision(project, 1, title, text);
    with_log(|log| log.approve(entry.id, None, false, &[], 2).unwrap())
}

fn log_entry(id: i64) -> gespan_core::decisions::Entry {
    with_log(|log| log.get(id).unwrap().unwrap().0)
}

/// The task with `key` once the decision log is read again, if any.
fn task_by_key(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    key: &str,
) -> Option<TaskItem> {
    window
        .update(visual, |shell, _, cx| shell.load_decision_tasks(cx))
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(visual, |shell, cx| {
            shell
                .my_tasks
                .as_ref()?
                .read(cx)
                .tasks()
                .iter()
                .find(|t| t.key == key)
                .cloned()
        })
        .unwrap()
}

/// Opens My tasks with the card on `key`.
fn open_task(window: WindowHandle<Shell>, visual: &mut VisualTestContext, key: &str) {
    window
        .update(visual, |shell, window, cx| shell.open_my_tasks(window, cx))
        .unwrap();
    visual.run_until_parked();
    page(window, visual).update(visual, |view, cx| view.select(key, cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(selected_key(window, visual).as_deref(), Some(key));
}

/// Whether the page lists `key` now, without reading the log again.
fn listed(window: WindowHandle<Shell>, visual: &mut VisualTestContext, key: &str) -> bool {
    window
        .read_with(visual, |shell, cx| {
            shell
                .my_tasks
                .as_ref()
                .is_some_and(|view| view.read(cx).tasks().iter().any(|t| t.key == key))
        })
        .unwrap()
}

fn on_screen(visual: &mut VisualTestContext, selector: &str) -> bool {
    visual.refresh().unwrap();
    visual
        .debug_bounds(Box::leak(selector.to_string().into_boxed_str()))
        .is_some()
}

#[gpui::test]
fn pending_proposal_appears_as_decision_task_with_question_and_answer(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let session = other_session(window, visual);
    let project = "/work/decisions-pending";
    let entry = propose_decision(project, session, "Errors as values", "Return Result");
    let key = format!("decision:{}", entry.id);
    let task = task_by_key(window, visual, &key).expect("the proposal is a task");
    assert_eq!(task.kind, Kind::Decision);
    assert_eq!(Kind::Decision.label(), "Decision to review");
    assert_eq!(task.headline, "Errors as values");
    assert_eq!(task.project, project);
    assert_eq!(task.source, Source::Session(session));
    let Body::Decision {
        entry: shown,
        flag,
        similar,
    } = &task.body
    else {
        panic!("a decision card");
    };
    assert_eq!(shown.text, "Return Result");
    assert_eq!(shown.why, "panics hid failures in A");
    assert_eq!(
        shown.user_answer.as_deref(),
        Some("Use Result<T, MigrationError>, never panics")
    );
    assert!(flag.is_none() && similar.is_empty());
    open_task(window, visual, &key);
    for part in [
        "my-tasks-review-text",
        "my-tasks-review-why",
        "my-tasks-review-answer",
        "my-tasks-review-approve",
        "my-tasks-review-edit",
        "my-tasks-review-pin",
        "my-tasks-review-reject",
    ] {
        assert!(on_screen(visual, part), "{part} is on the card");
    }
    // Nothing to replace: no "Replaces:" choices, and no flag.
    assert!(!on_screen(visual, "my-tasks-review-replaces"));
    assert!(!on_screen(visual, "my-tasks-review-flag"));
    // The link opens the session the user answered in.
    click(visual, "my-tasks-open-source");
    window
        .read_with(visual, |shell, _| {
            assert_eq!(shell.workspace.active, Some(session))
        })
        .unwrap();
}

#[gpui::test]
fn open_flag_appears_on_its_decision_task(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let session = other_session(window, visual);
    let entry = active_decision("/work/decisions-flag", "No panics", "Never panic");
    let flag = with_log(|log| {
        log.flag(entry.id, session, "FFI boundaries must abort instead", 3)
            .unwrap()
    });
    let key = format!("decision-flag:{}", flag.id);
    let task = task_by_key(window, visual, &key).expect("the flag is a task");
    assert_eq!(task.kind, Kind::Decision);
    assert_eq!(task.headline, "No panics");
    assert_eq!(task.source, Source::Session(session));
    let Body::Decision {
        entry: shown,
        flag: Some(shown_flag),
        ..
    } = &task.body
    else {
        panic!("a decision card with its flag");
    };
    assert_eq!(shown.id, entry.id);
    assert_eq!(shown_flag.reason, "FFI boundaries must abort instead");
    // An active decision is not also a proposal to review.
    assert!(task_by_key(window, visual, &format!("decision:{}", entry.id)).is_none());
    open_task(window, visual, &key);
    assert!(on_screen(visual, "my-tasks-review-flag"));
    assert!(!on_screen(visual, "my-tasks-review-approve"));
    assert!(on_screen(visual, "my-tasks-review-dismiss"));
    // "Replace with…" opens the fields on the current wording; saving makes
    // the new wording the active successor and closes the flag.
    click(visual, "my-tasks-review-replace");
    assert!(on_screen(visual, "my-tasks-review-edit-title"));
    page(window, visual).update(visual, |view, cx| {
        assert_eq!(
            view.decision_text.read(cx).content.to_string(),
            "Never panic"
        );
        view.decision_text.update(cx, |input, cx| {
            input.set_text("Never panic, except at FFI boundaries".into(), cx)
        });
    });
    click(visual, "my-tasks-review-save-replacement");
    let old = log_entry(entry.id);
    assert_eq!(old.status, gespan_core::decisions::Status::Replaced);
    let successor = log_entry(old.replaced_by.expect("a successor"));
    assert_eq!(successor.status, gespan_core::decisions::Status::Active);
    assert_eq!(successor.text, "Never panic, except at FFI boundaries");
    assert!(
        task_by_key(window, visual, &key).is_none(),
        "the flag is closed"
    );
}

#[gpui::test]
fn dismissing_a_flag_keeps_the_decision(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let entry = active_decision("/work/decisions-dismiss", "No panics", "Never panic");
    let flag = with_log(|log| log.flag(entry.id, 1, "disagree", 3).unwrap());
    let key = format!("decision-flag:{}", flag.id);
    assert!(task_by_key(window, visual, &key).is_some());
    open_task(window, visual, &key);
    click(visual, "my-tasks-review-dismiss");
    assert!(task_by_key(window, visual, &key).is_none());
    assert_eq!(
        log_entry(entry.id).status,
        gespan_core::decisions::Status::Active
    );
}

#[gpui::test]
fn approve_removes_the_task(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let project = "/work/decisions-approve";
    let old = active_decision(project, "Error handling", "Errors use anyhow");
    let entry = propose_decision(project, 1, "Error handling", "Errors use thiserror");
    let key = format!("decision:{}", entry.id);
    let task = task_by_key(window, visual, &key).expect("the proposal is a task");
    let Body::Decision { similar, .. } = &task.body else {
        panic!("a decision card");
    };
    assert_eq!(similar.iter().map(|e| e.id).collect::<Vec<_>>(), [old.id]);
    open_task(window, visual, &key);
    // Offered, unticked: approving alone would keep the old one.
    assert!(on_screen(visual, "my-tasks-review-replaces"));
    page(window, visual).read_with(visual, |view, _| {
        assert!(view.decision_replaces().is_empty());
        assert!(!view.decision_pinned());
    });
    click(visual, &format!("my-tasks-review-replaces-{}", old.id));
    click(visual, "my-tasks-review-pin");
    click(visual, "my-tasks-review-approve");
    assert!(
        task_by_key(window, visual, &key).is_none(),
        "approved leaves"
    );
    let approved = log_entry(entry.id);
    assert_eq!(approved.status, gespan_core::decisions::Status::Active);
    assert!(approved.pinned);
    assert_eq!(approved.text, "Errors use thiserror");
    assert_eq!(log_entry(old.id).replaced_by, Some(entry.id));
    // The review is kept on the Answered tab.
    let kept = window
        .read_with(visual, |shell, _| shell.past_tasks_for_test())
        .unwrap();
    let kept = kept
        .iter()
        .find(|t| t.key == key)
        .expect("the review is kept");
    assert_eq!(kept.answer.as_deref(), Some("Approved"));
}

#[gpui::test]
fn edit_then_approve_saves_the_new_wording_and_reject_leaves(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let project = "/work/decisions-edit";
    let entry = propose_decision(project, 1, "Errors", "Use anyhow");
    let other = propose_decision(project, 1, "Tabs", "Indent with tabs");
    let key = format!("decision:{}", entry.id);
    task_by_key(window, visual, &key).expect("the proposal is a task");
    open_task(window, visual, &key);
    click(visual, "my-tasks-review-edit");
    page(window, visual).update(visual, |view, cx| {
        assert_eq!(view.decision_title.read(cx).content.to_string(), "Errors");
        view.decision_text
            .update(cx, |input, cx| input.set_text("Return Result".into(), cx));
    });
    click(visual, "my-tasks-review-approve");
    let approved = log_entry(entry.id);
    assert_eq!(approved.status, gespan_core::decisions::Status::Active);
    assert_eq!(approved.text, "Return Result");
    assert_eq!(approved.title, "Errors");
    let key = format!("decision:{}", other.id);
    task_by_key(window, visual, &key).expect("the second proposal is a task");
    open_task(window, visual, &key);
    click(visual, "my-tasks-review-reject");
    assert!(task_by_key(window, visual, &key).is_none());
    assert_eq!(
        log_entry(other.id).status,
        gespan_core::decisions::Status::Rejected
    );
}

#[gpui::test]
fn a_proposal_revised_after_the_card_showed_it_is_refused_and_shown_again(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let entry = propose_decision("/work/decisions-revised", 1, "Errors", "Use thiserror");
    let key = format!("decision:{}", entry.id);
    task_by_key(window, visual, &key).expect("the proposal is a task");
    open_task(window, visual, &key);
    // The agent rewrites its proposal while the card shows the old wording.
    with_log(|log| {
        log.revise_pending(
            entry.id,
            gespan_core::decisions::Edit {
                title: entry.title.clone(),
                text: "Errors are plain strings".into(),
                why: entry.why.clone(),
            },
            entry.user_answer.as_deref(),
            &entry.source,
        )
        .unwrap()
    });
    click(visual, "my-tasks-review-approve");
    assert_eq!(
        log_entry(entry.id).status,
        gespan_core::decisions::Status::Pending
    );
    assert!(on_screen(visual, "my-tasks-review-error"));
    // The card now shows what the agent wrote, and approving that works.
    let task = task_by_key(window, visual, &key).expect("still waiting");
    let Body::Decision { entry: shown, .. } = &task.body else {
        panic!("a decision card");
    };
    assert_eq!(shown.text, "Errors are plain strings");
    visual.run_until_parked();
    click(visual, "my-tasks-review-approve");
    assert_eq!(
        log_entry(entry.id).status,
        gespan_core::decisions::Status::Active
    );
    assert!(!on_screen(visual, "my-tasks-review-error"));
}

#[gpui::test]
fn kind_filter_hides_decisions(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let entry = propose_decision("/work/decisions-filter", 1, "Errors", "Use anyhow");
    let key = format!("decision:{}", entry.id);
    task_by_key(window, visual, &key).expect("the proposal is a task");
    open_task(window, visual, &key);
    let row = format!("my-tasks-row-{key}");
    assert!(on_screen(visual, &row));
    click(visual, "my-tasks-filter");
    click(visual, "my-tasks-filter-kind-Decision to review");
    let shown = page(window, visual).read_with(visual, |view, cx| {
        view.visible(cx).iter().map(|t| t.kind).collect::<Vec<_>>()
    });
    assert!(!shown.contains(&Kind::Decision), "{shown:?}");
    assert!(!on_screen(visual, &row));
}

#[gpui::test]
fn decisions_are_not_tasks_while_the_log_is_off(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    // After the shell applied the (off) preference.
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let entry = propose_decision("/work/decisions-off", 1, "Errors", "Use anyhow");
    let key = format!("decision:{}", entry.id);
    gespan_core::decisions::set_enabled(false);
    let off = task_by_key(window, visual, &key);
    gespan_core::decisions::set_enabled(true);
    assert!(off.is_none());
    assert!(task_by_key(window, visual, &key).is_some());
}

#[gpui::test]
fn decisions_tab_groups_preferences_and_names_repo_scoped_decisions_by_project(
    cx: &mut TestAppContext,
) {
    use gespan_core::decisions::{Config, DecisionStore, Kind as EntryKind, NewEntry, Scope};
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    // The app's store and the decision log on one gespan.db, as in the app.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let store = gespan_core::store::Store::open_gespan(&path).unwrap();
    let log = DecisionStore::open(&path, Config::app()).unwrap();
    let gespan = Scope::Repo("github.com/o/gespan@0123456789ab".into());
    let widgets = Scope::Repo("github.com/o/widgets@ba9876543210".into());
    let approved = |kind, scope: &Scope, title: &str| {
        let entry = log
            .propose(
                NewEntry {
                    kind,
                    scope: scope.clone(),
                    title: title.into(),
                    text: title.into(),
                    why: "the user said so".into(),
                    user_answer: None,
                    source: Default::default(),
                },
                1,
            )
            .unwrap();
        log.approve(entry.id, None, false, &[], 2).unwrap().id
    };
    let short = approved(EntryKind::Preference, &Scope::Global, "Answer briefly");
    let tests = approved(EntryKind::Preference, &Scope::Global, "Tests first");
    let proposed = approved(EntryKind::Decision, &gespan, "Errors as values");
    let only_proposed = approved(EntryKind::Decision, &widgets, "Widgets are pure");
    let saved = store
        .save_decision_in(&gespan, "/work/gespan-main", "Squash merges", "", &[], 3)
        .unwrap();
    store
        .save_decision_in(
            &Scope::Project("/work/api".into()),
            "/work/api",
            "Tabs, not spaces",
            "",
            &[],
            3,
        )
        .unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(store));
    window
        .update(visual, |shell, window, cx| {
            shell.open_my_tasks(window, cx);
            let view = shell.my_tasks.clone().unwrap();
            view.update(cx, |view, cx| {
                view.store = Some(store.clone());
                view.reload(cx);
            });
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "my-tasks-tab-decisions");
    let groups = page(window, visual).read_with(visual, |view, _| view.decision_groups_for_test());
    let names: Vec<(&str, usize)> = groups
        .iter()
        .map(|(_, name, ids)| (name.as_str(), ids.len()))
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        [
            ("Your preferences", 2),
            ("api", 1),
            // Saved in the gespan-main checkout, with an agent's proposal for
            // the same repository beside it.
            ("gespan-main", 2),
            // Only an agent's proposal: named after its repository.
            ("widgets", 1),
        ]
    );
    let ids = |name: &str| {
        let mut ids = groups.iter().find(|(_, n, _)| n == name).unwrap().2.clone();
        ids.sort();
        ids
    };
    assert_eq!(ids("Your preferences"), {
        let mut both = vec![short, tests];
        both.sort();
        both
    });
    assert_eq!(ids("gespan-main"), {
        let mut both = vec![proposed, saved];
        both.sort();
        both
    });
    assert_eq!(ids("widgets"), [only_proposed]);
    for group in [
        "my-tasks-group-global",
        "my-tasks-group-repo:github.com/o/gespan@0123456789ab",
        "my-tasks-group-/work/api",
    ] {
        assert!(on_screen(visual, group), "{group} is on the list");
    }
    // The detail pane names the group as its list header does.
    click(visual, &format!("my-tasks-decision-{proposed}"));
    assert!(on_screen(visual, "my-tasks-decision-group-gespan-main"));
    // Editing and archiving work on the decision log's own entries.
    click(visual, "my-tasks-edit-decision");
    page(window, visual).update(visual, |view, cx| {
        view.statement.update(cx, |input, cx| {
            input.set_text("Return errors as values".into(), cx)
        })
    });
    click(visual, "my-tasks-edit-save");
    let entry = log.get(proposed).unwrap().unwrap().0;
    assert_eq!(entry.text, "Return errors as values");
    assert_eq!(entry.status, gespan_core::decisions::Status::Active);
    click(visual, &format!("my-tasks-decision-{short}"));
    click(visual, "my-tasks-archive-decision");
    assert_eq!(
        log.get(short).unwrap().unwrap().0.status,
        gespan_core::decisions::Status::Archived
    );
    assert!(!on_screen(visual, &format!("my-tasks-decision-{short}")));
}

#[gpui::test]
fn an_edit_started_before_the_agent_revised_is_refused(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let entry = propose_decision("/work/decisions-edit-race", 1, "Errors", "Use thiserror");
    let key = format!("decision:{}", entry.id);
    task_by_key(window, visual, &key).expect("the proposal is a task");
    open_task(window, visual, &key);
    click(visual, "my-tasks-review-edit");
    // The agent rewrites its proposal, and the card refreshes under the
    // user's open edit.
    with_log(|log| {
        log.revise_pending(
            entry.id,
            gespan_core::decisions::Edit {
                title: entry.title.clone(),
                text: "Errors are plain strings".into(),
                why: entry.why.clone(),
            },
            entry.user_answer.as_deref(),
            &entry.source,
        )
        .unwrap()
    });
    task_by_key(window, visual, &key).expect("still waiting");
    assert!(
        on_screen(visual, "my-tasks-review-edit-text"),
        "still editing"
    );
    click(visual, "my-tasks-review-approve");
    let after = log_entry(entry.id);
    assert_eq!(after.status, gespan_core::decisions::Status::Pending);
    assert_eq!(after.text, "Errors are plain strings");
    assert!(on_screen(visual, "my-tasks-review-error"));
}

#[gpui::test]
fn a_change_in_the_log_reloads_my_tasks_without_waiting_for_the_clock(cx: &mut TestAppContext) {
    let _log = log_lock();
    let window = setup(cx);
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            // Test shells leave the process-wide hook alone; this one listens.
            listen_for_decision_changes(cx);
            shell.open_my_tasks(window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    let entry = propose_decision("/work/decisions-pushed", 1, "Errors", "Use anyhow");
    let key = format!("decision:{}", entry.id);
    assert!(!listed(window, visual, &key), "not read yet");
    // What the toolkit does after a proposal, revision or flag.
    gespan_core::decisions::service::changed();
    visual.run_until_parked();
    assert!(listed(window, visual, &key));
    // Later tests must not wake this test's executor through the hook.
    gespan_core::decisions::service::set_changed(Box::new(|| {}));
}

/// Every test builds a shell on its own thread. If each shell took the
/// process-wide change hook, the one it replaced would be dropped on the new
/// test's thread and wake the old test's listener from there: GPUI's test
/// scheduler then panics with "Your test is not deterministic" in whichever
/// tests happen to overlap.
#[gpui::test]
fn a_test_shell_leaves_the_process_wide_change_hook_alone(cx: &mut TestAppContext) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let _log = log_lock();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    gespan_core::decisions::service::set_changed(Box::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let _window = setup(cx);
    gespan_core::decisions::service::changed();
    gespan_core::decisions::service::set_changed(Box::new(|| {}));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the shell replaced the hook"
    );
}

#[test]
fn a_replaced_change_hook_does_not_close_its_listener() {
    let (send, changes) = async_channel::unbounded::<()>();
    let hook = decision_change_hook(&send);
    hook();
    assert_eq!(changes.try_recv(), Ok(()));
    // The next shell's hook replaces this one, on another thread.
    std::thread::spawn(move || drop(hook)).join().unwrap();
    assert!(!changes.is_closed(), "dropping the hook closed the channel");
    // Once the listener's own sender is gone, a stale hook does nothing.
    let hook = decision_change_hook(&send);
    drop(send);
    assert!(changes.is_closed());
    hook();
    assert!(changes.try_recv().is_err());
}

#[gpui::test]
fn a_repo_scoped_proposal_is_keyed_by_its_scope_and_named_after_its_repository(
    cx: &mut TestAppContext,
) {
    use gespan_core::decisions::{Kind as EntryKind, NewEntry, Scope};
    let _log = log_lock();
    let window = setup(cx);
    decision_log();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let propose = |kind, scope: Scope, title: &str| {
        with_log(|log| {
            log.propose(
                NewEntry {
                    kind,
                    scope,
                    title: title.into(),
                    text: title.into(),
                    why: "the user said so".into(),
                    user_answer: None,
                    source: Default::default(),
                },
                1,
            )
            .unwrap()
        })
    };
    let repo = "github.com/o/widgets@ba9876543210";
    let decision = propose(
        EntryKind::Decision,
        Scope::Repo(repo.into()),
        "Pure widgets",
    );
    let preference = propose(EntryKind::Preference, Scope::Global, "Answer briefly");
    let task = task_by_key(window, visual, &format!("decision:{}", decision.id)).unwrap();
    assert_eq!(task.project, format!("repo:{repo}"));
    assert_eq!(crate::my_tasks::project_display(&task.project), "widgets");
    let task = task_by_key(window, visual, &format!("decision:{}", preference.id)).unwrap();
    assert_eq!(task.project, "global");
    assert_eq!(
        crate::my_tasks::project_display(&task.project),
        "Your preferences"
    );
    open_task(window, visual, &format!("decision:{}", decision.id));
    click(visual, "my-tasks-filter");
    assert!(on_screen(
        visual,
        &format!("my-tasks-filter-project-repo:{repo}")
    ));
}

#[gpui::test]
fn a_log_switched_on_but_not_open_lists_nothing_and_fails_nothing(_cx: &mut TestAppContext) {
    let _log = log_lock();
    // nextest runs each test in its own process, so nothing opened the log.
    gespan_core::decisions::set_enabled(true);
    let waiting = super::my_tasks::waiting_decisions();
    gespan_core::decisions::set_enabled(false);
    assert!(waiting.is_ok(), "{:?}", waiting.err());
}

/// My tasks refreshes every 30 s: a proposal's "may replace" search runs
/// again only when the proposal, its project's active entries or the
/// search mode changed, never for an unchanged refresh.
#[test]
fn decision_similar_entries_are_searched_once_per_change() {
    use gespan_core::decisions::{Config, DecisionStore, NewEntry, Scope, Source};
    use std::cell::Cell;
    let dir = tempfile::tempdir().unwrap();
    let log = DecisionStore::open(&dir.path().join("gespan.db"), Config::app()).unwrap();
    let scope = Scope::Repo("github.com/o/r@abc".into());
    let new = |title: &str| NewEntry {
        kind: gespan_core::decisions::Kind::Decision,
        scope: scope.clone(),
        title: title.into(),
        text: format!("{title} text"),
        why: "why".into(),
        user_answer: None,
        source: Source {
            session: Some(1),
            question_ref: Some("q".into()),
            ..Source::default()
        },
    };
    let first = log.propose(new("Use tabs"), 1).unwrap();
    log.propose(new("Use sqlite"), 2).unwrap();
    let searches = Cell::new(0);
    let search = |log: &DecisionStore, entry: &gespan_core::decisions::Entry| {
        searches.set(searches.get() + 1);
        gespan_core::decisions::review::similar_active(log, entry, 3)
    };
    let mut cache = super::my_tasks::SimilarCache::default();
    super::my_tasks::waiting_in(&log, &mut cache, &search).unwrap();
    assert_eq!(searches.get(), 2);
    // Nothing changed: no search at all.
    super::my_tasks::waiting_in(&log, &mut cache, &search).unwrap();
    super::my_tasks::waiting_in(&log, &mut cache, &search).unwrap();
    assert_eq!(searches.get(), 2);
    // An agent revises one proposal: only that one is searched again.
    log.revise_pending(
        first.id,
        gespan_core::decisions::Edit {
            title: "Use tabs everywhere".into(),
            text: "Indent with tabs".into(),
            why: "why".into(),
        },
        None,
        &first.source,
    )
    .unwrap();
    super::my_tasks::waiting_in(&log, &mut cache, &search).unwrap();
    assert_eq!(searches.get(), 3);
    // A new active entry in the project may be similar to both.
    let active = log.propose(new("Use spaces"), 3).unwrap();
    log.approve(active.id, None, false, &[], 4).unwrap();
    let waiting = super::my_tasks::waiting_in(&log, &mut cache, &search).unwrap();
    assert_eq!(searches.get(), 5);
    assert_eq!(waiting.len(), 2);
}

#[gpui::test]
fn the_shell_redrawing_does_not_redraw_the_my_tasks_page(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| shell.open_my_tasks(window, cx))
        .unwrap();
    visual.run_until_parked();
    set_tasks(window, visual, vec![flow_task("a", "gespan", 1)]);
    assert!(visual.debug_bounds("my-tasks-row-a").is_some());
    let view = page(window, visual);
    let renders = |visual: &mut VisualTestContext| view.read_with(visual, |view, _| view.renders);
    let before = renders(visual);
    // What a streamed token or the 1 s approval tick does: the shell
    // notifies and the window draws again.
    for _ in 0..5 {
        window.update(visual, |_, _, cx| cx.notify()).unwrap();
        visual.run_until_parked();
    }
    assert_eq!(renders(visual), before, "the cached page is reused");
    assert!(
        visual.debug_bounds("my-tasks-row-a").is_some(),
        "and still drawn"
    );
    // A change on the page itself still draws it.
    set_tasks(
        window,
        visual,
        vec![flow_task("a", "gespan", 1), flow_task("b", "gespan", 2)],
    );
    assert!(renders(visual) > before);
    assert!(visual.debug_bounds("my-tasks-row-b").is_some());
}
