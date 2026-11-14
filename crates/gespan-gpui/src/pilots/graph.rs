//! The Pipeline tab's graph: a Flow drawn top to bottom as the board draws
//! it. One card per step, a parallel step's branches side by side between a
//! fork and a join, a conditional branch dashed. With a run shown each card
//! wears its step's status in that run.
//!
//! Keyboard: Tab or Down walks the cards in reading order, Up or Shift-Tab
//! walks back, Left and Right move within a row, Enter opens the details of
//! the selected card and Escape clears the selection.
use super::*;
use gespan_core::flow::graph::{Card, CardStatus, GraphRow, branch_condition};
use gespan_core::flow::{Flow, FlowState};
use gpui::{FocusHandle, FontWeight, Hsla, relative};
use std::time::{Duration, Instant};

/// How long the "changed" mark stays on a step.
const CHANGED_FOR: Duration = Duration::from_secs(10);
/// The width of a step on its own row.
const STEP_W: f32 = 220.;
/// The gap between branches.
const BRANCH_GAP: f32 = 8.;
/// The stem between two rows, and the height of a fork or join.
const STEM: f32 = 10.;
const FORK: f32 = 16.;

#[derive(Clone, Debug)]
pub(crate) enum GraphEvent {
    /// The selected step changed (`None`: cleared).
    Selected(Option<String>),
    /// Enter or a double-click on a step.
    Open(String),
    /// "Answer" on a step waiting for the user: its session.
    Answer(i64),
}

pub(crate) struct PipelineGraph {
    flow: Option<Flow>,
    rows: Vec<GraphRow>,
    cards: HashMap<String, Card>,
    /// The step the run is on now.
    current: Option<String>,
    /// Running steps whose session waits for the user.
    waiting: HashSet<String>,
    /// When each step was marked changed, on the executor's clock.
    changed: HashMap<String, Instant>,
    selected: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<GraphEvent> for PipelineGraph {}

impl Focusable for PipelineGraph {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The board's ink for faint and muted text, and its title ink.
fn ink(alpha_light: f32, dark: u32) -> Hsla {
    if crate::theme::is_light() {
        content(alpha_light)
    } else {
        rgb(dark).into()
    }
}
fn title_ink() -> Hsla {
    ink(0.92, 0xebebeb)
}
pub(super) fn faint_ink() -> Hsla {
    ink(0.55, 0x8a8a8a)
}
fn line_ink() -> Hsla {
    content(0.22)
}

const OK: u32 = 0x6ee7b7;
const WAIT: u32 = 0xe8c547;
const BAD: u32 = 0xfda4af;
const RUN: u32 = 0x459bf7;

fn tint(hex: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(hex).into();
    color.a = alpha;
    color
}

impl PipelineGraph {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            flow: None,
            rows: Vec::new(),
            cards: HashMap::new(),
            current: None,
            waiting: HashSet::new(),
            changed: HashMap::new(),
            selected: None,
            // A tracked handle is a Tab stop only when the handle says so.
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// Shows `flow` laid out as `rows`, each step's status in `cards`.
    /// `changed` names steps newly changed by this update; they are marked
    /// for 10 s. The scroll position stays.
    pub fn set(
        &mut self,
        flow: Flow,
        rows: Vec<GraphRow>,
        cards: HashMap<String, Card>,
        current: Option<String>,
        changed: HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        if !changed.is_empty() {
            let now = cx.background_executor().now();
            for id in changed {
                self.changed.insert(id, now);
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CHANGED_FOR).await;
                let _ = this.update(cx, |this, cx| this.prune_changed(cx));
            })
            .detach();
        }
        self.flow = Some(flow);
        self.rows = rows;
        self.cards = cards;
        self.current = current;
        if self
            .selected
            .as_deref()
            .is_some_and(|id| !self.order().iter().any(|s| s == id))
        {
            self.select(None, cx);
        }
        cx.notify();
    }

    /// Running steps whose session now waits for the user.
    pub fn set_waiting(&mut self, waiting: HashSet<String>, cx: &mut Context<Self>) {
        if self.waiting != waiting {
            self.waiting = waiting;
            cx.notify();
        }
    }

    /// Whether a running step waits for the user.
    pub fn any_waiting(&self) -> bool {
        self.waiting.iter().any(|id| self.waits(id))
    }

    /// The steps drawn, in reading order.
    pub fn order(&self) -> Vec<String> {
        self.rows
            .iter()
            .flat_map(|row| row.cards().into_iter().map(str::to_owned))
            .collect()
    }

    #[cfg(test)]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The run's current step.
    #[cfg(test)]
    pub fn current(&self) -> Option<String> {
        self.current.clone()
    }

    /// Steps wearing the "changed" mark now, sorted.
    #[cfg(test)]
    pub fn badges(&self, cx: &App) -> Vec<String> {
        self.marked(cx)
    }

    /// Drops every mark, so a test can see whether an update adds any.
    #[cfg(test)]
    pub fn clear_badges_for_test(&mut self, cx: &mut Context<Self>) {
        self.changed.clear();
        cx.notify();
    }

    fn prune_changed(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let before = self.changed.len();
        self.changed
            .retain(|_, at| now.saturating_duration_since(*at) < CHANGED_FOR);
        if self.changed.len() != before {
            cx.notify();
        }
    }

    fn marked(&self, cx: &App) -> Vec<String> {
        let now = cx.background_executor().now();
        let mut ids: Vec<String> = self
            .changed
            .iter()
            .filter(|(_, at)| now.saturating_duration_since(**at) < CHANGED_FOR)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if id != self.selected {
            self.selected = id.clone();
            cx.emit(GraphEvent::Selected(id));
            cx.notify();
        }
    }

    /// Moves the selection; false when there is nowhere to go.
    fn step(&mut self, by: Move, cx: &mut Context<Self>) -> bool {
        let order = self.order();
        let at = self
            .selected
            .as_deref()
            .and_then(|id| order.iter().position(|s| s == id));
        let row_of = |id: &str| self.rows.iter().position(|r| r.cards().contains(&id));
        let next = match (by, at) {
            (Move::Next, None) => order.first().cloned(),
            (Move::Next, Some(i)) => order.get(i + 1).cloned(),
            (Move::Previous, Some(i)) if i > 0 => order.get(i - 1).cloned(),
            (Move::Left | Move::Right, Some(i)) => {
                let id = &order[i];
                let row = row_of(id).map(|r| self.rows[r].cards());
                row.and_then(|cards| {
                    let j = cards.iter().position(|c| c == id)?;
                    let k = if by == Move::Left {
                        j.checked_sub(1)?
                    } else {
                        j + 1
                    };
                    cards.get(k).map(|c| c.to_string())
                })
            }
            _ => None,
        };
        match next {
            Some(id) => {
                self.select(Some(id), cx);
                true
            }
            None => false,
        }
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keys = &event.keystroke;
        if keys.modifiers.platform || keys.modifiers.control || keys.modifiers.alt {
            return;
        }
        let handled = match keys.key.as_str() {
            "tab" if keys.modifiers.shift => self.step(Move::Previous, cx),
            "tab" | "down" => self.step(Move::Next, cx),
            "up" => self.step(Move::Previous, cx),
            "left" => self.step(Move::Left, cx),
            "right" => self.step(Move::Right, cx),
            "enter" => match self.selected.clone() {
                Some(id) => {
                    cx.emit(GraphEvent::Open(id));
                    true
                }
                None => false,
            },
            "escape" if self.selected.is_some() => {
                self.select(None, cx);
                true
            }
            _ => false,
        };
        // Arrow keys at a dead end are still ours; Tab at one moves on.
        if handled || matches!(keys.key.as_str(), "up" | "down" | "left" | "right") {
            cx.stop_propagation();
        }
    }

    /// The accessible name: the selected step as "<title>, <status>".
    fn aria_label(&self) -> String {
        let Some(flow) = &self.flow else {
            return "Pipeline".into();
        };
        match self.selected.as_deref().and_then(|id| flow.state(id)) {
            Some(state) => format!("{}, {}", state.title(), self.subtitle(flow, state.id())),
            None => {
                let steps = self.order().len();
                let noun = if steps == 1 { "step" } else { "steps" };
                format!("Pipeline, {steps} {noun}")
            }
        }
    }

    fn status(&self, id: &str) -> CardStatus {
        self.cards.get(id).map_or(CardStatus::Idle, |c| c.status)
    }

    fn waits(&self, id: &str) -> bool {
        self.status(id) == CardStatus::Running && self.waiting.contains(id)
    }

    /// The line under a step's title.
    fn subtitle(&self, flow: &Flow, id: &str) -> String {
        let card = self.cards.get(id);
        let visit = card.and_then(|c| c.visit.as_ref());
        let took = |end: i64| {
            let start = visit.map_or(end, |v| v.started_at);
            crate::shell::running_elapsed((end - start).max(0) as u64)
        };
        let condition = || branch_condition(flow, id);
        let brief = || match flow.state(id) {
            Some(FlowState::Agent(a)) => first_line(&a.brief),
            Some(FlowState::Approval(a)) => first_line(&a.message),
            _ => String::new(),
        };
        match card.map_or(CardStatus::Idle, |c| c.status) {
            CardStatus::Done => took(visit.and_then(|v| v.finished_at).unwrap_or_default()),
            CardStatus::Running if self.waits(id) => "Waiting on you".into(),
            CardStatus::Running => {
                let now = gespan_core::time::now_ms() as i64;
                format!("Running · {}", took(now))
            }
            CardStatus::Failed => visit.and_then(|v| v.error.clone()).map_or_else(
                || "Failed".into(),
                |e| format!("Failed · {}", first_line(&e)),
            ),
            CardStatus::Stopped => "Stopped".into(),
            CardStatus::Skipped => "Skipped".into(),
            CardStatus::Next => "next".into(),
            CardStatus::Idle => condition().unwrap_or_else(brief),
        }
    }

    fn card(&self, id: &str, wide: bool, marked: &[String], cx: &mut Context<Self>) -> AnyElement {
        let Some(flow) = &self.flow else {
            return div().into_any_element();
        };
        let title = flow.state(id).map_or(id, FlowState::title).to_owned();
        let status = self.status(id);
        let waits = self.waits(id);
        let conditional = branch_condition(flow, id).is_some();
        let selected = self.selected.as_deref() == Some(id);
        let changed = marked.iter().any(|m| m == id);
        let subtitle = self.subtitle(flow, id);
        let session = self
            .cards
            .get(id)
            .and_then(|c| c.visit.as_ref())
            .and_then(|v| v.session_id.as_deref())
            .and_then(|s| s.parse::<i64>().ok());
        let (border, fill) = match status {
            CardStatus::Done => (tint(OK, 0.35), tint(OK, 0.07)),
            CardStatus::Running if waits => (tint(WAIT, 0.55), tint(WAIT, 0.10)),
            CardStatus::Running => (tint(RUN, 0.55), tint(RUN, 0.10)),
            CardStatus::Failed => (tint(BAD, 0.45), tint(BAD, 0.08)),
            _ if conditional => (content(0.22), content(0.02)),
            _ => (content(0.12), content(0.035)),
        };
        let mark = match status {
            CardStatus::Done => crate::theme::icon("step-done", 12.)
                .text_color(rgb(OK))
                .into_any_element(),
            CardStatus::Failed => crate::theme::icon("step-failed", 12.)
                .text_color(rgb(BAD))
                .into_any_element(),
            // A live dot wears a 3px halo, drawn as a ring around it: a
            // spread shadow on a round element comes out square.
            CardStatus::Running => {
                let hex = if waits { WAIT } else { RUN };
                div()
                    .size(px(7.))
                    .flex_shrink_0()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .left(px(-3.))
                            .top(px(-3.))
                            .size(px(13.))
                            .rounded_full()
                            .bg(tint(hex, 0.2)),
                    )
                    .child(div().absolute().size(px(7.)).rounded_full().bg(rgb(hex)))
                    .into_any_element()
            }
            _ => div()
                .size(px(7.))
                .rounded_full()
                .flex_shrink_0()
                .bg(content(0.22))
                .into_any_element(),
        };
        let key = id.to_owned();
        let open = id.to_owned();
        let selector = format!("pipeline-step-{id}");
        let quiet = matches!(status, CardStatus::Stopped | CardStatus::Skipped);
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .when(wide, |d| d.flex_1().min_w_0())
            .when(!wide, |d| d.w(px(STEP_W)).flex_none())
            .px(px(9.))
            .py(px(7.))
            .rounded(px(10.))
            .border_1()
            .when(conditional && status == CardStatus::Idle, |d| {
                d.border_dashed()
            })
            .border_color(if selected || changed {
                tint(RUN, 0.85)
            } else {
                border
            })
            .bg(fill)
            .when(waits, |d| d.shadow(vec![ring(tint(WAIT, 0.08), 3.)]))
            .when(quiet, |d| d.opacity(0.6))
            .flex()
            .flex_col()
            .gap(px(2.))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.select(Some(key.clone()), cx);
                    if event.click_count() >= 2 {
                        cx.emit(GraphEvent::Open(open.clone()));
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .line_height(px(14.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(title_ink())
                    .child(mark)
                    .child(div().min_w_0().truncate().child(title)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .text_color(faint_ink())
                    .child(subtitle),
            )
            .when_some(session.filter(|_| waits), |d, session| {
                let id = format!("pipeline-answer-{id}");
                d.child(
                    crate::glass::button(
                        SharedString::from(id.clone()),
                        "Answer",
                        crate::glass::Button::Primary,
                    )
                    .debug_selector(move || id.clone())
                    .role(Role::Button)
                    .aria_label("Answer")
                    .mt(px(4.))
                    .h(px(24.))
                    .text_size(px(11.5))
                    .self_start()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(GraphEvent::Answer(session));
                    })),
                )
            })
            .into_any_element()
    }

    /// The rows with the stems, forks and joins between them.
    fn rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let rows = self.rows.clone();
        let marked = self.marked(cx);
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                out.push(stem());
            }
            match row {
                GraphRow::Step(id) => out.push(
                    div()
                        .flex()
                        .justify_center()
                        .child(self.card(id, false, &marked, cx))
                        .into_any_element(),
                ),
                GraphRow::Branches {
                    branches, retries, ..
                } => {
                    if i > 0 {
                        out.push(fork(branches.len(), true));
                    }
                    out.push(
                        div()
                            .flex()
                            .gap(px(BRANCH_GAP))
                            .children(branches.iter().map(|b| self.card(b, true, &marked, cx)))
                            .into_any_element(),
                    );
                    if *retries > 0 {
                        out.push(
                            div()
                                .debug_selector(|| "pipeline-retries".into())
                                .pt(px(4.))
                                .text_center()
                                .text_size(px(11.5))
                                .line_height(px(16.))
                                .text_color(faint_ink())
                                .child(format!(
                                    "↻ a failed branch is fixed and retried, up to {retries}×"
                                ))
                                .into_any_element(),
                        );
                    }
                    if rows.get(i + 1).is_some() {
                        out.push(fork(branches.len(), false));
                    }
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Move {
    Next,
    Previous,
    Left,
    Right,
}

fn ring(color: Hsla, spread: f32) -> gpui::BoxShadow {
    gpui::BoxShadow::new(px(0.), px(0.), color).spread_radius(px(spread))
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The 1px stem between two rows.
fn stem() -> AnyElement {
    div()
        .w(px(1.))
        .h(px(STEM))
        .self_center()
        .bg(line_ink())
        .into_any_element()
}

/// A fork above `n` branches (`down`) or the join below them: a stem at the
/// centre, a bar across the branch centres and a drop to each.
fn fork(n: usize, down: bool) -> AnyElement {
    let centre = |i: usize| (2 * i + 1) as f32 / (2 * n) as f32;
    let bar_y = FORK / 2.;
    let line = || div().absolute().bg(line_ink());
    let (stem_top, drop_top) = if down { (0., bar_y) } else { (bar_y, 0.) };
    div()
        .relative()
        .h(px(FORK))
        .child(
            line()
                .left(relative(0.5))
                .top(px(stem_top))
                .w(px(1.))
                .h(px(bar_y)),
        )
        .child(
            line()
                .left(relative(centre(0)))
                .right(relative(centre(0)))
                .top(px(bar_y))
                .h(px(1.)),
        )
        .children((0..n).map(|i| {
            line()
                .left(relative(centre(i)))
                .top(px(drop_top))
                .w(px(1.))
                .h(px(bar_y))
        }))
        .into_any_element()
}

impl Render for PipelineGraph {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        div()
            .id("pilot-roadmap-graph")
            .debug_selector(|| "pilot-roadmap-graph".into())
            .track_focus(&self.focus)
            .role(Role::Group)
            .aria_label(SharedString::from(self.aria_label()))
            .on_key_down(cx.listener(Self::key))
            // Tab walks the steps before moving on.
            .on_action(cx.listener(|this, _: &crate::shell::FocusNext, _, cx| {
                if !this.step(Move::Next, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::shell::FocusPrevious, _, cx| {
                if !this.step(Move::Previous, cx) {
                    cx.propagate();
                }
            }))
            .px(px(4.))
            .flex()
            .flex_col()
            .children(rows)
    }
}
