//! The Automations page: the list grouped by project with a run sparkline per
//! card, and the detail's summary cards and Instructions / Recent runs tabs.
use super::*;
use chrono::{Local, TimeZone};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::{cell::RefCell, rc::Rc};

const CODEX_BINARY: &str = "GESPAN_CODEX_BINARY";

/// A database of automations and the window showing the page over it.
struct Page {
    _dir: tempfile::TempDir,
    service: Arc<Mutex<Service>>,
    codex_binary: Option<std::ffi::OsString>,
}

impl Drop for Page {
    fn drop(&mut self) {
        // SAFETY: as in `page`; later tests see the environment they started with.
        unsafe {
            match self.codex_binary.take() {
                Some(value) => std::env::set_var(CODEX_BINARY, value),
                None => std::env::remove_var(CODEX_BINARY),
            }
        }
    }
}

fn page() -> Page {
    // Opening an automation loads its model's controls; a missing CLI makes
    // that fail at once instead of starting a real Codex.
    let codex_binary = std::env::var_os(CODEX_BINARY);
    // SAFETY: set before the window starts any thread that reads it, and
    // restored when `Page` drops.
    unsafe { std::env::set_var(CODEX_BINARY, "/missing/gespan-test/codex") };
    let dir = tempfile::tempdir().unwrap();
    let service = Service::open(&dir.path().join("gespan.db")).unwrap();
    Page {
        _dir: dir,
        service: Arc::new(Mutex::new(service)),
        codex_binary,
    }
}

fn now() -> i64 {
    gespan_core::time::now_ms() as i64
}

fn save(page: &Page, name: &str, cwd: &str, trigger: AutomationTrigger) -> Automation {
    let mut draft = AutomationUpsert::new(cwd.into(), "codex".into(), "gpt-5.5".into());
    draft.name = name.into();
    draft.prompt = format!("{name}: do the work");
    draft.triggers = Some(vec![trigger]);
    draft.sync_triggers(now());
    page.service.lock().unwrap().upsert(draft).unwrap()
}

fn open(
    page: &Page,
    cwd: &str,
    cx: &mut TestAppContext,
) -> (WindowHandle<Automations>, VisualTestContext) {
    let service = page.service.clone();
    let cwd = cwd.to_string();
    let window = cx.add_window(|_, cx| Automations::new(service, cwd.clone(), vec![cwd], cx));
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    (window, visual)
}

/// `debug_bounds` for a selector built at run time.
trait Drawn {
    fn drawn(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>>;
}
impl Drawn for VisualTestContext {
    fn drawn(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
    }
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    visual.refresh().unwrap();
    let bounds = visual
        .drawn(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn top(visual: &mut VisualTestContext, selector: &str) -> gpui::Pixels {
    visual
        .drawn(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .origin
        .y
}

/// Local noon `days` ago, so a run lands on that calendar day whatever the
/// time zone or daylight-saving change.
fn noon(days: i64) -> i64 {
    let date = Local::now().date_naive() - chrono::Duration::days(days);
    Local
        .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        .earliest()
        .unwrap()
        .timestamp_millis()
}

#[gpui::test]
fn list_groups_automations_by_project(cx: &mut TestAppContext) {
    let page = page();
    let time = AutomationTrigger::new("time", "daily");
    let first = save(&page, "Fix langfuse issues", "/work/langfuse", time.clone());
    let other = save(&page, "Fix cognis issues", "/work/cognis", time.clone());
    let second = save(&page, "Triage langfuse", "/work/langfuse", time);
    // The window opens on cognis, so its group comes first.
    let (window, mut visual) = open(&page, "/work/cognis", cx);

    let groups = window
        .read_with(&visual, |view, _| {
            view.project_groups(&view.automations)
                .into_iter()
                .map(|(cwd, items)| (cwd, items.iter().map(|a| a.id.clone()).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert_eq!(groups[0], ("/work/cognis".into(), vec![other.id.clone()]));
    assert_eq!(groups[1].0, "/work/langfuse");
    let mut langfuse = groups[1].1.clone();
    langfuse.sort();
    let mut expected = vec![first.id.clone(), second.id.clone()];
    expected.sort();
    assert_eq!(
        langfuse, expected,
        "both langfuse automations share one group"
    );

    // Drawn as heading, its cards, then the next heading and its cards.
    let cognis_heading = top(&mut visual, "automation-group-0");
    let langfuse_heading = top(&mut visual, "automation-group-1");
    let cognis_card = top(&mut visual, &format!("automation-card-{}", other.id));
    let first_card = top(&mut visual, &format!("automation-card-{}", first.id));
    let second_card = top(&mut visual, &format!("automation-card-{}", second.id));
    assert!(cognis_heading < cognis_card);
    assert!(cognis_card < langfuse_heading);
    assert!(langfuse_heading < first_card && langfuse_heading < second_card);
    assert!(visual.drawn("automation-group-2").is_none());

    // Filtering drops a group whose automations all stop matching.
    window
        .update(&mut visual, |view, _, cx| {
            view.filter
                .update(cx, |input, cx| input.set_text("langfuse".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.drawn("automation-group-1").is_none());
    assert!(
        visual
            .drawn(&format!("automation-card-{}", other.id))
            .is_none()
    );
    assert!(
        top(&mut visual, "automation-group-0")
            < top(&mut visual, &format!("automation-card-{}", first.id))
    );
}

#[gpui::test]
fn card_sparkline_has_ten_days(cx: &mut TestAppContext) {
    let page = page();
    let busy = save(
        &page,
        "Busy",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let quiet = save(
        &page,
        "Quiet",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    {
        let mut service = page.service.lock().unwrap();
        for (days_ago, runs) in [(0, 4), (3, 2), (9, 1), (10, 5), (30, 1)] {
            for _ in 0..runs {
                service.run_now(&busy.id, noon(days_ago)).unwrap();
            }
        }
    }
    let (window, mut visual) = open(&page, "/work/app", cx);

    // Oldest first, today last; runs 10 or more days ago fall outside.
    let counts = window
        .read_with(&visual, |view, _| {
            (
                view.sparklines.get(&busy.id).cloned(),
                view.sparklines.get(&quiet.id).cloned(),
            )
        })
        .unwrap();
    assert_eq!(counts.0, Some(vec![1, 0, 0, 0, 0, 0, 2, 0, 0, 4]));
    assert_eq!(counts.1, Some(vec![0; 10]));

    // One bar per day, each drawn; there is no eleventh.
    let bar = |visual: &mut VisualTestContext, id: &str, day: usize| {
        visual
            .drawn(&format!("automation-spark-{id}-{day}"))
            .unwrap_or_else(|| panic!("day {day} of {id} is not drawn"))
            .size
            .height
    };
    for day in 0..10 {
        bar(&mut visual, &busy.id, day);
        assert_eq!(
            bar(&mut visual, &quiet.id, day),
            gpui::px(2.),
            "empty day stub"
        );
    }
    assert!(
        visual
            .drawn(&format!("automation-spark-{}-10", busy.id))
            .is_none()
    );
    // Height follows the count: the busiest day is tallest, empty days a stub.
    let today = bar(&mut visual, &busy.id, 9);
    let three_ago = bar(&mut visual, &busy.id, 6);
    let nine_ago = bar(&mut visual, &busy.id, 0);
    assert_eq!(bar(&mut visual, &busy.id, 1), gpui::px(2.));
    assert!(today > three_ago && three_ago > nine_ago && nine_ago > gpui::px(2.));

    // A new run shows on the next refresh.
    page.service
        .lock()
        .unwrap()
        .run_now(&quiet.id, noon(0))
        .unwrap();
    window
        .update(&mut visual, |view, _, cx| view.refresh(cx))
        .unwrap();
    visual.run_until_parked();
    let quiet_counts = window
        .read_with(&visual, |view, _| view.sparklines.get(&quiet.id).cloned())
        .unwrap();
    assert_eq!(quiet_counts, Some(vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 1]));
}

#[gpui::test]
fn summary_cards_describe_project_trigger_and_agent(cx: &mut TestAppContext) {
    let page = page();
    let automation = {
        let mut draft =
            AutomationUpsert::new("/work/langfuse".into(), "codex".into(), "gpt-5.5".into());
        draft.name = "Auto-fix langfuse issues".into();
        draft.prompt = "Fix the issue".into();
        draft.workspace_mode = "worktree".into();
        draft.runtime_mode = "full-access".into();
        draft
            .model_settings
            .insert("reasoningEffort".into(), "high".into());
        draft.triggers = Some(vec![AutomationTrigger::new("github", "issue_opened")]);
        draft.sync_triggers(now());
        page.service.lock().unwrap().upsert(draft).unwrap()
    };
    let (window, mut visual) = open(&page, "/work/langfuse", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));

    let cards = window
        .read_with(&visual, |view, cx| view.summary_cards(cx))
        .unwrap();
    let text: Vec<_> = cards
        .iter()
        .map(|c| (c.title, c.primary.as_str(), c.secondary.as_str()))
        .collect();
    let model = cached_model_name(Kind::Codex, Some("gpt-5.5"));
    assert_eq!(
        text,
        [
            ("Runs in project", "langfuse", "Fresh worktree"),
            (
                "When",
                "Issue opened",
                "GitHub · by anyone · any repository"
            ),
            (
                "Agent",
                &*format!("Codex · {model}"),
                "Effort High · Full access"
            ),
        ]
    );
    for key in ["project", "when", "agent"] {
        assert!(
            visual.drawn(&format!("automation-summary-{key}")).is_some(),
            "{key} card is drawn"
        );
    }

    // The cards follow unsaved edits: a named project, a repository and actor.
    window
        .update(&mut visual, |view, _, cx| {
            let mut appearances = HashMap::new();
            appearances.insert(
                "/work/langfuse".to_string(),
                gespan_core::project_appearance::ProjectAppearance {
                    name: "Langfuse".into(),
                    ..Default::default()
                },
            );
            view.set_appearances(appearances, cx);
            let editor = view.editor.as_mut().unwrap();
            editor.draft.workspace_mode = "current".into();
            editor.draft.runtime_mode = "supervised".into();
            editor.draft.model_settings.clear();
            let fields = &editor.fields[0];
            fields.repos.update(cx, |input, cx| {
                input.set_text("langfuse/langfuse".into(), cx)
            });
            fields
                .actor
                .update(cx, |input, cx| input.set_text("octocat".into(), cx));
        })
        .unwrap();
    let cards = window
        .read_with(&visual, |view, cx| view.summary_cards(cx))
        .unwrap();
    assert_eq!(cards[0].primary, "Langfuse");
    assert_eq!(cards[0].secondary, "Current");
    assert_eq!(
        cards[1].secondary,
        "GitHub · by octocat · langfuse/langfuse"
    );
    assert_eq!(cards[2].secondary, "Supervised");

    // A schedule reads as its schedule.
    let scheduled = {
        let mut trigger = AutomationTrigger::new("time", "daily");
        trigger.time = "07:30".into();
        save(&page, "Morning", "/work/langfuse", trigger)
    };
    window
        .update(&mut visual, |view, _, cx| view.refresh(cx))
        .unwrap();
    visual.run_until_parked();
    click(&mut visual, &format!("automation-card-{}", scheduled.id));
    let cards = window
        .read_with(&visual, |view, cx| view.summary_cards(cx))
        .unwrap();
    assert_eq!(cards[1].primary, "Daily at 07:30");
    assert!(
        cards[1].secondary.starts_with("Next run "),
        "{}",
        cards[1].secondary
    );
}

#[gpui::test]
fn see_all_in_runs_emits_open_runs(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Check the build",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let (window, mut visual) = open(&page, "/work/app", cx);
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = opened.clone();
    let _subscription = window
        .update(&mut visual, |_, _, cx| {
            cx.subscribe(&cx.entity(), move |_, _, event: &OpenRuns, _| {
                sink.borrow_mut().push(event.automation.clone())
            })
        })
        .unwrap();

    click(&mut visual, &format!("automation-card-{}", automation.id));
    assert!(
        visual.drawn("automation-see-all-runs").is_none(),
        "only the Recent runs tab links to Runs"
    );
    click(&mut visual, "automation-history-tab");
    click(&mut visual, "automation-see-all-runs");
    assert_eq!(*opened.borrow(), std::slice::from_ref(&automation.id));
}

#[gpui::test]
fn tabs_switch_between_instructions_and_recent_runs(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Check the build",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    page.service
        .lock()
        .unwrap()
        .run_now(&automation.id, now())
        .unwrap();
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));
    let shown = |visual: &mut VisualTestContext| {
        (
            visual.drawn("automation-settings-scroll").is_some(),
            visual.drawn("automation-history-scroll").is_some(),
        )
    };
    // Opens on Instructions: the editor, not the runs.
    assert_eq!(shown(&mut visual), (true, false));

    click(&mut visual, "automation-history-tab");
    assert_eq!(shown(&mut visual), (false, true));
    assert!(window.read_with(&visual, |view, _| view.history).unwrap());
    assert!(
        visual.drawn("automation-run-row-0").is_some(),
        "the run is listed"
    );

    click(&mut visual, "automation-settings-tab");
    assert_eq!(shown(&mut visual), (true, false));

    // A summary card jumps back from Recent runs to its setting.
    click(&mut visual, "automation-history-tab");
    click(&mut visual, "automation-summary-when");
    assert_eq!(shown(&mut visual), (true, false));

    // A new automation has nothing to list yet, so it has no tabs.
    window
        .update(&mut visual, |view, _, cx| {
            view.selected = None;
            let draft = AutomationUpsert::new("/work/app".into(), "codex".into(), "gpt-5.5".into());
            view.set_editor(draft, false, cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.drawn("automation-history-tab").is_none());
    assert_eq!(shown(&mut visual), (true, false));
}
