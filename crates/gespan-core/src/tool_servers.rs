//! The app's own tools: one MCP hub, its toolkits, and how each provider is
//! told about it. A toolkit is offered only when the turn's grant names it.
//!
//! Adding a capability means registering a toolkit in `hub` and naming it in
//! the turn's [`Mount`]; no provider needs to change.
use crate::mcp_catalog::Catalog;
use crate::provider::Kind;
use mcp_hub::{Config, Connection, Grant, Hub, Observer, ServerSpec, Spawned, Spawner, Token};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
};

pub const VISUAL: &str = "visual";
pub const TICKET: &str = "ticket";
pub const APP: &str = "app";
pub const CONTROL: &str = "control";

/// The name agents know the hub by. Tools appear to Claude as
/// `mcp__gespan__<tool>`.
const SERVER_NAME: &str = "gespan";
/// Set on the relay process alone, by the agent that starts it, or for the
/// CLIs of agents that mount no MCP servers.
const ENV_URL: &str = "GESPAN_MCP_URL";
const ENV_TOKEN: &str = "GESPAN_MCP_TOKEN";

static HUB: OnceLock<Result<Hub, String>> = OnceLock::new();
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where the catalogue and the record of running servers live. Set once at
/// startup; without it the hub offers Gespan's own toolkits alone.
pub fn configure(data_dir: &Path) {
    let _ = DATA_DIR.set(data_dir.to_owned());
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
}

/// The processes the catalogue's servers are running as, for Settings.
pub fn running_servers() -> Vec<mcp_hub::Running> {
    match HUB.get() {
        Some(Ok(hub)) => hub.running(),
        _ => Vec::new(),
    }
}

/// Recent failures of the catalogue's servers, for Settings.
pub fn server_errors() -> Vec<mcp_hub::ServerError> {
    match HUB.get() {
        Some(Ok(hub)) => hub.errors(),
        _ => Vec::new(),
    }
}

/// The file listing server processes, read by the next launch's sweep.
pub fn running_record(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp-servers.pids")
}

/// One Mutex for the file, since servers start and stop on many threads.
fn edit_record(edit: impl FnOnce(&mut BTreeSet<u32>)) {
    static RECORD: Mutex<()> = Mutex::new(());
    let Some(data_dir) = DATA_DIR.get() else {
        return;
    };
    let _held = RECORD.lock().unwrap_or_else(|e| e.into_inner());
    let path = running_record(data_dir);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut pids: BTreeSet<u32> = text.lines().filter_map(|line| line.parse().ok()).collect();
    edit(&mut pids);
    let text: String = pids.iter().map(|pid| format!("{pid}\n")).collect();
    let _ = std::fs::write(path, text);
}

struct Processes;

impl Spawner for Processes {
    fn spawn(&self, spec: &ServerSpec, workspace: Option<&Path>) -> std::io::Result<Spawned> {
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
        edit_record(|pids| {
            pids.insert(pid);
        });
        let stdin = child.stdin.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
        let stdout = child.stdout.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
        Ok(Spawned {
            pid,
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stop: Box::new(move || {
                crate::process::terminate(&mut child);
                edit_record(|pids| {
                    pids.remove(&pid);
                });
            }),
        })
    }
}

/// Keeps what the hub learns about a server with its catalogue entry.
struct Keeper;

impl Keeper {
    fn edit(&self, edit: impl FnOnce(&mut Catalog)) {
        static EDITS: Mutex<()> = Mutex::new(());
        let Some(data_dir) = DATA_DIR.get() else {
            return;
        };
        let _held = EDITS.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(mut catalog) = Catalog::load(data_dir) {
            edit(&mut catalog);
            let _ = catalog.save(data_dir);
        }
    }
}

impl Observer for Keeper {
    fn demoted(&self, server: &str, method: &str) {
        tracing::info!(server, method, "a shared MCP server now runs per session");
        self.edit(|catalog| {
            if let Some(entry) = catalog.entry_mut(server) {
                entry.class = crate::mcp_catalog::Class::PerSession;
            }
        });
    }

    fn listed(&self, server: &str, tools: &[Value]) {
        self.edit(|catalog| {
            if let Some(entry) = catalog.entry_mut(server) {
                entry.tools = Some(tools.to_vec());
            }
        });
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

pub fn delivery(kind: Kind, provider_id: Option<&str>) -> Delivery {
    match kind {
        Kind::Claude => Delivery::ClaudeControl,
        Kind::Codex => Delivery::CodexThread,
        Kind::OpenCode => Delivery::OpenCodeApi,
        Kind::Pi | Kind::Omp => Delivery::Environment,
        // Their resume requests don't refresh mcpServers.
        Kind::Grok | Kind::Fx if provider_id.is_some() => Delivery::Environment,
        _ => Delivery::AcpSession,
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
            }),
        );
        json!({"mcp_servers":servers})
    }

    pub fn opencode_server(&self) -> Value {
        json!({"name":SERVER_NAME,"config":{
            "type":"remote",
            "url":self.url,
            "headers":{"Authorization":self.token.bearer()},
            "oauth":false,
        }})
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
    let Ok(hub) = hub() else {
        return HashMap::new();
    };
    let mut issued = ISSUED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !hub.has_grant(mount.session) {
        issued.remove(&mount.session);
    }
    let token = issued
        .entry(mount.session)
        .or_insert_with(|| hub.mint(mount.grant()));
    HashMap::from([
        (ENV_URL.to_owned(), hub.url().to_owned()),
        (ENV_TOKEN.to_owned(), token.secret().to_owned()),
    ])
}

/// Applies this turn's reach to a session already holding a credential. An
/// agent still running sees its tool list change.
pub fn refresh(mount: &Mount) {
    if let Ok(hub) = hub() {
        hub.update(mount.session, mount.toolkits.clone(), mount.servers.clone());
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
