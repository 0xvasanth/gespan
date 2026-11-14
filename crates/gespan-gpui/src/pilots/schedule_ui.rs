//! The Pilot's Schedule tab: "Runs by itself", and the sheet that adds or
//! edits a scheduled run (a habit). Sizes and colours follow the Schedule
//! boards (`PilotSchedule`, `PilotScheduleAdd`, `PilotScheduleEdit`).
use super::scheduler::{duration, grouped_habits, when};
use super::*;
use crate::glass::{self, Button};
use gespan_core::flow::{Flow, FlowState};
use gespan_core::pilot::habits::{Run, Schedule, Target, Tell, WEEKDAYS};
use gespan_core::time::now_ms;
use gpui::{BoxShadow, FontWeight, Hsla};

/// `--ink-muted` (#a3a3a3) and `--ink-faint` (#8a8a8a) over the panel's
/// dark glass, then the board's accent, ok, wait and bad.
fn muted() -> Hsla {
    content(0.68)
}
fn faint() -> Hsla {
    content(0.57)
}
const ACCENT: u32 = 0x459bf7;
const CLOCK: u32 = 0x5eead4;
const OK: u32 = 0x6ee7b7;
const WAIT: u32 = 0xe8c547;
const BAD: u32 = 0xfda4af;

/// `.t1`: 13px on 18px, the bright ink.
fn t1(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(13.))
        .line_height(px(18.))
        .text_color(content(1.))
        .child(text.into())
}
/// `.t2`: 12px on 17px, muted.
fn t2(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(muted())
        .child(text.into())
}
/// `.t3`: 11.5px on 16px, faint.
fn t3(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(faint())
        .child(text.into())
}
/// A form section: its bold label over the control, `padding: 8px 4px`.
fn section(label: &'static str) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .px(px(4.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            t2(label)
                .text_color(content(1.))
                .font_weight(FontWeight::SEMIBOLD),
        )
}
/// The board's `.toggle`: a 30×18 switch, accent when on.
fn toggle(on: bool) -> gpui::Div {
    div()
        .relative()
        .flex_shrink_0()
        .w(px(30.))
        .h(px(18.))
        .rounded_full()
        .bg(if on { rgb(ACCENT).into() } else { white(0.14) })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 14. } else { 2. }))
                .size(px(14.))
                .rounded_full()
                .bg(white(1.))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(1.), black(0.4)).blur_radius(px(3.)),
                ]),
        )
}
/// A radio dot: a 1.5px ring, and when on a dot inside a 3px gap.
fn radio(on: bool) -> gpui::Div {
    div()
        .mt(px(2.))
        .flex_shrink_0()
        .size(px(14.))
        .rounded_full()
        .border(px(1.5))
        .border_color(if on { rgb(ACCENT).into() } else { white(0.3) })
        // `box-shadow: inset 0 0 0 3px #1b1a1f` around the dot.
        .bg(rgb(0x1b1a1f))
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| {
            d.child(div().size(px(5.)).rounded_full().bg(rgb(ACCENT)))
        })
}
/// The board's `.select`: a 30px glass plate whose text ends in ▾.
fn select_plate() -> gpui::Div {
    div()
        .h(px(30.))
        .px(px(10.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.5))
        .whitespace_nowrap()
        .text_color(content(1.))
        .bg(glass::fill_strong())
        .border_1()
        .border_color(glass::edge())
        .shadow(vec![BoxShadow::new(px(0.), px(1.), white(0.10)).inset()])
}
/// A full-width segmented control of equal parts, 26px tall.
fn equal_segment(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    glass::segment(id, label, selected)
        .flex_1()
        .min_w_0()
        .justify_center()
        .px(px(4.))
        .text_size(px(12.))
        .role(Role::RadioButton)
        .aria_label(label)
        .aria_selected(selected)
}

/// The Pilot's Flow a schedule runs: the one the Pipeline tab shows, else
/// the newest.
fn pipeline<'a>(flows: &'a [Flow], shown: Option<&str>) -> Option<&'a Flow> {
    shown
        .and_then(|id| flows.iter().find(|f| f.id == id))
        .or_else(|| flows.first())
}
fn state_title(state: &FlowState) -> &str {
    match state {
        FlowState::Agent(s) => &s.title,
        FlowState::Approval(s) => &s.title,
    }
}
/// The time a run happened, from `now`: "7h ago".
pub(super) fn ago(at: u64, now: u64) -> String {
    let minutes = now.saturating_sub(at) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes}m ago"),
        60..=1439 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}
/// A run in a few words: its report's first line, or what it came to.
pub(super) fn run_summary(run: &Run) -> String {
    let first = |text: &str| {
        text.lines()
            .map(|l| l.trim().trim_matches(|c| "#*_`>-".contains(c)).trim())
            .find(|l| !l.is_empty())
            .map(|l| {
                let mut words: String = l.chars().take(60).collect();
                if l.chars().count() > 60 {
                    words.push('…');
                }
                words
            })
    };
    match run.outcome.as_str() {
        "quiet" => "nothing to report".into(),
        "posted" => run
            .report
            .as_deref()
            .and_then(first)
            .unwrap_or_else(|| "told you".into()),
        "interrupted" => "interrupted".into(),
        _ => "couldn't finish".into(),
    }
}
fn run_dot(run: &Run) -> Hsla {
    match run.outcome.as_str() {
        "quiet" => rgb(OK).into(),
        "posted" => rgb(WAIT).into(),
        "failed" => rgb(BAD).into(),
        _ => faint(),
    }
}
/// The list's third line: running, starting, paused, the last run or the next.
pub(super) fn status_line(h: &Habit, running: Option<u64>, busy: bool, now: u64) -> String {
    if let Some(since) = running {
        format!("Running now · {}", duration(now.saturating_sub(since)))
    } else if busy {
        "Starting…".into()
    } else if !h.enabled {
        "Paused".into()
    } else if let Some(run) = h.runs.first() {
        format!("Last ran {} · {}", ago(run.at, now), run_summary(run))
    } else {
        // A date further out than tomorrow reads without its time.
        let next = Schedule::when(h.next_run_at, now);
        match next.split_once(", ").and_then(|(day, rest)| {
            rest.split_once(" at ")
                .map(|(date, _)| format!("{day}, {date}"))
        }) {
            Some(date) => format!("Next run {date}"),
            None => format!("Next run {next}"),
        }
    }
}

/// Half-hour times for the time select, with the current one if it is not
/// on the half hour.
fn time_choices(current: &str) -> Vec<(String, String)> {
    let mut times: Vec<String> = (0..48)
        .map(|i| format!("{:02}:{:02}", i / 2, (i % 2) * 30))
        .collect();
    let valid = Schedule {
        time: current.into(),
        ..Schedule::default()
    }
    .validate()
    .is_ok();
    if valid && !times.iter().any(|t| t == current) {
        times.push(current.into());
        times.sort();
    }
    times.into_iter().map(|t| (t.clone(), t)).collect()
}

impl Pilots {
    pub(super) fn habits_list(&mut self, pilot: &Pilot, cx: &mut Context<Self>) -> AnyElement {
        let now = now_ms();
        let order = grouped_habits(&self.habits, now, |h| self.habit_running_since(h));
        let rows: Vec<AnyElement> = order
            .into_iter()
            .flatten()
            .filter_map(|ix| self.habits.get(ix).cloned())
            .map(|h| self.habit_row(&h, now, cx))
            .collect();
        let heading = div()
            .flex_shrink_0()
            .px(px(4.))
            .pt(px(14.))
            .pb(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(t1("Runs by itself").font_weight(FontWeight::SEMIBOLD))
            .child(div().flex_1())
            .child(
                glass::button("pilot-new-habit", "+ Add", Button::Ghost)
                    .debug_selector(|| "pilot-new-habit".into())
                    .role(Role::Button)
                    .aria_label("Add a scheduled run")
                    .h(px(26.))
                    .text_size(px(12.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.route = Some(Route::NewHabit);
                        this.clear_habit_fields();
                        cx.notify();
                    })),
            );
        let empty = self.habits.is_empty();
        div()
            .id("pilot-habits-list")
            .debug_selector(|| "pilot-habits-list".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .px(px(12.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(heading)
            .children(rows)
            .child(
                t3(if empty {
                    format!(
                        "Nothing runs by itself yet. Add one, or ask {} to put recurring work on a schedule.",
                        pilot.label()
                    )
                } else {
                    "A scheduled run starts the pipeline on its own and reports back in the chat."
                        .into()
                })
                .debug_selector(|| "pilot-habits-note".into())
                .flex_shrink_0()
                .px(px(6.))
                .py(px(10.)),
            )
            .into_any_element()
    }
    fn habit_row(&self, h: &Habit, now: u64, cx: &mut Context<Self>) -> AnyElement {
        let id = h.id.clone();
        let running = self.habit_running_since(h);
        let busy = self.habit_busy(h);
        let status = status_line(h, running, busy, now);
        let open = id.clone();
        let run = id.clone();
        let flip = id.clone();
        div()
            .id(SharedString::from(format!("pilot-habit-{id}")))
            .debug_selector({
                let id = id.clone();
                move || format!("pilot-habit-{id}")
            })
            .role(Role::Button)
            .aria_label(format!("{}, {}", h.name, h.schedule.label()))
            .cursor_pointer()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .items_start()
            .gap(px(10.))
            .p(px(10.))
            .rounded(px(10.))
            .hover(|s| s.bg(glass::hover_fill()))
            .child(icon("schedule-clock", 15.).text_color(rgb(CLOCK)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(t1(h.name.clone()).truncate())
                    .child(t2(h.schedule.label()).truncate())
                    .child(t3(status).truncate()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(SharedString::from(format!("pilot-habit-run-{id}")))
                            .debug_selector({
                                let id = id.clone();
                                move || format!("pilot-habit-run-{id}")
                            })
                            .role(Role::Button)
                            .aria_label(if running.is_some() {
                                "Already running"
                            } else if busy {
                                "Starting"
                            } else {
                                "Run now"
                            })
                            .tooltip(|_, cx| {
                                cx.new(|_| crate::conversation::HintTooltip("Run now"))
                                    .into()
                            })
                            .cursor_pointer()
                            .size(px(26.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(muted())
                            .hover(|s| s.bg(glass::hover_fill()).text_color(content(1.)))
                            .when(busy, |d| d.opacity(0.35))
                            .child(icon("schedule-play", 14.).text_color(muted()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if !busy {
                                    this.request_habit(&run, cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("pilot-habit-toggle-{id}")))
                            .debug_selector({
                                let id = id.clone();
                                move || format!("pilot-habit-toggle-{id}")
                            })
                            .role(Role::Switch)
                            .aria_label(h.name.clone())
                            .aria_toggled(if h.enabled {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .cursor_pointer()
                            .child(toggle(h.enabled))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.habit_toggle(&flip, cx);
                            })),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.route = Some(Route::Habit(open.clone()));
                this.clear_habit_fields();
                cx.notify();
            }))
            .into_any_element()
    }

    /// The add sheet, or with `id` the same sheet filled in for that habit.
    pub(super) fn habit_page(
        &mut self,
        pilot: &Pilot,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let habit = match id {
            Some(id) => match self.habits.iter().find(|h| h.id == id).cloned() {
                Some(h) => Some(h),
                None => {
                    return div()
                        .size_full()
                        .px(px(12.))
                        .flex()
                        .flex_col()
                        .child(self.breadcrumb("Scheduled run", cx))
                        .child(t2("This scheduled run is no longer available.").px(px(4.)))
                        .into_any_element();
                }
            },
            None => None,
        };
        self.schedule_form(pilot, habit, window, cx)
    }
    fn breadcrumb(&self, title: &str, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex_shrink_0()
            .w_full()
            .min_w_0()
            .px(px(4.))
            .pt(px(14.))
            .pb(px(4.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                t3("Schedule ›")
                    .id("pilot-habit-back")
                    .debug_selector(|| "pilot-habit-back".into())
                    .role(Role::Button)
                    .aria_label("Back to Schedule")
                    .flex_shrink_0()
                    .cursor_pointer()
                    .hover(|s| s.text_color(content(1.)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.route = None;
                        this.clear_habit_fields();
                        cx.notify();
                    })),
            )
            .child(
                t1(title.to_string())
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD),
            )
    }
    /// The fields' values the first time the sheet draws: the habit's, or
    /// a new one's (whole pipeline, daily at 09:00, every run).
    fn fill_habit_fields(&mut self, habit: Option<&Habit>, cx: &mut Context<Self>) {
        let (target, step, tell) = match habit.map(|h| (&h.target, h.tell)) {
            Some((Target::Pipeline { .. }, tell)) => ("pipeline", String::new(), tell),
            Some((Target::Step { step_id, .. }, tell)) => ("step", step_id.clone(), tell),
            Some((Target::Instruction, tell)) => ("instruction", String::new(), tell),
            // A new one starts as the board's: nightly, telling only when needed.
            None => ("pipeline", String::new(), Tell::OnlyIfNeeded),
        };
        let schedule = habit.map(|h| h.schedule.clone()).unwrap_or(Schedule {
            time: "02:00".into(),
            ..Schedule::default()
        });
        let values = [
            ("habit-target", target.to_string()),
            ("habit-step", step),
            (
                "habit-tell",
                match tell {
                    Tell::EveryRun => "everyRun",
                    Tell::OnlyIfNeeded => "onlyIfNeeded",
                }
                .into(),
            ),
            ("habit-kind", schedule.schedule_kind.clone()),
            ("habit-time", schedule.time.clone()),
            ("habit-minute", schedule.minute.to_string()),
            ("habit-weekday", schedule.day_of_week.to_string()),
        ];
        for (key, value) in values {
            self.field(key, key, value, false, cx);
        }
    }
    fn schedule_form(
        &mut self,
        pilot: &Pilot,
        habit: Option<Habit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fresh = !self.fields.contains_key("habit-name");
        let name = if let Some(field) = self.fields.get("habit-name") {
            field.clone()
        } else {
            let field = cx.new(|cx| {
                let mut f = Input::field("Name", "What is this for?", content(0.55), cx);
                f.set_font_size(12.5);
                f.set_max_chars(80);
                f.set_text(
                    habit.as_ref().map(|h| h.name.clone()).unwrap_or_default(),
                    cx,
                );
                f
            });
            self.observations
                .push(cx.subscribe(&field, |_, _, _: &input::Changed, cx| cx.notify()));
            self.observations
                .push(cx.subscribe(&field, |this, _, _: &input::Submitted, cx| {
                    this.save_habit(cx)
                }));
            self.fields.insert("habit-name".into(), field.clone());
            field
        };
        let instructions = if let Some(field) = self.fields.get("habit-instructions") {
            field.clone()
        } else {
            let field = cx.new(Input::habit_instructions);
            field.update(cx, |f, cx| {
                f.set_text(
                    habit
                        .as_ref()
                        .map(|h| h.instructions.clone())
                        .unwrap_or_default(),
                    cx,
                );
                // A saved instruction opens at its start, not scrolled to its end.
                f.select_range(0..0, cx);
            });
            self.observations
                .push(cx.subscribe(&field, |_, _, _: &input::Changed, cx| cx.notify()));
            self.fields
                .insert("habit-instructions".into(), field.clone());
            field
        };
        if fresh {
            self.fill_habit_fields(habit.as_ref(), cx);
            if habit.is_none() {
                self.focus_new_name(&name, window, cx);
            }
        }
        // The Pipeline tab's Flows: what "whole pipeline" and "one step" run.
        let roadmap = self.roadmap(&pilot.id, cx);
        if self
            .habit_ui
            .watching
            .as_ref()
            .is_none_or(|(id, _)| id != &pilot.id)
        {
            let watch = cx.observe(&roadmap, |_, _, cx| cx.notify());
            self.habit_ui.watching = Some((pilot.id.clone(), watch));
        }
        let (flow, steps) = {
            let r = roadmap.read(cx);
            let flow = match habit.as_ref().and_then(|h| h.target.flow_id()) {
                Some(id) => r
                    .flows
                    .iter()
                    .find(|f| f.id == id)
                    .or_else(|| pipeline(&r.flows, r.flow.as_deref())),
                None => pipeline(&r.flows, r.flow.as_deref()),
            };
            let steps: Vec<(String, String)> = flow
                .map(|f| {
                    f.states
                        .iter()
                        .map(|s| (s.id().to_string(), state_title(s).to_string()))
                        .collect()
                })
                .unwrap_or_default();
            (flow.map(|f| f.id.clone()), steps)
        };
        let target = self.value("habit-target", cx);
        let kind = self.value("habit-kind", cx);
        let tell = self.value("habit-tell", cx);
        let editing = habit.is_some();
        let now = now_ms();
        let schedule = self.habit_schedule(cx);
        let next = match schedule.next(now) {
            Ok(at) if editing => format!("Next run: {}.", Schedule::when(at, now)),
            Ok(at) => format!(
                "Next run: {}, then {}.",
                Schedule::when(at, now),
                schedule.cadence()
            ),
            Err(_) => "Pick a time for it to run.".into(),
        };
        let example = steps
            .get(steps.len().saturating_sub(1).min(1))
            .map(|(_, title)| format!("For example only {title}."))
            .unwrap_or_else(|| "No pipeline yet.".into());
        let whole = if steps.is_empty() {
            "No pipeline yet.".to_string()
        } else {
            format!(
                "All {} step{}, same as New run.",
                steps.len(),
                if steps.len() == 1 { "" } else { "s" }
            )
        };
        let has_flow = flow.is_some();
        let cards = div()
            .id("pilot-habit-target")
            .debug_selector(|| "pilot-habit-target".into())
            .role(Role::RadioGroup)
            .aria_label("What it runs")
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(self.target_card(
                "pipeline",
                "Run the whole pipeline",
                whole,
                target == "pipeline",
                has_flow,
                cx,
            ))
            .child(self.target_card(
                "step",
                "Run one step",
                example,
                target == "step",
                has_flow,
                cx,
            ))
            .child(self.target_card(
                "instruction",
                "Give it an instruction",
                "A one-off task in your words.".into(),
                target == "instruction",
                true,
                cx,
            ));
        let mut body = div()
            .id("pilot-habit-editor")
            .debug_selector(|| "pilot-habit-editor".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .px(px(12.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(
                self.breadcrumb(
                    &habit
                        .as_ref()
                        .map(|h| h.name.clone())
                        .unwrap_or_else(|| "New scheduled run".into()),
                    cx,
                ),
            )
            .child(
                section("Name").child(
                    glass::field()
                        .id("pilot-habit-name-field")
                        .debug_selector(|| "pilot-habit-name-field".into())
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(12.5))
                        .child(div().flex_1().min_w_0().child(name)),
                ),
            )
            .child(section("What it runs").child(cards));
        if target == "step" {
            let current = self.value("habit-step", cx);
            let label = steps
                .iter()
                .find(|(id, _)| *id == current)
                .map(|(_, title)| title.clone())
                .unwrap_or_else(|| "Pick a step".into());
            body = body.child(section("Step").child(self.schedule_picker(
                "habit-step",
                "Step",
                label,
                steps.clone(),
                cx,
            )));
        }
        if target == "instruction" {
            body = body.child(
                section("Instruction").child(
                    div()
                        .id("pilot-habit-prompt-field")
                        .debug_selector(|| "pilot-habit-prompt-field".into())
                        .w_full()
                        .min_w_0()
                        .h(px(58.))
                        .overflow_hidden()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(10.))
                        .bg(black(0.35))
                        .border_1()
                        .border_color(glass::edge())
                        .child(instructions),
                ),
            );
        }
        let kinds = [
            ("hourly", "Hourly"),
            ("daily", "Daily"),
            ("weekdays", "Weekdays"),
            ("weekly", "Weekly"),
        ];
        let mut selects = div()
            .id("pilot-habit-schedule")
            .debug_selector(|| "pilot-habit-schedule".into())
            .mt(px(6.))
            .w_full()
            .min_w_0()
            .flex()
            .gap(px(6.));
        if kind == "weekly" {
            let day = self.value("habit-weekday", cx);
            let label = day
                .parse::<usize>()
                .ok()
                .and_then(|d| WEEKDAYS.get(d))
                .map(|d| d.to_string())
                .unwrap_or_else(|| "Monday".into());
            selects = selects.child(
                self.schedule_picker(
                    "habit-weekday",
                    "Day",
                    label,
                    WEEKDAYS
                        .iter()
                        .enumerate()
                        .map(|(ix, d)| (ix.to_string(), d.to_string()))
                        .collect(),
                    cx,
                ),
            );
        }
        selects = if kind == "hourly" {
            let minute = self.value("habit-minute", cx);
            selects.child(
                self.schedule_picker(
                    "habit-minute",
                    "Minute",
                    format!(":{:0>2}", minute),
                    (0..12)
                        .map(|i| ((i * 5).to_string(), format!(":{:02}", i * 5)))
                        .collect(),
                    cx,
                ),
            )
        } else {
            let time = self.value("habit-time", cx);
            selects.child(self.schedule_picker(
                "habit-time",
                "Time",
                time.clone(),
                time_choices(&time),
                cx,
            ))
        };
        selects = selects.child(
            select_plate()
                .id("pilot-habit-zone")
                .debug_selector(|| "pilot-habit-zone".into())
                .flex_1()
                .min_w_0()
                .role(Role::ComboBox)
                .aria_label("Time zone: Local time")
                .child("Local time ▾"),
        );
        body = body
            .child(
                section("When")
                    .child(
                        glass::segmented()
                            .id("pilot-habit-kind")
                            .debug_selector(|| "pilot-habit-kind".into())
                            .role(Role::RadioGroup)
                            .aria_label("When")
                            .w_full()
                            .children(kinds.into_iter().map(|(value, label)| {
                                equal_segment(
                                    SharedString::from(format!("pilot-habit-kind-{value}")),
                                    label,
                                    kind == value,
                                )
                                .debug_selector(move || format!("pilot-habit-kind-{value}"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.set_habit_value("habit-kind", value, cx)
                                    },
                                ))
                            })),
                    )
                    .child(selects)
                    .child(t3(next).debug_selector(|| "pilot-habit-next".into())),
            )
            .child(
                section("Tell me").child(
                    glass::segmented()
                        .id("pilot-habit-tell")
                        .debug_selector(|| "pilot-habit-tell".into())
                        .role(Role::RadioGroup)
                        .aria_label("Tell me")
                        .w_full()
                        .children(
                            [
                                ("everyRun", "Every run"),
                                ("onlyIfNeeded", "Only if it needs me"),
                            ]
                            .into_iter()
                            .map(|(value, label)| {
                                equal_segment(
                                    SharedString::from(format!("pilot-habit-tell-{value}")),
                                    label,
                                    tell == value,
                                )
                                .debug_selector(move || format!("pilot-habit-tell-{value}"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.set_habit_value("habit-tell", value, cx)
                                    },
                                ))
                            }),
                        ),
                ),
            )
            .child(self.form_actions(habit.as_ref(), cx))
            .when_some(self.habit_ui.error.clone(), |d, error| {
                d.child(
                    t3(error)
                        .debug_selector(|| "pilot-habit-error".into())
                        .px(px(4.))
                        .text_color(rgb(BAD)),
                )
            });
        if let Some(h) = &habit {
            body = body.child(self.last_runs(pilot, h, cx));
        }
        body.into_any_element()
    }
    fn target_card(
        &self,
        value: &'static str,
        title: &'static str,
        detail: String,
        selected: bool,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("pilot-habit-target-{value}")))
            .debug_selector(move || format!("pilot-habit-target-{value}"))
            .role(Role::RadioButton)
            .aria_label(title)
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .min_w_0()
            .flex()
            .items_start()
            .gap(px(10.))
            .px(px(10.))
            .py(px(9.))
            .rounded(px(10.))
            .border_1()
            .when(selected, |d| {
                d.border_color(Hsla::from(rgb(ACCENT)).opacity(0.45))
                    .bg(Hsla::from(rgb(ACCENT)).opacity(0.08))
            })
            .when(!selected, |d| {
                d.border_color(glass::edge())
                    .bg(white(0.02))
                    .hover(|s| s.bg(white(0.05)))
            })
            .when(!enabled, |d| d.opacity(0.5))
            .child(radio(selected))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(t1(title))
                    .child(t3(detail)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    this.set_habit_value("habit-target", value, cx)
                }
            }))
    }
    fn form_actions(&self, habit: Option<&Habit>, cx: &mut Context<Self>) -> gpui::Div {
        let row = div()
            .flex_shrink_0()
            .w_full()
            .min_w_0()
            .px(px(4.))
            .py(px(12.))
            .flex()
            .items_center()
            .gap(px(8.));
        let saving = self.habit_ui.saving;
        let save = glass::button(
            "pilot-habit-save",
            match (habit.is_some(), saving) {
                (false, false) => "Add schedule",
                (true, false) => "Save",
                _ => "Saving…",
            },
            Button::Primary,
        )
        .debug_selector(|| "pilot-habit-save".into())
        .role(Role::Button)
        .h(px(30.))
        .when(saving, |d| d.opacity(0.6))
        .on_click(cx.listener(|this, _, _, cx| this.save_habit(cx)));
        let Some(h) = habit else {
            return row.child(save).child(
                glass::button("pilot-habit-cancel", "Cancel", Button::Ghost)
                    .debug_selector(|| "pilot-habit-cancel".into())
                    .role(Role::Button)
                    .h(px(30.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.route = None;
                        this.clear_habit_fields();
                        cx.notify();
                    })),
            );
        };
        let busy = self.habit_busy(h);
        let (run, flip, remove) = (h.id.clone(), h.id.clone(), h.id.clone());
        row.child(save)
            .child(
                glass::button("pilot-habit-run", "Run now", Button::Glass)
                    .debug_selector(|| "pilot-habit-run".into())
                    .role(Role::Button)
                    .h(px(30.))
                    .when(busy, |d| d.opacity(0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !busy {
                            this.request_habit(&run, cx)
                        }
                    })),
            )
            .child(
                glass::button(
                    "pilot-habit-toggle",
                    if h.enabled { "Pause" } else { "Resume" },
                    Button::Ghost,
                )
                .debug_selector(|| "pilot-habit-toggle".into())
                .role(Role::Button)
                .h(px(30.))
                .on_click(cx.listener(move |this, _, _, cx| this.habit_toggle(&flip, cx))),
            )
            .child(div().flex_1())
            .child(
                glass::button("pilot-habit-delete", "Delete", Button::Ghost)
                    .debug_selector(|| "pilot-habit-delete".into())
                    .role(Role::Button)
                    .h(px(30.))
                    .text_color(rgb(BAD))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_habit(&remove, cx))),
            )
    }
    fn last_runs(&self, pilot: &Pilot, h: &Habit, cx: &mut Context<Self>) -> gpui::Div {
        let now = now_ms();
        let mut list = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .pb(px(12.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(4.))
                    .pt(px(10.))
                    .pb(px(6.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(faint())
                    .child("LAST RUNS"),
            );
        if let Some(since) = self.habit_running_since(h) {
            list = list.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .px(px(4.))
                    .py(px(2.))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(muted())
                    .child(
                        div()
                            .mt(px(5.))
                            .size(px(7.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child("Running now"))
                    .child(t3(duration(now.saturating_sub(since)))),
            );
        } else if h.runs.is_empty() {
            list = list.child(t2("It hasn't run yet.").px(px(4.)).py(px(2.)));
        }
        for run in &h.runs {
            list = list.child(self.habit_run_row(pilot, h, run, now, cx));
        }
        list
    }
    fn habit_run_row(
        &self,
        pilot: &Pilot,
        h: &Habit,
        run: &Run,
        now: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = (h.id.clone(), run.at);
        let expanded = self.habit_ui.expanded.contains(&key);
        let detail = run
            .report
            .as_ref()
            .or(run.error.as_ref())
            .is_some_and(|text| !text.is_empty());
        let text = format!("{} · {}", when(run.at, now), run_summary(run));
        let mut card = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!(
                        "pilot-habit-history-{}-{}",
                        h.id, run.at
                    )))
                    .debug_selector({
                        let id = h.id.clone();
                        let at = run.at;
                        move || format!("pilot-habit-history-{id}-{at}")
                    })
                    .role(Role::Button)
                    .aria_label(text.clone())
                    .w_full()
                    .min_w_0()
                    .flex()
                    .gap(px(8.))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(muted())
                    .when(detail, |d| {
                        d.cursor_pointer()
                            .aria_expanded(expanded)
                            .hover(|s| s.bg(glass::hover_fill()))
                    })
                    .child(
                        div()
                            .mt(px(5.))
                            .size(px(7.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(run_dot(run)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(text))
                    .when(run.duration_ms > 0, |d| {
                        d.child(t3(duration(run.duration_ms)).flex_shrink_0())
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if detail {
                            if !this.habit_ui.expanded.remove(&key) {
                                this.habit_ui.expanded.insert(key.clone());
                            }
                            cx.notify();
                        }
                    })),
            );
        if expanded && detail {
            let mut body = div()
                .id(SharedString::from(format!(
                    "pilot-habit-report-{}-{}",
                    h.id, run.at
                )))
                .debug_selector({
                    let id = h.id.clone();
                    let at = run.at;
                    move || format!("pilot-habit-report-{id}-{at}")
                })
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .pl(px(19.))
                .pr(px(4.))
                .pb(px(6.))
                .text_size(px(12.))
                .line_height(px(18.));
            if let Some(report) = &run.report {
                for (ix, mut row) in crate::markdown_model::parse_full(report)
                    .blocks
                    .iter()
                    .flat_map(|b| crate::markdown::rows(&b.block))
                    .enumerate()
                {
                    crate::markdown::resolve_file_links(
                        &mut row,
                        &pilot.projects.first().cloned().unwrap_or_default(),
                    );
                    body = body.child(div().w_full().min_w_0().child(crate::markdown::body(
                        &row,
                        format!("habit-report-{}-{}-{ix}", h.id, run.at).into(),
                        crate::markdown::Tone::Normal,
                        None,
                        std::time::Instant::now(),
                    )));
                }
            } else if let Some(error) = &run.error {
                body = body.child(div().text_color(rgb(BAD)).child(error.clone()));
            }
            card = card.child(body);
        }
        card.into_any_element()
    }
    fn set_habit_value(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        if let Some(field) = self.fields.get(key) {
            field.update(cx, |f, cx| f.set_text(value.to_string(), cx));
        }
        self.habit_ui.error = None;
        cx.notify();
    }
    fn focus_new_name(
        &mut self,
        name: &Entity<Input>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Views are rendered lazily during prepaint. The chat composer may
        // already have claimed this frame's accessibility focus, so moving
        // focus here would let the new field claim it a second time.
        let manager = cx.weak_entity();
        let focus = name.focus_handle(cx);
        let field_id = name.entity_id();
        let revision = self.habit_ui.revision;
        window.on_next_frame(move |window, cx| {
            let _ = manager.update(cx, |this, cx| {
                if this.route == Some(Route::NewHabit)
                    && this.habit_ui.revision == revision
                    && this
                        .fields
                        .get("habit-name")
                        .is_some_and(|field| field.entity_id() == field_id)
                {
                    window.focus(&focus, cx);
                }
            });
        });
    }
    pub(super) fn habit_schedule(&self, cx: &App) -> Schedule {
        Schedule {
            schedule_kind: self.value("habit-kind", cx),
            time: self.value("habit-time", cx).trim().into(),
            minute: self.value("habit-minute", cx).parse().unwrap_or(60),
            day_of_week: self.value("habit-weekday", cx).parse().unwrap_or(7),
        }
    }
    /// What the sheet says to run, against the Pipeline tab's Flow.
    fn habit_target(&self, pilot: &str, cx: &mut Context<Self>) -> Target {
        let roadmap = self.roadmaps.get(pilot).map(|(r, _)| r.clone());
        let editing = match &self.route {
            Some(Route::Habit(id)) => self.habits.iter().find(|h| &h.id == id),
            _ => None,
        };
        let flow_id = roadmap
            .and_then(|r| {
                let r = r.read(cx);
                let kept = editing
                    .and_then(|h| h.target.flow_id())
                    .and_then(|id| r.flows.iter().find(|f| f.id == id));
                kept.or_else(|| pipeline(&r.flows, r.flow.as_deref()))
                    .map(|f| f.id.clone())
            })
            .or_else(|| editing.and_then(|h| h.target.flow_id().map(str::to_owned)))
            .unwrap_or_default();
        match self.value("habit-target", cx).as_str() {
            "pipeline" => Target::Pipeline { flow_id },
            "step" => Target::Step {
                flow_id,
                step_id: self.value("habit-step", cx),
            },
            _ => Target::Instruction,
        }
    }
    pub(super) fn save_habit(&mut self, cx: &mut Context<Self>) {
        let editing = match &self.route {
            Some(Route::NewHabit) => None,
            Some(Route::Habit(id)) => Some(id.clone()),
            _ => return,
        };
        if self.habit_ui.saving {
            return;
        }
        let Some(pilot) = self.active.clone() else {
            return;
        };
        let name: String = self
            .value("habit-name", cx)
            .trim()
            .chars()
            .take(80)
            .collect();
        let instructions: String = self
            .value("habit-instructions", cx)
            .trim()
            .chars()
            .take(4000)
            .collect();
        let target = self.habit_target(&pilot, cx);
        let tell = if self.value("habit-tell", cx) == "onlyIfNeeded" {
            Tell::OnlyIfNeeded
        } else {
            Tell::EveryRun
        };
        let schedule = self.habit_schedule(cx);
        let now = now_ms();
        // Checked here first, so a mistake keeps the sheet and its draft.
        let checked = Habit::create(
            name.clone(),
            target.clone(),
            instructions.clone(),
            tell,
            schedule.clone(),
            now,
        );
        let new = match checked {
            Ok(h) => h,
            Err(error) => {
                self.habit_ui.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.habit_ui.saving = true;
        self.habit_ui.error = None;
        let root = self.root.clone();
        let target_pilot = pilot.clone();
        let revision = self.habit_ui.revision;
        let route = self.route.clone();
        let work = cx.background_executor().spawn(async move {
            habits::update(&root, &target_pilot, |all| match &editing {
                None => {
                    all.push(new.clone());
                    Ok(())
                }
                Some(id) => all
                    .iter_mut()
                    .find(|h| &h.id == id)
                    .ok_or("This scheduled run is no longer available")?
                    .edit(
                        name.clone(),
                        target.clone(),
                        instructions.clone(),
                        tell,
                        schedule.clone(),
                        now,
                    ),
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.habit_ui.saving = false;
                let current = this.active.as_deref() == Some(&pilot)
                    && this.route == route
                    && this.habit_ui.revision == revision;
                match result {
                    Ok(()) => {
                        this.reload_files(&pilot, cx);
                        if current {
                            this.route = None;
                            this.clear_habit_fields();
                        }
                    }
                    Err(error) => {
                        if current {
                            this.habit_ui.error = Some(error);
                        }
                    }
                }
                this.changed(cx);
            });
        })
        .detach();
        cx.notify();
    }
    /// A `.select` that opens a menu of `choices` for `field`.
    fn schedule_picker(
        &self,
        field: &'static str,
        label: &'static str,
        shown: String,
        choices: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let anchor = AnchorSlot::default();
        let Some(focus) = self.fields.get(field).map(|f| f.focus_handle(cx)) else {
            return div().into_any_element();
        };
        let keyboard_choices = choices.clone();
        let keyboard_anchor = anchor.clone();
        let expanded = self.picker.is_some() && self.habit_ui.picker_field == Some(field);
        select_plate()
            .id(field)
            .debug_selector(move || format!("pilot-{field}-picker"))
            .role(Role::ComboBox)
            .aria_label(format!("{label}: {shown}"))
            .aria_expanded(expanded)
            .track_focus(&focus)
            .tab_index(0)
            .cursor_pointer()
            .relative()
            .flex_1()
            .min_w_0()
            .hover(|s| s.bg(white(0.10)))
            .child(anchor.probe())
            .child(div().min_w_0().truncate().child(format!("{shown} ▾")))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if matches!(
                        event.keystroke.key.as_str(),
                        "enter" | "space" | "down" | "up"
                    ) {
                        this.open_schedule_picker(
                            field,
                            label,
                            keyboard_choices.clone(),
                            keyboard_anchor.clone(),
                            window,
                            cx,
                        );
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(field) = this.fields.get(field) {
                    window.focus(&field.focus_handle(cx), cx);
                }
                this.open_schedule_picker(
                    field,
                    label,
                    choices.clone(),
                    anchor.clone(),
                    window,
                    cx,
                );
            }))
            .into_any_element()
    }
    fn open_schedule_picker(
        &mut self,
        field: &'static str,
        label: &'static str,
        choices: Vec<(String, String)>,
        anchor: AnchorSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.value(field, cx);
        let items = choices
            .into_iter()
            .map(|(value, label)| {
                crate::popover::MenuAction::new(value.clone(), label)
                    .checked(value == current)
                    .into()
            })
            .collect();
        let menu = cx.new(|cx| {
            Menu::new(label, items, window, cx)
                .width(200.)
                .select_value(&current)
        });
        let picked = cx.subscribe(&menu, move |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(value, _) = event
                && let Some(input) = this.fields.get(field)
            {
                input.update(cx, |input, cx| input.set_text(value.to_string(), cx));
                this.habit_ui.error = None;
            }
            this.picker = None;
            this.habit_ui.picker_field = None;
            cx.notify();
        });
        self.habit_ui.picker_field = Some(field);
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(anchor),
            width: 200.,
            align: crate::popover::Align::Start,
            _events: vec![picked],
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(outcome: &str, report: Option<&str>) -> Run {
        Run {
            at: 0,
            duration_ms: 0,
            outcome: outcome.into(),
            report: report.map(str::to_owned),
            error: None,
        }
    }

    #[test]
    fn a_run_reads_as_its_reports_first_line_or_what_it_came_to() {
        assert_eq!(
            run_summary(&run("posted", Some("**Nothing to ship**\n\nmain is clean"))),
            "Nothing to ship"
        );
        assert_eq!(run_summary(&run("posted", None)), "told you");
        assert_eq!(run_summary(&run("quiet", None)), "nothing to report");
        assert_eq!(run_summary(&run("failed", None)), "couldn't finish");
        let long = "a".repeat(80);
        assert_eq!(
            run_summary(&run("posted", Some(&long))).chars().count(),
            61,
            "60 characters and an ellipsis"
        );
    }

    #[test]
    fn the_status_line_says_running_paused_the_last_run_or_the_next() {
        let now = 10 * 3_600_000;
        let mut h = Habit::new("Nightly".into(), "Check".into(), Schedule::default(), now).unwrap();
        assert!(status_line(&h, None, false, now).starts_with("Next run "));
        h.runs.push(Run {
            at: now - 7 * 3_600_000,
            ..run("posted", Some("nothing to ship"))
        });
        assert_eq!(
            status_line(&h, None, false, now),
            "Last ran 7h ago · nothing to ship"
        );
        assert_eq!(status_line(&h, None, true, now), "Starting…");
        assert_eq!(
            status_line(&h, Some(now - 72_000), true, now),
            "Running now · 1m 12s"
        );
        h.enabled = false;
        assert_eq!(status_line(&h, None, false, now), "Paused");
    }

    #[test]
    fn time_choices_are_half_hours_plus_a_custom_time() {
        let times = time_choices("02:00");
        assert_eq!(times.len(), 48);
        assert_eq!(times[4].0, "02:00");
        let custom = time_choices("02:15");
        assert_eq!(custom.len(), 49);
        assert!(custom.iter().any(|(t, _)| t == "02:15"));
        assert_eq!(time_choices("99:99").len(), 48);
    }
}
