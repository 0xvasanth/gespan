//! Dictation in the Pilot chat composer: the same mic, caption row, shortcut
//! and Escape as a session composer, over the shared speech hub.
use super::tests::{Setup, setup};
use crate::conversation::Conversation;
use crate::conversation::dictation::{
    self, DictationState, HOLD_TO_TALK, tests::make_ready, with_sink,
};
use gespan_models::ModelStore;
use gespan_speech::{
    catalog,
    protocol::{Event, Request},
};
use gpui::{
    Entity, Focusable, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, px,
    size,
};
use std::{cell::RefCell, rc::Rc, time::Duration};

const SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd-shift-m"
} else {
    "ctrl-shift-m"
};

/// The app's speech hub over a temporary model folder, recording requests.
/// Install it before `setup()` so the Pilot chat's dictation joins it.
pub(super) struct Speech {
    _dir: tempfile::TempDir,
    sent: Rc<RefCell<Vec<Request>>>,
    hub: Entity<dictation::Speech>,
}

impl Speech {
    pub(super) fn new(ready: bool, cx: &mut TestAppContext) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if ready {
            make_ready(dir.path(), &catalog::PARAKEET_V2_EN);
        }
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let store = ModelStore::new(dir.path().to_path_buf());
        let hub = cx.update(|cx| {
            crate::input::bind(cx);
            crate::popover::bind(cx);
            crate::shell::bind_keys(cx);
            cx.set_reduce_motion(true);
            with_sink(store, move |r| log.borrow_mut().push(r), cx)
        });
        Self {
            _dir: dir,
            sent,
            hub,
        }
    }

    pub(super) fn emit(&self, event: Event, cx: &mut VisualTestContext) {
        self.hub.read_with(cx, |s, _| s.emit(event));
        cx.run_until_parked();
    }

    pub(super) fn take(&self) -> Vec<Request> {
        self.sent.borrow_mut().drain(..).collect()
    }
}

/// The open Pilot's chat with its settings dismissed and its field focused.
pub(super) fn pilot_chat(
    s: &Setup,
    cx: &mut TestAppContext,
) -> (VisualTestContext, Entity<Conversation>) {
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.dismiss_settings(w, cx));
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.run_until_parked();
    let chat = visual.update(|_, cx| s.manager.read(cx).chat().unwrap());
    focus(&chat, &mut visual);
    (visual, chat)
}

pub(super) fn focus(chat: &Entity<Conversation>, visual: &mut VisualTestContext) {
    visual.update(|w, cx| w.focus(&chat.read(cx).input.focus_handle(cx), cx));
    visual.run_until_parked();
}

pub(super) fn state(chat: &Entity<Conversation>, cx: &mut VisualTestContext) -> DictationState {
    chat.read_with(cx, |c, cx| c.dictation.read(cx).state().clone())
}

pub(super) fn text(chat: &Entity<Conversation>, cx: &mut VisualTestContext) -> String {
    chat.read_with(cx, |c, cx| c.input.read(cx).content.to_string())
}

pub(super) fn final_text(text: &str) -> Event {
    Event::Final {
        text: text.into(),
        capped: false,
    }
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    visual.refresh().unwrap();
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
}

fn bounds(
    visual: &mut VisualTestContext,
    selector: &'static str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    visual.refresh().unwrap();
    visual.debug_bounds(selector)
}

#[gpui::test]
fn pilot_composer_shows_the_mic_with_its_tools(cx: &mut TestAppContext) {
    let _speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, _chat) = pilot_chat(&s, cx);
    let visual = &mut visual;
    let mic = bounds(visual, "composer-mic").expect("the Pilot composer has a mic");
    let field = bounds(visual, "pilot-composer").unwrap();
    let attach = bounds(visual, "pilot-attach").unwrap();
    let model = bounds(visual, "pilot-model").unwrap();
    // On the tools row under the box, after attach and emoji, before the model.
    assert!(mic.top() >= field.bottom(), "{mic:?} vs {field:?}");
    assert!(mic.left() > attach.right());
    assert!(model.left() > mic.right());
    assert_eq!(mic.size, size(px(26.), px(26.)));
}

#[gpui::test]
fn pilot_mic_dictates_into_the_pilot_field(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, chat) = pilot_chat(&s, cx);
    let visual = &mut visual;
    let rows = chat.read_with(visual, |c, cx| c.pilot_rows(cx).len());

    click(visual, "composer-mic");
    assert!(
        speech
            .take()
            .iter()
            .any(|r| matches!(r, Request::Start { .. }))
    );
    speech.emit(Event::Listening, visual);
    assert_eq!(state(&chat, visual), DictationState::Listening);
    // The caption row sits inside the Pilot's box, as in a session.
    let field = bounds(visual, "pilot-composer").unwrap();
    let bar = bounds(visual, "dictation-bar").expect("the caption row shows");
    assert!(field.contains(&bar.center()), "{bar:?} outside {field:?}");
    speech.emit(Event::Level { rms: 0.2 }, visual);
    speech.emit(Event::Partial { text: "hel".into() }, visual);
    assert!(bounds(visual, "dictation-partial").is_some());
    assert_eq!(text(&chat, visual), "", "a partial is only a caption");

    click(visual, "composer-mic");
    assert_eq!(speech.take(), vec![Request::Stop]);
    // The agent got busy meanwhile: the final text still lands.
    chat.update(visual, |c, cx| {
        c.input.update(cx, |i, cx| {
            i.set_read_only(true);
            cx.notify();
        })
    });
    speech.emit(final_text("hello pilot"), visual);
    assert_eq!(text(&chat, visual), "hello pilot");
    assert_eq!(state(&chat, visual), DictationState::Idle);
    // Dictating never sends.
    chat.read_with(visual, |c, cx| {
        assert_eq!(c.pilot_rows(cx).len(), rows);
        assert!(c.pilot_delivered().is_empty());
    });
}

#[gpui::test]
fn pilot_shortcut_tap_toggles_and_hold_stops(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, chat) = pilot_chat(&s, cx);
    let visual = &mut visual;

    // Tap: press and release at once starts, and keeps listening.
    visual.simulate_keystrokes(SHORTCUT);
    assert_eq!(state(&chat, visual), DictationState::Starting);
    visual.simulate_modifiers_change(Modifiers::none());
    speech.emit(Event::Listening, visual);
    assert_eq!(state(&chat, visual), DictationState::Listening);
    speech.take();
    // Tap again: stops and inserts.
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_modifiers_change(Modifiers::none());
    assert_eq!(state(&chat, visual), DictationState::Finishing);
    assert_eq!(speech.take(), vec![Request::Stop]);
    speech.emit(final_text("tapped"), visual);
    assert_eq!(text(&chat, visual), "tapped");

    // Hold: key repeats are not presses, and letting go after 400 ms stops.
    visual.simulate_keystrokes(SHORTCUT);
    speech.emit(Event::Listening, visual);
    visual.executor().advance_clock(Duration::from_millis(250));
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_keystrokes(SHORTCUT);
    assert_eq!(state(&chat, visual), DictationState::Listening);
    assert!(!speech.take().contains(&Request::Stop));
    visual.executor().advance_clock(HOLD_TO_TALK);
    visual.simulate_modifiers_change(Modifiers::none());
    assert_eq!(state(&chat, visual), DictationState::Finishing);
    assert_eq!(speech.take(), vec![Request::Stop]);
    speech.emit(final_text("held"), visual);
    assert_eq!(text(&chat, visual), "tapped held");

    // Where a key-up arrives, it ends a hold too.
    visual.simulate_keystrokes(SHORTCUT);
    speech.emit(Event::Listening, visual);
    speech.take();
    visual.executor().advance_clock(Duration::from_millis(600));
    visual.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("m").unwrap(),
    });
    assert_eq!(speech.take(), vec![Request::Stop]);
}

#[gpui::test]
fn pilot_escape_cancels_dictation_before_menus(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, chat) = pilot_chat(&s, cx);
    let visual = &mut visual;
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_modifiers_change(Modifiers::none());
    speech.emit(Event::Listening, visual);
    speech.take();
    // The model list opens over the composer while dictation runs.
    visual.update(|w, cx| chat.update(cx, |c, cx| c.toggle_model_menu(w, cx)));
    visual.run_until_parked();
    assert_eq!(
        chat.read_with(visual, |c, _| c.pilot_menus_open()),
        (true, false)
    );
    assert_eq!(state(&chat, visual), DictationState::Listening);

    visual.simulate_keystrokes("escape");
    assert_eq!(state(&chat, visual), DictationState::Idle);
    assert_eq!(speech.take(), vec![Request::Cancel]);
    assert_eq!(
        chat.read_with(visual, |c, _| c.pilot_menus_open()),
        (true, false),
        "the list stays open: Escape went to dictation"
    );

    // The list has no Escape of its own: with nothing to cancel, a second
    // Escape sends nothing and the list stays.
    visual.simulate_keystrokes("escape");
    assert!(speech.take().is_empty());
    assert_eq!(state(&chat, visual), DictationState::Idle);
}

#[gpui::test]
fn pilot_without_a_model_shows_the_download_prompt(cx: &mut TestAppContext) {
    let speech = Speech::new(false, cx);
    let s = setup(cx);
    let (mut visual, chat) = pilot_chat(&s, cx);
    let visual = &mut visual;
    click(visual, "composer-mic");
    assert_eq!(state(&chat, visual), DictationState::NeedsModel);
    assert!(bounds(visual, "dictation-download").is_some());
    assert!(speech.take().is_empty(), "nothing starts without a model");

    visual.simulate_keystrokes("escape");
    assert_eq!(state(&chat, visual), DictationState::Idle);
    assert!(bounds(visual, "dictation-download").is_none());
}

#[gpui::test]
fn pilot_escape_cancels_dictation_before_a_composer_menu(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, chat) = pilot_chat(&s, cx);
    let visual = &mut visual;
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_modifiers_change(Modifiers::none());
    speech.emit(Event::Listening, visual);
    speech.take();
    // The recent models menu takes focus and closes on Escape itself.
    visual.update(|w, cx| chat.update(cx, |c, cx| c.toggle_recent_models(w, cx)));
    visual.run_until_parked();
    assert_eq!(
        chat.read_with(visual, |c, _| c.pilot_menus_open()),
        (false, true)
    );

    visual.simulate_keystrokes("escape");
    assert_eq!(state(&chat, visual), DictationState::Idle);
    assert_eq!(speech.take(), vec![Request::Cancel]);
    assert_eq!(
        chat.read_with(visual, |c, _| c.pilot_menus_open()),
        (false, true),
        "the menu stays open: Escape went to dictation"
    );

    // With nothing to cancel, the next Escape reaches the menu.
    visual.simulate_keystrokes("escape");
    assert_eq!(
        chat.read_with(visual, |c, _| c.pilot_menus_open()),
        (false, false)
    );
    assert!(speech.take().is_empty());
}

// One dictation at a time across Pilot and session composers.

/// The window's session (not a Pilot's), opened, with its field focused.
fn open_session(s: &Setup, visual: &mut VisualTestContext) -> Entity<Conversation> {
    let session = s
        .window
        .update(visual, |shell, w, cx| {
            let id = shell.workspace.active.expect("the window has a session");
            shell.open(id, w, cx);
            shell.session_view(id).unwrap()
        })
        .unwrap();
    visual.run_until_parked();
    focus(&session, visual);
    session
}

fn open_pilot(s: &Setup, id: &str, visual: &mut VisualTestContext) -> Entity<Conversation> {
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(visual, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.open(id, w, cx);
                m.dismiss_settings(w, cx);
            })
        })
        .unwrap();
    visual.run_until_parked();
    let chat = visual.update(|_, cx| s.manager.read(cx).open_chat(id).unwrap());
    focus(&chat, visual);
    chat
}

/// Starts dictating in `chat` through the shortcut, as the user would.
fn listen(speech: &Speech, chat: &Entity<Conversation>, visual: &mut VisualTestContext) {
    focus(chat, visual);
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_modifiers_change(Modifiers::none());
    speech.emit(Event::Listening, visual);
    assert_eq!(state(chat, visual), DictationState::Listening);
    speech.take();
}

/// The helper's tail of a cancelled take, still in flight.
fn late_final(speech: &Speech, visual: &mut VisualTestContext) {
    speech.emit(final_text("stray"), visual);
    speech.emit(Event::Cancelled, visual);
}

#[gpui::test]
fn opening_a_pilot_cancels_session_dictation(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, _) = pilot_chat(&s, cx);
    let visual = &mut visual;
    let session = open_session(&s, visual);
    assert_ne!(session.read_with(visual, |c, _| c.summary.id), s.session);
    listen(&speech, &session, visual);

    let pilot = open_pilot(&s, &s.id, visual);
    assert_eq!(state(&session, visual), DictationState::Idle);
    assert_eq!(speech.take(), vec![Request::Cancel]);
    late_final(&speech, visual);
    assert_eq!(text(&session, visual), "");
    assert_eq!(text(&pilot, visual), "");
}

#[gpui::test]
fn opening_a_session_cancels_pilot_dictation(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, pilot) = pilot_chat(&s, cx);
    let visual = &mut visual;
    listen(&speech, &pilot, visual);

    let session = open_session(&s, visual);
    assert_eq!(state(&pilot, visual), DictationState::Idle);
    assert_eq!(speech.take(), vec![Request::Cancel]);
    late_final(&speech, visual);
    assert_eq!(text(&pilot, visual), "");
    assert_eq!(text(&session, visual), "");
}

#[gpui::test]
fn switching_pilots_cancels_the_first_pilots_dictation(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, _) = pilot_chat(&s, cx);
    let visual = &mut visual;
    let handle: gpui::AnyWindowHandle = s.window.into();
    let second = handle
        .update(visual, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.create(vec![s.project.clone()], w, cx);
                m.current().unwrap().id.clone()
            })
        })
        .unwrap();
    visual.run_until_parked();
    assert_ne!(second, s.id);
    let first = open_pilot(&s, &s.id, visual);
    listen(&speech, &first, visual);

    let other = open_pilot(&s, &second, visual);
    assert_eq!(state(&first, visual), DictationState::Idle);
    assert_eq!(speech.take(), vec![Request::Cancel]);
    late_final(&speech, visual);
    assert_eq!(text(&first, visual), "");
    assert_eq!(text(&other, visual), "");
}

#[gpui::test]
fn opening_a_pilot_whose_chat_still_loads_cancels_dictation(cx: &mut TestAppContext) {
    let speech = Speech::new(true, cx);
    let s = setup(cx);
    let (mut visual, _) = pilot_chat(&s, cx);
    let visual = &mut visual;
    let handle: gpui::AnyWindowHandle = s.window.into();
    let second = handle
        .update(visual, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.create(vec![s.project.clone()], w, cx);
                m.current().unwrap().id.clone()
            })
        })
        .unwrap();
    visual.run_until_parked();
    let session = open_session(&s, visual);
    listen(&speech, &session, visual);
    // As after a restart: the Pilot's chat is not loaded until it opens.
    s.manager.update(visual, |m, _| m.chats.remove(&second));

    let pilot = open_pilot(&s, &second, visual);
    assert_eq!(state(&session, visual), DictationState::Idle);
    assert_eq!(speech.take(), vec![Request::Cancel]);
    late_final(&speech, visual);
    assert_eq!(text(&session, visual), "");
    assert_eq!(text(&pilot, visual), "");
}
