//! "What led here": the task's recent turns, read-only, oldest at the top so
//! the newest sits just above the card. Your messages are tinted bubbles on
//! the right, the agent's are plain text, and each tool call is a pill.
use super::*;
use crate::glass;
use gespan_core::tasks::Entry;
use gpui::{AnyElement, Hsla, SharedString, StyledText, TextRun, px};
use std::ops::Range;

/// How long scrolling waits before asking for another page, so one flick
/// loads one page rather than the whole session.
const LOAD_PAUSE: std::time::Duration = std::time::Duration::from_millis(400);
/// Tool pills a run of calls shows before the rest fold into "+N more".
pub(super) const TOOL_PILLS: usize = 3;
/// The history column's width, as in the approved design.
const COLUMN: f32 = 680.;

/// `text` without its backticks, with the byte ranges that were inside a
/// pair: the inline code. An unpaired backtick stays as written.
pub(super) fn code_spans(text: &str) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(text.len());
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        out.push_str(&rest[..open]);
        let start = out.len();
        out.push_str(&after[..close]);
        if close > 0 {
            spans.push(start..out.len());
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    (out, spans)
}

/// The user's bubble tint and its edge: the accent blue at `alpha`.
fn bubble(alpha: f32) -> Hsla {
    let mut color: Hsla = gpui::rgb(0x459bf7).into();
    color.a = alpha;
    color
}

/// An agent message: plain text, with any `inline code` in monospace on a
/// faint plate.
fn agent_text(text: &str) -> StyledText {
    let (plain, spans) = code_spans(text);
    let ink = content(if is_light() { 0.78 } else { 0.81 });
    let run = |len: usize, code: bool| TextRun {
        len,
        font: gpui::font(if code {
            crate::fonts::monospace()
        } else {
            crate::fonts::UI
        }),
        color: ink,
        background_color: code.then(|| content(0.07)),
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::new();
    let mut at = 0;
    for span in &spans {
        if span.start > at {
            runs.push(run(span.start - at, false));
        }
        runs.push(run(span.len(), true));
        at = span.end;
    }
    if plain.len() > at || runs.is_empty() {
        runs.push(run(plain.len() - at, false));
    }
    StyledText::new(plain).with_runs(runs)
}

/// One tool call as a pill with a green check.
fn tool_pill(selector: String, label: String, check: bool) -> AnyElement {
    div()
        .debug_selector(move || selector.clone())
        .flex_shrink_0()
        .h(px(26.))
        .px(px(11.))
        .rounded_full()
        .bg(glass::fill())
        .border_1()
        .border_color(glass::edge())
        .shadow(vec![super::list::pill_highlight()])
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.64))
        .when(check, |d| {
            d.child(icon("check-bold", 12.).text_color(diff_foreground(true)))
        })
        .child(label)
        .into_any_element()
}

impl MyTasks {
    pub(super) fn history(&self, task: &TaskItem, cx: &mut Context<Self>) -> AnyElement {
        let history = &task.history;
        let turns = history.turns.iter().enumerate().map(|(ix, turn)| {
            let selector = format!("my-tasks-turn-{ix}");
            div()
                .debug_selector(move || selector.clone())
                .flex()
                .flex_col()
                .gap(px(14.))
                .children(
                    turn.entries
                        .iter()
                        .enumerate()
                        .map(|(n, entry)| self.entry(ix, n, entry)),
                )
        });
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
                    .w_full()
                    .max_w(px(COLUMN + 48.))
                    .mx_auto()
                    .px(px(24.))
                    .pt(px(28.))
                    .pb(px(40.))
                    .opacity(0.9)
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .when(history.earlier > 0, |d| {
                        let more = history.earlier.min(HISTORY_PAGE);
                        d.child(
                            div().flex().justify_center().child(
                                div()
                                    .id("my-tasks-load-earlier")
                                    .debug_selector(|| "my-tasks-load-earlier".into())
                                    .cursor_pointer()
                                    .h(px(26.))
                                    .px(px(11.))
                                    .rounded_full()
                                    .bg(glass::fill())
                                    .border_1()
                                    .border_color(glass::edge())
                                    .flex()
                                    .items_center()
                                    .text_size(px(12.))
                                    .text_color(content(0.64))
                                    .hover(|s| s.bg(glass::hover_fill()))
                                    .child(format!("Load {more} earlier"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(Event::LoadEarlier(key.clone()))
                                    })),
                            ),
                        )
                    })
                    .children(turns),
            )
            .into_any_element()
    }

    fn entry(&self, turn: usize, n: usize, entry: &Entry) -> AnyElement {
        match entry {
            Entry::Tools { summaries, .. } => {
                let selector = format!("my-tasks-tools-{turn}-{}", summaries.len());
                let hidden = summaries.len().saturating_sub(TOOL_PILLS);
                div()
                    .debug_selector(move || selector.clone())
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(
                        summaries
                            .iter()
                            .take(TOOL_PILLS)
                            .enumerate()
                            .map(|(i, summary)| {
                                tool_pill(
                                    format!("my-tasks-tool-{turn}-{i}"),
                                    summary.clone(),
                                    true,
                                )
                            }),
                    )
                    .when(hidden > 0, |d| {
                        d.child(tool_pill(
                            format!("my-tasks-tools-more-{turn}"),
                            format!("+{hidden} more"),
                            false,
                        ))
                    })
                    .into_any_element()
            }
            Entry::You { text, .. } => div()
                .id(SharedString::from(format!("my-tasks-entry-{turn}-{n}")))
                .self_end()
                .max_w(px(520.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(14.))
                .bg(bubble(if is_light() { 0.12 } else { 0.18 }))
                .border_1()
                .border_color(bubble(0.22))
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(content(0.95))
                .child(text.clone())
                .into_any_element(),
            Entry::Agent { text, .. } => div()
                .id(SharedString::from(format!("my-tasks-entry-{turn}-{n}")))
                .py(px(10.))
                .text_size(px(13.))
                .line_height(px(20.))
                .child(agent_text(text))
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::code_spans;

    #[test]
    fn backticks_mark_inline_code_and_are_dropped() {
        let (text, spans) = code_spans("I traced it to `pinnedVersion` being dropped");
        assert_eq!(text, "I traced it to pinnedVersion being dropped");
        assert_eq!(&text[spans[0].clone()], "pinnedVersion");
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn several_code_spans_keep_their_own_ranges() {
        let (text, spans) = code_spans("`a` then `bc`.");
        assert_eq!(text, "a then bc.");
        let words: Vec<&str> = spans.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(words, ["a", "bc"]);
    }

    #[test]
    fn an_unpaired_backtick_is_left_as_written() {
        assert_eq!(code_spans("2 ` 3"), ("2 ` 3".to_string(), vec![]));
        let (text, spans) = code_spans("`x` then `oops");
        assert_eq!(text, "x then `oops");
        assert_eq!(spans, vec![0..1]);
    }

    #[test]
    fn empty_backticks_make_no_span_and_plain_text_has_none() {
        assert_eq!(code_spans("a `` b"), ("a  b".to_string(), vec![]));
        assert_eq!(code_spans("plain"), ("plain".to_string(), vec![]));
    }
}
