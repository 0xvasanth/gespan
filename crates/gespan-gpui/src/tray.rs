//! Close to tray on Windows: closing the window hides it so running agents
//! keep going, and the tray icon is the way back (src-tauri/src/tray.rs and
//! the close handler in src/app/App.tsx). Other platforms have no tray.

/// What closing the main window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Close {
    /// Hide to the tray; agents keep running.
    Hide,
    /// Ask first, as a quit does.
    Ask,
    /// Close, which quits.
    Quit,
}

/// `to_tray` is the Close to tray setting, which only Windows offers.
pub(crate) fn on_close(to_tray: bool, running_chats: bool, unsaved_files: bool) -> Close {
    if to_tray {
        Close::Hide
    } else if running_chats || unsaved_files {
        Close::Ask
    } else {
        Close::Quit
    }
}

#[cfg(windows)]
pub(crate) use windows_tray::{Action, install, set_visible};

#[cfg(windows)]
mod windows_tray {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    const SHOW: &str = "tray-show";
    const QUIT: &str = "tray-quit";

    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Action {
        Show,
        Quit,
    }

    /// Show Gespan and Quit Gespan on the menu; a left click shows the window.
    /// Events are delivered on the UI thread's message loop and forwarded to
    /// `actions`. Keep the returned icon alive: dropping it removes the icon.
    pub(crate) fn install(actions: async_channel::Sender<Action>) -> Result<TrayIcon, String> {
        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id(SHOW, "Show Gespan", true, None),
            &MenuItem::with_id(QUIT, "Quit Gespan", true, None),
        ])
        .map_err(|e| e.to_string())?;
        let clicks = actions.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = clicks.try_send(Action::Show);
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id().0.as_str() {
                SHOW => Action::Show,
                QUIT => Action::Quit,
                _ => return,
            };
            let _ = actions.try_send(action);
        }));
        let mut builder = TrayIconBuilder::new()
            .with_id("main")
            .with_tooltip("Gespan")
            .with_menu(Box::new(menu))
            // Left click reopens; the menu stays on the right button.
            .with_menu_on_left_click(false);
        // The app icon build.rs embeds as resource 1.
        if let Ok(icon) = Icon::from_resource(1, None) {
            builder = builder.with_icon(icon);
        }
        builder.build().map_err(|e| e.to_string())
    }

    /// Hide the window, or show it restored and in front.
    pub(crate) fn set_visible(hwnd: isize, visible: bool) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
        };
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);
        // SAFETY: `hwnd` is GPUI's live top-level window handle.
        unsafe {
            if !visible {
                let _ = ShowWindow(hwnd, SW_HIDE);
                return;
            }
            let show = if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            let _ = ShowWindow(hwnd, show);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_to_tray_always_hides_even_with_nothing_running() {
        // As in the Tauri app: with the setting on, close never quits.
        for running in [false, true] {
            for unsaved in [false, true] {
                assert_eq!(on_close(true, running, unsaved), Close::Hide);
            }
        }
    }

    #[test]
    fn without_the_tray_running_chats_or_unsaved_files_ask_first() {
        assert_eq!(on_close(false, true, false), Close::Ask);
        assert_eq!(on_close(false, false, true), Close::Ask);
        assert_eq!(on_close(false, true, true), Close::Ask);
    }

    #[test]
    fn without_the_tray_an_idle_window_just_closes() {
        assert_eq!(on_close(false, false, false), Close::Quit);
    }
}
