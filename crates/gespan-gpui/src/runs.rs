//! Runs: every automation run across all projects, newest first, grouped by
//! day. A run is listed the moment it is claimed (`pending`, shown as
//! Queued), and runs whose automation is gone stay listed.
use crate::{modal, theme::*};
use chrono::{Local, TimeZone};
use gespan_core::automations::{self, Automation, AutomationRun, Service};
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, Hsla, Role, SharedString, Task,
    Window, div, prelude::*, px,
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
}

impl RunRow {
    pub(crate) fn can_open(&self) -> bool {
        self.session.is_some()
    }
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

/// A chip's ink and fill, from the theme's existing colours.
fn chip_colors(status: &str) -> (Hsla, Hsla) {
    let ink = match status {
        "Succeeded" => diff_foreground(true),
        "Failed" => diff_foreground(false),
        "Queued" | "Running" => accent_color(),
        _ => return (content(0.55), content(0.08)),
    };
    let mut fill = ink;
    fill.a = 0.12;
    (ink, fill)
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
}

impl EventEmitter<Event> for Runs {}

impl Focusable for Runs {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Runs {
    pub(crate) fn new(service: Arc<Mutex<Service>>, cx: &mut Context<Self>) -> Self {
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
        };
        runs.refresh(cx);
        runs
    }

    /// Reloads the latest runs and the automations they belong to.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            let mut service = service
                .lock()
                .map_err(|_| "Automations are unavailable".to_string())?;
            let runs = service.recent_runs(LIMIT)?;
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
            self.filter = filter;
            cx.notify();
        }
    }

    pub(crate) fn filter(&self) -> &RunsFilter {
        &self.filter
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
                })
            })
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

    fn pill(id: &'static str, text: String) -> AnyElement {
        div()
            .debug_selector(move || id.into())
            .relative()
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(content(modal::BORDER))
            .child(modal::surface(11.))
            .child(
                div()
                    .relative()
                    .text_size(px(12.))
                    .text_color(content(0.75))
                    .child(text),
            )
            .into_any_element()
    }

    fn chip(
        &self,
        id: String,
        label: String,
        selected: bool,
        filter: RunsFilter,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(Role::Button)
            .aria_label(label.clone())
            .aria_selected(selected)
            .cursor_pointer()
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(content(if selected { 0.2 } else { modal::BORDER }))
            .text_size(px(12.))
            .text_color(content(if selected { 1. } else { 0.6 }))
            .when(selected, |d| d.bg(content(0.1)))
            .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
            .into_any_element()
    }

    fn filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.filter.clone();
        let mut projects = vec![self.chip(
            "runs-filter-project-all".into(),
            "All projects".into(),
            current.project.is_none(),
            RunsFilter {
                project: None,
                ..current.clone()
            },
            cx,
        )];
        for (index, project) in self.projects().into_iter().enumerate() {
            let selected = current.project.as_ref() == Some(&project);
            projects.push(self.chip(
                format!("runs-filter-project-{index}"),
                crate::workspace::project_name(&project),
                selected,
                RunsFilter {
                    project: Some(project),
                    ..current.clone()
                },
                cx,
            ));
        }
        let statuses = [
            (None, "All statuses"),
            (Some("running"), "Running"),
            (Some("succeeded"), "Succeeded"),
            (Some("failed"), "Failed"),
            (Some("cancelled"), "Cancelled"),
            (Some("skipped"), "Skipped"),
        ]
        .into_iter()
        .map(|(status, label)| {
            self.chip(
                format!("runs-filter-status-{}", status.unwrap_or("all")),
                label.into(),
                current.status == status,
                RunsFilter {
                    status,
                    ..current.clone()
                },
                cx,
            )
        })
        .collect::<Vec<_>>();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .children(projects)
            .child(div().w(px(1.)).h(px(16.)).mx(px(4.)).bg(content(0.1)))
            .children(statuses)
            .into_any_element()
    }

    fn row(&self, row: RunRow, cx: &mut Context<Self>) -> AnyElement {
        let (ink, fill) = chip_colors(row.status);
        let open = row.can_open();
        let run_id = row.id.clone();
        let open_id = format!("runs-open-{}", row.id);
        let what = match &row.item {
            Some(item) => format!("{} · {item}", row.trigger),
            None => row.trigger.clone(),
        };
        let owner = match &row.project {
            Some(project) => format!(
                "{} · {}",
                row.automation,
                crate::workspace::project_name(project)
            ),
            None => row.automation.clone(),
        };
        div()
            .id(SharedString::from(format!("runs-row-{}", row.id)))
            .debug_selector({
                let id = row.id.clone();
                move || format!("runs-row-{id}")
            })
            .role(Role::ListItem)
            .aria_label(format!("{what}, {}", row.status))
            .px(px(16.))
            .py(px(10.))
            .border_t_1()
            .border_color(content(0.06))
            .flex()
            .items_start()
            .gap(px(16.))
            .child(
                div()
                    .w(px(44.))
                    .flex_shrink_0()
                    .pt(px(1.))
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(clock(row.created_at)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(content(1.))
                            .child(what),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(owner),
                    )
                    .when_some(row.error.clone(), |d, error| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(content(0.45))
                                .line_clamp(2)
                                .text_ellipsis()
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(20.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(ink)
                    .bg(fill)
                    .child(row.status),
            )
            .child(
                div()
                    .id(SharedString::from(open_id.clone()))
                    .debug_selector(move || open_id.clone())
                    .role(Role::Button)
                    .aria_label(if open {
                        "Open thread"
                    } else {
                        "Open thread, unavailable: this run has no thread"
                    })
                    .flex_shrink_0()
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(content(if open { 0.8 } else { 0.3 }))
                    .when(open, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                    })
                    .child("Open thread")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_thread(&run_id, cx))),
            )
            .into_any_element()
    }

    fn panel(&self, rows: Vec<RunRow>, today_start: i64, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div()
            .debug_selector(|| "runs-panel".into())
            .relative()
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(content(modal::BORDER))
            .overflow_hidden()
            .child(modal::surface(9.));
        if rows.is_empty() {
            let message = if self.loading {
                "Loading runs…"
            } else if let Some(error) = &self.error {
                return panel
                    .child(
                        div()
                            .relative()
                            .px(px(16.))
                            .py(px(48.))
                            .text_center()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(format!("Could not load runs: {error}")),
                    )
                    .into_any_element();
            } else if self.runs.is_empty() {
                "No automation runs yet. Runs show here the moment an automation starts."
            } else {
                "No runs match these filters."
            };
            return panel
                .child(
                    div()
                        .relative()
                        .px(px(16.))
                        .py(px(48.))
                        .text_center()
                        .text_size(px(12.))
                        .text_color(content(0.45))
                        .child(message),
                )
                .into_any_element();
        }
        let mut day: Option<String> = None;
        for row in rows {
            let label = day_label(row.created_at, today_start);
            if day.as_ref() != Some(&label) {
                panel = panel.child(
                    div()
                        .relative()
                        .h(px(30.))
                        .px(px(16.))
                        .flex()
                        .items_end()
                        .pb(px(4.))
                        .when(day.is_some(), |d| {
                            d.border_t_1().border_color(content(0.06))
                        })
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(content(0.5))
                        .child(label.clone()),
                );
                day = Some(label);
            }
            panel = panel.child(div().relative().child(self.row(row, cx)));
        }
        panel.into_any_element()
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
                    .max_w(px(1024.))
                    .mx_auto()
                    .px(px(32.))
                    .pt(px(20.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Runs"),
                            )
                            .child(Self::pill(
                                "runs-count-running",
                                format!("{running} running"),
                            ))
                            .child(Self::pill("runs-count-today", format!("{today} today")))
                            .child(Self::pill(
                                "runs-count-failed",
                                format!("{failed} failed today"),
                            )),
                    )
                    // Rows from the last good load stay; the failure is said.
                    .when_some(
                        self.error.clone().filter(|_| !self.runs.is_empty()),
                        |d, error| {
                            d.child(
                                div()
                                    .debug_selector(|| "runs-refresh-error".into())
                                    .mt(px(-8.))
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(content(0.45))
                                    .child(format!("Could not refresh runs: {error}")),
                            )
                        },
                    )
                    .child(self.filters(cx))
                    .child(self.panel(rows, today_start, cx)),
            )
    }
}
