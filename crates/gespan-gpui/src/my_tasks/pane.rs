//! The right pane: a top bar with where the task comes from, how long it has
//! waited and "‹ i of N ›"; its history; and the answer card pinned under
//! it. The pane is translucent glass and GPUI has no mask, so the history
//! is not faded: any solid fade would show as a band over the glass.
use super::*;
use crate::glass::{self, Button, Tone};
use gpui::{AnyElement, Div, px, relative};

/// Every icon the page draws outside the kind marks, so a test can check
/// each one exists.
#[cfg(test)]
pub(super) const ICONS: &[&str] = &[
    "arrow-up-right",
    "chevron-left",
    "chevron-right",
    "terminal",
    "search",
    "filter",
    "check",
    "check-bold",
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

/// The widest the pane's content runs: the answer card's 720pt plus its
/// 24pt gutters, as in the design. On a wider pane the top bar, history and
/// card stay in this centred column instead of stretching across it; with
/// the 360pt queue the page tops out at 1128pt, inside Home's 1200pt cap.
pub(super) const PANE_COLUMN: f32 = 768.;
/// Below this pane width the top bar folds: "Open thread" to its icon, the
/// wait to its time, the stepper to its arrows and the project crumb away,
/// so the source keeps room to read beside them.
pub(super) const COMPACT_PANE: f32 = 640.;

/// Whether a page `width` wide shows the compact top bar.
pub(super) fn compact_bar(page_width: f32) -> bool {
    page_width - super::list::queue_width(page_width) < COMPACT_PANE
}

/// The pane's top bar: the project, then where the request comes from,
/// with whatever else the tab shows after it. Put it in [`bar_frame`]. A
/// compact bar leaves the project out: the queue row beside it names it.
pub(super) fn top_bar(project: &str, source: String, compact: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(px(12.))
        .when(!compact, |d| {
            d.child(
                div()
                    .flex_shrink_0()
                    .max_w(px(160.))
                    .truncate()
                    .text_color(content(0.62))
                    .child(super::project_display(project)),
            )
            .child(div().flex_shrink_0().text_color(content(0.4)).child("/"))
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(content(0.92))
                .child(source),
        )
}

/// The 56pt strip a [`top_bar`] sits in: its rule spans the pane, its
/// content keeps to the centred [`PANE_COLUMN`].
pub(super) fn bar_frame(bar: Div) -> Div {
    div()
        .h(px(56.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(glass::hairline())
        .flex()
        .justify_center()
        .child(
            bar.debug_selector(|| "my-tasks-bar".into())
                .w_full()
                .max_w(px(PANE_COLUMN))
                .h_full()
                .px(px(24.)),
        )
}

/// The ghost button that opens the request where it was asked; on a
/// compact bar, just its arrow.
pub(super) fn open_button(label: &'static str, compact: bool) -> gpui::Stateful<Div> {
    if compact {
        return div()
            .id("my-tasks-open-source")
            .debug_selector(|| "my-tasks-open-source".into())
            .role(gpui::Role::Button)
            .aria_label(label)
            .cursor_pointer()
            .flex_shrink_0()
            .size(px(30.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(content(0.62))
            .hover(|s| s.bg(glass::hover_fill()).text_color(content(1.)))
            .child(icon("arrow-up-right", 13.));
    }
    glass::button("my-tasks-open-source", label, Button::Ghost)
        .debug_selector(|| "my-tasks-open-source".into())
        .h(px(30.))
        .child(icon("arrow-up-right", 13.))
}

impl MyTasks {
    /// Empties the search field and turns every filter off.
    pub(super) fn clear_search_and_filters(&mut self, cx: &mut Context<Self>) {
        self.filter = gespan_core::tasks::Filter::default();
        self.search
            .update(cx, |search, cx| search.set_text(String::new(), cx));
        self.sync_selection(cx);
    }

    /// Whether the pane is too narrow for the full top bar, by the page's
    /// width when last laid out.
    pub(super) fn compact(&self) -> bool {
        self.page_bounds
            .get()
            .is_some_and(|b| compact_bar(f32::from(b.size.width)))
    }

    /// A tab's empty pane, with any banner over it.
    pub(super) fn empty_pane(
        &self,
        banner: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_any = match self.tab {
            Tab::Waiting => !self.tasks.is_empty(),
            Tab::Answered => !self.past.is_empty(),
            Tab::Decisions => !self.decisions.is_empty(),
        };
        let (glyph, title, hint) = empty_copy(self.tab, has_any);
        // Something is there but the search or filters hide it: offer the
        // way back in one click.
        let clear = has_any.then(|| {
            glass::button(
                "my-tasks-clear-filters",
                "Clear search and filters",
                Button::Glass,
            )
            .debug_selector(|| "my-tasks-clear-filters".into())
            .mt(px(4.))
            .on_click(cx.listener(|this, _, _, cx| this.clear_search_and_filters(cx)))
        });
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .children(banner)
            .child(
                div().flex_1().flex().items_center().justify_center().child(
                    glass::empty_state(glyph, title, hint)
                        .debug_selector(|| "my-tasks-empty".into())
                        .children(clear),
                ),
            )
            .into_any_element()
    }

    /// "‹ i of N ›" through the tasks shown, when there is more than one;
    /// a compact bar keeps just the arrows.
    fn stepper(&self, compact: bool, cx: &mut Context<Self>) -> Option<Div> {
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
                .when(!compact, |d| {
                    d.child(
                        div()
                            .debug_selector(|| "my-tasks-position".into())
                            .min_w(px(44.))
                            .flex()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(content(0.62))
                            .child(self.position(cx)),
                    )
                })
                .child(step("my-tasks-next", "Next task", "chevron-right", 1)),
        )
    }

    pub(super) fn task_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let task = self.selected_in(&self.visible(cx)).cloned();
        let banner = self.offer_banner(cx);
        let Some(task) = task else {
            return self.empty_pane(banner, cx);
        };
        let open_label = match task.source {
            Source::Session(_) => Some("Open thread"),
            Source::FlowRun(_) => Some("Open run"),
            Source::Pilot { .. } => Some("Open chat"),
            Source::DraftReview { .. } => Some("Open draft"),
            // An imported entry: no session to open.
            Source::Log => None,
        };
        let source = task.source.clone();
        let compact = self.compact();
        let waited = super::list::waited_long(self.now_ms.saturating_sub(task.since_ms));
        // The pill's colour already says it waits; a compact bar drops the word.
        let waited = if compact {
            waited
        } else {
            format!("Waiting {waited}")
        };
        let bar =
            top_bar(&task.project, task.source_label.clone(), compact)
                .child(
                    div()
                        .ml(px(8.))
                        .debug_selector(|| "my-tasks-waited".into())
                        .child(glass::status(waited, Tone::Waiting)),
                )
                .child(div().flex_1())
                .children(self.stepper(compact, cx))
                .when_some(open_label, |bar, label| {
                    bar.child(open_button(label, compact).ml(px(6.)).on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(Event::Open(source.clone()))),
                    ))
                });
        let history = self.history(&task, cx);
        let card = self.card(&task, cx);
        div()
            .debug_selector(|| "my-tasks-pane".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(bar_frame(bar))
            .children(banner)
            .child(div().flex_1().min_h_0().flex().flex_col().child(history))
            .child(
                div()
                    .id("my-tasks-card-dock")
                    .flex_shrink_0()
                    .max_h(relative(CARD_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .px(px(24.))
                    .pb(px(24.))
                    .flex()
                    .justify_center()
                    .child(card),
            )
            .into_any_element()
    }
}
