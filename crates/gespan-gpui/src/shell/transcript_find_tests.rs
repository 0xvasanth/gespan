//! Escape in a chat, through the shell's own `escape` binding. It outranks key
//! listeners, so a test without the shell around the session can't see this.
use super::*;
use base64::Engine;
use gespan_core::transcript::Row;
use gpui::{ExternalPaths, FileDropEvent, TestAppContext, VisualTestContext, WindowHandle};

fn row(role: &str, text: String) -> Row {
    Row {
        role: role.into(),
        text,
        first: true,
        sent_at: None,
    }
}

/// The real shell with a chat open: a message about the footer, an image, and
/// the composer focused.
fn chat(cx: &mut TestAppContext) -> (WindowHandle<Shell>, VisualTestContext, i64) {
    cx.update(|cx| {
        crate::input::bind(cx);
        crate::popover::bind(cx);
        crate::image_lightbox::bind(cx);
        crate::conversation::find::bind(cx);
        bind_keys(cx);
        cx.set_reduce_motion(true);
    });
    let (window, _, active) = super::tests::window(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let image = base64::engine::general_purpose::STANDARD.encode(br##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480"><rect width="640" height="480" fill="#4da3f5"/></svg>"##);
    let attachments = serde_json::json!([
        {"kind": "image", "name": "Screenshot", "mimeType": "image/svg+xml", "data": image}
    ]);
    window
        .update(&mut visual, |shell, w, cx| {
            let session = shell.sessions[&active].clone();
            session.update(cx, |view, cx| {
                view.set_loaded_for_test();
                view.summary.has_messages = true;
                view.transcript.update(cx, |t, cx| {
                    t.patch(
                        0,
                        vec![
                            row("user", "Review the footer".into()),
                            row(
                                gespan_core::transcript::attachments::ROLE,
                                attachments.to_string(),
                            ),
                        ],
                        cx,
                    )
                });
                cx.notify();
            });
            let composer = session.read(cx).input.focus_handle(cx);
            w.focus(&composer, cx);
        })
        .unwrap();
    visual.refresh().unwrap();
    visual.run_until_parked();
    (window, visual, active)
}

fn session(
    window: WindowHandle<Shell>,
    visual: &mut VisualTestContext,
    active: i64,
) -> Entity<Conversation> {
    window
        .update(visual, |shell, _, _| shell.sessions[&active].clone())
        .unwrap()
}

fn open_find(visual: &mut VisualTestContext) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    visual.simulate_keystrokes(&format!("{modifier}-f"));
    visual.simulate_input("footer");
    visual.run_until_parked();
}

fn press_escape(visual: &mut VisualTestContext) {
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
}

fn find_open(view: &Entity<Conversation>, visual: &mut VisualTestContext) -> bool {
    visual.update(|_, cx| view.read(cx).find_open())
}

fn composer_focused(view: &Entity<Conversation>, visual: &mut VisualTestContext) -> bool {
    visual.update(|w, cx| view.read(cx).input.focus_handle(cx).is_focused(w))
}

#[gpui::test]
fn escape_closes_find_and_returns_to_the_composer_without_stopping_the_turn(
    cx: &mut TestAppContext,
) {
    let (window, mut visual, active) = chat(cx);
    let view = session(window, &mut visual, active);
    open_find(&mut visual);
    visual.update(|_, cx| {
        assert_eq!(view.read(cx).find_state().0, [0]);
        assert_eq!(
            view.read(cx).transcript.read(cx).search_target(),
            Some((0, "footer".into()))
        );
        view.update(cx, |view, _| view.busy = true);
    });

    press_escape(&mut visual);

    assert!(!find_open(&view, &mut visual));
    assert!(composer_focused(&view, &mut visual));
    visual.update(|_, cx| {
        assert_eq!(view.read(cx).transcript.read(cx).search_target(), None);
        assert!(
            view.read(cx).busy,
            "closing the bar is all Escape does: the shell's `Session: Stop` waits"
        );
    });
}

#[gpui::test]
fn with_find_closed_escape_still_stops_the_turn(cx: &mut TestAppContext) {
    let (window, mut visual, active) = chat(cx);
    let view = session(window, &mut visual, active);
    open_find(&mut visual);
    press_escape(&mut visual);
    visual.update(|_, cx| view.update(cx, |view, _| view.busy = true));

    press_escape(&mut visual);

    visual.update(|_, cx| assert!(!view.read(cx).busy));
}

#[gpui::test]
fn the_lightbox_takes_escape_before_find_and_works_without_it(cx: &mut TestAppContext) {
    let (window, mut visual, active) = chat(cx);
    let view = session(window, &mut visual, active);
    let thumbnail = visual.debug_bounds("transcript-image-0").unwrap();

    visual.simulate_click(thumbnail.center(), Default::default());
    assert!(visual.debug_bounds("image-lightbox").is_some());
    press_escape(&mut visual);
    assert!(visual.debug_bounds("image-lightbox").is_none());
    assert!(!find_open(&view, &mut visual));

    open_find(&mut visual);
    visual.simulate_click(thumbnail.center(), Default::default());
    assert!(visual.debug_bounds("image-lightbox").is_some());
    press_escape(&mut visual);
    assert!(visual.debug_bounds("image-lightbox").is_none());
    assert!(find_open(&view, &mut visual), "one layer per press");

    press_escape(&mut visual);
    assert!(!find_open(&view, &mut visual));
}

#[gpui::test]
fn the_plan_reader_over_the_chat_closes_before_find(cx: &mut TestAppContext) {
    let (window, mut visual, active) = chat(cx);
    let view = session(window, &mut visual, active);
    open_find(&mut visual);
    visual.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.open_plan_reader_for_test("The plan", cx)
        })
    });
    visual.run_until_parked();

    press_escape(&mut visual);
    visual.update(|_, cx| assert!(!view.read(cx).plan_reader_open()));
    assert!(find_open(&view, &mut visual), "one layer per press");

    press_escape(&mut visual);
    assert!(!find_open(&view, &mut visual));
}

#[gpui::test]
fn escape_cancels_a_file_drag_before_it_touches_find(cx: &mut TestAppContext) {
    let (window, mut visual, active) = chat(cx);
    let view = session(window, &mut visual, active);
    open_find(&mut visual);
    let over_transcript = visual.debug_bounds("transcript-image-0").unwrap().center();
    visual.simulate_event(FileDropEvent::Entered {
        position: over_transcript,
        paths: ExternalPaths(vec![std::path::PathBuf::from("/work/notes.txt")].into()),
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert!(view.read(cx).file_drag_for_test());
        assert!(cx.has_active_drag());
    });

    press_escape(&mut visual);

    visual.update(|_, cx| {
        assert!(!view.read(cx).file_drag_for_test());
        assert!(!cx.has_active_drag());
    });
    assert!(find_open(&view, &mut visual), "one layer per press");

    press_escape(&mut visual);
    assert!(!find_open(&view, &mut visual));
}

#[gpui::test]
fn escape_closes_find_in_a_pilot_chat(cx: &mut TestAppContext) {
    let (window, mut visual, active) = chat(cx);
    let view = session(window, &mut visual, active);
    visual.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.settings.pilot = Some(gespan_core::agent::PilotContext {
                pilot: Default::default(),
                root: "/work/gespan".into(),
                executable: "/work/gespan/binary".into(),
            });
            cx.notify();
        })
    });
    visual.refresh().unwrap();
    visual.run_until_parked();
    open_find(&mut visual);
    assert!(find_open(&view, &mut visual));

    press_escape(&mut visual);

    assert!(!find_open(&view, &mut visual));
    assert!(composer_focused(&view, &mut visual));
}
