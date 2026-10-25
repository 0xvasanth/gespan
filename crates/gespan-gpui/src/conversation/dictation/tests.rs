use super::*;
use crate::conversation::Conversation;
use crate::input;
use gespan_core::store::Store;
use gpui::{KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use std::{cell::RefCell, path::Path, rc::Rc, sync::Mutex};

const DENIED: &str = "Gespan can't use the microphone. Allow it in System Settings → Privacy & Security → Microphone.";

/// The app's speech hub over a temporary model folder, recording requests.
struct Harness {
    _dir: tempfile::TempDir,
    sent: Rc<RefCell<Vec<Request>>>,
    speech: Entity<Speech>,
}

impl Harness {
    fn new(ready: bool, cx: &mut TestAppContext) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if ready {
            make_ready(dir.path(), &catalog::PARAKEET_V2_EN);
        }
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let store = ModelStore::new(dir.path().to_path_buf());
        let speech = cx.update(|cx| with_sink(store, move |r| log.borrow_mut().push(r), cx));
        Self {
            _dir: dir,
            sent,
            speech,
        }
    }

    fn emit(&self, event: Event, cx: &mut TestAppContext) {
        self.speech.read_with(cx, |s, _| s.emit(event));
        cx.run_until_parked();
    }

    fn take(&self) -> Vec<Request> {
        self.sent.borrow_mut().drain(..).collect()
    }
}

/// Marks the model as downloaded without writing its real bytes.
fn make_ready(dir: &Path, spec: &ModelSpec) {
    let model_dir = dir.join(spec.id);
    std::fs::create_dir_all(&model_dir).unwrap();
    for file in spec.files {
        let f = std::fs::File::create(model_dir.join(file.name)).unwrap();
        f.set_len(file.size).unwrap();
        std::fs::File::create(model_dir.join(format!("{}.verified", file.name))).unwrap();
    }
}

fn composer(text: &str, cx: &mut TestAppContext) -> (Entity<Input>, Entity<Dictation>) {
    cx.update(|cx| {
        let input = cx.new(Input::new);
        input.update(cx, |i, cx| i.set_text(text.into(), cx));
        let dictation = cx.new(|cx| Dictation::new(input.downgrade(), cx));
        (input, dictation)
    })
}

fn text(input: &Entity<Input>, cx: &mut TestAppContext) -> String {
    input.read_with(cx, |i, _| i.content.to_string())
}

fn state(d: &Entity<Dictation>, cx: &mut TestAppContext) -> DictationState {
    d.read_with(cx, |d, _| d.state().clone())
}

fn is_start(r: &Request) -> bool {
    matches!(r, Request::Start { .. })
}

fn is_load(r: &Request) -> bool {
    matches!(r, Request::Load { .. })
}

/// Starts dictating in `d` and has the helper report it is listening.
fn listen(h: &Harness, d: &Entity<Dictation>, cx: &mut TestAppContext) {
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    assert!(h.sent.borrow().last().is_some_and(is_start));
    h.emit(Event::Listening, cx);
    assert_eq!(state(d, cx), DictationState::Listening);
}

fn final_text(text: &str) -> Event {
    Event::Final {
        text: text.into(),
        capped: false,
    }
}

#[gpui::test]
fn final_text_goes_to_current_cursor(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (input, d) = composer("fix bug now", cx);
    listen(&h, &d, cx);
    let requests = h.take();
    assert!(
        is_load(&requests[0]) && is_start(&requests[1]),
        "{requests:?}"
    );

    // A partial is only a caption.
    h.emit(
        Event::Partial {
            text: "in pars".into(),
        },
        cx,
    );
    assert_eq!(d.read_with(cx, |d, _| d.partial().to_string()), "in pars");
    assert_eq!(text(&input, cx), "fix bug now");

    // The user moves the caret while still dictating; stop time counts.
    input.update(cx, |i, cx| i.select_range(7..7, cx));
    d.update(cx, |d, cx| d.toggle(cx));
    assert_eq!(state(&d, cx), DictationState::Finishing);
    assert_eq!(h.take(), vec![Request::Stop]);
    h.emit(final_text("in parser"), cx);

    assert_eq!(text(&input, cx), "fix bug in parser now");
    assert_eq!(input.read_with(cx, |i, _| i.selection()), 17..17);
    assert_eq!(state(&d, cx), DictationState::Idle);
    assert_eq!(d.read_with(cx, |d, _| d.partial().to_string()), "");
}

#[gpui::test]
fn leading_space_rule_matches_tauri(cx: &mut TestAppContext) {
    // insertDictation.test.ts, case for case.
    assert_eq!(spaced("rename the", "helper").as_deref(), Some(" helper"));
    assert_eq!(
        spaced("fix bug", "in parser").as_deref(),
        Some(" in parser")
    );
    assert_eq!(spaced("", "Hello").as_deref(), Some("Hello"));
    assert_eq!(spaced("say ", "hi").as_deref(), Some("hi"));
    assert_eq!(
        spaced("line one\n", "line two").as_deref(),
        Some("line two")
    );
    for mark in [".", ",", ";", ":", "!", "?", ")"] {
        assert_eq!(
            spaced("done", &format!("{mark} next")),
            Some(format!("{mark} next"))
        );
    }
    assert_eq!(spaced("say", "\"hi\"").as_deref(), Some(" \"hi\""));
    // The helper's surrounding whitespace is trimmed, and silence is nothing.
    assert_eq!(spaced("a", "  b \n").as_deref(), Some(" b"));
    assert_eq!(spaced("a", "   "), None);
    // Non-ASCII before the caret counts as a word character.
    assert_eq!(spaced("café", "noir").as_deref(), Some(" noir"));

    // A selection is replaced; a whole-draft selection gets no stray space.
    let h = Harness::new(true, cx);
    let (input, d) = composer("open main.ts now", cx);
    input.update(cx, |i, cx| i.select_range(5..12, cx));
    listen(&h, &d, cx);
    h.emit(final_text("lib.rs"), cx);
    assert_eq!(text(&input, cx), "open lib.rs now");
    assert_eq!(input.read_with(cx, |i, _| i.selection()), 11..11);

    input.update(cx, |i, cx| {
        i.set_text("old draft".into(), cx);
        i.select_range(0..9, cx);
    });
    listen(&h, &d, cx);
    h.emit(final_text("New draft"), cx);
    assert_eq!(text(&input, cx), "New draft");

    // A silent take leaves the draft alone.
    listen(&h, &d, cx);
    h.emit(final_text("  "), cx);
    assert_eq!(text(&input, cx), "New draft");
    assert_eq!(state(&d, cx), DictationState::Idle);
}

#[gpui::test]
fn switching_session_cancels_dictation(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (first_input, first) = composer("first", cx);
    let (second_input, second) = composer("second", cx);
    listen(&h, &first, cx);
    h.take();

    // Re-activating the same session keeps it going.
    cx.update(|cx| switched_to(&first, cx));
    assert!(h.take().is_empty());
    assert_eq!(state(&first, cx), DictationState::Listening);

    cx.update(|cx| switched_to(&second, cx));
    assert_eq!(h.take(), vec![Request::Cancel]);
    assert_eq!(state(&first, cx), DictationState::Idle);

    // What the helper had already decoded goes nowhere.
    h.emit(final_text("stray words"), cx);
    h.emit(Event::Cancelled, cx);
    assert_eq!(text(&first_input, cx), "first");
    assert_eq!(text(&second_input, cx), "second");
    assert_eq!(state(&second, cx), DictationState::Idle);
}

#[gpui::test]
fn closing_the_composer_cancels_without_insert(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (input, d) = composer("draft", cx);
    listen(&h, &d, cx);
    h.take();
    drop(d);
    // Released entities are dropped at the end of the next update.
    cx.update(|_| {});
    cx.run_until_parked();
    assert_eq!(h.take(), vec![Request::Cancel]);
    h.emit(final_text("late"), cx);
    assert_eq!(text(&input, cx), "draft");
}

#[gpui::test]
fn starting_in_another_composer_cancels_the_first_before_starting(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (first_input, first) = composer("one", cx);
    let (second_input, second) = composer("two", cx);
    listen(&h, &first, cx);
    h.take();

    second.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    // One queue, in order: the first take ends before the second begins, and
    // the loaded model is not loaded again.
    let requests = h.take();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(requests[0], Request::Cancel);
    assert!(is_start(&requests[1]));
    assert_eq!(state(&first, cx), DictationState::Idle);

    h.emit(Event::Listening, cx);
    h.emit(final_text("hello"), cx);
    assert_eq!(text(&first_input, cx), "one");
    assert_eq!(text(&second_input, cx), "two hello");
}

#[gpui::test]
fn stale_cancelled_does_not_end_next_attempt(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (input, d) = composer("note", cx);
    listen(&h, &d, cx);
    d.update(cx, |d, cx| d.cancel(cx));
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    let requests = h.take();
    assert_eq!(requests[requests.len() - 2], Request::Cancel);
    assert!(is_start(&requests[requests.len() - 1]));

    // The helper answers the first attempt's Cancel after the new Start went
    // out; it must not end the new attempt.
    h.emit(Event::Cancelled, cx);
    assert_eq!(state(&d, cx), DictationState::Starting);
    h.emit(Event::Listening, cx);
    assert_eq!(state(&d, cx), DictationState::Listening);
    h.emit(Event::Cancelled, cx);
    assert_eq!(state(&d, cx), DictationState::Listening);
    h.emit(final_text("kept"), cx);
    assert_eq!(text(&input, cx), "note kept");
}

#[gpui::test]
fn events_before_this_takes_listening_are_ignored(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (input, d) = composer("x", cx);
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    // A previous take's tail, still in flight.
    h.emit(Event::Level { rms: 0.2 }, cx);
    h.emit(Event::Partial { text: "old".into() }, cx);
    h.emit(final_text("old"), cx);
    assert_eq!(state(&d, cx), DictationState::Starting);
    assert_eq!(text(&input, cx), "x");
    assert_eq!(d.read_with(cx, |d, _| d.partial().to_string()), "");
}

#[gpui::test]
fn stop_before_the_start_is_sent_follows_it(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (input, d) = composer("", cx);
    // Stop while the vocabulary is still being gathered.
    d.update(cx, |d, cx| {
        d.toggle(cx);
        d.toggle(cx);
    });
    assert_eq!(state(&d, cx), DictationState::Finishing);
    assert!(h.sent.borrow().is_empty());
    cx.run_until_parked();
    let requests = h.take();
    assert!(is_start(&requests[requests.len() - 2]));
    assert_eq!(requests[requests.len() - 1], Request::Stop);
    h.emit(Event::Listening, cx);
    assert_eq!(state(&d, cx), DictationState::Finishing);
    h.emit(final_text("Quick"), cx);
    assert_eq!(text(&input, cx), "Quick");

    // Cancelled before the Start went out: nothing is sent at all.
    d.update(cx, |d, cx| {
        d.toggle(cx);
        d.cancel(cx);
    });
    cx.run_until_parked();
    assert!(h.take().is_empty());
    assert_eq!(state(&d, cx), DictationState::Idle);
}

#[gpui::test]
fn model_missing_shows_download_prompt(cx: &mut TestAppContext) {
    let h = Harness::new(false, cx);
    let window = conversation(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    click(visual, "composer-mic");
    let d = dictation(window, visual);
    assert_eq!(state(&d, visual), DictationState::NeedsModel);
    // The helper never starts for a model that cannot load.
    assert!(h.take().is_empty());
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("dictation-download").is_some());
    click(visual, "dictation-not-now");
    assert_eq!(state(&d, visual), DictationState::Idle);
    assert!(visual.debug_bounds("dictation-download").is_none());

    // The helper can still find the model gone (deleted elsewhere): the
    // prompt shows, and the next start loads the model again.
    make_ready(h._dir.path(), &catalog::PARAKEET_V2_EN);
    let (_input, d) = composer("", visual);
    listen(&h, &d, visual);
    h.take();
    h.emit(
        Event::Error {
            kind: ErrorKind::ModelMissing,
            message: "The speech model isn't downloaded yet.".into(),
        },
        visual,
    );
    assert_eq!(state(&d, visual), DictationState::NeedsModel);
    d.update(visual, |d, cx| d.cancel(cx));
    d.update(visual, |d, cx| d.toggle(cx));
    visual.run_until_parked();
    let requests = h.take();
    assert!(
        is_load(&requests[0]) && is_start(&requests[1]),
        "{requests:?}"
    );
}

#[gpui::test]
fn microphone_denied_returns_to_idle_with_message(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (input, d) = composer("hi", cx);
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    h.emit(
        Event::Error {
            kind: ErrorKind::MicrophoneDenied,
            message: DENIED.into(),
        },
        cx,
    );
    assert_eq!(state(&d, cx), DictationState::Idle);
    assert_eq!(
        d.read_with(cx, |d, _| d.error().map(|e| e.to_string())),
        Some(DENIED.into())
    );
    assert!(!d.read_with(cx, |d, _| d.dictating()));
    // Nothing from that attempt lands later.
    h.emit(Event::Listening, cx);
    h.emit(final_text("ghost"), cx);
    assert_eq!(text(&input, cx), "hi");

    // Trying again clears the message and reuses the loaded model.
    h.take();
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    assert!(d.read_with(cx, |d, _| d.error().is_none()));
    let requests = h.take();
    assert_eq!(requests.len(), 1);
    assert!(is_start(&requests[0]));
}

#[gpui::test]
fn speech_unavailable_shows_restart_message(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let (_input, d) = composer("", cx);
    // The supervisor's give-up message mid-take is shown as sent.
    listen(&h, &d, cx);
    h.emit(
        Event::Error {
            kind: ErrorKind::Engine,
            message: UNAVAILABLE.into(),
        },
        cx,
    );
    assert_eq!(
        d.read_with(cx, |d, _| d.error().map(|e| e.to_string())),
        Some(UNAVAILABLE.into())
    );

    // A start against a client that gave up says so and sends nothing.
    h.speech.update(cx, |s, _| s.give_up());
    h.take();
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    assert_eq!(state(&d, cx), DictationState::Error(UNAVAILABLE.into()));
    assert!(h.take().is_empty());
    assert!(!d.read_with(cx, |d, _| d.dictating()));
}

#[gpui::test]
fn cancelled_download_is_quiet(cx: &mut TestAppContext) {
    let h = Harness::new(false, cx);
    let (_input, d) = composer("", cx);
    let (_other_input, other) = composer("", cx);
    d.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    other.update(cx, |d, cx| d.toggle(cx));
    cx.run_until_parked();
    assert_eq!(state(&d, cx), DictationState::NeedsModel);
    assert_eq!(state(&other, cx), DictationState::NeedsModel);

    d.update(cx, |d, cx| d.download(cx));
    let id = catalog::PARAKEET_V2_EN.id;
    let progress = h.speech.read_with(cx, |s, _| s.progress(id));
    assert_eq!(progress.map(|p| p.downloaded), Some(0));
    // A second download of the same model is refused with a message.
    other.update(cx, |d, cx| d.download(cx));
    assert_eq!(
        other.read_with(cx, |d, _| d.error().map(|e| e.to_string())),
        Some(ALREADY_DOWNLOADING.into())
    );
    // Cancelled before the first byte, so no request leaves the machine.
    d.update(cx, |d, cx| d.cancel_download(cx));
    cx.run_until_parked();

    assert!(d.read_with(cx, |d, _| d.error().is_none()));
    assert!(d.read_with(cx, |d, cx| d.download_progress(cx).is_none()));
    assert_eq!(state(&d, cx), DictationState::NeedsModel);
    assert!(h.speech.read_with(cx, |s, _| s.progress(id).is_none()));
    assert!(h.take().is_empty());
}

// Composer wiring: the shortcut, the mic and the caption row's controls.

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let summary = store.create("claude", "/work/gespan").unwrap();
    let store = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        input::bind(cx);
        crate::popover::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Conversation::new(summary, store, cx))
        })
        .unwrap()
    });
    window
        .update(cx, |c, w, cx| w.focus(&c.input.focus_handle(cx), cx))
        .unwrap();
    cx.run_until_parked();
    window
}

fn dictation(window: WindowHandle<Conversation>, cx: &mut VisualTestContext) -> Entity<Dictation> {
    window.update(cx, |c, _, _| c.dictation.clone()).unwrap()
}

fn composer_input(window: WindowHandle<Conversation>, cx: &mut VisualTestContext) -> Entity<Input> {
    window.update(cx, |c, _, _| c.input.clone()).unwrap()
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    visual.refresh().unwrap();
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
}

const SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd-shift-m"
} else {
    "ctrl-shift-m"
};

#[gpui::test]
fn cmd_shift_m_tap_toggles_and_hold_stops(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let window = conversation(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let d = dictation(window, visual);
    let input = composer_input(window, visual);

    // Tap: press and release at once starts, and keeps listening.
    visual.simulate_keystrokes(SHORTCUT);
    assert_eq!(state(&d, visual), DictationState::Starting);
    visual.simulate_modifiers_change(Modifiers::none());
    h.emit(Event::Listening, visual);
    assert_eq!(state(&d, visual), DictationState::Listening);
    assert!(h.take().iter().any(is_start));
    // Tap again: stops and inserts.
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_modifiers_change(Modifiers::none());
    assert_eq!(state(&d, visual), DictationState::Finishing);
    assert_eq!(h.take(), vec![Request::Stop]);
    h.emit(final_text("tapped"), visual);
    assert_eq!(text(&input, visual), "tapped");

    // Hold: key repeats are not presses, and letting go after 400 ms stops.
    visual.simulate_keystrokes(SHORTCUT);
    h.emit(Event::Listening, visual);
    visual.executor().advance_clock(Duration::from_millis(250));
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_keystrokes(SHORTCUT);
    assert_eq!(state(&d, visual), DictationState::Listening);
    assert!(!h.take().contains(&Request::Stop));
    visual.executor().advance_clock(HOLD_TO_TALK);
    visual.simulate_modifiers_change(Modifiers::none());
    assert_eq!(state(&d, visual), DictationState::Finishing);
    assert_eq!(h.take(), vec![Request::Stop]);
    h.emit(final_text("held"), visual);
    assert_eq!(text(&input, visual), "tapped held");

    // Where a key-up arrives (not macOS with Command down), it ends a hold too.
    visual.simulate_keystrokes(SHORTCUT);
    h.emit(Event::Listening, visual);
    h.take();
    visual.executor().advance_clock(Duration::from_millis(600));
    visual.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("m").unwrap(),
    });
    assert_eq!(h.take(), vec![Request::Stop]);
}

#[gpui::test]
fn stop_and_cancel_controls_work_while_disabled(cx: &mut TestAppContext) {
    let h = Harness::new(true, cx);
    let window = conversation(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let d = dictation(window, visual);
    let input = composer_input(window, visual);
    let disable = |on: bool, visual: &mut VisualTestContext| {
        input.update(visual, |i, cx| {
            i.set_read_only(on);
            cx.notify();
        });
        visual.run_until_parked();
    };

    click(visual, "composer-mic");
    h.emit(Event::Listening, visual);
    h.take();
    disable(true, visual);
    // Escape still cancels.
    visual.simulate_keystrokes("escape");
    assert_eq!(state(&d, visual), DictationState::Idle);
    assert_eq!(h.take(), vec![Request::Cancel]);

    // Starting is not allowed while disabled, by the mic or the shortcut.
    click(visual, "composer-mic");
    visual.simulate_keystrokes(SHORTCUT);
    visual.simulate_modifiers_change(Modifiers::none());
    assert_eq!(state(&d, visual), DictationState::Idle);
    assert!(h.take().is_empty());

    disable(false, visual);
    click(visual, "composer-mic");
    h.emit(Event::Listening, visual);
    h.take();
    disable(true, visual);
    click(visual, "dictation-stop");
    assert_eq!(state(&d, visual), DictationState::Finishing);
    assert_eq!(h.take(), vec![Request::Stop]);
    // Finishing hides Stop; Cancel stays.
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("dictation-stop").is_none());
    click(visual, "dictation-cancel");
    assert_eq!(state(&d, visual), DictationState::Idle);
    assert_eq!(h.take(), vec![Request::Cancel]);
    assert!(visual.debug_bounds("dictation-cancel").is_none());
}
