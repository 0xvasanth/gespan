//! A thread's tab shown live in a pane: the view that streams its pictures,
//! the pictures decoded on their own thread as they come (the pane draws
//! one while the next decodes), and the user's input, sent one at a time
//! in the order given.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gespan_browser::live::{LiveEnd, LiveInput, LiveNav, MouseEvent, PanelSize};
use gespan_browser::protocol::TabId;
use gespan_browser::BrowserHost;

use super::{session_key, unpack_live, view, LivePicture};

/// How long dropping a pane waits for its threads before it lets them
/// finish on their own: a browser that stopped answering must not hold
/// the UI.
const JOIN_WAIT: Duration = Duration::from_secs(2);

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
    /// The last input that failed, for the pane to show.
    input_error: Mutex<Option<String>>,
    /// The newest decoded picture the pane has not taken yet.
    latest: Mutex<Option<Result<LivePicture, String>>>,
    fresh: Condvar,
    /// The pane's threads still running.
    running: Arc<AtomicUsize>,
    /// Signalled as each of them ends.
    gone: Condvar,
    gone_lock: Mutex<()>,
}

impl Shared {
    fn over(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
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
        let _gone = self.0.gone_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.0.running.fetch_sub(1, Ordering::AcqRel);
        self.0.gone.notify_all();
    }
}

/// One live view of a tab. Dropping it stops the view and waits for its
/// threads.
pub struct LivePane {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

static NEXT_VIEW: AtomicU64 = AtomicU64::new(1);

impl LivePane {
    /// Start showing `tab`, one of `session`'s tabs, at `size`, on the
    /// browser [`super::start`] started.
    pub fn start(session: i64, tab: TabId, size: PanelSize) -> Result<LivePane, String> {
        let host = super::host().ok_or("Browser access is not available.")?;
        LivePane::start_with(host, session, tab, size)
    }

    /// [`Self::start`] on `host`.
    pub fn start_with(
        host: Arc<BrowserHost>,
        session: i64,
        tab: TabId,
        size: PanelSize,
    ) -> Result<LivePane, String> {
        let session = session_key(session);
        // The host refuses another thread's tab too; asking first keeps a
        // pane from ever starting on one.
        let state = host.state();
        if !view::session_tabs(&state, &session).any(|shown| shown.tab.tab == tab) {
            return Err("That tab belongs to another thread, or is gone.".into());
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
            input_error: Mutex::new(None),
            latest: Mutex::new(None),
            fresh: Condvar::new(),
            running: Arc::new(AtomicUsize::new(0)),
            gone: Condvar::new(),
            gone_lock: Mutex::new(()),
        });
        // Should a thread not start, dropping the pane stops the others.
        let mut pane = LivePane {
            shared: shared.clone(),
            threads: Vec::with_capacity(3),
        };
        pane.spawn("browser-live-view", move |view| {
            let outcome = view
                .host
                .live_view(&view.session, &view.tab, &view.view, size, |_| {
                    if !view.started.swap(true, Ordering::AcqRel) {
                        view.wake();
                    }
                });
            let ended = match outcome {
                Ok(LiveEnd::Stopped) => Ended::Stopped,
                Ok(LiveEnd::TabGone) => Ended::TabGone,
                Err(error) => Ended::Failed(error),
            };
            *view.ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(ended);
            view.wake();
        })?;
        pane.spawn("browser-live-decode", |decoder| decode_pictures(&decoder))?;
        pane.spawn("browser-live-input", |sender| send_inputs(&sender))?;
        Ok(pane)
    }

    fn spawn(
        &mut self,
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
        match spawned {
            Ok(handle) => {
                self.threads.push(handle);
                Ok(())
            }
            // The closure, and the count it holds, went with the error.
            Err(error) => Err(error.to_string()),
        }
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

    /// The pane changed size.
    pub fn resize(&self, size: PanelSize) {
        self.shared.host.resize_live(&self.shared.view, size);
    }

    /// Queue the user's input for the tab.
    pub fn input(&self, input: LiveInput) {
        let mut inputs = self.shared.inputs.lock().unwrap_or_else(|e| e.into_inner());
        queue_input(&mut inputs.queue, input);
        drop(inputs);
        self.shared.input_ready.notify_one();
    }

    /// The last input the tab refused, once.
    pub fn take_input_error(&self) -> Option<String> {
        self.shared
            .input_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Back, Forward, Reload or an address. Blocks: call it off the UI
    /// thread.
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

    #[cfg(test)]
    fn view_name(&self) -> &str {
        &self.shared.view
    }
}

impl Drop for LivePane {
    fn drop(&mut self) {
        let shared = &self.shared;
        shared.stopping.store(true, Ordering::Release);
        shared
            .inputs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        shared.input_ready.notify_all();
        shared.host.stop_live(Some(&shared.view));
        shared.wake();
        // Each thread ends within a moment of the stop: the view at its
        // next look at it, the decoder and the sender when woken.
        let deadline = Instant::now() + JOIN_WAIT;
        let mut gone = shared.gone_lock.lock().unwrap_or_else(|e| e.into_inner());
        while shared.running.load(Ordering::Acquire) > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            gone = shared
                .gone
                .wait_timeout(gone, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(gone);
        let late = shared.running.load(Ordering::Acquire);
        if late > 0 {
            tracing::warn!(
                threads = late,
                view = %shared.view,
                "the live pane's threads are still ending; left to finish"
            );
            return;
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
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
        if shared.stopping.load(Ordering::Acquire) {
            return;
        }
        let sent = shared.host.live_input(&shared.session, &shared.tab, &input);
        if let Err(error) = sent {
            *shared.input_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
        }
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
    use std::sync::atomic::AtomicU64;

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
    fn screencast_browser(frames: i64) -> FakeCdp {
        let counter = Arc::new(AtomicU64::new(1));
        let jpegs: Vec<String> = (1..=frames.max(1))
            .map(|n| encoded(image::ImageFormat::Jpeg, 64, 48, colour(n)))
            .collect();
        let png = encoded(image::ImageFormat::Png, 64, 48, [1, 2, 3]);
        let viewport = Arc::new(Mutex::new((800i64, 600i64)));
        FakeCdp::start_with(move |received| {
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
        _turns: Vec<Turn>,
    }

    fn rig(frames: i64) -> Rig {
        let server = screencast_browser(frames);
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
        let mut seqs = Vec::new();
        while seqs.len() < 3 {
            let picture = next_picture(&pane);
            // The page's viewport, whatever the picture's own size.
            assert_eq!((picture.width, picture.height), (640.0, 480.0));
            assert_eq!(picture.pixels.dimensions(), (64, 48));
            if picture.seq <= 3 {
                let [b, g, r, a] = picture.pixels.get_pixel(10, 10).0;
                let [wr, wg, wb] = colour(picture.seq as i64);
                assert!(
                    b.abs_diff(wb) < 8 && g.abs_diff(wg) < 8 && r.abs_diff(wr) < 8 && a == 255,
                    "picture {} is BGRA of its colour: {b} {g} {r} {a}",
                    picture.seq
                );
            }
            assert!(seqs.last().is_none_or(|last| picture.seq > *last));
            seqs.push(picture.seq);
        }
        assert_eq!(seqs, [1, 2, 3]);
        // The still page gets its lossless picture next, decoded the same.
        let still = next_picture(&pane);
        assert_eq!(still.seq, 4);
        assert_eq!(still.pixels.get_pixel(0, 0).0, [3, 2, 1, 255]);
        assert_eq!(pane.ended(), None);
        assert!(pane.take_input_error().is_none());
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
        let error = refused
            .err()
            .expect("session 2 may not show session 1's tab");
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
        assert!(unknown.is_err());
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
        assert!(pane.take_input_error().is_none());
    }

    #[test]
    fn dropping_the_pane_stops_its_view_and_its_threads() {
        let rig = rig(1);
        let pane = LivePane::start_with(rig.host.clone(), 1, rig.tab.clone(), panel()).unwrap();
        next_picture(&pane);
        let threads = pane.running_threads();
        assert_eq!(threads.load(Ordering::Acquire), 3);
        let view = pane.view_name().to_string();
        let dropped = Instant::now();
        drop(pane);
        assert!(dropped.elapsed() < Duration::from_secs(2));
        // Every thread has ended by the time the drop returns.
        assert_eq!(threads.load(Ordering::Acquire), 0);
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
