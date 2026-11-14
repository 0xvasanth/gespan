//! The Schedule tab against its boards, and scheduled runs that start a
//! Flow, one step of it or an instruction, telling the chat per `tell`.
use super::tests::{
    Setup, finished, flow_events, flow_updates, flows_ok, habit_run, hold_flow_launches,
    hold_outbox, pilot_state, set_main_busy, setup, take_flow_updates,
};
use super::*;
use gespan_core::flow::{FlowRun, RunStatus};
use gespan_core::pilot::habits::{Run, Schedule, Target, Tell};
use gespan_core::time::now_ms;
use gpui::{TestAppContext, VisualTestContext};
use serde_json::json;

/// A two-step Flow with no inputs, so a schedule can start it unattended.
fn save_flow(s: &Setup, cx: &mut TestAppContext) -> String {
    let agent = |id: &str, title: &str, to: &str| {
        json!({"kind":"agent","id":id,"title":title,"harness":"codex","model":"",
            "brief":format!("Do {id}"),"outputs":[],"transitions":[{"to":to}],"maxVisits":3})
    };
    let flow = json!({"id":"","name":"Release","description":"Ship it","cwd":s.project,
        "inputs":[],"initial":"plan",
        "states":[agent("plan","Plan","build"),agent("build","Build & test","$succeeded")],
        "access":"auto"});
    flows_ok(s, "save-release", "flows.save", json!({"flow": flow}), cx)["flow"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn habit(id: &str, target: Target, tell: Tell) -> Habit {
    let mut h = Habit::create(
        format!("Habit {id}"),
        target,
        "Check main for unreleased fixes.".into(),
        tell,
        Schedule::default(),
        now_ms(),
    )
    .unwrap();
    h.id = id.into();
    h
}

/// Writes `all` to habits.json and shows them, as a reload would.
fn seed(s: &Setup, all: Vec<Habit>, cx: &mut TestAppContext) {
    habits::update(&s.root, &s.id, |stored| {
        *stored = all.clone();
        Ok(())
    })
    .unwrap();
    cx.update(|cx| s.manager.update(cx, |m, _| m.habits = all));
}

fn saved(s: &Setup) -> Vec<Habit> {
    habits::load(&s.root, &s.id).unwrap()
}

/// The Schedule tab in a 360px panel, as on the boards, on `route`.
fn schedule_tab(s: &Setup, route: Option<Route>, cx: &mut TestAppContext) -> VisualTestContext {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        input::bind(cx);
        crate::popover::bind(cx);
    });
    s.window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_width = 360.;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.overview_tab = OverviewTab::Schedule;
                m.route = route;
                m.clear_habit_fields();
                cx.notify();
            });
        })
        .unwrap();
    let visual = VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual
}

fn bounds(visual: &mut VisualTestContext, selector: &str) -> gpui::Bounds<gpui::Pixels> {
    visual
        .debug_bounds(Box::leak(selector.to_string().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    let at = bounds(visual, selector).center();
    visual.simulate_click(at, Default::default());
    visual.run_until_parked();
}

fn route(s: &Setup, visual: &mut VisualTestContext) -> Option<Route> {
    s.manager.read_with(visual, |m, _| m.route.clone())
}

fn the_run(s: &Setup, cx: &mut TestAppContext) -> FlowRun {
    let goals = pilot_state(s, cx).goals;
    let id = goals
        .iter()
        .flat_map(|g| g.runs.iter())
        .last()
        .expect("a scheduled run")
        .clone();
    s.window
        .update(cx, |shell, _, _| {
            let store = shell.pilot_flows_store().unwrap();
            let store = store.lock().unwrap();
            store.run(&id).unwrap().unwrap()
        })
        .unwrap()
}

/// Claims due habits and lets their start settle.
fn tick(s: &Setup, cx: &mut TestAppContext) {
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_habits(cx)));
    cx.run_until_parked();
}

#[gpui::test]
fn list_rows_follow_the_board_and_their_controls_act_in_place(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut nightly = habit("nightly", Target::Instruction, Tell::OnlyIfNeeded);
    nightly.runs.push(Run {
        at: now_ms() - 7 * 3_600_000,
        duration_ms: 180_000,
        outcome: "posted".into(),
        report: Some("Nothing to ship".into()),
        error: None,
    });
    let mut prune = habit("prune", Target::Instruction, Tell::EveryRun);
    prune.enabled = false;
    seed(&s, vec![prune, nightly], cx);
    let visual = &mut schedule_tab(&s, None, cx);

    let list = bounds(visual, "pilot-habits-list");
    let first = bounds(visual, "pilot-habit-nightly");
    let second = bounds(visual, "pilot-habit-prune");
    // `.side-row` inside the panel's 12px padding: 10px around three lines
    // of 18, 17 and 16px with 3px gaps.
    assert_eq!(first.left(), list.left() + px(12.));
    assert_eq!(first.right(), list.right() - px(12.));
    assert_eq!(first.size.height, px(77.));
    assert_eq!(second.top(), first.bottom(), "paused runs list last");
    let add = bounds(visual, "pilot-new-habit");
    assert_eq!(add.size.height, px(26.));
    assert_eq!(add.right(), first.right() - px(4.));
    let play = bounds(visual, "pilot-habit-run-nightly");
    let toggle = bounds(visual, "pilot-habit-toggle-nightly");
    assert_eq!((play.size.width, play.size.height), (px(26.), px(26.)));
    assert_eq!((toggle.size.width, toggle.size.height), (px(30.), px(18.)));
    assert_eq!(toggle.right(), first.right() - px(10.));
    assert_eq!(play.right() + px(8.), toggle.left());
    assert_eq!(play.top(), first.top() + px(10.));
    assert!(bounds(visual, "pilot-habits-note").top() >= second.bottom());

    // The switch pauses in place; it never opens the run.
    click(visual, "pilot-habit-toggle-nightly");
    assert!(
        !saved(&s)
            .iter()
            .find(|h| h.id == "nightly")
            .unwrap()
            .enabled
    );
    assert_eq!(route(&s, visual), None);
    // Play runs it now, with the prompt its "Tell me" asks for.
    set_main_busy(&s, true, visual);
    click(visual, "pilot-habit-run-nightly");
    assert_eq!(route(&s, visual), None);
    let prompts = s.manager.read_with(visual, |m, _| m.hidden_prompts.clone());
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].1.contains("Habit nightly"));
    assert!(prompts[0].1.contains("NOTHING_TO_REPORT"));
    // The row itself opens the edit sheet.
    click(visual, "pilot-habit-prune");
    assert_eq!(route(&s, visual), Some(Route::Habit("prune".into())));
}

#[gpui::test]
fn the_add_sheet_lays_out_like_the_board_and_saves_one_step(cx: &mut TestAppContext) {
    let s = setup(cx);
    let flow = save_flow(&s, cx);
    let visual = &mut schedule_tab(&s, Some(Route::NewHabit), cx);

    let editor = bounds(visual, "pilot-habit-editor");
    let name = bounds(visual, "pilot-habit-name-field");
    // The panel's 12px and each section's 4px.
    assert_eq!(name.left(), editor.left() + px(16.));
    assert_eq!(name.right(), editor.right() - px(16.));
    assert_eq!(name.size.height, px(32.));
    let cards = ["pipeline", "step", "instruction"]
        .map(|value| bounds(visual, &format!("pilot-habit-target-{value}")));
    for (ix, card) in cards.iter().enumerate() {
        assert_eq!(card.left(), name.left());
        assert_eq!(card.right(), name.right());
        // 9px padding around 18 + 2 + 16, and a 1px border.
        assert_eq!(card.size.height, px(56.), "card {ix}");
        if ix > 0 {
            assert_eq!(card.top(), cards[ix - 1].bottom() + px(6.));
        }
    }
    let kind = bounds(visual, "pilot-habit-kind");
    assert_eq!((kind.left(), kind.right()), (name.left(), name.right()));
    let time = bounds(visual, "pilot-habit-time-picker");
    let zone = bounds(visual, "pilot-habit-zone");
    assert_eq!(time.size.height, px(30.));
    assert_eq!(time.top(), kind.bottom() + px(12.));
    assert!((time.size.width - zone.size.width).abs() <= px(1.));
    assert_eq!(zone.left(), time.right() + px(6.));
    assert_eq!(zone.right(), name.right());
    assert!(visual.debug_bounds("pilot-habit-prompt-field").is_none());
    assert!(visual.debug_bounds("pilot-habit-step-picker").is_none());
    let save = bounds(visual, "pilot-habit-save");
    assert_eq!(save.size.height, px(30.));
    assert!(save.top() > bounds(visual, "pilot-habit-tell").bottom());
    // New runs start as the board's: the whole pipeline, nightly, telling
    // only when needed.
    let tell = s
        .manager
        .read_with(visual, |m, cx| m.value("habit-tell", cx));
    assert_eq!(tell, "onlyIfNeeded");

    // Without a name nothing is saved and the sheet stays.
    click(visual, "pilot-habit-save");
    assert!(visual.debug_bounds("pilot-habit-error").is_some());
    assert_eq!(route(&s, visual), Some(Route::NewHabit));
    assert!(saved(&s).is_empty());

    // An instruction shows its fixed 58px box; one step shows the steps.
    click(visual, "pilot-habit-target-instruction");
    assert_eq!(
        bounds(visual, "pilot-habit-prompt-field").size.height,
        px(58.)
    );
    click(visual, "pilot-habit-target-step");
    assert!(visual.debug_bounds("pilot-habit-prompt-field").is_none());
    click(visual, "pilot-habit-step-picker");
    click(visual, "menu-action-build");
    click(visual, "pilot-habit-kind-weekly");
    let day = bounds(visual, "pilot-habit-weekday-picker");
    assert_eq!(day.left(), name.left());
    click(visual, "pilot-habit-tell-everyRun");
    s.manager.update(visual, |m, cx| {
        m.fields["habit-name"].update(cx, |f, cx| f.set_text("Weekly build".into(), cx));
    });
    click(visual, "pilot-habit-save");

    let all = saved(&s);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "Weekly build");
    assert_eq!(
        all[0].target,
        Target::Step {
            flow_id: flow,
            step_id: "build".into()
        }
    );
    assert_eq!(all[0].tell, Tell::EveryRun);
    assert_eq!(all[0].schedule.schedule_kind, "weekly");
    assert_eq!(all[0].schedule.time, "02:00");
    assert_eq!(all[0].instructions, "");
    assert_eq!(route(&s, visual), None);
}

#[gpui::test]
fn the_edit_sheet_saves_changes_and_runs_pauses_and_deletes(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut h = habit("nightly", Target::Instruction, Tell::EveryRun);
    h.runs.push(Run {
        at: now_ms() - 60_000,
        duration_ms: 180_000,
        outcome: "posted".into(),
        report: Some("**Asked you** to plan 0.2.1".into()),
        error: None,
    });
    let next = h.next_run_at;
    let at = h.runs[0].at;
    seed(&s, vec![h], cx);
    let visual = &mut schedule_tab(&s, Some(Route::Habit("nightly".into())), cx);

    // Filled in from the saved run.
    let (name, target, tell) = s.manager.read_with(visual, |m, cx| {
        (
            m.value("habit-name", cx),
            m.value("habit-target", cx),
            m.value("habit-tell", cx),
        )
    });
    assert_eq!(
        (name.as_str(), target.as_str(), tell.as_str()),
        ("Habit nightly", "instruction", "everyRun")
    );
    let prompt = bounds(visual, "pilot-habit-prompt-field");
    assert_eq!(prompt.size.height, px(58.));
    // Save, Run now and Pause on the left, Delete on the right edge.
    let editor = bounds(visual, "pilot-habit-editor");
    let save = bounds(visual, "pilot-habit-save");
    let run = bounds(visual, "pilot-habit-run");
    let pause = bounds(visual, "pilot-habit-toggle");
    let delete = bounds(visual, "pilot-habit-delete");
    assert_eq!(save.left(), editor.left() + px(16.));
    assert_eq!(run.left(), save.right() + px(8.));
    assert_eq!(pause.left(), run.right() + px(8.));
    assert_eq!(delete.right(), editor.right() - px(16.));
    for button in [save, run, pause, delete] {
        assert_eq!(button.size.height, px(30.));
        assert_eq!(button.top(), save.top());
    }
    // Last runs sit under the actions and open their report.
    let row = bounds(visual, &format!("pilot-habit-history-nightly-{at}"));
    assert!(row.top() > save.bottom());
    click(visual, &format!("pilot-habit-history-nightly-{at}"));
    assert!(
        visual
            .debug_bounds(Box::leak(
                format!("pilot-habit-report-nightly-{at}").into_boxed_str()
            ))
            .is_some()
    );

    // Saving the same schedule keeps the next run.
    click(visual, "pilot-habit-tell-onlyIfNeeded");
    s.manager.update(visual, |m, cx| {
        m.fields["habit-name"].update(cx, |f, cx| f.set_text("Nightly check".into(), cx));
    });
    click(visual, "pilot-habit-save");
    let h = &saved(&s)[0];
    assert_eq!(h.name, "Nightly check");
    assert_eq!(h.tell, Tell::OnlyIfNeeded);
    assert_eq!(h.target, Target::Instruction);
    assert_eq!(h.next_run_at, next);
    assert_eq!(route(&s, visual), None);

    // Pause, Run now and Delete act on the open run.
    let reopened = saved(&s);
    s.manager.update(visual, |m, cx| {
        m.habits = reopened;
        m.route = Some(Route::Habit("nightly".into()));
        m.clear_habit_fields();
        cx.notify();
    });
    visual.run_until_parked();
    click(visual, "pilot-habit-toggle");
    assert!(!saved(&s)[0].enabled);
    assert_eq!(route(&s, visual), Some(Route::Habit("nightly".into())));
    set_main_busy(&s, true, visual);
    click(visual, "pilot-habit-run");
    assert_eq!(
        s.manager.read_with(visual, |m, _| m.hidden_prompts.len()),
        1,
        "Run now starts a paused run too"
    );
    click(visual, "pilot-habit-delete");
    assert!(saved(&s).is_empty());
    assert_eq!(route(&s, visual), None);
}

#[gpui::test]
fn a_scheduled_pipeline_starts_the_flow_and_tells_only_if_it_needs_you(cx: &mut TestAppContext) {
    let s = setup(cx);
    let flow = save_flow(&s, cx);
    hold_flow_launches(&s, cx);
    hold_outbox(&s, cx);
    let mut quiet = habit(
        "quiet",
        Target::Pipeline {
            flow_id: flow.clone(),
        },
        Tell::OnlyIfNeeded,
    );
    quiet.next_run_at = now_ms() - 1_000;
    let told = habit(
        "told",
        Target::Pipeline {
            flow_id: flow.clone(),
        },
        Tell::EveryRun,
    );
    seed(&s, vec![quiet, told], cx);

    tick(&s, cx);
    // The Flow ran in place of a hidden instruction session.
    assert!(cx.update(|cx| s.manager.read(cx).hidden_prompts.is_empty()));
    let run = the_run(&s, cx);
    assert_eq!(run.flow_id, flow);
    assert_eq!(run.flow.initial, "plan");
    assert_eq!(run.flow.states.len(), 2, "the whole pipeline");
    let tag = run.tag.clone().unwrap();
    assert_eq!(tag.goal, "Scheduled: Habit quiet");
    assert!(saved(&s)[0].running_since.is_some());
    // It does not hold up the other schedules while it runs.
    assert!(cx.update(|cx| s.manager.read(cx).active_run.is_none()));

    flow_events(&s, vec![finished(&run.id, RunStatus::Succeeded)], cx);
    assert!(
        flow_updates(&s, cx).is_empty(),
        "a success that needs no one stays out of the chat"
    );
    let quiet = &saved(&s)[0];
    assert_eq!(quiet.runs[0].outcome, "quiet");
    assert!(quiet.running_since.is_none());

    // The same run told every time rolls up into the chat.
    habits::update(&s.root, &s.id, |all| {
        all[1].next_run_at = now_ms() - 1_000;
        Ok(())
    })
    .unwrap();
    tick(&s, cx);
    let run = the_run(&s, cx);
    assert_eq!(run.tag.clone().unwrap().goal, "Scheduled: Habit told");
    flow_events(&s, vec![finished(&run.id, RunStatus::Succeeded)], cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].contains("Scheduled: Habit told"),
        "{}",
        updates[0]
    );
    assert_eq!(saved(&s)[1].runs[0].outcome, "posted");
}

#[gpui::test]
fn a_failed_scheduled_run_is_told_even_when_only_needed(cx: &mut TestAppContext) {
    let s = setup(cx);
    let flow = save_flow(&s, cx);
    hold_flow_launches(&s, cx);
    hold_outbox(&s, cx);
    let mut h = habit(
        "nightly",
        Target::Pipeline { flow_id: flow },
        Tell::OnlyIfNeeded,
    );
    h.next_run_at = now_ms() - 1_000;
    seed(&s, vec![h], cx);
    tick(&s, cx);
    let run = the_run(&s, cx);
    flow_events(&s, vec![finished(&run.id, RunStatus::Failed)], cx);
    // A failure needs the person: the Pilot hears it at once.
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(!updates[0].contains("[Goal finished]"), "{}", updates[0]);
    // The run counts as running until the Pilot's turn on it ends.
    assert!(saved(&s)[0].running_since.is_some());
    take_flow_updates(&s, cx);
    set_main_busy(&s, false, cx);
    let h = &saved(&s)[0];
    assert_eq!(h.runs[0].outcome, "failed");
    assert!(h.running_since.is_none());
    assert!(
        flow_updates(&s, cx)
            .iter()
            .any(|u| u.contains("[Goal finished] Scheduled: Habit nightly")),
        "even a schedule told only when needed rolls up a failure"
    );
}

#[gpui::test]
fn one_step_runs_alone_and_a_missing_step_fails_out_loud(cx: &mut TestAppContext) {
    let s = setup(cx);
    let flow = save_flow(&s, cx);
    hold_flow_launches(&s, cx);
    set_main_busy(&s, true, cx);
    let mut build = habit(
        "build",
        Target::Step {
            flow_id: flow.clone(),
            step_id: "build".into(),
        },
        Tell::OnlyIfNeeded,
    );
    build.next_run_at = now_ms() - 1_000;
    let gone = habit(
        "gone",
        Target::Step {
            flow_id: flow,
            step_id: "deleted".into(),
        },
        Tell::OnlyIfNeeded,
    );
    seed(&s, vec![build, gone], cx);
    tick(&s, cx);
    let run = the_run(&s, cx);
    assert_eq!(run.flow.initial, "build");
    let ids: Vec<_> = run.flow.states.iter().map(|s| s.id().to_owned()).collect();
    assert_eq!(ids, ["build"], "only that step runs");
    assert_eq!(run.tag.unwrap().task, "Step build");

    // A step the Flow no longer has never starts: it is recorded and told.
    habits::update(&s.root, &s.id, |all| {
        all[1].next_run_at = now_ms() - 1_000;
        Ok(())
    })
    .unwrap();
    tick(&s, cx);
    let gone = &saved(&s)[1];
    assert_eq!(gone.runs[0].outcome, "failed");
    assert!(
        gone.runs[0]
            .error
            .as_deref()
            .unwrap()
            .contains("no longer in this Flow")
    );
    let reports = pilot_state(&s, cx).pending_reports;
    assert!(
        reports
            .iter()
            .any(|r| r.starts_with("**Scheduled run failed**: Habit gone")),
        "{reports:?}"
    );
}

#[gpui::test]
fn an_instruction_told_every_run_reports_even_with_nothing_to_say(cx: &mut TestAppContext) {
    let s = setup(cx);
    set_main_busy(&s, true, cx);
    for (tell, expected) in [
        (
            Tell::EveryRun,
            Some("**Habit report**\n\nNothing to report."),
        ),
        (Tell::OnlyIfNeeded, None),
    ] {
        let run = habit_run(&s, cx);
        cx.update(|cx| {
            s.manager.update(cx, |m, cx| {
                m.roster.get_mut(&s.id).unwrap().pending_reports.clear();
                m.running.get_mut(&run).unwrap().tell = tell;
                m.habit_changed(run, cx);
            })
        });
        cx.run_until_parked();
        let reports = pilot_state(&s, cx).pending_reports;
        assert_eq!(
            reports.first().map(String::as_str),
            expected,
            "{tell:?}: {reports:?}"
        );
    }
    let every = habit("a", Target::Instruction, Tell::EveryRun);
    let needed = habit("b", Target::Instruction, Tell::OnlyIfNeeded);
    assert!(!scheduler::instruction_prompt(&every).contains("NOTHING_TO_REPORT"));
    assert!(scheduler::instruction_prompt(&needed).contains("NOTHING_TO_REPORT"));
    assert!(scheduler::instruction_prompt(&every).contains(&every.instructions));
}
