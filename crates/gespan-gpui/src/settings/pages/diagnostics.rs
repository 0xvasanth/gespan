//! Diagnostics: the local diagnostics actions and the last 7 days of
//! failure counters. Nothing here sends anything; a report only leaves the
//! machine when the user sends the saved zip on themselves.
use crate::settings::{SettingsView, widgets::secondary_button};
use crate::theme::*;
use chrono::NaiveDate;
use gespan_core::diagnostics::{self, metrics::Count};
use gpui::{AnyElement, ClipboardItem, Context, Task, div, prelude::*, px};
use std::collections::BTreeMap;

/// Days the counters view covers, today included.
const DAYS: u32 = 7;

/// One line of the counters view: a failure kind of one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CounterRow {
    pub name: String,
    pub kind: &'static str,
    pub today: u64,
    pub week: u64,
}

pub(crate) struct DiagnosticsState {
    /// `None` until loaded; cleared whenever the page is left.
    pub(crate) counters: Option<Result<Vec<CounterRow>, String>>,
    loading: Option<Task<()>>,
    /// Why "Open logs folder" failed.
    error: Option<String>,
}

impl DiagnosticsState {
    pub(crate) fn new() -> Self {
        Self {
            counters: None,
            loading: None,
            error: None,
        }
    }
}

/// Failures per operation and kind over the last [`DAYS`] days, summed over
/// app versions, most first. Successes are not listed.
pub(crate) fn summarize(counts: &[Count], today: NaiveDate) -> Vec<CounterRow> {
    let first = today - chrono::Days::new(u64::from(DAYS - 1));
    let mut rows: BTreeMap<(&str, &'static str), (u64, u64)> = BTreeMap::new();
    for count in counts {
        let Some(kind) = count.kind else { continue };
        if count.day < first || count.day > today {
            continue;
        }
        let row = rows.entry((&count.name, kind.code())).or_default();
        row.1 += count.count;
        if count.day == today {
            row.0 += count.count;
        }
    }
    let mut rows: Vec<CounterRow> = rows
        .into_iter()
        .map(|((name, kind), (today, week))| CounterRow {
            name: name.to_owned(),
            kind,
            today,
            week,
        })
        .collect();
    // The map gave name order; a stable sort keeps it among equal totals.
    rows.sort_by_key(|row| std::cmp::Reverse(row.week));
    rows
}

impl SettingsView {
    fn load_counters(&mut self, cx: &mut Context<Self>) {
        let work = cx.background_executor().spawn(async {
            diagnostics::metrics::last_days(DAYS)
                .map(|counts| summarize(&counts, chrono::Utc::now().date_naive()))
        });
        self.diagnostics.loading = Some(cx.spawn(async move |this, cx| {
            let counters = work.await;
            let _ = this.update(cx, |this, cx| {
                this.diagnostics.counters = Some(counters);
                this.diagnostics.loading = None;
                cx.notify();
            });
        }));
    }

    pub(super) fn diagnostics_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.diagnostics.counters.is_none() && self.diagnostics.loading.is_none() {
            self.load_counters(cx);
        }
        let help = vec![self.row(
            Some("diagnostics-actions"),
            true,
            "Report an issue",
            Some(
                self.diagnostics
                    .error
                    .clone()
                    .unwrap_or_else(|| {
                        "Logs stay on this machine. A report shows you every file before it is saved, and Gespan never sends it: you email it or attach it to a GitHub issue."
                            .into()
                    })
                    .into(),
            ),
            vec![
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        secondary_button("settings-report-issue", "Report an issue", false, false)
                            .debug_selector(|| "settings-report-issue".into())
                            .child("Report an issue…")
                            .on_click(cx.listener(|_, _, window, cx| {
                                window.dispatch_action(Box::new(crate::report::ReportIssue), cx)
                            })),
                    )
                    .child(
                        secondary_button("settings-open-logs", "Open logs folder", false, false)
                            .debug_selector(|| "settings-open-logs".into())
                            .child("Open logs folder")
                            .on_click(cx.listener(|_, _, _, cx| {
                                let open = cx
                                    .background_executor()
                                    .spawn(async { diagnostics::open_logs_folder() });
                                cx.spawn(async move |this, cx| {
                                    let error = open.await.err();
                                    let _ = this.update(cx, |this, cx| {
                                        this.diagnostics.error =
                                            error.map(|e| format!("Open logs folder: {e}"));
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })),
                    )
                    .child(
                        secondary_button(
                            "settings-copy-diagnostics",
                            "Copy diagnostics",
                            false,
                            false,
                        )
                        .debug_selector(|| "settings-copy-diagnostics".into())
                        .child("Copy diagnostics")
                        .on_click(cx.listener(|_, _, _, cx| {
                            // It scans the log and crash folders.
                            let summary = cx
                                .background_executor()
                                .spawn(async { diagnostics::summary() });
                            cx.spawn(async move |_, cx| {
                                let summary = summary.await;
                                cx.update(|cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(summary))
                                })
                            })
                            .detach();
                        })),
                    )
                    .into_any_element(),
            ],
        )];
        vec![
            self.group(None, true, "Help", None, None, help),
            self.group(
                None,
                false,
                "Last 7 days",
                Some("Failures counted on this machine, by kind. Never error text.".into()),
                None,
                self.counter_rows(),
            ),
        ]
    }

    fn counter_rows(&self) -> Vec<AnyElement> {
        let message = |selector: &'static str, text: String| {
            div()
                .debug_selector(move || selector.into())
                .px_4()
                .py_3()
                .text_size(px(12.))
                .text_color(content(0.5))
                .child(text)
                .into_any_element()
        };
        match &self.diagnostics.counters {
            None => vec![message("settings-counters-loading", "Loading…".into())],
            Some(Err(error)) => vec![message("settings-counters-error", error.clone())],
            Some(Ok(rows)) if rows.is_empty() => vec![message(
                "settings-counters-empty",
                "No failures in the last 7 days.".into(),
            )],
            Some(Ok(rows)) => rows
                .iter()
                .enumerate()
                .map(|(ix, row)| {
                    let selector = format!("settings-counter-{}-{}", row.name, row.kind);
                    div()
                        .debug_selector(move || selector.clone())
                        .when(ix > 0, |d| d.border_t_1().border_color(content(0.06)))
                        .px_4()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(crate::markdown::PILOT)
                                .text_color(content(0.9))
                                .child(row.name.clone()),
                        )
                        .child(
                            div()
                                .w(px(110.))
                                .font_family(crate::markdown::PILOT)
                                .text_color(content(0.55))
                                .child(row.kind),
                        )
                        .child(
                            div()
                                .w(px(70.))
                                .text_color(content(0.55))
                                .child(format!("{} today", row.today)),
                        )
                        .child(
                            div()
                                .w(px(80.))
                                .text_color(content(0.9))
                                .child(format!("{} in 7 days", row.week)),
                        )
                        .into_any_element()
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Preferences, Section, SettingsContext};
    use gespan_core::diagnostics::metrics::{Count, ErrorKind};
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, px, size,
    };

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn count(d: u32, name: &str, kind: Option<ErrorKind>, n: u64, version: &str) -> Count {
        Count {
            day: day(d),
            name: name.into(),
            kind,
            count: n,
            app_version: version.into(),
        }
    }

    #[test]
    fn summarize_totals_failures_per_name_and_kind() {
        let counts = vec![
            count(23, "agent.start", Some(ErrorKind::SpawnFailed), 2, "0.16.0"),
            count(25, "agent.start", Some(ErrorKind::SpawnFailed), 4, "0.16.0"),
            count(25, "agent.start", Some(ErrorKind::SpawnFailed), 2, "0.16.1"),
            count(25, "agent.start", None, 30, "0.16.1"),
            count(24, "git.command", Some(ErrorKind::ExitStatus), 3, "0.16.0"),
            count(
                24,
                "worktree.create",
                Some(ErrorKind::NotFound),
                8,
                "0.16.0",
            ),
            count(
                25,
                "app_update.check",
                Some(ErrorKind::Network),
                1,
                "0.16.1",
            ),
        ];
        assert_eq!(
            summarize(&counts, day(25)),
            vec![
                CounterRow {
                    name: "agent.start".into(),
                    kind: "spawn_failed",
                    today: 6,
                    week: 8
                },
                CounterRow {
                    name: "worktree.create".into(),
                    kind: "not_found",
                    today: 0,
                    week: 8
                },
                CounterRow {
                    name: "git.command".into(),
                    kind: "exit_status",
                    today: 0,
                    week: 3
                },
                CounterRow {
                    name: "app_update.check".into(),
                    kind: "network",
                    today: 1,
                    week: 1
                },
            ]
        );
    }

    #[test]
    fn diagnostics_is_in_the_app_group_after_general() {
        let (group, sections) = crate::settings::model::GROUPS[0];
        assert_eq!(group, "App");
        assert_eq!(&sections[..2], &[Section::General, Section::Diagnostics]);
        assert!(Section::ALL.contains(&Section::Diagnostics));
        assert_eq!(Section::Diagnostics.label(), "Diagnostics");
        for query in ["diagnostics", "crash", "logs folder", "report issue"] {
            let hit = crate::settings::model::search(query).into_iter().next();
            assert_eq!(
                hit.map(|r| r.section),
                Some(Section::Diagnostics),
                "{query}"
            );
        }
    }

    #[test]
    fn summarize_ignores_days_outside_the_week() {
        let counts = vec![
            count(18, "git.command", Some(ErrorKind::Io), 5, "0.16.0"),
            count(19, "git.command", Some(ErrorKind::Io), 1, "0.16.0"),
            count(26, "git.command", Some(ErrorKind::Io), 9, "0.16.0"),
        ];
        // The 19th through the 25th: seven days, today included.
        assert_eq!(
            summarize(&counts, day(25)),
            vec![CounterRow {
                name: "git.command".into(),
                kind: "io",
                today: 0,
                week: 1
            }]
        );
        assert!(summarize(&[], day(25)).is_empty());
    }

    fn window(
        prefs: Preferences,
        cx: &mut TestAppContext,
    ) -> (WindowHandle<SettingsView>, VisualTestContext) {
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1200.), px(1400.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut view = SettingsView::new(
                            Section::Diagnostics,
                            prefs,
                            SettingsContext::default(),
                            cx,
                        );
                        view.discovery_task = None;
                        view.discovery_loading = false;
                        view
                    })
                },
            )
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        (window, visual)
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is shown"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn shows_only_local_tools_and_stores_nothing(cx: &mut TestAppContext) {
        let (window, mut visual) = window(Preferences::default(), cx);
        for selector in [
            "settings-report-issue",
            "settings-open-logs",
            "settings-copy-diagnostics",
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "{selector}");
        }
        // No sending choices: Gespan never sends a report by itself.
        for selector in [
            "settings-crash-reports-ask",
            "settings-toggle-error-metrics",
            "settings-toggle-include-logs",
        ] {
            assert!(visual.debug_bounds(selector).is_none(), "{selector}");
        }
        let saved = window
            .update(&mut visual, |view, _, _| view.prefs.clone())
            .unwrap();
        assert_eq!(
            serde_json::to_value(&saved).unwrap(),
            serde_json::to_value(Preferences::default()).unwrap(),
            "looking at the page changes no preference"
        );
    }

    #[gpui::test]
    fn counters_render_rows_and_empty_state(cx: &mut TestAppContext) {
        let (window, mut visual) = window(Preferences::default(), cx);
        assert!(visual.debug_bounds("settings-counters-empty").is_some());
        window
            .update(&mut visual, |view, _, cx| {
                view.diagnostics.counters = Some(Ok(vec![
                    CounterRow {
                        name: "agent.start".into(),
                        kind: "spawn_failed",
                        today: 6,
                        week: 8,
                    },
                    CounterRow {
                        name: "git.command".into(),
                        kind: "exit_status",
                        today: 0,
                        week: 3,
                    },
                ]));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("settings-counters-empty").is_none());
        assert!(
            visual
                .debug_bounds("settings-counter-agent.start-spawn_failed")
                .is_some()
        );
        assert!(
            visual
                .debug_bounds("settings-counter-git.command-exit_status")
                .is_some()
        );
    }

    #[gpui::test]
    fn report_an_issue_dispatches_the_report_action(cx: &mut TestAppContext) {
        let reported = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = reported.clone();
        cx.update(|cx| {
            cx.on_action(move |_: &crate::report::ReportIssue, _| seen.set(true));
        });
        let (_, mut visual) = window(Preferences::default(), cx);
        click(&mut visual, "settings-report-issue");
        assert!(reported.get());
    }

    #[gpui::test]
    fn copy_diagnostics_writes_the_summary(cx: &mut TestAppContext) {
        let (_, mut visual) = window(Preferences::default(), cx);
        click(&mut visual, "settings-copy-diagnostics");
        let copied = visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .expect("copied");
        assert_eq!(copied, gespan_core::diagnostics::summary());
    }
}
