//! Bounded, expiring provider processes. Each parked process belongs to one
//! conversation, account, directory and launch configuration.
use super::*;
use crate::diagnostics::metrics;
use std::sync::{atomic::AtomicUsize, OnceLock};
type WriteRequest = (Vec<u8>, mpsc::Sender<std::io::Result<()>>);
pub(super) struct Input {
    sender: Option<mpsc::Sender<WriteRequest>>,
}
impl Write for Input {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let (reply, result) = mpsc::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Provider input closed")
            })?
            .send((bytes.to_vec(), reply))
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Provider input closed")
            })?;
        result
            .recv_timeout(Duration::from_secs(15))
            .map_err(|error| {
                std::io::Error::new(
                    if error == mpsc::RecvTimeoutError::Timeout {
                        std::io::ErrorKind::TimedOut
                    } else {
                        std::io::ErrorKind::BrokenPipe
                    },
                    "Provider stdin write failed or timed out",
                )
            })??;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) struct Wire {
    pub child: std::process::Child,
    pub stdin: Input,
    pub rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    pub tail: Arc<Mutex<Vec<u8>>>,
    /// Kiro's first sign-in failure on stderr; see `acp::kiro_stderr_auth`.
    pub auth: Arc<Mutex<Option<String>>>,
    reader: Option<thread::JoinHandle<()>>,
    stderr_reader: Option<thread::JoinHandle<()>>,
    writer: Option<thread::JoinHandle<()>>,
    _registration: process::Registration,
    _account: crate::accounts::Tracked,
}
impl Wire {
    /// `count_start`: a conversation's start, counted as `agent.start.<id>`;
    /// model-catalog probes are not.
    pub fn spawn(
        command: &mut Command,
        request: &Request,
        count_start: bool,
    ) -> Result<Self, String> {
        let counter = count_start.then(|| format!("agent.start.{}", request.kind.id()));
        let mut child = process::spawn(command).map_err(|e| {
            if let Some(counter) = &counter {
                metrics::failure(counter, metrics::ErrorKind::of_spawn(&e));
            }
            format!(
                "Cannot start {} at {}: {e}",
                request.kind.label(),
                request.binary.display()
            )
        })?;
        if let Some(counter) = &counter {
            metrics::success(counter);
        }
        let registration = process::register(child.id());
        let account = crate::accounts::track(child.id(), request.kind, &request.settings.account);
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        // Tauri's JSON-RPC transport has a 15s write deadline. A wedged child
        // must not leave a worker stuck forever in a full stdin pipe.
        let (writes, requests) = mpsc::channel::<WriteRequest>();
        let writer = thread::spawn(move || {
            while let Ok((bytes, reply)) = requests.recv() {
                let result = stdin.write_all(&bytes).and_then(|_| stdin.flush());
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
        });
        let (tx, rx) = mpsc::sync_channel(16);
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                // Match the Tauri transport: history and tool snapshots can be
                // large, so read each complete JSON line without a size cap.
                match reader.read_until(b'\n', &mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(Ok(bytes)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });
        let tail = Arc::new(Mutex::new(Vec::new()));
        let stderr_tail = tail.clone();
        let auth = Arc::new(Mutex::new(None));
        let stderr_auth = (request.kind == Kind::Kiro).then(|| auth.clone());
        let stderr_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut chunk = [0; 4096];
            let mut line = Vec::new();
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if let Some(auth) = &stderr_auth {
                    for &byte in &chunk[..n] {
                        if byte != b'\n' {
                            // One line never needs more than the whole tail.
                            if line.len() < 8192 {
                                line.push(byte);
                            }
                            continue;
                        }
                        let text = String::from_utf8_lossy(&line);
                        if let Some(detail) = super::acp::kiro_stderr_auth(&text) {
                            auth.lock()
                                .unwrap()
                                .get_or_insert_with(|| detail.to_owned());
                        }
                        line.clear();
                    }
                }
                let mut tail = stderr_tail.lock().unwrap();
                tail.extend_from_slice(&chunk[..n]);
                let excess = tail.len().saturating_sub(8192);
                tail.drain(..excess);
            }
            // The last line may end at exit without a newline.
            if let Some(auth) = &stderr_auth {
                if let Some(detail) = super::acp::kiro_stderr_auth(&String::from_utf8_lossy(&line))
                {
                    auth.lock()
                        .unwrap()
                        .get_or_insert_with(|| detail.to_owned());
                }
            }
        });
        Ok(Self {
            child,
            stdin: Input {
                sender: Some(writes),
            },
            rx,
            tail,
            auth,
            reader: Some(reader),
            stderr_reader: Some(stderr_reader),
            writer: Some(writer),
            _registration: registration,
            _account: account,
        })
    }
    /// Whether stderr reached its end, so nothing more can be learned from it.
    pub fn stderr_closed(&self) -> bool {
        self.stderr_reader
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }
}
impl Drop for Wire {
    fn drop(&mut self) {
        process::terminate(&mut self.child);
        self.stdin.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        // Disconnect before joining so a bounded stdout sender cannot deadlock.
        let (_, empty) = mpsc::channel();
        drop(std::mem::replace(&mut self.rx, empty));
        // The readers end at EOF, but a helper outside the process group
        // (Kiro's engine) can hold the pipes open for tens of seconds after
        // the kill. Wait briefly, then let them finish on their own.
        let deadline = Instant::now() + READER_GRACE;
        for reader in [self.reader.take(), self.stderr_reader.take()]
            .into_iter()
            .flatten()
        {
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}
const READER_GRACE: Duration = Duration::from_millis(250);
pub(super) struct Parked {
    pub wire: Wire,
    pub protocol: Protocol,
    pub key: String,
    at: Instant,
    owner: Option<i64>,
    /// Parked by Hold: kept with the warm pool off, and outside its limit.
    held: bool,
}
static LIMIT: AtomicUsize = AtomicUsize::new(0);
/// How long an idle agent is kept.
const IDLE: Duration = Duration::from_secs(180);
fn pinned_sessions() -> &'static Mutex<std::collections::HashSet<i64>> {
    static PINNED: OnceLock<Mutex<std::collections::HashSet<i64>>> = OnceLock::new();
    PINNED.get_or_init(Default::default)
}
fn pinned(owner: Option<i64>) -> bool {
    owner.is_some_and(|owner| {
        pinned_sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&owner)
    })
}
/// Hold: these sessions' parked agents, and the background work running in
/// them, stay alive however long the hold lasts. Quit and deleting a
/// session still end them.
pub fn pin_idle_agents(ids: impl IntoIterator<Item = i64>) {
    pinned_sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(ids);
}
/// Whether Hold is keeping this session's agent.
pub fn idle_agent_pinned(id: i64) -> bool {
    pinned(Some(id))
}
/// Resume: held agents get the usual idle period from now, so a resumed
/// turn can take its agent back even with the warm pool off.
pub fn unpin_idle_agents() {
    let unpinned =
        std::mem::take(&mut *pinned_sessions().lock().unwrap_or_else(|e| e.into_inner()));
    let now = Instant::now();
    for parked in idle_pool().lock().unwrap().iter_mut() {
        if parked.owner.is_some_and(|owner| unpinned.contains(&owner)) {
            parked.at = now;
        }
    }
}
fn expired(parked: &Parked, limit: usize) -> bool {
    if pinned(parked.owner) {
        return false;
    }
    let idle = parked.at.elapsed() >= IDLE;
    if parked.held {
        idle
    } else {
        limit == 0 || idle
    }
}
fn selected_sessions() -> &'static Mutex<std::collections::HashSet<i64>> {
    static SELECTED: OnceLock<Mutex<std::collections::HashSet<i64>>> = OnceLock::new();
    SELECTED.get_or_init(Default::default)
}
/// Snapshot the conversations currently shown, including split panes.
pub fn keep(ids: Vec<i64>) {
    let next = ids.into_iter().collect();
    let mut selected = selected_sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if *selected == next {
        return;
    }
    *selected = next;
    drop(selected);
    // Selection runs on the UI thread; child reaping belongs to a worker.
    thread::spawn(enforce_idle_limit);
}
fn idle_pool() -> &'static Mutex<Vec<Parked>> {
    static POOL: OnceLock<Mutex<Vec<Parked>>> = OnceLock::new();
    POOL.get_or_init(Default::default)
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Binding {
    kind: Kind,
    session: Option<String>,
    account: String,
    cwd: PathBuf,
}
impl Binding {
    fn request(request: &Request) -> Self {
        Self {
            kind: request.kind,
            session: request.provider_id.clone(),
            account: request.settings.account.id.clone(),
            cwd: request.cwd.clone(),
        }
    }
}
fn epochs() -> &'static Mutex<HashMap<Binding, u64>> {
    static EPOCHS: OnceLock<Mutex<HashMap<Binding, u64>>> = OnceLock::new();
    EPOCHS.get_or_init(Default::default)
}
pub fn configure(limit: usize) {
    LIMIT.store(limit.min(5), Ordering::Relaxed);
    thread::spawn(enforce_idle_limit);
}

/// The desktop registry applies one idle-child budget across every provider.
/// Serialize eviction decisions, but never hold either pool while reaping.
pub(super) fn enforce_idle_limit() {
    static EVICT: OnceLock<Mutex<()>> = OnceLock::new();
    let _evict = EVICT.get_or_init(Default::default).lock().unwrap();
    // Held and pinned agents expire on their own clock and don't take
    // warm-pool slots, so no other agent can evict them.
    let mut entries: Vec<_> = idle_pool()
        .lock()
        .unwrap()
        .iter()
        .filter(|entry| !entry.held && !pinned(entry.owner))
        .map(|entry| (entry.wire.child.id(), entry.at, entry.owner))
        .collect();
    entries.extend(opencode::idle_entries());
    entries.sort_by_key(|(_, at, _)| *at);
    let limit = LIMIT.load(Ordering::Relaxed);
    let excess = entries.len().saturating_sub(limit);
    let selected = selected_sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let retire: std::collections::HashSet<_> = entries
        .into_iter()
        .filter(|(_, _, owner)| limit == 0 || owner.is_none_or(|owner| !selected.contains(&owner)))
        .take(excess)
        .map(|(pid, _, _)| pid)
        .collect();
    if retire.is_empty() {
        return;
    }
    let retired = {
        let mut pool = idle_pool().lock().unwrap();
        let mut retired = Vec::new();
        let mut index = 0;
        while index < pool.len() {
            if retire.contains(&pool[index].wire.child.id()) {
                retired.push(pool.remove(index));
            } else {
                index += 1;
            }
        }
        retired
    };
    reap_in_background(retired);
    opencode::retire_idle(&retire);
}
pub(super) fn key(request: &Request) -> String {
    let mut settings = request.settings.clone();
    // Hermes and Kiro take only the model, so the session options they report
    // back (Kiro's autopilot or memory switches) never change their launch.
    if matches!(request.kind, Kind::Hermes | Kind::Kiro) {
        settings.model_settings.clear();
    }
    settings.turns.clear();
    settings.attachments.clear();
    settings.revision = 0;
    settings.handoff = None;
    settings.pending_provider_switch = None;
    sorted(json!([
        process::owner(),
        request.kind.id(),
        request.binary,
        request.cwd,
        request.provider_id,
        request.model,
        settings,
        request.settings.environment,
        request
            .settings
            .pilot
            .as_ref()
            .map(|pilot| (&pilot.root, &pilot.pilot.id)),
        request.settings.disable_claude_hooks
    ]))
    .to_string()
}
/// The app builds serde_json with `preserve_order` (through GPUI), so maps keep
/// the iteration order of the `HashMap`s they came from, which differs between
/// otherwise equal settings. Keys must not depend on it.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}
pub(super) fn take(key: &str) -> Option<Parked> {
    let limit = LIMIT.load(Ordering::Relaxed);
    let mut pool = idle_pool().lock().unwrap();
    let ix = pool
        .iter()
        .position(|p| p.key == key && (limit > 0 || p.held))?;
    let mut parked = pool.remove(ix);
    drop(pool);
    if expired(&parked, limit) || parked.wire.child.try_wait().ok().flatten().is_some() {
        reap_in_background(vec![parked]);
        return None;
    }
    process::mark_idle(parked.wire.child.id(), false);
    Some(parked)
}
pub(super) fn put(wire: Wire, protocol: Protocol) {
    park(wire, protocol, false);
}
/// A turn interrupted by Hold: its agent may still be running background
/// work, so it is kept even with the warm pool off.
pub(super) fn put_held(wire: Wire, protocol: Protocol) {
    park(wire, protocol, true);
}
fn park(wire: Wire, mut protocol: Protocol, held: bool) {
    if (LIMIT.load(Ordering::Relaxed) == 0 && !held) || protocol.request.provider_id.is_none() {
        return;
    }
    let key = key(&protocol.request);
    protocol.emitted.clear();
    protocol.tools.clear();
    protocol.active_compaction = None;
    protocol.claude_tools.clear();
    protocol.request.prompt.clear();
    protocol.request.settings.attachments.clear();
    protocol.request.settings.turns.clear();
    process::mark_idle(wire.child.id(), true);
    let mut pool = idle_pool().lock().unwrap();
    pool.push(Parked {
        wire,
        protocol,
        key,
        at: Instant::now(),
        owner: process::owner(),
        held,
    });
    drop(pool);
    enforce_idle_limit();
    static WATCH: OnceLock<()> = OnceLock::new();
    WATCH.get_or_init(|| {
        thread::spawn(|| loop {
            thread::sleep(Duration::from_secs(1));
            sweep();
            enforce_idle_limit();
        });
    });
}
/// Ends idle agents that expired or exited.
fn sweep() {
    let limit = LIMIT.load(Ordering::Relaxed);
    let retired: Vec<_> = {
        let mut pool = idle_pool().lock().unwrap();
        let mut retired = Vec::new();
        let mut ix = 0;
        while ix < pool.len() {
            if expired(&pool[ix], limit) || pool[ix].wire.child.try_wait().ok().flatten().is_some()
            {
                retired.push(pool.remove(ix));
            } else {
                ix += 1;
            }
        }
        retired
    };
    drop(retired);
}

/// Stop settles after the provider interrupt is written/acknowledged. Keep the
/// old pipe muted until its terminal frame before making it reusable; otherwise
/// that frame could complete a new user prompt on the same process.
pub(super) fn settle_cancelled(mut wire: Wire, mut protocol: Protocol) {
    if LIMIT.load(Ordering::Relaxed) == 0
        || (protocol.request.kind == Kind::Codex && protocol.active_turn.is_none())
        || (protocol.request.kind == Kind::Claude && !protocol.claude_prompt_sent)
    {
        return;
    }
    let owner = process::owner();
    let binding = Binding::request(&protocol.request);
    let epoch = *epochs().lock().unwrap().entry(binding.clone()).or_default();
    process::mark_idle(wire.child.id(), true);
    thread::spawn(move || {
        let settle = move || {
            let started = Instant::now();
            // This is the existing idle-child retention period after Stop,
            // not a deadline on an active provider conversation.
            while started.elapsed() < Duration::from_secs(180)
                && LIMIT.load(Ordering::Relaxed) > 0
                && epochs()
                    .lock()
                    .unwrap()
                    .get(&binding)
                    .copied()
                    .unwrap_or_default()
                    == epoch
            {
                match wire.rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(bytes)) => {
                        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                            continue;
                        };
                        let kind = protocol.request.kind;
                        let turn = value["params"]["turnId"]
                            .as_str()
                            .or_else(|| value["params"]["turn"]["id"].as_str());
                        let thread = value["params"]["threadId"].as_str();
                        let codex_terminal = kind == Kind::Codex
                            && matches!(
                                value["method"].as_str(),
                                Some("turn/completed" | "turn/aborted")
                            )
                            && turn.is_none_or(|id| Some(id) == protocol.active_turn.as_deref())
                            && thread.is_none_or(|id| {
                                Some(id) == protocol.request.provider_id.as_deref()
                            });
                        let claude_terminal = kind == Kind::Claude && value["type"] == "result";
                        if codex_terminal || claude_terminal {
                            // Discard already-buffered trailing frames too.
                            while wire.rx.try_recv().is_ok() {}
                            protocol.done = true;
                            park_cancelled(wire, protocol, &binding, epoch);
                            return;
                        }
                        if kind == Kind::Codex
                            && matches!(
                                value["method"].as_str(),
                                Some("turn/completed" | "turn/aborted")
                            )
                        {
                            continue;
                        }
                        // A stopped tool may still ask for input while it
                        // unwinds. Resolve it without resurfacing a UI card.
                        if let Ok(events) = protocol.frame(value, &mut wire.stdin) {
                            for event in events {
                                let id = match event {
                                    Event::Approval(approval) => Some(approval.id),
                                    Event::Question(question) => Some(question.id),
                                    _ => None,
                                };
                                if let Some(id) = id {
                                    let _ = protocol.respond(
                                        Control {
                                            id,
                                            decision: Decision::Deny,
                                        },
                                        &mut wire.stdin,
                                    );
                                }
                            }
                        }
                        if protocol.request.kind.is_pi() {
                            let _ = protocol.pi_tick(&mut wire.stdin);
                        }
                        if protocol.done {
                            while wire.rx.try_recv().is_ok() {}
                            park_cancelled(wire, protocol, &binding, epoch);
                            return;
                        }
                    }
                    Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !matches!(wire.child.try_wait(), Ok(None)) {
                            return;
                        }
                        if protocol.request.kind.is_pi() {
                            let _ = protocol.pi_tick(&mut wire.stdin);
                            if protocol.done {
                                park_cancelled(wire, protocol, &binding, epoch);
                                return;
                            }
                        }
                    }
                }
            }
        };
        if let Some(owner) = owner {
            process::owned_by(owner, settle)
        } else {
            settle()
        }
    });
}

fn park_cancelled(wire: Wire, protocol: Protocol, binding: &Binding, epoch: u64) {
    // Hold the invalidation lock through insertion: a cold replacement or a
    // forgotten binding must not race an old retirement back into the pool.
    let epochs = epochs().lock().unwrap();
    if epochs.get(binding).copied().unwrap_or_default() == epoch {
        put(wire, protocol);
    }
}

pub fn discard_worktree(path: &Path) {
    let root = path.to_string_lossy();
    let mut pool = idle_pool().lock().unwrap();
    let (removed, kept) = std::mem::take(&mut *pool)
        .into_iter()
        .partition(|p: &Parked| {
            crate::files::is_equal_or_inside(&p.protocol.request.cwd.to_string_lossy(), &root)
        });
    *pool = kept;
    drop(pool);
    reap_in_background(removed);
    for (binding, epoch) in epochs().lock().unwrap().iter_mut() {
        if crate::files::is_equal_or_inside(&binding.cwd.to_string_lossy(), &root) {
            *epoch += 1;
        }
    }
}

pub(super) fn forget_session(kind: Kind, session: &str, account: &str, cwd: &Path) {
    let binding = Binding {
        kind,
        session: Some(session.into()),
        account: account.into(),
        cwd: cwd.into(),
    };
    *epochs().lock().unwrap().entry(binding.clone()).or_default() += 1;
    let retired = {
        let mut pool = idle_pool().lock().unwrap();
        let mut retired = Vec::new();
        let mut ix = 0;
        while ix < pool.len() {
            if Binding::request(&pool[ix].protocol.request) == binding {
                retired.push(pool.remove(ix));
            } else {
                ix += 1;
            }
        }
        retired
    };
    if !retired.is_empty() {
        thread::spawn(move || drop(retired));
    }
}

/// A rewind changes provider context independently of the next launch options.
/// Evict every warm configuration for that binding, including an older model
/// or permission mode which the exact launch-key lookup would miss.
pub(super) fn discard_session(request: &Request) {
    *epochs()
        .lock()
        .unwrap()
        .entry(Binding::request(request))
        .or_default() += 1;
    let mut pool = idle_pool().lock().unwrap();
    let (discarded, kept) = std::mem::take(&mut *pool).into_iter().partition(|parked| {
        let old = &parked.protocol.request;
        old.kind == request.kind
            && old.provider_id == request.provider_id
            && old.cwd == request.cwd
            && old.settings.account.id == request.settings.account.id
    });
    *pool = kept;
    drop(pool);
    reap_in_background(discarded);
}
/// Ends agents on a worker. Dropping a `Wire` waits for its output pipes to
/// close, and helpers outside the process group (Kiro's engine) can hold them
/// open for tens of seconds after the kill; a new turn must not wait for that.
fn reap_in_background(parked: Vec<Parked>) {
    if !parked.is_empty() {
        thread::spawn(move || drop(parked));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn start_request(kind: Kind, binary: &str) -> Request {
        Request {
            kind,
            binary: PathBuf::from(binary),
            cwd: std::env::temp_dir(),
            prompt: String::new(),
            provider_id: None,
            model: None,
            settings: Settings::default(),
        }
    }

    #[test]
    fn agent_starts_are_counted_by_provider() {
        use crate::diagnostics::metrics::{capture, ErrorKind};
        let missing = start_request(Kind::Claude, "/nonexistent/gespan-test/claude");
        let (wire, counts) =
            capture(|| Wire::spawn(&mut Command::new(&missing.binary), &missing, true));
        assert!(wire.is_err());
        let got: Vec<_> = counts.into_iter().map(|c| (c.name, c.kind)).collect();
        assert_eq!(
            got,
            [("agent.start.claude".into(), Some(ErrorKind::NotFound))]
        );

        let started = start_request(Kind::Codex, "/bin/cat");
        let mut command = Command::new(&started.binary);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let (wire, counts) = capture(|| Wire::spawn(&mut command, &started, true));
        drop(wire.unwrap());
        let got: Vec<_> = counts.into_iter().map(|c| (c.name, c.kind)).collect();
        assert_eq!(got, [("agent.start.codex".into(), None)]);
    }

    #[test]
    fn parked_keys_separate_pilot_identity_and_private_storage_from_regular_sessions() {
        let mut request = Request {
            kind: Kind::Codex,
            binary: PathBuf::from("codex"),
            cwd: PathBuf::from("/project"),
            prompt: "hello".into(),
            provider_id: Some("saved-thread".into()),
            model: None,
            settings: Settings::default(),
        };
        let regular = key(&request);
        request.settings.pilot = Some(crate::agent::PilotContext {
            pilot: crate::pilot::Pilot {
                id: "first-pilot".into(),
                ..Default::default()
            },
            root: PathBuf::from("/desktop/pilots"),
            executable: PathBuf::from("gespan"),
        });
        let first = key(&request);
        assert_ne!(regular, first);
        request.settings.pilot.as_mut().unwrap().pilot.id = "other-pilot".into();
        assert_ne!(first, key(&request));
        request.settings.pilot.as_mut().unwrap().pilot.id = "first-pilot".into();
        request.settings.pilot.as_mut().unwrap().root = PathBuf::from("/isolated/pilots");
        assert_ne!(first, key(&request));
    }

    #[test]
    fn equal_settings_give_equal_keys_whatever_their_map_order() {
        // The app serializes maps in `HashMap` order (GPUI enables serde_json's
        // `preserve_order`); two equal turns must still find the same warm
        // agent. Core tests build without that feature, where maps are sorted.
        let names: Vec<String> = (0..12).map(|i| format!("GESPAN_VAR_{i}")).collect();
        let request = |names: &mut dyn Iterator<Item = &String>| {
            let mut request = start_request(Kind::Codex, "codex");
            request.provider_id = Some("thread".into());
            for name in names {
                request
                    .settings
                    .environment
                    .insert(name.clone(), format!("{name}-value"));
                request
                    .settings
                    .model_settings
                    .insert(name.clone(), "on".into());
            }
            request
        };
        let first = key(&request(&mut names.iter()));
        for _ in 0..20 {
            assert_eq!(key(&request(&mut names.iter().rev())), first);
        }
        // A real difference still separates them.
        let mut changed = request(&mut names.iter());
        changed
            .settings
            .environment
            .insert("GESPAN_VAR_0".into(), "other".into());
        assert_ne!(key(&changed), first);
    }

    #[test]
    fn reuses_only_the_same_conversation_and_launch_configuration() {
        const CHILD: &str = "GESPAN_PARKED_REUSE_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // The idle limit and pool are process-wide. Other provider tests
            // must not park competing children while this test checks reuse.
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "provider::parked::tests::reuses_only_the_same_conversation_and_launch_configuration",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("provider");
        let trace = dir.path().join("starts");
        std::fs::write(
            &binary,
            format!(
                r#"#!/usr/bin/env python3
import json,os,sys
with open({:?},'a') as f: f.write(str(os.getpid())+'\n')
def send(v): print(json.dumps(v),flush=True)
for line in sys.stdin:
 v=json.loads(line)
 if v.get('id')==1: send({{'id':1,'result':{{}}}})
 elif v.get('id')==2: send({{'id':2,'result':{{'thread':{{'id':'settings-thread'}}}}}})
 elif v.get('id')==3:
  send({{'method':'item/agentMessage/delta','params':{{'itemId':'answer','delta':'ok'}}}})
  send({{'method':'turn/completed','params':{{'turn':{{'status':'completed'}}}}}})
"#,
                trace.to_str().unwrap()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        configure(1);
        let mut request = Request {
            kind: Kind::Codex,
            binary,
            cwd: dir.path().into(),
            prompt: "hello".into(),
            provider_id: None,
            model: None,
            settings: Settings::default(),
        };
        run(
            request.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .unwrap();
        request.provider_id = Some("settings-thread".into());
        run(
            request.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&trace).unwrap().lines().count(), 1);
        request.model = Some("different-model".into());
        run(
            request.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&trace).unwrap().lines().count(), 2);
        let original = key(&request);
        request.settings.account.id = "isolated-account".into();
        assert_ne!(original, key(&request));
        request.settings.account = Default::default();
        request.cwd = dir.path().join("other");
        assert_ne!(original, key(&request));
        configure(0);
        idle_pool().lock().unwrap().clear();
    }

    #[test]
    fn kiro_follow_ups_stay_warm_and_replacements_do_not_wait_for_its_helpers() {
        const CHILD: &str = "GESPAN_PARKED_KIRO_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // The idle limit and pool are process-wide; see the reuse test.
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "provider::parked::tests::kiro_follow_ups_stay_warm_and_replacements_do_not_wait_for_its_helpers",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("kiro-cli");
        let trace = dir.path().join("starts");
        let helpers = dir.path().join("helpers");
        // Like Kiro's engine, a helper in its own process group keeps the
        // agent's output pipes open after the agent itself is killed.
        std::fs::write(
            &binary,
            format!(
                r#"#!/usr/bin/env python3
import json,os,subprocess,sys
with open({:?},'a') as f: f.write(str(os.getpid())+'\n')
helper=subprocess.Popen(['sleep','20'],start_new_session=True)
with open({:?},'a') as f: f.write(str(helper.pid)+'\n')
def send(v): print(json.dumps(v),flush=True)
for line in sys.stdin:
 v=json.loads(line)
 m,i=v.get('method'),v.get('id')
 if m=='initialize': send({{'id':i,'result':{{'protocolVersion':1}}}})
 elif m=='session/new': send({{'id':i,'result':{{'sessionId':'kiro-session'}}}})
 elif m=='session/prompt':
  send({{'method':'session/update','params':{{'sessionId':'kiro-session','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':'ok'}}}}}}}})
  send({{'id':i,'result':{{'stopReason':'end_turn'}}}})
 elif i is not None: send({{'id':i,'result':{{}}}})
"#,
                trace.to_str().unwrap(),
                helpers.to_str().unwrap()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        configure(1);
        let mut request = Request {
            kind: Kind::Kiro,
            binary,
            cwd: dir.path().into(),
            prompt: "hello".into(),
            provider_id: None,
            model: None,
            settings: Settings::default(),
        };
        let turn = |request: &Request| {
            let started = Instant::now();
            run(
                request.clone(),
                Arc::new(AtomicBool::new(false)),
                |_| Ok(()),
            )
            .unwrap();
            started.elapsed()
        };
        turn(&request);
        // The app saves the session options Kiro reports (autopilot, memory)
        // with the conversation; they must not cost the warm agent.
        request.provider_id = Some("kiro-session".into());
        request
            .settings
            .model_settings
            .insert("autopilot".into(), "on".into());
        turn(&request);
        assert_eq!(std::fs::read_to_string(&trace).unwrap().lines().count(), 1);
        // A new model needs a new agent, which must not wait for the old
        // agent's helper to release its pipes.
        request.model = Some("claude-haiku-4.5".into());
        let elapsed = turn(&request);
        assert_eq!(std::fs::read_to_string(&trace).unwrap().lines().count(), 2);
        assert!(
            elapsed < Duration::from_secs(10),
            "replacement waited {elapsed:?}"
        );
        // Runs that are never parked (model probes, background text) end their
        // agent at once; that must not wait for the helper either.
        let mut text = request.clone();
        text.provider_id = None;
        text.settings.text_only = true;
        let elapsed = turn(&text);
        assert_eq!(std::fs::read_to_string(&trace).unwrap().lines().count(), 3);
        assert!(
            elapsed < Duration::from_secs(10),
            "unparked run waited {elapsed:?}"
        );
        for pid in std::fs::read_to_string(&helpers).unwrap().lines() {
            unsafe { libc::kill(pid.parse().unwrap(), libc::SIGKILL) };
        }
        configure(0);
        idle_pool().lock().unwrap().clear();
    }

    #[test]
    fn held_agents_stay_parked_while_pinned_and_get_the_usual_idle_period_after() {
        const CHILD: &str = "GESPAN_PARKED_HOLD_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // The idle limit and pool are process-wide; run alone.
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "provider::parked::tests::held_agents_stay_parked_while_pinned_and_get_the_usual_idle_period_after",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        fn alive(pid: u32) -> bool {
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
        fn spawn(session: &str) -> (Wire, Protocol, u32) {
            let mut request = start_request(Kind::Codex, "/bin/cat");
            request.provider_id = Some(session.into());
            let mut command = Command::new(&request.binary);
            command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            process::isolate(&mut command);
            let wire = Wire::spawn(&mut command, &request, false).unwrap();
            let pid = wire.child.id();
            (wire, Protocol::new(request), pid)
        }
        fn backdate(pid: u32, by: Duration) {
            let mut pool = idle_pool().lock().unwrap();
            let entry = pool.iter_mut().find(|p| p.wire.child.id() == pid).unwrap();
            entry.at = Instant::now() - by;
        }
        fn parked(pid: u32) -> bool {
            idle_pool()
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.wire.child.id() == pid)
        }
        // The warm pool is off: an unheld agent is not kept.
        configure(0);
        let (wire, protocol, plain) = spawn("plain");
        process::owned_by(41, || put(wire, protocol));
        assert!(!parked(plain));

        pin_idle_agents([42]);
        let (wire, protocol, held) = spawn("held");
        // The key names its session, as in a real launch.
        let key = process::owned_by(42, || key(&protocol.request));
        process::owned_by(42, || put_held(wire, protocol));
        assert!(parked(held) && alive(held), "Hold keeps its agent");
        // However long the hold lasts.
        backdate(held, Duration::from_secs(3600));
        sweep();
        enforce_idle_limit();
        assert!(parked(held) && alive(held));

        // Resume: the usual idle period starts now, even with the pool off.
        unpin_idle_agents();
        sweep();
        enforce_idle_limit();
        assert!(parked(held), "a resumed turn can still take it back");
        let taken =
            process::owned_by(42, || take(&key)).expect("the resumed turn reuses the held agent");
        assert_eq!(taken.wire.child.id(), held);
        process::owned_by(42, || put_held(taken.wire, taken.protocol));
        backdate(held, IDLE);
        sweep();
        assert!(!parked(held), "then it expires like any idle agent");

        // With the warm pool on, an agent already idle when Hold pinned it
        // is not evicted to make room for others.
        configure(1);
        let (wire, protocol, idle) = spawn("idle-before-hold");
        process::owned_by(44, || put(wire, protocol));
        pin_idle_agents([44]);
        let (wire, protocol, later) = spawn("finished-during-hold");
        process::owned_by(45, || put(wire, protocol));
        assert!(parked(idle) && alive(idle), "the pinned agent stays");
        assert!(parked(later), "the newer one is kept within the limit");
        unpin_idle_agents();
        configure(0);
        idle_pool().lock().unwrap().clear();

        // Deleting a held session still ends its agent.
        pin_idle_agents([43]);
        let (wire, protocol, deleted) = spawn("deleted");
        let request = protocol.request.clone();
        process::owned_by(43, || put_held(wire, protocol));
        assert!(parked(deleted));
        forget_session(
            request.kind,
            "deleted",
            &request.settings.account.id,
            &request.cwd,
        );
        assert!(!parked(deleted));
        unpin_idle_agents();
        idle_pool().lock().unwrap().clear();
    }
}
