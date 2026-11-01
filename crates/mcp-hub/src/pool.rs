//! The server processes behind the hub. Each is started the first time a call
//! needs it, shared as far as its class allows, and stopped when idle. Agents
//! list tools from a cache, so listing never starts anything that has run once.
use crate::{
    endpoint::Inner,
    router::{Action, Reach},
    upstream::{Events, Upstream},
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
    starting: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<()>>>>,
    lists: Mutex<Lists>,
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
            starting: Mutex::default(),
            lists: Mutex::default(),
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

    fn record(&self, server: &str, message: String) {
        tracing::warn!(server, %message, "an MCP server failed");
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
        let stale: Vec<_> = {
            let mut running = relock(&self.running);
            let keys: Vec<_> = running
                .keys()
                .filter(|key| {
                    specs
                        .get(&key.server)
                        .is_none_or(|spec| spec.fingerprint() != key.fingerprint)
                })
                .cloned()
                .collect();
            keys.iter().filter_map(|key| running.remove(key)).collect()
        };
        *relock(&self.specs) = specs;
        for upstream in stale {
            upstream.shut();
        }
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
        let _turn = gate.lock().await;
        if let Some(upstream) = relock(&self.running).get(&key).cloned() {
            upstream.add_user(grant.session);
            return Ok(upstream);
        }
        self.make_room();
        let (workspace, reach) = match spec.sharing {
            Sharing::Shared => (None, Reach::Shared),
            Sharing::PerWorkspace => (Some(grant.workspace.clone()), Reach::Workspace),
            Sharing::PerSession => (Some(grant.workspace.clone()), Reach::Session),
        };
        let pool = self.clone();
        let (spec, session) = (spec.clone(), grant.session);
        let upstream = tokio::task::spawn_blocking(move || {
            let spawned = pool
                .spawner
                .spawn(&spec, workspace.as_deref())
                .map_err(|error| format!("{} could not start: {error}", spec.id));
            let started = spawned.and_then(|spawned| {
                let events = Arc::new(PoolEvents {
                    pool: Arc::downgrade(&pool),
                    key: key.clone(),
                });
                Upstream::start(&spec, spawned, workspace.as_deref(), reach, events)
            });
            let upstream = started.inspect_err(|error| pool.record(&spec.id, error.clone()))?;
            upstream.add_user(session);
            relock(&pool.running).insert(key, upstream.clone());
            Ok::<_, String>(upstream)
        })
        .await
        .map_err(|_| "The server start was interrupted".to_owned())??;
        Ok(upstream)
    }

    /// Stops the longest-idle processes until there's room for one more.
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
    }

    pub(crate) fn sweep_interval(&self) -> Duration {
        (self.idle / 4).clamp(Duration::from_millis(20), Duration::from_secs(15))
    }

    pub(crate) fn shutdown(&self) {
        let all: Vec<_> = relock(&self.running)
            .drain()
            .map(|(_, upstream)| upstream)
            .collect();
        for upstream in all {
            upstream.shut();
        }
    }

    /// A session ended: its own processes stop, and it lets go of the others.
    pub(crate) fn forget(&self, session: i64) {
        let (own, others): (Vec<_>, Vec<_>) = {
            let mut running = relock(&self.running);
            let keys: Vec<_> = running
                .keys()
                .filter(|key| key.scope == Scope::Session(session))
                .cloned()
                .collect();
            let own = keys.iter().filter_map(|key| running.remove(key)).collect();
            (own, running.values().cloned().collect())
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

    fn store(&self, spec: &ServerSpec, list: (&'static str, &'static str), items: Vec<Value>) {
        if list == TOOLS {
            self.observer.listed(&spec.id, &items);
        }
        relock(&self.lists)
            .entry((spec.id.clone(), spec.fingerprint()))
            .or_default()
            .insert(list.1, items);
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
        let upstream = match self.upstream(spec, grant).await {
            Ok(upstream) => upstream,
            Err(_) => return Vec::new(),
        };
        let fetched = tokio::task::spawn_blocking(move || upstream.list(list.0, list.1)).await;
        match fetched {
            Ok(Ok(items)) => {
                self.store(spec, list, items.clone());
                items
            }
            // A server without this list says so; it isn't a failure.
            Ok(Err(error)) if error["code"] == -32601 => {
                self.store(spec, list, Vec::new());
                Vec::new()
            }
            Ok(Err(error)) => {
                self.record(&spec.id, format!("{}: {error}", list.0));
                Vec::new()
            }
            Err(_) => Vec::new(),
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
                if list == RESOURCES {
                    let uri = item["uri"].as_str().unwrap_or("").to_owned();
                    item["uri"] = json!(resource_uri(&spec.id, &uri));
                } else {
                    let name = format!(
                        "{}{SEPARATOR}{}",
                        spec.id,
                        item["name"].as_str().unwrap_or("")
                    );
                    if name.len() > MAX_TOOL_NAME {
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
        if let Err(error) = &answer {
            self.record(
                server,
                error["message"].as_str().unwrap_or("failed").to_owned(),
            );
        }
        Some(answer)
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
        if on {
            upstream.subscribe(grant.session, uri);
        } else {
            upstream.unsubscribe(grant.session, uri);
        }
        Ok(())
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
        let Some(spec) = self.spec(&key.server) else {
            return;
        };
        let list = match method {
            "notifications/tools/list_changed" => TOOLS,
            "notifications/prompts/list_changed" => PROMPTS,
            _ => RESOURCES,
        };
        let pool = self.clone();
        let upstream = upstream.clone();
        let method = method.to_owned();
        std::thread::spawn(move || {
            match upstream.list(list.0, list.1) {
                Ok(items) => pool.store(&spec, list, items),
                Err(error) => pool.record(&spec.id, format!("{}: {error}", list.0)),
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
        // In-flight calls hold it until they finish.
        relock(&self.running).remove(key);
        self.observer.demoted(&key.server, method);
    }
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

    fn exited(&self, upstream: &Arc<Upstream>) {
        let Some(pool) = self.pool.upgrade() else {
            return;
        };
        let mut running = relock(&pool.running);
        if running
            .get(&self.key)
            .is_some_and(|current| current.pid == upstream.pid)
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
