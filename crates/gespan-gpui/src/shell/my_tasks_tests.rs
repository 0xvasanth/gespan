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
