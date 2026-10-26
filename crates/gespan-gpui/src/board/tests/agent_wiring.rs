//! The agent side of linked sessions (C5-11): write-back comments, the
//! needs-input and PR auto-move events, and how the timeline names agents.
use super::link_run::{FakeSource, add_link, install_turns, look_up_with, new_session, waiting};
use super::*;
use crate::board::turns;
use gespan_core::board::extras::SettingsPatch;

fn settle(
    f: &Fixture,
    cx: &mut TestAppContext,
    session: i64,
    turn: u64,
    end: (bool, bool),
    reply: &str,
) {
    let store = f.store.clone();
    let (stopped, ok) = end;
    let reply = Some(reply.to_string());
    cx.update(|cx| {
        turns::turn_started(&store, session, None, turn, cx);
        turns::turn_settled(&store, session, turn, stopped, ok, reply, cx);
    });
    cx.run_until_parked();
}

/// A new board with one card, its settings patched, the card linked to `key`.
fn board_card(f: &Fixture, name: &str, key: &str, patch: SettingsPatch) -> String {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.create(name).unwrap();
    let backlog = service.get(&board.id).unwrap().columns[0].id.clone();
    let card = service.card_create(&backlog, name, "").unwrap();
    service.update_settings(&board.id, &patch).unwrap();
    add_link(f, &card.id, "session", key);
    card.id
}

fn comments(f: &Fixture, card: &str) -> Vec<(String, String)> {
    Service::open(&f.db)
        .unwrap()
        .comments(card)
        .unwrap()
        .into_iter()
        .map(|comment| (comment.author, comment.body))
        .collect()
}

fn write_back_off() -> SettingsPatch {
    SettingsPatch {
        write_back: Some(false),
        ..Default::default()
    }
}

#[gpui::test]
fn completed_turn_writes_one_agent_comment_per_linked_card(cx: &mut TestAppContext) {
    let f = open(cx);
    install_turns(&f, cx);
    let (session, key) = new_session(&f);
    let one = board_card(&f, "One", &key, Default::default());
    let two = board_card(&f, "Two", &key, Default::default());
    settle(&f, cx, session, 1, (false, true), "Fixed the login check.");
    let expected = vec![(format!("agent:{key}"), "Fixed the login check.".to_string())];
    assert_eq!(comments(&f, &one), expected);
    assert_eq!(comments(&f, &two), expected);
    // The same turn settling again writes nothing more: once per turn.
    let store = f.store.clone();
    cx.update(|cx| turns::turn_settled(&store, session, 1, false, true, Some("Again".into()), cx));
    cx.run_until_parked();
    assert_eq!(comments(&f, &one).len(), 1);
    // The next completed turn writes its own reply.
    settle(&f, cx, session, 2, (false, true), "Added a test.");
    assert_eq!(comments(&f, &one).len(), 2);
    assert_eq!(comments(&f, &one)[1].1, "Added a test.");
}

#[gpui::test]
fn write_back_off_writes_no_comment(cx: &mut TestAppContext) {
    let f = open(cx);
    install_turns(&f, cx);
    let (session, key) = new_session(&f);
    let quiet = board_card(&f, "Quiet", &key, write_back_off());
    // Review Focus 3: the same session's card on a board with write-back on
    // still gets its comment.
    let loud = board_card(&f, "Loud", &key, Default::default());
    settle(&f, cx, session, 1, (false, true), "Done.");
    assert!(comments(&f, &quiet).is_empty());
    assert_eq!(comments(&f, &loud).len(), 1);
}

#[gpui::test]
fn failed_stopped_or_empty_turns_write_no_comment(cx: &mut TestAppContext) {
    let f = open(cx);
    install_turns(&f, cx);
    let (session, key) = new_session(&f);
    let card = board_card(&f, "Work", &key, Default::default());
    settle(&f, cx, session, 1, (false, false), "Half done");
    settle(&f, cx, session, 2, (true, false), "Stopped here");
    settle(&f, cx, session, 3, (true, true), "Stopped after finishing");
    settle(&f, cx, session, 4, (false, true), "   ");
    assert!(comments(&f, &card).is_empty());
}

#[gpui::test]
fn a_tracked_turn_carries_its_final_reply_to_write_back(cx: &mut TestAppContext) {
    let f = open(cx);
    install_turns(&f, cx);
    let (session, key) = new_session(&f);
    let card = board_card(&f, "Work", &key, Default::default());
    let mut tracker = turns::TurnTracker::default();
    let store = f.store.clone();
    let update = |finished: bool, reply: Option<&str>| turns::TurnUpdate {
        finished,
        stopping: false,
        succeeded: finished,
        has_rows: true,
        reply: reply.map(str::to_string),
    };
    cx.update(|cx| tracker.observe(&store, session, 1, update(false, None), cx));
    cx.update(|cx| tracker.observe(&store, session, 1, update(true, Some("The plan: …")), cx));
    cx.run_until_parked();
    assert_eq!(
        comments(&f, &card),
        [(format!("agent:{key}"), "The plan: …".to_string())]
    );
}

fn column_of(f: &Fixture, card: &str) -> String {
    let mut service = Service::open(&f.db).unwrap();
    let card = service.card(card).unwrap();
    let detail = service.get(&card.board_id).unwrap();
    detail
        .columns
        .iter()
        .find(|column| column.id == card.column_id)
        .unwrap()
        .name
        .clone()
}

fn kinds(f: &Fixture, card: &str, kind: &str) -> usize {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.card(card).unwrap().board_id;
    service
        .activity(&board, Some(card), 50)
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == kind)
        .count()
}

/// A board whose rule sends `event` to its Review column.
fn rule_to_review(f: &Fixture, name: &str, event: &str) -> (String, String) {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.create(name).unwrap();
    let columns = service.get(&board.id).unwrap().columns;
    let card = service.card_create(&columns[0].id, name, "").unwrap();
    let rules = format!(r#"{{"{event}":{{"column":"{}"}}}}"#, columns[2].id);
    service
        .update_settings(
            &board.id,
            &SettingsPatch {
                auto_move_json: Some(Some(rules)),
                ..Default::default()
            },
        )
        .unwrap();
    (board.id, card.id)
}

#[gpui::test]
fn needs_input_event_moves_card_per_custom_rule(cx: &mut TestAppContext) {
    let f = open(cx);
    install_turns(&f, cx);
    let (session, key) = new_session(&f);
    let (_, card) = rule_to_review(&f, "Ask", "needs_input");
    add_link(&f, &card, "session", &key);
    // A default board keeps the card where it is.
    let (_, stays) = rule_to_review(&f, "Plain", "pr_opened");
    add_link(&f, &stays, "session", &key);
    waiting(&f, cx, session, true);
    assert_eq!(column_of(&f, &card), "Review");
    assert_eq!(kinds(&f, &card, "needs_input"), 1);
    assert_eq!(column_of(&f, &stays), "Backlog");
    assert_eq!(kinds(&f, &stays, "needs_input"), 1);
    // Still waiting: no second event. Waiting again later is a new one.
    waiting(&f, cx, session, true);
    assert_eq!(kinds(&f, &card, "needs_input"), 1);
    waiting(&f, cx, session, false);
    waiting(&f, cx, session, true);
    assert_eq!(kinds(&f, &card, "needs_input"), 2);
}

#[gpui::test]
fn pr_merged_moves_card_once(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Shown"]);
    install_turns(&f, cx);
    let url = "https://github.com/acme/app/pull/42";
    let (_, card) = rule_to_review(&f, "Ship", "pr_merged");
    let mut service = Service::open(&f.db).unwrap();
    service.set_projects(&card, &["/work/app".into()]).unwrap();
    add_link(&f, &card, "pr", url);
    // The open board shows a card linked to the same PR, so its refresh
    // looks the PR up.
    let shown = card_id(&f, cx, "Shown");
    service.set_projects(&shown, &["/work/app".into()]).unwrap();
    add_link(&f, &shown, "pr", url);
    let merged = || FakeSource {
        pr: Some((url.into(), 42, "MERGED".into())),
        ..FakeSource::default()
    };
    look_up_with(&f, cx, merged());
    cx.run_until_parked();
    assert_eq!(column_of(&f, &card), "Review");
    assert_eq!(kinds(&f, &card, "pr_merged"), 1);
    assert_eq!(kinds(&f, &shown, "pr_merged"), 1);
    // The user moves it back; refreshes that see the merge again move nothing.
    let board = service.card(&card).unwrap().board_id;
    let backlog = service.get(&board).unwrap().columns[0].id.clone();
    service.card_move(&card, &backlog, 0, "user").unwrap();
    look_up_with(&f, cx, merged());
    // A new app run forgets what it sent; the store still fires once per card.
    install_turns(&f, cx);
    look_up_with(&f, cx, merged());
    cx.run_until_parked();
    assert_eq!(column_of(&f, &card), "Backlog");
    assert_eq!(kinds(&f, &card, "pr_merged"), 1);
}

#[gpui::test]
fn pr_seen_open_fires_pr_opened(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Shown"]);
    install_turns(&f, cx);
    let url = "https://github.com/acme/app/pull/7";
    let shown = card_id(&f, cx, "Shown");
    Service::open(&f.db)
        .unwrap()
        .set_projects(&shown, &["/work/app".into()])
        .unwrap();
    add_link(&f, &shown, "pr", url);
    look_up_with(
        &f,
        cx,
        FakeSource {
            pr: Some((url.into(), 7, "OPEN".into())),
            ..FakeSource::default()
        },
    );
    cx.run_until_parked();
    assert_eq!(kinds(&f, &shown, "pr_opened"), 1);
    assert_eq!(kinds(&f, &shown, "pr_merged"), 0);
}

#[test]
fn activity_timeline_labels_agent_actors_as_agent() {
    let row = |actor: &str| gespan_core::board::Activity {
        id: "a".into(),
        board_id: "b".into(),
        card_id: Some("c".into()),
        kind: "subtask_done".into(),
        actor: actor.into(),
        from_column: None,
        to_column: None,
        detail: None,
        created_at: 0,
    };
    let label = |actor: &str| {
        activity_text(&row(actor))
            .rsplit(" · ")
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(label("agent:42"), "Agent");
    assert_eq!(label("agent:remote:env-1:s-9"), "Agent");
    assert_eq!(label("agent"), "Agent");
    assert_eq!(label("user"), "You");
    assert_eq!(label("system"), "System");
}
