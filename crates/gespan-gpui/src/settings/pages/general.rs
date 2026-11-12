//! GeneralPage: the version card, then Alerts and Workspace.
use crate::glass::{self, Button, Tone};
use crate::notifications::updater::{self, Action};
use crate::settings::SettingsView;
use crate::theme::*;
use gpui::{
    AnyElement, Context, FontWeight, Role, SharedString, div, linear_color_stop, linear_gradient,
    prelude::*, px,
};

/// The pill beside the version: what the last check found. Before any check
/// has finished it says nothing rather than claim the build is current.
pub(in crate::settings) fn version_pill(action: &Action, current: bool) -> Option<(String, Tone)> {
    match action {
        Action::Install(_) => Some(("Update available".into(), Tone::Waiting)),
        Action::Busy(label) => Some((label.clone(), Tone::Running)),
        Action::Check if current => Some(("Up to date".into(), Tone::Ok)),
        Action::Check => None,
    }
}

impl SettingsView {
    pub(super) fn general_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.prefs.clone();
        let alerts = vec![
            self.row(
                Some("sounds"),
                true,
                "Sounds",
                Some("Play sounds for activity, completed turns, and updates.".into()),
                vec![
                    self.toggle("sounds", "Sounds", p.sounds, false, cx, |this, on, cx| {
                        this.update(cx, |p| p.sounds = on)
                    }),
                ],
            ),
            self.row(
                Some("notifications"),
                false,
                "Notifications",
                Some("Alert when reminders are due or agents finish or need input.".into()),
                {
                    let mut controls = self.notification_permission_controls(cx);
                    controls.push(self.toggle(
                        "notifications",
                        "Notifications",
                        p.notifications,
                        false,
                        cx,
                        |this, on, cx| {
                            this.update(cx, |p| p.notifications = on);
                            if on {
                                this.probe_notification_permission(true, cx);
                            }
                        },
                    ));
                    controls
                },
            ),
        ];
        let mut workspace = Vec::new();
        if cfg!(target_os = "macos") {
            // A shortcut another app holds reads as waiting on you, under
            // the description it qualifies.
            let description = div()
                .child(
                    div()
                        .mt(px(2.))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(content(0.62))
                        .child("Press")
                        .child(
                            div()
                                .debug_selector(|| "settings-quick-composer-keys".into())
                                .px(px(5.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(glass::edge())
                                .font_family(crate::fonts::monospace())
                                .text_size(px(10.5))
                                .line_height(px(16.))
                                .child(super::keybindings::quick_composer_label(&p)),
                        )
                        .child(
                            "from any app. Return starts a session in the background; ⌘Return opens it.",
                        ),
                )
                .when_some(self.ctx.quick_error.clone(), |d, error| {
                    d.child(
                        div()
                            .debug_selector(|| "settings-quick-composer-error".into())
                            .mt(px(6.))
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .text_color(Tone::Waiting.ink())
                            .child(error),
                    )
                })
                .into_any_element();
            workspace.push(self.row_parts(
                Some("quick-composer"),
                true,
                "Quick composer",
                Some(description),
                vec![self.toggle(
                    "quick-composer",
                    "Quick composer",
                    p.quick_composer,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.quick_composer = on),
                )],
                false,
            ));
        }
        workspace.push(self.row(
            Some("keep-awake"),
            !cfg!(target_os = "macos"),
            "Keep screen awake",
            Some("Stop the idle timer from dimming and locking the screen, so a locked laptop doesn't pause running agents.".into()),
            vec![self.segmented(
                "keep-awake",
                "Keep screen awake",
                &p.keep_awake,
                &gespan_keep_awake::Mode::ALL.map(|mode| (mode.as_str(), mode.label())),
                cx,
                |this, value, cx| this.update(cx, |p| p.keep_awake = value.into()),
            )],
        ));
        workspace.push(self.row(
            Some("action-opens-terminal"),
            false,
            "Show terminal for scripts",
            Some(
                "Open the terminal when a script runs. Off, scripts run in the background.".into(),
            ),
            vec![self.toggle(
                "action-opens-terminal",
                "Show terminal for scripts",
                p.action_opens_terminal,
                false,
                cx,
                |this, on, cx| this.update(cx, |p| p.action_opens_terminal = on),
            )],
        ));
        workspace.push(self.row(
            Some("terminal-placement"),
            false,
            "Terminal",
            Some("Where new terminals open: below the chat, in the right panel, or as their own tabs.".into()),
            vec![self.segmented(
                "terminal-placement",
                "Terminal",
                p.terminal_placement(),
                &[("bottom", "Bottom"), ("panel", "Side panel"), ("title", "Title bar")],
                cx,
                |this, v, cx| this.update(cx, |p| p.set_terminal_placement(v)),
            )],
        ));
        if cfg!(target_os = "windows") {
            workspace.push(self.row(
                Some("close-to-tray"),
                false,
                "Close to tray",
                Some("Closing a window hides it to the system tray instead of quitting, so running agents keep going. Reopen from the tray icon, and quit for real from its menu. Turn this off to have close end the window.".into()),
                vec![self.toggle(
                    "close-to-tray",
                    "Close to tray",
                    p.close_to_tray,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.close_to_tray = on),
                )],
            ));
        }
        if cfg!(target_os = "macos") {
            workspace.push(self.row(
                Some("pilot-menu-bar-icon"),
                false,
                "Menu bar icon",
                Some(
                    "Chat with a Pilot or open the quick composer from the macOS menu bar.".into(),
                ),
                vec![self.toggle(
                    "pilot-menu-bar-icon",
                    "Menu bar icon",
                    p.pilot_menu_bar_icon,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.pilot_menu_bar_icon = on),
                )],
            ));
        }
        vec![
            self.version_card(cx),
            self.group(None, true, "Alerts", None, None, alerts),
            self.group(None, false, "Workspace", None, None, workspace),
        ]
    }

    /// The build you are running, first, because it is what people come
    /// here to check: the app tile, the version and what the last check
    /// found, with release notes and harness updates as icon actions.
    fn version_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let updater = cx
            .try_global::<updater::Shared>()
            .map(|shared| shared.0.clone());
        let action = updater
            .as_ref()
            .map(|u| updater::action(u, cx))
            .unwrap_or(Action::Check);
        let detail = updater.as_ref().and_then(|u| updater::status(u, cx));
        let current = updater.as_ref().is_some_and(|u| updater::is_current(u, cx));
        let pill = version_pill(&action, current);
        // The pill already says the build is current.
        let detail = detail
            .filter(|_| !current)
            .unwrap_or_else(|| "View release notes and check for app and agent updates.".into());
        let flash = self.flashing(Some("update"));
        glass::card()
            .debug_selector(|| "settings-row-update".into())
            .relative()
            .p(px(16.))
            .flex()
            .items_center()
            .gap(px(14.))
            .when(flash, |d| d.border_color(accent_color().opacity(0.6)))
            .child(self.reveal_probe("update"))
            .child(
                div()
                    .size(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(11.))
                    .border_1()
                    .border_color(glass::edge())
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(gpui::Hsla::from(gpui::rgb(0xc0affc)).opacity(0.35), 0.),
                        linear_color_stop(accent_color().opacity(0.25), 1.),
                    ))
                    .text_size(px(16.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(content(1.))
                    .child("G"),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(1.))
                                    .child(format!("Gespan {}", updater::version())),
                            )
                            .when_some(pill, |d, (label, tone)| {
                                d.child(
                                    glass::status(label, tone)
                                        .debug_selector(|| "settings-version-status".into()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(content(0.55))
                            .child(detail),
                    ),
            )
            .child(
                self.icon_action_with_tip("whats-new", "star", "What's new")
                    .on_click(
                        cx.listener(|_, _, _, cx| {
                            cx.emit(crate::settings::SettingsEvent::WhatsNew)
                        }),
                    ),
            )
            .child(
                self.icon_action_with_tip(
                    "check-harness-updates",
                    "hammer",
                    "Check harness updates",
                )
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(crate::settings::SettingsEvent::CheckHarnessUpdates)
                })),
            )
            .child(self.app_update_button(action, updater, cx))
            .into_any_element()
    }

    fn icon_action_with_tip(
        &self,
        id: &'static str,
        icon: &'static str,
        label: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        glass::icon_button(id, icon)
            .role(Role::Button)
            .aria_label(label)
            .debug_selector(move || format!("settings-{id}"))
            .tooltip(move |_, cx| cx.new(|_| crate::conversation::HintTooltip(label)).into())
    }

    /// Check for updates, or once one is found, install it in place.
    fn app_update_button(
        &self,
        action: Action,
        updater: Option<gpui::Entity<updater::Updater>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let named = |button: gpui::Stateful<gpui::Div>, label: SharedString| {
            button
                .debug_selector(|| "settings-check-updates".into())
                .role(Role::Button)
                .aria_label(label)
                .h(px(30.))
        };
        match action {
            Action::Install(version) => {
                let label = SharedString::from(format!("Update to {version}"));
                named(
                    glass::button("check-updates", label.clone(), Button::Primary),
                    label,
                )
                .on_click(move |_, _, cx| {
                    if let Some(updater) = &updater {
                        updater::install(updater, cx)
                    }
                })
                .into_any_element()
            }
            Action::Busy(label) => {
                let label = SharedString::from(label);
                named(
                    glass::button("check-updates", label.clone(), Button::Glass),
                    label,
                )
                .opacity(0.6)
                .into_any_element()
            }
            Action::Check => named(
                glass::button("check-updates", "Check for updates", Button::Glass),
                "Check for updates".into(),
            )
            .on_click(
                cx.listener(|_, _, _, cx| cx.emit(crate::settings::SettingsEvent::CheckAppUpdates)),
            )
            .into_any_element(),
        }
    }
}
