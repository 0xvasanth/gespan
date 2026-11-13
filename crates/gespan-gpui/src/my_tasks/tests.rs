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
        agent: None,
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

#[gpui::test]
fn a_row_names_its_agent_after_its_kind_and_a_flow_step_does_not(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let mut asked = task("a", Kind::Approval, "/work/web", 5);
    asked.agent = Some("Claude Code".into());
    show(
        window,
        &mut visual,
        vec![asked, task("f", Kind::FlowApproval, "/work/web", 3)],
    );
    let pill = visual.debug_bounds("my-tasks-agent-a").expect("agent pill");
    let row = visual.debug_bounds("my-tasks-row-a").unwrap();
    assert!(row.contains(&pill.origin), "the pill sits in its own row");
    assert!(pill.size.height <= px(22.));
    // A Flow step's kind already says Flow: no second pill.
    assert!(visual.debug_bounds("my-tasks-agent-f").is_none());
}

#[test]
fn a_pilot_choice_keeps_its_word_but_wears_the_question_mark() {
    assert_eq!(super::list::kind_status(Kind::PilotChoice).0, "Choice");
    assert_eq!(
        super::list::kind_icon(Kind::PilotChoice),
        super::list::kind_icon(Kind::Question)
    );
    assert_ne!(
        super::list::kind_icon(Kind::Approval),
        super::list::kind_icon(Kind::FlowApproval)
    );
}

#[gpui::test]
fn a_draft_review_is_answered_from_its_card(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let mut draft = task("d", Kind::DraftReview, "/work/web", 5);
    draft.source = Source::DraftReview {
        run: "run-7".into(),
        url: "https://github.com/o/r/pull/1".into(),
    };
    draft.body = Body::DraftReview {
        body: "Looks close; two nits.".into(),
        comments: Vec::new(),
        outside: 0,
    };
    show(window, &mut visual, vec![draft]);
    assert_eq!(shown(&mut visual, &["d"]), ["d"], "the draft has a row");
    assert_eq!(list::kind_status(Kind::DraftReview).0, "Review");

    let decided = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = decided.clone();
    let _sub = window
        .update(&mut visual, |_, _, cx| {
            cx.subscribe(&cx.entity(), move |_, _, event: &Event, _| {
                if let Event::DraftDecide { run, submit } = event {
                    seen.borrow_mut().push((run.clone(), *submit));
                }
            })
        })
        .unwrap();
    click(&mut visual, "my-tasks-draft-submit");
    click(&mut visual, "my-tasks-draft-keep");
    assert_eq!(
        *decided.borrow(),
        [("run-7".to_owned(), true), ("run-7".to_owned(), false)],
        "Submit submits and Keep as draft keeps it, for the run that left it"
    );
}

/// The bounds of `selector` as (left, right, width).
fn span(visual: &mut VisualTestContext, selector: &str) -> (f32, f32, f32) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"));
    let left = f32::from(bounds.origin.x);
    let width = f32::from(bounds.size.width);
    (left, left + width, width)
}

/// Resizes the page and lets the width probe settle: the probe measures one
/// frame and the compact bar follows on the next.
fn resize(visual: &mut VisualTestContext, width: f32) {
    visual.simulate_resize(size(px(width), px(800.)));
    for _ in 0..2 {
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
}

#[test]
fn the_queue_keeps_the_design_width_and_narrows_on_small_pages() {
    use super::list::{LIST_MIN_WIDTH, LIST_WIDTH, queue_width};
    assert_eq!(queue_width(1128.), LIST_WIDTH, "the design's page");
    assert_eq!(queue_width(2400.), LIST_WIDTH, "a wide page keeps 360");
    assert_eq!(queue_width(1000.), LIST_WIDTH, "360 holds down to 1000");
    assert!((queue_width(900.) - 324.).abs() < 0.01, "then narrows");
    assert_eq!(queue_width(700.), LIST_MIN_WIDTH, "never under 300");
    assert_eq!(queue_width(400.), LIST_MIN_WIDTH);
}

#[test]
fn only_a_narrow_pane_folds_the_top_bar() {
    use super::pane::compact_bar;
    // 1100pt window less the sidebar: the pane is ~580pt.
    assert!(compact_bar(912.));
    assert!(
        super::list::compact_tabs(830.),
        "and the queue drops counts"
    );
    assert!(!super::list::compact_tabs(1128.));
    assert!(compact_bar(830.));
    assert!(compact_bar(560.), "the smallest window");
    // The design's 1440pt window: the pane is 768pt.
    assert!(!compact_bar(1128.));
    assert!(!compact_bar(2300.));
}

#[gpui::test]
fn the_queue_and_pane_share_the_page_without_overlap_at_every_width(cx: &mut TestAppContext) {
    use super::list::queue_width;
    use super::pane::PANE_COLUMN;
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::FlowApproval, "/work/web", 90),
            task("b", Kind::FlowApproval, "/work/api", 5),
        ],
    );
    for width in [640., 900., 1128., 2400.] {
        resize(&mut visual, width);
        let (queue_left, queue_right, queue) = span(&mut visual, "my-tasks-queue");
        assert_eq!(queue_left, 0., "{width}: the queue starts the page");
        assert!(
            (queue - queue_width(width)).abs() < 0.5,
            "{width}: the queue is {queue}, not {}",
            queue_width(width)
        );
        // The pane takes everything after the queue, to the window's edge.
        let (pane_left, pane_right, _) = span(&mut visual, "my-tasks-pane");
        assert!(
            (pane_left - queue_right).abs() < 0.5,
            "{width}: pane follows the queue"
        );
        assert!(
            (pane_right - width).abs() < 0.5,
            "{width}: pane fills the rest"
        );
        // The bar and the card stay in the pane, inside the centred column.
        let pane_mid = (pane_left + pane_right) / 2.;
        for selector in ["my-tasks-bar", "my-tasks-card"] {
            let (left, right, w) = span(&mut visual, selector);
            assert!(
                left >= pane_left - 0.5,
                "{width}: {selector} clears the queue"
            );
            assert!(
                right <= pane_right + 0.5,
                "{width}: {selector} fits the pane"
            );
            assert!(
                w <= PANE_COLUMN + 0.5,
                "{width}: {selector} is capped at {w}"
            );
            assert!(
                ((left + right) / 2. - pane_mid).abs() < 0.5,
                "{width}: {selector} is centred"
            );
        }
        // The tabs stay in the queue, however narrow it gets, and never
        // reach into the pane.
        let (_, tabs_right, _) = span(&mut visual, "my-tasks-tabs");
        assert!(
            tabs_right <= queue_right,
            "{width}: the tabs end at {tabs_right}, the queue at {queue_right}"
        );
        // Nothing in the bar runs past its end.
        let (_, bar_right, _) = span(&mut visual, "my-tasks-bar");
        for selector in ["my-tasks-waited", "my-tasks-next", "my-tasks-open-source"] {
            let (_, right, _) = span(&mut visual, selector);
            assert!(
                right <= bar_right + 0.5,
                "{width}: {selector} overflows the bar"
            );
        }
        let (_, _, open) = span(&mut visual, "my-tasks-open-source");
        let compact = super::pane::compact_bar(width);
        assert_eq!(
            open <= 30.5,
            compact,
            "{width}: Open folds only when narrow"
        );
    }
    // On the wide page the content is the capped column, not the pane.
    let (_, _, bar) = span(&mut visual, "my-tasks-bar");
    assert_eq!(bar, PANE_COLUMN);
}

#[gpui::test]
fn a_long_queue_lays_out_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let tasks: Vec<TaskItem> = (0..300)
        .map(|i| task(&format!("t{i}"), Kind::Question, "/work/web", 300 - i))
        .collect();
    show(window, &mut visual, tasks);
    let queue = window
        .read_with(&visual, |view, _| view.queue().len())
        .unwrap();
    // Two age groups' labels and every task.
    assert_eq!(queue, 302);
    let laid_out = (0..300)
        .filter(|i| {
            let selector: &'static str = Box::leak(format!("my-tasks-row-t{i}").into_boxed_str());
            visual.debug_bounds(selector).is_some()
        })
        .count();
    assert!(laid_out > 0, "the first rows are drawn");
    assert!(
        laid_out < 40,
        "{laid_out} rows laid out for an 800pt window"
    );
}

#[gpui::test]
fn stepping_to_a_row_off_screen_scrolls_it_into_view(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let tasks: Vec<TaskItem> = (0..60)
        .map(|i| task(&format!("t{i}"), Kind::Question, "/work/web", 100 - i))
        .collect();
    show(window, &mut visual, tasks);
    // The list opens at the top: the first group and the selected row show.
    assert!(visual.debug_bounds("my-tasks-group-1h").is_some());
    assert!(visual.debug_bounds("my-tasks-row-t0").is_some());
    assert!(visual.debug_bounds("my-tasks-row-t59").is_none());
    // Stepping to the next row, already on screen, leaves the list put.
    click(&mut visual, "my-tasks-next");
    assert_eq!(selected(window, &mut visual).as_deref(), Some("t1"));
    assert!(visual.debug_bounds("my-tasks-group-1h").is_some());
    click(&mut visual, "my-tasks-prev");
    // Back from the first task wraps to the last, at the bottom of the list.
    click(&mut visual, "my-tasks-prev");
    visual.refresh().unwrap();
    assert_eq!(selected(window, &mut visual).as_deref(), Some("t59"));
    let row = visual
        .debug_bounds("my-tasks-row-t59")
        .expect("the selected row is drawn");
    assert!(f32::from(row.bottom()) <= 800.5, "and inside the window");
}

#[test]
fn a_wait_only_changes_on_screen_when_the_minute_does() {
    use super::shown_waits_changed;
    assert!(!shown_waits_changed(NOW, NOW + 59_999));
    assert!(shown_waits_changed(NOW, NOW + MINUTE));
    assert!(!shown_waits_changed(NOW + 1, NOW + 2));
}

#[gpui::test]
fn an_unchanged_snapshot_does_not_redraw_the_page(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    let tasks = vec![
        task("a", Kind::Question, "/work/web", 5),
        task("b", Kind::Approval, "/work/api", 90),
    ];
    show(window, &mut visual, tasks.clone());
    // Every notify asks the window to draw the page again.
    let redraws = std::rc::Rc::new(std::cell::Cell::new(0));
    let view = window.update(&mut visual, |_, _, cx| cx.entity()).unwrap();
    let _counting = visual.update(|_, cx| {
        let redraws = redraws.clone();
        cx.observe(&view, move |_, _| redraws.set(redraws.get() + 1))
    });
    let set = |visual: &mut VisualTestContext, tasks: Vec<TaskItem>, now: u64| {
        window
            .update(visual, |view, _, cx| view.set_tasks(tasks, now, cx))
            .unwrap();
        visual.run_until_parked();
    };
    // A session streamed a token: the same tasks, a few seconds later.
    for seconds in [1, 5, 30] {
        set(&mut visual, tasks.clone(), NOW + seconds * 1_000);
    }
    assert_eq!(redraws.get(), 0, "nothing changed on screen");
    // A minute on, every wait reads one minute longer.
    set(&mut visual, tasks.clone(), NOW + MINUTE);
    assert!(redraws.get() > 0, "the new minute is drawn");
    // A task's headline changes within that minute: drawn too.
    let before = redraws.get();
    let mut changed = tasks;
    changed[0].headline = "Which branch?".into();
    set(&mut visual, changed, NOW + MINUTE + 1_000);
    assert!(redraws.get() > before, "the new headline is drawn");
}

#[gpui::test]
fn an_empty_queue_keeps_its_shape_with_placeholder_rows(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(window, &mut visual, vec![]);
    let (queue_left, queue_right, _) = span(&mut visual, "my-tasks-queue");
    let search = visual
        .debug_bounds("my-tasks-search")
        .expect("search field");
    // Three placeholders, in order down the column, under the search field
    // and inside the queue, each as tall as a real row.
    let mut above = f32::from(search.bottom());
    for ix in 0..3 {
        let selector: &'static str = Box::leak(format!("my-tasks-ghost-{ix}").into_boxed_str());
        let ghost = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        assert!(
            f32::from(ghost.top()) >= above,
            "{selector} sits below what is above it"
        );
        assert!(f32::from(ghost.left()) >= queue_left && f32::from(ghost.right()) <= queue_right);
        assert!(
            (f32::from(ghost.size.height) + 4. - 97.).abs() < 1.,
            "{selector} is a row's height, not {}",
            ghost.size.height
        );
        above = f32::from(ghost.bottom());
    }
    // The pane says why; with nothing hidden there is nothing to clear.
    assert!(visual.debug_bounds("my-tasks-empty").is_some());
    assert!(visual.debug_bounds("my-tasks-clear-filters").is_none());
}

#[gpui::test]
fn a_search_that_hides_everything_can_be_cleared_from_the_pane(cx: &mut TestAppContext) {
    let (window, mut visual) = page(cx);
    show(
        window,
        &mut visual,
        vec![
            task("a", Kind::Question, "/work/web", 5),
            task("b", Kind::Approval, "/work/api", 90),
        ],
    );
    let search = window
        .read_with(&visual, |view, _| view.search.clone())
        .unwrap();
    search.update(&mut visual, |input, cx| {
        input.set_text("nothing like this".into(), cx)
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(shown(&mut visual, &["a", "b"]).is_empty());
    assert!(visual.debug_bounds("my-tasks-ghost-0").is_some());
    click(&mut visual, "my-tasks-clear-filters");
    // Both tasks are back, the first selected, and the search is empty.
    assert_eq!(shown(&mut visual, &["a", "b"]), ["a", "b"]);
    assert!(visual.debug_bounds("my-tasks-ghost-0").is_none());
    assert!(visual.debug_bounds("my-tasks-card").is_some());
    let query = search.read_with(&visual, |input, _| input.content.to_string());
    assert_eq!(query, "");
}
