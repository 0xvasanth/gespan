//! Settings › Git › Code review on the Shell side: turning a repo's system
//! code review on or off, and keeping the list Settings shows.
use super::automation_runs::service_lock;
use super::*;
use gespan_core::{
    automations::system::{self, ReviewSettings},
    review_policy::ReviewResults,
};

impl Shell {
    /// Reads the system code reviews again and shows them in Settings.
    pub(super) fn refresh_code_reviews(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.automation_state.service.clone() else {
            self.initialize_automations(cx);
            return;
        };
        let work = cx
            .background_executor()
            .spawn(async move { service_lock(&service)?.code_reviews() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(reviews) => {
                    this.automation_state.code_reviews = reviews;
                    this.sync_settings(cx);
                }
                Err(error) => tracing::warn!(%error, "code reviews unavailable"),
            });
        })
        .detach();
    }

    /// Turns `repo`'s code review on (in `folder`) or off. Turning it on the
    /// first time makes it with the project's default reviewer, waiting to
    /// be asked on GitHub and on new merge requests on GitLab; turning it off
    /// keeps it, paused, with its history.
    pub(super) fn set_code_review(
        &mut self,
        repo: String,
        folder: String,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.automation_state.service.clone() else {
            // Applied once automations open, so the click isn't lost.
            self.automation_state.pending_code_review = Some((repo, folder, on));
            self.initialize_automations(cx);
            return;
        };
        let (kind, model) = crate::installed_providers::new_session_choice(
            &self.workspace.preferences,
            Some(&folder),
            cx,
        );
        let defaults = ReviewSettings {
            which: system::which_choices(&repo)[0],
            harness: kind.id().into(),
            model: model.unwrap_or_default(),
            results: ReviewResults::Gespan,
            enabled: true,
        };
        let work = cx.background_executor().spawn(async move {
            let mut service = service_lock(&service)?;
            let id = system::review_id(&repo);
            let existing = service.list()?.into_iter().find(|a| a.id == id);
            match (existing, on) {
                (Some(existing), true) => {
                    let mut settings = system::settings_of(&existing).unwrap_or(defaults);
                    settings.enabled = true;
                    service.ensure_code_review(&repo, &folder, &settings)?;
                }
                (Some(existing), false) => {
                    service.set_enabled(&existing.id, false)?;
                }
                (None, true) => {
                    service.ensure_code_review(&repo, &folder, &defaults)?;
                }
                (None, false) => {}
            }
            Ok::<_, String>(())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    tracing::warn!(%error, "code review change failed");
                    this.inbox_menu_failed(error, cx);
                }
                this.refresh_code_reviews(cx);
                this.refresh_run_views(cx);
            });
        })
        .detach();
    }
}
