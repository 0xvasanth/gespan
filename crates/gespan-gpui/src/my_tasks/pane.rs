//! The right pane: where the task comes from, its history, and the card
//! pinned at the bottom.
use super::*;
use crate::workspace::project_name;
use gpui::{AnyElement, px, relative};

impl MyTasks {
    pub(super) fn task_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let task = self.selected_in(&self.visible(cx)).cloned();
        let Some(task) = task else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.))
                .text_color(content(0.5))
                .child(if self.tasks.is_empty() {
                    "You're all caught up."
                } else {
                    "No task is selected."
                })
                .into_any_element();
        };
        let open_label = match task.source {
            Source::Session(_) => "Open session",
            Source::FlowRun(_) => "Open run",
            Source::Pilot { .. } => "Open chat",
        };
        let source = task.source.clone();
        let bar = div()
            .h(px(40.))
            .flex_shrink_0()
            .pl(px(20.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(content(0.07))
            .text_size(px(12.))
            .text_color(content(0.55))
            .child(div().flex_1().min_w_0().truncate().child(format!(
                "{}  \u{203a}  {}",
                project_name(&task.project),
                task.source_label
            )))
            .child(
                div()
                    .id("my-tasks-open-source")
                    .debug_selector(|| "my-tasks-open-source".into())
                    .cursor_pointer()
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .bg(content(0.06))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(content(0.9))
                    .hover(|s| s.bg(content(0.1)))
                    .child(open_label)
                    .child(icon("arrow-up-right", 11.))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::Open(source.clone())))),
            );
        let history = self.history(&task, cx);
        let card = self.card(&task, cx);
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(history)
            .child(
                div()
                    .id("my-tasks-card-dock")
                    .flex_shrink_0()
                    .max_h(relative(CARD_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(content(0.07))
                    .px(px(16.))
                    .pt(px(14.))
                    .pb(px(16.))
                    .child(card),
            )
            .into_any_element()
    }
}
