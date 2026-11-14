//! The setup card's shared pieces, sized and coloured to the onboarding
//! boards of the design canvas (OnboardWelcome/Agents/Look/Project): a 620pt
//! glass card, a 3-step bar in the accent, options on a darker inset list
//! and a footer with Back on the left. Every step builds from these so the
//! four cards stay identical in frame, header and footer.
use crate::glass::{self, Button};
use crate::theme::*;
use gpui::{
    AnyElement, BoxShadow, Div, ElementId, FontWeight, Hsla, Role, SharedString, Stateful, canvas,
    div, linear_color_stop, linear_gradient, prelude::*, px, rgb,
};

pub(crate) const WIDTH: f32 = 620.;
pub(crate) const RADIUS: f32 = 22.;
#[cfg(any(target_os = "macos", target_os = "windows"))]
const BLUR: f32 = 30.;

/// The accent at `alpha`: the progress bar, radios and the picked option's
/// tint all follow the user's accent colour.
pub(crate) fn accent(alpha: f32) -> Hsla {
    let mut color = accent_color();
    color.a = alpha;
    color
}
/// The card's 1px edge: the strong glass card's 12% white.
fn card_edge() -> Hsla {
    if is_light() {
        content(0.1)
    } else {
        white(0.12)
    }
}
/// Rules inside the card: the footer line and list separators.
pub(crate) fn rule() -> Hsla {
    content(0.07)
}
/// Secondary text, #a3a3a3 on dark.
pub(crate) fn muted() -> Hsla {
    content(0.62)
}
/// Meta text, #8a8a8a on dark.
pub(crate) fn faint() -> Hsla {
    content(0.55)
}
/// The darker inset well that lists and the welcome explainer sit in.
pub(crate) fn inset() -> Hsla {
    if is_light() {
        content(0.03)
    } else {
        black(0.14)
    }
}
/// The inset well's 1px edge.
pub(crate) fn inset_edge() -> Hsla {
    if is_light() {
        content(0.08)
    } else {
        white(0.08)
    }
}

/// The frosted backdrop behind the card: a blur of whatever is behind it.
/// Light themes paint it opaque, as every other modal does.
fn blur(radius: f32) -> impl IntoElement {
    let light = is_light();
    let opaque = floating_fill();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            let quad = if light {
                gpui::fill(bounds, opaque)
            } else {
                gpui::fill(bounds, gpui::transparent_black())
                    .border_color(gpui::backdrop_blur_marker(BLUR * window.scale_factor()))
            };
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let quad = {
                let _ = (light, &window);
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

/// The card's glass fill: white at 7.5% at the top fading to the glass
/// fill's 3.5% at the bottom.
fn sheen() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .rounded(px(RADIUS - 1.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(white(0.075), 0.),
            linear_color_stop(glass::fill(), 1.),
        ))
}

/// The 620pt card: blur, gradient glass, a 12% edge with the glass top
/// highlight, the long soft drop and 30/32/22 padding with 18pt between
/// blocks. Add blocks with `.child`: the glass layers are out of flow, so
/// they take no gap.
pub(crate) fn card() -> Stateful<Div> {
    let light = is_light();
    div()
        .id("setup-card")
        .debug_selector(|| "setup-card".into())
        .role(Role::Dialog)
        .aria_label("Setup")
        .relative()
        .w(px(WIDTH))
        .flex_shrink_0()
        .rounded(px(RADIUS))
        .border_1()
        .border_color(card_edge())
        .shadow(vec![
            BoxShadow::new(
                px(0.),
                px(1.),
                if light { white(0.95) } else { white(0.10) },
            )
            .inset(),
            BoxShadow::new(px(0.), px(30.), black(if light { 0.12 } else { 0.45 }))
                .blur_radius(px(80.)),
        ])
        .child(blur(RADIUS - 1.))
        .when(!light, |card| card.child(sheen()))
        .text_color(content(1.))
        .text_size(px(13.))
        .pt(px(30.))
        .px(px(32.))
        .pb(px(22.))
        .flex()
        .flex_col()
        .gap(px(18.))
}

/// The card's top row: the step bar (or Welcome's mark) and, unless on
/// Project, "Skip setup" on the right.
pub(crate) fn header(lead: impl IntoElement, skip: Option<Stateful<Div>>) -> Div {
    div()
        .flex()
        .items_center()
        .child(lead)
        .child(div().flex_1())
        .children(skip)
}

/// "Step N of 3" and three 26×3 bars, the first `step` in the accent.
pub(crate) fn steps(step: usize) -> Div {
    div()
        .debug_selector(|| "setup-step-header".into())
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(px(12.))
        .text_color(faint())
        .child(SharedString::from(format!("Step {step} of 3")))
        .children((1..=3).map(|n| {
            div().w(px(26.)).h(px(3.)).rounded(px(2.)).bg(if n <= step {
                accent(1.)
            } else {
                content(0.12)
            })
        }))
}

/// The quiet "Skip setup" link in the header.
pub(crate) fn skip_link(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label("Skip setup")
        .cursor_pointer()
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(faint())
        .hover(|s| s.text_color(content(1.)))
        .child("Skip setup")
}

/// The card title over its lead: 26pt semibold, then 13.5/21 muted, 8pt
/// apart.
pub(crate) fn intro(title: &'static str, lead: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(26.))
                .line_height(px(31.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(13.5))
                .line_height(px(21.))
                .text_color(muted())
                .child(lead.into()),
        )
}

/// 13/18 body ink.
pub(crate) fn t1(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .line_height(px(18.))
        .child(text.into())
}
/// 12/17 secondary.
pub(crate) fn t2(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(muted())
        .child(text.into())
}
/// 11.5/16 meta.
pub(crate) fn t3(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(faint())
        .child(text.into())
}

/// The main answer: a 34pt glass primary. Dimmed and inert when `enabled`
/// is false.
pub(crate) fn primary(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let button = glass::button(id, label.clone(), Button::Primary)
        .role(Role::Button)
        .aria_label(label)
        .h(px(34.))
        .px(px(18.));
    if enabled {
        button
    } else {
        button.opacity(0.35).cursor_default()
    }
}

/// The quiet way on: "Skip" (34pt) and "Back" (32pt).
pub(crate) fn ghost(id: impl Into<ElementId>, label: &'static str, height: f32) -> Stateful<Div> {
    glass::button(id, label, Button::Ghost)
        .role(Role::Button)
        .aria_label(label)
        .h(px(height))
}

/// The footer: a rule with 14pt above it, `left` (Back or a note) and the
/// given actions on the right.
pub(crate) fn footer(left: Option<AnyElement>, right: Vec<AnyElement>) -> Div {
    div()
        .pt(px(14.))
        .border_t_1()
        .border_color(rule())
        .flex()
        .items_center()
        .gap(px(10.))
        .children(left)
        .child(div().flex_1())
        .child(div().flex().items_center().gap(px(8.)).children(right))
}

/// The darker inset list options sit in, 14pt round.
pub(crate) fn list() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(14.))
        .overflow_hidden()
        .border_1()
        .border_color(inset_edge())
        .bg(inset())
}

/// A 54pt list row; every row but the first has a hairline above it.
pub(crate) fn row(first: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(54.))
        .flex_shrink_0()
        .px(px(14.))
        .text_size(px(13.))
        .when(!first, |d| d.border_t_1().border_color(content(0.06)))
}

/// A picked row's tint: the accent at 10%.
pub(crate) fn picked() -> Hsla {
    accent(0.10)
}

/// A 15pt radio: a 4.5pt accent ring round a white centre when on, a faint
/// ring when off.
pub(crate) fn radio(on: bool) -> Div {
    let ring = div().size(px(15.)).flex_shrink_0().rounded_full();
    if on {
        ring.border(px(4.5)).border_color(accent(1.)).bg(white(1.))
    } else {
        ring.border(px(1.5)).border_color(content(0.28))
    }
}

/// The 30pt glyph tile: a faint plate with a hairline edge.
pub(crate) fn tile() -> Div {
    div()
        .size(px(30.))
        .flex_shrink_0()
        .rounded(px(8.))
        .bg(content(0.06))
        .border_1()
        .border_color(inset_edge())
        .flex()
        .items_center()
        .justify_center()
}

/// A 7pt status dot.
pub(crate) fn dot(color: Hsla) -> Div {
    div().size(px(7.)).flex_shrink_0().rounded_full().bg(color)
}
pub(crate) fn green() -> Hsla {
    rgb(0x10b981).into()
}
pub(crate) fn yellow() -> Hsla {
    rgb(0xe8c547).into()
}
