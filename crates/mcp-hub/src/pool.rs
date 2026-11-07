//! The server processes behind the hub. Each is started the first time a call
//! needs it, shared as far as its class allows, and stopped when idle. Agents
//! list tools from a cache, so listing never starts anything that has run once.
use crate::{
    endpoint::Inner,
    router::{Action, Caller, Reach},
    upstream::{Answer, Events, Upstream},
    Grant, Spawner,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

/// The separator between a server and its tool: `github__create_issue`.
pub(crate) const SEPARATOR: &str = "__";
/// Gemini caps tool names at 63 characters and Kiro at 64.
const MAX_TOOL_NAME: usize = 63;
const REMEMBERED_ERRORS: usize = 32;
/// A server chooses what its errors say, so they're kept short.
const MAX_ERROR_CHARS: usize = 500;
/// Anything shorter in a server's environment is a setting, and hiding it would
/// blank out ordinary words, unless its name says otherwise.
const MIN_SECRET_CHARS: usize = 8;
const SECRET_NAMES: [&str; 8] = [
    "TOKEN",
    "KEY",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "AUTH",
    "CREDENTIAL",
    "COOKIE",
];

/// How many processes a server gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sharing {
    /// One process for every agent: stateless or read-only servers.
    Shared,
    /// One per workspace, started in that folder. The default, since most
    /// servers read the folder they start in.
    #[default]
    PerWorkspace,
    /// One per agent session: browsers, REPLs, anything stateful.
    PerSession,
}

/// One server the host wants the hub to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSpec {
    /// Short and free of the separator: it prefixes every tool name.
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub sharing: Sharing,
    /// The tool list from an earlier run, so listing starts nothing.
    pub tools: Option<Vec<Value>>,
}

impl ServerSpec {
    /// A hash of exactly what would run. Secrets are part of it, so servers
    /// with different credentials never share a process.
    pub fn fingerprint(&self) -> String {
        let text = json!([self.command, self.args, self.env]).to_string();
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Told about things the host may want to keep.
pub trait Observer: Send + Sync + 'static {
    /// A shared server asked for something only one agent could answer. It now
    /// runs per session.
    fn demoted(&self, _server: &str, _method: &str) {}
    /// A server's tool list, fresh from the server.
    fn listed(&self, _server: &str, _tools: &[Value]) {}
}

/// An observer with nothing to do.
pub struct NoObserver;
impl Observer for NoObserver {}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Scope {
    All,
    Workspace(PathBuf),
    Session(i64),
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    server: String,
    fingerprint: String,
    scope: Scope,
}

impl Key {
    /// Whether this process is what the spec would start now.
    fn serves(&self, spec: &ServerSpec) -> bool {
        self.fingerprint == spec.fingerprint()
            && matches!(
                (spec.sharing, &self.scope),
                (Sharing::Shared, Scope::All)
                    | (Sharing::PerWorkspace, Scope::Workspace(_))
                    | (Sharing::PerSession, Scope::Session(_))
            )
    }
}

fn key(spec: &ServerSpec, grant: &Grant) -> Key {
    Key {
        server: spec.id.clone(),
        fingerprint: spec.fingerprint(),
        scope: match spec.sharing {
            Sharing::Shared => Scope::All,
            Sharing::PerWorkspace => Scope::Workspace(grant.workspace.clone()),
            Sharing::PerSession => Scope::Session(grant.session),
        },
    }
}

/// One process, for a monitoring view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub server: String,
    pub pid: u32,
    pub sharing: Sharing,
    pub workspace: Option<PathBuf>,
    pub sessions: BTreeSet<i64>,
    pub uptime: Duration,
    /// How long it has had nothing to do; `None` while it's busy.
    pub idle: Option<Duration>,
}

/// One failure, for a monitoring view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerError {
    pub server: String,
    pub message: String,
    pub at: std::time::SystemTime,
}

fn relock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lists by server and fingerprint, then by list name.
type Lists = HashMap<(String, String), HashMap<&'static str, Vec<Value>>>;

pub(crate) struct Pool {
    spawner: Arc<dyn Spawner>,
    pub(crate) observer: Arc<dyn Observer>,
    idle: Duration,
    limit: usize,
    specs: Mutex<HashMap<String, ServerSpec>>,
    running: Mutex<HashMap<Key, Arc<Upstream>>>,
    /// Off the books, and stopped once nothing is waiting on them.
    retired: Mutex<Vec<Arc<Upstream>>>,
    starting: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<()>>>>,
    lists: Mutex<Lists>,
    /// How many times each server restarted, so work begun before one can tell.
    restarts: Mutex<HashMap<String, u64>>,
    errors: Mutex<VecDeque<ServerError>>,
    hub: Mutex<Weak<Inner>>,
}

/// The three lists a server has, and the method and field of each.
pub(crate) const TOOLS: (&str, &str) = ("tools/list", "tools");
pub(crate) const PROMPTS: (&str, &str) = ("prompts/list", "prompts");
pub(crate) const RESOURCES: (&str, &str) = ("resources/list", "resources");

impl Pool {
    pub(crate) fn new(
        spawner: Arc<dyn Spawner>,
        observer: Arc<dyn Observer>,
        idle: Duration,
        limit: usize,
    ) -> Self {
        Self {
            spawner,
            observer,
            idle,
            limit,
            specs: Mutex::default(),
            running: Mutex::default(),
            retired: Mutex::default(),
            starting: Mutex::default(),
            lists: Mutex::default(),
            restarts: Mutex::default(),
            errors: Mutex::default(),
            hub: Mutex::new(Weak::new()),
        }
    }

    pub(crate) fn attach(&self, hub: Weak<Inner>) {
        *relock(&self.hub) = hub;
    }

    fn hub(&self) -> Option<Arc<Inner>> {
        relock(&self.hub).upgrade()
    }

    /// Keeps a failure for the host to show its user, with secrets already
    /// out of it. The log gets only which server: its message is the server's
    /// own text and may quote anything.
    fn record(&self, server: &str, message: String) {
        tracing::warn!(server, "an MCP server failed");
        let message = message.chars().take(MAX_ERROR_CHARS).collect();
        let mut errors = relock(&self.errors);
        if errors.len() >= REMEMBERED_ERRORS {
            errors.pop_front();
        }
        errors.push_back(ServerError {
            server: server.to_owned(),
            message,
            at: std::time::SystemTime::now(),
        });
    }

    /// Replaces the servers on offer. A process for anything that changed or
    /// went away stops, whoever was using it.
    pub(crate) fn set(&self, specs: Vec<ServerSpec>) {
        let specs: HashMap<_, _> = specs
            .into_iter()
            .map(|spec| (spec.id.clone(), spec))
            .collect();
        // First, so a start that finishes from here on sees the new specs, and
        // one that finished before is in `running` for the look below.
        *relock(&self.specs) = specs;
        let stale: Vec<_> = {
            let mut running = relock(&self.running);
            let keys: Vec<_> = running
                .keys()
                .filter(|key| !self.current(key))
                .cloned()
                .collect();
            keys.iter().filter_map(|key| running.remove(key)).collect()
        };
        for upstream in stale {
            upstream.shut();
        }
    }

    /// Stops a server's processes and forgets its lists, so the next call
    /// starts over. For what changed outside its spec, such as a sign-in.
    pub(crate) fn restart(&self, server: &str) {
        // First, so nothing begun before this can land after the cleanup.
        *relock(&self.restarts).entry(server.to_owned()).or_default() += 1;
        relock(&self.lists).retain(|(id, _), _| id != server);
        if let Some(spec) = relock(&self.specs).get_mut(server) {
            spec.tools = None;
        }
        let stopped: Vec<_> = {
            let mut running = relock(&self.running);
            let keys: Vec<_> = running
                .keys()
                .filter(|key| key.server == server)
                .cloned()
                .collect();
            keys.iter().filter_map(|key| running.remove(key)).collect()
        };
        for upstream in stopped {
            upstream.shut();
        }
    }

    fn restarts_of(&self, server: &str) -> u64 {
        relock(&self.restarts).get(server).copied().unwrap_or(0)
    }

    fn current(&self, key: &Key) -> bool {
        relock(&self.specs)
            .get(&key.server)
            .is_some_and(|spec| key.serves(spec))
    }

    fn spec(&self, server: &str) -> Option<ServerSpec> {
        relock(&self.specs).get(server).cloned()
    }

    fn offered(&self, grant: &Grant) -> Vec<ServerSpec> {
        let specs = relock(&self.specs);
        let mut offered: Vec<_> = grant
            .servers
            .iter()
            .filter_map(|server| specs.get(server).cloned())
            .collect();
        offered.sort_by(|a, b| a.id.cmp(&b.id));
        offered
    }

    /// The process for this caller, started if it isn't running. Only one
    /// caller ever starts a given process.
    async fn upstream(
        self: &Arc<Self>,
        spec: &ServerSpec,
        grant: &Grant,
    ) -> Result<Arc<Upstream>, String> {
        let key = key(spec, grant);
        let gate = relock(&self.starting)
            .entry(key.clone())
            .or_default()
            .clone();
        let found = self
            .running_or_started(&gate, key.clone(), spec, grant)
            .await;
        let mut gates = relock(&self.starting);
        // The map's and ours, so nobody else is waiting on it.
        if gates
            .get(&key)
            .is_some_and(|held| Arc::strong_count(held) == 2)
        {
            gates.remove(&key);
        }
        found
    }

    async fn running_or_started(
        self: &Arc<Self>,
        gate: &tokio::sync::Mutex<()>,
        key: Key,
        spec: &ServerSpec,
        grant: &Grant,
    ) -> Result<Arc<Upstream>, String> {
        let _turn = gate.lock().await;
        if let Some(upstream) = relock(&self.running).get(&key).cloned() {
            upstream.add_user(grant.session);
            return Ok(upstream);
        }
        // The server may have changed while this waited its turn.
        if !self.current(&key) {
            return Err(changed(&spec.id));
        }
        self.make_room();
        let restarts = self.restarts_of(&spec.id);
        let (workspace, reach) = match spec.sharing {
            Sharing::Shared => (None, Reach::Shared),
            Sharing::PerWorkspace => (Some(grant.workspace.clone()), Reach::Workspace),
            Sharing::PerSession => (Some(grant.workspace.clone()), Reach::Session),
        };
        // The host's spawner may block: it resolves secrets and starts a process.
        let spawner = self.spawner.clone();
        let (owned, folder) = (spec.clone(), workspace.clone());
        let spawned = tokio::task::spawn_blocking(move || spawner.spawn(&owned, folder.as_deref()))
            .await
            .map_err(|_| "The server start was interrupted".to_owned())?
            .map_err(|error| format!("{} could not start: {error}", spec.id));
        let events = Arc::new(PoolEvents {
            pool: Arc::downgrade(self),
            key: key.clone(),
        });
        let started = match spawned {
            Ok(spawned) => {
                Upstream::start(spec, spawned, workspace.as_deref(), reach, events).await
            }
            Err(error) => Err(error),
        };
        let upstream = started
            .map_err(|error| without_secrets(error, &spec.env))
            .inspect_err(|error| self.record(&spec.id, error.clone()))?;
        let mut running = relock(&self.running);
        if !self.current(&key) || self.restarts_of(&spec.id) != restarts {
            drop(running);
            upstream.shut();
            return Err(changed(&spec.id));
        }
        upstream.add_user(grant.session);
        running.insert(key, upstream.clone());
        Ok(upstream)
    }

    /// Stops the longest-idle processes until there's room for one more. With
    /// every process busy there's nothing to stop, and the new one runs anyway:
    /// a call that fails helps nobody, so the limit is a target, not a wall.
    fn make_room(&self) {
        let mut stopped = Vec::new();
        {
            let mut running = relock(&self.running);
            while running.len() >= self.limit {
                let oldest = running
                    .iter()
                    .filter_map(|(key, upstream)| Some((key.clone(), upstream.idle_for()?)))
                    .max_by_key(|(_, idle)| *idle);
                let Some((key, _)) = oldest else {
                    break;
                };
                stopped.extend(running.remove(&key));
            }
        }
        for upstream in stopped {
            upstream.shut();
        }
    }

    /// Stops everything that has been idle for the configured time.
    pub(crate) fn sweep(&self) {
        let idle: Vec<_> = {
            let mut running = relock(&self.running);
            let keys: Vec<_> = running
                .iter()
                .filter(|(_, upstream)| upstream.idle_for().is_some_and(|idle| idle >= self.idle))
                .map(|(key, _)| key.clone())
                .collect();
            keys.iter().filter_map(|key| running.remove(key)).collect()
        };
        for upstream in idle {
            upstream.shut();
        }
        let mut drained = Vec::new();
        relock(&self.retired).retain(|upstream| {
            let done = upstream.drained();
            if done {
                drained.push(upstream.clone());
            }
            !done
        });
        for upstream in drained {
            upstream.shut();
        }
    }

    pub(crate) fn sweep_interval(&self) -> Duration {
        (self.idle / 4).clamp(Duration::from_millis(20), Duration::from_secs(15))
    }

    pub(crate) fn shutdown(&self) {
        let all: Vec<_> = relock(&self.running)
            .drain()
            .map(|(_, upstream)| upstream)
            .chain(relock(&self.retired).drain(..))
            .collect();
        for upstream in all {
            upstream.shut();
        }
    }

    /// A session ended: its own processes stop, and it lets go of the others.
    pub(crate) fn forget(&self, session: i64) {
        self.let_go(session, |_| true);
    }

    /// A session lost these servers: the same, for them alone.
    pub(crate) fn release(&self, session: i64, servers: &BTreeSet<String>) {
        self.let_go(session, |key| servers.contains(&key.server));
    }

    fn let_go(&self, session: i64, chosen: impl Fn(&Key) -> bool) {
        let (own, others): (Vec<_>, Vec<_>) = {
            let mut running = relock(&self.running);
            let keys: Vec<_> = running
                .keys()
                .filter(|key| chosen(key) && key.scope == Scope::Session(session))
                .cloned()
                .collect();
            let own = keys.iter().filter_map(|key| running.remove(key)).collect();
            let others = running
                .iter()
                .filter(|(key, _)| chosen(key))
                .map(|(_, upstream)| upstream.clone())
                .collect();
            (own, others)
        };
        for upstream in own {
            upstream.shut();
        }
        for upstream in others {
            upstream.forget(session);
        }
    }

    fn cached(&self, spec: &ServerSpec, list: (&'static str, &'static str)) -> Option<Vec<Value>> {
        let stored = relock(&self.lists)
            .get(&(spec.id.clone(), spec.fingerprint()))
            .and_then(|lists| lists.get(list.1))
            .cloned();
        stored.or_else(|| (list == TOOLS).then(|| spec.tools.clone()).flatten())
    }

    /// Keeps a list, unless the server restarted since it was asked for.
    fn store(
        &self,
        spec: &ServerSpec,
        list: (&'static str, &'static str),
        items: Vec<Value>,
        restarts: u64,
    ) {
        {
            let mut lists = relock(&self.lists);
            if self.restarts_of(&spec.id) != restarts {
                return;
            }
            lists
                .entry((spec.id.clone(), spec.fingerprint()))
                .or_default()
                .insert(list.1, items.clone());
        }
        if list == TOOLS {
            self.observer.listed(&spec.id, &items);
        }
    }

    /// A server's list, from the cache when it has one.
    async fn items(
        self: &Arc<Self>,
        spec: &ServerSpec,
        grant: &Grant,
        list: (&'static str, &'static str),
    ) -> Vec<Value> {
        if let Some(items) = self.cached(spec, list) {
            return items;
        }
        let restarts = self.restarts_of(&spec.id);
        let upstream = match self.upstream(spec, grant).await {
            Ok(upstream) => upstream,
            Err(_) => return Vec::new(),
        };
        match upstream.list(list.0, list.1).await {
            Ok(items) => {
                self.store(spec, list, items.clone(), restarts);
                items
            }
            // A server without this list says so; it isn't a failure.
            Err(error) if error["code"] == -32601 => {
                self.store(spec, list, Vec::new(), restarts);
                Vec::new()
            }
            Err(error) => {
                let message = format!("{}: {error}", list.0);
                self.record(&spec.id, without_secrets(message, &spec.env));
                Vec::new()
            }
        }
    }

    /// Everything this caller can use from one list, each name prefixed with
    /// its server.
    pub(crate) async fn listed(
        self: &Arc<Self>,
        grant: &Grant,
        list: (&'static str, &'static str),
    ) -> Vec<Value> {
        let mut all = Vec::new();
        for spec in self.offered(grant) {
            for mut item in self.items(&spec, grant, list).await {
                // A server's answer is untrusted; only an object can be named.
                if !item.is_object() {
                    continue;
                }
                if list == RESOURCES {
                    let uri = item["uri"].as_str().unwrap_or("").to_owned();
                    item["uri"] = json!(resource_uri(&spec.id, &uri));
                } else {
                    let name = format!(
                        "{}{SEPARATOR}{}",
                        spec.id,
                        item["name"].as_str().unwrap_or("")
                    );
                    if list == TOOLS && name.len() > MAX_TOOL_NAME {
                        continue;
                    }
                    item["name"] = json!(name);
                }
                all.push(item);
            }
        }
        all
    }

    fn resolve(&self, grant: &Grant, server: &str) -> Option<ServerSpec> {
        grant
            .servers
            .contains(server)
            .then(|| self.spec(server))
            .flatten()
    }

    /// Sends a request to the server a prefixed name belongs to. `None` when
    /// the name isn't one of this caller's servers'.
    pub(crate) async fn forward(
        self: &Arc<Self>,
        grant: &Grant,
        server: &str,
        method: &str,
        params: Value,
    ) -> Option<Result<Value, Value>> {
        let spec = self.resolve(grant, server)?;
        let upstream = match self.upstream(&spec, grant).await {
            Ok(upstream) => upstream,
            Err(error) => return Some(Err(json!({"code":-32603,"message":error}))),
        };
        let answer = upstream.request(grant.session, method, params).await;
        Some(answer.map_err(|mut error| {
            let message = error["message"].as_str().unwrap_or("failed").to_owned();
            let message = without_secrets(message, &spec.env);
            self.record(server, message.clone());
            set_message(&mut error, message);
            error
        }))
    }

    pub(crate) async fn subscribe(
        self: &Arc<Self>,
        grant: &Grant,
        server: &str,
        uri: &str,
        on: bool,
    ) -> Result<(), String> {
        let spec = self.resolve(grant, server).ok_or("Unknown resource")?;
        let upstream = self.upstream(&spec, grant).await?;
        if !on {
            upstream.unsubscribe(grant.session, uri).await;
            return Ok(());
        }
        upstream.subscribe(grant.session, uri).await
    }

    pub(crate) fn snapshot(&self) -> Vec<Running> {
        let mut running: Vec<_> = relock(&self.running)
            .iter()
            .map(|(key, upstream)| Running {
                server: key.server.clone(),
                pid: upstream.pid,
                sharing: match key.scope {
                    Scope::All => Sharing::Shared,
                    Scope::Workspace(_) => Sharing::PerWorkspace,
                    Scope::Session(_) => Sharing::PerSession,
                },
                workspace: match &key.scope {
                    Scope::Workspace(path) => Some(path.clone()),
                    _ => None,
                },
                sessions: upstream.users(),
                uptime: upstream.started.elapsed(),
                idle: upstream.idle_for(),
            })
            .collect();
        running.sort_by(|a, b| (&a.server, a.pid).cmp(&(&b.server, b.pid)));
        running
    }

    pub(crate) fn errors(&self) -> Vec<ServerError> {
        relock(&self.errors).iter().cloned().collect()
    }

    /// Drops a stale list and tells whoever can see this server.
    fn list_changed(self: &Arc<Self>, key: &Key, upstream: &Arc<Upstream>, method: &str) {
        let Some(spec) = self.spec(&key.server).filter(|spec| key.serves(spec)) else {
            return;
        };
        let restarts = self.restarts_of(&key.server);
        let list = match method {
            "notifications/tools/list_changed" => TOOLS,
            "notifications/prompts/list_changed" => PROMPTS,
            _ => RESOURCES,
        };
        let pool = self.clone();
        let upstream = upstream.clone();
        let method = method.to_owned();
        let key = key.clone();
        tokio::spawn(async move {
            match upstream.list(list.0, list.1).await {
                // Replaced while it was listing: this list is no one's.
                Ok(_) if !pool.current(&key) => return,
                Ok(items) => pool.store(&spec, list, items, restarts),
                Err(error) => {
                    let message = format!("{}: {error}", list.0);
                    pool.record(&spec.id, without_secrets(message, &spec.env));
                }
            }
            if let Some(hub) = pool.hub() {
                hub.server_list_changed(&spec.id, &method);
            }
        });
    }

    fn demote(&self, key: &Key, method: &str) {
        let Some(mut spec) = self.spec(&key.server) else {
            return;
        };
        spec.sharing = Sharing::PerSession;
        relock(&self.specs).insert(key.server.clone(), spec);
        // Calls in flight finish first; the sweep stops it after them.
        let retired = relock(&self.running).remove(key);
        relock(&self.retired).extend(retired);
        self.observer.demoted(&key.server, method);
    }
}

/// Whether an environment name says its value is a credential.
fn names_a_secret(name: &str) -> bool {
    let name = name.to_uppercase();
    SECRET_NAMES.iter().any(|word| name.contains(word))
}

/// A server's error can quote what it was started with. Each whole value goes,
/// and each word of it, longest first so a part never leaves the rest behind.
fn without_secrets(mut message: String, env: &BTreeMap<String, String>) -> String {
    let mut secrets: Vec<&str> = env
        .iter()
        .flat_map(|(name, value)| {
            let any_length = names_a_secret(name);
            std::iter::once(value.as_str())
                .chain(value.split_whitespace())
                .filter(move |word| {
                    !word.is_empty() && (any_length || word.chars().count() >= MIN_SECRET_CHARS)
                })
        })
        .collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    for secret in secrets {
        message = message.replace(secret, "[hidden]");
    }
    message
}

/// A server's answer is untrusted: its error may not be an object.
fn set_message(error: &mut Value, message: String) {
    if let Some(fields) = error.as_object_mut() {
        fields.insert("message".to_owned(), message.into());
    }
}

fn changed(server: &str) -> String {
    format!("{server} changed while it was starting; try again")
}

/// A resource's URI as agents see it, naming the server it came from.
pub(crate) fn resource_uri(server: &str, uri: &str) -> String {
    format!("hub+{server}:{uri}")
}

/// The server and the server's own URI.
pub(crate) fn split_resource_uri(uri: &str) -> Option<(&str, &str)> {
    uri.strip_prefix("hub+")?.split_once(':')
}

struct PoolEvents {
    pool: Weak<Pool>,
    key: Key,
}

impl Events for PoolEvents {
    fn action(&self, upstream: &Arc<Upstream>, action: Action) {
        let Some(pool) = self.pool.upgrade() else {
            return;
        };
        match action {
            Action::ListChanged { method } => pool.list_changed(&self.key, upstream, &method),
            Action::Demote { method } => pool.demote(&self.key, &method),
            action => {
                if let Some(hub) = pool.hub() {
                    hub.server_action(&self.key.server, upstream, action);
                }
            }
        }
    }

    fn ask(&self, caller: Caller, method: String, params: Value, answer: Answer) {
        match self.pool.upgrade().and_then(|pool| pool.hub()) {
            Some(hub) => hub.ask_agent(caller, method, params, answer),
            None => answer(Err("The hub has stopped".to_owned())),
        }
    }

    fn capabilities(&self, caller: Caller) -> Option<Value> {
        self.pool.upgrade()?.hub()?.capabilities_of(caller)
    }

    fn exited(&self, upstream: &Arc<Upstream>) {
        let Some(pool) = self.pool.upgrade() else {
            return;
        };
        let mut running = relock(&pool.running);
        if running
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, upstream))
        {
            running.remove(&self.key);
            drop(running);
            pool.record(&self.key.server, "The server exited".to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, env: &[(&str, &str)]) -> ServerSpec {
        ServerSpec {
            id: id.into(),
            command: "server".into(),
            args: vec!["--stdio".into()],
            env: env
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            sharing: Sharing::Shared,
            tools: None,
        }
    }

    struct Nothing;
    impl Spawner for Nothing {
        fn spawn(
            &self,
            _: &ServerSpec,
            _: Option<&std::path::Path>,
        ) -> std::io::Result<crate::Spawned> {
            Err(std::io::ErrorKind::Unsupported.into())
        }
    }

    fn pool() -> Pool {
        Pool::new(
            Arc::new(Nothing),
            Arc::new(NoObserver),
            Duration::from_secs(1),
            1,
        )
    }

    #[test]
    fn a_failure_never_keeps_the_secrets_a_server_quoted() {
        let env = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let quoted = "rejected s3cret-value at info";
        assert_eq!(
            without_secrets(
                quoted.into(),
                &env(&[("Authorization", "Bearer s3cret-value"), ("LEVEL", "info")])
            ),
            "rejected [hidden] at info"
        );
        assert_eq!(
            without_secrets("key ab1 ok".into(), &env(&[("API_KEY", "ab1")])),
            "key [hidden] ok",
            "the name says it's a secret, however short"
        );
        assert_eq!(
            without_secrets(
                "sent two short words".into(),
                &env(&[("PHRASE", "two short words")])
            ),
            "sent [hidden]",
            "a value of short words goes whole"
        );
        assert_eq!(
            without_secrets(
                "s3cret-value-longer".into(),
                &env(&[
                    ("A_TOKEN", "s3cret-value"),
                    ("B_TOKEN", "s3cret-value-longer")
                ])
            ),
            "[hidden]"
        );
    }

    #[test]
    fn an_error_that_isnt_an_object_is_left_alone_rather_than_a_panic() {
        for mut error in [json!("failed"), json!(7), json!([1])] {
            let before = error.clone();
            set_message(&mut error, "hidden".to_owned());
            assert_eq!(error, before);
        }
        let mut error = json!({"code":1,"message":"s3cret"});
        set_message(&mut error, "hidden".to_owned());
        assert_eq!(error, json!({"code":1,"message":"hidden"}));
    }

    #[test]
    fn a_restart_drops_every_list_and_refuses_one_asked_for_before_it() {
        let pool = pool();
        let mut server = spec("a", &[]);
        server.tools = Some(vec![json!({"name":"old"})]);
        pool.set(vec![server.clone()]);
        let before = pool.restarts_of("a");
        pool.store(&server, TOOLS, vec![json!({"name":"kept"})], before);
        pool.restart("a");
        let current = pool.spec("a").unwrap();
        assert_eq!(pool.cached(&current, TOOLS), None, "nothing old survives");
        pool.store(&current, TOOLS, vec![json!({"name":"late"})], before);
        assert_eq!(
            pool.cached(&current, TOOLS),
            None,
            "that list is out of date"
        );
        let after = pool.restarts_of("a");
        pool.store(&current, TOOLS, vec![json!({"name":"new"})], after);
        assert_eq!(pool.cached(&current, TOOLS).unwrap()[0]["name"], "new");
    }

    #[test]
    fn the_fingerprint_changes_with_anything_that_would_run() {
        let base = spec("a", &[("K", "1")]);
        assert_eq!(base.fingerprint(), spec("a", &[("K", "1")]).fingerprint());
        assert_ne!(base.fingerprint(), spec("a", &[("K", "2")]).fingerprint());
        let mut changed = base.clone();
        changed.args.push("--more".into());
        assert_ne!(base.fingerprint(), changed.fingerprint());
        let mut changed = base.clone();
        changed.command = "other".into();
        assert_ne!(base.fingerprint(), changed.fingerprint());
        // Sharing is a choice about the process, not what it is.
        let mut changed = base.clone();
        changed.sharing = Sharing::PerSession;
        assert_eq!(base.fingerprint(), changed.fingerprint());
    }

    #[test]
    fn resource_uris_keep_their_server_and_their_own_colons() {
        let uri = resource_uri("docs", "file:///a:b");
        assert_eq!(split_resource_uri(&uri), Some(("docs", "file:///a:b")));
        assert_eq!(split_resource_uri("file:///a"), None);
    }

    #[test]
    fn the_key_follows_the_sharing_class() {
        let grant = |session, workspace: &str| Grant {
            session,
            workspace: workspace.into(),
            toolkits: BTreeSet::new(),
            servers: BTreeSet::new(),
        };
        let mut server = spec("a", &[]);
        assert_eq!(key(&server, &grant(1, "/a")), key(&server, &grant(2, "/b")));
        server.sharing = Sharing::PerWorkspace;
        assert_eq!(key(&server, &grant(1, "/a")), key(&server, &grant(2, "/a")));
        assert_ne!(key(&server, &grant(1, "/a")), key(&server, &grant(1, "/b")));
        server.sharing = Sharing::PerSession;
        assert_ne!(key(&server, &grant(1, "/a")), key(&server, &grant(2, "/a")));
    }
}
