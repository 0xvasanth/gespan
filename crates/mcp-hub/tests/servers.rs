//! Real server processes behind a real endpoint. The fixture is this test
//! binary re-run as a line-delimited MCP server.
use mcp_hub::{Config, Grant, Hub, Observer, ServerSpec, Sharing, Spawned, Spawner, Token};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const FIXTURE: &str = "MCP_HUB_FIXTURE_SERVER";

/// Runs as the MCP server when the test binary is re-executed.
#[test]
fn fixture_server() {
    let Some(name) = std::env::var_os(FIXTURE) else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    let mut output = std::io::stdout();
    let mut send = |value: Value| {
        writeln!(output, "{value}").unwrap();
        output.flush().unwrap();
    };
    let mut version = 1;
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message["id"].clone();
        let result = match message["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion":"2025-11-25",
                "capabilities":{"tools":{"listChanged":true},"resources":{"subscribe":true}},
                "serverInfo":{"name":name,"version":"1"},
            }),
            "tools/list" => {
                let mut tools = vec![
                    json!({"name":"who","inputSchema":{"type":"object"}}),
                    // Junk a broken server might list.
                    json!(5),
                    json!({"name":"no-schema"}),
                ];
                tools.extend(
                    (1..version).map(
                        |n| json!({"name":format!("extra{n}"),"inputSchema":{"type":"object"}}),
                    ),
                );
                json!({"tools":tools})
            }
            "resources/list" => json!({"resources":[{"uri":"file:///r","name":"r"}]}),
            "resources/subscribe" | "resources/unsubscribe" => {
                send(
                    json!({"method":"notifications/message","params":{"level":"error","data":message["method"]}}),
                );
                json!({})
            }
            "tools/call" => {
                let arguments = &message["params"]["arguments"];
                if arguments["crash"] == true {
                    std::process::exit(9);
                }
                if arguments["junk"] == true {
                    send(json!({"jsonrpc":"2.0","id":id,"result":5}));
                    continue;
                }
                if arguments["change"] == true {
                    version += 1;
                    send(json!({"method":"notifications/tools/list_changed"}));
                }
                if arguments["sample"] == true {
                    send(
                        json!({"id":"ask","method":"sampling/createMessage","params":{"maxTokens":1,"messages":[]}}),
                    );
                }
                if let Some(token) = message["params"]["_meta"].get("progressToken") {
                    send(
                        json!({"method":"notifications/progress","params":{"progressToken":token,"progress":1}}),
                    );
                }
                if let Some(wait) = arguments["waitMs"].as_u64() {
                    std::thread::sleep(Duration::from_millis(wait));
                }
                let text = json!({
                    "server":name,
                    "pid":std::process::id(),
                    "cwd":std::env::current_dir().unwrap(),
                    "echo":arguments["echo"],
                });
                json!({"content":[{"type":"text","text":text.to_string()}],"isError":false})
            }
            "ping" => json!({}),
            _ if id.is_null() => continue,
            _ => {
                send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"no"}}));
                continue;
            }
        };
        if !id.is_null() {
            send(json!({"jsonrpc":"2.0","id":id,"result":result}));
        }
    }
    std::process::exit(0);
}

#[derive(Default)]
struct Processes {
    started: AtomicUsize,
    children: Mutex<Vec<Arc<Mutex<Child>>>>,
}

impl Processes {
    /// Processes of the fixture still running.
    fn alive(&self) -> usize {
        self.children
            .lock()
            .unwrap()
            .iter()
            .filter(|child| matches!(child.lock().unwrap().try_wait(), Ok(None)))
            .count()
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        for child in self.children.lock().unwrap().iter() {
            let _ = child.lock().unwrap().kill();
        }
    }
}

struct Spawn(Arc<Processes>);

impl Spawner for Spawn {
    fn spawn(&self, spec: &ServerSpec, workspace: Option<&Path>) -> std::io::Result<Spawned> {
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .envs(&spec.env)
            .env(FIXTURE, &spec.id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(workspace) = workspace {
            command.current_dir(workspace);
        }
        let mut child = command.spawn()?;
        let (stdin, stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        self.0.started.fetch_add(1, Ordering::SeqCst);
        self.0.children.lock().unwrap().push(child.clone());
        Ok(Spawned {
            pid,
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stop: Box::new(move || {
                let mut child = child.lock().unwrap();
                let _ = child.kill();
                let _ = child.wait();
            }),
        })
    }
}

fn server(id: &str, sharing: Sharing) -> ServerSpec {
    ServerSpec {
        id: id.into(),
        command: std::env::current_exe().unwrap().to_string_lossy().into(),
        args: vec![
            "--exact".into(),
            "fixture_server".into(),
            "--nocapture".into(),
        ],
        env: BTreeMap::new(),
        sharing,
        tools: None,
    }
}

#[derive(Default)]
struct Notes {
    demoted: Mutex<Vec<String>>,
    listed: AtomicUsize,
}

struct Noting(Arc<Notes>);

impl Observer for Noting {
    fn demoted(&self, server: &str, _: &str) {
        self.0.demoted.lock().unwrap().push(server.to_owned());
    }
    fn listed(&self, _: &str, _: &[Value]) {
        self.0.listed.fetch_add(1, Ordering::SeqCst);
    }
}

struct Rig {
    hub: Hub,
    processes: Arc<Processes>,
    notes: Arc<Notes>,
}

fn rig(servers: Vec<ServerSpec>, tune: impl FnOnce(&mut Config)) -> Rig {
    let processes = Arc::new(Processes::default());
    let notes = Arc::new(Notes::default());
    let mut config = Config::new("test-hub", "1");
    config.spawner = Some(Arc::new(Spawn(processes.clone())));
    config.observer = Arc::new(Noting(notes.clone()));
    tune(&mut config);
    let hub = Hub::start(config).unwrap();
    hub.set_servers(servers);
    Rig {
        hub,
        processes,
        notes,
    }
}

fn grant(session: i64, workspace: &Path, servers: &[&str]) -> Grant {
    Grant {
        session,
        workspace: workspace.to_owned(),
        toolkits: BTreeSet::new(),
        servers: servers.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// An MCP client over plain HTTP.
struct Agent {
    authority: String,
    bearer: String,
    session: String,
}

fn exchange(agent: &Agent, session: Option<&str>, body: &str) -> (String, String) {
    let extra = session.map_or_else(String::new, |session| {
        format!("Mcp-Session-Id: {session}\r\nMCP-Protocol-Version: 2025-11-25\r\n")
    });
    send(agent, &extra, body)
}

fn exchange_as(agent: &Agent, version: &str, body: &str) -> (String, String) {
    let method = serde_json::from_str::<Value>(body).unwrap()["method"].clone();
    let extra = format!(
        "MCP-Protocol-Version: {version}\r\nMcp-Method: {}\r\nMcp-Name: docs__who\r\n",
        method.as_str().unwrap()
    );
    send(agent, &extra, body)
}

fn send(agent: &Agent, extra: &str, body: &str) -> (String, String) {
    let mut stream = TcpStream::connect(&agent.authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAuthorization: {}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        agent.authority,
        agent.bearer,
        body.len()
    );
    request += extra;
    stream
        .write_all(format!("{request}\r\n{body}").as_bytes())
        .unwrap();
    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    while let Ok(read) = reader.read_line(&mut response) {
        if read == 0 || (response.contains("\"id\":") && response.ends_with("\n\n")) {
            break;
        }
    }
    let (headers, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    (headers.to_lowercase(), body.to_owned())
}

fn reply_of(body: &str) -> Value {
    body.lines()
        .rev()
        .filter_map(|line| {
            serde_json::from_str::<Value>(line.strip_prefix("data:").unwrap_or(line).trim()).ok()
        })
        .find(|message| message.get("id").is_some())
        .unwrap_or_else(|| panic!("no reply in {body:?}"))
}

impl Agent {
    fn connect(hub: &Hub, token: &Token) -> Self {
        let authority = hub.url()["http://".len()..]
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        let mut agent = Self {
            authority,
            bearer: token.bearer(),
            session: String::new(),
        };
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}});
        let (headers, _) = exchange(&agent, None, &init.to_string());
        agent.session = headers
            .lines()
            .find_map(|line| line.strip_prefix("mcp-session-id: "))
            .unwrap()
            .trim()
            .to_owned();
        let session = agent.session.clone();
        exchange(
            &agent,
            Some(&session),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        agent
    }

    fn request(&self, method: &str, params: Value) -> Value {
        let body = json!({"jsonrpc":"2.0","id":2,"method":method,"params":params});
        reply_of(&exchange(self, Some(&self.session), &body.to_string()).1)
    }

    fn tools(&self) -> Vec<String> {
        let reply = self.request("tools/list", json!({}));
        let mut names: Vec<_> = reply["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("{reply}"))
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        names
    }

    fn call(&self, tool: &str, arguments: Value) -> Value {
        let reply = self.request("tools/call", json!({"name":tool,"arguments":arguments}));
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{reply}"));
        serde_json::from_str(text).unwrap_or_else(|_| json!({"error":text}))
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// #186: 10 sessions across 10 worktrees using 3 shared servers need 3 server
/// processes and no process per session.
#[test]
fn ten_sessions_share_three_server_processes() {
    let folders: Vec<_> = (0..10).map(|_| tempfile::tempdir().unwrap()).collect();
    let rig = rig(
        ["docs", "search", "fetch"]
            .map(|id| server(id, Sharing::Shared))
            .to_vec(),
        |_| {},
    );
    let agents: Vec<_> = folders
        .iter()
        .enumerate()
        .map(|(n, folder)| {
            let token = rig.hub.mint(grant(
                n as i64 + 1,
                folder.path(),
                &["docs", "search", "fetch"],
            ));
            Agent::connect(&rig.hub, &token)
        })
        .collect();
    let mut pids = BTreeSet::new();
    std::thread::scope(|scope| {
        let calls: Vec<_> = agents
            .iter()
            .map(|agent| {
                scope.spawn(move || {
                    ["docs", "search", "fetch"].map(|id| {
                        agent.call(&format!("{id}__who"), json!({}))["pid"]
                            .as_u64()
                            .unwrap()
                    })
                })
            })
            .collect();
        for call in calls {
            pids.extend(call.join().unwrap());
        }
    });
    assert_eq!(pids.len(), 3, "{pids:?}");
    assert_eq!(rig.processes.started.load(Ordering::SeqCst), 3);
    assert_eq!(rig.processes.alive(), 3);
    let running = rig.hub.running();
    assert_eq!(running.len(), 3);
    assert!(running.iter().all(|process| process.sessions.len() == 10));
}

#[test]
fn a_workspace_server_runs_once_per_folder_in_that_folder() {
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let rig = rig(vec![server("files", Sharing::PerWorkspace)], |_| {});
    let a = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, first.path(), &["files"])));
    let b = Agent::connect(&rig.hub, &rig.hub.mint(grant(2, first.path(), &["files"])));
    let c = Agent::connect(&rig.hub, &rig.hub.mint(grant(3, second.path(), &["files"])));
    let (a, b, c) = (
        a.call("files__who", json!({})),
        b.call("files__who", json!({})),
        c.call("files__who", json!({})),
    );
    assert_eq!(a["pid"], b["pid"]);
    assert_ne!(a["pid"], c["pid"]);
    let folder = |reply: &Value| {
        PathBuf::from(reply["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    };
    assert_eq!(folder(&a), first.path().canonicalize().unwrap());
    assert_eq!(folder(&c), second.path().canonicalize().unwrap());
}

#[test]
fn a_per_session_server_runs_once_per_session_and_ends_with_it() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("browser", Sharing::PerSession)], |_| {});
    let a = Agent::connect(
        &rig.hub,
        &rig.hub.mint(grant(1, folder.path(), &["browser"])),
    );
    let b = Agent::connect(
        &rig.hub,
        &rig.hub.mint(grant(2, folder.path(), &["browser"])),
    );
    assert_ne!(
        a.call("browser__who", json!({}))["pid"],
        b.call("browser__who", json!({}))["pid"]
    );
    assert_eq!(rig.processes.alive(), 2);
    rig.hub.revoke(1);
    wait_until("the session's server to stop", || {
        rig.processes.alive() == 1
    });
}

#[test]
fn listing_is_served_from_the_cache_and_starts_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let mut docs = server("docs", Sharing::Shared);
    docs.tools = Some(vec![json!({"name":"who","inputSchema":{"type":"object"}})]);
    let rig = rig(vec![docs], |_| {});
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    assert_eq!(agent.tools(), ["docs__who"]);
    assert_eq!(rig.processes.started.load(Ordering::SeqCst), 0, "lazy");
    agent.call("docs__who", json!({}));
    assert_eq!(rig.processes.started.load(Ordering::SeqCst), 1);
}

#[test]
fn a_server_that_answers_junk_fails_that_call_and_nothing_else() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    assert_eq!(agent.tools(), ["docs__who"], "junk list items are left out");
    let failed = agent.request(
        "tools/call",
        json!({"name":"docs__who","arguments":{"junk":true}}),
    );
    assert!(!failed["error"].is_null(), "{failed}");
    assert_eq!(
        agent.call("docs__who", json!({"echo":"fine"}))["echo"],
        "fine"
    );
}

/// Kiro negotiates 2026-07-28, which requires this; old servers never send it.
#[test]
fn a_result_from_an_older_server_is_marked_complete_for_newer_agents() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    agent.tools();
    let body = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"docs__who",
        "arguments":{},
        "_meta":{
            "io.modelcontextprotocol/protocolVersion":"2026-07-28",
            "io.modelcontextprotocol/clientInfo":{"name":"t","version":"1"},
            "io.modelcontextprotocol/clientCapabilities":{},
        },
    }});
    let (_, reply) = exchange_as(&agent, "2026-07-28", &body.to_string());
    let reply = reply_of(&reply);
    assert_eq!(reply["result"]["resultType"], "complete", "{reply}");
}

#[test]
fn a_session_sees_only_the_servers_it_was_granted() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(
        vec![
            server("mine", Sharing::Shared),
            server("other", Sharing::Shared),
        ],
        |_| {},
    );
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["mine"])));
    assert_eq!(agent.tools(), ["mine__who"]);
    let refused = agent.request("tools/call", json!({"name":"other__who","arguments":{}}));
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert_eq!(
        rig.processes.started.load(Ordering::SeqCst),
        1,
        "only mine ran"
    );
}

#[test]
fn concurrent_calls_get_their_own_answers() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let agents: Vec<_> = (1..=4)
        .map(|n| Agent::connect(&rig.hub, &rig.hub.mint(grant(n, folder.path(), &["docs"]))))
        .collect();
    std::thread::scope(|scope| {
        for (n, agent) in agents.iter().enumerate() {
            scope.spawn(move || {
                for call in 0..5 {
                    let echo = format!("{n}-{call}");
                    // Every agent uses the same JSON-RPC id.
                    let reply = agent.call("docs__who", json!({"echo":echo}));
                    assert_eq!(reply["echo"], echo);
                }
            });
        }
    });
}

#[test]
fn an_idle_server_stops_and_comes_back_on_the_next_call() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |config| {
        config.server_idle = Duration::from_millis(150);
    });
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    assert_eq!(agent.tools(), ["docs__who"]);
    let first = agent.call("docs__who", json!({}))["pid"].clone();
    wait_until("the idle server to stop", || rig.processes.alive() == 0);
    assert_eq!(
        agent.tools(),
        ["docs__who"],
        "the cache outlives the process"
    );
    assert_eq!(rig.processes.alive(), 0);
    assert_ne!(agent.call("docs__who", json!({}))["pid"], first);
}

#[test]
fn a_busy_server_is_not_stopped_as_idle() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |config| {
        config.server_idle = Duration::from_millis(50);
    });
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    let reply = agent.call("docs__who", json!({"waitMs":400,"echo":"finished"}));
    assert_eq!(reply["echo"], "finished");
}

#[test]
fn the_cap_stops_the_longest_idle_server_first() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(
        ["a", "b", "c"]
            .map(|id| server(id, Sharing::Shared))
            .to_vec(),
        |config| config.max_servers = 2,
    );
    let agent = Agent::connect(
        &rig.hub,
        &rig.hub.mint(grant(1, folder.path(), &["a", "b", "c"])),
    );
    agent.call("a__who", json!({}));
    std::thread::sleep(Duration::from_millis(30));
    agent.call("b__who", json!({}));
    agent.call("c__who", json!({}));
    let running: Vec<_> = rig.hub.running().iter().map(|p| p.server.clone()).collect();
    assert_eq!(running, ["b", "c"]);
    wait_until("a to stop", || rig.processes.alive() == 2);
}

#[test]
fn a_server_that_crashes_fails_the_call_and_restarts_on_the_next() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    let first = agent.call("docs__who", json!({}))["pid"].clone();
    let failed = agent.request(
        "tools/call",
        json!({"name":"docs__who","arguments":{"crash":true}}),
    );
    assert_eq!(failed["result"]["isError"], true, "{failed}");
    assert_eq!(
        failed["result"]["content"][0]["text"],
        "The MCP server stopped"
    );
    wait_until("the crash to be noted", || rig.hub.running().is_empty());
    assert_ne!(agent.call("docs__who", json!({}))["pid"], first);
    assert!(rig.hub.errors().iter().any(|error| error.server == "docs"));
}

/// Reads the server-to-agent stream until `seen` or the timeout.
fn stream_has(agent: &Agent, seen: &str, wait: Duration, act: impl FnOnce()) -> bool {
    let mut stream = TcpStream::connect(&agent.authority).unwrap();
    stream.set_read_timeout(Some(wait)).unwrap();
    write!(
        stream,
        "GET /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nMcp-Session-Id: {}\r\nMCP-Protocol-Version: 2025-11-25\r\nAccept: text/event-stream\r\n\r\n",
        agent.authority, agent.bearer, agent.session
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains(" 200 "), "{line}");
    act();
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) if line.contains(seen) => return true,
            Ok(_) => {}
        }
    }
    false
}

#[test]
fn a_changed_tool_list_reaches_every_agent_using_that_server_after_the_cache_refreshes() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(
        vec![
            server("docs", Sharing::Shared),
            server("other", Sharing::Shared),
        ],
        |_| {},
    );
    let a = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    let b = Agent::connect(&rig.hub, &rig.hub.mint(grant(2, folder.path(), &["docs"])));
    let bystander = Agent::connect(&rig.hub, &rig.hub.mint(grant(3, folder.path(), &["other"])));
    assert_eq!(b.tools(), ["docs__who"]);
    bystander.tools();
    std::thread::scope(|scope| {
        let waiting_b =
            scope.spawn(|| stream_has(&b, "tools/list_changed", Duration::from_secs(5), || {}));
        let waiting_bystander = scope.spawn(|| {
            stream_has(
                &bystander,
                "tools/list_changed",
                Duration::from_millis(1500),
                || {},
            )
        });
        std::thread::sleep(Duration::from_millis(200));
        a.call("docs__who", json!({"change":true}));
        assert!(waiting_b.join().unwrap(), "the other user was told");
        assert!(!waiting_bystander.join().unwrap(), "a non-user was not");
    });
    assert_eq!(
        b.tools(),
        ["docs__extra1", "docs__who"],
        "the cache was refreshed"
    );
}

#[test]
fn a_shared_server_that_asks_for_sampling_is_refused_and_moves_to_one_per_session() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let a = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    let b = Agent::connect(&rig.hub, &rig.hub.mint(grant(2, folder.path(), &["docs"])));
    a.call("docs__who", json!({"sample":true}));
    wait_until("the server to be demoted", || {
        !rig.notes.demoted.lock().unwrap().is_empty()
    });
    assert_eq!(*rig.notes.demoted.lock().unwrap(), ["docs"]);
    assert_ne!(
        a.call("docs__who", json!({}))["pid"],
        b.call("docs__who", json!({}))["pid"]
    );
}

#[test]
fn subscriptions_reach_the_server_once_end_to_end() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let a = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    let b = Agent::connect(&rig.hub, &rig.hub.mint(grant(2, folder.path(), &["docs"])));
    let listed = a.request("resources/list", json!({}));
    let uri = listed["result"]["resources"][0]["uri"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(uri, "hub+docs:file:///r");
    assert!(a.request("resources/subscribe", json!({"uri":uri}))["error"].is_null());
    assert!(b.request("resources/subscribe", json!({"uri":uri}))["error"].is_null());
    let bad = a.request("resources/read", json!({"uri":"hub+nope:file:///r"}));
    assert!(!bad["error"].is_null());
    // A subscriber holds the process open past its idle time.
    assert!(rig.hub.running()[0].idle.is_none());
}

#[test]
fn changing_a_server_stops_its_old_process() {
    let folder = tempfile::tempdir().unwrap();
    let docs = server("docs", Sharing::Shared);
    let rig = rig(vec![docs.clone()], |_| {});
    let agent = Agent::connect(&rig.hub, &rig.hub.mint(grant(1, folder.path(), &["docs"])));
    agent.call("docs__who", json!({}));
    assert_eq!(rig.processes.alive(), 1);
    let mut changed = docs;
    changed.env.insert("TOKEN".into(), "new".into());
    rig.hub.set_servers(vec![changed]);
    wait_until("the old process to stop", || rig.processes.alive() == 0);
}

#[test]
fn stopping_the_hub_stops_every_server() {
    let folder = tempfile::tempdir().unwrap();
    let rig = rig(vec![server("docs", Sharing::Shared)], |_| {});
    let Rig { hub, processes, .. } = rig;
    let agent = Agent::connect(&hub, &hub.mint(grant(1, folder.path(), &["docs"])));
    agent.call("docs__who", json!({}));
    assert_eq!(processes.alive(), 1);
    drop(hub);
    assert_eq!(processes.alive(), 0);
}
