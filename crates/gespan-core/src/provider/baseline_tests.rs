//! The turn's workspace baseline records while the provider starts: the
//! process is launched and handshaking before the snapshot is done, and no
//! prompt reaches it until the snapshot is.
use super::*;
use crate::checkpoint::Baseline;
use std::os::unix::fs::PermissionsExt;

/// Appends its pid to `pids` in its folder, then logs every stdin line to
/// `stdin.log`. The body answers each parsed frame `v`.
const PRELUDE: &str = r#"#!/usr/bin/env python3
import json, os, sys
with open('pids', 'a') as pids:
    pids.write(f'{os.getpid()}\n')
def send(v):
    print(json.dumps(v), flush=True)
for line in sys.stdin:
    with open('stdin.log', 'a') as log:
        log.write(line)
    v = json.loads(line)
"#;

const CLAUDE: &str = r#"
    if v.get('type') == 'control_request' and v['request'].get('subtype') == 'initialize':
        send({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': v['request_id']}})
    elif v.get('type') == 'user':
        send({'type': 'system', 'subtype': 'init', 'session_id': 'claude-session'})
        send({'type': 'assistant', 'message': {'id': 'm1', 'role': 'assistant', 'content': [{'type': 'text', 'text': 'ok'}]}})
        send({'type': 'result', 'subtype': 'success', 'result': 'ok', 'session_id': 'claude-session'})
"#;

const ACP: &str = r#"
    if 'id' not in v or 'method' not in v:
        continue
    if v['method'] == 'initialize':
        send({'jsonrpc': '2.0', 'id': v['id'], 'result': {'protocolVersion': 1, 'agentCapabilities': {}}})
    elif v['method'] == 'session/new':
        send({'jsonrpc': '2.0', 'id': v['id'], 'result': {'sessionId': 'kiro-session'}})
    elif v['method'] == 'session/prompt':
        send({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 'kiro-session', 'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'ok'}}}})
        send({'jsonrpc': '2.0', 'id': v['id'], 'result': {'stopReason': 'end_turn'}})
    else:
        send({'jsonrpc': '2.0', 'id': v['id'], 'result': {}})
"#;

fn agent(kind: Kind, body: &str) -> (tempfile::TempDir, Request) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider");
    std::fs::write(&path, format!("{PRELUDE}{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let request = Request {
        kind,
        binary: path,
        cwd: tmp.path().into(),
        prompt: "the user's message".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    };
    (tmp, request)
}

/// A baseline that stays unrecorded until the returned sender fires.
fn pending_baseline() -> (Baseline, mpsc::Sender<()>) {
    let (release, released) = mpsc::channel::<()>();
    let baseline = Baseline::record(move || {
        let _ = released.recv();
    });
    (baseline, release)
}

fn stdin_log(tmp: &tempfile::TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("stdin.log")).unwrap_or_default()
}

/// Waits up to ten seconds for `ready`.
fn eventually(mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

type Turn = thread::JoinHandle<(Result<(), String>, String)>;

/// Runs the turn on its own thread; the result carries the streamed text.
fn start(request: Request, cancel: Arc<AtomicBool>) -> Turn {
    thread::spawn(move || {
        let mut text = String::new();
        let result = run(request, cancel, |events| {
            for event in events {
                if let Event::Text(chunk) = event {
                    text.push_str(&chunk);
                }
            }
            Ok(())
        });
        (result, text)
    })
}

/// The provider starts and handshakes while the baseline records; the
/// prompt goes out only once it is recorded.
fn assert_prompt_waits_for_baseline(kind: Kind, body: &str, prompt_frame: &str) {
    let (tmp, mut request) = agent(kind, body);
    let (baseline, release) = pending_baseline();
    request.settings.baseline = Some(baseline.clone());
    let turn = start(request, Arc::new(AtomicBool::new(false)));

    assert!(
        eventually(|| tmp.path().join("pids").exists()),
        "{}: the provider did not start while the baseline was recording",
        kind.label()
    );
    assert!(
        eventually(|| stdin_log(&tmp).contains("initialize")),
        "{}: the handshake waited for the baseline",
        kind.label()
    );
    // Room for a prompt that skipped the gate to arrive.
    thread::sleep(Duration::from_millis(300));
    assert!(!baseline.recorded());
    let log = stdin_log(&tmp);
    assert!(
        !log.contains(prompt_frame) && !log.contains("the user's message"),
        "{}: the prompt went out before the baseline was recorded:\n{log}",
        kind.label()
    );

    release.send(()).unwrap();
    let (result, text) = turn.join().unwrap();
    assert_eq!(result, Ok(()), "{}", kind.label());
    assert_eq!(text, "ok", "{}", kind.label());
    let log = stdin_log(&tmp);
    assert!(
        log.contains("the user's message"),
        "{}:\n{log}",
        kind.label()
    );
}

#[test]
fn claude_starts_before_the_baseline_and_prompts_after_it() {
    assert_prompt_waits_for_baseline(Kind::Claude, CLAUDE, "\"type\":\"user\"");
}

#[test]
fn kiro_starts_before_the_baseline_and_prompts_after_it() {
    assert_prompt_waits_for_baseline(Kind::Kiro, ACP, "session/prompt");
}

#[test]
fn a_kept_process_gets_no_prompt_before_the_baseline() {
    let (tmp, mut request) = agent(Kind::Claude, CLAUDE);
    request.provider_id = Some("claude-session".into());
    // Keep a started Claude for this conversation, as an earlier turn would.
    // Held agents are taken back even with the warm pool off.
    let mut command = Command::new(&request.binary);
    command
        .current_dir(&request.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    let wire = parked::Wire::spawn(&mut command, &request, false).unwrap();
    let kept = wire.child.id();
    parked::put_held(wire, Protocol::new(request.clone()));
    assert!(eventually(|| tmp.path().join("pids").exists()));

    let (baseline, release) = pending_baseline();
    request.settings.baseline = Some(baseline);
    let turn = start(request, Arc::new(AtomicBool::new(false)));
    // A kept process gets the prompt as its first frame, with no handshake
    // to wait for: allow a loaded machine time to show a prompt that skipped
    // the gate.
    thread::sleep(Duration::from_secs(1));
    let log = stdin_log(&tmp);
    assert!(
        !log.contains("the user's message"),
        "the kept process got the prompt before the baseline:\n{log}"
    );

    release.send(()).unwrap();
    let (result, text) = turn.join().unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(text, "ok");
    assert!(stdin_log(&tmp).contains("the user's message"));
    // The kept process answered; no new one was started.
    let pids = std::fs::read_to_string(tmp.path().join("pids")).unwrap();
    assert_eq!(pids.lines().collect::<Vec<_>>(), [kept.to_string()]);
}

#[test]
fn stop_while_the_baseline_records_ends_the_turn_without_a_prompt() {
    let (tmp, mut request) = agent(Kind::Claude, CLAUDE);
    let (baseline, release) = pending_baseline();
    request.settings.baseline = Some(baseline.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    let turn = start(request, cancel.clone());
    assert!(eventually(|| stdin_log(&tmp).contains("initialize")));
    cancel.store(true, Ordering::Relaxed);
    let (result, _) = turn.join().unwrap();
    assert_eq!(result, Err("Stopped".into()));
    assert!(!baseline.recorded(), "Stop does not wait for the snapshot");
    assert!(!stdin_log(&tmp).contains("the user's message"));
    release.send(()).unwrap();
    baseline.finish();
}
