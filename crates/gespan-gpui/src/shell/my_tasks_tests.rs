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
