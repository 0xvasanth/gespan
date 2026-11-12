//! The queue column: the page header, the Waiting / Answered / Decisions
//! control, search and filter, and the tasks as glass rows grouped by how
//! long they have waited.
use super::*;
use crate::glass::{self, Tone};
use crate::workspace::project_name;
use gpui::{AnyElement, BoxShadow, Div, FontWeight, Hsla, Role, SharedString, Stateful, px};

/// The queue column's width, as in the approved design.
pub(super) const LIST_WIDTH: f32 = 360.;
/// A task waiting this long is in the "Over 1 hour" group.
const LONG_WAIT_MS: u64 = 60 * 60 * 1000;
const EIGHT_HOURS_MS: u64 = 8 * LONG_WAIT_MS;

/// A kind's mark in the list and on the card.
pub(super) fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Approval => "task-terminal",
        Kind::Question | Kind::PilotChoice => "task-question",
        Kind::FlowApproval => "task-flow",
        Kind::DraftReview => "git-pull-request-draft",
        Kind::Decision => "note",
    }
}

/// The word on a row's status pill, and its tone: approvals wait in yellow,
/// the rest in the quiet tone.
pub(super) fn kind_status(kind: Kind) -> (&'static str, Tone) {
    match kind {
        Kind::Approval => ("Approval", Tone::Waiting),
        Kind::Question => ("Question", Tone::Muted),
        Kind::FlowApproval => ("Flow", Tone::Muted),
        Kind::PilotChoice => ("Choice", Tone::Muted),
        Kind::DraftReview => ("Review", Tone::Muted),
        Kind::Decision => ("Decision", Tone::Muted),
    }
}

/// The colour of a kind's mark on its tile.
fn kind_ink(kind: Kind) -> Hsla {
    let (dark, light) = match kind {
        Kind::Approval => (0xe8c547, 0xa86b00),
        Kind::Question | Kind::PilotChoice => (0xc0affc, 0x6d4fd6),
        Kind::FlowApproval => (0x6ee7b7, 0x15874f),
        // A draft review waits quietly, like the muted status beside it.
        Kind::DraftReview => (0xa3a3a3, 0x666666),
        // A decision to review: the questions' violet, since it comes from
        // one.
        Kind::Decision => (0xc0affc, 0x6d4fd6),
    };
    gpui::rgb(if is_light() { light } else { dark }).into()
}

/// The faint ink of times and hints (the design's third text tone).
pub(super) fn faint() -> Hsla {
    content(if is_light() { 0.5 } else { 0.56 })
}

/// The agent a task comes from, as a small pill after its kind.
pub(super) fn agent_pill(name: String) -> Div {
    div()
        .h(px(22.))
        .px(px(10.))
        .rounded_full()
        .bg(glass::fill())
        .border_1()
        .border_color(glass::edge())
        .flex()
        .items_center()
        .text_size(px(11.))
        .text_color(content(0.64))
        .child(name)
}

/// The square a kind's mark sits on, at the start of a row or the card.
pub(super) fn tile(kind: Kind, side: f32) -> Div {
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(side * 0.28))
        .bg(if is_light() {
            glass::fill_strong()
        } else {
            white(0.07)
        })
        .border_1()
        .border_color(glass::edge())
        .flex()
        .items_center()
        .justify_center()
        .child(icon(kind_icon(kind), side * 0.5).text_color(kind_ink(kind)))
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

/// "2h 14m", "1d 3h", "12m": the wait to the minute, for the pane's status.
pub(super) fn waited_long(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => match minutes % 60 {
            0 => format!("{}h", minutes / 60),
            m => format!("{}h {m}m", minutes / 60),
        },
        _ => match (minutes % 1440) / 60 {
            0 => format!("{}d", minutes / 1440),
            h => format!("{}d {h}h", minutes / 1440),
        },
    }
}

/// The age group a wait falls in, as the filter's WAITING choices name
/// them: its selector suffix and its label.
pub(super) fn age_group(wait_ms: u64) -> (&'static str, &'static str) {
    if wait_ms >= EIGHT_HOURS_MS {
        ("8h", "Over 8 hours")
    } else if wait_ms >= LONG_WAIT_MS {
        ("1h", "Over 1 hour")
    } else {
        ("recent", "Recent")
    }
}

/// A small uppercase label over a run of rows.
pub(super) fn group_label(
    selector: String,
    label: impl Into<SharedString>,
    first: bool,
) -> AnyElement {
    glass::eyebrow(label)
        .debug_selector(move || selector.clone())
        .px(px(8.))
        .pt(px(if first { 4. } else { 12. }))
        .pb(px(4.))
        .into_any_element()
}

/// A row in any of the three lists: transparent until hovered; the selected
/// one is a raised glass plate.
pub(super) fn task_row(id: String, on: bool) -> Stateful<Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .role(Role::ListBoxOption)
        .aria_selected(on)
        .cursor_pointer()
        .flex_shrink_0()
        .p(px(12.))
        .rounded(px(12.))
        .flex()
        .gap(px(12.))
        .border_1()
        .border_color(gpui::transparent_black())
        .when(on, |d| {
            d.bg(if is_light() {
                glass::fill_strong()
            } else {
                white(0.07)
            })
            .border_color(if is_light() {
                glass::edge()
            } else {
                white(0.12)
            })
            .shadow(vec![
                BoxShadow::new(px(0.), px(1.), white(if is_light() { 0.9 } else { 0.08 })).inset(),
            ])
        })
        .when(!on, |d| d.hover(|s| s.bg(glass::hover_fill())))
}

/// A row's first line: the bold title, and a time at the far end.
pub(super) fn row_title(title: String, time: Option<(String, Hsla)>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .line_height(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .when_some(time, |d, (time, ink)| {
            d.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.5))
                    .text_color(ink)
                    .child(time),
            )
        })
}

/// A row's second line, muted.
pub(super) fn row_line(text: String) -> Div {
    div()
        .truncate()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(content(0.62))
        .child(text)
}

impl MyTasks {
    fn waiting_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let shown = self.visible(cx);
        let selected = self.selected_in(&shown).map(|t| t.key.clone());
        let now = self.now_ms;
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut group = None;
        // `visible` is longest wait first, so each age group is one run.
        for task in &shown {
            let wait = now.saturating_sub(task.since_ms);
            let (id, label) = age_group(wait);
            if group != Some(id) {
                rows.push(group_label(
                    format!("my-tasks-group-{id}"),
                    label,
                    group.is_none(),
                ));
                group = Some(id);
            }
            let key = task.key.clone();
            let on = selected.as_deref() == Some(task.key.as_str());
            let (word, tone) = kind_status(task.kind);
            rows.push(
                task_row(format!("my-tasks-row-{}", task.key), on)
                    .aria_label(SharedString::from(task.headline.clone()))
                    .child(tile(task.kind, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(row_title(
                                task.headline.clone(),
                                Some((waited(wait), faint())),
                            ))
                            .child(row_line(format!(
                                "{} \u{b7} {}",
                                task.source_label,
                                project_name(&task.project)
                            )))
                            .child(
                                div()
                                    .mt(px(4.))
                                    .flex()
                                    .gap(px(6.))
                                    .child(glass::status(word, tone))
                                    .children(task.agent.clone().map(|name| {
                                        let selector = format!("my-tasks-agent-{}", task.key);
                                        agent_pill(name).debug_selector(move || selector.clone())
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(&key, cx)))
                    .into_any_element(),
            );
        }
        rows
    }

    /// Waiting, Answered and Decisions as one segmented control; what is
    /// waiting counts in the hot style.
    fn tab_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let segment = |tab: Tab, id: &'static str, label: &'static str, count: Option<Div>| {
            let on = self.tab == tab;
            glass::segment(id, label, on)
                .debug_selector(move || id.into())
                .role(Role::Tab)
                .aria_selected(on)
                .children(count)
                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
        };
        let waiting = self.tasks.len();
        glass::segmented()
            .id("my-tasks-tabs")
            .role(Role::TabList)
            .aria_label("Task view")
            .child(segment(
                Tab::Waiting,
                "my-tasks-tab-waiting",
                "Waiting",
                Some(
                    glass::count(waiting, waiting > 0)
                        .debug_selector(move || format!("my-tasks-count-waiting-{waiting}")),
                ),
            ))
            .child(segment(
                Tab::Answered,
                "my-tasks-tab-answered",
                "Answered",
                Some({
                    let answered = self.past.len();
                    glass::count(answered, false)
                        .debug_selector(move || format!("my-tasks-count-answered-{answered}"))
                }),
            ))
            .child(segment(
                Tab::Decisions,
                "my-tasks-tab-decisions",
                "Decisions",
                Some(glass::count(self.decisions.len(), false)),
            ))
            .into_any_element()
    }

    /// The list's empty text for the tab shown: a first-run line when the
    /// tab has nothing at all, another when search or filters hide it all.
    pub(super) fn list_empty_text(&self) -> &'static str {
        match self.tab {
            Tab::Waiting if self.tasks.is_empty() => "Nothing is waiting on you.",
            Tab::Waiting => "No tasks match the search or filters.",
            Tab::Answered if self.past.is_empty() => "Nothing answered yet.",
            Tab::Answered => "No answers match the search or filters.",
            Tab::Decisions if self.decisions.is_empty() => "No decisions yet.",
            Tab::Decisions => "No decisions match the search or filters.",
        }
    }

    pub(super) fn list_column(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.filter.is_active();
        let tools = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                glass::field()
                    .debug_selector(|| "my-tasks-search".into())
                    .flex_1()
                    .min_w_0()
                    .child(icon("search", 14.).text_color(content(0.45)))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .child(
                icon_button("my-tasks-filter", "Filter tasks", "filter")
                    .debug_selector(|| "my-tasks-filter".into())
                    .relative()
                    .size(px(32.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(glass::edge())
                    .bg(glass::fill())
                    .child(self.filter_anchor.probe())
                    .when(self.filters_open || active, |d| {
                        d.bg(glass::selected_fill())
                    })
                    .when(active, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "my-tasks-filter-on".into())
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(Tone::Waiting.ink()),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filters_open = !this.filters_open;
                        cx.notify();
                    })),
            );
        let rows = match self.tab {
            Tab::Waiting => self.waiting_rows(cx),
            Tab::Answered => self.past_rows(cx),
            Tab::Decisions => self.decision_rows(cx),
        };
        let empty = rows.is_empty().then(|| self.list_empty_text());
        div()
            .w(px(LIST_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(glass::hairline())
            .bg(if is_light() {
                content(0.025)
            } else {
                black(0.12)
            })
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .pt(px(22.))
                    .px(px(18.))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(glass::page_header(
                        "My tasks",
                        "Everything your agents are waiting on you for.",
                    ))
                    .child(div().flex().child(self.tab_control(cx)))
                    .child(tools),
            )
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
                    .px(px(10.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .when_some(empty, |d, text| {
                        d.child(
                            div()
                                .debug_selector(|| "my-tasks-list-empty".into())
                                .px(px(8.))
                                .py(px(6.))
                                .text_size(px(12.5))
                                .text_color(content(0.55))
                                .child(text),
                        )
                    })
                    .children(rows),
            )
            .into_any_element()
    }
}
