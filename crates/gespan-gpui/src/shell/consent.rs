//! The diagnostics consent sheet: shown once after install, and again when
//! `consent_version` goes up. A small card beside the other notices, never a
//! modal; closing it counts as "Ask me each time".
use super::*;
use gespan_core::diagnostics::{self, Answer, outbox};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct PromptState {
    pub(super) shown: bool,
    /// Where the send queues live; `None` before diagnostics start.
    pub(super) log_dir: Option<PathBuf>,
}

impl Shell {
    /// At launch: ask when there is no answer, or one to an older version.
    pub(crate) fn check_consent_prompt(&mut self, cx: &mut Context<Self>) {
        self.consent.log_dir = diagnostics::running_log_dir();
        self.consent.shown = self.workspace.preferences.diagnostics.needs_prompt();
        cx.notify();
    }

    fn answer_consent(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        let mut prefs = self.workspace.preferences.clone();
        let cleared = prefs
            .diagnostics
            .answer(answer, &diagnostics::consent::now());
        self.consent.shown = false;
        self.set_preferences(prefs, window, cx);
        clear_queues(self.consent.log_dir.clone(), cleared, cx);
    }

    pub(super) fn consent_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.consent.shown {
            return None;
        }
        let answer = |id: &'static str, label: &'static str, answer: Answer, primary: bool| {
            let button = if primary {
                crate::modal::primary_button(id, label)
            } else {
                crate::modal::ghost_button(id, label)
            };
            button
                .debug_selector(move || id.into())
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, window, cx| this.answer_consent(answer, window, cx)),
                )
        };
        Some(
            crate::notifications::surface::toast_frame()
                .id("consent-prompt")
                .debug_selector(|| "consent-prompt".into())
                .role(Role::Dialog)
                .aria_label("Help improve Gespan?")
                .child(
                    div()
                        .px(px(14.))
                        .py_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(px(13.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Help improve Gespan?"))
                                .child(
                                    icon_button("consent-dismiss", "Close", "close")
                                        .debug_selector(|| "consent-dismiss".into())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.answer_consent(Answer::Dismissed, window, cx)
                                        })),
                                ),
                        )
                        .child(
                            div().text_color(content(0.7)).child(
                                "Crash reports and failure counts help us fix bugs. They never include your code, prompts or file contents.",
                            ),
                        )
                        .child(
                            div()
                                .id("consent-privacy")
                                .debug_selector(|| "consent-privacy".into())
                                .role(Role::Link)
                                .cursor_pointer()
                                .text_color(content(0.6))
                                .underline()
                                .hover(|s| s.text_color(content(1.)))
                                .child("What we collect")
                                .on_click(|_, _, cx| cx.open_url(diagnostics::PRIVACY_NOTICE)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(answer(
                                    "consent-always",
                                    "Send crash reports automatically",
                                    Answer::Always,
                                    true,
                                ))
                                .child(answer("consent-ask", "Ask me each time", Answer::Ask, false))
                                .child(answer("consent-never", "No thanks", Answer::Never, false)),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// Empties the queues of kinds that no longer send, off the UI thread.
pub(crate) fn clear_queues<T>(
    log_dir: Option<PathBuf>,
    kinds: Vec<outbox::Kind>,
    cx: &mut Context<T>,
) {
    let Some(log_dir) = log_dir else {
        return;
    };
    if kinds.is_empty() {
        return;
    }
    cx.background_executor()
        .spawn(async move { outbox::revoke(&log_dir, &kinds) })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::diagnostics::{
        Answer, CONSENT_VERSION, Consent, ConsentState, CrashReports,
        outbox::{Kind, queue_dir},
    };
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    fn launch(
        state: ConsentState,
        log_dir: Option<PathBuf>,
        cx: &mut TestAppContext,
    ) -> (WindowHandle<Shell>, VisualTestContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        window
            .update(&mut visual, |shell, _, cx| {
                shell.workspace.preferences.diagnostics = state;
                shell.check_consent_prompt(cx);
                shell.consent.log_dir = log_dir;
            })
            .unwrap();
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

    fn stored(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> ConsentState {
        window
            .update(visual, |shell, _, _| {
                assert!(!shell.consent.shown, "the sheet closes once answered");
                shell.workspace.preferences.diagnostics.clone()
            })
            .unwrap()
    }

    #[gpui::test]
    fn first_run_shows_the_prompt_and_no_thanks_stores_never(cx: &mut TestAppContext) {
        let (window, mut visual) = launch(ConsentState::default(), None, cx);
        assert!(visual.debug_bounds("consent-prompt").is_some());
        assert!(visual.debug_bounds("consent-privacy").is_some());
        click(&mut visual, "consent-never");
        let state = stored(window, &mut visual);
        let record = state.record.expect("answered");
        assert_eq!(record.crash_reports, CrashReports::Never);
        assert_eq!(record.consent_version, CONSENT_VERSION);
        assert!(!record.metrics);
        assert!(record.decided_at.is_some());
        assert_eq!(state.install_id, None);
        assert!(visual.debug_bounds("consent-prompt").is_none());
    }

    #[gpui::test]
    fn dismissing_counts_as_ask_each_time(cx: &mut TestAppContext) {
        let (window, mut visual) = launch(ConsentState::default(), None, cx);
        click(&mut visual, "consent-dismiss");
        let record = stored(window, &mut visual).record.expect("answered");
        assert_eq!(record.crash_reports, CrashReports::Ask);
        assert!(!record.metrics);
        assert!(record.include_logs);
    }

    #[gpui::test]
    fn send_automatically_opts_in_crash_reports_only(cx: &mut TestAppContext) {
        let (window, mut visual) = launch(ConsentState::default(), None, cx);
        click(&mut visual, "consent-always");
        let state = stored(window, &mut visual);
        let record = state.record.expect("answered");
        assert_eq!(record.crash_reports, CrashReports::Always);
        assert!(!record.metrics, "no button turns on error metrics");
        assert!(state.install_id.is_some());
    }

    #[gpui::test]
    fn version_bump_shows_the_prompt_again(cx: &mut TestAppContext) {
        let old = ConsentState {
            record: Some(Consent {
                consent_version: CONSENT_VERSION - 1,
                crash_reports: CrashReports::Never,
                metrics: true,
                include_logs: false,
                decided_at: Some("2026-01-01T00:00:00.000Z".into()),
            }),
            install_id: Some("c2d8e4a1-7b3f-4e9c-a6d0-5f1b8e2c9a74".into()),
        };
        let (window, mut visual) = launch(old, None, cx);
        assert!(visual.debug_bounds("consent-prompt").is_some());
        click(&mut visual, "consent-ask");
        let state = stored(window, &mut visual);
        let record = state.record.expect("answered");
        assert_eq!(record.consent_version, CONSENT_VERSION);
        assert_eq!(record.crash_reports, CrashReports::Ask);
        assert!(record.metrics, "a re-prompt keeps the Settings choices");
        assert!(!record.include_logs);
        assert_eq!(
            state.install_id.as_deref(),
            Some("c2d8e4a1-7b3f-4e9c-a6d0-5f1b8e2c9a74")
        );
    }

    #[gpui::test]
    fn current_record_shows_no_prompt(cx: &mut TestAppContext) {
        let mut state = ConsentState::default();
        state.answer(Answer::Ask, "2026-10-25T09:00:00.000Z");
        let (window, mut visual) = launch(state, None, cx);
        assert!(visual.debug_bounds("consent-prompt").is_none());
        window
            .update(&mut visual, |shell, _, _| assert!(!shell.consent.shown))
            .unwrap();
    }

    /// Settings sends back its whole copy of the preferences on every
    /// change, so it must hear about the answer or it would undo it.
    #[gpui::test]
    fn an_answer_given_while_settings_is_open_survives_the_next_settings_change(
        cx: &mut TestAppContext,
    ) {
        let mut old = ConsentState::default();
        old.answer(Answer::Always, "2026-01-01T00:00:00.000Z");
        old.record.as_mut().unwrap().consent_version = CONSENT_VERSION - 1;
        let (window, mut visual) = launch(old, None, cx);
        window
            .update(&mut visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::Diagnostics, w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        click(&mut visual, "consent-never");
        click(&mut visual, "settings-toggle-include-logs");
        window
            .update(&mut visual, |shell, _, _| {
                let state = &shell.workspace.preferences.diagnostics;
                let record = state.record.as_ref().unwrap();
                assert_eq!(record.crash_reports, CrashReports::Never);
                assert!(!record.include_logs);
                assert_eq!(state.install_id, None);
            })
            .unwrap();
    }

    #[gpui::test]
    fn no_thanks_empties_the_crash_queue(cx: &mut TestAppContext) {
        let logs = tempfile::tempdir().unwrap();
        let queued = queue_dir(logs.path(), Kind::Crash).join("a.json");
        std::fs::create_dir_all(queued.parent().unwrap()).unwrap();
        std::fs::write(&queued, "{}").unwrap();
        let (_, mut visual) = launch(ConsentState::default(), Some(logs.path().to_owned()), cx);
        click(&mut visual, "consent-never");
        assert!(!queued.exists(), "revoking empties the queue");
    }
}
