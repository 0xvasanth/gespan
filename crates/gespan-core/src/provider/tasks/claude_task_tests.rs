//! Replays Claude Code 2.1 TaskCreate/TaskUpdate stream-json through
//! `Protocol::frame`, the entry point the provider loop uses for every line.
use super::*;
use crate::tool::tasks::{claude_task_lists, ClaudeTasks, Task, TaskStatus, CLAUDE_TASKS_KEY};
use crate::transcript::Row;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/claude/task-tools.jsonl"
);
const CONVERSATION: &str = "7f3c2a91-4b8e-4d2a-9c6f-1e5d8b0a3c47";
/// Fixture lines 1..=13 are turn 1; 14..=20 resume the same conversation.
const TURN_ONE: usize = 13;

fn frames() -> Vec<Value> {
    std::fs::read_to_string(FIXTURE)
        .expect("fixture")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture frame"))
        .collect()
}
fn claude(provider_id: Option<&str>, saved: Vec<ClaudeTasks>) -> Protocol {
    let mut p = Protocol::new(Request {
        kind: Kind::Claude,
        binary: "unused".into(),
        cwd: "unused".into(),
        prompt: "Hello".into(),
        provider_id: provider_id.map(str::to_owned),
        model: None,
        settings: Default::default(),
    });
    p.request.settings.claude_task_lists = saved;
    p.seed_claude_tasks();
    p
}
/// Every event each frame produced, in order.
fn replay(p: &mut Protocol, frames: &[Value]) -> Vec<Vec<Event>> {
    frames
        .iter()
        .map(|frame| p.frame(frame.clone(), &mut Vec::new()).expect("frame"))
        .collect()
}
fn cards(events: &[Event]) -> Vec<&ToolCall> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Tool(tool) if tool.id.starts_with(CLAUDE_TASKS_KEY) => Some(tool),
            _ => None,
        })
        .collect()
}
fn items(card: &ToolCall) -> Vec<(String, String, TaskStatus)> {
    let Some(Panel::Tasks(list)) = card.panel() else {
        panic!("{} is not a task card", card.id);
    };
    list.items
        .into_iter()
        .map(|task| (task.id.unwrap(), task.text, task.status))
        .collect()
}
fn item(id: &str, text: &str, status: TaskStatus) -> (String, String, TaskStatus) {
    (id.into(), text.into(), status)
}
/// The rows the runtime saves: each tool event rewrites its own row.
fn saved_rows(events: &[Vec<Event>]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for event in events.iter().flatten() {
        let Event::Tool(tool) = event else { continue };
        let text = serde_json::to_string(tool).unwrap();
        let existing = rows
            .iter_mut()
            .find(|row| serde_json::from_str::<ToolCall>(&row.text).unwrap().id == tool.id);
        match existing {
            Some(row) => row.text = text,
            None => rows.push(Row {
                role: "tool".into(),
                text,
                first: true,
                sent_at: None,
            }),
        }
    }
    rows
}

#[test]
fn task_create_and_update_build_one_task_card_per_turn() {
    let frames = frames();
    let mut p = claude(None, Vec::new());
    let events = replay(&mut p, &frames[..TURN_ONE]);
    let all: Vec<Event> = events.iter().flatten().cloned().collect();
    let cards = cards(&all);
    // Two creates and one status update changed the list; nothing else did.
    assert_eq!(cards.len(), 3);
    assert!(cards
        .iter()
        .all(|card| card.id == "claude-tasks:toolu_01Create1"));
    assert_eq!(
        items(cards[0]),
        [item("1", "Write parser tests", TaskStatus::Pending)]
    );
    assert_eq!(
        items(cards[2]),
        [
            item("1", "Write parser tests", TaskStatus::InProgress),
            item("2", "Ship the fix", TaskStatus::Pending),
        ]
    );
    assert_eq!(cards[2].state, ToolState::Completed);
    // The calls themselves stay ordinary tool rows, never subagents or lists.
    for id in ["toolu_01Create1", "toolu_01Update1", "toolu_01List"] {
        assert_eq!(p.tools[id].panel(), None, "{id}");
    }
}

#[test]
fn failed_unknown_and_read_only_task_results_leave_the_list_alone() {
    let frames = frames();
    let mut p = claude(None, Vec::new());
    replay(&mut p, &frames[..7]);
    let before = p.activities.claude_task_list.clone();
    assert_eq!(before.items.len(), 2);
    // Lines 8..=13: TaskUpdate of an unknown id (an error), a refused
    // TaskCreate, and a TaskList whose output only describes the list.
    let events = replay(&mut p, &frames[7..TURN_ONE]);
    assert!(events.iter().all(|frame| cards(frame).is_empty()));
    assert_eq!(p.activities.claude_task_list, before);
    assert_eq!(p.tools["toolu_01Missing"].state, ToolState::Failed);
    assert_eq!(p.tools["toolu_01Refused"].state, ToolState::Failed);
}

#[test]
fn a_resumed_turn_reseeds_from_saved_rows_and_applies_renames_and_deletes() {
    let frames = frames();
    let mut first = claude(None, Vec::new());
    let rows = saved_rows(&replay(&mut first, &frames[..TURN_ONE]));
    // A new Protocol, as after an app restart: only the saved rows remain.
    let mut p = claude(Some(CONVERSATION), claude_task_lists(&rows));
    let events = replay(&mut p, &frames[TURN_ONE..]);
    let all: Vec<Event> = events.iter().flatten().cloned().collect();
    let cards = cards(&all);
    assert_eq!(cards.len(), 3);
    // This turn shows its own card; the saved one stays as history.
    assert!(cards
        .iter()
        .all(|card| card.id == "claude-tasks:toolu_02Rename"));
    assert_eq!(
        items(cards[0]),
        [
            item("1", "Write stream-json parser tests", TaskStatus::Completed),
            item("2", "Ship the fix", TaskStatus::Pending),
        ]
    );
    assert_eq!(
        items(cards[2]),
        [
            item("1", "Write stream-json parser tests", TaskStatus::Completed),
            item("3", "Update the changelog", TaskStatus::Pending),
        ]
    );
    // The next turn reseeds from the newest card, not the first one.
    let rows = [rows, saved_rows(&events)].concat();
    let reseeded = claude(Some(CONVERSATION), claude_task_lists(&rows));
    assert_eq!(
        reseeded.activities.claude_task_list.items,
        p.activities.claude_task_list.items
    );
}

#[test]
fn only_the_resumed_conversations_saved_list_is_reused() {
    let frames = frames();
    let ours = ClaudeTasks {
        conversation: CONVERSATION.into(),
        items: vec![Task {
            id: Some("1".into()),
            text: "Ours".into(),
            status: TaskStatus::Pending,
        }],
    };
    // A later card from another conversation reuses the same task ids.
    let theirs = ClaudeTasks {
        conversation: "another-conversation".into(),
        items: vec![Task {
            id: Some("2".into()),
            text: "Theirs".into(),
            status: TaskStatus::Pending,
        }],
    };
    let saved = vec![ours.clone(), theirs.clone()];
    assert_eq!(
        claude(Some(CONVERSATION), saved.clone())
            .activities
            .claude_task_list,
        ours
    );
    // A new conversation starts empty, so its TaskUpdate of #1 finds nothing.
    let mut fresh = claude(None, saved);
    assert!(fresh.activities.claude_task_list.items.is_empty());
    let events = replay(&mut fresh, &frames[TURN_ONE..TURN_ONE + 3]);
    assert!(events.iter().all(|frame| cards(frame).is_empty()));
    // A conversation id change mid-session drops the old ids too.
    let mut switched = claude(Some(CONVERSATION), vec![theirs]);
    assert!(switched.activities.claude_task_list.items.is_empty());
    switched.activities.claude_task_list = ClaudeTasks {
        conversation: "another-conversation".into(),
        ..ours
    };
    let events = replay(&mut switched, &frames[TURN_ONE..TURN_ONE + 3]);
    assert!(events.iter().all(|frame| cards(frame).is_empty()));
    assert_eq!(
        switched.activities.claude_task_list.conversation,
        CONVERSATION
    );
}

#[test]
fn todo_write_still_shows_its_snapshot_without_a_claude_task_card() {
    let mut p = claude(None, Vec::new());
    let todos = json!({"todos":[
        {"content":"Read the parser","status":"completed","activeForm":"Reading the parser"},
        {"content":"Fix the bug","status":"in_progress","activeForm":"Fixing the bug"}
    ]});
    let mut events = Vec::new();
    for frame in [
        json!({"type":"assistant","message":{"id":"msg_todo","role":"assistant","model":"claude-opus-4-6","content":[{"type":"tool_use","id":"toolu_todo","name":"TodoWrite","input":todos}]},"parent_tool_use_id":null,"session_id":CONVERSATION}),
        json!({"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_todo","type":"tool_result","content":"Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress."}]},"parent_tool_use_id":null,"session_id":CONVERSATION,"tool_use_result":{"oldTodos":[],"newTodos":todos["todos"]}}),
    ] {
        events.extend(p.frame(frame, &mut Vec::new()).unwrap());
    }
    assert!(cards(&events).is_empty());
    assert_eq!(
        items_without_ids(&p.tools["toolu_todo"]),
        [
            ("Read the parser".to_owned(), TaskStatus::Completed),
            ("Fix the bug".to_owned(), TaskStatus::InProgress),
        ]
    );
    assert!(p.activities.claude_task_list.items.is_empty());
}
fn items_without_ids(call: &ToolCall) -> Vec<(String, TaskStatus)> {
    let Some(Panel::Tasks(list)) = call.panel() else {
        panic!("{} is not a task list", call.id);
    };
    list.items
        .into_iter()
        .map(|task| (task.text, task.status))
        .collect()
}

/// An inline subagent: Claude sends `task_started`, then only the parent's
/// tool result. No `task_notification` ever follows.
const INLINE_SUBAGENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/claude/inline-subagent.jsonl"
);
fn inline_subagent() -> Vec<Value> {
    std::fs::read_to_string(INLINE_SUBAGENT)
        .expect("fixture")
        .lines()
        .map(|line| serde_json::from_str(line).expect("fixture frame"))
        .collect()
}
fn finished(events: &[Vec<Event>]) -> bool {
    events.iter().flatten().any(|e| matches!(e, Event::Done))
}
/// One foreground call of `name` whose task `task_started` announces.
fn started_call(p: &mut Protocol, name: &str, task_type: &str, backgrounded: bool) {
    replay(
        p,
        &[
            json!({"type":"assistant","message":{"id":"m","content":[{
                "type":"tool_use","id":"call","name":name,"input":{"command":"sleep 1"}
            }]}}),
            json!({"type":"system","subtype":"task_started","task_id":"t","tool_use_id":"call",
                "description":"Work","task_type":task_type,"is_backgrounded":backgrounded}),
        ],
    );
}
fn tool_result(p: &mut Protocol, content: &str) {
    replay(
        p,
        &[json!({"type":"user","message":{"content":[{
            "type":"tool_result","tool_use_id":"call","content":content
        }]}})],
    );
}
fn result_frame() -> Value {
    json!({"type":"result","subtype":"success","result":"Done"})
}

#[test]
fn an_inline_subagent_that_reported_back_lets_the_turn_finish() {
    let mut p = claude(None, Vec::new());
    let events = replay(&mut p, &inline_subagent());
    assert!(finished(&events), "the turn must not wait for the subagent");
    assert!(p.activities.claude_pending.is_empty());
    assert!(!events
        .iter()
        .flatten()
        .any(|e| matches!(e, Event::Status(s) if s == "Waiting for background tasks…")));
    // Nothing is left to show as still running after the turn ended.
    assert!(p.activities.claude_tasks.is_empty());
    assert_eq!(p.tools["toolu_01Inline"].state, ToolState::Completed);
}

#[test]
fn an_inline_subagent_that_failed_lets_the_turn_finish() {
    let mut p = claude(None, Vec::new());
    started_call(&mut p, "Task", "local_agent", false);
    replay(
        &mut p,
        &[json!({"type":"user","message":{"content":[{
            "type":"tool_result","tool_use_id":"call","is_error":true,"content":"Rate limited"
        }]}})],
    );
    assert!(finished(&replay(&mut p, &[result_frame()])));
    assert_eq!(p.tools["call"].state, ToolState::Failed);
}

#[test]
fn a_backgrounded_subagent_still_holds_the_turn_past_its_launch_receipt() {
    let mut p = claude(None, Vec::new());
    started_call(&mut p, "Agent", "local_agent", true);
    tool_result(&mut p, "Backgrounded agent a1");
    let yielded = replay(&mut p, &[result_frame()]);
    assert!(!finished(&yielded), "the agent is still running");
    assert_eq!(p.activities.claude_tasks.len(), 1);

    replay(
        &mut p,
        &[
            json!({"type":"system","subtype":"task_notification","task_id":"t",
            "status":"completed","summary":"Finished"}),
        ],
    );
    assert!(p.activities.claude_pending.is_empty());
    assert!(p.activities.claude_tasks.is_empty());
}

#[test]
fn a_subagent_moved_to_the_background_mid_run_is_left_running() {
    let mut p = claude(None, Vec::new());
    started_call(&mut p, "Task", "local_agent", false);
    replay(
        &mut p,
        &[
            json!({"type":"system","subtype":"task_updated","task_id":"t",
            "patch":{"is_backgrounded":true}}),
        ],
    );
    tool_result(&mut p, "Backgrounded agent a1");
    assert!(!finished(&replay(&mut p, &[result_frame()])));
    assert_eq!(p.activities.claude_tasks.len(), 1);
}

#[test]
fn a_shell_task_is_not_settled_by_its_tool_result() {
    let mut p = claude(None, Vec::new());
    started_call(&mut p, "Bash", "local_bash", false);
    tool_result(&mut p, "Command running in background");
    assert!(!finished(&replay(&mut p, &[result_frame()])));
    assert!(p.activities.claude_pending.contains("t"));
}
