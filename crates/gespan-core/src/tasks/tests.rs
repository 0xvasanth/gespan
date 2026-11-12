use super::*;
use crate::tool::ToolCall;
use crate::transcript::Row;

fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: Some(1_000),
    }
}

fn more(role: &str, text: &str) -> Row {
    Row {
        first: false,
        sent_at: None,
        ..row(role, text)
    }
}

fn bash(id: &str, command: &str) -> Row {
    let mut call = ToolCall::new(id, "Bash");
    call.input = serde_json::json!({ "command": command }).to_string();
    row("tool", &serde_json::to_string(&call).unwrap())
}

fn kinds(turn: &Turn) -> Vec<&'static str> {
    turn.entries
        .iter()
        .map(|e| match e {
            Entry::You { .. } => "you",
            Entry::Agent { .. } => "agent",
            Entry::Tools { .. } => "tools",
        })
        .collect()
}

#[test]
fn history_groups_rows_into_turns_from_each_user_message() {
    let rows = [
        row("user", "Start the parity run"),
        row("assistant", "Started."),
        row("user", "Fix MF2"),
        bash("t1", "cargo test"),
        bash("t2", "git status"),
        row("assistant", "MF2 needs your call."),
    ];
    let history = history(&rows, None, 4);
    assert_eq!(history.earlier, 0);
    assert_eq!(history.turns.len(), 2);
    assert_eq!(kinds(&history.turns[0]), ["you", "agent"]);
    assert_eq!(kinds(&history.turns[1]), ["you", "tools", "agent"]);
    let Entry::Tools { summaries, .. } = &history.turns[1].entries[1] else {
        panic!("expected tools");
    };
    assert_eq!(summaries.len(), 2);
}

#[test]
fn history_returns_the_last_n_turns_and_counts_the_rest() {
    let rows: Vec<Row> = (1..=6)
        .flat_map(|n| {
            [
                row("user", &format!("ask {n}")),
                row("assistant", &format!("reply {n}")),
            ]
        })
        .collect();
    let history = history(&rows, None, 4);
    assert_eq!(history.turns.len(), 4);
    assert_eq!(history.earlier, 2);
    let Entry::You { text, .. } = &history.turns[0].entries[0] else {
        panic!("expected a user entry");
    };
    assert_eq!(text, "ask 3");
}

#[test]
fn continuation_rows_join_their_message() {
    let rows = [
        row("user", "Hi"),
        row("assistant", "Hello "),
        more("assistant", "world"),
    ];
    let history = history(&rows, None, 4);
    let Entry::Agent { text, .. } = &history.turns[0].entries[1] else {
        panic!("expected an agent entry");
    };
    assert_eq!(text, "Hello world");
}

#[test]
fn rows_before_the_first_user_message_make_their_own_turn() {
    let rows = [bash("t1", "ls"), row("assistant", "Listed.")];
    let history = history(&rows, None, 4);
    assert_eq!(history.turns.len(), 1);
    assert_eq!(kinds(&history.turns[0]), ["tools", "agent"]);
}

#[test]
fn reasoning_hidden_and_blank_rows_are_skipped() {
    let rows = [
        row("pilot-system", "internal"),
        row("user", "Go"),
        row("reasoning", "thinking…"),
        row("assistant", "   "),
        row("pilot-card", "{\"type\":\"pr\"}"),
        row("assistant", "Done."),
    ];
    let history = history(&rows, None, 4);
    assert_eq!(history.turns.len(), 1);
    assert_eq!(kinds(&history.turns[0]), ["you", "agent"]);
}

#[test]
fn tool_rows_use_the_readable_presentation() {
    let rows = [
        row("user", "Go"),
        bash("t1", "cargo fmt --all"),
        row("tool", "{"),
    ];
    let history = history(&rows, None, 4);
    let Entry::Tools { summaries, .. } = &history.turns[0].entries[1] else {
        panic!("expected tools");
    };
    assert!(summaries[0].contains("cargo fmt --all"), "{summaries:?}");
    assert_eq!(summaries[1], "Tool call");
}

#[test]
fn tools_label_counts() {
    let one = Entry::Tools {
        summaries: vec!["a".into()],
        at: None,
    };
    let three = Entry::Tools {
        summaries: vec!["a".into(), "b".into(), "c".into()],
        at: None,
    };
    assert_eq!(one.tools_label().as_deref(), Some("1 tool call"));
    assert_eq!(three.tools_label().as_deref(), Some("3 tool calls"));
    let you = Entry::You {
        text: "x".into(),
        at: None,
    };
    assert_eq!(you.tools_label(), None);
}

fn choices(options: &[&str]) -> Row {
    row(
        "pilot-card",
        &serde_json::json!({ "type": "choices", "options": options }).to_string(),
    )
}

#[test]
fn a_choices_card_is_open_until_a_user_row_follows_it() {
    let mut rows = vec![row("assistant", "Which first?"), choices(&["A", "B"])];
    let open = open_choice(&rows).expect("open");
    assert_eq!(open.row, 1);
    assert_eq!(open.options, ["A", "B"]);
    rows.push(row("user", "A"));
    assert_eq!(open_choice(&rows), None);
}

#[test]
fn only_the_latest_choices_card_counts() {
    let rows = [
        choices(&["Old"]),
        row("assistant", "Again"),
        choices(&["New", "Other"]),
    ];
    let open = open_choice(&rows).expect("open");
    assert_eq!(open.row, 2);
    assert_eq!(open.options, ["New", "Other"]);
}

#[test]
fn other_cards_and_malformed_json_are_not_choices() {
    assert_eq!(open_choice(&[row("pilot-card", "{\"type\":\"pr\"}")]), None);
    assert_eq!(open_choice(&[row("pilot-card", "{")]), None);
    assert_eq!(open_choice(&[choices(&[])]), None);
}

fn visit(
    seq: u32,
    state: &str,
    status: crate::flow::VisitStatus,
    prompt: Option<&str>,
) -> crate::flow::FlowVisit {
    crate::flow::FlowVisit {
        id: format!("v{seq}"),
        run_id: "run".into(),
        seq,
        state_id: state.into(),
        status,
        prompt: prompt.map(Into::into),
        session_id: None,
        outputs: None,
        error: None,
        retried: false,
        note: None,
        started_at: 1_000 * i64::from(seq),
        finished_at: None,
    }
}

#[test]
fn flow_history_lists_finished_steps_then_the_prompt() {
    use crate::flow::VisitStatus::*;
    let visits = [
        visit(1, "changelog", Completed, None),
        visit(2, "ci", Completed, None),
        visit(3, "publish", WaitingApproval, Some("Publish 0.1.3?")),
    ];
    assert_eq!(waiting_visit(&visits).map(|v| v.seq), Some(3));
    let history = flow_history(&visits);
    assert_eq!(history.earlier, 0);
    assert_eq!(history.turns.len(), 1);
    let entries = &history.turns[0].entries;
    assert_eq!(entries.len(), 2);
    let Entry::Tools { summaries, .. } = &entries[0] else {
        panic!("expected steps");
    };
    assert_eq!(
        summaries,
        &["Step 1 · changelog · passed", "Step 2 · ci · passed"]
    );
    let Entry::Agent { text, .. } = &entries[1] else {
        panic!("expected the prompt");
    };
    assert_eq!(text, "Publish 0.1.3?");
}

#[test]
fn a_run_without_a_waiting_step_has_none() {
    use crate::flow::VisitStatus::*;
    assert!(waiting_visit(&[visit(1, "a", Running, None)]).is_none());
}

#[test]
fn filter_keeps_by_project_kind_wait_and_query() {
    let hour = 60 * 60 * 1000;
    let mut filter = Filter::default();
    assert!(filter.keeps("gespan", Kind::Question, 0, "anything"));
    filter.hidden_projects.insert("gespan".into());
    assert!(!filter.keeps("gespan", Kind::Question, 0, ""));
    assert!(filter.keeps("ae-staging", Kind::Question, 0, ""));
    filter = Filter::default();
    filter.hidden_kinds.insert(Kind::FlowApproval);
    assert!(!filter.keeps("gespan", Kind::FlowApproval, 0, ""));
    assert!(filter.keeps("gespan", Kind::Approval, 0, ""));
    filter = Filter {
        min_wait: MinWait::Hour,
        ..Filter::default()
    };
    assert!(!filter.keeps("gespan", Kind::Question, hour - 60_000, ""));
    assert!(filter.keeps("gespan", Kind::Question, hour, ""));
    filter.min_wait = MinWait::EightHours;
    assert!(!filter.keeps("gespan", Kind::Question, hour, ""));
    filter = Filter {
        query: "FSID".into(),
        ..Filter::default()
    };
    assert!(filter.keeps("gespan", Kind::Question, 0, "How is fsidCount kept?"));
    assert!(!filter.keeps("gespan", Kind::Question, 0, "Run a script?"));
}

#[test]
fn filter_is_active_only_when_something_is_hidden_or_a_wait_is_set() {
    let mut filter = Filter {
        query: "words".into(),
        ..Filter::default()
    };
    assert!(!filter.is_active());
    filter.min_wait = MinWait::Hour;
    assert!(filter.is_active());
    filter = Filter::default();
    filter.hidden_kinds.insert(Kind::PilotChoice);
    assert!(filter.is_active());
    assert_eq!(MinWait::EightHours.minutes(), 480);
    assert_eq!(Kind::PilotChoice.label(), "Pilot choices");
}

#[test]
fn order_puts_the_longest_wait_first() {
    let mut items = [
        ("new", 300u64),
        ("oldest", 100),
        ("middle", 200),
        ("tie", 100),
    ];
    order(&mut items, |item| item.1);
    let names: Vec<_> = items.iter().map(|i| i.0).collect();
    assert_eq!(names, ["oldest", "tie", "middle", "new"]);
}

#[test]
fn a_leading_turn_without_a_user_message_counts_as_earlier() {
    let rows = [
        bash("t1", "ls"),
        row("assistant", "Listed."),
        row("user", "ask 1"),
        row("assistant", "reply 1"),
        row("user", "   "),
        row("user", "ask 2"),
        row("assistant", "reply 2"),
    ];
    let one = history(&rows, None, 1);
    assert_eq!(one.earlier, 2);
    assert_eq!(kinds(&one.turns[0]), ["you", "agent"]);
    let Entry::You { text, .. } = &one.turns[0].entries[0] else {
        panic!("expected a user entry");
    };
    assert_eq!(text, "ask 2");
    let all = history(&rows, None, 10);
    assert_eq!(all.earlier, 0);
    assert_eq!(all.turns.len(), 3);
    assert_eq!(kinds(&all.turns[0]), ["tools", "agent"]);
}
