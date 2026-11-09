//! A handoff or fork opens a new chat that continues the conversation it came
//! from; neither may open blank.
use super::*;
use crate::response_actions::Request as ResponseRequest;
use gespan_core::{
    agent::{Settings, TurnInfo},
    response::{Target, TransferKind},
    store::Store,
    transcript::Row,
};
use gpui::{TestAppContext, WindowHandle};
use std::path::Path;

const PROJECT: &str = "/work/gespan";

fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}

fn footer_chat() -> Vec<Row> {
    vec![
        row("user", "Plan the footer"),
        row("assistant", "Here is the plan: three columns."),
        row("user", "Use two columns instead"),
        row("assistant", "Switched to two columns."),
    ]
}

/// A window showing one Claude chat with `rows`, its history loaded.
fn fixture(rows: &[Row], cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64) {
    let mut store = Store::open(Path::new(":memory:")).unwrap();
    let mut chat = store.create("claude", PROJECT).unwrap();
    store.title(chat.id, "Footer").unwrap();
    chat.title = "Footer".into();
    let turn = |model: &str| TurnInfo {
        provider: "claude".into(),
        model: model.into(),
        ..Default::default()
    };
    store
        .save_agent_settings(
            chat.id,
            &Settings {
                model: Some("opus".into()),
                effort: Some("high".into()),
                turns: [(1, turn("Opus")), (3, turn("Opus"))].into(),
                ..Default::default()
            },
        )
        .unwrap();
    if !rows.is_empty() {
        store.write_rows(chat.id, 0, rows).unwrap();
        chat.has_messages = true;
    }
    let mut state = Workspace::default();
    state.add_project(PROJECT);
    state.open(chat.id);
    let saved = serde_json::to_string(&state).unwrap();
    let store = Arc::new(Mutex::new(store));
    let id = chat.id;
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(store, vec![chat], PROJECT.into(), Some(saved), cx);
                shell.ensure_session(id, cx);
                shell
            })
        })
        .unwrap()
    });
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |v, cx| v.load(cx))
        })
        .unwrap();
    cx.run_until_parked();
    (window, id)
}

/// The chat the window shows, once it is not `source`.
fn opened(window: WindowHandle<Shell>, source: i64, cx: &mut TestAppContext) -> i64 {
    let id = window
        .update(cx, |shell, _, _| shell.workspace.active)
        .unwrap()
        .expect("a chat is shown");
    assert_ne!(id, source, "the transfer opened a new chat");
    id
}

fn history(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> Vec<Row> {
    window
        .update(cx, |shell, _, cx| {
            let view = shell.sessions[&id].read(cx);
            assert!(view.history_loaded(), "the new chat loaded its history");
            view.transcript.read(cx).history_rows().to_vec()
        })
        .unwrap()
}

fn request(
    window: WindowHandle<Shell>,
    id: i64,
    request: ResponseRequest,
    cx: &mut TestAppContext,
) {
    window
        .update(cx, |shell, _, cx| {
            let transcript = shell.sessions[&id].read(cx).transcript.clone();
            transcript.update(cx, |_, cx| cx.emit(request));
        })
        .unwrap();
    cx.run_until_parked();
}

/// What the handoff picker sends once a target is chosen.
fn handoff(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) {
    window
        .update(cx, |shell, _, cx| {
            let view = shell.sessions[&id].read(cx);
            let transcript = view.transcript.read(cx);
            let last = transcript.history_rows().len().saturating_sub(1);
            let (turn, through) = transcript.response_rows(last);
            let transfer = Transfer {
                kind: TransferKind::Handoff,
                target: Target {
                    provider: "codex".into(),
                    model: Some("gpt-5.5".into()),
                    effort: None,
                    settings: Default::default(),
                },
                from: "claude".into(),
                turn,
                at: last,
                through,
                source: id,
                older: transcript.history_cursor(),
                turns: transcript.turns.clone(),
                cwd: view.summary.cwd.clone(),
                title: view.summary.title.clone(),
                settings: view.settings.clone(),
            };
            shell.transfer_response(transfer, cx);
        })
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn a_handoff_opens_the_conversation_with_its_recap(cx: &mut TestAppContext) {
    let chat = footer_chat();
    let (window, source) = fixture(&chat, cx);
    handoff(window, source, cx);
    let id = opened(window, source, cx);
    assert_eq!(history(window, id, cx), chat, "the handoff is not blank");
    window
        .update(cx, |shell, _, cx| {
            let view = shell.sessions[&id].read(cx);
            assert_eq!(view.summary.provider, "codex");
            assert_eq!(view.summary.title, "Footer");
            assert_eq!(view.settings.model.as_deref(), Some("gpt-5.5"));
            let handoff = view
                .settings
                .handoff
                .clone()
                .expect("the recap waits for the first send");
            assert_eq!(handoff.from, "claude");
            assert!(handoff.brief.contains("Use two columns instead"));
            // The copied turns keep the provider that answered them.
            assert_eq!(view.settings.turns[&1].provider, "claude");
            let saved = locked(&shell.store).unwrap().load(id).unwrap().rows;
            assert_eq!(saved, chat);
        })
        .unwrap();
    // The banner shows the recap, not only a label.
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("handoff-recap").is_some());
}

#[gpui::test]
fn a_handoff_from_an_empty_chat_opens_empty_without_a_recap(cx: &mut TestAppContext) {
    let (window, source) = fixture(&[], cx);
    handoff(window, source, cx);
    let id = opened(window, source, cx);
    assert!(history(window, id, cx).is_empty());
    window
        .update(cx, |shell, _, cx| {
            let handoff = shell.sessions[&id]
                .read(cx)
                .settings
                .handoff
                .clone()
                .unwrap();
            assert_eq!(handoff.brief, "");
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("handoff-recap").is_none());
}

#[gpui::test]
fn forking_at_a_response_copies_the_chat_through_that_turn(cx: &mut TestAppContext) {
    let chat = footer_chat();
    let (window, source) = fixture(&chat, cx);
    request(window, source, ResponseRequest::Fork { turn: 1 }, cx);
    let id = opened(window, source, cx);
    assert_eq!(history(window, id, cx), chat[..2]);
    window
        .update(cx, |shell, _, cx| {
            let view = shell.sessions[&id].read(cx);
            assert_eq!(view.summary.title, "Fork of Footer");
            // Same agent and model until the user picks another.
            assert_eq!(view.summary.provider, "claude");
            assert_eq!(view.summary.provider_id, None);
            assert_eq!(view.settings.model.as_deref(), Some("opus"));
            assert_eq!(view.settings.effort.as_deref(), Some("high"));
            assert!(view.settings.handoff.is_none());
            assert_eq!(view.settings.turns.keys().collect::<Vec<_>>(), [&1]);
            assert_eq!(view.draft_text(cx), "");
            // The source chat is untouched.
            let saved = locked(&shell.store).unwrap().load(source).unwrap().rows;
            assert_eq!(saved, chat);
        })
        .unwrap();
}

#[gpui::test]
fn forking_at_the_last_message_copies_the_whole_chat(cx: &mut TestAppContext) {
    let chat = footer_chat();
    let (window, source) = fixture(&chat, cx);
    request(
        window,
        source,
        ResponseRequest::Fork {
            turn: chat.len() - 1,
        },
        cx,
    );
    let id = opened(window, source, cx);
    assert_eq!(history(window, id, cx), chat);
}

#[gpui::test]
fn forking_at_the_first_message_starts_empty_with_it_in_the_composer(cx: &mut TestAppContext) {
    let (window, source) = fixture(&footer_chat(), cx);
    request(window, source, ResponseRequest::Fork { turn: 0 }, cx);
    let id = opened(window, source, cx);
    window
        .update(cx, |shell, _, cx| {
            let view = shell.sessions[&id].read(cx);
            assert!(view.transcript.read(cx).history_rows().is_empty());
            assert_eq!(view.draft_text(cx), "Plan the footer");
        })
        .unwrap();
}

#[gpui::test]
fn forking_at_a_later_prompt_branches_before_it(cx: &mut TestAppContext) {
    let chat = footer_chat();
    let (window, source) = fixture(&chat, cx);
    request(window, source, ResponseRequest::Fork { turn: 2 }, cx);
    let id = opened(window, source, cx);
    assert_eq!(history(window, id, cx), chat[..2]);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(
                shell.sessions[&id].read(cx).draft_text(cx),
                "Use two columns instead"
            );
        })
        .unwrap();
}
