//! A thread's tab shown live in a pane: the view that streams its pictures,
//! the pictures decoded on their own thread as they come (the pane draws
//! one while the next decodes), and the user's input, sent one at a time
//! in the order given.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gespan_browser::live::{LiveEnd, LiveInput, LiveNav, MouseEvent, PanelSize};
use gespan_browser::protocol::{BrowserState, TabId, TabState};
use gespan_browser::BrowserHost;

use super::{session_key, unpack_live, LivePicture};

/// How the view ended, once it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The pane stopped it, or another pane took the tab over.
    Stopped,
    /// The tab closed or left the thread, or the browser went.
    TabGone,
    /// It could not start or broke.
    Failed(String),
}

/// Whether the host knows the view by its name yet, and the size the pane
/// asked for before it did.
#[derive(Default)]
struct Sizing {
    registered: bool,
    pending: Option<PanelSize>,
}

#[derive(Default)]
struct Inputs {
    queue: VecDeque<LiveInput>,
    closed: bool,
}

struct Shared {
    host: Arc<BrowserHost>,
    session: String,
    tab: TabId,
    view: String,
    ended: Mutex<Option<Ended>>,
    /// The view runs: the browser sent its first picture.
    started: AtomicBool,
    stopping: AtomicBool,
    inputs: Mutex<Inputs>,
    input_ready: Condvar,
    /// How the last input sent went, until the pane takes it.
    input_outcome: Mutex<Option<Result<(), String>>>,
    /// The newest decoded picture the pane has not taken yet.
    latest: Mutex<Option<Result<LivePicture, String>>>,
    fresh: Condvar,
    sizing: Mutex<Sizing>,
    /// The pane's threads still running.
    running: Arc<AtomicUsize>,
}

impl Shared {
    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    fn over(&self) -> bool {
        self.stopping()
            || self
                .ended
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
    }

    /// Wake a pane waiting for a picture.
    fn wake(&self) {
        let _latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        self.fresh.notify_all();
    }
}

/// Counts a pane thread as running until it ends, even by a panic.
struct Running(Arc<Shared>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One live view of a tab. Dropping it stops the view at once, without
/// waiting: its threads hold what they need and end on their own.
pub struct LivePane {
    shared: Arc<Shared>,
}

static NEXT_VIEW: AtomicU64 = AtomicU64::new(1);

impl LivePane {
    /// Start showing `tab`, one of `session`'s tabs, at `size`, on the
    /// browser [`super::start`] started. Refused as [`Ended::TabGone`] when
    /// the tab has closed, or [`Ended::Failed`] with why.
    pub fn start(session: i64, tab: TabId, size: PanelSize) -> Result<LivePane, Ended> {
        let host = super::host()
            .ok_or_else(|| Ended::Failed("Browser access is not available.".into()))?;
        LivePane::start_with(host, session, tab, size)
    }

    /// [`Self::start`] on `host`.
    pub fn start_with(
        host: Arc<BrowserHost>,
        session: i64,
        tab: TabId,
        size: PanelSize,
    ) -> Result<LivePane, Ended> {
        LivePane::start_gated(host, session, tab, size, || {})
    }

    /// [`Self::start_with`], the view thread calling `gate` before it asks
    /// the host for the view: tests hold it there.
    fn start_gated(
        host: Arc<BrowserHost>,
        session: i64,
        tab: TabId,
        size: PanelSize,
        gate: impl FnOnce() + Send + 'static,
    ) -> Result<LivePane, Ended> {
        let session = session_key(session);
        // The host refuses another thread's tab too; asking first keeps a
        // pane from ever starting on one.
        match holding(&host.state(), &session, &tab) {
            Holding::Open => {}
            Holding::Gone => return Err(Ended::TabGone),
            Holding::Other => {
                return Err(Ended::Failed("That tab belongs to another thread.".into()));
            }
        }
        let shared = Arc::new(Shared {
            host,
            session,
            tab,
            view: format!(
                "pane-{}-{}",
                std::process::id(),
                NEXT_VIEW.fetch_add(1, Ordering::Relaxed)
            ),
            ended: Mutex::new(None),
            started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            inputs: Mutex::new(Inputs::default()),
            input_ready: Condvar::new(),
            input_outcome: Mutex::new(None),
            latest: Mutex::new(None),
            fresh: Condvar::new(),
            sizing: Mutex::new(Sizing::default()),
            running: Arc::new(AtomicUsize::new(0)),
        });
        // Should a thread not start, dropping the pane stops the others.
        let pane = LivePane { shared };
        pane.spawn("browser-live-view", move |view| {
            // Dropped before the view even began: nothing to start.
            let outcome = if view.stopping() {
                Ok(LiveEnd::Stopped)
            } else {
                gate();
                view.host.live_view_with(
                    &view.session,
                    &view.tab,
                    &view.view,
                    size,
                    || registered(&view),
                    |_| {
                        if !view.started.swap(true, Ordering::AcqRel) {
                            view.wake();
                        }
                    },
                )
            };
            let ended = match outcome {
                Ok(LiveEnd::Stopped) => Ended::Stopped,
                Ok(LiveEnd::TabGone) => Ended::TabGone,
                // The tab closed before the view could begin (a pane shown
                // again after its tab closed out of sight): gone, not broken.
                Err(_)
                    if holding(&view.host.state(), &view.session, &view.tab) == Holding::Gone =>
                {
                    Ended::TabGone
                }
                Err(error) => Ended::Failed(error),
            };
            *view.ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(ended);
            view.wake();
        })
        .map_err(Ended::Failed)?;
        pane.spawn("browser-live-decode", |decoder| decode_pictures(&decoder))
            .map_err(Ended::Failed)?;
        pane.spawn("browser-live-input", |sender| send_inputs(&sender))
            .map_err(Ended::Failed)?;
        Ok(pane)
    }

    fn spawn(
        &self,
        name: &str,
        work: impl FnOnce(Arc<Shared>) + Send + 'static,
    ) -> Result<(), String> {
        let shared = self.shared.clone();
        shared.running.fetch_add(1, Ordering::AcqRel);
        let running = Running(shared.clone());
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _running = running;
                work(shared);
            });
        // On an error the closure, and the count it holds, went with it.
        spawned.map(drop).map_err(|error| error.to_string())
    }

    /// The newest picture the pane has not drawn yet, waiting up to a
    /// second for one; `None` when none came, or the view is over
    /// ([`Self::ended`]). Blocks: call it off the UI thread.
    pub fn take(&self) -> Option<Result<LivePicture, String>> {
        let shared = &self.shared;
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut latest = shared.latest.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(picture) = latest.take() {
                return Some(picture);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || shared.over() {
                return None;
            }
            latest = shared
                .fresh
                .wait_timeout(latest, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// How the view ended, once it has.
    pub fn ended(&self) -> Option<Ended> {
        self.shared
            .ended
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The pane changed size. A size given before the view has begun is
    /// kept until it has.
    pub fn resize(&self, size: PanelSize) {
        let shared = &self.shared;
        let mut sizing = shared.sizing.lock().unwrap_or_else(|e| e.into_inner());
        if sizing.registered {
            shared.host.resize_live(&shared.view, size);
        } else {
            sizing.pending = Some(size);
        }
    }

    /// Queue the user's input for the tab.
    pub fn input(&self, input: LiveInput) {
        let mut inputs = self.shared.inputs.lock().unwrap_or_else(|e| e.into_inner());
        queue_input(&mut inputs.queue, input);
        drop(inputs);
        self.shared.input_ready.notify_one();
    }

    /// How the last input sent since the pane last asked went: taken by
    /// the tab, or refused and why. `None` when none was sent.
    pub fn take_input_outcome(&self) -> Option<Result<(), String>> {
        self.shared
            .input_outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Back, Forward or Reload. Blocks: call it off the UI thread.
    pub fn navigate(&self, nav: &LiveNav) -> Result<(), String> {
        let shared = &self.shared;
        shared.host.live_navigate(&shared.session, &shared.tab, nav)
    }

    pub fn tab(&self) -> &TabId {
        &self.shared.tab
    }

    #[cfg(test)]
    fn running_threads(&self) -> Arc<AtomicUsize> {
        self.shared.running.clone()
    }

    /// Whether a size is still waiting for the view to begin.
    #[cfg(test)]
    fn size_pending(&self) -> bool {
        self.shared
            .sizing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .is_some()
    }

    #[cfg(test)]
    fn view_name(&self) -> &str {
        &self.shared.view
    }
}

impl Drop for LivePane {
    /// Never waits (the UI drops panes): the view sees the stop at its next
    /// look at it, the decoder and the sender when woken, and a view not
    /// known to the host yet sees it as it registers ([`registered`]).
    fn drop(&mut self) {
        let shared = &self.shared;
        shared.stopping.store(true, Ordering::SeqCst);
        shared
            .inputs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        shared.input_ready.notify_all();
        shared.host.stop_live(Some(&shared.view));
        shared.wake();
    }
}

/// The host knows the view by its name now: a stop or a size the pane gave
/// before then, when the host could not take it, takes effect.
fn registered(shared: &Shared) {
    if shared.stopping() {
        shared.host.stop_live(Some(&shared.view));
        return;
    }
    let mut sizing = shared.sizing.lock().unwrap_or_else(|e| e.into_inner());
    sizing.registered = true;
    if let Some(size) = sizing.pending.take() {
        shared.host.resize_live(&shared.view, size);
    }
}

/// Take each picture the browser sends and decode it, keeping only the
/// newest for the pane, from the view's first picture until it is over.
fn decode_pictures(shared: &Shared) {
    // Asking before the view runs, or after it ended, gets nothing at once:
    // wait for its start, and between empty answers, instead of spinning.
    let pause = |wait: Duration| {
        let latest = shared.latest.lock().unwrap_or_else(|e| e.into_inner());
        if !shared.over() {
            let _ = shared.fresh.wait_timeout(latest, wait);
        }
    };
    while !shared.started.load(Ordering::Acquire) {
        if shared.over() {
            return;
        }
        pause(Duration::from_millis(250));
    }
    let mut after = 0;
    while !shared.over() {
        let packed = shared.host.live_take(&shared.view, after);
        if packed.len() < 8 {
            pause(Duration::from_millis(50));
            continue;
        }
        after = u64::from(u32::from_le_bytes(
            packed[4..8].try_into().unwrap_or_default(),
        ));
        let picture = unpack_live(&packed);
        *shared.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(picture);
        shared.fresh.notify_all();
    }
}

/// Whose `tab` is, for `session`'s pane.
#[derive(Debug, PartialEq)]
enum Holding {
    /// One of the session's tabs, open or opening.
    Open,
    /// Another session's.
    Other,
    /// Closed, crashed, taken back, or never there.
    Gone,
}

fn holding(state: &BrowserState, session: &str, tab: &TabId) -> Holding {
    match state.tabs.iter().find(|shown| shown.tab.tab == *tab) {
        Some(shown) if shown.session_id != session => Holding::Other,
        Some(shown) if matches!(shown.tab.state, TabState::Open | TabState::Attaching) => {
            Holding::Open
        }
        _ => Holding::Gone,
    }
}

/// Send queued input until the pane goes. Each input waits until the page
/// has taken it, so moves and wheels that come meanwhile join the queue as
/// one ([`queue_input`]).
fn send_inputs(shared: &Shared) {
    loop {
        let input = {
            let mut inputs = shared.inputs.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if inputs.closed {
                    return;
                }
                if let Some(input) = inputs.queue.pop_front() {
                    break input;
                }
                inputs = shared
                    .input_ready
                    .wait(inputs)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        if shared.stopping() {
            return;
        }
        let sent = shared.host.live_input(&shared.session, &shared.tab, &input);
        *shared
            .input_outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(sent);
    }
}

/// Add `input` to the inputs still waiting. Moves and scrolls that follow
/// one another become one: the pointer's latest place, and the scroll
/// distances added up, so a fast wheel or trackpad never builds a backlog
/// the page works through long after the hand has stopped.
pub fn queue_input(queue: &mut VecDeque<LiveInput>, input: LiveInput) {
    match (queue.back_mut(), &input) {
        (
            Some(LiveInput::Wheel {
                delta_x,
                delta_y,
                modifiers,
                x,
                y,
            }),
            LiveInput::Wheel {
                delta_x: dx,
                delta_y: dy,
                modifiers: m,
                x: nx,
                y: ny,
            },
        ) if modifiers == m => {
            *delta_x += dx;
            *delta_y += dy;
            *x = *nx;
            *y = *ny;
        }
        (
            Some(LiveInput::Mouse {
                event: MouseEvent::Moved,
                buttons,
                modifiers,
                ..
            }),
            LiveInput::Mouse {
                event: MouseEvent::Moved,
                buttons: b,
                modifiers: m,
                ..
            },
        ) if buttons == b && modifiers == m => {
            if let Some(last) = queue.back_mut() {
                *last = input;
            }
        }
        _ => queue.push_back(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::tests::{facts, host_on, wait_for};
    use crate::browser::toolkit::Toolkit;
    use crate::browser::{begin_turn_on, view, AccessMode, ConnectionState, Turn, TurnFacts};
    use gespan_browser::live::MouseButton;
    use gespan_browser::testing::{default_response, Event, FakeCdp, Response};
    use mcp_hub::{Grant, Toolkit as _};
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    fn moved(x: f64, buttons: u32) -> LiveInput {
        LiveInput::Mouse {
            event: MouseEvent::Moved,
            x,
            y: 0.0,
            button: MouseButton::None,
            buttons,
            click_count: 0,
            modifiers: 0,
        }
    }

    fn wheel(dy: f64, modifiers: u32) -> LiveInput {
        LiveInput::Wheel {
            x: 1.0,
            y: 2.0,
            delta_x: 0.0,
            delta_y: dy,
            modifiers,
        }
    }

    #[test]
    fn moves_and_wheels_that_follow_each_other_become_one() {
        let mut queue = VecDeque::new();
        queue_input(&mut queue, moved(1.0, 0));
        queue_input(&mut queue, moved(2.0, 0));
        assert_eq!(queue, [moved(2.0, 0)]);
        queue_input(&mut queue, moved(3.0, 1));
        assert_eq!(queue.len(), 2, "a drag is not a hover");
        queue_input(&mut queue, wheel(10.0, 0));
        queue_input(&mut queue, wheel(5.0, 0));
        assert_eq!(queue.back(), Some(&wheel(15.0, 0)));
        queue_input(&mut queue, wheel(5.0, 8));
        assert_eq!(queue.len(), 4, "a shifted wheel stays apart");
        queue_input(&mut queue, LiveInput::Text { text: "a".into() });
        queue_input(&mut queue, moved(4.0, 0));
        assert_eq!(queue.len(), 6);
        // Text and clicks never merge, even when the same one repeats.
        queue_input(&mut queue, LiveInput::Text { text: "a".into() });
        queue_input(&mut queue, LiveInput::Text { text: "a".into() });
        assert_eq!(queue.len(), 8);
    }

    /// The base64 of a `width`x`height` picture of one colour.
    fn encoded(format: image::ImageFormat, width: u32, height: u32, rgb: [u8; 3]) -> String {
        use base64::Engine as _;
        let mut bytes = Vec::new();
        image::RgbImage::from_pixel(width, height, image::Rgb(rgb))
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    }

    /// The colour of the `n`th moving picture.
    fn colour(n: i64) -> [u8; 3] {
        [200, (n * 40 % 256) as u8, 20]
    }

    /// A browser whose tabs stream `frames` real JPEG pictures, the next one
    /// after each acknowledgement, at the 640x480 panel the tests use (as
    /// 64x48 pictures, the same shape), and a PNG when the page is still.
    fn screencast_browser(frames: i64, stalled: Arc<AtomicBool>) -> FakeCdp {
        let counter = Arc::new(AtomicU64::new(1));
        let jpegs: Vec<String> = (1..=frames.max(1))
            .map(|n| encoded(image::ImageFormat::Jpeg, 64, 48, colour(n)))
            .collect();
        let png = encoded(image::ImageFormat::Png, 64, 48, [1, 2, 3]);
        let viewport = Arc::new(Mutex::new((800i64, 600i64)));
        FakeCdp::start_with(move |received| {
            // A browser that stopped answering.
            if stalled.load(Ordering::Acquire) {
                return Response::Hold;
            }
            let session = received.session.as_deref();
            let frame = |n: i64, size: (i64, i64)| {
                Event::new(
                    "Page.screencastFrame",
                    json!({
                        "data": jpegs[(n - 1) as usize],
                        "sessionId": n,
                        "metadata": {
                            "deviceWidth": size.0, "deviceHeight": size.1,
                            "offsetTop": 0, "pageScaleFactor": 1
                        }
                    }),
                    session,
                )
            };
            match received.method.as_str() {
                "Emulation.setDeviceMetricsOverride" => {
                    *viewport.lock().unwrap() = (
                        received.params["width"].as_i64().unwrap_or(0),
                        received.params["height"].as_i64().unwrap_or(0),
                    );
                    Response::Result(json!({}))
                }
                "Page.getLayoutMetrics" => Response::Result(json!({
                    "cssContentSize": { "x": 0, "y": 0, "width": 800, "height": 5000 },
                    "cssLayoutViewport": { "pageX": 0, "pageY": 0, "clientWidth": 800, "clientHeight": 600 },
                    "cssVisualViewport": { "pageX": 0, "pageY": 0, "clientWidth": 800, "clientHeight": 600, "scale": 1 },
                })),
                "Page.startScreencast" => {
                    Response::ResultThen(json!({}), vec![frame(1, *viewport.lock().unwrap())])
                }
                "Page.captureScreenshot" => Response::Result(json!({ "data": png })),
                "Page.screencastFrameAck" => {
                    let n = received.params["sessionId"].as_i64().unwrap_or(0);
                    if n < frames {
                        Response::ResultThen(
                            json!({}),
                            vec![frame(n + 1, *viewport.lock().unwrap())],
                        )
                    } else {
                        Response::Result(json!({}))
                    }
                }
                _ => default_response(received, &counter),
            }
        })
    }

    fn grant(session: i64) -> Grant {
        Grant {
            session,
            workspace: "/repo".into(),
            toolkits: BTreeSet::from([crate::tool_servers::BROWSER.to_owned()]),
            servers: BTreeSet::new(),
        }
    }

    /// A host on a fake browser streaming `frames` pictures, with sessions 1
    /// and 2 granted, and the tab session 1's agent opened.
    struct Rig {
        server: FakeCdp,
        host: Arc<BrowserHost>,
        kit: Toolkit,
        tab: TabId,
        stalled: Arc<AtomicBool>,
        _turns: Vec<Turn>,
    }

    fn rig(frames: i64) -> Rig {
        let stalled = Arc::new(AtomicBool::new(false));
        let server = screencast_browser(frames, stalled.clone());
        let host = host_on(&server.profile_dir(), AccessMode::Always);
        host.set_toggle(true);
        wait_for(&host, ConnectionState::Connected);
        let typed = |session| TurnFacts {
            session,
            user_typed: true,
            ..facts()
        };
        let turns = vec![
            begin_turn_on(Some(host.clone()), &typed(1)),
            begin_turn_on(Some(host.clone()), &typed(2)),
        ];
        let kit = Toolkit::with_host(host.clone());
        let opened = kit
            .call(&grant(1), "browser_tabs", json!({"action": "open"}))
            .unwrap();
        assert_eq!(opened["isError"], json!(false), "{opened}");
        let state = host.state();
        let tab = view::session_tabs(&state, &session_key(1))
            .next()
            .expect("the agent's tab")
            .tab
            .tab
            .clone();
        Rig {
            server,
            host,
            kit,
            tab,
            stalled,
            _turns: turns,
        }
    }

    fn panel() -> PanelSize {
        PanelSize::new(640, 480, 1.0)
    }

    /// Wait up to `limit` for the pane's view to end.
    fn ended_within(pane: &LivePane, limit: Duration) -> Option<Ended> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(ended) = pane.ended() {
                return Some(ended);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// How the inputs sent went, once the sender has told, for up to five
    /// seconds.
    fn next_outcome(pane: &LivePane) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(outcome) = pane.take_input_outcome() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "no outcome came");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The next picture, skipping empty waits, for up to five seconds.
    fn next_picture(pane: &LivePane) -> LivePicture {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "no picture came");
            if let Some(picture) = pane.take() {
                return picture.expect("the picture decodes");
            }
        }
    }

    #[test]
    fn the_pane_takes_decoded_pictures_in_order() {
        let rig = rig(3);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        assert_eq!(pane.tab(), &rig.tab);
        // The pane gets the newest picture when it asks, so one decoded
        // while it was busy may be skipped, never shown out of order.
        let mut seqs = Vec::new();
        let still = loop {
            let picture = next_picture(&pane);
            // The page's viewport, whatever the picture's own size.
            assert_eq!((picture.width, picture.height), (640.0, 480.0));
            assert_eq!(picture.pixels.dimensions(), (64, 48));
            assert!(seqs.last().is_none_or(|last| picture.seq > *last));
            seqs.push(picture.seq);
            if picture.seq > 3 {
                break picture;
            }
            let [b, g, r, a] = picture.pixels.get_pixel(10, 10).0;
            let [wr, wg, wb] = colour(picture.seq as i64);
            assert!(
                b.abs_diff(wb) < 8 && g.abs_diff(wg) < 8 && r.abs_diff(wr) < 8 && a == 255,
                "picture {} is BGRA of its colour: {b} {g} {r} {a}",
                picture.seq
            );
        };
        assert!(seqs.len() >= 2, "moving pictures came first: {seqs:?}");
        // The still page gets its lossless picture last, decoded the same.
        assert_eq!(still.seq, 4);
        assert_eq!(still.pixels.get_pixel(0, 0).0, [3, 2, 1, 255]);
        assert_eq!(pane.ended(), None);
        assert_eq!(pane.take_input_outcome(), None, "no input, no outcome");
    }

    #[test]
    fn the_agent_closing_the_tab_ends_the_pane_as_tab_gone() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        let closed = rig
            .kit
            .call(
                &grant(1),
                "browser_tabs",
                json!({"action": "close", "tab": rig.tab.0}),
            )
            .unwrap();
        assert_eq!(closed["isError"], json!(false), "{closed}");
        assert_eq!(
            ended_within(&pane, Duration::from_secs(5)),
            Some(Ended::TabGone)
        );
        // Over: asking for a picture answers at once, with none.
        let asked = Instant::now();
        assert!(pane.take().is_none());
        assert!(asked.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn switching_the_browser_off_ends_the_pane_within_a_second() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        rig.host.set_toggle(false);
        let ended = ended_within(&pane, Duration::from_secs(1));
        assert!(ended.is_some(), "the pane still runs after a second");
        assert_ne!(ended, Some(Ended::Stopped), "the pane did not stop it");
    }

    #[test]
    fn a_session_cannot_show_another_sessions_tab() {
        let rig = rig(1);
        let refused = LivePane::start_with(rig.host.clone(), 2, rig.tab.clone(), panel());
        let Some(Ended::Failed(error)) = refused.err() else {
            panic!("session 2 may not show session 1's tab");
        };
        assert!(error.contains("another thread"), "{error}");
        // Nothing reached the browser for it.
        std::thread::sleep(Duration::from_millis(200));
        assert!(!rig
            .server
            .received()
            .iter()
            .any(|r| r.method == "Page.startScreencast"));
        // An unknown tab is refused the same way.
        let unknown =
            LivePane::start_with(rig.host.clone(), 1, TabId("t_00000000".into()), panel());
        assert_eq!(unknown.err(), Some(Ended::TabGone));
    }

    #[test]
    fn a_pane_started_on_a_tab_that_closed_ends_as_tab_gone() {
        let rig = rig(1);
        rig.host.close_tab_by_user(&rig.tab).unwrap();
        // Shown again after its tab closed out of sight: gone, with nothing
        // to try again.
        let refused = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel());
        assert_eq!(refused.err(), Some(Ended::TabGone));
    }

    #[test]
    fn a_tab_that_closes_as_the_view_begins_ends_it_as_tab_gone() {
        let rig = rig(1);
        let (host, tab) = (rig.host.clone(), rig.tab.clone());
        // Open when the pane asks, closed by the time the view starts.
        let pane =
            LivePane::start_gated(rig.host.clone(), 1, rig.tab.clone(), panel(), move || {
                host.close_tab_by_user(&tab).unwrap();
            })
            .unwrap();
        assert_eq!(
            ended_within(&pane, Duration::from_secs(5)),
            Some(Ended::TabGone)
        );
    }

    #[test]
    fn each_input_tells_the_pane_how_it_went() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        let outcome = next_outcome;
        pane.input(LiveInput::Text { text: "a".into() });
        assert_eq!(outcome(&pane), Ok(()));
        assert_eq!(pane.take_input_outcome(), None, "told once");
        // The tab goes: the next input is refused, and the pane hears why.
        rig.host.close_tab_by_user(&rig.tab).unwrap();
        pane.input(LiveInput::Text { text: "b".into() });
        assert!(outcome(&pane).is_err());
    }

    #[test]
    fn no_pane_starts_while_the_browser_is_off() {
        let rig = rig(1);
        rig.host.set_toggle(false);
        wait_for(&rig.host, ConnectionState::Off);
        let refused = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel());
        assert!(refused.is_err());
        assert!(!rig
            .server
            .received()
            .iter()
            .any(|r| r.method == "Page.startScreencast"));
    }

    #[test]
    fn input_reaches_the_page_in_order_with_text_intact() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        pane.input(LiveInput::Mouse {
            event: MouseEvent::Pressed,
            x: 12.0,
            y: 34.0,
            button: MouseButton::Left,
            buttons: 1,
            click_count: 1,
            modifiers: 0,
        });
        pane.input(LiveInput::Text {
            text: "héllo, 世界 👋".into(),
        });
        assert!(rig.server.wait_for(
            |log| log.iter().any(|r| r.method == "Input.insertText"),
            Duration::from_secs(5)
        ));
        let sent = rig.server.received();
        let click = sent
            .iter()
            .position(|r| r.method == "Input.dispatchMouseEvent")
            .expect("the click");
        let text = sent
            .iter()
            .position(|r| r.method == "Input.insertText")
            .unwrap();
        assert!(click < text, "input keeps its order");
        assert_eq!(sent[click].params["x"], 12.0);
        assert_eq!(sent[text].params["text"], "héllo, 世界 👋");
        assert_eq!(next_outcome(&pane), Ok(()));
    }

    /// Wait up to five seconds for every thread of a dropped pane to end.
    fn threads_end(threads: &AtomicUsize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while threads.load(Ordering::Acquire) > 0 {
            assert!(
                Instant::now() < deadline,
                "{} pane threads still run",
                threads.load(Ordering::Acquire)
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn count(server: &FakeCdp, method: &str) -> usize {
        server
            .received()
            .iter()
            .filter(|r| r.method == method)
            .count()
    }

    #[test]
    fn dropping_the_pane_stops_its_view_and_its_threads() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        let threads = pane.running_threads();
        assert_eq!(threads.load(Ordering::Acquire), 3);
        let view = pane.view_name().to_string();
        drop(pane);
        // The threads end on their own, soon after.
        threads_end(&threads);
        assert!(rig
            .server
            .received()
            .iter()
            .any(|r| r.method == "Page.stopScreencast"));
        // The host holds no view for it any more.
        assert!(rig.host.live_take(&view, 0).is_empty());
        // The agent's tab stays open.
        let state = rig.host.state();
        assert!(view::session_tabs(&state, &session_key(1)).any(
            |t| t.tab.tab == rig.tab && t.tab.state == gespan_browser::protocol::TabState::Open
        ));
    }

    #[test]
    fn dropping_a_pane_never_waits_on_a_stalled_browser() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        rig.stalled.store(true, Ordering::Release);
        // The sender is stuck in this click: the browser never answers it.
        pane.input(LiveInput::Mouse {
            event: MouseEvent::Pressed,
            x: 1.0,
            y: 1.0,
            button: MouseButton::Left,
            buttons: 1,
            click_count: 1,
            modifiers: 0,
        });
        assert!(rig.server.wait_for(
            |log| log.iter().any(|r| r.method == "Input.dispatchMouseEvent"),
            Duration::from_secs(5)
        ));
        let threads = pane.running_threads();
        let dropped = Instant::now();
        drop(pane);
        let took = dropped.elapsed();
        assert!(took < Duration::from_millis(50), "the drop took {took:?}");
        assert!(
            threads.load(Ordering::Acquire) > 0,
            "the sender is still stuck"
        );
    }

    #[test]
    fn a_pane_dropped_while_its_view_registers_stops_it() {
        // The view thread is past its own look at the stop, the host does
        // not know the view yet: the drop's stop finds nothing to stop.
        let rig = rig(1);
        let (release, held) = std::sync::mpsc::channel::<()>();
        let pane =
            LivePane::start_gated(rig.host.clone(), 1, rig.tab.clone(), panel(), move || {
                let _ = held.recv();
            })
            .unwrap();
        let threads = pane.running_threads();
        drop(pane);
        release.send(()).unwrap();
        threads_end(&threads);
        // It saw the stop as it registered, and never showed the tab.
        assert_eq!(count(&rig.server, "Page.startScreencast"), 0);
        assert_eq!(count(&rig.server, "Emulation.setDeviceMetricsOverride"), 0);
    }

    #[test]
    fn starting_then_dropping_at_once_never_leaves_a_view_running() {
        let rig = rig(1);
        for _ in 0..200 {
            let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
            let threads = pane.running_threads();
            drop(pane);
            // A view left running would keep its thread, and the tab's
            // pictures, going.
            threads_end(&threads);
        }
        // Each screencast started was stopped, and its viewport given back.
        let started = count(&rig.server, "Page.startScreencast");
        assert!(count(&rig.server, "Page.stopScreencast") >= started);
        assert!(count(&rig.server, "Emulation.clearDeviceMetricsOverride") >= started);
    }

    #[test]
    fn a_size_given_before_the_view_begins_is_kept_for_it() {
        let rig = rig(1);
        let (release, held) = std::sync::mpsc::channel::<()>();
        let pane =
            LivePane::start_gated(rig.host.clone(), 1, rig.tab.clone(), panel(), move || {
                let _ = held.recv();
            })
            .unwrap();
        pane.resize(PanelSize::new(800, 600, 1.0));
        assert!(
            pane.size_pending(),
            "the host has no view to give it to yet"
        );
        release.send(()).unwrap();
        assert!(rig.server.wait_for(
            |log| log
                .iter()
                .any(|r| r.method == "Emulation.setDeviceMetricsOverride"
                    && r.params["width"] == 800),
            Duration::from_secs(5)
        ));
        assert!(!pane.size_pending());
        // Sizes after that go straight to the view.
        pane.resize(PanelSize::new(1000, 750, 1.0));
        assert!(!pane.size_pending());
        assert!(rig.server.wait_for(
            |log| log
                .iter()
                .any(|r| r.method == "Emulation.setDeviceMetricsOverride"
                    && r.params["width"] == 1000),
            Duration::from_secs(5)
        ));
    }

    #[test]
    fn a_resized_pane_restarts_the_pictures_at_its_new_size() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        pane.resize(PanelSize::new(800, 600, 1.0));
        assert!(rig.server.wait_for(
            |log| log
                .iter()
                .filter(|r| r.method == "Page.startScreencast")
                .count()
                == 2,
            Duration::from_secs(5)
        ));
        let metrics: Vec<_> = rig
            .server
            .received()
            .into_iter()
            .filter(|r| r.method == "Emulation.setDeviceMetricsOverride")
            .map(|r| r.params["width"].as_i64())
            .collect();
        assert_eq!(metrics.last(), Some(&Some(800)));
    }
}
