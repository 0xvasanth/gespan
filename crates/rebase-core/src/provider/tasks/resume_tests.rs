//! Replays the captured resumed-background-agent stream through `Protocol::frame`,
//! the same entry point the provider loop uses for every Claude stdout line.
use super::*;
use crate::tool::tasks::{desktop_agent, from_desktop};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/claude/resumed-background-agent.jsonl"
);
const AGENT: &str = "toolu_019x6aJjknXrg5CSy9dvntEn";
const SEND: &str = "toolu_01LfGgNGSbW1AVPVpfLsAYkh";
const AGENT_ID: &str = "aba491c492e906740";
/// Frames 1..=125 are turn 1; 126..=238 are turn 2 and Claude's self-started follow-up.
const TURN_ONE: usize = 125;
const RESUMED_TOOLS: [&str; 5] = [
    "echo resumed-1",
    "sleep 5",
    "echo resumed-2",
    "sleep 5",
    "echo resumed-3",
];

fn frames() -> Vec<Value> {
    std::fs::read_to_string(FIXTURE)
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
fn replay(p: &mut Protocol, frames: &[Value]) -> Vec<Vec<Event>> {
    frames
        .iter()
        .map(|frame| p.frame(frame.clone(), &mut Vec::new()).expect("frame"))
        .collect()
}
fn run(p: &Protocol, id: &str) -> AgentRun {
    match p.tools.get(id).and_then(ToolCall::panel) {
        Some(Panel::Agent(run)) => run,
        other => panic!("{id} is not an agent run: {other:?}"),
    }
}
fn tool_steps(run: &AgentRun) -> Vec<&AgentStep> {
    run.steps
        .iter()
        .filter(|step| step.kind == "tool")
        .collect()
}
fn command(step: &AgentStep) -> String {
    let input = step.display.as_ref().unwrap()["nativeInput"]
        .as_str()
        .unwrap()
        .to_owned();
    serde_json::from_str::<Value>(&input).unwrap()["command"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn summary(frames: &[Value], tool_use_id: &str) -> String {
    frames
        .iter()
        .find(|f| f["subtype"] == "task_notification" && f["tool_use_id"] == tool_use_id)
        .and_then(|f| f["summary"].as_str())
        .unwrap()
        .to_owned()
}
/// The frame index (0-based) of the 1-based fixture line.
fn line(n: usize) -> usize {
    n - 1
}
/// Turn 1's tool rows, persisted and loaded back the way the native store does.
fn persisted_turn_one() -> Vec<ToolCall> {
    let frames = frames();
    let mut p = claude();
    replay(&mut p, &frames[..TURN_ONE]);
    p.tools
        .values()
        .map(|tool| serde_json::to_string(tool).unwrap())
        .map(|row| serde_json::from_str::<ToolCall>(&row).unwrap())
        .collect()
}
fn assert_resumed_run(p: &Protocol, frames: &[Value]) {
    let resumed = run(p, SEND);
    assert_eq!(resumed.resume_of.as_deref(), Some(AGENT));
    assert_eq!(resumed.agent_id.as_deref(), Some(AGENT_ID));
    assert_eq!(
        resumed.display_name(),
        "Resumed · Run echo and sleep sequence"
    );
    let steps = tool_steps(&resumed);
    assert_eq!(
        steps.iter().map(|s| command(s)).collect::<Vec<_>>(),
        RESUMED_TOOLS
    );
    assert!(steps.iter().all(|s| s.state == Some(ToolState::Completed)));
    assert_eq!(steps[0].detail, "resumed-1");
    let final_text = frames[line(193)]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(resumed
        .steps
        .iter()
        .any(|s| s.kind == "message" && s.text == final_text));
    assert_eq!(p.tools[SEND].state, ToolState::Completed);
    assert_eq!(p.tools[SEND].output, summary(frames, SEND));
    assert_eq!(resumed.activity.as_deref(), Some("Running Print resumed-3"));
}

#[test]
fn turn_one_foreground_agent_keeps_its_child_steps() {
    let frames = frames();
    let expected: Vec<&str> = frames[..TURN_ONE]
        .iter()
        .filter(|f| f["type"] == "assistant" && f["parent_tool_use_id"] == AGENT)
        .flat_map(|f| f["message"]["content"].as_array().unwrap())
        .filter(|b| b["type"] == "tool_use")
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(expected.len(), 2);
    let mut p = claude();
    replay(&mut p, &frames[..TURN_ONE]);
    let agent = run(&p, AGENT);
    let steps = tool_steps(&agent);
    assert_eq!(
        steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        expected
    );
    assert!(steps.iter().all(|s| s.state == Some(ToolState::Completed)));
    // The hand-back trailer and the task events both name the agent.
    assert_eq!(agent.agent_id.as_deref(), Some(AGENT_ID));
    assert_eq!(agent.display_name(), "Run echo and sleep sequence");
    assert_eq!(p.tools[AGENT].state, ToolState::Completed);
}

#[test]
fn send_message_becomes_the_resumed_run_and_keeps_every_resumed_step() {
    let frames = frames();
    let mut p = claude();
    replay(&mut p, &frames);
    assert_resumed_run(&p, &frames);
    // History stays append-only: the original Agent block gains nothing.
    assert_eq!(tool_steps(&run(&p, AGENT)).len(), 2);
    assert!(p.tools[AGENT].output.contains("step-one"));
}

#[test]
fn resumed_run_status_summary_and_activity_come_from_task_events() {
    let frames = frames();
    let mut p = claude();
    // Through the last task_progress (line 191): still running, name intact.
    replay(&mut p, &frames[..line(192)]);
    let resumed = run(&p, SEND);
    assert_eq!(p.tools[SEND].state, ToolState::Running);
    assert_eq!(
        resumed.display_name(),
        "Resumed · Run echo and sleep sequence"
    );
    assert_eq!(resumed.activity.as_deref(), Some("Running Print resumed-3"));
    assert!(
        p.tools[SEND].output.is_empty(),
        "the launch receipt is not a report"
    );
    // Through the task_notification (line 195).
    replay(&mut p, &frames[line(192)..line(196)]);
    let resumed = run(&p, SEND);
    assert_eq!(p.tools[SEND].state, ToolState::Completed);
    assert_eq!(p.tools[SEND].output, summary(&frames, SEND));
    assert_eq!(
        resumed.display_name(),
        "Resumed · Run echo and sleep sequence"
    );
}

#[test]
fn background_card_points_at_the_resumed_block_and_counts_its_steps() {
    let frames = frames();
    let mut p = claude();
    replay(&mut p, &frames[..line(193)]);
    let task = p
        .activities
        .claude_tasks
        .iter()
        .find(|t| t.id == AGENT_ID)
        .expect("resumed agent is a running background task");
    assert_eq!(task.tool_id.as_deref(), Some(SEND));
    assert_eq!(task.latest.as_deref(), Some("Running Print resumed-3"));
    assert_eq!(task.steps, 5);
    assert!(task.agent);
    replay(&mut p, &frames[line(193)..line(196)]);
    assert!(!p.activities.claude_tasks.iter().any(|t| t.id == AGENT_ID));
}

#[test]
fn restart_seeds_the_registry_from_persisted_blocks_and_routes_turn_two() {
    let frames = frames();
    let blocks = persisted_turn_one();
    let mut p = claude();
    p.request.settings.agent_runs = blocks;
    p.seed_agent_runs();
    replay(&mut p, &frames[TURN_ONE..]);
    assert!(
        !p.tools.contains_key(AGENT),
        "turn 1's block is not re-emitted"
    );
    assert_resumed_run(&p, &frames);
}

#[test]
fn restart_card_is_published_with_the_resumed_block_id() {
    let frames = frames();
    let mut p = claude();
    p.request.settings.agent_runs = persisted_turn_one();
    p.seed_agent_runs();
    let mut published = Vec::new();
    for frame in &frames[TURN_ONE..line(193)] {
        p.frame(frame.clone(), &mut Vec::new()).unwrap();
        for event in p.lifecycle_tick(Instant::now(), &mut Vec::new()).unwrap() {
            if let Event::Background(tasks) = event {
                published.push(tasks);
            }
        }
    }
    let last = published
        .last()
        .expect("card published after turn 2 yielded");
    let task = last.iter().find(|t| t.id == AGENT_ID).unwrap();
    assert_eq!(task.tool_id.as_deref(), Some(SEND));
    assert_eq!(task.steps, 5);
    assert_eq!(task.latest.as_deref(), Some("Running Print resumed-3"));
}

#[test]
fn a_second_resume_gets_its_own_steps() {
    let frames = frames();
    // Duplicate the resume segment (SendMessage through its notification) as S2,
    // with fresh child ids, before Claude's self-started follow-up.
    let segment: Vec<Value> = frames[line(158)..line(196)]
        .iter()
        .map(|frame| {
            let mut text = frame.to_string().replace(SEND, "toolu_S2");
            for (n, child) in [
                "toolu_01HWq7foDHczTjtHuoNvz4M7",
                "toolu_013E6GSRiGhqgcQ3vbtT7XNg",
                "cUmhhfD6",
                "vCrXoa9C",
                "yozZ5QFk",
            ]
            .iter()
            .enumerate()
            {
                text = text.replace(child, &format!("S2_child{}", n + 1));
            }
            serde_json::from_str(&text).unwrap()
        })
        .collect();
    let mut all = frames[..line(196)].to_vec();
    all.extend(segment);
    all.extend_from_slice(&frames[line(196)..]);
    let mut p = claude();
    replay(&mut p, &all);
    let first = run(&p, SEND);
    let second = run(&p, "toolu_S2");
    assert_eq!(tool_steps(&first).len(), 5);
    assert_eq!(tool_steps(&second).len(), 5);
    assert_eq!(second.resume_of.as_deref(), Some(AGENT));
    assert!(tool_steps(&second)
        .iter()
        .any(|s| s.id.ends_with("S2_child1")));
    assert!(!tool_steps(&first).iter().any(|s| s.id.contains("S2_child")));
    assert_eq!(p.tools["toolu_S2"].state, ToolState::Completed);
}

#[test]
fn an_unknown_parent_becomes_a_visible_background_agent_step() {
    let mut p = claude();
    let events = p
        .frame(
            json!({"type":"assistant","parent_tool_use_id":"toolu_unknown","message":{"id":"m","content":[
                {"type":"tool_use","id":"child","name":"Bash","input":{"command":"ls"}}
            ]}}),
            &mut Vec::new(),
        )
        .unwrap();
    let Some(Event::Tool(block)) = events.last() else {
        panic!("the frame was swallowed: {events:?}")
    };
    let Some(Panel::Agent(fallback)) = block.panel() else {
        panic!()
    };
    assert_eq!(fallback.display_name(), "Background agent");
    assert_eq!(fallback.steps[0].id, "child");
    let block_id = block.id.clone();
    // Its result lands on the same step rather than being dropped.
    let events = p
        .frame(
            json!({"type":"user","parent_tool_use_id":"toolu_unknown","message":{"content":[
                {"type":"tool_result","tool_use_id":"child","content":"listing"}
            ]}}),
            &mut Vec::new(),
        )
        .unwrap();
    assert!(!events.is_empty());
    let fallback = run(&p, &block_id);
    assert_eq!(fallback.steps.len(), 1);
    assert_eq!(fallback.steps[0].detail, "listing");
    assert_eq!(fallback.steps[0].state, Some(ToolState::Completed));
}

#[test]
fn a_known_but_finished_dispatch_gets_one_continuation_block_per_turn() {
    let mut p = claude();
    p.request.settings.agent_runs = persisted_turn_one();
    p.seed_agent_runs();
    for id in ["late-1", "late-2"] {
        p.frame(
            json!({"type":"assistant","parent_tool_use_id":AGENT,"message":{"id":id,"content":[
                {"type":"tool_use","id":id,"name":"Bash","input":{"command":"pwd"}}
            ]}}),
            &mut Vec::new(),
        )
        .unwrap();
    }
    let blocks: Vec<&ToolCall> = p
        .tools
        .values()
        .filter(|t| t.id.starts_with("continued:"))
        .collect();
    assert_eq!(blocks.len(), 1);
    let Some(Panel::Agent(run)) = blocks[0].panel() else {
        panic!()
    };
    assert_eq!(
        run.display_name(),
        "Continued · Run echo and sleep sequence"
    );
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.resume_of.as_deref(), Some(AGENT));
    assert!(
        run.continuation,
        "the UI tells continuations apart by this flag"
    );
    // Nothing reports when stray work ends: the block must not stay Running
    // (the runtime would later settle it as failed).
    assert_eq!(blocks[0].state, ToolState::Completed);
    assert_eq!(run.steps[0].state, Some(ToolState::Running));
}

#[test]
fn self_started_turn_is_marked_with_the_notification_summary() {
    let frames = frames();
    let mut p = claude();
    let events = replay(&mut p, &frames);
    // The marker leads the follow-up: it is emitted with the turn's init frame.
    let marker = events[line(197)]
        .iter()
        .find_map(|event| match event {
            Event::Interjection {
                text, custom_type, ..
            } if custom_type == "task-notification" => Some(text.clone()),
            _ => None,
        })
        .expect("self-started turn marker");
    assert_eq!(marker, summary(&frames, SEND));
    // Only once, and never for the user's own turns.
    let markers = events
        .iter()
        .flatten()
        .filter(|e| matches!(e, Event::Interjection { custom_type, .. } if custom_type == "task-notification"))
        .count();
    assert_eq!(markers, 1);
}

#[test]
fn blocks_without_the_new_fields_still_load() {
    // A native row written before this change.
    let old = r#"{"id":"a","name":"Agent","input":"{}","output":"","state":"completed","panel":{"kind":"agent","ids":["t"],"name":"Review","model":null,"agent_type":null,"brief":"","steps":[]}}"#;
    let call: ToolCall = serde_json::from_str(old).unwrap();
    let Some(Panel::Agent(run)) = call.panel() else {
        panic!()
    };
    assert_eq!(
        (&run.agent_id, &run.resume_of, &run.activity),
        (&None, &None, &None)
    );
    // A desktop block written by either app before this change.
    let Some(Panel::Agent(run)) = from_desktop(&json!({
        "role":"tool","text":"Review","tool":{"kind":"agent"},
        "agentRun":{"name":"Review","nativeIds":["t"],"steps":[]}
    })) else {
        panic!()
    };
    assert_eq!(
        (&run.agent_id, &run.resume_of, &run.activity),
        (&None, &None, &None)
    );
    assert_eq!(run.display_name(), "Review");
    // Nothing new is written for a run without the fields.
    let value = desktop_agent(&run);
    for key in ["agentId", "resumeOf", "activity"] {
        assert!(value.get(key).is_none(), "{key} written as null");
    }
    let native = serde_json::to_value(Panel::Agent(run)).unwrap();
    for key in ["agentId", "resumeOf", "activity"] {
        assert!(native.get(key).is_none(), "{key} written as null");
    }
}

#[test]
fn new_fields_round_trip_with_camel_case_names() {
    let run = AgentRun {
        name: Some("Resumed · Review".into()),
        agent_id: Some("agent-x".into()),
        resume_of: Some("toolu_a".into()),
        activity: Some("Reading src".into()),
        ..AgentRun::default()
    };
    let desktop = desktop_agent(&run);
    assert_eq!(desktop["agentId"], "agent-x");
    assert_eq!(desktop["resumeOf"], "toolu_a");
    assert_eq!(desktop["activity"], "Reading src");
    let Some(Panel::Agent(back)) = from_desktop(&json!({"role":"tool","agentRun":desktop})) else {
        panic!()
    };
    assert_eq!(back.agent_id, run.agent_id);
    assert_eq!(back.resume_of, run.resume_of);
    assert_eq!(back.activity, run.activity);
    let native = serde_json::to_value(Panel::Agent(run.clone())).unwrap();
    assert_eq!(native["agentId"], "agent-x");
    assert_eq!(native["resumeOf"], "toolu_a");
    assert_eq!(native["activity"], "Reading src");
    assert_eq!(
        serde_json::from_value::<Panel>(native).unwrap(),
        Panel::Agent(run)
    );
}

#[test]
fn continuation_flag_round_trips_and_is_absent_when_false() {
    let run = AgentRun {
        continuation: true,
        ..AgentRun::default()
    };
    let desktop = desktop_agent(&run);
    assert_eq!(desktop["continuation"], true);
    let Some(Panel::Agent(back)) = from_desktop(&json!({"role":"tool","agentRun":desktop})) else {
        panic!()
    };
    assert!(back.continuation);
    let native = serde_json::to_value(Panel::Agent(run)).unwrap();
    assert_eq!(native["continuation"], true);
    let plain = serde_json::to_value(Panel::Agent(AgentRun::default())).unwrap();
    assert!(plain.get("continuation").is_none());
    assert!(desktop_agent(&AgentRun::default())
        .get("continuation")
        .is_none());
}

#[test]
fn the_title_comes_from_the_name_never_the_activity() {
    let mut run = AgentRun {
        name: Some("Correctness review".into()),
        ..AgentRun::default()
    };
    assert_eq!(run.display_name(), "Correctness review");
    run.activity = Some("Reading .".into());
    assert_eq!(run.display_name(), "Correctness review");
    let Some(Panel::Agent(loaded)) = from_desktop(&json!({
        "role":"tool","agentRun":{"name":"Correctness review","activity":"Reading ."}
    })) else {
        panic!()
    };
    assert_eq!(loaded.display_name(), "Correctness review");
}

fn late_child(p: &mut Protocol, parent: &str, id: &str) -> String {
    let events = p
        .frame(
            json!({"type":"assistant","parent_tool_use_id":parent,"message":{"id":id,"content":[
                {"type":"tool_use","id":id,"name":"Bash","input":{"command":"pwd"}}
            ]}}),
            &mut Vec::new(),
        )
        .unwrap();
    match events.last() {
        Some(Event::Tool(block)) => block.id.clone(),
        other => panic!("child frame not attached: {other:?}"),
    }
}
fn markers(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Interjection {
                text, custom_type, ..
            } if custom_type == "task-notification" => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn synthetic_block_ids_are_unique_across_turns_and_processes() {
    let init = json!({"type":"system","subtype":"init","session_id":"s"});
    // Two turns of one process, each with stray work for the same parent.
    let mut p = claude();
    p.request.settings.agent_runs = persisted_turn_one();
    p.seed_agent_runs();
    p.frame(init.clone(), &mut Vec::new()).unwrap();
    let first = late_child(&mut p, AGENT, "late-1");
    let first_unknown = late_child(&mut p, "toolu_unknown", "lost-1");
    p.frame(init.clone(), &mut Vec::new()).unwrap();
    let second = late_child(&mut p, AGENT, "late-2");
    let second_unknown = late_child(&mut p, "toolu_unknown", "lost-2");
    // A new process (or warm reuse) starts its own Protocol at turn 1 again.
    let mut q = claude();
    q.request.settings.agent_runs = persisted_turn_one();
    q.seed_agent_runs();
    q.frame(init, &mut Vec::new()).unwrap();
    let third = late_child(&mut q, AGENT, "late-3");
    let third_unknown = late_child(&mut q, "toolu_unknown", "lost-3");
    for id in [&first, &second, &third] {
        assert!(id.starts_with("continued:"), "{id}");
    }
    for id in [&first_unknown, &second_unknown, &third_unknown] {
        assert!(id.starts_with("background:"), "{id}");
    }
    let mut ids = vec![
        &first,
        &second,
        &third,
        &first_unknown,
        &second_unknown,
        &third_unknown,
    ];
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 6, "a synthetic block id repeated: {ids:?}");
    // Within one turn the same parent keeps one block.
    assert_eq!(late_child(&mut q, AGENT, "late-4"), third);
}

#[test]
fn a_user_follow_up_after_the_notification_is_not_marked_as_a_wake() {
    let frames = frames();
    let mut p = claude();
    p.request.settings.agent_runs = persisted_turn_one();
    p.seed_agent_runs();
    // Through the resumed agent's task_notification (line 195).
    replay(&mut p, &frames[TURN_ONE..line(196)]);
    assert!(p.claude_resume_at.is_some(), "inside the wake window");
    let sent = p
        .steer(
            crate::follow_up::Message {
                id: "follow-up".into(),
                text: "Thanks, what next?".into(),
                ..Default::default()
            },
            &mut Vec::new(),
        )
        .unwrap();
    assert!(matches!(sent.as_slice(), [Event::FollowUp(_)]), "{sent:?}");
    // The turn that answers the user starts, streams and finishes.
    let answer = replay(&mut p, &frames[line(196)..line(238)]);
    assert!(markers(&answer.concat()).is_empty());
    let user_result = p
        .frame(
            json!({"type":"result","subtype":"success","result":"Here is what is next."}),
            &mut Vec::new(),
        )
        .unwrap();
    assert!(markers(&user_result).is_empty());
    // Claude's own wake is still marked, by its result's origin.
    let wake = p.frame(frames[line(238)].clone(), &mut Vec::new()).unwrap();
    assert_eq!(markers(&wake), [summary(&frames, SEND)]);
}

#[test]
fn claudes_own_wake_after_a_restart_is_still_marked_when_it_starts() {
    let frames = frames();
    let mut p = claude();
    p.request.settings.agent_runs = persisted_turn_one();
    p.seed_agent_runs();
    let events = replay(&mut p, &frames[TURN_ONE..]);
    assert_eq!(
        markers(&events[line(197) - TURN_ONE]),
        [summary(&frames, SEND)]
    );
    assert_eq!(markers(&events.concat()).len(), 1);
}

#[test]
fn tauri_written_agent_rows_seed_the_registry() {
    let frames = frames();
    // The Tauri app's block: agentRun with nativeIds and no agentId.
    let block = json!({
        "id":"tauri-block","role":"tool","text":"Run echo and sleep sequence",
        "tool":{"callId":AGENT,"kind":"agent","title":"Run echo and sleep sequence","status":"completed"},
        "agentRun":{"name":"Run echo and sleep sequence","nativeIds":[AGENT_ID],"steps":[]}
    });
    let rows = crate::store::project_desktop_blocks(&[block]).unwrap();
    assert!(rows.iter().any(|row| row.role == "tool"));
    let runs = crate::tool::tasks::agent_runs(&rows);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, AGENT);
    let Some(Panel::Agent(seeded)) = runs[0].panel() else {
        panic!()
    };
    assert_eq!(seeded.agent_id, None);
    assert_eq!(seeded.ids, [AGENT_ID]);
    let mut p = claude();
    p.request.settings.agent_runs = runs;
    p.seed_agent_runs();
    replay(&mut p, &frames[TURN_ONE..]);
    assert_resumed_run(&p, &frames);
}
