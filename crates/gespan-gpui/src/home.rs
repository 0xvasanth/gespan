//! Home: one page that says how the day is going. What waits on you, the
//! agents working now, how the automations ran, the Inbox, plan usage and
//! today's events, each a click away from its own page.
//!
//! Nothing here is stored. The shell hands the page a [`Data`] snapshot of
//! what it already knows (My tasks, the open agents, the Inbox, the usage
//! cache) and the page reads the automation run records itself, the way
//! Runs does.
use crate::{
    glass::{self, Button, Tone},
    my_tasks::{self, Source},
    theme::*,
};
use chrono::{Datelike, Local, TimeZone, Timelike};
use gespan_core::agent::Decision;
use gespan_core::automations::{AutomationRun, Service};
use gespan_core::tasks::Kind;
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, Role,
    SharedString, Task, Window, canvas, div, prelude::*, px, relative,
};
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(test)]
mod tests;

const DAY_MS: i64 = 86_400_000;
/// How often the page asks the shell for a fresh snapshot while shown.
const TICK: Duration = Duration::from_secs(5);
/// Run records reload every this many ticks (30 seconds, as on Runs).
const RUNS_EVERY: u32 = 6;
/// Enough runs for a busy month; the service caps a read at 500.
const RUN_LIMIT: usize = 500;
/// Rows each list shows before it says how many more there are.
const WAITING_ROWS: usize = 3;
const WORKING_ROWS: usize = 4;
/// The widest Home's content grows. Past this a big screen gets margins,
/// not stretched panels.
pub(crate) const MAX_WIDTH: f32 = 1200.;
/// Narrower than this, the two columns stack into one.
const COLUMNS_FROM: f32 = 860.;
/// Narrower than this, the four tiles sit two to a row; narrower than
/// `ONE_TILE_BELOW`, one to a row.
const FOUR_TILES_FROM: f32 = 720.;
const ONE_TILE_BELOW: f32 = 380.;

/// How Home arranges itself in a content area this wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Waiting and Working on the left, the charts on the right; else one
    /// column, in that order.
    pub(crate) columns: bool,
    pub(crate) tiles_per_row: usize,
}

/// Before the page has been measured (`width` 0) it lays out as wide.
pub(crate) fn layout(width: f32) -> Layout {
    if width <= 0. {
        return Layout {
            columns: true,
            tiles_per_row: 4,
        };
    }
    Layout {
        columns: width >= COLUMNS_FROM,
        tiles_per_row: if width >= FOUR_TILES_FROM {
            4
        } else if width >= ONE_TILE_BELOW {
            2
        } else {
            1
        },
    }
}
const TIMELINE_ROWS: usize = 6;

/// The range the header's segmented control scopes the counts to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Range {
    #[default]
    Today,
    Week,
    Month,
}

impl Range {
    pub(crate) const ALL: [Range; 3] = [Range::Today, Range::Week, Range::Month];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Week => "7 days",
            Self::Month => "30 days",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
    /// Calendar days the range covers, today included.
    pub(crate) fn days(self) -> usize {
        match self {
            Self::Today => 1,
            Self::Week => 7,
            Self::Month => 30,
        }
    }
    /// Days the chart draws: a week for Today and 7 days, else the month.
    pub(crate) fn chart_days(self) -> usize {
        self.days().max(7)
    }
    /// How the summary sentence names the range.
    pub(crate) fn phrase(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week => "in the last 7 days",
            Self::Month => "in the last 30 days",
        }
    }
}

/// The page a headline tile or a panel link opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    MyTasks,
    Agents,
    Runs,
    Inbox,
}

pub(crate) enum Event {
    /// Answer a task the way My tasks does; the shell routes it there.
    Task(my_tasks::Event),
    /// Open My tasks on this task, to answer it in full.
    OpenTask(String),
    Open(Page),
    /// Open the agent's chat, or its Pilot.
    OpenAgent(i64),
    /// The page is on screen and wants the shell's latest snapshot.
    Refresh,
}

/// One task waiting on you, as Home lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Waiting {
    pub key: String,
    pub kind: Kind,
    pub headline: String,
    /// The chat, Pilot, Flow or review it comes from.
    pub source_label: String,
    pub project: String,
    pub since_ms: u64,
    pub source: Source,
    /// A tool approval Home may allow in one click; see [`one_click`].
    pub allow: Option<OneClick>,
}

/// A tool approval simple enough to allow from Home: its session, request
/// and the whole command, which the row shows before you can allow it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OneClick {
    pub session: i64,
    pub request: String,
    pub command: String,
}

/// Commands longer than this open in My tasks instead, so the row never
/// has to cut the command it asks you to allow.
const ONE_CLICK_MAX: usize = 120;

/// The command Home may offer "Allow once" for: the approval card's lone
/// command (a shell command with no alert, MCP server or other argument,
/// its hidden characters marked) on one line and short enough to show in
/// full. Anything else opens in My tasks, which shows the whole card.
pub(crate) fn one_click(lone_command: Option<&str>) -> Option<String> {
    let command = lone_command?.trim();
    (!command.is_empty() && !command.contains('\n') && command.chars().count() <= ONE_CLICK_MAX)
        .then(|| command.to_owned())
}

/// One agent working now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Working {
    pub id: i64,
    pub title: String,
    pub provider: String,
    /// "Claude Code", or the Pilot's name.
    pub origin: String,
    pub project: String,
    pub started_at: Option<u64>,
}

/// The Inbox's counts, from the items it last loaded.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InboxCounts {
    pub total: usize,
    pub unread: usize,
    pub review: usize,
    pub assigned: usize,
    /// Review requests with their last update, in milliseconds.
    pub reviews: Vec<(String, i64)>,
}

/// One provider's plan usage: its tightest window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Usage {
    pub provider: String,
    pub label: String,
    pub used: f64,
    pub resets_at: Option<i64>,
}

/// What the shell knows, handed over on every refresh.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Data {
    pub waiting: Vec<Waiting>,
    pub working: Vec<Working>,
    /// `None` until the Inbox has loaded.
    pub inbox: Option<InboxCounts>,
    pub usage: Vec<Usage>,
}

/// Runs in a range: every run started in it, and the finished ones by result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunCounts {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
}

impl RunCounts {
    /// Succeeded over succeeded and failed, rounded; `None` with neither.
    pub(crate) fn pass_rate(self) -> Option<u32> {
        let finished = self.succeeded + self.failed;
        (finished > 0).then(|| ((self.succeeded * 100) as f64 / finished as f64).round() as u32)
    }
}

/// One day's bar: its local midnight and what finished that day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DayBar {
    pub start: i64,
    pub succeeded: usize,
    pub failed: usize,
}

/// A timeline entry: when, its colour, the name in bold and the rest.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Moment {
    pub at: i64,
    pub tone: Tone,
    pub name: String,
    pub text: String,
}

/// The greeting for a local hour: morning until noon, afternoon until six,
/// evening after (and before five, when it is still the night before).
pub(crate) fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning.",
        12..=17 => "Good afternoon.",
        _ => "Good evening.",
    }
}

/// Local midnight of the day `days_ago` days before the one holding `now`.
pub(crate) fn day_start(now: i64, days_ago: usize) -> i64 {
    let Some(time) = Local.timestamp_millis_opt(now).single() else {
        return now - now.rem_euclid(DAY_MS) - days_ago as i64 * DAY_MS;
    };
    let date = time.date_naive() - chrono::Days::new(days_ago as u64);
    date.and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
        .map_or(
            now - now.rem_euclid(DAY_MS) - days_ago as i64 * DAY_MS,
            |m| m.timestamp_millis(),
        )
}

/// The runs started in `[since, ∞)`, counted by result.
pub(crate) fn run_counts(runs: &[AutomationRun], since: i64) -> RunCounts {
    let shown = runs.iter().filter(|run| run.created_at >= since);
    RunCounts {
        total: shown.clone().count(),
        succeeded: shown.clone().filter(|r| r.status == "succeeded").count(),
        failed: shown.filter(|r| r.status == "failed").count(),
    }
}

/// One bar per day starting at each of `starts` (oldest first), counting
/// the runs created that day that succeeded or failed.
pub(crate) fn day_bars(runs: &[AutomationRun], starts: &[i64]) -> Vec<DayBar> {
    starts
        .iter()
        .enumerate()
        .map(|(ix, &start)| {
            let end = starts.get(ix + 1).copied().unwrap_or(i64::MAX);
            let day = runs
                .iter()
                .filter(|run| run.created_at >= start && run.created_at < end);
            DayBar {
                start,
                succeeded: day.clone().filter(|r| r.status == "succeeded").count(),
                failed: day.filter(|r| r.status == "failed").count(),
            }
        })
        .collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The header's sentence, from the live counts and the range's runs.
pub(crate) fn summary(working: usize, waiting: usize, runs: RunCounts, range: Range) -> String {
    let agents = match working {
        0 => "No agents are working".to_owned(),
        1 => "1 agent is working".to_owned(),
        n => format!("{n} agents are working"),
    };
    let tasks = match waiting {
        0 => "nothing is waiting on you".to_owned(),
        1 => "1 thing is waiting on you".to_owned(),
        n => format!("{n} things are waiting on you"),
    };
    let when = range.phrase();
    let finished = runs.succeeded + runs.failed;
    let automations = match (finished, runs.failed) {
        (0, _) if runs.total > 0 => format!("automations are still running {when}"),
        (0, _) => format!("no automations ran {when}"),
        (_, 0) => format!("every automation passed {when}"),
        (n, f) if f == n => format!(
            "{} failed {when}",
            if n == 1 {
                "the one automation run".to_owned()
            } else {
                format!("all {n} automation runs")
            }
        ),
        (_, 1) => format!("every automation passed {when} except one"),
        (n, f) => format!("{f} of {n} automation runs failed {when}"),
    };
    format!("{agents}, {tasks}, and {automations}.")
}

/// A wait or age, coarse: `1m`, `45m`, `2h`, `3d`.
pub(crate) fn age(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0..60 => format!("{}m", minutes.max(1)),
        60..1_440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1_440),
    }
}

fn kind_noun(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Approval => ("approval", "approvals"),
        Kind::Question => ("question", "questions"),
        Kind::FlowApproval => ("Flow approval", "Flow approvals"),
        Kind::PilotChoice => ("choice", "choices"),
        Kind::DraftReview => ("draft review", "draft reviews"),
        Kind::Decision => ("decision to review", "decisions to review"),
    }
}

/// "1 approval · 1 question": the waiting tasks by kind, tool approvals
/// first as in the mock.
pub(crate) fn kinds_line(waiting: &[Waiting]) -> String {
    const ORDER: [Kind; 6] = [
        Kind::Approval,
        Kind::Question,
        Kind::FlowApproval,
        Kind::PilotChoice,
        Kind::DraftReview,
        Kind::Decision,
    ];
    let parts: Vec<String> = ORDER
        .iter()
        .filter_map(|kind| {
            let n = waiting.iter().filter(|w| w.kind == *kind).count();
            let (one, many) = kind_noun(*kind);
            (n > 0).then(|| plural(n, one, many))
        })
        .collect();
    if parts.is_empty() {
        "Nothing waiting".into()
    } else {
        parts.join(" · ")
    }
}

/// "4 Claude Code · 2 Codex": the working agents by origin, most first.
pub(crate) fn origins_line(working: &[Working]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for agent in working {
        match counts.iter_mut().find(|(o, _)| *o == agent.origin) {
            Some((_, n)) => *n += 1,
            None => counts.push((&agent.origin, 1)),
        }
    }
    counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    if counts.is_empty() {
        return "No agents running".into();
    }
    counts
        .iter()
        .map(|(origin, n)| format!("{n} {origin}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "3 reviews requested · 2 assigned", leaving out what is zero.
pub(crate) fn inbox_line(inbox: Option<&InboxCounts>) -> String {
    let Some(inbox) = inbox else {
        return "Loading the Inbox".into();
    };
    let mut parts = Vec::new();
    if inbox.review > 0 {
        parts.push(plural(
            inbox.review,
            "review requested",
            "reviews requested",
        ));
    }
    if inbox.assigned > 0 {
        parts.push(format!("{} assigned", inbox.assigned));
    }
    if parts.is_empty() {
        if inbox.total == 0 {
            "Nothing in the Inbox".into()
        } else {
            "No reviews or assignments".into()
        }
    } else {
        parts.join(" · ")
    }
}

/// Today's events, newest first: runs that finished or failed, tasks raised
/// and review requests that came in, all since `today_start`.
pub(crate) fn timeline(
    runs: &[AutomationRun],
    names: &HashMap<String, String>,
    data: &Data,
    today_start: i64,
) -> Vec<Moment> {
    let mut moments: Vec<Moment> = Vec::new();
    for run in runs {
        let Some(at) = run.completed_at.filter(|at| *at >= today_start) else {
            continue;
        };
        let name = names
            .get(&run.automation_id)
            .cloned()
            .unwrap_or_else(|| "An automation".into());
        let item = crate::runs::item_label(run.event_title.as_deref(), run.event_key.as_deref());
        let (tone, text) = match run.status.as_str() {
            "succeeded" => (
                Tone::Ok,
                match item {
                    Some(item) => format!("finished {item}"),
                    None => "finished".into(),
                },
            ),
            "failed" => (
                Tone::Bad,
                match run
                    .error
                    .as_deref()
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                {
                    Some(error) => format!("failed: {}", error.lines().next().unwrap_or(error)),
                    None => "failed".into(),
                },
            ),
            _ => continue,
        };
        moments.push(Moment {
            at,
            tone,
            name,
            text,
        });
    }
    for task in &data.waiting {
        let at = task.since_ms as i64;
        if at < today_start {
            continue;
        }
        let verb = if task.kind == Kind::Approval {
            "wants approval"
        } else {
            "asked"
        };
        moments.push(Moment {
            at,
            tone: Tone::Waiting,
            name: task.source_label.clone(),
            text: format!("{verb}: {}", task.headline),
        });
    }
    if let Some(inbox) = &data.inbox {
        for (label, at) in &inbox.reviews {
            if *at >= today_start {
                moments.push(Moment {
                    at: *at,
                    tone: Tone::Running,
                    name: label.clone(),
                    text: "is waiting for your review".into(),
                });
            }
        }
    }
    moments.sort_by_key(|m| std::cmp::Reverse(m.at));
    moments
}

fn clock(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

/// The eyebrow's date: "Thursday, 12 November".
fn date_line(now: i64) -> String {
    Local
        .timestamp_millis_opt(now)
        .single()
        .map(|time| time.format("%A, %-d %B").to_string())
        .unwrap_or_default()
}

fn weekday(start: i64) -> String {
    Local
        .timestamp_millis_opt(start)
        .single()
        .map(|time| time.weekday().to_string())
        .unwrap_or_default()
}

pub(crate) struct Home {
    focus: FocusHandle,
    range: Range,
    data: Data,
    service: Option<Arc<Mutex<Service>>>,
    runs: Vec<AutomationRun>,
    /// Automation names by id, for the timeline.
    names: HashMap<String, String>,
    runs_loaded: bool,
    generation: u64,
    ticker: Option<Task<()>>,
    /// The content's width at the last paint, for [`layout`].
    width: Rc<Cell<f32>>,
}

impl EventEmitter<Event> for Home {}

impl Focusable for Home {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Home {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            range: Range::default(),
            data: Data::default(),
            service: None,
            runs: Vec::new(),
            names: HashMap::new(),
            runs_loaded: false,
            generation: 0,
            ticker: None,
            width: Rc::new(Cell::new(0.)),
        }
    }

    #[cfg(test)]
    pub(crate) fn range(&self) -> Range {
        self.range
    }

    pub(crate) fn set_range(&mut self, range: Range, cx: &mut Context<Self>) {
        if self.range != range {
            self.range = range;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn waiting_for_test(&self) -> &[Waiting] {
        &self.data.waiting
    }

    pub(crate) fn set_data(&mut self, data: Data, cx: &mut Context<Self>) {
        if self.data != data {
            self.data = data;
            cx.notify();
        }
    }

    /// The automation service, once the shell has opened it. The first one
    /// handed over loads the runs.
    pub(crate) fn set_service(&mut self, service: Arc<Mutex<Service>>, cx: &mut Context<Self>) {
        if self.service.is_none() {
            self.service = Some(service);
            self.reload_runs(cx);
        }
    }

    /// The run records and automation names, as Runs reads them.
    pub(crate) fn set_runs(
        &mut self,
        runs: Vec<AutomationRun>,
        names: HashMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        self.runs = runs;
        self.names = names;
        self.runs_loaded = true;
        cx.notify();
    }

    fn reload_runs(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let work = cx.background_executor().spawn(async move {
            let mut service = service
                .lock()
                .map_err(|_| "Automations are unavailable".to_string())?;
            let runs = service.recent_runs(RUN_LIMIT)?;
            let names = service
                .list()?
                .into_iter()
                .map(|automation| (automation.id, automation.name))
                .collect();
            Ok::<_, String>((runs, names))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((runs, names)) => this.set_runs(runs, names, cx),
                    Err(error) => tracing::warn!(%error, "Home could not read runs"),
                }
            });
        })
        .detach();
    }

    /// On screen: ask for a snapshot now and every few seconds, and reload
    /// the runs every half minute.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        self.reload_runs(cx);
        cx.emit(Event::Refresh);
        if self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                let mut ticks = 0u32;
                loop {
                    cx.background_executor().timer(TICK).await;
                    ticks += 1;
                    let reload = ticks.is_multiple_of(RUNS_EVERY);
                    let alive = this.update(cx, |this, cx| {
                        if reload {
                            this.reload_runs(cx);
                        }
                        cx.emit(Event::Refresh);
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        }
    }

    pub(crate) fn hidden(&mut self) {
        self.ticker = None;
    }

    /// The counts the tiles and the summary show for the current range.
    pub(crate) fn run_counts(&self, now: i64) -> RunCounts {
        run_counts(&self.runs, day_start(now, self.range.days() - 1))
    }

    /// The chart's days, oldest first, ending today.
    pub(crate) fn bars(&self, now: i64) -> Vec<DayBar> {
        let days = self.range.chart_days();
        let starts: Vec<i64> = (0..days).rev().map(|ago| day_start(now, ago)).collect();
        day_bars(&self.runs, &starts)
    }

    pub(crate) fn summary(&self, now: i64) -> String {
        let working = self.data.working.len();
        summary(
            working,
            self.data.waiting.len(),
            self.run_counts(now),
            self.range,
        )
    }
}

/// What one headline tile shows.
struct Tile {
    page: Page,
    label: &'static str,
    pill: Option<(String, Tone)>,
    value: String,
    ink: Hsla,
    line: String,
}

/// The 12px panel heading with an optional note and a link on the right.
fn panel_head(title: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .pt(px(14.))
        .pb(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(title),
        )
}

fn note(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(content(0.55))
        .child(text.into())
}

fn row() -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(11.))
        .px(px(16.))
        .py(px(9.))
        .border_t_1()
        .border_color(glass::hairline())
}

/// A tile's big number, in the mock's 30px display weight.
fn big(value: String, ink: Hsla) -> gpui::Div {
    div()
        .text_size(px(30.))
        .line_height(px(36.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ink)
        .child(value)
}

fn small_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    kind: Button,
) -> gpui::Stateful<gpui::Div> {
    glass::button(id, label, kind)
        .h(px(28.))
        .px(px(12.))
        .text_size(px(12.))
}

/// The wait yellow a tile's number takes while something waits.
fn wait_number() -> Hsla {
    gpui::rgb(if is_light() { 0xa86b00 } else { 0xf5dc84 }).into()
}

fn running_number() -> Hsla {
    if is_light() {
        accent_color()
    } else {
        gpui::rgb(0x8cc3ff).into()
    }
}

/// A provider's swatch: its brand fill, else plain ink. A near-black brand
/// tile (Cursor, Grok, OpenCode…) takes ink too, or it would vanish on the
/// dark bar track.
pub(crate) fn provider_color(provider: &str) -> Hsla {
    provider_tile(provider)
        .map(|(top, _)| top)
        .filter(|top| top.l > 0.3)
        .unwrap_or_else(|| content(0.9))
}

impl Home {
    fn header(&self, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let hour = Local
            .timestamp_millis_opt(now)
            .single()
            .map_or(12, |time| time.hour());
        div()
            .flex()
            .items_end()
            .gap(px(16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        glass::eyebrow(date_line(now))
                            .line_height(px(14.))
                            .mb(px(6.)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "home-greeting".into())
                            .text_size(px(26.))
                            .line_height(px(30.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(greeting(hour)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "home-summary".into())
                            .mt(px(4.))
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(content(0.62))
                            .child(self.summary(now)),
                    ),
            )
            .child(
                glass::segmented()
                    .id("home-range")
                    .role(Role::TabList)
                    .aria_label("Time range")
                    .children(Range::ALL.map(|range| {
                        let selected = self.range == range;
                        glass::segment(
                            SharedString::from(format!("home-range-{}", range.id())),
                            range.label(),
                            selected,
                        )
                        .debug_selector(move || format!("home-range-{}", range.id()))
                        .role(Role::Tab)
                        .aria_selected(selected)
                        .on_click(cx.listener(move |this, _, _, cx| this.set_range(range, cx)))
                    })),
            )
            .into_any_element()
    }

    fn tile(&self, tile: Tile, cx: &mut Context<Self>) -> AnyElement {
        let Tile {
            page,
            label,
            pill,
            value,
            ink,
            line,
        } = tile;
        let id = match page {
            Page::MyTasks => "home-tile-needs-you",
            Page::Agents => "home-tile-working",
            Page::Runs => "home-tile-runs",
            Page::Inbox => "home-tile-inbox",
        };
        glass::card()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(label)
            .cursor_pointer()
            .flex_1()
            .min_w_0()
            .px(px(16.))
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .hover(|s| s.bg(glass::fill_strong()))
            .child(
                div()
                    // As tall as the pill, so tiles with and without one align.
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(content(0.62))
                            .child(label),
                    )
                    .child(div().flex_1())
                    .when_some(pill, |d, (text, tone)| {
                        d.child(glass::status(text, tone).h(px(20.)))
                    }),
            )
            .child(big(value, ink))
            .child(note(line).truncate())
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::Open(page))))
            .into_any_element()
    }

    fn tiles(&self, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let waiting = &self.data.waiting;
        let oldest = waiting.iter().map(|w| w.since_ms).min();
        let runs = self.run_counts(now);
        let working = self.data.working.len();
        let inbox = self.data.inbox.as_ref();
        let needs = self.tile(
            Tile {
                page: Page::MyTasks,
                label: "Needs you",
                pill: oldest.map(|since| {
                    (
                        format!("oldest {}", age((now as u64).saturating_sub(since))),
                        Tone::Waiting,
                    )
                }),
                value: waiting.len().to_string(),
                ink: if waiting.is_empty() {
                    content(1.)
                } else {
                    wait_number()
                },
                line: kinds_line(waiting),
            },
            cx,
        );
        let agents = self.tile(
            Tile {
                page: Page::Agents,
                label: "Working now",
                pill: (working > 0).then(|| ("live".to_owned(), Tone::Running)),
                value: working.to_string(),
                ink: if working > 0 {
                    running_number()
                } else {
                    content(1.)
                },
                line: origins_line(&self.data.working),
            },
            cx,
        );
        let automation = self.tile(
            Tile {
                page: Page::Runs,
                label: "Automation runs",
                // Green while most runs pass.
                pill: runs.pass_rate().map(|rate| {
                    let tone = if rate >= 50 { Tone::Ok } else { Tone::Bad };
                    (format!("{rate}% passed"), tone)
                }),
                value: runs.total.to_string(),
                ink: content(1.),
                line: format!("{} succeeded · {} failed", runs.succeeded, runs.failed),
            },
            cx,
        );
        let inbox_tile = self.tile(
            Tile {
                page: Page::Inbox,
                label: "Inbox",
                pill: inbox
                    .filter(|inbox| inbox.unread > 0)
                    .map(|inbox| (format!("{} unread", inbox.unread), Tone::Muted)),
                value: inbox.map_or_else(|| "—".into(), |inbox| inbox.total.to_string()),
                ink: content(1.),
                line: inbox_line(inbox),
            },
            cx,
        );
        let per_row = layout(self.width.get()).tiles_per_row;
        let mut tiles = vec![needs, agents, automation, inbox_tile].into_iter();
        let mut rows = Vec::new();
        loop {
            let row: Vec<AnyElement> = tiles.by_ref().take(per_row).collect();
            if row.is_empty() {
                break;
            }
            rows.push(div().flex().gap(px(12.)).children(row));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .children(rows)
            .into_any_element()
    }

    fn link(
        id: &'static str,
        label: &'static str,
        page: Page,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Link)
            .aria_label(label)
            .cursor_pointer()
            .ml_auto()
            .text_size(px(12.))
            .text_color(content(0.62))
            .hover(|s| s.text_color(content(1.)))
            .child(label)
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::Open(page))))
    }

    fn waiting_panel(&self, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let waiting = &self.data.waiting;
        let mut sorted: Vec<&Waiting> = waiting.iter().collect();
        sorted.sort_by_key(|w| w.since_ms);
        let more = sorted.len().saturating_sub(WAITING_ROWS);
        glass::card()
            .debug_selector(|| "home-waiting".into())
            .overflow_hidden()
            .flex_none()
            .child(panel_head("Waiting on you").child(Self::link(
                "home-open-my-tasks",
                "Open My tasks",
                Page::MyTasks,
                cx,
            )))
            .when(waiting.is_empty(), |d| {
                d.child(empty_box("home-waiting-empty").child(glass::empty_state(
                    "check-bold",
                    "Nothing is waiting on you",
                    "Approvals and questions from your agents show here.",
                )))
            })
            .children(sorted.into_iter().take(WAITING_ROWS).map(|task| {
                let approval = task.kind == Kind::Approval;
                let meta = [
                    task.source_label.clone(),
                    crate::workspace::project_name(&task.project),
                    age((now as u64).saturating_sub(task.since_ms)),
                ]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                let key = task.key.clone();
                let selector = format!("home-waiting-{key}");
                row()
                    .debug_selector(move || selector.clone())
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(content(0.07))
                            .border_1()
                            .border_color(glass::edge())
                            .child(
                                icon(
                                    if approval {
                                        "task-terminal"
                                    } else {
                                        "task-question"
                                    },
                                    13.,
                                )
                                .text_color(if approval {
                                    Tone::Waiting.ink()
                                } else {
                                    violet()
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(1.))
                                    .child(task.headline.clone()),
                            )
                            .when_some(task.allow.as_ref(), |d, allow| {
                                // The whole command, never cut: it's what Allow once allows.
                                d.child(
                                    div()
                                        .debug_selector({
                                            let key = key.clone();
                                            move || format!("home-command-{key}")
                                        })
                                        .mt(px(2.))
                                        .font_family(crate::fonts::monospace())
                                        .text_size(px(12.))
                                        .line_height(px(17.))
                                        .text_color(content(0.9))
                                        .child(allow.command.clone()),
                                )
                            })
                            .child(note(meta).truncate()),
                    )
                    .map(|d| match (&task.allow, &task.source) {
                        (Some(allow), source) => {
                            let (session, request, source) =
                                (allow.session, allow.request.clone(), source.clone());
                            d.child(
                                small_button(
                                    SharedString::from(format!("home-allow-{key}")),
                                    "Allow once",
                                    Button::Primary,
                                )
                                .debug_selector({
                                    let key = key.clone();
                                    move || format!("home-allow-{key}")
                                })
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.emit(Event::Task(my_tasks::Event::Decide {
                                            session,
                                            request: request.clone(),
                                            decision: Decision::Allow,
                                        }))
                                    },
                                )),
                            )
                            .child(
                                small_button(
                                    SharedString::from(format!("home-open-{key}")),
                                    "Open",
                                    Button::Glass,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.emit(Event::Task(my_tasks::Event::Open(source.clone())))
                                    },
                                )),
                            )
                        }
                        (None, _) => {
                            // Anything Home can't show in full opens in My tasks.
                            let label = if approval { "Open" } else { "Answer" };
                            d.child(
                                small_button(
                                    SharedString::from(format!("home-answer-{key}")),
                                    label,
                                    Button::Glass,
                                )
                                .debug_selector({
                                    let key = key.clone();
                                    move || format!("home-answer-{key}")
                                })
                                .on_click(cx.listener(
                                    move |_, _, _, cx| cx.emit(Event::OpenTask(key.clone())),
                                )),
                            )
                        }
                    })
            }))
            .when(more > 0, |d| {
                d.child(
                    row()
                        .text_size(px(12.))
                        .text_color(content(0.62))
                        .child(format!("+ {more} more")),
                )
            })
            .into_any_element()
    }

    fn working_panel(&self, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let working = &self.data.working;
        let projects = {
            let mut projects: Vec<&str> = working.iter().map(|w| w.project.as_str()).collect();
            projects.sort_unstable();
            projects.dedup();
            projects.len()
        };
        let more = working.len().saturating_sub(WORKING_ROWS);
        glass::card()
            .debug_selector(|| "home-working".into())
            .overflow_hidden()
            .child(
                panel_head("Working now")
                    .when(!working.is_empty(), |d| {
                        d.child(note(format!(
                            "{} across {}",
                            plural(working.len(), "agent", "agents"),
                            plural(projects, "project", "projects")
                        )))
                    })
                    .child(Self::link(
                        "home-all-agents",
                        "All agents",
                        Page::Agents,
                        cx,
                    )),
            )
            .when(working.is_empty(), |d| {
                d.child(empty_box("home-working-empty").child(glass::empty_state(
                    "bot",
                    "No agents are working",
                    "Agents show here while they run a turn.",
                )))
            })
            .children(working.iter().take(WORKING_ROWS).map(|agent| {
                let id = agent.id;
                let meta = [
                    agent.origin.clone(),
                    crate::workspace::project_name(&agent.project),
                ]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                let started = agent
                    .started_at
                    .map(|at| crate::shell::running_elapsed((now as u64).saturating_sub(at)));
                row()
                    .id(SharedString::from(format!("home-agent-{id}")))
                    .debug_selector(move || format!("home-agent-{id}"))
                    .role(Role::Button)
                    .aria_label(agent.title.clone())
                    .cursor_pointer()
                    .hover(|s| s.bg(glass::hover_fill()))
                    .child(
                        div()
                            .size(px(14.))
                            .flex_none()
                            .rounded(px(4.))
                            .bg(provider_color(&agent.provider)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .text_color(content(1.))
                                    .child(agent.title.clone()),
                            )
                            .child(note(meta).truncate()),
                    )
                    .child(
                        glass::status(started.unwrap_or_else(|| "running".into()), Tone::Running)
                            .h(px(20.)),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::OpenAgent(id))))
            }))
            .when(more > 0, |d| {
                d.child(
                    row()
                        .text_size(px(12.))
                        .text_color(content(0.62))
                        .child(format!("+ {more} more")),
                )
            })
            .into_any_element()
    }

    fn chart_panel(&self, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let bars = self.bars(now);
        let today = day_start(now, 0);
        let busiest = bars
            .iter()
            .map(|bar| bar.succeeded + bar.failed)
            .max()
            .unwrap_or(0);
        let wide = bars.len() <= 7;
        let title: &'static str = if wide {
            "Automation runs, last 7 days"
        } else {
            "Automation runs, last 30 days"
        };
        let ok = Tone::Ok.ink();
        let bad = Tone::Bad.ink();
        glass::card()
            .debug_selector(|| "home-chart".into())
            .overflow_hidden()
            .flex_none()
            .child(panel_head(title).child(Self::link(
                "home-open-runs",
                "Open Runs",
                Page::Runs,
                cx,
            )))
            .map(|d| {
                if busiest == 0 {
                    return d.child(empty_box("home-chart-empty").child(glass::empty_state(
                        "clock",
                        if self.runs_loaded {
                            "No automation runs finished"
                        } else {
                            "Loading runs"
                        },
                        "Each day's succeeded and failed runs show here.",
                    )));
                }
                d.child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(if wide { 6. } else { 2. }))
                        .px(px(16.))
                        .pt(px(4.))
                        .pb(px(12.))
                        .children(bars.iter().enumerate().map(|(ix, bar)| {
                            let is_today = bar.start == today;
                            let share = |n: usize| (n as f32 / busiest as f32).clamp(0., 1.);
                            let label = if is_today {
                                "Today".to_owned()
                            } else if wide || (bars.len() - 1 - ix).is_multiple_of(7) {
                                weekday(bar.start)
                            } else {
                                String::new()
                            };
                            let selector = format!("home-bar-{ix}");
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .debug_selector(move || selector.clone())
                                        .w(px(if wide { 18. } else { 6. }))
                                        .h(px(96.))
                                        .rounded(px(if wide { 5. } else { 3. }))
                                        .overflow_hidden()
                                        .bg(content(0.04))
                                        .flex()
                                        .flex_col()
                                        .justify_end()
                                        .when(is_today, |d| {
                                            d.border_1().border_color(content(0.18))
                                        })
                                        .child(
                                            div()
                                                .w_full()
                                                .h(relative(share(bar.failed)))
                                                .bg(bad.opacity(if is_today { 0.95 } else { 0.9 })),
                                        )
                                        .child(
                                            div()
                                                .w_full()
                                                .h(relative(share(bar.succeeded)))
                                                .bg(ok.opacity(if is_today { 0.85 } else { 0.75 })),
                                        ),
                                )
                                .child(
                                    note(label)
                                        .h(px(16.))
                                        .whitespace_nowrap()
                                        .when(is_today, |d| d.text_color(content(1.))),
                                )
                        })),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(14.))
                        .px(px(16.))
                        .pb(px(14.))
                        .child(legend(ok, "Succeeded"))
                        .child(legend(bad, "Failed")),
                )
            })
            .into_any_element()
    }

    fn usage_panel(&self, now: i64) -> AnyElement {
        let usage = &self.data.usage;
        let reset = usage
            .iter()
            .filter_map(|u| u.resets_at)
            .filter(|at| *at > now)
            .min();
        glass::card()
            .debug_selector(|| "home-usage".into())
            .overflow_hidden()
            .flex_none()
            .child(panel_head("Plan usage").when_some(reset, |d, at| {
                d.child(note(format!(
                    "resets in {}",
                    gespan_core::usage::duration(at - now)
                )))
            }))
            .map(|d| {
                if usage.is_empty() {
                    return d.child(empty_box("home-usage-empty").child(glass::empty_state(
                        "sliders",
                        "No plan usage yet",
                        "A provider's usage shows once it reports some.",
                    )));
                }
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .px(px(16.))
                        .pt(px(2.))
                        .pb(px(16.))
                        .children(usage.iter().map(|u| {
                            let used = u.used.clamp(0., 100.);
                            let provider = u.provider.clone();
                            div()
                                .debug_selector(move || format!("home-usage-{provider}"))
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .mb(px(6.))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .line_height(px(17.))
                                                .text_color(content(1.))
                                                .child(u.label.clone()),
                                        )
                                        .child(note(format!("{used:.0}%"))),
                                )
                                .child(
                                    div()
                                        .h(px(6.))
                                        .rounded_full()
                                        .overflow_hidden()
                                        .bg(content(0.08))
                                        .child(
                                            div()
                                                .h_full()
                                                .rounded_full()
                                                .w(relative((used / 100.) as f32))
                                                .bg(provider_color(&u.provider)),
                                        ),
                                )
                        })),
                )
            })
            .into_any_element()
    }

    fn today_panel(&self, now: i64) -> AnyElement {
        let moments = timeline(&self.runs, &self.names, &self.data, day_start(now, 0));
        glass::card()
            .debug_selector(|| "home-today".into())
            .overflow_hidden()
            .child(panel_head("Today"))
            .when(moments.is_empty(), |d| {
                d.child(empty_box("home-today-empty").child(glass::empty_state(
                    "clock",
                    "Nothing has happened yet today",
                    "Finished runs, new tasks and review requests show here.",
                )))
            })
            .children(
                moments
                    .into_iter()
                    .take(TIMELINE_ROWS)
                    .enumerate()
                    .map(|(ix, moment)| {
                        div()
                            .debug_selector(move || format!("home-moment-{ix}"))
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .px(px(16.))
                            .py(px(7.))
                            .child(
                                note(clock(moment.at))
                                    .w(px(44.))
                                    .flex_none()
                                    .font_family(crate::fonts::monospace()),
                            )
                            .child(
                                div()
                                    .w(px(14.))
                                    .flex_none()
                                    .pt(px(5.))
                                    .child(div().size(px(8.)).rounded_full().bg(moment.tone.ink())),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .line_height(px(17.))
                                    .text_color(content(0.62))
                                    .child(
                                        gpui::StyledText::new(format!(
                                            "{} {}",
                                            moment.name, moment.text
                                        ))
                                        .with_highlights(
                                            [(
                                                0..moment.name.len(),
                                                gpui::HighlightStyle {
                                                    color: Some(content(1.)),
                                                    font_weight: Some(FontWeight::MEDIUM),
                                                    ..Default::default()
                                                },
                                            )],
                                        ),
                                    ),
                            )
                    }),
            )
            .into_any_element()
    }
}

/// The padded box an empty panel's [`glass::empty_state`] sits in.
fn empty_box(id: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || id.into())
        .px(px(16.))
        .pb(px(16.))
}

fn legend(color: Hsla, label: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(div().size(px(8.)).rounded(px(2.)).bg(color))
        .child(note(label))
}

fn violet() -> Hsla {
    gpui::rgb(if is_light() { 0x6d4fd6 } else { 0xc0affc }).into()
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = gespan_core::time::now_ms() as i64;
        let Layout { columns, .. } = layout(self.width.get());
        let left = |this: &Self, cx: &mut Context<Self>| {
            [this.waiting_panel(now, cx), this.working_panel(now, cx)]
        };
        let right = |this: &Self, cx: &mut Context<Self>| {
            [
                this.chart_panel(now, cx),
                this.usage_panel(now),
                this.today_panel(now),
            ]
        };
        let body = if columns {
            div()
                .flex()
                .items_start()
                .gap(px(14.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .children(left(self, cx)),
                )
                .child(
                    div()
                        .w(relative(0.42))
                        .flex_none()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .children(right(self, cx)),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .children(left(self, cx))
                .children(right(self, cx))
        };
        // Measures the content each paint; a new width lays the page out again.
        let width = self.width.clone();
        let entity = cx.entity_id();
        let measure = canvas(
            move |bounds, _, cx| {
                let now = bounds.size.width.as_f32();
                if (now - width.get()).abs() >= 1. {
                    width.set(now);
                    cx.notify(entity);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .id("home")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .text_color(content(1.))
            .child(
                div()
                    .w_full()
                    .max_w(px(MAX_WIDTH + 64.))
                    .mx_auto()
                    .px(px(32.))
                    .pt(px(26.))
                    .pb(px(24.))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .child(measure)
                            .child(self.header(now, cx))
                            .child(div().pt(px(18.)).child(self.tiles(now, cx)))
                            .child(div().pt(px(14.)).child(body)),
                    ),
            )
    }
}

/// How Home names an Inbox item: `gespan#476` for a pull request or issue,
/// the tracker's own key (`ENG-7`) otherwise, else its title.
pub(crate) fn item_reference(item: &gespan_core::inbox::Item) -> String {
    let repo = item.repo.rsplit('/').next().unwrap_or("");
    if !repo.is_empty() && item.number > 0 {
        format!("{repo}#{}", item.number)
    } else if !item.identifier.trim().is_empty() {
        item.identifier.trim().to_owned()
    } else {
        item.title.trim().to_owned()
    }
}
