//! "Open logs folder" and "Copy diagnostics", shared by the command palette
//! and Settings → Diagnostics. Both stay on this machine.
use super::*;

impl Shell {
    /// Reveals the log folder, or says why it could not.
    pub(super) fn open_logs_folder(&mut self, cx: &mut Context<Self>) {
        let open = cx
            .background_executor()
            .spawn(async { gespan_core::diagnostics::open_logs_folder() });
        cx.spawn(async move |this, cx| {
            if let Err(error) = open.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Open logs folder: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Puts the plain-text summary on the clipboard.
    pub(super) fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        // It scans the log and crash folders: keep that off the UI thread.
        let summary = cx
            .background_executor()
            .spawn(async { gespan_core::diagnostics::summary() });
        cx.spawn(async move |_, cx| {
            let summary = summary.await;
            cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(summary)))
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::Section;
    use gpui::{TestAppContext, VisualTestContext};

    fn click(id: &'static str, visual: &mut VisualTestContext) {
        let bounds = visual.debug_bounds(id).unwrap_or_else(|| panic!("{id}"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }

    #[gpui::test]
    fn settings_diagnostics_buttons_copy_open_and_report(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_settings(Section::Diagnostics, w, cx)
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();

        click("diagnostics-copy", visual);
        assert_eq!(
            visual.read_from_clipboard().and_then(|item| item.text()),
            Some(gespan_core::diagnostics::summary())
        );

        // Tests never start diagnostics, so the folder action says why.
        click("diagnostics-open-logs", visual);
        window
            .update(visual, |shell, _, _| {
                let error = shell.error.clone().unwrap_or_default();
                assert!(error.contains("Diagnostics have not started"), "{error}");
            })
            .unwrap();

        click("diagnostics-report", visual);
        window
            .update(visual, |shell, _, _| {
                assert!(shell.report.open.is_some());
                assert!(shell.settings.is_some(), "the sheet opens over Settings");
            })
            .unwrap();
    }
}
