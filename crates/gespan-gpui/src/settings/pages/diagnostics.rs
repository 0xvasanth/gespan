//! The Diagnostics page: local tools for reporting a problem, and the
//! failure counters #71 keeps on this computer. Nothing here sends anything.
use crate::{
    settings::{
        SettingsEvent, SettingsView,
        widgets::{note, secondary_button},
    },
    theme::*,
};
use gespan_core::diagnostics::metrics::{self, Count, ErrorKind};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use std::{collections::HashMap, sync::Arc};

/// The week of counters the page shows.
const DAYS: u32 = 7;
const COUNTERS_NOTE: &str =
    "How often each operation failed, by kind. Counted on this computer and never sent.";

/// Reads the counters; `metrics::last_days` outside tests.
pub(in crate::settings) type CounterSource =
    Arc<dyn Fn(u32) -> Result<Vec<Count>, String> + Send + Sync>;

pub(in crate::settings) fn last_days() -> CounterSource {
    Arc::new(metrics::last_days)
}

/// One operation's failures of one kind, summed over the week.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::settings) struct CounterRow {
    pub name: String,
    pub kind: ErrorKind,
    pub count: u64,
}

/// Failures only, summed across days and app versions, most frequent first.
pub(in crate::settings) fn failure_rows(counts: &[Count]) -> Vec<CounterRow> {
    let mut sums: HashMap<(&str, ErrorKind), u64> = HashMap::new();
    for count in counts {
        if let Some(kind) = count.kind {
            *sums.entry((count.name.as_str(), kind)).or_default() += count.count;
        }
    }
    let mut rows: Vec<CounterRow> = sums
        .into_iter()
        .map(|((name, kind), count)| CounterRow {
            name: name.to_owned(),
            kind,
            count,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    rows
}

impl SettingsView {
    /// Reads the counters afresh on every visit, off the UI thread.
    pub(in crate::settings) fn enter_diagnostics(&mut self, cx: &mut Context<Self>) {
        let source = self.counter_source.clone();
        let read = cx
            .background_executor()
            .spawn(async move { source(DAYS).map(|counts| failure_rows(&counts)) });
        self.counters_loading = Some(cx.spawn(async move |this, cx| {
            let rows = read.await;
            let _ = this.update(cx, |this, cx| {
                this.counters = Some(rows);
                this.counters_loading = None;
                cx.notify();
            });
        }));
    }

    /// Drops the rows and any load still running, so a late load cannot
    /// fill the page after it was left, and the next visit reads again.
    pub(in crate::settings) fn leave_diagnostics(&mut self) {
        self.counters = None;
        self.counters_loading = None;
    }

    pub(super) fn diagnostics_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tools = vec![
            self.row(
                Some("report-issue"),
                true,
                "Report an issue",
                Some(
                    "Describe what went wrong and choose what to attach before anything leaves."
                        .into(),
                ),
                vec![action(
                    "diagnostics-report",
                    "Report an issue…",
                    || SettingsEvent::ReportIssue,
                    cx,
                )],
            ),
            self.row(
                Some("open-logs"),
                false,
                "Open logs folder",
                Some("Show Gespan's log and crash files.".into()),
                vec![action(
                    "diagnostics-open-logs",
                    "Open folder",
                    || SettingsEvent::OpenLogsFolder,
                    cx,
                )],
            ),
            self.row(
                Some("copy-diagnostics"),
                false,
                "Copy diagnostics",
                Some("Copy a short plain-text summary of this build and its log files.".into()),
                vec![action(
                    "diagnostics-copy",
                    "Copy",
                    || SettingsEvent::CopyDiagnostics,
                    cx,
                )],
            ),
        ];
        let counters = match &self.counters {
            None => vec![note("Reading the counters…")],
            Some(Err(error)) => vec![
                div()
                    .debug_selector(|| "diagnostics-counters-error".into())
                    .child(note(format!("Couldn't read the failure counters: {error}")))
                    .into_any_element(),
            ],
            Some(Ok(rows)) if rows.is_empty() => vec![
                div()
                    .debug_selector(|| "diagnostics-counters-empty".into())
                    .child(note("No failures counted in the last 7 days."))
                    .into_any_element(),
            ],
            Some(Ok(rows)) => rows
                .iter()
                .enumerate()
                .map(|(ix, row)| counter_row(ix, row))
                .collect(),
        };
        vec![
            self.group(None, true, "Get help", None, None, tools),
            self.group(
                Some("failure-counters"),
                false,
                "Failures in the last 7 days",
                Some(COUNTERS_NOTE.into()),
                None,
                counters,
            ),
        ]
    }
}

fn action(
    id: &'static str,
    label: &'static str,
    event: fn() -> SettingsEvent,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    secondary_button(id, label, false, false)
        .debug_selector(move || id.into())
        .child(label)
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(event())))
        .into_any_element()
}

fn counter_row(ix: usize, row: &CounterRow) -> AnyElement {
    div()
        .debug_selector(move || format!("diagnostics-counter-{ix}"))
        .flex()
        .items_center()
        .gap_3()
        .when(ix > 0, |d| d.border_t_1().border_color(content(0.05)))
        .px_4()
        .py(px(10.))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .child(row.name.clone()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(content(0.45))
                .child(SharedString::from(row.kind.code())),
        )
        .child(
            div()
                .w(px(48.))
                .text_right()
                .text_size(px(13.))
                .child(row.count.to_string()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Preferences, Section, SettingsContext};
    use chrono::{Days, NaiveDate, Utc};
    use gespan_core::diagnostics::metrics::{Count, Counters, ErrorKind};
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, px, size,
    };
    use std::sync::{Arc, Mutex};

    /// Rows as the #71 store returns them: `(days ago, version, name, kind)`.
    fn stored(records: &[(u64, &str, &str, Option<ErrorKind>)]) -> Vec<Count> {
        let today = Utc::now().date_naive();
        let mut stores: Vec<(&str, Counters)> = Vec::new();
        for &(ago, version, name, kind) in records {
            if !stores.iter().any(|(v, _)| *v == version) {
                stores.push((version, Counters::open_in_memory(version).unwrap()));
            }
            let store = &stores.iter().find(|(v, _)| *v == version).unwrap().1;
            store.record(today - Days::new(ago), name, kind).unwrap();
        }
        stores
            .iter()
            .flat_map(|(_, store)| store.since(NaiveDate::MIN).unwrap())
            .collect()
    }

    #[test]
    fn failures_sum_across_days_and_versions() {
        let counts = stored(&[
            (0, "1.0", "git.fetch", Some(ErrorKind::Network)),
            (3, "1.0", "git.fetch", Some(ErrorKind::Network)),
            (1, "1.1", "git.fetch", Some(ErrorKind::Network)),
            (1, "1.1", "git.fetch", Some(ErrorKind::Auth)),
            (2, "1.1", "agent.spawn", Some(ErrorKind::SpawnFailed)),
            (2, "1.1", "agent.spawn", None),
        ]);
        let rows = failure_rows(&counts);
        let shown: Vec<(&str, ErrorKind, u64)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.kind, r.count))
            .collect();
        assert_eq!(
            shown,
            [
                ("git.fetch", ErrorKind::Network, 3),
                ("agent.spawn", ErrorKind::SpawnFailed, 1),
                ("git.fetch", ErrorKind::Auth, 1),
            ]
        );
    }

    #[test]
    fn successes_alone_are_the_empty_state() {
        let counts = stored(&[
            (0, "1.0", "git.fetch", None),
            (1, "1.0", "agent.spawn", None),
        ]);
        assert!(failure_rows(&counts).is_empty());
    }

    /// A store the tests can change between visits.
    fn source(data: Arc<Mutex<Result<Vec<Count>, String>>>) -> CounterSource {
        Arc::new(move |days| {
            assert_eq!(days, 7, "the page shows a week");
            data.lock().unwrap().clone()
        })
    }

    fn window(
        data: Arc<Mutex<Result<Vec<Count>, String>>>,
        cx: &mut TestAppContext,
    ) -> WindowHandle<SettingsView> {
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1200.), px(800.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut view = SettingsView::new(
                            Section::General,
                            Preferences::default(),
                            SettingsContext::default(),
                            cx,
                        );
                        view.discovery_task = None;
                        view.discovery_loading = false;
                        view.counter_source = source(data);
                        view
                    })
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        window
    }

    fn visit(window: WindowHandle<SettingsView>, section: Section, cx: &mut TestAppContext) {
        window
            .update(cx, |view, _, cx| view.select_section(section, cx))
            .unwrap();
    }

    #[gpui::test]
    fn the_page_lists_counted_failures_and_an_empty_state(cx: &mut TestAppContext) {
        let data = Arc::new(Mutex::new(Ok(stored(&[
            (0, "1.0", "git.fetch", Some(ErrorKind::Network)),
            (1, "1.0", "git.fetch", Some(ErrorKind::Network)),
        ]))));
        let window = window(data.clone(), cx);
        visit(window, Section::Diagnostics, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("diagnostics-counter-0").is_some());
        assert!(visual.debug_bounds("diagnostics-counter-1").is_none());
        assert!(visual.debug_bounds("diagnostics-counters-empty").is_none());

        *data.lock().unwrap() = Ok(stored(&[(0, "1.0", "git.fetch", None)]));
        visit(window, Section::General, visual);
        visit(window, Section::Diagnostics, visual);
        visual.run_until_parked();
        assert!(visual.debug_bounds("diagnostics-counter-0").is_none());
        assert!(visual.debug_bounds("diagnostics-counters-empty").is_some());
    }

    #[gpui::test]
    fn leaving_mid_load_drops_it_and_the_next_visit_reloads(cx: &mut TestAppContext) {
        let data = Arc::new(Mutex::new(Ok(stored(&[(
            0,
            "1.0",
            "git.fetch",
            Some(ErrorKind::Network),
        )]))));
        let window = window(data.clone(), cx);
        // Leave before the load has had a chance to finish.
        visit(window, Section::Diagnostics, cx);
        visit(window, Section::General, cx);
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                assert!(view.counters.is_none(), "a late load filled stale counters");
                assert!(view.counters_loading.is_none());
            })
            .unwrap();

        *data.lock().unwrap() = Ok(stored(&[
            (0, "1.0", "agent.spawn", Some(ErrorKind::Timeout)),
            (0, "1.0", "agent.spawn", Some(ErrorKind::Timeout)),
        ]));
        visit(window, Section::Diagnostics, cx);
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                let rows = view.counters.clone().expect("loaded again").unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].name, "agent.spawn");
                assert_eq!(rows[0].count, 2);
                assert!(view.counters_loading.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_store_error_is_shown_not_an_empty_state(cx: &mut TestAppContext) {
        let data = Arc::new(Mutex::new(Err("database is locked".to_owned())));
        let window = window(data, cx);
        visit(window, Section::Diagnostics, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("diagnostics-counters-error").is_some());
        assert!(visual.debug_bounds("diagnostics-counters-empty").is_none());
    }
}
