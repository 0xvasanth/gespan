//! The loopback HTTP listener and its gate. Host and Origin are checked before
//! anything else, which is what stops a web page reaching the hub through DNS
//! rebinding; the bearer token then names the session.
use crate::{
    grants::{Grants, Refusal},
    session::Session,
    Grant, Token, Toolkit,
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
    service::{Peer, RoleServer},
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
}

impl Config {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            idle: Duration::from_secs(60 * 60),
        }
    }
}

pub(crate) struct Inner {
    pub(crate) name: String,
    pub(crate) version: String,
    toolkits: RwLock<Vec<Arc<dyn Toolkit>>>,
    grants: Mutex<Grants>,
    peers: Mutex<HashMap<i64, Vec<Peer<RoleServer>>>>,
    changes: broadcast::Sender<Option<i64>>,
    runtime: tokio::runtime::Handle,
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

    pub(crate) fn watch(&self, session: i64, peer: Peer<RoleServer>) {
        let mut peers = relock(&self.peers);
        let list = peers.entry(session).or_default();
        list.retain(|peer| !peer.is_transport_closed());
        list.push(peer);
    }

    pub(crate) fn changes(&self) -> broadcast::Receiver<Option<i64>> {
        self.changes.subscribe()
    }

    fn notify(&self, session: Option<i64>) {
        let _ = self.changes.send(session);
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
            self.runtime.spawn(async move {
                let _ = peer.notify_tool_list_changed().await;
            });
        }
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
        let inner = Arc::new(Inner {
            name: config.name,
            version: config.version,
            toolkits: RwLock::default(),
            grants: Mutex::new(Grants::new(config.idle)),
            peers: Mutex::default(),
            changes: broadcast::channel(64).0,
            runtime: runtime.handle().clone(),
        });
        let stop = CancellationToken::new();
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
        self.inner.notify(None);
    }

    /// A credential for a new agent process. Any earlier one for the session
    /// stops working.
    pub fn mint(&self, grant: Grant) -> Token {
        relock(&self.inner.grants).mint(grant)
    }

    /// Ends a session's access, for an archived, deleted or moved session.
    pub fn revoke(&self, session: i64) {
        relock(&self.inner.grants).revoke(session, Refusal::Ended);
    }

    /// Keeps a session's credential alive while its turn runs.
    pub fn touch(&self, session: i64) {
        relock(&self.inner.grants).touch(session);
    }

    pub fn has_grant(&self, session: i64) -> bool {
        relock(&self.inner.grants).has(session)
    }

    /// Changes what a session may reach and tells its agent, whose tool list
    /// then changes with no restart. Returns whether anything changed.
    pub fn update(&self, session: i64, toolkits: BTreeSet<String>) -> bool {
        let updated = relock(&self.inner.grants).update(session, toolkits);
        if updated {
            self.inner.notify(Some(session));
        }
        updated
    }

    /// Tells a session's agent, or every agent, to read its tool list again.
    pub fn tools_changed(&self, session: Option<i64>) {
        self.inner.notify(session);
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

async fn admit(State(gate): State<Gate>, mut request: Request, next: Next) -> Response {
    let headers = request.headers();
    if !host_allowed(headers, gate.address) {
        return refuse("Host not allowed");
    }
    if !origin_allowed(headers, gate.address) {
        return refuse("Origin not allowed");
    }
    let admitted = bearer(headers)
        .ok_or(Refusal::Invalid)
        .and_then(|secret| relock(&gate.inner.grants).admit(secret));
    let grant = match admitted {
        Ok(grant) => grant,
        Err(refusal) => return refuse(&refusal.to_string()),
    };
    request.extensions_mut().insert(grant);
    let request = request.map(|body| Body::new(http_body_util::Limited::new(body, MAX_BODY_BYTES)));
    next.run(request).await
}
