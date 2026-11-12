//! The app's own tools: one MCP hub, its toolkits, and how each provider is
//! told about it. A toolkit is offered only when the turn's grant names it.
//!
//! Adding a capability means registering a toolkit in `hub` and naming it in
//! the turn's [`Mount`]; no provider needs to change.
use crate::mcp_catalog::{Catalog, SignIn};
use crate::provider::Kind;
use mcp_hub::{Config, Connection, Grant, Hub, Observer, ServerSpec, Spawned, Spawner, Token};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, LazyLock, Mutex, OnceLock},
};

pub const VISUAL: &str = "visual";
pub const TICKET: &str = "ticket";
pub const APP: &str = "app";
pub const CONTROL: &str = "control";
pub const ACTIONS: &str = "actions";
pub const ACTIONS_READ: &str = "actions_read";
pub const BROWSER: &str = "browser";
/// The decision log: read, search, propose and flag.
pub const DECISIONS: &str = "decisions";
/// The decision log for orchestration workers: read, search and flag.
pub const DECISIONS_READ: &str = "decisions_read";

/// The name agents know the hub by. Tools appear to Claude as
/// `mcp__gespan__<tool>`.
const SERVER_NAME: &str = "gespan";
/// How long Codex waits on one call to the app's MCP server. Its default
/// gives up on long tools such as a browser action that waits for the
/// browser to start or connect ([`gespan_browser::HOST_WAIT`]) on top of its
/// own time limit.
const CODEX_TOOL_TIMEOUT_SECS: u64 = 300;
/// Set on the relay process alone, by the agent that starts it, or for the
/// CLIs of agents that mount no MCP servers.
const ENV_URL: &str = "GESPAN_MCP_URL";
const ENV_TOKEN: &str = "GESPAN_MCP_TOKEN";

static HUB: OnceLock<Result<Hub, String>> = OnceLock::new();
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
/// The latest mount of each session, so a catalogue change can reach it.
static LIVE: LazyLock<Mutex<HashMap<i64, Mount>>> = LazyLock::new(Default::default);

/// Where the catalogue and the record of running servers live. Set once at
/// startup; without it the hub offers Gespan's own toolkits alone.
pub fn configure(data_dir: &Path) {
    if DATA_DIR.set(data_dir.to_owned()).is_err() && DATA_DIR.get().is_some_and(|d| d != data_dir) {
        tracing::warn!("the tool endpoint keeps the data folder it started with");
    }
}

pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.get().cloned()
}

/// Hands the hub the catalogue's approved servers, when they've changed.
pub fn sync_servers() {
    static OFFERED: Mutex<Vec<ServerSpec>> = Mutex::new(Vec::new());
    let (Some(data_dir), Ok(hub)) = (DATA_DIR.get(), hub()) else {
        return;
    };
    let specs = match Catalog::load(data_dir) {
        Ok(catalog) => catalog.specs(data_dir),
        Err(error) => {
            tracing::warn!(%error, "could not read the MCP catalogue");
            return;
        }
    };
    let mut offered = OFFERED.lock().unwrap_or_else(|e| e.into_inner());
    if *offered != specs {
        hub.set_servers(specs.clone());
        *offered = specs;
    }
    drop(offered);
    // A server turned on now reaches sessions already running.
    let Ok(catalog) = Catalog::load(data_dir) else {
        return;
    };
    let live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    for mount in live.values() {
        hub.update(
            mount.session,
            mount.toolkits.clone(),
            catalog.for_project(&mount.project),
        );
    }
}

/// Whether a server's command is really a remote address.
pub(crate) fn is_remote(command: &str) -> bool {
    url::Url::parse(command)
        .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.has_host())
}

/// One browser sign-in to a remote server, and every agent can use it. Blocks
/// until the browser comes back.
pub fn sign_in(id: &str, url: &str, open: impl FnOnce(&str)) -> Result<(), String> {
    if !mcp_hub::safe_for_tokens(url) {
        return Err("Sign-in needs an https address".into());
    }
    let data_dir = DATA_DIR.get().ok_or("Gespan's data folder isn't set")?;
    let store = Arc::new(SignIn::new(data_dir, id, url));
    mcp_hub::sign_in(
        url,
        "Gespan",
        store,
        open,
        std::time::Duration::from_secs(300),
    )?;
    // A connection already open started without this sign-in.
    if let Ok(hub) = hub() {
        hub.restart(id);
    }
    Ok(())
}

pub use mcp_hub::{
    Running as RunningServer, ServerError as ServerFailure, Sharing as ServerSharing,
};

/// The processes the catalogue's servers are running as, for Settings.
pub fn running_servers() -> Vec<RunningServer> {
    match HUB.get() {
        Some(Ok(hub)) => hub.running(),
        _ => Vec::new(),
    }
}

/// Recent failures of the catalogue's servers, for Settings.
pub fn server_errors() -> Vec<ServerFailure> {
    match HUB.get() {
        Some(Ok(hub)) => hub.errors(),
        _ => Vec::new(),
    }
}

/// What the catalogue gives a project's sessions: the servers they may reach,
/// and whether agents leave their own off. A catalogue that can't be read is one
/// the user has, so the agent stays off its own servers rather than quietly
/// running them, and the turn goes on with Gespan's own tools.
pub fn catalog_reach(data_dir: Option<&Path>, project: &str) -> (BTreeSet<String>, bool) {
    let Some(data_dir) = data_dir else {
        return Default::default();
    };
    match Catalog::load(data_dir) {
        Ok(catalog) => (catalog.for_project(project), catalog.managed),
        Err(error) => {
            tracing::warn!(%error, "could not read the MCP catalogue");
            (BTreeSet::new(), true)
        }
    }
}

/// The folder of server records, read by the next launch's sweep. One file per
/// running Gespan, named for its pid, so none can touch another's.
pub fn running_record(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp-servers")
}

/// One Mutex for the file, since servers start and stop on many threads.
fn edit_record(record: &Path, edit: impl FnOnce(&mut BTreeSet<u32>)) -> std::io::Result<()> {
    static RECORD: Mutex<()> = Mutex::new(());
    let _held = RECORD.lock().unwrap_or_else(|e| e.into_inner());
    let text = std::fs::read_to_string(record).unwrap_or_default();
    let mut pids: BTreeSet<u32> = text.lines().filter_map(|line| line.parse().ok()).collect();
    edit(&mut pids);
    if pids.is_empty() {
        return match std::fs::remove_file(record) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    if let Some(folder) = record.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let text: String = pids.iter().map(|pid| format!("{pid}\n")).collect();
    std::fs::write(record, text)
}

/// Starts a server and records it. One that can't be recorded doesn't run,
/// since a crash would leave it behind where the sweep can't find it.
fn start_server(
    spec: &ServerSpec,
    workspace: Option<&Path>,
    record: Option<PathBuf>,
) -> std::io::Result<Spawned> {
    // npx and uvx are scripts on Windows, which only cmd can start.
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", &spec.command]);
        command
    };
    #[cfg(not(windows))]
    let mut command = Command::new(&spec.command);
    crate::scm::shell::apply_gui_env(&mut command);
    command
        .args(&spec.args)
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(workspace) = workspace {
        command.current_dir(workspace);
    }
    crate::process::isolate(&mut command);
    let mut child = crate::process::spawn(&mut command)?;
    let pid = child.id();
    if let Some(record) = &record {
        if let Err(error) = edit_record(record, |pids| {
            pids.insert(pid);
        }) {
            crate::process::terminate(&mut child);
            return Err(error);
        }
    }
    let stdin = child.stdin.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
    let stdout = child.stdout.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
    Ok(Spawned {
        pid,
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        stop: Box::new(move || {
            crate::process::terminate(&mut child);
            if let Some(record) = &record {
                let stopped = edit_record(record, |pids| {
                    pids.remove(&pid);
                });
                if let Err(error) = stopped {
                    tracing::warn!(%error, "could not update the MCP server record");
                }
            }
        }),
    })
}

struct Processes;

impl Spawner for Processes {
    fn spawn(&self, spec: &ServerSpec, workspace: Option<&Path>) -> std::io::Result<Spawned> {
        if is_remote(&spec.command) {
            let oauth = DATA_DIR
                .get()
                .map(|data_dir| SignIn::new(data_dir, &spec.id, &spec.command))
                .filter(SignIn::exists);
            return mcp_hub::connect(
                &spec.command,
                spec.env.clone(),
                oauth.map(|store| Arc::new(store) as Arc<dyn mcp_hub::TokenStore>),
            );
        }
        let record = DATA_DIR
            .get()
            .map(|dir| running_record(dir).join(std::process::id().to_string()));
        start_server(spec, workspace, record)
    }
}

/// Keeps what the hub learns about a server with its catalogue entry.
struct Keeper;

fn keep(data_dir: &Path, edit: impl FnOnce(&mut Catalog)) {
    let kept = crate::mcp_catalog::update(data_dir, |catalog| {
        edit(catalog);
        Ok(())
    });
    if let Err(error) = kept {
        tracing::warn!(%error, "could not save to the MCP catalogue");
    }
}

fn keep_class(data_dir: &Path, server: &str) {
    keep(data_dir, |catalog| {
        if let Some(entry) = catalog.entry_mut(server) {
            entry.class = crate::mcp_catalog::Class::PerSession;
        }
    });
}

fn keep_tools(data_dir: &Path, server: &str, tools: &[Value]) {
    keep(data_dir, |catalog| {
        if let Some(entry) = catalog.entry_mut(server) {
            entry.tools = Some(tools.to_vec());
        }
    });
}

impl Observer for Keeper {
    fn demoted(&self, server: &str, method: &str) {
        tracing::info!(server, method, "a shared MCP server now runs per session");
        if let Some(data_dir) = DATA_DIR.get() {
            keep_class(data_dir, server);
        }
    }

    fn listed(&self, server: &str, tools: &[Value]) {
        if let Some(data_dir) = DATA_DIR.get() {
            keep_tools(data_dir, server, tools);
        }
    }
}

fn hub() -> Result<&'static Hub, String> {
    HUB.get_or_init(|| {
        let mut config = Config::new(SERVER_NAME, env!("CARGO_PKG_VERSION"));
        config.spawner = Some(Arc::new(Processes));
        config.observer = Arc::new(Keeper);
        let hub = Hub::start(config)?;
        hub.register(Arc::new(crate::visual_bridge::Toolkit));
        hub.register(Arc::new(crate::ticket_bridge::Toolkit));
        hub.register(Arc::new(crate::pilot::bridge::Toolkit::APP));
        hub.register(Arc::new(crate::pilot::bridge::Toolkit::CONTROL));
        hub.register(Arc::new(crate::project_actions::toolkit::Toolkit {
            may_run: true,
        }));
        hub.register(Arc::new(crate::project_actions::toolkit::Toolkit {
            may_run: false,
        }));
        hub.register(Arc::new(crate::browser::toolkit::Toolkit::GLOBAL));
        hub.register(Arc::new(crate::decisions::toolkit::Toolkit {
            may_propose: true,
        }));
        hub.register(Arc::new(crate::decisions::toolkit::Toolkit {
            may_propose: false,
        }));
        Ok(hub)
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// What one turn's agent may reach through the hub. Transport-only: it lives
/// on the turn's settings and is never saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mount {
    pub session: i64,
    pub worktree: PathBuf,
    pub toolkits: BTreeSet<String>,
    /// The project's path, for what's turned on where.
    pub project: String,
    /// Catalogue servers this session may reach.
    pub servers: BTreeSet<String>,
    /// Gespan owns the user's servers, so the agent loads none of its own.
    pub exclusive: bool,
    /// Codex's own servers, switched off by name since its overrides merge.
    pub codex_own: Vec<String>,
}

impl Mount {
    fn grant(&self) -> Grant {
        Grant {
            session: self.session,
            workspace: self.worktree.clone(),
            toolkits: self.toolkits.clone(),
            servers: self.servers.clone(),
        }
    }
}

/// How a provider receives the endpoint and its credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// `mcp_set_servers` on the stream-json control channel.
    ClaudeControl,
    /// `config` on `thread/start` and `thread/resume`, over stdin.
    CodexThread,
    /// `mcpServers` on `session/new` and `session/load`, over stdin.
    AcpSession,
    /// `POST /mcp` on the local server Gespan already holds open.
    OpenCodeApi,
    /// No way in but the environment: the agent reaches the tools through the
    /// `app` and `visual` CLIs, and every command it runs inherits the token.
    Environment,
}

/// The `_meta` key OpenCode v2 stamps on every tool call with the conversation
/// making it. Its model can't set it.
const OPENCODE_CONVERSATION: &str = "ai.opencode/sessionID";

/// OpenCode v2's service is shared by every conversation in a folder, so each
/// session's server there has a name of its own. No name is a prefix of
/// another's up to the `_` OpenCode puts before each tool name.
pub fn opencode_name(session: i64) -> String {
    format!("{SERVER_NAME}_s{session}")
}

/// Permission rows that hide every session's server on OpenCode v2's shared
/// service but this one's, which keeps `effect`. The last matching row wins.
pub fn opencode_permissions(session: Option<i64>, effect: &str) -> Vec<Value> {
    let mut rows =
        vec![json!({"action":format!("{SERVER_NAME}_*"),"resource":"*","effect":"deny"})];
    if let Some(session) = session {
        rows.push(json!({
            "action":format!("{}_*", opencode_name(session)),
            "resource":"*",
            "effect":effect,
        }));
    }
    rows
}

pub fn delivery(kind: Kind, provider_id: Option<&str>) -> Delivery {
    use crate::provider::Family;
    match kind.family() {
        Family::Claude => Delivery::ClaudeControl,
        Family::Codex => Delivery::CodexThread,
        Family::OpenCode => Delivery::OpenCodeApi,
        Family::Pi => Delivery::Environment,
        // Their resume requests don't refresh mcpServers.
        Family::Acp if matches!(kind, Kind::Grok | Kind::Fx) && provider_id.is_some() => {
            Delivery::Environment
        }
        Family::Acp => Delivery::AcpSession,
    }
}

/// A live credential on its way to a new provider process.
pub struct Access {
    url: String,
    token: Token,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Access(..)")
    }
}

impl Access {
    /// A new provider process gets a new token, and the one it replaces loses
    /// its own. `None` leaves the turn running with no tools.
    pub fn open(mount: &Mount) -> Option<Self> {
        let hub = hub()
            .inspect_err(|error| tracing::warn!(%error, "the tool endpoint isn't running"))
            .ok()?;
        Some(Self {
            url: hub.url().to_owned(),
            token: hub.mint(mount.grant()),
        })
    }

    /// A credential for OpenCode v2's service, which every conversation in the
    /// folder can reach, so it serves calls from `conversation` only. It is
    /// bound before it exists anywhere outside the hub.
    pub fn open_opencode(mount: &Mount, conversation: &str) -> Option<Self> {
        let hub = hub()
            .inspect_err(|error| tracing::warn!(%error, "the tool endpoint isn't running"))
            .ok()?;
        Some(Self {
            url: hub.url().to_owned(),
            token: hub.mint_bound(mount.grant(), OPENCODE_CONVERSATION, json!(conversation)),
        })
    }

    /// Ends this credential, when the provider never received it.
    pub fn withdraw(&self, session: i64) {
        if let Ok(hub) = hub() {
            hub.withdraw(session, &self.token);
        }
    }

    /// Whether this credential is still the one its session can use.
    pub fn is_live(&self, session: i64) -> bool {
        hub().is_ok_and(|hub| hub.holds(session, &self.token))
    }

    pub fn claude_control(&self, request_id: &str) -> Value {
        json!({"type":"control_request","request_id":request_id,"request":{
            "subtype":"mcp_set_servers",
            "servers":{SERVER_NAME:{
                "type":"http",
                "url":self.url,
                "headers":{"Authorization":self.token.bearer()},
            }},
        }})
    }

    pub fn codex_config(&self, mount: &Mount) -> Value {
        let mut servers = serde_json::Map::new();
        for name in &mount.codex_own {
            servers.insert(name.clone(), json!({"enabled":false}));
        }
        servers.insert(
            SERVER_NAME.to_owned(),
            json!({
                "url":self.url,
                "http_headers":{"Authorization":self.token.bearer()},
                "tool_timeout_sec":CODEX_TOOL_TIMEOUT_SECS,
            }),
        );
        json!({"mcp_servers":servers})
    }

    pub fn opencode_server(&self) -> Value {
        json!({"name":SERVER_NAME,"config":self.opencode_config()})
    }

    pub fn opencode_config(&self) -> Value {
        json!({
            "type":"remote",
            "url":self.url,
            "headers":{"Authorization":self.token.bearer()},
            "oauth":false,
        })
    }

    /// For a relay Gespan runs itself, so the token stays in this process.
    pub fn connection(&self) -> Connection {
        Connection {
            url: self.url.clone(),
            token: self.token.secret().to_owned(),
        }
    }

    /// ACP only promises stdio MCP. An agent that advertises HTTP gets the
    /// endpoint; any other runs the relay, which alone holds the token.
    pub fn acp_servers(&self, http: bool) -> Value {
        if http {
            return json!([{
                "type":"http",
                "name":SERVER_NAME,
                "url":self.url,
                "headers":[{"name":"Authorization","value":self.token.bearer()}],
            }]);
        }
        let Ok(executable) = std::env::current_exe() else {
            return json!([]);
        };
        json!([{
            "name":SERVER_NAME,
            "command":executable,
            "args":["mcp-relay"],
            "env":[
                {"name":ENV_URL,"value":self.url},
                {"name":ENV_TOKEN,"value":self.token.secret()},
            ],
        }])
    }
}

/// The fallback credential of [`Delivery::Environment`], stable across a
/// session's turns so a parked provider stays reusable.
pub fn environment(mount: &Mount) -> HashMap<String, String> {
    static ISSUED: OnceLock<Mutex<HashMap<i64, Token>>> = OnceLock::new();
    let hub = match hub() {
        Ok(hub) => hub,
        Err(error) => {
            tracing::warn!(%error, "the tool endpoint isn't running");
            return HashMap::new();
        }
    };
    let mut issued = ISSUED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // A session can change route between turns, and `Access::open` then
    // replaces the token held here.
    let token = match issued.get(&mount.session) {
        Some(token) if hub.holds(mount.session, token) => token.clone(),
        _ => {
            let token = hub.mint(mount.grant());
            issued.insert(mount.session, token.clone());
            token
        }
    };
    HashMap::from([
        (ENV_URL.to_owned(), hub.url().to_owned()),
        (ENV_TOKEN.to_owned(), token.secret().to_owned()),
    ])
}

/// Applies this turn's reach to a session already holding a credential, which
/// a turn with no toolkits empties. An agent still running sees its tool list
/// change. With no endpoint yet there's no credential, and none is started.
pub fn refresh(mount: &Mount) {
    LIVE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(mount.session, mount.clone());
    if let Some(Ok(hub)) = HUB.get() {
        hub.update(mount.session, mount.toolkits.clone(), mount.servers.clone());
    }
}

/// The project of a session's latest turn, once Gespan has mounted it with one.
pub(crate) fn project_of(session: i64) -> Option<String> {
    LIVE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&session)
        .map(|mount| mount.project.clone())
        .filter(|project| !project.is_empty())
}

/// A connection as `mount`'s agent, for tests that call through the hub.
#[cfg(test)]
pub(crate) fn test_connection(mount: &Mount) -> Connection {
    refresh(mount);
    let access = Access::open(mount).expect("the tool endpoint runs");
    Connection {
        url: access.url,
        token: access.token.secret().to_owned(),
    }
}

/// Whether a session can reach the endpoint right now.
pub fn is_open(session: i64) -> bool {
    hub().is_ok_and(|hub| hub.has_grant(session))
}

/// A running turn holds its credential open between tool calls.
pub fn touch(session: i64) {
    if let Ok(hub) = hub() {
        hub.touch(session);
    }
}

/// The `mcp-relay` subcommand: a stdio front for an agent that can't speak HTTP.
pub fn relay() -> Result<(), String> {
    mcp_hub::relay(connection()?)
}

fn connection() -> Result<Connection, String> {
    let get = |key| std::env::var(key).map_err(|_| "This needs a running Gespan session");
    Ok(Connection {
        url: get(ENV_URL)?,
        token: get(ENV_TOKEN)?,
    })
}

/// One tool call from a CLI, as this session.
pub fn call_tool(tool: &str, arguments: Value) -> Result<Value, String> {
    mcp_hub::call_tool(connection()?, tool, arguments)
}

/// Ends a session's tool access. The store calls this for every archive and
/// delete, whatever asked for it. With no endpoint yet there's nothing to end,
/// and none is started.
pub fn revoke(session: i64) {
    LIVE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session);
    if let Some(Ok(hub)) = HUB.get() {
        hub.revoke(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(session: i64) -> Mount {
        Mount {
            session,
            worktree: "/work".into(),
            toolkits: BTreeSet::from([VISUAL.to_owned(), TICKET.to_owned()]),
            ..Mount::default()
        }
    }

    fn secret_of(access: &Access) -> String {
        access.token.secret().to_owned()
    }

    #[test]
    fn a_remote_address_is_told_from_a_command_by_what_it_parses_as() {
        for command in [
            "https://a.example/mcp",
            "HTTPS://A.example/mcp",
            "http://[::1]:9",
        ] {
            assert!(is_remote(command), "{command}");
        }
        for command in [
            "npx",
            "C:\\tools\\server.exe",
            "ws://a.example/mcp",
            "http://",
        ] {
            assert!(!is_remote(command), "{command}");
        }
    }

    #[test]
    fn claude_gets_the_endpoint_over_its_control_channel() {
        let access = Access::open(&mount(5001)).unwrap();
        let request = access.claude_control("native-mcp");
        assert_eq!(request["request_id"], "native-mcp");
        assert_eq!(request["request"]["subtype"], "mcp_set_servers");
        let server = &request["request"]["servers"]["gespan"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], access.url);
        assert_eq!(
            server["headers"]["Authorization"],
            format!("Bearer {}", secret_of(&access))
        );
    }

    #[test]
    fn codex_gets_the_endpoint_in_its_thread_config() {
        let access = Access::open(&mount(5002)).unwrap();
        let config = access.codex_config(&Mount {
            codex_own: vec!["github".into()],
            ..mount(5002)
        });
        assert_eq!(config["mcp_servers"]["github"], json!({"enabled":false}));
        let server = &config["mcp_servers"]["gespan"];
        assert_eq!(server["url"], access.url);
        assert_eq!(
            server["http_headers"]["Authorization"],
            format!("Bearer {}", secret_of(&access))
        );
    }

    #[test]
    fn codex_config_sets_a_long_tool_timeout() {
        let access = Access::open(&mount(5002)).unwrap();
        let config = access.codex_config(&mount(5002));
        assert_eq!(
            config["mcp_servers"]["gespan"]["tool_timeout_sec"],
            CODEX_TOOL_TIMEOUT_SECS
        );
        // Longer than the browser holds a call beyond the action's limit.
        assert!(
            std::time::Duration::from_secs(CODEX_TOOL_TIMEOUT_SECS) > gespan_browser::HOST_WAIT
        );
    }

    #[test]
    fn acp_uses_http_only_when_the_agent_offers_it() {
        let access = Access::open(&mount(5003)).unwrap();
        let http = access.acp_servers(true);
        assert_eq!(http[0]["type"], "http");
        assert_eq!(http[0]["url"], access.url);
        assert_eq!(http[0]["headers"][0]["name"], "Authorization");
        let relay = access.acp_servers(false);
        assert!(relay[0].get("type").is_none(), "stdio has no discriminator");
        assert_eq!(relay[0]["args"], json!(["mcp-relay"]));
        assert_eq!(relay[0]["env"][1]["name"], ENV_TOKEN);
        assert_eq!(relay[0]["env"][1]["value"], secret_of(&access));
        // The relay's arguments never carry it.
        assert!(!relay[0]["args"].to_string().contains(&secret_of(&access)));
    }

    #[test]
    fn only_agents_with_no_way_in_take_the_environment_route() {
        assert_eq!(delivery(Kind::Claude, None), Delivery::ClaudeControl);
        assert_eq!(delivery(Kind::Codex, Some("t")), Delivery::CodexThread);
        for kind in [Kind::Cursor, Kind::Kiro, Kind::Hermes, Kind::Devin] {
            assert_eq!(delivery(kind, Some("s")), Delivery::AcpSession, "{kind:?}");
        }
        assert_eq!(delivery(Kind::Grok, None), Delivery::AcpSession);
        assert_eq!(delivery(Kind::Grok, Some("s")), Delivery::Environment);
        assert_eq!(delivery(Kind::Pi, None), Delivery::Environment);
        assert_eq!(delivery(Kind::Omp, None), Delivery::Environment);
        assert_eq!(delivery(Kind::OpenCode, Some("s")), Delivery::OpenCodeApi);
    }

    #[test]
    fn a_debug_print_never_shows_the_token() {
        let access = Access::open(&mount(5004)).unwrap();
        assert_eq!(format!("{access:?}"), "Access(..)");
    }

    #[test]
    fn the_environment_route_keeps_one_token_until_the_grant_ends() {
        let mount = mount(5005);
        let first = environment(&mount);
        assert_eq!(first, environment(&mount));
        assert!(first[ENV_URL].starts_with("http://127.0.0.1:"));
        revoke(5005);
        assert_ne!(first[ENV_TOKEN], environment(&mount)[ENV_TOKEN]);
    }

    #[test]
    fn the_environment_route_replaces_a_token_a_new_process_took_over() {
        let mount = mount(5007);
        let stale = environment(&mount);
        let access = Access::open(&mount).unwrap();
        let fresh = environment(&mount);
        assert_ne!(stale[ENV_TOKEN], fresh[ENV_TOKEN]);
        let connection = Connection {
            url: fresh[ENV_URL].clone(),
            token: fresh[ENV_TOKEN].clone(),
        };
        let reply = mcp_hub::call_tool(connection, "no_such_tool", json!({}));
        assert!(
            reply.unwrap_err().contains("isn't available"),
            "the credential itself was accepted"
        );
        assert!(!access.is_live(5007), "and it took over in turn");
    }

    #[test]
    fn a_turn_with_no_toolkits_takes_them_from_a_process_still_running() {
        let mut mount = mount(5008);
        let access = Access::open(&mount).unwrap();
        let connection = || Connection {
            url: access.url.clone(),
            token: access.token.secret().to_owned(),
        };
        let reachable = || {
            mcp_hub::call_tool(connection(), "visual_docs", json!({}))
                .map_err(|error| assert!(error.contains("isn't available"), "{error}"))
                .is_ok()
        };
        assert!(reachable());
        mount.toolkits.clear();
        refresh(&mount);
        assert!(
            access.is_live(5008),
            "the credential stays; its reach doesn't"
        );
        assert!(!reachable());
    }

    #[test]
    fn what_the_hub_learns_lands_in_the_catalogue() {
        gespan_tracker_secrets::testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::default();
        catalog.entries.push(crate::mcp_catalog::Entry {
            id: "docs".into(),
            ..Default::default()
        });
        catalog.save(dir.path()).unwrap();
        keep_tools(dir.path(), "docs", &[json!({"name":"who"})]);
        keep_class(dir.path(), "docs");
        keep_tools(dir.path(), "gone", &[]);
        let saved = Catalog::load(dir.path()).unwrap();
        assert_eq!(saved.entries[0].tools, Some(vec![json!({"name":"who"})]));
        assert_eq!(
            saved.entries[0].class,
            crate::mcp_catalog::Class::PerSession
        );
    }

    #[test]
    fn an_unreadable_catalogue_keeps_agents_off_their_own_servers() {
        gespan_tracker_secrets::testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            catalog_reach(Some(dir.path()), "/work"),
            (BTreeSet::new(), false),
            "no catalogue yet: nothing changes for the agent"
        );
        let mut catalog = Catalog {
            managed: true,
            ..Default::default()
        };
        catalog.entries.push(crate::mcp_catalog::Entry {
            id: "docs".into(),
            enabled: true,
            ..Default::default()
        });
        catalog.save(dir.path()).unwrap();
        assert_eq!(
            catalog_reach(Some(dir.path()), "/work"),
            (BTreeSet::from(["docs".to_owned()]), true)
        );
        std::fs::write(dir.path().join("mcp-catalog.json"), "not json").unwrap();
        assert_eq!(
            catalog_reach(Some(dir.path()), "/work"),
            (BTreeSet::new(), true)
        );
        assert_eq!(catalog_reach(None, "/work"), (BTreeSet::new(), false));
    }

    #[cfg(unix)]
    fn sleeper() -> ServerSpec {
        ServerSpec {
            id: "sleeper".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            env: Default::default(),
            sharing: mcp_hub::Sharing::Shared,
            tools: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_server_is_on_the_record_for_as_long_as_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("mcp-servers").join("1");
        let spawned = start_server(&sleeper(), None, Some(record.clone())).unwrap();
        let text = std::fs::read_to_string(&record).unwrap();
        assert_eq!(text.trim(), spawned.pid.to_string());
        (spawned.stop)();
        assert!(!record.exists(), "the record ends with its last server");
    }

    #[cfg(unix)]
    #[test]
    fn a_server_that_cant_be_recorded_doesnt_run() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let unwritable = file.join("mcp-servers").join("1");
        let refused = start_server(&sleeper(), None, Some(unwritable));
        assert!(refused.is_err());
    }

    /// The fake toolkit the original ticket asked for: registered once, it
    /// reaches every provider's shape, because they all name the same endpoint.
    #[test]
    fn a_new_toolkit_reaches_every_delivery_without_provider_changes() {
        struct Fake;
        impl mcp_hub::Toolkit for Fake {
            fn name(&self) -> &'static str {
                "fake"
            }
            fn tools(&self, _: &Grant) -> Vec<Value> {
                vec![json!({"name":"fake_ping","inputSchema":{"type":"object"}})]
            }
            fn call(&self, _: &Grant, _: &str, _: Value) -> Result<Value, String> {
                Ok(json!("pong"))
            }
        }
        hub().unwrap().register(Arc::new(Fake));
        let mut mount = mount(5006);
        mount.toolkits = BTreeSet::from(["fake".to_owned()]);
        let access = Access::open(&mount).unwrap();
        for delivered in [
            access.claude_control("x")["request"]["servers"]["gespan"]["url"].clone(),
            access.codex_config(&mount)["mcp_servers"]["gespan"]["url"].clone(),
            access.acp_servers(true)[0]["url"].clone(),
            access.opencode_server()["config"]["url"].clone(),
        ] {
            assert_eq!(delivered, access.url);
        }
        let reply = mcp_hub::call_tool(
            Connection {
                url: access.url.clone(),
                token: access.token.secret().to_owned(),
            },
            "fake_ping",
            json!({}),
        );
        assert_eq!(reply.unwrap(), json!("pong"));
    }
}
