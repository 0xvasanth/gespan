//! A Pilot's turn shows live: its messages appear as the provider writes
//! them, while the turn is still open, and fold to one line once it settles.
//! This is the 24 November incident, end to end: a turn the provider kept
//! open hid the Pilot's question for 28 minutes.
use super::*;
use crate::conversation::Conversation;
use serde_json::json;

/// A Pilot on the test's project, its chat, and its session id.
fn pilot(e2e: &E2e, cx: &mut TestAppContext) -> (String, Entity<Conversation>, i64) {
    let project = e2e.repo().to_string_lossy().into_owned();
    let manager = e2e
        .window()
        .update(cx, |shell, _, _| shell.pilot_manager())
        .unwrap();
    let handle: gpui::AnyWindowHandle = e2e.window().into();
    handle
        .update(cx, |_, window, cx| {
            manager.update(cx, |m, cx| m.create(vec![project], window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let (id, session) = cx.update(|cx| {
        let pilot = manager.read(cx).current().unwrap();
        (pilot.id.clone(), pilot.session_id.unwrap())
    });
    let chat = cx.update(|cx| manager.read(cx).chat().unwrap());
    // The fake agent speaks Claude's stream-json.
    chat.update(cx, |chat, cx| {
        chat.pilot_config(Kind::Claude, None, Default::default(), cx)
    })
    .unwrap();
    // Prose lands whole, as with "Reveal replies gradually" off: a test
    // window paints no frames to pace words on.
    let transcript = chat.read_with(cx, |chat, _| chat.transcript.clone());
    transcript.update(cx, |t, cx| {
        t.apply_preferences(
            &crate::settings::Preferences {
                paced_reveal: false,
                ..Default::default()
            },
            cx,
        )
    });
    cx.run_until_parked();
    (id, chat, session)
}

fn assistant(id: &str, text: &str) -> Value {
    json!({
        "type": "assistant", "session_id": "fake", "parent_tool_use_id": null,
        "message": {
            "id": id, "role": "assistant", "model": "fake",
            "content": [{"type": "text", "text": text}],
            "usage": {"input_tokens": 1, "output_tokens": 1},
        },
    })
}

fn shown_prose(chat: &Entity<Conversation>, cx: &mut TestAppContext) -> Vec<String> {
    chat.read_with(cx, |chat, cx| chat.transcript.read(cx).shown_prose())
}

#[gpui::test]
fn a_pilots_question_shows_while_its_turn_is_still_open(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (id, chat, session) = pilot(&e2e, cx);
    // The agent's folder must exist before the first turn provisions it.
    std::fs::create_dir_all(e2e.agent_dir(session)).unwrap();
    e2e.script_frames(
        session,
        json!({
            "frames": [
                assistant("m1", "Started the API lead."),
                json!({
                    "type": "assistant", "session_id": "fake", "parent_tool_use_id": null,
                    "message": {
                        "id": "m-tool", "role": "assistant", "model": "fake",
                        "content": [{"type": "tool_use", "id": "copy", "name": "Bash",
                            "input": {"command": "echo copied", "description": "Copy fixtures to the test database"}}],
                        "usage": {"input_tokens": 1, "output_tokens": 1},
                    },
                }),
                json!({
                    "type": "user", "session_id": "fake", "parent_tool_use_id": null,
                    "message": {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "copy", "content": "copied"}
                    ]},
                }),
                assistant("m2", "The API lead stopped with 12 decisions. Go with my recommendations?"),
            ],
            // The provider keeps the turn open, as a background watcher did.
            "wait": true,
            "text": "Applied all 12 recommendations.",
        }),
    );
    let manager = e2e
        .window()
        .update(cx, |shell, _, _| shell.pilot_manager())
        .unwrap();
    cx.update(|cx| {
        manager.update(cx, |m, cx| {
            m.mobile_send(&id, "Start the API lead on the batch".into(), Vec::new(), cx)
        })
    })
    .unwrap();
    e2e.until(cx, "the Pilot's question while its turn runs", |_, cx| {
        shown_prose(&chat, cx)
            .iter()
            .any(|text| text.contains("12 decisions"))
    });
    let live = chat.read_with(cx, |chat, cx| {
        (chat.busy, chat.transcript.read(cx).shown_prose())
    });
    assert!(live.0, "the provider still holds the turn open");
    assert_eq!(
        live.1,
        [
            "Started the API lead.",
            "The API lead stopped with 12 decisions. Go with my recommendations?"
        ],
        "every message of the live turn shows, in order"
    );
    let steps = chat.read_with(cx, |chat, cx| chat.transcript.read(cx).shown_trace());
    assert!(
        steps
            .iter()
            .any(|title| title == "Copy fixtures to the test database"),
        "the step shows as a trace line named by its description: {steps:?}"
    );

    e2e.release(session);
    e2e.until(cx, "the turn to settle", |_, cx| {
        chat.read_with(cx, |chat, _| !chat.busy)
    });
    // Settled, the prose said before the step folds into the trace; the
    // question and the answer after it stay, under one summary line.
    // (Claude appends the turn's last text to the message before it.)
    e2e.until(cx, "the settled view", |_, cx| {
        let shown = shown_prose(&chat, cx);
        shown.len() == 1
            && shown[0].starts_with("The API lead stopped with 12 decisions.")
            && shown[0].ends_with("Applied all 12 recommendations.")
    });
    let (steps, summary) = chat.read_with(cx, |chat, cx| {
        let t = chat.transcript.read(cx);
        (t.shown_trace(), t.shown_summary())
    });
    assert!(steps.is_empty(), "the trace folds: {steps:?}");
    assert!(
        summary
            .as_deref()
            .is_some_and(|s| s.starts_with("Worked for") && s.ends_with("1 step")),
        "{summary:?}"
    );
}
