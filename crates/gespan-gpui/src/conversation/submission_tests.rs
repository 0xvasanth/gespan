use super::*;
use crate::conversation::remote_plans::PlanAction;
use gespan_core::{store::Store, transcript::attachments};
use gpui::{TestAppContext, WindowHandle};
use std::sync::Mutex;

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    // Fail after saving the turn, without ever launching a real provider.
    let mut summary = store
        .create("codex", "/missing/optimistic-send-project")
        .unwrap();
    summary.provider = "fixture".into();
    cx.update(|cx| {
        input::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    })
}

#[gpui::test]
fn pilot_messages_steer_the_ready_provider_once_and_keep_their_send_time(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let (send, receive) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.summary.provider = "codex".into();
            view.configure_pilot(
                gespan_core::pilot::Pilot::default(),
                "/tmp".into(),
                Default::default(),
                cx,
            );
            view.busy = true;
            view.controls = Some(send);
            let message = gespan_core::pilot::Message {
                id: "follow-up".into(),
                text: "is".into(),
                sent_at: Some(1234),
                internal: false,
                error: None,
                attachments: vec!["/tmp/image.png".into()],
                report: Vec::new(),
            };
            assert!(
                !view.pilot_dispatch(&message, cx),
                "wait for provider readiness"
            );
            view.follow_ups.ready = true;
            assert!(view.pilot_dispatch(&message, cx));
            let control = receive.try_recv().unwrap();
            let Decision::Steer(steer) = control.decision else {
                panic!("expected a real steer");
            };
            assert_eq!(steer.id, message.id);
            assert_eq!(steer.text, "is");
            assert_eq!(steer.sent_at, Some(1234));
            assert_eq!(
                steer.attachments,
                vec![std::path::PathBuf::from("/tmp/image.png")]
            );
            assert!(view.busy);
            assert!(
                !view.pilot_dispatch(&message, cx),
                "do not send twice while acknowledgement is pending"
            );
            assert!(receive.try_recv().is_err());
            view.pilot_message_delivered(message.id.clone(), cx);
            assert!(
                !view.pilot_dispatch(&message, cx),
                "an acknowledged message cannot be resent"
            );
            let second = gespan_core::pilot::Message {
                id: "next".into(),
                ..message
            };
            assert!(view.pilot_dispatch(&second, cx));
            view.pilot_message_failed(second.id.clone(), "Could not steer".into());
            assert_eq!(
                view.pilot_take_failures(),
                vec![(second.id, "Could not steer".into())]
            );
            assert!(view.pilot_ui.sending.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn send_paints_before_storage_and_keeps_one_message_after_startup_failure(cx: &mut TestAppContext) {
    for text in [
        "First message".to_owned(),
        "🙂 café\n".repeat(400),
        String::new(),
    ] {
        let window = conversation(cx);
        let attachment = std::path::PathBuf::from("/missing/optimistic-attachment.png");
        let sent_at = window
            .update(cx, |view, _, cx| {
                view.attachments.push(attachment.clone());
                view.input
                    .update(cx, |input, cx| input.set_text(text.clone(), cx));
                view.send(cx);
                assert!(view.busy);
                assert!(view.summary.has_messages);
                assert!(view.input.read(cx).content.is_empty());
                assert!(view.attachments.is_empty());
                let rows = view.transcript.read(cx).source_rows();
                assert_eq!(
                    rows.iter()
                        .filter(|row| row.first && row.is_prompt())
                        .count(),
                    1
                );
                assert_eq!(
                    rows.iter()
                        .filter(|row| row.is_prompt())
                        .map(|row| row.text.as_str())
                        .collect::<String>(),
                    text
                );
                assert_eq!(rows.last().unwrap().role, "assistant");
                let manifest = rows
                    .iter()
                    .find(|row| row.role == attachments::ROLE)
                    .unwrap();
                assert_eq!(
                    attachments::paths(manifest).unwrap(),
                    std::slice::from_ref(&attachment)
                );
                assert!(
                    locked(&view.store)
                        .unwrap()
                        .load(view.summary.id)
                        .unwrap()
                        .rows
                        .is_empty(),
                    "the bubble must exist before the background save runs"
                );
                rows[0].sent_at
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                assert!(!view.busy);
                let rows = view.transcript.read(cx).source_rows();
                assert_eq!(
                    rows.iter()
                        .filter(|row| row.first && row.is_prompt())
                        .count(),
                    1
                );
                assert_eq!(
                    rows[0].sent_at, sent_at,
                    "acceptance keeps the original send time"
                );
                assert_eq!(rows.last().unwrap().role, "error");
                assert_eq!(
                    rows,
                    locked(&view.store)
                        .unwrap()
                        .load(view.summary.id)
                        .unwrap()
                        .rows
                );
                // A later send must append after the failed message, not replace it.
                view.input
                    .update(cx, |input, cx| input.set_text("Try again".into(), cx));
                view.send(cx);
                assert_eq!(
                    view.transcript
                        .read(cx)
                        .source_rows()
                        .iter()
                        .filter(|row| row.first && row.is_prompt())
                        .count(),
                    2
                );
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                let rows = view.transcript.read(cx).source_rows();
                assert_eq!(
                    rows.iter()
                        .filter(|row| row.first && row.is_prompt())
                        .count(),
                    2
                );
                assert_eq!(
                    rows,
                    locked(&view.store)
                        .unwrap()
                        .load(view.summary.id)
                        .unwrap()
                        .rows
                );
            })
            .unwrap();
    }
}

#[gpui::test]
fn worktree_setup_shows_the_prompt_immediately_and_preserves_it_on_failure(
    cx: &mut TestAppContext,
) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.settings.new_worktree = true;
            view.settings.plan = true;
            view.input.update(cx, |input, cx| {
                input.set_text("Start in a worktree".into(), cx)
            });
            view.send(cx);
            assert!(view.busy && view.preparing_worktree);
            assert_eq!(view.status, "Creating worktree…");
            // The new worktree's branch is named after this message.
            assert_eq!(view.branch_seed.as_deref(), Some("Start in a worktree"));
            assert!(view.input.read(cx).content.is_empty());
            assert!(!view.settings.plan);
            assert_eq!(
                view.transcript.read(cx).source_rows()[0].text,
                "Start in a worktree"
            );
            view.input
                .update(cx, |input, cx| input.set_text("Follow up".into(), cx));
            view.send(cx);
            assert_eq!(view.follow_ups.state.messages[0].text, "Follow up");
            assert_eq!(view.follow_ups.state.messages[0].mode, None);
            view.input
                .update(cx, |input, cx| input.set_text("Unsent draft".into(), cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(!view.busy && !view.preparing_worktree);
            assert_eq!(view.input.read(cx).content, "Unsent draft");
            assert!(view.follow_ups.state.paused);
            assert_eq!(view.follow_ups.state.messages[0].text, "Follow up");
            let rows = view.transcript.read(cx).source_rows();
            assert_eq!(rows[0].text, "Start in a worktree");
            assert_eq!(rows.last().unwrap().role, "error");
            assert_eq!(
                rows,
                locked(&view.store)
                    .unwrap()
                    .load(view.summary.id)
                    .unwrap()
                    .rows
            );
        })
        .unwrap();
}

#[gpui::test]
fn stopping_before_the_worker_runs_keeps_the_optimistic_message(cx: &mut TestAppContext) {
    for new_worktree in [false, true] {
        let window = conversation(cx);
        window
            .update(cx, |view, _, cx| {
                view.settings.new_worktree = new_worktree;
                view.input.update(cx, |input, cx| {
                    input.set_text("Keep this message".into(), cx)
                });
                view.send(cx);
                view.stop(cx);
                assert!(!view.busy);
                assert_eq!(
                    view.transcript.read(cx).source_rows()[0].text,
                    "Keep this message"
                );
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                assert!(!view.stopping);
                assert!(!view.preparing_worktree);
                let rows = view.transcript.read(cx).source_rows();
                assert_eq!(
                    rows.iter()
                        .filter(|row| row.first && row.is_prompt())
                        .count(),
                    1
                );
                assert_eq!(rows[0].text, "Keep this message");
                assert_eq!(
                    rows,
                    locked(&view.store)
                        .unwrap()
                        .load(view.summary.id)
                        .unwrap()
                        .rows
                );
            })
            .unwrap();
    }
}

#[gpui::test]
fn a_created_worktree_hands_its_first_message_to_branch_naming(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("project");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    std::fs::write(repo.join("file"), "x\n").unwrap();
    git(&["add", "file"]);
    git(&["commit", "-m", "init"]);
    let store = Store::open(Path::new(":memory:")).unwrap();
    // The worktree is created before the fixture provider fails the turn.
    let mut summary = store.create("codex", &repo.to_string_lossy()).unwrap();
    summary.provider = "fixture".into();
    let window = cx.update(|cx| {
        input::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    });
    window
        .update(cx, |view, _, cx| {
            view.settings.new_worktree = true;
            view.input.update(cx, |input, cx| {
                input.set_text("Fix the login flow".into(), cx)
            });
            view.send(cx);
            assert_eq!(view.branch_seed.as_deref(), Some("Fix the login flow"));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(
                view.settings.worktree_cwd.is_some(),
                "the worktree was created"
            );
            assert_eq!(view.branch_seed, None, "and handed its message to naming");
        })
        .unwrap();
}

#[gpui::test]
fn a_retry_after_a_failed_worktree_is_named_after_the_retry(cx: &mut TestAppContext) {
    // The project folder is missing, so the first worktree cannot be created.
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.settings.new_worktree = true;
            view.input
                .update(cx, |input, cx| input.set_text("First attempt".into(), cx));
            view.send(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.settings.worktree_cwd.is_none(), "creation failed");
            view.settings.new_worktree = true;
            view.input
                .update(cx, |input, cx| input.set_text("Second attempt".into(), cx));
            view.send(cx);
            assert_eq!(view.branch_seed.as_deref(), Some("Second attempt"));
        })
        .unwrap();
}

fn pilot_message(id: &str, internal: bool) -> gespan_core::pilot::Message {
    gespan_core::pilot::Message {
        id: id.into(),
        text: format!("{id} text"),
        sent_at: Some(1),
        internal,
        error: None,
        attachments: Vec::new(),
        report: Vec::new(),
    }
}

/// A Codex Pilot chat mid-turn, ready to take a steer, with `outbox` queued.
fn busy_pilot(
    view: &mut Conversation,
    outbox: Vec<gespan_core::pilot::Message>,
    cx: &mut Context<Conversation>,
) -> std::sync::mpsc::Receiver<Control> {
    let (send, receive) = std::sync::mpsc::channel();
    view.summary.provider = "codex".into();
    view.configure_pilot(
        gespan_core::pilot::Pilot {
            outbox,
            ..Default::default()
        },
        "/tmp".into(),
        Default::default(),
        cx,
    );
    view.busy = true;
    view.controls = Some(send);
    view.follow_ups.ready = true;
    receive
}

#[gpui::test]
fn pilot_follow_ups_wait_for_the_turn_when_the_setting_is_queue(cx: &mut TestAppContext) {
    let window = conversation(cx);
    cx.update(|cx| {
        cx.set_global(crate::settings::Preferences {
            follow_up: "queue".into(),
            ..Default::default()
        })
    });
    window
        .update(cx, |view, _, cx| {
            let message = pilot_message("next", false);
            let controls = busy_pilot(view, vec![message.clone()], cx);
            assert!(
                !view.pilot_dispatch(&message, cx),
                "Queue keeps the message for the next turn, as in a session"
            );
            assert!(controls.try_recv().is_err(), "nothing was steered");
            assert!(view.pilot_ui.sending.is_none());

            // Background work alone may never end (a dev server), so the
            // message joins the turn, as a session's does.
            view.background
                .tasks
                .push(gespan_core::provider::BackgroundTask {
                    id: "dev".into(),
                    description: "Dev server".into(),
                    agent: false,
                    command: Some("npm run dev".into()),
                    started_at: 0,
                    tool_id: None,
                    latest: None,
                    steps: 0,
                });
            assert!(view.pilot_dispatch(&message, cx));
            assert!(matches!(
                controls.try_recv().unwrap().decision,
                Decision::Steer(_)
            ));
        })
        .unwrap();
}

#[gpui::test]
fn stopping_a_pilot_turn_pauses_its_queue_at_the_first_waiting_message(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            let _controls = busy_pilot(
                view,
                vec![
                    pilot_message("report", true),
                    pilot_message("first", false),
                    pilot_message("second", false),
                ],
                cx,
            );
            view.stop(cx);
            let failures = view.pilot_take_failures();
            assert_eq!(
                failures
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>(),
                vec!["first"],
                "the first message the user queued holds the rest"
            );
            assert!(failures[0].1.contains("stopped"), "{}", failures[0].1);
        })
        .unwrap();
}

#[gpui::test]
fn a_provider_switch_stop_does_not_pause_the_pilot_queue(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            let _controls = busy_pilot(view, vec![pilot_message("first", false)], cx);
            view.settings.pending_provider_switch = Some(Default::default());
            view.stop(cx);
            assert!(
                view.pilot_take_failures().is_empty(),
                "the switch stops the turn so this message can go to the new provider"
            );
        })
        .unwrap();
}

#[gpui::test]
fn a_pilot_steer_the_turn_never_accepted_fails_instead_of_resending(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            let message = pilot_message("late", false);
            let _controls = busy_pilot(view, vec![message.clone()], cx);
            assert!(view.pilot_dispatch(&message, cx));
            assert_eq!(view.pilot_ui.sending.as_deref(), Some("late"));
            view.pilot_turn_ended();
            assert_eq!(
                view.pilot_take_failures(),
                vec![(
                    "late".into(),
                    "The turn ended before this message was accepted".into()
                )]
            );
            assert!(view.pilot_ui.sending.is_none());
            // An accepted steer leaves nothing to fail.
            let next = pilot_message("accepted", false);
            view.busy = true;
            assert!(view.pilot_dispatch(&next, cx));
            view.pilot_message_delivered(next.id.clone(), cx);
            view.pilot_turn_ended();
            assert!(view.pilot_take_failures().is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn the_pilot_send_button_stops_a_running_turn_when_nothing_is_typed(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            let _controls = busy_pilot(view, Vec::new(), cx);
            view.input
                .update(cx, |input, cx| input.set_text("steer this".into(), cx));
            assert!(!view.pilot_stoppable(cx), "typed text is sent, not a stop");
            view.input
                .update(cx, |input, cx| input.set_text(String::new(), cx));
            assert!(view.pilot_stoppable(cx));
            view.pilot_send_or_stop(cx);
            assert!(view.stopping && !view.busy);
            assert!(
                !view.pilot_stoppable(cx),
                "a stopping turn offers no second stop"
            );
        })
        .unwrap();
}

fn planned(cx: &mut TestAppContext) -> (WindowHandle<Conversation>, &'static str) {
    let plan = "# Login plan\n\n1. Add the form\n2. Wire auth";
    let rows = vec![
        gespan_core::transcript::Row {
            role: "user".into(),
            text: "Plan the login".into(),
            first: true,
            sent_at: None,
        },
        gespan_core::transcript::Row {
            role: gespan_core::plan::ROLE.into(),
            text: plan.into(),
            first: true,
            sent_at: None,
        },
    ];
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.settings.plan = true;
            locked(&view.store)
                .unwrap()
                .write_rows(view.summary.id, 0, &rows)
                .unwrap();
            view.transcript
                .update(cx, |transcript, cx| transcript.patch(0, rows, cx));
            view.summary.has_messages = true;
        })
        .unwrap();
    (window, plan)
}

/// Opens a saved chat from the store, as after a restart.
fn reopened(rows: Vec<gespan_core::transcript::Row>, cx: &mut TestAppContext) -> Option<String> {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            locked(&view.store)
                .unwrap()
                .write_rows(view.summary.id, 0, &rows)
                .unwrap();
            view.loaded = false;
            view.summary.has_messages = true;
            view.load(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(cx, |view, cx| {
            assert_eq!(
                view.transcript.read(cx).source_rows().len(),
                rows.len(),
                "the history loaded"
            );
            view.orchestration_plan
                .as_ref()
                .map(|plan| plan.title.clone())
        })
        .unwrap()
}

fn row(role: &str, text: &str) -> gespan_core::transcript::Row {
    gespan_core::transcript::Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}

const PLAN_REPLY: &str = "Here is the plan.\n<gespan_orchestration>{\"title\":\"Docs\",\"summary\":\"Files\",\"tasks\":[{\"id\":\"task-1\",\"title\":\"A\",\"prompt\":\"Write a\",\"files\":[\"docs\"],\"dependsOn\":[]}]}</gespan_orchestration>";

#[gpui::test]
fn an_unstarted_orchestration_plan_comes_back_as_a_card_on_reload(cx: &mut TestAppContext) {
    let rows = vec![row("user", "Split it up"), row("assistant", PLAN_REPLY)];
    assert_eq!(reopened(rows, cx).as_deref(), Some("Docs"));
}

#[gpui::test]
fn a_started_or_broken_orchestration_plan_stays_gone_on_reload(cx: &mut TestAppContext) {
    let started = vec![
        row("user", "Split it up"),
        row("assistant", PLAN_REPLY),
        row("pilot-system", "Run the approved plan"),
        row("assistant", "Delegating now."),
    ];
    assert_eq!(reopened(started, cx), None);

    let broken = vec![
        row("user", "Split it up"),
        row(
            "assistant",
            "<gespan_orchestration>{oops}</gespan_orchestration>",
        ),
    ];
    assert_eq!(reopened(broken, cx), None);
}

fn prompts(view: &Conversation, cx: &gpui::App) -> Vec<String> {
    view.transcript
        .read(cx)
        .source_rows()
        .iter()
        .filter(|row| row.is_prompt())
        .map(|row| row.text.clone())
        .collect()
}

#[gpui::test]
fn a_local_plan_card_opens_the_plan_and_builds_it_with_plan_mode_off(cx: &mut TestAppContext) {
    let (window, plan) = planned(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let open = visual.debug_bounds("plan-open").unwrap();
    visual.simulate_click(open.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, cx| {
            assert_eq!(
                view.remote_plans.reader_text(),
                Some(plan),
                "Open shows the plan"
            );
            assert!(!view.busy, "opening never sends");
            assert!(view.close_remote_plan(cx));
        })
        .unwrap();
    visual.run_until_parked();
    let build = visual.debug_bounds("plan-build").unwrap();
    visual.simulate_click(build.center(), gpui::Modifiers::default());
    window
        .update(visual, |view, _, cx| {
            assert!(!view.settings.plan, "the build may change files");
            assert_eq!(
                prompts(view, cx),
                [
                    "Plan the login".to_owned(),
                    gespan_core::plan::build_prompt(plan)
                ]
            );
        })
        .unwrap();
    // The fixture provider fails once the turn is saved; the plan stays built.
    visual.run_until_parked();
    window
        .update(visual, |view, window, cx| {
            assert!(!view.busy);
            let card = view.transcript.read(cx).plan_cards()[0].clone();
            assert_eq!(card.build_label(), "Built");
            view.local_plan_request(&card, PlanAction::Build, window, cx);
            view.local_plan_request(&card, PlanAction::BuildWith, window, cx);
            assert_eq!(prompts(view, cx).len(), 2, "a plan builds once");
            assert!(!view.menu_is(ComposerMenu::Model));
        })
        .unwrap();
}

#[gpui::test]
fn build_with_another_model_switches_the_provider_then_builds(cx: &mut TestAppContext) {
    let (window, plan) = planned(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let other = visual.debug_bounds("plan-build-with").unwrap();
    visual.simulate_click(other.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    window
        .update(visual, |view, window, cx| {
            assert!(view.menu_is(ComposerMenu::Model), "the model menu opens");
            assert_eq!(view.remote_plans.build_with.as_deref(), Some(plan));
            assert!(!view.busy, "nothing is sent until a model is picked");
            // Closing and reopening the menu the usual way forgets the plan.
            view.close_menu(window, cx);
            view.toggle_menu(ComposerMenu::Model, window, cx);
            assert_eq!(view.remote_plans.build_with, None);
            assert!(!view.build_plan_with(&ModelPicked(Kind::Claude, None), cx));
            assert!(!view.busy);
            view.close_menu(window, cx);
        })
        .unwrap();
    let other = visual.debug_bounds("plan-build-with").unwrap();
    visual.simulate_click(other.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, cx| {
            assert!(view.build_plan_with(
                &ModelPicked(Kind::Claude, Some("claude-opus-5-5".into())),
                cx
            ));
            assert_eq!(view.summary.provider, "claude");
            assert_eq!(view.model.as_deref(), Some("claude-opus-5-5"));
            assert!(view.busy);
            assert!(!view.settings.plan);
            assert_eq!(
                prompts(view, cx).last().unwrap(),
                &gespan_core::plan::build_prompt(plan)
            );
        })
        .unwrap();
}

fn asking(view: &mut Conversation) {
    view.set_inbox_ask(gespan_core::inbox::ask::Context {
        key: "linear:eng-1".into(),
        title: "Item".into(),
        provider: "linear".into(),
        ..Default::default()
    });
}

#[gpui::test]
fn an_ask_conversation_cannot_save_a_draft(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            asking(view);
            view.input.update(cx, |input, cx| {
                input.set_text("/draft Keep this for later".into(), cx)
            });
            view.send(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.follow_ups.state.draft.is_none());
            assert_eq!(view.status, "Ask conversations cannot save drafts.");
            assert_eq!(
                view.input.read(cx).content.as_ref(),
                "/draft Keep this for later",
                "the text stays in the composer"
            );
        })
        .unwrap();
    assert!(turn_capture::last().is_none());
}

#[gpui::test]
fn a_queued_ask_message_is_wrapped_once_when_it_launches(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            asking(view);
            let message = view.message("Queued question".into(), Vec::new(), None, cx);
            view.launch_message(message, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let (_, prompt) = turn_capture::last().unwrap();
    assert_eq!(prompt.matches("USER MESSAGE:").count(), 1);
    assert!(prompt.ends_with("USER MESSAGE:\nQueued question"));
}

#[gpui::test]
fn an_ask_steer_carries_the_wrapper_and_the_queue_keeps_the_typed_text(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            asking(view);
            view.summary.provider = "codex".into();
            view.busy = true;
            view.controls = Some(controls);
            view.follow_ups.ready = true;
            let mut message = view.message("Also check CI".into(), Vec::new(), None, cx);
            message.steer = true;
            view.follow_ups.state.messages.push(message);
            view.pump_follow_ups(cx);
            let queued = &view.follow_ups.state.messages[0];
            assert!(queued.prepared_prompt.is_none(), "only the agent's copy");
            view.busy = false;
            view.controls = None;
        })
        .unwrap();
    let Decision::Steer(steer) = received.try_recv().unwrap().decision else {
        panic!("expected a steer");
    };
    assert_eq!(steer.text, "Also check CI");
    let prompt = steer.prepared_prompt.unwrap();
    assert!(prompt.starts_with("# Inbox discussion instructions"));
    assert!(prompt.ends_with("USER MESSAGE:\nAlso check CI"));
}

#[gpui::test]
fn a_raw_native_command_reaches_the_agent_unwrapped(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, _| {
            asking(view);
            view.summary.provider = "omp".into();
            assert_eq!(view.ask_prompt("/compact now".into()), "/compact now");
            assert!(
                view.ask_prompt("compact now".into())
                    .contains("USER MESSAGE:")
            );
            view.summary.provider = "codex".into();
            assert!(
                view.ask_prompt("/compact now".into())
                    .contains("USER MESSAGE:")
            );
        })
        .unwrap();
}
