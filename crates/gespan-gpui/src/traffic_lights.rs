//! src-tauri/src/macos.rs's traffic lights: 14 pt buttons, 6 pt apart, 12 pt
//! from the left edge. GPUI's `traffic_light_position` only moves the row and
//! keeps AppKit's own spacing, which on recent macOS crowds the sidebar
//! toggle. Auto Layout pins the buttons, so GPUI's frame updates on resize,
//! focus and fullscreen don't move them back.

use gpui::{App, Window};
use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::Duration;

const BUTTON_SIZE: f64 = 14.;
const LEFT_MARGIN: f64 = 12.;
const BUTTON_SPACING: f64 = 6.;

/// Pins the buttons once; AppKit can recreate them, so call again whenever
/// the window may have rebuilt its titlebar.
pub(crate) fn pin(window: &Window, title_bar_height: f32) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };
    let bottom = (title_bar_height as f64 - BUTTON_SIZE) / 2.;
    let kinds = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ];
    for (i, kind) in kinds.into_iter().enumerate() {
        let Some(button) = ns_window.standardWindowButton(kind) else {
            continue;
        };
        // Already pinned: a second set of constraints would only conflict.
        if !button.translatesAutoresizingMaskIntoConstraints() {
            continue;
        }
        let Some(titlebar) = (unsafe { button.superview() }) else {
            continue;
        };
        let x = LEFT_MARGIN + i as f64 * (BUTTON_SIZE + BUTTON_SPACING);
        button.setTranslatesAutoresizingMaskIntoConstraints(false);
        // Bottom, like GPUI's own origin: the titlebar view's height varies
        // between windowed and fullscreen, the distance from its bottom does not.
        for constraint in [
            button.widthAnchor().constraintEqualToConstant(BUTTON_SIZE),
            button.heightAnchor().constraintEqualToConstant(BUTTON_SIZE),
            button
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&titlebar.leadingAnchor(), x),
            button
                .bottomAnchor()
                .constraintEqualToAnchor_constant(&titlebar.bottomAnchor(), -bottom),
        ] {
            constraint.setActive(true);
        }
    }
}

fn ns_window(window: &Window) -> Option<objc2::rc::Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: GPUI's AppKit handle points at its live content view.
    let view = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    view.window()
}
/// Return keyboard input to GPUI after interacting with an embedded native view.
pub(crate) fn focus_content(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let view = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    if let Some(window) = view.window() {
        window.makeFirstResponder(Some(view));
    }
}
/// Orders the window out; the app keeps running in the menu bar. A
/// full-screen window leaves full screen first: ordered out there, it leaves
/// its Space behind as a black screen that only a force quit clears.
#[cfg(not(test))]
pub(crate) fn hide(window: &Window, cx: &App) {
    hide_then(window, cx, |window| {
        if let Some(window) = ns_window(window) {
            window.orderOut(None);
        }
    });
}

/// How long `hide` waits for AppKit's full-screen exit animation, which
/// takes about half a second, before ordering the window out anyway.
const FULL_SCREEN_EXIT_POLL: Duration = Duration::from_millis(50);
const FULL_SCREEN_EXIT_POLLS: usize = 60;

/// `hide` with the final ordering out passed in, so tests can watch for it.
fn hide_then(window: &Window, cx: &App, order_out: fn(&Window)) {
    if !window.is_fullscreen() {
        order_out(window);
        return;
    }
    window.toggle_fullscreen();
    window
        .spawn(cx, async move |cx| {
            for _ in 0..FULL_SCREEN_EXIT_POLLS {
                cx.background_executor().timer(FULL_SCREEN_EXIT_POLL).await;
                if !matches!(cx.update(|window, _| window.is_fullscreen()), Ok(true)) {
                    break;
                }
            }
            let _ = cx.update(|window, _| order_out(window));
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Context, IntoElement, Render, TestAppContext, div};
    use std::cell::Cell;

    struct Blank;
    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    thread_local! {
        static ORDERED_OUT: Cell<usize> = const { Cell::new(0) };
    }
    fn record(_: &Window) {
        ORDERED_OUT.with(|count| count.set(count.get() + 1));
    }
    fn ordered_out() -> usize {
        ORDERED_OUT.with(Cell::get)
    }

    fn open(cx: &mut TestAppContext, fullscreen: bool) -> gpui::WindowHandle<Blank> {
        ORDERED_OUT.with(|count| count.set(0));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| Blank))
                .unwrap()
        });
        if fullscreen {
            window
                .update(cx, |_, window, _| window.toggle_fullscreen())
                .unwrap();
        }
        window
    }

    #[gpui::test]
    fn a_full_screen_window_leaves_full_screen_before_it_is_ordered_out(cx: &mut TestAppContext) {
        let window = open(cx, true);
        window
            .update(cx, |_, window, cx| hide_then(window, cx, record))
            .unwrap();
        window
            .update(cx, |_, window, _| assert!(!window.is_fullscreen()))
            .unwrap();
        assert_eq!(ordered_out(), 0, "not while the exit animation runs");
        cx.executor().advance_clock(FULL_SCREEN_EXIT_POLL * 2);
        cx.run_until_parked();
        assert_eq!(ordered_out(), 1, "ordered out once full screen has ended");
        cx.executor()
            .advance_clock(FULL_SCREEN_EXIT_POLL * FULL_SCREEN_EXIT_POLLS as u32);
        cx.run_until_parked();
        assert_eq!(ordered_out(), 1, "only once");
    }

    #[gpui::test]
    fn a_window_that_is_not_full_screen_is_ordered_out_at_once(cx: &mut TestAppContext) {
        let window = open(cx, false);
        window
            .update(cx, |_, window, cx| hide_then(window, cx, record))
            .unwrap();
        assert_eq!(ordered_out(), 1);
        window
            .update(cx, |_, window, _| assert!(!window.is_fullscreen()))
            .unwrap();
    }
}
