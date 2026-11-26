//! The "Two zones" left nav, driven by real sessions: real chats with their
//! own agents go live and idle, and the project card's Now and Sessions
//! lists follow.
use super::*;
use gpui::TestAppContext;

/// The project card's session rows, Now's then Sessions', each with whether
/// it is live.
fn rail(e2e: &E2e, cx: &mut TestAppContext) -> Vec<(i64, bool)> {
    e2e.window()
        .update(cx, |shell, _, cx| shell.e2e_rail(cx))
        .unwrap()
}

fn ids(rows: &[(i64, bool)]) -> Vec<i64> {
    rows.iter().map(|(id, _)| *id).collect()
}

/// Three chats, each talked to once, list in Sessions newest first and once
/// each, with nothing in Now. Fake by default; `GESPAN_E2E_REAL=1` runs the
/// same turns through the real `claude`.
#[gpui::test]
fn talked_to_sessions_list_once_each_newest_first(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let chats: Vec<i64> = (0..3)
        .map(|n| {
            let id = e2e.start_session(Kind::Claude, cx);
            e2e.script_reply(id, &format!("Answer {n}"));
            id
        })
        .collect();
    for id in &chats {
        e2e.send(*id, "Reply with one word.", cx);
    }
    let rows = rail(&e2e, cx);
    assert_eq!(ids(&rows), chats.iter().rev().copied().collect::<Vec<_>>());
    assert!(
        rows.iter().all(|(_, live)| !live),
        "nothing in Now: {rows:?}"
    );
}

/// A chat whose turn is running leaves Sessions for Now, listed once, and
/// returns to Sessions when the turn ends. Fake only: a real turn's length
/// can't be held.
#[gpui::test]
fn a_running_chat_moves_to_now_and_back_when_its_turn_ends(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let chats: Vec<i64> = (0..3)
        .map(|n| {
            let id = e2e.start_session(Kind::Claude, cx);
            e2e.script_reply(id, &format!("Answer {n}"));
            id
        })
        .collect();
    for id in &chats {
        e2e.send(*id, "hello", cx);
    }
    let oldest = chats[0];
    assert_eq!(ids(&rail(&e2e, cx)).last(), Some(&oldest));

    // The oldest chat starts a turn that holds until released.
    e2e.script_frames(oldest, serde_json::json!({"text": "done", "wait": true}));
    e2e.session(oldest, cx).update(cx, |view, cx| {
        view.input
            .update(cx, |input, cx| input.set_text("long job".into(), cx));
        view.send_typed(cx);
    });
    e2e.until(cx, "the oldest chat to run", |e2e, cx| e2e.busy(oldest, cx));
    let running = rail(&e2e, cx);
    assert_eq!(
        running.first(),
        Some(&(oldest, true)),
        "in Now: {running:?}"
    );
    assert_eq!(running.len(), 3, "still listed once");
    assert!(running[1..].iter().all(|(_, live)| !live));

    // The turn ends: out of Now, and its new message makes it the newest.
    e2e.release(oldest);
    e2e.until(cx, "the turn to end", |e2e, cx| !e2e.busy(oldest, cx));
    let after = rail(&e2e, cx);
    assert!(after.iter().all(|(_, live)| !live), "{after:?}");
    assert_eq!(after.first(), Some(&(oldest, false)));
    assert_eq!(after.len(), 3);
}
