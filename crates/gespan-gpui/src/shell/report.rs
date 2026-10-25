//! Entry points for "Report an issue": the Help menu and palette action,
//! the app error toast's Report, and the crash notice on the next launch.
use super::*;
use crate::report::{Facts, Prefill, ReportEvent, ReportIssue, ReportSheet, SheetContext};
use gespan_core::diagnostics::{self, Destination, PendingCrash};
use std::{path::PathBuf, time::Duration};

/// Error lines logged this long before a toast appears are linked to it.
const TOAST_EVENTS: Duration = Duration::from_secs(10);

pub(super) struct Open {
    pub(super) sheet: Entity<ReportSheet>,
    restore: Option<FocusHandle>,
    _events: Subscription,
}

#[derive(Default)]
pub(crate) struct ReportState {
    pub(super) open: Option<Open>,
    /// Unsent crash records the crash notice is showing.
    pub(super) crashes: Vec<PendingCrash>,
    crash_log_dir: Option<PathBuf>,
    /// The app error toast's text and the error `event_id`s logged just
    /// before it first showed.
    toast: Option<(String, Vec<String>)>,
}

impl Shell {
    pub(super) fn open_report(
        &mut self,
        prefill: Prefill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let restore = self
            .report
            .open
            .take()
            .and_then(|open| open.restore)
            .or_else(|| window.focused(cx));
        self.close_menu(cx);
        let context = self.report_context();
        let sheet = cx.new(|cx| ReportSheet::new(prefill, context, cx));
        let events = cx.subscribe_in(
            &sheet,
            window,
            |this, _, event: &ReportEvent, window, cx| match event {
                ReportEvent::Close => this.close_report(window, cx),
            },
        );
        window.focus(&sheet.focus_handle(cx), cx);
        self.report.open = Some(Open {
            sheet,
            restore,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn close_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.report.open.take() {
            match open.restore {
                Some(restore) => window.focus(&restore, cx),
                None => self.focus_active(window, cx),
            }
        }
        cx.notify();
    }

    pub(super) fn report_context(&self) -> SheetContext {
        let (version, channel) = diagnostics::running_config().unwrap_or_else(|| {
            (
                crate::notifications::updater::version().into(),
                diagnostics::Channel::Dev,
            )
        });
        SheetContext {
            log_dir: diagnostics::running_log_dir(),
            roots: diagnostics::report_roots(
                &self.workspace.projects,
                &self
                    .workspace
                    .archived_projects
                    .iter()
                    .cloned()
                    .chain(self.session_summaries.values().map(|s| s.cwd.clone()))
                    .collect::<Vec<_>>(),
            ),
            projects_count: self.workspace.projects.len(),
            sessions_count: self.session_summaries.len(),
            version,
            channel,
            downloads: diagnostics::downloads_dir(),
            // Until the intake exists (#75), every report uses the fallback.
            destination: Destination::Fallback,
            facts: test_facts(),
            reveal: Arc::new(|path: &std::path::Path| gespan_core::files::reveal_path(path)),
        }
    }

    pub(super) fn report_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.report.open.as_ref()?;
        let viewport = window.viewport_size();
        let width = 560f32.min(viewport.width.as_f32() - 32.);
        let anchor = Anchor::Point(gpui::point(
            px((viewport.width.as_f32() - width) / 2.),
            px(48.),
        ));
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("report-scrim"))
                .child(
                    popover("report-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(80)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.close_report(window, cx));
                        })
                        .child(open.sheet.clone()),
                )
                .into_any_element(),
        )
    }

    /// Looks for crash records from earlier runs, off the UI thread.
    pub(crate) fn check_crash_notice(&mut self, cx: &mut Context<Self>) {
        let Some(log_dir) = diagnostics::running_log_dir() else {
            return;
        };
        let scan = {
            let log_dir = log_dir.clone();
            cx.background_executor()
                .spawn(async move { diagnostics::pending_crashes(&log_dir) })
        };
        cx.spawn(async move |this, cx| {
            let crashes = scan.await;
            let _ = this.update(cx, |this, cx| this.show_crash_notice(log_dir, crashes, cx));
        })
        .detach();
    }

    pub(super) fn show_crash_notice(
        &mut self,
        log_dir: PathBuf,
        crashes: Vec<PendingCrash>,
        cx: &mut Context<Self>,
    ) {
        self.report.crash_log_dir = Some(log_dir);
        self.report.crashes = crashes;
        cx.notify();
    }

    /// "Don't send": the noticed records are deleted.
    fn discard_crashes(&mut self, cx: &mut Context<Self>) {
        let crashes = std::mem::take(&mut self.report.crashes);
        if let Some(log_dir) = self.report.crash_log_dir.clone() {
            let names: Vec<String> = crashes
                .iter()
                .map(|c| format!("crashes/{}", c.name))
                .collect();
            cx.background_executor()
                .spawn(async move { diagnostics::remove_crashes(&log_dir, &names) })
                .detach();
        }
        cx.notify();
    }

    pub(super) fn crash_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.report.crashes.is_empty() {
            return None;
        }
        Some(
            crate::notifications::surface::toast_frame()
                .id("crash-notice")
                .debug_selector(|| "crash-notice".into())
                .role(Role::Alert)
                .child(
                    div()
                        .px(px(14.))
                        .py_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(px(13.))
                        .child("Gespan quit unexpectedly last time. Send a report?")
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    crate::modal::primary_button("crash-notice-report", "Report…")
                                        .debug_selector(|| "crash-notice-report".into())
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let title = this.report.crashes[0].title.clone();
                                            this.report.crashes.clear();
                                            this.open_report(
                                                Prefill {
                                                    title,
                                                    event_ids: Vec::new(),
                                                },
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    crate::modal::ghost_button(
                                        "crash-notice-dismiss",
                                        "Don't send",
                                    )
                                    .debug_selector(|| "crash-notice-dismiss".into())
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.discard_crashes(cx)),
                                    ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Notes which error lines a newly shown app error toast follows.
    pub(super) fn sync_error_toast(&mut self) {
        if self.report.toast.as_ref().map(|(message, _)| message) != self.error.as_ref() {
            self.report.toast = self
                .error
                .clone()
                .map(|message| (message, diagnostics::recent_error_events(TOAST_EVENTS)));
        }
    }

    /// The app error toast's Report button.
    pub(super) fn report_error_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("report-error")
            .debug_selector(|| "report-error".into())
            .role(Role::Button)
            .aria_label("Report this error")
            .px_1p5()
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(content(0.75))
            .hover(|s| s.bg(content(0.08)))
            .child("Report")
            .on_click(cx.listener(|this, _, window, cx| {
                let (message, event_ids) = this.report.toast.clone().unwrap_or_default();
                // Never cut inside the text: scrubbing matches whole paths and
                // tokens, so a cut-off one would survive into the issue.
                let title = message.lines().next().unwrap_or_default().to_owned();
                this.error = None;
                this.open_report(Prefill { title, event_ids }, window, cx);
            }))
            .into_any_element()
    }
}

/// Tests never run provider CLIs or `sw_vers`.
#[cfg(test)]
fn test_facts() -> Option<Facts> {
    Some(Facts {
        system: diagnostics::System {
            version: "0.1.0".into(),
            build_channel: "dev",
            os: "macos".into(),
            os_version: "15.1".into(),
            arch: "aarch64".into(),
            locale: "en-GB".into(),
        },
        providers: Vec::new(),
    })
}

#[cfg(not(test))]
fn test_facts() -> Option<Facts> {
    None
}

/// The Help menu's and the palette's entry.
pub(super) fn report_issue(
    this: &mut Shell,
    _: &ReportIssue,
    window: &mut Window,
    cx: &mut Context<Shell>,
) {
    this.open_report(Prefill::default(), window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    fn sheet_title(shell: &Shell, cx: &App) -> String {
        let open = shell
            .report
            .open
            .as_ref()
            .expect("the report sheet is open");
        open.sheet.read(cx).title.read(cx).content.to_string()
    }

    #[gpui::test]
    fn report_issue_action_opens_the_sheet(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, window, cx| {
                // As at launch, where `main` focuses the active pane.
                shell.focus_active(window, cx);
                window.dispatch_action(Box::new(ReportIssue), cx)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(sheet_title(shell, cx), "")
            })
            .unwrap();
        // It renders, with the public-attachment warning.
        assert!(visual.debug_bounds("report-public-warning").is_some());
        let cancel = visual.debug_bounds("report-cancel").unwrap();
        visual.simulate_click(cancel.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.report.open.is_none()))
            .unwrap();
    }

    #[gpui::test]
    fn help_menu_reports_an_issue(cx: &mut TestAppContext) {
        let menus = cx.update(|cx| {
            crate::file_editor::menus(&crate::settings::Preferences::default(), cx);
            cx.get_menus().unwrap()
        });
        let help = menus
            .iter()
            .find(|menu| menu.name == "Help")
            .expect("a Help menu");
        assert!(help.items.iter().any(|item| matches!(
            item,
            gpui::OwnedMenuItem::Action { name, action, .. }
                if name == "Report an issue…" && action.partial_eq(&ReportIssue)
        )));
    }

    #[gpui::test]
    fn error_toast_report_prefills_title_and_event_ids(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        gespan_core::diagnostics::note_error_event("toast-event-1".into());
        window
            .update(visual, |shell, _, cx| {
                shell.error = Some("Could not create the worktree".into());
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let report = visual
            .debug_bounds("report-error")
            .expect("the toast has Report");
        visual.simulate_click(report.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(sheet_title(shell, cx), "Could not create the worktree");
                let open = shell.report.open.as_ref().unwrap();
                assert!(
                    open.sheet
                        .read(cx)
                        .event_ids()
                        .contains(&"toast-event-1".to_string())
                );
                assert!(shell.error.is_none(), "the toast gives way to the sheet");
            })
            .unwrap();
    }

    /// Scrubbing matches whole paths and tokens, so the toast's text must
    /// not be cut before it is scrubbed: a cut-off home path survives.
    #[gpui::test]
    fn a_long_toast_is_not_cut_before_scrubbing(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let message = format!(
            "{} /Users/alice/code/app failed",
            "git exited 128;".repeat(9)
        );
        window
            .update(visual, |shell, _, cx| {
                shell.error = Some(format!("{message}\nsecond line"));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let report = visual.debug_bounds("report-error").unwrap();
        visual.simulate_click(report.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(sheet_title(shell, cx), message)
            })
            .unwrap();
    }

    #[gpui::test]
    fn reports_hide_worktree_and_session_folders(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, _| {
                shell
                    .workspace
                    .archived_projects
                    .push("/work/old-secret".into());
                let roots = shell.report_context().roots;
                for root in [
                    "/work/gespan",
                    "/work/another-project",
                    "/work/gespan-worktrees",
                    "/work/another-project-worktrees",
                    "/work/old-secret",
                ] {
                    assert!(roots.iter().any(|r| r == root), "{root} in {roots:?}");
                }
                assert_eq!(
                    shell.report_context().projects_count,
                    shell.workspace.projects.len()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn crash_notice_report_prefills_the_crash_title(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let dir = tempfile::tempdir().unwrap();
        window
            .update(visual, |shell, _, cx| {
                shell.show_crash_notice(
                    dir.path().to_owned(),
                    vec![PendingCrash {
                        name: "a.json".into(),
                        title: "Crash in src/a.rs:12".into(),
                    }],
                    cx,
                )
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("crash-notice").is_some());
        let report = visual.debug_bounds("crash-notice-report").unwrap();
        visual.simulate_click(report.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(sheet_title(shell, cx), "Crash in src/a.rs:12");
                assert!(shell.report.crashes.is_empty());
            })
            .unwrap();
        assert!(visual.debug_bounds("crash-notice").is_none());
    }

    #[gpui::test]
    fn crash_notice_dont_send_deletes_the_records(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let dir = tempfile::tempdir().unwrap();
        let crashes = gespan_core::diagnostics::crash_dir(dir.path());
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(crashes.join("a.json"), "{}").unwrap();
        std::fs::write(crashes.join("later.json"), "{}").unwrap();
        let pending = vec![PendingCrash {
            name: "a.json".into(),
            title: "Crash in src/a.rs:12".into(),
        }];
        window
            .update(visual, |shell, _, cx| {
                shell.show_crash_notice(dir.path().to_owned(), pending, cx)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let dismiss = visual.debug_bounds("crash-notice-dismiss").unwrap();
        visual.simulate_click(dismiss.center(), Default::default());
        visual.run_until_parked();
        assert!(!crashes.join("a.json").exists());
        assert!(
            crashes.join("later.json").exists(),
            "only the noticed records go"
        );
        window
            .update(visual, |shell, _, _| {
                assert!(shell.report.crashes.is_empty());
                assert!(shell.report.open.is_none());
            })
            .unwrap();
    }
}
