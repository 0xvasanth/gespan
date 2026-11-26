//! One automation run as a row: the All runs feed and an automation's
//! Recent runs draw it with the same words, chip and colours.
use crate::{glass, selectable::SelectionScope, theme::*};
use chrono::{Local, TimeZone};
use gespan_core::automations::{self, AutomationRun};
use gpui::{
    AnyElement, App, ClickEvent, Role, SharedString, Stateful, Window, div, prelude::*, px,
};

/// One listed run, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunRow {
    pub id: String,
    pub automation_id: String,
    pub created_at: i64,
    /// What fired it: an event label, "Scheduled" or "Test run".
    pub trigger: String,
    /// The item an event was about, or its key when the title is unknown.
    pub item: Option<String>,
    pub automation: String,
    /// The automation's project, unknown once the automation is gone.
    pub project: Option<String>,
    /// The chip's word: Queued, Running, Succeeded, Failed, Cancelled or
    /// Skipped.
    pub status: &'static str,
    pub error: Option<String>,
    pub session: Option<String>,
    /// How long a finished run took, such as `6m 02s`.
    pub duration: Option<String>,
    /// How long a running run has been going, such as `14m 03s`.
    pub elapsed: Option<String>,
}

impl RunRow {
    /// `owner` is the automation's name and project, `None` once it is gone.
    pub(crate) fn new(run: &AutomationRun, owner: Option<(&str, &str)>, now: i64) -> Self {
        Self {
            id: run.id.clone(),
            automation_id: run.automation_id.clone(),
            created_at: run.created_at,
            trigger: trigger_label(run),
            item: item_label(run.event_title.as_deref(), run.event_key.as_deref()),
            automation: owner.map_or_else(|| "Deleted automation".into(), |(name, _)| name.into()),
            project: owner.map(|(_, cwd)| cwd.to_string()),
            status: status_chip(run),
            error: run.error.clone().filter(|error| !error.trim().is_empty()),
            session: run.session_id.clone(),
            duration: duration(run),
            elapsed: elapsed(run, now),
        }
    }

    pub(crate) fn can_open(&self) -> bool {
        self.session.is_some()
    }

    /// Running or queued.
    pub(crate) fn live(&self) -> bool {
        matches!(self.status, "Running" | "Queued")
    }

    /// The title: what fired it, then the item it fired for or, with none,
    /// the automation.
    pub(crate) fn title(&self) -> (String, String) {
        (
            self.trigger.clone(),
            self.item.clone().unwrap_or_else(|| self.automation.clone()),
        )
    }

    /// The quieter line under the title. The automation is named when the
    /// title names an item and `with_automation` is set; the project is
    /// named when `with_project` is set.
    pub(crate) fn meta(&self, with_automation: bool, with_project: bool) -> String {
        [
            (with_automation && self.item.is_some()).then(|| self.automation.clone()),
            self.project
                .as_deref()
                .filter(|_| with_project)
                .map(crate::workspace::project_name),
            self.duration.clone(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }

    /// What the chip reads: a running run's time so far, else its status.
    pub(crate) fn chip(&self) -> String {
        match (&self.elapsed, self.status) {
            (Some(elapsed), "Running") => elapsed.clone(),
            _ => self.status.to_string(),
        }
    }

    /// Whether the search text appears, ignoring case, in what the row
    /// shows: its trigger, item, automation, project or error.
    pub(crate) fn matches(&self, query: &str) -> bool {
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

/// What fired a run, named the same on every page.
pub(crate) fn trigger_label(run: &AutomationRun) -> String {
    match run.trigger.as_str() {
        "manual" => "Test run".into(),
        "event" => automations::event_label(
            run.event_kind.as_deref().unwrap_or(""),
            run.event.as_deref().unwrap_or(""),
        ),
        _ => "Scheduled".into(),
    }
}

/// The chip a run's recorded status shows. A run claimed but not started
/// (`pending`), and a status this build does not know, read Queued.
pub(crate) fn status_chip(run: &AutomationRun) -> &'static str {
    match run.status.as_str() {
        "running" => "Running",
        "succeeded" => "Succeeded",
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        "skipped" => "Skipped",
        _ => "Queued",
    }
}

/// What a row names its item by: the event's title, else its key, with a
/// repository named by its own name only, as `langfuse#20110 bug: …`.
/// A title starting `owner/repo#12` loses `owner/`; a key such as
/// `github:issue:owner/repo:12` reads `repo#12`, and a tracker key such as
/// `linear:issue:ENG-4` reads `ENG-4`. Anything else is shown as it is.
pub(crate) fn item_label(title: Option<&str>, key: Option<&str>) -> Option<String> {
    if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
        let (head, rest) = title.split_once(char::is_whitespace).unwrap_or((title, ""));
        let short = head
            .split_once('#')
            .filter(|(path, number)| {
                path.contains('/')
                    && !number.is_empty()
                    && number.bytes().all(|b| b.is_ascii_digit())
            })
            .and_then(|(path, number)| {
                let repo = path.rsplit('/').next().filter(|repo| !repo.is_empty())?;
                Some(format!("{repo}#{number}"))
            });
        return Some(match short {
            Some(short) if rest.trim().is_empty() => short,
            Some(short) => format!("{short} {}", rest.trim_start()),
            None => title.to_string(),
        });
    }
    let key = key.map(str::trim).filter(|key| !key.is_empty())?;
    let parts: Vec<&str> = key.split(':').collect();
    Some(match parts.as_slice() {
        [_, _, path, number, ..]
            if path.contains('/')
                && !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit()) =>
        {
            let repo = path.rsplit('/').next().unwrap_or(path);
            format!("{repo}#{number}")
        }
        [_, _, id] if !id.is_empty() => id.to_string(),
        _ => key.to_string(),
    })
}

/// `45s`, `6m 02s`, `1h 03m`.
fn span(ms: i64) -> Option<String> {
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

/// How long a run took from start to finish. Runs that never started or
/// have not finished have none.
pub(crate) fn duration(run: &AutomationRun) -> Option<String> {
    span(run.completed_at? - run.started_at?)
}

/// How long a running run has been going at `now`.
pub(crate) fn elapsed(run: &AutomationRun, now: i64) -> Option<String> {
    if run.status != "running" || run.completed_at.is_some() {
        return None;
    }
    span(now - run.started_at?)
}

pub(crate) fn clock(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

/// How a row sits in its card and what its second line names.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub first: bool,
    pub last: bool,
    /// Name the automation: off on an automation's own Recent runs.
    pub automation: bool,
    /// Name the project: off on an automation's own Recent runs.
    pub project: bool,
}

const GROUP: &str = "runs-row";

/// A run as a row: its time (a spinner while it runs), what fired it and
/// for what, the automation, project and duration, its error, its status
/// chip and, under the pointer, "Open thread".
pub(crate) fn run_row(
    row: &RunRow,
    text: &SelectionScope,
    look: Look,
    on_open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    let tone = glass::Tone::of_status(row.status);
    let (trigger, subject) = row.title();
    let meta = row.meta(look.automation, look.project);
    let open_id = format!("runs-open-{}", row.id);
    let aria = match (&row.elapsed, row.status) {
        (Some(elapsed), "Running") => format!("{trigger} · {subject}, Running for {elapsed}"),
        _ => format!("{trigger} · {subject}, {}", row.status),
    };
    let lead = if row.status == "Running" {
        let ink = tone.ink();
        crate::pulse::looping(
            SharedString::from(format!("runs-spinner-{}", row.id)),
            std::time::Duration::from_secs(1),
            crate::pulse::Rate::Smooth,
            move |delta| {
                icon("loader", 12.)
                    .text_color(ink)
                    .with_transformation(gpui::Transformation::rotate(gpui::percentage(delta)))
            },
        )
        .into_any_element()
    } else {
        div()
            .font_family(crate::fonts::monospace())
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(content(0.62))
            .child(clock(row.created_at))
            .into_any_element()
    };
    // The trigger reads quieter than what it fired for.
    let title = div()
        .min_w_0()
        .flex()
        .gap(px(4.))
        .text_size(px(13.))
        .line_height(px(18.))
        .child(
            div()
                .flex_none()
                .text_color(content(0.62))
                .child(text.text(format!("{trigger} ·"))),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(content(1.))
                .child(text.text(subject)),
        );
    let button = if row.can_open() {
        glass::button(
            SharedString::from(open_id.clone()),
            "Open thread",
            glass::Button::Glass,
        )
        .h(px(28.))
        .child(icon("arrow-up-right", 12.).text_color(content(0.9)))
        .aria_label("Open thread")
        .on_click(on_open)
    } else {
        div()
            .id(SharedString::from(open_id.clone()))
            .h(px(28.))
            .px(px(14.))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(9.))
            .text_size(px(13.))
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
        .aria_label(aria)
        .px(px(16.))
        .py(px(11.))
        .when(!look.first, |d| {
            d.border_t_1().border_color(glass::hairline())
        })
        // The card clips square, so the end rows round their own hover fill
        // to the card's inner corner.
        .when(look.first, |d| d.rounded_t(px(13.)))
        .when(look.last, |d| d.rounded_b(px(13.)))
        .when(row.status == "Failed", |d| {
            d.bg(glass::Tone::Bad.ink().opacity(0.05))
        })
        .hover(|s| s.bg(glass::hover_fill()))
        .flex()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .w(px(48.))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .child(lead),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(title)
                .when(!meta.is_empty(), |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(content(0.55))
                            .child(text.text(meta)),
                    )
                })
                .when_some(row.error.clone(), |d, error| {
                    d.child(
                        div()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(glass::Tone::Bad.ink().opacity(0.85))
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(text.text(error)),
                    )
                }),
        )
        .child(
            div()
                .w(px(112.))
                .flex_shrink_0()
                .flex()
                .child(glass::status(row.chip(), tone)),
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
                        .role(Role::Button),
                ),
        )
}

/// A labelled card of rows: a day, or "Running now". `ink` colours the label.
pub(crate) fn section(
    selector: String,
    label: impl Into<SharedString>,
    ink: gpui::Hsla,
    rows: Vec<AnyElement>,
) -> AnyElement {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .px(px(4.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(glass::eyebrow(label).line_height(px(16.)).text_color(ink))
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(16.))
                        .text_color(content(0.55))
                        .child(run_count(count)),
                ),
        )
        .child(
            glass::card()
                .debug_selector(move || selector)
                .overflow_hidden()
                .children(rows),
        )
        .into_any_element()
}

/// The heading over one card: `1 run` or `N runs`.
pub(crate) fn run_count(count: usize) -> String {
    if count == 1 {
        "1 run".to_string()
    } else {
        format!("{count} runs")
    }
}
