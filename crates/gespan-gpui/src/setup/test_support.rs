//! Opens a setup card in a test window and records what it emits.
use super::{
    SetupStep,
    card::{SetupCard, SetupEvent},
};
use crate::settings::Preferences;
use gpui::{
    Bounds, Entity, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
    point, prelude::*, px, size,
};
use std::{cell::RefCell, rc::Rc};

pub(crate) struct Opened {
    pub window: WindowHandle<SetupCard>,
    pub card: Entity<SetupCard>,
    pub events: Rc<RefCell<Vec<SetupEvent>>>,
}

/// A 1440×900 window holding a card on `step`, launched from `launch` with
/// home `/Users/a`.
pub(crate) fn open(step: SetupStep, launch: &str, cx: &mut TestAppContext) -> Opened {
    let events = Rc::new(RefCell::new(Vec::new()));
    let launch = launch.to_string();
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1440.), px(900.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    SetupCard::new(step, Preferences::default(), launch, "/Users/a".into(), cx)
                })
            },
        )
        .unwrap()
    });
    let card = window.root(cx).unwrap();
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&card, move |_, event: &SetupEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    cx.run_until_parked();
    Opened {
        window,
        card,
        events,
    }
}

/// Clicks the element tagged with debug selector `selector`.
pub(crate) fn click(opened: &Opened, selector: &'static str, cx: &mut TestAppContext) {
    let visual = &mut VisualTestContext::from_window(opened.window.into(), cx);
    visual.run_until_parked();
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no element tagged {selector}"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
}

/// Whether an element tagged `selector` is on screen.
pub(crate) fn has(opened: &Opened, selector: &'static str, cx: &mut TestAppContext) -> bool {
    let visual = &mut VisualTestContext::from_window(opened.window.into(), cx);
    visual.run_until_parked();
    visual.debug_bounds(selector).is_some()
}
