//! Home inside a real shell: its sidebar entry, the tiles and links that
//! open each page, the range control, answering from it, and its empty
//! states.
use super::my_tasks_tests::{click, setup};
use super::*;
use crate::home::{Home, Range};
use crate::workspace::AppView;
use gespan_core::automations::{AutomationRun, Service};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

fn open_home(window: WindowHandle<Shell>, visual: &mut VisualTestContext) {
    window
        .update(visual, |shell, window, cx| shell.open_home(window, cx))
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn view(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> Option<AppView> {
    window
        .read_with(visual, |shell, _| shell.active_app_view)
        .unwrap()
}

fn home(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> Entity<Home> {
    window
        .read_with(visual, |shell, _| shell.home.clone().expect("Home view"))
        .unwrap()
}

fn drawn(visual: &mut VisualTestContext, selector: &str) -> bool {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    visual.debug_bounds(selector).is_some()
}

/// A real automation service on a temporary database, so Runs can open.
fn with_service(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::open(&dir.path().join("automations.db")).unwrap();
    window
        .update(visual, |shell, _, _| {
            shell.automation_state.service = Some(Arc::new(Mutex::new(service)));
        })
        .unwrap();
    dir
}

fn run(id: &str, status: &str, created_at: i64) -> AutomationRun {
    AutomationRun {
        id: id.into(),
        automation_id: "auto".into(),
        trigger: "manual".into(),
        scheduled_for: created_at,
        created_at,
        started_at: Some(created_at),
        completed_at: Some(created_at + 1_000),
        status: status.into(),
        session_id: None,
        error: None,
        event_key: None,
        event_kind: None,
        event: None,
        event_title: None,
        prompt: None,
        pilot_message_id: None,
    }
}

#[gpui::test]
fn home_is_the_first_page_icon_and_opens_the_page(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let bounds = |visual: &mut VisualTestContext, selector: &'static str| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector}"))
    };
    let home = bounds(visual, "sidebar-navigation-Home");
    let tasks = bounds(visual, "sidebar-navigation-My tasks");
    // Under the project switcher, Home leads the icon row.
    assert!(bounds(visual, "sidebar-project-button").bottom() <= home.top());
    assert_eq!(home.top(), tasks.top());
    assert!(home.right() <= tasks.left(), "Home comes before My tasks");
    click(visual, "sidebar-navigation-Home");
    assert_eq!(view(window, visual), Some(AppView::Home));
    assert!(drawn(visual, "home-greeting"));
    assert!(drawn(visual, "home-summary"));
}

#[gpui::test]
fn each_tile_opens_its_page(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let _db = with_service(window, visual);

    open_home(window, visual);
    click(visual, "home-tile-needs-you");
    assert_eq!(view(window, visual), Some(AppView::MyTasks));

    open_home(window, visual);
    click(visual, "home-tile-runs");
    assert_eq!(view(window, visual), Some(AppView::Runs));

    open_home(window, visual);
    click(visual, "home-tile-inbox");
    assert_eq!(view(window, visual), Some(AppView::Inbox));

    // Working now opens the sidebar's Agents panel and stays on Home.
    open_home(window, visual);
    assert!(!drawn(visual, "agents-panel"));
    click(visual, "home-tile-working");
    assert_eq!(view(window, visual), Some(AppView::Home));
    assert!(drawn(visual, "agents-panel"), "the Agents panel opened");
}

#[gpui::test]
fn the_panel_links_open_the_same_pages(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let _db = with_service(window, visual);
    open_home(window, visual);
    click(visual, "home-open-my-tasks");
    assert_eq!(view(window, visual), Some(AppView::MyTasks));
    open_home(window, visual);
    click(visual, "home-open-runs");
    assert_eq!(view(window, visual), Some(AppView::Runs));
    open_home(window, visual);
    click(visual, "home-all-agents");
    assert!(drawn(visual, "agents-panel"));
}

#[gpui::test]
fn switching_the_range_changes_the_counts(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    open_home(window, visual);
    let home = home(window, visual);
    let now = gespan_core::time::now_ms() as i64;
    let today = crate::home::day_start(now, 0);
    home.update(visual, |home, cx| {
        home.set_runs(
            vec![
                run("today", "succeeded", today + 1),
                run("three-days", "failed", crate::home::day_start(now, 3) + 1),
                run(
                    "twenty-days",
                    "succeeded",
                    crate::home::day_start(now, 20) + 1,
                ),
            ],
            Default::default(),
            cx,
        )
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    let read = |visual: &mut VisualTestContext| {
        home.read_with(visual, |home, _| {
            (home.range(), home.run_counts(now).total, home.summary(now))
        })
    };
    let (range, total, summary) = read(visual);
    assert_eq!((range, total), (Range::Today, 1));
    assert!(
        summary.ends_with("every automation passed today."),
        "{summary}"
    );

    click(visual, "home-range-week");
    let (range, total, summary) = read(visual);
    assert_eq!((range, total), (Range::Week, 2));
    assert!(
        summary.ends_with("every automation passed in the last 7 days except one."),
        "{summary}"
    );

    click(visual, "home-range-month");
    let (range, total, _) = read(visual);
    assert_eq!((range, total), (Range::Month, 3));
    // The month draws thirty bars; the others a week.
    assert!(drawn(visual, "home-bar-29"));
    click(visual, "home-range-today");
    assert!(drawn(visual, "home-bar-6"));
    assert!(!drawn(visual, "home-bar-7"));
}

#[gpui::test]
fn an_empty_home_says_so_in_every_panel(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    open_home(window, visual);
    // The test shell has no accounts installed and no Inbox loaded.
    for panel in ["waiting", "working", "chart", "usage", "today"] {
        assert!(drawn(visual, &format!("home-{panel}-empty")), "{panel}");
    }
    let summary = home(window, visual).read_with(visual, |home, _| {
        home.summary(gespan_core::time::now_ms() as i64)
    });
    assert_eq!(
        summary,
        "No agents are working, nothing is waiting on you, and no automations ran today."
    );
}

#[gpui::test]
fn allow_once_on_home_answers_the_request_like_my_tasks(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    let (send, controls) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, _| {
                view.controls = Some(send);
                view.set_approvals_for_test(vec![bash("a1", "gh pr create --fill")]);
            });
        })
        .unwrap();
    open_home(window, visual);
    let key = format!("approval:{id}:a1");
    assert!(drawn(visual, &format!("home-waiting-{key}")));
    assert!(!drawn(visual, "home-waiting-empty"));
    // The row shows the whole command it would allow.
    assert!(drawn(visual, &format!("home-command-{key}")));
    let shown =
        home(window, visual).read_with(visual, |home, _| home.waiting_for_test()[0].allow.clone());
    assert_eq!(
        shown.map(|a| a.command).as_deref(),
        Some("gh pr create --fill")
    );
    click(visual, &format!("home-allow-{key}"));
    let sent = controls.try_recv().expect("a decision was sent");
    assert_eq!(sent.id, "a1");
    assert!(matches!(sent.decision, gespan_core::agent::Decision::Allow));
    // Answering keeps you on Home.
    assert_eq!(view(window, visual), Some(AppView::Home));
}

/// A Bash tool approval, as Claude sends one.
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
fn a_request_home_cannot_show_in_full_opens_in_my_tasks_instead(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    let (send, controls) = std::sync::mpsc::channel();
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, _| {
                view.controls = Some(send);
                view.set_approvals_for_test(vec![
                    // A hidden character makes `rm` look like something else:
                    // the card raises an alert, so no one-click allow.
                    bash("hidden", "r\u{200B}m -rf build"),
                    // Two lines can't sit on one row whole.
                    bash("multi", "cd app\nrm -rf build"),
                ]);
            });
        })
        .unwrap();
    open_home(window, visual);
    for request in ["hidden", "multi"] {
        let key = format!("approval:{id}:{request}");
        assert!(
            drawn(visual, &format!("home-waiting-{key}")),
            "{request} is listed"
        );
        assert!(
            !drawn(visual, &format!("home-allow-{key}")),
            "{request}: no Allow once"
        );
        assert!(!drawn(visual, &format!("home-command-{key}")), "{request}");
    }
    // Its Open goes to My tasks on that request, where the full card is.
    let key = format!("approval:{id}:hidden");
    click(visual, &format!("home-answer-{key}"));
    assert_eq!(view(window, visual), Some(AppView::MyTasks));
    let selected = window
        .read_with(visual, |shell, cx| {
            shell
                .my_tasks
                .as_ref()
                .and_then(|view| view.read(cx).selected().map(|t| t.key.clone()))
        })
        .unwrap();
    assert_eq!(selected, Some(key));
    assert!(controls.try_recv().is_err(), "nothing was decided");
}

#[gpui::test]
fn a_question_on_home_opens_my_tasks_on_that_task(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let id = other_session(window, visual);
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.add_question(
                    gespan_core::agent::Question {
                        id: "q1".into(),
                        title: "A question".into(),
                        auto_resolve_at: None,
                        items: vec![gespan_core::agent::QuestionItem {
                            id: "only".into(),
                            prompt: "Which release branch?".into(),
                            multiple: false,
                            allow_custom: false,
                            options: vec![gespan_core::agent::QuestionOption {
                                id: "a".into(),
                                label: "main".into(),
                                description: String::new(),
                            }],
                        }],
                    },
                    cx,
                );
            });
        })
        .unwrap();
    open_home(window, visual);
    let key = format!("question:{id}:q1");
    // A question has no one-click answer here: Answer opens it in My tasks.
    assert!(!drawn(visual, &format!("home-allow-{key}")));
    click(visual, &format!("home-answer-{key}"));
    assert_eq!(view(window, visual), Some(AppView::MyTasks));
    let selected = window
        .read_with(visual, |shell, cx| {
            shell
                .my_tasks
                .as_ref()
                .and_then(|view| view.read(cx).selected().map(|t| t.key.clone()))
        })
        .unwrap();
    assert_eq!(selected, Some(key));
}

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

#[gpui::test]
fn home_caps_its_width_on_a_big_screen_and_stacks_on_a_narrow_one(cx: &mut TestAppContext) {
    let window = setup(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let bounds = |visual: &mut VisualTestContext, selector: &'static str| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector}"))
    };
    let lay_out = |visual: &mut VisualTestContext, width: f32, height: f32| {
        visual.simulate_resize(gpui::size(px(width), px(height)));
        // One frame measures the new width, the next lays out for it.
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.run_until_parked();
    };
    open_home(window, visual);

    lay_out(visual, 2600., 1400.);
    let needs = bounds(visual, "home-tile-needs-you");
    let inbox = bounds(visual, "home-tile-inbox");
    let content = (inbox.right() - needs.left()).as_f32();
    assert!(
        (crate::home::MAX_WIDTH - 1.0..=crate::home::MAX_WIDTH + 1.).contains(&content),
        "content stops at the cap on a big screen, got {content}"
    );
    assert_eq!(needs.top(), inbox.top(), "four tiles in one row");
    let waiting = bounds(visual, "home-waiting");
    let chart = bounds(visual, "home-chart");
    assert!(chart.left() > waiting.right(), "two columns side by side");
    let today = bounds(visual, "home-today");
    assert!(
        today.bottom().as_f32() < 1400. - 200.,
        "panels are as tall as what they show, not stretched to the window"
    );

    // A wider window only widens the margins, half on each side.
    lay_out(visual, 3000., 1400.);
    let moved = (bounds(visual, "home-tile-needs-you").left() - needs.left()).as_f32();
    assert!(
        (199.0..=201.).contains(&moved),
        "content stays centred, moved {moved}"
    );

    // Narrow: one column, Waiting first, the chart under Working; tiles two a row.
    lay_out(visual, 900., 1400.);
    let working = bounds(visual, "home-working");
    let chart = bounds(visual, "home-chart");
    assert!(chart.top() >= working.bottom(), "columns stack when narrow");
    assert_eq!(chart.left(), working.left());
    assert!(
        bounds(visual, "home-tile-runs").top() > bounds(visual, "home-tile-needs-you").bottom(),
        "tiles wrap two to a row"
    );
    assert_eq!(
        bounds(visual, "home-tile-runs").top(),
        bounds(visual, "home-tile-inbox").top()
    );
}
