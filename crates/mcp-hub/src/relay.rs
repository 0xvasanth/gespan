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
/// The hub sends a stream comment every 15 seconds while a tool runs, so this
/// much silence means it has stopped answering, not that a tool is slow.
const READ_WAIT: Duration = Duration::from_secs(60);
/// A remote server owes us no such comments, so only a stalled connection is
/// cut off; a slow tool is the pool's to time out.
const REMOTE_READ_WAIT: Duration = Duration::from_secs(5 * 60);
const GONE: &str = "The tool endpoint isn't reachable, so this tool can't run";
const SESSION_HEADER: &str = "Mcp-Session-Id";
const VERSION_HEADER: &str = "MCP-Protocol-Version";

/// Where a hub is and the token to present to it.
#[derive(Clone)]
pub struct Connection {
    pub url: String,
    pub token: String,
}

fn agent(read_wait: Option<Duration>) -> ureq::Agent {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .redirects(0);
    match read_wait {
        Some(wait) => agent.timeout_read(wait),
        None => agent,
    }
    .build()
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
    /// For the server-to-client stream, which may stay quiet for as long as
    /// the agent runs.
    listener: ureq::Agent,
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
        Self::waiting(url, Credential::Bearer(token), BTreeMap::new(), READ_WAIT)
    }

    pub(crate) fn new(
        url: String,
        credential: Credential,
        headers: BTreeMap<String, String>,
    ) -> Self {
        Self::waiting(url, credential, headers, REMOTE_READ_WAIT)
    }

    fn waiting(
        url: String,
        credential: Credential,
        headers: BTreeMap<String, String>,
        read_wait: Duration,
    ) -> Self {
        Self {
            agent: agent(Some(read_wait)),
            listener: agent(None),
            url,
            credential,
            headers,
            session: Arc::default(),
            version: Arc::default(),
        }
    }

    fn request(&self, method: &str) -> Result<ureq::Request, String> {
        self.request_on(&self.agent, method)
    }

    fn request_on(&self, agent: &ureq::Agent, method: &str) -> Result<ureq::Request, String> {
        let mut request = agent
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

    /// Sends, and if the server turns down a held token that may have gone
    /// stale, asks for a fresh one and sends once more.
    #[allow(clippy::result_large_err)] // ureq's error is the one it returns.
    fn with_fresh_token(
        &self,
        send: impl Fn() -> Result<Result<ureq::Response, ureq::Error>, String>,
    ) -> Result<Result<ureq::Response, ureq::Error>, String> {
        let sent = send()?;
        if let (Err(ureq::Error::Status(401, _)), Credential::Source(source)) =
            (&sent, &self.credential)
        {
            source.forget();
            return send();
        }
        Ok(sent)
    }

    /// One message out, and everything the hub answers it with, until the
    /// reply to its own id. Notifications have no reply to wait for; a request
    /// whose reply never came is an error, or its caller would wait forever.
    #[allow(clippy::result_large_err)] // ureq's error is the one it returns.
    fn exchange(&self, message: &Value, deliver: &mut dyn FnMut(Value)) -> Result<(), String> {
        let body = message.to_string();
        let send = || {
            self.request("POST")
                .map(|request| request.set("Content-Type", "application/json"))
                .map(|request| request.send_string(&body))
        };
        let response = match self.with_fresh_token(send)? {
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
        let mut answered = id.is_none();
        if streamed {
            read_events(reader, |event| {
                answered |= id.is_some_and(|id| event.get("id") == Some(id));
                self.record_version(&event);
                deliver(event);
                !answered || id.is_none()
            })?;
        } else {
            let mut body = String::new();
            reader
                .take(MAX_LINE_BYTES as u64)
                .read_to_string(&mut body)
                .map_err(|_| GONE.to_owned())?;
            if let Ok(event) = serde_json::from_str::<Value>(&body) {
                answered |= id.is_some_and(|id| event.get("id") == Some(id));
                self.record_version(&event);
                deliver(event);
            }
        }
        answered.then_some(()).ok_or_else(|| GONE.to_owned())
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
#[allow(clippy::result_large_err)] // ureq's error is the one it returns.
fn listen(link: Link, output: Arc<Mutex<impl Write + Send + 'static>>) {
    thread::spawn(move || {
        let opened = link.with_fresh_token(|| {
            link.request_on(&link.listener, "GET")
                .map(|request| request.call())
        });
        let Ok(Ok(response)) = opened else {
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

enum Frame {
    Line(String),
    /// Longer than `MAX_LINE_BYTES`, and already read past.
    Oversized,
}

/// The next line of input. It never holds more than `MAX_LINE_BYTES` of one,
/// however long the line is.
fn read_frame(input: &mut impl BufRead) -> std::io::Result<Option<Frame>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = input.fill_buf()?;
        let end = buffer.iter().position(|byte| *byte == b'\n');
        let at_end = buffer.is_empty();
        let chunk = &buffer[..end.unwrap_or(buffer.len())];
        oversized |= line.len() + chunk.len() > MAX_LINE_BYTES;
        if oversized {
            line.clear();
        } else {
            line.extend_from_slice(chunk);
        }
        let used = chunk.len() + usize::from(end.is_some());
        input.consume(used);
        if end.is_some() || at_end {
            return Ok(if oversized {
                Some(Frame::Oversized)
            } else if at_end && line.is_empty() {
                None
            } else {
                Some(Frame::Line(String::from_utf8_lossy(&line).into_owned()))
            });
        }
    }
}

pub(crate) fn pump(
    link: Link,
    mut input: impl BufRead,
    output: Arc<Mutex<impl Write + Send + 'static>>,
) -> Result<(), String> {
    let mut workers = Vec::new();
    while let Some(frame) = read_frame(&mut input).map_err(|e| e.to_string())? {
        let Frame::Line(line) = frame else {
            let reply = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Oversized MCP frame"}});
            write_line(&output, &reply)?;
            continue;
        };
        if line.trim().is_empty() {
            continue;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Grant, Hub, Toolkit};
    use std::{
        io::Cursor,
        net::{TcpListener, TcpStream},
    };

    struct Echo;
    impl Toolkit for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn tools(&self, _: &Grant) -> Vec<Value> {
            vec![json!({"name":"echo","inputSchema":{"type":"object"}})]
        }
        fn call(&self, _: &Grant, _: &str, arguments: Value) -> Result<Value, String> {
            Ok(arguments)
        }
    }

    fn hub() -> (Hub, Connection) {
        let hub = Hub::start(Config::new("relay-test", "1")).unwrap();
        hub.register(Arc::new(Echo));
        let token = hub.mint(Grant {
            session: 1,
            workspace: "/work".into(),
            toolkits: ["echo".to_owned()].into(),
            servers: Default::default(),
        });
        let connection = Connection {
            url: hub.url().to_owned(),
            token: token.secret().to_owned(),
        };
        (hub, connection)
    }

    fn lines(messages: &[Value]) -> Vec<u8> {
        messages
            .iter()
            .map(|message| format!("{message}\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn run(link: Link, input: Vec<u8>) -> Vec<Value> {
        let output = Arc::new(Mutex::new(Vec::new()));
        pump(link, Cursor::new(input), output.clone()).unwrap();
        let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn initialize() -> Value {
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"test","version":"1"},
        }})
    }

    fn call(id: i64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
            "name":"echo","arguments":{"word":"hello"},
        }})
    }

    fn reply_to(replies: &[Value], id: i64) -> &Value {
        replies
            .iter()
            .find(|reply| reply["id"] == id)
            .unwrap_or_else(|| panic!("no reply to {id}: {replies:?}"))
    }

    /// Answers every request with this response, after reading its body.
    fn scripted(response: &'static str) -> Connection {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || answer(stream, response));
            }
        });
        Connection {
            url,
            token: "token".to_owned(),
        }
    }

    fn answer(mut stream: TcpStream, response: &str) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            if line.trim().is_empty() {
                break;
            }
        }
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
        let _ = stream.write_all(response.as_bytes());
    }

    #[test]
    fn a_tool_call_goes_through_to_the_hub_and_its_reply_comes_back() {
        let (_hub, connection) = hub();
        let replies = run(
            Link::to_hub(connection),
            lines(&[
                initialize(),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
                call(2),
            ]),
        );
        assert_eq!(reply_to(&replies, 1)["result"]["tools"][0]["name"], "echo");
        let text = reply_to(&replies, 2)["result"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert_eq!(text, r#"{"word":"hello"}"#);
    }

    #[test]
    fn a_refused_token_reaches_the_agent_as_the_hubs_own_error() {
        let (_hub, mut connection) = hub();
        connection.token = "forged".to_owned();
        let replies = run(Link::to_hub(connection), lines(&[call(7)]));
        assert_eq!(
            reply_to(&replies, 7)["error"]["message"],
            "This tool credential is not valid"
        );
    }

    #[test]
    fn a_reply_that_never_comes_fails_the_call_instead_of_leaving_the_agent_waiting() {
        for response in [
            // An empty body.
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            // Not JSON.
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\nnope",
            // Someone else's reply.
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 25\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":99}",
            // A stream that ends cleanly first.
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\n\r\n",
        ] {
            let replies = run(Link::to_hub(scripted(response)), lines(&[call(3)]));
            assert_eq!(reply_to(&replies, 3)["error"]["message"], GONE, "{response}");
        }
    }

    #[test]
    fn a_notification_expects_no_reply() {
        let link = Link::to_hub(scripted(
            "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n",
        ));
        let replies = run(
            link,
            lines(&[json!({"jsonrpc":"2.0","method":"notifications/initialized"})]),
        );
        assert!(replies.is_empty(), "{replies:?}");
    }

    #[test]
    fn a_hub_that_stops_answering_fails_the_call() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        // Holds the connection open and never writes.
        let _stalled = thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(2).collect();
            thread::sleep(Duration::from_secs(5));
            drop(held);
        });
        let link = Link::waiting(
            url,
            Credential::Bearer("token".to_owned()),
            BTreeMap::new(),
            Duration::from_millis(100),
        );
        let started = std::time::Instant::now();
        let replies = run(link.clone(), lines(&[call(4)]));
        assert_eq!(reply_to(&replies, 4)["error"]["message"], GONE);
        assert!(started.elapsed() < Duration::from_secs(4), "it gave up");
    }

    #[test]
    fn an_oversized_frame_is_refused_and_the_relay_carries_on() {
        let (_hub, connection) = hub();
        let mut input = vec![b'x'; MAX_LINE_BYTES + 1];
        input.push(b'\n');
        input.extend(lines(&[initialize(), call(5)]));
        let replies = run(Link::to_hub(connection), input);
        assert_eq!(replies[0]["error"]["message"], "Oversized MCP frame");
        assert!(reply_to(&replies, 5)["result"].is_object());
    }

    #[test]
    fn a_frame_is_read_up_to_its_limit_and_no_further() {
        let frame = |line: Vec<u8>| {
            let mut input = Cursor::new(line);
            let first = read_frame(&mut input).unwrap().unwrap();
            assert!(read_frame(&mut input).unwrap().is_none());
            first
        };
        assert!(matches!(
            frame(vec![b'x'; MAX_LINE_BYTES]),
            Frame::Line(line) if line.len() == MAX_LINE_BYTES
        ));
        assert!(matches!(
            frame(vec![b'x'; MAX_LINE_BYTES + 1]),
            Frame::Oversized
        ));
    }
}
