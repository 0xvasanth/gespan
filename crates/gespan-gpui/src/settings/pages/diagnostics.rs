//! Diagnostics: the local diagnostics actions and the last 7 days of
//! failure counters. Nothing here sends anything; a report only leaves the
//! machine when the user sends the saved zip on themselves.
use crate::glass::Button;
use crate::settings::SettingsView;
use crate::theme::*;
use chrono::NaiveDate;
use gespan_core::diagnostics::{self, metrics::Count};
use gpui::{AnyElement, ClipboardItem, Context, SharedString, Task, div, prelude::*, px};
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

/// The 7-day bar's track, as long as the design draws it.
const BAR_TRACK: f32 = 84.;

/// The tone a failure kind's chip shows in: a crash is the one that's red.
pub(in crate::settings) fn kind_tone(kind: &str) -> crate::glass::Tone {
    if kind == "panic" {
        crate::glass::Tone::Bad
    } else {
        crate::glass::Tone::Muted
    }
}

/// A failure kind as a chip that hugs its text.
fn kind_chip(kind: &'static str) -> gpui::Div {
    let tone = kind_tone(kind);
    let (ink, fill, edge) = match tone {
        crate::glass::Tone::Muted => (content(0.7), content(0.07), crate::glass::edge()),
        _ => (tone.ink(), tone.ink().opacity(0.1), tone.ink().opacity(0.3)),
    };
    div()
        .debug_selector(move || format!("settings-counter-kind-{kind}"))
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .bg(fill)
        .border_1()
        .border_color(edge)
        .font_family(crate::fonts::monospace())
        .text_size(px(11.))
        .text_color(ink)
        .child(kind)
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
        let description: SharedString = self
            .diagnostics
            .error
            .clone()
            .unwrap_or_else(|| {
                "Logs stay on this machine. A report shows you every file before it is saved, and Gespan never sends it: you email it or attach it to a GitHub issue."
                    .into()
            })
            .into();
        let flash = self.flashing(Some("diagnostics-actions"));
        let card = crate::glass::card()
            .debug_selector(|| "settings-row-diagnostics-actions".into())
            .relative()
            .p(px(16.))
            .flex()
            .items_center()
            .gap(px(14.))
            .when(flash, |d| d.border_color(accent_color().opacity(0.6)))
            .child(self.reveal_probe("diagnostics-actions"))
            .child(
                div()
                    .debug_selector(|| "settings-report-tile".into())
                    .size(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .bg(crate::glass::Tone::Ok.ink().opacity(0.12))
                    .border_1()
                    .border_color(crate::glass::Tone::Ok.ink().opacity(0.25))
                    .child(icon("shield-alert", 20.).text_color(crate::glass::Tone::Ok.ink())),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("Report an issue"),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .text_color(content(0.62))
                            .child(description),
                    ),
            );
        let buttons = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                crate::glass::button("settings-report-issue", "Report an issue…", Button::Primary)
                    .debug_selector(|| "settings-report-issue".into())
                    .role(gpui::Role::Button)
                    .aria_label("Report an issue")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(crate::report::ReportIssue), cx)
                    })),
            )
            .child(
                crate::glass::button("settings-open-logs", "Open logs folder", Button::Glass)
                    .debug_selector(|| "settings-open-logs".into())
                    .role(gpui::Role::Button)
                    .aria_label("Open logs folder")
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
                crate::glass::button(
                    "settings-copy-diagnostics",
                    "Copy diagnostics",
                    Button::Glass,
                )
                .debug_selector(|| "settings-copy-diagnostics".into())
                .role(gpui::Role::Button)
                .aria_label("Copy diagnostics")
                .on_click(cx.listener(|_, _, _, cx| {
                    // It scans the log and crash folders.
                    let summary = cx
                        .background_executor()
                        .spawn(async { diagnostics::summary() });
                    cx.spawn(async move |_, cx| {
                        let summary = summary.await;
                        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(summary)))
                    })
                    .detach();
                })),
            )
            .into_any_element();
        // Today's failures, once counted, wait on the section's right.
        let today = match &self.diagnostics.counters {
            Some(Ok(rows)) => rows.iter().map(|r| r.today).sum::<u64>(),
            _ => 0,
        };
        vec![
            // The buttons sit close under the card they act for.
            div()
                .flex()
                .flex_col()
                .gap(px(11.))
                .child(self.glass_group(
                    None,
                    "Help",
                    None,
                    None,
                    None,
                    vec![card.into_any_element()],
                ))
                .child(buttons)
                .into_any_element(),
            self.glass_group(
                None,
                "Last 7 days",
                Some("Failures counted on this machine, by kind. Never error text.".into()),
                (today > 0).then(|| {
                    crate::glass::status(format!("{today} today"), crate::glass::Tone::Waiting)
                        .debug_selector(|| "settings-counters-today".into())
                        .into_any_element()
                }),
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
            Some(Ok(rows)) if rows.is_empty() => vec![
                crate::settings::widgets::empty(
                    "settings-counters-empty",
                    "check-circle",
                    "No failures",
                    "Nothing has failed in the last 7 days.",
                )
                .into_any_element(),
            ],
            Some(Ok(rows)) => {
                let most = rows.iter().map(|r| r.week).max().unwrap_or(1).max(1);
                let head = |label: &'static str| crate::glass::eyebrow(label);
                std::iter::once(
                    div()
                        .px_4()
                        .py(px(8.))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().flex_1().min_w_0().child(head("Name")))
                        .child(div().w(px(120.)).child(head("Kind")))
                        .child(div().w(px(70.)).child(head("Today")))
                        .child(div().w(px(150.)).child(head("Last 7 days")))
                        .into_any_element(),
                )
                .chain(rows.iter().map(|row| {
                    let selector = format!("settings-counter-{}-{}", row.name, row.kind);
                    let share = row.week as f32 / most as f32;
                    div()
                        .debug_selector(move || selector.clone())
                        .border_t_1()
                        .border_color(crate::glass::hairline())
                        .px_4()
                        .py(px(9.))
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(crate::fonts::monospace())
                                .text_color(content(0.9))
                                .child(row.name.clone()),
                        )
                        .child(div().w(px(120.)).flex().child(kind_chip(row.kind)))
                        .child(
                            div()
                                .w(px(70.))
                                .text_color(if row.today > 0 {
                                    crate::glass::Tone::Waiting.ink()
                                } else {
                                    content(0.55)
                                })
                                .child(format!("{} today", row.today)),
                        )
                        .child(
                            div()
                                .w(px(150.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(BAR_TRACK))
                                        .h(px(4.))
                                        .rounded_full()
                                        .bg(content(0.08))
                                        .child(
                                            div()
                                                .h_full()
                                                .w(gpui::relative(share))
                                                .rounded_full()
                                                .bg(crate::glass::Tone::Bad.ink()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_color(content(0.9))
                                        .child(format!("{} in 7 days", row.week)),
                                ),
                        )
                        .into_any_element()
                }))
                .collect()
            }
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
    fn diagnostics_closes_the_app_group() {
        let (group, sections) = crate::settings::model::GROUPS[0];
        assert_eq!(group, "App");
        assert_eq!(sections.last(), Some(&Section::Diagnostics));
        assert!(Section::ALL.contains(&Section::Diagnostics));
        assert_eq!(Section::Diagnostics.label(), "Diagnostics");
        for query in ["diagnostics", "crash", "logs folder", "report issue"] {
            let hit = crate::settings::model::search(query, "").into_iter().next();
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
