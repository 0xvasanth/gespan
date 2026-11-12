//! The right pane: a top bar with where the task comes from, how long it has
//! waited and "‹ i of N ›"; its history, faded toward the bottom; and the
//! answer card pinned under it.
use super::*;
use crate::glass::{self, Button, Tone};
use crate::workspace::project_name;
use gpui::{AnyElement, Div, linear_color_stop, linear_gradient, px, relative};

/// Every icon the page draws outside the kind marks, so a test can check
/// each one exists.
#[cfg(test)]
pub(super) const ICONS: &[&str] = &[
    "external-link",
    "chevron-left",
    "chevron-right",
    "terminal",
    "search",
    "filter",
    "check",
    "clock",
    "chat",
];

/// The icon, sentence and hint of a tab's empty pane: one pair for a tab
/// with nothing at all, another for one whose search or filters hide it all.
pub(super) fn empty_copy(tab: Tab, has_any: bool) -> (&'static str, &'static str, &'static str) {
    const HIDDEN: &str = "Clear the search or filters to see everything.";
    match (tab, has_any) {
        (Tab::Waiting, false) => (
            "check",
            "You're all caught up.",
            "Approvals and questions from your agents land here.",
        ),
        (Tab::Waiting, true) => ("search", "No task is selected.", HIDDEN),
        (Tab::Answered, false) => (
            "clock",
            "Nothing answered yet.",
            "Every request you answer is kept here with your answer.",
        ),
        (Tab::Answered, true) => ("search", "No answers match the search or filters.", HIDDEN),
        (Tab::Decisions, false) => (
            "chat",
            "No decisions yet.",
            "Answer a question, then save it as a decision.",
        ),
        (Tab::Decisions, true) => (
            "search",
            "No decisions match the search or filters.",
            HIDDEN,
        ),
    }
}

/// The pane's top bar: the project, then where the request comes from,
/// with whatever else the tab shows after it.
pub(super) fn top_bar(project: &str, source: String) -> Div {
    div()
        .h(px(56.))
        .flex_shrink_0()
        .px(px(24.))
        .flex()
        .items_center()
        .gap(px(10.))
        .border_b_1()
        .border_color(glass::hairline())
        .text_size(px(12.))
        .child(
            div()
                .flex_shrink_0()
                .text_color(content(0.62))
                .child(project_name(project)),
        )
        .child(div().flex_shrink_0().text_color(content(0.4)).child("/"))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(content(0.92))
                .child(source),
        )
}

/// The wash under the answer card. The history fades into it, so the card
/// sits on a slightly darker floor with no seam above it.
fn dock_wash() -> gpui::Hsla {
    background_base().opacity(if is_light() { 0.5 } else { 0.4 })
}

/// The ghost button that opens the request where it was asked.
pub(super) fn open_button(label: &'static str) -> gpui::Stateful<Div> {
    glass::button("my-tasks-open-source", label, Button::Ghost)
        .debug_selector(|| "my-tasks-open-source".into())
        .h(px(30.))
        .child(icon("external-link", 13.))
}

impl MyTasks {
    /// A tab's empty pane, with any banner over it.
    pub(super) fn empty_pane(&self, banner: Option<AnyElement>) -> AnyElement {
        let has_any = match self.tab {
            Tab::Waiting => !self.tasks.is_empty(),
            Tab::Answered => !self.past.is_empty(),
            Tab::Decisions => !self.decisions.is_empty(),
        };
        let (glyph, title, hint) = empty_copy(self.tab, has_any);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .children(banner)
            .child(div().flex_1().flex().items_center().justify_center().child(
                glass::empty_state(glyph, title, hint).debug_selector(|| "my-tasks-empty".into()),
            ))
            .into_any_element()
    }

    /// "‹ i of N ›" through the tasks shown, when there is more than one.
    fn stepper(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.visible(cx).len() < 2 {
            return None;
        }
        let step = |id: &'static str, label: &'static str, glyph: &'static str, by: isize| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .cursor_pointer()
                .size(px(30.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .text_color(content(0.62))
                .hover(|s| s.bg(glass::hover_fill()).text_color(content(1.)))
                .child(icon(glyph, 15.))
                .on_click(cx.listener(move |this, _, _, cx| this.step(by, cx)))
        };
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(step("my-tasks-prev", "Previous task", "chevron-left", -1))
                .child(
                    div()
                        .debug_selector(|| "my-tasks-position".into())
                        .min_w(px(44.))
                        .flex()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(content(0.62))
                        .child(self.position(cx)),
                )
                .child(step("my-tasks-next", "Next task", "chevron-right", 1)),
        )
    }

    pub(super) fn task_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let task = self.selected_in(&self.visible(cx)).cloned();
        let banner = self.offer_banner(cx);
        let Some(task) = task else {
            return self.empty_pane(banner);
        };
        let open_label = match task.source {
            Source::Session(_) => "Open session",
            Source::FlowRun(_) => "Open run",
            Source::Pilot { .. } => "Open chat",
        };
        let source = task.source.clone();
        let waited = super::list::waited_long(self.now_ms.saturating_sub(task.since_ms));
        let bar =
            top_bar(&task.project, task.source_label.clone())
                .child(
                    div()
                        .ml(px(8.))
                        .debug_selector(|| "my-tasks-waited".into())
                        .child(glass::status(format!("Waiting {waited}"), Tone::Waiting)),
                )
                .child(div().flex_1())
                .children(self.stepper(cx))
                .child(open_button(open_label).ml(px(6.)).on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(Event::Open(source.clone()))),
                ));
        let history = self.history(&task, cx);
        let card = self.card(&task, cx);
        // The history fades out above the card, so the ask is the focus.
        let fade = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(40.))
            .bg(linear_gradient(
                180.,
                linear_color_stop(background_base().opacity(0.), 0.),
                linear_color_stop(dock_wash(), 1.),
            ));
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(bar)
            .children(banner)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(history)
                    .child(fade),
            )
            .child(
                div()
                    .id("my-tasks-card-dock")
                    .flex_shrink_0()
                    .max_h(relative(CARD_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .bg(dock_wash())
                    .px(px(24.))
                    .pt(px(8.))
                    .pb(px(24.))
                    .flex()
                    .justify_center()
                    .child(card),
            )
            .into_any_element()
    }
}
