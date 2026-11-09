//! The agent's browser tools against a real Chrome, headless, in `Managed`
//! mode with a throwaway profile: each call goes the way an agent's does,
//! `map_call` → `handle_request` → `tool_result`.
//!
//! Ignored by default; they run only where `GESPAN_TEST_CHROME` names a
//! Chrome executable (CI sets it on every OS):
//!
//! ```text
//! GESPAN_TEST_CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//!   cargo test -p gespan-browser --test real_browser -- --ignored
//! ```
//!
//! None of them touches the user's own Chrome or profile: Gespan starts its
//! own process on a temp profile, and every test stops that process, even
//! when it fails. A test that hangs past [`TEST_LIMIT`] kills its Chrome and
//! ends the test run, so CI never waits on it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use gespan_browser::live::{KeyEvent, LiveInput};
use gespan_browser::protocol::{
    AccessMode, ActionRequest, BrowserMode, BrowserSettings, ConnectionState, TabId,
};
use gespan_browser::testing::PageServer;
use gespan_browser::{agent, launch, BrowserHost};
use serde_json::{json, Value};

/// Longest a single test may take before its Chrome is killed and the run
/// ends with an error.
const TEST_LIMIT: Duration = Duration::from_secs(120);

/// Longest side of a screenshot (the crate's `MAX_SCREENSHOT_SIDE`).
const MAX_SCREENSHOT_SIDE: u32 = 8000;

/// Every non-ASCII case the agent's text must survive: accents, an emoji
/// outside the BMP, CJK, and a combining mark that NFC would fold into é.
const TEXT: &str = "héllo 😀 漢字 e\u{0301}";

fn chrome() -> Option<PathBuf> {
    match std::env::var_os("GESPAN_TEST_CHROME") {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => {
            eprintln!("skipped: set GESPAN_TEST_CHROME to a Chrome executable to run it");
            None
        }
    }
}

/// One Gespan browser for one test, with thread `s1` granted.
struct Browser {
    host: Arc<BrowserHost>,
    dir: PathBuf,
    done: Arc<AtomicBool>,
    calls: std::cell::Cell<u32>,
}

impl Browser {
    fn start(chrome: PathBuf) -> Browser {
        let dir = std::env::temp_dir().join(format!("gespan-real-{}", uuid::Uuid::new_v4()));
        let host = BrowserHost::for_tests(BrowserSettings {
            mode: BrowserMode::Managed,
            access: AccessMode::Always,
            show_window: false,
            ..BrowserSettings::default()
        });
        host.set_executable_for_tests(chrome);
        host.set_profile_dir(dir.join("browser-profile"));
        host.set_toggle(true);
        host.turn_started("s1", "main", "/repo", Some(true));
        let done = Arc::new(AtomicBool::new(false));
        watchdog(host.clone(), done.clone());
        Browser {
            host,
            dir,
            done,
            calls: std::cell::Cell::new(0),
        }
    }

    /// One tool call by thread `s1`, as the MCP hub makes it.
    fn call(&self, name: &str, arguments: Value) -> Value {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        let mapped = agent::tools::map_call(name, Some(&arguments)).expect("arguments map");
        let result = self.host.handle_request(
            "s1",
            ActionRequest {
                request_id: mapped.request_id.unwrap_or_else(|| format!("r{n}")),
                action: mapped.kind.name().to_string(),
                input: mapped.input,
            },
        );
        agent::mcp::tool_result(Some(mapped.kind), &result)
    }

    /// A call that must succeed.
    fn ok(&self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments);
        assert_eq!(result["isError"], json!(false), "{name}: {result}");
        result
    }

    /// Open `url` in a new agent tab; its id.
    fn open(&self, url: &str) -> String {
        let ids = || -> Vec<String> {
            let tabs = self.host.policy().read().tabs_for("s1");
            tabs.into_iter().map(|t| t.tab.0).collect()
        };
        let before = ids();
        let opened = self.ok("browser_tabs", json!({ "action": "open", "url": url }));
        let new: Vec<String> = ids()
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();
        assert_eq!(new.len(), 1, "{new:?}");
        assert!(text_of(&opened).contains(&new[0]), "{opened}");
        new[0].clone()
    }

    fn agent_tabs(&self) -> usize {
        self.host.policy().read().tabs_for("s1").len()
    }

    fn profile(&self) -> PathBuf {
        self.dir.join("browser-profile")
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        let pid = self.host.managed_pid_for_tests();
        self.host.set_toggle(false);
        // OFF closes the browser in the background; make sure it is gone
        // before the test process ends (a headless Chrome never exits by
        // itself).
        let profile = self.profile();
        let gone = eventually(Duration::from_secs(15), || {
            launch::running(&profile).is_none()
        });
        if let Some(pid) = pid {
            if !gone {
                eprintln!("the Gespan browser outlived OFF; killing {pid}");
            }
            kill(pid);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Kill the test's Chrome and end the run when a test hangs.
fn watchdog(host: Arc<BrowserHost>, done: Arc<AtomicBool>) {
    let started = Instant::now();
    thread::spawn(move || {
        while started.elapsed() < TEST_LIMIT {
            if done.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        eprintln!("a real-browser test ran past {TEST_LIMIT:?}; stopping its Chrome and the run");
        if let Some(pid) = host.managed_pid_for_tests() {
            kill(pid);
        }
        std::process::exit(101);
    });
}

fn kill(pid: u32) {
    let pid = pid.to_string();
    #[cfg(unix)]
    let status = std::process::Command::new("kill")
        .args(["-9", &pid])
        .output();
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid])
        .output();
    let _ = status;
}

fn eventually(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    done()
}

fn text_of(result: &Value) -> String {
    result["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `[ref=…]` on the first snapshot line that contains `needle`.
fn ref_of(snapshot: &str, needle: &str) -> String {
    let line = snapshot
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no {needle} in {snapshot}"));
    let start = line
        .find("[ref=")
        .unwrap_or_else(|| panic!("no ref on {line}"))
        + 5;
    line[start..start + line[start..].find(']').unwrap()].to_string()
}

/// `text` as code points in hex, the way the fixture page reports a value:
/// a check that cannot be fooled by normalization on the way back.
fn code_points(text: &str) -> String {
    text.chars()
        .map(|c| format!("{:x}", u32::from(c)))
        .collect::<Vec<_>>()
        .join(",")
}

/// The form page: a button that marks the page, two text fields, and a
/// line that reports both fields' values as code points.
const FORM: &str = r#"<!doctype html><meta charset="utf-8"><title>form</title>
<button onclick="document.getElementById('status').textContent='clicked'">Go</button>
<input aria-label="Name" id="name">
<input aria-label="Other" id="other">
<p id="status">idle</p>
<p id="out"></p>
<script>
const hex = v => Array.from(v).map(c => c.codePointAt(0).toString(16)).join(',');
setInterval(() => {
  document.getElementById('out').textContent =
    'name=[' + hex(document.getElementById('name').value) + '] other=[' +
    hex(document.getElementById('other').value) + ']';
}, 30);
</script>"#;

/// The todo page of the 2026-11-09 manual check: Add appends the typed
/// todo to the list synchronously, then logs an error.
const TODO: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Todo demo</title></head>
<body style="font-family:system-ui;padding:24px">
<h1>Todo demo</h1>
<input aria-label="New todo" id="t"> <button id="add">Add</button>
<ul id="list"></ul>
<p><a href="/about.html">About</a></p>
<script>
document.getElementById('add').onclick = () => {
  const v = document.getElementById('t').value;
  if (!v) { console.error('Add clicked with an empty todo'); return; }
  const li = document.createElement('li'); li.textContent = v;
  document.getElementById('list').appendChild(li);
  console.error('TypeError: cannot read properties of undefined (reading "count")');
};
</script></body></html>"#;

/// The editing page: one field, and a line that reports its selection,
/// its value and how many copy and cut events it saw. What the live pane's
/// Cmd shortcuts did is read from there, not from the clipboard.
const EDIT: &str = r#"<!doctype html><meta charset="utf-8"><title>edit</title>
<input aria-label="Field" id="f">
<p id="out"></p>
<script>
const f = document.getElementById('f');
let copies = 0, cuts = 0;
document.addEventListener('copy', () => copies++);
document.addEventListener('cut', () => cuts++);
setInterval(() => {
  document.getElementById('out').textContent = 'sel=[' + f.selectionStart + ',' +
    f.selectionEnd + '] value=[' + f.value + '] copies=' + copies + ' cuts=' + cuts;
}, 30);
</script>"#;

fn pages() -> PageServer {
    PageServer::start(|path| match path {
        "/form" => FORM.to_string(),
        "/edit" => EDIT.to_string(),
        "/todo" => TODO.to_string(),
        "/pop" => "<!doctype html><title>pop</title>\
                   <button onclick=\"window.open('/popped')\">Pop</button>"
            .to_string(),
        "/popped" => "<!doctype html><title>popped</title><p>popped</p>".to_string(),
        "/tall" => "<!doctype html><title>tall</title><body style=\"margin:0\">\
                    <div style=\"height:30000px;background:linear-gradient(red,blue)\"></div>"
            .to_string(),
        "/console" => "<!doctype html><title>console</title>\
                       <script>console.error(\"boom\")</script><p>logged</p>"
            .to_string(),
        _ => "<!doctype html><title>plain</title><p>plain page</p>".to_string(),
    })
}

/// Snapshot, click, then type and fill text the agent sends as typed:
/// both tools must deliver it to the page code point for code point.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn open_snapshot_click_type() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    let tab = browser.open(&pages.url("/form"));

    let snapshot = text_of(&browser.ok("browser_snapshot", json!({ "tab": tab })));
    let go = ref_of(&snapshot, "button \"Go\"");
    let name = ref_of(&snapshot, "\"Name\"");
    let other = ref_of(&snapshot, "\"Other\"");

    browser.ok(
        "browser_click",
        json!({ "tab": tab, "ref": go, "snapshot": false }),
    );
    browser.ok(
        "browser_wait",
        json!({ "tab": tab, "for": "text", "value": "clicked", "snapshot": false }),
    );

    browser.ok(
        "browser_type",
        json!({ "tab": tab, "ref": name, "text": TEXT, "snapshot": false }),
    );
    browser.ok(
        "browser_fill",
        json!({ "tab": tab, "fields": [{ "ref": other, "text": TEXT }], "snapshot": false }),
    );

    let want = format!("name=[{0}] other=[{0}]", code_points(TEXT));
    let mut last = String::new();
    let arrived = eventually(Duration::from_secs(5), || {
        let read = browser.call("browser_read", json!({ "tab": tab }));
        last = text_of(&read);
        last.contains(&want)
    });
    assert!(arrived, "want {want} in\n{last}");
    assert!(last.contains("clicked"), "{last}");
}

/// A page's `window.open` is the agent's second tab.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn window_open_popup_becomes_an_agent_tab() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    let tab = browser.open(&pages.url("/pop"));
    let snapshot = text_of(&browser.ok("browser_snapshot", json!({ "tab": tab })));
    let pop = ref_of(&snapshot, "button \"Pop\"");
    browser.ok(
        "browser_click",
        json!({ "tab": tab, "ref": pop, "snapshot": false }),
    );
    assert!(
        eventually(Duration::from_secs(10), || browser.agent_tabs() == 2),
        "the popup is not an agent tab: {:?}",
        browser.host.policy().read().tabs_for("s1")
    );
    let listed = browser.ok("browser_tabs", json!({ "action": "list" }));
    let text = text_of(&listed);
    for tab in browser.host.policy().read().tabs_for("s1") {
        assert!(
            text.contains(&tab.tab.0),
            "{} missing from {text}",
            tab.tab.0
        );
    }
    assert!(text.contains("/popped"), "{text}");
}

/// A 30,000 px page comes back as a full-page PNG scaled to fit
/// [`MAX_SCREENSHOT_SIDE`], not as an error.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn tall_page_screenshot_is_capped() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    let tab = browser.open(&pages.url("/tall"));
    let shot = browser.ok(
        "browser_screenshot",
        json!({ "tab": tab, "full_page": true }),
    );
    let image = shot["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap_or_else(|| panic!("no image in {shot}"));
    assert_eq!(image["mimeType"], "image/png", "{}", text_of(&shot));
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert!(
        width <= MAX_SCREENSHOT_SIDE && height <= MAX_SCREENSHOT_SIDE,
        "{width}x{height}"
    );
    // The whole page, scaled down: not the viewport, not a crop.
    assert!(height > 7000 && height > width * 3, "{width}x{height}");
}

/// What the page logs with `console.error` reaches `browser_console`.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn console_error_is_reported() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    let tab = browser.open(&pages.url("/console"));
    let mut last = String::new();
    let seen = eventually(Duration::from_secs(5), || {
        last = text_of(&browser.ok("browser_console", json!({ "tab": tab })));
        last.contains("boom")
    });
    assert!(seen, "{last}");
}

/// The Gespan browser dies under the agent: the next call starts a new one
/// and succeeds.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn browser_crash_then_reconnect() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    browser.open(&pages.url("/plain"));
    let pid = browser
        .host
        .managed_pid_for_tests()
        .expect("a started browser");
    kill(pid);
    assert!(
        eventually(Duration::from_secs(10), || {
            browser.host.connection().state != ConnectionState::Connected
        }),
        "the host never noticed the browser was gone"
    );
    // The very next call: a new browser, a new tab, a page that loads.
    let tab = browser.open(&pages.url("/plain"));
    let restarted = browser.host.managed_pid_for_tests().expect("a new browser");
    assert_ne!(restarted, pid);
    let read = text_of(&browser.ok("browser_read", json!({ "tab": tab })));
    assert!(read.contains("plain page"), "{read}");
}

/// The 2026-11-09 check: the page view a click returns shows the todo its
/// handler added, though the agent reads interactive snapshots (which
/// leave text out) and the page is too small for a list of changes.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn the_page_after_a_click_shows_what_it_added() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    let tab = browser.open(&pages.url("/todo"));
    let page = text_of(&browser.ok("browser_snapshot", json!({ "tab": tab })));
    let field = ref_of(&page, "textbox \"New todo\"");
    let add = ref_of(&page, "button \"Add\"");

    let typed = text_of(&browser.ok(
        "browser_type",
        json!({ "tab": tab, "ref": field, "text": "Milk" }),
    ));
    assert!(typed.contains("\"New todo\" [focused] ["), "{typed}");
    // The field's text is its value, not a todo.
    assert!(!typed.contains("text \"Milk\""), "{typed}");

    let clicked = text_of(&browser.ok("browser_click", json!({ "tab": tab, "ref": add })));
    let item = clicked
        .lines()
        .find(|l| l.trim_start().starts_with("- text \"Milk\" [ref="))
        .unwrap_or_else(|| panic!("no Milk item after the click:\n{clicked}"));
    assert!(clicked.contains("because text on it changed"), "{clicked}");
    // The item's ref is usable.
    let item = &item[item.find("[ref=").unwrap() + 5..item.rfind(']').unwrap()];
    browser.ok(
        "browser_hover",
        json!({ "tab": tab, "ref": item, "snapshot": false }),
    );
}

/// The live pane's Cmd+A, C, X, Z and Shift+Cmd+Z, sent as the pane sends
/// them on macOS (Meta plus the edit's command name): each one edits the
/// focused field. Without the command Chromium on macOS ignores them.
#[test]
#[ignore = "starts the Chrome GESPAN_TEST_CHROME names"]
fn the_panes_editing_shortcuts_edit_the_page() {
    let Some(chrome) = chrome() else { return };
    let pages = pages();
    let browser = Browser::start(chrome);
    let tab = browser.open(&pages.url("/edit"));
    let page = text_of(&browser.ok("browser_snapshot", json!({ "tab": tab })));
    let field = ref_of(&page, "textbox \"Field\"");
    browser.ok(
        "browser_click",
        json!({ "tab": tab, "ref": field, "snapshot": false }),
    );
    let id = TabId(tab.clone());
    let send = |input: LiveInput| browser.host.live_input("s1", &id, &input).unwrap();
    send(LiveInput::Text {
        text: "hello".into(),
    });
    let wait = |want: &str| {
        let mut last = String::new();
        let seen = eventually(Duration::from_secs(5), || {
            last = text_of(&browser.call("browser_read", json!({ "tab": tab })));
            last.contains(want)
        });
        assert!(seen, "want {want} in\n{last}");
    };
    // Typed: the caret at the end, nothing selected.
    wait("sel=[5,5] value=[hello] copies=0 cuts=0");
    // Cmd (Meta, 4) and Shift (8), down then up, the edit with the down.
    let shortcut = |key: &str, shift: bool, command: &str| {
        let upper = key.to_ascii_uppercase();
        for event in [KeyEvent::RawKeyDown, KeyEvent::KeyUp] {
            send(LiveInput::Key {
                event,
                key: key.into(),
                code: format!("Key{upper}"),
                text: None,
                key_code: u32::from(upper.as_bytes()[0]),
                modifiers: 4 | if shift { 8 } else { 0 },
                commands: vec![command.into()],
            });
        }
    };
    shortcut("a", false, "selectAll");
    wait("sel=[0,5] value=[hello] copies=0 cuts=0");
    shortcut("c", false, "copy");
    wait("sel=[0,5] value=[hello] copies=1 cuts=0");
    shortcut("x", false, "cut");
    wait("sel=[0,0] value=[] copies=1 cuts=1");
    shortcut("z", false, "undo");
    wait("value=[hello] copies=1 cuts=1");
    shortcut("z", true, "redo");
    wait("sel=[0,0] value=[] copies=1 cuts=1");
}
