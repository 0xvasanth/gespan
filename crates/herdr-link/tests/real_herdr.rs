//! Runs herdr-link against a real herdr server, the pinned release. CI points
//! `HERDR_TEST_BINARY` at it; without it each test says so and passes.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use herdr_link::{Client, Connection, Event, LocalSocket, Online, Snapshot};
use serde_json::{json, Value};

/// The herdr this crate is written against; keep in step with
/// `scripts/herdr-release.json`.
const PINNED_VERSION: &str = "0.9.5";
const WAIT: Duration = Duration::from_secs(10);
/// A unix socket path must fit `sun_path` (104 bytes on macOS).
const MAX_SOCKET_PATH: usize = 103;

fn herdr_binary() -> Option<PathBuf> {
    match std::env::var_os("HERDR_TEST_BINARY") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("skipped: HERDR_TEST_BINARY not set");
            None
        }
    }
}

/// A herdr server in the foreground of a child process, with its config and
/// state in a private temp dir. It is stopped when dropped.
struct Server {
    child: Child,
    socket: PathBuf,
    cwd: tempfile::TempDir,
    _home: tempfile::TempDir,
}

impl Server {
    fn start(binary: &Path) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        // Short base: a deep temp dir would push the socket past `sun_path`.
        let home = tempfile::Builder::new()
            .prefix("hl")
            .tempdir_in("/tmp")
            .unwrap();
        let session = format!(
            "hl-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let config = home.path().join("config");
        let socket = config
            .join("herdr/sessions")
            .join(&session)
            .join("herdr.sock");
        assert!(
            socket.as_os_str().len() <= MAX_SOCKET_PATH,
            "test socket path is {} bytes, over the {MAX_SOCKET_PATH} a unix socket allows: {}",
            socket.as_os_str().len(),
            socket.display()
        );
        let child = Command::new(binary)
            .args(["--session", &session, "server"])
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env_remove("HERDR_SOCKET_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start herdr server");
        let mut server = Self {
            child,
            socket,
            cwd: tempfile::tempdir().unwrap(),
            _home: home,
        };
        server.wait_for_socket();
        server
    }

    fn wait_for_socket(&mut self) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("herdr server exited early: {status}");
            }
            // The file appears before the server accepts; ping to be sure.
            if self.socket.exists() && self.client().ping().is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "no herdr socket at {} within {WAIT:?}",
            self.socket.display()
        );
    }

    fn client(&self) -> Client<LocalSocket> {
        Client::new(LocalSocket(self.socket.clone()))
    }

    fn create_workspace(&self) -> Value {
        self.client()
            .call("workspace.create", json!({ "cwd": self.cwd.path() }))
            .expect("workspace.create")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.client().call("server.stop", json!({}));
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline && self.child.try_wait().ok().flatten().is_none() {
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Wait for a snapshot that `want` accepts, failing with the last one seen.
fn wait_for(rx: &mpsc::Receiver<Snapshot>, want: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + WAIT;
    let mut last = None;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(s) if want(&s) => return s,
            Ok(s) => last = Some(s),
            Err(_) => break,
        }
    }
    panic!("no matching snapshot within {WAIT:?}; last seen: {last:?}");
}

#[test]
fn ping_reports_pinned_version() {
    let Some(binary) = herdr_binary() else { return };
    let server = Server::start(&binary);
    let pong = server.client().ping().unwrap();
    assert_eq!(pong.version, PINNED_VERSION);
    assert_eq!(pong.protocol, 22);
}

#[test]
fn a_missing_socket_is_an_error_not_a_hang() {
    // Needs no herdr: the socket path simply does not exist.
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(LocalSocket(dir.path().join("nothing.sock")));
    assert!(client.ping().is_err());
}

#[test]
fn workspace_create_emits_pane_created() {
    let Some(binary) = herdr_binary() else { return };
    let server = Server::start(&binary);
    let mut events = server
        .client()
        .subscribe(&[json!({ "type": "pane.created" })])
        .unwrap();
    // Never wait forever on the stream: cut it if nothing arrives.
    let closer = events.closer().expect("local streams can be cut");
    let watchdog = thread::spawn(move || {
        thread::sleep(WAIT);
        closer.close();
    });
    server.create_workspace();
    let event = events.next().expect("stream ended").expect("stream error");
    assert!(
        matches!(&event, Event::PaneCreated { pane_id } if !pane_id.is_empty()),
        "expected PaneCreated, got {event:?}"
    );
    drop(watchdog);
}

#[test]
fn connection_sees_new_pane() {
    let Some(binary) = herdr_binary() else { return };
    let server = Server::start(&binary);
    let first = server.create_workspace();
    let first_id = first["workspace"]["workspace_id"]
        .as_str()
        .expect("workspace.create names the workspace")
        .to_owned();

    let (tx, rx) = mpsc::channel();
    let connection = Connection::start(LocalSocket(server.socket.clone()), move |s: &Snapshot| {
        let _ = tx.send(s.clone());
    });
    let online = wait_for(&rx, |s| {
        s.online == Online::Online && s.workspaces.iter().any(|w| w.workspace_id == first_id)
    });
    assert_eq!(online.version.as_deref(), Some(PINNED_VERSION));

    // A workspace made while connected arrives through the event stream.
    let second = server.create_workspace();
    let second_id = second["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(first_id, second_id);
    let grown = wait_for(&rx, |s| {
        s.workspaces.iter().any(|w| w.workspace_id == second_id)
    });
    assert!(grown.workspaces.iter().any(|w| w.workspace_id == first_id));
    assert_eq!(connection.snapshot().online, Online::Online);
    connection.stop();
}

#[test]
fn connection_goes_offline_when_the_server_stops() {
    let Some(binary) = herdr_binary() else { return };
    let server = Server::start(&binary);
    let (tx, rx) = mpsc::channel();
    let connection = Connection::start(LocalSocket(server.socket.clone()), move |s: &Snapshot| {
        let _ = tx.send(s.clone());
    });
    wait_for(&rx, |s| s.online == Online::Online);
    server.client().call("server.stop", json!({})).unwrap();
    let down = wait_for(&rx, |s| matches!(s.online, Online::Offline { .. }));
    assert!(matches!(down.online, Online::Offline { .. }));
    connection.stop();
}

/// Run `f` on a thread and fail if it does not return in time, so a hang
/// (for instance in draining ssh's stderr) fails the test, not the CI job.
#[cfg(target_os = "linux")]
fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| panic!("{what} did not finish within 20 s"))
}

/// Needs a local sshd with key login for this user and a herdr server in the
/// default session on that machine; CI sets both up and names the target.
#[cfg(target_os = "linux")]
#[test]
fn ssh_bridge_against_local_sshd() {
    use herdr_link::{Machine, SshBridge};

    let Ok(target) = std::env::var("HERDR_SSHD_TARGET") else {
        eprintln!("skipped: HERDR_SSHD_TARGET not set");
        return;
    };
    let machine = Machine {
        id: "sshd-test".into(),
        label: "local sshd".into(),
        target,
        session: "default".into(),
        enabled: true,
    };
    let bridge = SshBridge::for_machine(&machine);
    let control = bridge
        .control_dir
        .clone()
        .expect("a private control dir is available on the CI runner");
    let client = std::sync::Arc::new(Client::new(bridge));

    let first = {
        let client = client.clone();
        within("first ping through ssh", move || client.ping())
    }
    .expect("first ping");
    assert_eq!(first.version, PINNED_VERSION);
    assert!(
        control.join("cm").exists(),
        "the first call leaves an ssh master at {}",
        control.display()
    );

    // The second call rides the master; a hang while draining stderr after
    // ControlPersist forks the master would show up here.
    let second = {
        let client = client.clone();
        within("second ping through ssh", move || client.ping())
    }
    .expect("second ping");
    assert_eq!(second.version, PINNED_VERSION);
    assert!(control.join("cm").exists());
}
