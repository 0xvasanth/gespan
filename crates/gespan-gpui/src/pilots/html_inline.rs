//! Hosts the sandboxed page in a system web view on Windows and X11 Linux.
//!
//! Paint only records where the page should be. The web view is built and moved
//! in a task outside paint, because WebView2 pumps window messages while it
//! starts, and that must not reenter GPUI.
use super::html::allows_navigation;
use gpui::{App, Bounds, Context, Pixels, Window};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, WindowHandle as BorrowedHandle,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use wry::{
    NewWindowResponse, Rect, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
};

#[derive(Clone, PartialEq)]
struct Target {
    bounds: Bounds<Pixels>,
    visible: bool,
    html: Arc<str>,
}
#[derive(Default)]
struct State {
    web: Option<WebView>,
    parent: Option<RawWindowHandle>,
    target: Option<Target>,
    shown: Option<Target>,
    queued: bool,
    failed: bool,
}
#[derive(Clone, Default)]
pub(super) struct Inline(Rc<RefCell<State>>);

/// The parent window, for the moment the web view is built.
struct Parent(RawWindowHandle);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<BorrowedHandle<'_>, HandleError> {
        // The handle comes from the window this panel is painting into.
        Ok(unsafe { BorrowedHandle::borrow_raw(self.0) })
    }
}

fn rect(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        position: LogicalPosition::new(f64::from(bounds.left()), f64::from(bounds.top())).into(),
        size: LogicalSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)).into(),
    }
}

/// No message handlers, file base URL, app credentials or persistent website
/// data are installed, the same as the macOS host.
fn build(parent: RawWindowHandle, target: &Target) -> Result<WebView, wry::Error> {
    WebViewBuilder::new()
        .with_html(target.html.to_string())
        .with_bounds(rect(target.bounds))
        .with_visible(target.visible)
        .with_incognito(true)
        .with_devtools(false)
        .with_navigation_handler(|url| allows_navigation(&url))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .build_as_child(&Parent(parent))
}

impl Inline {
    pub(super) fn failed(&self) -> bool {
        self.0.try_borrow().is_ok_and(|state| state.failed)
    }
    /// Asks for the page to be at `bounds`, in window coordinates.
    pub(super) fn show(
        &self,
        bounds: Bounds<Pixels>,
        html: Arc<str>,
        window: &Window,
        panel: gpui::WeakEntity<super::html::Html>,
        cx: &mut App,
    ) {
        let parent = HasWindowHandle::window_handle(window)
            .ok()
            .map(|handle| handle.as_raw());
        let visible = bounds.size.width > gpui::px(0.) && bounds.size.height > gpui::px(0.);
        let target = Target {
            bounds,
            visible,
            html,
        };
        self.request(target, parent, Some(panel), cx);
    }
    pub(super) fn hide(&self, cx: &App) {
        let target = self.0.try_borrow().ok().and_then(|s| s.target.clone());
        if let Some(target) = target.filter(|target| target.visible) {
            let target = Target {
                visible: false,
                ..target
            };
            self.request(target, None, None, cx);
        }
    }
    fn request(
        &self,
        target: Target,
        parent: Option<RawWindowHandle>,
        panel: Option<gpui::WeakEntity<super::html::Html>>,
        cx: &App,
    ) {
        // Busy means a task is applying a change, and the next paint catches up.
        let Ok(mut state) = self.0.try_borrow_mut() else {
            return;
        };
        if state.failed || state.target.as_ref() == Some(&target) {
            return;
        }
        state.target = Some(target);
        state.parent = parent.or(state.parent);
        if std::mem::replace(&mut state.queued, true) {
            return;
        }
        drop(state);
        let state = self.0.clone();
        cx.spawn(async move |cx| {
            let failed = apply(&state);
            if failed && let Some(panel) = panel {
                let _ = panel.update(cx, |_, cx: &mut Context<super::html::Html>| cx.notify());
            }
        })
        .detach();
    }
}

/// Runs outside GPUI's borrows. Returns whether the web view could not start.
fn apply(state: &Rc<RefCell<State>>) -> bool {
    let (target, parent, needs_web) = {
        let mut state = state.borrow_mut();
        state.queued = false;
        let Some(target) = state.target.clone() else {
            return false;
        };
        (target, state.parent, state.web.is_none())
    };
    if needs_web {
        if !target.visible {
            return false;
        }
        let Some(parent) = parent else {
            return false;
        };
        // Nothing stays borrowed here: starting WebView2 runs window messages.
        let built = build(parent, &target);
        let mut state = state.borrow_mut();
        match built {
            Ok(web) => {
                #[cfg(target_os = "linux")]
                gtk_loop::retain();
                state.web = Some(web);
                state.shown = Some(target.clone());
            }
            Err(error) => {
                tracing::warn!(%error, "cannot host an HTML panel");
                state.failed = true;
                return true;
            }
        }
    }
    let mut state = state.borrow_mut();
    let shown = state.shown.clone();
    let Some(web) = state.web.as_ref() else {
        return false;
    };
    // A visible change comes last, so a page never flashes at its old size.
    if shown.as_ref().is_none_or(|old| old.html != target.html) {
        let _ = web.load_html(&target.html);
    }
    if shown.as_ref().is_none_or(|old| old.bounds != target.bounds) {
        let _ = web.set_bounds(rect(target.bounds));
    }
    if shown
        .as_ref()
        .is_none_or(|old| old.visible != target.visible)
    {
        let _ = web.set_visible(target.visible);
    }
    state.shown = Some(target);
    false
}

/// Whether this window can hold a child web view. Wayland cannot: a child
/// surface would need a GTK parent window, and GPUI's is not one.
pub(super) fn can_host(window: &Window) -> bool {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    !matches!(handle.as_raw(), RawWindowHandle::Wayland(_)) && platform_ready()
}

#[cfg(windows)]
fn platform_ready() -> bool {
    true
}
#[cfg(target_os = "linux")]
fn platform_ready() -> bool {
    gtk_loop::ready()
}

#[cfg(target_os = "linux")]
impl Drop for State {
    fn drop(&mut self) {
        if self.web.is_some() {
            gtk_loop::release();
        }
    }
}

/// WebKitGTK needs GTK started on this thread and its events run alongside
/// GPUI's, for as long as a web view is open.
#[cfg(target_os = "linux")]
mod gtk_loop {
    use std::{cell::Cell, time::Duration};

    thread_local! {
        static READY: Cell<Option<bool>> = const { Cell::new(None) };
        static OPEN: Cell<usize> = const { Cell::new(0) };
    }
    pub(super) fn ready() -> bool {
        READY.with(|ready| {
            let started = ready.get().unwrap_or_else(|| gtk::init().is_ok());
            ready.set(Some(started));
            started
        })
    }
    pub(super) fn retain() {
        OPEN.with(|open| open.set(open.get() + 1));
    }
    pub(super) fn release() {
        OPEN.with(|open| open.set(open.get().saturating_sub(1)));
    }
    /// Runs GTK's pending events. Returns whether a web view is still open.
    fn pump() -> bool {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        OPEN.with(Cell::get) > 0
    }
    pub(super) fn run(cx: &gpui::App) {
        let timer = cx.background_executor().clone();
        cx.spawn(async move |_| {
            while pump() {
                timer.timer(Duration::from_millis(16)).await;
            }
        })
        .detach();
    }
}

/// Starts GTK's loop along with the first panel of each run of panels.
#[cfg(target_os = "linux")]
pub(super) fn start_events(cx: &App) {
    gtk_loop::run(cx);
}
#[cfg(windows)]
pub(super) fn start_events(_: &App) {}
