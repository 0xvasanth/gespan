//! The app's side of the browser module: the state Settings → Browser
//! follows, and the user's commands, which run off the UI thread because
//! most of them wait for the browser.
use gespan_core::browser::{self as core, BrowserHost, BrowserState};
use gpui::{App, AppContext, Entity, Global};
use std::{collections::HashSet, sync::Arc};

pub(crate) use core::session_key;

/// The host's newest state, and the last command's error.
pub(crate) struct Browser {
    host: Arc<BrowserHost>,
    pub(crate) state: BrowserState,
    pub(crate) error: Option<String>,
    /// Sessions whose agent tab already showed beside the thread on its
    /// own: that happens once a session, for as long as the app runs.
    auto_shown: HashSet<i64>,
}

struct Shared(Entity<Browser>);
impl Global for Shared {}

pub(crate) fn get(cx: &App) -> Option<Entity<Browser>> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

/// The state now, when the module runs.
pub(crate) fn state(cx: &App) -> Option<BrowserState> {
    get(cx).map(|browser| browser.read(cx).state.clone())
}

/// The host the app follows: the running module's, or a test's own.
pub(crate) fn host(cx: &App) -> Option<Arc<BrowserHost>> {
    get(cx).map(|browser| browser.read(cx).host.clone())
}

/// Whether `session`'s agent tab may still show beside the thread on its
/// own; `true` only the first time it is asked for each session.
pub(crate) fn claim_auto_show(cx: &mut App, session: i64) -> bool {
    get(cx).is_some_and(|browser| browser.update(cx, |b, _| b.auto_shown.insert(session)))
}

/// Follow the running browser module. After `gespan_core::browser::start`;
/// without a module, Settings → Browser says it could not start.
pub(crate) fn install(cx: &mut App) {
    let Some(host) = core::host() else {
        return;
    };
    let browser = adopt(host, cx);
    let receive = core::subscribe();
    browser.update(cx, |_, cx| {
        cx.spawn(async move |this, cx| {
            // Any change reads the host again, so a burst draws once.
            while receive.recv().await.is_ok() {
                while receive.try_recv().is_ok() {}
                let updated = this.update(cx, |browser: &mut Browser, cx| {
                    browser.state = browser.host.state();
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    });
}

/// Show a test's own host. It is not followed: its connection thread's
/// changes would wake the UI from another thread, which the test scheduler
/// rejects. The page reads the host again after each command.
#[cfg(test)]
pub(crate) fn install_host(host: Arc<BrowserHost>, cx: &mut App) -> Entity<Browser> {
    adopt(host, cx)
}

/// Read a test's own host again, as following it would after a change its
/// connection thread made.
#[cfg(test)]
pub(crate) fn refresh(cx: &mut App) {
    if let Some(browser) = get(cx) {
        browser.update(cx, |browser, cx| {
            browser.state = browser.host.state();
            cx.notify();
        });
    }
}

fn adopt(host: Arc<BrowserHost>, cx: &mut App) -> Entity<Browser> {
    let browser = cx.new(|_| Browser {
        state: host.state(),
        host,
        error: None,
        auto_shown: HashSet::new(),
    });
    cx.set_global(Shared(browser.clone()));
    browser
}

/// Whether `session`'s running turn carries the browser, from the host
/// itself: the state the page follows may still lag a turn that just began.
pub(crate) fn turn_carries(cx: &App, session: i64) -> bool {
    get(cx).is_some_and(|browser| core::turn_carries(&browser.read(cx).host.state(), session))
}

/// Run one of the user's commands on the host, off the UI thread and one
/// at a time; an error shows in Settings → Browser.
pub(crate) fn command(
    cx: &mut App,
    work: impl FnOnce(&Arc<BrowserHost>) -> Result<(), String> + Send + 'static,
) {
    let Some(browser) = get(cx) else {
        return;
    };
    let host = browser.read(cx).host.clone();
    let task = cx
        .background_executor()
        .spawn(async move { host.serialized(|| work(&host)) });
    cx.spawn(async move |cx| {
        let result = task.await;
        browser.update(cx, |browser, cx| {
            browser.error = result.err();
            browser.state = browser.host.state();
            cx.notify();
        });
    })
    .detach();
}

/// A web link clicked in a thread, for the shell to open in that thread's
/// in-app browser. `session: None` is the thread the user is looking at (a
/// link in the dock's terminal).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = browser, no_json)]
pub(crate) struct OpenThreadLink {
    pub(crate) session: Option<i64>,
    pub(crate) url: String,
}

/// An `http(s)` address: what the in-app browser opens. Everything else
/// (`mailto:`, `file:`, `gespan://`, …) keeps its own handling.
pub(crate) fn is_web(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// A web link the user clicked in a thread: in the thread's in-app browser,
/// or the system browser for Cmd+click (Ctrl+click off macOS, `system`) and
/// anything that is not `http(s)`. `thread: None` (no thread around the
/// link) is the system browser as well.
pub(crate) fn open_link_in(
    thread: Option<i64>,
    url: &str,
    system: bool,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    match thread {
        Some(session) if !system && is_web(url) => window.dispatch_action(
            Box::new(OpenThreadLink {
                session: Some(session),
                url: url.to_owned(),
            }),
            cx,
        ),
        _ => cx.open_url(url),
    }
}

/// Read the host again after one of the app's own commands changed it.
pub(crate) fn reread(cx: &mut App) {
    if let Some(browser) = get(cx) {
        browser.update(cx, |browser, cx| {
            browser.state = browser.host.state();
            cx.notify();
        });
    }
}
