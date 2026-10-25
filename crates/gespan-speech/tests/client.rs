use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gespan_speech::protocol::{ErrorKind, Event, Request};
use gespan_speech::{RestartPolicy, SpeechClient};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-speech-helper");
const WAIT: Duration = Duration::from_secs(5);

fn fast_policy() -> RestartPolicy {
    RestartPolicy {
        delays: vec![Duration::from_millis(10); 3],
        window: Duration::from_secs(60),
        max_restarts: 3,
    }
}

fn spawn(mode: &str, log: Option<&PathBuf>) -> (SpeechClient, Receiver<Event>) {
    let mut envs = vec![("FAKE_SPEECH_MODE".to_string(), mode.to_string())];
    if let Some(log) = log {
        envs.push(("FAKE_SPEECH_LOG".to_string(), log.display().to_string()));
    }
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    let client = SpeechClient::spawn_with(
        PathBuf::from(FAKE),
        Vec::new(),
        envs,
        fast_policy(),
        move |event| {
            let _ = tx.lock().unwrap().send(event);
        },
    );
    (client, rx)
}

fn next(rx: &Receiver<Event>) -> Event {
    rx.recv_timeout(WAIT).expect("event within timeout")
}

fn load() -> Request {
    Request::Load {
        model_dir: PathBuf::from("/models/parakeet"),
        model_id: "parakeet-tdt-0.6b".into(),
    }
}

fn pid_count(path: &PathBuf) -> usize {
    log_lines(path)
        .iter()
        .filter(|l| l.starts_with("pid "))
        .count()
}

/// Whether a process with this pid exists, on unix and windows.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .any(|word| word == pid.to_string())
}

fn log_lines(path: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

#[test]
fn forwards_events_in_order() {
    let (client, rx) = spawn("echo", None);
    // Sent before the helper reports Ready, so these are queued in order.
    client.send(load());
    client.send(Request::Start { terms: vec![] });
    client.send(Request::Stop);
    assert_eq!(next(&rx), Event::Loaded);
    assert_eq!(next(&rx), Event::Listening);
    assert_eq!(
        next(&rx),
        Event::Final {
            text: "ok".into(),
            capped: false
        }
    );
    client.send(Request::Start { terms: vec![] });
    client.send(Request::Cancel);
    assert_eq!(next(&rx), Event::Listening);
    assert_eq!(next(&rx), Event::Cancelled);
    client.shutdown();
}

#[test]
fn restarts_after_crash_and_reloads_model() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("fake.log");
    let (client, rx) = spawn("crash-after-start", Some(&log));
    client.send(load());
    assert_eq!(next(&rx), Event::Loaded);
    client.send(Request::Start { terms: vec![] });
    assert_eq!(next(&rx), Event::Listening);
    match next(&rx) {
        Event::Error { kind, message } => {
            assert_eq!(kind, ErrorKind::Engine);
            assert_eq!(message, "Dictation stopped unexpectedly.");
        }
        other => panic!("expected crash error, got {other:?}"),
    }
    // The second process gets the same Load again, without the caller asking.
    assert_eq!(next(&rx), Event::Loaded);
    let lines = log_lines(&log);
    let pids: Vec<_> = lines.iter().filter(|l| l.starts_with("pid ")).collect();
    assert_eq!(pids.len(), 2, "two processes started: {lines:?}");
    assert_ne!(pids[0], pids[1]);
    assert_eq!(lines.iter().filter(|l| *l == "load").count(), 2);
    client.shutdown();
}

#[test]
fn restart_reloads_model_before_queued_requests() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("fake.log");
    let (client, rx) = spawn("crash-after-start", Some(&log));
    client.send(load());
    assert_eq!(next(&rx), Event::Loaded);
    client.send(Request::Start { terms: vec![] });
    assert_eq!(next(&rx), Event::Listening);
    assert!(matches!(next(&rx), Event::Error { .. }));
    // Queued while the helper is restarting.
    client.send(Request::Cancel);
    assert_eq!(next(&rx), Event::Loaded);
    assert_eq!(next(&rx), Event::Cancelled);
    client.shutdown();
}

#[test]
fn gives_up_after_three_crashes_in_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("fake.log");
    let (client, rx) = spawn("crash-after-start", Some(&log));
    let deadline = Instant::now() + WAIT;
    let mut unavailable = 0;
    while unavailable == 0 {
        assert!(Instant::now() < deadline, "never gave up");
        client.send(Request::Start { terms: vec![] });
        if let Ok(Event::Error { message, .. }) = rx.recv_timeout(Duration::from_millis(20)) {
            if message == "Dictation is unavailable. Restart Gespan to try again." {
                unavailable += 1;
            } else {
                assert_eq!(message, "Dictation stopped unexpectedly.");
            }
        }
    }
    // The initial run plus exactly three restarts.
    assert_eq!(pid_count(&log), 4);
    // After giving up, send is a no-op: nothing spawns and nothing is emitted,
    // and the give-up error never repeats.
    client.send(Request::Start { terms: vec![] });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(pid_count(&log), 4);
    while let Ok(event) = rx.try_recv() {
        if let Event::Error { message, .. } = event {
            assert_eq!(message, "Dictation stopped unexpectedly.");
        }
    }
    client.shutdown();
}

#[test]
fn crash_before_listening_reports_error() {
    let (client, rx) = spawn("crash-on-start", None);
    client.send(load());
    assert_eq!(next(&rx), Event::Loaded);
    client.send(Request::Start { terms: vec![] });
    // The helper dies on Start without ever saying Listening; the app must
    // still hear about it instead of waiting forever.
    assert_eq!(
        next(&rx),
        Event::Error {
            kind: ErrorKind::Engine,
            message: "Dictation stopped unexpectedly.".into()
        }
    );
    client.shutdown();
}

#[test]
fn on_event_can_send() {
    let slot: Arc<Mutex<Option<Arc<SpeechClient>>>> = Arc::new(Mutex::new(None));
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    let cb_slot = slot.clone();
    let client = Arc::new(SpeechClient::spawn_with(
        PathBuf::from(FAKE),
        Vec::new(),
        vec![("FAKE_SPEECH_MODE".into(), "echo".into())],
        fast_policy(),
        move |event| {
            if event == Event::Listening {
                if let Some(c) = cb_slot.lock().unwrap().as_ref() {
                    c.send(Request::Stop);
                }
            }
            let _ = tx.lock().unwrap().send(event);
        },
    ));
    *slot.lock().unwrap() = Some(client.clone());
    client.send(Request::Start { terms: vec![] });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Event::Listening
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Event::Final {
            text: "ok".into(),
            capped: false
        }
    );
    // Release the callback's reference so the client can be shut down.
    slot.lock().unwrap().take();
    Arc::try_unwrap(client).ok().expect("sole owner").shutdown();
}

#[test]
fn crash_while_listening_reports_error() {
    let (client, rx) = spawn("crash-after-start", None);
    client.send(Request::Start { terms: vec![] });
    assert_eq!(next(&rx), Event::Listening);
    assert_eq!(
        next(&rx),
        Event::Error {
            kind: ErrorKind::Engine,
            message: "Dictation stopped unexpectedly.".into()
        }
    );
    client.shutdown();
}

#[test]
fn shutdown_kills_hung_helper_within_3s() {
    let (client, _rx) = spawn("hang", None);
    client.send(Request::Start { terms: vec![] });
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    client.shutdown();
    let took = started.elapsed();
    assert!(took >= Duration::from_secs(1), "waited for graceful exit");
    assert!(took < Duration::from_secs(3), "took {took:?}");
}

#[test]
fn shutdown_is_prompt_when_helper_exits() {
    let (client, rx) = spawn("echo", None);
    client.send(load());
    assert_eq!(next(&rx), Event::Loaded);
    let started = Instant::now();
    client.shutdown();
    let took = started.elapsed();
    assert!(took < Duration::from_millis(1500), "took {took:?}");
    // A clean exit after Shutdown is not a crash.
    assert!(rx.try_recv().is_err());
}

#[test]
fn drop_without_shutdown_kills_helper() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("fake.log");
    let (client, _rx) = spawn("hang", Some(&log));
    client.send(Request::Stop);
    // Wait until the process is up.
    let deadline = Instant::now() + WAIT;
    while log_lines(&log).is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid: u32 = log_lines(&log)[0]["pid ".len()..].parse().unwrap();
    assert!(process_alive(pid), "helper should be running before drop");
    drop(client);
    assert!(!process_alive(pid), "helper {pid} still running after drop");
}
