use super::*;
use crate::provider::{Kind, Request};
use rusqlite::Connection;
use std::path::Path;

/// A Cursor parent session store and, optionally, a child agent's store.
fn store(dir: &Path, folder: &[&str]) -> Connection {
    let path = folder
        .iter()
        .fold(dir.to_owned(), |path, part| path.join(part))
        .join("store.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);")
        .unwrap();
    connection
}
fn insert(connection: &Connection, id: &str, message: Value) {
    connection
        .execute(
            "INSERT INTO blobs VALUES (?1, ?2)",
            rusqlite::params![id, serde_json::to_vec(&message).unwrap()],
        )
        .unwrap();
}
fn child(dir: &Path, agent: &str, parent: &str, call: &str) -> Connection {
    let connection = store(dir, &["chats", "project", agent]);
    let meta = json!({"agentId": agent, "subagentInfo": {"parentAgentId": parent, "toolCallId": call, "typeName": "code_reviewer"}});
    let hex: String = meta
        .to_string()
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    connection
        .execute("INSERT INTO meta VALUES ('0', ?1)", [hex])
        .unwrap();
    connection
}
fn cursor(dir: &Path) -> Protocol {
    let mut protocol = Protocol::new(Request {
        kind: Kind::Cursor,
        binary: "unused".into(),
        cwd: "/tmp".into(),
        prompt: "Hello".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    });
    protocol.acp.session = "parent".into();
    protocol.acp.cursor.dir = Some(dir.to_owned());
    protocol
}
fn tool_event(events: &[Event], id: &str) -> ToolCall {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::Tool(call) if call.id == id => Some(call.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no event for {id}: {events:?}"))
}

#[test]
fn a_tool_without_arguments_gets_them_from_cursors_store() {
    let dir = tempfile::tempdir().unwrap();
    let parent = store(dir.path(), &["acp-sessions", "parent"]);
    let mut protocol = cursor(dir.path());
    protocol.acp_update(
        &json!({"sessionUpdate":"tool_call","toolCallId":"call-12345678","title":"Read File","kind":"read"}),
        &mut Vec::new(),
    );
    assert!(empty_input(&protocol.tools["call-12345678"].input));
    // Not stored yet: nothing changes, and it is asked about again.
    assert!(protocol.cursor_tick(false).is_empty());
    insert(
        &parent,
        "assistant",
        json!({"role":"assistant","content":[{"type":"tool-call","toolCallId":"call-12345678\nfc_87654321","toolName":"Read","args":{"path":"/repo/src/main.rs"}}]}),
    );
    protocol.acp.cursor.next = Some(Instant::now());
    let events = protocol.cursor_tick(false);
    let call = tool_event(&events, "call-12345678");
    assert_eq!(call.summary(), ("Read".into(), "/repo/src/main.rs".into()));
    assert!(protocol.acp.cursor.pending_args.is_empty());
    assert_eq!(protocol.acp.cursor.next, None, "nothing left to ask");
}

#[test]
fn arguments_already_sent_are_never_replaced_and_missing_ones_give_up() {
    let dir = tempfile::tempdir().unwrap();
    let _parent = store(dir.path(), &["acp-sessions", "parent"]);
    let mut protocol = cursor(dir.path());
    protocol.acp_update(
        &json!({"sessionUpdate":"tool_call","toolCallId":"sent-12345678","title":"Read","kind":"read","rawInput":{"path":"a.rs"}}),
        &mut Vec::new(),
    );
    assert!(protocol.acp.cursor.pending_args.is_empty());
    protocol.acp_update(
        &json!({"sessionUpdate":"tool_call","toolCallId":"lost-12345678","title":"Read","kind":"read"}),
        &mut Vec::new(),
    );
    for _ in 0..ARGS_ATTEMPTS {
        protocol.acp.cursor.next = Some(Instant::now());
        protocol.cursor_tick(false);
    }
    assert!(
        protocol.acp.cursor.pending_args.is_empty(),
        "it stops asking"
    );
}

#[test]
fn a_subagents_own_work_fills_its_agent_card_by_lineage() {
    let dir = tempfile::tempdir().unwrap();
    let mut protocol = cursor(dir.path());
    protocol.acp_update(
        &json!({"sessionUpdate":"tool_call","toolCallId":"spawn-12345678","title":"Subagent task","kind":"other","rawInput":{"subagent_type":"code_reviewer","prompt":"x"}}),
        &mut Vec::new(),
    );
    protocol
        .tools
        .get_mut("spawn-12345678")
        .unwrap()
        .display
        .as_mut()
        .unwrap()["kind"] = json!("agent");
    protocol.cursor_track("spawn-12345678");
    assert_eq!(protocol.acp.cursor.agents, ["spawn-12345678"]);
    let work = child(dir.path(), "child", "parent", "spawn-12345678");
    insert(
        &work,
        "user",
        json!({"role":"user","content":[{"type":"text","text":"<user_query>Perform a code review of the parser in /repo/src.</user_query>"}]}),
    );
    insert(
        &work,
        "a",
        json!({"role":"assistant","providerOptions":{"cursor":{"systemPromptFingerprint":{"model":"grok-4.6"}}},"content":[
            {"type":"text","text":"Reading the parser."},
            {"type":"tool-call","toolCallId":"read","toolName":"Read","args":{"path":"/repo/src/parse.rs"}},
            {"type":"tool-call","toolCallId":"test","toolName":"Shell","args":{"command":"cargo test"}}
        ]}),
    );
    insert(
        &work,
        "r",
        json!({"role":"tool","content":[
            {"type":"tool-result","toolCallId":"read","result":"fn parse() {}"},
            {"type":"tool-result","toolCallId":"test","isError":true,"result":"1 failed"}
        ]}),
    );
    // Another parent's child with the same call id is never mixed in.
    let other = child(dir.path(), "stranger", "other-parent", "spawn-12345678");
    insert(
        &other,
        "a",
        json!({"role":"assistant","content":[{"type":"text","text":"Not mine."}]}),
    );
    drop((work, other));

    let events = protocol.cursor_tick(false);
    let call = tool_event(&events, "spawn-12345678");
    let Some(Panel::Agent(run)) = call.panel() else {
        panic!("an agent card")
    };
    assert_eq!(run.name.as_deref(), Some("Review the parser"));
    assert_eq!(run.model.as_deref(), Some("grok-4.6"));
    assert_eq!(run.agent_type.as_deref(), Some("code_reviewer"));
    let steps: Vec<_> = run
        .steps
        .iter()
        .map(|step| (step.kind.as_str(), step.text.as_str(), step.state.clone()))
        .collect();
    assert_eq!(steps.len(), 3, "{steps:?}");
    assert_eq!(steps[0], ("message", "Reading the parser.", None));
    assert_eq!(steps[1].0, "tool");
    assert!(steps[1].1.contains("parse.rs"), "{steps:?}");
    assert_eq!(steps[1].2, Some(ToolState::Completed));
    assert_eq!(steps[2].2, Some(ToolState::Failed));
    assert_eq!(run.steps[2].detail, "1 failed");
    assert!(!steps.iter().any(|step| step.1.contains("Not mine")));

    // An unchanged child is not read or reported again.
    protocol.acp.cursor.next = Some(Instant::now());
    assert!(protocol.cursor_tick(false).is_empty());
}

#[test]
fn the_turn_end_reads_the_store_one_last_time() {
    let dir = tempfile::tempdir().unwrap();
    let parent = store(dir.path(), &["acp-sessions", "parent"]);
    let mut protocol = cursor(dir.path());
    protocol.acp_update(
        &json!({"sessionUpdate":"tool_call","toolCallId":"late-12345678","title":"Edit","kind":"edit"}),
        &mut Vec::new(),
    );
    insert(
        &parent,
        "a",
        json!({"role":"assistant","content":[{"type":"tool-call","toolCallId":"late-12345678","toolName":"Edit","args":{"path":"/repo/b.rs"}}]}),
    );
    // Not due yet, but the turn is over.
    protocol.acp.cursor.next = Some(Instant::now() + Duration::from_secs(60));
    assert!(protocol.cursor_tick(false).is_empty());
    let events = protocol.cursor_tick(true);
    assert_eq!(
        tool_event(&events, "late-12345678").summary().1,
        "/repo/b.rs"
    );
    assert_eq!(protocol.acp.cursor.next, None);
}

#[test]
fn other_providers_never_touch_cursors_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut protocol = cursor(dir.path());
    protocol.request.kind = Kind::Devin;
    protocol.acp_update(
        &json!({"sessionUpdate":"tool_call","toolCallId":"call-12345678","title":"Read","kind":"read"}),
        &mut Vec::new(),
    );
    assert!(protocol.acp.cursor.pending_args.is_empty());
}

#[test]
fn labels_follow_cursors_rules() {
    assert_eq!(
        agent_label("Agent: Fix the tests").as_deref(),
        Some("Fix the tests")
    );
    assert_eq!(agent_label("Subagent task"), None);
    assert_eq!(agent_label(" · unspecified"), None);
    assert_eq!(
        prompt_label("Conduct a thorough code review of ACP routing in C:\\repo.\nMore").as_deref(),
        Some("Review ACP routing")
    );
    assert_eq!(
        prompt_label("Find flaky tests.").as_deref(),
        Some("Find flaky tests")
    );
    assert_eq!(format_agent_type("code_reviewer"), "Code Reviewer");
    assert_eq!(format_agent_type("generalPurpose"), "GeneralPurpose");
    assert_eq!(kind_from_tool_name("StrReplace"), Some("edit"));
    assert_eq!(kind_from_tool_name("codebase_search"), Some("search"));
    assert_eq!(kind_from_tool_name("Unknown"), None);
}
