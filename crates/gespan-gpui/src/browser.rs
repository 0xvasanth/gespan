//! The app's side of the browser module: the state Settings → Browser
//! follows, and the user's commands, which run off the UI thread because
//! most of them wait for the browser.
use gespan_core::browser::{self as core, BrowserHost, BrowserState};
use gpui::{App, AppContext, Entity, Global};
use std::sync::Arc;

/// The host's newest state, and the last command's error.
pub(crate) struct Browser {
    host: Arc<BrowserHost>,
    pub(crate) state: BrowserState,
    pub(crate) error: Option<String>,
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

fn adopt(host: Arc<BrowserHost>, cx: &mut App) -> Entity<Browser> {
    let browser = cx.new(|_| Browser {
        state: host.state(),
        host,
        error: None,
    });
    cx.set_global(Shared(browser.clone()));
    browser
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
