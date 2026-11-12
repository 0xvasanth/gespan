//! The list column: the Waiting tab, search and filter, and the tasks grouped
//! by project, in Inbox's shape.
use super::*;
use crate::workspace::project_name;
use gpui::{AnyElement, FontWeight, Role, SharedString, px};

/// The list column's width, as in the approved design.
pub(super) const LIST_WIDTH: f32 = 330.;
/// A task waiting this long shows its wait in amber.
const LONG_WAIT_MS: u64 = 60 * 60 * 1000;

/// A kind's mark in the list and on the card.
pub(super) fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Question => "chat",
        Kind::Approval => "terminal",
        Kind::FlowApproval => "git-merge",
        Kind::PilotChoice => "queue-list",
        Kind::DraftReview => "git-pull-request-draft",
    }
}

/// "3m", "2h", "1d": how long a task has waited.
pub(super) fn waited(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "now".into(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

impl MyTasks {
    pub(super) fn list_column(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.visible(cx);
        let selected = self.selected_in(&shown).map(|t| t.key.clone());
        let mut groups: Vec<(String, Vec<&TaskItem>)> = Vec::new();
        for task in &shown {
            match groups
                .iter_mut()
                .find(|(project, _)| *project == task.project)
            {
                Some((_, tasks)) => tasks.push(task),
                None => groups.push((task.project.clone(), vec![task])),
            }
        }
        let now = self.now_ms;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (project, tasks) in &groups {
            let header = format!("{} \u{b7} {}", project_name(project), tasks.len());
            let selector = format!("my-tasks-group-{project}");
            rows.push(
                div()
                    .debug_selector(move || selector.clone())
                    .px(px(8.))
                    .pt(px(10.))
                    .pb(px(4.))
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(0.4))
                    .child(header.to_uppercase())
                    .into_any_element(),
            );
            for task in tasks {
                let key = task.key.clone();
                let on = selected.as_deref() == Some(task.key.as_str());
                let wait = now.saturating_sub(task.since_ms);
                let selector = format!("my-tasks-row-{}", task.key);
                rows.push(
                    div()
                        .id(SharedString::from(format!("my-tasks-row-{}", task.key)))
                        .debug_selector(move || selector.clone())
                        .role(Role::ListBoxOption)
                        .aria_label(SharedString::from(task.headline.clone()))
                        .aria_selected(on)
                        .cursor_pointer()
                        .p(px(8.))
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .when(on, |d| d.bg(content(0.1)))
                        .hover(|s| s.bg(content(0.06)))
                        .child(
                            div()
                                .size(px(26.))
                                .flex_shrink_0()
                                .rounded(px(6.))
                                .bg(content(0.06))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(kind_icon(task.kind), 13.).text_color(content(0.7))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .truncate()
                                        .child(task.headline.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(content(0.55))
                                        .truncate()
                                        .child(task.source_label.clone()),
                                ),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(if wait >= LONG_WAIT_MS {
                                    gpui::rgb(0xffb900).into()
                                } else {
                                    content(0.45)
                                })
                                .child(waited(wait)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.select(&key, cx)))
                        .into_any_element(),
                );
            }
        }
        let active = self.filter.is_active();
        let tabs = self.tab_pills(cx);
        let tools = div()
            .h(px(40.))
            .flex_shrink_0()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .border_b_1()
            .border_color(content(0.07))
            .child(
                div()
                    .debug_selector(|| "my-tasks-search".into())
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .bg(content(0.05))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("search", 12.))
                    .child(self.search.clone()),
            )
            .child(
                icon_button("my-tasks-filter", "Filter tasks", "filter")
                    .debug_selector(|| "my-tasks-filter".into())
                    .relative()
                    .child(self.filter_anchor.probe())
                    .when(self.filters_open || active, |d| d.bg(content(0.1)))
                    .when(active, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "my-tasks-filter-on".into())
                                .absolute()
                                .top(px(3.))
                                .right(px(3.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(gpui::rgb(0xffb900)),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filters_open = !this.filters_open;
                        cx.notify();
                    })),
            );
        let (rows, empty_text) = match self.tab {
            Tab::Waiting => (
                rows,
                if self.tasks.is_empty() {
                    "Nothing is waiting on you."
                } else {
                    "No tasks match the search or filters."
                },
            ),
            Tab::Answered => (
                self.past_rows(cx),
                if self.past.is_empty() {
                    "Nothing answered yet."
                } else {
                    "No answers match the search or filters."
                },
            ),
            Tab::Decisions => (
                self.decision_rows(cx),
                if self.decisions.is_empty() {
                    "No decisions yet."
                } else {
                    "No decisions match the search or filters."
                },
            ),
        };
        let empty = rows.is_empty();
        div()
            .w(px(LIST_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(content(0.07))
            .flex()
            .flex_col()
            .child(tabs)
            .child(tools)
            .child(
                div()
                    .id("my-tasks-list")
                    .role(Role::ListBox)
                    .aria_label(match self.tab {
                        Tab::Waiting => "Waiting on you",
                        Tab::Answered => "Answered",
                        Tab::Decisions => "Decisions",
                    })
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(12.))
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child(empty_text),
                        )
                    })
                    .children(rows),
            )
            .into_any_element()
    }
}
