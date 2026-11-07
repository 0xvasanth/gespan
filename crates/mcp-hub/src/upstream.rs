//! One running MCP server, spoken to over stdio with rmcp's client. The hub
//! starts it once with its own capabilities, whoever asked for it first.
//!
//! rmcp settles which protocol it speaks: `server/discover` first, and
//! `initialize` for a server that refuses it. On the sessionless protocol rmcp
//! says who's asking on every request and sends a call that answers
//! `input_required` again with the input; notifications come on a
//! `subscriptions/listen` stream the hub opens.
//!
//! rmcp sees one client. The [`Router`] keeps what sharing it between agents
//! needs: whose call is whose, their progress tokens, who holds which
//! subscription, and which agent a server's question can go to.
use crate::{
    router::{Action, Caller, Input, Reach, Router},
    ServerSpec,
};
use rmcp::{
    model::{
        CallToolRequestParams, ClientCapabilities, ClientConfig, ClientJsonRpcMessage,
        ClientRequest, CreateMessageRequestParams, CreateMessageResult, CustomRequest,
        CustomResult, ElicitRequestParams, ElicitResult, ErrorCode, ErrorData,
        GetPromptRequestParams, Implementation, ListRootsResult, LoggingMessageNotificationParam,
        ProgressNotificationParam, ProtocolVersion, ReadResourceRequestParams,
        ResourceUpdatedNotificationParam, ServerJsonRpcMessage, ServerNotification,
        SubscriptionFilter,
    },
    service::{
        ClientCacheConfig, ClientInitializeError, ClientLifecycleMode, ClientServiceExt,
        NotificationContext, PeerRequestOptions, RequestContext, RoleClient, RunningService,
        ServiceError,
    },
    transport::Transport,
    ClientHandler, Peer,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    future::Future,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};
use tokio::{runtime::Handle, sync::oneshot, task::AbortHandle};
use tokio_util::sync::CancellationToken;

const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;
/// More rounds of `input_required` than a real call needs, so a server can't
/// keep one going forever.
const MAX_ROUNDS: usize = 10;
/// A server that doesn't answer in this long is treated as failed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// More pages than any real list, so a server can't keep one going forever.
const MAX_PAGES: usize = 100;
/// Marks which agent call a request on the wire belongs to. It's read and
/// taken out on the way to the server, which never sees it.
const CALL: &str = "mcp-hub/call";
const VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// What a server answered was no result at all.
pub(crate) const MALFORMED: i64 = -32700;

/// A started server process, handed back by the host's [`Spawner`].
pub struct Spawned {
    pub pid: u32,
    pub stdin: Box<dyn Write + Send>,
    pub stdout: Box<dyn Read + Send>,
    /// Stops the process and everything it started.
    pub stop: Box<dyn FnOnce() + Send>,
}

/// Starts server processes. The host owns how: process groups, recorded pids
/// and resolved secrets are its business.
pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self, spec: &ServerSpec, workspace: Option<&Path>) -> io::Result<Spawned>;
}

/// Called once with what an agent answered.
pub(crate) type Answer = Box<dyn FnOnce(Result<Value, String>) + Send>;

/// What an upstream reports to the pool.
pub(crate) trait Events: Send + Sync + 'static {
    fn action(&self, upstream: &Arc<Upstream>, action: Action);
    fn exited(&self, upstream: &Arc<Upstream>);
    /// Puts a server's question to the agent whose call it belongs to.
    fn ask(&self, caller: Caller, method: String, params: Value, answer: Answer);
    /// What the agent of this session said it can do, when it said.
    fn capabilities(&self, _caller: Caller) -> Option<Value> {
        None
    }
}

tokio::task_local! {
    /// The agent whose call rmcp is getting input for. rmcp asks for it within
    /// the call's own task; a server's request on the older protocol comes on
    /// a task of rmcp's, without one.
    static CALLER: Caller;
}

/// An agent's call in flight.
struct Call {
    /// What the server is told this agent can do, on the sessionless protocol.
    capabilities: Value,
    /// rmcp's progress tokens for this call's requests, one per round.
    tokens: Vec<String>,
    /// Ends the call when its session goes.
    stop: CancellationToken,
}

struct Link {
    router: Router,
    /// Sessions that have used this process.
    users: BTreeSet<Caller>,
    used: Instant,
    /// Requests the hub is waiting on for itself.
    owned: usize,
    closed: bool,
    /// By the router's id.
    calls: HashMap<u64, Call>,
    /// rmcp's progress token on the wire, to the call it was sent for.
    tokens: HashMap<String, u64>,
    sessionless: bool,
    /// The newest `subscriptions/listen` asked for, and the one in place.
    listens: u64,
    listening: Option<(u64, AbortHandle)>,
}

/// What the transport, the handler and the upstream all hold.
struct Shared {
    link: Mutex<Link>,
    events: Arc<dyn Events>,
    /// Set once the server has started.
    upstream: OnceLock<Weak<Upstream>>,
    runtime: Handle,
    /// Whether this process serves a folder, which is then its root.
    rooted: bool,
}

impl Shared {
    fn linked(&self) -> std::sync::MutexGuard<'_, Link> {
        self.link.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn upstream(&self) -> Option<Arc<Upstream>> {
        self.upstream.get().and_then(Weak::upgrade)
    }

    fn act(&self, action: Action) {
        if let Some(upstream) = self.upstream() {
            self.events.action(&upstream, action);
        }
    }

    /// A notification from the server, as a frame for the router.
    fn notified(&self, frame: Value) {
        let actions = self.linked().router.inbound(frame);
        for action in actions {
            self.act(action);
        }
    }

    /// rmcp numbers progress itself, so the call is found by its token.
    fn progressed(&self, params: ProgressNotificationParam) {
        let Ok(mut frame) = serde_json::to_value(&params) else {
            return;
        };
        let token = frame["progressToken"].to_string();
        let ours = {
            let link = self.linked();
            let call = link.tokens.get(&token).copied();
            call.and_then(|call| link.router.progress_of(call).map(str::to_owned))
        };
        let Some(ours) = ours else {
            return;
        };
        frame["progressToken"] = ours.into();
        self.notified(json!({"method":"notifications/progress","params":frame}));
    }

    /// Takes out which call a request belongs to and says, on the sessionless
    /// protocol, what that call's agent can do.
    fn stamp(&self, frame: &mut Value) {
        let Some(meta) = frame
            .get_mut("params")
            .and_then(|params| params.get_mut("_meta"))
            .and_then(Value::as_object_mut)
        else {
            return;
        };
        let Some(call) = meta.remove(CALL).and_then(|call| call.as_u64()) else {
            return;
        };
        let mut link = self.linked();
        let Link { calls, tokens, .. } = &mut *link;
        let Some(entry) = calls.get_mut(&call) else {
            return;
        };
        if let Some(token) = meta.get("progressToken") {
            let token = token.to_string();
            entry.tokens.push(token.clone());
            tokens.insert(token, call);
        }
        if meta.contains_key(VERSION) {
            meta.insert(CAPABILITIES.to_owned(), entry.capabilities.clone());
        }
    }

    fn closed(&self) {
        let mut link = self.linked();
        link.closed = true;
        for call in link.calls.values() {
            call.stop.cancel();
        }
    }

    fn exited(&self) {
        self.closed();
        if let Some(upstream) = self.upstream() {
            self.events.exited(&upstream);
        }
    }

    /// Puts a server's question to an agent and waits for its answer.
    async fn put(&self, caller: Caller, method: &str, params: Value) -> Result<Value, ErrorData> {
        let (send, receive) = oneshot::channel();
        let answer: Answer = Box::new(move |outcome| {
            let _ = send.send(outcome);
        });
        self.events.ask(caller, method.to_owned(), params, answer);
        let outcome = receive
            .await
            .unwrap_or_else(|_| Err("The agent that made this call has gone".into()));
        outcome.map_err(|message| ErrorData::internal_error(message, None))
    }
}

pub(crate) struct Upstream {
    pub(crate) pid: u32,
    pub(crate) started: Instant,
    shared: Arc<Shared>,
    service: RunningService<RoleClient, Client>,
    stop: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

fn gone() -> Value {
    json!({"code":-32000,"message":"The MCP server stopped"})
}

fn too_long() -> Value {
    json!({"code":-32001,"message":"The MCP server took too long"})
}

fn failed(message: impl Into<String>) -> Value {
    json!({"code":-32603,"message":message.into()})
}

fn failure(error: ServiceError) -> Value {
    match error {
        ServiceError::McpError(error) => serde_json::to_value(error).unwrap_or_else(|_| gone()),
        ServiceError::Timeout { .. } => too_long(),
        ServiceError::InputRequiredRoundsExceeded { .. } => {
            failed("The MCP server kept asking for more input")
        }
        ServiceError::UnexpectedResponse => json!({
            "code":MALFORMED,
            "message":"The MCP server's answer wasn't a result",
        }),
        ServiceError::TransportClosed | ServiceError::Cancelled { .. } => gone(),
        error => failed(error.to_string()),
    }
}

fn answer_of(result: impl serde::Serialize) -> Result<Value, Value> {
    serde_json::to_value(result).map_err(|error| failed(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ErrorData> {
    serde_json::from_value(value)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

impl Upstream {
    fn linked(&self) -> std::sync::MutexGuard<'_, Link> {
        self.shared.linked()
    }

    fn peer(&self) -> &Peer<RoleClient> {
        self.service.peer()
    }

    /// Starts the server's protocol. Done once it's ready.
    pub(crate) async fn start(
        spec: &ServerSpec,
        spawned: Spawned,
        workspace: Option<&Path>,
        reach: Reach,
        events: Arc<dyn Events>,
    ) -> Result<Arc<Self>, String> {
        let Spawned {
            pid,
            stdin,
            stdout,
            stop,
        } = spawned;
        let root = workspace.and_then(root_uri);
        let shared = Arc::new(Shared {
            link: Mutex::new(Link {
                router: Router::new(reach, root.clone()),
                users: BTreeSet::new(),
                used: Instant::now(),
                owned: 0,
                closed: false,
                calls: HashMap::new(),
                tokens: HashMap::new(),
                sessionless: false,
                listens: 0,
                listening: None,
            }),
            events,
            upstream: OnceLock::new(),
            runtime: Handle::current(),
            rooted: root.is_some(),
        });
        let did_not_start = |message: String| format!("{} did not start: {message}", spec.id);
        let pipes = match Pipes::open(stdin, stdout, shared.clone()) {
            Ok(pipes) => pipes,
            Err(error) => {
                stop();
                return Err(did_not_start(error.to_string()));
            }
        };
        let client = || Client {
            shared: shared.clone(),
            info: client_info(reach),
        };
        let auto = ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        };
        let spare = pipes.spare();
        let started = tokio::time::timeout(START_TIMEOUT, async {
            match client().serve_with_lifecycle(pipes, auto).await {
                // Every server out there takes `initialize`, so one whose
                // answer to `server/discover` made no sense is asked that way.
                Err(error) if answered_oddly(&error) => {
                    let pipes = spare.again().ok_or(error)?;
                    client()
                        .serve_with_lifecycle(pipes, ClientLifecycleMode::Initialize)
                        .await
                }
                started => started,
            }
        })
        .await;
        let service = match started {
            Ok(Ok(service)) => service,
            Ok(Err(error)) => {
                stop();
                return Err(did_not_start(reason(&error)));
            }
            Err(_) => {
                stop();
                return Err(did_not_start("no answer".into()));
            }
        };
        // Answers are per caller, so none is kept for the next one.
        service
            .peer()
            .set_response_cache_config(ClientCacheConfig::disabled())
            .await;
        let sessionless = service
            .peer()
            .peer_info()
            .is_some_and(|info| !info.protocol_version.has_initialize());
        shared.linked().sessionless = sessionless;
        let upstream = Arc::new(Self {
            pid,
            started: Instant::now(),
            shared,
            service,
            stop: Mutex::new(Some(stop)),
        });
        let _ = upstream.shared.upstream.set(Arc::downgrade(&upstream));
        if sessionless
            && tokio::time::timeout(START_TIMEOUT, upstream.listen())
                .await
                .is_err()
        {
            upstream.shut();
            return Err(did_not_start(
                "it never took the notification stream".into(),
            ));
        }
        Ok(upstream)
    }

    /// On the sessionless protocol a server sends notifications only on a
    /// stream the client opens, saying what it wants. One stream carries all
    /// of them, and it's opened again whenever the resources wanted change:
    /// the new one first, so nothing falls between them.
    async fn listen(&self) {
        let (generation, filter) = {
            let mut link = self.linked();
            link.listens += 1;
            let mut filter = SubscriptionFilter::builder()
                .tools_list_changed()
                .prompts_list_changed()
                .resources_list_changed()
                .build();
            let uris = link.router.subscribed();
            if !uris.is_empty() {
                filter.resource_subscriptions = Some(uris);
            }
            (link.listens, filter)
        };
        let mut subscription = match self.peer().listen(filter).await {
            Ok(subscription) => subscription,
            Err(error) => {
                tracing::warn!(%error, "an MCP server refused its notification stream");
                return;
            }
        };
        let shared = self.shared.clone();
        let task = self.shared.runtime.spawn(async move {
            while let Ok(Some(notification)) = subscription.next().await {
                if let Ok(frame) = serde_json::to_value(&notification) {
                    shared.notified(frame);
                }
            }
        });
        let replaced = {
            let mut link = self.linked();
            match &link.listening {
                // A newer one is already in place.
                Some((installed, _)) if *installed > generation => {
                    task.abort();
                    return;
                }
                _ => link.listening.replace((generation, task.abort_handle())),
            }
        };
        // Dropping a stream's handle cancels it.
        if let Some((_, old)) = replaced {
            old.abort();
        }
    }

    fn sessionless(&self) -> bool {
        self.linked().sessionless
    }

    /// A request of the hub's own. It counts as work for as long as it takes,
    /// so the idle sweep leaves the server be.
    async fn own(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, Value> {
        {
            let mut link = self.linked();
            if link.closed {
                return Err(gone());
            }
            link.owned += 1;
        }
        let _work = Owned(&self.shared);
        let request = ClientRequest::CustomRequest(CustomRequest::new(method, Some(params)));
        let handle = self
            .peer()
            .send_request_with_option(request, PeerRequestOptions::with_timeout(timeout))
            .await
            .map_err(failure)?;
        // rmcp tells the server of a timeout itself, but not of giving up.
        let mut abandon = Abandon {
            peer: self.peer().clone(),
            id: Some(handle.id.clone()),
            runtime: self.shared.runtime.clone(),
        };
        let answer = handle.await_response().await;
        abandon.id = None;
        answer.map_err(failure).and_then(answer_of)
    }

    /// A list of `key`, across every page, for the hub's own cache.
    pub(crate) async fn list(&self, method: &str, key: &str) -> Result<Vec<Value>, Value> {
        let mut items = Vec::new();
        let mut cursor = Value::Null;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let params = if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            };
            let page = self.own(method, params, START_TIMEOUT).await?;
            items.extend(page[key].as_array().into_iter().flatten().cloned());
            cursor = page.get("nextCursor").cloned().unwrap_or(Value::Null);
            if cursor.is_null() {
                return Ok(items);
            }
            if !seen.insert(cursor.to_string()) {
                break;
            }
        }
        Err(json!({"code":-32603,"message":"The server's list never ended"}))
    }

    /// An agent's request. Its id and progress token never reach the server,
    /// and dropping the future tells the server the agent gave up.
    pub(crate) async fn request(
        &self,
        caller: Caller,
        method: &str,
        params: Value,
    ) -> Result<Value, Value> {
        self.request_within(caller, method, params, REQUEST_TIMEOUT)
            .await
    }

    async fn request_within(
        &self,
        caller: Caller,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Value> {
        let capabilities = self.capabilities(caller, &params);
        let stop = CancellationToken::new();
        let (id, mut params) = {
            let mut link = self.linked();
            if link.closed {
                return Err(gone());
            }
            link.users.insert(caller);
            link.used = Instant::now();
            let (id, mut frame) = link.router.request(caller, method, params);
            link.calls.insert(
                id,
                Call {
                    capabilities,
                    tokens: Vec::new(),
                    stop: stop.clone(),
                },
            );
            (id, frame["params"].take())
        };
        let _finished = Finished {
            shared: &self.shared,
            id,
        };
        if !params.is_object() {
            params = json!({});
        }
        params["_meta"][CALL] = id.into();
        let call = CALLER.scope(caller, self.send(method, params));
        tokio::select! {
            outcome = tokio::time::timeout(timeout, call) => outcome.unwrap_or_else(|_| Err(too_long())),
            () = stop.cancelled() => Err(gone()),
        }
    }

    /// The three requests that can need an agent's input go through rmcp's
    /// rounds of it; anything else goes as it is.
    async fn send(&self, method: &str, params: Value) -> Result<Value, Value> {
        let malformed = |error: serde_json::Error| failed(error.to_string());
        match method {
            "tools/call" => {
                let params: CallToolRequestParams =
                    serde_json::from_value(params).map_err(malformed)?;
                let result = self
                    .service
                    .call_tool_with_mrtr_max_rounds(params, MAX_ROUNDS)
                    .await;
                result.map_err(failure).and_then(answer_of)
            }
            "prompts/get" => {
                let params: GetPromptRequestParams =
                    serde_json::from_value(params).map_err(malformed)?;
                let result = self
                    .service
                    .get_prompt_with_mrtr_max_rounds(params, MAX_ROUNDS)
                    .await;
                result.map_err(failure).and_then(answer_of)
            }
            "resources/read" => {
                let params: ReadResourceRequestParams =
                    serde_json::from_value(params).map_err(malformed)?;
                let result = self
                    .service
                    .read_resource_with_mrtr_max_rounds(params, MAX_ROUNDS)
                    .await;
                result.map_err(failure).and_then(answer_of)
            }
            _ => {
                let request =
                    ClientRequest::CustomRequest(CustomRequest::new(method, Some(params)));
                let result = self.peer().send_request(request).await;
                result.map_err(failure).and_then(answer_of)
            }
        }
    }

    /// What the server is told this caller can do: what its agent said, and
    /// the folder when this process has one, which the hub answers for.
    fn capabilities(&self, caller: Caller, params: &Value) -> Value {
        let said = params["_meta"]
            .get(CAPABILITIES)
            .filter(|said| said.is_object())
            .cloned()
            .or_else(|| self.shared.events.capabilities(caller))
            .unwrap_or_else(|| json!({}));
        let mut capabilities = json!({});
        if self.shared.rooted || said.get("roots").is_some() {
            capabilities["roots"] = json!({"listChanged":false});
        }
        for key in ["sampling", "elicitation"] {
            if let Some(capability) = said.get(key) {
                capabilities[key] = capability.clone();
            }
        }
        capabilities
    }

    /// Done once the server has taken the subscription, when it's the first
    /// for this resource. One it refuses isn't held by anyone.
    pub(crate) async fn subscribe(&self, caller: Caller, uri: &str) -> Result<(), String> {
        if self.linked().router.subscribe(caller, uri).is_none() {
            return Ok(());
        }
        if self.sessionless() {
            // The stream has no answer to wait on: the server takes what it can.
            self.listen().await;
            return Ok(());
        }
        self.own("resources/subscribe", json!({"uri":uri}), START_TIMEOUT)
            .await
            .map(|_| ())
            .map_err(|error| {
                self.linked().router.refused(uri);
                error["message"]
                    .as_str()
                    .unwrap_or("The server refused the subscription")
                    .to_owned()
            })
    }

    pub(crate) async fn unsubscribe(&self, caller: Caller, uri: &str) {
        if self.linked().router.unsubscribe(caller, uri).is_none() {
            return;
        }
        if self.sessionless() {
            self.listen().await;
        } else {
            let _ = self
                .own("resources/unsubscribe", json!({"uri":uri}), START_TIMEOUT)
                .await;
        }
    }

    /// Everything a session held here goes when it does.
    pub(crate) fn forget(&self, caller: Caller) {
        let (uris, sessionless) = {
            let mut link = self.linked();
            link.users.remove(&caller);
            let frames = link.router.forget(caller);
            let mut uris = Vec::new();
            for frame in frames {
                if frame["method"] == "resources/unsubscribe" {
                    uris.extend(frame["params"]["uri"].as_str().map(str::to_owned));
                } else if let Some(call) = frame["params"]["requestId"]
                    .as_u64()
                    .and_then(|id| link.calls.get(&id))
                {
                    call.stop.cancel();
                }
            }
            (uris, link.sessionless)
        };
        let Some(upstream) = self.shared.upstream().filter(|_| !uris.is_empty()) else {
            return;
        };
        self.shared.runtime.spawn(async move {
            if sessionless {
                upstream.listen().await;
                return;
            }
            for uri in uris {
                let _ = upstream
                    .own("resources/unsubscribe", json!({"uri":uri}), START_TIMEOUT)
                    .await;
            }
        });
    }

    pub(crate) fn users(&self) -> BTreeSet<Caller> {
        self.linked().users.clone()
    }

    pub(crate) fn add_user(&self, caller: Caller) {
        self.linked().users.insert(caller);
    }

    pub(crate) fn idle_for(&self) -> Option<Duration> {
        let link = self.linked();
        (link.owned == 0 && link.router.is_idle()).then(|| link.used.elapsed())
    }

    /// Nothing is waiting on this process, so stopping it fails nobody.
    pub(crate) fn drained(&self) -> bool {
        let link = self.linked();
        link.closed || (link.owned == 0 && link.router.is_quiet())
    }

    pub(crate) fn shut(&self) {
        let stop = self.stop.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(stop) = stop {
            stop();
        }
        self.service.cancellation_token().cancel();
        self.shared.closed();
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.shut();
        if let Some((_, listening)) = self.linked().listening.take() {
            listening.abort();
        }
    }
}

/// The capabilities the hub starts a server with. Nothing a shared process
/// asks for on the older protocol could reach one agent.
fn client_info(reach: Reach) -> ClientConfig {
    let capabilities = match reach {
        Reach::Shared => json!({}),
        Reach::Workspace => json!({"roots":{"listChanged":false}}),
        Reach::Session => json!({"roots":{"listChanged":false},"sampling":{},"elicitation":{}}),
    };
    let capabilities: ClientCapabilities = serde_json::from_value(capabilities).unwrap_or_default();
    ClientConfig::new(
        capabilities,
        Implementation::new("mcp-hub", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25)
}

/// Whether a failed start got answers, just not the ones `server/discover`
/// should have, so the server may still take `initialize`.
fn answered_oddly(error: &ClientInitializeError) -> bool {
    !matches!(
        error,
        ClientInitializeError::LegacyFallbackFailed { .. }
            | ClientInitializeError::ConnectionClosed(_)
            | ClientInitializeError::TransportError { .. }
            | ClientInitializeError::Cancelled
    )
}

/// What the server said when it refused to start, rather than rmcp's summary.
fn reason(error: &ClientInitializeError) -> String {
    match error {
        ClientInitializeError::JsonRpcError(error) => error.message.to_string(),
        ClientInitializeError::LegacyFallbackFailed { fallback, .. } => reason(fallback),
        error => error.to_string(),
    }
}

/// The folder as a URI, which a plain path isn't on Windows or with a space in
/// it. `None` for a relative path, which has no URI.
fn root_uri(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// Ends an agent's call in the router however the call ended.
struct Finished<'a> {
    shared: &'a Shared,
    id: u64,
}

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        let mut link = self.shared.linked();
        link.router.finish(self.id);
        if let Some(call) = link.calls.remove(&self.id) {
            for token in call.tokens {
                link.tokens.remove(&token);
            }
        }
        link.used = Instant::now();
    }
}

struct Owned<'a>(&'a Shared);

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        let mut link = self.0.linked();
        link.owned -= 1;
        link.used = Instant::now();
    }
}

/// Tells the server a request of the hub's was given up on.
struct Abandon {
    peer: Peer<RoleClient>,
    id: Option<rmcp::model::RequestId>,
    runtime: Handle,
}

impl Drop for Abandon {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let peer = self.peer.clone();
        self.runtime.spawn(async move {
            let params = rmcp::model::CancelledNotificationParam::new(Some(id), None);
            let _ = peer.notify_cancelled(params).await;
        });
    }
}

/// What the server asks of the hub and tells it.
struct Client {
    shared: Arc<Shared>,
    info: ClientConfig,
}

impl Client {
    /// A server's question, answered by the hub or put to the agent it
    /// belongs to.
    async fn ask(
        &self,
        method: &str,
        params: Value,
        context: &RequestContext<RoleClient>,
    ) -> Result<Value, ErrorData> {
        // An input for a call on the sessionless protocol, whose agent is known.
        if let Ok(caller) = CALLER.try_with(|caller| *caller) {
            let input = self.shared.linked().router.input(method);
            return match input {
                Input::Answer(result) => Ok(result),
                Input::Refuse(message) => Err(ErrorData::internal_error(message, None)),
                Input::Ask => self.shared.put(caller, method, params).await,
            };
        }
        let id = serde_json::to_value(&context.id).unwrap_or(Value::Null);
        let frame = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let actions = self.shared.linked().router.inbound(frame);
        let mut outcome = Err(ErrorData::internal_error("Nobody answered", None));
        for action in actions {
            match action {
                Action::Reply(reply) => {
                    outcome = match reply.get("error") {
                        Some(error) => Err(ErrorData::new(
                            ErrorCode(error["code"].as_i64().unwrap_or(-32603) as i32),
                            error["message"].as_str().unwrap_or("").to_owned(),
                            None,
                        )),
                        None => Ok(reply["result"].clone()),
                    };
                }
                Action::Ask {
                    caller,
                    method,
                    params,
                    ..
                } => outcome = self.shared.put(caller, &method, params).await,
                action => self.shared.act(action),
            }
        }
        outcome
    }
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        self.info.clone()
    }

    async fn create_message(
        &self,
        params: CreateMessageRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, ErrorData> {
        let params = serde_json::to_value(params).unwrap_or_default();
        decode(self.ask("sampling/createMessage", params, &context).await?)
    }

    async fn list_roots(
        &self,
        context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, ErrorData> {
        decode(self.ask("roots/list", json!({}), &context).await?)
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        let params = serde_json::to_value(request).unwrap_or_default();
        decode(self.ask("elicitation/create", params, &context).await?)
    }

    async fn on_custom_request(
        &self,
        request: CustomRequest,
        context: RequestContext<RoleClient>,
    ) -> Result<CustomResult, ErrorData> {
        let params = request.params.unwrap_or_else(|| json!({}));
        self.ask(&request.method, params, &context)
            .await
            .map(CustomResult)
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _: NotificationContext<RoleClient>,
    ) {
        self.shared
            .notified(json!({"method":"notifications/message","params":params}));
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _: NotificationContext<RoleClient>,
    ) {
        self.shared
            .notified(json!({"method":"notifications/resources/updated","params":params}));
    }

    async fn on_resource_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.shared
            .notified(json!({"method":"notifications/resources/list_changed"}));
    }

    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.shared
            .notified(json!({"method":"notifications/tools/list_changed"}));
    }

    async fn on_prompt_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.shared
            .notified(json!({"method":"notifications/prompts/list_changed"}));
    }
}

/// The process's pipes as an rmcp transport. They're blocking, so a thread
/// moves lines each way; everything else happens on the runtime.
struct Pipes {
    lines: std::sync::mpsc::Sender<String>,
    frames: Option<Frames>,
    /// Where the frames go back to when this transport is dropped, so a start
    /// that failed can try again on the same pipes.
    spare: Spare,
    shared: Arc<Shared>,
}

type Frames = tokio::sync::mpsc::Receiver<ServerJsonRpcMessage>;

#[derive(Clone)]
struct Spare {
    lines: std::sync::mpsc::Sender<String>,
    frames: Arc<Mutex<Option<Frames>>>,
    shared: Arc<Shared>,
}

impl Spare {
    fn again(&self) -> Option<Pipes> {
        let frames = self
            .frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        Some(Pipes {
            lines: self.lines.clone(),
            frames: Some(frames),
            spare: self.clone(),
            shared: self.shared.clone(),
        })
    }
}

impl Pipes {
    fn open(
        mut stdin: Box<dyn Write + Send>,
        stdout: Box<dyn Read + Send>,
        shared: Arc<Shared>,
    ) -> io::Result<Self> {
        let (lines, outgoing) = std::sync::mpsc::channel::<String>();
        let (incoming, frames) = tokio::sync::mpsc::channel(64);
        std::thread::Builder::new()
            .name("mcp-upstream-out".into())
            .spawn(move || {
                for line in outgoing {
                    if writeln!(stdin, "{line}")
                        .and_then(|()| stdin.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("mcp-upstream-in".into())
            .spawn(move || {
                let mut lines = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match (&mut lines).take(MAX_LINE_BYTES).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    // Some servers print banners on stdout.
                    let Ok(mut frame) = serde_json::from_str::<Value>(line.trim()) else {
                        continue;
                    };
                    // And some leave out the version, which says nothing.
                    if let Some(fields) = frame.as_object_mut() {
                        fields.entry("jsonrpc").or_insert_with(|| "2.0".into());
                    }
                    let Ok(frame) = serde_json::from_value::<ServerJsonRpcMessage>(frame) else {
                        continue;
                    };
                    if incoming.blocking_send(frame).is_err() {
                        break;
                    }
                }
            })?;
        let spare = Spare {
            lines: lines.clone(),
            frames: Arc::default(),
            shared: shared.clone(),
        };
        Ok(Self {
            lines,
            frames: Some(frames),
            spare,
            shared,
        })
    }

    fn spare(&self) -> Spare {
        self.spare.clone()
    }
}

impl Drop for Pipes {
    fn drop(&mut self) {
        if let Some(frames) = self.frames.take() {
            *self.spare.frames.lock().unwrap_or_else(|e| e.into_inner()) = Some(frames);
        }
    }
}

impl Transport<RoleClient> for Pipes {
    type Error = io::Error;

    fn send(
        &mut self,
        item: ClientJsonRpcMessage,
    ) -> impl Future<Output = Result<(), io::Error>> + Send + 'static {
        let sent = serde_json::to_value(&item)
            .map_err(io::Error::other)
            .and_then(|mut frame| {
                self.shared.stamp(&mut frame);
                self.lines
                    .send(frame.to_string())
                    .map_err(|_| io::ErrorKind::BrokenPipe.into())
            });
        std::future::ready(sent)
    }

    async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
        loop {
            let frame = match self.frames.as_mut() {
                Some(frames) => frames.recv().await,
                None => None,
            };
            // rmcp hands notifications to tasks of their own, so progress
            // is passed on here, before the answer it came ahead of.
            if let Some(ServerJsonRpcMessage::Notification(notification)) = &frame {
                if let ServerNotification::ProgressNotification(progress) =
                    &notification.notification
                {
                    self.shared.progressed(progress.params.clone());
                    continue;
                }
            }
            if frame.is_none() {
                self.shared.exited();
            }
            return frame;
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), io::Error>> + Send {
        std::future::ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sharing;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicBool, Ordering},
    };

    const SHORT: Duration = Duration::from_millis(60);

    /// Keeps what the upstream reports, and says what each caller's agent can do.
    #[derive(Default)]
    struct Kept {
        actions: Mutex<Vec<Action>>,
        capabilities: BTreeMap<Caller, Value>,
    }

    impl Events for Kept {
        fn action(&self, _: &Arc<Upstream>, action: Action) {
            self.actions.lock().unwrap().push(action);
        }
        fn exited(&self, _: &Arc<Upstream>) {}
        fn ask(&self, _: Caller, _: String, _: Value, answer: Answer) {
            answer(Err("nobody to ask".into()));
        }
        fn capabilities(&self, caller: Caller) -> Option<Value> {
            self.capabilities.get(&caller).cloned()
        }
    }

    struct Server {
        upstream: Arc<Upstream>,
        frames: Arc<Mutex<Vec<Value>>>,
        stopped: Arc<AtomicBool>,
        events: Arc<Kept>,
    }

    /// A server on real pipes, speaking as it's told; it leaves `silent` methods unanswered
    /// and hands the rest to `answer`, which may also say what to send first.
    /// Every frame it reads is kept.
    #[derive(Clone, Copy, PartialEq)]
    enum Speaks {
        /// Refuses `server/discover`.
        Old,
        /// Answers `server/discover` with whatever it answers everything.
        Anything,
        /// Refuses `initialize`.
        New,
    }

    async fn server_with(
        speaks: Speaks,
        silent: &'static [&'static str],
        events: Kept,
        answer: impl Fn(&Value) -> (Vec<Value>, Value) + Send + 'static,
    ) -> Server {
        let (to_server, written) = std::io::pipe().map(|(read, write)| (write, read)).unwrap();
        let (read_back, from_server) = std::io::pipe().unwrap();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let kept = frames.clone();
        std::thread::spawn(move || {
            let mut output = from_server;
            let mut send = |frame: Value| writeln!(output, "{frame}").unwrap();
            for line in BufReader::new(written).lines().map_while(Result::ok) {
                let frame: Value = serde_json::from_str(&line).unwrap();
                kept.lock().unwrap().push(frame.clone());
                let method = frame["method"].as_str().unwrap_or("");
                if frame.get("id").is_none() || silent.contains(&method) {
                    continue;
                }
                let reply = match method {
                    "server/discover" if speaks == Speaks::New => json!({"result":{
                        "resultType":"complete",
                        "supportedVersions":["2026-07-28"],
                        "capabilities":{"tools":{}},
                        "ttlMs":0,
                        "cacheScope":"private",
                        "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"test","version":"1"}},
                    }}),
                    "server/discover" if speaks == Speaks::Old => {
                        json!({"error":{"code":-32601,"message":"no"}})
                    }
                    "initialize" if speaks == Speaks::New => {
                        json!({"error":{"code":-32022,"message":"no"}})
                    }
                    "initialize" => {
                        json!({"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"test","version":"1"}}})
                    }
                    "subscriptions/listen" => {
                        send(
                            json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{
                                "_meta":{"io.modelcontextprotocol/subscriptionId":frame["id"]},
                                "notifications":frame["params"]["notifications"],
                            }}),
                        );
                        continue;
                    }
                    _ => {
                        let (first, reply) = answer(&frame);
                        first.into_iter().for_each(&mut send);
                        reply
                    }
                };
                let mut reply = reply;
                reply["jsonrpc"] = "2.0".into();
                reply["id"] = frame["id"].clone();
                send(reply);
            }
        });
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let spawned = Spawned {
            pid: 1,
            stdin: Box::new(to_server),
            stdout: Box::new(read_back),
            stop: Box::new(move || stop.store(true, Ordering::SeqCst)),
        };
        let spec = ServerSpec {
            id: "test".into(),
            command: "test".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            sharing: Sharing::PerSession,
            tools: None,
        };
        let events = Arc::new(events);
        let upstream = Upstream::start(&spec, spawned, None, Reach::Session, events.clone())
            .await
            .unwrap();
        Server {
            upstream,
            frames,
            stopped,
            events,
        }
    }

    async fn server(
        silent: &'static [&'static str],
        answer: impl Fn(&Value) -> Value + Send + 'static,
    ) -> Server {
        server_with(Speaks::Old, silent, Kept::default(), move |frame| {
            (Vec::new(), answer(frame))
        })
        .await
    }

    fn sent(frames: &Mutex<Vec<Value>>, method: &str) -> Vec<Value> {
        let frames = frames.lock().unwrap();
        frames
            .iter()
            .filter(|frame| frame["method"] == method)
            .cloned()
            .collect()
    }

    async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn cancelled_ids(frames: &Mutex<Vec<Value>>) -> Vec<Value> {
        sent(frames, "notifications/cancelled")
            .iter()
            .map(|frame| frame["params"]["requestId"].clone())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_request_nobody_answers_is_cancelled_and_forgotten() {
        let server = server(&["tools/list"], |_| json!({"result":{}})).await;
        let answer = server.upstream.own("tools/list", json!({}), SHORT).await;
        assert_eq!(answer.unwrap_err()["code"], -32001);
        let id = sent(&server.frames, "tools/list")[0]["id"].clone();
        wait_for("the cancel", || !cancelled_ids(&server.frames).is_empty()).await;
        assert_eq!(cancelled_ids(&server.frames), [id]);
        assert!(
            server.upstream.idle_for().is_some(),
            "it no longer counts as work"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_request_given_up_on_is_cancelled_too() {
        let server = server(&["tools/list"], |_| json!({"result":{}})).await;
        let listing = server
            .upstream
            .own("tools/list", json!({}), REQUEST_TIMEOUT);
        assert!(tokio::time::timeout(SHORT, listing).await.is_err());
        wait_for("the cancel", || !cancelled_ids(&server.frames).is_empty()).await;
        assert!(server.upstream.idle_for().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_request_in_flight_keeps_the_server_from_looking_idle() {
        let server = server(&["tools/list"], |_| json!({"result":{}})).await;
        assert!(server.upstream.idle_for().is_some());
        let upstream = server.upstream.clone();
        let listing = tokio::spawn(async move {
            let _ = upstream
                .own("tools/list", json!({}), Duration::from_millis(300))
                .await;
        });
        wait_for("the request", || server.upstream.linked().owned == 1).await;
        assert!(server.upstream.idle_for().is_none());
        assert!(!server.upstream.drained());
        listing.await.unwrap();
        assert!(server.upstream.idle_for().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_request_that_times_out_is_removed_and_cancelled() {
        let server = server(&["tools/call"], |_| json!({"result":{}})).await;
        let answer = server
            .upstream
            .request_within(1, "tools/call", json!({"name":"a"}), SHORT)
            .await;
        assert_eq!(answer.unwrap_err()["code"], -32001);
        assert!(server.upstream.linked().calls.is_empty());
        wait_for("the cancel", || cancelled_ids(&server.frames).len() == 1).await;
        assert!(server.upstream.idle_for().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_that_gives_up_cancels_its_request() {
        let server = server(&["tools/call"], |_| json!({"result":{}})).await;
        let call = server
            .upstream
            .request(1, "tools/call", json!({"name":"a"}));
        assert!(tokio::time::timeout(SHORT, call).await.is_err());
        assert!(server.upstream.linked().calls.is_empty());
        assert!(server.upstream.linked().tokens.is_empty());
        wait_for("the cancel", || cancelled_ids(&server.frames).len() == 1).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_that_ends_ends_its_calls_in_flight() {
        let server = server(&["tools/call"], |_| json!({"result":{}})).await;
        let upstream = server.upstream.clone();
        let call =
            tokio::spawn(
                async move { upstream.request(1, "tools/call", json!({"name":"a"})).await },
            );
        wait_for("the call", || {
            !sent(&server.frames, "tools/call").is_empty()
        })
        .await;
        server.upstream.forget(1);
        let answer = tokio::time::timeout(Duration::from_secs(5), call).await;
        assert!(answer.unwrap().unwrap().is_err());
        wait_for("the cancel", || cancelled_ids(&server.frames).len() == 1).await;
        assert!(server.upstream.users().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_that_takes_initialize_never_hears_of_the_new_protocol() {
        let server = server(&[], |_| json!({"result":{"tools":[]}})).await;
        server.upstream.list("tools/list", "tools").await.unwrap();
        server.upstream.subscribe(1, "file:///r").await.unwrap();
        server
            .upstream
            .request(1, "tools/call", json!({"name":"a"}))
            .await
            .unwrap_err();
        let frames = server.frames.lock().unwrap();
        let methods: Vec<_> = frames.iter().map(|frame| frame["method"].clone()).collect();
        assert_eq!(
            methods,
            [
                // Asked first, and refused.
                "server/discover",
                "initialize",
                "notifications/initialized",
                "tools/list",
                "resources/subscribe",
                "tools/call",
            ]
        );
        for frame in &frames[3..] {
            let meta = frame["params"]["_meta"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            assert!(meta.keys().all(|key| key == "progressToken"), "{frame}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_whose_discover_answer_makes_no_sense_is_initialized_instead() {
        let server = server_with(Speaks::Anything, &[], Kept::default(), |_| {
            (Vec::new(), json!({"result":{"tools":[{"name":"a"}]}}))
        })
        .await;
        assert_eq!(
            server
                .upstream
                .list("tools/list", "tools")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(!server.upstream.sessionless());
        assert_eq!(sent(&server.frames, "initialize").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_page_of_a_list_is_read() {
        let server = server(&[], |frame| {
            let next = (frame["params"]["cursor"].is_null()).then_some("2");
            json!({"result":{"tools":[{"name":"a"}],"nextCursor":next}})
        })
        .await;
        let tools = server.upstream.list("tools/list", "tools").await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(sent(&server.frames, "tools/list").len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_list_that_repeats_a_page_or_never_ends_fails() {
        let repeating = server(&[], |_| json!({"result":{"nextCursor":"same"}})).await;
        assert!(repeating
            .upstream
            .list("tools/list", "tools")
            .await
            .is_err());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let endless = server(&[], move |_| {
            let page = next.fetch_add(1, Ordering::SeqCst);
            json!({"result":{"nextCursor":page.to_string()}})
        })
        .await;
        assert!(endless.upstream.list("tools/list", "tools").await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_subscription_counts_only_once_the_server_takes_it() {
        let server = server(&[], |frame| {
            if frame["params"]["uri"] == "file:///refused" {
                json!({"error":{"code":-32602,"message":"No such resource"}})
            } else {
                json!({"result":{}})
            }
        })
        .await;
        let upstream = &server.upstream;
        assert_eq!(
            upstream.subscribe(1, "file:///refused").await.unwrap_err(),
            "No such resource"
        );
        assert!(upstream.idle_for().is_some(), "nobody holds it");
        upstream.subscribe(1, "file:///ok").await.unwrap();
        assert!(upstream.idle_for().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_runs_once_and_ends_whatever_was_waiting() {
        let server = server(&["tools/call"], |_| json!({"result":{}})).await;
        let upstream = server.upstream.clone();
        let call =
            tokio::spawn(
                async move { upstream.request(1, "tools/call", json!({"name":"a"})).await },
            );
        wait_for("the call", || {
            !sent(&server.frames, "tools/call").is_empty()
        })
        .await;
        server.upstream.shut();
        assert!(server.stopped.load(Ordering::SeqCst));
        let answer = tokio::time::timeout(Duration::from_secs(5), call).await;
        assert_eq!(answer.unwrap().unwrap(), Err(gone()));
        assert!(server.upstream.drained());
        let listed = server.upstream.own("tools/list", json!({}), SHORT).await;
        assert_eq!(listed, Err(gone()));
    }

    /// Two agents number their progress alike; rmcp numbers it again for the
    /// wire, and each agent hears of its own call with its own token.
    #[tokio::test(flavor = "multi_thread")]
    async fn progress_goes_back_to_the_call_it_was_for_with_its_token() {
        let server = server_with(Speaks::Old, &[], Kept::default(), |frame| {
            let token = frame["params"]["_meta"]["progressToken"].clone();
            let echo = frame["params"]["arguments"]["echo"].to_string();
            let progress = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{
                "progressToken":token,"progress":1,"message":echo,
            }});
            (vec![progress], json!({"result":{"content":[]}}))
        })
        .await;
        for caller in [1, 2] {
            let params =
                json!({"name":"a","arguments":{"echo":caller},"_meta":{"progressToken":7}});
            server
                .upstream
                .request(caller, "tools/call", params)
                .await
                .unwrap();
        }
        wait_for("both progress notes", || {
            server.events.actions.lock().unwrap().len() == 2
        })
        .await;
        let mut heard: Vec<_> = server
            .events
            .actions
            .lock()
            .unwrap()
            .iter()
            .map(|action| match action {
                Action::Notify { caller, message } => (
                    *caller,
                    message["params"]["progressToken"].clone(),
                    message["params"]["message"].clone(),
                ),
                action => panic!("{action:?}"),
            })
            .collect();
        heard.sort_by_key(|(caller, ..)| *caller);
        assert_eq!(
            heard,
            [(1, json!(7), json!("1")), (2, json!(7), json!("2"))]
        );
        let calls = sent(&server.frames, "tools/call");
        assert_ne!(
            calls[0]["params"]["_meta"]["progressToken"],
            calls[1]["params"]["_meta"]["progressToken"]
        );
        assert!(
            server.upstream.linked().tokens.is_empty(),
            "forgotten once done"
        );
    }

    /// On the sessionless protocol each request says who's asking, so each
    /// says what its own agent can do, not what every agent can.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_sessionless_server_is_told_what_each_callers_agent_can_do() {
        let events = Kept {
            capabilities: BTreeMap::from([
                (
                    1,
                    json!({"elicitation":{"form":{}},"experimental":{"x":{}}}),
                ),
                (2, json!({"sampling":{}})),
            ]),
            ..Kept::default()
        };
        let server = server_with(Speaks::New, &[], events, |_| {
            (
                Vec::new(),
                json!({"result":{"resultType":"complete","content":[]}}),
            )
        })
        .await;
        for caller in [1, 2, 3] {
            server
                .upstream
                .request(caller, "tools/call", json!({"name":"a"}))
                .await
                .unwrap();
        }
        // An agent on the sessionless protocol says it with its own request.
        let said = json!({"name":"a","_meta":{CAPABILITIES:{"roots":{}}}});
        server
            .upstream
            .request(4, "tools/call", said)
            .await
            .unwrap();
        let told: Vec<_> = sent(&server.frames, "tools/call")
            .iter()
            .map(|frame| {
                let meta = &frame["params"]["_meta"];
                assert!(
                    meta.get(CALL).is_none(),
                    "the hub's mark stays with the hub"
                );
                assert_eq!(meta[VERSION], "2026-07-28");
                meta[CAPABILITIES].clone()
            })
            .collect();
        assert_eq!(
            told,
            [
                json!({"elicitation":{"form":{}}}),
                json!({"sampling":{}}),
                json!({}),
                json!({"roots":{"listChanged":false}}),
            ]
        );
        let discover = &sent(&server.frames, "server/discover")[0];
        assert_eq!(
            discover["params"]["_meta"][CAPABILITIES],
            json!({"roots":{"listChanged":false},"sampling":{},"elicitation":{}}),
            "starting it, the hub says what its class allows"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sessionless_server_gets_one_stream_that_follows_the_subscriptions() {
        let server = server_with(Speaks::New, &[], Kept::default(), |_| {
            (Vec::new(), json!({"result":{}}))
        })
        .await;
        let upstream = &server.upstream;
        upstream.subscribe(1, "file:///a").await.unwrap();
        upstream.subscribe(2, "file:///a").await.unwrap();
        upstream.unsubscribe(1, "file:///a").await;
        upstream.unsubscribe(2, "file:///a").await;
        let listens = sent(&server.frames, "subscriptions/listen");
        let wanted: Vec<_> = listens
            .iter()
            .map(|frame| frame["params"]["notifications"]["resourceSubscriptions"].clone())
            .collect();
        assert_eq!(wanted, [Value::Null, json!(["file:///a"]), Value::Null]);
        // Each stream but the last was ended after the next one opened.
        let ids: Vec<_> = listens.iter().map(|frame| frame["id"].clone()).collect();
        wait_for("the old streams to end", || {
            cancelled_ids(&server.frames).len() == 2
        })
        .await;
        let mut ended = cancelled_ids(&server.frames);
        ended.sort_by_key(|id| id.as_u64());
        assert_eq!(ended, ids[..2]);
        assert!(sent(&server.frames, "resources/subscribe").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_root_is_a_proper_file_uri() {
        #[cfg(unix)]
        {
            assert_eq!(
                root_uri(Path::new("/work/my folder#1")).unwrap(),
                "file:///work/my%20folder%231"
            );
            assert_eq!(root_uri(Path::new("relative")), None);
        }
        #[cfg(windows)]
        assert_eq!(
            root_uri(Path::new(r"C:\work\my folder")).unwrap(),
            "file:///C:/work/my%20folder"
        );
    }
}
