//! OpenCode 2.2's client-hosted MCP: Gespan opens a WebSocket to the service
//! and answers on it as the MCP server, relaying each message to the hub. The
//! service is the MCP client, so it never holds the hub's credential.
use super::*;
use base64::Engine;
use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::net::TcpStream;
use tungstenite::{client::IntoClientRequest, protocol::frame::coding::CloseCode, Message};

/// How long a read waits before the socket's replies are sent. It's also how
/// soon a stop is noticed.
const POLL: Duration = Duration::from_millis(25);
/// The service lists the tools once it has initialized; without that list the
/// turn's first prompt would start before they're there.
const LISTED_WAIT: Duration = Duration::from_secs(10);
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(10);

type Socket = tungstenite::WebSocket<TcpStream>;

/// A served socket. Dropping it closes the socket, and with it the service's
/// copy of the tools.
pub(super) struct Hosted {
    stop: Arc<AtomicBool>,
}

impl Drop for Hosted {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Serves the tools as `name` until the result is dropped, reconnecting if
/// the service closes the socket, as it does when it reloads. `None` if the
/// service won't take the socket or never lists the tools.
pub(super) fn host(client: &Client, name: &str, hub: mcp_hub::Connection) -> Option<Hosted> {
    let socket = open(client, name)
        .inspect_err(|error| tracing::info!(%error, "OpenCode didn't take the tool socket"))
        .ok()?;
    let stop = Arc::new(AtomicBool::new(false));
    let (listed, ready) = mpsc::channel();
    let (client, name, stopped) = (client.clone(), name.to_owned(), stop.clone());
    thread::spawn(move || {
        let mut socket = Some(socket);
        let mut wait = RECONNECT_MIN;
        while !stopped.load(Ordering::SeqCst) {
            if let Some(socket) = socket.take() {
                if serve(socket, hub.clone(), &stopped, &listed) {
                    wait = RECONNECT_MIN;
                }
                continue;
            }
            thread::sleep(wait);
            wait = (wait * 2).min(RECONNECT_MAX);
            socket = open(&client, &name).ok();
        }
    });
    let hosted = Hosted { stop };
    ready.recv_timeout(LISTED_WAIT).ok().map(|()| hosted)
}

fn open(client: &Client, name: &str) -> Result<Socket, String> {
    let mut url = url::Url::parse(&client.url).map_err(|e| e.to_string())?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|()| "OpenCode's URL has no WebSocket form")?;
    url.set_path("/api/experimental/client-mcp");
    url.query_pairs_mut()
        .clear()
        .append_pair("name", name)
        .append_pair("location[directory]", &client.directory);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| e.to_string())?;
    let headers = request.headers_mut();
    let header = |value: String| value.parse().map_err(|_| "unusable header".to_owned());
    headers.insert(
        "x-opencode-directory",
        header(client::encode(&client.directory))?,
    );
    if let Some(password) = &client.password {
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("opencode:{password}"));
        headers.insert("authorization", header(format!("Basic {auth}"))?);
    }
    let host = url.host_str().ok_or("OpenCode's URL has no host")?;
    let port = url
        .port_or_known_default()
        .ok_or("OpenCode's URL has no port")?;
    let address = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("OpenCode's host didn't resolve")?;
    let stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let (socket, _) = tungstenite::client(request, stream).map_err(|error| match error {
        tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response)) => {
            format!("OpenCode HTTP {}", response.status())
        }
        error => error.to_string(),
    })?;
    socket
        .get_ref()
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    Ok(socket)
}

/// One socket's life: frames in go to the hub, its answers go back out.
/// Returns whether the service listed the tools on it.
fn serve(
    mut socket: Socket,
    hub: mcp_hub::Connection,
    stop: &AtomicBool,
    listed: &mpsc::Sender<()>,
) -> bool {
    let (to_hub, hub_input) = mpsc::channel::<Vec<u8>>();
    let (hub_output, from_hub) = mpsc::channel::<String>();
    let relay = thread::spawn(move || {
        mcp_hub::relay_lines(
            hub,
            BufReader::new(Lines::new(hub_input)),
            Frames::new(hub_output),
        )
    });
    // The ids of the service's tool listings that haven't been answered yet.
    let mut listing = HashSet::new();
    let mut served = false;
    'open: loop {
        if stop.load(Ordering::SeqCst) {
            let _ = socket.close(Some(tungstenite::protocol::CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            }));
            let _ = socket.flush();
            break;
        }
        let text = match socket.read() {
            Ok(Message::Text(text)) => Some(text.as_str().to_owned()),
            Ok(Message::Binary(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Ok(Message::Close(_)) => break,
            Ok(_) => None,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(_) => break,
        };
        // The relay reads lines, so each frame is passed on as one.
        if let Some(message) = text.and_then(|text| serde_json::from_str::<Value>(&text).ok()) {
            if message["method"] == "tools/list" {
                listing.insert(message["id"].to_string());
            }
            if to_hub.send(format!("{message}\n").into_bytes()).is_err() {
                break;
            }
        }
        while let Ok(line) = from_hub.try_recv() {
            let answer = serde_json::from_str::<Value>(&line).unwrap_or_default();
            if answer.get("result").is_some() && listing.remove(&answer["id"].to_string()) {
                served = true;
                let _ = listed.send(());
            }
            if socket.send(Message::text(line)).is_err() {
                break 'open;
            }
        }
        let _ = socket.flush();
    }
    drop(to_hub);
    // A tool still running finishes on its own; nothing waits for it here.
    drop(relay);
    served
}

/// The relay's input: each frame the socket received, as a line.
struct Lines {
    frames: mpsc::Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
}

impl Lines {
    fn new(frames: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            frames,
            current: Cursor::new(Vec::new()),
        }
    }
}

impl Read for Lines {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let read = self.current.read(buffer)?;
            if read > 0 || buffer.is_empty() {
                return Ok(read);
            }
            // The socket is gone once its sender is.
            match self.frames.recv() {
                Ok(frame) => self.current = Cursor::new(frame),
                Err(_) => return Ok(0),
            }
        }
    }
}

/// The relay's output: each line it writes, as a frame for the socket.
struct Frames {
    pending: Vec<u8>,
    socket: mpsc::Sender<String>,
}

impl Frames {
    fn new(socket: mpsc::Sender<String>) -> Self {
        Self {
            pending: Vec::new(),
            socket,
        }
    }
}

impl Write for Frames {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..end]).into_owned();
            if !line.trim().is_empty() && self.socket.send(line).is_err() {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
