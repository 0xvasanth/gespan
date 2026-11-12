//! The development benchmark driver. `bun run bench` (scripts/bench.mjs)
//! launches the dev build with `GESPAN_BENCH=<mode>` and an isolated data
//! directory. This seeds that directory, drives real workspace actions on the
//! main window, and prints one `GESPAN_BENCH {json}` line per event on stderr,
//! next to GPUI's own `frame duration:` lines (`ZED_MEASUREMENTS=1`). The
//! runner turns both into results. Debug builds only, like `dev_reload`.
//!
//! Modes:
//! - `startup`: wait for the first input round trip, sample idle memory, quit.
//! - `interaction`: startup, then the measured phases (tab switching, long
//!   chat scroll, panes) and `GESPAN_BENCH_CYCLES` open/close cycles for leaks.
//! - `soak`: startup, then loop the phases for `GESPAN_BENCH_SOAK_SECS`.
use gespan_core::{store::Store, transcript::Transcript};
use gpui::{
    Action, AnyWindowHandle, App, AsyncApp, Modifiers, MouseMoveEvent, PlatformInput, ScrollDelta,
    ScrollWheelEvent, TouchPhase, point, px,
};
use std::{
    io::Write,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MODE: &str = "GESPAN_BENCH";
/// Seeded chats, one tab each. The first is the long one.
pub(crate) const TABS: usize = 8;
/// User turns in the long chat; every fifth reply is the tool-heavy fixture.
const LONG_CHAT_TURNS: usize = 250;
/// A frame that never comes (an occluded or minimized window) must not hang
/// the run; it is reported instead.
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);
/// Splits create their session on a background thread before the pane opens.
const ASYNC_SETTLE: Duration = Duration::from_millis(350);
const STEP_GAP: Duration = Duration::from_millis(80);
const SOAK_MEMORY_EVERY: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    Startup,
    Interaction { cycles: usize },
    Soak { secs: u64 },
}

impl Mode {
    pub(crate) fn from_env() -> Option<Self> {
        let get = |key: &str| std::env::var(key).ok();
        Self::parse(
            get(MODE).as_deref(),
            get("GESPAN_BENCH_CYCLES").as_deref(),
            get("GESPAN_BENCH_SOAK_SECS").as_deref(),
        )
    }

    fn parse(mode: Option<&str>, cycles: Option<&str>, soak_secs: Option<&str>) -> Option<Self> {
        match mode? {
            "startup" => Some(Self::Startup),
            "interaction" => Some(Self::Interaction {
                cycles: cycles.and_then(|c| c.parse().ok()).unwrap_or(20),
            }),
            "soak" => Some(Self::Soak {
                secs: soak_secs.and_then(|s| s.parse().ok()).unwrap_or(15 * 60),
            }),
            _ => None,
        }
    }
}

/// Fills an empty isolated database with the benchmark's chats and a saved
/// workspace that opens all of them as tabs, the long chat first and active.
/// A database that already has sessions is left alone, so a relaunch inside
/// one soak keeps the state the crash left behind.
pub(crate) fn seed(db: &mut Store, cwd: &str) -> Result<(), String> {
    if !db.sessions()?.is_empty() {
        return Ok(());
    }
    let mut ids = Vec::with_capacity(TABS);
    for i in 0..TABS {
        let provider = if i % 2 == 0 { "claude" } else { "codex" };
        let session = db.create(provider, cwd)?;
        let (title, rows) = if i == 0 {
            ("Bench · long chat", long_chat().rows)
        } else {
            ("Bench · chat", short_chat().rows)
        };
        db.title(session.id, &format!("{title} {}", i + 1))?;
        db.write_rows(session.id, 0, &rows)?;
        ids.push(session.id);
    }
    db.save_ui_state(&workspace_state(&ids, cwd))
}

fn workspace_state(ids: &[i64], cwd: &str) -> String {
    let state = crate::workspace::Workspace {
        tabs: ids.to_vec(),
        active: ids.first().copied(),
        project: cwd.into(),
        projects: vec![cwd.into()],
        sidebar_visible: true,
        ..Default::default()
    };
    serde_json::to_string(&state).expect("the workspace serializes")
}

/// A few turns of the tool-heavy fixture. Each starts with a prompt: a chat
/// without one opens on the empty new-thread view, which would make tab
/// switching measure nothing.
pub(crate) fn short_chat() -> Transcript {
    let mut t = Transcript::default();
    for turn in 0..3 {
        t.begin("user");
        t.append(&format!("Turn {turn}: run the tests and fix what fails."));
        t.rows.extend(crate::fixtures::chat().rows);
    }
    t
}

pub(crate) fn long_chat() -> Transcript {
    let mut t = Transcript::default();
    for turn in 0..LONG_CHAT_TURNS {
        t.begin("user");
        t.append(&format!(
            "Turn {turn}: check the pane layout code for regressions and explain what changed."
        ));
        if turn % 5 == 0 {
            t.rows.extend(crate::fixtures::chat().rows);
        } else {
            t.begin("assistant");
            t.append(&format!(
                "Here is turn {turn}.\n\n- The **layout** keeps its split ratios.\n- Closing a pane focuses its neighbour.\n\n```rust\nfn ratio(turn: usize) -> f32 {{\n    (turn % 7) as f32 / 7.0\n}}\n```\n\nThe tests in `pane_layout.rs` cover both cases."
            ));
        }
    }
    t
}

/// One scripted input.
#[derive(Debug)]
pub(crate) enum Step {
    Act(&'static str, Box<dyn Action>),
    /// Wheel pixels; positive scrolls toward older messages.
    Scroll(f32),
    /// Waits without measuring, for work that finishes off the main thread.
    Settle(Duration),
}

fn act(name: &'static str, action: impl Action) -> Step {
    Step::Act(name, Box::new(action))
}

pub(crate) fn tab_round() -> Vec<Step> {
    use crate::shell::{ActivateTab, NextTab};
    let mut steps: Vec<_> = (0..TABS).map(|_| act("next_tab", NextTab)).collect();
    steps.extend((0..TABS).map(|i| act("activate_tab", ActivateTab(i as isize))));
    steps
}

pub(crate) fn scroll_round(events: usize) -> Vec<Step> {
    use crate::shell::ActivateTab;
    let mut steps = vec![
        act("activate_long_chat", ActivateTab(0)),
        Step::Settle(ASYNC_SETTLE),
    ];
    steps.extend((0..events).map(|_| Step::Scroll(240.)));
    steps.extend((0..events).map(|_| Step::Scroll(-240.)));
    steps
}

/// Opens two splits, the dock terminal and the right panel, then closes all
/// of them, so a cycle ends on the layout it started from.
pub(crate) fn pane_cycle() -> Vec<Step> {
    use crate::shell::{CloseTab, SplitDown, SplitRight, ToggleRightPanel, ToggleTerminal};
    vec![
        act("split_right", SplitRight),
        Step::Settle(ASYNC_SETTLE),
        act("split_down", SplitDown),
        Step::Settle(ASYNC_SETTLE),
        act("terminal_open", ToggleTerminal),
        Step::Settle(ASYNC_SETTLE),
        act("terminal_close", ToggleTerminal),
        act("right_panel_open", ToggleRightPanel),
        act("right_panel_close", ToggleRightPanel),
        act("close_pane", CloseTab),
        act("close_pane", CloseTab),
        Step::Settle(ASYNC_SETTLE),
    ]
}

fn emit(event: &str, mut fields: serde_json::Value) {
    fields["event"] = event.into();
    fields["wall_ms"] = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.)
        .into();
    // A protocol line for the runner, not a log line: it must reach stderr
    // in order with GPUI's frame lines.
    let _ = writeln!(std::io::stderr().lock(), "{MODE} {fields}");
}

pub(crate) fn install(cx: &mut App) {
    let Some(mode) = Mode::from_env() else {
        return;
    };
    emit(
        "app_run",
        serde_json::json!({ "mode": format!("{mode:?}") }),
    );
    cx.spawn(async move |cx| {
        let window = loop {
            if let Some(window) = cx.update(|cx| cx.windows().first().copied()) {
                break window;
            }
            cx.background_executor()
                .timer(Duration::from_millis(5))
                .await;
        };
        let mut driver = Driver {
            window,
            frame_timeouts: 0,
        };
        driver.run(mode, cx).await;
        emit(
            "done",
            serde_json::json!({ "frame_timeouts": driver.frame_timeouts }),
        );
        cx.update(crate::quit);
    })
    .detach();
}

struct Driver {
    window: AnyWindowHandle,
    frame_timeouts: usize,
}

impl Driver {
    async fn run(&mut self, mode: Mode, cx: &mut AsyncApp) {
        let _ = self
            .window
            .update(cx, |_, window, _| window.activate_window());
        cx.update(|cx| cx.activate(true));
        // The first input the shell answers with a painted frame.
        let started = Instant::now();
        let probe = self
            .step(&act("probe", crate::shell::DismissMenu), cx)
            .await;
        emit(
            "interactive",
            serde_json::json!({ "probe_ms": probe, "waited_ms": ms(started.elapsed()) }),
        );
        self.idle(Duration::from_secs(3), cx).await;
        self.memory("idle");
        self.layout("start", cx);
        match mode {
            Mode::Startup => {}
            Mode::Interaction { cycles } => {
                self.phase(
                    "tab_switch",
                    &[tab_round(), tab_round(), tab_round()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>(),
                    cx,
                )
                .await;
                self.phase("chat_scroll", &scroll_round(60), cx).await;
                let panes: Vec<_> = (0..5).flat_map(|_| pane_cycle()).collect();
                self.phase("panes", &panes, cx).await;
                self.layout("after_phases", cx);
                self.idle(Duration::from_secs(2), cx).await;
                self.memory("before_cycles");
                for cycle in 1..=cycles {
                    self.phase(
                        "cycle",
                        &[pane_cycle(), tab_round()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>(),
                        cx,
                    )
                    .await;
                    if cycle % 5 == 0 || cycle == cycles {
                        self.memory(&format!("cycle_{cycle}"));
                    }
                }
                self.layout("after_cycles", cx);
                self.idle(Duration::from_secs(3), cx).await;
                self.memory("after_cycles");
            }
            Mode::Soak { secs } => {
                let deadline = Instant::now() + Duration::from_secs(secs);
                let mut last_memory = Instant::now();
                let mut iteration = 0usize;
                while Instant::now() < deadline {
                    iteration += 1;
                    let steps = match iteration % 3 {
                        0 => tab_round(),
                        1 => scroll_round(20),
                        _ => pane_cycle(),
                    };
                    self.phase("soak", &steps, cx).await;
                    if last_memory.elapsed() >= SOAK_MEMORY_EVERY {
                        last_memory = Instant::now();
                        self.memory(&format!("soak_{iteration}"));
                        self.layout("soak", cx);
                    }
                }
                self.memory("soak_end");
                emit("soak_end", serde_json::json!({ "iterations": iteration }));
            }
        }
    }

    async fn phase(&mut self, name: &str, steps: &[Step], cx: &mut AsyncApp) {
        emit("phase_start", serde_json::json!({ "phase": name }));
        for step in steps {
            match step {
                Step::Settle(wait) => cx.background_executor().timer(*wait).await,
                _ => {
                    let latency = self.step(step, cx).await;
                    let active = self
                        .window
                        .update(cx, |_, window, _| window.is_window_active())
                        .unwrap_or(false);
                    emit(
                        "step",
                        serde_json::json!({
                            "phase": name,
                            "step": step_name(step),
                            "latency_ms": latency,
                            "active": active,
                        }),
                    );
                    if !matches!(step, Step::Scroll(_)) {
                        cx.background_executor().timer(STEP_GAP).await;
                    }
                }
            }
        }
        emit("phase_end", serde_json::json!({ "phase": name }));
    }

    /// Input to the start of the frame after the one that painted it.
    async fn step(&mut self, step: &Step, cx: &mut AsyncApp) -> f64 {
        let started = Instant::now();
        let _ = self.window.update(cx, |_, window, cx| match step {
            Step::Act(_, action) => window.dispatch_action(action.boxed_clone(), cx),
            Step::Scroll(dy) => {
                let size = window.viewport_size();
                // Over the transcript: right of the sidebar, mid-height.
                let position = point(size.width * 0.62, size.height * 0.5);
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(0.), px(*dy))),
                        modifiers: Modifiers::default(),
                        touch_phase: TouchPhase::Moved,
                    }),
                    cx,
                );
            }
            Step::Settle(_) => {}
        });
        self.frames(2, cx).await;
        ms(started.elapsed())
    }

    async fn frames(&mut self, count: usize, cx: &mut AsyncApp) {
        for _ in 0..count {
            let (tx, rx) = async_channel::bounded(2);
            let timeout = tx.clone();
            let executor = cx.background_executor().clone();
            cx.background_executor()
                .spawn(async move {
                    executor.timer(FRAME_TIMEOUT).await;
                    let _ = timeout.try_send(false);
                })
                .detach();
            let scheduled = self.window.update(cx, |_, window, _| {
                window.on_next_frame(move |_, _| {
                    let _ = tx.try_send(true);
                })
            });
            if scheduled.is_err() || !rx.recv().await.unwrap_or(false) {
                self.frame_timeouts += 1;
                return;
            }
        }
    }

    async fn idle(&mut self, wait: Duration, cx: &mut AsyncApp) {
        cx.background_executor().timer(wait).await;
        self.frames(1, cx).await;
    }

    fn memory(&self, label: &str) {
        let (resident, footprint) = process_memory();
        #[cfg(target_os = "macos")]
        let gpu = gpui_apple::metal_renderer::gpu_memory().allocated;
        #[cfg(not(target_os = "macos"))]
        let gpu = 0u64;
        let mb = |bytes: u64| (bytes as f64 / (1024. * 1024.) * 10.).round() / 10.;
        emit(
            "memory",
            serde_json::json!({
                "label": label,
                "rss_mb": mb(resident),
                "footprint_mb": mb(footprint),
                "gpu_mb": mb(gpu),
            }),
        );
    }

    /// The open tabs and split panes, so a cycle that leaves something behind
    /// shows up instead of skewing the leak numbers.
    fn layout(&self, label: &str, cx: &mut AsyncApp) {
        let layout = self.window.update(cx, |_, window, cx| {
            let shell = window.root::<crate::shell::Shell>().flatten()?;
            let workspace = &shell.read(cx).workspace;
            let panes: usize = workspace
                .tabs
                .iter()
                .map(|tab| workspace.layout(*tab).leaves().len())
                .sum();
            Some((workspace.tabs.len(), panes))
        });
        let (tabs, panes) = layout.ok().flatten().unwrap_or_default();
        emit(
            "layout",
            serde_json::json!({ "label": label, "tabs": tabs, "panes": panes }),
        );
    }
}

fn step_name(step: &Step) -> &'static str {
    match step {
        Step::Act(name, _) => name,
        Step::Scroll(dy) if *dy > 0. => "scroll_up",
        Step::Scroll(_) => "scroll_down",
        Step::Settle(_) => "settle",
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.
}

/// Resident size and the footprint Activity Monitor shows as Memory.
#[cfg(target_os = "macos")]
fn process_memory() -> (u64, u64) {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer is a rusage_info_v2, the flavor this call fills.
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast(),
        )
    };
    if status != 0 {
        return (0, 0);
    }
    // SAFETY: a successful call initialized the whole struct.
    let info = unsafe { info.assume_init() };
    (info.ri_resident_size, info.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn process_memory() -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_with_their_defaults() {
        assert_eq!(Mode::parse(None, None, None), None);
        assert_eq!(Mode::parse(Some("release"), None, None), None);
        assert_eq!(
            Mode::parse(Some("startup"), None, None),
            Some(Mode::Startup)
        );
        assert_eq!(
            Mode::parse(Some("interaction"), None, None),
            Some(Mode::Interaction { cycles: 20 })
        );
        assert_eq!(
            Mode::parse(Some("interaction"), Some("3"), None),
            Some(Mode::Interaction { cycles: 3 })
        );
        assert_eq!(
            Mode::parse(Some("soak"), None, Some("90")),
            Some(Mode::Soak { secs: 90 })
        );
        // A typo falls back to the default instead of a zero-length soak.
        assert_eq!(
            Mode::parse(Some("soak"), None, Some("15m")),
            Some(Mode::Soak { secs: 900 })
        );
    }

    fn count(steps: &[Step], name: &str) -> usize {
        steps.iter().filter(|s| step_name(s) == name).count()
    }

    /// Leak numbers only mean something if every cycle returns to the
    /// layout it started from.
    #[test]
    fn a_pane_cycle_closes_everything_it_opens() {
        let steps = pane_cycle();
        let opened = count(&steps, "split_right") + count(&steps, "split_down");
        assert_eq!(opened, 2);
        assert_eq!(count(&steps, "close_pane"), opened);
        assert_eq!(
            count(&steps, "terminal_open"),
            count(&steps, "terminal_close")
        );
        assert_eq!(
            count(&steps, "right_panel_open"),
            count(&steps, "right_panel_close")
        );
        // A split's session is created off the main thread: the next step
        // waits for it.
        for (ix, step) in steps.iter().enumerate() {
            if matches!(step_name(step), "split_right" | "split_down") {
                assert!(matches!(steps[ix + 1], Step::Settle(_)));
            }
        }
    }

    #[test]
    fn a_scroll_round_returns_to_where_it_started() {
        let steps = scroll_round(60);
        let total: f32 = steps
            .iter()
            .filter_map(|s| match s {
                Step::Scroll(dy) => Some(*dy),
                _ => None,
            })
            .sum();
        assert_eq!(total, 0.);
        assert_eq!(count(&steps, "scroll_up"), 60);
        assert_eq!(step_name(&steps[0]), "activate_long_chat");
    }

    #[test]
    fn a_tab_round_visits_every_seeded_tab() {
        let steps = tab_round();
        assert_eq!(count(&steps, "next_tab"), TABS);
        assert_eq!(count(&steps, "activate_tab"), TABS);
    }

    #[test]
    fn seeding_opens_every_chat_as_a_tab_with_the_long_chat_active() {
        let mut db = Store::open_gespan(std::path::Path::new(":memory:")).unwrap();
        let cwd = "/tmp/gespan-bench-project";
        seed(&mut db, cwd).unwrap();
        let sessions = db.sessions().unwrap();
        assert_eq!(sessions.len(), TABS);
        let saved = db.ui_state().unwrap();
        let metadata: Vec<_> = sessions.iter().map(|s| (s.id, s.cwd.clone())).collect();
        let workspace = crate::workspace::Workspace::restore(saved.as_deref(), &metadata, cwd);
        assert_eq!(workspace.tabs.len(), TABS);
        let long = sessions
            .iter()
            .find(|s| s.title.contains("long chat"))
            .unwrap();
        assert_eq!(workspace.active, Some(long.id));
        assert_eq!(workspace.tabs[0], long.id);
        // Long enough to scroll for a while: hundreds of messages.
        let rows = db.load(long.id).unwrap().rows;
        assert!(rows.iter().filter(|r| r.first).count() >= 2 * LONG_CHAT_TURNS);
        // Every chat opens on its transcript, not the empty new-thread view.
        for session in &sessions {
            let rows = db.load(session.id).unwrap().rows;
            assert_eq!(rows.first().map(|r| r.role.as_str()), Some("user"));
            assert!(rows.iter().any(|r| r.role == "assistant"));
        }
        // The seeded workspace is not first-run setup.
        assert!(!crate::setup::rules::setup_pending(saved.as_deref()));
    }

    #[test]
    fn seeding_leaves_an_existing_database_alone() {
        let mut db = Store::open_gespan(std::path::Path::new(":memory:")).unwrap();
        let cwd = "/tmp/gespan-bench-project";
        let kept = db.create("codex", cwd).unwrap();
        seed(&mut db, cwd).unwrap();
        let sessions = db.sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, kept.id);
    }
}
