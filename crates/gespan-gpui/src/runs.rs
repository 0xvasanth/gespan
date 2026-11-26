//! All runs: every automation run across all projects, newest first,
//! grouped by day, as the Automations page's default pane. Runs still going
//! sit in "Running now" on top, and an automation whose latest run failed is
//! pinned above them until it succeeds again. A run is listed the moment it
//! is claimed (`pending`, shown as Queued), and runs whose automation is
//! gone stay listed.
use crate::{
    glass,
    input::{self, Input},
    popover::{Anchor, AnchorSlot, Menu, MenuAction, MenuEvent},
    theme::*,
};
use chrono::{Local, TimeZone};
use gespan_core::automations::{Automation, AutomationRun, Service};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) mod row;
#[cfg(test)]
mod tests;

pub(crate) use row::{Look, RunRow, clock, item_label, run_row, section};

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
    /// A pinned failure's "Edit automation".
    EditAutomation(String),
    /// A pinned failure's "Run now".
    RunNow(String),
}

/// Local midnight of the day holding `at`.
pub(crate) fn local_day_start(at: i64) -> i64 {
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
pub(crate) fn day_label(at: i64, today_start: i64) -> String {
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

/// Running or queued. An unknown status shows as Queued, so it counts too.
pub(crate) fn live(status: &str) -> bool {
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

/// The runs that keep an automation failing: per automation, its newest
/// finished run when that run failed, newest first. A later success, a
/// skip or a cancel clears it; a run still going does not.
pub(crate) fn failing(runs: &[AutomationRun]) -> Vec<&AutomationRun> {
    let mut finished: Vec<&AutomationRun> = runs.iter().filter(|run| !live(&run.status)).collect();
    finished.sort_by_key(|run| std::cmp::Reverse(run.created_at));
    let mut seen = HashSet::new();
    finished
        .into_iter()
        .filter(|run| seen.insert(run.automation_id.as_str()))
        .filter(|run| run.status == "failed")
        .collect()
}

/// What the rail's bolt shows: how many runs are going (queued included)
/// and how many automations are failing.
pub(crate) fn rail_counts(runs: &[AutomationRun]) -> (usize, usize) {
    (
        runs.iter().filter(|run| live(&run.status)).count(),
        failing(runs).len(),
    )
}

struct Owner {
    name: String,
    cwd: String,
}

pub(crate) struct Runs {
    /// The run list's selectable text.
    pub(crate) text: crate::selectable::SelectionScope,
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
    /// The open "More" status menu: Cancelled and Skipped.
    more_menu: Option<(Entity<Menu>, Subscription)>,
    more_anchor: AnchorSlot,
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
            text: crate::selectable::SelectionScope::new(cx),
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
            more_menu: None,
            more_anchor: AnchorSlot::default(),
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

    /// The statuses the "More" segment holds, in its menu's order.
    const MORE: [(&'static str, &'static str); 2] =
        [("cancelled", "Cancelled"), ("skipped", "Skipped")];

    /// What the "More" segment reads: the status it holds once one is
    /// chosen, so the filter in force stays visible.
    pub(crate) fn more_label(&self) -> &'static str {
        Self::MORE
            .iter()
            .find(|(status, _)| self.filter.status == Some(*status))
            .map_or("More", |(_, label)| label)
    }

    /// A pick from the "More" menu: filter to that status.
    pub(crate) fn pick_more(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some((status, _)) = Self::MORE.iter().find(|(status, _)| *status == id) {
            self.set_filter(
                RunsFilter {
                    status: Some(status),
                    ..self.filter.clone()
                },
                cx,
            );
        }
    }

    fn open_more(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = Self::MORE
            .iter()
            .map(|(status, label)| {
                MenuAction::new(*status, *label)
                    .checked(self.filter.status == Some(*status))
                    .into()
            })
            .collect();
        let menu = cx.new(|cx| Menu::new("More statuses", items, window, cx));
        let events = cx.subscribe_in(&menu, window, |this, _, event: &MenuEvent, _, cx| {
            this.more_menu = None;
            if let MenuEvent::Picked(id, _) = event {
                this.pick_more(id, cx);
            }
            cx.notify();
        });
        self.more_menu = Some((menu, events));
        cx.notify();
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
        let now = gespan_core::time::now_ms() as i64;
        self.runs
            .iter()
            .filter_map(|run| self.row_of(run, now))
            .collect()
    }

    /// `run` as a row, or `None` when the filter or search leaves it out.
    fn row_of(&self, run: &AutomationRun, now: i64) -> Option<RunRow> {
        let owner = self.owners.get(&run.automation_id);
        let filter = &self.filter;
        if let Some(project) = &filter.project
            && owner.is_none_or(|owner| {
                gespan_core::files::path_key(&owner.cwd) != gespan_core::files::path_key(project)
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
        let owner = owner.map(|owner| (owner.name.as_str(), owner.cwd.as_str()));
        Some(RunRow::new(run, owner, now)).filter(|row| row.matches(&self.query))
    }

    /// Whether the "More" status menu is open, so Escape closes it first.
    pub(crate) fn menu_open(&self) -> bool {
        self.more_menu.is_some()
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
        let mut statuses = [
            (None, "All", None),
            (Some("running"), "Running", Some(glass::Tone::Running)),
            (Some("succeeded"), "Succeeded", Some(glass::Tone::Ok)),
            (Some("failed"), "Failed", Some(glass::Tone::Bad)),
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
        statuses.push({
            // Cancelled and Skipped sit behind one segment.
            let label = self.more_label();
            let selected = label != "More";
            glass::segment("runs-filter-status-more", label, selected)
                .relative()
                .child(self.more_anchor.probe())
                .debug_selector(|| "runs-filter-status-more".into())
                .role(Role::Button)
                .aria_label(if selected {
                    format!("{label}. More statuses")
                } else {
                    "More statuses".to_string()
                })
                .aria_selected(selected)
                .on_click(cx.listener(|this, _, window, cx| this.open_more(window, cx)))
                .into_any_element()
        });
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
            .child(glass::segmented().flex_wrap().children(statuses))
            // One automation lives in one project, so while its chip shows
            // the project control would change nothing.
            .when(automation.is_none(), |d| {
                d.child(glass::segmented().flex_wrap().children(projects))
            })
            .children(automation)
            .child(
                glass::field()
                    .debug_selector(|| "runs-search".into())
                    .ml_auto()
                    .w(px(220.))
                    .child(icon("search", 14.).text_color(content(0.45)))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .into_any_element()
    }

    /// The runs pinned as failing, as rows, under the current filter. Runs
    /// of a deleted automation are not pinned: there is nothing to edit or
    /// run again.
    pub(crate) fn pinned(&self) -> Vec<RunRow> {
        if !matches!(self.filter.status, None | Some("failed")) {
            return Vec::new();
        }
        let now = gespan_core::time::now_ms() as i64;
        failing(&self.runs)
            .into_iter()
            .filter(|run| self.owners.contains_key(&run.automation_id))
            .filter_map(|run| self.row_of(run, now))
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

    fn row(&self, row: &RunRow, first: bool, last: bool, cx: &mut Context<Self>) -> AnyElement {
        let run_id = row.id.clone();
        // One automation's runs all share its name and project.
        let one = self.filter.automation.is_some();
        run_row(
            row,
            &self.text,
            Look {
                first,
                last,
                automation: !one,
                project: !one,
            },
            cx.listener(move |this, _, _, cx| this.open_thread(&run_id, cx)),
        )
        .into_any_element()
    }

    /// A failing automation, pinned: its last run's error and the ways to
    /// fix it.
    fn failure(&self, row: RunRow, today_start: i64, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id.clone();
        let automation = row.automation_id.clone();
        let edit = automation.clone();
        let when = format!(
            "{} {}",
            day_label(row.created_at, today_start),
            clock(row.created_at)
        );
        let ran = [
            row.duration
                .as_ref()
                .map(|duration| format!("ran {duration}")),
            row.project.as_deref().map(crate::workspace::project_name),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let open = row.can_open();
        let bad = glass::Tone::Bad.ink();
        glass::card()
            .id(SharedString::from(format!("runs-failing-{id}")))
            .debug_selector({
                let id = id.clone();
                move || format!("runs-failing-{id}")
            })
            .role(Role::Alert)
            .aria_label(format!("{} failed {when}", row.automation))
            .border_color(bad.opacity(0.30))
            .bg(bad.opacity(0.05))
            .px(px(16.))
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(glass::status("Failed", glass::Tone::Bad))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(self.text.text(format!("{} · {when}", row.automation))),
                    )
                    .when(!ran.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(content(0.55))
                                .child(self.text.text(ran)),
                        )
                    })
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .gap(px(6.))
                            .child(
                                glass::button(
                                    SharedString::from(format!("runs-failing-edit-{id}")),
                                    "Edit automation",
                                    glass::Button::Ghost,
                                )
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("runs-failing-edit-{id}")
                                })
                                .role(Role::Button)
                                .h(px(28.))
                                .on_click(cx.listener(
                                    move |_, _, _, cx| cx.emit(Event::EditAutomation(edit.clone())),
                                )),
                            )
                            .when(open, |d| {
                                let run_id = id.clone();
                                d.child(
                                    glass::button(
                                        SharedString::from(format!("runs-failing-open-{id}")),
                                        "Open thread",
                                        glass::Button::Glass,
                                    )
                                    .debug_selector({
                                        let id = id.clone();
                                        move || format!("runs-failing-open-{id}")
                                    })
                                    .role(Role::Button)
                                    .h(px(28.))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| this.open_thread(&run_id, cx),
                                    )),
                                )
                            })
                            .child(
                                glass::button(
                                    SharedString::from(format!("runs-failing-run-{id}")),
                                    "Run now",
                                    glass::Button::Primary,
                                )
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("runs-failing-run-{id}")
                                })
                                .role(Role::Button)
                                .h(px(28.))
                                .on_click(cx.listener(
                                    move |_, _, _, cx| cx.emit(Event::RunNow(automation.clone())),
                                )),
                            ),
                    ),
            )
            .when_some(row.error.clone(), |d, error| {
                d.child(
                    div()
                        .rounded(px(10.))
                        .bg(black(0.25))
                        .border_1()
                        .border_color(glass::hairline())
                        .px(px(12.))
                        .py(px(8.))
                        .font_family(crate::fonts::monospace())
                        .text_size(px(11.5))
                        .line_height(px(18.))
                        .text_color(bad)
                        .child(self.text.text(error)),
                )
            })
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

    /// "Running now" on top, then the rest by day, one glass card each.
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
        let (running, rest): (Vec<RunRow>, Vec<RunRow>) = rows.into_iter().partition(RunRow::live);
        let cards = |rows: &[RunRow], cx: &mut Context<Self>| {
            let count = rows.len();
            rows.iter()
                .enumerate()
                .map(|(i, row)| self.row(row, i == 0, i + 1 == count, cx))
                .collect::<Vec<_>>()
        };
        let running = (!running.is_empty()).then(|| {
            section(
                "runs-running".into(),
                "Running now",
                glass::Tone::Running.ink(),
                cards(&running, cx),
            )
        });
        let days = group_by_day(rest, today_start)
            .into_iter()
            .enumerate()
            .map(|(index, (label, rows))| {
                section(
                    format!("runs-day-{index}"),
                    label,
                    content(0.92),
                    cards(&rows, cx),
                )
            })
            .collect::<Vec<_>>();
        column.children(running).children(days).into_any_element()
    }
}

impl Render for Runs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today_start = local_day_start(gespan_core::time::now_ms() as i64);
        let (running, today, failed) = counts(&self.runs, today_start);
        let rows = self.rows();
        let pinned = self
            .pinned()
            .into_iter()
            .map(|row| self.failure(row, today_start, cx))
            .collect::<Vec<_>>();
        div()
            .id("runs")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .text_color(content(1.))
            .child(
                self.text.region(
                    "runs-text",
                    div()
                        .w_full()
                        .max_w(px(1040.))
                        .mx_auto()
                        .px(px(32.))
                        .pt(px(24.))
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
                                    div()
                                        .flex_1()
                                        .min_w(px(220.))
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.))
                                        .child(glass::eyebrow("Every automation"))
                                        .child(
                                            div()
                                                .text_size(px(22.))
                                                .line_height(px(28.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child("All runs"),
                                        ),
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
                                        .child(
                                            self.text
                                                .text(format!("Could not refresh runs: {error}")),
                                        ),
                                )
                            },
                        )
                        .child(self.filters(cx))
                        .children(pinned)
                        .child(self.panel(rows, today_start, cx)),
                ),
            )
            .children(self.more_menu.as_ref().map(|(menu, _)| {
                crate::popover::menu_popover(menu, Anchor::Slot(self.more_anchor.clone()), cx)
            }))
    }
}
