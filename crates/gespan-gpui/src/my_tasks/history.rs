//! "What led here": the task's recent turns, read-only, oldest at the top so
//! the newest sits just above the card.
use super::*;
use gespan_core::tasks::Entry;
use gpui::{AnyElement, FontWeight, SharedString, px};

/// How long scrolling waits before asking for another page, so one flick
/// loads one page rather than the whole session.
const LOAD_PAUSE: std::time::Duration = std::time::Duration::from_millis(400);

/// "07:23", from Unix milliseconds, in local time.
fn clock(at: Option<u64>) -> String {
    use chrono::TimeZone;
    at.and_then(|ms| i64::try_from(ms).ok())
        .and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single())
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn entry_at(entry: &Entry) -> Option<u64> {
    match entry {
        Entry::You { at, .. } | Entry::Agent { at, .. } | Entry::Tools { at, .. } => *at,
    }
}

impl MyTasks {
    pub(super) fn history(&self, task: &TaskItem, cx: &mut Context<Self>) -> AnyElement {
        let history = &task.history;
        let last = history.turns.len().saturating_sub(1);
        let turns = history.turns.iter().enumerate().map(|(ix, turn)| {
            let first_at = turn.entries.first().and_then(entry_at);
            let label = if ix == last {
                format!("Latest \u{b7} {}", clock(first_at))
            } else {
                clock(first_at)
            };
            let selector = format!("my-tasks-turn-{ix}");
            div()
                .debug_selector(move || selector.clone())
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .child(label)
                        .child(div().flex_1().h(px(1.)).bg(content(0.07))),
                )
                .children(
                    turn.entries
                        .iter()
                        .enumerate()
                        .map(|(n, entry)| self.entry(ix, n, entry, &task.who).into_any_element()),
                )
        });
        let start = history
            .turns
            .first()
            .and_then(|t| t.entries.first())
            .and_then(entry_at);
        let key = task.key.clone();
        let wheel_key = task.key.clone();
        let more = history.earlier > 0;
        div()
            .id("my-tasks-history")
            .debug_selector(|| "my-tasks-history".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.history_scroll)
            // Scrolling up past the oldest turn shown loads the ones before it.
            .on_scroll_wheel(
                cx.listener(move |this, event: &gpui::ScrollWheelEvent, _, cx| {
                    let up = event.delta.pixel_delta(px(16.)).y > px(0.);
                    let rested = this.last_load.is_none_or(|at| at.elapsed() >= LOAD_PAUSE);
                    if more && up && rested && this.history_scroll.offset().y >= px(-1.) {
                        this.last_load = Some(std::time::Instant::now());
                        cx.emit(Event::LoadEarlier(wheel_key.clone()));
                    }
                }),
            )
            .child(
                div()
                    .px(px(24.))
                    .pt(px(20.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .when(history.earlier > 0, |d| {
                        let more = history.earlier.min(HISTORY_PAGE);
                        d.child(
                            div().flex().justify_center().child(
                                div()
                                    .id("my-tasks-load-earlier")
                                    .debug_selector(|| "my-tasks-load-earlier".into())
                                    .cursor_pointer()
                                    .h(px(28.))
                                    .px(px(12.))
                                    .rounded(px(6.))
                                    .bg(content(0.06))
                                    .flex()
                                    .items_center()
                                    .text_size(px(12.))
                                    .text_color(content(0.75))
                                    .hover(|s| s.bg(content(0.1)))
                                    .child(format!("Load {more} earlier"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(Event::LoadEarlier(key.clone()))
                                    })),
                            ),
                        )
                    })
                    .when(history.earlier == 0 && !history.turns.is_empty(), |d| {
                        d.child(
                            div()
                                .debug_selector(|| "my-tasks-history-start".into())
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child(div().flex_1().h(px(1.)).bg(content(0.07)))
                                .child(format!("Start of session \u{b7} {}", clock(start)))
                                .child(div().flex_1().h(px(1.)).bg(content(0.07))),
                        )
                    })
                    .children(turns),
            )
            .into_any_element()
    }

    fn entry(&self, turn: usize, n: usize, entry: &Entry, who: &str) -> impl IntoElement {
        match entry {
            Entry::Tools { summaries, at } => {
                let selector = format!("my-tasks-tools-{turn}-{}", summaries.len());
                div()
                    .debug_selector(move || selector.clone())
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child(icon("terminal", 12.))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(content(0.75))
                            .child(entry.tools_label().unwrap_or_default()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(summaries.join("; ")),
                    )
                    .child(div().text_color(content(0.4)).child(clock(*at)))
                    .into_any_element()
            }
            Entry::You { text, at } | Entry::Agent { text, at } => {
                let mine = matches!(entry, Entry::You { .. });
                let name = if mine {
                    "You".to_string()
                } else {
                    who.trim_end_matches(" asks")
                        .trim_end_matches(" wants approval")
                        .to_string()
                };
                div()
                    .id(SharedString::from(format!("my-tasks-entry-{turn}-{n}")))
                    .p(px(10.))
                    .rounded(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(mine, |d| d.ml(px(48.)).bg(content(0.06)))
                    .when(!mine, |d| {
                        d.mr(px(48.)).border_1().border_color(content(0.07))
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(gpui::rgb(if mine { 0x93c5fd } else { 0xc0affc }))
                                    .child(name),
                            )
                            .child(div().text_color(content(0.45)).child(clock(*at))),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(content(0.85))
                            .child(text.clone()),
                    )
                    .into_any_element()
            }
        }
    }
}
