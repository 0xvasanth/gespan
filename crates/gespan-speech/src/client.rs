//! Supervisor for the speech helper process, shared by both apps.
//!
//! One supervisor thread owns the child: it spawns it, reads its events and
//! restarts it with backoff when it exits unexpectedly. [`SpeechClient::send`]
//! writes straight to the child's stdin from the caller's thread, or queues
//! the request while the helper is starting or restarting.

use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::protocol::{read_line, write_line, ErrorKind, Event, Request};

const CRASHED: &str = "Dictation stopped unexpectedly.";
const UNAVAILABLE: &str = "Dictation is unavailable. Restart Gespan to try again.";
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Backoff and give-up rules for restarting the helper.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// Delay before the nth restart in the window; the last entry repeats.
    pub delays: Vec<Duration>,
    pub window: Duration,
    /// Restarts allowed within `window`; the next exit gives up.
    pub max_restarts: usize,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            delays: vec![
                Duration::from_millis(500),
                Duration::from_secs(2),
                Duration::from_secs(5),
            ],
            window: Duration::from_secs(60),
            max_restarts: 3,
        }
    }
}

/// The command that runs the helper from the app's own binary.
pub fn helper_command() -> io::Result<(PathBuf, Vec<String>)> {
    Ok((std::env::current_exe()?, vec!["speech-helper".into()]))
}

#[derive(Default)]
struct State {
    stdin: Option<ChildStdin>,
    /// The current helper has said `Ready`, so requests can be written.
    ready: bool,
    /// A `Listening` event was seen and nothing has ended it yet.
    listening: bool,
    queue: Vec<Request>,
    last_load: Option<Request>,
    gave_up: bool,
    closing: bool,
    /// The supervisor thread has finished.
    done: bool,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    child: Mutex<Option<Child>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn kill_child(&self) {
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = child.kill();
        }
    }
}

pub struct SpeechClient {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl SpeechClient {
    pub fn spawn(
        program: PathBuf,
        args: Vec<String>,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> SpeechClient {
        Self::spawn_with_env(program, args, Vec::new(), on_event)
    }

    pub fn spawn_with_env(
        program: PathBuf,
        args: Vec<String>,
        envs: Vec<(String, String)>,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> SpeechClient {
        Self::spawn_with(program, args, envs, RestartPolicy::default(), on_event)
    }

    pub fn spawn_with(
        program: PathBuf,
        args: Vec<String>,
        envs: Vec<(String, String)>,
        policy: RestartPolicy,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> SpeechClient {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
            child: Mutex::new(None),
        });
        let sup = Supervisor {
            shared: shared.clone(),
            command: (program, args, envs),
            policy,
            on_event: Box::new(on_event),
        };
        let thread = thread::Builder::new()
            .name("speech-client".into())
            .spawn(move || {
                sup.run();
                let mut st = sup.shared.lock();
                st.done = true;
                sup.shared.cv.notify_all();
            })
            .expect("spawn speech client thread");
        SpeechClient {
            shared,
            thread: Some(thread),
        }
    }

    /// Sends a request, or queues it until the helper is ready. Does nothing
    /// once the client has given up or been shut down.
    pub fn send(&self, req: Request) {
        let mut st = self.shared.lock();
        if st.gave_up || st.closing {
            return;
        }
        if matches!(req, Request::Load { .. }) {
            st.last_load = Some(req.clone());
        }
        if st.ready {
            if let Some(stdin) = st.stdin.as_mut() {
                // A failed write means the helper is exiting; the reader
                // thread sees that as end of file and restarts it.
                let _ = write_line(stdin, &req);
                return;
            }
        }
        st.queue.push(req);
    }

    /// Asks the helper to exit, waits up to two seconds, then kills it.
    pub fn shutdown(mut self) {
        {
            let mut st = self.shared.lock();
            st.closing = true;
            if let Some(stdin) = st.stdin.as_mut() {
                let _ = write_line(stdin, &Request::Shutdown);
            }
            self.shared.cv.notify_all();
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        let mut st = self.shared.lock();
        while !st.done {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            st = self
                .shared
                .cv
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(st);
        self.finish();
    }

    /// Kills whatever is still running and joins the supervisor thread.
    fn finish(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        {
            let mut st = self.shared.lock();
            st.closing = true;
            st.stdin = None;
            self.shared.cv.notify_all();
        }
        self.shared.kill_child();
        let _ = thread.join();
    }
}

impl Drop for SpeechClient {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Supervisor {
    shared: Arc<Shared>,
    command: (PathBuf, Vec<String>, Vec<(String, String)>),
    policy: RestartPolicy,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
}

impl Supervisor {
    fn emit_error(&self, message: &str) {
        (self.on_event)(Event::Error {
            kind: ErrorKind::Engine,
            message: message.into(),
        });
    }

    fn run(&self) {
        let mut restarts: Vec<Instant> = Vec::new();
        loop {
            if self.shared.lock().closing {
                return;
            }
            self.run_once();

            let was_listening = {
                let mut st = self.shared.lock();
                st.ready = false;
                st.stdin = None;
                std::mem::take(&mut st.listening)
            };
            if self.shared.lock().closing {
                return;
            }
            if was_listening {
                self.emit_error(CRASHED);
            }

            let now = Instant::now();
            restarts.retain(|t| now.duration_since(*t) < self.policy.window);
            if restarts.len() >= self.policy.max_restarts {
                {
                    let mut st = self.shared.lock();
                    st.gave_up = true;
                    st.queue.clear();
                }
                self.emit_error(UNAVAILABLE);
                return;
            }
            let delay = self
                .policy
                .delays
                .get(restarts.len())
                .or(self.policy.delays.last())
                .copied()
                .unwrap_or_default();
            restarts.push(now);
            if !self.sleep(delay) {
                return;
            }
        }
    }

    /// Sleeps for `delay`; returns false if the client is closing.
    fn sleep(&self, delay: Duration) -> bool {
        let deadline = Instant::now() + delay;
        let mut st = self.shared.lock();
        while !st.closing {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            st = self
                .shared
                .cv
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        false
    }

    /// Runs one helper process until its stdout closes, then reaps it.
    fn run_once(&self) {
        let (program, args, envs) = &self.command;
        let spawned = Command::new(program)
            .args(args)
            .envs(envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => {
                note_spawn_failure(&err);
                return;
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        *self.shared.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        {
            let mut st = self.shared.lock();
            if st.closing {
                drop(st);
                self.shared.kill_child();
            } else {
                st.stdin = stdin;
            }
        }

        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_line::<Event>(&mut reader) {
                    Ok(Some(Event::Ready)) => self.on_ready(),
                    Ok(Some(event)) => self.on_event(event),
                    Ok(None) => break,
                    // A malformed line is skipped; anything else is a broken pipe.
                    Err(err) if err.kind() == io::ErrorKind::InvalidData => continue,
                    Err(_) => break,
                }
            }
        }

        // Stdout is closed, so the helper is gone or about to be.
        self.shared.kill_child();
        if let Some(mut child) = self
            .shared
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = child.wait();
        }
    }

    /// Marks the helper ready, then replays the last `Load` and the queue.
    fn on_ready(&self) {
        let mut st = self.shared.lock();
        let queue = std::mem::take(&mut st.queue);
        let mut pending = Vec::with_capacity(queue.len() + 1);
        if !queue.iter().any(|r| matches!(r, Request::Load { .. })) {
            pending.extend(st.last_load.clone());
        }
        pending.extend(queue);
        if let Some(stdin) = st.stdin.as_mut() {
            for req in &pending {
                if write_line(stdin, req).is_err() {
                    break;
                }
            }
        }
        st.ready = true;
    }

    fn on_event(&self, event: Event) {
        match event {
            Event::Listening => self.shared.lock().listening = true,
            Event::Final { .. } | Event::Cancelled | Event::Error { .. } => {
                self.shared.lock().listening = false
            }
            _ => {}
        }
        (self.on_event)(event);
    }
}

/// Spawn failures are treated like an immediate exit; stderr is the only
/// diagnostic channel in this crate.
fn note_spawn_failure(err: &io::Error) {
    eprintln!("speech helper failed to start: {}", err.kind());
}
