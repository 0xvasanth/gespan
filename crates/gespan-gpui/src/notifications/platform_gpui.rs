//! Windows and Linux notifications through GPUI. Neither asks for permission:
//! on Windows the per-app switch in Settings decides, which `permission`
//! reads; the freedesktop bus has no permission model.
use std::{
    collections::VecDeque,
    sync::{Mutex, mpsc::Receiver},
};

use gpui::App;

use super::{Notification, Permission, Target, clicked, resolved, system_notification};

/// The Windows AppUserModelID `main` registers for the toasts.
#[cfg(windows)]
pub const APP_ID: &str = "dev.gespan.native";

static CLICKS: Mutex<VecDeque<Target>> = Mutex::new(VecDeque::new());

pub fn initialize(cx: &App) -> bool {
    cx.on_system_notification_response(|response, _| {
        if let Some(target) = clicked(&response)
            && let Ok(mut queue) = CLICKS.lock()
        {
            queue.push_back(target);
        }
    });
    true
}

/// Whether Windows Settings blocks toasts for Gespan Native. `None` when it
/// can't be asked, such as before the first toast registers the app.
#[cfg(windows)]
fn blocked() -> Option<bool> {
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&windows::core::HSTRING::from(APP_ID))
            .ok()?;
    Some(notifier.Setting().ok()? != NotificationSetting::Enabled)
}

pub fn permission() -> Receiver<Permission> {
    #[cfg(windows)]
    {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(if blocked() == Some(true) {
                Permission::Denied
            } else {
                Permission::Granted
            });
        });
        receiver
    }
    #[cfg(not(windows))]
    resolved(Permission::Granted)
}

pub fn request_permission() -> Receiver<Permission> {
    // There is no prompt to show; Settings owns the decision.
    permission()
}

/// GPUI posts without reporting back; a Windows block is checked first so
/// the caller's in-app sound stands in when no banner will show. The sound
/// setting isn't passed on: the OS plays its own.
pub fn show(notification: Notification, cx: &App) -> Receiver<Result<(), String>> {
    #[cfg(windows)]
    if blocked() == Some(true) {
        return resolved(Err(
            "Notifications are turned off for Gespan Native in Windows Settings".into(),
        ));
    }
    cx.show_system_notification(system_notification(&notification, cfg!(not(windows))));
    resolved(Ok(()))
}

pub fn drain_clicks() -> Vec<Target> {
    CLICKS
        .lock()
        .map(|mut queue| queue.drain(..).collect())
        .unwrap_or_default()
}

pub fn set_dock_badge(_: usize) {}

pub fn dismiss(tag: &str, cx: &App) {
    cx.dismiss_system_notification(tag);
}

pub fn open_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", "ms-settings:notifications"]);
        gespan_core::process::configure_child(&mut command);
        command.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    Err("Notification settings are unavailable on this platform".into())
}
