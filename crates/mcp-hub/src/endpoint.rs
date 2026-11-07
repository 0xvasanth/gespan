//! The loopback HTTP listener and its gate. Host and Origin are checked before
//! anything else, which is what stops a web page reaching the hub through DNS
//! rebinding; the bearer token then names the session.
use crate::{
    grants::{Grants, Refusal},
    pool::Pool,
    router::{log_rank, Action},
    session::Session,
    upstream::{Answer, Upstream},
    Grant, NoObserver, Observer, Running, ServerError, ServerSpec, Spawned, Spawner, Token,
    Toolkit,
};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::{
    model::{
        CreateMessageRequest, CreateMessageRequestParams, ElicitRequest, GetExtensions,
        ListRootsRequest, RequestId, ServerRequest,
    },
    service::{OriginatingRequestId, Peer, RoleServer},
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

pub struct Config {
    /// The name agents see in `initialize`.
    pub name: String,
    /// The version agents see in `initialize`.
    pub version: String,
    /// How long a credential lasts with no use. A running turn holds it open
    /// with [`Hub::touch`].
    pub idle: Duration,
    /// How long a server process waits with nothing to do before it stops.
    pub server_idle: Duration,
    /// The most server processes at once. Idle ones stop first.
    pub max_servers: usize,
    /// Starts server processes. Without one, the hub serves toolkits alone.
    pub spawner: Option<Arc<dyn Spawner>>,
    pub observer: Arc<dyn Observer>,
}

impl Config {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            idle: Duration::from_secs(60 * 60),
            server_idle: Duration::from_secs(5 * 60),
            max_servers: 20,
            spawner: None,
            observer: Arc::new(NoObserver),
        }
    }
}

pub(crate) struct Inner {
    pub(crate) name: String,
    pub(crate) version: String,
    toolkits: RwLock<Vec<Arc<dyn Toolkit>>>,
    grants: Mutex<Grants>,
    peers: Mutex<HashMap<i64, Vec<Peer<RoleServer>>>>,
    changes: broadcast::Sender<(Option<i64>, Listing)>,
    runtime: tokio::runtime::Handle,
    pub(crate) pool: Arc<Pool>,
    /// The peer and request of each session's latest call, for messages about
    /// that call.
    callers: Mutex<HashMap<i64, (Peer<RoleServer>, RequestId)>>,
    /// What each session asked to hear from server logs.
    levels: Mutex<HashMap<i64, u8>>,
}

/// The lists a client can be told to read again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Listing {
    Tools,
    Prompts,
    Resources,
}

impl Listing {
    pub(crate) const ALL: [Self; 3] = [Self::Tools, Self::Prompts, Self::Resources];

    fn of_method(method: &str) -> Self {
        match method {
            "notifications/prompts/list_changed" => Self::Prompts,
            "notifications/resources/list_changed" => Self::Resources,
            _ => Self::Tools,
        }
    }
}

fn relock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Inner {
    fn toolkits(&self) -> Vec<Arc<dyn Toolkit>> {
        self.toolkits
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn tools(&self, grant: &Grant) -> Vec<Value> {
        self.toolkits()
            .iter()
            .filter(|toolkit| grant.allows(toolkit.name()))
            .flat_map(|toolkit| toolkit.tools(grant))
            .collect()
    }

    /// Finds the tool in the caller's own list, so a tool it wasn't offered
    /// can't be reached by name.
    pub(crate) fn call(
        &self,
        grant: &Grant,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let toolkit = self
            .toolkits()
            .into_iter()
            .filter(|toolkit| grant.allows(toolkit.name()))
            .find(|toolkit| {
                toolkit
                    .tools(grant)
                    .iter()
                    .any(|listed| listed["name"] == tool)
            })
            .ok_or_else(|| format!("Tool {tool} isn't available in this session"))?;
        toolkit.call(grant, tool, arguments)
    }

    pub(crate) fn calling(&self, session: i64, peer: Peer<RoleServer>, request: RequestId) {
        relock(&self.callers).insert(session, (peer, request));
    }

    pub(crate) fn set_level(&self, session: i64, level: &str) {
        relock(&self.levels).insert(session, log_rank(level));
    }

    fn peers_of(&self, session: i64) -> Vec<Peer<RoleServer>> {
        let mut peers = relock(&self.peers)
            .get(&session)
            .cloned()
            .unwrap_or_default();
        peers.retain(|peer| !peer.is_transport_closed());
        peers
    }

    /// Turns away a tool call made for another caller of a bound credential.
    pub(crate) fn check_caller(
        &self,
        session: i64,
        meta: &serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        match relock(&self.grants).bound(session) {
            Some((key, value)) if meta.get(&key) != Some(&value) => {
                Err("This tool credential belongs to another conversation".to_owned())
            }
            _ => Ok(()),
        }
    }

    fn peer_of(&self, session: i64) -> Option<Peer<RoleServer>> {
        self.call_of(session)
            .map(|(peer, _)| peer)
            .or_else(|| self.peers_of(session).pop())
    }

    fn call_of(&self, session: i64) -> Option<(Peer<RoleServer>, RequestId)> {
        relock(&self.callers)
            .get(&session)
            .filter(|(peer, _)| !peer.is_transport_closed())
            .cloned()
    }

    /// A list of a server's changed: everyone who can see it reads it again.
    pub(crate) fn server_list_changed(&self, server: &str, method: &str) {
        let listing = Listing::of_method(method);
        let holders = relock(&self.grants).holders(server);
        for session in holders {
            let _ = self.changes.send((Some(session), listing));
            for peer in self.peers_of(session) {
                self.tell(peer, listing);
            }
        }
    }

    fn tell(&self, peer: Peer<RoleServer>, listing: Listing) {
        self.runtime.spawn(async move {
            let _ = match listing {
                Listing::Tools => peer.notify_tool_list_changed().await,
                Listing::Prompts => peer.notify_prompt_list_changed().await,
                Listing::Resources => peer.notify_resource_list_changed().await,
            };
        });
    }

    /// Ends idle credentials and whatever their sessions held, then stops
    /// idle servers.
    fn sweep(&self) {
        let ended = relock(&self.grants).expire();
        for session in ended {
            self.pool.forget(session);
        }
        self.pool.sweep();
    }

    pub(crate) fn server_action(
        self: &Arc<Self>,
        server: &str,
        upstream: &Arc<Upstream>,
        action: Action,
    ) {
        match action {
            Action::Notify { caller, message } => {
                let Some(peer) = self.peer_of(caller) else {
                    return;
                };
                let params = serde_json::from_value(message["params"].clone());
                self.runtime.spawn(async move {
                    if let Ok(params) = params {
                        let _ = peer.notify_progress(params).await;
                    }
                });
            }
            Action::ResourceUpdated { callers, message } => {
                let uri = message["params"]["uri"].as_str().unwrap_or("");
                let uri = crate::pool::resource_uri(server, uri);
                for peer in callers.iter().flat_map(|caller| self.peers_of(*caller)) {
                    let uri = uri.clone();
                    self.runtime.spawn(async move {
                        let params = rmcp::model::ResourceUpdatedNotificationParam::new(uri);
                        let _ = peer.notify_resource_updated(params).await;
                    });
                }
            }
            Action::Log { level, message } => {
                // A log line says nothing of whose work it was about, so with
                // several users it can't be sent to any of them.
                let users = upstream.users();
                if users.len() != 1 {
                    return;
                }
                let rank = log_rank(&level);
                let levels = relock(&self.levels).clone();
                for session in users {
                    // An agent that never set a level hears warnings and up.
                    if rank < levels.get(&session).copied().unwrap_or(log_rank("warning")) {
                        continue;
                    }
                    for peer in self.peers_of(session) {
                        let params = serde_json::from_value(message["params"].clone());
                        self.runtime.spawn(async move {
                            if let Ok(params) = params {
                                let _ = peer.notify_logging_message(params).await;
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// What a session's agent said it can do when it connected.
    pub(crate) fn capabilities_of(&self, session: i64) -> Option<Value> {
        let info = self.peer_of(session)?.peer_info()?;
        serde_json::to_value(&info.capabilities).ok()
    }

    /// Puts a server's question to a session's agent, on the call it's
    /// making if there is one.
    pub(crate) fn ask_agent(&self, caller: i64, method: String, params: Value, answer: Answer) {
        let call = self
            .call_of(caller)
            .map(|(peer, request)| (peer, Some(request)))
            .or_else(|| self.peer_of(caller).map(|peer| (peer, None)));
        self.runtime.spawn(async move {
            let outcome = match call {
                Some((peer, origin)) => ask(&peer, origin, &method, params).await,
                None => Err("The agent that made this call has gone".to_owned()),
            };
            answer(outcome);
        });
    }

    pub(crate) fn watch(&self, session: i64, peer: Peer<RoleServer>) {
        let mut peers = relock(&self.peers);
        let list = peers.entry(session).or_default();
        list.retain(|peer| !peer.is_transport_closed());
        list.push(peer);
    }

    pub(crate) fn changes(&self) -> broadcast::Receiver<(Option<i64>, Listing)> {
        self.changes.subscribe()
    }

    fn notify(&self, session: Option<i64>, listings: &[Listing]) {
        for listing in listings {
            let _ = self.changes.send((session, *listing));
        }
        let peers: Vec<_> = {
            let mut watched = relock(&self.peers);
            watched.retain(|_, list| {
                list.retain(|peer| !peer.is_transport_closed());
                !list.is_empty()
            });
            watched
                .iter()
                .filter(|(watched, _)| session.is_none_or(|session| session == **watched))
                .flat_map(|(_, list)| list.iter().cloned())
                .collect()
        };
        for peer in peers {
            for listing in listings {
                self.tell(peer.clone(), *listing);
            }
        }
    }
}

/// Passes a server's question to the agent it belongs to, on the stream of the
/// call it came from: an agent needn't hold the standalone stream open. rmcp
/// marks this itself only inside a handler's own task.
async fn ask(
    peer: &Peer<RoleServer>,
    origin: Option<RequestId>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let mut request = match method {
        "sampling/createMessage" => {
            let params: CreateMessageRequestParams =
                serde_json::from_value(params).map_err(|e| e.to_string())?;
            if (params.tools.is_some() || params.tool_choice.is_some())
                && !peer.supports_sampling_tools()
            {
                return Err("This agent can't offer tools to a sampling request".to_owned());
            }
            params.validate()?;
            ServerRequest::CreateMessageRequest(CreateMessageRequest::new(params))
        }
        "elicitation/create" => {
            let params = serde_json::from_value(params).map_err(|e| e.to_string())?;
            ServerRequest::ElicitRequest(ElicitRequest::new(params))
        }
        "roots/list" => ServerRequest::ListRootsRequest(ListRootsRequest::default()),
        _ => return Err(format!("{method} isn't available here")),
    };
    if let Some(origin) = origin {
        request
            .extensions_mut()
            .insert(OriginatingRequestId(origin));
    }
    let answer = peer
        .send_request(request)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(answer).map_err(|e| e.to_string())
}

/// Used when the host gave no spawner: nothing can be offered, so nothing runs.
struct NoSpawner;
impl Spawner for NoSpawner {
    fn spawn(&self, _: &ServerSpec, _: Option<&std::path::Path>) -> std::io::Result<Spawned> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// The running endpoint. Dropping it stops the listener, and with it every
/// request in flight: agents see a failed call, not a hang.
pub struct Hub {
    inner: Arc<Inner>,
    url: String,
    stop: CancellationToken,
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.stop.cancel();
        self.inner.pool.shutdown();
    }
}

#[derive(Clone)]
struct Gate {
    inner: Arc<Inner>,
    address: SocketAddr,
}

impl Hub {
    pub fn start(config: Config) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mcp-hub")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let pool = Arc::new(Pool::new(
            config.spawner.unwrap_or_else(|| Arc::new(NoSpawner)),
            config.observer,
            config.server_idle,
            config.max_servers,
        ));
        let inner = Arc::new(Inner {
            name: config.name,
            version: config.version,
            toolkits: RwLock::default(),
            grants: Mutex::new(Grants::new(config.idle)),
            peers: Mutex::default(),
            changes: broadcast::channel(64).0,
            runtime: runtime.handle().clone(),
            pool: pool.clone(),
            callers: Mutex::default(),
            levels: Mutex::default(),
        });
        pool.attach(Arc::downgrade(&inner));
        let stop = CancellationToken::new();
        let sweeper = stop.clone();
        let swept = inner.clone();
        runtime.spawn(async move {
            let mut ticks = tokio::time::interval(pool.sweep_interval());
            loop {
                tokio::select! {
                    () = sweeper.cancelled() => return,
                    _ = ticks.tick() => swept.sweep(),
                }
            }
        });
        let service = {
            let inner = inner.clone();
            StreamableHttpService::new(
                move || {
                    Ok(Session {
                        inner: inner.clone(),
                    })
                },
                LocalSessionManager::default().into(),
                StreamableHttpServerConfig::default()
                    .with_cancellation_token(stop.child_token())
                    .with_allowed_hosts(["127.0.0.1", "localhost"]),
            )
        };
        let gate = Gate {
            inner: inner.clone(),
            address,
        };
        let app = Router::new()
            .route_service("/mcp", service)
            .layer(middleware::from_fn_with_state(gate, admit));
        let shutdown = stop.clone();
        std::thread::Builder::new()
            .name("mcp-hub".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                        return;
                    };
                    tokio::select! {
                        () = shutdown.cancelled() => {}
                        result = axum::serve(listener, app) => {
                            if let Err(error) = result {
                                tracing::warn!(%error, "the tool endpoint stopped");
                            }
                        }
                    }
                });
                runtime.shutdown_background();
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            inner,
            url: format!("http://{address}/mcp"),
            stop,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn register(&self, toolkit: Arc<dyn Toolkit>) {
        self.inner
            .toolkits
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(toolkit);
        self.inner.notify(None, &[Listing::Tools]);
    }

    /// A credential for a new agent process. Any earlier one for the session
    /// stops working.
    pub fn mint(&self, grant: Grant) -> Token {
        relock(&self.inner.grants).mint(grant)
    }

    /// Binds a session's credential to one caller of a client that shares its
    /// connections: from now on its tool calls must carry `value` under `key`
    /// in their `_meta`, which that client sets and its model can't, and calls
    /// without it are turned away. A new credential starts unbound. Returns
    /// whether the session had a live credential to bind.
    pub fn bind(&self, session: i64, key: &str, value: Value) -> bool {
        relock(&self.inner.grants).bind(session, key, value)
    }

    /// Ends a session's access, for an archived, deleted or moved session.
    pub fn revoke(&self, session: i64) {
        relock(&self.inner.grants).revoke(session, Refusal::Ended);
        self.inner.pool.forget(session);
    }

    /// Keeps a session's credential alive while its turn runs.
    pub fn touch(&self, session: i64) {
        relock(&self.inner.grants).touch(session);
    }

    pub fn has_grant(&self, session: i64) -> bool {
        relock(&self.inner.grants).has(session)
    }

    /// Whether this token is still the one a session can use.
    pub fn holds(&self, session: i64, token: &Token) -> bool {
        relock(&self.inner.grants).holds(session, token)
    }

    /// Changes what a session may reach and tells its agent, whose tool list
    /// then changes with no restart. Returns whether anything changed.
    pub fn update(
        &self,
        session: i64,
        toolkits: BTreeSet<String>,
        servers: BTreeSet<String>,
    ) -> bool {
        let (updated, before) = {
            let mut grants = relock(&self.inner.grants);
            let before = grants.servers_of(session);
            (grants.update(session, toolkits, servers.clone()), before)
        };
        if !updated {
            return false;
        }
        if before == servers {
            self.inner.notify(Some(session), &[Listing::Tools]);
        } else {
            self.inner.pool.release(session, &(&before - &servers));
            self.inner.notify(Some(session), &Listing::ALL);
        }
        true
    }

    /// Replaces the servers on offer and tells every agent. A process for
    /// anything that changed stops.
    pub fn set_servers(&self, servers: Vec<ServerSpec>) {
        self.inner.pool.set(servers);
        self.inner.notify(None, &Listing::ALL);
    }

    /// Stops a server so its next call starts over, and tells every agent.
    /// For what changed outside its spec, such as a sign-in.
    pub fn restart(&self, server: &str) {
        self.inner.pool.restart(server);
        self.inner.notify(None, &Listing::ALL);
    }

    /// The server processes running now.
    pub fn running(&self) -> Vec<Running> {
        self.inner.pool.snapshot()
    }

    /// Recent server failures, oldest first.
    pub fn errors(&self) -> Vec<ServerError> {
        self.inner.pool.errors()
    }

    /// Tells a session's agent, or every agent, to read its tool list again.
    pub fn tools_changed(&self, session: Option<i64>) {
        self.inner.notify(session, &[Listing::Tools]);
    }
}

fn authority_allowed(host: &str, address: SocketAddr) -> bool {
    let port = address.port();
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// Native clients send no `Origin`. Anything else is a browser, and only the
/// hub's own origin would be same-site, which nothing legitimate is.
fn origin_allowed(headers: &HeaderMap, address: SocketAddr) -> bool {
    headers.get(header::ORIGIN).is_none_or(|origin| {
        origin.to_str().is_ok_and(|origin| {
            origin
                .strip_prefix("http://")
                .is_some_and(|authority| authority_allowed(authority, address))
        })
    })
}

fn host_allowed(headers: &HeaderMap, address: SocketAddr) -> bool {
    headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| authority_allowed(host, address))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Always 403, never 401: Claude Code, OpenCode and Gemini answer a 401 by
/// opening a browser for an OAuth sign-in that can't exist here.
fn refuse(message: &str) -> Response {
    let body = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":message}});
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_default()
}

/// Reads the request body before answering. A connection closed with part of
/// a request unread is reset, and on Windows the reset discards a refusal the
/// agent hasn't read yet, so the agent would see an unreachable endpoint
/// instead of the reason.
async fn turn_away(request: Request, message: &str) -> Response {
    let _ = axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await;
    refuse(message)
}

async fn admit(State(gate): State<Gate>, mut request: Request, next: Next) -> Response {
    let headers = request.headers();
    if !host_allowed(headers, gate.address) {
        return turn_away(request, "Host not allowed").await;
    }
    if !origin_allowed(headers, gate.address) {
        return turn_away(request, "Origin not allowed").await;
    }
    let admitted = bearer(headers)
        .ok_or(Refusal::Invalid)
        .and_then(|secret| relock(&gate.inner.grants).admit(secret));
    let grant = match admitted {
        Ok(grant) => grant,
        Err(refusal) => return turn_away(request, &refusal.to_string()).await,
    };
    request.extensions_mut().insert(grant);
    let request = request.map(|body| Body::new(http_body_util::Limited::new(body, MAX_BODY_BYTES)));
    next.run(request).await
}
