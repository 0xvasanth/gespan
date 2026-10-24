//! Claude's ~30 s `tool_progress` heartbeats are keep-alives, not work. These
//! replay captured streams through `Protocol::frame`, the same entry point the
//! provider loop uses for every Claude stdout line.
use super::*;
use crate::tool::tasks::{format_elapsed, is_heartbeat_frame, is_heartbeat_id};
use serde_json::json;

const AGENT_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/claude/heartbeat-agent.jsonl"
);
const BASH_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/claude/heartbeat-bash.jsonl"
);
const AGENT: &str = "toolu_019fPfTyVvoUGhPAWW4zfBqW";
const BASH: &str = "toolu_014zyZxk9DWdV8hmrxt56vP3";
const TICKS: [&str; 4] = [
    "toolu_01HjQzn3h7erFXipRYQSjxeL",
    "toolu_015g5TJkNKhLxuMPkQWQLWPK",
    "toolu_01B7AtVFQaSXgRJ34CohvfzU",
    "toolu_01UxyjEqbg3MqK2tHRnh2pik",
];

fn frames(path: &str) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("fixture")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture frame"))
        .collect()
}
fn claude() -> Protocol {
    Protocol::new(Request {
        kind: Kind::Claude,
        binary: "unused".into(),
        cwd: "/tmp".into(),
        prompt: "Hello".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    })
}
fn replay(p: &mut Protocol, frames: &[Value]) -> Vec<Event> {
    frames
        .iter()
        .flat_map(|frame| p.frame(frame.clone(), &mut Vec::new()).expect("frame"))
        .collect()
}
fn run(p: &Protocol, id: &str) -> AgentRun {
    match p.tools.get(id).and_then(ToolCall::panel) {
        Some(Panel::Agent(run)) => run,
        other => panic!("{id} is not an agent run: {other:?}"),
    }
}
fn heartbeat(parent: &str, n: u64) -> Value {
    json!({
        "type": "tool_progress",
        "tool_use_id": format!("{parent}-heartbeat-{n}"),
        "tool_name": "Bash",
        "parent_tool_use_id": parent,
        "elapsed_time_seconds": 30 * (n + 1),
        "heartbeat": true,
    })
}
/// 1-based index of the first fixture frame matching `pred`.
fn position(frames: &[Value], pred: impl Fn(&Value) -> bool) -> usize {
    frames.iter().position(pred).expect("frame") + 1
}

#[test]
fn heartbeat_agent_run_keeps_only_the_subagents_real_tool_calls() {
    let frames = frames(AGENT_FIXTURE);
    assert_eq!(frames.iter().filter(|f| is_heartbeat_frame(f)).count(), 4);
    let mut p = claude();
    replay(&mut p, &frames);
    let agent = run(&p, AGENT);
    let tools: Vec<_> = agent.steps.iter().filter(|s| s.kind == "tool").collect();
    assert_eq!(
        tools.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        TICKS
    );
    assert!(
        agent.steps.iter().all(|s| !s.id.contains("-heartbeat-")),
        "{:?}",
        agent.steps.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    assert!(agent
        .steps
        .iter()
        .all(|s| s.tool_kind.as_deref() != Some("Agent")));
    assert!(tools.iter().all(|s| s.state == Some(ToolState::Completed)));
    assert!(p.tools.keys().all(|id| !id.starts_with("background:")));
}

#[test]
fn heartbeat_agent_parent_shows_the_latest_elapsed_while_it_runs() {
    let frames = frames(AGENT_FIXTURE);
    let fourth = position(&frames, |f| {
        f["tool_use_id"] == format!("{AGENT}-heartbeat-3")
    });
    let mut p = claude();
    replay(&mut p, &frames[..fourth]);
    let parent = &p.tools[AGENT];
    assert_eq!(parent.state, ToolState::Running);
    assert_eq!(parent.live_elapsed_secs, Some(120));
}

#[test]
fn heartbeat_bash_creates_no_background_block_and_records_live_elapsed() {
    let frames = frames(BASH_FIXTURE);
    let second = position(&frames, |f| {
        f["tool_use_id"] == format!("{BASH}-heartbeat-1")
    });
    let mut p = claude();
    let events = replay(&mut p, &frames[..second]);
    assert_eq!(p.tools[BASH].state, ToolState::Running);
    assert_eq!(p.tools[BASH].live_elapsed_secs, Some(60));
    assert!(events
        .iter()
        .all(|e| !matches!(e, Event::Tool(t) if t.id.starts_with("background:"))));
    replay(&mut p, &frames[second..]);
    assert!(p.tools.keys().all(|id| !id.starts_with("background:")));
    assert_eq!(p.tools.len(), 1);
    assert_eq!(p.tools[BASH].state, ToolState::Completed);
    assert_eq!(p.tools[BASH].output, "done");
}

#[test]
fn heartbeat_elapsed_is_never_saved() {
    let frames = frames(BASH_FIXTURE);
    let second = position(&frames, |f| {
        f["tool_use_id"] == format!("{BASH}-heartbeat-1")
    });
    let mut p = claude();
    replay(&mut p, &frames[..second]);
    assert_eq!(p.tools[BASH].live_elapsed_secs, Some(60));
    let saved = serde_json::to_string(&p.tools[BASH]).unwrap();
    assert!(!saved.contains("elapsed"), "{saved}");
    let loaded: ToolCall = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.live_elapsed_secs, None);
}

#[test]
fn heartbeat_without_elapsed_field_derives_it_from_the_counter() {
    let frames = frames(BASH_FIXTURE);
    let started = position(&frames, |f| f["subtype"] == "task_started");
    let mut p = claude();
    replay(&mut p, &frames[..started]);
    let mut frame = heartbeat(BASH, 4);
    frame
        .as_object_mut()
        .unwrap()
        .remove("elapsed_time_seconds");
    replay(&mut p, &[frame]);
    assert_eq!(p.tools[BASH].live_elapsed_secs, Some(150));
}

#[test]
fn heartbeat_for_an_unknown_parent_creates_nothing() {
    let mut p = claude();
    let events = replay(&mut p, &[heartbeat("toolu_gone", 0)]);
    assert!(p.tools.is_empty());
    assert!(events.is_empty(), "{events:?}");
    // The flag alone marks it, whatever the id looks like.
    let flagged = json!({
        "type": "tool_progress", "tool_use_id": "odd-id", "tool_name": "Bash",
        "parent_tool_use_id": "toolu_gone", "heartbeat": true,
    });
    assert!(replay(&mut p, &[flagged]).is_empty());
    assert!(p.tools.is_empty());
}

#[test]
fn heartbeat_for_a_settled_parent_changes_nothing() {
    let frames = frames(BASH_FIXTURE);
    let mut p = claude();
    replay(&mut p, &frames);
    assert_eq!(p.tools[BASH].state, ToolState::Completed);
    let before = p.tools[BASH].clone();
    let events = replay(&mut p, &[heartbeat(BASH, 5)]);
    assert!(events.is_empty());
    assert_eq!(p.tools[BASH], before);
}

#[test]
fn child_step_mentioning_heartbeat_is_still_a_step() {
    let frames = frames(AGENT_FIXTURE);
    let started = position(&frames, |f| f["subtype"] == "task_started");
    let mut p = claude();
    replay(&mut p, &frames[..started]);
    let child = json!({
        "type": "assistant",
        "message": {
            "id": "msg_child", "role": "assistant", "model": "claude-haiku-5-5",
            "content": [{
                "type": "tool_use", "id": "toolu_child_note", "name": "Bash",
                "input": {"command": "echo heartbeat; grep -r heartbeat: ."},
            }],
        },
        "parent_tool_use_id": AGENT,
        "agent_id": "aef28cb7e5c5bc1d3",
    });
    replay(&mut p, &[child]);
    let agent = run(&p, AGENT);
    let step = agent
        .steps
        .iter()
        .find(|s| s.id == "toolu_child_note")
        .expect("child step");
    assert_eq!(step.kind, "tool");
}

#[test]
fn heartbeat_detection_matches_the_shared_definition() {
    // Flag alone, or id shape with a matching parent.
    assert!(is_heartbeat_frame(&heartbeat("toolu_a", 0)));
    assert!(is_heartbeat_frame(&json!({
        "type": "tool_progress", "tool_use_id": "p-heartbeat-12",
        "parent_tool_use_id": "p",
    })));
    assert!(is_heartbeat_frame(&json!({
        "type": "tool_progress", "tool_use_id": "x", "heartbeat": true,
    })));
    // Not heartbeats: wrong type, mismatched parent, no counter, `false` flag.
    assert!(!is_heartbeat_frame(&json!({
        "type": "assistant", "tool_use_id": "p-heartbeat-1",
        "parent_tool_use_id": "p", "heartbeat": true,
    })));
    assert!(!is_heartbeat_frame(&json!({
        "type": "tool_progress", "tool_use_id": "p-heartbeat-1",
        "parent_tool_use_id": "q",
    })));
    assert!(!is_heartbeat_frame(&json!({
        "type": "tool_progress", "tool_use_id": "p-heartbeat-",
        "parent_tool_use_id": "p",
    })));
    assert!(!is_heartbeat_frame(&json!({
        "type": "tool_progress", "tool_use_id": "child", "tool_name": "Bash",
        "parent_tool_use_id": "p", "heartbeat": false,
    })));
    assert!(is_heartbeat_id("toolu_1-heartbeat-0"));
    assert!(is_heartbeat_id("background:x-heartbeat-31"));
    assert!(!is_heartbeat_id("toolu_1-heartbeat-"));
    assert!(!is_heartbeat_id("toolu_1-heartbeat-x"));
    assert!(!is_heartbeat_id("-heartbeat-1"));
    assert!(!is_heartbeat_id("heartbeat-1"));
    assert!(!is_heartbeat_id("toolu_heartbeat-1-real"));
}

#[test]
fn heartbeat_elapsed_formats_as_seconds_then_minutes() {
    assert_eq!(format_elapsed(0), "0s");
    assert_eq!(format_elapsed(30), "30s");
    assert_eq!(format_elapsed(59), "59s");
    assert_eq!(format_elapsed(60), "1m 0s");
    assert_eq!(format_elapsed(150), "2m 30s");
    assert_eq!(format_elapsed(3725), "62m 5s");
}
