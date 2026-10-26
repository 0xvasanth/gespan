//! Agent work on the board: auto-move rules, write-back, blockers, the "Run
//! with…" prompt and the card tools.
use gespan_board_store::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

fn fresh() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn column(id: &str, position: i64, role: Option<&str>) -> Column {
    Column {
        id: id.into(),
        board_id: "b".into(),
        name: id.into(),
        position,
        role: role.map(Into::into),
        color: None,
        wip_limit: None,
        collapsed: false,
    }
}

fn roles() -> Vec<Column> {
    vec![
        column("todo", 0, Some("todo")),
        column("doing", 1, Some("doing")),
        column("review", 2, Some("review")),
        column("done", 3, Some("done")),
    ]
}

fn target<'a>(
    columns: &'a [Column],
    from: &str,
    event: LinkedEvent,
    rules: &AutoMoveRules,
) -> Option<&'a str> {
    let current = columns.iter().find(|c| c.id == from).unwrap();
    auto_move_target(columns, current, event, rules).map(|c| c.id.as_str())
}

fn defaults(columns: &[Column], from: &str, event: LinkedEvent) -> Option<String> {
    target(columns, from, event, &AutoMoveRules::default()).map(str::to_string)
}

fn rules_json(conn: &Connection, board: &Board, raw: &str) {
    update_board_settings(
        conn,
        &board.id,
        &SettingsPatch {
            auto_move_json: Some(Some(raw.into())),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
}

fn write_back_on(conn: &Connection, board: &Board, on: bool) {
    update_board_settings(
        conn,
        &board.id,
        &SettingsPatch {
            write_back: Some(on),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
}

fn columns_of(conn: &Connection, board: &Board) -> Vec<Column> {
    get_board(conn, &board.id).unwrap().columns
}

/// The card's column name and its activity kinds, newest first.
fn where_is(conn: &Connection, card_id: &str) -> (String, Vec<String>) {
    let card = get_card(conn, card_id).unwrap();
    let detail = get_board(conn, &card.board_id).unwrap();
    let column = detail
        .columns
        .iter()
        .find(|c| c.id == card.column_id)
        .unwrap()
        .name
        .clone();
    let kinds = list_activity(conn, &card.board_id, Some(card_id), 100)
        .unwrap()
        .into_iter()
        .map(|row| row.kind)
        .collect();
    (column, kinds)
}

fn count_kind(kinds: &[String], kind: &str) -> usize {
    kinds.iter().filter(|k| *k == kind).count()
}

/// A board with the default columns and one card in `column_index`, linked to
/// `session`.
fn linked_card(conn: &Connection, name: &str, column_index: usize, session: &str) -> (Board, Card) {
    let board = create_board(conn, name).unwrap();
    let column = columns_of(conn, &board)[column_index].id.clone();
    let card = create_card(conn, &column, &format!("{name} card"), "").unwrap();
    add_link(conn, &card.id, "session", session, None).unwrap();
    (board, card)
}

#[test]
fn default_rules_match_todays_forward_moves() {
    use LinkedEvent::*;
    let all = roles();
    // started -> doing, forward only.
    assert_eq!(defaults(&all, "todo", Started).as_deref(), Some("doing"));
    assert_eq!(defaults(&all, "doing", Started), None);
    assert_eq!(defaults(&all, "review", Started), None);
    assert_eq!(defaults(&all, "done", Started), None);
    // completed -> review from doing or todo, never back out of review/done.
    assert_eq!(
        defaults(&all, "doing", Completed).as_deref(),
        Some("review")
    );
    assert_eq!(defaults(&all, "todo", Completed).as_deref(), Some("review"));
    assert_eq!(defaults(&all, "review", Completed), None);
    assert_eq!(defaults(&all, "done", Completed), None);
    // No review column: completed falls back to done.
    let no_review: Vec<Column> = roles().into_iter().filter(|c| c.id != "review").collect();
    assert_eq!(
        defaults(&no_review, "doing", Completed).as_deref(),
        Some("done")
    );
    // No doing column: started stays.
    let no_doing: Vec<Column> = roles().into_iter().filter(|c| c.id != "doing").collect();
    assert_eq!(defaults(&no_doing, "todo", Started), None);
    // Neither review nor done: completed stays.
    let two = vec![
        column("todo", 0, Some("todo")),
        column("doing", 1, Some("doing")),
    ];
    assert_eq!(defaults(&two, "doing", Completed), None);
    // Columns out of position order.
    let mut shuffled = roles();
    shuffled.reverse();
    assert_eq!(
        defaults(&shuffled, "todo", Started).as_deref(),
        Some("doing")
    );
    assert_eq!(
        defaults(&shuffled, "doing", Completed).as_deref(),
        Some("review")
    );
    // A user column between doing and review.
    let qa = vec![
        column("todo", 0, Some("todo")),
        column("doing", 1, Some("doing")),
        column("qa", 2, None),
        column("review", 3, Some("review")),
        column("done", 4, Some("done")),
    ];
    assert_eq!(defaults(&qa, "qa", Completed).as_deref(), Some("review"));
    assert_eq!(defaults(&qa, "qa", Started), None);
    // A role-less column left of doing counts as before it.
    let inbox = vec![
        column("inbox", 0, None),
        column("doing", 1, Some("doing")),
        column("done", 2, Some("done")),
    ];
    assert_eq!(defaults(&inbox, "inbox", Started).as_deref(), Some("doing"));
    // Every other event stays by default.
    for event in [Failed, NeedsInput, PrOpened, PrMerged] {
        for from in ["todo", "doing", "review", "done"] {
            assert_eq!(defaults(&all, from, event), None, "{event:?} from {from}");
        }
    }
    // The defaults as data, and the snake_case event names.
    let rules = AutoMoveRules::default();
    assert_eq!(rules.started, RuleTarget::Default);
    assert_eq!(rules.completed, RuleTarget::Default);
    for rule in [
        &rules.needs_input,
        &rules.failed,
        &rules.pr_opened,
        &rules.pr_merged,
    ] {
        assert_eq!(*rule, RuleTarget::Stay);
    }
    let names: Vec<&str> = [Started, NeedsInput, Completed, Failed, PrOpened, PrMerged]
        .iter()
        .map(|event| event.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "started",
            "needs_input",
            "completed",
            "failed",
            "pr_opened",
            "pr_merged"
        ]
    );
    assert_eq!(serde_json::to_value(PrMerged).unwrap(), json!("pr_merged"));
    assert_eq!(
        serde_json::from_value::<LinkedEvent>(json!("needs_input")).unwrap(),
        NeedsInput
    );

    // End to end: each linked card moves on its own board as `system`, the
    // event recorded after the move; a card that must stay only records.
    let conn = fresh();
    let (a, one) = linked_card(&conn, "A", 0, "s-1");
    let (b, two) = linked_card(&conn, "B", 0, "s-1");
    let (_, shipped) = linked_card(&conn, "C", 3, "s-1");
    let touched = on_linked_work(&conn, "session", "s-1", Started).unwrap();
    for card in [&one, &two] {
        let (column, kinds) = where_is(&conn, &card.id);
        assert_eq!(column, "In Progress");
        assert_eq!(kinds[..2], ["started".to_string(), "moved".to_string()]);
        let moved = list_activity(&conn, &card.board_id, Some(&card.id), 10)
            .unwrap()
            .into_iter()
            .find(|row| row.kind == "moved")
            .unwrap();
        assert_eq!(moved.actor, "system");
    }
    let (column, kinds) = where_is(&conn, &shipped.id);
    assert_eq!(column, "Done");
    assert_eq!(count_kind(&kinds, "started"), 1);
    assert_eq!(count_kind(&kinds, "moved"), 0);
    assert_eq!(touched.len(), 3);
    assert!(touched.contains(&a.id) && touched.contains(&b.id));
    // Completed work lands at the end of review; failed runs record only.
    let review = columns_of(&conn, &a)[2].id.clone();
    create_card(&conn, &review, "Already", "").unwrap();
    on_linked_work(&conn, "session", "s-1", Completed).unwrap();
    let moved = get_card(&conn, &one.id).unwrap();
    assert_eq!(
        (moved.column_id.as_str(), moved.position),
        (review.as_str(), 1)
    );
    on_linked_work(&conn, "session", "s-1", Failed).unwrap();
    let (column, kinds) = where_is(&conn, &one.id);
    assert_eq!((column.as_str(), kinds[0].as_str()), ("Review", "failed"));
    // A link with no cards touches nothing.
    assert!(on_linked_work(&conn, "session", "nobody", Started)
        .unwrap()
        .is_empty());
}

#[test]
fn on_linked_work_keeps_going_after_a_failing_card_then_reports_it() {
    let conn = fresh();
    let board = create_board(&conn, "A").unwrap();
    let backlog = columns_of(&conn, &board)[0].id.clone();
    let broken = create_card(&conn, &backlog, "Broken", "").unwrap();
    let fine = create_card(&conn, &backlog, "Fine", "").unwrap();
    for card in [&broken, &fine] {
        add_link(&conn, &card.id, "session", "s-1", None).unwrap();
    }
    conn.execute_batch(&format!(
        "CREATE TEMP TRIGGER refuse BEFORE INSERT ON board_activity \
         WHEN NEW.card_id = '{}' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        broken.id
    ))
    .unwrap();
    let error = on_linked_work(&conn, "session", "s-1", LinkedEvent::Started).unwrap_err();
    assert!(error.contains("refused"), "{error}");
    assert_eq!(where_is(&conn, &fine.id).0, "In Progress");
    // The broken card's move and record share one transaction: neither stuck.
    assert_eq!(get_card(&conn, &broken.id).unwrap().column_id, backlog);
}

#[test]
fn custom_rule_moves_to_the_chosen_column_even_leftward() {
    let all = roles();
    let rules = AutoMoveRules {
        completed: RuleTarget::Column("todo".into()),
        failed: RuleTarget::Column("doing".into()),
        ..AutoMoveRules::default()
    };
    assert_eq!(
        target(&all, "review", LinkedEvent::Completed, &rules),
        Some("todo")
    );
    assert_eq!(
        target(&all, "done", LinkedEvent::Failed, &rules),
        Some("doing")
    );
    // Already there: nothing to do.
    assert_eq!(target(&all, "todo", LinkedEvent::Completed, &rules), None);

    let conn = fresh();
    let (board, card) = linked_card(&conn, "A", 2, "s-1");
    let backlog = columns_of(&conn, &board)[0].id.clone();
    rules_json(
        &conn,
        &board,
        &json!({ "failed": { "column": backlog } }).to_string(),
    );
    assert_eq!(
        auto_move_rules(&board_settings(&conn, &board.id).unwrap()).failed,
        RuleTarget::Column(backlog.clone())
    );
    on_linked_work(&conn, "session", "s-1", LinkedEvent::Failed).unwrap();
    let (column, kinds) = where_is(&conn, &card.id);
    assert_eq!(column, "Backlog");
    assert_eq!(kinds[..2], ["failed".to_string(), "moved".to_string()]);
    // A "stay" rule overrides the default forward move.
    rules_json(&conn, &board, r#"{"started":"stay"}"#);
    on_linked_work(&conn, "session", "s-1", LinkedEvent::Started).unwrap();
    assert_eq!(where_is(&conn, &card.id).0, "Backlog");
}

#[test]
fn rule_naming_a_deleted_column_stays() {
    let all = roles();
    let rules = AutoMoveRules {
        started: RuleTarget::Column("gone".into()),
        ..AutoMoveRules::default()
    };
    assert_eq!(target(&all, "todo", LinkedEvent::Started, &rules), None);

    let conn = fresh();
    let (board, card) = linked_card(&conn, "A", 0, "s-1");
    let extra = create_column(&conn, &board.id, "Parking", None).unwrap();
    rules_json(
        &conn,
        &board,
        &json!({ "started": { "column": extra.id } }).to_string(),
    );
    delete_column(&conn, &extra.id).unwrap();
    // A column on another board is just as missing.
    let other = create_board(&conn, "Other").unwrap();
    let foreign = columns_of(&conn, &other)[1].id.clone();
    on_linked_work(&conn, "session", "s-1", LinkedEvent::Started).unwrap();
    let (column, kinds) = where_is(&conn, &card.id);
    assert_eq!(column, "Backlog");
    assert_eq!(kinds[0], "started");
    rules_json(
        &conn,
        &board,
        &json!({ "started": { "column": foreign } }).to_string(),
    );
    on_linked_work(&conn, "session", "s-1", LinkedEvent::Started).unwrap();
    assert_eq!(where_is(&conn, &card.id).0, "Backlog");
}

#[test]
fn pr_merged_moves_only_when_configured_and_fires_once() {
    let conn = fresh();
    // Default rules: recorded, never moved.
    let (plain, stays) = linked_card(&conn, "Plain", 2, "pr-unused");
    remove_link(&conn, &stays.id, "session", "pr-unused").unwrap();
    add_link(&conn, &stays.id, "pr", "org/repo#7", None).unwrap();
    // The agent-pipeline template routes pr_merged to Merged.
    let pipeline = create_board_from_template(&conn, "Pipeline", "agent-pipeline").unwrap();
    let pipeline_columns = columns_of(&conn, &pipeline);
    let merged = pipeline_columns
        .iter()
        .find(|c| c.name == "Merged")
        .unwrap()
        .clone();
    let review = pipeline_columns
        .iter()
        .find(|c| c.name == "Needs review")
        .unwrap()
        .id
        .clone();
    let moves = create_card(&conn, &review, "Ship it", "").unwrap();
    add_link(&conn, &moves.id, "pr", "org/repo#7", None).unwrap();

    let touched = on_linked_work(&conn, "pr", "org/repo#7", LinkedEvent::PrMerged).unwrap();
    assert_eq!(touched.len(), 2);
    let (column, kinds) = where_is(&conn, &stays.id);
    assert_eq!(column, "Review");
    assert_eq!(count_kind(&kinds, "pr_merged"), 1);
    let card = get_card(&conn, &moves.id).unwrap();
    assert_eq!(card.column_id, merged.id);
    assert_eq!(count_kind(&where_is(&conn, &moves.id).1, "pr_merged"), 1);

    // The user drags it back; a repeated event neither moves nor records.
    move_card(&conn, &moves.id, &review, 0, "user").unwrap();
    let again = on_linked_work(&conn, "pr", "org/repo#7", LinkedEvent::PrMerged).unwrap();
    assert!(again.is_empty(), "{again:?}");
    assert_eq!(get_card(&conn, &moves.id).unwrap().column_id, review);
    assert_eq!(count_kind(&where_is(&conn, &moves.id).1, "pr_merged"), 1);
    assert_eq!(count_kind(&where_is(&conn, &stays.id).1, "pr_merged"), 1);
    let _ = plain;

    // pr_opened is deduplicated on its own kind.
    on_linked_work(&conn, "pr", "org/repo#7", LinkedEvent::PrOpened).unwrap();
    on_linked_work(&conn, "pr", "org/repo#7", LinkedEvent::PrOpened).unwrap();
    assert_eq!(count_kind(&where_is(&conn, &moves.id).1, "pr_opened"), 1);
    // Other events are never deduplicated.
    on_linked_work(&conn, "pr", "org/repo#7", LinkedEvent::Failed).unwrap();
    on_linked_work(&conn, "pr", "org/repo#7", LinkedEvent::Failed).unwrap();
    assert_eq!(count_kind(&where_is(&conn, &moves.id).1, "failed"), 2);
}

#[test]
fn needs_input_records_without_moving_by_default() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 1, "s-1");
    on_linked_work(&conn, "session", "s-1", LinkedEvent::NeedsInput).unwrap();
    let (column, kinds) = where_is(&conn, &card.id);
    assert_eq!(column, "In Progress");
    assert_eq!(kinds[0], "needs_input");
    assert_eq!(count_kind(&kinds, "moved"), 0);
    // The new kinds can be recorded directly too.
    for kind in ["needs_input", "pr_opened", "pr_merged"] {
        record_card(&conn, &card.id, kind, None).unwrap();
    }
    assert!(record_card(&conn, &card.id, "moved", None).is_err());
}

#[test]
fn bad_auto_move_json_reads_as_defaults() {
    let read = |raw: Option<&str>| {
        auto_move_rules(&BoardSettings {
            auto_move_json: raw.map(Into::into),
            ..BoardSettings::default()
        })
    };
    let defaults = AutoMoveRules::default();
    for raw in [None, Some("{"), Some("[]"), Some("\"stay\""), Some("null")] {
        assert_eq!(read(raw), defaults, "{raw:?}");
    }
    // A rule that can't be read falls back to that event's default; the rest
    // still apply. Unknown events are ignored.
    let mixed = read(Some(
        r#"{"started":"doing","completed":{"column":7},"failed":{"column":"c9"},
            "pr_opened":"default","later":"stay"}"#,
    ));
    assert_eq!(
        mixed,
        AutoMoveRules {
            failed: RuleTarget::Column("c9".into()),
            pr_opened: RuleTarget::Default,
            ..AutoMoveRules::default()
        }
    );
    // A board whose stored JSON is broken still moves forward on start.
    let conn = fresh();
    let (board, card) = linked_card(&conn, "A", 0, "s-1");
    conn.execute(
        "UPDATE board_settings SET auto_move_json = '{' WHERE board_id = ?1",
        [&board.id],
    )
    .unwrap();
    on_linked_work(&conn, "session", "s-1", LinkedEvent::Started).unwrap();
    assert_eq!(where_is(&conn, &card.id).0, "In Progress");
}

#[test]
fn write_back_comments_only_where_write_back_is_on_marked_agent_session() {
    let conn = fresh();
    let (on, on_card) = linked_card(&conn, "On", 1, "s-1");
    let (off, off_card) = linked_card(&conn, "Off", 1, "s-1");
    write_back_on(&conn, &off, false);
    let (_, other) = linked_card(&conn, "Elsewhere", 1, "s-2");
    // An archived linked card gets nothing.
    let column = columns_of(&conn, &on)[0].id.clone();
    let archived = create_card(&conn, &column, "Old", "").unwrap();
    add_link(&conn, &archived.id, "session", "s-1", None).unwrap();
    archive_card(&conn, &archived.id).unwrap();

    let commented = write_back(&conn, "s-1", "  Fixed the login bug.\n").unwrap();
    assert_eq!(commented, vec![on_card.id.clone()]);
    let comments = list_comments(&conn, &on_card.id).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "Fixed the login bug.");
    assert_eq!(comments[0].author, "agent:s-1");
    for card in [&off_card, &other, &archived] {
        assert!(list_comments(&conn, &card.id).unwrap().is_empty());
    }
    // Default settings (no row) mean write-back on.
    assert!(board_settings(&conn, &on.id).unwrap().write_back);
}

#[test]
fn write_back_truncates_to_4000_chars_and_skips_empty() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 1, "s-1");
    for empty in ["", "   \n\t "] {
        assert!(write_back(&conn, "s-1", empty).unwrap().is_empty());
    }
    assert!(list_comments(&conn, &card.id).unwrap().is_empty());
    // Exactly 4000 characters stays whole.
    let exact = "é".repeat(4000);
    write_back(&conn, "s-1", &exact).unwrap();
    // Longer is cut to 4000 characters ending in "…".
    let long = format!("{}{}", "a".repeat(3999), "bcdef");
    write_back(&conn, "s-1", &long).unwrap();
    let comments = list_comments(&conn, &card.id).unwrap();
    assert_eq!(comments[0].body, exact);
    assert_eq!(comments[1].body.chars().count(), 4000);
    assert_eq!(comments[1].body, format!("{}…", "a".repeat(3999)));
    // A session key that can't be an author is refused, writing nothing.
    for bad in ["", "  ", "bad\nkey"] {
        assert!(write_back(&conn, bad, "hello").is_err(), "{bad:?}");
    }
    assert_eq!(list_comments(&conn, &card.id).unwrap().len(), 2);
}

#[test]
fn write_back_accepts_a_remote_session_key() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 1, "remote:env:sess");
    let commented = write_back(&conn, "remote:env:sess", "Done remotely").unwrap();
    assert_eq!(commented, vec![card.id.clone()]);
    let comments = list_comments(&conn, &card.id).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].author, "agent:remote:env:sess");
    assert_eq!(comments[0].body, "Done remotely");
}

#[test]
fn subtasks_for_a_session_with_no_linked_cards_is_empty() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 1, "s-1");
    add_subtask(&conn, &card.id, "Not reachable").unwrap();
    assert_eq!(
        answer_card(&conn, "s-none", &CardTool::Subtasks, false).unwrap(),
        json!({ "cards": [] })
    );
}

#[test]
fn agent_title_checks_every_ticket_link_not_only_the_newest() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 0, "s-1");
    update_card(&conn, &card.id, "Old ticket title", "").unwrap();
    set_ticket(
        &conn,
        &card.id,
        Some(r#"{"tracker":"jira","key":"ABC-1","title":"old ticket title"}"#),
    )
    .unwrap();
    // A second ticket row (written by another build) that is newer.
    conn.execute(
        "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
         VALUES (?1, 'linear', ?2, 'ticket', 9999999999999)",
        [
            card.id.as_str(),
            r#"{"tracker":"linear","key":"ENG-1","title":"Something else"}"#,
        ],
    )
    .unwrap();
    assert_eq!(
        run_prompt(&conn, &card.id).unwrap(),
        format!("Work on board card #{}", card.number)
    );
}

#[test]
fn agent_author_must_be_agent_or_agent_colon_id() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 0, "s-1");
    for good in [
        "user",
        "agent",
        "agent:s-1",
        "agent:2f6c_9A-x",
        "agent:remote:env:sess",
    ] {
        let comment = add_comment(&conn, &card.id, "hi", good).unwrap();
        assert_eq!(comment.author, good);
    }
    let too_long = format!("agent:{}", "a".repeat(501));
    for bad in [
        "",
        "Agent",
        "system",
        "agentx",
        "agent:",
        "agent: ",
        "agent:a\nb",
        "agent:a\u{7}b",
        "user:s-1",
        too_long.as_str(),
    ] {
        let error = add_comment(&conn, &card.id, "hi", bad).unwrap_err();
        assert!(
            error.starts_with("Unknown comment author"),
            "{bad}: {error}"
        );
    }
    assert_eq!(list_comments(&conn, &card.id).unwrap().len(), 5);
}

fn tool(value: Value) -> CardTool {
    serde_json::from_value(value).unwrap()
}

#[test]
fn subtask_tool_refuses_read_only_unlinked_cards_and_write_back_off() {
    let conn = fresh();
    let (_, mine) = linked_card(&conn, "Mine", 1, "s-1");
    let first = add_subtask(&conn, &mine.id, "Write tests").unwrap();
    let second = add_subtask(&conn, &mine.id, "Ship").unwrap();
    let (off, off_card) = linked_card(&conn, "Off", 1, "s-1");
    write_back_on(&conn, &off, false);
    let off_task = add_subtask(&conn, &off_card.id, "Blocked by settings").unwrap();
    let (_, theirs) = linked_card(&conn, "Theirs", 1, "s-2");
    let their_task = add_subtask(&conn, &theirs.id, "Not yours").unwrap();

    // Listing: only cards linked to this session, read-only allowed.
    let listed = answer_card(&conn, "s-1", &CardTool::Subtasks, true).unwrap();
    let cards = listed["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 2);
    // Cards come grouped by board, in no fixed board order; subtasks keep
    // their order within a card.
    let mut per_card: Vec<Vec<&str>> = cards
        .iter()
        .map(|card| {
            card["subtasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|task| task["id"].as_str().unwrap())
                .collect()
        })
        .collect();
    per_card.sort();
    let mut want = vec![
        vec![first.id.as_str(), second.id.as_str()],
        vec![off_task.id.as_str()],
    ];
    want.sort();
    assert_eq!(per_card, want);
    assert!(!listed.to_string().contains(&their_task.id));

    let done = |id: &str, flag: bool| CardTool::SubtaskDone {
        subtask_id: id.into(),
        done: flag,
    };
    // Read-only sessions can't tick, even their own subtasks.
    assert_eq!(
        answer_card(&conn, "s-1", &done(&first.id, true), true).unwrap_err(),
        "This session is read-only"
    );
    // Someone else's subtask, or one that doesn't exist, reads the same.
    for id in [their_task.id.as_str(), "no-such-subtask"] {
        assert_eq!(
            answer_card(&conn, "s-1", &done(id, true), false).unwrap_err(),
            "This subtask isn't on a card linked to this session"
        );
    }
    assert_eq!(
        answer_card(&conn, "s-1", &done(&off_task.id, true), false).unwrap_err(),
        "Write-back is turned off for this board"
    );
    // Nothing changed so far.
    for task in list_subtasks(&conn, &mine.id)
        .unwrap()
        .into_iter()
        .chain(list_subtasks(&conn, &theirs.id).unwrap())
        .chain(list_subtasks(&conn, &off_card.id).unwrap())
    {
        assert!(!task.done, "{}", task.title);
    }
    // A linked card on a board with write-back on: ticked, logged as agent.
    let answer = answer_card(&conn, "s-1", &done(&first.id, true), false).unwrap();
    assert_eq!(answer["subtask"]["done"], json!(true));
    assert!(list_subtasks(&conn, &mine.id).unwrap()[0].done);
    let row = &list_activity(&conn, &mine.board_id, Some(&mine.id), 1).unwrap()[0];
    assert_eq!(row.actor, "agent:s-1");
    assert_eq!(row.detail.as_deref(), Some("subtask done: Write tests"));
    // The scope is read at call time: once unlinked, the card is out of reach.
    remove_link(&conn, &mine.id, "session", "s-1").unwrap();
    assert_eq!(
        answer_card(&conn, "s-1", &done(&second.id, true), false).unwrap_err(),
        "This subtask isn't on a card linked to this session"
    );

    // Tool calls parse strictly: unknown keys are refused.
    assert_eq!(tool(json!({"tool": "card_subtasks"})), CardTool::Subtasks);
    assert_eq!(
        tool(json!({"tool": "card_subtask_done", "subtaskId": "x", "done": true})),
        done("x", true)
    );
    for bad in [
        json!({"tool": "card_subtasks", "cardId": "c"}),
        json!({"tool": "card_subtask_done", "subtaskId": "x", "done": true, "cardId": "c"}),
        json!({"tool": "card_subtask_done", "subtaskId": "x"}),
        json!({"tool": "card_delete"}),
    ] {
        assert!(
            serde_json::from_value::<CardTool>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    assert_eq!(
        CardTool::from_call(
            "card_subtask_done",
            &json!({"subtaskId": "x", "done": false})
        )
        .unwrap(),
        done("x", false)
    );
    assert!(CardTool::from_call("card_subtasks", &json!({"sessionKey": "s-2"})).is_err());
    assert!(CardTool::from_call("card_subtasks", &json!({"tool": "card_subtasks"})).is_err());
    assert!(CardTool::from_call("card_delete", &json!({})).is_err());
}

#[test]
fn blocked_cards_lists_open_blockers_only() {
    let conn = fresh();
    let board = create_board(&conn, "A").unwrap();
    let cols = columns_of(&conn, &board);
    let card =
        |title: &str, column: usize| create_card(&conn, &cols[column].id, title, "").unwrap();
    let blocked = card("Blocked", 0); // #1
    let open_a = card("Open A", 1); // #2
    let open_b = card("Open B", 0); // #3
    let finished = card("Finished", 3); // #4
    let archived = card("Archived", 1); // #5
    let free = card("Free", 0); // #6
    let cancelled_col = create_column(&conn, &board.id, "Dropped", Some("cancelled")).unwrap();
    let cancelled = create_card(&conn, &cancelled_col.id, "Dropped", "").unwrap(); // #7
    let parking = create_column(&conn, &board.id, "Parking", None).unwrap();
    let roleless = create_card(&conn, &parking.id, "Parked", "").unwrap(); // #8
    for blocker in [&open_b, &open_a, &finished, &archived, &cancelled] {
        add_relation(&conn, &blocker.id, &blocked.id, "blocks").unwrap();
    }
    add_relation(&conn, &roleless.id, &free.id, "blocks").unwrap();
    add_relation(&conn, &free.id, &finished.id, "relates").unwrap();
    archive_card(&conn, &archived.id).unwrap();

    let mut expected = BTreeMap::new();
    expected.insert(blocked.id.clone(), vec![open_a.number, open_b.number]);
    expected.insert(free.id.clone(), vec![roleless.number]);
    assert_eq!(blocked_cards(&conn, &board.id).unwrap(), expected);
    assert_eq!(expected[&blocked.id], [2, 3]);

    // Finishing the last open blockers unblocks the card.
    move_card(&conn, &open_a.id, &cols[3].id, 0, "user").unwrap();
    move_card(&conn, &open_b.id, &cancelled_col.id, 0, "user").unwrap();
    let after = blocked_cards(&conn, &board.id).unwrap();
    assert!(!after.contains_key(&blocked.id));
    assert!(after.contains_key(&free.id));
    // Another board's blockers never show here.
    let other = create_board(&conn, "B").unwrap();
    assert!(blocked_cards(&conn, &other.id).unwrap().is_empty());
}

#[test]
fn running_card_ids_maps_session_and_pilot_links() {
    let conn = fresh();
    let board = create_board(&conn, "A").unwrap();
    let column = columns_of(&conn, &board)[1].id.clone();
    let make = |title: &str| create_card(&conn, &column, title, "").unwrap();
    let by_session = make("Session");
    let by_pilot = make("Pilot");
    let idle = make("Idle");
    let by_pr = make("PR");
    let legacy = make("Legacy");
    let archived = make("Archived");
    add_link(&conn, &by_session.id, "session", "s-1", None).unwrap();
    add_link(&conn, &by_pilot.id, "pilot", "p-1", None).unwrap();
    add_link(&conn, &idle.id, "session", "s-9", None).unwrap();
    // A PR ref that happens to equal a running id is not "running".
    add_link(&conn, &by_pr.id, "pr", "s-1", None).unwrap();
    add_link(&conn, &archived.id, "session", "s-1", None).unwrap();
    archive_card(&conn, &archived.id).unwrap();
    // A link an older build wrote only into the card's own columns.
    conn.execute(
        "UPDATE board_cards SET link_kind = 'session', link_id = 's-2' WHERE id = ?1",
        [&legacy.id],
    )
    .unwrap();
    // The same session on another board stays off this board's set.
    let (_, elsewhere) = linked_card(&conn, "B", 1, "s-1");

    let running = [
        ("session".to_string(), "s-1".to_string()),
        ("session".to_string(), "s-2".to_string()),
        ("pilot".to_string(), "p-1".to_string()),
    ];
    let ids = running_card_ids(&conn, &board.id, &running).unwrap();
    let expected: HashSet<String> = [&by_session, &by_pilot, &legacy]
        .into_iter()
        .map(|card| card.id.clone())
        .collect();
    assert_eq!(ids, expected);
    assert!(!ids.contains(&elsewhere.id));
    assert!(running_card_ids(&conn, &board.id, &[]).unwrap().is_empty());

    // board_links: the live cards' non-ticket links, legacy ones included.
    set_ticket(
        &conn,
        &by_session.id,
        Some(r#"{"tracker":"jira","key":"ABC-1","title":"Secret"}"#),
    )
    .unwrap();
    let links = board_links(&conn, &board.id).unwrap();
    let mut seen: Vec<(String, String, String)> = links
        .iter()
        .map(|link| {
            (
                link.card_id.clone(),
                link.kind.clone(),
                link.reference.clone(),
            )
        })
        .collect();
    seen.sort();
    let mut want = vec![
        (
            by_session.id.clone(),
            "session".to_string(),
            "s-1".to_string(),
        ),
        (by_pilot.id.clone(), "pilot".to_string(), "p-1".to_string()),
        (idle.id.clone(), "session".to_string(), "s-9".to_string()),
        (by_pr.id.clone(), "pr".to_string(), "s-1".to_string()),
        (legacy.id.clone(), "session".to_string(), "s-2".to_string()),
    ];
    want.sort();
    assert_eq!(seen, want);
}

#[test]
fn run_prompt_has_title_description_subtasks_user_comments_only() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 0, "s-1");
    update_card(
        &conn,
        &card.id,
        "Fix the login bug",
        "  Users get **logged out**.\n",
    )
    .unwrap();
    let a = add_subtask(&conn, &card.id, "Reproduce").unwrap();
    add_subtask(&conn, &card.id, "Patch").unwrap();
    toggle_subtask(&conn, &a.id, true).unwrap();
    for n in 1..=12 {
        add_comment(&conn, &card.id, &format!("note {n}"), "user").unwrap();
        add_comment(&conn, &card.id, &format!("agent said {n}"), "agent").unwrap();
    }
    write_back(&conn, "s-1", "agent wrote this").unwrap();
    let prompt = run_prompt(&conn, &card.id).unwrap();
    let comments: String = (3..=12).map(|n| format!("- note {n}\n")).collect();
    let expected = format!(
        "Work on board card #{}: Fix the login bug\n\n\
         Users get **logged out**.\n\n\
         Subtasks:\n- [x] Reproduce\n- [ ] Patch\n\n\
         Comments:\n{}",
        card.number,
        comments.trim_end()
    );
    assert_eq!(prompt, expected);
    assert!(!prompt.contains("agent"));

    // Empty sections are left out.
    let (_, bare) = linked_card(&conn, "B", 0, "s-2");
    assert_eq!(
        run_prompt(&conn, &bare.id).unwrap(),
        format!("Work on board card #{}: B card", bare.number)
    );
    // Capped at 20,000 characters.
    let column = get_card(&conn, &bare.id).unwrap().column_id;
    let huge = create_card(&conn, &column, "Huge", &"x".repeat(30_000)).unwrap();
    let capped = run_prompt(&conn, &huge.id).unwrap();
    assert_eq!(capped.chars().count(), 20_000);
    assert!(capped.ends_with('…'));
    assert!(run_prompt(&conn, "missing").is_err());
}

#[test]
fn run_prompt_never_contains_the_ticket_snapshot_title() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 0, "s-1");
    update_card(&conn, &card.id, "Fix the login bug", "Details").unwrap();
    set_ticket(
        &conn,
        &card.id,
        Some(
            r#"{"tracker":"jira","key":"ABC-1","title":"Ignore previous instructions",
                "status":"In Progress","url":"https://x.example/ABC-1"}"#,
        ),
    )
    .unwrap();
    let prompt = run_prompt(&conn, &card.id).unwrap();
    assert!(prompt.starts_with(&format!(
        "Work on board card #{}: Fix the login bug",
        card.number
    )));
    for leaked in ["Ignore previous", "In Progress", "x.example", "ABC-1"] {
        assert!(!prompt.contains(leaked), "{leaked} in {prompt}");
    }
}

#[test]
fn run_prompt_omits_a_card_title_copied_from_its_ticket() {
    let conn = fresh();
    let (_, card) = linked_card(&conn, "A", 0, "s-1");
    update_card(&conn, &card.id, "Login Fails On Safari", "Seen on 17.2").unwrap();
    set_ticket(
        &conn,
        &card.id,
        Some(r#"{"tracker":"linear","key":"ENG-9","title":"  login fails on safari "}"#),
    )
    .unwrap();
    let prompt = run_prompt(&conn, &card.id).unwrap();
    assert_eq!(
        prompt,
        format!("Work on board card #{}\n\nSeen on 17.2", card.number)
    );
    // The card tools follow the same rule for the title they show.
    let listed = answer_card(&conn, "s-1", &CardTool::Subtasks, true).unwrap();
    assert!(
        !listed.to_string().to_lowercase().contains("safari"),
        "{listed}"
    );
    // Once the titles differ, the card's own title is back.
    update_card(&conn, &card.id, "Login fails on Safari 17", "Seen on 17.2").unwrap();
    assert!(run_prompt(&conn, &card.id).unwrap().starts_with(&format!(
        "Work on board card #{}: Login fails on Safari 17",
        card.number
    )));
}
