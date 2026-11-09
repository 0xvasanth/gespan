//! The pane against a fake browser: the tab's picture is drawn, the
//! user's clicks and typing reach the tab, and a tab that goes away ends
//! the pane with its message.
use super::*;
use crate::browser_test_rig::Rig;
use gpui::{
    Bounds, KeyDownEvent, Keystroke, TestAppContext, VisualTestContext, WindowBounds, WindowHandle,
    WindowOptions, point, size,
};
use std::time::{Duration, Instant};

/// `pane` alone in a window of its own.
struct Host {
    pane: Entity<BrowserPane>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.pane.clone())
    }
}

fn pane_window(
    rig: &Rig,
    session: i64,
    tab: TabId,
    cx: &mut TestAppContext,
) -> (Entity<BrowserPane>, WindowHandle<Host>, VisualTestContext) {
    let host = rig.host.clone();
    let (window, pane) = cx.update(|cx| {
        crate::input::bind(cx);
        crate::browser::install_host(host, cx);
        let mut pane = None;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(800.), px(600.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    let made = cx.new(|cx| BrowserPane::new(session, tab, cx));
                    pane = Some(made.clone());
                    cx.new(|_| Host { pane: made })
                },
            )
            .unwrap();
        (window, pane.unwrap())
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    // The first frame measures the stage; the next starts the view.
    visual.refresh().unwrap();
    visual.run_until_parked();
    (pane, window, visual)
}

/// Take what the view sends until `done`, for up to five seconds.
fn pull_until(
    pane: &Entity<BrowserPane>,
    visual: &mut VisualTestContext,
    done: impl Fn(&BrowserPane) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if visual.update(|_, cx| done(pane.read(cx))) {
            break;
        }
        if Instant::now() >= deadline {
            let (live, ended, size, error) = visual.update(|_, cx| {
                let pane = pane.read(cx);
                (
                    pane.live.is_some(),
                    pane.ended.clone(),
                    pane.size,
                    pane.error.clone(),
                )
            });
            panic!(
                "the pane never got there: live {live}, ended {ended:?}, size {size:?}, error {error:?}"
            );
        }
        visual.update(|_, cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx)));
        // Drawn between pulls, as on screen.
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
    visual.run_until_parked();
    visual.refresh().unwrap();
}

#[gpui::test]
fn the_tab_is_drawn_and_a_click_lands_where_it_was_made(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(7);
    let (pane, _window, mut visual) = pane_window(&rig, 7, tab, cx);
    assert!(
        visual.debug_bounds("browser-pane-picture").is_none(),
        "nothing is drawn before the first picture"
    );
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    let drawn = visual
        .debug_bounds("browser-pane-picture")
        .expect("the tab's picture is drawn");
    let (width, height) = visual.update(|_, cx| {
        let shown = pane.read(cx).picture.as_ref().unwrap();
        (shown.width, shown.height)
    });
    assert!(width > 100. && height > 100., "{width}x{height}");
    // A click a quarter of the way into the picture is a quarter of the way
    // into the page, in the page's own pixels.
    let at = point(
        drawn.left() + drawn.size.width / 4.,
        drawn.top() + drawn.size.height / 4.,
    );
    visual.simulate_click(at, Default::default());
    visual.run_until_parked();
    let near = |value: &serde_json::Value, want: f64| {
        value.as_f64().is_some_and(|got| (got - want).abs() <= 1.)
    };
    for kind in ["mousePressed", "mouseReleased"] {
        assert!(
            rig.received("Input.dispatchMouseEvent", |p| {
                p["type"] == kind && near(&p["x"], width / 4.) && near(&p["y"], height / 4.)
            }),
            "{kind} at ({}, {}): {:?}",
            width / 4.,
            height / 4.,
            rig.server
                .received()
                .iter()
                .filter(|r| r.method == "Input.dispatchMouseEvent")
                .map(|r| r.params.clone())
                .collect::<Vec<_>>()
        );
    }
}

#[gpui::test]
fn non_ascii_typed_in_the_pane_reaches_the_tab_intact(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(8);
    let (pane, _window, mut visual) = pane_window(&rig, 8, tab, cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    // Clicking the page gives the pane the keys.
    let drawn = visual.debug_bounds("browser-pane-picture").unwrap();
    visual.simulate_click(drawn.center(), Default::default());
    visual.run_until_parked();
    for typed in ["é", "日本語", "😀"] {
        visual.simulate_event(KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Default::default(),
                key: "a".into(),
                key_char: Some(typed.into()),
            },
            is_held: false,
            prefer_character_input: false,
        });
        visual.run_until_parked();
        assert!(
            rig.received("Input.insertText", |p| p["text"] == typed),
            "{typed:?} reached the tab as it was typed"
        );
    }
}

#[gpui::test]
fn a_tab_the_agent_closes_ends_the_pane_with_its_message(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(9);
    let (pane, _window, mut visual) = pane_window(&rig, 9, tab.clone(), cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    rig.host.close_tab_by_user(&tab).unwrap();
    pull_until(&pane, &mut visual, |pane| pane.ended.is_some());
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.ended, Some(Ended::TabGone));
        assert_eq!(
            pane.ended_message(),
            Some(("This tab closed or left the thread.".to_owned(), false))
        );
    });
    assert!(visual.debug_bounds("browser-pane-ended").is_some());
    assert!(
        visual.debug_bounds("browser-pane-retry").is_none(),
        "a closed tab has nothing to show again"
    );
}

#[gpui::test]
fn another_threads_tab_never_shows(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let theirs = rig.open_tab(11);
    let (pane, _window, mut visual) = pane_window(&rig, 12, theirs, cx);
    visual.run_until_parked();
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert!(
            matches!(pane.ended, Some(Ended::Failed(_))),
            "{:?}",
            pane.ended
        );
        assert!(pane.picture.is_none());
    });
    assert!(visual.debug_bounds("browser-pane-ended").is_some());
}
