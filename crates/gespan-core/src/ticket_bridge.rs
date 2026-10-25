//! Session-scoped ticket tools for coding agents. The stdio server forwards
//! each call over loopback to the app, which acts through `Tickets` with the
//! keychain credentials; the agent only ever holds a per-session token.
use crate::provider_boards::RequiredField;
use crate::tickets::{TicketError, TicketRef, Tickets};
use serde::{Deserialize, Serialize};
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
const STATUS_OFF: &str = "Changing ticket status is turned off for this session";
const APP_GONE: &str = "Gespan stopped handling this session's ticket tools";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum TicketTool {
    Get,
    Comment {
        markdown: String,
    },
    Statuses,
    SetStatus {
        target: String,
        #[serde(default)]
        fields: BTreeMap<String, Value>,
    },
}

pub struct Call {
    pub tool: TicketTool,
    pub reply: mpsc::Sender<Result<Value, String>>,
    generation: u64,
}

/// A request without a tool asks which ticket the session is linked to.
#[derive(Serialize, Deserialize)]
struct Wire {
    session: i64,
    token: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    tool: Option<TicketTool>,
}

struct ActiveTurn {
    generation: u64,
    send: mpsc::SyncSender<Call>,
    key: String,
    may_change_status: bool,
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
/// provider keeps working; the ticket and permission are this turn's.
pub struct Server {
    hub: Arc<Hub>,
    token: String,
    session: i64,
    generation: u64,
    ticket: TicketRef,
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
    pub fn start(session: i64, ticket: TicketRef, may_change_status: bool) -> Result<Self, String> {
        let _span =
            tracing::info_span!("mcp_start", server = "ticket", session_id = session).entered();
        Self::open(session, ticket, may_change_status)
            .inspect_err(|error| tracing::warn!(%error, "could not start a tool server"))
    }

    fn open(session: i64, ticket: TicketRef, may_change_status: bool) -> Result<Self, String> {
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
                key: ticket.key.clone(),
                may_change_status,
            });
            (capability.token.clone(), capability.generation)
        };
        Ok(Self {
            hub,
            token,
            session,
            generation,
            ticket,
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

    pub fn ticket(&self) -> &TicketRef {
        &self.ticket
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
        // The token is in the agent's environment, so scope is enforced here
        // and not only by the stdio server.
        if let Some(key) = &wire.key {
            if !same_key(key, &active.key) {
                return Err(format!("This session is linked to {}", active.key));
            }
        }
        let Some(tool) = wire.tool else {
            return Ok(json!({"key":active.key,"mayChangeStatus":active.may_change_status}));
        };
        if matches!(tool, TicketTool::SetStatus { .. }) && !active.may_change_status {
            return Err(STATUS_OFF.into());
        }
        let (reply, result) = mpsc::channel();
        active
            .send
            .try_send(Call {
                tool,
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

/// The app's side of a call. Errors are the user-facing tracker messages,
/// never credentials or raw responses.
pub fn answer(tickets: &Tickets, ticket: &TicketRef, tool: &TicketTool) -> Result<Value, String> {
    let result = match tool {
        TicketTool::Get => tickets.read(ticket).map(|(fresh, description)| {
            json!({
                "key": fresh.key,
                "tracker": fresh.tracker.label(),
                "title": fresh.title,
                "status": fresh.status,
                "url": fresh.url,
                "description": description,
            })
        }),
        TicketTool::Comment { markdown } => tickets
            .comment(ticket, markdown)
            .map(|url| json!({"commented":true,"url":url})),
        TicketTool::Statuses => tickets.statuses(ticket).map(|choices| {
            json!({
                "key": ticket.key,
                "statuses": choices
                    .into_iter()
                    .map(|choice| json!({
                        "id": choice.id,
                        "name": choice.name,
                        "group": choice.group,
                        "requiredFields": choice
                            .required_fields
                            .iter()
                            .map(|field| json!({"key":field.key,"name":field.name,"allowed":field.allowed}))
                            .collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>(),
            })
        }),
        TicketTool::SetStatus { target, fields } => tickets
            .set_status(ticket, target, fields.clone())
            .map(|moved| json!({"key":moved.key,"status":moved.status,"url":moved.url})),
    };
    result.map_err(|error| match &error {
        TicketError::NeedsFields(fields) => format!(
            "{error}. Pass them in fields by key: {}",
            fields
                .iter()
                .map(describe_field)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => error.to_string(),
    })
}

fn describe_field(field: &RequiredField) -> String {
    if field.allowed.is_empty() {
        format!("{} ({})", field.key, field.name)
    } else {
        format!(
            "{} ({}; one of: {})",
            field.key,
            field.name,
            field.allowed.join(", ")
        )
    }
}

/// One loopback round trip. `tool: None` asks for the session's scope.
fn request(
    environment: &HashMap<String, String>,
    key: Option<&str>,
    tool: Option<TicketTool>,
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

pub fn tool_definitions() -> Vec<Value> {
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
        let linked = scope["key"].as_str().unwrap_or_default();
        if key.is_some_and(|key| !same_key(key, linked)) {
            return Err(format!("This session is linked to {linked}"));
        }
        let may_change_status = scope["mayChangeStatus"] == true;
        if matches!(tool, TicketTool::SetStatus { .. }) && !may_change_status {
            return Err(STATUS_OFF.into());
        }
        let statuses = matches!(tool, TicketTool::Statuses);
        let mut result = request(&self.environment, key, Some(tool))?;
        if statuses {
            result["mayChangeStatus"] = may_change_status.into();
        }
        Ok(result)
    }

    fn mcp_response(&self, request: Value) -> Option<Value> {
        let id = request.get("id")?.clone();
        let result = match request["method"].as_str().unwrap_or("") {
            "initialize" => Ok(
                json!({"protocolVersion":request["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"gespan_ticket","version":env!("CARGO_PKG_VERSION")}}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":tool_definitions()})),
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
    use crate::tickets::{Connection, Tracker};
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
        let client = client(server);
        let worker = std::thread::spawn(move || client.mcp_response(request).unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() && Instant::now() < deadline {
            for call in server.drain() {
                let _ = call
                    .reply
                    .send(answer(tickets, server.ticket(), &call.tool));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        worker.join().unwrap()["result"].clone()
    }

    fn text(result: &Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    #[test]
    fn ticket_tools_list_three_tools() {
        let server = Server::start(401, linked("ENG-1"), true).unwrap();
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
        let server = Server::start(402, linked("ENG-1"), true).unwrap();
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
        let server = Server::start(403, linked("ENG-1"), true).unwrap();
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
        let server = Server::start(404, linked("ABC-1"), true).unwrap();
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
        let server = Server::start(405, linked("ABC-1"), true).unwrap();
        let environment = server.environment(Path::new("/test/app"));
        let error = request(&environment, Some("ABC-99"), Some(TicketTool::Get)).unwrap_err();
        assert_eq!(error, "This session is linked to ABC-1");
        assert!(server.drain().is_empty());
    }

    #[test]
    fn status_change_refused_when_not_allowed() {
        let server = Server::start(406, linked("ENG-1"), false).unwrap();
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
            Some(TicketTool::SetStatus {
                target: "Done".into(),
                fields: BTreeMap::new(),
            }),
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
        let server = Server::start(407, linked("ENG-1"), true).unwrap();
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
        let server = Server::start(408, linked("ENG-1"), true).unwrap();
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
        let server = Server::start(409, linked("ENG-1"), true).unwrap();
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
        assert!(request(&environment, None, Some(TicketTool::Get)).is_err());
        environment.insert(ENV_TOKEN.into(), token);
        environment.insert(ENV_SESSION.into(), "999409".into());
        assert!(request(&environment, None, Some(TicketTool::Get)).is_err());
        assert!(server.drain().is_empty());
    }

    #[test]
    fn app_gone_returns_error_not_hang() {
        let server = Server::start(410, linked("ENG-1"), true).unwrap();
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
        let environment = Server::start(411, linked("ENG-1"), true)
            .unwrap()
            .environment(Path::new("/test/app"));
        assert!(request(&environment, None, Some(TicketTool::Get)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_restarted_turn_keeps_the_token_but_takes_the_new_ticket() {
        let first = Server::start(412, linked("ENG-1"), true).unwrap();
        let environment = first.environment(Path::new("/test/app"));
        assert!(Server::start(412, linked("ENG-1"), true).is_err());
        drop(first);
        let next = Server::start(412, linked("ENG-2"), false).unwrap();
        assert_eq!(environment, next.environment(Path::new("/test/app")));
        assert_eq!(
            request(&environment, None, None).unwrap(),
            json!({"key":"ENG-2","mayChangeStatus":false})
        );
    }

    #[test]
    fn tool_results_and_server_logs_never_carry_the_token() {
        let server = Server::start(413, linked("ENG-1"), true).unwrap();
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
}
