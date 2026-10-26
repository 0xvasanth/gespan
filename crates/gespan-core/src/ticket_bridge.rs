//! Session-scoped ticket and card tools for coding agents. The stdio server
//! forwards each call over loopback to the app, which acts through `Tickets`
//! with the keychain credentials (or the board store, for card tools); the
//! agent only ever holds a per-session token. The tools themselves and their
//! refusal rules live in `gespan_tracker_client::tools`, shared with Tauri.
use crate::tickets::TicketRef;
use gespan_board_store::CardTool;
pub use gespan_tracker_client::tools::{answer, Answered, Effect, TicketTool};
use gespan_tracker_client::tools::{refusal, NO_TICKET, READ_ONLY};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const MAX_WIRE_BYTES: usize = 1_048_576;
/// Tracker calls are slow but bounded; past this the agent gets an error.
const APP_TIMEOUT: Duration = Duration::from_secs(60);
const ENV_ENDPOINT: &str = "GESPAN_TICKET_ENDPOINT";
const ENV_TOKEN: &str = "GESPAN_TICKET_TOKEN";
const ENV_SESSION: &str = "GESPAN_TICKET_SESSION";
const ENV_EXECUTABLE: &str = "GESPAN_TICKET_EXECUTABLE";
pub(crate) const ENV_KEYS: [&str; 4] = [ENV_ENDPOINT, ENV_TOKEN, ENV_SESSION, ENV_EXECUTABLE];
const APP_GONE: &str = "Gespan stopped handling this session's ticket tools";
const NO_CARDS: &str = "Card tools aren't available in this session";

/// What one turn may reach, set by the app and never by the agent. The
/// ticket tools exist only with a `ticket`; the card tools only with `cards`.
/// `read_only` sessions (read-only access or plan mode) may read and list
/// but never comment, move the ticket or tick a subtask.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub ticket: Option<TicketRef>,
    pub may_change_status: bool,
    pub read_only: bool,
    pub cards: bool,
}

/// A tool call as it crosses the bridge: a ticket tool or a card tool.
#[derive(Clone, Debug, PartialEq)]
pub enum BridgeTool {
    Ticket(TicketTool),
    Card(CardTool),
}

impl From<TicketTool> for BridgeTool {
    fn from(tool: TicketTool) -> Self {
        Self::Ticket(tool)
    }
}

impl From<CardTool> for BridgeTool {
    fn from(tool: CardTool) -> Self {
        Self::Card(tool)
    }
}

impl Serialize for BridgeTool {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Ticket(tool) => tool.serialize(serializer),
            Self::Card(CardTool::Subtasks) => json!({"tool":"card_subtasks"}).serialize(serializer),
            Self::Card(CardTool::SubtaskDone { subtask_id, done }) => {
                json!({"tool":"card_subtask_done","subtaskId":subtask_id,"done":done})
                    .serialize(serializer)
            }
        }
    }
}

/// The token is in the agent's environment, so the hub reads this wire from
/// a caller it can't trust: every tool refuses keys it doesn't define.
impl<'de> Deserialize<'de> for BridgeTool {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let name = value
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| serde::de::Error::custom("a tool needs a name"))?;
        if name.starts_with("card_") {
            return serde_json::from_value(value)
                .map(Self::Card)
                .map_err(serde::de::Error::custom);
        }
        let allowed: &[&str] = match name {
            "get" | "statuses" => &["tool"],
            "comment" => &["tool", "markdown"],
            "set_status" => &["tool", "target", "fields"],
            _ => return Err(serde::de::Error::custom("unknown ticket tool")),
        };
        if let Some(extra) = value
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys())
            .find(|key| !allowed.contains(&key.as_str()))
        {
            return Err(serde::de::Error::custom(format!("unknown field `{extra}`")));
        }
        serde_json::from_value(value)
            .map(Self::Ticket)
            .map_err(serde::de::Error::custom)
    }
}

/// Why a card tool may not run in this turn, before it reaches the app.
fn card_refusal(tool: &CardTool, read_only: bool) -> Option<&'static str> {
    (read_only && matches!(tool, CardTool::SubtaskDone { .. })).then_some(READ_ONLY)
}

pub struct Call {
    pub tool: BridgeTool,
    /// The session's ticket when the hub accepted the call, for ticket tools;
    /// the app answers on this link, never on a key the agent named.
    pub ticket: Option<TicketRef>,
    pub reply: mpsc::Sender<Result<Value, String>>,
    generation: u64,
}

/// A request without a tool asks for the session's scope.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    session: i64,
    token: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    tool: Option<BridgeTool>,
}

struct ActiveTurn {
    generation: u64,
    send: mpsc::SyncSender<Call>,
    scope: Scope,
}

struct SessionCapability {
    token: String,
    generation: u64,
    active: Option<ActiveTurn>,
}

struct Hub {
    endpoint: String,
    sessions: Mutex<HashMap<i64, SessionCapability>>,
}

fn hub() -> Result<Arc<Hub>, String> {
    static HUB: OnceLock<Mutex<Option<Arc<Hub>>>> = OnceLock::new();
    let mut slot = HUB
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Ticket tools unavailable")?;
    if let Some(hub) = &*slot {
        return Ok(hub.clone());
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let endpoint = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let hub = Arc::new(Hub {
        endpoint,
        sessions: Mutex::new(HashMap::new()),
    });
    let owner = hub.clone();
    let connections = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            if connections.fetch_add(1, Ordering::Relaxed) >= 16 {
                connections.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let owner = owner.clone();
            let connections = connections.clone();
            std::thread::spawn(move || {
                let _ = serve(stream, &owner);
                connections.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    *slot = Some(hub.clone());
    Ok(hub)
}

/// One turn's ticket capability. The token outlives turns so a long-lived
/// provider keeps working; the scope is this turn's, read at each call.
pub struct Server {
    hub: Arc<Hub>,
    token: String,
    session: i64,
    generation: u64,
    calls: mpsc::Receiver<Call>,
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.hub.sessions.lock() {
            if let Some(session) = sessions.get_mut(&self.session) {
                if session.generation == self.generation {
                    session.active = None;
                }
            }
        }
    }
}

impl Server {
    /// Refuses a scope with neither a ticket nor cards: there would be no tools.
    pub fn start(session: i64, scope: Scope) -> Result<Self, String> {
        let _span =
            tracing::info_span!("mcp_start", server = "ticket", session_id = session).entered();
        Self::open(session, scope)
            .inspect_err(|error| tracing::warn!(%error, "could not start a tool server"))
    }

    fn open(session: i64, scope: Scope) -> Result<Self, String> {
        if scope.ticket.is_none() && !scope.cards {
            return Err("No ticket or cards for this session's tools".into());
        }
        let hub = hub()?;
        let (send, calls) = mpsc::sync_channel(16);
        let (token, generation) = {
            let mut sessions = hub
                .sessions
                .lock()
                .map_err(|_| "Ticket session unavailable")?;
            let capability = sessions
                .entry(session)
                .or_insert_with(|| SessionCapability {
                    token: format!(
                        "{:032x}{:032x}",
                        rand::random::<u128>(),
                        rand::random::<u128>()
                    ),
                    generation: 0,
                    active: None,
                });
            if capability.active.is_some() {
                return Err("A ticket session turn is already active".into());
            }
            capability.generation = capability.generation.wrapping_add(1);
            capability.active = Some(ActiveTurn {
                generation: capability.generation,
                send,
                scope,
            });
            (capability.token.clone(), capability.generation)
        };
        Ok(Self {
            hub,
            token,
            session,
            generation,
            calls,
        })
    }

    pub fn environment(&self, executable: &std::path::Path) -> HashMap<String, String> {
        HashMap::from([
            (ENV_ENDPOINT.into(), self.hub.endpoint.clone()),
            (ENV_TOKEN.into(), self.token.clone()),
            (ENV_SESSION.into(), self.session.to_string()),
            (
                ENV_EXECUTABLE.into(),
                executable.to_string_lossy().into_owned(),
            ),
        ])
    }

    /// The session's ticket as of now.
    pub fn ticket(&self) -> Option<TicketRef> {
        self.hub
            .sessions
            .lock()
            .ok()?
            .get(&self.session)?
            .active
            .as_ref()
            .filter(|active| active.generation == self.generation)?
            .scope
            .ticket
            .clone()
    }

    /// Replaces this turn's scope, for a link or permission the user changes
    /// while the turn runs. Calls from then on use the new scope. A turn that
    /// has already ended changes nothing.
    pub fn set_scope(&self, scope: Scope) {
        let Ok(mut sessions) = self.hub.sessions.lock() else {
            return;
        };
        if let Some(active) = sessions
            .get_mut(&self.session)
            .and_then(|session| session.active.as_mut())
            .filter(|active| active.generation == self.generation)
        {
            active.scope = scope;
        }
    }

    pub fn drain(&self) -> Vec<Call> {
        self.calls
            .try_iter()
            .filter_map(|call| {
                if call.generation == self.generation
                    && active_generation(&self.hub, self.session) == Some(self.generation)
                {
                    Some(call)
                } else {
                    let _ = call.reply.send(Err(APP_GONE.into()));
                    None
                }
            })
            .collect()
    }
}

fn active_generation(hub: &Hub, session: i64) -> Option<u64> {
    hub.sessions
        .lock()
        .ok()?
        .get(&session)?
        .active
        .as_ref()
        .map(|active| active.generation)
}

fn same_key(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn serve(mut stream: TcpStream, hub: &Hub) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new((&stream).take((MAX_WIRE_BYTES + 1) as u64))
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let result = handle(&line, hub);
    let response = match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    writeln!(stream, "{response}").map_err(|e| e.to_string())
}

fn handle(line: &str, hub: &Hub) -> Result<Value, String> {
    if line.len() > MAX_WIRE_BYTES || !line.ends_with('\n') {
        return Err("Invalid ticket request".into());
    }
    let wire: Wire = serde_json::from_str(line).map_err(|_| "Invalid ticket request")?;
    let (generation, result) = {
        let sessions = hub
            .sessions
            .lock()
            .map_err(|_| "Ticket session unavailable")?;
        let capability = sessions
            .get(&wire.session)
            .ok_or("Invalid ticket capability")?;
        let secret = &capability.token;
        let equal = wire.token.len() == secret.len()
            && wire
                .token
                .bytes()
                .zip(secret.bytes())
                .fold(0u8, |diff, (a, b)| diff | a ^ b)
                == 0;
        if !equal {
            return Err("Invalid ticket capability".into());
        }
        let active = capability.active.as_ref().ok_or(APP_GONE)?;
        let scope = &active.scope;
        // Card tools come first: their scope is the session's cards, so a
        // key (which only names tickets) neither helps nor refuses them.
        if let Some(BridgeTool::Card(tool)) = &wire.tool {
            if !scope.cards {
                return Err(NO_CARDS.into());
            }
            if let Some(refused) = card_refusal(tool, scope.read_only) {
                return Err(refused.into());
            }
        }
        // The token is in the agent's environment, so scope is enforced here
        // and not only by the stdio server.
        if let Some(key) = wire
            .key
            .as_ref()
            .filter(|_| !matches!(wire.tool, Some(BridgeTool::Card(_))))
        {
            match &scope.ticket {
                Some(ticket) if same_key(key, &ticket.key) => {}
                Some(ticket) => return Err(format!("This session is linked to {}", ticket.key)),
                None => return Err(NO_TICKET.into()),
            }
        }
        let Some(tool) = wire.tool else {
            return Ok(json!({
                "key": scope.ticket.as_ref().map(|ticket| &ticket.key),
                "mayChangeStatus": scope.may_change_status,
                "readOnly": scope.read_only,
                "cards": scope.cards,
            }));
        };
        let ticket = match &tool {
            BridgeTool::Ticket(tool) => {
                let ticket = scope.ticket.clone().ok_or(NO_TICKET)?;
                if let Some(refused) = refusal(tool, scope.may_change_status, scope.read_only) {
                    return Err(refused.into());
                }
                Some(ticket)
            }
            // Checked above.
            BridgeTool::Card(_) => None,
        };
        let (reply, result) = mpsc::channel();
        active
            .send
            .try_send(Call {
                tool,
                ticket,
                reply,
                generation: active.generation,
            })
            .map_err(|_| "Too many ticket calls are waiting; retry shortly")?;
        (active.generation, result)
    };
    let started = Instant::now();
    loop {
        if active_generation(hub, wire.session) != Some(generation) {
            return Err(APP_GONE.into());
        }
        match result.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(APP_GONE.into()),
            Err(mpsc::RecvTimeoutError::Timeout) if started.elapsed() < APP_TIMEOUT => {}
            Err(_) => return Err("Gespan did not answer the ticket call in time".into()),
        }
    }
}

/// One loopback round trip. `tool: None` asks for the session's scope.
fn request(
    environment: &HashMap<String, String>,
    key: Option<&str>,
    tool: Option<BridgeTool>,
) -> Result<Value, String> {
    let get = |name| {
        environment
            .get(name)
            .ok_or("Ticket tools need a running Gespan session")
    };
    let endpoint: SocketAddr = get(ENV_ENDPOINT)?
        .parse()
        .map_err(|_| "Invalid ticket endpoint")?;
    if !endpoint.ip().is_loopback() {
        return Err("Ticket endpoint must be loopback".into());
    }
    let wire = Wire {
        session: get(ENV_SESSION)?
            .parse()
            .map_err(|_| "Invalid ticket session")?,
        token: get(ENV_TOKEN)?.clone(),
        key: key.map(str::to_owned),
        tool,
    };
    let mut stream =
        TcpStream::connect_timeout(&endpoint, Duration::from_secs(3)).map_err(|_| APP_GONE)?;
    stream
        .set_read_timeout(Some(APP_TIMEOUT + Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    writeln!(
        stream,
        "{}",
        serde_json::to_string(&wire).map_err(|e| e.to_string())?
    )
    .map_err(|_| APP_GONE)?;
    let mut response = String::new();
    BufReader::new(stream.take(MAX_WIRE_BYTES as u64))
        .read_line(&mut response)
        .map_err(|_| APP_GONE)?;
    let response: Value = serde_json::from_str(&response).map_err(|_| APP_GONE)?;
    if response["ok"] != true {
        return Err(response["error"]
            .as_str()
            .unwrap_or("Ticket call failed")
            .into());
    }
    Ok(response["result"].clone())
}

fn current_environment() -> HashMap<String, String> {
    ENV_KEYS
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
        .collect()
}

/// The tools a session's scope offers: ticket tools with a ticket, card
/// tools with cards.
pub fn tool_definitions(ticket: bool, cards: bool) -> Vec<Value> {
    let mut tools = Vec::new();
    if ticket {
        tools.extend(ticket_definitions());
    }
    if cards {
        tools.extend(card_definitions());
    }
    tools
}

fn card_definitions() -> Vec<Value> {
    vec![
        json!({"name":"card_subtasks","description":"List the subtasks of the board cards linked to this session, with each subtask's id and whether it is done.","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
        json!({"name":"card_subtask_done","description":"Mark a subtask on a board card linked to this session as done or not done. Use a subtask id from card_subtasks.","inputSchema":{"type":"object","properties":{"subtaskId":{"type":"string"},"done":{"type":"boolean"}},"required":["subtaskId","done"],"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false}}),
    ]
}

fn ticket_definitions() -> Vec<Value> {
    let key = json!({"type":"string","description":"Optional. Must be this session's ticket key."});
    vec![
        json!({"name":"ticket_get","description":"Read the ticket linked to this session: key, title, status, link and description (Markdown).","inputSchema":{"type":"object","properties":{"key":key},"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":true}}),
        json!({"name":"ticket_comment","description":"Post a Markdown comment on the ticket linked to this session. Returns the comment's link.","inputSchema":{"type":"object","properties":{"body":{"type":"string","description":"Comment in Markdown"},"key":key},"required":["body"],"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":true}}),
        json!({"name":"ticket_status","description":"Without target, list the statuses the linked ticket can move to. With target (a status id or name from that list), move the ticket. Some moves need fields, passed by field key.","inputSchema":{"type":"object","properties":{"target":{"type":"string"},"fields":{"type":"object","description":"Values for fields a move requires, by field key"},"key":key},"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":true}}),
    ]
}

/// The stdio side. It checks scope before forwarding, so a refused call never
/// reaches the app; the hub checks again for callers that skip this process.
struct Client {
    environment: HashMap<String, String>,
}

impl Client {
    fn new(environment: HashMap<String, String>) -> Self {
        Self { environment }
    }

    fn call(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        if !arguments.is_object() && !arguments.is_null() {
            return Err("Arguments must be an object".into());
        }
        let text = |field: &str| -> Result<Option<&str>, String> {
            match arguments.get(field) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) => Ok(Some(value)),
                Some(_) => Err(format!("{field} must be a string")),
            }
        };
        if name.starts_with("card_") {
            return self.call_card(CardTool::from_call(name, arguments)?);
        }
        let allowed: &[&str] = match name {
            "ticket_get" => &["key"],
            "ticket_comment" => &["body", "key"],
            "ticket_status" => &["target", "fields", "key"],
            _ => return Err("Unknown ticket tool".into()),
        };
        if let Some(extra) = arguments
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys())
            .find(|key| !allowed.contains(&key.as_str()))
        {
            return Err(format!("Unknown argument: {extra}"));
        }
        let key = text("key")?;
        let tool = match name {
            "ticket_get" => TicketTool::Get,
            "ticket_comment" => {
                let body = text("body")?.unwrap_or_default();
                if body.trim().is_empty() {
                    return Err("body must be a non-empty Markdown string".into());
                }
                TicketTool::Comment {
                    markdown: body.to_owned(),
                }
            }
            "ticket_status" => {
                let fields = match arguments.get("fields") {
                    None | Some(Value::Null) => BTreeMap::new(),
                    Some(Value::Object(fields)) => fields.clone().into_iter().collect(),
                    Some(_) => return Err("fields must be an object".into()),
                };
                match text("target")?.filter(|target| !target.trim().is_empty()) {
                    Some(target) => TicketTool::SetStatus {
                        target: target.to_owned(),
                        fields,
                    },
                    None if fields.is_empty() => TicketTool::Statuses,
                    None => return Err("fields need a target status".into()),
                }
            }
            _ => return Err("Unknown ticket tool".into()),
        };
        // Read per call: the link may change between turns of a long-lived provider.
        let scope = request(&self.environment, None, None)?;
        let linked = scope["key"].as_str().ok_or(NO_TICKET)?;
        if key.is_some_and(|key| !same_key(key, linked)) {
            return Err(format!("This session is linked to {linked}"));
        }
        let may_change_status = scope["mayChangeStatus"] == true;
        let read_only = scope["readOnly"] == true;
        if let Some(refused) = refusal(&tool, may_change_status, read_only) {
            return Err(refused.into());
        }
        let statuses = matches!(tool, TicketTool::Statuses);
        let mut result = request(&self.environment, key, Some(tool.into()))?;
        if statuses {
            result["mayChangeStatus"] = (may_change_status && !read_only).into();
        }
        Ok(result)
    }

    fn call_card(&self, tool: CardTool) -> Result<Value, String> {
        // Read per call, like the ticket link: cards may be linked mid-session.
        let scope = request(&self.environment, None, None)?;
        if scope["cards"] != true {
            return Err(NO_CARDS.into());
        }
        if let Some(refused) = card_refusal(&tool, scope["readOnly"] == true) {
            return Err(refused.into());
        }
        request(&self.environment, None, Some(tool.into()))
    }

    /// The scope decides the catalog, so a session without a ticket or cards
    /// is never offered tools that can only refuse. When the scope can't be
    /// read (no active turn yet, say), every tool is listed: each call is
    /// checked against the live scope anyway.
    fn catalog(&self) -> Value {
        let tools = match request(&self.environment, None, None) {
            Ok(scope) => tool_definitions(scope["key"].is_string(), scope["cards"] == true),
            Err(_) => tool_definitions(true, true),
        };
        json!({ "tools": tools })
    }

    fn mcp_response(&self, request: Value) -> Option<Value> {
        let id = request.get("id")?.clone();
        let result = match request["method"].as_str().unwrap_or("") {
            "initialize" => Ok(
                json!({"protocolVersion":request["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"gespan_ticket","version":env!("CARGO_PKG_VERSION")}}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.catalog()),
            "tools/call" => Ok(
                match self.call(
                    request["params"]["name"].as_str().unwrap_or(""),
                    &request["params"]["arguments"],
                ) {
                    Ok(value) => {
                        json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                    }
                    Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
                },
            ),
            _ => Err(json!({"code":-32601,"message":"Method not found"})),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
        })
    }
}

/// Runs before GUI initialization. stdout carries MCP JSON-RPC frames only.
pub fn mcp_stdio() -> Result<(), String> {
    let client = Client::new(current_environment());
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut line = String::new();
        (&mut input)
            .take((MAX_WIRE_BYTES + 1) as u64)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if line.is_empty() {
            return Ok(());
        }
        if line.len() > MAX_WIRE_BYTES || !line.ends_with('\n') {
            return Err("Oversized or incomplete MCP frame".into());
        }
        let response = match serde_json::from_str(&line) {
            Ok(request) => client.mcp_response(request),
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let Some(response) = response {
            writeln!(output, "{response}")
                .and_then(|_| output.flush())
                .map_err(|e| e.to_string())?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_boards::fake::FakeProvider;
    use crate::provider_boards::{
        BoardProvider, ProviderResult, RemoteIssue, RequiredField, StateGroup, Transition,
        WorkflowState,
    };
    use crate::tickets::{Connection, Tickets, Tracker};
    use gespan_board_store::CardTool;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct Conn {
        fake: Arc<FakeProvider>,
        transitions: Option<Vec<Transition>>,
    }

    impl Connection for Conn {
        fn board(&self) -> &dyn BoardProvider {
            &*self.fake
        }

        fn transitions(&self, _: &str) -> ProviderResult<Option<Vec<Transition>>> {
            Ok(self.transitions.clone())
        }
    }

    fn issue(key: &str) -> RemoteIssue {
        RemoteIssue {
            id: key.into(),
            key: key.into(),
            title: "Fix login".into(),
            url: format!("https://tracker.test/{key}"),
            container_id: "C1".into(),
            updated_at: "2026-03-01T10:00:00Z".into(),
            state: WorkflowState {
                id: "todo".into(),
                name: "Todo".into(),
                group: StateGroup::Unstarted,
            },
            description: Some("Login fails on **Safari**".into()),
            ..Default::default()
        }
    }

    fn linked(key: &str) -> TicketRef {
        TicketRef {
            tracker: Tracker::Linear,
            key: key.into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        }
    }

    fn tickets(fake: Arc<FakeProvider>, transitions: Option<Vec<Transition>>) -> Tickets {
        gespan_tracker_secrets::testing::use_memory_keychain();
        Tickets::with_providers(
            PathBuf::from("/unused"),
            vec![(Tracker::Linear, Arc::new(Conn { fake, transitions }))],
        )
    }

    fn tool_call(name: &str, arguments: Value) -> Value {
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":name,"arguments":arguments}})
    }

    fn client(server: &Server) -> Client {
        Client::new(server.environment(Path::new("/test/gespan-gpui")))
    }

    /// Runs the call on a worker, as the stdio process would, while this
    /// thread plays the app and answers through `tickets`.
    fn call_answered(server: &Server, tickets: &Tickets, request: Value) -> Value {
        answered_by(server, request, |call| match &call.tool {
            BridgeTool::Ticket(tool) => {
                let ticket = call
                    .ticket
                    .as_ref()
                    .expect("a ticket call carries its link");
                answer(tickets, ticket, tool).map(|answered| answered.value)
            }
            BridgeTool::Card(_) => Err("not a ticket call".into()),
        })
    }

    /// Like `call_answered`, with the app's answer supplied by `app`.
    fn answered_by(
        server: &Server,
        request: Value,
        app: impl Fn(&Call) -> Result<Value, String>,
    ) -> Value {
        let client = client(server);
        let worker = std::thread::spawn(move || client.mcp_response(request).unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() && Instant::now() < deadline {
            for call in server.drain() {
                let _ = call.reply.send(app(&call));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        worker.join().unwrap()["result"].clone()
    }

    fn scope(key: &str, may_change_status: bool, read_only: bool) -> Scope {
        Scope {
            ticket: Some(linked(key)),
            may_change_status,
            read_only,
            cards: false,
        }
    }

    fn with_cards(ticket: Option<&str>, read_only: bool) -> Scope {
        Scope {
            ticket: ticket.map(linked),
            may_change_status: true,
            read_only,
            cards: true,
        }
    }

    fn listed(server: &Server) -> Vec<Value> {
        let catalog = client(server)
            .mcp_response(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .unwrap();
        catalog["result"]["tools"].as_array().unwrap().clone()
    }

    fn names(tools: &[Value]) -> Vec<String> {
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    /// One raw line to the hub, as a caller holding the token could send.
    fn raw(server: &Server, wire: Value) -> Result<Value, String> {
        let environment = server.environment(Path::new("/test/app"));
        let mut wire = wire;
        wire["session"] = environment[ENV_SESSION].parse::<i64>().unwrap().into();
        wire["token"] = environment[ENV_TOKEN].clone().into();
        handle(&format!("{wire}\n"), &hub().unwrap())
    }

    fn text(result: &Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    #[test]
    fn ticket_tools_list_three_tools() {
        let server = Server::start(401, scope("ENG-1", true, false)).unwrap();
        let catalog = client(&server)
            .mcp_response(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .unwrap();
        let tools = catalog["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["ticket_get", "ticket_comment", "ticket_status"]);
        for tool in tools {
            assert_eq!(tool["inputSchema"]["properties"]["key"]["type"], "string");
        }
        assert_eq!(tools[1]["inputSchema"]["required"], json!(["body"]));
        assert!(tools[2]["inputSchema"]["required"].is_null());
        assert!(tools[2]["inputSchema"]["properties"]["target"].is_object());
        assert!(tools[2]["inputSchema"]["properties"]["fields"].is_object());
        let init = client(&server)
            .mcp_response(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}))
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "gespan_ticket");
        assert!(client(&server)
            .mcp_response(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none());
    }

    #[test]
    fn ticket_get_returns_key_title_status_description() {
        let server = Server::start(402, scope("ENG-1", true, false)).unwrap();
        let tickets = tickets(
            Arc::new(FakeProvider::new().with_issue(issue("ENG-1"))),
            None,
        );
        let result = call_answered(&server, &tickets, tool_call("ticket_get", json!({})));
        assert_eq!(result["isError"], false, "{result}");
        let ticket: Value = serde_json::from_str(text(&result)).unwrap();
        assert_eq!(
            ticket,
            json!({
                "key": "ENG-1",
                "tracker": "Linear",
                "title": "Fix login",
                "status": "Todo",
                "url": "https://tracker.test/ENG-1",
                "description": "Login fails on **Safari**",
            })
        );
        // The session's own key, in any case, is the same ticket.
        let result = call_answered(
            &server,
            &tickets,
            tool_call("ticket_get", json!({"key":"eng-1"})),
        );
        assert_eq!(result["isError"], false, "{result}");
    }

    #[test]
    fn ticket_comment_reaches_the_app_and_returns_url() {
        let server = Server::start(403, scope("ENG-1", true, false)).unwrap();
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let tickets = tickets(fake.clone(), None);
        let result = call_answered(
            &server,
            &tickets,
            tool_call("ticket_comment", json!({"body":"Fixed in `abc123`"})),
        );
        assert_eq!(result["isError"], false, "{result}");
        let comments = fake.comments("ENG-1");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].1, "Fixed in `abc123`");
        let reply: Value = serde_json::from_str(text(&result)).unwrap();
        assert_eq!(reply["url"], comments[0].0.url);
        assert!(!comments[0].0.url.is_empty());
        let empty = call_answered(
            &server,
            &tickets,
            tool_call("ticket_comment", json!({"body":"  "})),
        );
        assert_eq!(empty["isError"], true);
        assert_eq!(fake.comments("ENG-1").len(), 1);
    }

    #[test]
    fn ticket_tools_refuse_another_key() {
        let server = Server::start(404, scope("ABC-1", true, false)).unwrap();
        for request in [
            tool_call("ticket_get", json!({"key":"ABC-99"})),
            tool_call("ticket_comment", json!({"key":"ABC-99","body":"hi"})),
            tool_call("ticket_status", json!({"key":"ABC-99","target":"Done"})),
        ] {
            let response = client(&server).mcp_response(request).unwrap();
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(text(&response["result"]), "This session is linked to ABC-1");
        }
        assert!(server.drain().is_empty(), "No call may reach the app");
    }

    #[test]
    fn hub_refuses_another_key_without_the_stdio_process() {
        // The token is in the agent's environment, so the hub is the boundary.
        let server = Server::start(405, scope("ABC-1", true, false)).unwrap();
        let environment = server.environment(Path::new("/test/app"));
        let error =
            request(&environment, Some("ABC-99"), Some(TicketTool::Get.into())).unwrap_err();
        assert_eq!(error, "This session is linked to ABC-1");
        assert!(server.drain().is_empty());
    }

    #[test]
    fn status_change_refused_when_not_allowed() {
        let server = Server::start(406, scope("ENG-1", false, false)).unwrap();
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let tickets = tickets(fake.clone(), None);
        let listing = call_answered(&server, &tickets, tool_call("ticket_status", json!({})));
        assert_eq!(listing["isError"], false, "{listing}");
        let listing: Value = serde_json::from_str(text(&listing)).unwrap();
        let names: Vec<_> = listing["statuses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|status| status["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Todo", "Doing", "Done"]);
        assert_eq!(listing["mayChangeStatus"], false);
        let response = client(&server)
            .mcp_response(tool_call("ticket_status", json!({"target":"Done"})))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            text(&response["result"]),
            "Changing ticket status is turned off for this session"
        );
        assert!(server.drain().is_empty());
        // Nor can the agent bypass the stdio process with its token.
        let environment = server.environment(Path::new("/test/app"));
        let error = request(
            &environment,
            None,
            Some(
                TicketTool::SetStatus {
                    target: "Done".into(),
                    fields: BTreeMap::new(),
                }
                .into(),
            ),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "Changing ticket status is turned off for this session"
        );
        assert!(server.drain().is_empty());
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
    }

    #[test]
    fn allowed_status_change_moves_the_ticket() {
        let server = Server::start(407, scope("ENG-1", true, false)).unwrap();
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let tickets = tickets(fake.clone(), None);
        let result = call_answered(
            &server,
            &tickets,
            tool_call("ticket_status", json!({"target":"done"})),
        );
        assert_eq!(result["isError"], false, "{result}");
        let moved: Value = serde_json::from_str(text(&result)).unwrap();
        assert_eq!(moved["status"], "Done");
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Done");
    }

    #[test]
    fn needs_fields_is_reported_as_tool_error_with_field_list() {
        let server = Server::start(408, scope("ENG-1", true, false)).unwrap();
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let transition = Transition {
            // The fake moves by state id, so the transition shares it.
            id: "done".into(),
            name: "Finish".into(),
            to: WorkflowState {
                id: "done".into(),
                name: "Done".into(),
                group: StateGroup::Completed,
            },
            required_fields: vec![RequiredField {
                key: "resolution".into(),
                name: "Resolution".into(),
                schema_type: "option".into(),
                allowed: vec!["Fixed".into(), "Won't Do".into()],
            }],
        };
        let tickets = tickets(fake.clone(), Some(vec![transition]));
        let listing = call_answered(&server, &tickets, tool_call("ticket_status", json!({})));
        let listing: Value = serde_json::from_str(text(&listing)).unwrap();
        assert_eq!(listing["statuses"][0]["id"], "done");
        assert_eq!(
            listing["statuses"][0]["requiredFields"][0]["key"],
            "resolution"
        );
        let result = call_answered(
            &server,
            &tickets,
            tool_call("ticket_status", json!({"target":"done"})),
        );
        assert_eq!(result["isError"], true);
        assert_eq!(
            text(&result),
            "Moving this ticket needs: Resolution. Pass them in fields by key: resolution (Resolution; one of: Fixed, Won't Do)"
        );
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
        let result = call_answered(
            &server,
            &tickets,
            tool_call(
                "ticket_status",
                json!({"target":"done","fields":{"resolution":{"name":"Fixed"}}}),
            ),
        );
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Done");
    }

    #[test]
    fn bad_token_is_rejected() {
        let server = Server::start(409, scope("ENG-1", true, false)).unwrap();
        let mut environment = server.environment(Path::new("/test/app"));
        let token = environment[ENV_TOKEN].clone();
        assert!(token.len() >= 32, "at least 128 bits");
        let mut forged = token.clone();
        forged.replace_range(..1, if token.starts_with('0') { "1" } else { "0" });
        environment.insert(ENV_TOKEN.into(), forged);
        let response = Client::new(environment.clone())
            .mcp_response(tool_call("ticket_get", json!({})))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(text(&response["result"]), "Invalid ticket capability");
        environment.insert(ENV_TOKEN.into(), token[..8].into());
        assert!(request(&environment, None, Some(TicketTool::Get.into())).is_err());
        environment.insert(ENV_TOKEN.into(), token);
        environment.insert(ENV_SESSION.into(), "999409".into());
        assert!(request(&environment, None, Some(TicketTool::Get.into())).is_err());
        assert!(server.drain().is_empty());
    }

    #[test]
    fn app_gone_returns_error_not_hang() {
        let server = Server::start(410, scope("ENG-1", true, false)).unwrap();
        let client = client(&server);
        let worker = std::thread::spawn(move || {
            client
                .mcp_response(tool_call("ticket_get", json!({})))
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let held = loop {
            let mut calls = server.drain();
            if let Some(call) = calls.pop() {
                break call;
            }
            assert!(Instant::now() < deadline, "The call never reached the app");
            std::thread::sleep(Duration::from_millis(5));
        };
        let dropped = Instant::now();
        drop(server);
        let response = worker.join().unwrap();
        assert!(dropped.elapsed() < Duration::from_secs(1));
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            text(&response["result"]),
            "Gespan stopped handling this session's ticket tools"
        );
        drop(held);
        // With the turn over, a new call fails at once instead of waiting.
        let started = Instant::now();
        let environment = Server::start(411, scope("ENG-1", true, false))
            .unwrap()
            .environment(Path::new("/test/app"));
        assert!(request(&environment, None, Some(TicketTool::Get.into())).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_restarted_turn_keeps_the_token_but_takes_the_new_ticket() {
        let first = Server::start(412, scope("ENG-1", true, false)).unwrap();
        let environment = first.environment(Path::new("/test/app"));
        assert!(Server::start(412, scope("ENG-1", true, false)).is_err());
        drop(first);
        let next = Server::start(412, scope("ENG-2", false, false)).unwrap();
        assert_eq!(environment, next.environment(Path::new("/test/app")));
        assert_eq!(
            request(&environment, None, None).unwrap(),
            json!({"key":"ENG-2","mayChangeStatus":false,"readOnly":false,"cards":false})
        );
    }

    #[test]
    fn tool_results_and_server_logs_never_carry_the_token() {
        let server = Server::start(413, scope("ENG-1", true, false)).unwrap();
        let environment = server.environment(Path::new("/test/app"));
        let tickets = tickets(
            Arc::new(FakeProvider::new().with_issue(issue("ENG-1"))),
            None,
        );
        let mut result = Value::Null;
        let logged = crate::diagnostics::test_capture(|| {
            result = call_answered(&server, &tickets, tool_call("ticket_get", json!({})));
        });
        assert!(!result.to_string().contains(&environment[ENV_TOKEN]));
        assert!(!format!("{logged:?}").contains(&environment[ENV_TOKEN]));
    }

    #[test]
    fn read_only_sessions_can_read_and_list_but_not_write() {
        let server = Server::start(414, scope("ENG-1", true, true)).unwrap();
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let tickets = tickets(fake.clone(), None);
        for request in [
            tool_call("ticket_comment", json!({"body":"Done"})),
            tool_call("ticket_status", json!({"target":"Done"})),
        ] {
            let response = client(&server).mcp_response(request).unwrap();
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(text(&response["result"]), "This session is read-only");
        }
        assert!(server.drain().is_empty(), "Refused before reaching the app");
        // The hub refuses the same writes from a caller that skips the stdio server.
        let environment = server.environment(Path::new("/test/app"));
        for tool in [
            TicketTool::Comment {
                markdown: "Done".into(),
            },
            TicketTool::SetStatus {
                target: "Done".into(),
                fields: BTreeMap::new(),
            },
        ] {
            assert_eq!(
                request(&environment, None, Some(tool.into())).unwrap_err(),
                "This session is read-only"
            );
        }
        assert!(server.drain().is_empty());
        assert_eq!(
            request(&environment, None, None).unwrap(),
            json!({"key":"ENG-1","mayChangeStatus":true,"readOnly":true,"cards":false})
        );
        let read = call_answered(&server, &tickets, tool_call("ticket_get", json!({})));
        assert_eq!(read["isError"], false, "{read}");
        let listing = call_answered(&server, &tickets, tool_call("ticket_status", json!({})));
        assert_eq!(listing["isError"], false, "{listing}");
        let listing: Value = serde_json::from_str(text(&listing)).unwrap();
        assert_eq!(listing["mayChangeStatus"], false);
        assert!(fake.comments("ENG-1").is_empty());
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
    }

    #[test]
    fn card_tools_listed_only_when_scope_has_cards() {
        let tickets_only = Server::start(420, scope("ENG-1", true, false)).unwrap();
        assert_eq!(
            names(&listed(&tickets_only)),
            ["ticket_get", "ticket_comment", "ticket_status"]
        );
        // Without cards in scope, a card call is refused before the app,
        // whether it comes through the stdio process or straight to the hub.
        let response = client(&tickets_only)
            .mcp_response(tool_call("card_subtasks", json!({})))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            text(&response["result"]),
            "Card tools aren't available in this session"
        );
        let environment = tickets_only.environment(Path::new("/test/app"));
        assert_eq!(
            request(&environment, None, Some(CardTool::Subtasks.into())).unwrap_err(),
            "Card tools aren't available in this session"
        );
        assert!(tickets_only.drain().is_empty());

        let both = Server::start(421, with_cards(Some("ENG-1"), false)).unwrap();
        let tools = listed(&both);
        assert_eq!(
            names(&tools),
            [
                "ticket_get",
                "ticket_comment",
                "ticket_status",
                "card_subtasks",
                "card_subtask_done"
            ]
        );
        for tool in &tools {
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{tool}");
        }
        assert_eq!(tools[3]["inputSchema"]["properties"], json!({}));
        assert_eq!(
            tools[4]["inputSchema"]["required"],
            json!(["subtaskId", "done"])
        );
        assert_eq!(
            tools[4]["inputSchema"]["properties"]["subtaskId"]["type"],
            "string"
        );
        assert_eq!(
            tools[4]["inputSchema"]["properties"]["done"]["type"],
            "boolean"
        );

        let cards_only = Server::start(422, with_cards(None, false)).unwrap();
        assert_eq!(
            names(&listed(&cards_only)),
            ["card_subtasks", "card_subtask_done"]
        );
        // A card call reaches the app as a card tool, with no ticket attached.
        let result = answered_by(
            &cards_only,
            tool_call("card_subtask_done", json!({"subtaskId":"t-1","done":true})),
            |call| {
                assert_eq!(
                    call.tool,
                    BridgeTool::Card(CardTool::SubtaskDone {
                        subtask_id: "t-1".into(),
                        done: true
                    })
                );
                assert_eq!(call.ticket, None);
                Ok(json!({"ticked":true}))
            },
        );
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(text(&result), r#"{"ticked":true}"#);
    }

    #[test]
    fn ticket_tools_absent_without_a_ticket() {
        let server = Server::start(423, with_cards(None, false)).unwrap();
        let tools = names(&listed(&server));
        assert!(
            tools.iter().all(|name| !name.starts_with("ticket_")),
            "{tools:?}"
        );
        for request in [
            tool_call("ticket_get", json!({})),
            tool_call("ticket_get", json!({"key":"ENG-1"})),
            tool_call("ticket_comment", json!({"body":"hi"})),
            tool_call("ticket_status", json!({"target":"Done"})),
        ] {
            let response = client(&server).mcp_response(request).unwrap();
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(
                text(&response["result"]),
                "This session isn't linked to a ticket"
            );
        }
        // The hub refuses too, even when the agent names a key itself.
        let environment = server.environment(Path::new("/test/app"));
        for key in [None, Some("ENG-1")] {
            assert_eq!(
                request(&environment, key, Some(TicketTool::Get.into())).unwrap_err(),
                "This session isn't linked to a ticket"
            );
        }
        assert_eq!(
            request(&environment, None, None).unwrap(),
            json!({"key":null,"mayChangeStatus":true,"readOnly":false,"cards":true})
        );
        assert!(server.drain().is_empty(), "No call may reach the app");
        // With neither a ticket nor cards there is nothing to serve.
        assert!(Server::start(
            424,
            Scope {
                ticket: None,
                may_change_status: true,
                read_only: false,
                cards: false
            }
        )
        .is_err());
    }

    #[test]
    fn card_subtask_done_refused_for_read_only_before_reaching_the_app() {
        let server = Server::start(425, with_cards(Some("ENG-1"), true)).unwrap();
        let response = client(&server)
            .mcp_response(tool_call(
                "card_subtask_done",
                json!({"subtaskId":"t-1","done":true}),
            ))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(text(&response["result"]), "This session is read-only");
        assert!(server.drain().is_empty(), "Refused before reaching the app");
        // A caller that skips the stdio process meets the same refusal.
        let environment = server.environment(Path::new("/test/app"));
        let untick = CardTool::SubtaskDone {
            subtask_id: "t-1".into(),
            done: false,
        };
        assert_eq!(
            request(&environment, None, Some(untick.into())).unwrap_err(),
            "This session is read-only"
        );
        assert!(server.drain().is_empty());
        // Listing subtasks is a read, so it still goes through.
        let result = answered_by(&server, tool_call("card_subtasks", json!({})), |call| {
            assert_eq!(call.tool, BridgeTool::Card(CardTool::Subtasks));
            Ok(json!({"cards":[]}))
        });
        assert_eq!(result["isError"], false, "{result}");
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        let server = Server::start(426, with_cards(Some("ENG-1"), false)).unwrap();
        for request in [
            tool_call("ticket_get", json!({"ticket":"ENG-9"})),
            tool_call("ticket_comment", json!({"body":"hi","sessionKey":"s-2"})),
            tool_call("ticket_status", json!({"target":"Done","tracker":"jira"})),
            tool_call("ticket_status", json!({"fields":{},"session":7})),
            tool_call("card_subtasks", json!({"cardId":"c-9"})),
            tool_call(
                "card_subtask_done",
                json!({"subtaskId":"t-1","done":true,"sessionKey":"s-2"}),
            ),
            tool_call("card_subtask_done", json!({"subtaskId":"t-1"})),
            tool_call("ticket_get", json!("ENG-1")),
        ] {
            let response = client(&server).mcp_response(request.clone()).unwrap();
            assert_eq!(response["result"]["isError"], true, "{request}");
        }
        let response = client(&server)
            .mcp_response(tool_call(
                "ticket_comment",
                json!({"body":"hi","cardId":"c"}),
            ))
            .unwrap();
        assert_eq!(text(&response["result"]), "Unknown argument: cardId");
        assert!(server.drain().is_empty(), "No call may reach the app");
        // The hub rejects unknown keys on its own wire too.
        for wire in [
            json!({"tool":{"tool":"get","key":"ENG-9"}}),
            json!({"tool":{"tool":"statuses","session":1}}),
            json!({"tool":{"tool":"comment","markdown":"hi","ticket":"ENG-9"}}),
            json!({"tool":{"tool":"set_status","target":"Done","fields":{},"key":"ENG-9"}}),
            json!({"tool":{"tool":"card_subtasks","cardId":"c-9"}}),
            json!({"tool":{"tool":"card_subtask_done","subtaskId":"t","done":true,"sessionKey":"s"}}),
            json!({"tool":{"tool":"card_delete"}}),
            json!({"sessionKey":"s-2"}),
            json!({"tool":{"tool":"get"},"ticket":"ENG-9"}),
        ] {
            assert_eq!(
                raw(&server, wire.clone()).unwrap_err(),
                "Invalid ticket request",
                "{wire}"
            );
        }
        assert!(server.drain().is_empty());
        // The same wire without the extra keys is accepted.
        assert_eq!(
            raw(&server, json!({})).unwrap(),
            json!({"key":"ENG-1","mayChangeStatus":true,"readOnly":false,"cards":true})
        );
    }

    #[test]
    fn a_relink_mid_turn_applies_to_the_next_call_not_the_agents_key() {
        let server = Server::start(427, scope("ENG-1", true, false)).unwrap();
        let fake = Arc::new(
            FakeProvider::new()
                .with_issue(issue("ENG-1"))
                .with_issue(issue("ENG-2")),
        );
        let tickets = tickets(fake.clone(), None);
        // The user relinks the session while the turn runs.
        server.set_scope(scope("ENG-2", true, false));
        assert_eq!(
            server.ticket().map(|ticket| ticket.key),
            Some("ENG-2".into())
        );
        // The agent still names the old key: refused, naming the new link.
        let response = client(&server)
            .mcp_response(tool_call(
                "ticket_comment",
                json!({"key":"ENG-1","body":"hi"}),
            ))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(text(&response["result"]), "This session is linked to ENG-2");
        assert!(server.drain().is_empty());
        // Without a key, the call goes to the ticket linked at call time.
        let result = call_answered(
            &server,
            &tickets,
            tool_call("ticket_comment", json!({"body":"Relinked"})),
        );
        assert_eq!(result["isError"], false, "{result}");
        assert!(fake.comments("ENG-1").is_empty());
        assert_eq!(fake.comments("ENG-2")[0].1, "Relinked");
        // The permission is read at call time as well.
        server.set_scope(scope("ENG-2", false, false));
        let response = client(&server)
            .mcp_response(tool_call("ticket_status", json!({"target":"Done"})))
            .unwrap();
        assert_eq!(
            text(&response["result"]),
            "Changing ticket status is turned off for this session"
        );
        // Unlinked mid-turn: ticket tools stop at once.
        server.set_scope(Scope {
            ticket: None,
            may_change_status: true,
            read_only: false,
            cards: false,
        });
        assert_eq!(server.ticket(), None);
        let response = client(&server)
            .mcp_response(tool_call("ticket_get", json!({})))
            .unwrap();
        assert_eq!(
            text(&response["result"]),
            "This session isn't linked to a ticket"
        );
        assert!(server.drain().is_empty());
        // A finished turn's server no longer changes the session's scope.
        drop(server);
        let next = Server::start(427, scope("ENG-3", true, false)).unwrap();
        let environment = next.environment(Path::new("/test/app"));
        assert_eq!(request(&environment, None, None).unwrap()["key"], "ENG-3");
    }

    #[test]
    fn tools_list_falls_back_to_every_tool_without_a_scope() {
        let server = Server::start(428, scope("ENG-1", true, false)).unwrap();
        let environment = server.environment(Path::new("/test/app"));
        let every = [
            "ticket_get",
            "ticket_comment",
            "ticket_status",
            "card_subtasks",
            "card_subtask_done",
        ];
        let listed_by = |environment: HashMap<String, String>| {
            let catalog = Client::new(environment)
                .mcp_response(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
                .unwrap();
            assert!(catalog["error"].is_null(), "{catalog}");
            names(catalog["result"]["tools"].as_array().unwrap())
        };
        // With no active turn the scope can't be read: list every tool.
        drop(server);
        assert_eq!(listed_by(environment.clone()), every);
        // A forged token reads no scope either.
        let mut forged = environment.clone();
        forged.insert(ENV_TOKEN.into(), "0".repeat(64));
        assert_eq!(listed_by(forged), every);
        // The calls themselves are still refused.
        let response = Client::new(environment.clone())
            .mcp_response(tool_call("ticket_get", json!({})))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        let response = Client::new(environment)
            .mcp_response(tool_call("card_subtasks", json!({})))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn card_calls_are_checked_before_the_ticket_key() {
        // A card-only session: a key on a card call is not a ticket refusal.
        let cards_only = Server::start(429, with_cards(None, false)).unwrap();
        let worker = {
            let environment = cards_only.environment(Path::new("/test/app"));
            std::thread::spawn(move || {
                request(&environment, Some("ENG-1"), Some(CardTool::Subtasks.into()))
            })
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() && Instant::now() < deadline {
            for call in cards_only.drain() {
                assert_eq!(call.tool, BridgeTool::Card(CardTool::Subtasks));
                let _ = call.reply.send(Ok(json!({"cards":[]})));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(worker.join().unwrap().unwrap(), json!({"cards":[]}));
        // A ticket-only session refuses the card tool as a card tool,
        // whatever key comes with it.
        let tickets_only = Server::start(430, scope("ENG-1", true, false)).unwrap();
        let environment = tickets_only.environment(Path::new("/test/app"));
        assert_eq!(
            request(&environment, Some("ENG-9"), Some(CardTool::Subtasks.into())).unwrap_err(),
            "Card tools aren't available in this session"
        );
        assert!(tickets_only.drain().is_empty());
        // Ticket calls still check the key first.
        assert_eq!(
            request(&environment, Some("ENG-9"), Some(TicketTool::Get.into())).unwrap_err(),
            "This session is linked to ENG-1"
        );
    }
}
