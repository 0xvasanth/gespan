//! The My tasks page on its own: the list, search and filter.
use super::*;
use gespan_core::tasks::History;
use gpui::{TestAppContext, VisualTestContext, WindowHandle, px, size};

const MINUTE: u64 = 60_000;
/// The clock the page is given; tasks are dated back from it.
const NOW: u64 = 1_000 * MINUTE;

pub(super) fn task(key: &str, kind: Kind, project: &str, waited_min: u64) -> TaskItem {
    TaskItem {
        key: key.into(),
        kind,
        project: project.into(),
        headline: format!("Task {key}"),
        who: "Builder asks".into(),
        source_label: format!("Session {key}"),
        source: Source::FlowRun(key.into()),
        body: Body::Flow {
            prompt: format!("Prompt {key}"),
        },
        since_ms: NOW - waited_min * MINUTE,
        history: History::default(),
    }
}

pub(super) fn page(cx: &mut TestAppContext) -> (WindowHandle<MyTasks>, VisualTestContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| cx.new(MyTasks::new))
            .unwrap()
    });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    (window, visual)
}

pub(super) fn show(
    window: WindowHandle<MyTasks>,
    visual: &mut VisualTestContext,
    tasks: Vec<TaskItem>,
) {
    window
        .update(visual, |view, _, cx| view.set_tasks(tasks, NOW, cx))
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
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

fn top(visual: &mut VisualTestContext, selector: &str) -> f32 {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    f32::from(
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"))
            .origin
            .y,
    )
}

fn shown(visual: &mut VisualTestContext, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter(|key| {
            let selector: &'static str = Box::leak(format!("my-tasks-row-{key}").into_boxed_str());
            visual.debug_bounds(selector).is_some()
        })
        .map(|key| key.to_string())
        .collect()
}

fn selected(window: WindowHandle<MyTasks>, visual: &mut VisualTestContext) -> Option<String> {
    window
        .read_with(visual, |view, _| view.selected().map(|t| t.key.clone()))
        .unwrap()
}

#[gpui::test]
fn rows_are_grouped_by_project_with_the_longest_wait_first(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 5),
            task("b", Kind::Approval, "/work/api", 90),
            task("c", Kind::FlowApproval, "/work/web", 30),
        ],
    );
    // The project waiting longest leads; inside it, the longest wait first.
    assert!(
        top(&mut visual, "my-tasks-group-/work/api") < top(&mut visual, "my-tasks-group-/work/web")
    );
    assert!(top(&mut visual, "my-tasks-row-b") < top(&mut visual, "my-tasks-group-/work/web"));
    assert!(top(&mut visual, "my-tasks-row-c") < top(&mut visual, "my-tasks-row-a"));
    assert_eq!(selected(window, &mut visual).as_deref(), Some("b"));
}

#[gpui::test]
fn the_filter_hides_projects_kinds_and_short_waits_and_marks_itself_on(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 5),
            task("b", Kind::Approval, "/work/api", 90),
            task("c", Kind::FlowApproval, "/work/web", 600),
        ],
    );
    let keys = ["a", "b", "c"];
    assert!(visual.debug_bounds("my-tasks-filter-on").is_none());
    click(&mut visual, "my-tasks-filter");
    click(&mut visual, "my-tasks-filter-kind-Flow approvals");
    assert_eq!(shown(&mut visual, &keys), ["a", "b"]);
    assert!(visual.debug_bounds("my-tasks-filter-on").is_some());
    click(&mut visual, "my-tasks-filter-project-/work/api");
    assert_eq!(shown(&mut visual, &keys), ["a"]);
    click(&mut visual, "my-tasks-filter-clear");
    assert_eq!(shown(&mut visual, &keys), keys);
    click(&mut visual, "my-tasks-filter");
    click(&mut visual, "my-tasks-filter-wait-1h");
    assert_eq!(shown(&mut visual, &keys), ["b", "c"]);
    click(&mut visual, "my-tasks-filter-wait-8h");
    assert_eq!(shown(&mut visual, &keys), ["c"]);
}

#[gpui::test]
fn search_matches_headline_and_source(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let mut fsid = task("a", Kind::Question, "/work/web", 5);
    fsid.headline = "How should fsidCount be kept?".into();
    let mut script = task("b", Kind::Approval, "/work/web", 6);
    script.source_label = "Count to 3000 in words".into();
    show(window, &mut visual, vec![fsid, script]);
    let search = window
        .read_with(&visual, |view, _| view.search.clone())
        .unwrap();
    search.update(&mut visual, |input, cx| input.set_text("FSID".into(), cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(shown(&mut visual, &["a", "b"]), ["a"]);
    search.update(&mut visual, |input, cx| input.set_text("3000".into(), cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(shown(&mut visual, &["a", "b"]), ["b"]);
}

#[gpui::test]
fn a_filter_that_hides_the_selection_selects_the_first_visible_task(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("old", Kind::FlowApproval, "/work/web", 90),
            task("new", Kind::Question, "/work/web", 5),
        ],
    );
    assert_eq!(selected(window, &mut visual).as_deref(), Some("old"));
    click(&mut visual, "my-tasks-filter");
    click(&mut visual, "my-tasks-filter-kind-Flow approvals");
    assert_eq!(selected(window, &mut visual).as_deref(), Some("new"));
}

#[gpui::test]
fn clicking_a_row_selects_it(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 90),
            task("b", Kind::Question, "/work/web", 5),
        ],
    );
    click(&mut visual, "my-tasks-row-b");
    assert_eq!(selected(window, &mut visual).as_deref(), Some("b"));
}
