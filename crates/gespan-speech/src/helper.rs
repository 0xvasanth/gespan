//! The speech helper's request loop: model loading, microphone sessions,
//! periodic partials and the final transcript.
//!
//! Threads: the caller's thread only reads requests. Each dictation gets one
//! worker thread that re-decodes for partials and, at the recording cap, runs
//! the final decode itself; a `Stop` runs its final decode on a short-lived
//! thread. The audio source calls the sink on its own thread. They share
//! [`Shared`]; the `engine` mutex serialises decodes, so a `Stop` that arrives
//! mid-partial waits for it. The session `gen` counter makes a partial that
//! finishes after a `Stop` or `Cancel` drop itself, and `cancel_epoch` does the
//! same for a final. The session lock is never held across a call into the
//! audio source, because stopping a source waits for its sink, which takes
//! that lock. `finishing` keeps `Start` and `Load` busy until the previous
//! take's `Final` is out.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
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
    /// Set once `Listening` is out, so no `Level` precedes it.
    announced: bool,
    /// Id (the take's `gen` after finishing) of a final decode still running.
    finishing: Option<u64>,
    buffer: Vec<f32>,
    terms: Vec<String>,
}

impl Session {
    fn is_busy(&self) -> bool {
        self.active || self.finishing.is_some()
    }
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
            Request::Start { terms } => start(&shared, terms),
            Request::Stop => stop(&shared),
            Request::Cancel => cancel(&shared),
            Request::Shutdown => break Ok(()),
        }
    };

    // Release the microphone. Worker threads are left to wind down: they
    // emit nothing once the take is abandoned.
    abandon(&shared);
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
    if shared.session().is_busy() {
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

/// Begins a take and spawns its worker.
fn start(shared: &Arc<Shared>, terms: Vec<String>) {
    let gen = {
        let mut session = shared.session();
        if session.is_busy() {
            return shared.error(ErrorKind::Busy, "Dictation is already running.");
        }
        if !session.loaded {
            return shared.error(
                ErrorKind::ModelMissing,
                "The speech model isn't downloaded yet.",
            );
        }
        session.gen += 1;
        session.active = true;
        session.announced = false;
        session.capped = false;
        session.buffer.clear();
        session.terms = terms;
        session.gen
    };

    // The session lock is not held here: the source's thread calls the sink,
    // which takes it.
    let started = shared.source().start(sink(shared.clone(), gen));
    match started {
        Ok(()) => {
            {
                let mut session = shared.session();
                if session.gen != gen {
                    return;
                }
                session.announced = true;
                shared.emit(&Event::Listening);
            }
            let shared = shared.clone();
            thread::spawn(move || worker(&shared, gen));
        }
        Err(err) => {
            {
                let mut session = shared.session();
                if session.gen == gen {
                    session.active = false;
                    session.gen += 1;
                }
            }
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
        if session.announced && last_level.is_none_or(|at| at.elapsed() >= LEVEL_INTERVAL) {
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
            if let Some(job) = begin_finish(shared, Some(gen)) {
                stop_source(shared);
                decode_final(shared, job, true);
            }
            return;
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

/// A take that has ended and is waiting for its final decode.
struct FinalJob {
    id: u64,
    epoch: u64,
    samples: Vec<f32>,
    terms: Vec<String>,
}

/// Marks the take as ended (and `finishing`) and hands out its audio.
/// `expect_gen` restricts it to one specific take.
fn begin_finish(shared: &Shared, expect_gen: Option<u64>) -> Option<FinalJob> {
    let mut session = shared.session();
    if !session.active || expect_gen.is_some_and(|g| g != session.gen) {
        return None;
    }
    session.active = false;
    session.gen += 1;
    session.finishing = Some(session.gen);
    Some(FinalJob {
        id: session.gen,
        epoch: session.cancel_epoch,
        samples: std::mem::take(&mut session.buffer),
        terms: std::mem::take(&mut session.terms),
    })
}

/// Stops the microphone unless a newer take has already claimed the source.
fn stop_source(shared: &Shared) {
    let mut source = shared.source();
    if !shared.session().active {
        source.stop();
    }
}

/// `Stop`: ends the take, releases the microphone, and decodes the final off
/// the request-reading thread.
fn stop(shared: &Arc<Shared>) {
    let Some(job) = begin_finish(shared, None) else {
        return;
    };
    stop_source(shared);
    let shared = shared.clone();
    thread::spawn(move || decode_final(&shared, job, false));
}

/// Decodes the whole buffer once and sends the `Final`, unless a `Cancel`
/// arrived meanwhile.
fn decode_final(shared: &Shared, job: FinalJob, capped: bool) {
    let decoded = shared
        .engine()
        .as_mut()
        .map(|engine| engine.transcribe(&job.samples, &job.terms));
    let event = match decoded {
        Some(Ok(raw)) => Event::Final {
            text: postprocess(&raw, &job.terms).trim().to_string(),
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
    let mut session = shared.session();
    if session.cancel_epoch == job.epoch {
        shared.emit(&event);
    }
    if session.finishing == Some(job.id) {
        session.finishing = None;
    }
}

fn cancel(shared: &Shared) {
    {
        let mut session = shared.session();
        session.active = false;
        session.gen += 1;
        session.cancel_epoch += 1;
        session.finishing = None;
        session.buffer.clear();
        session.terms.clear();
    }
    // Cancelled means the microphone is already off.
    stop_source(shared);
    shared.emit(&Event::Cancelled);
}

/// Drops any live take silently and releases the microphone.
fn abandon(shared: &Shared) {
    {
        let mut session = shared.session();
        session.active = false;
        session.gen += 1;
        session.cancel_epoch += 1;
        session.finishing = None;
        session.buffer.clear();
    }
    stop_source(shared);
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
