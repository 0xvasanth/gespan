//! The harness's own proof: one message through a real Claude session, from
//! the composer to the agent process and back into the transcript.
use super::*;
use gpui::TestAppContext;

/// "hi" leaves the composer, reaches the agent as its prompt, and the
/// agent's answer is what the chat shows. Fake by default; `GESPAN_E2E_REAL=1`
/// runs it against the real `claude`.
#[gpui::test]
fn hi_reaches_the_agent_and_its_reply_shows(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let id = e2e.start_session(Kind::Claude, cx);
    e2e.script_reply(id, "Hello from the fake agent");

    let reply = e2e.send(id, "hi", cx);

    let prompts = e2e.prompts(id);
    assert_eq!(prompts.len(), 1, "one turn, one prompt: {prompts:?}");
    assert!(
        prompts[0].contains("hi"),
        "the agent received {:?}",
        prompts[0]
    );
    let rows = e2e.transcript(id, cx);
    let roles: Vec<&str> = rows.iter().map(|row| row.role.as_str()).collect();
    assert_eq!(roles.first(), Some(&"user"), "{rows:?}");
    assert_eq!(rows[0].text, "hi", "the transcript shows what was typed");
    assert!(
        !roles.contains(&"error"),
        "the turn failed: {:?}",
        rows.iter()
            .filter(|row| row.role == "error")
            .collect::<Vec<_>>()
    );
    match e2e.mode() {
        Mode::Fake => {
            assert_eq!(reply, "Hello from the fake agent");
            assert_eq!(roles, ["user", "assistant"], "{rows:?}");
            // Claude got the stream-json launch and its initialize first.
            let launch = &e2e.launches(id)[0];
            assert_eq!(
                std::fs::canonicalize(launch["cwd"].as_str().unwrap()).unwrap(),
                std::fs::canonicalize(e2e.repo()).unwrap(),
                "the agent works in the project"
            );
            let argv: Vec<&str> = launch["argv"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|arg| arg.as_str())
                .collect();
            assert!(
                argv.windows(2)
                    .any(|w| w == ["--input-format", "stream-json"])
            );
            assert_eq!(e2e.received(id)[0]["request"]["subtype"], "initialize");
        }
        Mode::Real => assert!(!reply.trim().is_empty(), "Claude answered nothing"),
    }
}

/// A second message in the same chat is a second prompt to the same agent,
/// answered by the script's next reply.
#[gpui::test]
fn a_follow_up_gets_the_next_scripted_reply(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let id = e2e.start_session(Kind::Claude, cx);
    e2e.script_reply(id, "First answer");
    e2e.script_reply(id, "Second answer");

    assert_eq!(e2e.send(id, "one", cx), "First answer");
    assert_eq!(e2e.send(id, "two", cx), "Second answer");

    let prompts = e2e.prompts(id);
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[0].contains("one") && prompts[1].contains("two"));
    let texts: Vec<String> = e2e
        .transcript(id, cx)
        .into_iter()
        .map(|row| row.text)
        .collect();
    assert_eq!(texts, ["one", "First answer", "two", "Second answer"]);
}

/// Two sessions in one window have separate agents and separate scripts.
#[gpui::test]
fn sessions_keep_their_own_agents(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let first = e2e.start_session(Kind::Claude, cx);
    let second = e2e.start_session(Kind::Claude, cx);
    assert_ne!(first, second);
    e2e.script_reply(first, "For the first");
    e2e.script_reply(second, "For the second");

    assert_eq!(e2e.send(second, "to second", cx), "For the second");
    assert_eq!(e2e.send(first, "to first", cx), "For the first");

    assert_eq!(e2e.prompts(first).len(), 1);
    assert!(e2e.prompts(first)[0].contains("to first"));
    assert_eq!(e2e.prompts(second).len(), 1);
    assert!(e2e.prompts(second)[0].contains("to second"));
}
