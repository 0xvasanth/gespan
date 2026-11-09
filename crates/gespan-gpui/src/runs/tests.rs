use super::*;
use gespan_core::automations::{AutomationTrigger, AutomationUpsert};
use gpui::TestAppContext;
use std::{cell::RefCell, path::Path, rc::Rc};

const HOUR: i64 = 3_600_000;

fn service() -> Arc<Mutex<Service>> {
    Arc::new(Mutex::new(Service::open(Path::new(":memory:")).unwrap()))
}

/// A project path that exists nowhere; only its last component is shown.
fn project(name: &str) -> String {
    std::env::temp_dir()
        .join("gespan-runs-tests")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn save(service: &Arc<Mutex<Service>>, cwd: &str, name: &str) -> String {
    let mut draft = AutomationUpsert::new(cwd.into(), "codex".into(), String::new());
    draft.name = name.into();
    draft.prompt = "Check the build".into();
    draft.workspace_mode = "current".into();
    draft.triggers = Some(vec![AutomationTrigger::new("time", "hourly")]);
    draft.sync_triggers(now_ms());
    service.lock().unwrap().upsert(draft).unwrap().id
}

fn run_now(service: &Arc<Mutex<Service>>, automation: &str, at: i64) -> String {
    service.lock().unwrap().run_now(automation, at).unwrap().id
}

fn now_ms() -> i64 {
    gespan_core::time::now_ms() as i64
}

fn run(id: &str, automation: &str, status: &str, created_at: i64) -> AutomationRun {
    AutomationRun {
        id: id.into(),
        automation_id: automation.into(),
        trigger: "manual".into(),
        scheduled_for: created_at,
        created_at,
        started_at: None,
        completed_at: None,
        status: status.into(),
        session_id: None,
        error: None,
        event_key: None,
        event_kind: None,
        event: None,
        event_title: None,
        prompt: None,
    }
}

/// A saved automation as `Service::list` returns it, under the id `id`.
fn automation(id: &str, name: &str, cwd: &str) -> Automation {
    let service = service();
    save(&service, cwd, name);
    let mut automation = service.lock().unwrap().list().unwrap().remove(0);
    automation.id = id.into();
    automation
}

fn ids(rows: &[RunRow]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

#[test]
fn status_chip_maps_every_status() {
    for (status, chip) in [
        ("pending", "Queued"),
        ("running", "Running"),
        ("succeeded", "Succeeded"),
        ("failed", "Failed"),
        ("cancelled", "Cancelled"),
        ("skipped", "Skipped"),
    ] {
        assert_eq!(status_chip(&run("r", "a", status, 0)), chip, "{status}");
    }
}

#[test]
fn counts_include_queued_as_running() {
    let today = 10 * 24 * HOUR;
    let runs = [
        run("1", "a", "pending", today + HOUR),
        run("2", "a", "running", today + 2 * HOUR),
        run("3", "a", "failed", today + 3 * HOUR),
        run("4", "a", "succeeded", today),
        // Yesterday's failure is not a failure today.
        run("5", "a", "failed", today - 1),
        // A run still going since yesterday is running, not today's.
        run("6", "a", "running", today - HOUR),
    ];
    assert_eq!(counts(&runs, today), (3, 4, 1));
    // A status this build does not know shows as Queued, so it counts as
    // running too.
    let unknown = run("7", "a", "waiting", today + HOUR);
    assert_eq!(status_chip(&unknown), "Queued");
    assert_eq!(counts(&[unknown], today), (1, 1, 0));
    assert_eq!(counts(&[], today), (0, 0, 0));
}

#[test]
fn day_label_names_today_and_yesterday_and_dates_older_days() {
    let now = chrono::Local::now();
    let today = local_day_start(now.timestamp_millis());
    assert_eq!(day_label(today + HOUR, today), "Today");
    assert_eq!(day_label(today, today), "Today");
    assert_eq!(day_label(today - 1, today), "Yesterday");
    let older = day_label(today - 3 * 24 * HOUR, today);
    assert_ne!(older, "Yesterday");
    assert_ne!(older, "Today");
}

#[gpui::test]
fn runs_lists_runs_across_projects_newest_first(cx: &mut TestAppContext) {
    let service = service();
    let alpha = save(&service, &project("alpha"), "Triage");
    let beta = save(&service, &project("beta"), "Release notes");
    let now = now_ms();
    let first = run_now(&service, &alpha, now - 2 * HOUR);
    let second = run_now(&service, &beta, now - HOUR);
    let third = run_now(&service, &alpha, now);
    let view = cx.new(|cx| Runs::new(service.clone(), cx));
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        let rows = view.rows();
        assert_eq!(ids(&rows), vec![third.as_str(), &second, &first]);
        assert_eq!(rows[0].automation, "Triage");
        assert_eq!(rows[0].project.as_deref(), Some(project("alpha").as_str()));
        assert_eq!(rows[1].automation, "Release notes");
        assert_eq!(rows[1].project.as_deref(), Some(project("beta").as_str()));
        // A run just claimed is listed as Queued.
        assert!(rows.iter().all(|row| row.status == "Queued"));
        assert_eq!(view.projects(), vec![project("alpha"), project("beta")]);
    });
    // A new run shows on the next refresh.
    let fourth = run_now(&service, &beta, now + 1);
    view.update(cx, |view, cx| view.refresh(cx));
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.rows()[0].id, fourth);
        assert_eq!(view.rows().len(), 4);
    });
}

#[gpui::test]
fn project_filter_limits_rows(cx: &mut TestAppContext) {
    let service = service();
    let alpha = save(&service, &project("alpha"), "Triage");
    let beta = save(&service, &project("beta"), "Release notes");
    let now = now_ms();
    let a1 = run_now(&service, &alpha, now - 2 * HOUR);
    let b1 = run_now(&service, &beta, now - HOUR);
    let a2 = run_now(&service, &alpha, now);
    service
        .lock()
        .unwrap()
        .update_run(&a2, "failed", None, Some("boom".into()), now)
        .unwrap();
    service
        .lock()
        .unwrap()
        .update_run(&a1, "running", None, None, now)
        .unwrap();
    let view = cx.new(|cx| Runs::new(service.clone(), cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.set_filter(
            RunsFilter {
                project: Some(project("beta")),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(ids(&view.rows()), vec![b1.as_str()]);
        // Every project that has runs still offers its chip.
        assert_eq!(view.projects().len(), 2);

        view.set_filter(
            RunsFilter {
                project: Some(project("alpha")),
                status: Some("failed"),
                ..Default::default()
            },
            cx,
        );
        let rows = view.rows();
        assert_eq!(ids(&rows), vec![a2.as_str()]);
        assert_eq!(rows[0].error.as_deref(), Some("boom"));

        // Queued counts as running, so the Running chip keeps queued runs.
        view.set_filter(
            RunsFilter {
                status: Some("running"),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(ids(&view.rows()), vec![b1.as_str(), &a1]);

        view.set_filter(
            RunsFilter {
                automation: Some(alpha.clone()),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(ids(&view.rows()), vec![a2.as_str(), &a1]);

        view.set_filter(RunsFilter::default(), cx);
        assert_eq!(view.rows().len(), 3);
    });
}

#[gpui::test]
fn runs_whose_automation_is_missing_show_deleted_automation(cx: &mut TestAppContext) {
    let view = cx.new(|cx| Runs::new(service(), cx));
    cx.run_until_parked();
    let cwd = project("alpha");
    let mut kept = run("kept", "present", "succeeded", 2_000);
    kept.session_id = Some("11".into());
    let orphan = run("orphan", "gone", "succeeded", 1_000);
    view.update(cx, |view, cx| {
        view.apply(
            vec![kept, orphan],
            vec![automation("present", "Triage", &cwd)],
            cx,
        );
        let rows = view.rows();
        assert_eq!(ids(&rows), vec!["kept", "orphan"]);
        assert_eq!(rows[0].automation, "Triage");
        assert!(rows[0].can_open());
        assert_eq!(rows[1].automation, "Deleted automation");
        assert_eq!(rows[1].project, None);
        assert!(!rows[1].can_open(), "a run without a session cannot open");
        // A project filter has nothing to match a missing automation on.
        view.set_filter(
            RunsFilter {
                project: Some(cwd.clone()),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(ids(&view.rows()), vec!["kept"]);
    });
}

#[gpui::test]
fn open_thread_emits_open_session(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| Runs::new(service(), cx));
    cx.run_until_parked();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let seen = opened.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &Event, _| match event {
            Event::OpenSession(id) => seen.borrow_mut().push(id.clone()),
        })
    });
    let mut with_session = run("with", "gone", "succeeded", 2_000);
    with_session.session_id = Some("42".into());
    let without = run("without", "gone", "skipped", 1_000);
    view.update(cx, |view, cx| {
        view.apply(vec![with_session, without], vec![], cx)
    });
    cx.run_until_parked();

    // Clicking "Open thread" on a run with a session opens it.
    let open = cx
        .debug_bounds("runs-open-with")
        .expect("the row offers Open thread");
    cx.simulate_click(open.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(*opened.borrow(), vec!["42".to_string()]);

    // Without a session the button is drawn but does nothing.
    let disabled = cx
        .debug_bounds("runs-open-without")
        .expect("the button stays in place, disabled");
    cx.simulate_click(disabled.center(), Default::default());
    cx.run_until_parked();
    view.update(cx, |view, cx| view.open_thread("without", cx));
    assert_eq!(*opened.borrow(), vec!["42".to_string()]);
}

#[gpui::test]
fn runs_shown_as_queued_stay_under_the_running_chip(cx: &mut TestAppContext) {
    let view = cx.new(|cx| Runs::new(service(), cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.apply(
            vec![
                run("odd", "a", "waiting", 3_000),
                run("queued", "a", "pending", 2_000),
                run("done", "a", "succeeded", 1_000),
            ],
            vec![],
            cx,
        );
        view.set_filter(
            RunsFilter {
                status: Some("running"),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(ids(&view.rows()), vec!["odd", "queued"]);
    });
}

#[gpui::test]
fn one_project_spelled_two_ways_gets_one_chip(cx: &mut TestAppContext) {
    let view = cx.new(|cx| Runs::new(service(), cx));
    cx.run_until_parked();
    let alpha = project("alpha");
    view.update(cx, |view, cx| {
        view.apply(
            vec![
                run("1", "one", "succeeded", 2_000),
                run("2", "two", "failed", 1_000),
            ],
            vec![
                automation("one", "Triage", &alpha),
                automation("two", "Release notes", &format!("{alpha}/")),
            ],
            cx,
        );
        assert_eq!(view.projects(), vec![alpha.clone()]);
        // And that one chip keeps both automations' runs.
        view.set_filter(
            RunsFilter {
                project: Some(alpha.clone()),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(ids(&view.rows()), vec!["1", "2"]);
    });
}

#[gpui::test]
fn a_failed_refresh_keeps_the_rows_and_says_so(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| Runs::new(service(), cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.apply(vec![run("kept", "gone", "succeeded", 1_000)], vec![], cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("runs-refresh-error").is_none());

    view.update(cx, |view, cx| {
        view.loaded(Err("database is locked".into()), cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(ids(&view.rows()), vec!["kept"], "the last good rows stay");
        assert_eq!(view.error(), Some("database is locked"));
    });
    assert!(
        cx.debug_bounds("runs-refresh-error").is_some(),
        "the header says the refresh failed"
    );
    assert!(cx.debug_bounds("runs-row-kept").is_some());

    // The next good load clears it.
    view.update(cx, |view, cx| {
        view.loaded(
            Ok((vec![run("new", "gone", "succeeded", 2_000)], vec![])),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("runs-refresh-error").is_none());
}

#[gpui::test]
fn an_automations_runs_load_even_when_older_than_the_latest_200(cx: &mut TestAppContext) {
    let service = service();
    let old = save(&service, &project("alpha"), "Old and quiet");
    let busy: Vec<String> = ["One", "Two", "Three"]
        .iter()
        .map(|name| save(&service, &project("beta"), name))
        .collect();
    let base = now_ms() - 1_000_000;
    let mut quiet = Vec::new();
    for offset in 0..3 {
        quiet.push(run_now(&service, &old, base + offset));
    }
    // Three busy automations push past the 200 latest runs.
    for step in 0..100 {
        for (index, id) in busy.iter().enumerate() {
            run_now(&service, id, base + 10_000 + step * 10 + index as i64);
        }
    }
    let view = cx.new(|cx| Runs::new(service.clone(), cx));
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.rows().len(), LIMIT, "the latest 200 of everything");
        assert!(view.rows().iter().all(|row| !quiet.contains(&row.id)));
    });
    // "See all in Runs" for the quiet one lists exactly its own runs.
    view.update(cx, |view, cx| {
        view.set_filter(
            RunsFilter {
                automation: Some(old.clone()),
                ..Default::default()
            },
            cx,
        )
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        let mut shown: Vec<_> = ids(&view.rows()).into_iter().map(String::from).collect();
        shown.sort();
        quiet.sort();
        assert_eq!(shown, quiet);
    });
    // Clearing goes back to the latest 200.
    view.update(cx, |view, cx| view.set_filter(RunsFilter::default(), cx));
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert_eq!(view.rows().len(), LIMIT));
}

#[gpui::test]
fn the_automation_chip_is_not_deleted_while_the_first_load_is_in_flight(cx: &mut TestAppContext) {
    let service = service();
    let id = save(&service, &project("alpha"), "Triage");
    let view = cx.new(|cx| Runs::new(service.clone(), cx));
    // Before the load lands: the carried name, or none, never "Deleted".
    view.update(cx, |view, cx| {
        view.set_filter(
            RunsFilter {
                automation: Some(id.clone()),
                automation_name: Some("Triage".into()),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(view.automation_label().as_deref(), Some("Triage"));
        view.set_filter(
            RunsFilter {
                automation: Some(id.clone()),
                ..Default::default()
            },
            cx,
        );
        assert_eq!(view.automation_label().as_deref(), Some(""));
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.automation_label().as_deref(), Some("Triage"));
    });
    // Loaded, and the automation is not there: now it is gone.
    view.update(cx, |view, cx| {
        view.set_filter(
            RunsFilter {
                automation: Some("missing".into()),
                ..Default::default()
            },
            cx,
        )
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.automation_label().as_deref(),
            Some("Deleted automation")
        );
    });
}
