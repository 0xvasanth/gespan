//! Test doubles for the browser module: a fake CDP endpoint speaking the same
//! WebSocket/JSON framing as Chrome's `/devtools/browser/<id>`.
//!
//! Only compiled for tests (this crate's, and the app's through the
//! `testing` feature). The fake records every frame it receives, answers
//! through a pluggable responder, can emit events and can close or refuse the
//! connection, which is all the link, target and host tests need.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

/// One command frame the fake received.
#[derive(Clone, Debug)]
pub struct Received {
    pub id: u64,
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
    pub at: Instant,
}

/// An event the fake sends to the client.
#[derive(Clone, Debug)]
pub struct Event {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

impl Event {
    pub fn new(method: &str, params: Value, session: Option<&str>) -> Event {
        Event {
            method: method.to_string(),
            params,
            session: session.map(str::to_string),
        }
    }
}

/// What the responder wants done with a received command.
pub enum Response {
    /// `{"id", "result"}` right away.
    Result(Value),
    /// `{"id", "error": {code, message}}` right away.
    Error { code: i64, message: String },
    /// Never answer (the client must time out).
    Hold,
    /// Answer with the value after the delay.
    Delay(Duration, Value),
    /// Answer right away, then emit the events in order.
    ResultThen(Value, Vec<Event>),
    /// Emit the events, then answer: how Chrome replays what it kept on
    /// `Runtime.enable` or `Log.enable`.
    EventsThen(Vec<Event>, Value),
}

type Responder = Box<dyn FnMut(&Received) -> Response + Send>;

enum Mode {
    /// Full WebSocket server driven by the responder.
    Serve(Mutex<Responder>),
    /// Accept TCP, answer the upgrade with `403 Forbidden` (policy / wrong origin).
    Refuse,
    /// Accept TCP, never answer the upgrade (Chrome's consent dialog).
    Silent,
}

struct Shared {
    mode: Mode,
    received: Mutex<Vec<Received>>,
    outbox: Mutex<VecDeque<Event>>,
    close_requested: AtomicBool,
    client_closed: AtomicBool,
    connections: AtomicUsize,
    stop: AtomicBool,
    /// Streams held open in `Silent` mode so the client keeps waiting.
    held: Mutex<Vec<TcpStream>>,
    /// How long a `Serve` fake holds the upgrade before answering it: the
    /// user taking a while to accept Chrome's consent dialog.
    consent_delay: Duration,
}

/// A fake browser endpoint on a loopback port.
pub struct FakeCdp {
    pub url: String,
    shared: Arc<Shared>,
}

impl FakeCdp {
    /// A fake that answers with [`default_response`].
    pub fn start() -> FakeCdp {
        let counter = Arc::new(AtomicU64::new(1));
        FakeCdp::start_with(move |received| default_response(received, &counter))
    }

    /// A fake whose answers come from `responder`.
    pub fn start_with(responder: impl FnMut(&Received) -> Response + Send + 'static) -> FakeCdp {
        FakeCdp::launch(Mode::Serve(Mutex::new(Box::new(responder))))
    }

    /// A fake that refuses the WebSocket upgrade with an HTTP error.
    pub fn start_refusing() -> FakeCdp {
        FakeCdp::launch(Mode::Refuse)
    }

    /// A fake that accepts TCP and never completes the upgrade.
    pub fn start_silent() -> FakeCdp {
        FakeCdp::launch(Mode::Silent)
    }

    /// A [`FakeCdp::start`] fake that answers each WebSocket upgrade only
    /// after `delay`, as Chrome does once the user accepts its consent
    /// dialog.
    pub fn start_with_consent_delay(delay: Duration) -> FakeCdp {
        let counter = Arc::new(AtomicU64::new(1));
        FakeCdp::launch_with(
            Mode::Serve(Mutex::new(Box::new(move |received: &Received| {
                default_response(received, &counter)
            }))),
            delay,
        )
    }

    fn launch(mode: Mode) -> FakeCdp {
        FakeCdp::launch_with(mode, Duration::ZERO)
    }

    fn launch_with(mode: Mode, consent_delay: Duration) -> FakeCdp {
        FakeCdp::launch_on(
            TcpListener::bind("127.0.0.1:0").expect("bind loopback"),
            mode,
            consent_delay,
        )
    }

    fn launch_on(listener: TcpListener, mode: Mode, consent_delay: Duration) -> FakeCdp {
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let port = listener.local_addr().expect("local addr").port();
        let shared = Arc::new(Shared {
            mode,
            received: Mutex::new(Vec::new()),
            outbox: Mutex::new(VecDeque::new()),
            close_requested: AtomicBool::new(false),
            client_closed: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            held: Mutex::new(Vec::new()),
            consent_delay,
        });
        let acceptor = shared.clone();
        thread::Builder::new()
            .name("fake-cdp-accept".into())
            .spawn(move || accept_loop(listener, acceptor))
            .expect("spawn acceptor");
        FakeCdp {
            url: format!("ws://127.0.0.1:{port}/devtools/browser/fake-{port}"),
            shared,
        }
    }

    /// Every command frame received so far, in arrival order.
    pub fn received(&self) -> Vec<Received> {
        self.shared
            .received
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Queue an event for the current (or next) connection.
    pub fn emit(&self, method: &str, params: Value, session: Option<&str>) {
        self.shared
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(Event::new(method, params, session));
    }

    /// Poll until `pred` holds for the received log or `timeout` passes.
    pub fn wait_for(&self, pred: impl Fn(&[Received]) -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let log = self
                    .shared
                    .received
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if pred(&log) {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(3));
        }
    }

    /// Close the current WebSocket connection from the server side.
    pub fn close_current(&self) {
        self.shared.close_requested.store(true, Ordering::Release);
    }

    /// Wait until the client closed (or lost) its connection.
    pub fn wait_for_close(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !self.shared.client_closed.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(3));
        }
        true
    }

    /// How many WebSocket connections completed the upgrade so far.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::Acquire)
    }

    /// A fresh profile folder whose `DevToolsActivePort` points at this
    /// fake, for [`crate::protocol::BrowserChoice::UserDataDir`] in Attach
    /// mode.
    pub fn profile_dir(&self) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gespan-fake-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("profile dir");
        let rest = self
            .url
            .strip_prefix("ws://127.0.0.1:")
            .expect("loopback url");
        let (port, path) = rest.split_once('/').expect("port and path");
        std::fs::write(
            dir.join(crate::discovery::ACTIVE_PORT_FILE),
            format!("{port}\n/{path}\n"),
        )
        .expect("DevToolsActivePort");
        dir
    }
}

impl Drop for FakeCdp {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let held = std::mem::take(&mut *self.shared.held.lock().unwrap_or_else(|e| e.into_inner()));
        for stream in held {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Makes a [`FakeCdp::start_stalling`] fake stop reading its socket.
pub struct Stall {
    armed: Arc<AtomicBool>,
    stalled: mpsc::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
    receive_buffer: usize,
}

/// What [`FakeCdp::start_stalling`] asks for as the fake's socket receive
/// buffer, and what a stall test asks for as the client's send buffer. Small
/// and fixed, so a stall needs only a few MiB on every OS: left alone, the
/// loopback buffers autotune (on Windows CI they took a whole 48 MiB frame).
pub const STALL_SOCKET_BUFFER: usize = 32 * 1024;

impl Stall {
    /// The receive buffer the OS gave the fake's sockets (Linux doubles the
    /// requested size).
    pub fn receive_buffer(&self) -> usize {
        self.receive_buffer
    }

    /// The next `Browser.getVersion` the fake reads makes it stop reading
    /// (until [`Stall::release`]); send one, then call [`Stall::wait`].
    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    /// Block until the fake has stopped reading.
    pub fn wait(&self) {
        self.stalled
            .recv_timeout(Duration::from_secs(5))
            .expect("the fake stopped reading");
    }

    /// Let the fake read again.
    pub fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl FakeCdp {
    /// A [`FakeCdp::start`] fake that can be told to stop reading its
    /// socket, so the client's writes fill the socket buffers and stall.
    pub fn start_stalling() -> (FakeCdp, Stall) {
        let armed = Arc::new(AtomicBool::new(false));
        let (stalled_tx, stalled) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let counter = AtomicU64::new(1);
        let trigger = armed.clone();
        // Sized on the listener, before any connection: an accepted socket
        // inherits it, and its TCP window is negotiated from it.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let receive_buffer =
            set_socket_buffer(&listener, SocketBuffer::Receive, STALL_SOCKET_BUFFER)
                .expect("size the fake's receive buffer");
        let responder = move |received: &Received| {
            if received.method == "Browser.getVersion" && trigger.swap(false, Ordering::AcqRel) {
                let _ = stalled_tx.send(());
                let _ = release_rx.recv();
            }
            default_response(received, &counter)
        };
        let server = FakeCdp::launch_on(
            listener,
            Mode::Serve(Mutex::new(Box::new(responder))),
            Duration::ZERO,
        );
        let stall = Stall {
            armed,
            stalled,
            release: Some(release),
            receive_buffer,
        };
        (server, stall)
    }
}

/// A socket's kernel buffer, for [`set_socket_buffer`].
#[derive(Clone, Copy, Debug)]
pub enum SocketBuffer {
    /// `SO_SNDBUF`
    Send,
    /// `SO_RCVBUF`
    Receive,
}

#[cfg(unix)]
use std::os::fd::AsRawFd as AsRawSock;
#[cfg(windows)]
use std::os::windows::io::AsRawSocket as AsRawSock;

/// Set a socket's send or receive buffer and return the size the OS settled
/// on. Setting it also turns off the OS's autotuning of that buffer, which
/// is what makes a stalled write deterministic across platforms.
pub fn set_socket_buffer(
    socket: &impl AsRawSock,
    which: SocketBuffer,
    bytes: usize,
) -> std::io::Result<usize> {
    let requested = i32::try_from(bytes)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "buffer size too large"))?;
    #[cfg(unix)]
    {
        let fd = socket.as_raw_fd();
        let name = match which {
            SocketBuffer::Send => libc::SO_SNDBUF,
            SocketBuffer::Receive => libc::SO_RCVBUF,
        };
        let size = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let value: libc::c_int = requested;
        // SAFETY: `fd` is an open socket borrowed for the call; the option
        // value is a live `c_int` of the length passed.
        let set = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                name,
                (&value as *const libc::c_int).cast(),
                size,
            )
        };
        if set != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut actual: libc::c_int = 0;
        let mut len = size;
        // SAFETY: as above; `actual` and `len` outlive the call.
        let got = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                name,
                (&mut actual as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        if got != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(usize::try_from(actual).unwrap_or(0))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Networking::WinSock::{
            getsockopt, setsockopt, WSAGetLastError, SOCKET, SOL_SOCKET, SO_RCVBUF, SO_SNDBUF,
        };
        let raw = socket.as_raw_socket() as SOCKET;
        let name = match which {
            SocketBuffer::Send => SO_SNDBUF,
            SocketBuffer::Receive => SO_RCVBUF,
        };
        let size = std::mem::size_of::<i32>() as i32;
        let value: i32 = requested;
        // SAFETY: `raw` is an open socket borrowed for the call; the option
        // value is a live `i32` of the length passed.
        let set = unsafe { setsockopt(raw, SOL_SOCKET, name, (&value as *const i32).cast(), size) };
        if set != 0 {
            return Err(std::io::Error::from_raw_os_error(unsafe {
                WSAGetLastError()
            }));
        }
        let mut actual: i32 = 0;
        let mut len = size;
        // SAFETY: as above; `actual` and `len` outlive the call.
        let got = unsafe {
            getsockopt(
                raw,
                SOL_SOCKET,
                name,
                (&mut actual as *mut i32).cast(),
                &mut len,
            )
        };
        if got != 0 {
            return Err(std::io::Error::from_raw_os_error(unsafe {
                WSAGetLastError()
            }));
        }
        Ok(usize::try_from(actual).unwrap_or(0))
    }
}

/// The stock Chrome-like answers used by [`FakeCdp::start`].
pub fn default_response(received: &Received, counter: &AtomicU64) -> Response {
    match received.method.as_str() {
        "Browser.getVersion" => Response::Result(json!({
            "protocolVersion": "1.3",
            "product": "Chrome/144.0.7559.60",
            "revision": "@fake",
            "userAgent": "Mozilla/5.0 FakeChrome/144.0.0.0",
            "jsVersion": "14.4.0"
        })),
        "Target.getTargets" => Response::Result(json!({ "targetInfos": [] })),
        "Target.createTarget" => {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            Response::Result(json!({ "targetId": format!("T{n}") }))
        }
        "Target.attachToTarget" => {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            Response::Result(json!({ "sessionId": format!("S{n}") }))
        }
        "Page.createIsolatedWorld" => {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            Response::Result(json!({ "executionContextId": n }))
        }
        "DOM.resolveNode" if received.params["backendNodeId"] == json!(-1) => Response::Error {
            code: -32000,
            message: "No node with given id found".into(),
        },
        _ => Response::Result(json!({})),
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let conn = shared.clone();
                thread::Builder::new()
                    .name("fake-cdp-conn".into())
                    .spawn(move || handle_connection(stream, conn))
                    .expect("spawn connection");
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn handle_connection(mut stream: TcpStream, shared: Arc<Shared>) {
    // On macOS and Windows an accepted socket inherits the listener's
    // non-blocking mode; the read timeouts below need a blocking one.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    match &shared.mode {
        Mode::Silent => {
            // Hold the stream so the client sees neither data nor EOF.
            shared
                .held
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(stream);
        }
        Mode::Refuse => {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        Mode::Serve(responder) => {
            if !shared.consent_delay.is_zero() {
                thread::sleep(shared.consent_delay);
            }
            // Chrome takes messages as large as the link sends (the stall
            // tests write tens of MiB); tungstenite's default stops at 16.
            let config = tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(crate::link::MAX_MESSAGE_BYTES))
                .max_frame_size(Some(crate::link::MAX_MESSAGE_BYTES));
            let socket = match tungstenite::accept_with_config(stream, Some(config)) {
                Ok(socket) => socket,
                Err(_) => return,
            };
            shared.connections.fetch_add(1, Ordering::AcqRel);
            shared.client_closed.store(false, Ordering::Release);
            shared.close_requested.store(false, Ordering::Release);
            serve(socket, responder, &shared);
            shared.client_closed.store(true, Ordering::Release);
        }
    }
}

fn serve(mut socket: WebSocket<TcpStream>, responder: &Mutex<Responder>, shared: &Shared) {
    let _ = socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(5)));
    let mut delayed: Vec<(Instant, String)> = Vec::new();
    let mut closing_sent = false;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            let _ = socket.close(None);
            let _ = socket.flush();
            return;
        }
        if shared.close_requested.swap(false, Ordering::AcqRel) && !closing_sent {
            closing_sent = true;
            if socket.close(None).is_err() || socket.flush().is_err() {
                return;
            }
        }
        // Events queued by the test.
        loop {
            let next = shared
                .outbox
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front();
            let Some(event) = next else { break };
            if write_frame(&mut socket, event_frame(&event)).is_err() {
                return;
            }
        }
        // Delayed answers whose time came.
        let now = Instant::now();
        let mut i = 0;
        while i < delayed.len() {
            if delayed[i].0 <= now {
                let (_, text) = delayed.remove(i);
                if write_frame(&mut socket, text).is_err() {
                    return;
                }
            } else {
                i += 1;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let Some(received) = parse_command(text.as_str()) else {
                    continue;
                };
                let session = received.session.clone();
                let id = received.id;
                shared
                    .received
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(received.clone());
                let response = (responder.lock().unwrap_or_else(|e| e.into_inner()))(&received);
                let frames: Vec<String> = match response {
                    Response::Result(value) => vec![result_frame(id, &session, value)],
                    Response::Error { code, message } => {
                        vec![error_frame(id, &session, code, &message)]
                    }
                    Response::Hold => Vec::new(),
                    Response::Delay(delay, value) => {
                        delayed.push((Instant::now() + delay, result_frame(id, &session, value)));
                        Vec::new()
                    }
                    Response::ResultThen(value, events) => {
                        let mut frames = vec![result_frame(id, &session, value)];
                        frames.extend(events.iter().map(event_frame));
                        frames
                    }
                    Response::EventsThen(events, value) => {
                        let mut frames: Vec<_> = events.iter().map(event_frame).collect();
                        frames.push(result_frame(id, &session, value));
                        frames
                    }
                };
                for frame in frames {
                    if write_frame(&mut socket, frame).is_err() {
                        return;
                    }
                }
            }
            Ok(Message::Close(_)) => {
                // tungstenite queued the close reply; keep reading until it is flushed.
                shared.client_closed.store(true, Ordering::Release);
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}

fn write_frame(socket: &mut WebSocket<TcpStream>, text: String) -> Result<(), tungstenite::Error> {
    socket.send(Message::text(text))?;
    socket.flush()
}

fn parse_command(text: &str) -> Option<Received> {
    let value: Value = serde_json::from_str(text).ok()?;
    Some(Received {
        id: value.get("id")?.as_u64()?,
        method: value.get("method")?.as_str()?.to_string(),
        params: value.get("params").cloned().unwrap_or(Value::Null),
        session: value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
        at: Instant::now(),
    })
}

fn result_frame(id: u64, session: &Option<String>, result: Value) -> String {
    let mut frame = json!({ "id": id, "result": result });
    if let Some(session) = session {
        frame["sessionId"] = Value::String(session.clone());
    }
    frame.to_string()
}

fn error_frame(id: u64, session: &Option<String>, code: i64, message: &str) -> String {
    let mut frame = json!({ "id": id, "error": { "code": code, "message": message } });
    if let Some(session) = session {
        frame["sessionId"] = Value::String(session.clone());
    }
    frame.to_string()
}

fn event_frame(event: &Event) -> String {
    let mut frame = json!({ "method": event.method, "params": event.params });
    if let Some(session) = &event.session {
        frame["sessionId"] = Value::String(session.clone());
    }
    frame.to_string()
}

/// A tiny HTTP server on 127.0.0.1 for tests that drive a real browser:
/// `page` turns each request path into the HTML served for it. Every path
/// asked for is recorded. The server runs until the test process ends.
pub struct PageServer {
    port: u16,
    asked: Arc<Mutex<Vec<String>>>,
}

impl PageServer {
    pub fn start(page: impl Fn(&str) -> String + Send + 'static) -> PageServer {
        let server = TcpListener::bind("127.0.0.1:0").expect("bind page server");
        let port = server.local_addr().expect("page server address").port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let recorded = asked.clone();
        thread::spawn(move || {
            for mut stream in server.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let line = String::from_utf8_lossy(&buf[..n]);
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(path.clone());
                let body = page(&path);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        PageServer { port, asked }
    }

    /// `http://127.0.0.1:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Whether `path` was asked for.
    pub fn asked(&self, path: &str) -> bool {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|p| p == path)
    }
}
