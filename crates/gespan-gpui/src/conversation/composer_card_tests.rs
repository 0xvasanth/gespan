use super::composer_card::ComposerCard;
use super::*;
use gespan_core::{inbox::Item, notes::Note, store::Store};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::sync::Mutex;

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    // Fail after saving the turn, without ever launching a real provider.
    let mut summary = store.create("codex", "/missing/card-project").unwrap();
    summary.provider = "fixture".into();
    cx.update(|cx| {
        input::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    })
}
fn item() -> ComposerCard {
    ComposerCard::Inbox {
        item: Box::new(Item {
            provider: "github".into(),
            kind: "pr".into(),
            number: 42,
            title: "Fix the parser".into(),
            url: "https://github.com/acme/app/pull/42".into(),
            repo: "acme/app".into(),
            ..Item::default()
        }),
        prompt: "Work on #42 Fix the parser".into(),
    }
}
fn note() -> ComposerCard {
    ComposerCard::Note(Box::new(Note {
        id: "note-1".into(),
        slug: "read-me".into(),
        title: "Reference".into(),
        body: "The secret sauce".into(),
        tags: Vec::new(),
        source_session_id: None,
        source_cwd: Some("/repo".into()),
        slug_pending: false,
        created_at: 0,
        updated_at: 0,
    }))
}
fn roles_and_prompt(view: &Conversation, cx: &gpui::App) -> (Vec<String>, String) {
    let rows = view.transcript.read(cx).source_rows();
    (
        rows.iter().map(|row| row.role.clone()).collect(),
        rows.iter()
            .find(|row| row.role == "user")
            .map(|row| row.text.clone())
            .unwrap_or_default(),
    )
}

#[gpui::test]
fn an_inbox_card_shows_above_the_composer_and_sends_with_or_without_a_note(
    cx: &mut TestAppContext,
) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.set_composer_card(item(), cx);
            assert_eq!(
                view.input.read(cx).placeholder().as_ref(),
                "Add a note, or send to start…"
            );
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("composer-card").is_some());
    window
        .update(visual, |view, _, cx| {
            view.input.update(cx, |input, cx| {
                input.set_text("Start with the tests".into(), cx)
            });
            view.send(cx);
            let (_, prompt) = roles_and_prompt(view, cx);
            assert_eq!(prompt, "Work on #42 Fix the parser\n\nStart with the tests");
            assert!(
                view.composer_card.is_none(),
                "the card goes with the message"
            );
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("composer-card").is_none());
}

#[gpui::test]
fn an_inbox_card_alone_is_enough_to_send(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.set_composer_card(item(), cx);
            view.send(cx);
            assert_eq!(roles_and_prompt(view, cx).1, "Work on #42 Fix the parser");
        })
        .unwrap();
}

#[gpui::test]
fn removing_a_card_restores_the_composer_and_sends_nothing(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let placeholder = window
        .update(cx, |view, _, cx| {
            let placeholder = view.input.read(cx).placeholder().clone();
            view.set_composer_card(note(), cx);
            placeholder
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let remove = visual.debug_bounds("composer-card-remove").expect("remove");
    visual.simulate_click(remove.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, cx| {
            assert!(view.composer_card.is_none());
            assert_eq!(view.input.read(cx).placeholder(), &placeholder);
            view.send(cx);
            assert!(roles_and_prompt(view, cx).0.is_empty(), "nothing to send");
        })
        .unwrap();
}

#[gpui::test]
fn a_note_stays_a_card_on_the_turn_and_only_the_agent_gets_its_body(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.set_composer_card(note(), cx);
            view.input
                .update(cx, |input, cx| input.set_text("Follow this".into(), cx));
            view.send(cx);
            let (roles, prompt) = roles_and_prompt(view, cx);
            assert_eq!(prompt, "Follow this", "the transcript keeps what was typed");
            assert!(
                roles
                    .iter()
                    .any(|role| role == gespan_core::notes::CARD_ROLE),
                "{roles:?}"
            );
            let card = view
                .transcript
                .read(cx)
                .source_rows()
                .iter()
                .find(|row| row.role == gespan_core::notes::CARD_ROLE)
                .map(|row| row.text.clone())
                .unwrap();
            assert!(card.contains("Reference") && card.contains("read-me"));
            assert!(
                !card.contains("secret sauce"),
                "the body stays out of the card"
            );
            assert!(view.pending_note.is_none(), "it went with this turn");
        })
        .unwrap();
}

#[gpui::test]
fn a_note_sent_while_busy_waits_as_a_reference_resolved_at_send(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            view.set_composer_card(note(), cx);
            view.input
                .update(cx, |input, cx| input.set_text("Then this".into(), cx));
            view.send(cx);
            assert_eq!(view.follow_ups.state.messages.len(), 1);
            assert_eq!(
                view.follow_ups.state.messages[0].text,
                "Then this @note/read-me"
            );
            assert!(view.pending_note.is_none(), "no later turn picks it up");
        })
        .unwrap();
}
