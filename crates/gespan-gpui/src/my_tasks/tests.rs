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
fn rows_are_grouped_by_age_with_the_longest_wait_first(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 5),
            task("b", Kind::Approval, "/work/api", 90),
            task("c", Kind::FlowApproval, "/work/web", 600),
            task("d", Kind::Question, "/work/api", 20),
        ],
    );
    // The age groups follow the filter's WAITING choices, oldest first, and
    // each holds only the rows of its age, longest wait first.
    let order = [
        "my-tasks-group-8h",
        "my-tasks-row-c",
        "my-tasks-group-1h",
        "my-tasks-row-b",
        "my-tasks-group-recent",
        "my-tasks-row-d",
        "my-tasks-row-a",
    ];
    for pair in order.windows(2) {
        assert!(
            top(&mut visual, pair[0]) < top(&mut visual, pair[1]),
            "{} sits above {}",
            pair[0],
            pair[1]
        );
    }
    assert_eq!(selected(window, &mut visual).as_deref(), Some("c"));
}

#[gpui::test]
fn an_age_group_with_no_tasks_has_no_label(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 5),
            task("b", Kind::Approval, "/work/api", 90),
        ],
    );
    assert!(visual.debug_bounds("my-tasks-group-1h").is_some());
    assert!(visual.debug_bounds("my-tasks-group-recent").is_some());
    assert!(visual.debug_bounds("my-tasks-group-8h").is_none());
}

#[test]
fn ages_split_where_the_wait_filter_does() {
    use super::list::age_group;
    assert_eq!(age_group(59 * MINUTE).0, "recent");
    assert_eq!(age_group(60 * MINUTE).0, "1h");
    assert_eq!(age_group(8 * 60 * MINUTE - 1).0, "1h");
    assert_eq!(age_group(8 * 60 * MINUTE).0, "8h");
}

#[test]
fn the_pane_spells_the_wait_out_to_the_minute() {
    use super::list::waited_long;
    assert_eq!(waited_long(30_000), "just now");
    assert_eq!(waited_long(12 * MINUTE), "12m");
    assert_eq!(waited_long(120 * MINUTE), "2h");
    assert_eq!(waited_long(134 * MINUTE), "2h 14m");
    assert_eq!(waited_long(24 * 60 * MINUTE), "1d");
    assert_eq!(waited_long(27 * 60 * MINUTE + 59 * MINUTE), "1d 3h");
}

#[gpui::test]
fn the_waiting_segment_counts_every_task_even_when_filtered(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    assert!(visual.debug_bounds("my-tasks-count-waiting-0").is_some());
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 5),
            task("b", Kind::Approval, "/work/api", 90),
            task("c", Kind::FlowApproval, "/work/web", 600),
        ],
    );
    assert!(visual.debug_bounds("my-tasks-count-waiting-3").is_some());
    click(&mut visual, "my-tasks-filter");
    click(&mut visual, "my-tasks-filter-project-/work/api");
    // The count is what waits on you, not what the filter shows.
    assert!(visual.debug_bounds("my-tasks-count-waiting-3").is_some());
}

#[gpui::test]
fn the_top_bar_shows_the_wait_and_steps_through_the_tasks(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![task("a", Kind::FlowApproval, "/work/web", 134)],
    );
    assert!(visual.debug_bounds("my-tasks-waited").is_some());
    // One task has nowhere to step to.
    assert!(visual.debug_bounds("my-tasks-next").is_none());
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::FlowApproval, "/work/web", 134),
            task("b", Kind::FlowApproval, "/work/web", 5),
        ],
    );
    // The stepper sits in the bar, above the history, not on the card.
    let card = top(&mut visual, "my-tasks-card");
    assert!(top(&mut visual, "my-tasks-next") < top(&mut visual, "my-tasks-history"));
    assert!(top(&mut visual, "my-tasks-position") < card);
    click(&mut visual, "my-tasks-next");
    assert_eq!(selected(window, &mut visual).as_deref(), Some("b"));
}

#[gpui::test]
fn every_tab_shows_its_empty_state_when_it_has_nothing(cx: &mut TestAppContext) {
    let (_, mut visual) = page(cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    for tab in [
        "my-tasks-tab-waiting",
        "my-tasks-tab-answered",
        "my-tasks-tab-decisions",
    ] {
        click(&mut visual, tab);
        assert!(
            visual.debug_bounds("my-tasks-empty").is_some(),
            "{tab} shows its empty state"
        );
        assert!(visual.debug_bounds("my-tasks-card").is_none());
    }
}

#[test]
fn each_tab_says_why_it_is_empty() {
    use super::pane::empty_copy;
    let title = |tab, has_any| empty_copy(tab, has_any).1;
    assert_eq!(title(Tab::Waiting, false), "You're all caught up.");
    assert_eq!(title(Tab::Waiting, true), "No task is selected.");
    assert_eq!(title(Tab::Answered, false), "Nothing answered yet.");
    assert_eq!(
        title(Tab::Answered, true),
        "No answers match the search or filters."
    );
    assert_eq!(title(Tab::Decisions, false), "No decisions yet.");
    assert_eq!(
        title(Tab::Decisions, true),
        "No decisions match the search or filters."
    );
    // A first-run pane says what will land there; a filtered one says how
    // to get everything back.
    assert_eq!(
        empty_copy(Tab::Decisions, false).2,
        "Answer a question, then save it as a decision."
    );
    for tab in [Tab::Waiting, Tab::Answered, Tab::Decisions] {
        assert_ne!(empty_copy(tab, false).2, empty_copy(tab, true).2);
    }
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

#[test]
fn every_icon_the_page_uses_exists() {
    use gpui::AssetSource;
    let names = Kind::ALL
        .iter()
        .map(|kind| super::list::kind_icon(*kind))
        .chain(super::pane::ICONS.iter().copied());
    for name in names {
        assert!(
            crate::assets::Assets.load(name).unwrap().is_some(),
            "{name} is not an icon the app has"
        );
    }
}

#[gpui::test]
fn a_filter_that_hides_the_card_moves_it_and_clears_the_note(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("old", Kind::FlowApproval, "/work/web", 90),
            task("new", Kind::FlowApproval, "/work/api", 5),
        ],
    );
    let note = window
        .read_with(&visual, |view, _| view.note.clone())
        .unwrap();
    note.update(&mut visual, |input, cx| {
        input.set_text("ship it".into(), cx)
    });
    click(&mut visual, "my-tasks-filter");
    click(&mut visual, "my-tasks-filter-project-/work/web");
    assert_eq!(selected(window, &mut visual).as_deref(), Some("new"));
    // The note was written for the old run; it must not reach the new one.
    note.read_with(&visual, |input, _| assert_eq!(input.content.as_ref(), ""));
}

#[gpui::test]
fn scrolling_up_at_the_top_loads_one_page_per_gesture(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let mut long = task("a", Kind::FlowApproval, "/work/web", 5);
    long.history.earlier = 12;
    long.history.turns = vec![gespan_core::tasks::Turn::default(); 4];
    show(window, &mut visual, vec![long]);
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = loads.clone();
    let _sub = window
        .update(&mut visual, |_, _, cx| {
            cx.subscribe(&cx.entity(), move |_, _, event: &Event, _| {
                if matches!(event, Event::LoadEarlier(_)) {
                    counted.set(counted.get() + 1);
                }
            })
        })
        .unwrap();
    let history = visual.debug_bounds("my-tasks-history").unwrap();
    for _ in 0..30 {
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: history.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(400.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
    }
    visual.run_until_parked();
    assert_eq!(
        loads.get(),
        1,
        "one flick asks for one page, not all of them"
    );
}
