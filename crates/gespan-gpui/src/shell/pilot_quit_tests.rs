//! A Pilot's chat lives through a quit, a crash or an update restart like any
//! other chat: it counts as running, is recorded while it runs, and the next
//! launch marks its cut-off turn and continues it.
use super::*;
use gespan_core::interrupted::{CONTINUE, MESSAGE};
use gespan_core::pilot::{Message, Pilot, Roster};
use gespan_core::store::Store;
use gespan_core::tool::{ToolCall, ToolState};
use gespan_core::transcript::Row;
use gpui::{TestAppContext, WindowHandle};
use std::path::Path;
use std::sync::{Arc, Mutex};

const CWD: &str = "/missing/pilot-quit";
const PILOT: &str = "builder";

fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}

/// The `AskUserQuestion` row a turn leaves open while it waits on the user.
fn open_question() -> Row {
    let mut call = ToolCall::new("ask-1", "AskUserQuestion");
    call.state = ToolState::Running;
    row("tool", &serde_json::to_string(&call).unwrap())
}

/// A store with one Pilot whose chat was asking the user a question.
/// `thread` is the provider's saved thread, which a continue needs.
fn pilot_store(thread: bool, outbox: Vec<Message>) -> (Store, i64) {
    let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
    let session = store.create("codex", CWD).unwrap().id;
    store
        .write_rows(
            session,
            0,
            &[
                row("user", "Check the E101 scope"),
                row("assistant", "Two questions first"),
                open_question(),
            ],
        )
        .unwrap();
    if thread {
        store.provider_id(session, "thread-1").unwrap();
    }
    let mut roster = Roster::default();
    roster.pilots.push(Pilot {
        id: PILOT.into(),
        session_id: Some(session),
        name: "Builder".into(),
        outbox,
        ..Default::default()
    });
    roster.save(&store).unwrap();
    (store, session)
}

fn shell(store: Store, cx: &mut TestAppContext) -> WindowHandle<Shell> {
    let summaries = store.sessions().unwrap();
    let store = Arc::new(Mutex::new(store));
    cx.update(crate::shell::bind_keys);
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(store, summaries, CWD.into(), None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    window
}

fn rows(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> Vec<Row> {
    window
        .update(cx, |shell, _, _| {
            locked(&shell.store).unwrap().load(id).unwrap().rows
        })
        .unwrap()
}

fn taken(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<i64> {
    window
        .update(cx, |shell, _, _| {
            locked(&shell.store).unwrap().take_in_flight().unwrap()
        })
        .unwrap()
}

fn user_rows(rows: &[Row]) -> Vec<&str> {
    rows.iter()
        .filter(|row| row.role == "user")
        .map(|row| row.text.as_str())
        .collect()
}

fn question_state(rows: &[Row]) -> ToolState {
    rows.iter()
        .filter(|row| row.role == "tool")
        .filter_map(|row| serde_json::from_str::<ToolCall>(&row.text).ok())
        .find(|call| call.id == "ask-1")
        .unwrap()
        .state
}

/// Opens the Pilot's chat and shows it mid-turn (or done).
fn set_pilot_busy(window: WindowHandle<Shell>, busy: bool, cx: &mut TestAppContext) {
    let manager = window
        .update(cx, |shell, _, _| shell.pilot_manager())
        .unwrap();
    manager.update(cx, |pilots, cx| pilots.pane_chat(PILOT, cx));
    cx.run_until_parked();
    let chat = manager
        .read_with(cx, |pilots, _| pilots.open_chat(PILOT))
        .unwrap();
    chat.update(cx, |chat, cx| {
        chat.busy = busy;
        cx.emit(SessionChanged);
    });
    cx.run_until_parked();
}

fn hold_on(store: &Store, session: i64) {
    let mut hold = gespan_core::hold::HoldState::new(1);
    hold.track(session, true);
    store.save_hold(&hold).unwrap();
}

#[gpui::test]
fn a_running_pilot_chat_is_recorded_while_it_runs(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(true, Vec::new());
    let window = shell(store, cx);
    set_pilot_busy(window, true, cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.running_chats(cx), vec![session])
        })
        .unwrap();
    assert_eq!(
        taken(window, cx),
        vec![session],
        "a crash now still finds it"
    );
    // The turn ending takes it off the list.
    set_pilot_busy(window, false, cx);
    window
        .update(cx, |shell, _, cx| {
            locked(&shell.store)
                .unwrap()
                .set_in_flight(&[session])
                .unwrap();
            assert!(shell.running_chats(cx).is_empty());
        })
        .unwrap();
    set_pilot_busy(window, true, cx);
    set_pilot_busy(window, false, cx);
    assert!(taken(window, cx).is_empty());
}

#[gpui::test]
fn a_pilot_chat_stays_recorded_once_a_quit_starts(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(true, Vec::new());
    let window = shell(store, cx);
    set_pilot_busy(window, true, cx);
    cx.update(|cx| cx.set_global(crate::Quitting));
    // The shutdown ends the turn; it must stay listed for the next launch.
    set_pilot_busy(window, false, cx);
    assert_eq!(taken(window, cx), vec![session]);
}

#[gpui::test]
fn quitting_while_a_pilot_works_asks_first(cx: &mut TestAppContext) {
    let (store, _) = pilot_store(true, Vec::new());
    let window = shell(store, cx);
    set_pilot_busy(window, true, cx);
    window
        .update(cx, |shell, window, cx| shell.request_quit(window, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "a working Pilot is a running chat");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.update(|cx| crate::quitting(cx)));
}

#[gpui::test]
fn a_pilot_chat_cut_off_by_a_quit_is_marked_and_continued_at_launch(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(true, Vec::new());
    store.set_in_flight(&[session]).unwrap();
    let window = shell(store, cx);
    let rows = rows(window, session, cx);
    // Nothing answers the open question after a restart: it is closed.
    assert_eq!(question_state(&rows), ToolState::Cancelled);
    let marker = rows
        .iter()
        .position(|row| row.role == "system" && row.text == MESSAGE)
        .expect("the cut-off turn is marked");
    assert_eq!(
        user_rows(&rows[marker..]),
        vec![CONTINUE],
        "continued once, so the Pilot asks again"
    );
    assert!(taken(window, cx).is_empty(), "the list is taken once");
}

#[gpui::test]
fn a_pilot_chat_with_no_provider_thread_is_marked_but_not_continued(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(false, Vec::new());
    store.set_in_flight(&[session]).unwrap();
    let window = shell(store, cx);
    let rows = rows(window, session, cx);
    assert!(gespan_core::interrupted::ends_interrupted(&rows));
    assert_eq!(user_rows(&rows), vec!["Check the E101 scope"]);
}

#[gpui::test]
fn a_pilot_chat_not_running_at_quit_is_left_alone(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(true, Vec::new());
    let window = shell(store, cx);
    let rows = rows(window, session, cx);
    assert!(!rows.iter().any(|row| row.text == MESSAGE));
    assert_eq!(question_state(&rows), ToolState::Running);
}

#[gpui::test]
fn on_hold_a_cut_off_pilot_chat_waits_and_resume_continues_it(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(true, Vec::new());
    store.set_in_flight(&[session]).unwrap();
    hold_on(&store, session);
    let window = shell(store, cx);
    let held = rows(window, session, cx);
    assert!(gespan_core::interrupted::ends_interrupted(&held));
    assert_eq!(user_rows(&held), vec!["Check the E101 scope"], "Hold waits");
    // Nothing has opened the Pilot since the restart; Resume still finds it.
    window
        .update(cx, |shell, _, cx| shell.resume_all(cx))
        .unwrap();
    cx.run_until_parked();
    let resumed = rows(window, session, cx);
    let marker = resumed.iter().position(|row| row.text == MESSAGE).unwrap();
    assert_eq!(user_rows(&resumed[marker..]), vec![CONTINUE]);
}

fn queued(id: &str, text: &str) -> Message {
    Message {
        id: id.into(),
        text: text.into(),
        sent_at: Some(1),
        internal: false,
        error: None,
        attachments: Vec::new(),
        report: Vec::new(),
    }
}

#[gpui::test]
fn a_message_queued_before_the_quit_is_delivered_without_opening_the_pilot(
    cx: &mut TestAppContext,
) {
    let (store, session) = pilot_store(true, vec![queued("m1", "Ship it")]);
    let window = shell(store, cx);
    let rows = rows(window, session, cx);
    assert_eq!(user_rows(&rows).last(), Some(&"Ship it"));
}

#[gpui::test]
fn on_hold_a_queued_pilot_message_waits(cx: &mut TestAppContext) {
    let (store, session) = pilot_store(true, vec![queued("m1", "Ship it")]);
    hold_on(&store, session);
    let window = shell(store, cx);
    let rows = rows(window, session, cx);
    assert!(!user_rows(&rows).contains(&"Ship it"));
}
