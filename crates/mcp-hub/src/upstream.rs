//! One running MCP server, spoken to over stdio. The hub initializes it once
//! with its own capabilities, whoever asked for it first.
use crate::{
    router::{Action, Caller, Reach, Router},
    ServerSpec,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-11-25";
/// A server that doesn't answer in this long is treated as failed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const START_TIMEOUT: Duration = Duration::from_secs(60);

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

/// What an upstream reports to the pool.
pub(crate) trait Events: Send + Sync + 'static {
    fn action(&self, upstream: &Arc<Upstream>, action: Action);
    fn exited(&self, upstream: &Arc<Upstream>);
}

type Reply = oneshot::Sender<Result<Value, Value>>;

struct Link {
    router: Router,
    replies: HashMap<u64, Reply>,
    /// Sessions that have used this process.
    users: BTreeSet<Caller>,
    used: Instant,
    closed: bool,
}

pub(crate) struct Upstream {
    pub(crate) pid: u32,
    pub(crate) started: Instant,
    link: Mutex<Link>,
    input: Mutex<Box<dyn Write + Send>>,
    stop: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

fn gone() -> Value {
    json!({"code":-32000,"message":"The MCP server stopped"})
}

impl Upstream {
    fn linked(&self) -> std::sync::MutexGuard<'_, Link> {
        self.link.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self, frame: &Value) -> bool {
        let mut input = self.input.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(input, "{frame}")
            .and_then(|()| input.flush())
            .is_ok()
    }

    /// Starts reading, then initializes. Blocks until the server is ready.
    pub(crate) fn start(
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
        let root = workspace.map(|path| path.to_string_lossy().into_owned());
        let upstream = Arc::new(Self {
            pid,
            started: Instant::now(),
            link: Mutex::new(Link {
                router: Router::new(reach, root),
                replies: HashMap::new(),
                users: BTreeSet::new(),
                used: Instant::now(),
                closed: false,
            }),
            input: Mutex::new(stdin),
            stop: Mutex::new(Some(stop)),
        });
        let reader = upstream.clone();
        std::thread::Builder::new()
            .name("mcp-upstream".into())
            .spawn(move || reader.read(stdout, events))
            .map_err(|e| e.to_string())?;
        let capabilities = match reach {
            // Nothing it asks a shared process for could reach one agent.
            Reach::Shared => json!({}),
            Reach::Workspace => json!({"roots":{"listChanged":false}}),
            Reach::Session => {
                json!({"roots":{"listChanged":false},"sampling":{},"elicitation":{}})
            }
        };
        let answer = upstream.own(
            "initialize",
            json!({
                "protocolVersion":PROTOCOL_VERSION,
                "capabilities":capabilities,
                "clientInfo":{"name":"mcp-hub","version":env!("CARGO_PKG_VERSION")},
            }),
            START_TIMEOUT,
        );
        match answer {
            Ok(_) => {
                upstream.write(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
                Ok(upstream)
            }
            Err(error) => {
                upstream.shut();
                Err(format!(
                    "{} did not start: {}",
                    spec.id,
                    error["message"].as_str().unwrap_or("no answer")
                ))
            }
        }
    }

    fn read(self: Arc<Self>, stdout: Box<dyn Read + Send>, events: Arc<dyn Events>) {
        let mut lines = BufReader::new(stdout.take(u64::MAX));
        let mut line = String::new();
        loop {
            line.clear();
            match (&mut lines).take(MAX_LINE_BYTES).read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // Some servers print banners on stdout.
            let Ok(frame) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            let actions = self.linked().router.inbound(frame);
            for action in actions {
                match action {
                    Action::Reply(reply) => {
                        self.write(&reply);
                    }
                    Action::Answered { id, outcome } => {
                        if let Some(reply) = self.linked().replies.remove(&id) {
                            let _ = reply.send(outcome);
                        }
                    }
                    action => events.action(&self, action),
                }
            }
        }
        self.closed();
        events.exited(&self);
    }

    fn closed(&self) {
        let mut link = self.linked();
        link.closed = true;
        for (_, reply) in link.replies.drain() {
            let _ = reply.send(Err(gone()));
        }
    }

    fn own(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, Value> {
        let (send, receive) = oneshot::channel();
        let frame = {
            let mut link = self.linked();
            let (id, frame) = link.router.own_request(method, params);
            link.replies.insert(id, send);
            frame
        };
        if !self.write(&frame) {
            return Err(gone());
        }
        let deadline = Instant::now() + timeout;
        let mut receive = receive;
        loop {
            match receive.try_recv() {
                Ok(outcome) => return outcome,
                Err(oneshot::error::TryRecvError::Closed) => return Err(gone()),
                Err(oneshot::error::TryRecvError::Empty) if Instant::now() >= deadline => {
                    return Err(json!({"code":-32001,"message":"timed out"}));
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// A list of `key`, across every page, for the hub's own cache.
    pub(crate) fn list(&self, method: &str, key: &str) -> Result<Vec<Value>, Value> {
        let mut items = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let params = if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            };
            let page = self.own(method, params, START_TIMEOUT)?;
            items.extend(page[key].as_array().into_iter().flatten().cloned());
            cursor = page.get("nextCursor").cloned().unwrap_or(Value::Null);
            if cursor.is_null() {
                return Ok(items);
            }
        }
    }

    /// An agent's request. Its id and progress token never reach the server,
    /// and dropping the future tells the server the agent gave up.
    pub(crate) async fn request(
        &self,
        caller: Caller,
        method: &str,
        params: Value,
    ) -> Result<Value, Value> {
        let (send, receive) = oneshot::channel();
        let (id, frame) = {
            let mut link = self.linked();
            if link.closed {
                return Err(gone());
            }
            link.users.insert(caller);
            link.used = Instant::now();
            let (id, frame) = link.router.request(caller, method, params);
            link.replies.insert(id, send);
            (id, frame)
        };
        let mut guard = Abandon {
            upstream: self,
            id: Some(id),
        };
        if !self.write(&frame) {
            return Err(gone());
        }
        let outcome = tokio::time::timeout(REQUEST_TIMEOUT, receive).await;
        guard.id = None;
        self.linked().used = Instant::now();
        match outcome {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(gone()),
            Err(_) => {
                self.abandon(id);
                Err(json!({"code":-32001,"message":"The MCP server took too long"}))
            }
        }
    }

    fn abandon(&self, id: u64) {
        let frame = {
            let mut link = self.linked();
            link.replies.remove(&id);
            link.router.cancel(id)
        };
        if let Some(frame) = frame {
            self.write(&frame);
        }
    }

    /// Answers a request the server sent an agent.
    pub(crate) fn respond(&self, id: Value, outcome: Result<Value, String>) {
        let frame = match outcome {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(message) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":message}})
            }
        };
        self.write(&frame);
    }

    pub(crate) fn subscribe(&self, caller: Caller, uri: &str) {
        let frame = self.linked().router.subscribe(caller, uri);
        if let Some(frame) = frame {
            self.write(&frame);
        }
    }

    pub(crate) fn unsubscribe(&self, caller: Caller, uri: &str) {
        let frame = self.linked().router.unsubscribe(caller, uri);
        if let Some(frame) = frame {
            self.write(&frame);
        }
    }

    pub(crate) fn forget(&self, caller: Caller) {
        let frames = {
            let mut link = self.linked();
            link.users.remove(&caller);
            link.router.forget(caller)
        };
        for frame in frames {
            self.write(&frame);
        }
    }

    pub(crate) fn users(&self) -> BTreeSet<Caller> {
        self.linked().users.clone()
    }

    pub(crate) fn add_user(&self, caller: Caller) {
        self.linked().users.insert(caller);
    }

    pub(crate) fn idle_for(&self) -> Option<Duration> {
        let link = self.linked();
        link.router.is_idle().then(|| link.used.elapsed())
    }

    pub(crate) fn shut(&self) {
        let stop = self.stop.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(stop) = stop {
            stop();
        }
        self.closed();
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.shut();
    }
}

struct Abandon<'a> {
    upstream: &'a Upstream,
    id: Option<u64>,
}

impl Drop for Abandon<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.upstream.abandon(id);
        }
    }
}
