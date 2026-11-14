//! Glass parts shared by My tasks, Inbox, Runs and Automations: one level of
//! frosted card over the content plane, a segmented control in place of chip
//! rows, status pills that pair a dot with a word, and one page header.
//!
//! Values follow the Gespan design system's glass tokens (`glass-fill`,
//! `glass-edge`, `glass-highlight`). Light themes swap the white washes for
//! opaque-leaning fills so cards still separate from the page.
// Each page uses its own subset of these parts until all four pages land.
// A panic while drawing takes the whole app down, so render code here may
// not index, slice, unwrap or expect: use `get`, `let ... else` or a default.
// Tests may (clippy.toml). See `settings/render_smoke_tests.rs`.
#![deny(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]
#![allow(dead_code)]
use crate::theme::{accent_color, black, content, diff_foreground, is_light, white};
use gpui::{
    AnyElement, BoxShadow, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Stateful, Styled, div, prelude::FluentBuilder, px, rgb,
};

/// A card's fill: `glass-fill`, white at 3.5% over the dark plane.
pub(crate) fn fill() -> Hsla {
    if is_light() {
        white(0.62)
    } else {
        white(0.035)
    }
}
/// The raised card's fill: `glass-fill-strong`.
pub(crate) fn fill_strong() -> Hsla {
    if is_light() { white(0.85) } else { white(0.06) }
}
/// `glass-edge`: the 1px border on every glass surface.
pub(crate) fn edge() -> Hsla {
    if is_light() {
        content(0.08)
    } else {
        white(0.09)
    }
}
/// Hairline between rows inside a card.
pub(crate) fn hairline() -> Hsla {
    content(0.06)
}
/// Row hover and selection washes.
pub(crate) fn hover_fill() -> Hsla {
    content(0.06)
}
pub(crate) fn selected_fill() -> Hsla {
    content(0.10)
}

/// `glass-highlight`: the 1px specular line along a card's top edge that
/// makes it read as glass instead of grey.
fn highlight(alpha: f32) -> BoxShadow {
    let color = if is_light() {
        white(0.95)
    } else {
        white(alpha)
    };
    BoxShadow::new(px(0.), px(1.), color).inset()
}

/// The default glass card: lists, side panels, checks.
pub(crate) fn card() -> Div {
    div()
        .rounded(px(14.))
        .bg(fill())
        .border_1()
        .border_color(edge())
        .shadow(vec![highlight(0.10)])
}

/// A raised card for the one thing the page asks of you (My tasks' answer,
/// Inbox's ask bar): stronger fill, brighter top line and a long soft drop.
pub(crate) fn card_strong() -> Div {
    let drop = BoxShadow::new(px(0.), px(20.), black(if is_light() { 0.10 } else { 0.35 }))
        .blur_radius(px(50.));
    div()
        .rounded(px(16.))
        .bg(fill_strong())
        .border_1()
        .border_color(if is_light() {
            content(0.1)
        } else {
            white(0.12)
        })
        .shadow(vec![highlight(0.14), drop])
}

/// Title and one line saying what the page is for.
pub(crate) fn page_header(title: &'static str, subtitle: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(26.))
                .line_height(px(30.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(title),
        )
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(content(0.62))
                .child(subtitle),
        )
}

/// A small uppercase group label: day headings, repositories, task ages.
pub(crate) fn eyebrow(label: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(content(0.55))
        .child(label.into().to_uppercase())
}

/// A count tile beside a page header: a big number over its label.
pub(crate) fn stat(id: &'static str, value: String, label: &'static str, ink: Hsla) -> AnyElement {
    card()
        .debug_selector(move || id.into())
        .rounded(px(12.))
        .min_w(px(116.))
        .px(px(14.))
        .py(px(10.))
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(22.))
                .line_height(px(24.2))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ink)
                .child(value),
        )
        .child(
            div()
                .mt(px(2.))
                .text_size(px(11.5))
                .line_height(px(14.))
                .text_color(content(0.62))
                .child(label),
        )
        .into_any_element()
}

/// The segmented control's track. Put [`segment`]s inside.
pub(crate) fn segmented() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .p(px(3.))
        .rounded(px(10.))
        .bg(if is_light() {
            content(0.06)
        } else {
            black(0.28)
        })
        .border_1()
        .border_color(edge())
}

/// One option in a [`segmented`] track. Selected carries a raised thumb.
pub(crate) fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    let thumb = if is_light() { white(1.) } else { white(0.12) };
    div()
        .id(id)
        .cursor_pointer()
        .h(px(26.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .text_size(px(12.5))
        .whitespace_nowrap()
        .text_color(content(if selected { 1. } else { 0.62 }))
        .when(selected, |d| {
            d.bg(thumb).shadow(vec![
                highlight(0.12),
                BoxShadow::new(px(0.), px(1.), black(0.3)).blur_radius(px(3.)),
            ])
        })
        .when(!selected, |d| d.hover(|s| s.text_color(content(1.))))
        // A segment squeezed below its label's width truncates the label.
        .child(div().min_w_0().truncate().child(label.into()))
}

/// A count inside a segment or tab. `hot` marks what waits on you.
pub(crate) fn count(n: usize, hot: bool) -> Div {
    let (ink, fill) = if hot {
        (skill_ink(), skill_ink().opacity(0.22))
    } else {
        (content(0.9), content(0.1))
    };
    div()
        .h(px(16.))
        .min_w(px(18.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(fill)
        .text_size(px(11.))
        .text_color(ink)
        .child(n.to_string())
}

/// `skill` yellow: waiting states.
fn skill_ink() -> Hsla {
    if is_light() {
        rgb(0xa07c10).into()
    } else {
        rgb(0xe8c547).into()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tone {
    Ok,
    Bad,
    Running,
    Waiting,
    Muted,
}

impl Tone {
    /// The tone a run status shows in.
    pub(crate) fn of_status(status: &str) -> Self {
        match status {
            "Succeeded" => Self::Ok,
            "Failed" => Self::Bad,
            "Queued" | "Running" => Self::Running,
            _ => Self::Muted,
        }
    }
    pub(crate) fn ink(self) -> Hsla {
        match self {
            Self::Ok => diff_foreground(true),
            Self::Bad => diff_foreground(false),
            Self::Running => accent_color(),
            Self::Waiting => skill_ink(),
            Self::Muted => content(0.62),
        }
    }
}

/// A status: a dot and a word on a 10% tint of the same colour, so colour is
/// never the only cue.
pub(crate) fn status(label: impl Into<SharedString>, tone: Tone) -> Div {
    let ink = tone.ink();
    let (fill, border) = match tone {
        Tone::Muted => (content(0.05), edge()),
        _ => (ink.opacity(0.10), ink.opacity(0.22)),
    };
    div()
        .flex_none()
        .h(px(22.))
        .pl(px(8.))
        .pr(px(9.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .bg(fill)
        .border_1()
        .border_color(border)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .child(div().size(px(6.)).rounded_full().bg(ink))
        .child(label.into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    /// Content-coloured fill with base-coloured ink: the one main answer.
    Primary,
    /// A glass plate: secondary answers.
    Glass,
    /// Text only: the quiet way out.
    Ghost,
}

/// A 32px button in one of the three glass weights.
pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: Button,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .cursor_pointer()
        .h(px(32.))
        .px(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .rounded(px(9.))
        .text_size(px(13.))
        .whitespace_nowrap()
        .child(label.into());
    match kind {
        Button::Primary => base
            .bg(content(0.94))
            .text_color(crate::theme::background_base())
            .font_weight(FontWeight::SEMIBOLD)
            .shadow(vec![
                highlight(0.4),
                BoxShadow::new(px(0.), px(6.), black(0.3)).blur_radius(px(18.)),
            ])
            .hover(|s| s.bg(content(1.))),
        Button::Glass => base
            .bg(fill_strong())
            .border_1()
            .border_color(edge())
            .text_color(content(0.9))
            .shadow(vec![highlight(0.10)])
            .hover(|s| s.bg(content(0.12)).text_color(content(1.))),
        Button::Ghost => base
            .text_color(content(0.62))
            .hover(|s| s.bg(content(0.08)).text_color(content(1.))),
    }
}

/// The well a search or note field sits in.
pub(crate) fn field() -> Div {
    div()
        .h(px(32.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(9.))
        .bg(if is_light() { white(0.7) } else { black(0.25) })
        .border_1()
        .border_color(edge())
}

/// A centred empty state: a glass orb with a glyph, a sentence and a hint.
pub(crate) fn empty_state(
    icon: &'static str,
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .text_center()
        .child(
            div()
                .size(px(56.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(fill_strong())
                .border_1()
                .border_color(if is_light() {
                    content(0.1)
                } else {
                    white(0.14)
                })
                .shadow(vec![
                    highlight(0.2),
                    BoxShadow::new(px(0.), px(12.), accent_color().opacity(0.18))
                        .blur_radius(px(40.)),
                ])
                .child(crate::theme::icon(icon, 22.).text_color(content(0.9))),
        )
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(content(0.62))
                .child(hint.into()),
        )
}

/// A 30px square action on a glass plate, for actions whose glyph says
/// enough on its own. Give it a role, an accessible name and a tooltip.
pub(crate) fn icon_button(id: impl Into<ElementId>, icon: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .size(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(fill())
        .border_1()
        .border_color(edge())
        .text_color(content(0.62))
        .hover(|s| s.bg(hover_fill()).text_color(content(1.)))
        .child(crate::theme::icon(icon, 15.).text_color(content(0.75)))
}

/// An on/off chip: one of several choices that each switch on their own,
/// laid out in a wrapping row.
pub(crate) fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .h(px(28.))
        .px(px(11.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .border_1()
        .text_size(px(12.))
        .whitespace_nowrap()
        .map(|d| {
            if on {
                d.bg(content(0.10))
                    .border_color(content(0.18))
                    .text_color(content(1.))
            } else {
                d.border_color(edge())
                    .text_color(content(0.62))
                    .hover(|s| s.bg(hover_fill()).text_color(content(1.)))
            }
        })
        .child(label.into())
}

/// A [`segment`] whose word follows a status dot in `tone`, for filters by
/// status. Added by the Runs page.
pub(crate) fn dot_segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    tone: Tone,
) -> Stateful<Div> {
    let thumb = if is_light() { white(1.) } else { white(0.12) };
    div()
        .id(id)
        .cursor_pointer()
        .h(px(26.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .text_size(px(12.5))
        .whitespace_nowrap()
        .text_color(content(if selected { 1. } else { 0.62 }))
        .when(selected, |d| {
            d.bg(thumb).shadow(vec![
                highlight(0.12),
                BoxShadow::new(px(0.), px(1.), black(0.3)).blur_radius(px(3.)),
            ])
        })
        .when(!selected, |d| d.hover(|s| s.text_color(content(1.))))
        .child(div().flex_none().size(px(6.)).rounded_full().bg(tone.ink()))
        .child(label.into())
}

/// A violet tag beside a title for something early: Beta, Invite-only beta.
pub(crate) fn beta(label: impl Into<SharedString>) -> Div {
    let ink: Hsla = if is_light() {
        rgb(0x6d4fd6).into()
    } else {
        rgb(0xc0affc).into()
    };
    div()
        .flex_none()
        .px(px(7.))
        .py(px(2.))
        .rounded_full()
        .bg(ink.opacity(0.12))
        .border_1()
        .border_color(ink.opacity(0.25))
        .text_size(px(10.5))
        .line_height(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ink)
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_statuses_map_to_the_tones_their_colours_mean() {
        assert_eq!(Tone::of_status("Succeeded"), Tone::Ok);
        assert_eq!(Tone::of_status("Failed"), Tone::Bad);
        // Queued shows as in progress, the way the old chip coloured it.
        assert_eq!(Tone::of_status("Queued"), Tone::Running);
        assert_eq!(Tone::of_status("Running"), Tone::Running);
        assert_eq!(Tone::of_status("Cancelled"), Tone::Muted);
        assert_eq!(Tone::of_status("Skipped"), Tone::Muted);
    }
}
