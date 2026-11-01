//! The tool endpoint against the agent CLIs installed on this machine. Each
//! turn is a real, billed model call, so these run only when asked:
//!
//! `cargo test --test real_agents -- --ignored --nocapture --test-threads=1`
use gespan_core::{
    agent::{AccessMode, Settings},
    mcp::Importable,
    mcp_catalog::{Catalog, Class},
    provider::{self, Event, Kind, Request},
    tool_servers::{Mount, APP, VISUAL},
    visual_bridge,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

fn installed(binary: &str) -> PathBuf {
    let path = std::env::var_os("PATH").expect("PATH");
    std::env::split_paths(&path)
        .map(|folder| folder.join(binary))
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| panic!("{binary} is not installed"))
}

fn mount(session: i64, cwd: &Path, toolkits: &[&str]) -> Mount {
    Mount {
        session,
        worktree: cwd.to_owned(),
        toolkits: toolkits
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>(),
        ..Mount::default()
    }
}

#[derive(Default)]
struct Turn {
    text: String,
    tools: Vec<String>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    provider_id: Option<String>,
    process: Option<u32>,
}

fn turn(
    kind: Kind,
    binary: &Path,
    cwd: &Path,
    model: Option<&str>,
    provider_id: Option<String>,
    mount: Mount,
    prompt: &str,
) -> Turn {
    let _visual = visual_bridge::Server::start(mount.session).unwrap();
    gespan_core::tool_servers::refresh(&mount);
    let settings = Settings {
        access: AccessMode::FullAccess,
        mount: Some(mount.clone()),
        ..Default::default()
    };
    let request = Request {
        kind,
        binary: binary.to_owned(),
        cwd: cwd.to_owned(),
        prompt: prompt.to_owned(),
        provider_id,
        model: model.map(str::to_owned),
        settings,
    };
    let mut seen = Turn::default();
    let (_controls, receiver) = std::sync::mpsc::channel();
    gespan_core::process::owned_by(mount.session, || {
        provider::run_with_controls(
            request,
            Arc::new(AtomicBool::new(false)),
            Some(receiver),
            |events| {
                seen.process = gespan_core::process::agents()
                    .iter()
                    .find(|agent| agent.session == mount.session)
                    .map(|agent| agent.pid)
                    .or(seen.process);
                for event in events {
                    match event {
                        Event::Text(text) => seen.text.push_str(&text),
                        Event::Tool(tool) => {
                            seen.tools.push(tool.name.clone());
                            seen.inputs.push(tool.input.clone());
                            seen.outputs.push(tool.output.clone());
                        }
                        Event::SessionId(id) => seen.provider_id = Some(id),
                        _ => {}
                    }
                }
                Ok(())
            },
        )
    })
    .unwrap_or_else(|error| panic!("{kind:?} turn failed: {error}"));
    eprintln!(
        "{kind:?}: tools {:?}\n{kind:?}: said {}",
        seen.tools, seen.text
    );
    seen
}

const SERVER: &str = r#"
import json, os, sys
open(os.environ['MARKER'], 'a').write(os.path.basename(os.getcwd()) + '\n')
for line in sys.stdin:
    message = json.loads(line)
    if 'id' not in message:
        continue
    result = {}
    if message['method'] == 'initialize':
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'probe', 'version': '1'}}
    elif message['method'] == 'tools/list':
        result = {'tools': [{'name': 'secret_word', 'description': 'Returns the secret word.', 'inputSchema': {'type': 'object', 'properties': {}}}]}
    elif message['method'] == 'tools/call':
        result = {'content': [{'type': 'text', 'text': os.environ['WORD']}]}
    print(json.dumps({'jsonrpc': '2.0', 'id': message['id'], 'result': result}), flush=True)
"#;

/// The hub keeps the data folder it's first given, so these tests share one.
fn shared_data() -> &'static Path {
    static DATA: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DATA.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// A catalogue holding one approved server, and a folder whose own `.mcp.json`
/// names a server of its own. Each answers with a word of its own, so a reply
/// says which of them handled the call.
fn with_catalogue(folder: &Path) -> PathBuf {
    let script = folder.join("probe.py");
    std::fs::write(&script, SERVER).unwrap();
    let hub_marker = folder.join("hub-started");
    let own_marker = folder.join("own-started");
    let own = serde_json::json!({"mcpServers":{"own":{
        "command":"python3",
        "args":[script],
        "env":{"MARKER":own_marker,"WORD":"MANGO"},
    }}});
    std::fs::write(folder.join(".mcp.json"), own.to_string()).unwrap();
    let data = shared_data();
    let mut catalog = Catalog::default();
    catalog
        .import(
            data,
            vec![Importable {
                provider: "test".into(),
                scope: "user".into(),
                name: "probe".into(),
                command: "python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                env: [
                    (
                        "MARKER".to_owned(),
                        hub_marker.to_string_lossy().into_owned(),
                    ),
                    ("WORD".to_owned(), "PINEAPPLE".to_owned()),
                ]
                .into(),
                url: None,
            }],
        )
        .unwrap();
    catalog.entries[0].class = Class::Shared;
    catalog.approve(data, "probe").unwrap();
    catalog.managed = true;
    catalog.save(data).unwrap();
    gespan_core::tool_servers::configure(data);
    gespan_core::tool_servers::sync_servers();
    hub_marker
}

const PROXIED: &str =
    "Call the tool whose name contains probe__secret_word, then reply with only the word it returned.";

fn check_proxied(folder: &Path, marker: &Path, seen: &Turn) {
    assert!(
        seen.tools.iter().any(|name| name.contains("secret_word")),
        "{:?}",
        seen.tools
    );
    assert!(seen.text.contains("PINEAPPLE"), "{}", seen.text);
    assert_eq!(std::fs::read_to_string(marker).unwrap().lines().count(), 1);
    assert!(
        !folder.join("own-started").exists(),
        "the agent loaded its own copy"
    );
}

/// A real `app_call` reaching a stand-in for the app, as this session.
fn pilot_tools_reach_the_app(kind: Kind, binary: &str, session: i64, model: Option<&str>) {
    let folder = tempfile::tempdir().unwrap();
    let (dispatcher, calls) = gespan_core::pilot::bridge::Server::start().unwrap();
    let _route = dispatcher.environment(session);
    let answering = std::thread::spawn(move || {
        let call = calls.recv_blocking().unwrap();
        let seen = (call.session, call.action.clone());
        call.reply
            .send(Ok(serde_json::json!({"notes":[{"title":"MANGO"}]})))
            .unwrap();
        seen
    });
    let seen = turn(
        kind,
        &installed(binary),
        folder.path(),
        model,
        None,
        mount(session, folder.path(), &[APP]),
        "Call the app_call tool with action notes.list and input {}, then reply with only the title it returned.",
    );
    assert_eq!(
        answering.join().unwrap(),
        (session, "notes.list".to_owned())
    );
    assert!(seen.text.contains("MANGO"), "{}", seen.text);
}

#[test]
#[ignore = "a real, billed Claude turn"]
fn claude_reaches_the_pilot_tools() {
    pilot_tools_reach_the_app(Kind::Claude, "claude", 900_011, Some("claude-haiku-5-5"));
}

#[test]
#[ignore = "a real, billed Codex turn"]
fn codex_reaches_the_pilot_tools() {
    pilot_tools_reach_the_app(Kind::Codex, "codex", 900_012, None);
}

#[test]
#[ignore = "a real, billed Kiro turn"]
fn kiro_reaches_the_pilot_tools() {
    pilot_tools_reach_the_app(Kind::Kiro, "kiro-cli", 900_013, None);
}

const REACH: &str = "This is a test of your tools. First call the visual_docs tool. Then run this exact shell command: env | grep -c GESPAN_MCP || true. Then reply with only the word DONE and the number that command printed.";

fn check_reach(seen: &Turn) {
    assert!(
        seen.tools.iter().any(|name| name.contains("visual_docs")),
        "{:?}",
        seen.tools
    );
    assert!(seen.text.contains("DONE"), "{}", seen.text);
    let output = seen
        .inputs
        .iter()
        .zip(&seen.outputs)
        .rev()
        .find(|(input, output)| input.contains("grep -c GESPAN_MCP") && !output.is_empty())
        .map(|(_, output)| output.trim());
    assert_eq!(
        output,
        Some("0"),
        "the agent's own shell found the credential"
    );
}

#[test]
#[ignore = "a real, billed Claude turn"]
fn claude_reaches_the_tools_and_its_shell_cannot_find_the_token() {
    let folder = tempfile::tempdir().unwrap();
    let seen = turn(
        Kind::Claude,
        &installed("claude"),
        folder.path(),
        Some("claude-haiku-5-5"),
        None,
        mount(900_001, folder.path(), &[VISUAL]),
        REACH,
    );
    check_reach(&seen);
}

#[test]
#[ignore = "a real, billed Claude turn"]
fn a_tool_added_to_a_kept_alive_claude_arrives_without_a_restart() {
    let folder = tempfile::tempdir().unwrap();
    let binary = installed("claude");
    let session = 900_002;
    provider::configure_idle_agents(1);
    let first = turn(
        Kind::Claude,
        &binary,
        folder.path(),
        Some("claude-haiku-5-5"),
        None,
        mount(session, folder.path(), &[VISUAL]),
        "Reply with only the word READY, and the number of tools whose name contains app_help.",
    );
    let (_dispatcher, _calls) = gespan_core::pilot::bridge::Server::start().unwrap();
    let second = turn(
        Kind::Claude,
        &binary,
        folder.path(),
        Some("claude-haiku-5-5"),
        first.provider_id.clone(),
        mount(session, folder.path(), &[VISUAL, APP]),
        "A tool named app_help should be available to you now. Call it, then reply with only the word DONE.",
    );
    assert!(first.process.is_some());
    assert_eq!(first.process, second.process, "the process was restarted");
    assert!(
        second.tools.iter().any(|name| name.contains("app_help")),
        "{:?}: {}",
        second.tools,
        second.text
    );
}

#[test]
#[ignore = "a real, billed Codex turn"]
fn codex_reaches_the_tools_and_its_shell_cannot_find_the_token() {
    let folder = tempfile::tempdir().unwrap();
    let seen = turn(
        Kind::Codex,
        &installed("codex"),
        folder.path(),
        None,
        None,
        mount(900_003, folder.path(), &[VISUAL]),
        REACH,
    );
    check_reach(&seen);
}

#[test]
#[ignore = "a real, billed Codex turn"]
fn a_resumed_codex_thread_keeps_the_tools() {
    let folder = tempfile::tempdir().unwrap();
    let binary = installed("codex");
    let session = 900_004;
    let first = turn(
        Kind::Codex,
        &binary,
        folder.path(),
        None,
        None,
        mount(session, folder.path(), &[VISUAL]),
        "Reply with only the word READY.",
    );
    gespan_core::process::shutdown_all();
    let second = turn(
        Kind::Codex,
        &binary,
        folder.path(),
        None,
        first.provider_id,
        mount(session, folder.path(), &[VISUAL]),
        "Call the visual_docs tool, then reply with only the word DONE.",
    );
    assert!(
        second.tools.iter().any(|name| name.contains("visual_docs")),
        "{:?}: {}",
        second.tools,
        second.text
    );
}

#[test]
#[ignore = "a real, billed Claude turn"]
fn claude_uses_a_catalogue_server_and_loads_none_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let marker = with_catalogue(folder.path());
    let mut mount = mount(900_006, folder.path(), &[VISUAL]);
    mount.servers = ["probe".to_owned()].into();
    mount.exclusive = true;
    let seen = turn(
        Kind::Claude,
        &installed("claude"),
        folder.path(),
        Some("claude-haiku-5-5"),
        None,
        mount,
        PROXIED,
    );
    check_proxied(folder.path(), &marker, &seen);
}

#[test]
#[ignore = "a real, billed Claude turn"]
fn a_server_turned_on_in_the_catalogue_reaches_a_running_claude() {
    let folder = tempfile::tempdir().unwrap();
    let binary = installed("claude");
    let session = 900_010;
    provider::configure_idle_agents(1);
    with_catalogue(folder.path());
    let data = folder.path().join("data");
    let mut catalog = Catalog::load(&data).unwrap();
    catalog.entry_mut("probe").unwrap().enabled = false;
    catalog.save(&data).unwrap();
    gespan_core::tool_servers::sync_servers();
    let project = folder.path().to_string_lossy().into_owned();
    let mut first_mount = mount(session, folder.path(), &[VISUAL]);
    first_mount.project = project;
    first_mount.exclusive = true;
    let first = turn(
        Kind::Claude,
        &binary,
        folder.path(),
        Some("claude-haiku-5-5"),
        None,
        first_mount.clone(),
        "Reply with only the word READY.",
    );
    // The user turns it on while the agent sits idle, process alive.
    let mut catalog = Catalog::load(&data).unwrap();
    catalog.entry_mut("probe").unwrap().enabled = true;
    catalog.save(&data).unwrap();
    gespan_core::tool_servers::sync_servers();
    let mut second_mount = first_mount;
    second_mount.servers = ["probe".to_owned()].into();
    let second = turn(
        Kind::Claude,
        &binary,
        folder.path(),
        Some("claude-haiku-5-5"),
        first.provider_id.clone(),
        second_mount,
        PROXIED,
    );
    assert_eq!(first.process, second.process, "the process was restarted");
    assert!(
        second
            .tools
            .iter()
            .any(|name| name.contains("gespan__probe__secret_word")),
        "{:?}",
        second.tools
    );
    assert!(second.text.contains("PINEAPPLE"), "{}", second.text);
}

#[test]
#[ignore = "a real, billed Claude turn"]
fn claude_uses_a_public_remote_server_through_the_hub() {
    let folder = tempfile::tempdir().unwrap();
    let data = folder.path().join("data");
    let mut catalog = Catalog::default();
    catalog
        .import(
            &data,
            vec![Importable {
                provider: "test".into(),
                scope: "user".into(),
                name: "deepwiki".into(),
                command: String::new(),
                args: Vec::new(),
                env: Default::default(),
                url: Some("https://mcp.deepwiki.com/mcp".into()),
            }],
        )
        .unwrap();
    catalog.approve(&data, "deepwiki").unwrap();
    catalog.save(&data).unwrap();
    gespan_core::tool_servers::configure(&data);
    gespan_core::tool_servers::sync_servers();
    let mut mount = mount(900_009, folder.path(), &[VISUAL]);
    mount.servers = ["deepwiki".to_owned()].into();
    mount.exclusive = true;
    let seen = turn(
        Kind::Claude,
        &installed("claude"),
        folder.path(),
        Some("claude-haiku-5-5"),
        None,
        mount,
        "Use the deepwiki read_wiki_structure tool on the repository modelcontextprotocol/rust-sdk, then reply with only the word DONE.",
    );
    assert!(
        seen.tools
            .iter()
            .any(|name| name.contains("deepwiki__read_wiki_structure")),
        "{:?}: {}",
        seen.tools,
        seen.text
    );
    assert!(seen.text.contains("DONE"), "{}", seen.text);
}

#[test]
#[ignore = "a real, billed Codex turn"]
fn codex_uses_a_catalogue_server() {
    let folder = tempfile::tempdir().unwrap();
    let marker = with_catalogue(folder.path());
    let mut mount = mount(900_007, folder.path(), &[VISUAL]);
    mount.servers = ["probe".to_owned()].into();
    mount.exclusive = true;
    let seen = turn(
        Kind::Codex,
        &installed("codex"),
        folder.path(),
        None,
        None,
        mount,
        PROXIED,
    );
    check_proxied(folder.path(), &marker, &seen);
}

#[test]
#[ignore = "a real, billed Kiro turn"]
fn kiro_uses_a_catalogue_server() {
    let folder = tempfile::tempdir().unwrap();
    let marker = with_catalogue(folder.path());
    let mut mount = mount(900_008, folder.path(), &[VISUAL]);
    mount.servers = ["probe".to_owned()].into();
    let seen = turn(
        Kind::Kiro,
        &installed("kiro-cli"),
        folder.path(),
        None,
        None,
        mount,
        PROXIED,
    );
    assert!(seen.text.contains("PINEAPPLE"), "{}", seen.text);
    assert!(marker.exists());
}

#[test]
#[ignore = "a real, billed Kiro turn"]
fn kiro_reaches_the_tools_over_http_and_its_shell_cannot_find_the_token() {
    let folder = tempfile::tempdir().unwrap();
    let seen = turn(
        Kind::Kiro,
        &installed("kiro-cli"),
        folder.path(),
        None,
        None,
        mount(900_005, folder.path(), &[VISUAL]),
        REACH,
    );
    check_reach(&seen);
}
