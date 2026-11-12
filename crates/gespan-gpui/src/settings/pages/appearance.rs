//! AppearancePage: a live preview, then Theme, Color, Glass, Layout, Chat
//! background and Diff colors cards.
use crate::glass::{self, Button};
use crate::popover::{Align, Anchor, Side, popover};
use crate::settings::{
    Preferences, SettingsEvent, SettingsView, SliderKind, widgets::picture_option,
};
use crate::theme::*;
use crate::window_blur::BLUR_ADJUSTABLE;
use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, Hsla, Role, SharedString, Window, div,
    linear_color_stop, linear_gradient, prelude::*, px, rgb,
};

/// ACCENT_COLOR_PRESETS, after the Default swatch.
pub(crate) const ACCENTS: [(u32, &str); 6] = [
    (0x4da3f5, "Blue"),
    (0x8b5cf6, "Violet"),
    (0xec4899, "Pink"),
    (0xef4444, "Red"),
    (0xf59e0b, "Orange"),
    (0x10b981, "Green"),
];

const EFFECTS: [(&str, &str); 6] = [
    ("none", "None"),
    ("dither", "Dither"),
    ("ascii", "ASCII"),
    ("halftone", "Halftone"),
    ("scanlines", "Scanlines"),
    ("gradient-blur", "Haze"),
];
fn effect_description(effect: &str) -> &'static str {
    match effect {
        "dither" => "Rebuilds the artwork with a dithered color palette.",
        "ascii" => "Recreates the artwork with colored characters on black.",
        "halftone" => "Recreates the artwork with colored print dots on black.",
        "scanlines" => "Adds a pronounced horizontal display-line texture.",
        "gradient-blur" => "Blurs and fades the artwork into the background below.",
        _ => "Shows the original artwork.",
    }
}

/// A preset swatch's accent dot: the mock's 34% radial stop inside a 26px
/// swatch, about 10px across.
const SWATCH_DOT: f32 = 10.;

/// The sidebar entries Layout offers as chips, in the sidebar's own order:
/// (id for Shell::change_sidebar_option, label).
const SIDEBAR_CHIPS: [(&str, &str); 6] = [
    ("sidebar-inbox", "Inbox"),
    ("sidebar-automations", "Automations"),
    ("sidebar-pilots", "Pilots"),
    ("sidebar-board", "Board"),
    ("sidebar-notes", "Notes"),
    ("sidebar-activity", "Activity"),
];

pub(crate) fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

/// Preview the palette's neutral tones and accent at a menu icon's 16px.
pub(crate) fn swatch(surface: u32, panel: u32, solid: u32, accent: u32) -> AnyElement {
    div()
        .relative()
        .size_4()
        .overflow_hidden()
        .rounded(px(4.))
        .border_1()
        .border_color(content(0.15))
        .bg(rgb(surface))
        .child(
            div()
                .absolute()
                .top(px(2.))
                .bottom(px(2.))
                .left(px(4.))
                .right(px(-1.))
                .rounded_l(px(2.))
                .border_1()
                .border_color(rgb(solid))
                .bg(rgb(panel)),
        )
        .child(
            div()
                .absolute()
                .top(px(5.))
                .left(px(7.))
                .size(px(5.))
                .rounded_full()
                .bg(rgb(accent)),
        )
        .into_any_element()
}

fn transparent_window_description(platform: bool, light: bool) -> &'static str {
    if !platform {
        "The window stays opaque on Linux."
    } else if light {
        "Light mode keeps the window opaque. Your dark-mode choice is remembered."
    } else {
        "Show the desktop through the window. Turn off to keep the window opaque."
    }
}

/// The accent's name as the picker labels it.
fn accent_name(accent: Option<&str>) -> &'static str {
    match accent {
        None => "Default",
        Some(hex) => ACCENTS
            .iter()
            .find(|(c, _)| Some(*c) == parse_hex(hex))
            .map_or("Custom", |(_, label)| label),
    }
}

/// The preview's one line: every choice the little window reflects.
pub(in crate::settings) fn appearance_summary(
    p: &Preferences,
    light: bool,
    boxy: bool,
    platform: bool,
    blur: bool,
) -> String {
    let theme = match p.theme.as_str() {
        "light" => "Light",
        "dark" => "Dark",
        _ => "System",
    };
    let preset = crate::settings::palettes::PALETTES
        .iter()
        .find(|(id, ..)| *id == p.workspace_theme)
        .map_or("Custom", |(_, label, ..)| label);
    let glass = if !platform {
        "Opaque window".to_owned()
    } else if light {
        "Opaque in light mode".to_owned()
    } else if !p.inset_glass {
        "Glass off".to_owned()
    } else if blur {
        format!(
            "Glass on, {}% opacity, {}px blur",
            p.sidebar_opacity, p.sidebar_blur
        )
    } else {
        format!("Glass on, {}% opacity", p.sidebar_opacity)
    };
    format!(
        "{theme} · {preset} preset · {} accent · {glass} · {} workspace",
        accent_name(p.accent_color.as_deref()),
        if boxy { "Boxy" } else { "Rounded" },
    )
}

/// A bar of placeholder text in the preview.
fn bar(width: f32, alpha: f32) -> Div {
    div()
        .h(px(7.))
        .w(gpui::relative(width))
        .rounded(px(3.))
        .bg(content(alpha))
}

/// The ring a selected round swatch wears.
fn ringed(d: gpui::Stateful<Div>, selected: bool) -> gpui::Stateful<Div> {
    d.border_2().map(|d| {
        if selected {
            d.border_color(content(1.)).shadow(vec![
                gpui::BoxShadow::new(px(0.), px(0.), content(0.2)).spread_radius(px(2.)),
            ])
        } else {
            d.border_color(gpui::transparent_black())
        }
    })
}

/// A stand-in for each background effect, drawn over the chosen image's
/// colours so the six read apart at a glance.
fn effect_tile(effect: &str, image: Option<&str>) -> Div {
    let deep: Hsla = rgb(0x3b2a5e).into();
    let sea: Hsla = rgb(0x1f3a5c).into();
    let ink = accent_color();
    let tile = div().w(px(84.)).h(px(52.)).rounded(px(10.));
    match effect {
        "none" => match image {
            Some(path) => tile.child(
                crate::image_memory::preview(std::path::PathBuf::from(path))
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover),
            ),
            None => tile.bg(linear_gradient(
                135.,
                linear_color_stop(deep, 0.),
                linear_color_stop(sea, 1.),
            )),
        },
        "dither" => tile
            .bg(sea)
            .flex()
            .flex_wrap()
            .children((0..84).map(move |ix| {
                div()
                    .size(px(4.))
                    .when((ix / 21 + ix) % 2 == 0, |d| d.bg(deep))
            })),
        "ascii" => tile
            .bg(black(1.))
            .p(px(3.))
            .font_family(crate::fonts::monospace())
            .text_size(px(8.))
            .line_height(px(9.))
            .text_color(ink)
            .child("#%*+=-:.#%*+=-:.#%*+=-:.#%*+=-:.#%*+=-:.#%*+="),
        "halftone" => tile
            .bg(black(1.))
            .p(px(2.))
            .flex()
            .flex_wrap()
            .gap(px(2.))
            .children((0..96).map(move |_| div().size(px(3.)).rounded_full().bg(ink))),
        "scanlines" => tile
            .bg(rgb(0x10101a))
            .flex()
            .flex_col()
            .gap(px(2.))
            .children((0..13).map(move |_| div().h(px(2.)).w_full().bg(deep))),
        _ => tile.bg(linear_gradient(
            135.,
            linear_color_stop(deep.opacity(0.6), 0.),
            linear_color_stop(sea.opacity(0.4), 1.),
        )),
    }
}

impl SettingsView {
    pub(super) fn appearance_page(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = self.prefs.clone();
        let light = p.theme == "light"
            || (p.theme == "system"
                && matches!(
                    cx.window_appearance(),
                    gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight
                ));
        let mut page = vec![self.preview_card(light, cx)];

        let mut theme = vec![self.row(
            Some("theme"),
            true,
            "Theme",
            Some("System follows the OS appearance.".into()),
            vec![self.segmented(
                "theme",
                "Theme",
                &p.theme,
                &[("system", "System"), ("dark", "Dark"), ("light", "Light")],
                cx,
                |this, v, cx| this.update(cx, |p| p.theme = v.into()),
            )],
        )];
        theme.push(self.row(
            Some("workspace-theme"),
            false,
            "Workspace theme",
            Some("Choose a color preset.".into()),
            vec![self.preset_swatches(light, cx)],
        ));
        #[cfg(target_os = "linux")]
        if cx.compositor_name() == "Wayland" {
            theme.push(self.row(
                Some("wayland-window-decorations"),
                false,
                "Wayland window decorations",
                Some("Show compositor decorations. Gespan's title bar stays visible.".into()),
                vec![self.toggle(
                    "wayland-window-decorations",
                    "Wayland window decorations",
                    p.wayland_window_decorations,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.wayland_window_decorations = on),
                )],
            ));
        }
        let accent_picker = self.accent_picker(cx);
        theme.push(self.row(
            Some("accent-color"),
            false,
            "Accent color",
            Some("Used for the composer send button and your message bubbles.".into()),
            vec![accent_picker],
        ));
        page.push(self.group(None, true, "Theme", None, None, theme));

        let color = vec![
                self.row(
                    Some("hue"),
                    true,
                    "Hue",
                    Some("Base hue for the window and main-pane surfaces.".into()),
                    vec![self.slider(SliderKind::Hue, format!("{}°", p.theme_hue), false, cx)],
                ),
                self.row(
                    Some("saturation"),
                    false,
                    "Saturation",
                    Some("How strongly the hue tints the interface. Zero keeps it neutral.".into()),
                    vec![self.slider(
                        SliderKind::Saturation,
                        format!("{}%", p.theme_saturation),
                        false,
                        cx,
                    )],
                ),
                self.row(
                    Some("dark-lightness"),
                    false,
                    "Dark-mode lightness",
                    Some(if light {
                        "This only affects dark mode. Your dark-mode value is preserved.".into()
                    } else {
                        "Base brightness of the dark theme. Lower values are darker; zero is true black.".into()
                    }),
                    vec![self.slider(
                        SliderKind::DarkLightness,
                        format!("{}%", p.theme_dark_lightness),
                        light,
                        cx,
                    )],
                ),
            ];
        page.push(self.group(
            None,
            false,
            "Colour",
            Some("Tint the window and main-pane surfaces.".into()),
            None,
            color,
        ));

        let opaque = light || !p.inset_glass || !GLASS_PLATFORM;
        let mut translucency = vec![self.row(
            Some("inset-glass"),
            true,
            "Transparent window",
            Some(transparent_window_description(GLASS_PLATFORM, light).into()),
            vec![self.toggle(
                "inset-glass",
                "Transparent window",
                p.inset_glass,
                light || !GLASS_PLATFORM,
                cx,
                |this, on, cx| this.update(cx, |p| p.inset_glass = on),
            )],
        )];
        translucency.push(self.row(
            Some("sidebar-opacity"),
            false,
            "Sidebar opacity",
            Some("Applies to the sidebar and the other glass panes.".into()),
            vec![self.slider(
                SliderKind::SidebarOpacity,
                format!("{}%", p.sidebar_opacity),
                opaque,
                cx,
            )],
        ));
        if BLUR_ADJUSTABLE {
            translucency.push(self.row(
                Some("blur"),
                false,
                "Blur radius",
                Some("Background blur behind the window.".into()),
                vec![self.slider(
                    SliderKind::Blur,
                    format!("{} px", p.sidebar_blur),
                    opaque,
                    cx,
                )],
            ));
        }
        translucency.push(
                self.row(
                    Some("main-pane-glass"),
                    false,
                    "Main pane glass",
                    Some("Extend the translucent treatment to the main pane behind sessions and editors.".into()),
                    vec![self.toggle(
                        "main-pane-glass",
                        "Main pane glass",
                        p.body_glass,
                        opaque,
                        cx,
                        |this, on, cx| this.update(cx, |p| p.body_glass = on),
                    )],
                ),
        );
        translucency.push(self.backdrop_row(opaque, cx));
        page.push(self.group(
            None,
            false,
            "Glass",
            Some(if light {
                "Translucency is available in dark mode.".into()
            } else if cfg!(target_os = "linux") && cx.compositor_name() == "Wayland" {
                "How much of the desktop shows through Gespan. Background blur depends on your Wayland compositor.".into()
            } else if cfg!(target_os = "linux") && cx.compositor_name() == "X11" {
                "How much of the desktop shows through Gespan. Requires compositing; configure background blur in your compositor.".into()
            } else {
                "How much of the desktop shows through Gespan. Translucency is available in dark mode.".into()
            }),
            None,
            translucency,
        ));
        page.push(self.layout_group(cx));
        page.push(self.chat_background_card(cx));
        page.push(self.diff_group(cx));
        page
    }

    /// Backdrop: the desktop behind the glass, or a soft gradient that
    /// looks the same on every desktop. Only a see-through window has one.
    fn backdrop_row(&mut self, opaque: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = self.prefs.clone();
        let gradient = p.backdrop == "gradient";
        let swatches = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .when(opaque, |d| d.opacity(0.4))
            .child(self.segmented(
                "backdrop",
                "Backdrop",
                &p.backdrop,
                &[("desktop", "Desktop"), ("gradient", "Gradient")],
                cx,
                |this, v, cx| this.update(cx, |p| p.backdrop = v.into()),
            ))
            .children(crate::settings::BACKDROPS.map(|(id, name, a, b)| {
                let on = gradient && p.backdrop_gradient == id;
                ringed(
                    div()
                        .id(SharedString::from(format!("backdrop-{id}")))
                        .debug_selector(move || format!("settings-backdrop-{id}"))
                        .role(Role::RadioButton)
                        .aria_label(format!("Gradient: {name}"))
                        .aria_selected(on)
                        .tooltip(move |_, cx| {
                            cx.new(|_| crate::conversation::HintTooltip(name)).into()
                        }),
                    on,
                )
                .size(px(22.))
                .rounded_full()
                .cursor_pointer()
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .border_1()
                        .border_color(content(0.15))
                        .bg(linear_gradient(
                            135.,
                            linear_color_stop(rgb(a), 0.),
                            linear_color_stop(rgb(b), 1.),
                        )),
                )
                .when(!opaque, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |p| {
                            p.backdrop = "gradient".into();
                            p.backdrop_gradient = id.into();
                        })
                    }))
                })
            }))
            .into_any_element();
        self.row(
            Some("backdrop"),
            false,
            div()
                .flex()
                .items_center()
                .gap_2()
                .child("Backdrop")
                .child(crate::glass::beta("New")),
            Some(if opaque {
                "A backdrop shows behind a see-through window, in dark mode with Transparent window on.".into()
            } else {
                "Show your desktop behind the glass, or a soft gradient that stays the same on every desktop.".into()
            }),
            vec![swatches],
        )
    }

    /// A small window drawn from the current choices, with the line that
    /// names them and Restore defaults.
    fn preview_card(&mut self, light: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.prefs;
        let boxy = self.ctx.new_layout_boxy;
        let glass_on = GLASS_PLATFORM && !light && p.inset_glass;
        let palette = crate::settings::palettes::palette(&p.workspace_theme, light);
        let mut shell: Hsla = rgb(palette.surface).into();
        shell.a = if glass_on {
            (p.sidebar_opacity as f32 / 100.).clamp(0.2, 1.)
        } else {
            1.
        };
        let mut pane: Hsla = rgb(palette.panel).into();
        pane.a = if glass_on && p.body_glass { 0.55 } else { 1. };
        let picked = p
            .accent_color
            .as_deref()
            .and_then(parse_hex)
            .map(|c| Hsla::from(rgb(c)));
        let summary = appearance_summary(p, light, boxy, GLASS_PLATFORM, BLUR_ADJUSTABLE);
        let desk = div()
            .debug_selector(|| "appearance-preview".into())
            .relative()
            .w(px(340.))
            .h(px(190.))
            .flex_none()
            .overflow_hidden()
            .rounded(px(12.))
            .bg(rgb(0x17151c))
            // The desktop behind the window: a soft violet-to-blue wash.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(12.))
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(Hsla::from(rgb(0x8f64f6)).opacity(0.55), 0.),
                        linear_color_stop(Hsla::from(rgb(0x459bf7)).opacity(0.4), 1.),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .inset(px(14.))
                    .rounded(px(10.))
                    .bg(shell)
                    .border_1()
                    .border_color(content(0.12))
                    .flex()
                    .when(!boxy, |d| d.p(px(6.)).gap(px(6.)))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .p(px(4.))
                            .when(boxy, |d| d.pt(px(10.)))
                            .child(bar(0.7, 0.18))
                            .child(bar(1., 0.08))
                            .child(bar(1., 0.08))
                            .child(bar(0.8, 0.08)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .bg(pane)
                            .map(|d| {
                                if boxy {
                                    d.border_l_1().border_color(content(0.1)).rounded_r(px(9.))
                                } else {
                                    d.rounded(px(8.))
                                }
                            })
                            .p(px(10.))
                            .flex()
                            .flex_col()
                            .justify_end()
                            .gap(px(6.))
                            .child(
                                div().flex().justify_end().child(
                                    div()
                                        .w(px(110.))
                                        .h(px(18.))
                                        .rounded(px(9.))
                                        .bg(picked.map_or(content(0.1), |c| c.opacity(0.35))),
                                ),
                            )
                            .child(bar(0.75, 0.14))
                            .child(bar(0.55, 0.1))
                            .child(
                                div()
                                    .h(px(22.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(content(0.12))
                                    .bg(content(0.04))
                                    .flex()
                                    .justify_end()
                                    .p(px(3.))
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .rounded(px(5.))
                                            .bg(picked.unwrap_or_else(|| content(0.9))),
                                    ),
                            ),
                    ),
            );
        glass::card()
            .p(px(18.))
            .flex()
            .items_center()
            .gap(px(20.))
            .child(desk)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child("Preview"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "appearance-summary".into())
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(content(0.62))
                            .child(summary),
                    )
                    .child(
                        div().mt(px(4.)).flex().child(
                            glass::button("restore-defaults", "Restore defaults", Button::Ghost)
                                .debug_selector(|| "settings-restore-defaults".into())
                                .role(Role::Button)
                                .aria_label("Restore defaults")
                                .h(px(30.))
                                .on_click(cx.listener(|this, _, _, cx| this.restore_defaults(cx))),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// Every workspace preset as a split swatch, its surface with its
    /// accent dot, and the chosen preset named beneath.
    fn preset_swatches(&self, light: bool, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.prefs.workspace_theme.clone();
        let name = crate::settings::palettes::PALETTES
            .iter()
            .find(|(id, ..)| *id == selected)
            .map_or("Custom", |(_, label, ..)| label);
        // The chosen preset's name sits left of the swatches, on their line.
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .debug_selector(|| "settings-workspace-theme-name".into())
                    .min_w(px(64.))
                    .text_right()
                    .text_size(px(13.))
                    .text_color(content(1.))
                    .child(name),
            )
            .child(
                div()
                    .id("workspace-theme-presets")
                    .debug_selector(|| "settings-workspace-theme-presets".into())
                    .role(Role::RadioGroup)
                    .aria_label("Workspace theme")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children(crate::settings::palettes::PALETTES.iter().map(
                        |&(id, label, dark, pale)| {
                            let palette = if light { pale } else { dark };
                            let on = id == selected;
                            ringed(
                                div()
                                    .id(SharedString::from(format!("workspace-theme-{id}")))
                                    .debug_selector(move || {
                                        format!("settings-workspace-theme-{id}")
                                    })
                                    .role(Role::RadioButton)
                                    .aria_label(label)
                                    .aria_selected(on)
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| crate::conversation::HintTooltip(label)).into()
                                    }),
                                on,
                            )
                            .size(px(26.))
                            .rounded_full()
                            .cursor_pointer()
                            .child(
                                // The preset's surface, its accent a
                                // centred dot.
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(rgb(palette.surface))
                                    .border_1()
                                    .border_color(content(0.15))
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("settings-workspace-theme-dot-{id}")
                                            })
                                            .size(px(SWATCH_DOT))
                                            .rounded_full()
                                            .bg(rgb(palette.accent)),
                                    ),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.update(cx, |p| p.apply_workspace_theme(id.to_owned()))
                                },
                            ))
                        },
                    )),
            )
            .into_any_element()
    }

    /// Workspace style as pictures, Interface scale, Tab animations, the
    /// sidebar's entries as chips and Compact view.
    fn layout_group(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.prefs.clone();
        let boxy = self.ctx.new_layout_boxy;
        let styles = div()
            .id("segmented-new-layout-style")
            .debug_selector(|| "settings-segmented-new-layout-style".into())
            .role(Role::RadioGroup)
            .aria_label("Workspace style")
            .flex()
            .gap(px(14.))
            .children(
                [("rounded", "Rounded"), ("boxy", "Boxy")].map(|(value, label)| {
                    let on = (value == "boxy") == boxy;
                    let frame: Hsla = content(0.06);
                    let preview = div()
                        .w(px(112.))
                        .h(px(72.))
                        .rounded(px(10.))
                        .bg(black(if is_light() { 0.08 } else { 0.4 }))
                        .map(|d| {
                            if value == "boxy" {
                                d.pl(px(20.)).child(
                                    div()
                                        .size_full()
                                        .bg(frame)
                                        .border_l_1()
                                        .border_color(content(0.1)),
                                )
                            } else {
                                d.p(px(6.)).pl(px(26.)).child(
                                    div()
                                        .size_full()
                                        .rounded(px(6.))
                                        .bg(frame)
                                        .border_1()
                                        .border_color(content(0.1)),
                                )
                            }
                        });
                    picture_option(
                        SharedString::from(format!("segmented-new-layout-style-{value}")),
                        preview,
                        label,
                        on,
                    )
                    .debug_selector(move || format!("settings-new-layout-style-{value}"))
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(on)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(SettingsEvent::NewLayoutBoxy(value == "boxy"))
                    }))
                }),
            )
            .into_any_element();
        let v = self.ctx.sidebar;
        let chip_on = |id: &str| match id {
            "sidebar-inbox" => v.inbox,
            "sidebar-automations" => v.automations,
            "sidebar-pilots" => p.pilots,
            "sidebar-board" => p.board,
            "sidebar-notes" => p.notes,
            _ => p.working_agents,
        };
        let chips = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(6.))
            .max_w(px(360.))
            .children(SIDEBAR_CHIPS.map(|(id, label)| {
                let on = chip_on(id);
                let flash = self.flashing(Some(id));
                div().relative().child(self.reveal_probe(id)).child(
                    glass::chip(SharedString::from(format!("toggle-{id}")), label, on)
                        .debug_selector(move || format!("settings-toggle-{id}"))
                        .role(Role::Switch)
                        .aria_label(label)
                        .aria_toggled(if on {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .when(flash, |d| d.border_color(accent_color()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::SidebarOption(id))
                        })),
                )
            }))
            .into_any_element();
        let rows = vec![
            self.row_parts(
                Some("new-layout-style"),
                true,
                "Workspace style",
                Some(self.description(if cfg!(target_os = "macos") {
                    "Choose a rounded card or a full-height boxy workspace. ⌘⌥L switches between them."
                } else {
                    "Choose a rounded card or a full-height boxy workspace. Ctrl+Alt+L switches between them."
                })),
                vec![styles],
                true,
            ),
            self.row(
                Some("interface-scale"),
                false,
                "Interface scale",
                Some("Zoom the whole interface. You can also use Ctrl+=, Ctrl+-, and Ctrl+0 (Cmd on macOS).".into()),
                vec![self.slider(
                    SliderKind::InterfaceScale,
                    format!("{}%", self.shown_ui_scale()),
                    false,
                    cx,
                )],
            ),
            self.row(
                Some("tab-animations"),
                false,
                "Tab animations",
                Some("Animate tabs as they open and close.".into()),
                vec![self.toggle(
                    "tab-animations",
                    "Tab animations",
                    p.tab_animations,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.tab_animations = on),
                )],
            ),
            self.row_parts(
                None,
                false,
                "Sidebar",
                Some(self.description("Choose what the sidebar lists.")),
                vec![chips],
                true,
            ),
            self.row(
                Some("compact-session-cards"),
                false,
                "Compact view",
                Some("Tighter sidebar rows and session cards.".into()),
                vec![self.toggle(
                    "compact-session-cards",
                    "Compact view",
                    v.compact,
                    false,
                    cx,
                    |_, _, cx| cx.emit(SettingsEvent::SidebarOption("compact-session-cards")),
                )],
            ),
        ];
        self.group(None, false, "Layout", None, None, rows)
    }

    /// Diff colors, with a two-line diff in the chosen palette.
    fn diff_group(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let line = |add: bool, text: &'static str| {
            div()
                .px(px(8.))
                .bg(diff_background(add, false))
                .text_color(diff_foreground(add))
                .whitespace_nowrap()
                .child(text)
        };
        let preview = div()
            .debug_selector(|| "settings-diff-preview".into())
            .w(px(220.))
            .overflow_hidden()
            .rounded(px(8.))
            .border_1()
            .border_color(glass::edge())
            .font_family(crate::fonts::monospace())
            .text_size(px(11.))
            .line_height(px(17.))
            .child(line(false, "− const pinned = run.version"))
            .child(line(true, "+ const pinned = run.pinnedVersion"))
            .into_any_element();
        let row = self.row(
            Some("diff-colors"),
            true,
            "Diff colors",
            Some("Colorblind and High contrast use blue and orange for changes.".into()),
            vec![
                preview,
                self.segmented(
                    "diff-colors",
                    "Diff colors",
                    &self.prefs.diff_palette.clone(),
                    &[
                        ("default", "Default"),
                        ("colorblind", "Colorblind"),
                        ("high-contrast", "High contrast"),
                    ],
                    cx,
                    |this, v, cx| this.update(cx, |p| p.diff_palette = v.into()),
                ),
            ],
        );
        self.group(None, false, "Diff colors", None, None, vec![row])
    }

    /// AccentColorPicker: Default plus six presets and a custom color.
    fn accent_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let value = self.prefs.accent_color.clone();
        let preset = match &value {
            None => Some(0),
            Some(hex) => ACCENTS
                .iter()
                .position(|(c, _)| Some(*c) == parse_hex(hex))
                .map(|ix| ix + 1),
        };
        let custom = preset
            .is_none()
            .then(|| value.as_deref().and_then(parse_hex))
            .flatten();
        let weak = cx.weak_entity();
        // Default keeps the theme's neutral ink: half ink, half page.
        let default_fill = linear_gradient(
            135.,
            linear_color_stop(content(1.), 0.5),
            linear_color_stop(background_base(), 0.5),
        );
        let colors: Vec<(gpui::Background, &'static str, Option<String>)> =
            std::iter::once((default_fill, "Default", None))
                .chain(ACCENTS.iter().map(|&(c, label)| {
                    (
                        gpui::Background::from(Hsla::from(rgb(c))),
                        label,
                        Some(format!("#{c:06x}")),
                    )
                }))
                .collect();
        div()
            .relative()
            .child(self.accent_anchor.probe())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .children(
                        colors
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (fill, label, hex))| {
                                let on = preset == Some(ix);
                                ringed(
                                    div()
                                        .id(("accent-color", ix))
                                        .debug_selector(move || format!("accent-color-{ix}"))
                                        .role(Role::Button)
                                        .aria_label(label)
                                        .aria_selected(on),
                                    on,
                                )
                                .size(px(26.))
                                .rounded_full()
                                .cursor_pointer()
                                .child(
                                    div()
                                        .size_full()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(content(0.15))
                                        .bg(fill),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.accent_custom_open = false;
                                        let hex = hex.clone();
                                        this.update(cx, |p| p.accent_color = hex);
                                    },
                                ))
                            }),
                    )
                    .child(
                        ringed(
                            div()
                                .id("accent-custom")
                                .role(Role::Button)
                                .aria_label("Custom color")
                                .aria_expanded(self.accent_custom_open),
                            custom.is_some() || self.accent_custom_open,
                        )
                        .size(px(26.))
                        .rounded_full()
                        .cursor_pointer()
                        .child(
                            div()
                                .size_full()
                                .rounded_full()
                                .overflow_hidden()
                                .border_1()
                                .border_color(content(0.15))
                                .map(|d| match custom {
                                    Some(c) => d.bg(rgb(c)),
                                    None => d.bg(linear_gradient(
                                        135.,
                                        linear_color_stop(rgb(0xff4d4d), 0.),
                                        linear_color_stop(rgb(0x4d6bff), 1.),
                                    )),
                                }),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.accent_custom_open = !this.accent_custom_open;
                            if this.accent_custom_open {
                                let text = this
                                    .prefs
                                    .accent_color
                                    .clone()
                                    .unwrap_or_else(|| "#4da3f5".into());
                                this.accent_custom.update(cx, |i, cx| {
                                    i.set_text(text, cx);
                                    i.select_range(0..i.content.len(), cx);
                                });
                                window.focus(&this.accent_custom.focus_handle(cx), cx);
                            }
                            cx.notify();
                        })),
                    ),
            )
            .when(self.accent_custom_open, |d| {
                d.child(
                    popover(
                        "accent-custom-popover",
                        Anchor::Slot(self.accent_anchor.clone()),
                    )
                    .side(Side::Bottom)
                    .align(Align::End)
                    .width(248.)
                    .padding(8.)
                    .on_dismiss(move |_, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.accent_custom_open = false;
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .h(px(30.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(content(0.1))
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(custom.unwrap_or(0x4da3f5))),
                            )
                            .child(div().flex_1().min_w_0().child(self.accent_custom.clone())),
                    ),
                )
            })
            .into_any_element()
    }

    /// ChatBackgroundCard: the global image behind every chat pane.
    fn chat_background_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.prefs.clone();
        let image = p
            .chat_background
            .clone()
            .filter(|path| std::path::Path::new(path).is_file());
        let empty = p.chat_background_empty;
        let session = p.chat_background_session;
        let thumb = div()
            .debug_selector(|| "settings-chat-background-thumb".into())
            .w(px(56.))
            .h(px(36.))
            .flex_none()
            .overflow_hidden()
            .rounded(px(7.))
            .border_1()
            .border_color(content(0.12))
            .map(|d| match &image {
                Some(path) => d.child(
                    crate::image_memory::preview(std::path::PathBuf::from(path))
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover)
                        .opacity(empty.max(20) as f32 / 100.),
                ),
                None => d
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(content(0.04))
                    .child(icon("image-add", 16.).text_color(content(0.4))),
            });
        let actions: Vec<AnyElement> = match image {
            Some(_) => vec![
                glass::button("change-chat-background", "Change", Button::Glass)
                    .role(Role::Button)
                    .aria_label("Change")
                    .h(px(30.))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ChooseBackground)))
                    .into_any_element(),
                glass::button("remove-chat-background", "Remove", Button::Ghost)
                    .role(Role::Button)
                    .aria_label("Remove")
                    .h(px(30.))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.update(cx, |p| p.chat_background = None)),
                    )
                    .into_any_element(),
            ],
            None => vec![
                glass::button("choose-chat-background", "Choose an image", Button::Glass)
                    .role(Role::Button)
                    .aria_label("Choose an image")
                    .h(px(30.))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ChooseBackground)))
                    .into_any_element(),
            ],
        };
        let mut rows =
            vec![
                self.row(
                    None,
                    true,
                    "Image",
                    Some(match &image {
                        Some(_) => format!(
                            "Shown faintly behind the chat. Empty chat preview at {empty}%."
                        )
                        .into(),
                        None => "Shown faintly behind the chat.".into(),
                    }),
                    vec![
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(thumb)
                            .children(actions)
                            .into_any_element(),
                    ],
                ),
            ];
        if let Some(path) = image.as_deref() {
            let tiles = div()
                .id("segmented-background-effect")
                .debug_selector(|| "settings-segmented-background-effect".into())
                .role(Role::RadioGroup)
                .aria_label("Background effect")
                .grid()
                .grid_cols(3)
                .gap(px(10.))
                .children(EFFECTS.iter().map(|&(option, label)| {
                    let on = p.chat_background_effect == option;
                    picture_option(
                        SharedString::from(format!("segmented-background-effect-{option}")),
                        effect_tile(option, Some(path)),
                        label,
                        on,
                    )
                    .debug_selector(move || format!("settings-background-effect-{option}"))
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |p| p.chat_background_effect = option.into())
                    }))
                }))
                .into_any_element();
            rows.push(self.row_parts(
                None,
                false,
                "Background effect",
                Some(self.description(effect_description(&p.chat_background_effect))),
                vec![tiles],
                true,
            ));
            rows.push(self.row(
                None,
                false,
                "Show on",
                Some("Empty sessions only, or every conversation.".into()),
                vec![self.segmented(
                    "background-scope",
                    "Show background on",
                    &p.chat_background_scope,
                    &[("empty", "Empty only"), ("all", "All sessions")],
                    cx,
                    |this, v, cx| this.update(cx, |p| p.chat_background_scope = v.into()),
                )],
            ));
            rows.push(self.row(
                None,
                false,
                "Empty chat visibility",
                Some("Background strength before a chat has messages.".into()),
                vec![self.slider(SliderKind::BackgroundEmpty, format!("{empty}%"), false, cx)],
            ));
            rows.push(self.row(
                None,
                false,
                "Session visibility",
                Some("Background strength once the conversation has messages.".into()),
                vec![self.slider(
                    SliderKind::BackgroundSession,
                    format!("{session}%"),
                    false,
                    cx,
                )],
            ));
        }
        self.group(
            Some("chat-background"),
            false,
            "Chat background",
            Some("An image behind your chat panes. It stays on this device.".into()),
            None,
            rows,
        )
    }
}
