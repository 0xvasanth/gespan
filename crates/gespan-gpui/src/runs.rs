//! Runs: every automation run across all projects, newest first, grouped by
//! day. A run is listed the moment it is claimed (`pending`, shown as
//! Queued), and runs whose automation is gone stay listed.
use crate::{
    glass,
    input::{self, Input},
    theme::*,
};
use chrono::{Local, TimeZone};
use gespan_core::automations::{self, Automation, AutomationRun, Service};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(test)]
mod tests;

/// Enough for every automation's kept history to be paged in later.
pub(crate) const LIMIT: usize = 200;
/// How often the list reloads while it is on screen.
const REFRESH: Duration = Duration::from_secs(30);
const DAY_MS: i64 = 86_400_000;

/// Which runs are shown. The default shows every run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunsFilter {
    /// A project path, compared with the automation's own.
    pub project: Option<String>,
    /// A run status; `running` also keeps queued (`pending`) runs.
    pub status: Option<&'static str>,
    pub automation: Option<String>,
    /// The name of `automation`, shown on its chip until the first load
    /// resolves it.
    pub automation_name: Option<String>,
}

pub(crate) enum Event {
    /// The run's persisted session id, resolved by the shell the same way
    /// the Automations run history opens a run.
    OpenSession(String),
}

/// One listed run, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunRow {
    pub id: String,
    pub created_at: i64,
    /// What fired it: an event label, "Scheduled" or "Test run".
    pub trigger: String,
    /// The item an event was about, or its key when the title is unknown.
    pub item: Option<String>,
    pub automation: String,
    /// The automation's project, unknown once the automation is gone.
    pub project: Option<String>,
    pub status: &'static str,
    pub error: Option<String>,
    pub session: Option<String>,
    /// How long a finished run took, such as `6m 02s`.
    pub duration: Option<String>,
}

impl RunRow {
    pub(crate) fn can_open(&self) -> bool {
        self.session.is_some()
    }

    /// Whether the search text appears, ignoring case, in what the row
    /// shows: its trigger, item, automation, project or error.
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let project = self.project.as_deref().map(crate::workspace::project_name);
        [
            Some(self.trigger.as_str()),
            self.item.as_deref(),
            Some(self.automation.as_str()),
            project.as_deref(),
            self.error.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|text| text.to_lowercase().contains(&query))
    }
}

/// How long a run took from start to finish: `45s`, `6m 02s`, `1h 03m`.
/// Runs that never started or have not finished have none.
pub(crate) fn duration(run: &AutomationRun) -> Option<String> {
    let ms = run.completed_at? - run.started_at?;
    if ms < 0 {
        return None;
    }
    let secs = ms / 1000;
    Some(match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    })
}

/// The heading over one day's card: `1 run` or `N runs`.
pub(crate) fn run_count(count: usize) -> String {
    if count == 1 {
        "1 run".to_string()
    } else {
        format!("{count} runs")
    }
}

/// Rows grouped under their day label, keeping their order. Rows arrive
/// newest first, so each day appears once.
pub(crate) fn group_by_day(rows: Vec<RunRow>, today_start: i64) -> Vec<(String, Vec<RunRow>)> {
    let mut days: Vec<(String, Vec<RunRow>)> = Vec::new();
    for row in rows {
        let label = day_label(row.created_at, today_start);
        match days.last_mut() {
            Some((day, rows)) if *day == label => rows.push(row),
            _ => days.push((label, vec![row])),
        }
    }
    days
}

/// The chip a run's recorded status shows.
pub(crate) fn status_chip(run: &AutomationRun) -> &'static str {
    match run.status.as_str() {
        "pending" => "Queued",
        "running" => "Running",
        "succeeded" => "Succeeded",
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        "skipped" => "Skipped",
        _ => "Queued",
    }
}

/// Running or queued. An unknown status shows as Queued, so it counts too.
fn live(status: &str) -> bool {
    !matches!(status, "succeeded" | "failed" | "cancelled" | "skipped")
}

/// The header counts: (running, including queued; started today; failed
/// today). `today_start` is local midnight in milliseconds.
pub(crate) fn counts(runs: &[AutomationRun], today_start: i64) -> (usize, usize, usize) {
    let running = runs.iter().filter(|run| live(&run.status)).count();
    let today = runs.iter().filter(|run| run.created_at >= today_start);
    let failed = today.clone().filter(|run| run.status == "failed").count();
    (running, today.count(), failed)
}

/// Local midnight of the day holding `at`.
fn local_day_start(at: i64) -> i64 {
    let Some(time) = Local.timestamp_millis_opt(at).single() else {
        return at - at.rem_euclid(DAY_MS);
    };
    time.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
        .map_or(at - at.rem_euclid(DAY_MS), |midnight| {
            midnight.timestamp_millis()
        })
}

/// The day heading a run sits under.
fn day_label(at: i64, today_start: i64) -> String {
    if at >= today_start {
        return "Today".into();
    }
    if at >= local_day_start(today_start - 1) {
        return "Yesterday".into();
    }
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%A, %-d %B").to_string())
        .unwrap_or_else(|| "Earlier".into())
}

fn clock(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

fn trigger_label(run: &AutomationRun) -> String {
    match run.trigger.as_str() {
        "manual" => "Test run".into(),
        "event" => automations::event_label(
            run.event_kind.as_deref().unwrap_or(""),
            run.event.as_deref().unwrap_or(""),
        ),
        _ => "Scheduled".into(),
    }
}

struct Owner {
    name: String,
    cwd: String,
}

pub(crate) struct Runs {
    service: Arc<Mutex<Service>>,
    runs: Vec<AutomationRun>,
    owners: HashMap<String, Owner>,
    filter: RunsFilter,
    loading: bool,
    error: Option<String>,
    focus: FocusHandle,
    generation: u64,
    ticker: Option<Task<()>>,
    search: Entity<Input>,
    query: String,
    _search_changed: Subscription,
}

impl EventEmitter<Event> for Runs {}

impl Focusable for Runs {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Runs {
    pub(crate) fn new(service: Arc<Mutex<Service>>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| Input::field("Search runs", "Search runs", content(0.35), cx));
        let search_changed = cx.subscribe(&search, |this, search, _: &input::Changed, cx| {
            let query = search.read(cx).content.to_string();
            this.set_query(query, cx);
        });
        let mut runs = Self {
            service,
            runs: Vec::new(),
            owners: HashMap::new(),
            filter: RunsFilter::default(),
            loading: true,
            error: None,
            focus: cx.focus_handle(),
            generation: 0,
            ticker: None,
            search,
            query: String::new(),
            _search_changed: search_changed,
        };
        runs.refresh(cx);
        runs
    }

    /// Reloads the latest runs and the automations they belong to.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let service = self.service.clone();
        // One automation's own kept history reaches back past the latest 200
        // runs of everything.
        let automation = self.filter.automation.clone();
        let work = cx.background_executor().spawn(async move {
            let mut service = service
                .lock()
                .map_err(|_| "Automations are unavailable".to_string())?;
            let runs = match &automation {
                Some(id) => service.runs(id)?,
                None => service.recent_runs(LIMIT)?,
            };
            let automations = service.list()?;
            Ok::<_, String>((runs, automations))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                // A later refresh supersedes this one.
                if this.generation != generation {
                    return;
                }
                this.loaded(result, cx);
            });
        })
        .detach();
    }

    /// A refresh finished. A failure keeps the rows it had and says so.
    pub(crate) fn loaded(
        &mut self,
        result: Result<(Vec<AutomationRun>, Vec<Automation>), String>,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        match result {
            Ok((runs, automations)) => self.apply(runs, automations, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn apply(
        &mut self,
        runs: Vec<AutomationRun>,
        automations: Vec<Automation>,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        self.error = None;
        self.runs = runs;
        self.owners = automations
            .into_iter()
            .map(|automation| {
                (
                    automation.id,
                    Owner {
                        name: automation.name,
                        cwd: automation.cwd,
                    },
                )
            })
            .collect();
        cx.notify();
    }

    pub(crate) fn set_filter(&mut self, filter: RunsFilter, cx: &mut Context<Self>) {
        if self.filter != filter {
            let reload = self.filter.automation != filter.automation;
            self.filter = filter;
            if reload {
                self.refresh(cx);
            }
            cx.notify();
        }
    }

    pub(crate) fn filter(&self) -> &RunsFilter {
        &self.filter
    }

    /// Narrows the list to runs whose words contain `query`.
    pub(crate) fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.query != query {
            self.query = query;
            cx.notify();
        }
    }

    /// The view came on screen: reload now and every 30 seconds after.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        if self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(REFRESH).await;
                    if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                        break;
                    }
                }
            }));
        }
    }

    /// The view left the screen: stop reloading.
    pub(crate) fn hidden(&mut self) {
        self.ticker = None;
    }

    /// Every project that has runs, ordered by name.
    pub(crate) fn projects(&self) -> Vec<String> {
        // Keyed like the filter, so one folder spelled two ways is one chip.
        let mut keys = std::collections::HashSet::new();
        let mut projects: Vec<String> = Vec::new();
        for run in &self.runs {
            if let Some(owner) = self.owners.get(&run.automation_id)
                && keys.insert(gespan_core::files::path_key(&owner.cwd))
            {
                projects.push(owner.cwd.clone());
            }
        }
        projects.sort_by_key(|path| crate::workspace::project_name(path).to_lowercase());
        projects
    }

    /// The runs the filter keeps, newest first.
    pub(crate) fn rows(&self) -> Vec<RunRow> {
        self.runs
            .iter()
            .filter_map(|run| {
                let owner = self.owners.get(&run.automation_id);
                let filter = &self.filter;
                if let Some(project) = &filter.project
                    && owner.is_none_or(|owner| {
                        gespan_core::files::path_key(&owner.cwd)
                            != gespan_core::files::path_key(project)
                    })
                {
                    return None;
                }
                if filter
                    .automation
                    .as_ref()
                    .is_some_and(|id| *id != run.automation_id)
                {
                    return None;
                }
                if let Some(status) = filter.status {
                    let keep = if status == "running" {
                        live(&run.status)
                    } else {
                        run.status == status
                    };
                    if !keep {
                        return None;
                    }
                }
                Some(RunRow {
                    id: run.id.clone(),
                    created_at: run.created_at,
                    trigger: trigger_label(run),
                    item: run
                        .event_title
                        .clone()
                        .or_else(|| run.event_key.clone())
                        .filter(|item| !item.trim().is_empty()),
                    automation: owner
                        .map_or_else(|| "Deleted automation".into(), |owner| owner.name.clone()),
                    project: owner.map(|owner| owner.cwd.clone()),
                    status: status_chip(run),
                    error: run.error.clone().filter(|error| !error.trim().is_empty()),
                    session: run.session_id.clone(),
                    duration: duration(run),
                })
            })
            .filter(|row| row.matches(&self.query))
            .collect()
    }

    /// Opens the run's thread; a run without a session has none to open.
    pub(crate) fn open_thread(&mut self, run_id: &str, cx: &mut Context<Self>) {
        if let Some(session) = self
            .runs
            .iter()
            .find(|run| run.id == run_id)
            .and_then(|run| run.session_id.clone())
        {
            cx.emit(Event::OpenSession(session));
        }
    }

    /// A filter option: one segment of the project or status control. A
    /// status segment leads with its tone's dot; `aria` names it in full
    /// where the word alone would be vague.
    #[allow(clippy::too_many_arguments)]
    fn chip(
        &self,
        id: String,
        label: String,
        aria: String,
        dot: Option<glass::Tone>,
        selected: bool,
        filter: RunsFilter,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = id.clone();
        let id = SharedString::from(id);
        match dot {
            Some(tone) => glass::dot_segment(id, label, selected, tone),
            None => glass::segment(id, label, selected),
        }
        .debug_selector(move || selector.clone())
        .role(Role::Button)
        .aria_label(aria)
        .aria_selected(selected)
        .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
        .into_any_element()
    }

    /// What the automation chip names: the automation's own name, the one
    /// the filter carried while the first load is in flight, and only once
    /// loaded without it, "Deleted automation".
    pub(crate) fn automation_label(&self) -> Option<String> {
        let id = self.filter.automation.as_ref()?;
        Some(match self.owners.get(id) {
            Some(owner) => owner.name.clone(),
            None if self.loading => self.filter.automation_name.clone().unwrap_or_default(),
            None => "Deleted automation".to_string(),
        })
    }

    /// One glass toolbar: the project control, the status control, the
    /// search field and, when the page was opened from an automation, the
    /// chip that clears it.
    fn filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.filter.clone();
        let mut projects = vec![self.chip(
            "runs-filter-project-all".into(),
            "All projects".into(),
            "All projects".into(),
            None,
            current.project.is_none(),
            RunsFilter {
                project: None,
                ..current.clone()
            },
            cx,
        )];
        for (index, project) in self.projects().into_iter().enumerate() {
            let selected = current.project.as_ref() == Some(&project);
            let name = crate::workspace::project_name(&project);
            projects.push(self.chip(
                format!("runs-filter-project-{index}"),
                name.clone(),
                name,
                None,
                selected,
                RunsFilter {
                    project: Some(project),
                    ..current.clone()
                },
                cx,
            ));
        }
        let statuses = [
            (None, "All", None),
            (Some("running"), "Running", Some(glass::Tone::Running)),
            (Some("succeeded"), "Succeeded", Some(glass::Tone::Ok)),
            (Some("failed"), "Failed", Some(glass::Tone::Bad)),
            (Some("cancelled"), "Cancelled", None),
            (Some("skipped"), "Skipped", None),
        ]
        .into_iter()
        .map(|(status, label, dot)| {
            self.chip(
                format!("runs-filter-status-{}", status.unwrap_or("all")),
                label.into(),
                if status.is_none() {
                    "All statuses".into()
                } else {
                    label.into()
                },
                dot,
                current.status == status,
                RunsFilter {
                    status,
                    ..current.clone()
                },
                cx,
            )
        })
        .collect::<Vec<_>>();
        // A filter set from an automation's page shows what it keeps and the
        // way back to every automation.
        let automation = self.automation_label().map(|name| {
            glass::segmented().child(self.chip(
                "runs-filter-automation".into(),
                format!("{name}  \u{d7}"),
                format!("Showing {name} only. Show every automation"),
                None,
                true,
                RunsFilter {
                    automation: None,
                    ..current.clone()
                },
                cx,
            ))
        });
        glass::card()
            .rounded(px(12.))
            .p(px(8.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            // One automation lives in one project, so while its chip shows
            // the project control would change nothing.
            .when(automation.is_none(), |d| {
                d.child(glass::segmented().flex_wrap().children(projects))
            })
            .children(automation)
            .child(glass::segmented().flex_wrap().children(statuses))
            .child(
                glass::field()
                    .debug_selector(|| "runs-search".into())
                    .ml_auto()
                    // Shrinks before it wraps under the status control.
                    .flex_1()
                    .min_w(px(160.))
                    .max_w(px(220.))
                    .child(icon("search", 14.).text_color(content(0.45)))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .into_any_element()
    }

    fn row(&self, row: RunRow, first: bool, last: bool, cx: &mut Context<Self>) -> AnyElement {
        const GROUP: &str = "runs-row";
        let tone = glass::Tone::of_status(row.status);
        let open = row.can_open();
        let run_id = row.id.clone();
        let open_id = format!("runs-open-{}", row.id);
        let what = match &row.item {
            Some(item) => format!("{} · {item}", row.trigger),
            None => row.trigger.clone(),
        };
        let owner = [
            Some(row.automation.clone()),
            row.project.as_deref().map(crate::workspace::project_name),
            row.duration.clone(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        // The trigger reads quieter than the item it fired for.
        let title = div()
            .min_w_0()
            .flex()
            .gap(px(4.))
            .text_size(px(13.))
            .line_height(px(18.))
            .map(|d| match &row.item {
                Some(item) => d
                    .child(
                        div()
                            .flex_none()
                            .text_color(content(0.64))
                            .child(format!("{} ·", row.trigger)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(content(1.))
                            .child(item.clone()),
                    ),
                None => d.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(content(1.))
                        .child(row.trigger.clone()),
                ),
            });
        let button = if open {
            glass::button(
                SharedString::from(open_id.clone()),
                "Open thread",
                glass::Button::Glass,
            )
            .h(px(28.))
            .px(px(11.))
            .text_size(px(12.))
            .child(icon("external-link", 12.).text_color(content(0.9)))
            .aria_label("Open thread")
        } else {
            div()
                .id(SharedString::from(open_id.clone()))
                .h(px(28.))
                .px(px(11.))
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(9.))
                .text_size(px(12.))
                .whitespace_nowrap()
                .text_color(content(0.3))
                .child("Open thread")
                .aria_label("Open thread, unavailable: this run has no thread")
        };
        div()
            .id(SharedString::from(format!("runs-row-{}", row.id)))
            .group(GROUP)
            .debug_selector({
                let id = row.id.clone();
                move || format!("runs-row-{id}")
            })
            .role(Role::ListItem)
            .aria_label(format!("{what}, {}", row.status))
            .px(px(16.))
            .py(px(11.))
            .when(!first, |d| d.border_t_1().border_color(glass::hairline()))
            // The card clips square, so the end rows round their own hover
            // fill to the card's inner corner.
            .when(first, |d| d.rounded_t(px(13.)))
            .when(last, |d| d.rounded_b(px(13.)))
            .hover(|s| s.bg(glass::hover_fill()))
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .w(px(52.))
                    .flex_shrink_0()
                    .font_family(crate::fonts::monospace())
                    .text_size(px(12.))
                    .text_color(content(0.64))
                    .child(clock(row.created_at)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(title)
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child(owner),
                    )
                    .when_some(row.error.clone(), |d, error| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(glass::Tone::Bad.ink().opacity(0.85))
                                .line_clamp(2)
                                .text_ellipsis()
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .flex()
                    .child(glass::status(row.status, tone)),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    // Shown on the row under the pointer, like the mock.
                    .opacity(0.)
                    .group_hover(GROUP, |s| s.opacity(1.))
                    .child(
                        button
                            .debug_selector(move || open_id.clone())
                            .role(Role::Button)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open_thread(&run_id, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The empty state's sentence and hint, or `None` when rows show.
    pub(crate) fn empty_text(&self, rows_shown: bool) -> Option<(String, String)> {
        if rows_shown {
            return None;
        }
        Some(if self.loading {
            ("Loading runs…".to_string(), String::new())
        } else if let Some(error) = &self.error {
            (format!("Could not load runs: {error}"), String::new())
        } else if self.runs.is_empty() {
            (
                "No automation runs yet.".to_string(),
                "Runs show here the moment an automation starts.".to_string(),
            )
        } else if !self.query.trim().is_empty() {
            (
                format!("No runs match \u{201c}{}\u{201d}.", self.query.trim()),
                "Try another word, or clear the search.".to_string(),
            )
        } else {
            (
                "No runs match these filters.".to_string(),
                "Choose All projects or All statuses to see more.".to_string(),
            )
        })
    }

    /// Runs grouped by day, one glass card per day under its label.
    fn panel(&self, rows: Vec<RunRow>, today_start: i64, cx: &mut Context<Self>) -> AnyElement {
        let column = div()
            .debug_selector(|| "runs-panel".into())
            .w_full()
            .flex()
            .flex_col()
            .gap(px(20.));
        if let Some((title, hint)) = self.empty_text(!rows.is_empty()) {
            return column
                .child(
                    glass::card()
                        .debug_selector(|| "runs-empty".into())
                        .py(px(56.))
                        .child(glass::empty_state("clock", title, hint)),
                )
                .into_any_element();
        }
        column
            .children(group_by_day(rows, today_start).into_iter().enumerate().map(
                |(index, (label, rows))| {
                    let count = rows.len();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .px(px(4.))
                                .flex()
                                .items_baseline()
                                .gap(px(10.))
                                .child(glass::eyebrow(label).text_color(content(0.92)))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(content(0.45))
                                        .child(run_count(count)),
                                ),
                        )
                        .child(
                            glass::card()
                                .debug_selector(move || format!("runs-day-{index}"))
                                .overflow_hidden()
                                .children(
                                    rows.into_iter()
                                        .enumerate()
                                        .map(|(i, row)| self.row(row, i == 0, i + 1 == count, cx)),
                                ),
                        )
                },
            ))
            .into_any_element()
    }
}

impl Render for Runs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today_start = local_day_start(gespan_core::time::now_ms() as i64);
        let (running, today, failed) = counts(&self.runs, today_start);
        let rows = self.rows();
        div()
            .id("runs")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .text_color(content(1.))
            .child(
                div()
                    .w_full()
                    .max_w(px(1040.))
                    .mx_auto()
                    .px(px(40.))
                    .pt(px(26.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_end()
                            .gap(px(16.))
                            .child(
                                glass::page_header(
                                    "Runs",
                                    "Every automation run across your projects, newest first.",
                                )
                                .flex_1()
                                .min_w(px(260.)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(glass::stat(
                                        "runs-count-running",
                                        running.to_string(),
                                        "Running",
                                        glass::Tone::Running.ink(),
                                    ))
                                    .child(glass::stat(
                                        "runs-count-today",
                                        today.to_string(),
                                        "Today",
                                        content(1.),
                                    ))
                                    .child(glass::stat(
                                        "runs-count-failed",
                                        failed.to_string(),
                                        "Failed today",
                                        // Green while nothing failed today.
                                        if failed > 0 {
                                            glass::Tone::Bad.ink()
                                        } else {
                                            glass::Tone::Ok.ink()
                                        },
                                    )),
                            ),
                    )
                    // Rows from the last good load stay; the failure is said.
                    .when_some(
                        self.error.clone().filter(|_| !self.runs.is_empty()),
                        |d, error| {
                            d.child(
                                div()
                                    .debug_selector(|| "runs-refresh-error".into())
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(glass::Tone::Bad.ink())
                                    .child(format!("Could not refresh runs: {error}")),
                            )
                        },
                    )
                    .child(self.filters(cx))
                    .child(self.panel(rows, today_start, cx)),
            )
    }
}
