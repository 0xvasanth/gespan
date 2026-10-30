//! The Windows tray icon and Close to tray (see `crate::tray`).
use super::*;

#[cfg(windows)]
struct Installed(#[allow(dead_code)] tray_icon::TrayIcon);
#[cfg(windows)]
impl gpui::Global for Installed {}

impl Shell {
    /// Show Gespan brings the window back; Quit Gespan shows it, then quits as
    /// the menu does, asking first when chats are running.
    #[cfg(windows)]
    pub(super) fn start_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (actions, received) = async_channel::unbounded();
        match crate::tray::install(actions) {
            Ok(icon) => cx.set_global(Installed(icon)),
            Err(error) => {
                tracing::warn!(%error, "cannot add the tray icon");
                return;
            }
        }
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(action) = received.recv().await {
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.set_window_visible(window, true, cx);
                    if matches!(action, crate::tray::Action::Quit) {
                        shell.request_quit(window, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Hides the window, or shows and focuses it: through the tray on
    /// Windows, and on macOS as the red button and the Dock icon do.
    pub(super) fn set_window_visible(
        &mut self,
        window: &mut Window,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = HasWindowHandle::window_handle(window)
                && let RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                crate::tray::set_visible(win32.hwnd.get(), visible);
            }
        }
        #[cfg(all(target_os = "macos", not(test)))]
        if !visible {
            crate::traffic_lights::hide(window, cx);
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        let _ = cx;
        if visible {
            window.activate_window();
        }
    }
}

/// The Dock icon, or launching Gespan again while it runs: bring back the
/// workspace window a close hid. Floating Pilot and Quick Composer windows
/// stay as they are.
pub(crate) fn reopen(cx: &mut App) {
    for window in cx.windows() {
        if let Some(shell) = window.downcast::<Shell>() {
            let _ = shell.update(cx, |shell, window, cx| {
                shell.set_window_visible(window, true, cx)
            });
        }
    }
    cx.activate(true);
}
