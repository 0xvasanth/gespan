use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, Error, Events};
use crate::transport::Transport;
use crate::{Agent, Event, SshFailure, Workspace};

/// Whether a machine's herdr can be reached right now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Online {
    #[default]
    Connecting,
    Online,
    /// Lost or never reached; tries again after `retry_in`.
    Offline {
        reason: String,
        retry_in: Duration,
    },
    /// Needs a person to sign in once; waits for `refresh`.
    NeedsSignIn {
        hint: String,
    },
    HerdrMissing,
    HerdrTooOld,
}

/// What is known about one machine. The lists are kept while offline.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub online: Online,
    pub version: Option<String>,
    pub workspaces: Vec<Workspace>,
    pub agents: Vec<Agent>,
}

/// The wait before retry number `attempt`: 1 s, doubling, capped at 60 s.
pub fn backoff(attempt: u32) -> Duration {
    const CAP: u64 = 60;
    let secs = 1u64.checked_shl(attempt).unwrap_or(CAP).min(CAP);
    Duration::from_secs(secs)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Flags {
    stopped: bool,
    refresh: bool,
}

/// Wakes the connection thread for `stop` and `refresh`.
#[derive(Default)]
struct Signal {
    flags: Mutex<Flags>,
    wake: Condvar,
}

impl Signal {
    /// Block until stopped or asked to refresh, or until `limit` passes.
    fn wait(&self, limit: Option<Duration>) {
        let flags = lock(&self.flags);
        let idle = |f: &mut Flags| !f.stopped && !f.refresh;
        // A poisoned lock only means another thread panicked; stop waiting.
        match limit {
            Some(d) => drop(self.wake.wait_timeout_while(flags, d, idle)),
            None => drop(self.wake.wait_while(flags, idle)),
        }
    }

    fn stop(&self) {
        lock(&self.flags).stopped = true;
        self.wake.notify_all();
    }

    fn refresh(&self) {
        lock(&self.flags).refresh = true;
        self.wake.notify_all();
    }

    fn stopped(&self) -> bool {
        lock(&self.flags).stopped
    }

    /// Whether a refresh was asked for since the last call.
    fn take_refresh(&self) -> bool {
        std::mem::take(&mut lock(&self.flags).refresh)
    }
}

/// Waits out a back-off, returning early on `stop` or `refresh`.
trait Sleeper: Send + 'static {
    fn sleep(&self, d: Duration, signal: &Signal);
}

struct WallClock;

impl Sleeper for WallClock {
    fn sleep(&self, d: Duration, signal: &Signal) {
        signal.wait(Some(d));
    }
}

#[derive(Default)]
struct Shared {
    snapshot: Mutex<Snapshot>,
    signal: Signal,
}

/// A live, cached view of one machine's herdr, kept current on its own
/// thread. Dropping it stops the thread.
///
/// The thread blocks while reading the event stream, so `stop` and
/// `refresh` reach it at once while it waits to retry, and otherwise when
/// the stream next delivers a line or ends. Nothing is published after it
/// sees the stop.
pub struct Connection {
    shared: Arc<Shared>,
}

impl Connection {
    pub fn start<T: Transport + 'static>(
        transport: T,
        on_change: impl Fn(&Snapshot) + Send + 'static,
    ) -> Self {
        Self::spawn(transport, WallClock, on_change)
    }

    fn spawn<T: Transport + 'static>(
        transport: T,
        sleeper: impl Sleeper,
        on_change: impl Fn(&Snapshot) + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = Worker {
            client: Client::new(transport),
            shared: shared.clone(),
            sleeper: Box::new(sleeper),
            on_change: Box::new(on_change),
        };
        std::thread::Builder::new()
            .name("herdr-connection".into())
            .spawn(move || worker.run())
            .expect("spawn the herdr connection thread");
        Self { shared }
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.shared.snapshot).clone()
    }

    /// Re-list now; also retries at once from any wait.
    pub fn refresh(&self) {
        self.shared.signal.refresh();
    }

    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shared.signal.stop();
    }
}

/// The status, pane-change and agent-found subscriptions for these panes.
fn subscriptions<'a>(panes: impl IntoIterator<Item = &'a str>) -> Vec<Value> {
    let mut subs: Vec<Value> = panes
        .into_iter()
        .map(|pane| json!({ "type": "pane.agent_status_changed", "pane_id": pane }))
        .collect();
    for kind in ["pane.created", "pane.closed", "pane.agent_detected"] {
        subs.push(json!({ "type": kind }));
    }
    subs
}

fn pane_ids(agents: &[Agent]) -> HashSet<String> {
    agents.iter().map(|a| a.pane_id.clone()).collect()
}

/// What to do after a session ends.
enum Next {
    Retry(String),
    Wait(Online),
}

fn next_step(err: Error) -> Next {
    match err {
        Error::Ssh(SshFailure::Unreachable(why)) => Next::Retry(why),
        Error::Ssh(SshFailure::NeedsSignIn { hint }) => Next::Wait(Online::NeedsSignIn { hint }),
        Error::Ssh(SshFailure::HerdrMissing) => Next::Wait(Online::HerdrMissing),
        Error::Ssh(SshFailure::HerdrTooOld) => Next::Wait(Online::HerdrTooOld),
        other => Next::Retry(other.to_string()),
    }
}

struct Worker<T: Transport> {
    client: Client<T>,
    shared: Arc<Shared>,
    sleeper: Box<dyn Sleeper>,
    on_change: Box<dyn Fn(&Snapshot) + Send>,
}

impl<T: Transport> Worker<T> {
    fn run(self) {
        let signal = &self.shared.signal;
        let mut attempt = 0u32;
        loop {
            // Connecting now covers any refresh asked for until here.
            signal.take_refresh();
            let err = self.session(&mut attempt);
            if signal.stopped() {
                return;
            }
            match next_step(err) {
                Next::Retry(reason) => {
                    let retry_in = backoff(attempt);
                    attempt = attempt.saturating_add(1);
                    self.publish(|s| s.online = Online::Offline { reason, retry_in });
                    self.sleeper.sleep(retry_in, signal);
                }
                Next::Wait(state) => {
                    self.publish(|s| s.online = state);
                    signal.wait(None);
                }
            }
            if signal.stopped() {
                return;
            }
            self.publish(|s| s.online = Online::Connecting);
        }
    }

    /// Apply `change` and, if it changed anything, tell `on_change` after
    /// the lock is released.
    fn publish(&self, change: impl FnOnce(&mut Snapshot)) {
        let changed = {
            let mut snap = lock(&self.shared.snapshot);
            let before = snap.clone();
            change(&mut snap);
            (*snap != before).then(|| snap.clone())
        };
        if let Some(snap) = changed {
            if !self.shared.signal.stopped() {
                (self.on_change)(&snap);
            }
        }
    }

    /// Connect and follow events until something ends the session.
    fn session(&self, attempt: &mut u32) -> Error {
        match self.connect() {
            Ok(events) => {
                *attempt = 0;
                self.follow(events)
            }
            Err(e) => e,
        }
    }

    fn connect(&self) -> Result<Events, Error> {
        let pong = self.client.ping()?;
        self.publish(|s| s.version = Some(pong.version));
        let agents = self.client.agents()?;
        let workspaces = self.client.workspaces()?;
        let events = self
            .client
            .subscribe(&subscriptions(agents.iter().map(|a| a.pane_id.as_str())))?;
        self.publish(|s| {
            s.online = Online::Online;
            s.agents = agents;
            s.workspaces = workspaces;
        });
        Ok(events)
    }

    fn follow(&self, mut events: Events) -> Error {
        loop {
            let event = match events.next() {
                Some(Ok(event)) => event,
                Some(Err(e)) => return e,
                None => return Error::Closed,
            };
            if self.shared.signal.stopped() {
                return Error::Closed;
            }
            let panes_changed = match event {
                Event::AgentStatusChanged {
                    pane_id,
                    agent_status,
                    ..
                } => {
                    self.publish(|s| {
                        if let Some(a) = s.agents.iter_mut().find(|a| a.pane_id == pane_id) {
                            a.agent_status = agent_status;
                        }
                    });
                    false
                }
                Event::PaneClosed { pane_id } => {
                    self.publish(|s| s.agents.retain(|a| a.pane_id != pane_id));
                    false
                }
                Event::PaneCreated { .. } | Event::AgentDetected => true,
                Event::Other(_) => false,
            };
            // Not `||`: a pending refresh is consumed either way.
            if panes_changed | self.shared.signal.take_refresh() {
                // The old stream is dropped only once the new one is up.
                events = match self.resubscribe() {
                    Ok(new) => new,
                    Err(e) => return e,
                };
            }
        }
    }

    /// Open a stream for the current panes, then re-read the lists to cover
    /// whatever happened before it was acknowledged. An agent that turns up
    /// in that re-read gets a stream that includes it.
    fn resubscribe(&self) -> Result<Events, Error> {
        let mut panes = pane_ids(&self.client.agents()?);
        loop {
            let events = self
                .client
                .subscribe(&subscriptions(panes.iter().map(String::as_str)))?;
            let agents = self.client.agents()?;
            let now = pane_ids(&agents);
            if now.is_subset(&panes) {
                let workspaces = self.client.workspaces()?;
                self.publish(|s| {
                    s.agents = agents;
                    s.workspaces = workspaces;
                });
                return Ok(events);
            }
            panes = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{FakeTransport, Step};
    use crate::{AgentStatus, SshFailure};
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::sync::OnceLock;

    const PONG: &str = "{\"id\":\"ping-0\",\"result\":{\"version\":\"0.9.5\",\"protocol\":1}}\n";
    const WORKSPACES: &str =
        "{\"id\":\"x\",\"result\":{\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"main\"}]}}\n";
    const ACK: &str = "{\"id\":\"x\",\"result\":{\"type\":\"subscription_started\"}}\n";
    const DETECTED: &str = "{\"event\":\"pane_agent_detected\",\"data\":{\"pane_id\":\"w2:p1\",\"workspace_id\":\"w2\",\"agent\":\"claude\"}}\n";
    const LONG: Duration = Duration::from_secs(5);

    fn agents(panes: &[(&str, &str)]) -> String {
        let list: Vec<Value> = panes
            .iter()
            .map(|(pane, status)| {
                let ws = pane.split(':').next().unwrap();
                json!({"terminal_id": format!("t-{pane}"), "pane_id": pane, "workspace_id": ws,
                       "tab_id": format!("{ws}:t1"), "agent_status": status})
            })
            .collect();
        format!("{}\n", json!({"id": "x", "result": {"agents": list}}))
    }

    fn status(pane: &str, status: &str) -> String {
        let ws = pane.split(':').next().unwrap();
        format!(
            "{}\n",
            json!({"event": "pane.agent_status_changed",
                   "data": {"pane_id": pane, "workspace_id": ws, "agent_status": status}})
        )
    }

    fn closed(pane: &str) -> String {
        format!(
            "{}\n",
            json!({"event": "pane_closed", "data": {"pane_id": pane, "workspace_id": "w1"}})
        )
    }

    /// What the connection thread did, in the order it did it.
    #[derive(Debug)]
    enum Note {
        Changed(Snapshot),
        Slept(Duration),
    }

    /// Reports each back-off; the first `free` return at once, the rest wait
    /// for `refresh` or `stop` so a test never spins or sleeps for real.
    struct TestSleeper {
        notes: Mutex<mpsc::Sender<Note>>,
        free: Mutex<usize>,
    }

    impl Sleeper for TestSleeper {
        fn sleep(&self, d: Duration, signal: &Signal) {
            let _ = self.notes.lock().unwrap().send(Note::Slept(d));
            let mut free = self.free.lock().unwrap();
            if *free > 0 {
                *free -= 1;
                return;
            }
            drop(free);
            signal.wait(None);
        }
    }

    struct Harness {
        conn: Connection,
        fake: FakeTransport,
        rx: Receiver<Note>,
        seen: RefCell<Vec<Snapshot>>,
        slept: RefCell<Vec<Duration>>,
    }

    fn start(fake: &FakeTransport, free: usize) -> Harness {
        let (tx, rx) = mpsc::channel();
        let sleeper = TestSleeper {
            notes: Mutex::new(tx.clone()),
            free: Mutex::new(free),
        };
        let conn = Connection::spawn(fake.clone(), sleeper, move |s: &Snapshot| {
            let _ = tx.send(Note::Changed(s.clone()));
        });
        Harness {
            conn,
            fake: fake.clone(),
            rx,
            seen: RefCell::default(),
            slept: RefCell::default(),
        }
    }

    impl Harness {
        /// The next published snapshot that matches, failing after a while.
        fn wait_for(&self, what: &str, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
            loop {
                let note = self
                    .rx
                    .recv_timeout(LONG)
                    .unwrap_or_else(|e| panic!("waiting for {what}: {e}; saw {:?}", self.seen));
                match note {
                    Note::Slept(d) => self.slept.borrow_mut().push(d),
                    Note::Changed(s) => {
                        self.seen.borrow_mut().push(s.clone());
                        if done(&s) {
                            return s;
                        }
                    }
                }
            }
        }

        /// Back-offs taken before the last snapshot `wait_for` returned.
        fn slept(&self) -> Vec<Duration> {
            self.slept.borrow().clone()
        }

        fn methods(&self) -> Vec<String> {
            self.fake
                .requests()
                .iter()
                .map(|r| r["method"].as_str().unwrap().to_owned())
                .collect()
        }
    }

    /// Waits for the thread to exit (it owns the senders) and returns what
    /// it reported on the way out.
    fn notes_until_exit(rx: Receiver<Note>) -> Vec<Note> {
        let mut after = Vec::new();
        loop {
            match rx.recv_timeout(LONG) {
                Ok(note) => after.push(note),
                Err(RecvTimeoutError::Disconnected) => return after,
                Err(RecvTimeoutError::Timeout) => panic!("thread did not stop"),
            }
        }
    }

    fn is_online(s: &Snapshot) -> bool {
        s.online == Online::Online
    }

    fn is_offline(s: &Snapshot) -> bool {
        matches!(s.online, Online::Offline { .. })
    }

    fn connected(stream: &str) -> FakeTransport {
        FakeTransport::new(&[PONG, &agents(&[("w1:p1", "idle")]), WORKSPACES, stream])
    }

    fn panes(subscribe: &Value) -> Vec<&str> {
        let mut panes: Vec<&str> = subscribe["params"]["subscriptions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["pane_id"].as_str())
            .collect();
        panes.sort_unstable();
        panes
    }

    #[test]
    fn connect_lists_then_subscribes_per_agent_pane() {
        let h = start(&connected(ACK), 0);
        let snap = h.wait_for("online", is_online);
        assert_eq!(snap.agents.len(), 1);
        assert_eq!(snap.workspaces[0].label, "main");
        assert_eq!(
            h.methods(),
            ["ping", "agent.list", "workspace.list", "events.subscribe"]
        );
        let subs = h.fake.requests().last().unwrap()["params"]["subscriptions"].clone();
        let subs = subs.as_array().unwrap();
        assert!(subs.contains(&json!({"type": "pane.agent_status_changed", "pane_id": "w1:p1"})));
        for t in ["pane.created", "pane.closed", "pane.agent_detected"] {
            assert!(
                subs.contains(&json!({ "type": t })),
                "missing {t}: {subs:?}"
            );
        }
        assert_eq!(subs.len(), 4);
    }

    #[test]
    fn status_event_updates_the_cached_agent() {
        let stream = format!(
            "{ACK}{}{}",
            status("w9:p9", "done"),
            status("w1:p1", "working")
        );
        let h = start(&connected(&stream), 0);
        let snap = h.wait_for("working", |s| {
            s.agents.first().map(|a| a.agent_status) == Some(AgentStatus::Working)
        });
        assert_eq!(snap.agents.len(), 1, "an unknown pane is not added");
        assert!(h.seen.borrow().len() >= 2, "online, then the status change");
        assert_eq!(
            h.conn.snapshot().agents[0].agent_status,
            AgentStatus::Working
        );
    }

    #[test]
    fn agent_detected_relists_and_resubscribes() {
        let both = agents(&[("w1:p1", "idle"), ("w2:p1", "working")]);
        let workspaces2 = "{\"id\":\"x\",\"result\":{\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"main\"},{\"workspace_id\":\"w2\",\"label\":\"side\"}]}}\n";
        let fake = connected(&format!("{ACK}{DETECTED}"));
        for script in [both.as_str(), ACK, &both, workspaces2] {
            fake.push(script);
        }
        let h = start(&fake, 0);
        let snap = h.wait_for("two agents", |s| s.agents.len() == 2);
        assert!(is_online(&snap));
        assert_eq!(snap.workspaces.len(), 2);
        h.wait_for("the new stream to end", is_offline);

        let requests = h.fake.requests();
        assert_eq!(
            h.methods()[4..],
            [
                "agent.list",
                "events.subscribe",
                "agent.list",
                "workspace.list"
            ]
        );
        assert_eq!(panes(&requests[5]), ["w1:p1", "w2:p1"]);
        let log = h.fake.log();
        let at = |step| log.iter().position(|s| *s == step).unwrap();
        assert!(
            at(Step::Closed(3)) > at(Step::Opened(7)),
            "old stream dropped only after the new one is up and the lists re-read: {log:?}"
        );
    }

    #[test]
    fn agent_found_while_resubscribing_is_subscribed_too() {
        let two = agents(&[("w1:p1", "idle"), ("w2:p1", "idle")]);
        let three = agents(&[("w1:p1", "idle"), ("w2:p1", "idle"), ("w3:p1", "idle")]);
        let fake = connected(&format!("{ACK}{DETECTED}"));
        for script in [two.as_str(), ACK, &three, ACK, &three, WORKSPACES] {
            fake.push(script);
        }
        let h = start(&fake, 0);
        h.wait_for("three agents", |s| s.agents.len() == 3);
        let requests = h.fake.requests();
        assert_eq!(panes(&requests[7]), ["w1:p1", "w2:p1", "w3:p1"]);
    }

    #[test]
    fn pane_closed_drops_the_agent() {
        let fake = FakeTransport::new(&[
            PONG,
            &agents(&[("w1:p1", "idle"), ("w2:p1", "idle")]),
            WORKSPACES,
            &format!("{ACK}{}", closed("w1:p1")),
        ]);
        let h = start(&fake, 0);
        let snap = h.wait_for("one agent", |s| s.agents.len() == 1);
        assert_eq!(snap.agents[0].pane_id, "w2:p1");
    }

    #[test]
    fn stream_end_goes_offline_keeps_cache_then_reconnects() {
        let fake = connected(ACK);
        for script in [PONG, &agents(&[("w1:p1", "working")]), WORKSPACES, ACK] {
            fake.push(script);
        }
        let h = start(&fake, 1);
        h.wait_for("online", is_online);
        let offline = h.wait_for("offline", is_offline);
        assert_eq!(
            offline.online,
            Online::Offline {
                reason: "connection closed".into(),
                retry_in: Duration::from_secs(1)
            }
        );
        assert_eq!(offline.agents.len(), 1, "cache kept while offline");
        assert_eq!(offline.version.as_deref(), Some("0.9.5"));
        let back = h.wait_for("online again", is_online);
        assert_eq!(back.agents[0].agent_status, AgentStatus::Working);
        let again = h.wait_for("offline again", is_offline);
        assert!(
            matches!(again.online, Online::Offline { retry_in, .. } if retry_in == Duration::from_secs(1)),
            "back-off resets after a good connect: {again:?}"
        );
        assert_eq!(h.slept(), [Duration::from_secs(1)]);
    }

    #[test]
    fn unreachable_retries_with_growing_backoff() {
        let fake = FakeTransport::default();
        for _ in 0..3 {
            fake.push_failure(SshFailure::Unreachable("no route to host".into()));
        }
        let h = start(&fake, 2);
        let mut waits = Vec::new();
        for _ in 0..3 {
            let s = h.wait_for("offline", is_offline);
            let Online::Offline { reason, retry_in } = s.online else {
                unreachable!()
            };
            assert_eq!(reason, "no route to host");
            waits.push(retry_in.as_secs());
        }
        assert_eq!(waits, [1, 2, 4]);
        assert_eq!(h.slept(), [Duration::from_secs(1), Duration::from_secs(2)]);
        assert_eq!(h.fake.opens(), 3);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(5), Duration::from_secs(32));
        assert_eq!(backoff(6), Duration::from_secs(60));
        assert_eq!(backoff(10), Duration::from_secs(60));
        assert_eq!(backoff(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn sign_in_failure_waits_for_refresh() {
        let fake = FakeTransport::default();
        fake.push_failure(SshFailure::NeedsSignIn {
            hint: "ssh box".into(),
        });
        let h = start(&fake, 0);
        h.wait_for("sign-in", |s| {
            s.online
                == Online::NeedsSignIn {
                    hint: "ssh box".into(),
                }
        });
        assert_eq!(h.fake.opens(), 1);
        for script in [PONG, &agents(&[("w1:p1", "idle")]), WORKSPACES, ACK] {
            fake.push(script);
        }
        h.conn.refresh();
        h.wait_for("connecting", |s| s.online == Online::Connecting);
        h.wait_for("online", is_online);
        assert!(h.slept().is_empty(), "never retried on its own");
        assert_eq!(h.fake.opens(), 5);
    }

    #[test]
    fn herdr_problems_wait_for_refresh() {
        for (failure, want) in [
            (SshFailure::HerdrMissing, Online::HerdrMissing),
            (SshFailure::HerdrTooOld, Online::HerdrTooOld),
        ] {
            let fake = FakeTransport::default();
            fake.push_failure(failure);
            fake.push_failure(SshFailure::Unreachable("down".into()));
            let h = start(&fake, 0);
            h.wait_for("the herdr problem", |s| s.online == want);
            h.conn.refresh();
            h.wait_for("the retry", is_offline);
            assert_eq!(h.slept().len(), 0, "the first wait is not a back-off");
            assert_eq!(h.fake.opens(), 2);
        }
    }

    #[test]
    fn version_comes_from_ping() {
        let h = start(&connected(ACK), 0);
        let snap = h.wait_for("online", is_online);
        assert_eq!(snap.version.as_deref(), Some("0.9.5"));
    }

    #[test]
    fn stop_ends_a_waiting_thread() {
        let fake = FakeTransport::default();
        fake.push_failure(SshFailure::NeedsSignIn {
            hint: "ssh box".into(),
        });
        let h = start(&fake, 0);
        h.wait_for("sign-in", |s| {
            matches!(s.online, Online::NeedsSignIn { .. })
        });
        let Harness { conn, fake, rx, .. } = h;
        conn.stop();
        let after = notes_until_exit(rx);
        assert!(after.is_empty(), "nothing happens after stop: {after:?}");
        assert_eq!(fake.opens(), 1);
    }

    #[test]
    fn dropping_ends_a_backoff_thread() {
        let fake = FakeTransport::default();
        fake.push_failure(SshFailure::Unreachable("down".into()));
        let h = start(&fake, 0);
        h.wait_for("offline", is_offline);
        let Harness { conn, rx, .. } = h;
        drop(conn);
        let notes = notes_until_exit(rx);
        assert!(
            matches!(notes[..], [Note::Slept(_)]),
            "only the back-off it was in: {notes:?}"
        );
    }

    #[test]
    fn wall_clock_sleep_ends_early_on_stop_or_refresh() {
        for refresh in [false, true] {
            let signal = Arc::new(Signal::default());
            let (tx, rx) = mpsc::channel();
            let s = signal.clone();
            std::thread::spawn(move || {
                WallClock.sleep(Duration::from_secs(60), &s);
                tx.send(()).unwrap();
            });
            if refresh {
                signal.refresh();
            } else {
                signal.stop();
            }
            rx.recv_timeout(LONG)
                .expect("woke before the 60 s back-off");
        }
    }

    #[test]
    fn wall_clock_sleep_waits_out_a_short_backoff() {
        let signal = Signal::default();
        let start = std::time::Instant::now();
        WallClock.sleep(Duration::from_millis(20), &signal);
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn on_change_runs_without_the_snapshot_lock() {
        let shared: Arc<OnceLock<Arc<Shared>>> = Arc::default();
        let (tx, rx) = mpsc::channel();
        let (notes, _sleeps) = mpsc::channel();
        let cell = shared.clone();
        // Starts out waiting for a sign-in, so every later change happens
        // with the lock reachable from `on_change`.
        let fake = FakeTransport::default();
        fake.push_failure(SshFailure::NeedsSignIn { hint: "h".into() });
        let conn = Connection::spawn(
            fake.clone(),
            TestSleeper {
                notes: Mutex::new(notes),
                free: Mutex::new(0),
            },
            move |s: &Snapshot| {
                let free = cell
                    .get()
                    .map(|sh| sh.snapshot.try_lock().is_ok())
                    .unwrap_or(true);
                let _ = tx.send((free, s.online.clone()));
            },
        );
        let (_, online) = rx.recv_timeout(LONG).unwrap();
        assert!(matches!(online, Online::NeedsSignIn { .. }));
        shared.set(conn.shared.clone()).ok().unwrap();
        for script in [PONG, &agents(&[("w1:p1", "idle")]), WORKSPACES, ACK] {
            fake.push(script);
        }
        conn.refresh();
        loop {
            let (free, online) = rx.recv_timeout(LONG).unwrap();
            assert!(free, "on_change was called holding the lock");
            if matches!(online, Online::Offline { .. }) {
                break;
            }
        }
    }
}
