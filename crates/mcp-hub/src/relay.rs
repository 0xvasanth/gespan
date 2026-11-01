//! The hub's stdio side, for agents that can't speak HTTP MCP and for command
//! line clients. It holds no state of its own: JSON-RPC lines go to the
//! endpoint and whatever comes back goes to stdout.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
const GONE: &str = "The tool endpoint isn't reachable, so this tool can't run";
const SESSION_HEADER: &str = "Mcp-Session-Id";
const VERSION_HEADER: &str = "MCP-Protocol-Version";

/// Where a hub is and the token to present to it.
#[derive(Clone)]
pub struct Connection {
    pub url: String,
    pub token: String,
}

/// What proves who is calling.
#[derive(Clone)]
pub(crate) enum Credential {
    None,
    Bearer(String),
    /// Fetched when needed, and held briefly: finding a token can take several
    /// round trips to an authorization server.
    Source(Arc<TokenSource>),
}

/// How long a fetched token is used before asking again.
const TOKEN_LIFE: Duration = Duration::from_secs(60);

type Fetch = dyn Fn() -> Result<String, String> + Send + Sync;

pub(crate) struct TokenSource {
    fetch: Box<Fetch>,
    held: Mutex<Option<(String, Instant)>>,
}

impl TokenSource {
    pub(crate) fn new(fetch: Box<Fetch>) -> Self {
        Self {
            fetch,
            held: Mutex::new(None),
        }
    }

    fn token(&self) -> Result<String, String> {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((token, at)) = held.as_ref() {
            if at.elapsed() < TOKEN_LIFE {
                return Ok(token.clone());
            }
        }
        let token = (self.fetch)()?;
        *held = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    fn forget(&self) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[derive(Clone)]
pub(crate) struct Link {
    agent: ureq::Agent,
    url: String,
    credential: Credential,
    headers: BTreeMap<String, String>,
    session: Arc<Mutex<Option<String>>>,
    version: Arc<Mutex<Option<String>>>,
}

fn held(slot: &Mutex<Option<String>>) -> Option<String> {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

impl Link {
    fn to_hub(Connection { url, token }: Connection) -> Self {
        Self::new(url, Credential::Bearer(token), BTreeMap::new())
    }

    pub(crate) fn new(
        url: String,
        credential: Credential,
        headers: BTreeMap<String, String>,
    ) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(3))
                .redirects(0)
                .build(),
            url,
            credential,
            headers,
            session: Arc::default(),
            version: Arc::default(),
        }
    }

    fn request(&self, method: &str) -> Result<ureq::Request, String> {
        let mut request = self
            .agent
            .request(method, &self.url)
            .set("Accept", "application/json, text/event-stream");
        for (name, value) in &self.headers {
            request = request.set(name, value);
        }
        let token = match &self.credential {
            Credential::None => None,
            Credential::Bearer(token) => Some(token.clone()),
            Credential::Source(source) => Some(source.token()?),
        };
        if let Some(token) = token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        if let Some(session) = held(&self.session) {
            request = request.set(SESSION_HEADER, &session);
        }
        if let Some(version) = held(&self.version) {
            request = request.set(VERSION_HEADER, &version);
        }
        Ok(request)
    }

    /// One message out, and everything the hub answers it with, until the
    /// reply to its own id. Notifications have no reply to wait for.
    fn exchange(&self, message: &Value, deliver: &mut dyn FnMut(Value)) -> Result<(), String> {
        let body = message.to_string();
        let send = || {
            self.request("POST")
                .map(|request| request.set("Content-Type", "application/json"))
                .map(|request| request.send_string(&body))
        };
        let mut sent = send()?;
        // A held token may have gone stale; ask for a fresh one once.
        if let (Err(ureq::Error::Status(401, _)), Credential::Source(source)) =
            (&sent, &self.credential)
        {
            source.forget();
            sent = send()?;
        }
        let response = match sent {
            Ok(response) => response,
            Err(ureq::Error::Status(401, _)) => {
                return Err("This server needs a sign-in".to_owned());
            }
            Err(ureq::Error::Status(_, response)) => {
                return Err(refusal_text(response).unwrap_or_else(|| GONE.to_owned()));
            }
            Err(_) => return Err(GONE.to_owned()),
        };
        if let Some(session) = response.header(SESSION_HEADER) {
            *self
                .session
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(session.to_owned());
        }
        let id = message.get("id").filter(|id| !id.is_null());
        let streamed = response
            .header("Content-Type")
            .is_some_and(|kind| kind.starts_with("text/event-stream"));
        let reader = response.into_reader();
        if streamed {
            read_events(reader, |event| {
                let done = id.is_some_and(|id| event.get("id") == Some(id));
                self.record_version(&event);
                deliver(event);
                !done
            })
        } else {
            let mut body = String::new();
            reader
                .take(MAX_LINE_BYTES as u64)
                .read_to_string(&mut body)
                .map_err(|_| GONE.to_owned())?;
            if let Ok(event) = serde_json::from_str(&body) {
                self.record_version(&event);
                deliver(event);
            }
            Ok(())
        }
    }

    fn record_version(&self, event: &Value) {
        if let Some(version) = event["result"]["protocolVersion"].as_str() {
            *self
                .version
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(version.to_owned());
        }
    }
}

fn refusal_text(response: ureq::Response) -> Option<String> {
    let body: Value = response.into_json().ok()?;
    body["error"]["message"].as_str().map(str::to_owned)
}

/// Reads `data:` payloads until `keep_going` says stop or the stream ends.
fn read_events(reader: impl Read, mut keep_going: impl FnMut(Value) -> bool) -> Result<(), String> {
    let mut lines = BufReader::new(reader).lines();
    let mut data = String::new();
    loop {
        let Some(line) = lines.next() else {
            return Ok(());
        };
        let line = line.map_err(|_| GONE.to_owned())?;
        if let Some(payload) = line.strip_prefix("data:") {
            data.push_str(payload.trim_start());
        } else if line.is_empty() && !data.is_empty() {
            if let Ok(event) = serde_json::from_str(&std::mem::take(&mut data)) {
                if !keep_going(event) {
                    return Ok(());
                }
            }
        }
    }
}

fn write_line(output: &Mutex<impl Write>, message: &Value) -> Result<(), String> {
    let mut output = output
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    writeln!(output, "{message}")
        .and_then(|()| output.flush())
        .map_err(|e| e.to_string())
}

/// Reads the server-to-client stream, so a changed tool list reaches the agent.
fn listen(link: Link, output: Arc<Mutex<impl Write + Send + 'static>>) {
    thread::spawn(move || {
        let Ok(response) = link
            .request("GET")
            .and_then(|request| request.call().map_err(|error| error.to_string()))
        else {
            return;
        };
        let _ = read_events(response.into_reader(), |event| {
            write_line(&output, &event).is_ok()
        });
    });
}

fn forward(link: &Link, message: &Value, output: &Arc<Mutex<impl Write>>) {
    let failed = link
        .exchange(message, &mut |event| {
            let _ = write_line(output, &event);
        })
        .err();
    if let (Some(error), Some(id)) = (failed, message.get("id").filter(|id| !id.is_null())) {
        let reply = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error}});
        let _ = write_line(output, &reply);
    }
}

pub(crate) fn pump(
    link: Link,
    input: impl BufRead,
    output: Arc<Mutex<impl Write + Send + 'static>>,
) -> Result<(), String> {
    let mut workers = Vec::new();
    for line in input.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            return Err("Oversized MCP frame".into());
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            let reply =
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}});
            write_line(&output, &reply)?;
            continue;
        };
        // The handshake is in order; a tool call can run alongside others.
        if message.get("id").is_none() || message["method"] == "initialize" {
            forward(&link, &message, &output);
            if message["method"] == "notifications/initialized" {
                listen(link.clone(), output.clone());
            }
        } else {
            let (link, output) = (link.clone(), output.clone());
            workers.push(thread::spawn(move || forward(&link, &message, &output)));
        }
        workers.retain(|worker| !worker.is_finished());
    }
    for worker in workers {
        let _ = worker.join();
    }
    if let Ok(request) = link.request("DELETE") {
        let _ = request.call();
    }
    Ok(())
}

/// Serves stdio until it closes. Runs before any GUI of the host's; stdout
/// carries MCP frames only.
pub fn relay(connection: Connection) -> Result<(), String> {
    pump(
        Link::to_hub(connection),
        std::io::stdin().lock(),
        Arc::new(Mutex::new(std::io::stdout())),
    )
}

/// One tool call over a short-lived connection.
pub fn call_tool(connection: Connection, tool: &str, arguments: Value) -> Result<Value, String> {
    let link = Link::to_hub(connection);
    link.exchange(
        &json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"mcp-hub-client","version":env!("CARGO_PKG_VERSION")},
        }}),
        &mut |_| {},
    )?;
    link.exchange(
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        &mut |_| {},
    )?;
    let mut reply = None;
    let result = link.exchange(
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}}),
        &mut |event| {
            if event["id"] == 1 {
                reply = Some(event);
            }
        },
    );
    if let Ok(request) = link.request("DELETE") {
        let _ = request.call();
    }
    result?;
    let reply = reply.ok_or(GONE)?;
    if let Some(error) = reply["error"]["message"].as_str() {
        return Err(error.to_owned());
    }
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or("");
    if reply["result"]["isError"] == true {
        return Err(text.to_owned());
    }
    Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())))
}
