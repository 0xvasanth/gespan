use super::*;
use herdr_link::AgentStatus::{Blocked, Done, Idle, Unknown, Working};
use herdr_link::{AgentStatus, Conn, Online, Transport};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn a(status: AgentStatus, completion_seq: Option<u64>) -> Agent {
    Agent {
        terminal_id: "t1".into(),
        pane_id: "w1:p1".into(),
        workspace_id: "w1".into(),
        tab_id: "w1:t1".into(),
        name: None,
        agent: Some("claude".into()),
        title: None,
        agent_status: status,
        completion_seq,
        cwd: None,
    }
}

fn machine(id: &str, target: &str, enabled: bool) -> Machine {
    Machine {
        id: id.into(),
        label: id.into(),
        target: target.into(),
        session: "default".into(),
        enabled,
    }
}

fn view_with_version(version: &str) -> MachineView {
    MachineView::new(
        machine("m1", "box", true),
        Snapshot {
            version: Some(version.into()),
            ..Snapshot::default()
        },
    )
}

#[test]
fn mark_maps_herdr_status() {
    assert_eq!(mark(&a(Blocked, None), None), Some(Mark::Attention));
    assert_eq!(mark(&a(Blocked, Some(9)), Some(1)), Some(Mark::Attention));
    assert_eq!(mark(&a(Working, None), None), Some(Mark::Working));
    assert_eq!(mark(&a(Working, Some(9)), Some(1)), Some(Mark::Working));
    assert_eq!(mark(&a(Unknown, Some(9)), None), None);
}

#[test]
fn finished_turn_is_done_until_seen() {
    assert_eq!(mark(&a(Idle, Some(4)), Some(3)), Some(Mark::Done));
    assert_eq!(mark(&a(Idle, Some(4)), None), Some(Mark::Done));
    assert_eq!(mark(&a(Done, Some(4)), Some(3)), Some(Mark::Done));
    assert_eq!(mark(&a(Idle, Some(4)), Some(4)), Some(Mark::Idle));
    assert_eq!(mark(&a(Done, Some(4)), Some(4)), Some(Mark::Idle));
    assert_eq!(mark(&a(Idle, None), None), Some(Mark::Idle));
    assert_eq!(mark(&a(Done, None), None), Some(Mark::Idle));
    // A restarted server counts from zero again; an older mark means seen.
    assert_eq!(mark(&a(Idle, Some(2)), Some(7)), Some(Mark::Idle));
}

#[test]
fn seen_survives_reload() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SeenStore::open(Some(dir.path()));
    assert_eq!(store.get("m1", "t1"), None);
    store.set("m1", "t1", 4);
    store.set("m2", "t1", 9);
    let again = SeenStore::open(Some(dir.path()));
    assert_eq!(again.get("m1", "t1"), Some(4));
    assert_eq!(again.get("m2", "t1"), Some(9));
    assert_eq!(again.get("m1", "t2"), None);
    let raw = std::fs::read_to_string(dir.path().join("herdr-seen.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(json["m1/t1"], 4);
    // Only the store file is left behind, no temporary files.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("herdr-seen.json")]);
}

#[test]
fn seen_mark_can_move_back_after_a_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SeenStore::open(Some(dir.path()));
    store.set("m1", "t1", 7);
    store.set("m1", "t1", 2);
    assert_eq!(SeenStore::open(Some(dir.path())).get("m1", "t1"), Some(2));
}

#[test]
fn corrupt_seen_file_reads_as_empty_and_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("herdr-seen.json");
    std::fs::write(&path, "{not json").unwrap();
    let mut store = SeenStore::open(Some(dir.path()));
    assert_eq!(store.get("m1", "t1"), None);
    store.set("m1", "t1", 3);
    assert_eq!(SeenStore::open(Some(dir.path())).get("m1", "t1"), Some(3));
}

#[test]
fn seen_without_a_data_dir_stays_in_memory() {
    let mut store = SeenStore::open(None);
    store.set("m1", "t1", 3);
    assert_eq!(store.get("m1", "t1"), Some(3));
}

#[test]
fn seen_in_a_missing_folder_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("not/yet");
    let mut store = SeenStore::open(Some(&nested));
    store.set("m1", "t1", 1);
    assert_eq!(SeenStore::open(Some(&nested)).get("m1", "t1"), Some(1));
}

#[test]
fn version_mismatch_is_reported() {
    assert_eq!(
        view_with_version("0.9.4").version_mismatch.as_deref(),
        Some("0.9.4")
    );
    assert!(view_with_version(HERDR_VERSION).version_mismatch.is_none());
    let unknown = MachineView::new(machine("m1", "box", true), Snapshot::default());
    assert!(unknown.version_mismatch.is_none());
    assert!(!unknown.needs_install);
}

#[test]
fn versions_compare_by_number_not_text() {
    for newer in ["0.9.6", "0.9.10", "0.10.0", "1.0.0", "0.9.5.1"] {
        assert!(is_newer_than_pinned(newer), "{newer}");
    }
    for not_newer in ["0.9.5", "0.9.4", "0.8.99", "0.9", "", "nightly"] {
        assert!(!is_newer_than_pinned(not_newer), "{not_newer}");
    }
    // A pre-release tag does not hide the number in front of it.
    assert!(is_newer_than_pinned("0.9.6-rc.1"));
    assert!(!is_newer_than_pinned("0.9.4-rc.1"));
}

#[test]
fn missing_or_too_old_herdr_needs_install() {
    let with = |online| {
        MachineView::new(
            machine("m1", "box", true),
            Snapshot {
                online,
                ..Snapshot::default()
            },
        )
    };
    assert!(with(Online::HerdrMissing).needs_install);
    assert!(with(Online::HerdrTooOld).needs_install);
    assert!(!with(Online::Online).needs_install);
    assert!(
        !with(Online::NeedsSignIn {
            hint: "ssh box".into()
        })
        .needs_install
    );
    assert!(
        !with(Online::Offline {
            reason: "down".into(),
            retry_in: Duration::from_secs(1)
        })
        .needs_install
    );
    // An answering machine on the pinned version needs nothing.
    let current = view_with_version(HERDR_VERSION);
    assert!(current.version_mismatch.is_none() && !current.needs_install);
}

#[test]
fn remote_assets_match_the_packaging_pin() {
    let pin: serde_json::Value =
        serde_json::from_str(include_str!("../../../../scripts/herdr-release.json")).unwrap();
    assert_eq!(pin["version"], HERDR_VERSION);
    let pinned = pin["assets"].as_object().unwrap();
    assert_eq!(pinned.len(), REMOTE_ASSETS.len());
    for asset in &REMOTE_ASSETS {
        let entry = &pinned[asset.name];
        assert_eq!(entry["url"], asset.url, "{}", asset.name);
        assert_eq!(entry["sha256"], asset.sha256, "{}", asset.name);
    }
}

#[test]
fn uname_picks_the_build() {
    let name = |uname: &str| asset_for(uname, &REMOTE_ASSETS).map(|a| a.name);
    assert_eq!(name("Linux x86_64\n"), Ok("linux-x86_64"));
    assert_eq!(name("Linux amd64"), Ok("linux-x86_64"));
    assert_eq!(name("Linux aarch64"), Ok("linux-aarch64"));
    assert_eq!(name("Linux arm64"), Ok("linux-aarch64"));
    assert_eq!(name("Darwin x86_64"), Ok("macos-x86_64"));
    assert_eq!(name("Darwin arm64"), Ok("macos-aarch64"));
    assert_eq!(
        name("FreeBSD amd64"),
        Err("herdr has no build for FreeBSD amd64".into())
    );
    assert_eq!(
        name("Linux armv7l"),
        Err("herdr has no build for Linux armv7l".into())
    );
    assert!(name("").is_err());
}

#[test]
fn commands_quote_paths_and_names() {
    let herdr = Path::new("/A pp/herdr");
    assert_eq!(
        attach_command_with(herdr, "id1", "my agent"),
        "'/A pp/herdr' --machine 'id1' agent attach 'my agent'"
    );
    assert_eq!(
        attach_command_with(herdr, "id1", "it's"),
        r"'/A pp/herdr' --machine 'id1' agent attach 'it'\''s'"
    );
    assert_eq!(
        add_command_with(herdr, "me@box", None),
        "'/A pp/herdr' machine add 'me@box'"
    );
    assert_eq!(
        add_command_with(herdr, "me@box", Some("work; rm -rf ~")),
        "'/A pp/herdr' machine add 'me@box' --label 'work; rm -rf ~'"
    );
    let mut m = machine("id1", "me@box", true);
    m.session = "agents".into();
    assert_eq!(
        upgrade_command_with(herdr, &m, Path::new("/data dir/herdr-0.9.5-linux-aarch64")),
        "HERDR_REMOTE_BINARY='/data dir/herdr-0.9.5-linux-aarch64' '/A pp/herdr' --remote 'me@box' --session 'agents'"
    );
}

#[test]
fn env_override_wins_for_binary() {
    let dir = tempfile::tempdir().unwrap();
    let bundled = dir.path().join(HERDR_FILE);
    std::fs::write(&bundled, "").unwrap();
    let chosen = PathBuf::from("/opt/custom/herdr");
    assert_eq!(
        find_herdr(Some(chosen.clone().into()), Some(dir.path()), String::new),
        Some(chosen)
    );
    // An empty override is no override.
    assert_eq!(
        find_herdr(Some("".into()), Some(dir.path()), String::new),
        Some(bundled)
    );
}

#[test]
fn missing_herdr_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(find_herdr(None, Some(dir.path()), String::new), None);
    assert_eq!(find_herdr(None, None, String::new), None);
}

#[cfg(unix)]
fn fake_herdr(dir: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("herdr");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn path_herdr_with_other_version_is_ignored() {
    let bin = tempfile::tempdir().unwrap();
    fake_herdr(bin.path(), "echo 'herdr 0.9.4'");
    let path = bin.path().to_string_lossy().into_owned();
    assert_eq!(find_herdr(None, None, || path.clone()), None);
}

#[cfg(unix)]
#[test]
fn path_herdr_with_pinned_version_is_used() {
    let bin = tempfile::tempdir().unwrap();
    let herdr = fake_herdr(bin.path(), &format!("echo 'herdr {HERDR_VERSION}'"));
    let path = bin.path().to_string_lossy().into_owned();
    assert_eq!(find_herdr(None, None, || path.clone()), Some(herdr));
}

#[cfg(unix)]
#[test]
fn bundled_herdr_beats_path_without_running_it() {
    let exe_dir = tempfile::tempdir().unwrap();
    // The bundled copy is trusted as is; it is never asked its version.
    let bundled = fake_herdr(exe_dir.path(), "exit 1");
    let bin = tempfile::tempdir().unwrap();
    fake_herdr(bin.path(), &format!("echo 'herdr {HERDR_VERSION}'"));
    let path = bin.path().to_string_lossy().into_owned();
    assert_eq!(
        find_herdr(None, Some(exe_dir.path()), || path.clone()),
        Some(bundled)
    );
}

#[cfg(unix)]
#[test]
fn machine_list_is_read_from_herdr() {
    let bin = tempfile::tempdir().unwrap();
    let herdr = fake_herdr(
        bin.path(),
        r#"[ "$*" = "machine list --json" ] || exit 3
echo '[{"id":"abc","label":"lab","target":"me@box","session":"default","enabled":false,"selected":true}]'"#,
    );
    let list = list_machines_with(&herdr).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "abc");
    assert!(!list[0].enabled);
}

#[cfg(unix)]
#[test]
fn herdr_failure_carries_its_message() {
    let bin = tempfile::tempdir().unwrap();
    let herdr = fake_herdr(
        bin.path(),
        "echo 'machine profile x was not found' >&2; exit 1",
    );
    let error = remove_with(&herdr, "x").unwrap_err();
    assert!(error.contains("was not found"), "{error}");
    let garbage = fake_herdr(bin.path(), "echo 'not json'");
    assert!(list_machines_with(&garbage).is_err());
}

#[cfg(unix)]
#[test]
fn enable_disable_and_remove_pass_the_id() {
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("args");
    let herdr = fake_herdr(bin.path(), &format!("echo \"$*\" >> '{}'", log.display()));
    set_enabled_with(&herdr, "abc", true).unwrap();
    set_enabled_with(&herdr, "abc", false).unwrap();
    remove_with(&herdr, "abc").unwrap();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "machine enable abc\nmachine disable abc\nmachine remove abc\n"
    );
}

/// A machine that can never be reached. Counts the attempts.
struct Unreachable(Arc<AtomicUsize>);

impl Transport for Unreachable {
    fn open(&self) -> std::io::Result<Conn> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::other("no route"))
    }
}

/// How a test machine answers. Only the unreachable kind is used where
/// the online one cannot be built.
#[derive(Clone, Copy)]
enum Reply {
    Unreachable,
    /// Needs a sign-in, so its connection waits for a refresh.
    #[cfg(unix)]
    SignIn,
    /// A herdr with no agents; a stream stays open until cut.
    #[cfg(unix)]
    Online,
}

/// A test machine answering as `reply` says. Counts the attempts.
struct TestLink {
    opens: Arc<AtomicUsize>,
    reply: Reply,
}

impl Transport for TestLink {
    fn open(&self) -> std::io::Result<Conn> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        match self.reply {
            Reply::Unreachable => Err(std::io::Error::other("no route")),
            #[cfg(unix)]
            Reply::SignIn => Err(std::io::Error::other(herdr_link::SshFailure::NeedsSignIn {
                hint: "ssh box".into(),
            })),
            #[cfg(unix)]
            Reply::Online => online_conn(),
        }
    }
}

/// One connection to a pretend herdr: answers the request it is sent.
#[cfg(unix)]
fn online_conn() -> std::io::Result<Conn> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    let (ours, theirs) = UnixStream::pair()?;
    std::thread::spawn(move || {
        let mut reader = BufReader::new(theirs.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        let result = match request["method"].as_str().unwrap() {
            "ping" => serde_json::json!({"version": HERDR_VERSION, "protocol": 22}),
            "agent.list" => serde_json::json!({"agents": []}),
            "workspace.list" => serde_json::json!({"workspaces": []}),
            _ => serde_json::json!({"type": "subscription_started"}),
        };
        let reply = serde_json::json!({"id": request["id"], "result": result});
        let mut out = &theirs;
        let _ = writeln!(out, "{reply}");
        // Held open until the other end goes.
        let _ = reader.read_line(&mut line);
    });
    let cut = ours.try_clone()?;
    Ok(Conn {
        writer: Box::new(ours.try_clone()?),
        reader: Box::new(std::io::BufReader::new(ours)),
        guard: None,
        closer: Some(herdr_link::Closer::new(move || {
            let _ = cut.shutdown(std::net::Shutdown::Both);
        })),
    })
}

struct Harness {
    registry: Arc<Registry>,
    list: Arc<Mutex<Result<Vec<Machine>, String>>>,
    started: Arc<Mutex<Vec<String>>>,
    /// Connection attempts per machine id.
    opens: Arc<Mutex<HashMap<String, Arc<AtomicUsize>>>>,
    changes: Arc<AtomicUsize>,
    /// `on_change` calls made on the calling thread, not by a connection.
    direct: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

fn harness(machines: Vec<Machine>) -> Harness {
    harness_with(machines, |_| Reply::Unreachable)
}

fn harness_with(machines: Vec<Machine>, reply: fn(&str) -> Reply) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let list = Arc::new(Mutex::new(Ok(machines)));
    let started = Arc::new(Mutex::new(Vec::new()));
    let changes = Arc::new(AtomicUsize::new(0));
    let direct = Arc::new(AtomicUsize::new(0));
    let d = direct.clone();
    let opens: Arc<Mutex<HashMap<String, Arc<AtomicUsize>>>> = Arc::default();
    let (l, s, c, o) = (
        list.clone(),
        started.clone(),
        changes.clone(),
        opens.clone(),
    );
    let registry = Registry::with(
        Box::new(move || l.lock().unwrap().clone()),
        Box::new(move |m: &Machine, on_change| {
            s.lock().unwrap().push(m.id.clone());
            let count = Arc::new(AtomicUsize::new(0));
            o.lock().unwrap().insert(m.id.clone(), count.clone());
            let reply = reply(&m.id);
            Connection::start(
                TestLink {
                    opens: count,
                    reply,
                },
                on_change,
            )
        }),
        SeenStore::open(Some(dir.path())),
        move || {
            c.fetch_add(1, Ordering::SeqCst);
            if std::thread::current().name() != Some("herdr-connection") {
                d.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    Harness {
        registry,
        list,
        started,
        opens,
        changes,
        direct,
        _dir: dir,
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn opens_of(h: &Harness, id: &str) -> usize {
    h.opens.lock().unwrap()[id].load(Ordering::SeqCst)
}

fn online_of(registry: &Registry, id: &str) -> Online {
    registry
        .machines()
        .into_iter()
        .find(|v| v.machine.id == id)
        .unwrap()
        .snapshot
        .online
}

#[test]
fn reload_connects_enabled_machines_only() {
    let h = harness(vec![machine("on", "a", true), machine("off", "b", false)]);
    h.registry.reload();
    assert_eq!(*h.started.lock().unwrap(), vec!["on".to_string()]);
    let views = h.registry.machines();
    assert_eq!(
        views
            .iter()
            .map(|v| v.machine.id.as_str())
            .collect::<Vec<_>>(),
        vec!["on", "off"]
    );
    wait_until("the enabled machine to go offline", || {
        matches!(online_of(&h.registry, "on"), Online::Offline { .. })
    });
    assert_eq!(online_of(&h.registry, "off"), Online::Connecting);
    // The reload announced itself, and so did the connection.
    wait_until("a change from the connection", || {
        h.changes.load(Ordering::SeqCst) > h.direct.load(Ordering::SeqCst)
    });
    assert_eq!(h.direct.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[test]
fn reload_retries_waiting_machines_but_leaves_online_ones() {
    let h = harness_with(
        vec![machine("signin", "a", true), machine("up", "b", true)],
        |id| match id {
            "signin" => Reply::SignIn,
            _ => Reply::Online,
        },
    );
    h.registry.reload();
    wait_until("one waiting and one online", || {
        matches!(online_of(&h.registry, "signin"), Online::NeedsSignIn { .. })
            && online_of(&h.registry, "up") == Online::Online
    });
    let (waiting, up) = (opens_of(&h, "signin"), opens_of(&h, "up"));
    assert_eq!(waiting, 1, "a sign-in failure is not retried on its own");
    // The user signed in, then the list is read again.
    h.registry.reload();
    wait_until("the waiting machine to try again", || {
        opens_of(&h, "signin") > waiting
    });
    assert_eq!(h.started.lock().unwrap().len(), 2, "kept, not reconnected");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(opens_of(&h, "up"), up, "an online machine is left alone");
    assert_eq!(online_of(&h.registry, "up"), Online::Online);
}

#[test]
fn reload_keeps_unchanged_connections_and_restarts_moved_ones() {
    let h = harness(vec![machine("m1", "a", true), machine("m2", "b", true)]);
    h.registry.reload();
    h.registry.reload();
    assert_eq!(h.started.lock().unwrap().len(), 2);
    // m1 now points elsewhere, m2 is turned off.
    *h.list.lock().unwrap() = Ok(vec![machine("m1", "c", true), machine("m2", "b", false)]);
    h.registry.reload();
    assert_eq!(
        *h.started.lock().unwrap(),
        vec!["m1".to_string(), "m2".to_string(), "m1".to_string()]
    );
    assert_eq!(online_of(&h.registry, "m2"), Online::Connecting);
    assert_eq!(h.registry.machines()[0].machine.target, "c");
}

#[test]
fn failed_reload_keeps_the_last_list_and_reports_why() {
    let h = harness(vec![machine("m1", "a", true)]);
    h.registry.reload();
    assert_eq!(h.registry.list_error(), None);
    *h.list.lock().unwrap() = Err("herdr is not installed".into());
    h.registry.reload();
    assert_eq!(h.registry.machines().len(), 1);
    assert_eq!(h.started.lock().unwrap().len(), 1);
    assert_eq!(
        h.registry.list_error().as_deref(),
        Some("herdr is not installed")
    );
    *h.list.lock().unwrap() = Ok(vec![]);
    h.registry.reload();
    assert!(h.registry.machines().is_empty());
    assert_eq!(h.registry.list_error(), None);
}

#[test]
fn callbacks_from_stopped_connections_are_ignored() {
    let h = harness(vec![machine("m1", "a", true)]);
    h.registry.reload();
    let first = h.registry.generation("m1").unwrap();
    *h.list.lock().unwrap() = Ok(vec![machine("m1", "moved", true)]);
    h.registry.reload();
    let second = h.registry.generation("m1").unwrap();
    assert_ne!(first, second);
    // Count only the calls made here; the live connection keeps reporting.
    wait_until("the new connection to go offline", || {
        matches!(online_of(&h.registry, "m1"), Online::Offline { .. })
    });
    let before = h.direct.load(Ordering::SeqCst);
    h.registry.changed("m1", first);
    h.registry.changed("gone", second);
    assert_eq!(h.direct.load(Ordering::SeqCst), before);
    h.registry.changed("m1", second);
    assert_eq!(h.direct.load(Ordering::SeqCst), before + 1);
}

#[test]
fn mark_seen_is_kept_and_announced() {
    let h = harness(vec![machine("m1", "a", true)]);
    let before = h.changes.load(Ordering::SeqCst);
    h.registry.mark_seen("m1", "t1", 5);
    assert_eq!(h.registry.seen_seq("m1", "t1"), Some(5));
    assert_eq!(h.registry.seen_seq("m1", "t2"), None);
    assert_eq!(h.changes.load(Ordering::SeqCst), before + 1);
    // Seeing the same turn again changes nothing.
    h.registry.mark_seen("m1", "t1", 5);
    assert_eq!(h.changes.load(Ordering::SeqCst), before + 1);
    let agent = a(Idle, Some(5));
    assert_eq!(
        mark(&agent, h.registry.seen_seq("m1", &agent.terminal_id)),
        Some(Mark::Idle)
    );
}

#[test]
fn dropping_the_registry_ends_its_callbacks() {
    let h = harness(vec![machine("m1", "a", true)]);
    h.registry.reload();
    wait_until("the machine to go offline", || {
        matches!(online_of(&h.registry, "m1"), Online::Offline { .. })
    });
    let weak = Arc::downgrade(&h.registry);
    let Harness { registry, .. } = h;
    drop(registry);
    assert!(
        weak.upgrade().is_none(),
        "a connection kept the registry alive"
    );
}

#[test]
fn removed_and_disabled_machines_stop_connecting() {
    let h = harness(vec![
        machine("kept", "a", true),
        machine("removed", "b", true),
        machine("disabled", "c", true),
    ]);
    h.registry.reload();
    // Retries come after 1 s, then 2 s.
    wait_until("two attempts each", || {
        ["kept", "removed", "disabled"]
            .iter()
            .all(|id| opens_of(&h, id) >= 2)
    });
    *h.list.lock().unwrap() = Ok(vec![
        machine("kept", "a", true),
        machine("disabled", "c", false),
    ]);
    // Read before the reload: it retries the waiting kept machine at once,
    // which can land before a later read and leave the next try 8 s away.
    let kept = opens_of(&h, "kept");
    h.registry.reload();
    let (removed, disabled) = (opens_of(&h, "removed"), opens_of(&h, "disabled"));
    std::thread::sleep(Duration::from_millis(3000));
    assert_eq!(opens_of(&h, "removed"), removed);
    assert_eq!(opens_of(&h, "disabled"), disabled);
    // The kept machine went on connecting meanwhile.
    assert!(opens_of(&h, "kept") > kept);
}

#[test]
fn concurrent_reloads_apply_one_at_a_time() {
    // The first reload's lister waits for a second reload to list too, then
    // returns late. Unserialised, the older list would land last.
    let dir = tempfile::tempdir().unwrap();
    let gate = Arc::new((Mutex::new(0u32), std::sync::Condvar::new()));
    let calls = gate.clone();
    let registry = Registry::with(
        Box::new(move || {
            let (n, cv) = &*calls;
            let mut count = n.lock().unwrap();
            *count += 1;
            let me = *count;
            cv.notify_all();
            if me == 1 {
                let waited = cv
                    .wait_timeout_while(count, Duration::from_millis(500), |n| *n < 2)
                    .unwrap();
                drop(waited);
                std::thread::sleep(Duration::from_millis(200));
                Ok(vec![machine("old", "a", true)])
            } else {
                Ok(vec![machine("new", "b", true)])
            }
        }),
        Box::new(|_: &Machine, on_change| {
            Connection::start(Unreachable(Arc::default()), on_change)
        }),
        SeenStore::open(Some(dir.path())),
        || {},
    );
    let first = {
        let r = registry.clone();
        std::thread::spawn(move || r.reload())
    };
    {
        let (n, cv) = &*gate;
        let _listing = cv
            .wait_timeout_while(n.lock().unwrap(), Duration::from_secs(5), |n| *n < 1)
            .unwrap();
    }
    let second = {
        let r = registry.clone();
        std::thread::spawn(move || r.reload())
    };
    first.join().unwrap();
    second.join().unwrap();
    let ids: Vec<_> = registry
        .machines()
        .into_iter()
        .map(|v| v.machine.id)
        .collect();
    assert_eq!(ids, vec!["new".to_string()]);
}

#[cfg(unix)]
fn fake_ssh(dir: &Path, uname: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("ssh-args");
    let path = dir.join("ssh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\necho '{uname}'\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (path, log)
}

const BUILD: &[u8] = b"pretend herdr build";

fn test_assets() -> [RemoteAsset; 1] {
    [RemoteAsset {
        name: "linux-aarch64",
        url: "https://example.invalid/herdr-linux-aarch64",
        sha256: "",
    }]
}

#[cfg(unix)]
#[test]
fn remote_binary_is_downloaded_verified_and_reused() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (ssh, log) = fake_ssh(tmp.path(), "Linux aarch64");
    let assets_dir = tmp.path().join("data/herdr-assets");
    let sha = sha256_hex(BUILD);
    let mut assets = test_assets();
    assets[0].sha256 = Box::leak(sha.into_boxed_str());
    let fetches = AtomicUsize::new(0);
    let fetch = |url: &str| {
        assert_eq!(url, "https://example.invalid/herdr-linux-aarch64");
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok(BUILD.to_vec())
    };
    let m = machine("m1", "me@box", true);
    let path = prepare_remote_binary_with(&m, &ssh, &assets_dir, &assets, &fetch).unwrap();
    assert_eq!(path, assets_dir.join("herdr-0.9.5-linux-aarch64"));
    assert_eq!(std::fs::read(&path).unwrap(), BUILD);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "-o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=10 -- me@box uname -sm\n"
    );
    // A verified copy is used again without downloading.
    let again = prepare_remote_binary_with(&m, &ssh, &assets_dir, &assets, &fetch).unwrap();
    assert_eq!(again, path);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    // A damaged copy is replaced.
    std::fs::write(&path, b"damaged").unwrap();
    prepare_remote_binary_with(&m, &ssh, &assets_dir, &assets, &fetch).unwrap();
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&path).unwrap(), BUILD);
}

#[cfg(unix)]
#[test]
fn checksum_mismatch_leaves_no_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (ssh, _) = fake_ssh(tmp.path(), "Linux aarch64");
    let assets_dir = tmp.path().join("herdr-assets");
    let mut assets = test_assets();
    assets[0].sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
    let error = prepare_remote_binary_with(
        &machine("m1", "me@box", true),
        &ssh,
        &assets_dir,
        &assets,
        &|_| Ok(BUILD.to_vec()),
    )
    .unwrap_err();
    assert!(error.contains("checksum"), "{error}");
    let left = std::fs::read_dir(&assets_dir)
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0);
}

#[cfg(unix)]
#[test]
fn unknown_platform_errors_without_downloading() {
    let tmp = tempfile::tempdir().unwrap();
    let (ssh, _) = fake_ssh(tmp.path(), "SunOS i86pc");
    let error = prepare_remote_binary_with(
        &machine("m1", "me@box", true),
        &ssh,
        &tmp.path().join("herdr-assets"),
        &REMOTE_ASSETS,
        &|_| panic!("nothing to download"),
    )
    .unwrap_err();
    assert_eq!(error, "herdr has no build for SunOS i86pc");
}

#[test]
fn hostile_target_never_reaches_ssh() {
    let tmp = tempfile::tempdir().unwrap();
    let error = prepare_remote_binary_with(
        &machine("m1", "-oProxyCommand=touch /tmp/x", true),
        Path::new("/nonexistent/ssh"),
        &tmp.path().join("herdr-assets"),
        &REMOTE_ASSETS,
        &|_| panic!("nothing to download"),
    )
    .unwrap_err();
    assert!(error.contains("invalid SSH target"), "{error}");
}

#[cfg(unix)]
#[test]
fn unreachable_machine_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let ssh = fake_herdr(
        tmp.path(),
        "echo 'ssh: connect to host box: Connection refused' >&2; exit 255",
    );
    let error = prepare_remote_binary_with(
        &machine("m1", "box", true),
        &ssh,
        &tmp.path().join("herdr-assets"),
        &REMOTE_ASSETS,
        &|_| panic!("nothing to download"),
    )
    .unwrap_err();
    assert!(error.contains("Connection refused"), "{error}");
}
