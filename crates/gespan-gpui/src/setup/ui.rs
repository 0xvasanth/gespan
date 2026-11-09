//! The setup card's shared pieces, sized and coloured to the design canvas
//! (docs/design/first-run-setup.md, "Card spec"). Every step builds from
//! these so the four cards stay identical in frame, header and footer.
use crate::theme::*;
use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, Role, SharedString, Stateful, canvas, div,
    linear_color_stop, linear_gradient, prelude::*, px, rgb, rgba,
};

pub(crate) const WIDTH: f32 = 720.;
pub(crate) const RADIUS: f32 = 24.;
#[cfg(any(target_os = "macos", target_os = "windows"))]
const BLUR: f32 = 40.;

/// Violet 300 from the logo ramp: progress, radios and selected rings.
pub(crate) fn violet() -> Hsla {
    rgb(0xaf97fa).into()
}
/// The selected option's fill.
pub(crate) fn selected_fill() -> Hsla {
    rgba(0x8f64f61f).into()
}
/// The selected option's 1px edge.
pub(crate) fn selected_edge() -> Hsla {
    rgba(0xaf97fa8c).into()
}
/// The 1px glass edge, the popovers' 10% hairline.
pub(crate) fn glass_edge() -> Hsla {
    content(if is_light() { 0.08 } else { 0.10 })
}
/// The frosted pane under lists and options: a thin ink tint, as the
/// composer's surface is, so the card's blur shows through it.
pub(crate) fn well() -> Hsla {
    content(if is_light() { 0.03 } else { 0.04 })
}
/// Rules inside the card: the footer line and list separators.
pub(crate) fn rule() -> Hsla {
    if is_light() {
        content(0.07)
    } else {
        rgba(0xffffff12).into()
    }
}
/// Secondary text, #a3a3a3 on dark.
pub(crate) fn muted() -> Hsla {
    content(0.64)
}
/// Meta text, #737373 on dark.
pub(crate) fn faint() -> Hsla {
    content(0.45)
}

/// The frosted fill behind the card: a 6% ink tint under a 40pt blur, the
/// composer's glass, so the shell reads through it. Light themes paint it
/// opaque, as every other modal does.
fn glass(radius: f32) -> impl IntoElement {
    let light = is_light();
    let tint = content(0.06);
    let opaque = floating_fill();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            let quad = if light {
                gpui::fill(bounds, opaque)
            } else {
                gpui::fill(bounds, tint)
                    .border_color(gpui::backdrop_blur_marker(BLUR * window.scale_factor()))
            };
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let quad = {
                let _ = (light, tint, &window);
                gpui::fill(bounds, opaque)
            };
            window.paint_quad(quad.corner_radii(px(radius)));
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The light falling across the glass: a faint white wash at the top that
/// fades out a third of the way down.
fn sheen() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .rounded(px(RADIUS - 1.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(white(0.07), 0.),
            linear_color_stop(white(0.), 0.35),
        ))
}

/// The 720pt card: glass, a hairline edge, the long soft drop and the
/// 28/40/28 padding. `top` is 36 on Welcome, 28 elsewhere. Add the card's
/// blocks with `.child`: the glass layers are out of flow, so they take no
/// gap.
pub(crate) fn card(top: f32, gap: f32) -> Div {
    div()
        .relative()
        .w(px(WIDTH))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(glass_edge())
        .shadow(vec![
            gpui::BoxShadow::new(px(0.), px(24.), black(if is_light() { 0.12 } else { 0.55 }))
                .blur_radius(px(64.)),
        ])
        .child(glass(RADIUS - 1.))
        .when(!is_light(), |card| card.child(sheen()))
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(RADIUS))
                .right(px(RADIUS))
                .h(px(1.))
                .bg(if is_light() { white(0.95) } else { white(0.18) }),
        )
        .text_color(content(1.))
        .text_size(px(14.))
        .pt(px(top))
        .px(px(40.))
        .pb(px(28.))
        .flex()
        .flex_col()
        .gap(px(gap))
}

/// "Step N of 3", the three progress pills and, unless on Project, "Skip
/// setup" on the right.
pub(crate) fn header(step: usize, skip: Option<Stateful<Div>>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(muted())
                .child(SharedString::from(format!("Step {step} of 3"))),
        )
        .child(div().flex().gap(px(4.)).children((1..=3).map(|n| {
            div().w(px(28.)).h(px(4.)).rounded_full().bg(if n <= step {
                violet()
            } else {
                content(0.14)
            })
        })))
        .child(div().flex_1())
        .children(skip)
}

/// A text link styled control: "Skip setup", "Back".
pub(crate) fn text_button(
    id: impl Into<ElementId>,
    label: &'static str,
    size: f32,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .text_size(px(size))
        .text_color(muted())
        .hover(|s| s.text_color(content(1.)))
        .child(label)
}

/// The card title: 28pt (34 on Welcome), medium weight.
pub(crate) fn title(text: &'static str, size: f32) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(size * 1.15))
        .font_weight(FontWeight::MEDIUM)
        .child(text)
}

/// The lead under a title: 14/22 (16/25 on Welcome) in muted ink.
pub(crate) fn lead(text: impl Into<SharedString>, size: f32, line: f32) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(line))
        .text_color(muted())
        .child(text.into())
}

/// The light primary pill with the brand glow; dimmed and inert when
/// `enabled` is false.
pub(crate) fn primary(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let base = div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .px(px(20.))
        .py(px(9.))
        .rounded_full()
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .child(label);
    if enabled {
        base.cursor_pointer()
            .bg(content(0.92))
            .text_color(if is_light() {
                white(1.)
            } else {
                rgb(0x111111).into()
            })
            .shadow(vec![
                gpui::BoxShadow::new(px(0.), px(0.), rgba(0xaf97fa59).into()).spread_radius(px(1.)),
                gpui::BoxShadow::new(px(0.), px(8.), rgba(0x8f64f673).into()).blur_radius(px(32.)),
            ])
            .hover(|s| s.bg(content(0.8)))
    } else {
        base.bg(content(0.12)).text_color(faint())
    }
}

/// The ghost pill: "Skip".
pub(crate) fn ghost(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .px(px(16.))
        .py(px(9.))
        .rounded_full()
        .text_color(muted())
        .hover(|s| s.bg(content(0.07)).text_color(content(1.)))
        .child(label)
}

/// The footer: a rule with 16pt above it, Back on the left and the given
/// actions on the right.
pub(crate) fn footer(back: Option<Stateful<Div>>, right: Vec<AnyElement>) -> Div {
    div()
        .pt(px(16.))
        .border_t_1()
        .border_color(rule())
        .flex()
        .items_center()
        .gap(px(12.))
        .children(back)
        .child(div().flex_1())
        .children(right)
}

/// An 18pt radio: a violet ring and 8pt dot when on, a faint ring when off.
pub(crate) fn radio(on: bool) -> Div {
    div()
        .size(px(18.))
        .flex_shrink_0()
        .rounded_full()
        .border(px(1.5))
        .border_color(if on { violet() } else { content(0.30) })
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| {
            d.child(div().size(px(8.)).rounded_full().bg(violet()))
        })
}

/// A 7pt status dot.
pub(crate) fn dot(color: Hsla) -> Div {
    div().size(px(7.)).rounded_full().bg(color)
}
pub(crate) fn green() -> Hsla {
    rgb(0x10b981).into()
}
pub(crate) fn yellow() -> Hsla {
    rgb(0xe8c547).into()
}
