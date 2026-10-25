use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gespan_speech::protocol::{read_line, write_line, ErrorKind, Event, Request};
use gespan_speech::{
    run_helper_with, AudioSource, CaptureError, EngineError, HelperLimits, SpeechEngine,
};

/// Blocking reader over a channel; end of file when the sender is dropped.
struct ChanReader {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.buf.is_empty() {
            match self.rx.recv() {
                Ok(bytes) => self.buf = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len());
        out[..n].copy_from_slice(&self.buf[..n]);
        self.buf.drain(..n);
        Ok(n)
    }
}

#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Counts how many captures are open, so tests can see the mic is released.
#[derive(Clone, Default)]
struct MicState {
    open: Arc<AtomicUsize>,
    starts: Arc<AtomicUsize>,
}

struct FakeSource {
    chunks: Vec<Vec<f32>>,
    gap: Duration,
    fail: Option<CaptureError>,
    state: MicState,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeSource {
    fn new(chunks: Vec<Vec<f32>>, gap: Duration) -> (Self, MicState) {
        let state = MicState::default();
        let source = Self {
            chunks,
            gap,
            fail: None,
            state: state.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        (source, state)
    }

    fn failing(err: CaptureError) -> Self {
        let (mut source, _) = Self::new(vec![], Duration::ZERO);
        source.fail = Some(err);
        source
    }
}

impl AudioSource for FakeSource {
    fn start(&mut self, mut sink: Box<dyn FnMut(&[f32]) + Send>) -> Result<(), CaptureError> {
        if let Some(err) = self.fail {
            return Err(err);
        }
        self.state.starts.fetch_add(1, Ordering::SeqCst);
        self.state.open.fetch_add(1, Ordering::SeqCst);
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let chunks = self.chunks.clone();
        let gap = self.gap;
        self.thread = Some(thread::spawn(move || {
            for chunk in chunks {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                sink(&chunk);
                thread::sleep(gap);
            }
        }));
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stop.store(true, Ordering::SeqCst);
            let _ = thread.join();
            self.state.open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct FakeEngine {
    fixed: Option<&'static str>,
    delay: Duration,
    /// Zero-based indices of `transcribe` calls that fail.
    fail_on: Vec<usize>,
    calls: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
struct EngineCfg {
    fixed: Option<&'static str>,
    delay: Duration,
    fail_on: Vec<usize>,
    calls: Arc<AtomicUsize>,
}

impl SpeechEngine for FakeEngine {
    fn transcribe(&mut self, samples: &[f32], _terms: &[String]) -> Result<String, EngineError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.delay);
        if self.fail_on.contains(&call) {
            return Err(EngineError::Transcribe("scripted failure".into()));
        }
        Ok(match self.fixed {
            Some(text) => text.to_string(),
            None => format!("{} samples", samples.len()),
        })
    }
}

struct Harness {
    tx: Option<Sender<Vec<u8>>>,
    out: SharedBuf,
    handle: JoinHandle<io::Result<()>>,
}

fn spawn(
    source: FakeSource,
    fixed: Option<&'static str>,
    delay: Duration,
    limits: HelperLimits,
) -> Harness {
    spawn_with(
        source,
        EngineCfg {
            fixed,
            delay,
            ..EngineCfg::default()
        },
        limits,
    )
}

fn spawn_with(source: FakeSource, cfg: EngineCfg, limits: HelperLimits) -> Harness {
    let (tx, rx) = channel();
    let out = SharedBuf::default();
    let writer = out.clone();
    let handle = thread::spawn(move || {
        let input = BufReader::new(ChanReader {
            rx,
            buf: Vec::new(),
        });
        run_helper_with(
            input,
            writer,
            |_dir| {
                Ok(Box::new(FakeEngine {
                    fixed: cfg.fixed,
                    delay: cfg.delay,
                    fail_on: cfg.fail_on.clone(),
                    calls: cfg.calls.clone(),
                }) as Box<dyn SpeechEngine>)
            },
            Box::new(source),
            limits,
        )
    });
    Harness {
        tx: Some(tx),
        out,
        handle,
    }
}

fn limits(max_ms: u64, partial_ms: u64) -> HelperLimits {
    HelperLimits {
        max_recording: Duration::from_millis(max_ms),
        partial_interval: Duration::from_millis(partial_ms),
    }
}

fn chunk(n: usize) -> Vec<f32> {
    vec![0.1; n]
}

impl Harness {
    fn send(&self, req: &Request) {
        let mut line = Vec::new();
        write_line(&mut line, req).unwrap();
        self.tx.as_ref().unwrap().send(line).unwrap();
    }

    fn load(&self) {
        self.send(&Request::Load {
            model_dir: std::env::temp_dir(),
            model_id: "fake".into(),
        });
        self.wait(|e| matches!(e, Event::Loaded));
    }

    fn events(&self) -> Vec<Event> {
        let bytes = self.out.0.lock().unwrap().clone();
        let mut reader = BufReader::new(&bytes[..]);
        let mut events = Vec::new();
        while let Some(event) = read_line::<Event>(&mut reader).expect("every line parses") {
            events.push(event);
        }
        events
    }

    fn wait(&self, pred: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            if let Some(e) = self.events().into_iter().find(|e| pred(e)) {
                return e;
            }
            assert!(
                Instant::now() < deadline,
                "timed out; got {:?}",
                self.events()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Closes the input and returns every event once the helper exits.
    fn finish(mut self) -> (Vec<Event>, io::Result<()>) {
        self.tx.take();
        let result = self.handle.join().unwrap();
        let bytes = self.out.0.lock().unwrap().clone();
        let mut reader = BufReader::new(&bytes[..]);
        let mut events = Vec::new();
        while let Some(e) = read_line::<Event>(&mut reader).unwrap() {
            events.push(e);
        }
        (events, result)
    }
}

fn start(terms: &[&str]) -> Request {
    Request::Start {
        terms: terms.iter().map(|t| t.to_string()).collect(),
    }
}

#[test]
fn announces_ready_first() {
    let (source, _) = FakeSource::new(vec![], Duration::ZERO);
    let h = spawn(source, None, Duration::ZERO, HelperLimits::default());
    h.wait(|e| matches!(e, Event::Ready));
    let (events, _) = h.finish();
    assert_eq!(events[0], Event::Ready);
}

#[test]
fn start_before_load_is_model_missing() {
    let (source, state) = FakeSource::new(vec![], Duration::ZERO);
    let h = spawn(source, None, Duration::ZERO, HelperLimits::default());
    h.send(&start(&[]));
    let event = h.wait(|e| matches!(e, Event::Error { .. }));
    let Event::Error { kind, message } = event else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::ModelMissing);
    assert_eq!(message, "The speech model isn't downloaded yet.");
    assert_eq!(state.starts.load(Ordering::SeqCst), 0, "mic never opened");
}

#[test]
fn load_of_a_missing_directory_is_model_missing() {
    let (source, _) = FakeSource::new(vec![], Duration::ZERO);
    let h = spawn(source, None, Duration::ZERO, HelperLimits::default());
    h.send(&Request::Load {
        model_dir: PathBuf::from("/definitely/not/here"),
        model_id: "fake".into(),
    });
    let Event::Error { kind, .. } = h.wait(|e| matches!(e, Event::Error { .. })) else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::ModelMissing);
}

#[test]
fn stop_emits_postprocessed_final() {
    let (source, state) = FakeSource::new(vec![chunk(1600)], Duration::ZERO);
    let h = spawn(
        source,
        Some("  use inbox unseen "),
        Duration::ZERO,
        limits(10_000, 10_000),
    );
    h.load();
    h.send(&start(&["useInboxUnseen"]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&Request::Stop);
    let fin = h.wait(|e| matches!(e, Event::Final { .. }));
    assert_eq!(
        fin,
        Event::Final {
            text: "useInboxUnseen".into(),
            capped: false
        }
    );
    let (_, result) = h.finish();
    assert!(result.is_ok());
    assert_eq!(
        state.open.load(Ordering::SeqCst),
        0,
        "mic released after stop"
    );
}

#[test]
fn empty_transcript_is_still_a_final() {
    let (source, _) = FakeSource::new(vec![], Duration::ZERO);
    let h = spawn(source, Some("   "), Duration::ZERO, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    h.send(&Request::Stop);
    assert_eq!(
        h.wait(|e| matches!(e, Event::Final { .. })),
        Event::Final {
            text: String::new(),
            capped: false
        }
    );
}

#[test]
fn emits_partials_while_listening_and_only_when_the_buffer_grew() {
    let (source, _) = FakeSource::new(vec![chunk(1600), chunk(1600)], Duration::from_millis(80));
    let h = spawn(source, None, Duration::ZERO, limits(10_000, 20));
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Partial { text } if text == "3200 samples"));
    // Audio has stopped growing; give the worker time to (wrongly) repeat.
    thread::sleep(Duration::from_millis(150));
    h.send(&Request::Stop);
    h.wait(|e| matches!(e, Event::Final { .. }));
    let (events, _) = h.finish();
    let partials: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Partial { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(partials.iter().filter(|t| **t == "3200 samples").count(), 1);
    assert!(!partials.is_empty());
}

#[test]
fn caps_recording_at_limit() {
    // 1 s of audio against a 0.5 s cap.
    let chunks = (0..10).map(|_| chunk(1600)).collect();
    let (source, state) = FakeSource::new(chunks, Duration::from_millis(5));
    let h = spawn(source, None, Duration::ZERO, limits(500, 10_000));
    h.load();
    h.send(&start(&[]));
    let fin = h.wait(|e| matches!(e, Event::Final { .. }));
    assert_eq!(
        fin,
        Event::Final {
            text: "8000 samples".into(),
            capped: true
        }
    );
    let (_, result) = h.finish();
    assert!(result.is_ok());
    assert_eq!(
        state.open.load(Ordering::SeqCst),
        0,
        "mic released at the cap"
    );
}

#[test]
fn cancel_discards_audio() {
    let (source, state) = FakeSource::new(vec![chunk(1600); 3], Duration::from_millis(5));
    let h = spawn(source, None, Duration::ZERO, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&Request::Cancel);
    h.wait(|e| matches!(e, Event::Cancelled));
    assert_eq!(state.open.load(Ordering::SeqCst), 0, "mic closed by cancel");
    // Nothing was decoded or emitted for the cancelled take, and a Stop
    // afterwards has no take to finish.
    h.send(&Request::Stop);
    thread::sleep(Duration::from_millis(150));
    let (events, _) = h.finish();
    let kinds: Vec<_> = events
        .iter()
        .filter(|e| !matches!(e, Event::Level { .. }))
        .collect();
    assert_eq!(
        kinds,
        vec![
            &Event::Ready,
            &Event::Loaded,
            &Event::Listening,
            &Event::Cancelled
        ]
    );
}

#[test]
fn a_take_after_cancel_holds_only_its_own_audio() {
    let (source, _) = FakeSource::new(vec![chunk(1600); 3], Duration::from_millis(5));
    let h = spawn(source, None, Duration::ZERO, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&Request::Cancel);
    h.wait(|e| matches!(e, Event::Cancelled));
    h.send(&start(&[]));
    h.send(&Request::Stop);
    let Event::Final { text, .. } = h.wait(|e| matches!(e, Event::Final { .. })) else {
        unreachable!()
    };
    let n: usize = text.split(' ').next().unwrap().parse().unwrap();
    assert!(n <= 4800, "got {n}");
}

#[test]
fn second_start_is_busy() {
    let (source, _) = FakeSource::new(vec![chunk(160)], Duration::ZERO);
    let h = spawn(source, None, Duration::ZERO, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&start(&[]));
    let Event::Error { kind, message } = h.wait(|e| matches!(e, Event::Error { .. })) else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::Busy);
    assert_eq!(message, "Dictation is already running.");
    // The first dictation is unaffected.
    h.send(&Request::Stop);
    h.wait(|e| matches!(e, Event::Final { .. }));
}

#[test]
fn no_input_device_reports_error() {
    let h = spawn(
        FakeSource::failing(CaptureError::NoInputDevice),
        None,
        Duration::ZERO,
        HelperLimits::default(),
    );
    h.load();
    h.send(&start(&[]));
    let Event::Error { kind, message } = h.wait(|e| matches!(e, Event::Error { .. })) else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::NoInputDevice);
    assert_eq!(message, "No microphone was found.");
    let (events, _) = h.finish();
    assert!(!events.contains(&Event::Listening));
}

#[test]
fn denied_microphone_reports_system_settings_hint() {
    let h = spawn(
        FakeSource::failing(CaptureError::Denied),
        None,
        Duration::ZERO,
        HelperLimits::default(),
    );
    h.load();
    h.send(&start(&[]));
    let Event::Error { kind, message } = h.wait(|e| matches!(e, Event::Error { .. })) else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::MicrophoneDenied);
    assert_eq!(
        message,
        "Gespan can't use the microphone. Allow it in System Settings → Privacy & Security → Microphone."
    );
    // The failed start must not leave the helper stuck in "listening".
    h.send(&start(&[]));
    let deadline = Instant::now() + Duration::from_millis(1500);
    while h
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Error { .. }))
        .count()
        < 2
    {
        assert!(Instant::now() < deadline, "second error never arrived");
        thread::sleep(Duration::from_millis(5));
    }
    let errors: Vec<_> = h
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Error { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(errors, vec![ErrorKind::MicrophoneDenied; 2]);
}

#[test]
fn eof_exits_cleanly() {
    let (source, _) = FakeSource::new(vec![], Duration::ZERO);
    let h = spawn(source, None, Duration::ZERO, HelperLimits::default());
    let (events, result) = h.finish();
    assert!(result.is_ok());
    assert_eq!(events, vec![Event::Ready]);
}

#[test]
fn shutdown_exits_and_releases_a_live_capture() {
    let (source, state) = FakeSource::new(vec![chunk(160); 50], Duration::from_millis(5));
    let h = spawn(source, None, Duration::ZERO, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&Request::Shutdown);
    let result = h.handle.join().unwrap();
    assert!(result.is_ok());
    assert_eq!(state.open.load(Ordering::SeqCst), 0);
}

#[test]
fn level_is_finite_and_throttled() {
    let mut chunks = vec![Vec::new(), vec![f32::NAN; 160], vec![f32::INFINITY; 160]];
    chunks.extend((0..300).map(|_| vec![0.5f32; 160]));
    let (source, _) = FakeSource::new(chunks, Duration::from_millis(2));
    let h = spawn(source, None, Duration::ZERO, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    let t0 = Instant::now();
    h.wait(|e| matches!(e, Event::Listening));
    thread::sleep(Duration::from_millis(900));
    let elapsed = t0.elapsed().as_secs_f64();
    h.send(&Request::Stop);
    h.wait(|e| matches!(e, Event::Final { .. }));
    let (events, _) = h.finish();
    let levels: Vec<f32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Level { rms } => Some(*rms),
            _ => None,
        })
        .collect();
    assert!(!levels.is_empty());
    assert!(levels
        .iter()
        .all(|r| r.is_finite() && (0.0..=1.0).contains(r)));
    let budget = (20.0 * elapsed).ceil() as usize + 1;
    assert!(
        levels.len() <= budget,
        "{} levels in {elapsed:.2}s",
        levels.len()
    );
}

#[test]
fn stop_during_a_partial_waits_and_decodes_the_full_buffer() {
    let (source, _) = FakeSource::new(vec![chunk(1600), chunk(1600)], Duration::from_millis(120));
    let h = spawn(source, None, Duration::from_millis(300), limits(10_000, 20));
    h.load();
    h.send(&start(&[]));
    // The first partial decode (of 1600 samples) is now running for 300 ms,
    // and the second chunk lands while it runs.
    thread::sleep(Duration::from_millis(200));
    h.send(&Request::Stop);
    let fin = h.wait(|e| matches!(e, Event::Final { .. }));
    assert_eq!(
        fin,
        Event::Final {
            text: "3200 samples".into(),
            capped: false
        }
    );
    let (events, _) = h.finish();
    let after_final = events
        .iter()
        .skip_while(|e| !matches!(e, Event::Final { .. }))
        .any(|e| matches!(e, Event::Partial { .. }));
    assert!(!after_final, "a late partial must not follow the final");
}

#[test]
fn cancel_during_a_partial_drops_the_late_partial() {
    let (source, _) = FakeSource::new(vec![chunk(1600)], Duration::ZERO);
    let h = spawn(source, None, Duration::from_millis(300), limits(10_000, 20));
    h.load();
    h.send(&start(&[]));
    thread::sleep(Duration::from_millis(120));
    h.send(&Request::Cancel);
    h.wait(|e| matches!(e, Event::Cancelled));
    thread::sleep(Duration::from_millis(450));
    let (events, _) = h.finish();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Partial { .. } | Event::Final { .. })),
        "got {events:?}"
    );
}

#[test]
fn start_racing_capped_finish_does_not_deadlock() {
    // 50 ms cap, 5 ms chunks; hammer Start across many cap/finish cycles.
    let chunks = vec![chunk(80); 200];
    let (source, state) = FakeSource::new(chunks, Duration::from_millis(5));
    let h = spawn(source, None, Duration::from_millis(3), limits(50, 10_000));
    h.load();
    for _ in 0..300 {
        h.send(&start(&[]));
        thread::sleep(Duration::from_millis(1));
    }
    h.send(&Request::Shutdown);
    let Harness { handle, out, .. } = h;
    let (done_tx, done_rx) = channel();
    thread::spawn(move || {
        let _ = done_tx.send(handle.join());
    });
    let joined = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("helper must answer Shutdown within 2 s");
    assert!(joined.unwrap().is_ok());
    assert_eq!(state.open.load(Ordering::SeqCst), 0);
    // Every Final is capped (nobody sent Stop) and none precedes its Listening.
    let bytes = out.0.lock().unwrap().clone();
    let mut reader = BufReader::new(&bytes[..]);
    let mut listening = 0usize;
    let mut finals = 0usize;
    while let Some(e) = read_line::<Event>(&mut reader).unwrap() {
        match e {
            Event::Listening => listening += 1,
            Event::Final { capped, .. } => {
                assert!(capped);
                finals += 1;
                assert!(finals <= listening, "final before its Listening");
            }
            _ => {}
        }
    }
    assert!(finals >= 1, "the race never exercised a capped finish");
}

#[test]
fn capped_final_precedes_next_listening() {
    let (source, _) = FakeSource::new(vec![chunk(400); 20], Duration::from_millis(5));
    // Cap 100 ms (1600 samples); the final decode takes 300 ms.
    let h = spawn(
        source,
        None,
        Duration::from_millis(300),
        limits(100, 10_000),
    );
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    thread::sleep(Duration::from_millis(100));
    // The take has capped and its final is still decoding.
    h.send(&start(&[]));
    let busy = h.wait(|e| matches!(e, Event::Error { .. }));
    assert!(matches!(
        busy,
        Event::Error {
            kind: ErrorKind::Busy,
            ..
        }
    ));
    h.wait(|e| matches!(e, Event::Final { .. }));
    h.send(&start(&[]));
    thread::sleep(Duration::from_millis(50));
    h.send(&Request::Cancel);
    h.wait(|e| matches!(e, Event::Cancelled));
    let (events, _) = h.finish();
    let sequence: Vec<_> = events
        .into_iter()
        .filter(|e| !matches!(e, Event::Level { .. } | Event::Partial { .. }))
        .collect();
    assert_eq!(sequence.len(), 7, "{sequence:?}");
    assert_eq!(sequence[2], Event::Listening);
    assert!(matches!(
        sequence[3],
        Event::Error {
            kind: ErrorKind::Busy,
            ..
        }
    ));
    assert!(matches!(sequence[4], Event::Final { capped: true, .. }));
    assert_eq!(sequence[5], Event::Listening);
    assert_eq!(sequence[6], Event::Cancelled);
}

#[test]
fn cancel_during_final_decode_suppresses_final() {
    let (source, _) = FakeSource::new(vec![chunk(1600)], Duration::ZERO);
    let h = spawn(
        source,
        None,
        Duration::from_millis(400),
        limits(10_000, 10_000),
    );
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&Request::Stop);
    thread::sleep(Duration::from_millis(100));
    let sent = Instant::now();
    h.send(&Request::Cancel);
    h.wait(|e| matches!(e, Event::Cancelled));
    assert!(
        sent.elapsed() < Duration::from_millis(250),
        "cancel was read while the final decode ran"
    );
    // The take is over, so a new one can start while the old decode finishes.
    h.send(&start(&[]));
    thread::sleep(Duration::from_millis(500));
    let (events, _) = h.finish();
    assert!(
        !events.iter().any(|e| matches!(e, Event::Final { .. })),
        "got {events:?}"
    );
    assert_eq!(events.iter().filter(|e| **e == Event::Listening).count(), 2);
}

#[test]
fn failed_partial_still_produces_final() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cfg = EngineCfg {
        fail_on: vec![0],
        calls: calls.clone(),
        ..EngineCfg::default()
    };
    let (source, _) = FakeSource::new(vec![chunk(1600)], Duration::ZERO);
    let h = spawn_with(source, cfg, limits(10_000, 20));
    h.load();
    h.send(&start(&[]));
    let deadline = Instant::now() + Duration::from_millis(1000);
    while calls.load(Ordering::SeqCst) < 1 {
        assert!(Instant::now() < deadline, "partial decode never ran");
        thread::sleep(Duration::from_millis(5));
    }
    h.send(&Request::Stop);
    assert_eq!(
        h.wait(|e| matches!(e, Event::Final { .. })),
        Event::Final {
            text: "1600 samples".into(),
            capped: false
        }
    );
    let (events, _) = h.finish();
    assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
    assert!(!events.iter().any(|e| matches!(e, Event::Partial { .. })));
}

#[test]
fn failed_final_reports_engine_error_and_closes_mic() {
    let cfg = EngineCfg {
        fail_on: vec![0],
        ..EngineCfg::default()
    };
    let (source, state) = FakeSource::new(vec![chunk(1600)], Duration::ZERO);
    let h = spawn_with(source, cfg, limits(10_000, 10_000));
    h.load();
    h.send(&start(&[]));
    h.wait(|e| matches!(e, Event::Listening));
    h.send(&Request::Stop);
    let Event::Error { kind, .. } = h.wait(|e| matches!(e, Event::Error { .. })) else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::Engine);
    assert_eq!(state.open.load(Ordering::SeqCst), 0);
    // The helper is usable again.
    h.send(&start(&[]));
    h.send(&Request::Stop);
    h.wait(|e| matches!(e, Event::Final { .. }));
    let (events, _) = h.finish();
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::Busy,
            ..
        }
    )));
}
