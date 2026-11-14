use gpui::{AnyElement, Div, Hsla, Role, Stateful, Svg, div, img, prelude::*, px, rgb, rgba, svg};
// Shared appearance tokens. Interaction fills use translucent content ink
// rather than opaque replacement greys.
pub(crate) const CHAT: u32 = 0x141414;
pub(crate) const TEXT: u32 = 0xebebeb;
pub(crate) const ACCENT: u32 = 0x4da3f5;
pub(crate) const COLUMN: f32 = 832.;
/// The transcript's side padding inside `COLUMN`: the docked composer's
/// 10px inset, 1px border and 12px text padding, so prose lines up with the
/// composer's text as it scrolls past.
pub(crate) const COLUMN_PAD: f32 = 23.;
pub(crate) const TITLE_BAR_HEIGHT: f32 = 40.;
pub(crate) const TITLE_TAB_TEXT_SIZE: f32 = 13.;
pub(crate) const TITLE_TAB_MAX_WIDTH: f32 = 224.;
pub(crate) const PANE_HEADER_HEIGHT: f32 = 36.;
/// Every tab's plate: title, file and terminal tabs, and the new-tab button.
pub(crate) const TAB_PLATE_HEIGHT: f32 = 27.;

/// Attachment previews keep a fixed crop, independent of image dimensions.
pub(crate) fn attachment_thumbnail(source: impl Into<gpui::ImageSource>) -> gpui::AnyElement {
    div()
        .size(px(36.))
        .min_w(px(36.))
        .min_h(px(36.))
        .max_w(px(36.))
        .max_h(px(36.))
        .flex_shrink_0()
        .rounded_lg()
        .overflow_hidden()
        .child(
            img(source)
                .size(px(36.))
                .aspect_square()
                .flex_shrink_0()
                .rounded_lg()
                .object_fit(gpui::ObjectFit::Cover),
        )
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq)]
struct Tokens {
    light: bool,
    ink: Hsla,
    accent: Hsla,
    user_accent: Option<Hsla>,
    surfaces: Surfaces,
    syntax: [u32; 11],
    diff: [u32; 4],
    diff_strong: bool,
}
/// The planes: the shell around content, a pane behind sessions and
/// editors, and the content card.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Surfaces {
    shell: Hsla,
    pane: Hsla,
    card: Hsla,
}
impl Surfaces {
    /// The light planes: the theme base at the saved opacity, with a main
    /// pane that lets the glass through when Main pane glass is on.
    fn light(base: Hsla, glass: bool, body_glass: bool) -> Self {
        let mut pane = base;
        pane.a = if glass && body_glass { 0.28 } else { 1. };
        // A glass card reveals the window's own tint; adding another would
        // hide the desktop twice. Solid content is white.
        let mut card = base;
        card.l = 1.;
        card.a = if glass && body_glass { 0. } else { 1. };
        Self {
            shell: base,
            pane,
            card,
        }
    }
    /// The dark planes after the desktop stylesheet: see [`inset_surfaces`].
    fn dark(base: Hsla, glass: bool, body_glass: bool) -> Self {
        let (shell, card) = inset_surfaces(base, glass, body_glass);
        let mut pane = shell;
        pane.a = if glass && body_glass { 0.28 } else { 1. };
        Self { shell, pane, card }
    }
    fn mix(&self, to: &Self, color: impl Fn(Hsla, Hsla) -> Hsla) -> Self {
        Self {
            shell: color(self.shell, to.shell),
            pane: color(self.pane, to.pane),
            card: color(self.card, to.card),
        }
    }
}
thread_local! {
    /// What views paint with: the target, or a step of the tween toward it.
    static TOKENS: std::cell::RefCell<Option<Tokens>> = const { std::cell::RefCell::new(None) };
    static TARGET: std::cell::RefCell<Option<Tokens>> = const { std::cell::RefCell::new(None) };
    static TWEEN: std::cell::Cell<Option<(Tokens, std::time::Instant)>> = const { std::cell::Cell::new(None) };
}
/// Switching to a project with other colors blends into them over this long.
const TWEEN_LENGTH: std::time::Duration = std::time::Duration::from_millis(360);

#[cfg(test)]
pub(crate) fn configure(p: &crate::settings::Preferences, system_light: bool) -> bool {
    configure_project(p, None, system_light, false)
}
/// Paint with `project`'s colors in place of the app-wide ones. With
/// `animate`, a change of colors blends from what is on screen; call again
/// each frame while [`tweening`] so the blend advances. Returns whether the
/// painted colors changed.
pub(crate) fn configure_project(
    p: &crate::settings::Preferences,
    project: Option<&gespan_core::project_appearance::ProjectTheme>,
    system_light: bool,
    animate: bool,
) -> bool {
    let light = p.theme == "light" || (p.theme == "system" && system_light);
    let (workspace_theme, accent_color, hue, saturation, dark_lightness) = match project {
        Some(t) => (
            t.workspace_theme.as_str(),
            t.accent_color.as_deref(),
            t.hue,
            t.saturation,
            t.dark_lightness,
        ),
        None => (
            p.workspace_theme.as_str(),
            p.accent_color.as_deref(),
            p.theme_hue,
            p.theme_saturation,
            p.theme_dark_lightness,
        ),
    };
    let palette = crate::settings::palette(workspace_theme, light);
    let base = |glass: bool| {
        gpui::hsla(
            hue.min(360) as f32 / 360.,
            saturation.min(100) as f32 / 100.,
            if light {
                0.97
            } else {
                dark_lightness.min(30) as f32 / 100.
            },
            if glass {
                p.sidebar_opacity.clamp(15, 100) as f32 / 100.
            } else {
                1.
            },
        )
    };
    let glass = layout_glass(p, system_light);
    let surfaces = if light {
        Surfaces::light(base(glass), glass, p.body_glass)
    } else {
        Surfaces::dark(base(glass), glass, p.body_glass)
    };
    let custom = accent_color
        .filter(|s| s.len() == 7)
        .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok());
    let tokens = Tokens {
        light,
        ink: rgb(palette.content).into(),
        accent: rgb(custom.unwrap_or(palette.accent)).into(),
        user_accent: custom.map(|c| rgb(c).into()),
        surfaces,
        diff: diff_palette(&p.diff_palette, light),
        diff_strong: p.diff_palette == "high-contrast",
        syntax: syntax_palette(workspace_theme, light),
    };
    let now = std::time::Instant::now();
    if TARGET.with(|t| t.replace(Some(tokens))) != Some(tokens) {
        let shown = TOKENS.with(|t| *t.borrow());
        // A light/dark flip repaints too much at once to blend well.
        TWEEN.set(
            shown
                .filter(|shown| animate && shown.light == light)
                .map(|shown| (shown, now)),
        );
    }
    let painted = match TWEEN.get() {
        Some((from, start)) => {
            let t = (now - start).as_secs_f32() / TWEEN_LENGTH.as_secs_f32();
            if t >= 1. {
                TWEEN.set(None);
                tokens
            } else {
                // Ease out: quick to respond, gentle to settle.
                from.mix(&tokens, 1. - (1. - t).powi(3))
            }
        }
        None => tokens,
    };
    TOKENS.with(|current| current.replace(Some(painted)) != Some(painted))
}
/// The dark inset layout's two planes, after the desktop stylesheet: the
/// frame around the card (sidebar, tabs, footer) is a near-black `#0d0d0d`
/// at 80% over the window blur, and the card a deeper `#060606` at 40% on top
/// of it, so content sits darker than the chrome and colour reads clearly.
/// Without glass the card is a solid `#141414`. Both scale with the saved
/// dark lightness and opacity, landing on those values at the defaults
/// (9% lightness, 85% opacity).
fn inset_surfaces(base: Hsla, glass: bool, body_glass: bool) -> (Hsla, Hsla) {
    const DEFAULT_LIGHTNESS: f32 = 0.09;
    const DEFAULT_OPACITY: f32 = 0.85;
    let scale = base.l / DEFAULT_LIGHTNESS;
    let mut frame = base;
    frame.l = 0x0d as f32 / 255. * scale;
    frame.a = if glass {
        (base.a * 0.8 / DEFAULT_OPACITY).clamp(0.15, 1.)
    } else {
        1.
    };
    let mut card = base;
    if glass && body_glass {
        card.l = 0x06 as f32 / 255. * scale;
        card.a = 0.4;
    } else {
        card.l = 0x14 as f32 / 255. * scale;
        card.a = 1.;
    }
    (frame, card)
}
/// Whether colors are still blending toward a project's theme.
pub(crate) fn tweening() -> bool {
    TWEEN.get().is_some()
}
impl Tokens {
    fn mix(&self, to: &Self, t: f32) -> Self {
        let color = |a: Hsla, b: Hsla| -> Hsla {
            let (a, b) = (a.to_rgb(), b.to_rgb());
            gpui::Rgba {
                r: a.r + (b.r - a.r) * t,
                g: a.g + (b.g - a.g) * t,
                b: a.b + (b.b - a.b) * t,
                a: a.a + (b.a - a.a) * t,
            }
            .into()
        };
        let hex = |a: u32, b: u32| -> u32 {
            [16, 8, 0].iter().fold(0, |out, shift| {
                let (a, b) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
                out | (((a + (b - a) * t).round() as u32) << shift)
            })
        };
        Self {
            ink: color(self.ink, to.ink),
            accent: color(self.accent, to.accent),
            // Going to or from the theme's own accent blends through it.
            user_accent: match (self.user_accent, to.user_accent) {
                (None, None) => None,
                (from, to_accent) => Some(color(
                    from.unwrap_or(self.accent),
                    to_accent.unwrap_or(to.accent),
                )),
            },
            surfaces: self.surfaces.mix(&to.surfaces, color),
            syntax: std::array::from_fn(|i| hex(self.syntax[i], to.syntax[i])),
            diff: std::array::from_fn(|i| hex(self.diff[i], to.diff[i])),
            ..*to
        }
    }
}

/// Platforms that can reveal the desktop behind the window. macOS vibrancy
/// and Windows Acrylic supply blur; Linux blur is compositor-dependent.
pub(crate) const GLASS_PLATFORM: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));
pub(crate) fn window_glass(p: &crate::settings::Preferences, system_light: bool) -> bool {
    GLASS_PLATFORM && p.theme != "light" && !(p.theme == "system" && system_light)
}
/// `window_glass` as Transparent window allows it.
pub(crate) fn layout_glass(p: &crate::settings::Preferences, system_light: bool) -> bool {
    window_glass(p, system_light) && p.inset_glass
}
/// What the compositor puts behind a window that does or doesn't paint glass.
pub(crate) fn window_backdrop(glass: bool) -> gpui::WindowBackgroundAppearance {
    if glass && GLASS_PLATFORM {
        gpui::WindowBackgroundAppearance::Blurred
    } else {
        gpui::WindowBackgroundAppearance::Opaque
    }
}
pub(crate) fn accent_color() -> Hsla {
    TOKENS.with(|t| {
        t.borrow()
            .as_ref()
            .map(|t| t.accent)
            .unwrap_or_else(|| rgb(ACCENT).into())
    })
}
pub(crate) fn content(alpha: f32) -> Hsla {
    let mut color: Hsla = TOKENS.with(|t| {
        t.borrow()
            .as_ref()
            .map(|t| t.ink)
            .unwrap_or_else(|| rgb(TEXT).into())
    });
    color.a = alpha;
    color
}

/// `[data-composer-box]::before` uses content ink at 3% over its backdrop.
/// Light mode instead gives the box the solid `background-base` surface.
pub(crate) fn composer_fill() -> Hsla {
    let light = TOKENS.with(|tokens| tokens.borrow().as_ref().is_some_and(|t| t.light));
    if light {
        let mut background = shell_fill();
        background.a = 1.;
        background
    } else {
        content(0.03)
    }
}

/// The inset composer's frost; the solid light surface has no backdrop blur.
pub(crate) fn composer_blur() -> f32 {
    let light = TOKENS.with(|tokens| tokens.borrow().as_ref().is_some_and(|t| t.light));
    if light { 0. } else { 24. }
}

/// The accent picked in Settings; `None` keeps the theme's neutral ink.
fn user_accent() -> Option<Hsla> {
    TOKENS.with(|t| t.borrow().as_ref().and_then(|t| t.user_accent))
}
/// `var(--user-accent-color, var(--color-accent))`.
pub(crate) fn picked_accent() -> Hsla {
    user_accent().unwrap_or_else(accent_color)
}
/// `.user-message-bubble`: a picked accent tints sent messages at 24%.
pub(crate) fn user_bubble_fill() -> Hsla {
    user_accent().map_or(content(0.1), |a| a.opacity(0.24))
}
/// `.user-message-bubble[data-draft]`: an 8% wash inside a 38% dashed edge.
pub(crate) fn draft_bubble_fill() -> Hsla {
    user_accent().map_or(content(0.04), |a| a.opacity(0.08))
}
pub(crate) fn draft_bubble_edge() -> Hsla {
    user_accent().map_or(content(0.3), |a| a.opacity(0.38))
}
/// `.primary-action` under `has-user-accent`: the accent, with black or white
/// ink by its relative luminance; `None` keeps the default solid button.
pub(crate) fn primary_action(enabled: bool) -> Option<(Hsla, Hsla)> {
    let accent = user_accent()?;
    if !enabled {
        return Some((accent.opacity(0.28), content(0.35)));
    }
    let rgba = accent.to_rgb();
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b);
    Some((
        accent,
        if luminance > 0.179 {
            black(1.)
        } else {
            white(1.)
        },
    ))
}

/// Selected text is tinted with the accent; light surfaces get a paler wash
/// so dark ink stays readable on top.
pub(crate) fn text_selection() -> Hsla {
    accent_color().opacity(if is_light() { 0.2 } else { 0.35 })
}

/// Light mode lifts the solid composer off the matching page with a soft
/// two-layer drop; dark mode's frosted box needs no shadow.
pub(crate) fn composer_shadow() -> Vec<gpui::BoxShadow> {
    if !is_light() {
        return Vec::new();
    }
    vec![
        gpui::BoxShadow::new(px(0.), px(1.), black(0.04)).blur_radius(px(2.)),
        gpui::BoxShadow::new(px(0.), px(6.), black(0.07))
            .blur_radius(px(20.))
            .spread_radius(px(-4.)),
    ]
}

/// Light themes paint floating surfaces opaque instead of frosting them.
pub(crate) fn is_light() -> bool {
    TOKENS.with(|tokens| tokens.borrow().as_ref().is_some_and(|t| t.light))
}
/// The opaque card behind popovers and modals in light themes, and wherever
/// the renderer has no backdrop blur.
pub(crate) fn floating_fill() -> Hsla {
    let mut fill = if is_light() {
        inset_pane_fill()
    } else {
        pane_fill()
    };
    if is_light() {
        fill.l = fill.l.max(0.995);
    }
    fill.a = 1.;
    fill
}

pub(crate) fn composer_border(focused: bool) -> Hsla {
    content(if focused { 0.2 } else { 0.1 })
}

pub(crate) fn title_tab_fill(selected: bool, hovered: bool) -> Hsla {
    content(if selected {
        0.1
    } else if hovered {
        0.07
    } else {
        0.04
    })
}

/// The plate for session tabs in the title bar.
/// Headline ink owns inactive dimming; the plate never fades its children.
/// Its icon tile sits as far from the left edge as from the top and bottom.
pub(crate) fn title_tab_plate(selected: bool, hovered: bool) -> Div {
    div()
        .h(px(TAB_PLATE_HEIGHT))
        .rounded_md()
        .pl(px(crate::tab_tile::tab_tile_inset()))
        .pr_2()
        .flex()
        .items_center()
        .bg(title_tab_fill(selected, hovered))
}
/// The planes views paint with.
fn surfaces() -> Option<Surfaces> {
    TOKENS.with(|t| t.borrow().as_ref().map(|t| t.surfaces))
}
pub(crate) fn shell_fill() -> Hsla {
    if let Some(fill) = surfaces().map(|s| s.shell) {
        return fill;
    }
    if GLASS_PLATFORM {
        // #0d0d0d at 80%, with the stylesheet's #ebebeb 5% wash above it.
        rgba(0x1b1b1bcf).into()
    } else {
        rgb(0x181818).into()
    }
}
pub(crate) fn pane_fill() -> Hsla {
    if let Some(fill) = surfaces().map(|s| s.pane) {
        return fill;
    }
    if GLASS_PLATFORM {
        rgba(0x06060666).into()
    } else {
        rgb(CHAT).into()
    }
}
/// `--color-background-base`: the theme's opaque surface, also the ink on
/// solid `bg-content` buttons.
pub(crate) fn background_base() -> Hsla {
    let mut base = shell_fill();
    base.a = 1.;
    base
}
/// `--zen-rail`: ink 14% over the opaque base, the tree rails under tool
/// phases and orchestrator/operator rows. Opaque, so the curve and spine
/// overlapping where they meet don't stack into a brighter pixel.
pub(crate) fn rail_color() -> Hsla {
    let (base, ink) = (background_base().to_rgb(), content(1.).to_rgb());
    let mix = |b: f32, i: f32| b + (i - b) * 0.14;
    gpui::Rgba {
        r: mix(base.r, ink.r),
        g: mix(base.g, ink.g),
        b: mix(base.b, ink.b),
        a: 1.,
    }
    .into()
}
/// Apply to surrounding chrome, leaving the content on the window's glass.
pub(crate) fn inset_chrome_wash() -> Hsla {
    content(0.05)
}
pub(crate) fn inset_pane_fill() -> Hsla {
    surfaces().map_or_else(pane_fill, |s| s.card)
}
pub(crate) fn icon(path: &'static str, side: f32) -> Svg {
    svg()
        .path(path)
        .size(px(side))
        .text_color(content(0.5))
        .flex_shrink_0()
}
/// The colour a navigation icon takes wherever it shows (sidebar rows, the
/// rail and title tabs), so each page reads at a glance instead of in plain
/// ink. Light themes take a deeper shade to keep the glyph's contrast on white.
pub(crate) fn nav_tint(label: &str) -> Option<Hsla> {
    let (dark, light) = match label {
        // Home is the bright ink (`#ebebeb` in the dark design), not a
        // hue: it follows the theme's ink so it stays bright in both.
        "Home" => return Some(content(1.)),
        "New thread" => (0x3DD68C, 0x15874F),
        "My tasks" => (0xFFB900, 0xA86B00),
        "Inbox" => (0x8B95FF, 0x4F5BD5),
        "Notes" => (0xFF7EA8, 0xCC2F66),
        "Search" => (0x5AA9FF, 0x1F6FD6),
        "Automations" => (0xF5B544, 0xC77C0A),
        "Board" => (0xD08CFF, 0xA13FD6),
        "Runs" => (0x3CCFCF, 0x0E8A8A),
        "Settings" => (0xFF7A5C, 0xD6452A),
        _ => return None,
    };
    Some(rgb(if is_light() { light } else { dark }).into())
}
/// A page's mark on a neutral title-tab tile: its navigation tint, so the tab
/// matches its sidebar row, else the tile's usual ink.
pub(crate) fn nav_tile_ink(label: &str) -> Hsla {
    nav_tint(label).unwrap_or_else(crate::tab_tile::tile_ink)
}
/// A provider's brand fill for its icon tile, top and bottom of a gentle
/// gradient. `None` for a multicolour logo that keeps a light tile.
pub(crate) fn provider_tile(provider: &str) -> Option<(Hsla, Hsla)> {
    let (top, bottom) = match provider {
        "claude" => (0xE3876A, 0xD06A49),
        "antigravity" => return None,
        // The violet-to-pink of its mark.
        "omp" => (0xC45BE0, 0x8A5CF2),
        "cursor" | "grok" | "pi" | "fx" => (0x343434, 0x161616),
        "opencode" => (0x3B3636, 0x1F1C1C),
        "hermes" | "devin" => (0x3A3A3E, 0x1E1E22),
        // Kiro's purple.
        "kiro" => (0x9B6BFF, 0x7A3DF0),
        // Codex: the blue of its mark's gradient.
        _ => (0x7A8CFF, 0x4048FF),
    };
    Some((rgb(top).into(), rgb(bottom).into()))
}

/// The provider's mark in white, to sit on its [`provider_tile`] colour.
/// A multicolour logo sits on a neutral tile in that tile's ink.
pub(crate) fn provider_glyph(provider: &str, side: f32) -> AnyElement {
    let (path, scale) = match provider {
        // A multicolour logo on a neutral tile: one colour, like the rest.
        "antigravity" => {
            return svg()
                .path("antigravity.svg")
                .size(px(side))
                .text_color(crate::tab_tile::tile_ink())
                .flex_shrink_0()
                .into_any_element();
        }
        // Codex's mark without the white disc behind it, so the prompt cut
        // out of it shows the tile's colour.
        "codex" => ("codex-glyph.svg", 1.),
        "claude" => ("claude.svg", 1.),
        "cursor" => ("cursor.svg", 1.),
        "grok" => ("grok.svg", 1.),
        "opencode" => ("opencode.svg", 1.),
        "pi" => ("pi.svg", 1.),
        "omp" => ("omp.svg", 1.),
        "fx" => ("fx.svg", 1.),
        "hermes" => ("hermes.svg", 0.72),
        "devin" => ("devin.svg", 0.88),
        "kiro" => ("kiro.svg", 0.72),
        _ => ("codex-glyph.svg", 1.),
    };
    svg()
        .path(path)
        .size(px(side * scale))
        .text_color(gpui::white())
        .flex_shrink_0()
        .into_any_element()
}

/// Asset path, whether it follows the theme colour, and its size scale.
fn provider_mark(provider: &str) -> (&'static str, bool, f32) {
    match provider {
        "claude" => ("claude.svg", false, 1.),
        "cursor" => ("cursor.svg", true, 1.),
        "grok" => ("grok.svg", true, 1.),
        "opencode" => ("opencode.svg", true, 1.),
        "antigravity" => ("antigravity.svg", false, 1.),
        "pi" => ("pi.svg", true, 1.),
        "omp" => ("omp.svg", false, 1.),
        "fx" => ("fx.svg", true, 1.),
        "hermes" => ("hermes.svg", true, 0.72),
        "devin" => ("devin.svg", true, 0.88),
        // Kiro's ghost on its purple tile, in colour like Claude and Codex.
        "kiro" => ("kiro-color.svg", false, 1.),
        _ => ("codex.svg", false, 1.),
    }
}
pub(crate) fn provider_icon(provider: &str, side: f32) -> Div {
    let (path, monochrome, scale) = provider_mark(provider);
    // Like React's currentColor / CSS masks, GPUI's SVG mask follows the theme.
    // Image rendering preserves brand colors but has no inherited currentColor.
    let mark = if monochrome {
        svg()
            .path(path)
            .size(px(side * scale))
            .text_color(content(1.))
            .flex_shrink_0()
            .into_any_element()
    } else {
        img(path).size(px(side)).flex_shrink_0().into_any_element()
    };
    div()
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(mark)
}
pub(crate) fn icon_button(
    id: &'static str,
    label: &'static str,
    path: &'static str,
) -> Stateful<Div> {
    icon_button_hover(id, label, path, 0.1)
}
pub(crate) fn icon_button_hover(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    hover_alpha: f32,
) -> Stateful<Div> {
    chrome_button(id, label, path, true, hover_alpha, 0.5, None)
}
/// An icon button drawn over busy content, where the usual half ink fades out.
pub(crate) fn icon_button_ink(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    ink: f32,
) -> Stateful<Div> {
    chrome_button(id, label, path, true, 0.1, ink, None)
}
pub(crate) fn icon_button_enabled(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    enabled: bool,
) -> Stateful<Div> {
    chrome_button(id, label, path, enabled, 0.1, 0.5, None)
}
fn chrome_button(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    enabled: bool,
    hover_alpha: f32,
    ink: f32,
    tint: Option<Hsla>,
) -> Stateful<Div> {
    let ink = content(if enabled { ink } else { 0.25 });
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .group("chrome-button")
        .cursor_pointer()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(24.))
        .rounded_md()
        .text_color(ink)
        .when(enabled, |d| {
            d.hover(|s| s.bg(content(hover_alpha)).text_color(content(1.)))
        })
        .child(match tint {
            Some(tint) => icon(path, 14.).text_color(tint),
            None => icon(path, 14.).text_color(ink).when(enabled, |i| {
                i.group_hover("chrome-button", |s| s.text_color(content(1.)))
            }),
        })
}
// `.primary-action` paints with literal white and black, not theme ink.
pub(crate) fn white(alpha: f32) -> Hsla {
    gpui::hsla(0., 0., 1., alpha)
}
pub(crate) fn black(alpha: f32) -> Hsla {
    gpui::hsla(0., 0., 0., alpha)
}

const SYNTAX_KEYS: [&str; 11] = [
    "keyword", "callable", "string", "type", "tag", "number", "comment", "property", "meta",
    "invalid", "heading",
];
fn syntax_palette(theme: &str, light: bool) -> [u32; 11] {
    static PALETTES: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let palettes = PALETTES.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/editor-palettes.json"))
            .expect("bundled editor palettes")
    });
    let palette = palettes.get(theme).unwrap_or(&palettes["default"]);
    let palette = &palette[if light { "light" } else { "dark" }];
    SYNTAX_KEYS.map(|key| {
        u32::from_str_radix(
            palette[key]
                .as_str()
                .unwrap_or("#ebebeb")
                .trim_start_matches('#'),
            16,
        )
        .unwrap_or(0xebebeb)
    })
}
pub(crate) fn syntax_color(kind: &str) -> Hsla {
    let name = kind
        .strip_prefix("var(--syntax-")
        .and_then(|s| s.strip_suffix(')'));
    let index = if kind == "var(--color-markdown-heading)" {
        Some(10)
    } else {
        name.and_then(|name| SYNTAX_KEYS.iter().position(|key| *key == name))
    };
    index.map_or_else(
        || content(1.),
        |index| {
            let color = TOKENS
                .with(|t| t.borrow().as_ref().map(|t| t.syntax[index]))
                .unwrap_or_else(|| syntax_palette("default", false)[index]);
            rgb(color).into()
        },
    )
}

fn diff_palette(name: &str, light: bool) -> [u32; 4] {
    match (name, light) {
        ("colorblind", false) => [0x388bfd, 0xdb6d28, 0x79c0ff, 0xffa657],
        ("colorblind", true) => [0x0969da, 0xbc4c00, 0x0550ae, 0x953800],
        ("high-contrast", false) => [0x58a6ff, 0xf0883e, 0xcae8ff, 0xffdfb6],
        ("high-contrast", true) => [0x0550ae, 0x953800, 0x032563, 0x471700],
        (_, true) => [0x10b981, 0xf43f5e, 0x047857, 0xbe123c],
        _ => [0x10b981, 0xf43f5e, 0x6ee7b7, 0xfda4af],
    }
}
pub(crate) fn diff_foreground(add: bool) -> Hsla {
    let index = if add { 2 } else { 3 };
    let color = TOKENS
        .with(|t| t.borrow().as_ref().map(|t| t.diff[index]))
        .unwrap_or_else(|| diff_palette("default", false)[index]);
    rgb(color).into()
}
pub(crate) fn diff_background(add: bool, gutter: bool) -> Hsla {
    let index = if add { 0 } else { 1 };
    let (color, strong) = TOKENS
        .with(|t| t.borrow().as_ref().map(|t| (t.diff[index], t.diff_strong)))
        .unwrap_or((diff_palette("default", false)[index], false));
    let mut color: Hsla = rgb(color).into();
    color.a = match (gutter, strong) {
        (true, true) => 0.45,
        (true, false) => 0.25,
        (false, true) => 0.28,
        _ => 0.15,
    };
    color
}

#[cfg(test)]
mod composer_tests {
    use super::*;

    struct Restore(Option<Tokens>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TOKENS.with(|tokens| *tokens.borrow_mut() = self.0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_glass_follows_theme() {
        let mut preferences = crate::settings::Preferences::default();
        for (theme, system_light, expected) in [
            ("dark", false, true),
            ("dark", true, true),
            ("light", false, false),
            ("light", true, false),
            ("system", false, true),
            ("system", true, false),
        ] {
            preferences.theme = theme.into();
            let glass = window_glass(&preferences, system_light);
            assert_eq!(glass, expected);
            assert_eq!(
                window_backdrop(glass),
                if expected {
                    gpui::WindowBackgroundAppearance::Blurred
                } else {
                    gpui::WindowBackgroundAppearance::Opaque
                }
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_surface_alpha_tracks_saved_opacity_and_main_pane_glass() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        let mut preferences = crate::settings::Preferences {
            theme: "dark".into(),
            ..Default::default()
        };
        // The frame lands on 80% at the default 85% opacity.
        let frame = |opacity: f32| (opacity * 0.8 / 0.85).clamp(0.15, 1.);
        for (opacity, saved) in [(0, 0.15), (15, 0.15), (40, 0.4), (100, 1.), (200, 1.)] {
            preferences.sidebar_opacity = opacity;
            configure(&preferences, false);
            assert!((shell_fill().a - frame(saved)).abs() < 0.001, "{opacity}");
            assert_eq!(pane_fill().a, 0.28);
            assert_eq!(inset_pane_fill().a, 0.4);
        }

        preferences.sidebar_opacity = 40;
        preferences.body_glass = false;
        configure(&preferences, false);
        assert!((shell_fill().a - frame(0.4)).abs() < 0.001);
        assert_eq!(pane_fill().a, 1.);
        assert_eq!(inset_pane_fill().a, 1.);

        preferences.theme = "light".into();
        preferences.body_glass = true;
        configure(&preferences, false);
        assert_eq!(shell_fill().a, 1.);
        assert_eq!(pane_fill().a, 1.);
        assert_eq!(inset_pane_fill().a, 1.);
        assert_eq!(preferences.sidebar_opacity, 40);
        assert!(preferences.body_glass);

        preferences.theme = "dark".into();
        configure(&preferences, false);
        assert!((shell_fill().a - frame(0.4)).abs() < 0.001);
        assert_eq!(inset_pane_fill().a, 0.4);
    }

    #[test]
    fn transparent_window_turns_glass_off_and_is_kept_in_light_mode() {
        let mut preferences = crate::settings::Preferences {
            theme: "dark".into(),
            inset_glass: false,
            ..Default::default()
        };
        assert!(!layout_glass(&preferences, false));
        preferences.inset_glass = true;
        assert_eq!(layout_glass(&preferences, false), GLASS_PLATFORM);
        preferences.theme = "light".into();
        assert!(!layout_glass(&preferences, false));
        assert!(preferences.inset_glass, "light mode remembers the choice");
    }

    #[test]
    fn the_dark_theme_uses_the_desktop_apps_deep_frame_and_card() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        let hex = |c: Hsla| {
            let c = gpui::Rgba::from(c);
            ((c.r * 255.).round() as u32) << 16
                | ((c.g * 255.).round() as u32) << 8
                | (c.b * 255.).round() as u32
        };
        // The desktop app's colours, which were the defaults before the
        // neutral black ones.
        let mut preferences = crate::settings::Preferences {
            theme: "dark".into(),
            theme_hue: 240,
            theme_dark_lightness: 9,
            sidebar_opacity: 85,
            sidebar_blur: 24,
            ..Default::default()
        };
        configure_project(&preferences, None, false, false);
        if GLASS_PLATFORM {
            // At those values: #0d0d0d at 80% around a #060606 card at 40%.
            assert_eq!(hex(shell_fill()), 0x0d0d0d);
            assert!((shell_fill().a - 0.8).abs() < 0.001, "{:?}", shell_fill());
            assert_eq!(hex(inset_pane_fill()), 0x060606);
            assert_eq!(inset_pane_fill().a, 0.4);
        }
        // Content sits darker than the chrome around it.
        assert!(inset_pane_fill().l < shell_fill().l);

        // Without main pane glass the card is the solid #141414.
        preferences.body_glass = false;
        configure_project(&preferences, None, false, false);
        assert_eq!(hex(inset_pane_fill()), 0x141414);
        assert_eq!(inset_pane_fill().a, 1.);

        // The saved lightness and opacity still steer both planes.
        preferences.body_glass = true;
        preferences.theme_dark_lightness = 18;
        preferences.sidebar_opacity = 40;
        configure_project(&preferences, None, false, false);
        assert_eq!(hex(shell_fill()), 0x1a1a1a, "twice the default lightness");
        if GLASS_PLATFORM {
            assert!(shell_fill().a < 0.8, "a lower opacity shows more desktop");
        }
        preferences.theme_dark_lightness = 0;
        configure_project(&preferences, None, false, false);
        assert_eq!(hex(shell_fill()), 0, "zero is true black");

        // Light mode keeps a white card.
        let light = crate::settings::Preferences {
            theme: "light".into(),
            ..Default::default()
        };
        configure_project(&light, None, true, false);
        assert_eq!(inset_pane_fill().l, 1.);
    }

    #[test]
    fn configuring_the_same_colors_again_repaints_nothing() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        let preferences = crate::settings::Preferences {
            theme: "dark".into(),
            ..Default::default()
        };
        configure_project(&preferences, None, false, false);
        let shell = shell_fill();
        assert!(!configure_project(&preferences, None, false, false));
        assert_eq!(shell_fill(), shell);
    }

    #[test]
    fn opaque_inset_window_paints_every_surface_solid() {
        struct Restore(Option<Tokens>);
        impl Drop for Restore {
            fn drop(&mut self) {
                TOKENS.with(|tokens| *tokens.borrow_mut() = self.0);
            }
        }
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        let preferences = crate::settings::Preferences {
            theme: "dark".into(),
            inset_glass: false,
            sidebar_opacity: 40,
            ..Default::default()
        };
        configure_project(&preferences, None, false, false);
        assert_eq!(shell_fill().a, 1.);
        assert_eq!(pane_fill().a, 1.);
        assert_eq!(inset_pane_fill().a, 1.);
    }

    #[test]
    fn kiro_shows_its_ghost_on_its_own_purple_in_every_picker() {
        use gpui::AssetSource;
        let (path, monochrome, scale) = provider_mark("kiro");
        assert_ne!(
            path,
            provider_mark("unknown").0,
            "kiro must not use the codex fallback"
        );
        // Drawn as an image in its brand colours, like Claude and Codex,
        // instead of a theme-ink mask that reads as a plain white ghost.
        assert_eq!(path, "kiro-color.svg");
        assert!(!monochrome, "the ghost keeps its brand colours");
        assert_eq!(scale, 1.);
        let svg = crate::assets::Assets
            .load(path)
            .unwrap()
            .expect("embedded kiro-color.svg");
        let svg = String::from_utf8_lossy(&svg);
        assert!(svg.contains("<svg"));
        // The icon's purple is the tab tile's, so the two never drift apart.
        let tile = |hsla: Hsla| {
            let rgba = gpui::Rgba::from(hsla);
            format!(
                "#{:02X}{:02X}{:02X}",
                (rgba.r * 255.).round() as u8,
                (rgba.g * 255.).round() as u8,
                (rgba.b * 255.).round() as u8
            )
        };
        let (top, bottom) = provider_tile("kiro").expect("kiro has a tile");
        assert!(svg.contains(&tile(top)), "{svg} uses the tile's top colour");
        assert!(
            svg.contains(&tile(bottom)),
            "{svg} uses the tile's bottom colour"
        );
        assert!(svg.contains("fill=\"#fff\""), "a white ghost on the purple");
        // On a coloured tab tile the plain ghost still sits in white.
        assert!(crate::assets::Assets.load("kiro.svg").unwrap().is_some());
        // Every other provider keeps its own mark.
        assert_eq!(provider_mark("hermes").0, "hermes.svg");
        assert_eq!(provider_mark("devin").0, "devin.svg");
    }

    #[test]
    fn composer_surface_matches_react_dark_ink_and_solid_light_background() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        let mut preferences = crate::settings::Preferences::default();
        configure(&preferences, false);
        assert_eq!(composer_fill(), content(0.03));
        assert_eq!(composer_blur(), 24.);
        assert_eq!(composer_border(false), content(0.1));
        assert_eq!(composer_border(true), content(0.2));

        preferences.theme = "light".into();
        preferences.theme_hue = 180;
        preferences.theme_saturation = 20;
        configure(&preferences, false);
        assert_eq!(composer_fill(), gpui::hsla(0.5, 0.2, 0.97, 1.));
        assert_eq!(composer_blur(), 0.);
        assert_eq!(composer_border(false), content(0.1));
        assert_eq!(composer_border(true), content(0.2));
    }

    #[test]
    fn project_colors_replace_the_app_colors_and_blend_in() {
        let preferences = crate::settings::Preferences::default();
        configure(&preferences, false);
        let app_accent = accent_color();
        let theme = gespan_core::project_appearance::ProjectTheme {
            accent_color: Some("#ef4444".into()),
            hue: 20,
            saturation: 40,
            ..Default::default()
        };
        assert!(configure_project(&preferences, Some(&theme), false, true));
        assert!(tweening());
        // The blend starts from the colors on screen.
        assert_eq!(accent_color(), app_accent);
        std::thread::sleep(TWEEN_LENGTH);
        configure_project(&preferences, Some(&theme), false, true);
        assert!(!tweening());
        assert_eq!(accent_color(), rgb(0xef4444).into());
        assert_eq!(picked_accent(), rgb(0xef4444).into());
        assert_eq!(shell_fill().h, 20. / 360.);
        // Without animation the app colors return at once.
        assert!(configure_project(&preferences, None, false, false));
        assert!(!tweening());
        assert_eq!(accent_color(), app_accent);
    }
}

#[cfg(test)]
mod nav_tint_tests {
    use super::*;

    struct Restore(Option<Tokens>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TOKENS.with(|tokens| *tokens.borrow_mut() = self.0);
        }
    }

    fn with_theme(theme: &str) {
        let preferences = crate::settings::Preferences {
            theme: theme.into(),
            ..Default::default()
        };
        configure(&preferences, false);
    }

    /// Every row the sidebar can list, plus Search. Flows is hidden from
    /// the UI and keeps plain ink.
    const TINTED: [&str; 8] = [
        "New thread",
        "Inbox",
        "Notes",
        "Search",
        "Automations",
        "Board",
        "Runs",
        "Settings",
    ];

    #[test]
    fn home_is_drawn_in_the_bright_ink_not_the_muted_grey() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        for theme in ["dark", "light"] {
            with_theme(theme);
            let home = nav_tint("Home").expect("Home has a colour like the other rows");
            assert_eq!(home, content(1.), "{theme}: Home is the full ink");
        }
        with_theme("dark");
        let dark: Hsla = rgb(0xebebeb).into();
        assert_eq!(nav_tint("Home"), Some(dark), "the design's #ebebeb");
    }

    #[test]
    fn every_navigation_row_gets_its_own_colour() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        for theme in ["dark", "light"] {
            with_theme(theme);
            let tints: Vec<Hsla> = TINTED
                .iter()
                .map(|label| nav_tint(label).expect("tinted row"))
                .collect();
            for (i, a) in tints.iter().enumerate() {
                let label = TINTED[i];
                assert!(a.s > 0.5, "{theme}: {label} is saturated, not grey ink");
                assert_eq!(a.a, 1., "{theme}: {label} is opaque");
                for (j, b) in tints.iter().enumerate().skip(i + 1) {
                    // Hues far enough apart that no two rows read as one colour.
                    let gap = (a.h - b.h).abs().min(1. - (a.h - b.h).abs());
                    assert!(gap > 0.04, "{theme}: {label} and {} share a hue", TINTED[j]);
                }
            }
        }
    }

    #[test]
    fn title_tabs_mark_each_page_in_its_sidebar_colour() {
        use crate::workspace::AppView;
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        for theme in ["dark", "light"] {
            with_theme(theme);
            // Keyed by the label the tab draws, so a renamed page can't
            // quietly fall back to plain ink.
            for view in [
                AppView::MyTasks,
                AppView::Inbox,
                AppView::Notes,
                AppView::Automations,
                AppView::Board,
                AppView::Runs,
            ] {
                let tint = nav_tint(view.label()).expect("page has a tint");
                assert_eq!(nav_tile_ink(view.label()), tint, "{theme}: {view:?}");
            }
            assert_eq!(nav_tile_ink("Search"), nav_tint("Search").unwrap());
            assert_eq!(nav_tile_ink("Settings"), nav_tint("Settings").unwrap());
            // Pages without a tint keep the tile's own ink.
            assert_eq!(
                nav_tile_ink(AppView::Flows.label()),
                crate::tab_tile::tile_ink()
            );
        }
    }

    #[test]
    fn other_rows_stay_untinted() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        with_theme("dark");
        for label in ["Flows", "search", "inbox", ""] {
            assert_eq!(nav_tint(label), None, "{label:?} stays untinted");
        }
    }

    #[test]
    fn light_themes_take_a_deeper_shade_than_dark_ones() {
        let _restore = Restore(TOKENS.with(|tokens| *tokens.borrow()));
        for label in TINTED {
            with_theme("dark");
            let dark = nav_tint(label).unwrap();
            with_theme("light");
            let light = nav_tint(label).unwrap();
            assert!(
                light.l < dark.l,
                "{label}: light tint {light:?} is darker than dark tint {dark:?} for contrast on white"
            );
        }
    }
}
