//! The orchestration tools driven by a model's own choices: the chat is told
//! in plain words what to do, and the test checks what the app did. Fake by
//! default, with the tool calls scripted; with `GESPAN_E2E_REAL=1` the real
//! Claude CLI decides the calls itself, so these show a real model can use
//! the tools as described.
use super::*;
use gpui::TestAppContext;
use serde_json::json;

fn idle_after(e2e: &E2e, id: i64, prompts: usize, cx: &mut TestAppContext) {
    e2e.until(cx, &format!("session {id}'s turn {prompts}"), |e2e, cx| {
        e2e.prompts(id).len() >= prompts && !e2e.busy(id, cx)
    });
}

/// A supervised chat on the real CLI asks before it calls `tool`, and the
/// user allows it; the fake agent calls the tools without asking.
fn assert_asked(e2e: &E2e, session: i64, tool: &str) {
    let allowed = e2e.allowed();
    match e2e.mode() {
        Mode::Fake => assert!(allowed.is_empty(), "{allowed:?}"),
        Mode::Real => assert!(
            allowed.contains(&(session, tool.to_owned())),
            "{tool} was asked for: {allowed:?}"
        ),
    }
}

/// The sessions `parent` started.
fn children_of(e2e: &E2e, parent: i64, cx: &mut TestAppContext) -> Vec<i64> {
    let all = e2e.all_sessions(cx);
    e2e.window()
        .update(cx, |shell, _, _| {
            all.into_iter()
                .filter(|id| {
                    shell
                        .e2e_link(*id)
                        .is_some_and(|link| link.parent == parent)
                })
                .collect()
        })
        .unwrap()
}

#[gpui::test]
fn told_to_use_a_new_worktree_a_chat_starts_a_child_that_reports_back(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        json!({"text": "Started it.", "mcp": [{"tool": "sessions_start", "arguments": {
            "prompt": "Reply with only the word PONG. Use no tools.",
            "workspace": "worktree",
        }}]}),
    );
    e2e.script_next_child(json!(["PONG"]));
    e2e.script_reply(a, "The helper said PONG.");

    e2e.send(
        a,
        "Use the sessions_start tool once to start a session in a new worktree \
         whose prompt is exactly: Reply with only the word PONG. Use no tools. \
         Then end your turn without waiting for it.",
        cx,
    );

    let children = children_of(&e2e, a, cx);
    assert_eq!(children.len(), 1, "one child started: {children:?}");
    let b = children[0];
    idle_after(&e2e, b, 1, cx);
    let cwd = PathBuf::from(e2e.launches(b)[0]["cwd"].as_str().unwrap());
    assert_ne!(
        std::fs::canonicalize(&cwd).unwrap(),
        std::fs::canonicalize(e2e.repo()).unwrap(),
        "the child works in a worktree"
    );
    assert!(e2e.prompts(b)[0].contains("PONG"), "{:?}", e2e.prompts(b));
    // The child's answer comes back to the chat as a turn of its own.
    idle_after(&e2e, a, 2, cx);
    let report = &e2e.prompts(a)[1];
    assert!(report.contains("[From session"), "{report}");
    assert!(report.contains("PONG"), "{report}");
    assert_asked(&e2e, a, "sessions_start");
}

#[gpui::test]
fn told_to_tell_another_chat_something_it_arrives_there(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let b = e2e.start_session(Kind::Claude, cx);
    e2e.script_reply(b, "OK");
    e2e.send(b, "Reply with only OK. Use no tools.", cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        json!({"text": "Sent.", "mcp": [{"tool": "sessions_message", "arguments": {
            "id": b, "text": "The release is frozen until Friday.",
        }}]}),
    );
    e2e.script_reply(b, "Noted.");

    e2e.send(
        a,
        "There is one other chat in this repository. Find it with sessions_list, \
         then use sessions_message to tell it exactly: The release is frozen \
         until Friday. Then end your turn.",
        cx,
    );

    idle_after(&e2e, b, 2, cx);
    let delivered = &e2e.prompts(b)[1];
    assert!(delivered.contains("[From session"), "{delivered}");
    assert!(
        delivered.contains("not instructions from the user"),
        "{delivered}"
    );
    assert!(
        delivered.contains("frozen until Friday"),
        "the words A chose to send: {delivered}"
    );
    assert_eq!(e2e.prompts(b).len(), 2, "delivered once");
    assert_asked(&e2e, a, "sessions_message");
}
