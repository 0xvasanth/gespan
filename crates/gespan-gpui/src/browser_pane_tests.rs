//! The pane against a fake browser: the tab's picture is drawn, the
//! user's clicks and typing reach the tab, and a tab that goes away ends
//! the pane with its message.
use super::*;
use crate::browser_test_rig::Rig;
use gpui::{
    Bounds, Entity, KeyDownEvent, Keystroke, MouseButton, TestAppContext, VisualTestContext,
    WindowBounds, WindowHandle, WindowOptions, point, size,
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
                    pane.error.as_ref().map(|error| error.text.clone()),
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

#[gpui::test]
fn a_shown_pane_over_a_still_page_keeps_its_view_however_long(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(13);
    let (pane, _window, mut visual) = pane_window(&rig, 13, tab, cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    // The page stays still, and nothing draws the pane again, for longer
    // than any wait inside the view or the pane.
    std::thread::sleep(Duration::from_millis(2500));
    visual.update(|_, cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx)));
    // Then the page moves: its next picture is taken and drawn.
    rig.push_frame(50);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !visual.update(|_, cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx))) {
        assert!(Instant::now() < deadline, "the next picture never came");
    }
    visual.update(|_, cx| assert!(pane.read(cx).is_live(), "the view still runs"));
    let started = || {
        rig.server
            .received()
            .iter()
            .filter(|r| r.method == "Page.startScreencast")
            .count()
    };
    assert_eq!(
        started(),
        1,
        "the same view all along, never stopped and started again"
    );
    visual.run_until_parked();
    visual.refresh().unwrap();
    // And the user's click still reaches the tab.
    let drawn = visual.debug_bounds("browser-pane-picture").unwrap();
    visual.simulate_click(drawn.center(), Default::default());
    visual.run_until_parked();
    assert!(rig.received("Input.dispatchMouseEvent", |p| p["type"] == "mousePressed"));
}

/// The mouse events the tab got, as CDP params.
fn mouse_events(rig: &Rig, kind: &str) -> Vec<serde_json::Value> {
    rig.server
        .received()
        .iter()
        .filter(|r| r.method == "Input.dispatchMouseEvent" && r.params["type"] == kind)
        .map(|r| r.params.clone())
        .collect()
}

#[gpui::test]
fn a_button_let_go_outside_the_pane_lets_go_in_the_page(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(14);
    let (pane, _window, mut visual) = pane_window(&rig, 14, tab, cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    let drawn = visual.debug_bounds("browser-pane-picture").unwrap();
    let toolbar = visual.debug_bounds("browser-pane-address").unwrap();
    // A let-go over the toolbar that never went down in the page is not
    // the page's.
    visual.simulate_mouse_up(toolbar.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    // A drag from the page's middle, let go over the toolbar above it.
    visual.simulate_mouse_down(drawn.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert!(rig.received("Input.dispatchMouseEvent", |p| p["type"] == "mousePressed"));
    visual.update(|_, cx| assert_eq!(pane.read(cx).buttons, 1, "held"));
    visual.simulate_mouse_up(toolbar.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert!(
        rig.received("Input.dispatchMouseEvent", |p| p["type"] == "mouseReleased"),
        "the page let go"
    );
    let released = mouse_events(&rig, "mouseReleased");
    assert_eq!(
        released.len(),
        1,
        "only the page's own button: {released:?}"
    );
    let up = &released[0];
    assert_eq!(up["button"], "left");
    assert_eq!(up["buttons"], 0, "no button held after");
    // At the picture's edge nearest the pointer: its top.
    assert_eq!(up["y"].as_f64(), Some(0.), "{up}");
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.buttons, 0);
        assert!(pane.press.is_none());
    });
    // The next move over the page is a hover, not the drag going on.
    visual.simulate_mouse_move(drawn.center(), None, Default::default());
    visual.run_until_parked();
    assert!(rig.received("Input.dispatchMouseEvent", |p| {
        p["type"] == "mouseMoved" && p["buttons"] == 0
    }));
}

#[gpui::test]
fn the_error_line_clears_once_a_picture_or_an_input_goes_through(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(15);
    let (pane, _window, mut visual) = pane_window(&rig, 15, tab, cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    let error = |visual: &mut VisualTestContext| {
        visual.update(|_, cx| pane.read(cx).error.as_ref().map(|e| e.text.clone()))
    };
    // A refused input shows; a picture right after leaves it up to be read.
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.accept_for_tests((None, None, Some(Err("The browser refused it".into()))), cx)
        })
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("browser-pane-error").is_some());
    rig.push_frame(60);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !visual.update(|_, cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx))) {
        assert!(Instant::now() < deadline, "no picture came");
    }
    assert_eq!(
        error(&mut visual).as_deref(),
        Some("The browser refused it")
    );
    // Up long enough, the next picture clears it.
    visual.update(|_, cx| {
        pane.update(cx, |pane, _| {
            pane.set_error_for_tests("old", Duration::from_secs(4))
        })
    });
    rig.push_frame(61);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !visual.update(|_, cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx))) {
        assert!(Instant::now() < deadline, "no picture came");
    }
    assert_eq!(error(&mut visual), None, "a picture came: the error went");
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("browser-pane-error").is_none());
    // An input the tab took clears it too, and a refused one does not.
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.set_error_for_tests("old", Duration::from_secs(4));
            pane.accept_for_tests((None, None, Some(Err("refused again".into()))), cx);
        })
    });
    assert_eq!(error(&mut visual).as_deref(), Some("refused again"));
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.set_error_for_tests("old", Duration::from_secs(4));
            pane.accept_for_tests((None, None, Some(Ok(()))), cx);
        })
    });
    assert_eq!(error(&mut visual), None, "an input went through");
    // A click the real tab takes goes the whole way and clears it as well.
    visual.update(|_, cx| {
        pane.update(cx, |pane, _| {
            pane.set_error_for_tests("old", Duration::from_secs(4))
        })
    });
    let drawn = visual.debug_bounds("browser-pane-picture").unwrap();
    visual.simulate_click(drawn.center(), Default::default());
    visual.run_until_parked();
    let deadline = Instant::now() + Duration::from_secs(5);
    while error(&mut visual).is_some() {
        assert!(
            Instant::now() < deadline,
            "the click never cleared the error"
        );
        visual.update(|_, cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx)));
    }
}

#[gpui::test]
fn a_pane_shown_again_after_its_tab_closed_says_the_tab_is_gone(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(16);
    let (pane, _window, mut visual) = pane_window(&rig, 16, tab.clone(), cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    // Out of sight, the view stops; the tab closes meanwhile.
    visual.update(|_, cx| pane.update(cx, |pane, cx| pane.set_shown(false, cx)));
    visual.run_until_parked();
    visual.update(|_, cx| assert!(!pane.read(cx).is_live()));
    rig.host.close_tab_by_user(&tab).unwrap();
    visual.update(|_, cx| pane.update(cx, |pane, cx| pane.set_shown(true, cx)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    pull_until(&pane, &mut visual, |pane| pane.ended.is_some());
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.ended, Some(Ended::TabGone));
        assert_eq!(
            pane.ended_message(),
            Some(("This tab closed or left the thread.".to_owned(), false))
        );
    });
    assert!(
        visual.debug_bounds("browser-pane-retry").is_none(),
        "nothing to show again"
    );
}

#[gpui::test]
fn the_address_is_shown_and_never_typed_into(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let tab = rig.open_tab(17);
    let (pane, _window, mut visual) = pane_window(&rig, 17, tab, cx);
    pull_until(&pane, &mut visual, |pane| pane.picture.is_some());
    let address = visual
        .debug_bounds("browser-pane-address")
        .expect("the tab's address shows");
    // Clicked and typed at, the address takes nothing: no keys reach the
    // page and the tab goes nowhere.
    visual.simulate_click(address.center(), Default::default());
    visual.run_until_parked();
    for key in ["e", "enter"] {
        visual.simulate_event(KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Default::default(),
                key: key.into(),
                key_char: (key == "e").then(|| "e".into()),
            },
            is_held: false,
            prefer_character_input: false,
        });
        visual.run_until_parked();
    }
    std::thread::sleep(Duration::from_millis(200));
    let sent: Vec<_> = rig
        .server
        .received()
        .iter()
        .map(|r| r.method.clone())
        .collect();
    assert!(!sent.iter().any(|m| m == "Page.navigate"), "{sent:?}");
    assert!(!sent.iter().any(|m| m == "Input.insertText"), "{sent:?}");
    assert!(
        !sent.iter().any(|m| m == "Input.dispatchKeyEvent"),
        "{sent:?}"
    );
    visual.update(|_, cx| assert!(pane.read(cx).error.is_none()));
}
