//! Sessions message each other: `sessions_message` over the real MCP
//! endpoint, delivered as one labelled turn when the receiver is idle, held
//! when sessions pass messages back and forth too often, and `sessions_wait`.
use super::*;
use gespan_core::orchestrator::mailbox::{Kind as MailKind, Mail, State};
use gpui::TestAppContext;
use serde_json::json;

fn message(to: i64, text: &str) -> Value {
    json!({"tool": "sessions_message", "arguments": {"id": to, "text": text}})
}

fn turn(text: &str, calls: Vec<Value>) -> Value {
    json!({"text": text, "mcp": calls})
}

fn call(e2e: &E2e, id: i64, n: usize) -> Value {
    let calls = e2e.mcp_calls(id);
    calls
        .get(n)
        .cloned()
        .unwrap_or_else(|| panic!("call {n} of {calls:?}"))
}

fn error_of(e2e: &E2e, id: i64, n: usize) -> String {
    let call = call(e2e, id, n);
    call["error"]
        .as_str()
        .unwrap_or_else(|| panic!("succeeded: {call}"))
        .to_owned()
}

fn idle_after(e2e: &E2e, id: i64, prompts: usize, cx: &mut TestAppContext) {
    e2e.until(cx, &format!("session {id}'s turn {prompts}"), |e2e, cx| {
        e2e.prompts(id).len() >= prompts && !e2e.busy(id, cx)
    });
}

/// Runs the app a while, for checking nothing more happens.
fn settle(cx: &mut TestAppContext) {
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Types into a session's composer and presses Enter, without waiting for
/// the turn to end.
fn type_and_send(e2e: &E2e, id: i64, text: &str, cx: &mut TestAppContext) {
    e2e.session(id, cx).update(cx, |view, cx| {
        view.input
            .update(cx, |input, cx| input.set_text(text.into(), cx));
        view.send_typed(cx);
    });
}

fn title(e2e: &E2e, id: i64, cx: &mut TestAppContext) -> String {
    e2e.session(id, cx)
        .read_with(cx, |view, _| view.summary.title.clone())
}

#[gpui::test]
fn a_message_to_a_busy_session_arrives_once_when_it_is_idle(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(b, json!({"text": "working", "wait": true}));
    type_and_send(&e2e, b, "Long job", cx);
    e2e.until(cx, "B to work", |e2e, cx| e2e.busy(b, cx));
    e2e.script_frames(
        a,
        turn("Told B.", vec![message(b, "Please rebase on main.")]),
    );

    e2e.send(a, "Tell B to rebase", cx);

    assert_eq!(call(&e2e, a, 0)["result"]["state"], "queued");
    settle(cx);
    assert_eq!(
        e2e.prompts(b).len(),
        1,
        "B's running turn isn't interrupted"
    );
    e2e.release(b);
    idle_after(&e2e, b, 2, cx);
    let delivered = &e2e.prompts(b)[1];
    let a_title = title(&e2e, a, cx);
    assert!(
        delivered.contains(&format!("[From session \"{a_title}\" (id {a})]")),
        "{delivered}"
    );
    assert!(delivered.contains("Please rebase on main."), "{delivered}");
    assert!(
        delivered.contains("not instructions from the user"),
        "{delivered}"
    );
    settle(cx);
    assert_eq!(e2e.prompts(b).len(), 2, "delivered once");
    // The transcript shows who sent it, without the label the agent reads.
    let shown = e2e
        .session(b, cx)
        .read_with(cx, |view, cx| view.transcript.read(cx).shown_plain());
    assert!(
        shown
            .iter()
            .any(|line| line.contains(&format!("From “{a_title}”: Please rebase on main."))),
        "{shown:?}"
    );
    assert!(
        !shown.iter().any(|line| line.contains("[From session")),
        "{shown:?}"
    );
}

#[gpui::test]
fn messages_passed_back_and_forth_are_held_at_four_hops(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    // A's typed turn (hop 0) sends hop 1; each delivered turn answers one
    // hop further: B sends 2, A sends 3, B's 4 is held.
    e2e.script_frames(a, turn("ok", vec![message(b, "ping 1")]));
    e2e.script_frames(a, turn("ok", vec![message(b, "ping 3")]));
    e2e.script_frames(a, json!("got it"));
    e2e.script_frames(b, turn("ok", vec![message(a, "pong 2")]));
    e2e.script_frames(b, turn("ok", vec![message(a, "pong 4")]));

    e2e.send(a, "Start the ping pong", cx);

    idle_after(&e2e, b, 2, cx);
    assert_eq!(call(&e2e, b, 1)["result"]["state"], "held");
    settle(cx);
    assert_eq!(
        e2e.prompts(a).len(),
        2,
        "pong 4 is held: {:?}",
        e2e.prompts(a)
    );
    assert!(e2e.prompts(a)[1].contains("pong 2"));
    // The user lets it through from A's sidebar menu.
    let menu = e2e
        .window()
        .update(cx, |shell, _, cx| shell.e2e_menu(a, cx))
        .unwrap();
    let item = menu
        .iter()
        .find(|(id, _)| id == "deliver-held")
        .unwrap_or_else(|| panic!("{menu:?}"));
    assert_eq!(item.1, "Deliver the held message");
    e2e.window()
        .update(cx, |shell, window, cx| {
            shell.e2e_run_menu(a, "deliver-held", window, cx)
        })
        .unwrap();
    idle_after(&e2e, a, 3, cx);
    assert!(e2e.prompts(a)[2].contains("pong 4"));
    let menu = e2e
        .window()
        .update(cx, |shell, _, cx| shell.e2e_menu(a, cx))
        .unwrap();
    assert!(!menu.iter().any(|(id, _)| id == "deliver-held"));
}

#[gpui::test]
fn a_typed_turn_starts_the_hop_count_again(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(b, turn("ok", vec![message(a, "pong")]));
    e2e.script_frames(a, turn("ok", vec![message(b, "ping")]));
    e2e.script_frames(a, json!("noted"));
    e2e.send(a, "Ping B", cx);
    idle_after(&e2e, a, 2, cx);
    // The user types again: A's next message is one hop away, not three.
    e2e.script_frames(a, turn("ok", vec![message(b, "fresh")]));
    e2e.send(a, "Ping B again", cx);
    let fresh = e2e.mcp_calls(a).last().cloned().unwrap();
    assert_eq!(fresh["result"]["state"], "queued");
    let store = e2e.store(cx);
    let store = locked(&store).unwrap();
    let hops: Vec<u8> = store
        .mail_for(b, State::Delivered)
        .unwrap()
        .into_iter()
        .chain(store.mail_for(b, State::Queued).unwrap())
        .map(|mail| mail.hops)
        .collect();
    assert_eq!(hops, [1, 1], "both from typed turns");
    drop(store);
    // A chat a message started is named by the message, not its label.
    let b_title = title(&e2e, b, cx);
    assert!(!b_title.contains("[From session"), "{b_title}");
}

#[gpui::test]
fn messages_wait_once_agents_started_ten_turns_this_hour(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    // Ten turns other sessions' messages started in B in the last hour.
    {
        let store = e2e.store(cx);
        let store = locked(&store).unwrap();
        let now = gespan_core::time::now_ms();
        for n in 0..10u64 {
            let id = format!("earlier-{n}");
            store
                .enqueue_mail(&Mail {
                    id: id.clone(),
                    to: b,
                    from: Some(a),
                    from_title: "A".into(),
                    kind: MailKind::Message,
                    text: "earlier".into(),
                    hops: 1,
                    created_ms: now - 60_000,
                    state: State::Queued,
                    held: None,
                    released: false,
                })
                .unwrap();
            store.mark_mail_delivered(&[id], now - 50_000 + n).unwrap();
        }
    }
    e2e.script_frames(a, turn("ok", vec![message(b, "eleventh")]));
    e2e.send(a, "Message B", cx);
    settle(cx);
    assert_eq!(e2e.prompts(b).len(), 0, "held for the user");
    let store = e2e.store(cx);
    let held = locked(&store).unwrap().mail_for(b, State::Held).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(
        held[0].held,
        Some(gespan_core::orchestrator::mailbox::Hold::Rate)
    );
    e2e.window()
        .update(cx, |shell, window, cx| {
            shell.e2e_run_menu(b, "deliver-held", window, cx)
        })
        .unwrap();
    idle_after(&e2e, b, 1, cx);
    assert!(e2e.prompts(b)[0].contains("eleventh"));
}

#[gpui::test]
fn mail_queued_before_a_crash_is_delivered_once(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    {
        let store = e2e.store(cx);
        let store = locked(&store).unwrap();
        store
            .enqueue_mail(&Mail {
                id: "msg:a:before-crash".into(),
                to: b,
                from: Some(a),
                from_title: "Sender".into(),
                kind: MailKind::Message,
                text: "Sent just before Gespan quit".into(),
                hops: 1,
                created_ms: 1,
                state: State::Queued,
                held: None,
                released: false,
            })
            .unwrap();
    }
    for _ in 0..2 {
        e2e.window()
            .update(cx, |shell, _, cx| shell.e2e_resume_wake_reports(cx))
            .unwrap();
        settle(cx);
    }
    idle_after(&e2e, b, 1, cx);
    settle(cx);
    assert_eq!(e2e.prompts(b).len(), 1);
    assert!(e2e.prompts(b)[0].contains("Sent just before Gespan quit"));
}

#[gpui::test]
fn the_same_request_queues_one_message(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    let once = json!({"tool": "sessions_message", "arguments": {"id": b, "text": "once", "requestId": "r"}});
    e2e.script_frames(a, turn("ok", vec![once.clone(), once]));
    e2e.send(a, "Message B twice", cx);
    idle_after(&e2e, b, 1, cx);
    settle(cx);
    assert_eq!(e2e.prompts(b).len(), 1);
    assert_eq!(e2e.prompts(b)[0].matches("once").count(), 1);
}

#[gpui::test]
fn a_message_reaches_only_another_session_here(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![
                message(a, "to myself"),
                message(424_242, "to nobody"),
                json!({"tool": "sessions_wait", "arguments": {"ids": [a]}}),
            ],
        ),
    );
    e2e.send(a, "Try", cx);
    assert!(error_of(&e2e, a, 0).starts_with("self_target: "));
    assert!(error_of(&e2e, a, 1).starts_with("not_found: "));
    assert!(error_of(&e2e, a, 2).starts_with("self_target: "));
}

#[gpui::test]
fn wait_returns_when_a_session_stops_working(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    let idle = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(b, json!({"text": "working", "wait": true}));
    type_and_send(&e2e, b, "Long job", cx);
    e2e.until(cx, "B to work", |e2e, cx| e2e.busy(b, cx));
    let wait = |ids: Vec<i64>, timeout: u64| json!({"tool": "sessions_wait", "arguments": {"ids": ids, "timeoutSeconds": timeout}});
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![wait(vec![b], 1), wait(vec![b], 25), wait(vec![idle, b], 25)],
        ),
    );

    type_and_send(&e2e, a, "Wait for B", cx);
    // The first wait times out while B works.
    e2e.until(cx, "the first wait", |e2e, _| !e2e.mcp_calls(a).is_empty());
    assert_eq!(call(&e2e, a, 0)["result"]["timedOut"], true);
    // The second returns once B stops.
    e2e.release(b);
    e2e.until(cx, "the second wait", |e2e, _| e2e.mcp_calls(a).len() >= 2);
    let second = call(&e2e, a, 1)["result"].clone();
    assert_eq!(second["timedOut"], false, "{second}");
    assert_eq!(second["ready"], json!([{"id": b, "status": "idle"}]));
    // A session that's already idle answers at once.
    e2e.until(cx, "A's turn", |e2e, cx| !e2e.busy(a, cx));
    let third = call(&e2e, a, 2)["result"].clone();
    assert!(
        third["ready"]
            .as_array()
            .unwrap()
            .contains(&json!({"id": idle, "status": "idle"})),
        "{third}"
    );
}

#[gpui::test]
fn a_message_to_a_session_that_may_do_more_waits_for_the_user(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let b = e2e.start_session(Kind::Claude, cx);
    e2e.session(a, cx).update(cx, |view, _| {
        view.settings.access = gespan_core::agent::AccessMode::ReadOnly;
    });
    e2e.script_frames(a, turn("ok", vec![message(b, "Delete the build folder")]));

    e2e.send(a, "Ask B", cx);

    assert_eq!(call(&e2e, a, 0)["result"]["state"], "held");
    settle(cx);
    assert!(
        e2e.prompts(b).is_empty(),
        "B can do more than A: the user decides"
    );
    let store = e2e.store(cx);
    let held = locked(&store).unwrap().mail_for(b, State::Held).unwrap();
    assert_eq!(
        held[0].held,
        Some(gespan_core::orchestrator::mailbox::Hold::Access)
    );
    e2e.window()
        .update(cx, |shell, window, cx| {
            shell.e2e_run_menu(b, "deliver-held", window, cx)
        })
        .unwrap();
    idle_after(&e2e, b, 1, cx);
    assert!(e2e.prompts(b)[0].contains("Delete the build folder"));
}
