//! Hold against scripted Codex and Claude processes: a running command always
//! finishes, approvals stay pending, and background work is left alone.
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

const READER: &str = r#"#!/usr/bin/env python3
import json, sys, subprocess, threading, queue, os
MODE = open('mode').read().strip()
q = queue.Queue()
def reader():
    for line in sys.stdin:
        with open('stdin.log', 'a') as log:
            log.write(line)
        q.put(json.loads(line))
    q.put(None)
threading.Thread(target=reader, daemon=True).start()
def send(v):
    print(json.dumps(v), flush=True)
def get(timeout=None):
    try:
        v = q.get(timeout=timeout)
    except queue.Empty:
        return 'timeout'
    if v is None:
        sys.exit(0)
    return v
"#;

const CODEX: &str = r#"
interrupted = False
def handle(v):
    global interrupted
    if v == 'timeout':
        return
    if v.get('method') == 'turn/interrupt':
        send({'id': v['id'], 'result': {}})
        send({'method': 'turn/completed', 'params': {'turn': {'id': 'turn1', 'status': 'interrupted'}}})
        interrupted = True
    elif v.get('id') == 'permission-1':
        open('answered', 'w').write(json.dumps(v))
while True:
    v = get()
    if v.get('id') == 1:
        send({'id': 1, 'result': {}})
    elif v.get('id') == 2:
        send({'id': 2, 'result': {'thread': {'id': 'thread1'}}})
    elif v.get('id') == 3:
        if MODE == 'slowstart':
            import time
            time.sleep(0.3)
        send({'id': 3, 'result': {'turn': {'id': 'turn1'}}})
        break
if MODE == 'slowstart':
    # The turn has not started yet; Hold must wait for it instead of dropping the prompt.
    pass
if MODE == 'tool':
    child = subprocess.Popen(['sleep', '30'])
    open('child.pid', 'w').write(str(child.pid))
    send({'method': 'item/started', 'params': {'item': {'id': 'c1', 'type': 'commandExecution', 'command': 'sleep 1', 'status': 'inProgress'}}})
    subprocess.run('sleep 1 && echo done > out', shell=True)
    send({'method': 'item/completed', 'params': {'item': {'id': 'c1', 'type': 'commandExecution', 'command': 'sleep 1', 'status': 'completed', 'aggregatedOutput': 'done'}}})
if MODE == 'approval':
    send({'id': 'permission-1', 'method': 'item/commandExecution/requestApproval', 'params': {'command': 'cargo build', 'itemId': 'c1'}})
    while True:
        handle(get())
while not interrupted:
    send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'a', 'delta': 'x'}})
    handle(get(0.05))
while True:
    handle(get())
"#;

const CLAUDE: &str = r#"
interrupted = False
def handle(v):
    global interrupted
    if v == 'timeout':
        return
    if v.get('type') == 'control_request' and v['request'].get('subtype') == 'interrupt':
        send({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': v['request_id']}})
        send({'type': 'result', 'subtype': 'error_during_execution', 'result': ''})
        interrupted = True
while True:
    v = get()
    if v.get('type') == 'control_request' and v['request'].get('subtype') == 'initialize':
        send({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': v['request_id']}})
    elif v.get('type') == 'user':
        break
send({'type': 'system', 'subtype': 'init', 'session_id': 'claude-session'})
if MODE == 'tool':
    send({'type': 'assistant', 'message': {'id': 'm1', 'role': 'assistant', 'content': [{'type': 'tool_use', 'id': 't1', 'name': 'Bash', 'input': {'command': 'sleep 1'}}]}})
    subprocess.run('sleep 1 && echo done > out', shell=True)
    send({'type': 'user', 'message': {'role': 'user', 'content': [{'type': 'tool_result', 'tool_use_id': 't1', 'content': 'done'}]}})
if MODE == 'background':
    send({'type': 'system', 'subtype': 'task_started', 'task_id': 'b1', 'tool_use_id': 't0', 'description': 'dev server', 'is_backgrounded': True})
    send({'type': 'result', 'subtype': 'success', 'result': 'Started the dev server.'})
    while True:
        handle(get())
n = 0
while not interrupted:
    n += 1
    send({'type': 'assistant', 'message': {'id': 'm2', 'role': 'assistant', 'content': [{'type': 'text', 'text': 'x' * n}]}})
    handle(get(0.05))
while True:
    handle(get())
"#;

fn agent(kind: Kind, mode: &str) -> (tempfile::TempDir, Request) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider");
    let body = if kind == Kind::Claude { CLAUDE } else { CODEX };
    std::fs::write(&path, format!("{READER}{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(tmp.path().join("mode"), mode).unwrap();
    let request = Request {
        kind,
        binary: path,
        cwd: tmp.path().into(),
        prompt: "hello".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    };
    (tmp, request)
}

/// Runs a turn and sends Hold the first time `when` matches an event.
fn run_held(
    request: Request,
    cancel: Arc<AtomicBool>,
    when: impl Fn(&Event) -> bool,
    mut seen: impl FnMut(&Event),
) -> (Result<(), String>, Vec<Event>) {
    let (tx, rx) = mpsc::channel();
    let mut sent = false;
    let mut events = Vec::new();
    let result = run_with_controls(request, cancel, Some(rx), |batch| {
        for event in batch {
            if !sent && when(&event) {
                sent = true;
                tx.send(Control {
                    id: String::new(),
                    decision: Decision::Hold,
                })
                .unwrap();
            }
            seen(&event);
            events.push(event);
        }
        Ok(())
    });
    (result, events)
}

fn stdin_log(tmp: &tempfile::TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("stdin.log")).unwrap_or_default()
}

fn running_tool(event: &Event) -> bool {
    matches!(event, Event::Tool(tool) if tool.state == ToolState::Running)
}

#[test]
fn hold_waits_for_running_tool_then_interrupts() {
    for kind in [Kind::Codex, Kind::Claude] {
        let (tmp, request) = agent(kind, "tool");
        let (result, events) = run_held(
            request,
            Arc::new(AtomicBool::new(false)),
            running_tool,
            |_| {},
        );
        assert_eq!(result, Err("Held".into()), "{}", kind.label());
        assert!(
            tmp.path().join("out").exists(),
            "{}: the running command must finish",
            kind.label()
        );
        let completed = events
            .iter()
            .position(|e| matches!(e, Event::Tool(t) if t.state != ToolState::Running))
            .unwrap_or_else(|| panic!("{}: no completed tool", kind.label()));
        let settled = events
            .iter()
            .position(|e| *e == Event::HoldSettled(HoldPoint::Interrupted))
            .unwrap_or_else(|| panic!("{}: never settled", kind.label()));
        assert!(completed < settled, "{}", kind.label());
        let log = stdin_log(&tmp);
        assert!(log.contains("interrupt"), "{}: {log}", kind.label());
        assert!(!log.contains("stop_task"), "{}: {log}", kind.label());
        assert!(!log.contains("\"deny\""), "{}: {log}", kind.label());
        assert!(!log.contains("decline"), "{}: {log}", kind.label());
    }
}

#[test]
fn hold_interrupts_streaming_immediately() {
    for kind in [Kind::Codex, Kind::Claude] {
        let (_tmp, request) = agent(kind, "stream");
        let held_at = Mutex::new(None);
        let (result, _) = run_held(
            request,
            Arc::new(AtomicBool::new(false)),
            |e| matches!(e, Event::Text(_)),
            |e| {
                if matches!(e, Event::Text(_)) && held_at.lock().unwrap().is_none() {
                    *held_at.lock().unwrap() = Some(Instant::now());
                }
            },
        );
        assert_eq!(result, Err("Held".into()), "{}", kind.label());
        let elapsed = held_at.lock().unwrap().unwrap().elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "{}: {elapsed:?}",
            kind.label()
        );
    }
}

/// Waits until `flag` is set, gives the loop `ms` to misbehave, records what
/// the agent saw, then stops the turn so the test can finish.
fn watch(
    tmp: &tempfile::TempDir,
    flag: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    ms: u64,
) -> thread::JoinHandle<(bool, String)> {
    let dir = tmp.path().to_path_buf();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !flag.load(Ordering::Relaxed) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(ms));
        let answered = dir.join("answered").exists();
        let log = std::fs::read_to_string(dir.join("stdin.log")).unwrap_or_default();
        cancel.store(true, Ordering::Relaxed);
        (answered, log)
    })
}

#[test]
fn hold_leaves_pending_approval_pending() {
    let (tmp, request) = agent(Kind::Codex, "approval");
    let cancel = Arc::new(AtomicBool::new(false));
    let settled = Arc::new(AtomicBool::new(false));
    let watcher = watch(&tmp, settled.clone(), cancel.clone(), 600);
    let (result, events) = run_held(
        request,
        cancel,
        |e| matches!(e, Event::Approval(_)),
        |e| {
            if *e == Event::HoldSettled(HoldPoint::AwaitingInput) {
                settled.store(true, Ordering::Relaxed);
            }
        },
    );
    let (answered, log) = watcher.join().unwrap();
    assert!(events.contains(&Event::HoldSettled(HoldPoint::AwaitingInput)));
    assert!(!answered, "the approval must stay pending while held");
    assert!(!log.contains("interrupt"), "{log}");
    // Only the test's own Stop ended the turn.
    assert_eq!(result, Err("Stopped".into()));
}

#[test]
fn hold_does_not_stop_background_tasks() {
    let (tmp, request) = agent(Kind::Claude, "background");
    let cancel = Arc::new(AtomicBool::new(false));
    let settled = Arc::new(AtomicBool::new(false));
    let watcher = watch(&tmp, settled.clone(), cancel.clone(), 600);
    let (_, events) = run_held(
        request,
        cancel,
        |e| matches!(e, Event::Background(tasks) if !tasks.is_empty()),
        |e| {
            if *e == Event::HoldSettled(HoldPoint::Background) {
                settled.store(true, Ordering::Relaxed);
            }
        },
    );
    let (_, log) = watcher.join().unwrap();
    assert!(events.contains(&Event::HoldSettled(HoldPoint::Background)));
    let before_stop = log
        .lines()
        .filter(|l| l.contains("stop_task") || l.contains("interrupt"));
    assert_eq!(before_stop.count(), 0, "{log}");
}

#[test]
fn held_process_is_parked_not_killed() {
    let (tmp, request) = agent(Kind::Codex, "tool");
    // The app keeps finished agents warm; tests run with the pool off by default.
    parked::configure(1);
    let (result, _) = run_held(
        request,
        Arc::new(AtomicBool::new(false)),
        running_tool,
        |_| {},
    );
    parked::configure(0);
    assert_eq!(result, Err("Held".into()));
    let pid: i32 = std::fs::read_to_string(tmp.path().join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    assert!(alive, "Hold must not kill the agent's process group");
}

#[test]
fn hold_before_the_turn_starts_waits_for_the_prompt_to_be_delivered() {
    let (tmp, request) = agent(Kind::Codex, "slowstart");
    let (tx, rx) = mpsc::channel();
    tx.send(Control {
        id: String::new(),
        decision: Decision::Hold,
    })
    .unwrap();
    let result = run_with_controls(request, Arc::new(AtomicBool::new(false)), Some(rx), |_| {
        Ok(())
    });
    assert_eq!(result, Err("Held".into()));
    let log = stdin_log(&tmp);
    let start = log
        .find("turn/start")
        .expect("the prompt must reach the agent");
    let interrupt = log
        .find("turn/interrupt")
        .expect("then the turn is interrupted");
    assert!(start < interrupt, "{log}");
}
