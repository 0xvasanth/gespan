//! The endpoint over real HTTP, as an agent reaches it.
use mcp_hub::{Config, Grant, Hub, Token, Toolkit};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

/// A toolkit that names its caller, and can hold a call open.
struct Fake {
    hold: Mutex<Option<mpsc::Sender<()>>>,
    name: &'static str,
}

impl Fake {
    fn new(name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            hold: Mutex::new(None),
            name,
        })
    }
}

impl Toolkit for Fake {
    fn name(&self) -> &'static str {
        self.name
    }

    fn tools(&self, _: &Grant) -> Vec<Value> {
        let name = format!("{}_who", self.name);
        vec![json!({"name":name,"description":"Who is calling","inputSchema":{"type":"object"}})]
    }

    fn call(&self, grant: &Grant, tool: &str, arguments: Value) -> Result<Value, String> {
        if let Some(entered) = self.hold.lock().unwrap().take() {
            entered.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        if arguments["fail"] == true {
            return Err("The work failed".into());
        }
        if let Some(reply) = arguments.get("reply") {
            return Ok(reply.clone());
        }
        Ok(json!({"tool":tool,"session":grant.session,"workspace":grant.workspace}))
    }
}

fn grant(session: i64, toolkits: &[&str]) -> Grant {
    Grant {
        session,
        workspace: format!("/work/{session}").into(),
        toolkits: toolkits.iter().map(|name| (*name).to_owned()).collect(),
        servers: BTreeSet::new(),
    }
}

fn hub() -> Hub {
    Hub::start(Config::new("test-hub", "1")).unwrap()
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

/// Plain HTTP, so a test can set or forge any header.
fn send(hub: &Hub, method: &str, extra: &[(&str, &str)], body: &str) -> Reply {
    let url = hub.url().strip_prefix("http://").unwrap();
    let (authority, path) = url.split_once('/').unwrap();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = format!("{method} /{path} HTTP/1.1\r\nConnection: close\r\n");
    if !extra.iter().any(|(name, _)| *name == "Host") {
        request += &format!("Host: {authority}\r\n");
    }
    for (name, value) in extra {
        request += &format!("{name}: {value}\r\n");
    }
    request += &format!(
        "Accept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    while let Ok(read) = reader.read_line(&mut response) {
        // An SSE reply stays open after its one message.
        if read == 0 || response.contains("\"jsonrpc\"") && response.ends_with("\n\n") {
            break;
        }
    }
    let (headers, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    Reply {
        status: headers.split_whitespace().nth(1).unwrap().parse().unwrap(),
        headers: headers.to_lowercase(),
        body: body.to_owned(),
    }
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

fn bearer(token: &Token) -> String {
    token.bearer()
}

fn message(reply: &Reply) -> Value {
    let line = reply
        .body
        .lines()
        .find_map(|line| {
            let line = line.strip_prefix("data:").unwrap_or(line).trim();
            line.starts_with('{').then_some(line)
        })
        .unwrap_or_else(|| panic!("no message in {:?}", reply.body));
    serde_json::from_str(line).unwrap()
}

/// An initialized client of the hub.
struct Client<'a> {
    hub: &'a Hub,
    bearer: String,
    session: String,
}

impl<'a> Client<'a> {
    fn connect(hub: &'a Hub, token: &Token) -> Self {
        let bearer = bearer(token);
        let reply = send(hub, "POST", &[("Authorization", &bearer)], INITIALIZE);
        assert_eq!(reply.status, 200, "{}", reply.body);
        let session = reply
            .headers
            .lines()
            .find_map(|line| line.strip_prefix("mcp-session-id: "))
            .unwrap()
            .trim()
            .to_owned();
        let client = Self {
            hub,
            bearer,
            session,
        };
        assert_eq!(
            client
                .post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .status,
            202
        );
        client
    }

    fn post(&self, body: &str) -> Reply {
        send(
            self.hub,
            "POST",
            &[
                ("Authorization", &self.bearer),
                ("Mcp-Session-Id", &self.session),
                ("MCP-Protocol-Version", "2025-11-25"),
            ],
            body,
        )
    }

    fn tools(&self) -> Vec<String> {
        let reply = self.post(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let listed = message(&reply);
        listed["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("{listed}"))
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    fn call(&self, tool: &str, arguments: Value) -> Value {
        let body = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":tool,"arguments":arguments}});
        message(&self.post(&body.to_string()))
    }
}

fn tool_output(reply: &Value) -> Value {
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn a_session_sees_and_calls_only_its_own_toolkits() {
    let hub = hub();
    hub.register(Fake::new("visual"));
    hub.register(Fake::new("ticket"));
    let token = hub.mint(grant(7, &["visual"]));
    let client = Client::connect(&hub, &token);
    assert_eq!(client.tools(), ["visual_who"]);
    let called = client.call("visual_who", json!({}));
    assert_eq!(
        tool_output(&called),
        json!({"tool":"visual_who","session":7,"workspace":"/work/7"})
    );
    let refused = client.call("ticket_who", json!({}));
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(
        refused["result"]["content"][0]["text"],
        "Tool ticket_who isn't available in this session"
    );
}

#[test]
fn the_token_decides_the_caller_whatever_the_request_claims() {
    let hub = hub();
    hub.register(Fake::new("visual"));
    let mine = hub.mint(grant(1, &["visual"]));
    let _theirs = hub.mint(grant(2, &["visual"]));
    let client = Client::connect(&hub, &mine);
    let called = client.call("visual_who", json!({"session":2,"workspace":"/work/2"}));
    assert_eq!(tool_output(&called)["session"], 1);
    assert_eq!(tool_output(&called)["workspace"], "/work/1");
}

#[test]
fn a_tool_error_is_a_result_and_not_a_failed_request() {
    let hub = hub();
    hub.register(Fake::new("visual"));
    let token = hub.mint(grant(1, &["visual"]));
    let client = Client::connect(&hub, &token);
    let reply = client.call("visual_who", json!({"fail":true}));
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(reply["result"]["content"][0]["text"], "The work failed");
}

#[test]
fn other_origins_and_hosts_get_403_even_with_a_good_token() {
    let hub = hub();
    let token = hub.mint(grant(1, &["visual"]));
    let auth = bearer(&token);
    for origin in [
        "https://evil.example",
        "http://127.0.0.1:1",
        "null",
        "http://localhost.evil.example",
    ] {
        let reply = send(
            &hub,
            "POST",
            &[("Authorization", &auth), ("Origin", origin)],
            INITIALIZE,
        );
        assert_eq!(reply.status, 403, "{origin}");
        assert_eq!(
            message(&reply)["error"]["message"],
            "Origin not allowed",
            "{origin}"
        );
    }
    // DNS rebinding: the browser connects here but names its own site.
    for host in [
        "evil.example",
        "evil.example:80",
        "127.0.0.1",
        "127.0.0.1:1",
    ] {
        let reply = send(
            &hub,
            "POST",
            &[("Authorization", &auth), ("Host", host)],
            INITIALIZE,
        );
        assert_eq!(reply.status, 403, "{host}");
        assert_eq!(message(&reply)["error"]["message"], "Host not allowed");
    }
    assert_eq!(
        send(&hub, "POST", &[("Authorization", &auth)], INITIALIZE).status,
        200
    );
}

#[test]
fn a_bad_token_gets_403_and_never_invites_an_oauth_sign_in() {
    let hub = hub();
    let _token = hub.mint(grant(1, &["visual"]));
    for auth in [None, Some("Bearer forged"), Some("Basic abc")] {
        let headers: Vec<_> = auth.iter().map(|auth| ("Authorization", *auth)).collect();
        let reply = send(&hub, "POST", &headers, INITIALIZE);
        assert_eq!(reply.status, 403, "{auth:?}");
        assert!(!reply.headers.contains("www-authenticate"), "{auth:?}");
        assert_eq!(
            message(&reply)["error"]["message"],
            "This tool credential is not valid"
        );
    }
}

/// A refusal sent before the request body arrives must not end in a reset:
/// on Windows a reset throws away the refusal the agent hasn't read yet, and
/// the agent sees an unreachable endpoint instead of the hub's reason.
#[test]
fn a_refusal_waits_for_the_body_so_the_connection_ends_cleanly() {
    let hub = hub();
    let url = hub.url().strip_prefix("http://").unwrap();
    let (authority, path) = url.split_once('/').unwrap();
    for _ in 0..20 {
        let mut stream = TcpStream::connect(authority).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let head = format!(
            "POST /{path} HTTP/1.1\r\nConnection: close\r\nHost: {authority}\r\nAuthorization: Bearer forged\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            INITIALIZE.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        // Long enough for a hub that answers from the headers alone to have
        // answered and stopped reading.
        std::thread::sleep(Duration::from_millis(50));
        stream.write_all(INITIALIZE.as_bytes()).unwrap();
        let mut response = Vec::new();
        let ended = stream.read_to_end(&mut response);
        let response = String::from_utf8_lossy(&response);
        assert!(ended.is_ok(), "{ended:?} after {response:?}");
        assert!(response.starts_with("HTTP/1.1 403"), "{response:?}");
        assert!(
            response.contains("This tool credential is not valid"),
            "{response:?}"
        );
    }
}

#[test]
fn a_revoked_session_gets_a_clear_error_at_once() {
    let hub = hub();
    hub.register(Fake::new("visual"));
    let token = hub.mint(grant(1, &["visual"]));
    let client = Client::connect(&hub, &token);
    assert_eq!(client.tools().len(), 1);
    hub.revoke(1);
    let reply = client.post(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert_eq!(reply.status, 403);
    assert_eq!(
        message(&reply)["error"]["message"],
        "This session has ended, so its tools are no longer available"
    );
}

#[test]
fn an_idle_token_expires_with_a_clear_error() {
    let mut config = Config::new("test-hub", "1");
    config.idle = Duration::from_millis(100);
    let hub = Hub::start(config).unwrap();
    let token = hub.mint(grant(1, &["visual"]));
    std::thread::sleep(Duration::from_millis(150));
    let reply = send(
        &hub,
        "POST",
        &[("Authorization", &bearer(&token))],
        INITIALIZE,
    );
    assert_eq!(reply.status, 403);
    assert_eq!(
        message(&reply)["error"]["message"],
        "This tool credential expired while idle; start a new turn"
    );
    assert!(!hub.has_grant(1));
}

/// Waits on the server-to-client stream for a changed tool list.
fn await_list_changed(hub: &Hub, client: &Client, wait: Duration, change: impl FnOnce()) -> bool {
    let authority = hub
        .url()
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream.set_read_timeout(Some(wait)).unwrap();
    write!(
        stream,
        "GET /mcp HTTP/1.1\r\nHost: {authority}\r\nAuthorization: {}\r\nMcp-Session-Id: {}\r\nMCP-Protocol-Version: 2025-11-25\r\nAccept: text/event-stream\r\n\r\n",
        client.bearer, client.session
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains(" 200 "), "{line}");
    change();
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        line.clear();
        if reader.read_line(&mut line).is_err() {
            return false;
        }
        if line.contains("notifications/tools/list_changed") {
            return true;
        }
    }
    false
}

#[test]
fn a_tool_added_mid_session_reaches_the_connected_agent() {
    let hub = hub();
    hub.register(Fake::new("visual"));
    hub.register(Fake::new("ticket"));
    let token = hub.mint(grant(1, &["visual"]));
    let client = Client::connect(&hub, &token);
    assert_eq!(client.tools(), ["visual_who"]);
    assert!(await_list_changed(
        &hub,
        &client,
        Duration::from_secs(5),
        || {
            assert!(hub.update(
                1,
                BTreeSet::from(["visual".into(), "ticket".into()]),
                BTreeSet::new()
            ));
        }
    ));
    assert_eq!(client.tools(), ["visual_who", "ticket_who"]);
}

#[test]
fn a_change_for_one_session_stays_with_that_session() {
    let hub = hub();
    let mine = hub.mint(grant(1, &[]));
    let theirs = hub.mint(grant(2, &[]));
    let client = Client::connect(&hub, &mine);
    let _other = Client::connect(&hub, &theirs);
    assert!(!await_list_changed(
        &hub,
        &client,
        Duration::from_millis(300),
        || hub.tools_changed(Some(2))
    ));
    assert!(await_list_changed(
        &hub,
        &client,
        Duration::from_secs(5),
        || hub.tools_changed(Some(1))
    ));
}

#[test]
fn stopping_the_hub_mid_call_fails_the_call_instead_of_hanging_it() {
    let hub = hub();
    let toolkit = Fake::new("visual");
    let (entered, reached) = mpsc::channel();
    *toolkit.hold.lock().unwrap() = Some(entered);
    hub.register(toolkit);
    let token = hub.mint(grant(1, &["visual"]));
    let client = Client::connect(&hub, &token);
    let authority = hub.url()["http://".len()..]
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let body = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"visual_who","arguments":{}}}).to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {authority}\r\nAuthorization: {}\r\nMcp-Session-Id: {}\r\nMCP-Protocol-Version: 2025-11-25\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        client.bearer, client.session, body.len()
    );
    let mut stream = TcpStream::connect(&authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    reached.recv_timeout(Duration::from_secs(3)).unwrap();
    let started = Instant::now();
    drop(client);
    drop(hub);
    let mut rest = Vec::new();
    let ended = stream.read_to_end(&mut rest);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the caller waited {:?} ({ended:?})",
        started.elapsed()
    );
}

#[test]
fn the_host_names_the_server() {
    let hub = hub();
    let token = hub.mint(grant(1, &[]));
    let reply = send(
        &hub,
        "POST",
        &[("Authorization", &bearer(&token))],
        INITIALIZE,
    );
    assert_eq!(message(&reply)["result"]["serverInfo"]["name"], "test-hub");
}

/// What a client gets back when the toolkit answers with `reply`.
fn answer(reply: Value) -> Value {
    let hub = hub();
    hub.register(Fake::new("visual"));
    let token = hub.mint(grant(1, &["visual"]));
    let client = Client::connect(&hub, &token);
    client.call("visual_who", json!({"reply":reply}))["result"].clone()
}

#[test]
fn a_toolkit_result_with_content_passes_through() {
    let result = answer(json!({"content":[
        {"type":"text","text":"hi"},
        {"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}
    ],"isError":false}));
    let blocks = result["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["text"], "hi");
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[1]["data"], "iVBORw0KGgo=");
    assert_eq!(blocks[1]["mimeType"], "image/png");
    assert_ne!(result["isError"], true);
}

#[test]
fn a_toolkit_result_can_flag_an_error() {
    let result = answer(json!({"content":[{"type":"text","text":"no"}],"isError":true}));
    assert_eq!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "no");
}

#[test]
fn plain_values_are_still_wrapped_as_text() {
    let result = answer(json!({"ok":1}));
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
    assert_eq!(result["content"][0]["text"], "{\"ok\":1}");
    assert_ne!(result["isError"], true);
}

#[test]
fn a_content_value_that_is_not_a_result_is_wrapped_as_text() {
    let value = json!({"content":[{"type":"bogus"}]});
    let result = answer(value.clone());
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
    assert_eq!(result["content"][0]["text"], value.to_string());
    assert_ne!(result["isError"], true);
}

#[test]
fn a_content_value_that_is_not_an_array_is_wrapped_as_text() {
    let value = json!({"content":"hi"});
    let result = answer(value.clone());
    assert_eq!(result["content"][0]["text"], value.to_string());
}

/// A toolkit that lists its tool only while open, and claims it either way.
struct Gated {
    open: std::sync::atomic::AtomicBool,
    claims: bool,
}

impl Toolkit for Gated {
    fn name(&self) -> &'static str {
        "gated"
    }

    fn tools(&self, _: &Grant) -> Vec<Value> {
        if self.open.load(std::sync::atomic::Ordering::SeqCst) {
            vec![json!({"name":"gated_go","description":"Go","inputSchema":{"type":"object"}})]
        } else {
            Vec::new()
        }
    }

    fn call(&self, _: &Grant, tool: &str, _: Value) -> Result<Value, String> {
        if self.open.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(json!({"ran":tool}))
        } else {
            Err("The gate is closed".into())
        }
    }

    fn routes(&self, _: &Grant, tool: &str) -> bool {
        self.claims && tool == "gated_go"
    }
}

fn gated(claims: bool) -> Arc<Gated> {
    Arc::new(Gated {
        open: std::sync::atomic::AtomicBool::new(true),
        claims,
    })
}

#[test]
fn a_toolkit_can_explain_a_tool_it_no_longer_lists() {
    let hub = hub();
    let kit = gated(true);
    hub.register(kit.clone());
    let token = hub.mint(grant(1, &["gated"]));
    let client = Client::connect(&hub, &token);
    assert_eq!(client.tools(), ["gated_go"]);
    assert_eq!(
        tool_output(&client.call("gated_go", json!({}))),
        json!({"ran":"gated_go"})
    );

    kit.open.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(client.tools().is_empty());
    let refused = client.call("gated_go", json!({}));
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(
        refused["result"]["content"][0]["text"],
        "The gate is closed"
    );
    // A tool nobody claims still gets the hub's own answer.
    let unknown = client.call("gated_other", json!({}));
    assert_eq!(
        unknown["result"]["content"][0]["text"],
        "Tool gated_other isn't available in this session"
    );
}

#[test]
fn unlisted_tools_stay_unreachable_without_a_claim_or_a_grant() {
    let first = hub();
    let quiet = gated(false);
    quiet.open.store(false, std::sync::atomic::Ordering::SeqCst);
    first.register(quiet);
    let client = Client::connect(&first, &first.mint(grant(1, &["gated"])));
    let refused = client.call("gated_go", json!({}));
    assert_eq!(
        refused["result"]["content"][0]["text"],
        "Tool gated_go isn't available in this session"
    );

    // A claiming toolkit the grant doesn't name is never asked.
    let other = hub();
    let claiming = gated(true);
    claiming
        .open
        .store(false, std::sync::atomic::Ordering::SeqCst);
    other.register(claiming);
    let client = Client::connect(&other, &other.mint(grant(2, &["visual"])));
    let refused = client.call("gated_go", json!({}));
    assert_eq!(
        refused["result"]["content"][0]["text"],
        "Tool gated_go isn't available in this session"
    );
}
