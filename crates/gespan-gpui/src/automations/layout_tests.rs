//! The Automations page: the list grouped by project with a run sparkline per
//! card, and the detail's summary cards and Instructions / Recent runs tabs.
use super::*;
use chrono::{Local, TimeZone};
use gespan_core::review_policy::ReviewResults;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::{cell::RefCell, rc::Rc};

const CODEX_BINARY: &str = "GESPAN_CODEX_BINARY";

/// A database of automations and the window showing the page over it.
pub(super) struct Page {
    _dir: tempfile::TempDir,
    pub(super) service: Arc<Mutex<Service>>,
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

pub(super) fn page() -> Page {
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

pub(super) fn open(
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
pub(super) trait Drawn {
    fn drawn(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>>;
}
impl Drawn for VisualTestContext {
    fn drawn(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
    }
}

pub(super) fn click(visual: &mut VisualTestContext, selector: &str) {
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
fn folders_spelled_with_and_without_a_trailing_slash_share_a_group(cx: &mut TestAppContext) {
    let page = page();
    let time = AutomationTrigger::new("time", "daily");
    let plain = save(&page, "One", "/work/langfuse", time.clone());
    let slashed = save(&page, "Two", "/work/langfuse/", time);
    let (window, visual) = open(&page, "/work/langfuse", cx);
    let groups = window
        .read_with(&visual, |view, _| {
            view.project_groups(&view.automations)
                .into_iter()
                .map(|(_, items)| items.len())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(groups, vec![2], "{} and {}", plain.id, slashed.id);
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
            gpui::px(3.),
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
    assert_eq!(bar(&mut visual, &busy.id, 1), gpui::px(3.));
    assert!(today > three_ago && three_ago > nine_ago && nine_ago > gpui::px(3.));

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
            fields
                .property("repository")
                .unwrap()
                .update(cx, |input, cx| {
                    input.set_text("langfuse/langfuse".into(), cx)
                });
            fields
                .property("actor")
                .unwrap()
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

// --- "Do": start a session, or wake a Pilot.

fn stored(page: &Page, id: &str) -> Automation {
    page.service
        .lock()
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|automation| automation.id == id)
        .unwrap()
}

#[gpui::test]
fn waking_a_pilot_hides_the_session_settings_and_saves_the_pilot(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Review comments",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let (window, mut visual) = open(&page, "/work/app", cx);
    window
        .update(&mut visual, |view, _, cx| {
            view.set_pilots(
                vec![
                    ("pilot-1".into(), "ReviewPilot".into()),
                    ("pilot-2".into(), "Shroom".into()),
                ],
                cx,
            )
        })
        .unwrap();
    click(&mut visual, &format!("automation-card-{}", automation.id));
    let choose = |visual: &mut VisualTestContext, field: Choice, value: &str| {
        window
            .update(visual, |view, _, cx| {
                view.apply_choice(field, value.into(), cx)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    };
    let action = |visual: &VisualTestContext| {
        window
            .read_with(visual, |view, _| {
                view.editor.as_ref().unwrap().draft.action.clone()
            })
            .unwrap()
    };

    // A saved automation starts a session, with every session setting shown.
    assert_eq!(action(&visual), AutomationAction::Session);
    for shown in [
        "automation-action",
        "automation-model",
        "automation-session-settings",
    ] {
        assert!(visual.drawn(shown).is_some(), "{shown}");
    }
    assert!(visual.drawn("automation-pilot").is_none());
    choose(&mut visual, Choice::Access, "supervised");
    choose(&mut visual, Choice::Workspace, "current");
    choose(&mut visual, Choice::Conversation, "reuse");
    choose(&mut visual, Choice::Folder, "folder-1");

    // "Wake a Pilot" picks the first Pilot and hides what only a session uses.
    choose(&mut visual, Choice::Action, "pilot");
    assert_eq!(
        action(&visual),
        AutomationAction::Pilot {
            pilot_id: "pilot-1".into()
        }
    );
    assert!(visual.drawn("automation-pilot").is_some());
    for hidden in ["automation-model", "automation-session-settings"] {
        assert!(visual.drawn(hidden).is_none(), "{hidden} is still drawn");
    }
    choose(&mut visual, Choice::Pilot, "pilot-2");
    let cards = window
        .read_with(&visual, |view, cx| view.summary_cards(cx))
        .unwrap();
    let agent = cards.iter().find(|card| card.key == "agent").unwrap();
    assert_eq!((agent.title, agent.primary.as_str()), ("Pilot", "Shroom"));

    // Back to a session: its settings are as they were left.
    choose(&mut visual, Choice::Action, "session");
    assert_eq!(action(&visual), AutomationAction::Session);
    assert!(visual.drawn("automation-session-settings").is_some());
    assert!(visual.drawn("automation-model").is_some());
    assert!(visual.drawn("automation-pilot").is_none());
    let kept = window
        .read_with(&visual, |view, _| {
            let draft = &view.editor.as_ref().unwrap().draft;
            (
                draft.runtime_mode.clone(),
                draft.workspace_mode.clone(),
                draft.reuse_session,
                draft.session_folder_id.clone(),
                draft.harness.clone(),
                draft.model.clone(),
            )
        })
        .unwrap();
    assert_eq!(
        kept,
        (
            "supervised".into(),
            "current".into(),
            true,
            "folder-1".into(),
            "codex".into(),
            "gpt-5.5".into()
        )
    );

    // And back again returns to the Pilot chosen before, which is what saves.
    choose(&mut visual, Choice::Action, "pilot");
    assert_eq!(
        action(&visual),
        AutomationAction::Pilot {
            pilot_id: "pilot-2".into()
        }
    );
    window
        .update(&mut visual, |view, _, cx| view.save(cx))
        .unwrap();
    visual.run_until_parked();
    let saved = stored(&page, &automation.id);
    assert_eq!(
        serde_json::to_value(&saved.action).unwrap(),
        serde_json::json!({"kind": "pilot", "pilotId": "pilot-2"})
    );
    assert_eq!(saved.runtime_mode, "supervised");
    assert_eq!(saved.session_folder_id, "folder-1");
    assert!(saved.reuse_session);

    // The list names the Pilot; one that was deleted is said to be gone.
    let labels = window
        .read_with(&visual, |view, _| {
            (view.pilot_label("pilot-2"), view.pilot_label("deleted"))
        })
        .unwrap();
    assert_eq!(labels, ("Shroom".into(), "Unavailable Pilot".into()));
}

#[gpui::test]
fn waking_a_pilot_with_no_pilots_cannot_be_saved(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Review comments",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Action, "pilot".into(), cx);
            view.save(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        window
            .read_with(&visual, |view, _| view.error.clone())
            .unwrap()
            .as_deref(),
        Some("Choose a Pilot for this automation.")
    );
    assert_eq!(
        stored(&page, &automation.id).action,
        AutomationAction::Session
    );
}

/// A saved automation, as a review one when `results` is given.
fn save_review(page: &Page, harness: &str, results: Option<ReviewResults>) -> Automation {
    let mut draft = AutomationUpsert::new("/work/app".into(), harness.into(), "m".into());
    draft.name = "Review".into();
    draft.prompt = "Review the pull request.".into();
    draft.triggers = Some(vec![AutomationTrigger::new("time", "daily")]);
    draft.review_results = results;
    draft.sync_triggers(now());
    page.service.lock().unwrap().upsert(draft).unwrap()
}

fn menu_options(
    window: &WindowHandle<Automations>,
    visual: &VisualTestContext,
) -> Vec<(String, String)> {
    window
        .read_with(visual, |view, _| {
            view.choice.as_ref().unwrap().options.clone()
        })
        .unwrap()
}

#[gpui::test]
fn editor_offers_read_only_access(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Triage",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));
    click(&mut visual, "automation-access");
    let options = menu_options(&window, &visual);
    assert_eq!(
        options.first().cloned(),
        Some(("read-only".into(), "Read-only".into()))
    );
    assert_eq!(options.len(), 5);

    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Access, "read-only".into(), cx);
            view.save(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(stored(&page, &automation.id).runtime_mode, "read-only");
    // The control names it.
    assert_eq!(super::editor::access_label("read-only"), "Read-only");
}

#[gpui::test]
fn results_row_only_for_review_automations(cx: &mut TestAppContext) {
    let page = page();
    let plain = save_review(&page, "codex", None);
    let review = save_review(&page, "codex", Some(ReviewResults::Gespan));
    let (window, mut visual) = open(&page, "/work/app", cx);

    click(&mut visual, &format!("automation-card-{}", plain.id));
    assert!(visual.drawn("automation-results").is_none());

    click(&mut visual, &format!("automation-card-{}", review.id));
    assert!(visual.drawn("automation-results").is_some());
    click(&mut visual, "automation-results");
    let labels: Vec<_> = menu_options(&window, &visual)
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    assert_eq!(
        labels,
        [
            "keeps the review in Gespan",
            "drafts a review for you to approve",
            "posts the review on the PR",
            "leaves a draft review on the PR for you to submit"
        ]
    );
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Results, "draft".into(), cx);
            view.save(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        stored(&page, &review.id).review_results,
        Some(ReviewResults::Draft)
    );
    assert_eq!(stored(&page, &plain.id).review_results, None);
}

#[gpui::test]
fn results_row_respects_harness(cx: &mut TestAppContext) {
    let page = page();
    let review = save_review(&page, "pi", Some(ReviewResults::Gespan));
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", review.id));
    click(&mut visual, "automation-results");
    assert_eq!(
        menu_options(&window, &visual),
        [
            (
                "gespan".to_string(),
                "keeps the review in Gespan".to_string()
            ),
            (
                "pending".to_string(),
                "leaves a draft review on the PR for you to submit".to_string()
            )
        ]
    );
    // A forced choice the harness can't deliver is ignored.
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Results, "post".into(), cx)
        })
        .unwrap();
    let results = window
        .read_with(&visual, |view, _| {
            view.editor.as_ref().unwrap().draft.review_results
        })
        .unwrap();
    assert_eq!(results, Some(ReviewResults::Gespan));
}

#[gpui::test]
fn changing_to_a_harness_that_cannot_deliver_results_falls_back_to_gespan(cx: &mut TestAppContext) {
    let page = page();
    let review = save_review(&page, "codex", Some(ReviewResults::Post));
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", review.id));
    let results = |visual: &VisualTestContext| {
        window
            .read_with(visual, |view, _| {
                view.editor.as_ref().unwrap().draft.review_results
            })
            .unwrap()
    };
    // Another harness that can post keeps it.
    window
        .update(&mut visual, |view, _, cx| {
            view.editor
                .as_mut()
                .unwrap()
                .set_model(Kind::Claude, Some("sonnet".into()), cx)
        })
        .unwrap();
    assert_eq!(results(&visual), Some(ReviewResults::Post));
    // Pi can only keep the review in Gespan.
    window
        .update(&mut visual, |view, _, cx| {
            view.editor
                .as_mut()
                .unwrap()
                .set_model(Kind::Pi, Some("m".into()), cx)
        })
        .unwrap();
    assert_eq!(results(&visual), Some(ReviewResults::Gespan));
}

#[gpui::test]
fn a_review_automation_cannot_choose_to_wake_a_pilot(cx: &mut TestAppContext) {
    let page = page();
    let plain = save_review(&page, "codex", None);
    let review = save_review(&page, "codex", Some(ReviewResults::Gespan));
    let (window, mut visual) = open(&page, "/work/app", cx);
    window
        .update(&mut visual, |view, _, cx| {
            view.set_pilots(vec![("pilot-1".into(), "Shroom".into())], cx)
        })
        .unwrap();

    click(&mut visual, &format!("automation-card-{}", plain.id));
    click(&mut visual, "automation-action");
    assert_eq!(menu_options(&window, &visual).len(), 2);
    window
        .update(&mut visual, |view, _, cx| {
            view.choice = None;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();

    click(&mut visual, &format!("automation-card-{}", review.id));
    click(&mut visual, "automation-action");
    assert_eq!(
        menu_options(&window, &visual),
        [("session".to_string(), "Start a session".to_string())]
    );
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Action, "pilot".into(), cx)
        })
        .unwrap();
    let action = window
        .read_with(&visual, |view, _| {
            view.editor.as_ref().unwrap().draft.action.clone()
        })
        .unwrap();
    assert_eq!(action, AutomationAction::Session);
}

#[gpui::test]
fn a_legacy_review_pilot_automation_can_be_switched_to_a_session_and_enabled(
    cx: &mut TestAppContext,
) {
    let page = page();
    let mut draft = AutomationUpsert::new("/work/app".into(), "codex".into(), "m".into());
    draft.name = "Legacy".into();
    draft.prompt = "Review.".into();
    draft.triggers = Some(vec![AutomationTrigger::new("time", "daily")]);
    draft.review_results = Some(ReviewResults::Post);
    draft.action = AutomationAction::Pilot {
        pilot_id: "pilot-1".into(),
    };
    draft.enabled = false;
    draft.sync_triggers(now());
    let legacy = page.service.lock().unwrap().upsert(draft).unwrap();
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", legacy.id));
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Action, "session".into(), cx);
            view.save(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(stored(&page, &legacy.id).action, AutomationAction::Session);
    assert!(
        page.service
            .lock()
            .unwrap()
            .set_enabled(&legacy.id, true)
            .is_ok()
    );
}

#[gpui::test]
fn a_review_automation_runs_read_only_and_cannot_change_it(cx: &mut TestAppContext) {
    let page = page();
    let plain = save_review(&page, "codex", None);
    let review = save_review(&page, "codex", Some(ReviewResults::Gespan));
    let (window, mut visual) = open(&page, "/work/app", cx);

    click(&mut visual, &format!("automation-card-{}", review.id));
    let before = window
        .read_with(&visual, |view, _| {
            view.editor.as_ref().unwrap().draft.runtime_mode.clone()
        })
        .unwrap();
    // Clicking the locked chip opens no menu, and a forced choice is ignored.
    click(&mut visual, "automation-access");
    assert!(
        window
            .read_with(&visual, |v, _| v.choice.is_none())
            .unwrap()
    );
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Access, "full-access".into(), cx)
        })
        .unwrap();
    let (mode, dirty) = window
        .read_with(&visual, |view, _| {
            let editor = view.editor.as_ref().unwrap();
            (editor.draft.runtime_mode.clone(), editor.dirty)
        })
        .unwrap();
    assert_eq!((mode, dirty), (before.clone(), false));

    // The Agent card says what the run does, not the stored mode.
    assert_ne!(before, "read-only", "the stored mode is not read-only");
    let agent = |window: &WindowHandle<Automations>, visual: &VisualTestContext| {
        window
            .read_with(visual, |view, cx| {
                let cards = view.summary_cards(cx);
                cards
                    .into_iter()
                    .find(|c| c.key == "agent")
                    .unwrap()
                    .secondary
            })
            .unwrap()
    };
    assert_eq!(agent(&window, &visual), "Read-only");
    // The locked chip explains itself.
    assert_eq!(
        window
            .read_with(&visual, |view, _| super::editor::access_hint(
                &view.editor.as_ref().unwrap().draft
            ))
            .unwrap(),
        Some("Reviews always run read-only")
    );

    // A plain automation still chooses its access.
    click(&mut visual, &format!("automation-card-{}", plain.id));
    assert_eq!(agent(&window, &visual), "Auto");
    assert_eq!(
        window
            .read_with(&visual, |view, _| super::editor::access_hint(
                &view.editor.as_ref().unwrap().draft
            ))
            .unwrap(),
        None
    );
    click(&mut visual, "automation-access");
    assert_eq!(menu_options(&window, &visual).len(), 5);
}

#[gpui::test]
fn an_ignored_choice_does_not_dirty_the_draft(cx: &mut TestAppContext) {
    let page = page();
    let review = save_review(&page, "codex", Some(ReviewResults::Gespan));
    let plain = save_review(&page, "codex", None);
    let (window, mut visual) = open(&page, "/work/app", cx);
    let dirty = |window: &WindowHandle<Automations>, visual: &VisualTestContext| {
        window
            .read_with(visual, |v, _| v.editor.as_ref().unwrap().dirty)
            .unwrap()
    };
    click(&mut visual, &format!("automation-card-{}", review.id));
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Action, "pilot".into(), cx)
        })
        .unwrap();
    assert!(!dirty(&window, &visual));
    // Results on a non-review automation is ignored the same way.
    click(&mut visual, &format!("automation-card-{}", plain.id));
    window
        .update(&mut visual, |view, _, cx| {
            view.apply_choice(Choice::Results, "post".into(), cx)
        })
        .unwrap();
    assert!(!dirty(&window, &visual));
    assert_eq!(
        window
            .read_with(&visual, |v, _| v
                .editor
                .as_ref()
                .unwrap()
                .draft
                .review_results)
            .unwrap(),
        None
    );
}

// --- The project an automation runs in.

/// A window that knows both `/work/app` and `/work/other`, showing the saved
/// `automation`, or a new automation in `/work/app` when there is none.
fn open_with_projects(
    page: &Page,
    automation: Option<&Automation>,
    cx: &mut TestAppContext,
) -> (WindowHandle<Automations>, VisualTestContext) {
    // The picker's search field steps and picks with the app's input keys.
    cx.update(crate::input::bind);
    let service = page.service.clone();
    let window = cx.add_window(|_, cx| {
        Automations::new(
            service,
            "/work/app".into(),
            vec!["/work/app".into(), "/work/other".into()],
            cx,
        )
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    match automation {
        Some(automation) => click(&mut visual, &format!("automation-card-{}", automation.id)),
        None => {
            window
                .update(&mut visual, |view, _, cx| {
                    let mut draft =
                        AutomationUpsert::new("/work/app".into(), "codex".into(), "gpt-5.5".into());
                    draft.name = "Check the build".into();
                    draft.prompt = "Check the build: do the work".into();
                    view.set_editor(draft, false, cx);
                    let editor = view.editor.as_ref().unwrap();
                    let (name, prompt) = (editor.name.clone(), editor.prompt.clone());
                    name.update(cx, |i, cx| i.set_text("Check the build".into(), cx));
                    prompt.update(cx, |i, cx| {
                        i.set_text("Check the build: do the work".into(), cx)
                    });
                })
                .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
        }
    }
    (window, visual)
}

fn project_focused(window: WindowHandle<Automations>, visual: &mut VisualTestContext) -> bool {
    window
        .update(visual, |view, window, _| {
            view.project_focus.is_focused(window)
        })
        .unwrap()
}

fn picker_open(window: WindowHandle<Automations>, visual: &VisualTestContext) -> bool {
    window
        .read_with(visual, |view, _| view.project_picker.is_some())
        .unwrap()
}

#[gpui::test]
fn project_control_is_fully_visible_at_narrow_and_wide_widths(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Check the build",
        "/work/a-project-with-a-rather-long-folder-name",
        AutomationTrigger::new("time", "daily"),
    );
    let (_window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));
    // 560 is the page in the narrowest window (820 less the 260 sidebar).
    for width in [560., 900., 1440.] {
        visual.simulate_resize(gpui::size(px(width), px(800.)));
        visual.run_until_parked();
        visual.refresh().unwrap();
        let control = visual
            .drawn("automation-project")
            .unwrap_or_else(|| panic!("the project control is drawn at {width}px"));
        // Wide enough to read a name, and nothing of it past the window edge.
        assert!(
            control.size.width >= px(160.),
            "{width}px: the project control is squeezed to {:?}",
            control.size.width
        );
        assert!(
            control.size.height >= px(26.),
            "{width}px: {:?}",
            control.size
        );
        assert!(
            control.right() <= px(width),
            "{width}px: the control runs off the window at {:?}",
            control.right()
        );
        let name = visual
            .drawn("automation-project-name")
            .unwrap_or_else(|| panic!("the project name is drawn at {width}px"));
        assert!(
            name.size.width >= px(48.),
            "{width}px: the project name is clipped to {:?}",
            name.size.width
        );
    }
}

fn cwd(window: WindowHandle<Automations>, visual: &VisualTestContext) -> String {
    window
        .read_with(visual, |view, _| {
            view.editor.as_ref().unwrap().draft.cwd.clone()
        })
        .unwrap()
}

#[gpui::test]
fn project_control_opens_the_picker_with_mouse_and_keyboard(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with_projects(&page, None, cx);

    // The mouse opens it, under the control, and Escape gives focus back.
    let control = visual.drawn("automation-project").unwrap();
    click(&mut visual, "automation-project");
    assert!(picker_open(window, &visual));
    let picker = visual.drawn("project-picker").expect("the picker is drawn");
    assert!(
        picker.top() > control.center().y,
        "the picker opens below the control: {picker:?} vs {control:?}"
    );
    assert!(
        visual.drawn("automation-duplicate-hint").is_none(),
        "a new automation picks its project, it does not duplicate"
    );
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!picker_open(window, &visual));
    assert!(
        project_focused(window, &mut visual),
        "focus returns to the project control"
    );

    // The "Runs in project" card of a new automation opens the same picker.
    click(&mut visual, "automation-summary-project");
    assert!(
        picker_open(window, &visual),
        "the project card opens the picker"
    );
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!picker_open(window, &visual));

    // Tab reaches the control from the page, and Enter, Space or Down open it.
    for key in ["enter", "space", "down"] {
        window
            .update(&mut visual, |view, window, cx| {
                window.focus(&view.focus, cx)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let mut presses = 0;
        while !project_focused(window, &mut visual) {
            presses += 1;
            assert!(presses <= 30, "Tab never reaches the project control");
            // Tab is the app's FocusNext, bound by the shell around this page.
            visual.update(|window, cx| window.focus_next(cx));
            visual.run_until_parked();
            visual.refresh().unwrap();
        }
        visual.simulate_keystrokes(key);
        visual.run_until_parked();
        assert!(picker_open(window, &visual), "{key} opens the picker");
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(!picker_open(window, &visual));
    }
}

#[gpui::test]
fn a_new_automation_saves_the_project_picked_for_it(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with_projects(&page, None, cx);

    // By keyboard: open, step down to the other project, pick it.
    window
        .update(&mut visual, |view, window, cx| {
            window.focus(&view.project_focus, cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(picker_open(window, &visual));
    visual.simulate_keystrokes("down enter");
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(!picker_open(window, &visual));
    assert_eq!(cwd(window, &visual), "/work/other");
    assert_eq!(
        window
            .read_with(&visual, |view, _| view.project_control_label())
            .unwrap(),
        "Project: other"
    );
    assert!(project_focused(window, &mut visual));

    // By mouse: back to the first project, which is what is created.
    click(&mut visual, "automation-project");
    click(&mut visual, "project-picker-project-1");
    assert_eq!(cwd(window, &visual), "/work/app");
    window
        .update(&mut visual, |view, _, cx| view.save(cx))
        .unwrap();
    visual.run_until_parked();
    let saved = page.service.lock().unwrap().list().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].cwd, "/work/app");
}

#[gpui::test]
fn a_saved_automation_keeps_its_project_and_duplicates_into_another(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Check the build",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let (window, mut visual) = open_with_projects(&page, Some(&automation), cx);

    // Its card goes to the settings, not to a project picker.
    click(&mut visual, "automation-summary-project");
    assert!(!picker_open(window, &visual));

    // The control still opens the picker, which says it duplicates.
    click(&mut visual, "automation-project");
    assert!(picker_open(window, &visual));
    assert!(visual.drawn("automation-duplicate-hint").is_some());
    // /work/app is listed first as the current project, /work/other second.
    click(&mut visual, "project-picker-project-1");
    assert!(!picker_open(window, &visual));

    // A new, unsaved copy in the other project is open; the original is as it was.
    let (selected, saved, dirty, draft_id) = window
        .read_with(&visual, |view, _| {
            let editor = view.editor.as_ref().unwrap();
            (
                view.selected.clone(),
                editor.saved,
                editor.dirty,
                editor.draft.id.clone(),
            )
        })
        .unwrap();
    assert_eq!(selected, None);
    assert!(!saved && dirty, "the copy is new and unsaved");
    assert_ne!(draft_id, automation.id);
    assert_eq!(cwd(window, &visual), "/work/other");
    assert_eq!(stored(&page, &automation.id).cwd, "/work/app");

    window
        .update(&mut visual, |view, _, cx| view.save(cx))
        .unwrap();
    visual.run_until_parked();
    let mut all = page
        .service
        .lock()
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .map(|a| (a.name, a.cwd))
        .collect::<Vec<_>>();
    all.sort();
    assert_eq!(
        all,
        [
            ("Check the build".to_string(), "/work/app".to_string()),
            ("Check the build".to_string(), "/work/other".to_string()),
        ]
    );
    assert_eq!(stored(&page, &automation.id).cwd, "/work/app");
}

#[gpui::test]
fn saving_a_saved_automation_into_another_project_is_refused(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Check the build",
        "/work/app",
        AutomationTrigger::new("time", "daily"),
    );
    let (window, mut visual) = open_with_projects(&page, Some(&automation), cx);
    // Whatever path gets there, the service keeps the project fixed.
    window
        .update(&mut visual, |view, _, cx| {
            view.editor.as_mut().unwrap().draft.cwd = "/work/other".into();
            view.save(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        window
            .read_with(&visual, |view, _| view.error.clone())
            .unwrap()
            .as_deref(),
        Some(gespan_core::automations::PROJECT_FROZEN)
    );
    assert_eq!(stored(&page, &automation.id).cwd, "/work/app");
}

// --- Trigger properties: each kind of trigger declares its own settings.

fn property_text(
    window: WindowHandle<Automations>,
    visual: &VisualTestContext,
    trigger: usize,
    key: &str,
) -> Option<String> {
    window
        .read_with(visual, |view, cx| {
            view.editor.as_ref().unwrap().fields[trigger]
                .property(key)
                .map(|input| input.read(cx).content.to_string())
        })
        .unwrap()
}

#[gpui::test]
fn each_trigger_shows_the_properties_its_kind_declares(cx: &mut TestAppContext) {
    let page = page();
    let automation = {
        let mut draft = AutomationUpsert::new("/work/app".into(), "codex".into(), "gpt-5.5".into());
        draft.name = "Review".into();
        draft.prompt = "Review it".into();
        let mut github = AutomationTrigger::new("github", "pull_request_review_requested");
        github.repos = vec!["acme/app".into()];
        github.actor = "octocat".into();
        draft.triggers = Some(vec![
            github,
            AutomationTrigger::new("linear", "issue_created"),
            AutomationTrigger::new("time", "daily"),
        ]);
        draft.sync_triggers(now());
        page.service.lock().unwrap().upsert(draft).unwrap()
    };
    let (window, mut visual) = open(&page, "/work/app", cx);
    click(&mut visual, &format!("automation-card-{}", automation.id));
    let order = window
        .read_with(&visual, |view, _| {
            view.editor
                .as_ref()
                .unwrap()
                .draft
                .triggers
                .iter()
                .flatten()
                .map(|t| t.kind.clone())
                .collect::<Vec<_>>()
        })
        .unwrap();
    let at = |kind: &str| order.iter().position(|k| k == kind).unwrap();
    let (github, linear, time) = (at("github"), at("linear"), at("time"));

    // GitHub: a labelled Repository and By, each holding what was saved.
    for key in ["repository", "actor"] {
        assert!(
            visual.drawn(&format!("trigger-{github}-{key}")).is_some(),
            "GitHub's {key} is drawn"
        );
    }
    assert_eq!(
        property_text(window, &visual, github, "repository").as_deref(),
        Some("acme/app")
    );
    assert_eq!(
        property_text(window, &visual, github, "actor").as_deref(),
        Some("octocat")
    );
    // The Repository row is a real row: label and field side by side, readable.
    let row = visual
        .drawn(&format!("trigger-{github}-repository"))
        .unwrap();
    assert!(row.size.height >= px(40.), "{row:?}");

    // Linear only says who; a schedule has no extra settings.
    assert!(visual.drawn(&format!("trigger-{linear}-actor")).is_some());
    assert!(
        visual
            .drawn(&format!("trigger-{linear}-repository"))
            .is_none()
    );
    assert_eq!(
        property_text(window, &visual, linear, "actor").as_deref(),
        Some("")
    );
    assert!(
        visual
            .drawn(&format!("trigger-properties-{time}"))
            .is_none()
    );
}

#[gpui::test]
fn a_new_forge_trigger_is_prefilled_with_the_projects_repository(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with_projects(&page, None, cx);
    // As the forge reports it for the project: its own repository, then a
    // fork's parent.
    let add_github = |visual: &mut VisualTestContext| {
        window
            .update(visual, |view, _, cx| {
                view.repositories.insert(
                    "github".into(),
                    vec!["acme/app".into(), "upstream/app".into()],
                );
                view.repositories_cwd = "/work/app".into();
                let editor = view.editor.as_mut().unwrap();
                editor
                    .draft
                    .triggers
                    .get_or_insert_with(Vec::new)
                    .push(AutomationTrigger::new("github", "pull_request_opened"));
                view.rebuild_trigger_fields(cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    };
    add_github(&mut visual);
    assert_eq!(
        property_text(window, &visual, 0, "repository").as_deref(),
        Some("acme/app")
    );

    // Clearing it sticks: it is not filled in again.
    window
        .update(&mut visual, |view, _, cx| {
            let input = view.editor.as_ref().unwrap().fields[0]
                .property("repository")
                .unwrap()
                .clone();
            input.update(cx, |input, cx| input.set_text(String::new(), cx));
            view.prefill_repositories(cx);
        })
        .unwrap();
    assert_eq!(
        property_text(window, &visual, 0, "repository").as_deref(),
        Some("")
    );
    window
        .update(&mut visual, |view, _, cx| {
            let input = view.editor.as_ref().unwrap().fields[0]
                .property("repository")
                .unwrap()
                .clone();
            input.update(cx, |input, cx| input.set_text("acme/app".into(), cx));
        })
        .unwrap();

    window
        .update(&mut visual, |view, _, cx| view.save(cx))
        .unwrap();
    visual.run_until_parked();
    let saved = page.service.lock().unwrap().list().unwrap();
    assert_eq!(
        saved.len(),
        1,
        "{:?}",
        window.read_with(&visual, |v, _| v.error.clone())
    );
    let trigger = &saved[0].triggers()[0];
    assert_eq!(
        (trigger.repos.clone(), trigger.repo.clone()),
        (vec!["acme/app".to_string()], String::new())
    );
}

#[gpui::test]
fn a_saved_trigger_without_a_repository_shows_the_projects_without_becoming_unsaved(
    cx: &mut TestAppContext,
) {
    let page = page();
    let automation = save(
        &page,
        "Review",
        "/work/app",
        AutomationTrigger::new("github", "pull_request_opened"),
    );
    let (window, mut visual) = open_with_projects(&page, Some(&automation), cx);
    assert_eq!(
        property_text(window, &visual, 0, "repository").as_deref(),
        Some("")
    );
    window
        .update(&mut visual, |view, _, cx| {
            view.repositories
                .insert("github".into(), vec!["acme/app".into()]);
            view.prefill_repositories(cx);
        })
        .unwrap();
    assert_eq!(
        property_text(window, &visual, 0, "repository").as_deref(),
        Some("acme/app")
    );
    assert!(
        !window
            .read_with(&visual, |view, _| view.editor.as_ref().unwrap().dirty)
            .unwrap(),
        "showing the repository it already matched is not an edit"
    );
}

#[gpui::test]
fn a_trigger_listens_to_one_repository(cx: &mut TestAppContext) {
    let page = page();
    let automation = save(
        &page,
        "Review",
        "/work/app",
        AutomationTrigger::new("github", "pull_request_opened"),
    );
    let (window, mut visual) = open_with_projects(&page, Some(&automation), cx);
    let set = |visual: &mut VisualTestContext, text: &str| {
        let text = text.to_string();
        window
            .update(visual, |view, _, cx| {
                let input = view.editor.as_ref().unwrap().fields[0]
                    .property("repository")
                    .unwrap()
                    .clone();
                input.update(cx, |input, cx| input.set_text(text, cx));
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    };
    let error = |visual: &VisualTestContext| {
        window
            .read_with(visual, |view, cx| {
                view.property_error(cx).map(|(_, key, why)| (key, why))
            })
            .unwrap()
    };

    set(&mut visual, "acme/web, acme/app");
    assert_eq!(
        error(&visual),
        Some(("repository", "One repository per trigger, like owner/name"))
    );
    assert!(
        visual.drawn("trigger-0-repository").is_some(),
        "the row with the problem is drawn"
    );
    set(&mut visual, "acme");
    assert_eq!(
        error(&visual),
        Some(("repository", "Write the repository as owner/name"))
    );

    set(&mut visual, "acme/app");
    assert_eq!(error(&visual), None);
    window
        .update(&mut visual, |view, _, cx| view.save(cx))
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        stored(&page, &automation.id).triggers()[0].repos,
        vec!["acme/app".to_string()]
    );
}

#[gpui::test]
fn only_a_recent_success_shows_as_a_pill_and_failures_colour_their_day(cx: &mut TestAppContext) {
    let page = page();
    let time = AutomationTrigger::new("time", "daily");
    let failed = save(&page, "Failed", "/work/app", time.clone());
    let passed = save(&page, "Passed", "/work/app", time.clone());
    let cancelled = save(&page, "Cancelled", "/work/app", time.clone());
    let never = save(&page, "Never", "/work/app", time);
    {
        let mut service = page.service.lock().unwrap();
        // Passed: a failure three days ago, then a success today.
        for (automation, days_ago, status) in [
            (&passed, 3, "failed"),
            (&passed, 0, "succeeded"),
            (&failed, 0, "failed"),
            (&cancelled, 0, "cancelled"),
        ] {
            let run = service.run_now(&automation.id, noon(days_ago)).unwrap();
            service
                .update_run(&run.id, status, None, None, noon(days_ago))
                .unwrap();
        }
    }
    let (window, mut visual) = open(&page, "/work/app", cx);

    // Only the recent success is a pill, right after its bars.
    let pill = format!("automation-last-run-pill-{}", passed.id);
    assert!(visual.drawn(&pill).is_some());
    assert!(
        visual
            .drawn(&format!("automation-last-run-{}", passed.id))
            .is_none()
    );
    let bars = visual
        .drawn(&format!("automation-spark-{}", passed.id))
        .unwrap();
    let pill_bounds = visual.drawn(&pill).unwrap();
    assert_eq!(
        pill_bounds.origin.x,
        bars.origin.x + bars.size.width + gpui::px(8.),
        "the pill follows the bars, not the card's edge"
    );
    // A failure, a cancellation or no run at all reads as the plain line,
    // pushed against the card's right edge.
    for id in [&failed.id, &cancelled.id, &never.id] {
        assert!(
            visual
                .drawn(&format!("automation-last-run-pill-{id}"))
                .is_none()
        );
        let line = visual
            .drawn(&format!("automation-last-run-{id}"))
            .unwrap_or_else(|| panic!("{id} has no last-run line"));
        let card = visual.drawn(&format!("automation-card-{id}")).unwrap();
        let right = |b: gpui::Bounds<gpui::Pixels>| b.origin.x + b.size.width;
        assert!(right(card) - right(line) <= gpui::px(14.), "{id}");
    }

    // Days with a failed run are marked so their bar draws rose.
    let failed_days = window
        .read_with(&visual, |view, _| {
            (
                view.spark_failed.get(&passed.id).cloned().unwrap(),
                view.spark_failed.get(&failed.id).cloned().unwrap(),
                view.spark_failed.get(&never.id).cloned().unwrap(),
            )
        })
        .unwrap();
    let mut expected = vec![false; SPARK];
    expected[SPARK - 4] = true;
    assert_eq!(
        failed_days.0, expected,
        "three days ago failed, today passed"
    );
    let mut today = vec![false; SPARK];
    today[SPARK - 1] = true;
    assert_eq!(failed_days.1, today);
    assert_eq!(failed_days.2, vec![false; SPARK]);
}

const SPARK: usize = layout::SPARK_DAYS;

#[gpui::test]
fn new_button_leaves_the_open_automation_for_the_gallery(cx: &mut TestAppContext) {
    let page = page();
    let time = AutomationTrigger::new("time", "daily");
    let first = save(&page, "First", "/work/app", time.clone());
    let second = save(&page, "Second", "/work/app", time);
    let (window, mut visual) = open(&page, "/work/app", cx);
    let state = |visual: &mut VisualTestContext| {
        window
            .read_with(visual, |view, _| (view.gallery, view.selected.clone()))
            .unwrap()
    };
    assert_eq!(state(&mut visual), (true, None), "opens on the gallery");
    assert!(visual.drawn("automation-scratch").is_some());

    // Clicking a card opens it in the editor; only that card is selected.
    click(&mut visual, &format!("automation-card-{}", second.id));
    assert_eq!(state(&mut visual), (false, Some(second.id.clone())));
    assert!(visual.drawn("automation-scratch").is_none());
    click(&mut visual, &format!("automation-card-{}", first.id));
    assert_eq!(state(&mut visual), (false, Some(first.id.clone())));

    // "New" (the old + icon's action) goes back to the gallery and clears
    // the selection.
    click(&mut visual, "new-automation");
    assert_eq!(state(&mut visual), (true, None));
    assert!(visual.drawn("automation-scratch").is_some());
}

#[gpui::test]
fn gallery_tabs_switch_category_in_a_two_column_grid(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open(&page, "/work/app", cx);
    let row = |visual: &mut VisualTestContext, selector: &str| {
        visual
            .drawn(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
            .origin
    };

    // Popular: scratch and the first example share a row, side by side; the
    // third card starts the next row under the scratch card.
    let scratch = row(&mut visual, "automation-scratch");
    let bugs = row(&mut visual, "template-find-critical-bugs");
    let scan = row(&mut visual, "template-scan-vulnerabilities");
    assert_eq!(scratch.y, bugs.y);
    assert!(scratch.x < bugs.x);
    assert!(scan.y > scratch.y);
    assert_eq!(scan.x, scratch.x);
    assert!(visual.drawn("template-secret-scan").is_none());

    // The category tabs are one segmented control along a single row.
    let popular = row(&mut visual, "automation-category-popular");
    let security = row(&mut visual, "automation-category-security");
    assert_eq!(popular.y, security.y);
    assert!(popular.x < security.x);

    click(&mut visual, "automation-category-security");
    let category = window
        .read_with(&visual, |view, _| view.category.clone())
        .unwrap();
    assert_eq!(category, "security");
    for id in ["scan-vulnerabilities", "dependency-audit", "secret-scan"] {
        assert!(
            visual.drawn(&format!("template-{id}")).is_some(),
            "{id} is a security example"
        );
    }
    assert!(visual.drawn("template-find-critical-bugs").is_none());
    assert!(
        visual.drawn("automation-scratch").is_some(),
        "always offered"
    );
}
