//! The ticket chip a session row and a board card both show: tracker, key,
//! title and a status pill. The caller adds what a click does.
use crate::theme::*;
use gespan_core::{
    provider_boards::StateGroup,
    tickets::{TicketRef, Tracker},
};
use gpui::{ElementId, Render, Role, SharedString, Stateful, Window, div, prelude::*, px, rgb};

/// What the ticket chip shows, apart from how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TicketChip {
    pub(crate) tracker: Tracker,
    pub(crate) key: String,
    pub(crate) title: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) tone: u32,
    pub(crate) stale: bool,
}
impl TicketChip {
    pub(crate) fn new(ticket: &TicketRef, stale: bool) -> Self {
        Self {
            tracker: ticket.tracker,
            key: ticket.key.clone(),
            title: ticket.title.clone().filter(|title| !title.is_empty()),
            status: ticket.status.clone().filter(|status| !status.is_empty()),
            tone: ticket.status_group.map_or(GROUP_UNKNOWN, group_tone),
            stale,
        }
    }
}
impl TicketChip {
    pub(crate) fn stale_hint(&self) -> String {
        format!(
            "{} is disconnected. Reconnect in Settings.",
            self.tracker.label()
        )
    }
}
struct HintTooltip(SharedString);
impl Render for HintTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut background = shell_fill();
        background.a = 1.;
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(background)
            .border_1()
            .border_color(content(0.12))
            .text_size(px(11.))
            .text_color(content(0.85))
            .child(self.0.clone())
    }
}
pub(crate) const GROUP_UNKNOWN: u32 = 0x8a8a8a;
pub(crate) fn group_tone(group: StateGroup) -> u32 {
    match group {
        StateGroup::Started => ACCENT,
        StateGroup::Completed => 0x4ade80,
        StateGroup::Cancelled => 0xf87171,
        StateGroup::Triage | StateGroup::Backlog | StateGroup::Unstarted | StateGroup::Unknown => {
            GROUP_UNKNOWN
        }
    }
}
fn tracker_icon(tracker: Tracker) -> &'static str {
    match tracker {
        Tracker::Jira => "inbox-jira",
        Tracker::Linear => "inbox-linear",
        Tracker::Plane => "inbox-plane",
    }
}

/// `key` names the chip for element ids and test selectors: a session's id
/// or a card's number.
pub(crate) fn chip_element(key: &str, chip: &TicketChip, compact: bool) -> Stateful<gpui::Div> {
    let tone = gpui::Hsla::from(rgb(chip.tone));
    let (selector, title_selector, status_selector, stale_selector) = (
        format!("ticket-chip-{key}"),
        format!("ticket-chip-title-{key}"),
        format!("ticket-chip-status-{key}"),
        format!("ticket-chip-stale-{key}"),
    );
    div()
        .id(ElementId::from(SharedString::from(selector.clone())))
        .debug_selector(move || selector.clone())
        .role(Role::Link)
        .aria_label(format!("Open {} in {}", chip.key, chip.tracker.label()))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(2.))
        .rounded_sm()
        .text_size(px(11.))
        .text_color(rgb(ACCENT))
        .child(icon(tracker_icon(chip.tracker), 12.).text_color(rgb(ACCENT)))
        .child(div().flex_shrink_0().child(chip.key.clone()))
        .when(!compact, |d| {
            d.when_some(chip.title.clone(), |d, title| {
                d.child(
                    div()
                        .debug_selector(move || title_selector.clone())
                        .min_w_0()
                        .max_w(px(120.))
                        .truncate()
                        .text_color(content(0.6))
                        .child(title),
                )
            })
            .when_some(chip.status.clone(), |d, status| {
                d.child(
                    div()
                        .debug_selector(move || status_selector.clone())
                        .flex_shrink_0()
                        .px(px(5.))
                        .rounded_full()
                        .bg(tone.opacity(0.15))
                        .text_color(tone)
                        .child(status),
                )
            })
        })
        .when(chip.stale, |d| {
            let hint = SharedString::from(chip.stale_hint());
            d.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "ticket-stale-{key}"
                    ))))
                    .debug_selector(move || stale_selector.clone())
                    .aria_label(hint.clone())
                    .tooltip(move |_, cx| cx.new(|_| HintTooltip(hint.clone())).into())
                    .flex_shrink_0()
                    .child(icon("clock", 11.).text_color(content(0.45))),
            )
        })
}
