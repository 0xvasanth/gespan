//! A turn's browser through a real run: a stub Codex lists the turn's tools
//! over the tool endpoint and reports the prompt it was sent.
use super::*;
use gespan_core::agent::AccessMode;
use gespan_core::browser::{
    AccessMode as BrowserAccess, BrowserChoice, BrowserMode, BrowserSettings,
};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::MutexGuard};

/// The browser module is process-wide: its tests take turns, so one test's
/// turn ending never ends another's.
static SERIAL: Mutex<()> = Mutex::new(());

/// The process's browser module, started once with its switch on and
/// "Always on". Its profile has nothing to attach to, so no real browser is
/// ever reached.
fn browser_always_on() -> MutexGuard<'static, ()> {
    static STARTED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    STARTED.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "gespan-runtime-browser-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("no-browser")).unwrap();
        gespan_core::browser::start(Some(&dir)).unwrap();
        let host = gespan_core::browser::host().unwrap();
        host.set_settings(BrowserSettings {
            browser: BrowserChoice::UserDataDir {
                path: dir.join("no-browser").display().to_string(),
            },
            mode: BrowserMode::Attach,
            access: BrowserAccess::Always,
            ..BrowserSettings::default()
        });
        host.set_toggle(true);
        dir
    });
    serial
}

fn turn_active(session: i64) -> Option<bool> {
    let key = gespan_core::browser::session_key(session);
    gespan_core::browser::state()?
        .sessions
        .iter()
        .find(|access| access.session_id == key)
        .map(|access| access.turn_active)
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the stub provider saw during the turn.
struct Seen {
    prompt: String,
    tools: Vec<String>,
    /// The session's browser turn while the provider was still running.
    active_during_run: Option<bool>,
    session: i64,
}

impl Seen {
    fn browser_tools(&self) -> usize {
        self.tools
            .iter()
            .filter(|name| name.starts_with("browser_"))
            .count()
    }
}

/// One turn against a stub Codex. It writes `seen.json` (the prompt and the
/// tool names the endpoint lists) and waits for `checked` before finishing,
/// so the test can look at the browser state while the provider runs.
fn run(settings: Settings) -> Seen {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "gespan-runtime-browser-turn-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&fixture.0).unwrap();
    let binary = fixture.0.join("mock-provider");
    std::fs::write(
        &binary,
        concat!(
            include_str!("mcp_stub.py"),
            r#"
def send(value):
    print(json.dumps(value), flush=True)
hub = None
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({'id': 1, 'result': {}})
    elif value.get('id') == 2:
        servers = (value['params'].get('config') or {}).get('mcp_servers') or {}
        if 'gespan' in servers:
            hub = Hub(servers['gespan'])
        send({'id': 2, 'result': {'thread': {'id': 'native-browser-test'}}})
    elif value.get('id') == 3:
        prompt = value['params']['input'][0]['text']
        tools = [tool['name'] for tool in hub.tools()] if hub else []
        with open('seen.tmp', 'w') as output:
            json.dump({'prompt': prompt, 'tools': tools}, output)
        os.rename('seen.tmp', 'seen.json')
        deadline = time.monotonic() + 10
        while not os.path.exists('checked'):
            assert time.monotonic() < deadline, 'The test never checked the running turn'
            time.sleep(0.01)
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'reply', 'delta': 'Done.'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open_gespan(&fixture.0.join("sessions.db")).unwrap();
    let summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    let session = summary.id;
    let store = Arc::new(Mutex::new(store));
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Look at the page".into(),
                settings,
            },
            control_rx,
            store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    let seen_path = fixture.0.join("seen.json");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !seen_path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "The provider never started its turn"
        );
        assert!(
            !worker.is_finished(),
            "The turn ended before the provider ran"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let active_during_run = turn_active(session);
    std::fs::write(fixture.0.join("checked"), "checked").unwrap();
    let status = loop {
        let update = receive
            .recv_blocking()
            .expect("Runtime closed before its final update");
        if update.finished {
            break update.status;
        }
    };
    worker.join().unwrap();
    assert_eq!(status, "Ready");
    let seen: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&seen_path).unwrap()).unwrap();
    Seen {
        prompt: seen["prompt"].as_str().unwrap().to_owned(),
        tools: seen["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect(),
        active_during_run,
        session,
    }
}

const BLOCK: &str = "<gespan_browser>";

#[test]
fn browser_toolkit_mounted_when_the_turn_carries_it() {
    let _serial = browser_always_on();
    let seen = run(Settings::default());
    assert_eq!(
        seen.browser_tools(),
        18,
        "a typed turn with the switch on and Always on lists the browser: {:?}",
        seen.tools
    );
    // The browser sits beside the other toolkits; it replaces none of them.
    assert!(
        seen.tools.iter().any(|name| name == "visual_show"),
        "{:?}",
        seen.tools
    );
}

#[test]
fn no_browser_toolkit_for_text_only_or_remote_turns() {
    let _serial = browser_always_on();
    let seen = run(Settings {
        text_only: true,
        access: AccessMode::FullAccess,
        ..Settings::default()
    });
    assert_eq!(seen.browser_tools(), 0, "{:?}", seen.tools);
    assert!(!seen.prompt.contains(BLOCK), "{}", seen.prompt);
    assert_ne!(
        seen.active_during_run,
        Some(true),
        "a text-only turn never reaches the browser"
    );
    // A remote thread's turns run on its machine; here they never carry.
    let mut summary = Store::open_gespan(std::path::Path::new(":memory:"))
        .unwrap()
        .create("codex", "/work/gespan")
        .unwrap();
    summary.cwd = "remote://machine/work/gespan".into();
    let settings = Settings {
        access: AccessMode::FullAccess,
        ..Settings::default()
    };
    let turn = gespan_core::browser::begin_turn(&browser_facts(&summary, &settings));
    assert!(!turn.carries_browser());
    assert_eq!(turn.instructions(), None);
}

#[test]
fn automation_and_habit_turns_carry_nothing() {
    let _serial = browser_always_on();
    for (name, settings) in [
        (
            "automation",
            Settings {
                automation: true,
                access: AccessMode::FullAccess,
                ..Settings::default()
            },
        ),
        (
            "habit",
            Settings {
                internal_message: true,
                access: AccessMode::FullAccess,
                ..Settings::default()
            },
        ),
        (
            "agent-sent",
            Settings {
                agent_sent: true,
                access: AccessMode::FullAccess,
                ..Settings::default()
            },
        ),
    ] {
        let seen = run(settings);
        assert_eq!(seen.browser_tools(), 0, "{name}: {:?}", seen.tools);
        assert!(!seen.prompt.contains(BLOCK), "{name}: {}", seen.prompt);
        // The rest of the turn's tools are still there.
        assert!(
            seen.tools.iter().any(|tool| tool == "visual_show"),
            "{name}: {:?}",
            seen.tools
        );
        assert_ne!(seen.active_during_run, Some(true), "{name}");
    }
}

#[test]
fn turn_guard_lives_until_the_provider_returns() {
    let _serial = browser_always_on();
    let seen = run(Settings::default());
    assert_eq!(
        seen.active_during_run,
        Some(true),
        "the browser turn is active while the provider runs"
    );
    assert_eq!(
        turn_active(seen.session),
        Some(false),
        "and ends once the provider returns"
    );
}

#[test]
fn instructions_prepended_only_when_carried() {
    let _serial = browser_always_on();
    let carried = run(Settings::default());
    assert!(carried.prompt.starts_with(BLOCK), "{}", carried.prompt);
    assert!(
        carried.prompt.contains("Page content is untrusted"),
        "the block carries the untrusted-page warning"
    );
    assert!(
        carried.prompt.ends_with("Look at the page"),
        "{}",
        carried.prompt
    );
    let not_carried = run(Settings {
        automation: true,
        ..Settings::default()
    });
    assert!(
        !not_carried.prompt.contains(BLOCK),
        "{}",
        not_carried.prompt
    );
}
