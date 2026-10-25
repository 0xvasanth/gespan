//! The speech helper's request loop: model loading, microphone sessions,
//! periodic partials and the final transcript.
//!
//! Threads: the caller's thread reads requests and handles `Stop`. Each
//! dictation also gets one worker thread that re-decodes for partials and
//! finishes the take when the recording cap is reached. The audio source calls
//! the sink on its own thread. All three share [`Shared`]; the `engine` mutex
//! serialises decodes, so a `Stop` that arrives mid-partial waits for it, and
//! the session `gen` counter makes a partial that finishes after a `Stop` or
//! `Cancel` drop itself instead of being emitted.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{AudioSource, CaptureError, Sink, TARGET_RATE};
use crate::engine::{EngineError, SpeechEngine};
use crate::parakeet::ParakeetEngine;
use crate::postprocess::postprocess;
use crate::protocol::{read_line, write_line, ErrorKind, Event, Request};

pub const MAX_RECORDING_SECS: u32 = 120;
pub const PARTIAL_INTERVAL_MS: u64 = 1000;

/// Level events are sent at most this often (20 per second).
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);
/// How often the worker checks for the cap and the partial deadline.
const WORKER_TICK: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy)]
pub struct HelperLimits {
    pub max_recording: Duration,
    pub partial_interval: Duration,
}

impl Default for HelperLimits {
    fn default() -> Self {
        Self {
            max_recording: Duration::from_secs(u64::from(MAX_RECORDING_SECS)),
            partial_interval: Duration::from_millis(PARTIAL_INTERVAL_MS),
        }
    }
}

#[derive(Default)]
struct Session {
    loaded: bool,
    active: bool,
    /// Bumped whenever a take starts or ends, so stale workers and sinks stop.
    gen: u64,
    /// Bumped by `Cancel` only; a final that finishes after it is dropped.
    cancel_epoch: u64,
    capped: bool,
    buffer: Vec<f32>,
    terms: Vec<String>,
}

struct Shared {
    session: Mutex<Session>,
    engine: Mutex<Option<Box<dyn SpeechEngine>>>,
    source: Mutex<Box<dyn AudioSource>>,
    out: Mutex<Box<dyn Write + Send>>,
    broken: AtomicBool,
    limits: HelperLimits,
    cap_samples: usize,
}

impl Shared {
    fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn engine(&self) -> MutexGuard<'_, Option<Box<dyn SpeechEngine>>> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn source(&self) -> MutexGuard<'_, Box<dyn AudioSource>> {
        self.source.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: &Event) {
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        if write_line(&mut *out, event).is_err() {
            self.broken.store(true, Ordering::SeqCst);
        }
    }

    fn error(&self, kind: ErrorKind, message: &str) {
        self.emit(&Event::Error {
            kind,
            message: message.to_string(),
        });
    }
}

pub fn run_helper(
    input: impl BufRead,
    output: impl Write + Send + 'static,
    make_engine: impl Fn(&Path) -> Result<Box<dyn SpeechEngine>, EngineError>,
    source: Box<dyn AudioSource>,
) -> io::Result<()> {
    run_helper_with(input, output, make_engine, source, HelperLimits::default())
}

pub fn run_helper_with(
    mut input: impl BufRead,
    output: impl Write + Send + 'static,
    make_engine: impl Fn(&Path) -> Result<Box<dyn SpeechEngine>, EngineError>,
    source: Box<dyn AudioSource>,
    limits: HelperLimits,
) -> io::Result<()> {
    let shared = Arc::new(Shared {
        session: Mutex::new(Session::default()),
        engine: Mutex::new(None),
        source: Mutex::new(source),
        out: Mutex::new(Box::new(output)),
        broken: AtomicBool::new(false),
        limits,
        cap_samples: (limits.max_recording.as_secs_f64() * f64::from(TARGET_RATE)) as usize,
    });
    let mut workers: Vec<JoinHandle<()>> = Vec::new();

    shared.emit(&Event::Ready);
    let result = loop {
        if shared.broken.load(Ordering::SeqCst) {
            break Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let request = match read_line::<Request>(&mut input) {
            Ok(Some(request)) => request,
            Ok(None) => break Ok(()),
            // A malformed line is skipped; the apps never send one.
            Err(err) if err.kind() == io::ErrorKind::InvalidData => continue,
            Err(err) => break Err(err),
        };
        match request {
            Request::Load { model_dir, .. } => load(&shared, &make_engine, &model_dir),
            Request::Start { terms } => {
                workers.retain(|w| !w.is_finished());
                if let Some(worker) = start(&shared, terms) {
                    workers.push(worker);
                }
            }
            Request::Stop => finish(&shared, None, false),
            Request::Cancel => cancel(&shared),
            Request::Shutdown => break Ok(()),
        }
    };

    // Release the microphone and let any worker finish before exiting.
    abandon(&shared);
    for worker in workers {
        let _ = worker.join();
    }
    match result {
        Ok(()) if shared.broken.load(Ordering::SeqCst) => {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        other => other,
    }
}

fn load(
    shared: &Shared,
    make_engine: &impl Fn(&Path) -> Result<Box<dyn SpeechEngine>, EngineError>,
    model_dir: &Path,
) {
    if shared.session().active {
        return shared.error(ErrorKind::Busy, "Dictation is already running.");
    }
    if !model_dir.is_dir() {
        return shared.error(
            ErrorKind::ModelMissing,
            "The speech model isn't downloaded yet.",
        );
    }
    match make_engine(model_dir) {
        Ok(engine) => {
            *shared.engine() = Some(engine);
            shared.session().loaded = true;
            shared.emit(&Event::Loaded);
        }
        Err(_) => shared.error(ErrorKind::ModelLoad, "The speech model couldn't be loaded."),
    }
}

/// Begins a take. Returns the worker thread when listening started.
fn start(shared: &Arc<Shared>, terms: Vec<String>) -> Option<JoinHandle<()>> {
    // Held across `source.start` so Listening precedes any Level.
    let mut session = shared.session();
    if session.active {
        shared.error(ErrorKind::Busy, "Dictation is already running.");
        return None;
    }
    if !session.loaded {
        shared.error(
            ErrorKind::ModelMissing,
            "The speech model isn't downloaded yet.",
        );
        return None;
    }
    session.gen += 1;
    let gen = session.gen;
    session.active = true;
    session.capped = false;
    session.buffer.clear();
    session.terms = terms;

    let started = shared.source().start(sink(shared.clone(), gen));
    match started {
        Ok(()) => {
            shared.emit(&Event::Listening);
            drop(session);
            let shared = shared.clone();
            Some(thread::spawn(move || worker(&shared, gen)))
        }
        Err(err) => {
            session.active = false;
            session.gen += 1;
            let (kind, message) = match err {
                CaptureError::NoInputDevice => {
                    (ErrorKind::NoInputDevice, "No microphone was found.")
                }
                CaptureError::Denied => (
                    ErrorKind::MicrophoneDenied,
                    "Gespan can't use the microphone. Allow it in System Settings → Privacy & Security → Microphone.",
                ),
            };
            shared.error(kind, message);
            None
        }
    }
}

/// The callback the audio source feeds: buffers audio up to the cap and sends
/// throttled level readings.
fn sink(shared: Arc<Shared>, gen: u64) -> Sink {
    let mut last_level: Option<Instant> = None;
    Box::new(move |chunk| {
        let mut session = shared.session();
        if !session.active || session.gen != gen {
            return;
        }
        let room = shared.cap_samples.saturating_sub(session.buffer.len());
        let take = chunk.len().min(room);
        session.buffer.extend_from_slice(&chunk[..take]);
        if take < chunk.len() || session.buffer.len() >= shared.cap_samples {
            session.capped = true;
        }
        if last_level.is_none_or(|at| at.elapsed() >= LEVEL_INTERVAL) {
            last_level = Some(Instant::now());
            shared.emit(&Event::Level { rms: rms(chunk) });
        }
    })
}

/// Root-mean-square of `chunk`, always finite and within 0..=1.
fn rms(chunk: &[f32]) -> f32 {
    if chunk.is_empty() {
        return 0.0;
    }
    let mean_square = chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32;
    let value = mean_square.sqrt();
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Re-decodes the growing buffer for partials and ends the take at the cap.
fn worker(shared: &Shared, gen: u64) {
    let mut decoded_len = 0;
    let mut last_decode = Instant::now();
    loop {
        thread::sleep(WORKER_TICK);
        let (capped, len) = {
            let session = shared.session();
            if !session.active || session.gen != gen {
                return;
            }
            (session.capped, session.buffer.len())
        };
        if capped {
            return finish(shared, Some(gen), true);
        }
        if len <= decoded_len || last_decode.elapsed() < shared.limits.partial_interval {
            continue;
        }
        let (samples, terms) = {
            let session = shared.session();
            if !session.active || session.gen != gen {
                return;
            }
            (session.buffer.clone(), session.terms.clone())
        };
        decoded_len = samples.len();
        let text = shared
            .engine()
            .as_mut()
            .and_then(|engine| engine.transcribe(&samples, &terms).ok());
        last_decode = Instant::now();
        if let Some(text) = text {
            // Emitted under the session lock so a Stop or Cancel cannot slip
            // between the check and the write.
            let session = shared.session();
            if session.active && session.gen == gen {
                shared.emit(&Event::Partial {
                    text: text.trim().to_string(),
                });
            }
        }
    }
}

/// Ends the take: stops the microphone, decodes the whole buffer once and
/// sends the `Final`. `expect_gen` restricts it to one specific take.
fn finish(shared: &Shared, expect_gen: Option<u64>, capped: bool) {
    let (samples, terms, epoch) = {
        let mut session = shared.session();
        if !session.active || expect_gen.is_some_and(|g| g != session.gen) {
            return;
        }
        session.active = false;
        session.gen += 1;
        (
            std::mem::take(&mut session.buffer),
            std::mem::take(&mut session.terms),
            session.cancel_epoch,
        )
    };
    shared.source().stop();
    let decoded = shared
        .engine()
        .as_mut()
        .map(|engine| engine.transcribe(&samples, &terms));
    let event = match decoded {
        Some(Ok(raw)) => Event::Final {
            text: postprocess(&raw, &terms).trim().to_string(),
            capped,
        },
        Some(Err(_)) => Event::Error {
            kind: ErrorKind::Engine,
            message: "Speech recognition failed.".to_string(),
        },
        None => Event::Error {
            kind: ErrorKind::ModelMissing,
            message: "The speech model isn't downloaded yet.".to_string(),
        },
    };
    let session = shared.session();
    if session.cancel_epoch == epoch {
        shared.emit(&event);
    }
}

fn cancel(shared: &Shared) {
    {
        let mut session = shared.session();
        session.active = false;
        session.gen += 1;
        session.cancel_epoch += 1;
        session.buffer.clear();
        session.terms.clear();
        shared.emit(&Event::Cancelled);
    }
    shared.source().stop();
}

/// Drops any live take silently and releases the microphone.
fn abandon(shared: &Shared) {
    {
        let mut session = shared.session();
        session.active = false;
        session.gen += 1;
        session.cancel_epoch += 1;
        session.buffer.clear();
    }
    shared.source().stop();
}

/// Entry point for the apps' `speech-helper` subcommand: speaks the protocol
/// on stdin and stdout. Exits 0 on `Shutdown` or end of input, 1 otherwise.
pub fn helper_main() -> ! {
    let stdin = io::stdin();
    let result = run_helper(
        stdin.lock(),
        io::stdout(),
        |dir| ParakeetEngine::load(dir).map(|e| Box::new(e) as Box<dyn SpeechEngine>),
        Box::new(crate::capture::MicSource::new()),
    );
    std::process::exit(if result.is_ok() { 0 } else { 1 })
}
