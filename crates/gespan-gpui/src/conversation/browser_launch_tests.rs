//! Which launches count as the user's own: only those can carry the browser.
//! The mark is per launch (and per queued message), so a thread's settings
//! can never turn a later continuation, resume or app send into one.
use super::*;
use gespan_core::store::Store;
use gpui::{TestAppContext, WindowHandle};
use std::sync::Mutex;

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    // Fails after saving the turn, without ever launching a real provider.
    let mut summary = store.create("codex", "/missing/browser-launch").unwrap();
    summary.provider = "fixture".into();
    cx.update(|cx| {
        input::bind(cx);
        cx.set_global(crate::settings::Preferences {
            follow_up: "queue".into(),
            ..Default::default()
        });
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view.summary.has_messages = true;
                view
            })
        })
        .unwrap()
    })
}

/// Whether the launch of `prompt` was marked as typed by the user.
fn launched_typed(prompt: &str) -> bool {
    let launches = turn_capture::typed();
    launches
        .iter()
        .rev()
        .find(|(sent, _)| sent == prompt)
        .unwrap_or_else(|| panic!("{prompt:?} never launched: {launches:?}"))
        .1
}

/// The turn after Gespan quit mid-turn: the transcript ends interrupted.
fn interrupted(view: &mut Conversation, cx: &mut Context<Conversation>) {
    let mut transcript = gespan_core::transcript::Transcript::default();
    transcript.begin("user");
    transcript.append("Fix the build");
    transcript.begin("system");
    transcript.append(gespan_core::interrupted::MESSAGE);
    view.transcript
        .update(cx, |t, cx| t.replace_history(transcript.rows, cx));
}

#[gpui::test]
fn browser_counts_a_composer_typed_turn_as_the_users(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            view.input
                .update(cx, |input, cx| input.set_text("Open the docs".into(), cx));
            // Enter in the composer.
            view.input.update(cx, |_, cx| cx.emit(input::Submitted));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(launched_typed("Open the docs"));
}

#[gpui::test]
fn browser_stays_out_of_automation_and_agent_sends(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            // An automation's first turn, and a Flow follow-up.
            view.quick_submit("Nightly triage".into(), Settings::default(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!launched_typed("Nightly triage"));
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            // `sessions.send` from an agent, with the thread's settings.
            view.install_transfer(view.settings.clone(), Some("Agent says hi".into()), cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!launched_typed("Agent says hi"));
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            // The quick composer: the user typed it elsewhere.
            view.user_quick_submit("From the quick composer".into(), Settings::default(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(launched_typed("From the quick composer"));
}

#[gpui::test]
fn browser_stays_out_of_the_continuation_after_quit(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            interrupted(view, cx);
            // The thread's last turn was the user's: nothing carries over.
            view.settings.user_typed = true;
            view.continue_after_quit(cx);
            assert!(view.busy, "the continuation starts a turn");
        })
        .unwrap();
    cx.run_until_parked();
    let launches = turn_capture::typed();
    assert_eq!(launches.len(), 1, "{launches:?}");
    assert!(!launches[0].1, "{launches:?}");
}

#[gpui::test]
fn browser_stays_out_of_a_usage_limit_resume(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            view.settings.user_typed = true;
            view.settings.usage_limit = Some(gespan_core::usage::Limit {
                resets_at: Some(0),
                resume_at_reset: true,
            });
            // What the `resume_due` tick runs.
            view.tick_usage(cx);
            assert!(view.busy, "the resume starts a turn");
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!launched_typed("Continue from where you left off."));
}

#[gpui::test]
fn browser_stays_out_of_a_hold_resume_of_an_interrupted_turn(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            view.settings.user_typed = true;
            view.held = Some(gespan_core::hold::Reason::Interrupted);
            assert!(view.resume_from_hold(gespan_core::hold::Reason::Interrupted, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!launched_typed("Continue from where you left off."));
}

#[gpui::test]
fn browser_marks_follow_each_queued_message_not_the_thread(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            // The user types while a turn runs, then an agent sends, then the
            // user types again: all three wait in the queue.
            view.input
                .update(cx, |input, cx| input.set_text("Mine first".into(), cx));
            view.send_typed(cx);
            view.install_transfer(view.settings.clone(), Some("From an agent".into()), cx);
            view.input
                .update(cx, |input, cx| input.set_text("Mine again".into(), cx));
            view.send_typed(cx);
            let queued = view
                .follow_ups
                .state
                .messages
                .iter()
                .map(|message| (message.text.clone(), message.typed))
                .collect::<Vec<_>>();
            assert_eq!(
                queued,
                [
                    ("Mine first".to_owned(), true),
                    ("From an agent".to_owned(), false),
                    ("Mine again".to_owned(), true),
                ]
            );
            // Each goes out as itself when its turn comes.
            for message in std::mem::take(&mut view.follow_ups.state.messages) {
                view.busy = false;
                view.launch_message(message, cx);
            }
        })
        .unwrap();
    cx.run_until_parked();
    assert!(launched_typed("Mine first"));
    assert!(!launched_typed("From an agent"));
    assert!(launched_typed("Mine again"));
}

#[gpui::test]
fn browser_marks_survive_a_hold_but_never_a_restore_from_disk(cx: &mut TestAppContext) {
    let window = conversation(cx);
    turn_capture::start();
    cx.update(|cx| {
        cx.set_global(crate::hold::Hold(Some(gespan_core::hold::HoldState::new(
            1,
        ))))
    });
    window
        .update(cx, |view, _, cx| {
            view.input.update(cx, |input, cx| {
                input.set_text("Typed during hold".into(), cx)
            });
            view.send_typed(cx);
            assert!(!view.busy, "a hold keeps it queued");
            assert!(view.follow_ups.state.messages[0].typed);
            // The queue saved to disk never keeps the mark.
            let saved = serde_json::to_value(&view.follow_ups.state).unwrap();
            let restored: gespan_core::follow_up::State = serde_json::from_value(saved).unwrap();
            assert!(!restored.messages[0].typed);
        })
        .unwrap();
}
