//! herdr machines: the bundled herdr binary, the machines herdr has saved,
//! one live connection per enabled machine, and which finished turns the
//! user has seen.
//!
//! Anything here that runs herdr or writes to disk blocks. Call it from a
//! background task, never the UI thread.
use crate::scm::shell::{dirs_home, gui_search_path, which_in_path};
pub use herdr_link::{Agent, AgentStatus, Machine, Online, Snapshot, Workspace};
use herdr_link::{Connection, SshBridge};
use std::{
    collections::HashMap,
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak,
    },
    time::Duration,
};

/// The herdr release Gespan ships and expects on every machine.
pub const HERDR_VERSION: &str = "0.9.5";

/// An explicit herdr binary, used instead of the bundled one.
const ENV_BINARY: &str = "GESPAN_HERDR_BINARY";
#[cfg(windows)]
const HERDR_FILE: &str = "herdr.exe";
#[cfg(not(windows))]
const HERDR_FILE: &str = "herdr";
const SEEN_FILE: &str = "herdr-seen.json";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const NOT_FOUND: &str = "herdr is not installed";

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where the seen-turn store lives. Set once at startup; without it seen
/// turns last only until Gespan quits.
pub fn configure(data_dir: &Path) {
    if DATA_DIR.set(data_dir.to_owned()).is_err() && DATA_DIR.get().is_some_and(|d| d != data_dir) {
        tracing::warn!("herdr machines keep the data folder they started with");
    }
}

pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.get().cloned()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The herdr to run: `GESPAN_HERDR_BINARY`, else the copy bundled next to
/// Gespan, else a herdr on the login PATH that reports [`HERDR_VERSION`].
/// May run `herdr --version`, so it blocks.
pub fn herdr_binary() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_owned));
    find_herdr(
        std::env::var_os(ENV_BINARY),
        exe_dir.as_deref(),
        gui_search_path,
    )
}

/// The override and the bundled copy are trusted as they are; a PATH hit is
/// used only at the pinned version, since a different herdr could disagree
/// with the machines' installs.
fn find_herdr(
    override_path: Option<OsString>,
    exe_dir: Option<&Path>,
    search_path: impl FnOnce() -> String,
) -> Option<PathBuf> {
    if let Some(path) = override_path.filter(|p| !p.is_empty()) {
        return Some(path.into());
    }
    if let Some(bundled) = exe_dir
        .map(|dir| dir.join(HERDR_FILE))
        .filter(|p| p.is_file())
    {
        return Some(bundled);
    }
    let found = which_in_path(&search_path(), "herdr")?;
    let version = crate::cli::output(&found, &["--version".into()], &home(), VERSION_TIMEOUT)
        .unwrap_or_default();
    if version.trim() == format!("herdr {HERDR_VERSION}") {
        Some(found)
    } else {
        tracing::info!(path = %found.display(), version = %version.trim(), "ignoring a herdr of another version");
        None
    }
}

fn home() -> PathBuf {
    dirs_home()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Runs herdr with `args` and returns what it printed.
fn run_herdr(herdr: &Path, args: &[&str]) -> Result<String, String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    crate::cli::output(herdr, &args, &home(), COMMAND_TIMEOUT).map_err(|error| {
        if error.is_empty() {
            "herdr failed".into()
        } else {
            error
        }
    })
}

/// The machines herdr has saved. Blocks; call from a background task.
pub fn list_machines() -> Result<Vec<Machine>, String> {
    list_machines_with(&herdr_binary().ok_or(NOT_FOUND)?)
}

fn list_machines_with(herdr: &Path) -> Result<Vec<Machine>, String> {
    let json = run_herdr(herdr, &["machine", "list", "--json"])?;
    herdr_link::parse_machines(&json)
        .map_err(|e| format!("herdr printed a machine list Gespan cannot read: {e}"))
}

/// Turns a saved machine on or off. Blocks; call from a background task.
pub fn set_enabled(id: &str, on: bool) -> Result<(), String> {
    set_enabled_with(&herdr_binary().ok_or(NOT_FOUND)?, id, on)
}

fn set_enabled_with(herdr: &Path, id: &str, on: bool) -> Result<(), String> {
    let action = if on { "enable" } else { "disable" };
    run_herdr(herdr, &["machine", action, id]).map(drop)
}

/// Forgets a saved machine; its remote sessions keep running. Blocks; call
/// from a background task.
pub fn remove(id: &str) -> Result<(), String> {
    remove_with(&herdr_binary().ok_or(NOT_FOUND)?, id)
}

fn remove_with(herdr: &Path, id: &str) -> Result<(), String> {
    run_herdr(herdr, &["machine", "remove", id]).map(drop)
}

/// Always quoted, so a terminal runs it word for word. POSIX shells only;
/// the Machines UI is hidden on Windows.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

fn command_line<'a>(herdr: &Path, words: impl IntoIterator<Item = &'a str>) -> String {
    let mut line = quote(&herdr.to_string_lossy());
    for word in words {
        line.push(' ');
        line.push_str(word);
    }
    line
}

/// The terminal command that adds a machine. herdr asks its install
/// question there. `None` when no herdr is found; may block like
/// [`herdr_binary`].
pub fn add_command(target: &str, label: Option<&str>) -> Option<String> {
    herdr_binary().map(|herdr| add_command_with(&herdr, target, label))
}

pub(crate) fn add_command_with(herdr: &Path, target: &str, label: Option<&str>) -> String {
    let target = quote(target);
    let label = label.map(quote);
    let mut words = vec!["machine", "add", target.as_str()];
    if let Some(label) = &label {
        words.extend(["--label", label.as_str()]);
    }
    command_line(herdr, words)
}

/// The terminal command that opens one agent's terminal. `None` when no
/// herdr is found; may block like [`herdr_binary`].
pub fn attach_command(machine_id: &str, agent: &str) -> Option<String> {
    herdr_binary().map(|herdr| attach_command_with(&herdr, machine_id, agent))
}

pub(crate) fn attach_command_with(herdr: &Path, machine_id: &str, agent: &str) -> String {
    let (id, agent) = (quote(machine_id), quote(agent));
    command_line(herdr, ["--machine", &id, "agent", "attach", &agent])
}

/// The terminal command that installs this herdr on a machine, through
/// herdr's own remote attach. `remote_binary` comes from
/// [`prepare_remote_binary`]; handing it over in `HERDR_REMOTE_BINARY`
/// makes herdr install it, after asking, even over a working older herdr.
/// `machine add` is not used: it would save a second machine under a new
/// id. `None` when no herdr is found; may block like [`herdr_binary`].
pub fn upgrade_command(m: &Machine, remote_binary: &Path) -> Option<String> {
    herdr_binary().map(|herdr| upgrade_command_with(&herdr, m, remote_binary))
}

pub(crate) fn upgrade_command_with(herdr: &Path, m: &Machine, remote_binary: &Path) -> String {
    // POSIX shells only; the Machines UI is hidden on Windows.
    let (target, session) = (quote(&m.target), quote(&m.session));
    format!(
        "HERDR_REMOTE_BINARY={} {}",
        quote(&remote_binary.to_string_lossy()),
        command_line(herdr, ["--remote", &target, "--session", &session])
    )
}

/// One herdr release build a machine can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RemoteAsset {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// The builds of [`HERDR_VERSION`], as pinned in `scripts/herdr-release.json`.
pub(crate) const REMOTE_ASSETS: [RemoteAsset; 4] = [
    RemoteAsset {
        name: "linux-x86_64",
        url: "https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-linux-x86_64",
        sha256: "14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d",
    },
    RemoteAsset {
        name: "linux-aarch64",
        url: "https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-linux-aarch64",
        sha256: "17d7536f2da50859523680ef279dffcac8e4a235f4e46fe8da6c2ec44141897c",
    },
    RemoteAsset {
        name: "macos-x86_64",
        url: "https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-macos-x86_64",
        sha256: "5fb0a908ec7f6745e3e245738b06bfad56c46391c391ab3c0955fbead01f1369",
    },
    RemoteAsset {
        name: "macos-aarch64",
        url: "https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-macos-aarch64",
        sha256: "fb77c2232a05a8035421382c894392f4174e4e38a18e48af3dd0290157c3a7e2",
    },
];

const SSH_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// Far above any herdr build; stops a runaway download.
const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

/// The build for a machine, from its `uname -sm`.
fn asset_for<'a>(uname: &str, assets: &'a [RemoteAsset]) -> Result<&'a RemoteAsset, String> {
    let mut words = uname.split_whitespace();
    let (os, arch) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let name = match (os, arch) {
        ("Linux", "x86_64" | "amd64") => "linux-x86_64",
        ("Linux", "aarch64" | "arm64") => "linux-aarch64",
        ("Darwin", "x86_64") => "macos-x86_64",
        ("Darwin", "arm64") => "macos-aarch64",
        _ => return Err(format!("herdr has no build for {os} {arch}")),
    };
    assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| format!("herdr has no build for {os} {arch}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let response = ureq::get(url)
        .timeout(DOWNLOAD_TIMEOUT)
        .call()
        .map_err(|e| format!("could not download herdr: {e}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(DOWNLOAD_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not download herdr: {e}"))?;
    Ok(bytes)
}

/// The verified herdr build for this machine's platform, downloaded once
/// into `<data dir>/herdr-assets`. Runs ssh and may download, so it blocks;
/// call from a background task.
pub fn prepare_remote_binary(m: &Machine) -> Result<PathBuf, String> {
    let dir = data_dir()
        .ok_or("Gespan has no data folder")?
        .join("herdr-assets");
    prepare_remote_binary_with(m, Path::new("ssh"), &dir, &REMOTE_ASSETS, &download)
}

fn prepare_remote_binary_with(
    m: &Machine,
    ssh: &Path,
    dir: &Path,
    assets: &[RemoteAsset],
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let target = herdr_link::validate_target(&m.target).map_err(|e| e.to_string())?;
    let args: Vec<String> = [
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=10",
        "--",
        target,
        "uname",
        "-sm",
    ]
    .map(String::from)
    .to_vec();
    let uname = crate::cli::output(ssh, &args, &home(), SSH_TIMEOUT)
        .map_err(|e| format!("could not reach {target}: {e}"))?;
    let asset = asset_for(&uname, assets)?;
    let path = dir.join(format!("herdr-{HERDR_VERSION}-{}", asset.name));
    if std::fs::read(&path).is_ok_and(|bytes| sha256_hex(&bytes) == asset.sha256) {
        return Ok(path);
    }
    let bytes = fetch(asset.url)?;
    if sha256_hex(&bytes) != asset.sha256 {
        return Err(format!(
            "the downloaded herdr for {} does not match its checksum",
            asset.name
        ));
    }
    store_executable(&path, &bytes).map_err(|e| format!("could not save herdr: {e}"))?;
    Ok(path)
}

/// Written to a temporary file, made executable, then renamed into place.
fn store_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// What a machine's agent row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Attention,
    Working,
    /// Finished a turn the user has not seen yet.
    Done,
    Idle,
}

/// `seen_seq` is the last `completion_seq` the user saw for this terminal.
/// An unknown status shows no mark.
pub fn mark(agent: &Agent, seen_seq: Option<u64>) -> Option<Mark> {
    Some(match agent.agent_status {
        AgentStatus::Blocked => Mark::Attention,
        AgentStatus::Working => Mark::Working,
        AgentStatus::Idle | AgentStatus::Done => {
            if agent.completion_seq.unwrap_or(0) > seen_seq.unwrap_or(0) {
                Mark::Done
            } else {
                Mark::Idle
            }
        }
        AgentStatus::Unknown => return None,
    })
}

/// One saved machine and what is known about it. A disabled machine has no
/// connection, so its snapshot stays the default. Offer the upgrade when
/// `version_mismatch.is_some() || needs_install`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineView {
    pub machine: Machine,
    pub snapshot: Snapshot,
    /// The machine's herdr version, when it answers and differs from
    /// [`HERDR_VERSION`].
    pub version_mismatch: Option<String>,
    /// The machine has no herdr, or one too old to answer at all.
    pub needs_install: bool,
}

impl MachineView {
    fn new(machine: Machine, snapshot: Snapshot) -> Self {
        let version_mismatch = snapshot
            .version
            .clone()
            .filter(|version| version != HERDR_VERSION);
        let needs_install = matches!(snapshot.online, Online::HerdrMissing | Online::HerdrTooOld);
        Self {
            machine,
            snapshot,
            version_mismatch,
            needs_install,
        }
    }
}

/// The last seen `completion_seq` per `<machine_id>/<terminal_id>`, kept in
/// `herdr-seen.json`. A missing or unreadable file reads as empty.
pub(crate) struct SeenStore {
    path: Option<PathBuf>,
    seen: HashMap<String, u64>,
}

impl SeenStore {
    pub(crate) fn open(dir: Option<&Path>) -> Self {
        let path = dir.map(|dir| dir.join(SEEN_FILE));
        let seen = path.as_deref().map(read_seen).unwrap_or_default();
        Self { path, seen }
    }

    fn key(machine_id: &str, terminal_id: &str) -> String {
        format!("{machine_id}/{terminal_id}")
    }

    pub(crate) fn get(&self, machine_id: &str, terminal_id: &str) -> Option<u64> {
        self.seen.get(&Self::key(machine_id, terminal_id)).copied()
    }

    /// Records the mark and saves it. A failed save is logged; the mark
    /// still holds until Gespan quits.
    /// Whether the mark changed.
    pub(crate) fn set(&mut self, machine_id: &str, terminal_id: &str, completion_seq: u64) -> bool {
        let key = Self::key(machine_id, terminal_id);
        if self.seen.insert(key, completion_seq) == Some(completion_seq) {
            return false;
        }
        if let Some(path) = &self.path {
            if let Err(error) = write_seen(path, &self.seen) {
                tracing::warn!(%error, path = %path.display(), "could not save seen herdr turns");
            }
        }
        true
    }
}

fn read_seen(path: &Path) -> HashMap<String, u64> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return HashMap::new(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not read seen herdr turns");
            return HashMap::new();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(%error, path = %path.display(), "ignoring unreadable seen herdr turns");
        HashMap::new()
    })
}

/// Written to a temporary file and renamed over the old one, so a crash
/// never leaves half a file.
fn write_seen(path: &Path, seen: &HashMap<String, u64>) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(&serde_json::to_vec(seen)?)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

type ListMachines = Box<dyn Fn() -> Result<Vec<Machine>, String> + Send + Sync>;
type OnSnapshot = Box<dyn Fn(&Snapshot) + Send>;
type Connect = Box<dyn Fn(&Machine, OnSnapshot) -> Connection + Send + Sync>;

struct Live {
    machine: Machine,
    /// Tells this connection's callbacks from those of one it replaced.
    generation: u64,
    connection: Connection,
}

#[derive(Default)]
struct State {
    machines: Vec<Machine>,
    live: HashMap<String, Live>,
    error: Option<String>,
}

/// Every saved machine, with a live connection to each enabled one.
/// `on_change` runs, on any thread, whenever what [`Registry::machines`]
/// returns may have changed.
pub struct Registry {
    me: Weak<Registry>,
    list: ListMachines,
    connect: Connect,
    on_change: Box<dyn Fn() + Send + Sync>,
    state: Mutex<State>,
    /// Held across a whole reload, so two never apply out of order.
    /// Callbacks never take it.
    reloading: Mutex<()>,
    seen: Mutex<SeenStore>,
    generations: AtomicU64,
}

impl Registry {
    /// Starts empty and loads the machine list on a thread of its own.
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Arc<Registry> {
        let registry = Self::with(
            Box::new(list_machines),
            Box::new(|m, on_snapshot| Connection::start(SshBridge::for_machine(m), on_snapshot)),
            SeenStore::open(data_dir().as_deref()),
            on_change,
        );
        let first = Arc::downgrade(&registry);
        let spawned = std::thread::Builder::new()
            .name("herdr-machines".into())
            .spawn(move || {
                if let Some(registry) = first.upgrade() {
                    registry.reload();
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not load herdr machines");
        }
        registry
    }

    fn with(
        list: ListMachines,
        connect: Connect,
        seen: SeenStore,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Registry> {
        Arc::new_cyclic(|me| Registry {
            me: me.clone(),
            list,
            connect,
            on_change: Box::new(on_change),
            state: Mutex::default(),
            reloading: Mutex::default(),
            seen: Mutex::new(seen),
            generations: AtomicU64::new(0),
        })
    }

    /// Every saved machine, in herdr's order.
    pub fn machines(&self) -> Vec<MachineView> {
        let state = lock(&self.state);
        state
            .machines
            .iter()
            .map(|m| {
                let snapshot = state
                    .live
                    .get(&m.id)
                    .map(|live| live.connection.snapshot())
                    .unwrap_or_default();
                MachineView::new(m.clone(), snapshot)
            })
            .collect()
    }

    /// Why the last [`Registry::reload`] could not read the machine list;
    /// the list before it is kept.
    pub fn list_error(&self) -> Option<String> {
        lock(&self.state).error.clone()
    }

    /// Re-reads herdr's saved machines, connects newly enabled ones and
    /// drops connections to removed or disabled ones. A machine whose
    /// target or session changed is reconnected. Runs herdr, so it blocks;
    /// call from a background task.
    pub fn reload(&self) {
        let _reloading = lock(&self.reloading);
        let listed = (self.list)();
        let mut stale = Vec::new();
        {
            let mut state = lock(&self.state);
            match listed {
                Err(error) => {
                    tracing::warn!(%error, "could not list herdr machines");
                    state.error = Some(error);
                }
                Ok(machines) => {
                    state.error = None;
                    let mut old = std::mem::take(&mut state.live);
                    for m in machines.iter().filter(|m| m.enabled) {
                        let live = match old.remove(&m.id) {
                            Some(mut live)
                                if live.machine.target == m.target
                                    && live.machine.session == m.session =>
                            {
                                live.machine = m.clone();
                                live
                            }
                            replaced => {
                                stale.extend(replaced);
                                self.connect_to(m)
                            }
                        };
                        state.live.insert(m.id.clone(), live);
                    }
                    stale.extend(old.into_values());
                    state.machines = machines;
                }
            }
        }
        // Dropping a connection only signals its thread.
        drop(stale);
        (self.on_change)();
    }

    fn connect_to(&self, m: &Machine) -> Live {
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let (me, id) = (self.me.clone(), m.id.clone());
        let connection = (self.connect)(
            m,
            Box::new(move |_: &Snapshot| {
                if let Some(registry) = me.upgrade() {
                    registry.changed(&id, generation);
                }
            }),
        );
        Live {
            machine: m.clone(),
            generation,
            connection,
        }
    }

    /// A connection's snapshot changed. One stopped or replaced may still
    /// call once; that call is ignored.
    fn changed(&self, id: &str, generation: u64) {
        let current = lock(&self.state)
            .live
            .get(id)
            .is_some_and(|live| live.generation == generation);
        if current {
            (self.on_change)();
        }
    }

    #[cfg(test)]
    fn generation(&self, id: &str) -> Option<u64> {
        lock(&self.state).live.get(id).map(|live| live.generation)
    }

    /// Re-lists one machine now, or retries it at once if it is waiting.
    pub fn refresh(&self, id: &str) {
        if let Some(live) = lock(&self.state).live.get(id) {
            live.connection.refresh();
        }
    }

    /// The user saw this terminal's turn number `completion_seq`. Writes the
    /// seen store, so call from a background task.
    pub fn mark_seen(&self, machine_id: &str, terminal_id: &str, completion_seq: u64) {
        let changed = lock(&self.seen).set(machine_id, terminal_id, completion_seq);
        if changed {
            (self.on_change)();
        }
    }

    /// The last turn number the user saw for this terminal, for [`mark`].
    pub fn seen_seq(&self, machine_id: &str, terminal_id: &str) -> Option<u64> {
        lock(&self.seen).get(machine_id, terminal_id)
    }
}

#[cfg(test)]
mod tests;
