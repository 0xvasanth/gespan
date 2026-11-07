use super::*;
use herdr_link::AgentStatus::{Blocked, Done, Idle, Unknown, Working};
use herdr_link::{AgentStatus, Conn, Online, Transport};
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
        upgrade_command_with(herdr, &m),
        "'/A pp/herdr' --remote 'me@box' --session 'agents'"
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

/// A machine that can never be reached.
struct Unreachable;

impl Transport for Unreachable {
    fn open(&self) -> std::io::Result<Conn> {
        Err(std::io::Error::other("no route"))
    }
}

struct Harness {
    registry: Arc<Registry>,
    list: Arc<Mutex<Result<Vec<Machine>, String>>>,
    started: Arc<Mutex<Vec<String>>>,
    changes: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

fn harness(machines: Vec<Machine>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let list = Arc::new(Mutex::new(Ok(machines)));
    let started = Arc::new(Mutex::new(Vec::new()));
    let changes = Arc::new(AtomicUsize::new(0));
    let (l, s, c) = (list.clone(), started.clone(), changes.clone());
    let registry = Registry::with(
        Box::new(move || l.lock().unwrap().clone()),
        Box::new(move |m: &Machine, on_change| {
            s.lock().unwrap().push(m.id.clone());
            Connection::start(Unreachable, on_change)
        }),
        SeenStore::open(Some(dir.path())),
        move || {
            c.fetch_add(1, Ordering::SeqCst);
        },
    );
    Harness {
        registry,
        list,
        started,
        changes,
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
    assert!(h.changes.load(Ordering::SeqCst) >= 2);
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
    // Settle, then count only the late callbacks below.
    wait_until("the new connection to go offline", || {
        matches!(online_of(&h.registry, "m1"), Online::Offline { .. })
    });
    let before = h.changes.load(Ordering::SeqCst);
    h.registry.changed("m1", first);
    h.registry.changed("gone", second);
    assert_eq!(h.changes.load(Ordering::SeqCst), before);
    h.registry.changed("m1", second);
    assert_eq!(h.changes.load(Ordering::SeqCst), before + 1);
}

#[test]
fn mark_seen_is_kept_and_announced() {
    let h = harness(vec![machine("m1", "a", true)]);
    let before = h.changes.load(Ordering::SeqCst);
    h.registry.mark_seen("m1", "t1", 5);
    assert_eq!(h.registry.seen_seq("m1", "t1"), Some(5));
    assert_eq!(h.registry.seen_seq("m1", "t2"), None);
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
    let weak = Arc::downgrade(&h.registry);
    let Harness { registry, .. } = h;
    drop(registry);
    assert!(
        weak.upgrade().is_none(),
        "a connection kept the registry alive"
    );
}
