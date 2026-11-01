//! One running MCP server, spoken to over stdio. The hub initializes it once
//! with its own capabilities, whoever asked for it first.
use crate::{
    router::{cancelled, Action, Caller, Reach, Router},
    ServerSpec,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
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
/// More pages than any real list, so a server can't keep one going forever.
const MAX_PAGES: usize = 100;

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
    /// Requests the hub is waiting on for itself.
    owned: usize,
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
        let root = workspace.and_then(root_uri);
        let upstream = Arc::new(Self {
            pid,
            started: Instant::now(),
            link: Mutex::new(Link {
                router: Router::new(reach, root),
                replies: HashMap::new(),
                users: BTreeSet::new(),
                used: Instant::now(),
                owned: 0,
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
        let (id, frame) = self.linked().router.own_request(method, params);
        self.exchange(id, &frame, timeout)
    }

    /// Sends a request of the hub's and waits for its answer. It counts as
    /// work for as long as it takes, so the idle sweep leaves the server be.
    fn exchange(&self, id: u64, frame: &Value, timeout: Duration) -> Result<Value, Value> {
        let (send, receive) = oneshot::channel();
        {
            let mut link = self.linked();
            if link.closed {
                return Err(gone());
            }
            link.replies.insert(id, send);
            link.owned += 1;
        }
        let outcome = if self.write(frame) {
            wait(receive, timeout)
        } else {
            Err(gone())
        };
        let unanswered = {
            let mut link = self.linked();
            link.owned -= 1;
            link.used = Instant::now();
            link.replies.remove(&id).is_some()
        };
        if unanswered {
            self.write(&cancelled(id));
        }
        outcome
    }

    /// A list of `key`, across every page, for the hub's own cache.
    pub(crate) fn list(&self, method: &str, key: &str) -> Result<Vec<Value>, Value> {
        let mut items = Vec::new();
        let mut cursor = Value::Null;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
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
        let outcome = tokio::time::timeout(timeout, receive).await;
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

    /// Blocks until the server has taken the subscription, when it's the first
    /// for this resource. One it refuses isn't held by anyone.
    pub(crate) fn subscribe(&self, caller: Caller, uri: &str) -> Result<(), String> {
        let frame = self.linked().router.subscribe(caller, uri);
        let Some((id, frame)) = frame.and_then(|frame| Some((frame["id"].as_u64()?, frame))) else {
            return Ok(());
        };
        self.exchange(id, &frame, START_TIMEOUT)
            .map(|_| ())
            .map_err(|error| {
                self.linked().router.refused(uri);
                error["message"]
                    .as_str()
                    .unwrap_or("The server refused the subscription")
                    .to_owned()
            })
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
        self.closed();
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.shut();
    }
}

/// The folder as a URI, which a plain path isn't on Windows or with a space in
/// it. `None` for a relative path, which has no URI.
fn root_uri(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

fn wait(
    mut receive: oneshot::Receiver<Result<Value, Value>>,
    timeout: Duration,
) -> Result<Value, Value> {
    let deadline = Instant::now() + timeout;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sharing;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicBool, Ordering},
    };

    const SHORT: Duration = Duration::from_millis(60);

    struct NoEvents;
    impl Events for NoEvents {
        fn action(&self, _: &Arc<Upstream>, _: Action) {}
        fn exited(&self, _: &Arc<Upstream>) {}
    }

    /// A server on real pipes. It answers `initialize`, leaves `silent` methods
    /// unanswered and hands the rest to `answer`; every frame it reads is kept.
    fn server(
        silent: &'static [&'static str],
        answer: impl Fn(&Value) -> Value + Send + 'static,
    ) -> (Arc<Upstream>, Arc<Mutex<Vec<Value>>>, Arc<AtomicBool>) {
        let (to_server, written) = std::io::pipe().map(|(read, write)| (write, read)).unwrap();
        let (read_back, from_server) = std::io::pipe().unwrap();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let kept = frames.clone();
        std::thread::spawn(move || {
            let mut output = from_server;
            for line in BufReader::new(written).lines().map_while(Result::ok) {
                let frame: Value = serde_json::from_str(&line).unwrap();
                kept.lock().unwrap().push(frame.clone());
                let method = frame["method"].as_str().unwrap_or("");
                if frame.get("id").is_none() || silent.contains(&method) {
                    continue;
                }
                let reply = if method == "initialize" {
                    json!({"jsonrpc":"2.0","id":frame["id"],"result":{"protocolVersion":PROTOCOL_VERSION}})
                } else {
                    let mut reply = answer(&frame);
                    reply["jsonrpc"] = "2.0".into();
                    reply["id"] = frame["id"].clone();
                    reply
                };
                writeln!(output, "{reply}").unwrap();
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
        let upstream =
            Upstream::start(&spec, spawned, None, Reach::Session, Arc::new(NoEvents)).unwrap();
        (upstream, frames, stopped)
    }

    fn cancelled_ids(frames: &Mutex<Vec<Value>>) -> Vec<Value> {
        frames
            .lock()
            .unwrap()
            .iter()
            .filter(|frame| frame["method"] == "notifications/cancelled")
            .map(|frame| frame["params"]["requestId"].clone())
            .collect()
    }

    fn sent(frames: &Mutex<Vec<Value>>, method: &str) -> Value {
        let frames = frames.lock().unwrap();
        let frame = frames.iter().find(|frame| frame["method"] == method);
        frame.unwrap_or_else(|| panic!("no {method}")).clone()
    }

    fn wait_for_cancel(frames: &Mutex<Vec<Value>>) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while cancelled_ids(frames).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        cancelled_ids(frames)
    }

    #[test]
    fn a_hub_request_nobody_answers_is_cancelled_and_forgotten() {
        let (upstream, frames, _) = server(&["tools/list"], |_| json!({"result":{}}));
        let answer = upstream.own("tools/list", json!({}), SHORT);
        assert_eq!(answer.unwrap_err()["message"], "timed out");
        assert!(upstream.linked().replies.is_empty());
        let id = sent(&frames, "tools/list")["id"].clone();
        assert_eq!(wait_for_cancel(&frames), [id]);
        assert!(upstream.idle_for().is_some(), "it no longer counts as work");
    }

    #[test]
    fn a_hub_request_in_flight_keeps_the_server_from_looking_idle() {
        let (upstream, _, _) = server(&["tools/list"], |_| json!({"result":{}}));
        assert!(upstream.idle_for().is_some());
        let listing = upstream.clone();
        let worker = std::thread::spawn(move || {
            let _ = listing.own("tools/list", json!({}), Duration::from_millis(300));
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while upstream.linked().owned == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(upstream.idle_for().is_none());
        assert!(!upstream.drained());
        worker.join().unwrap();
        assert!(upstream.idle_for().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_request_that_times_out_is_removed_and_cancelled() {
        let (upstream, frames, _) = server(&["tools/call"], |_| json!({"result":{}}));
        let answer = upstream
            .request_within(1, "tools/call", json!({}), SHORT)
            .await;
        assert_eq!(answer.unwrap_err()["code"], -32001);
        assert!(upstream.linked().replies.is_empty());
        assert_eq!(wait_for_cancel(&frames).len(), 1);
        assert!(upstream.idle_for().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_that_gives_up_cancels_its_request() {
        let (upstream, frames, _) = server(&["tools/call"], |_| json!({"result":{}}));
        let call = upstream.request(1, "tools/call", json!({}));
        assert!(tokio::time::timeout(SHORT, call).await.is_err());
        assert!(upstream.linked().replies.is_empty());
        assert_eq!(wait_for_cancel(&frames).len(), 1);
    }

    #[test]
    fn every_page_of_a_list_is_read() {
        let (upstream, frames, _) = server(&[], |frame| {
            let next = (frame["params"]["cursor"].is_null()).then_some("2");
            json!({"result":{"tools":[{"name":"a"}],"nextCursor":next}})
        });
        assert_eq!(upstream.list("tools/list", "tools").unwrap().len(), 2);
        assert_eq!(
            frames.lock().unwrap().len(),
            4,
            "initialize, ready, 2 pages"
        );
    }

    #[test]
    fn a_list_that_repeats_a_page_or_never_ends_fails() {
        let (upstream, _, _) = server(&[], |_| json!({"result":{"nextCursor":"same"}}));
        assert!(upstream.list("tools/list", "tools").is_err());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let (upstream, _, _) = server(&[], move |_| {
            let page = next.fetch_add(1, Ordering::SeqCst);
            json!({"result":{"nextCursor":page.to_string()}})
        });
        assert!(upstream.list("tools/list", "tools").is_err());
    }

    #[test]
    fn a_subscription_counts_only_once_the_server_takes_it() {
        let (upstream, _, _) = server(&[], |frame| {
            if frame["params"]["uri"] == "file:///refused" {
                json!({"error":{"code":-32602,"message":"No such resource"}})
            } else {
                json!({"result":{}})
            }
        });
        assert_eq!(
            upstream.subscribe(1, "file:///refused").unwrap_err(),
            "No such resource"
        );
        assert!(upstream.idle_for().is_some(), "nobody holds it");
        upstream.subscribe(1, "file:///ok").unwrap();
        assert!(upstream.idle_for().is_none());
    }

    #[test]
    fn stopping_runs_once_and_ends_whatever_was_waiting() {
        let (upstream, _, stopped) = server(&[], |_| json!({"result":{}}));
        upstream.shut();
        assert!(stopped.load(Ordering::SeqCst));
        assert!(upstream.drained());
        assert!(upstream.own("tools/list", json!({}), SHORT).is_err());
    }

    #[test]
    fn a_root_is_a_proper_file_uri() {
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
