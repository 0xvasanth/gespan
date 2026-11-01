//! A remote server and its authorization server, on loopback, with the hub
//! signing in once and proxying for agents that never see a token.
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use mcp_hub::{
    call_tool, connect, sign_in, Config, Connection, Grant, Hub, ServerSpec, Sharing, Spawned,
    Spawner, TokenStore,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Seen {
    authorizations: Vec<String>,
    token_requests: usize,
    lifetime: Option<u64>,
    refreshed: bool,
    plain_token_endpoint: bool,
    stale_on_listen: bool,
    hold_listen: bool,
    listens: Vec<String>,
}

type Shared = Arc<Mutex<Seen>>;

async fn resource_metadata(headers: HeaderMap) -> Json<Value> {
    let host = headers["host"].to_str().unwrap();
    Json(
        json!({"resource":format!("http://{host}/mcp"),"authorization_servers":[format!("http://{host}")]}),
    )
}

async fn server_metadata(State(seen): State<Shared>, headers: HeaderMap) -> Json<Value> {
    let host = headers["host"].to_str().unwrap();
    let token = if seen.lock().unwrap().plain_token_endpoint {
        "http://auth.example/token".to_owned()
    } else {
        format!("http://{host}/token")
    };
    Json(json!({
        "issuer":format!("http://{host}"),
        "authorization_endpoint":format!("http://{host}/authorize"),
        "token_endpoint":token,
        "registration_endpoint":format!("http://{host}/register"),
        "response_types_supported":["code"],
        "code_challenge_methods_supported":["S256"],
        "token_endpoint_auth_methods_supported":["none"],
    }))
}

async fn register(Json(body): Json<Value>) -> impl IntoResponse {
    let mut client = body;
    client["client_id"] = json!("client-1");
    (StatusCode::CREATED, Json(client))
}

async fn authorize(Query(query): Query<HashMap<String, String>>) -> Redirect {
    Redirect::to(&format!(
        "{}?code=the-code&state={}",
        query["redirect_uri"], query["state"]
    ))
}

async fn token(State(seen): State<Shared>, body: String) -> Json<Value> {
    let mut seen = seen.lock().unwrap();
    seen.token_requests += 1;
    if body.contains("grant_type=refresh_token") {
        seen.refreshed = true;
        return Json(
            json!({"access_token":"refreshed-token","token_type":"Bearer","expires_in":3600}),
        );
    }
    let lifetime = seen.lifetime.unwrap_or(3600);
    Json(json!({
        "access_token":"remote-access-token",
        "refresh_token":"refresh-1",
        "token_type":"Bearer",
        "expires_in":lifetime,
    }))
}

async fn mcp(State(seen): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let authorization = headers
        .get("authorization")
        .map_or("", |value| value.to_str().unwrap())
        .to_owned();
    seen.lock()
        .unwrap()
        .authorizations
        .push(authorization.clone());
    if !["Bearer remote-access-token", "Bearer refreshed-token"].contains(&authorization.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let message: Value = serde_json::from_slice(&body).unwrap();
    if message.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let result = match message["method"].as_str().unwrap() {
        "initialize" => {
            json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"remote","version":"1"}})
        }
        "tools/list" => json!({"tools":[{"name":"ping","inputSchema":{"type":"object"}}]}),
        "tools/call" => json!({"content":[{"type":"text","text":"\"pong\""}]}),
        _ => json!({}),
    };
    Json(json!({"jsonrpc":"2.0","id":message["id"],"result":result})).into_response()
}

/// The server-to-client stream, which says nothing.
async fn events(State(seen): State<Shared>, headers: HeaderMap) -> Response {
    let authorization = headers
        .get("authorization")
        .map_or("", |value| value.to_str().unwrap())
        .to_owned();
    let (stale, hold) = {
        let mut seen = seen.lock().unwrap();
        seen.listens.push(authorization);
        (
            seen.stale_on_listen && seen.listens.len() == 1,
            seen.hold_listen,
        )
    };
    if stale {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if hold {
        tokio::time::sleep(Duration::from_secs(120)).await;
    }
    ([("content-type", "text/event-stream")], "").into_response()
}

/// Slow enough that a stopped connection ends after its replacement started.
async fn end_session() -> StatusCode {
    tokio::time::sleep(Duration::from_millis(150)).await;
    StatusCode::OK
}

/// Starts the remote and returns its URL.
fn remote(seen: Shared) -> String {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender.send(listener.local_addr().unwrap()).unwrap();
                let app = Router::new()
                    .route(
                        "/.well-known/oauth-protected-resource/mcp",
                        get(resource_metadata),
                    )
                    .route(
                        "/.well-known/oauth-protected-resource",
                        get(resource_metadata),
                    )
                    .route(
                        "/.well-known/oauth-authorization-server",
                        get(server_metadata),
                    )
                    .route("/register", post(register))
                    .route("/authorize", get(authorize))
                    .route("/token", post(token))
                    .route("/mcp", post(mcp).get(events).delete(end_session))
                    .with_state(seen);
                axum::serve(listener, app).await.unwrap();
            });
    });
    format!("http://{}/mcp", receiver.recv().unwrap())
}

#[derive(Default)]
struct Memory(Mutex<Option<String>>);

impl TokenStore for Memory {
    fn load(&self) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, value: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(value.to_owned());
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

struct Remote(Arc<Memory>);

impl Spawner for Remote {
    fn spawn(&self, spec: &ServerSpec, _: Option<&Path>) -> std::io::Result<Spawned> {
        connect(&spec.command, BTreeMap::new(), Some(self.0.clone()))
    }
}

/// Follows the redirects the way a browser would.
fn browser(url: &str) {
    let url = url.to_owned();
    std::thread::spawn(move || {
        ureq::get(&url).call().unwrap();
    });
}

fn hub_for(url: &str, store: Arc<Memory>) -> Hub {
    let mut config = Config::new("test-hub", "1");
    config.spawner = Some(Arc::new(Remote(store)));
    let hub = Hub::start(config).unwrap();
    hub.set_servers(vec![ServerSpec {
        id: "remote".into(),
        command: url.to_owned(),
        args: Vec::new(),
        env: BTreeMap::new(),
        sharing: Sharing::Shared,
        tools: None,
    }]);
    hub
}

fn grant(session: i64) -> Grant {
    Grant {
        session,
        workspace: "/work".into(),
        toolkits: BTreeSet::new(),
        servers: BTreeSet::from(["remote".to_owned()]),
    }
}

#[test]
fn one_sign_in_serves_every_agent_and_no_agent_sees_the_token() {
    let seen = Shared::default();
    let url = remote(seen.clone());
    let store = Arc::new(Memory::default());
    sign_in(
        &url,
        "test-hub",
        store.clone(),
        browser,
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(seen.lock().unwrap().token_requests, 1);
    assert!(store
        .load()
        .unwrap()
        .unwrap()
        .contains("remote-access-token"));

    let hub = hub_for(&url, store);
    for session in [1, 2] {
        let token = hub.mint(grant(session));
        let reply = mcp_hub::call_tool(
            mcp_hub::Connection {
                url: hub.url().to_owned(),
                token: token.secret().to_owned(),
            },
            "remote__ping",
            json!({}),
        )
        .unwrap();
        assert_eq!(reply, json!("pong"));
        assert_ne!(token.secret(), "remote-access-token");
        // The agent's own token stops at the hub.
        let seen = seen.lock().unwrap();
        assert!(!seen
            .authorizations
            .iter()
            .any(|sent| sent.contains(token.secret())));
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.token_requests, 1, "one sign-in, for both agents");
    assert!(seen
        .authorizations
        .iter()
        .all(|sent| sent == "Bearer remote-access-token"));
}

#[test]
fn an_expired_token_is_refreshed_without_a_new_sign_in() {
    let seen = Shared::default();
    seen.lock().unwrap().lifetime = Some(0);
    let url = remote(seen.clone());
    let store = Arc::new(Memory::default());
    sign_in(
        &url,
        "test-hub",
        store.clone(),
        browser,
        Duration::from_secs(10),
    )
    .unwrap();
    let hub = hub_for(&url, store.clone());
    let token = hub.mint(grant(1));
    let reply = mcp_hub::call_tool(
        mcp_hub::Connection {
            url: hub.url().to_owned(),
            token: token.secret().to_owned(),
        },
        "remote__ping",
        json!({}),
    )
    .unwrap();
    assert_eq!(reply, json!("pong"));
    assert!(seen.lock().unwrap().refreshed);
    assert!(store.load().unwrap().unwrap().contains("refreshed-token"));
}

#[test]
fn a_call_before_any_sign_in_says_so() {
    let seen = Shared::default();
    let url = remote(seen);
    let hub = hub_for(&url, Arc::new(Memory::default()));
    let token = hub.mint(grant(1));
    let error = mcp_hub::call_tool(
        mcp_hub::Connection {
            url: hub.url().to_owned(),
            token: token.secret().to_owned(),
        },
        "remote__ping",
        json!({}),
    )
    .unwrap_err();
    assert!(
        error.contains("isn't available") || error.contains("sign-in"),
        "{error}"
    );
}

/// Speaks to a connection the way the hub does.
fn say(spawned: &mut Spawned, message: Value) {
    writeln!(spawned.stdin, "{message}").unwrap();
    spawned.stdin.flush().unwrap();
}

fn ended_in_time(mut stdout: Box<dyn Read + Send>) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = stdout.read_to_end(&mut rest);
        let _ = done.send(());
    });
    finished.recv_timeout(Duration::from_secs(5)).is_ok()
}

fn wait_for_listens(seen: &Shared, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while seen.lock().unwrap().listens.len() < count && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen.lock().unwrap().listens.len(), count);
}

#[test]
fn stopping_a_connection_ends_it_even_with_its_event_stream_open() {
    let seen = Shared::default();
    seen.lock().unwrap().hold_listen = true;
    let url = remote(seen.clone());
    let mut spawned = connect(&url, BTreeMap::new(), None).unwrap();
    say(
        &mut spawned,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    wait_for_listens(&seen, 1);
    let Spawned { stop, stdout, .. } = spawned;
    stop();
    assert!(ended_in_time(stdout), "the connection's threads are stuck");
}

#[test]
fn a_stale_token_on_the_event_stream_is_refreshed_and_retried() {
    let seen = Shared::default();
    let url = remote(seen.clone());
    let store = Arc::new(Memory::default());
    sign_in(&url, "t", store.clone(), browser, Duration::from_secs(5)).unwrap();
    // Discovery looks at the same address.
    seen.lock().unwrap().listens.clear();
    seen.lock().unwrap().stale_on_listen = true;
    let mut spawned = connect(&url, BTreeMap::new(), Some(store)).unwrap();
    say(
        &mut spawned,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    wait_for_listens(&seen, 2);
    assert_eq!(
        seen.lock().unwrap().listens[1],
        "Bearer remote-access-token"
    );
}

#[test]
fn a_new_connection_survives_the_end_of_the_one_it_replaced() {
    let seen = Shared::default();
    let url = remote(seen);
    let store = Arc::new(Memory::default());
    sign_in(&url, "t", store.clone(), browser, Duration::from_secs(5)).unwrap();
    let hub = hub_for(&url, store);
    let connection = Connection {
        url: hub.url().to_owned(),
        token: hub.mint(grant(1)).secret().to_owned(),
    };
    for _ in 0..3 {
        call_tool(connection.clone(), "remote__ping", json!({})).unwrap();
        hub.restart("remote");
    }
    call_tool(connection, "remote__ping", json!({})).unwrap();
    // A stopped connection ending must not take the live one with it.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(hub.running().len(), 1);
    assert!(hub.errors().is_empty(), "{:?}", hub.errors());
}

#[test]
fn a_restart_starts_the_server_again_with_what_a_sign_in_saved() {
    let seen = Shared::default();
    let url = remote(seen);
    let store = Arc::new(Memory::default());
    let hub = hub_for(&url, store.clone());
    let connection = Connection {
        url: hub.url().to_owned(),
        token: hub.mint(grant(1)).secret().to_owned(),
    };
    assert!(call_tool(connection.clone(), "remote__ping", json!({})).is_err());
    sign_in(&url, "t", store, browser, Duration::from_secs(5)).unwrap();
    hub.restart("remote");
    call_tool(connection, "remote__ping", json!({})).unwrap();
}

#[test]
fn credentials_never_leave_for_an_address_that_would_show_them() {
    let headers = BTreeMap::from([("Authorization".to_owned(), "Bearer key".to_owned())]);
    let plain = "http://mcp.example/mcp";
    assert!(connect(plain, headers.clone(), None).is_err());
    let oauth = Arc::new(Memory::default());
    assert!(connect(plain, BTreeMap::new(), Some(oauth)).is_err());
    assert!(
        connect(plain, BTreeMap::new(), None).is_ok(),
        "nothing to read"
    );
    assert!(connect("http://127.0.0.1:9/mcp", headers.clone(), None).is_ok());
    assert!(connect("https://mcp.example/mcp", headers, None).is_ok());
}

#[test]
fn a_server_that_signs_in_over_plain_http_is_refused_before_the_browser_opens() {
    let seen = Shared::default();
    seen.lock().unwrap().plain_token_endpoint = true;
    let url = remote(seen);
    let store = Arc::new(Memory::default());
    let opened = std::sync::atomic::AtomicBool::new(false);
    let error = sign_in(
        &url,
        "t",
        store.clone(),
        |_| opened.store(true, std::sync::atomic::Ordering::SeqCst),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(error.contains("unsafe"), "{error}");
    assert!(!opened.load(std::sync::atomic::Ordering::SeqCst));
    assert!(store.load().unwrap().is_none());
}

#[test]
fn a_server_that_never_answers_discovery_cant_outlast_the_sign_in() {
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", silent.local_addr().unwrap());
    let error = sign_in(
        &url,
        "t",
        Arc::new(Memory::default()),
        |_| {},
        Duration::from_millis(200),
    )
    .unwrap_err();
    assert_eq!(error, "The sign-in timed out");
}

#[test]
fn a_sign_in_nobody_finishes_times_out() {
    let url = remote(Shared::default());
    let store = Arc::new(Memory::default());
    let error = sign_in(&url, "t", store.clone(), |_| {}, Duration::from_millis(100)).unwrap_err();
    assert_eq!(error, "The sign-in timed out");
    assert!(store.load().unwrap().is_none());
}
