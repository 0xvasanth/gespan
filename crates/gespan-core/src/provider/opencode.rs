//! Native OpenCode HTTP/SSE transport.
//! v1 owns a local `serve` child; v2 connects to OpenCode's managed service.
use super::*;
use crate::diagnostics::metrics;
use std::collections::VecDeque;
use std::sync::{atomic::AtomicUsize, OnceLock};
mod client;
mod client_mcp;
mod events;
use client::Client;

const START: &str = "agent.start.opencode";
/// The turn's prompt may name these, so a turn that expected them and didn't
/// get them is told.
const NO_TOOLS: &str = "Gespan's own tools and CLIs (visual, ticket, card, app and control) aren't available in this OpenCode session, whatever follows says. Don't look for them; tell the user if the task needs one.";

enum Wire {
    Connected,
    Frame(Value),
    Ended(String),
    Admitted(Result<Option<String>, String>),
    Steered(crate::follow_up::Message, Result<Option<String>, String>),
}
struct StreamStop {
    stopped: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<std::net::TcpStream>>>,
}
impl Drop for StreamStop {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(socket) = self.socket.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}
struct Server {
    child: std::process::Child,
    url: String,
    /// The tool credential this server was given, while it may still work.
    access: Option<crate::tool_servers::Access>,
    _registration: process::Registration,
    _account: crate::accounts::Tracked,
}
impl Drop for Server {
    fn drop(&mut self) {
        process::terminate(&mut self.child);
    }
}
struct Idle {
    server: Server,
    session: String,
    account_id: String,
    cwd: PathBuf,
    key: String,
    at: Instant,
    owner: Option<i64>,
}
static IDLE_LIMIT: AtomicUsize = AtomicUsize::new(0);
fn idle() -> &'static Mutex<Vec<Idle>> {
    static IDLE: OnceLock<Mutex<Vec<Idle>>> = OnceLock::new();
    IDLE.get_or_init(Default::default)
}
pub(super) fn configure(limit: usize) {
    IDLE_LIMIT.store(limit.min(5), Ordering::Relaxed);
    thread::spawn(parked::enforce_idle_limit);
}
pub(super) fn idle_entries() -> Vec<(u32, Instant, Option<i64>)> {
    idle()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|entry| (entry.server.child.id(), entry.at, entry.owner))
        .collect()
}
pub(super) fn retire_idle(pids: &std::collections::HashSet<u32>) {
    let retired = {
        let mut pool = idle().lock().unwrap_or_else(|e| e.into_inner());
        let mut retired = Vec::new();
        let mut index = 0;
        while index < pool.len() {
            if pids.contains(&pool[index].server.child.id()) {
                retired.push(pool.remove(index));
            } else {
                index += 1;
            }
        }
        retired
    };
    drop(retired);
}
pub(super) fn discard_worktree(cwd: &Path) {
    let retired = {
        let mut pool = idle().lock().unwrap_or_else(|e| e.into_inner());
        let mut retired = Vec::new();
        let mut index = 0;
        while index < pool.len() {
            if crate::files::is_equal_or_inside(
                &pool[index].cwd.to_string_lossy(),
                &cwd.to_string_lossy(),
            ) {
                retired.push(pool.remove(index));
            } else {
                index += 1;
            }
        }
        retired
    };
    drop(retired);
}
pub(super) fn forget_session(provider_id: &str, account_id: &str, cwd: &Path) {
    events::forget_session(provider_id, account_id, cwd);
    let retired = {
        let mut pool = idle().lock().unwrap_or_else(|e| e.into_inner());
        let mut retired = Vec::new();
        let mut index = 0;
        while index < pool.len() {
            if pool[index].session == provider_id
                && pool[index].account_id == account_id
                && pool[index].cwd == cwd
            {
                retired.push(pool.remove(index));
            } else {
                index += 1;
            }
        }
        retired
    };
    drop(retired);
}
fn park(server: Server, request: &Request, session: String) {
    if IDLE_LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    process::mark_idle(server.child.id(), true);
    {
        let mut pool = idle().lock().unwrap_or_else(|e| e.into_inner());
        pool.push(Idle {
            server,
            session,
            account_id: request.settings.account.id.clone(),
            cwd: request.cwd.clone(),
            key: parked::key(request),
            at: Instant::now(),
            owner: process::owner(),
        });
    }
    parked::enforce_idle_limit();
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(3 * 60));
        let retired = {
            let mut pool = idle().lock().unwrap_or_else(|e| e.into_inner());
            let mut retired = Vec::new();
            let mut index = 0;
            while index < pool.len() {
                if pool[index].at.elapsed() >= Duration::from_secs(3 * 60) {
                    retired.push(pool.remove(index));
                } else {
                    index += 1;
                }
            }
            retired
        };
        drop(retired);
    });
}
fn command(request: &Request) -> Result<Command, String> {
    let mut command = Command::new(&request.binary);
    crate::scm::shell::apply_gui_env(&mut command);
    command.envs(&request.settings.environment);
    request.settings.account.apply(request.kind, &mut command)?;
    command
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    Ok(command)
}
fn exec(request: &Request, args: &[&str], cancel: &AtomicBool) -> Result<String, String> {
    let mut command = command(request)?;
    command.args(args);
    let mut child = process::spawn(&mut command).map_err(|e| e.to_string())?;
    let _registration = process::register(child.id());
    let _account = crate::accounts::track(child.id(), request.kind, &request.settings.account);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let output = Arc::new(Mutex::new(String::new()));
    let mut readers = Vec::new();
    for stream in [Box::new(stdout) as Box<dyn Read + Send>, Box::new(stderr)] {
        let output = output.clone();
        readers.push(thread::spawn(move || {
            // Lossy: one invalid byte must not discard the whole
            // stream (and with it the version line).
            let mut bytes = Vec::new();
            let _ = stream.take(8 * 1024 * 1024).read_to_end(&mut bytes);
            output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_str(&String::from_utf8_lossy(&bytes));
        }));
    }
    let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(ProviderError::Stopped.into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(format!("OpenCode {} failed", args.join(" ")))
                };
            }
            Err(error) => break Err(error.to_string()),
            _ => {}
        }
        if started.elapsed() >= Duration::from_secs(30) {
            break Err(format!("OpenCode {} timed out", args.join(" ")));
        }
        thread::sleep(TICK);
    };
    process::terminate(&mut child);
    for reader in readers {
        let _ = reader.join();
    }
    let text = output.lock().unwrap_or_else(|e| e.into_inner()).clone();
    result.map(|_| text.clone()).map_err(|error: String| {
        if text.trim().is_empty() {
            error
        } else {
            format!("{error}: {}", text.trim())
        }
    })
}
fn announce_missing_tools(request: &mut Request, mounted: bool) {
    if !mounted && request.settings.mount.is_some() {
        request.prompt = format!("{NO_TOOLS}\n\n{}", request.prompt);
    }
}

/// A v2 turn's tools, for as long as the turn runs.
struct MountedV2 {
    /// The turn's credential, held like v1's server holds its own.
    _access: crate::tool_servers::Access,
    route: RouteV2,
}

enum RouteV2 {
    /// Gespan serves them over a socket of its own, and keeps the credential.
    /// Dropping the socket takes them off the service.
    Hosted { _socket: client_mcp::Hosted },
    /// The service holds the credential and calls the hub itself.
    Remote { client: Client, path: String },
}

impl Drop for MountedV2 {
    /// The server would otherwise stay on the shared service after the turn,
    /// with its token, for as long as the service keeps the folder open.
    fn drop(&mut self) {
        if let RouteV2::Remote { client, path } = &self.route {
            if client
                .request_timeout("DELETE", path, None, Duration::from_secs(5))
                .is_err()
            {
                tracing::warn!("OpenCode kept the tool endpoint after the turn");
            }
        }
    }
}

/// v2's service is shared by every conversation in the folder, so this
/// session's server gets a name of its own, the session's permissions hide
/// everyone else's, and the credential serves calls from this conversation
/// only, from before the service ever sees it. Each turn mounts it again with
/// a fresh credential, and takes it away when it ends. From 2.2.0 the service
/// can call a server its client hosts, so the token never leaves Gespan; older
/// ones are handed the endpoint.
fn mount_tools_v2(
    client: &Client,
    mount: Option<&crate::tool_servers::Mount>,
    conversation: &str,
    hosted: bool,
) -> Option<MountedV2> {
    let mount = mount?;
    let access = crate::tool_servers::Access::open_opencode(mount, conversation)?;
    let name = crate::tool_servers::opencode_name(mount.session);
    if hosted {
        if let Some(socket) = client_mcp::host(client, &name, access.connection()) {
            return Some(MountedV2 {
                _access: access,
                route: RouteV2::Hosted { _socket: socket },
            });
        }
    }
    let path = client.path(&format!("/experimental/mcp/{}", client::encode(&name)));
    let body = json!({"config":access.opencode_config()});
    match client.request("PUT", &path, Some(body)) {
        Ok(_) => Some(MountedV2 {
            _access: access,
            route: RouteV2::Remote {
                client: client.clone(),
                path,
            },
        }),
        // The error stays out of the log: it may quote the token back.
        Err(_) => {
            tracing::warn!("OpenCode refused the tool endpoint");
            // A refusal may still have stored the server: its credential
            // ends here, and the entry goes too where the service allows.
            access.withdraw(mount.session);
            let _ = client.request("DELETE", &path, None);
            None
        }
    }
}

/// A kept server whose credential still works already has the tools; a new
/// one would only make it reconnect. The turn goes on without them if this
/// server can't take them.
fn mount_tools(
    client: &Client,
    held: &mut Option<crate::tool_servers::Access>,
    mount: Option<&crate::tool_servers::Mount>,
) {
    let Some(mount) = mount else {
        return;
    };
    if held
        .as_ref()
        .is_some_and(|access| access.is_live(mount.session))
    {
        return;
    }
    let Some(access) = crate::tool_servers::Access::open(mount) else {
        return;
    };
    match client.request("POST", &client.path("/mcp"), Some(access.opencode_server())) {
        Ok(_) => *held = Some(access),
        // The error stays out of the log: it may quote the token back.
        Err(_) => tracing::warn!("OpenCode refused the tool endpoint"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Generation {
    v2: bool,
    /// The service can call an MCP server its client hosts over a WebSocket.
    hosts_tools: bool,
}

fn generation(request: &Request, cancel: &AtomicBool) -> Result<Generation, String> {
    parse_generation(&exec(request, &["--version"], cancel)?)
}

fn parse_generation(output: &str) -> Result<Generation, String> {
    // No word boundaries: `v1.14.20` and ANSI-colored output must still match.
    let re = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)").unwrap();
    let version = re.captures(output).ok_or_else(|| {
        format!(
            "Unable to determine OpenCode version from {:?}. Gespan requires v1.14.19 or newer.",
            output.trim().chars().take(200).collect::<String>()
        )
    })?;
    let components: Vec<u64> = (1..=3).map(|ix| version[ix].parse().unwrap_or(0)).collect();
    let at_least = |least: [u64; 3]| components.as_slice() >= least.as_slice();
    match components[0] {
        1 if at_least([1, 14, 19]) => Ok(Generation {
            v2: false,
            hosts_tools: false,
        }),
        2 if at_least([2, 0, 15]) => Ok(Generation {
            v2: true,
            hosts_tools: at_least([2, 2, 0]),
        }),
        1 => Err(format!(
            "OpenCode v{} is too old. Upgrade to v1.14.19 or newer.",
            &version[0]
        )),
        2 => Err(format!(
            "OpenCode v{} is too old. Upgrade to v2.0.15 or newer.",
            &version[0]
        )),
        _ => Err(format!(
            "OpenCode v{} is not supported. Gespan supports OpenCode v1 and v2.",
            &version[0]
        )),
    }
}
fn service_url(output: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)^https?://(?:127\.0\.0\.1|localhost):\d+/?$").unwrap();
    output
        .lines()
        .map(str::trim)
        .find(|line| re.is_match(line))
        .map(|line| line.trim_end_matches('/').into())
}
fn connect(
    request: &Request,
    cancel: &AtomicBool,
    v2: bool,
) -> Result<(Client, Option<Server>), String> {
    let directory = request.cwd.to_string_lossy().into_owned();
    if v2 {
        let mut url = exec(request, &["service", "status"], cancel)
            .ok()
            .and_then(|output| service_url(&output));
        if url.is_none() {
            url = service_url(&exec(request, &["service", "start"], cancel)?);
            if url.is_none() {
                url = service_url(&exec(request, &["service", "status"], cancel)?);
            }
        }
        let url = url.ok_or("OpenCode background service did not report a local URL.")?;
        let password = exec(request, &["service", "get", "password"], cancel)?
            .trim()
            .to_owned();
        if password.is_empty() {
            return Err("OpenCode background service did not report a password.".into());
        }
        return Ok((Client::new(url, directory, true, Some(password)), None));
    }
    if let Some(id) = request.provider_id.as_deref() {
        let found = {
            let mut pool = idle().lock().unwrap_or_else(|e| e.into_inner());
            let key = parked::key(request);
            pool.iter()
                .position(|idle| idle.session == id && idle.key == key)
                .map(|index| pool.remove(index))
        };
        if let Some(mut found) = found {
            if found.at.elapsed() < Duration::from_secs(3 * 60)
                && matches!(found.server.child.try_wait(), Ok(None))
            {
                process::mark_idle(found.server.child.id(), false);
                return Ok((
                    Client::new(found.server.url.clone(), directory, false, None),
                    Some(found.server),
                ));
            }
        }
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    let mut child = command(request)?
        .args(["serve", "--hostname=127.0.0.1", &format!("--port={port}")])
        .spawn()
        .map_err(|e| e.to_string())?;
    let registration = process::register(child.id());
    let account = crate::accounts::track(child.id(), request.kind, &request.settings.account);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (send, receive) = mpsc::channel();
    for stream in [Box::new(stdout) as Box<dyn Read + Send>, Box::new(stderr)] {
        let send = send.clone();
        thread::spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if !sent && line.to_lowercase().contains("listening") {
                    // The advertised URL may contain punctuation after it.
                    let url = line
                        .split_whitespace()
                        .find(|part| part.starts_with("http://") || part.starts_with("https://"));
                    if let Some(url) = url {
                        let _ = send.send(url.trim_end_matches([',', '.', ';']).to_owned());
                        sent = true;
                    }
                }
            }
        });
    }
    drop(send);
    let mut server = Server {
        access: None,
        child,
        url: String::new(),
        _registration: registration,
        _account: account,
    };
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Stopped.into());
        }
        if !matches!(server.child.try_wait(), Ok(None)) {
            return Err("OpenCode server exited before it was ready".into());
        }
        match receive.recv_timeout(TICK) {
            Ok(url) => {
                server.url = url.clone();
                return Ok((Client::new(url, directory, false, None), Some(server)));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("OpenCode server exited before reporting a URL".into());
            }
            _ => {}
        }
        if started.elapsed() >= Duration::from_secs(30) {
            return Err("OpenCode server startup timed out".into());
        }
    }
}

pub(super) fn run(
    mut request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    probe: bool,
    compact: bool,
    mut emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    let started = generation(&request, &cancel);
    if probe {
        return catalog(&request, &cancel, started?.v2).map(Some);
    }
    // A start is the version check and the connection; it counts once. A
    // Stop is not a failed start.
    let start_failed = |error: &String| {
        if !cancel.load(Ordering::Relaxed) {
            metrics::failure(START, metrics::ErrorKind::of_message(error));
        }
    };
    let started = started.inspect_err(start_failed)?;
    let v2 = started.v2;
    if cancel.load(Ordering::Relaxed) {
        return Err(ProviderError::Stopped.into());
    }
    request.prompt = commands::prepare_prompt(
        Kind::OpenCode,
        request
            .settings
            .prepared_prompt
            .as_deref()
            .unwrap_or(&request.prompt),
        &request.cwd.to_string_lossy(),
        &request.settings.attachments,
    )?;
    let (client, mut server) = connect(&request, &cancel, v2)
        .inspect(|_| metrics::success(START))
        .inspect_err(start_failed)?;
    // v1's `serve` child belongs to this conversation, and gets the tools
    // before its session. v2's service is shared, so they wait for the
    // conversation they're bound to.
    if let Some(server) = server.as_mut() {
        mount_tools(&client, &mut server.access, request.settings.mount.as_ref());
    }
    let session = client.session(&request)?;
    let v2_tools = v2
        .then(|| {
            mount_tools_v2(
                &client,
                request.settings.mount.as_ref(),
                &session,
                started.hosts_tools,
            )
        })
        .flatten();
    let mounted = if v2 {
        v2_tools.is_some()
    } else {
        server
            .as_ref()
            .is_some_and(|server| server.access.is_some())
    };
    announce_missing_tools(&mut request, mounted);
    if request.provider_id.is_some() {
        let _ = client.repair_unsupported_file_turn(&session);
    }
    request.provider_id = Some(session.clone());
    emit(vec![Event::SessionId(session.clone())])?;
    let mut state = events::State::new(request.clone(), session.clone(), compact);
    let (send, receive) = mpsc::channel();
    let stop = StreamStop {
        stopped: Arc::new(AtomicBool::new(false)),
        socket: Arc::new(Mutex::new(None)),
    };
    client.events(send.clone(), stop.stopped.clone(), stop.socket.clone());
    let started = Instant::now();
    let mut connected = false;
    let mut steering = false;
    let mut aborted = false;
    let mut held = false;
    let mut gate = super::hold::Gate::default();
    let mut queue = VecDeque::new();
    let result = (|| -> Result<(), String> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                state.cancel(&client);
                aborted = true;
                return Err(ProviderError::Stopped.into());
            }
            if let Some(server) = server.as_mut() {
                if !matches!(server.child.try_wait(), Ok(None)) {
                    return Err("OpenCode server exited".into());
                }
            }
            if let Some(controls) = &controls {
                while let Ok(control) = controls.try_recv() {
                    if let Decision::Steer(message) = control.decision {
                        queue.push_back(message);
                    } else if matches!(control.decision, Decision::Hold) {
                        gate.request();
                    } else if matches!(control.decision, Decision::Release) {
                        gate.release();
                    } else {
                        let events = state.respond(&client, control)?;
                        if !events.is_empty() {
                            emit(events)?;
                        }
                    }
                }
            }
            if connected && state.admitted && !steering {
                if let Some(message) = queue.pop_front() {
                    steering = true;
                    let client = client.clone();
                    let session = session.clone();
                    let mut request = request.clone();
                    let send = send.clone();
                    thread::spawn(move || {
                        request.settings.attachments = message.attachments.clone();
                        let result = commands::prepare_prompt(
                            Kind::OpenCode,
                            message.prepared_prompt.as_deref().unwrap_or(&message.text),
                            &request.cwd.to_string_lossy(),
                            &message.attachments,
                        )
                        .and_then(|prompt| {
                            request.prompt = prompt;
                            client.prompt(&request, &session, true, false)
                        });
                        let _ = send.send(Wire::Steered(message, result));
                    });
                }
            }
            match receive.recv_timeout(TICK) {
                Ok(Wire::Connected) => {
                    connected = true;
                    let client = client.clone();
                    let request = request.clone();
                    let session = session.clone();
                    let send = send.clone();
                    thread::spawn(move || {
                        let _ = send.send(Wire::Admitted(
                            client.prompt(&request, &session, false, compact),
                        ));
                    });
                }
                Ok(Wire::Frame(event)) => {
                    let events = state.frame(&client, event, controls.is_some())?;
                    for event in &events {
                        gate.observe(event);
                    }
                    if !events.is_empty() {
                        emit(events)?;
                    }
                }
                Ok(Wire::Admitted(result)) => {
                    state.admit(result?);
                    if compact && !v2 {
                        return emit(vec![Event::Done]);
                    }
                    if !compact {
                        emit(vec![Event::Ready])?;
                    }
                }
                Ok(Wire::Steered(message, result)) => {
                    steering = false;
                    match result {
                        Ok(_) => {
                            state.protocol.text_key = None;
                            emit(vec![Event::FollowUp(message)])?;
                        }
                        Err(error) => emit(vec![Event::FollowUpFailed {
                            id: message.id,
                            error,
                        }])?,
                    }
                }
                Ok(Wire::Ended(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("OpenCode event stream ended unexpectedly.".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(result) = state.terminal.take() {
                result?;
                return emit(vec![Event::Done]);
            }
            let awaiting = !state.approvals.is_empty() || !state.questions.is_empty();
            match gate.decide(awaiting) {
                Some(super::hold::Action::Interrupt) => {
                    emit(vec![Event::HoldSettled(HoldPoint::Interrupted)])?;
                    state.cancel(&client);
                    aborted = true;
                    held = true;
                    return Err(ProviderError::Held.into());
                }
                Some(super::hold::Action::Settle(point)) => emit(vec![Event::HoldSettled(point)])?,
                None => {}
            }
            if request.settings.mount.is_some() {
                emit(Vec::new())?;
            }
            if !connected && started.elapsed() >= Duration::from_secs(30) {
                return Err("OpenCode event stream startup timed out".into());
            }
            if compact && started.elapsed() >= Duration::from_secs(30 * 60) {
                return Err("OpenCode compaction timed out".into());
            }
            if request.settings.text_only && started.elapsed() >= Duration::from_secs(45) {
                return Err("Background text request timed out".into());
            }
        }
    })();
    drop(stop);
    drop(v2_tools);
    if result.is_err() && !aborted {
        state.cancel(&client);
        aborted = true;
    }
    // Stop aborts the prompt while retaining the local HTTP server, just as
    // cancelOpenCodeTurn does. A transport failure still retires its server.
    if result.is_ok() || (aborted && (cancel.load(Ordering::Relaxed) || held)) {
        if let Some(server) = server.take() {
            park(server, &request, session);
        }
    }
    result.map(|_| None)
}

fn catalog(request: &Request, cancel: &AtomicBool, v2: bool) -> Result<Value, String> {
    let mut models = Vec::new();
    let mut agents = Vec::new();
    if v2 {
        let (mut client, _) = connect(request, cancel, true)?;
        client.directory.clear();
        let raw = client.request("GET", &client.path("/model"), None)?;
        let providers = client.request("GET", &client.path("/provider"), None)?;
        let raw_agents = client
            .request("GET", &client.path("/agent"), None)
            .unwrap_or_else(|_| json!([]));
        for agent in raw_agents.as_array().into_iter().flatten() {
            let id = agent["id"]
                .as_str()
                .or_else(|| agent["name"].as_str())
                .unwrap_or("");
            let primary = agent["mode"]
                .as_str()
                .is_none_or(|mode| matches!(mode, "primary" | "all"));
            if primary
                && !id.is_empty()
                && agent["hidden"] != true
                && !matches!(id, "compaction" | "summary" | "title")
            {
                agents.push(json!({"value":id,"name":agent["name"].as_str().unwrap_or(id)}));
            }
        }
        for row in raw.as_array().into_iter().flatten() {
            let Some(provider) = row["providerID"].as_str() else {
                continue;
            };
            let Some(raw_id) = row["id"].as_str().or_else(|| row["modelID"].as_str()) else {
                continue;
            };
            if row["enabled"] == false {
                continue;
            }
            let id = raw_id
                .strip_prefix(&format!("{provider}/"))
                .unwrap_or(raw_id);
            let provider_name = providers
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["id"] == provider)
                .and_then(|p| p["name"].as_str())
                .unwrap_or(provider);
            models.push(catalog_row(
                &format!("{provider}/{id}"),
                row,
                Some(provider_name),
                &agents,
            ));
        }
    } else {
        if let Ok(output) = exec(request, &["agent", "list"], cancel) {
            let re = regex::Regex::new(r"^(.+)\s+\((\S+)\)\s*$").unwrap();
            for line in output.lines() {
                if let Some(row) = re.captures(line) {
                    let id = row[1].trim();
                    if matches!(&row[2], "primary" | "all")
                        && !matches!(id, "compaction" | "summary" | "title")
                    {
                        agents.push(json!({"value":id,"name":id}));
                    }
                }
            }
        }
        let output = exec(request, &["models", "--verbose"], cancel)?;
        let slug = regex::Regex::new(r"^(\S+/\S+)\s*$").unwrap();
        let mut current: Option<String> = None;
        let mut json = String::new();
        for line in output.lines() {
            let next = slug.captures(line).map(|row| row[1].to_owned());
            if next.is_some() {
                if let Some(id) = current.take() {
                    if let Ok(row) = serde_json::from_str::<Value>(&json) {
                        models.push(catalog_row(&id, &row, None, &agents));
                    }
                }
                json.clear();
                current = next;
            } else if current.is_some() {
                json.push_str(line);
                json.push('\n');
            }
        }
        if let Some(id) = current {
            if let Ok(row) = serde_json::from_str::<Value>(&json) {
                models.push(catalog_row(&id, &row, None, &agents));
            }
        }
    }
    models.sort_by_key(|row| row["name"].as_str().unwrap_or("").to_lowercase());
    Ok(json!({"availableModels":models}))
}
fn catalog_row(id: &str, row: &Value, provider: Option<&str>, agents: &[Value]) -> Value {
    let mut variants: Vec<Value> = if let Some(rows) = row["variants"].as_array() {
        rows.iter()
            .filter_map(|row| row["id"].as_str().map(|id| json!({"value":id,"name":id})))
            .collect()
    } else {
        row["variants"]
            .as_object()
            .into_iter()
            .flat_map(|rows| rows.keys())
            .map(|id| json!({"value":id,"name":id}))
            .collect()
    };
    let order = [
        "none",
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "extra-high",
        "max",
        "ultra",
    ];
    variants.sort_by_key(|row| {
        let value = row["value"].as_str().unwrap_or("").to_lowercase();
        (
            order
                .iter()
                .position(|rank| *rank == value)
                .unwrap_or(usize::MAX),
            value,
        )
    });
    let mut config = Vec::new();
    let provider_id = id.split('/').next().unwrap_or("");
    if !variants.is_empty() {
        let preferred = if provider_id == "anthropic" || provider_id.starts_with("google") {
            ["high", ""]
        } else {
            ["medium", "high"]
        };
        let selected = preferred
            .iter()
            .find_map(|preferred| variants.iter().find(|row| row["value"] == *preferred))
            .unwrap_or(&variants[0])["value"]
            .clone();
        config.push(json!({"id":"variant","name":"Variant","type":"select","currentValue":selected,"options":variants}));
    }
    if !agents.is_empty() {
        let selected = agents
            .iter()
            .find(|row| row["value"] == "build")
            .unwrap_or(&agents[0])["value"]
            .clone();
        config.push(json!({"id":"agent","name":"Agent","type":"select","currentValue":selected,"options":agents}));
    }
    let name = row["name"].as_str().unwrap_or(id);
    let name = provider
        .map(|provider| format!("{name} · {provider}"))
        .unwrap_or_else(|| name.into());
    json!({"modelId":id,"name":name,"configOptions":config})
}
pub(super) fn rewind(
    request: &Request,
    _before_turn: Option<&str>,
) -> Result<String, rewind::RewindError> {
    let cancel = AtomicBool::new(false);
    let session = request
        .provider_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("The conversation has no saved provider session to rewind")?;
    // Retire every warm launch configuration for this binding before changing
    // durable history, including a process parked with an older model.
    forget_session(session, &request.settings.account.id, &request.cwd);
    let v2 = generation(request, &cancel)?.v2;
    let (client, server) = connect(request, &cancel, v2)?;
    let messages = client.request(
        "GET",
        &client.path(&format!("/session/{}/message", client::encode(session))),
        None,
    )?;
    let mut candidates: Vec<_> = messages
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let info = row.get("info").unwrap_or(row);
            if info["role"] == "user" || info["type"] == "user" {
                info["id"]
                    .as_str()
                    .map(|id| (id.to_owned(), info["time"]["created"].as_f64()))
            } else {
                None
            }
        })
        .collect();
    if candidates.iter().all(|(_, time)| time.is_some()) {
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    }
    let message = candidates
        .last()
        .map(|(id, _)| id)
        .ok_or("OpenCode did not expose the last user message")?;
    let result = client.revert(session, message);
    result.map_err(|message| rewind::RewindError {
        message,
        provider_may_have_changed: true,
    })?;
    // A reverted history must never be resumed by the old warm generation.
    drop(server);
    Ok(session.into())
}

#[cfg(test)]
mod tool_tests {
    use super::*;
    use crate::tool_servers::{Access, Mount, VISUAL};
    use std::io::{BufRead, BufReader, Read, Write};

    /// The stand-in hands each request on from a thread of its own, so it can
    /// land just after the call that made it returns.
    const POSTED: Duration = Duration::from_secs(5);
    /// Long enough for a request that was made to have landed.
    const QUIET: Duration = Duration::from_millis(300);

    /// A `serve` stand-in: answers `POST /mcp` with this status, and counts them.
    fn opencode(status: u16) -> (Client, mpsc::Receiver<serde_json::Value>) {
        opencode_with(status, false, |_, body| body)
    }

    /// The stand-in, keeping each request's first line with its body.
    fn opencode_seen(
        status: u16,
        v2: bool,
    ) -> (Client, mpsc::Receiver<(String, serde_json::Value)>) {
        opencode_with(status, v2, |line, body| (line, body))
    }

    /// The stand-in, running `probe` on each body before it answers.
    fn opencode_probing(
        status: u16,
        v2: bool,
        probe: impl Fn(&serde_json::Value) + Send + 'static,
    ) -> (Client, mpsc::Receiver<(String, serde_json::Value)>) {
        opencode_with(status, v2, move |line, body| {
            probe(&body);
            (line, body)
        })
    }

    /// The stand-in, passing on what `seen` makes of each request before it
    /// answers, so a request is there to see once the call returns.
    fn opencode_with<T: Send + 'static>(
        status: u16,
        v2: bool,
        seen: impl Fn(String, serde_json::Value) -> T + Send + 'static,
    ) -> (Client, mpsc::Receiver<T>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (posted, received) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && !line.trim().is_empty() {
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    line.clear();
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let body = serde_json::from_slice(&body).unwrap_or_default();
                posted.send(seen(first.trim().to_owned(), body)).unwrap();
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        (Client::new(url, "/work".into(), v2, None), received)
    }

    fn mount(session: i64) -> Mount {
        Mount {
            session,
            worktree: "/work".into(),
            toolkits: [VISUAL.to_owned()].into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_server_gets_the_endpoint_once_and_keeps_its_token_when_reused() {
        let (client, posted) = opencode(200);
        let mount = mount(5101);
        let mut held = None;
        mount_tools(&client, &mut held, Some(&mount));
        let server = posted.recv_timeout(POSTED).unwrap();
        assert_eq!(server["name"], "gespan");
        let authorization = server["config"]["headers"]["Authorization"].clone();
        assert!(held.as_ref().unwrap().is_live(5101));
        // The next turn on this kept server.
        mount_tools(&client, &mut held, Some(&mount));
        assert!(
            posted.recv_timeout(QUIET).is_err(),
            "a reused server isn't remounted"
        );
        assert!(
            held.as_ref().unwrap().is_live(5101),
            "nor is its token revoked"
        );
        assert_eq!(authorization, server["config"]["headers"]["Authorization"]);
    }

    #[test]
    fn a_server_whose_token_was_replaced_gets_the_new_one() {
        let (client, posted) = opencode(200);
        let mount = mount(5102);
        let mut held = None;
        mount_tools(&client, &mut held, Some(&mount));
        let first = posted.recv_timeout(POSTED).unwrap();
        // Another process of this session took over, then this server came back.
        let _other = Access::open(&mount).unwrap();
        mount_tools(&client, &mut held, Some(&mount));
        let second = posted.recv_timeout(POSTED).unwrap();
        assert_ne!(first["config"]["headers"], second["config"]["headers"]);
        assert!(held.as_ref().unwrap().is_live(5102));
    }

    #[test]
    fn a_refused_mount_leaves_nothing_held_and_is_tried_again() {
        let (client, posted) = opencode(500);
        let mount = mount(5103);
        let mut held = None;
        mount_tools(&client, &mut held, Some(&mount));
        assert!(posted.recv_timeout(POSTED).is_ok());
        assert!(held.is_none(), "the turn goes on without tools");
        mount_tools(&client, &mut held, Some(&mount));
        assert!(posted.recv_timeout(POSTED).is_ok());
    }

    #[test]
    fn a_turn_with_no_tools_mounts_nothing() {
        let (client, posted) = opencode(200);
        let mut held = None;
        mount_tools(&client, &mut held, None);
        assert!(posted.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn only_a_v2_turn_that_expected_tools_is_told_they_are_missing() {
        let request = |mounted: bool| {
            let settings = crate::agent::Settings {
                mount: mounted.then(|| mount(5104)),
                ..Default::default()
            };
            Request {
                kind: Kind::OpenCode,
                binary: "unused".into(),
                cwd: "/tmp".into(),
                prompt: "Hello".into(),
                provider_id: None,
                model: None,
                settings,
            }
        };
        let told = |expected, mounted| {
            let mut request = request(expected);
            announce_missing_tools(&mut request, mounted);
            request.prompt
        };
        assert!(told(true, false).starts_with(NO_TOOLS));
        assert!(told(true, false).ends_with("Hello"));
        assert_eq!(told(true, true), "Hello");
        assert_eq!(told(false, false), "Hello");
    }

    #[test]
    fn a_v2_session_gets_a_server_of_its_own_that_serves_only_its_conversation() {
        let (client, seen) = opencode_seen(204, true);
        let mount = mount(5105);
        let mounted = mount_tools_v2(&client, Some(&mount), "ses_mine", false).unwrap();
        let (line, body) = seen.try_recv().unwrap();
        assert!(
            line.starts_with("PUT /api/experimental/mcp/gespan_s5105?"),
            "{line}"
        );
        assert_eq!(body["config"]["type"], "remote");
        assert_eq!(body["config"]["oauth"], false);
        let config = mounted._access.opencode_config();
        assert_eq!(body["config"], config);
        let connection = |token: &str| mcp_hub::Connection {
            url: config["url"].as_str().unwrap().to_owned(),
            token: token.trim_start_matches("Bearer ").to_owned(),
        };
        let bearer = config["headers"]["Authorization"].as_str().unwrap();
        // OpenCode names the conversation in `_meta`; a call without it is
        // another conversation's, or the model's own doing.
        let refused = mcp_hub::call_tool(connection(bearer), "visual_show", json!({}));
        assert!(
            refused
                .unwrap_err()
                .contains("belongs to another conversation"),
            "a call from elsewhere is turned away"
        );
    }

    /// A call through the hub with the credential in an OpenCode server
    /// config, from no conversation at all.
    fn call_as_anyone(config: &serde_json::Value) -> Result<serde_json::Value, String> {
        let bearer = config["headers"]["Authorization"].as_str().unwrap_or("");
        let connection = mcp_hub::Connection {
            url: config["url"].as_str().unwrap_or("").to_owned(),
            token: bearer.trim_start_matches("Bearer ").to_owned(),
        };
        mcp_hub::call_tool(connection, "visual_show", json!({}))
    }

    #[test]
    fn a_v2_credential_is_bound_before_the_service_sees_it() {
        let (probed, probes) = mpsc::channel();
        let (client, _seen) = opencode_probing(204, true, move |body| {
            // Called while the PUT is still unanswered.
            let _ = probed.send(call_as_anyone(&body["config"]));
        });
        let access = mount_tools_v2(&client, Some(&mount(5108)), "ses_mine", false).unwrap();
        let refused = probes.try_recv().unwrap().unwrap_err();
        assert!(
            refused.contains("belongs to another conversation"),
            "no other caller is served, even mid-mount: {refused}"
        );
        assert!(access._access.is_live(5108));
    }

    #[test]
    fn a_refused_v2_mount_ends_its_credential_and_removes_the_server() {
        let (client, seen) = opencode_seen(500, true);
        let mount = mount(5109);
        assert!(mount_tools_v2(&client, Some(&mount), "ses_mine", false).is_none());
        let (put, body) = seen.try_recv().unwrap();
        assert!(
            put.starts_with("PUT /api/experimental/mcp/gespan_s5109?"),
            "{put}"
        );
        let (delete, _) = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            delete.starts_with("DELETE /api/experimental/mcp/gespan_s5109?"),
            "{delete}"
        );
        let refused = call_as_anyone(&body["config"]).unwrap_err();
        assert!(refused.contains("session has ended"), "{refused}");
    }

    #[test]
    fn a_refused_v2_mount_leaves_a_newer_credential_alone() {
        let mount = mount(5110);
        let (newer, took) = mpsc::channel();
        let (client_ok, _ok_seen) = opencode_seen(204, true);
        // The session moves on to a credential of its own while this mount
        // is still waiting on the service.
        let (slow, _slow_seen) = opencode_probing(500, true, {
            let mount = mount.clone();
            move |body| {
                // On the PUT only, not the DELETE that follows its refusal.
                if body["config"].is_null() {
                    return;
                }
                let _ = newer.send(mount_tools_v2(&client_ok, Some(&mount), "ses_mine", false));
            }
        });
        assert!(mount_tools_v2(&slow, Some(&mount), "ses_mine", false).is_none());
        let newer = took.try_recv().unwrap().unwrap();
        assert!(
            newer._access.is_live(5110),
            "only the refused mount's own credential ends"
        );
    }

    #[test]
    fn each_v2_turn_mounts_again_with_a_fresh_credential() {
        let (client, seen) = opencode_seen(204, true);
        let mount = mount(5106);
        let first = mount_tools_v2(&client, Some(&mount), "ses_a", false).unwrap();
        let second = mount_tools_v2(&client, Some(&mount), "ses_a", false).unwrap();
        assert_eq!(
            seen.try_iter().count(),
            2,
            "the service may have dropped it"
        );
        assert!(!first._access.is_live(5106) && second._access.is_live(5106));
    }

    #[test]
    fn a_v2_service_that_refuses_the_server_leaves_the_turn_without_tools() {
        let (client, _seen) = opencode_seen(500, true);
        assert!(mount_tools_v2(&client, Some(&mount(5107)), "ses_a", false).is_none());
        let (client, seen) = opencode_seen(204, true);
        assert!(mount_tools_v2(&client, None, "ses_a", true).is_none());
        assert!(
            seen.try_recv().is_err(),
            "a turn with no tools mounts nothing"
        );
    }

    fn review_request(results: crate::review_policy::ReviewResults, access: AccessMode) -> Request {
        let mut request = Request {
            kind: Kind::OpenCode,
            binary: "unused".into(),
            cwd: "/tmp".into(),
            prompt: String::new(),
            provider_id: None,
            model: None,
            settings: crate::agent::Settings {
                access,
                ..Default::default()
            },
        };
        request.settings.review = Some(crate::review_policy::ReviewPolicy {
            results,
            target: Some(crate::review_policy::ReviewTarget {
                forge: crate::review_policy::Forge::GitHub,
                repo: "acme/app".into(),
                number: 7,
            }),
        });
        request
    }

    /// A bash permission as opencode 1.19 sends it.
    fn bash(command: &str) -> serde_json::Value {
        serde_json::json!({"permission":"bash","patterns":[command],"always":[],"metadata":{"command":command},"tool":{"messageID":"m","callID":"c"}})
    }

    fn review_state(
        results: crate::review_policy::ReviewResults,
        access: AccessMode,
    ) -> events::State {
        events::State::new(review_request(results, access), "ses_a".into(), false)
    }

    /// Feeds one permission to a review run; returns the events and the reply sent.
    fn review_permission(
        state: &mut events::State,
        permission: serde_json::Value,
        interactive: bool,
    ) -> (Vec<Event>, Option<String>) {
        let (client, seen) = opencode_seen(204, false);
        let mut permission = permission;
        permission["id"] = "per_1".into();
        let events = state
            .frame(
                &client,
                serde_json::json!({"type":"permission.asked","properties":permission}),
                interactive,
            )
            .unwrap();
        let reply = seen
            .recv_timeout(QUIET)
            .ok()
            .map(|(line, body)| format!("{line} {}", body["reply"]));
        (events, reply)
    }

    #[test]
    fn opencode_review_policy_decides_bash() {
        use crate::review_policy::ReviewResults;
        let post = "gh pr review 7 --repo acme/app --comment --body 'x'";
        for access in [
            AccessMode::FullAccess,
            AccessMode::Supervised,
            AccessMode::Auto,
        ] {
            let mut draft = review_state(ReviewResults::Draft, access);
            let (events, reply) =
                review_permission(&mut draft, bash("gh pr view 7 --repo acme/app"), true);
            assert!(events.is_empty(), "{access:?}");
            assert!(reply.unwrap().contains("\"once\""), "{access:?}");

            let (events, reply) = review_permission(&mut draft, bash(post), true);
            assert!(reply.is_none(), "an approval is asked, nothing replied");
            match &events[..] {
                [Event::Approval(a)] => {
                    assert!(!a.can_remember);
                    assert!(a.detail.contains("gh pr review 7"), "{}", a.detail);
                }
                other => panic!("{other:?}"),
            }

            let mut posting = review_state(ReviewResults::Post, access);
            let (events, reply) = review_permission(&mut posting, bash(post), true);
            assert!(events.is_empty());
            assert!(reply.unwrap().contains("\"once\""));

            let (events, reply) =
                review_permission(&mut posting, bash("gh pr merge 7 --repo acme/app"), true);
            assert!(events.is_empty());
            assert!(reply.unwrap().contains("\"reject\""));
        }
    }

    #[test]
    fn opencode_review_policy_rejects_what_it_cannot_judge() {
        use crate::review_policy::ReviewResults;
        use serde_json::json;
        let post = "gh pr review 7 --repo acme/app --comment --body 'x'";
        let rejected = |permission: serde_json::Value, interactive: bool| {
            let mut state = review_state(ReviewResults::Post, AccessMode::FullAccess);
            let (events, reply) = review_permission(&mut state, permission, interactive);
            assert!(
                events.is_empty() && state.approvals.is_empty(),
                "{events:?}"
            );
            assert!(reply.unwrap().contains("\"reject\""));
        };
        for kind in [
            "edit",
            "write",
            "patch",
            "webfetch",
            "websearch",
            "task",
            "other",
        ] {
            rejected(json!({"permission":kind,"pattern":"*"}), true);
        }
        // No command, or one that changes what runs beside it.
        rejected(json!({"permission":"bash"}), true);
        rejected(
            json!({"permission":"bash","metadata":{"command":"  "}}),
            true,
        );
        // patterns alone are never judged.
        rejected(json!({"permission":"bash","patterns":[post]}), true);
        rejected(json!({"permission":"bash","pattern":post}), true);
        rejected(
            json!({"permission":"bash","metadata":{"command":["gh","pr","view"]}}),
            true,
        );
        let mut env = bash(post);
        env["env"] = json!({"GH_HOST":"evil"});
        rejected(env, true);
        let mut args = bash(post);
        args["metadata"]["args"] = json!(["--x"]);
        rejected(args, true);
        // Harmless-looking patterns over a command that does more.
        let mut redirected = bash("gh pr view 7 --repo acme/app > x");
        redirected["patterns"] = json!(["gh pr view 7 --repo acme/app"]);
        let mut state = review_state(ReviewResults::Post, AccessMode::FullAccess);
        let (events, reply) = review_permission(&mut state, redirected, true);
        assert!(events.is_empty());
        assert!(reply.unwrap().contains("\"reject\""));
        let mut unrelated = bash(post);
        unrelated["patterns"] = json!(["gh pr view 7", "rm -rf /"]);
        rejected(unrelated, true);
        let mut consistent = bash("gh pr view 7 --repo acme/app");
        consistent["always"] = json!(["gh pr view *"]);
        let (_, reply) = review_permission(&mut state, consistent, true);
        assert!(reply.unwrap().contains("\"once\""));
        // Nobody to ask under Draft.
        let mut draft = review_state(ReviewResults::Draft, AccessMode::FullAccess);
        let (events, reply) = review_permission(&mut draft, bash(post), false);
        assert!(events.is_empty());
        assert!(reply.unwrap().contains("\"reject\""));
        // Reads pass.
        let mut state = review_state(ReviewResults::Post, AccessMode::FullAccess);
        for read in ["read", "grep", "glob", "search", "find"] {
            let (_, reply) =
                review_permission(&mut state, json!({"permission":read,"pattern":"*"}), true);
            assert!(reply.unwrap().contains("\"once\""), "{read}");
        }
    }

    #[test]
    fn opencode_review_policy_ignores_the_access_mode_and_never_remembers() {
        use crate::review_policy::ReviewResults;
        let (v2, _) = opencode_seen(204, true);
        let rows = v2.permissions(&review_request(ReviewResults::Post, AccessMode::FullAccess))
            ["permissions"]
            .clone();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows[0]["resource"], "*");
        assert_eq!(rows[0]["effect"], "ask", "{rows:?}");
        assert!(
            !rows
                .iter()
                .any(|r| r["action"] == "edit" && r["effect"] == "allow"),
            "{rows:?}"
        );
        // Approving an asked command replies once, even for a "session" decision.
        let mut draft = review_state(ReviewResults::Draft, AccessMode::FullAccess);
        let post = "gh pr review 7 --repo acme/app --comment --body 'x'";
        let (events, _) = review_permission(&mut draft, bash(post), true);
        let Event::Approval(a) = &events[0] else {
            panic!()
        };
        let (client, seen) = opencode_seen(204, false);
        draft
            .respond(
                &client,
                Control {
                    id: a.id.clone(),
                    decision: Decision::Session,
                },
            )
            .unwrap();
        let (_, body) = seen.recv_timeout(POSTED).unwrap();
        assert_eq!(body["reply"], "once");
    }

    #[test]
    fn a_v2_session_sees_only_its_own_gespan_server() {
        let request = |access, mounted: bool| Request {
            kind: Kind::OpenCode,
            binary: "unused".into(),
            cwd: "/tmp".into(),
            prompt: String::new(),
            provider_id: None,
            model: None,
            settings: crate::agent::Settings {
                access,
                mount: mounted.then(|| mount(12)),
                ..Default::default()
            },
        };
        let (v2, _) = opencode_seen(204, true);
        let rows = v2.permissions(&request(AccessMode::FullAccess, true))["permissions"].clone();
        let rows = rows.as_array().unwrap();
        let tail: Vec<_> = rows[rows.len() - 2..]
            .iter()
            .map(|row| {
                (
                    row["action"].as_str().unwrap(),
                    row["effect"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(tail, [("gespan_*", "deny"), ("gespan_s12_*", "allow")]);
        // The last match wins, so a session that asks before acting asks for its own too.
        let asking = v2.permissions(&request(AccessMode::ReadOnly, true))["permissions"].clone();
        assert_eq!(asking.as_array().unwrap().last().unwrap()["effect"], "ask");
        let bare = v2.permissions(&request(AccessMode::FullAccess, false))["permissions"].clone();
        let last = bare.as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            last["action"], "gespan_*",
            "with no tools, every session's are hidden"
        );
        assert_eq!(last["effect"], "deny");
        let (v1, _) = opencode_seen(204, false);
        let rows = v1
            .permissions(&request(AccessMode::FullAccess, true))
            .to_string();
        assert!(!rows.contains("gespan_"), "v1's server is its own: {rows}");
    }

    #[test]
    fn ending_a_v2_turn_takes_its_server_off_the_shared_service() {
        let (client, seen) = opencode_seen(204, true);
        let mounted = mount_tools_v2(&client, Some(&mount(5108)), "ses_a", false).unwrap();
        let (put, _) = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(put.starts_with("PUT "), "{put}");
        assert!(
            seen.try_recv().is_err(),
            "nothing is removed while the turn runs"
        );
        drop(mounted);
        let (removed, _) = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            removed.starts_with("DELETE /api/experimental/mcp/gespan_s5108?"),
            "{removed}"
        );
    }

    #[test]
    fn a_service_that_wont_remove_the_server_doesnt_hold_up_the_turn() {
        let (client, _seen) = opencode_seen(204, true);
        let mounted = mount_tools_v2(&client, Some(&mount(5109)), "ses_a", false).unwrap();
        // The service has gone by the time the turn ends.
        let RouteV2::Remote { client, path } = &mounted.route else {
            panic!("mounted over the socket");
        };
        let gone = Client::new(
            "http://127.0.0.1:9".into(),
            client.directory.clone(),
            true,
            None,
        );
        let started = Instant::now();
        drop(MountedV2 {
            _access: Access::open(&mount(5109)).unwrap(),
            route: RouteV2::Remote {
                client: gone,
                path: path.clone(),
            },
        });
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn only_opencode_2_2_and_newer_host_the_tools() {
        let of = |output: &str| parse_generation(output).unwrap();
        assert_eq!(
            of("1.14.19"),
            Generation {
                v2: false,
                hosts_tools: false
            }
        );
        assert_eq!(
            of("v2.1.9"),
            Generation {
                v2: true,
                hosts_tools: false
            }
        );
        assert!(of("2.2.0").hosts_tools);
        assert!(of("\u{1b}[1mopencode 2.2.6\u{1b}[0m").hosts_tools);
        assert!(of("2.10.0").hosts_tools);
        assert!(parse_generation("2.0.14").is_err());
        assert!(parse_generation("3.0.0").is_err());
    }

    type Hosting = tungstenite::WebSocket<std::net::TcpStream>;

    /// What the stand-in saw of a request: its first line and its headers.
    struct Head {
        line: String,
        headers: String,
    }

    fn send(socket: &mut Hosting, message: Value) {
        socket
            .send(tungstenite::Message::text(message.to_string()))
            .unwrap();
    }

    /// The next answer to `id`, skipping anything else the server says.
    fn answer(socket: &mut Hosting, id: i64) -> Value {
        loop {
            let message = socket.read().unwrap();
            let Ok(text) = message.to_text() else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if message["id"] == id {
                return message;
            }
        }
    }

    /// An OpenCode 2.2 stand-in. A socket it takes is initialized and its
    /// tools listed the way OpenCode does, then handed over; with `sockets`
    /// false it turns sockets away, and answers anything else with 204.
    fn hosting_opencode(
        sockets: bool,
    ) -> (
        Client,
        mpsc::Receiver<Head>,
        mpsc::Receiver<(Hosting, Value)>,
    ) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (heads, seen) = mpsc::channel();
        let (sockets_out, taken) = mpsc::channel();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut peeked = vec![0; 8192];
                let head = loop {
                    let read = stream.peek(&mut peeked).unwrap();
                    let text = String::from_utf8_lossy(&peeked[..read]).into_owned();
                    if let Some(end) = text.find("\r\n\r\n") {
                        break text[..end].to_owned();
                    }
                };
                let (line, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
                let upgrade = headers.to_lowercase().contains("upgrade: websocket");
                let _ = heads.send(Head {
                    line: line.to_owned(),
                    headers: headers.to_owned(),
                });
                if upgrade && sockets {
                    let mut socket = tungstenite::accept(stream).unwrap();
                    send(
                        &mut socket,
                        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                            "protocolVersion":"2025-11-25","capabilities":{},
                            "clientInfo":{"name":"opencode","version":"2.2.6"}}}),
                    );
                    let initialized = answer(&mut socket, 1);
                    send(
                        &mut socket,
                        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                    );
                    send(
                        &mut socket,
                        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                    );
                    let listed = answer(&mut socket, 2);
                    let _ = sockets_out.send((socket, json!([initialized, listed])));
                    continue;
                }
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && !line.trim().is_empty() {
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    line.clear();
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let status = if upgrade {
                    "404 Not Found"
                } else {
                    "204 No Content"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });
        let client = Client::new(url, "/work tree".into(), true, Some("hunter2".into()));
        (client, seen, taken)
    }

    fn call(socket: &mut Hosting, id: i64, conversation: &str) -> Value {
        send(
            socket,
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
                "name":"visual_docs","arguments":{},
                "_meta":{"ai.opencode/sessionID":conversation}}}),
        );
        answer(socket, id)
    }

    #[test]
    fn on_2_2_the_service_calls_tools_gespan_hosts_and_never_sees_the_token() {
        let (client, heads, taken) = hosting_opencode(true);
        let mounted = mount_tools_v2(&client, Some(&mount(5110)), "ses_mine", true).unwrap();
        assert!(matches!(mounted.route, RouteV2::Hosted { .. }));
        let head = heads.recv_timeout(Duration::from_secs(5)).unwrap();
        let url = url::Url::parse(&format!(
            "http://x{}",
            head.line.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(url.path(), "/api/experimental/client-mcp");
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["name"], "gespan_s5110");
        assert_eq!(query["location[directory]"], "/work tree");
        let auth = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "opencode:hunter2",
        );
        assert!(
            head.headers.contains(&format!("Basic {auth}")),
            "{}",
            head.headers
        );
        let (mut socket, handshake) = taken.recv_timeout(Duration::from_secs(5)).unwrap();
        let tools = handshake[1]["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|tool| tool["name"] == "visual_docs"));

        let own = call(&mut socket, 3, "ses_mine");
        assert!(
            own["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("documentation"),
            "{own}"
        );
        let other = call(&mut socket, 4, "ses_other");
        assert!(
            other
                .to_string()
                .contains("belongs to another conversation"),
            "another conversation on the service is turned away: {other}"
        );
        let secret = mounted._access.connection().token;
        let spoken = format!("{}{}{handshake}{own}{other}", head.line, head.headers);
        assert!(!spoken.contains(&secret), "the token reached the service");
        assert!(heads.try_recv().is_err(), "no endpoint was handed over");
    }

    #[test]
    fn ending_a_hosted_turn_closes_the_socket() {
        let (client, _heads, taken) = hosting_opencode(true);
        let mounted = mount_tools_v2(&client, Some(&mount(5111)), "ses_a", true).unwrap();
        let (mut socket, _) = taken.recv_timeout(Duration::from_secs(5)).unwrap();
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        drop(mounted);
        let closed = loop {
            match socket.read() {
                Ok(tungstenite::Message::Close(frame)) => break frame,
                Ok(_) => {}
                Err(error) => panic!("the socket wasn't closed: {error}"),
            }
        };
        assert_eq!(
            closed.map(|frame| frame.code),
            Some(tungstenite::protocol::frame::coding::CloseCode::Normal)
        );
    }

    #[test]
    fn a_hosted_socket_comes_back_after_the_service_reloads() {
        let (client, heads, taken) = hosting_opencode(true);
        let _mounted = mount_tools_v2(&client, Some(&mount(5112)), "ses_a", true).unwrap();
        let (mut socket, _) = taken.recv_timeout(Duration::from_secs(5)).unwrap();
        // OpenCode closes every client's socket with 1012 when it reloads.
        socket
            .close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Restart,
                reason: "reload".into(),
            }))
            .unwrap();
        let _ = socket.flush();
        let (mut again, handshake) = taken.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(handshake[1]["result"]["tools"].is_array());
        assert_eq!(heads.try_iter().count(), 2);
        assert!(call(&mut again, 3, "ses_a")["result"].is_object());
    }

    #[test]
    fn the_socket_and_its_password_go_only_to_a_plain_loopback_service() {
        let elsewhere = |url: &str| {
            let client = Client::new(url.into(), "/work".into(), true, Some("hunter2".into()));
            client_mcp::open(&client, "gespan_s1").unwrap_err()
        };
        assert!(elsewhere("https://127.0.0.1:4096").contains("plain HTTP"));
        assert!(elsewhere("http://192.0.2.1:4096").contains("this machine"));
    }

    /// The service of the OpenCode installed here, or of `GESPAN_OPENCODE`.
    fn real_v2(folder: &Path) -> (Client, Generation) {
        let request = Request {
            kind: Kind::OpenCode,
            binary: std::env::var_os("GESPAN_OPENCODE")
                .map_or_else(|| "opencode".into(), PathBuf::from),
            cwd: folder.to_owned(),
            prompt: String::new(),
            provider_id: None,
            model: None,
            settings: Default::default(),
        };
        let cancel = AtomicBool::new(false);
        let generation = generation(&request, &cancel).unwrap();
        assert!(generation.v2);
        (connect(&request, &cancel, true).unwrap().0, generation)
    }

    /// No model is called.
    #[test]
    #[ignore = "needs OpenCode 2.2 or newer installed"]
    fn a_real_2_2_service_takes_the_tools_over_the_socket_and_holds_no_token() {
        let folder = tempfile::tempdir().unwrap();
        let (client, generation) = real_v2(folder.path());
        assert!(generation.hosts_tools);
        // Hosting succeeds only once the service has initialized over the
        // socket and listed the tools through it.
        let mounted = mount_tools_v2(&client, Some(&mount(5120)), "ses_real", true).unwrap();
        assert!(matches!(mounted.route, RouteV2::Hosted { .. }));
        // The service lists only servers it was given, each with its headers;
        // one it calls over a client's socket isn't among them.
        let given = client.request("GET", &client.path("/mcp"), None).unwrap();
        assert!(!given.to_string().contains("gespan_"), "{given}");
    }

    /// No model is called.
    #[test]
    #[ignore = "needs OpenCode v2 installed"]
    fn a_real_v2_service_given_the_endpoint_drops_it_when_the_turn_ends() {
        let folder = tempfile::tempdir().unwrap();
        let (client, _) = real_v2(folder.path());
        let given = || {
            client
                .request("GET", &client.path("/mcp"), None)
                .unwrap()
                .to_string()
        };
        let mounted = mount_tools_v2(&client, Some(&mount(5121)), "ses_real", false).unwrap();
        assert!(given().contains("gespan_s5121"), "{}", given());
        drop(mounted);
        assert!(!given().contains("gespan_s5121"), "{}", given());
    }

    #[test]
    fn a_2_2_service_that_wont_take_the_socket_gets_the_endpoint_instead() {
        let (client, heads, _taken) = hosting_opencode(false);
        let mounted = mount_tools_v2(&client, Some(&mount(5113)), "ses_a", true).unwrap();
        assert!(matches!(mounted.route, RouteV2::Remote { .. }));
        let lines: Vec<_> = heads.try_iter().map(|head| head.line).collect();
        assert!(
            lines[0].starts_with("GET /api/experimental/client-mcp?"),
            "{lines:?}"
        );
        assert!(
            lines[1].starts_with("PUT /api/experimental/mcp/gespan_s5113?"),
            "{lines:?}"
        );
    }
}
