//! React's command names, defaults, scopes, filter and shortcut recorder.
use crate::{
    settings::{Preferences, SettingsView},
    theme::*,
};
use gpui::{AnyElement, Context, FontWeight, Role, div, prelude::*, px};
#[derive(Clone, Copy)]
pub(crate) struct Binding {
    pub command: &'static str,
    pub keys: &'static str,
    pub when: &'static str,
    pub fixed: bool,
}
pub(crate) const BINDINGS: &[Binding] = &[
    Binding {
        command: "App: Settings",
        keys: "⌘,",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Search",
        keys: "⌘K",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Go to File",
        keys: "⌘P",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Command Palette",
        keys: "⌘⇧P",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Find in Files",
        keys: "⌘⇧F",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Hold Everything",
        keys: "⌘⇧H",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Open Project",
        keys: "⌘O",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: New Window",
        keys: "⌘⇧N",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Quick Composer",
        keys: "⌘⇧Space",
        when: "Anywhere",
        fixed: false,
    },
    Binding {
        command: "App: Toggle Sidebar",
        keys: "⌘B",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Toggle Session Sidebar",
        keys: "⌘⇧B",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Switch Model",
        keys: "⌘.",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "App: Toggle Pilot",
        keys: "⌘I",
        when: "Project with a pilot",
        fixed: false,
    },
    Binding {
        command: "Composer: Toggle Workspace",
        keys: "⌘⇧G",
        when: "Draft session composer",
        fixed: false,
    },
    Binding {
        command: "Composer: Dictate",
        keys: "⌘⇧M",
        when: "Composer focused",
        fixed: false,
    },
    Binding {
        command: "View: Reload",
        keys: "⌘⇧R",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "View: Zoom In",
        keys: "⌘+",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "View: Zoom Out",
        keys: "⌘-",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "View: Reset Zoom",
        keys: "⌘0",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: New",
        keys: "⌘T",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Close Others",
        keys: "⌘⌥T",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Close All",
        keys: "⌘⇧W",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Next",
        keys: "⌘⇧]",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Previous",
        keys: "⌘⇧[",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Cycle Next",
        keys: "⌃Tab",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Cycle Previous",
        keys: "⌃⇧Tab",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Back",
        keys: "⌘[",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Forward",
        keys: "⌘]",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Activate 1–8",
        keys: "⌘1 … ⌘8",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Tab: Activate Last",
        keys: "⌘9",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Session: Archive",
        keys: "⌘⇧A",
        when: "sessionFocus && !overlay",
        fixed: false,
    },
    Binding {
        command: "Session: Stop",
        keys: "Esc",
        when: "sessionBusy && !terminalFocus",
        fixed: true,
    },
    Binding {
        command: "Session: Previous",
        keys: "⌘⇧↑",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Session: Next",
        keys: "⌘⇧↓",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Session: Previous in Current Tab",
        keys: "⌘↑",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Session: Next in Current Tab",
        keys: "⌘↓",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Project: Previous",
        keys: "⌘⇧←",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Project: Next",
        keys: "⌘⇧→",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Project: Move Back",
        keys: "⌘⌥⇧←",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Project: Move Forward",
        keys: "⌘⌥⇧→",
        when: "!overlay && (!textFocus || emptyComposer)",
        fixed: false,
    },
    Binding {
        command: "Pane: Close",
        keys: "⌘W",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Pane: Split Right",
        keys: "⌘D",
        when: "!editorFocus",
        fixed: false,
    },
    Binding {
        command: "Pane: Split Down",
        keys: "⌘⇧D",
        when: "!editorFocus",
        fixed: false,
    },
    Binding {
        command: "Pane: Focus Left",
        keys: "⌘⌥←",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Pane: Focus Right",
        keys: "⌘⌥→",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Pane: Focus Up",
        keys: "⌘⌥↑",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Pane: Focus Down",
        keys: "⌘⌥↓",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Terminal: New",
        keys: "⌘`",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Terminal: New Tab",
        keys: "⌘⇧`",
        when: "Always",
        fixed: false,
    },
    Binding {
        command: "Terminal: Toggle Dock",
        keys: "⌘J",
        when: "Always",
        fixed: false,
    },
    // The board's own keys, shown for reference: they can't be rebound.
    Binding {
        command: "Board: New Card",
        keys: "⌘N",
        when: "Board focused",
        fixed: true,
    },
    Binding {
        command: "Board: New card (N, board focused)",
        keys: "N",
        when: "Board focused, not typing",
        fixed: true,
    },
    Binding {
        command: "Board: Search",
        keys: "/",
        when: "Board focused, not typing",
        fixed: true,
    },
    Binding {
        command: "Board: Open Card",
        keys: "E",
        when: "Board focused, not typing",
        fixed: true,
    },
    Binding {
        command: "Board: Delete Card or Selection",
        keys: "Delete / ⌫",
        when: "Board focused, not typing, not in the drawer",
        fixed: true,
    },
    Binding {
        command: "Editor: Find",
        keys: "⌘F",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Replace",
        keys: "⌘⌥F",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Toggle Comment",
        keys: "⌘/",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Move Line Up",
        keys: "⌥↑",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Move Line Down",
        keys: "⌥↓",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Copy Line Up",
        keys: "⇧⌥↑",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Copy Line Down",
        keys: "⇧⌥↓",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Delete Line",
        keys: "⌘⇧K",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Select to Matching Bracket",
        keys: "⌘⇧\\",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Previous Change",
        keys: "⇧⌥F5",
        when: "editorFocus",
        fixed: false,
    },
    Binding {
        command: "Editor: Next Change",
        keys: "⌥F5",
        when: "editorFocus",
        fixed: false,
    },
];
/// Bindings by the area their command names (`App: Settings` is App), in
/// the order areas first appear.
pub(in crate::settings) fn by_area(rows: &[Binding]) -> Vec<(&'static str, Vec<Binding>)> {
    let mut areas: Vec<(&'static str, Vec<Binding>)> = Vec::new();
    for row in rows {
        let area = row
            .command
            .split_once(": ")
            .map_or("Other", |(area, _)| area);
        match areas.iter_mut().find(|(a, _)| *a == area) {
            Some((_, list)) => list.push(*row),
            None => areas.push((area, vec![*row])),
        }
    }
    areas
}

/// A shortcut's keys, one cap each: macOS's modifier glyphs stand alone,
/// other platforms join keys with `+`.
pub(in crate::settings) fn key_caps(label: &str) -> Vec<String> {
    if !label.contains(|c| "⌘⇧⌥⌃".contains(c)) && label.contains('+') && label.len() > 1 {
        // `Ctrl++` is Ctrl and the plus key.
        let (head, plus) = match label.strip_suffix("++") {
            Some(head) => (head, true),
            None => (label, false),
        };
        let mut caps: Vec<String> = head.split('+').map(str::to_owned).collect();
        if plus {
            caps.push("+".into());
        }
        return caps;
    }
    let mut caps = Vec::new();
    let mut rest = String::new();
    for ch in label.chars() {
        if "⌘⇧⌥⌃".contains(ch) {
            caps.push(ch.to_string());
        } else {
            rest.push(ch);
        }
    }
    if !rest.is_empty() {
        caps.push(rest);
    }
    caps
}

pub(super) fn quick_composer_label(p: &Preferences) -> String {
    p.keybindings
        .get("App: Quick Composer")
        .and_then(|s| s.clone())
        .map(|s| shortcut_label(&s))
        .unwrap_or_else(|| "⌘⇧Space".into())
}
/// A command's shortcut as shown, honoring the user's rebinding; `None`
/// once they have disabled it.
pub(crate) fn command_shortcut(p: &Preferences, command: &str) -> Option<String> {
    match p.keybindings.get(command) {
        Some(Some(s)) => Some(shortcut_label(s)),
        Some(None) => None,
        None => {
            let keys = BINDINGS.iter().find(|r| r.command == command)?.keys;
            Some(if cfg!(target_os = "macos") {
                keys.into()
            } else {
                keys.replace('⌘', "Ctrl+")
                    .replace('⇧', "Shift+")
                    .replace('⌥', "Alt+")
                    .replace('⌃', "Ctrl+")
            })
        }
    }
}
pub(crate) fn shortcut_label(shortcut: &str) -> String {
    let parts: Vec<_> = shortcut.split('-').collect();
    let mut label = String::new();
    for part in parts {
        label.push_str(&match part {
            "cmd" => "⌘".into(),
            "ctrl" => if cfg!(target_os = "macos") {
                "⌃"
            } else {
                "Ctrl+"
            }
            .into(),
            "alt" => if cfg!(target_os = "macos") {
                "⌥"
            } else {
                "Alt+"
            }
            .into(),
            "shift" => if cfg!(target_os = "macos") {
                "⇧"
            } else {
                "Shift+"
            }
            .into(),
            "space" => "Space".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            "tab" => "Tab".into(),
            key => key.to_uppercase(),
        });
    }
    label
}
impl SettingsView {
    pub(super) fn keybindings_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.keybinding_query.trim().to_lowercase();
        let rows: Vec<_> = BINDINGS
            .iter()
            .filter(|r| cfg!(target_os = "macos") || r.command != "App: Quick Composer")
            .filter(|r| {
                format!("{} {} {}", r.command, self.binding_label(r), r.when)
                    .to_lowercase()
                    .contains(&query)
            })
            .copied()
            .collect();
        let areas = by_area(&rows);
        let count = div()
            .text_size(px(11.5))
            .text_color(content(0.55))
            .child(format!(
                "{} {}",
                rows.len(),
                if rows.len() == 1 {
                    "binding"
                } else {
                    "bindings"
                }
            ))
            .into_any_element();
        let filter = crate::glass::field()
            .h(px(36.))
            .debug_selector(|| "settings-keybinding-filter".into())
            .child(icon("search-01", 14.).text_color(content(0.5)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(self.keybinding_filter.clone()),
            );
        let picked = self.keybinding_area;
        let chips = div()
            .flex()
            .flex_wrap()
            .gap(px(5.))
            .children(areas.iter().map(|(area, rows)| {
                let area: &'static str = area;
                crate::glass::chip(
                    gpui::SharedString::from(format!("keybinding-area-{area}")),
                    area,
                    picked == Some(area),
                )
                .h(px(26.))
                .px(px(9.))
                .debug_selector(move || format!("settings-keybinding-jump-{area}"))
                .role(Role::Button)
                .aria_label(format!("Jump to {area}"))
                .aria_selected(picked == Some(area))
                .child(crate::glass::count(rows.len(), false))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.keybinding_area = Some(area);
                    this.reveal(Some(area.to_owned()), cx)
                }))
            }));
        let mut page = vec![self.glass_group(
            None,
            "Shortcuts",
            Some(
                "Click a shortcut to record new keys. Press Delete while recording to disable it."
                    .into(),
            ),
            Some(count),
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(filter)
                    .when(!areas.is_empty(), |d| d.child(chips))
                    .into_any_element(),
            ),
            if rows.is_empty() {
                vec![
                    super::super::widgets::empty(
                        "settings-keybindings-empty",
                        "keyboard",
                        "No matching bindings",
                        "Try another command, key or area.",
                    )
                    .into_any_element(),
                ]
            } else {
                Vec::new()
            },
        )];
        if !areas.is_empty() {
            page.push(
                div()
                    .px_4()
                    .flex()
                    .gap(px(16.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(0.55))
                    .child(div().min_w_0().flex_1().child("COMMAND"))
                    .child(div().w(px(200.)).flex_shrink_0().child("KEYBINDING"))
                    .child(div().w(px(200.)).flex_shrink_0().child("WHEN"))
                    .into_any_element(),
            );
        }
        let error = self.recording_error.clone();
        for (area, rows) in areas {
            let cards: Vec<AnyElement> = rows
                .iter()
                .enumerate()
                .map(|(ix, row)| self.binding_row(ix == 0, *row, error.clone(), cx))
                .collect();
            page.push(
                self.glass_group(
                    Some(area),
                    div().flex().items_baseline().gap_1().child(area).child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(content(0.55))
                            .child(rows.len().to_string()),
                    ),
                    None,
                    None,
                    None,
                    cards,
                ),
            );
        }
        // A failure with nothing recording, such as a shortcut that couldn't
        // be saved, still shows.
        if let Some(error) = error.filter(|_| self.recording.is_none()) {
            page.push(
                div()
                    .px_4()
                    .text_size(px(11.5))
                    .text_color(gpui::rgb(0xf87171))
                    .child(error)
                    .into_any_element(),
            );
        }
        page
    }

    /// One binding: the command without its area, its keys as caps (or
    /// Record…, or Disabled), reset once changed, and when it applies.
    fn binding_row(
        &self,
        first: bool,
        row: Binding,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let command = row.command;
        let name = command.split_once(": ").map_or(command, |(_, name)| name);
        let recording = self.recording.as_deref() == Some(command);
        let modified = self.prefs.keybindings.contains_key(command);
        let disabled = self.prefs.keybindings.get(command) == Some(&None);
        let keys: AnyElement = if recording {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(accent_color())
                .child(div().size(px(6.)).rounded_full().bg(accent_color()))
                .child(if self.recording_preview.is_empty() {
                    "Record…".to_owned()
                } else {
                    self.recording_preview.clone()
                })
                .into_any_element()
        } else if disabled {
            div()
                .text_color(content(0.45))
                .child("Disabled")
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .children(key_caps(&self.binding_label(&row)).into_iter().map(|cap| {
                    div()
                        .h(px(20.))
                        .min_w(px(20.))
                        .px(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .bg(content(0.07))
                        .border_1()
                        .border_color(crate::glass::edge())
                        .text_color(content(if row.fixed { 0.6 } else { 0.9 }))
                        .child(cap)
                }))
                .into_any_element()
        };
        div()
            .min_h(px(40.))
            .px_4()
            .flex()
            .items_center()
            .gap(px(16.))
            .when(!first, |d| {
                d.border_t_1().border_color(crate::glass::hairline())
            })
            .when(recording, |d| d.bg(crate::glass::selected_fill()))
            .text_size(px(12.5))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .py(px(9.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_color(content(if disabled { 0.45 } else { 1. }))
                                    .child(name),
                            )
                            .when(modified, |d| {
                                d.child(
                                    div()
                                        .debug_selector(move || {
                                            format!("modified-shortcut-{command}")
                                        })
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(accent_color()),
                                )
                            }),
                    )
                    .when_some(error.filter(|_| recording), |d, error| {
                        d.child(
                            div()
                                .mt(px(2.))
                                .text_size(px(11.5))
                                .text_color(gpui::rgb(0xf87171))
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(200.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!("shortcut-{command}")))
                            .debug_selector(move || format!("shortcut-{command}"))
                            .role(if row.fixed { Role::Label } else { Role::Button })
                            .aria_label(format!("Change {command} shortcut"))
                            .min_h(px(26.))
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .rounded(px(7.))
                            .border_1()
                            .border_color(if recording {
                                accent_color()
                            } else {
                                gpui::transparent_black()
                            })
                            .font_family(crate::fonts::monospace())
                            .text_size(px(11.))
                            .child(keys)
                            .when(!row.fixed, |d| {
                                d.cursor_pointer()
                                    .hover(|s| s.bg(crate::glass::hover_fill()))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.recording = Some(command.into());
                                        this.recording_preview.clear();
                                        this.recording_error = None;
                                        window.focus(&this.recorder_focus, cx);
                                        cx.notify();
                                    }))
                            }),
                    )
                    .when(modified, |d| {
                        d.child(
                            div()
                                .id(gpui::SharedString::from(format!("reset-{command}")))
                                .debug_selector(move || format!("reset-shortcut-{command}"))
                                .role(Role::Button)
                                .aria_label(format!("Reset {command} shortcut"))
                                .p_1()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(content(0.1)))
                                .child(icon("rotate-ccw", 14.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.update(cx, |p| {
                                        p.keybindings.remove(command);
                                    })
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(200.))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(content(0.55))
                    .child(row.when),
            )
            .into_any_element()
    }
    fn binding_label(&self, row: &Binding) -> String {
        command_shortcut(&self.prefs, row.command).unwrap_or_else(|| "Disabled".into())
    }
    pub(in crate::settings) fn record_key(
        &mut self,
        key: &gpui::Keystroke,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = self.recording.clone() else {
            return;
        };
        cx.stop_propagation();
        if key.key == "escape" || key.key == "tab" {
            self.recording = None;
            cx.notify();
            return;
        }
        if matches!(
            key.key.as_str(),
            "platform" | "control" | "alt" | "shift" | "function"
        ) {
            return;
        }
        let bare = !key.modifiers.platform
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.shift;
        if bare && matches!(key.key.as_str(), "backspace" | "delete") {
            self.recording = None;
            self.update(cx, |p| {
                p.keybindings.insert(command, None);
            });
            return;
        }
        let shortcut = key.unparse();
        self.recording_preview = shortcut_label(&shortcut);
        if bare || (!key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt) {
            self.recording_error = Some("Use Command, Control, or Option with a key.".into());
            cx.notify();
            return;
        }
        if command == "Tab: Activate 1–8"
            && !matches!(
                key.key.as_str(),
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8"
            )
        {
            self.recording_error = Some("Tab: Activate 1–8 needs a number key from 1 to 8".into());
            cx.notify();
            return;
        }
        if let Some(other) = BINDINGS.iter().find(|r| {
            r.command != command
                && (default_shortcuts(r).contains(&shortcut)
                    || self
                        .prefs
                        .keybindings
                        .get(r.command)
                        .and_then(|s| s.as_ref())
                        == Some(&shortcut))
        }) {
            self.recording_error = Some(format!("Already used by {}", other.command));
            cx.notify();
            return;
        }
        self.recording = None;
        self.recording_error = None;
        self.update(cx, |p| {
            p.keybindings.insert(command, Some(shortcut));
        });
    }
}

fn default_shortcuts(row: &Binding) -> Vec<String> {
    let shortcut = row
        .keys
        .replace(
            '⌘',
            if cfg!(target_os = "macos") {
                "cmd-"
            } else {
                "ctrl-"
            },
        )
        .replace('⌃', "ctrl-")
        .replace('⌥', "alt-")
        .replace('⇧', "shift-")
        .replace('↑', "up")
        .replace('↓', "down")
        .replace('←', "left")
        .replace('→', "right")
        .replace('+', "=")
        .to_lowercase();
    let chords = if shortcut.contains('…') {
        let prefix = shortcut.split('1').next().unwrap_or("");
        (1..=8).map(|i| format!("{prefix}{i}")).collect()
    } else {
        vec![shortcut]
    };
    chords
        .into_iter()
        .filter_map(|s| gpui::Keystroke::parse(&s).ok().map(|s| s.unparse()))
        .collect()
}
