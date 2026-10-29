//! OS bridge. Call initialization and Dock updates on the UI thread. All other
//! results are channels so authorization/dispatch never block the UI thread.
//!
//! macOS goes through `UNUserNotificationCenter` directly. Windows and Linux
//! use GPUI's system notifications (WinRT toasts, the freedesktop bus), as
//! the Tauri app's `notifications.rs` does through its own crates.

use std::sync::mpsc::{self, Receiver};

use gpui::App;

use super::delivery::{Permission, Target};

pub struct Notification {
    pub tag: String,
    pub subtitle: String,
    pub body: String,
    pub sound: bool,
}

#[cfg(target_os = "macos")]
#[path = "platform_macos.rs"]
mod implementation;

#[cfg(not(target_os = "macos"))]
#[path = "platform_gpui.rs"]
mod implementation;

pub use implementation::{
    drain_clicks, open_settings, permission, request_permission, set_dock_badge,
};

pub fn initialize(cx: &App) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        implementation::initialize()
    }
    #[cfg(not(target_os = "macos"))]
    implementation::initialize(cx)
}

/// Result arrives only when the OS accepts/rejects dispatch. The host should
/// wait at most five seconds in a background task and then play its fallback.
pub fn show(notification: Notification, cx: &App) -> Receiver<Result<(), String>> {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        implementation::show(notification)
    }
    #[cfg(not(target_os = "macos"))]
    implementation::show(notification, cx)
}

pub fn dismiss(tag: &str, cx: &App) {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        implementation::dismiss(tag)
    }
    #[cfg(not(target_os = "macos"))]
    implementation::dismiss(tag, cx)
}

/// The button that opens the session, besides a click on the body.
#[cfg_attr(target_os = "macos", allow(dead_code))]
const SHOW: &str = "show";

/// What Windows and Linux show: the session as the title (the app name is
/// already on the banner), the agent's text, and a Show button.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn system_notification(notification: &Notification, markup: bool) -> gpui::SystemNotification {
    gpui::SystemNotification {
        tag: notification.tag.clone().into(),
        title: notification.subtitle.clone().into(),
        body: if markup {
            escape_markup(&notification.body)
        } else {
            notification.body.clone()
        }
        .into(),
        actions: vec![gpui::SystemNotificationAction {
            id: SHOW.into(),
            label: "Show".into(),
        }],
    }
}

/// A click on the body or on Show opens what the notification was about.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn clicked(response: &gpui::SystemNotificationResponse) -> Option<Target> {
    if response.action_id.as_ref().is_some_and(|id| id != SHOW) {
        return None;
    }
    Target::from_tag(&response.tag)
}

/// Freedesktop servers read the body as a subset of HTML; agent output is text.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn resolved<T>(value: T) -> Receiver<T> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(value);
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(body: &str) -> Notification {
        Notification {
            tag: Target::Session(42).tag(7),
            subtitle: "Fix the flaky test".into(),
            body: body.into(),
            sound: true,
        }
    }

    #[test]
    fn the_session_is_the_title_with_a_show_button() {
        let shown = system_notification(&notification("Done"), false);
        assert_eq!(shown.title.as_ref(), "Fix the flaky test");
        assert_eq!(shown.body.as_ref(), "Done");
        assert_eq!(shown.tag.as_ref(), "gespan-session:42:7");
        assert_eq!(shown.actions.len(), 1);
        assert_eq!(shown.actions[0].label.as_ref(), "Show");
    }

    #[test]
    fn agent_text_is_escaped_only_where_it_would_render_as_markup() {
        let text = "<b>x</b> & y";
        assert_eq!(
            system_notification(&notification(text), true).body.as_ref(),
            "&lt;b&gt;x&lt;/b&gt; &amp; y"
        );
        // Toasts take the text as text.
        assert_eq!(
            system_notification(&notification(text), false)
                .body
                .as_ref(),
            text
        );
    }

    fn response(tag: String, action: Option<&str>) -> gpui::SystemNotificationResponse {
        gpui::SystemNotificationResponse {
            tag: tag.into(),
            action_id: action.map(Into::into),
        }
    }

    #[test]
    fn a_click_on_the_body_or_show_opens_the_session_or_reminder() {
        let session = Target::Session(42).tag(7);
        assert!(matches!(
            clicked(&response(session.clone(), None)),
            Some(Target::Session(42))
        ));
        assert!(matches!(
            clicked(&response(session, Some("show"))),
            Some(Target::Session(42))
        ));
        let reminder = Target::Reminder {
            session_id: 9,
            due_at: 1_700_000_000,
        }
        .tag(0);
        assert!(matches!(
            clicked(&response(reminder, None)),
            Some(Target::Reminder {
                session_id: 9,
                due_at: 1_700_000_000
            })
        ));
    }

    #[test]
    fn other_buttons_and_foreign_tags_open_nothing() {
        assert!(clicked(&response(Target::Session(42).tag(7), Some("other"))).is_none());
        assert!(clicked(&response("someone-else:1".into(), None)).is_none());
    }
}
