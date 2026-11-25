//! Trace: a turn's steps as a list of lines, after shadcn's Trace component.
//!
//! Each line has an icon, a one-line title and a short description (a
//! duration, an exit code, diff stats or counts) and one of four states:
//! done, running, error or pending. A running line shimmers and counts up; an
//! error line is red. A line with detail or nested lines gets a chevron, and
//! what it holds opens inside the line, never as raw output in the chat.
//!
//! The component knows nothing about Pilots or tools: callers build [`Line`]s
//! and say which keys are open, so ordinary sessions can reuse it.
use crate::theme::{content, icon};
use gpui::{AnyElement, App, SharedString, Window, div, prelude::*, px, rgb};
use std::rc::Rc;

pub(crate) const ERROR: u32 = 0xf87171;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Done,
    Running,
    Error,
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// Stable across rebuilds: what is open is remembered by it.
    pub key: String,
    pub icon: &'static str,
    pub title: String,
    pub description: String,
    pub status: Status,
    /// Unix milliseconds the running step began; its description counts up.
    pub since: Option<u64>,
    /// Output or notes that open inside the line.
    pub detail: Option<String>,
    pub children: Vec<Line>,
}

impl Line {
    pub(crate) fn new(
        key: impl Into<String>,
        icon: &'static str,
        title: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            icon,
            title: title.into(),
            description: String::new(),
            status: Status::Done,
            since: None,
            detail: None,
            children: Vec::new(),
        }
    }
    pub(crate) fn expandable(&self) -> bool {
        self.detail.as_deref().is_some_and(|d| !d.trim().is_empty()) || !self.children.is_empty()
    }
    /// The description as shown at `now_ms`: a running line leads with its
    /// elapsed time, which counts up each second.
    pub(crate) fn shown_description(&self, now_ms: u64) -> String {
        let elapsed = (self.status == Status::Running)
            .then_some(self.since)
            .flatten()
            .map(|since| duration(now_ms.saturating_sub(since) / 1000));
        match (elapsed, self.description.is_empty()) {
            (Some(elapsed), true) => elapsed,
            (Some(elapsed), false) => format!("{elapsed} · {}", self.description),
            (None, _) => self.description.clone(),
        }
    }
}

/// "4s", "1m 48s", "31m", "2h 04m": short enough for a description.
pub(crate) fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 if secs.is_multiple_of(60) => format!("{}m", secs / 60),
        60..3600 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// What a line's chevron does with the line's key.
pub(crate) type Toggle = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// How a trace draws: what is open, the clock, and what a chevron does.
#[derive(Clone)]
pub(crate) struct Paint {
    pub open: Rc<dyn Fn(&str) -> bool>,
    pub toggle: Toggle,
    pub now_ms: u64,
    /// Seconds since the trace began animating, for the running shimmer.
    pub shimmer: f32,
    pub reduce_motion: bool,
}

/// The output inside an open line keeps to this many lines.
const DETAIL_LINES: usize = 40;

/// One line of a trace, with what it holds when open.
pub(crate) fn line(line: &Line, depth: usize, paint: &Paint) -> AnyElement {
    let open = line.expandable() && (paint.open)(&line.key);
    let key: SharedString = line.key.clone().into();
    let error = line.status == Status::Error;
    let title_color = match line.status {
        Status::Error => rgb(ERROR).into(),
        Status::Pending => content(0.35),
        _ => content(0.8),
    };
    let glyph = match line.status {
        Status::Running if !paint.reduce_motion => div()
            .size(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .child(crate::session_mark::spinner(
                gpui::ElementId::Name(format!("trace-spin-{key}").into()),
                crate::pulse::Rate::Half,
            ))
            .into_any_element(),
        Status::Error => icon("alert-circle", 14.)
            .text_color(rgb(ERROR))
            .into_any_element(),
        Status::Pending => icon(line.icon, 14.)
            .text_color(content(0.25))
            .into_any_element(),
        _ => icon(line.icon, 14.)
            .text_color(content(0.45))
            .into_any_element(),
    };
    let title = if line.status == Status::Running && !paint.reduce_motion {
        crate::activity::shimmer(line.title.clone(), paint.shimmer)
    } else {
        div().child(line.title.clone()).into_any_element()
    };
    let description = line.shown_description(paint.now_ms);
    let toggle = paint.toggle.clone();
    let toggle_key = line.key.clone();
    let row = div()
        .id(gpui::ElementId::Name(format!("trace-line-{key}").into()))
        .debug_selector({
            let key = key.clone();
            move || format!("trace-line-{key}")
        })
        .w_full()
        .min_w_0()
        .h(px(26.))
        .pl(px(depth as f32 * 20.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(13.))
        .line_height(px(18.))
        .child(glyph)
        .child(
            div()
                .min_w_0()
                .flex_shrink(1.)
                .truncate()
                .text_color(title_color)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(title),
        )
        .when(!description.is_empty(), |d| {
            d.child(
                div()
                    .min_w_0()
                    .flex_shrink(1.)
                    .truncate()
                    .text_size(px(12.))
                    .text_color(if error {
                        gpui::Hsla::from(rgb(ERROR)).opacity(0.7)
                    } else {
                        content(0.4)
                    })
                    .child(description),
            )
        })
        .when(line.expandable(), |d| {
            d.cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(format!(
                    "{}: {}",
                    line.title,
                    if open { "hide" } else { "show" }
                ))
                .aria_expanded(open)
                .hover(|s| s.opacity(0.8))
                .child(
                    icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        12.,
                    )
                    .text_color(content(0.35)),
                )
                .on_click(move |_, window, cx| toggle(toggle_key.clone(), window, cx))
        });
    let detail = line
        .detail
        .as_deref()
        .filter(|d| open && !d.trim().is_empty())
        .map(|detail| {
            let lines: Vec<&str> = detail.trim_end().lines().collect();
            let skipped = lines.len().saturating_sub(DETAIL_LINES);
            let mut text = lines[skipped..].join("\n");
            if skipped > 0 {
                text = format!("… {skipped} earlier lines\n{text}");
            }
            div()
                .debug_selector({
                    let key = key.clone();
                    move || format!("trace-detail-{key}")
                })
                .ml(px(depth as f32 * 20. + 6.))
                .mb_1()
                .pl(px(14.))
                .py_1()
                .border_l_1()
                .border_color(content(0.1))
                .font_family(crate::fonts::monospace())
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(content(0.55))
                .whitespace_normal()
                .child(text)
        });
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .child(row)
        .children(detail)
        .when(open, |d| {
            d.children(
                line.children
                    .iter()
                    .map(|child| self::line(child, depth + 1, paint)),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_short_at_every_scale() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(59), "59s");
        assert_eq!(duration(60), "1m");
        assert_eq!(duration(108), "1m 48s");
        assert_eq!(duration(31 * 60), "31m");
        assert_eq!(duration(2 * 3600 + 4 * 60 + 9), "2h 04m");
    }

    #[test]
    fn a_running_line_counts_up_and_a_settled_one_does_not() {
        let mut line = Line::new("a", "terminal", "Copy fixtures");
        line.status = Status::Running;
        line.since = Some(10_000);
        assert_eq!(line.shown_description(13_400), "3s");
        assert_eq!(line.shown_description(14_000), "4s", "it ticks each second");
        line.description = "staging".into();
        assert_eq!(line.shown_description(80_000), "1m 10s · staging");
        line.status = Status::Done;
        line.description = "1m 48s · exit 0".into();
        assert_eq!(
            line.shown_description(999_999),
            "1m 48s · exit 0",
            "a done line keeps its recorded duration"
        );
    }

    #[test]
    fn only_lines_with_something_inside_open() {
        let mut line = Line::new("a", "terminal", "Run");
        assert!(!line.expandable());
        line.detail = Some("  \n".into());
        assert!(!line.expandable(), "blank output is nothing to open");
        line.detail = Some("ok".into());
        assert!(line.expandable());
        let mut parent = Line::new("p", "terminal", "Chain");
        parent.children.push(Line::new("c", "terminal", "Step"));
        assert!(parent.expandable(), "nested lines open under their parent");
    }
}
