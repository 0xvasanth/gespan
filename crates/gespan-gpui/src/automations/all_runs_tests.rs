//! "All runs": the list's first row and default selection, whose pane is the
//! run feed. Picking an automation, New and Escape move between it, the
//! gallery and the editor; a failing automation is pinned above the feed.
use super::layout_tests::{Drawn, Page, click, open, page};
use super::*;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::{cell::RefCell, rc::Rc};

fn now() -> i64 {
    gespan_core::time::now_ms() as i64
}

fn save(page: &Page, name: &str, cwd: &str) -> Automation {
    let mut draft = AutomationUpsert::new(cwd.into(), "codex".into(), "gpt-5.5".into());
    draft.name = name.into();
    draft.prompt = format!("{name}: do the work");
    draft.triggers = Some(vec![AutomationTrigger::new("time", "daily")]);
    draft.sync_triggers(now());
    page.service.lock().unwrap().upsert(draft).unwrap()
}

/// A test run of `automation`, then left at `status`.
fn run(page: &Page, automation: &str, status: &str, error: Option<&str>) -> AutomationRun {
    let mut service = page.service.lock().unwrap();
    let run = service.run_now(automation, now()).unwrap();
    if status == "pending" {
        return run;
    }
    service
        .update_run(
            &run.id,
            status,
            Some(format!("session-{}", run.id)),
            error.map(Into::into),
            now(),
        )
        .unwrap()
}

fn reload(window: &WindowHandle<Automations>, visual: &mut VisualTestContext) {
    window
        .update(visual, |view, _, cx| view.refresh(cx))
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn showing_runs(window: &WindowHandle<Automations>, visual: &mut VisualTestContext) -> bool {
    window
        .read_with(visual, |view, _| view.showing_runs())
        .unwrap()
}

fn escape(visual: &mut VisualTestContext) {
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    visual.refresh().unwrap();
}

#[gpui::test]
fn all_runs_is_the_first_row_and_the_default_pane(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let first = run(&page, &automation.id, "succeeded", None);
    let (window, mut visual) = open(&page, "/work/app", cx);

    assert!(showing_runs(&window, &mut visual));
    let all = visual
        .drawn("automations-all-runs")
        .expect("All runs is listed");
    let card = visual
        .drawn(&format!("automation-card-{}", automation.id))
        .expect("the automation is listed");
    assert!(all.bottom() <= card.top(), "All runs leads the list");
    assert!(visual.drawn("runs-panel").is_some(), "the feed is the pane");
    assert!(visual.drawn(&format!("runs-row-{}", first.id)).is_some());
    assert!(
        visual.drawn("automation-scratch").is_none(),
        "the gallery waits for New"
    );
}

#[gpui::test]
fn with_nothing_set_up_the_gallery_shows_and_all_runs_is_not_listed(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open(&page, "/work/app", cx);
    assert!(!showing_runs(&window, &mut visual));
    assert!(visual.drawn("automations-all-runs").is_none());
    assert!(visual.drawn("automations-none").is_some());
    assert!(visual.drawn("automation-scratch").is_some());

    // The first automation brings All runs, and its feed, back.
    save(&page, "Nightly triage", "/work/app");
    reload(&window, &mut visual);
    assert!(showing_runs(&window, &mut visual));
    assert!(visual.drawn("automations-all-runs").is_some());
    assert!(visual.drawn("automations-none").is_none());
}

#[gpui::test]
fn picking_an_automation_opens_its_editor_and_all_runs_brings_the_feed_back(
    cx: &mut TestAppContext,
) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let (window, mut visual) = open(&page, "/work/app", cx);

    click(&mut visual, &format!("automation-card-{}", automation.id));
    assert!(!showing_runs(&window, &mut visual));
    assert!(window.read_with(&visual, |view, _| view.editing()).unwrap());
    assert!(visual.drawn("runs-panel").is_none());

    click(&mut visual, "automations-all-runs");
    assert!(showing_runs(&window, &mut visual));
    let (editing, selected) = window
        .read_with(&visual, |view, _| (view.editing(), view.selected.clone()))
        .unwrap();
    assert!(!editing, "the editor closed");
    assert_eq!(selected, None);
    assert!(visual.drawn("runs-panel").is_some());
}

#[gpui::test]
fn new_opens_the_gallery_then_the_editor_with_a_draft_card(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let (window, mut visual) = open(&page, "/work/app", cx);

    click(&mut visual, "new-automation");
    assert!(!showing_runs(&window, &mut visual));
    assert!(visual.drawn("automation-scratch").is_some(), "the gallery");
    assert!(
        visual.drawn("automation-draft-card").is_none(),
        "nothing is drafted until an example or scratch is picked"
    );

    click(&mut visual, "automation-scratch");
    assert!(window.read_with(&visual, |view, _| view.editing()).unwrap());
    let draft = visual
        .drawn("automation-draft-card")
        .expect("the new automation is listed while it is drafted");
    // It sits in its project's group, ahead of the saved automation.
    let saved = visual
        .drawn(&format!("automation-card-{}", automation.id))
        .unwrap();
    assert!(draft.bottom() <= saved.top());

    // Escape drops the draft and goes back to All runs.
    window
        .update(&mut visual, |view, window, cx| {
            window.focus(&view.focus, cx)
        })
        .unwrap();
    escape(&mut visual);
    assert!(showing_runs(&window, &mut visual));
    assert!(visual.drawn("automation-draft-card").is_none());
}

#[gpui::test]
fn a_draft_in_a_project_with_no_automations_starts_its_own_group(cx: &mut TestAppContext) {
    let page = page();
    save(&page, "Nightly triage", "/work/other");
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, "new-automation");
    click(&mut visual, "automation-scratch");
    assert!(visual.drawn("automation-draft-card").is_some());
    let groups = (0..3)
        .filter(|index| visual.drawn(&format!("automation-group-{index}")).is_some())
        .count();
    assert_eq!(groups, 2, "the draft's project and the saved one's");
    let cwd = window
        .read_with(&visual, |view, _| {
            view.editor.as_ref().unwrap().draft.cwd.clone()
        })
        .unwrap();
    assert_eq!(cwd, "/work/app");
}

#[gpui::test]
fn escape_steps_back_to_all_runs_and_then_closes(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let (window, mut visual) = open(&page, "/work/app", cx);
    let closed = Rc::new(RefCell::new(0));
    let sink = closed.clone();
    let _subscription = window
        .update(&mut visual, |_, _, cx| {
            cx.subscribe(&cx.entity(), move |_, _, event: &Event, _| {
                if matches!(event, Event::Close) {
                    *sink.borrow_mut() += 1;
                }
            })
        })
        .unwrap();
    let focus = |visual: &mut VisualTestContext| {
        window
            .update(visual, |view, window, cx| window.focus(&view.focus, cx))
            .unwrap();
    };

    click(&mut visual, &format!("automation-card-{}", automation.id));
    focus(&mut visual);
    escape(&mut visual);
    assert!(showing_runs(&window, &mut visual), "the editor gives way");
    assert_eq!(*closed.borrow(), 0);

    click(&mut visual, "new-automation");
    focus(&mut visual);
    escape(&mut visual);
    assert!(showing_runs(&window, &mut visual), "the gallery gives way");
    assert_eq!(*closed.borrow(), 0);

    focus(&mut visual);
    escape(&mut visual);
    assert_eq!(*closed.borrow(), 1, "All runs closes the page");
}

#[gpui::test]
fn see_all_runs_shows_all_runs_for_that_automation_only(cx: &mut TestAppContext) {
    let page = page();
    let first = save(&page, "Nightly triage", "/work/app");
    let second = save(&page, "Find bugs", "/work/app");
    let mine = run(&page, &first.id, "succeeded", None);
    let other = run(&page, &second.id, "succeeded", None);
    let (window, mut visual) = open(&page, "/work/app", cx);

    click(&mut visual, &format!("automation-card-{}", first.id));
    assert!(
        visual.drawn("automation-see-all-runs").is_none(),
        "only the Recent runs tab links to All runs"
    );
    click(&mut visual, "automation-history-tab");
    click(&mut visual, "automation-see-all-runs");
    assert!(showing_runs(&window, &mut visual));
    let (filter, rows) = window
        .read_with(&visual, |view, cx| {
            let feed = view.feed().read(cx);
            (feed.filter().clone(), feed.rows())
        })
        .unwrap();
    assert_eq!(filter.automation.as_deref(), Some(first.id.as_str()));
    assert_eq!(filter.automation_name.as_deref(), Some("Nightly triage"));
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![mine.id.as_str()]
    );

    // All runs, chosen in the list, is every automation's again.
    click(&mut visual, "automations-all-runs");
    let ids = window
        .read_with(&visual, |view, cx| {
            let feed = view.feed().read(cx);
            assert_eq!(feed.filter().automation, None);
            feed.rows()
                .into_iter()
                .map(|row| row.id)
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert!(ids.contains(&mine.id) && ids.contains(&other.id), "{ids:?}");
}

#[gpui::test]
fn a_failing_automation_is_pinned_with_its_error_and_ways_to_fix_it(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let failed = run(
        &page,
        &automation.id,
        "failed",
        Some("HTTP 401: Bad credentials"),
    );
    let (window, mut visual) = open(&page, "/work/app", cx);
    let launched = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = launched.clone();
    let _subscription = window
        .update(&mut visual, |_, _, cx| {
            cx.subscribe(&cx.entity(), move |_, _, event: &Event, _| {
                if let Event::Launch(run) = event {
                    sink.borrow_mut().push(run.automation.id.clone());
                }
            })
        })
        .unwrap();

    let card = format!("runs-failing-{}", failed.id);
    assert!(visual.drawn(&card).is_some(), "the failure is pinned");
    let (running, failing, today) = window.read_with(&visual, |view, _| view.totals).unwrap();
    assert_eq!((running, failing, today), (0, 1, 1));
    assert!(
        window
            .read_with(&visual, |view, _| view.failing.contains(&automation.id))
            .unwrap(),
        "its card in the list is edged rose"
    );

    // Run now runs that automation.
    click(&mut visual, &format!("runs-failing-run-{}", failed.id));
    assert_eq!(*launched.borrow(), vec![automation.id.clone()]);

    // Edit automation opens its editor.
    reload(&window, &mut visual);
    click(&mut visual, &format!("runs-failing-edit-{}", failed.id));
    let selected = window
        .read_with(&visual, |view, _| view.selected.clone())
        .unwrap();
    assert_eq!(selected.as_deref(), Some(automation.id.as_str()));
    assert!(!showing_runs(&window, &mut visual));

    // A later success unpins it.
    let mut service = page.service.lock().unwrap();
    let again = service.run_now(&automation.id, now() + 1).unwrap();
    service
        .update_run(&again.id, "succeeded", None, None, now() + 2)
        .unwrap();
    drop(service);
    click(&mut visual, "automations-all-runs");
    reload(&window, &mut visual);
    assert!(visual.drawn(&card).is_none(), "fixed, so no longer pinned");
    let failing = window.read_with(&visual, |view, _| view.totals.1).unwrap();
    assert_eq!(failing, 0);
}

#[gpui::test]
fn running_runs_sit_under_running_now_above_the_days(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let done = run(&page, &automation.id, "succeeded", None);
    let going = run(&page, &automation.id, "running", None);
    let queued = run(&page, &automation.id, "pending", None);
    let (window, mut visual) = open(&page, "/work/app", cx);

    let section = visual
        .drawn("runs-running")
        .expect("Running now is drawn while runs are going");
    let day = visual.drawn("runs-day-0").expect("today's card");
    assert!(section.bottom() <= day.top(), "Running now leads");
    let inside = |visual: &mut VisualTestContext, card: gpui::Bounds<gpui::Pixels>, id: &str| {
        let row = visual.drawn(&format!("runs-row-{id}")).unwrap();
        card.top() <= row.top() && row.bottom() <= card.bottom()
    };
    assert!(inside(&mut visual, section, &going.id));
    assert!(inside(&mut visual, section, &queued.id));
    assert!(inside(&mut visual, day, &done.id));
    assert!(!inside(&mut visual, day, &going.id), "listed once");

    // The list's All runs row says so.
    let running = window.read_with(&visual, |view, _| view.totals.0).unwrap();
    assert_eq!(running, 2, "queued counts as running");
}

#[gpui::test]
fn recent_runs_draws_the_feeds_rows(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(&page, "Nightly triage", "/work/app");
    let queued = run(&page, &automation.id, "pending", None);
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));
    click(&mut visual, "automation-history-tab");
    assert!(
        visual.drawn(&format!("runs-row-{}", queued.id)).is_some(),
        "the same row the feed draws"
    );
    let rows = window
        .read_with(&visual, |view, _| view.history_rows())
        .unwrap();
    assert_eq!(rows.len(), 1);
    // One word for a claimed run, the same trigger label, no project or
    // automation repeated on its own page.
    assert_eq!(rows[0].status, "Queued");
    assert_eq!(rows[0].trigger, "Test run");
    assert_eq!(rows[0].meta(false, false), "");
    assert_eq!(rows[0].title().1, "Nightly triage");
}
