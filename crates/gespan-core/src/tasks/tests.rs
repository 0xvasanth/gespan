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
        .flat_map(|n| [row("user", &format!("ask {n}")), row("assistant", &format!("reply {n}"))])
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
    let rows = [row("user", "Go"), bash("t1", "cargo fmt --all"), row("tool", "{")];
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
