//! Pending-review runs: when a review run finishes, Gespan leaves its
//! findings on the pull or merge request as a draft review, then asks in My
//! tasks whether to submit it or keep it as a draft.
use super::automation_runs::Running;
use super::*;
use gespan_core::automations::review_drafts::{KEPT, SUBMITTED};
use gespan_core::review_draft::{self, DraftReview, GitlabHttp};
use gespan_core::review_policy::ReviewTarget;
use std::path::{Path, PathBuf};

/// Runs a forge write off the UI thread. Tests swap it so they never reach
/// `gh` or GitLab.
pub(crate) type DraftForge =
    Arc<dyn Fn(DraftJob) -> Result<Option<DraftReview>, String> + Send + Sync>;

/// What the forge is asked to do.
pub(crate) enum DraftJob {
    Create {
        cwd: String,
        data_dir: PathBuf,
        target: ReviewTarget,
        reply: String,
    },
    Submit {
        cwd: String,
        data_dir: PathBuf,
        draft: DraftReview,
    },
}

/// The real forges: this machine's `gh` and the saved GitLab token.
pub(crate) fn live_forge() -> DraftForge {
    Arc::new(|job| {
        let (cwd, data_dir) = match &job {
            DraftJob::Create { cwd, data_dir, .. } | DraftJob::Submit { cwd, data_dir, .. } => {
                (cwd.clone(), data_dir.clone())
            }
        };
        let mut gh = |args: &[&str], input: Option<&str>| {
            review_draft::run_gh_with_input(Path::new(&cwd), args, input)
        };
        let mut gitlab = || {
            gespan_core::inbox::gitlab::GitlabApi::connect(&data_dir)
                .map(|api| Box::new(api) as Box<dyn GitlabHttp>)
        };
        match job {
            DraftJob::Create { target, reply, .. } => {
                let findings = review_draft::parse_reply(&reply);
                review_draft::create(&target, findings, &mut gh, &mut gitlab).map(Some)
            }
            DraftJob::Submit { draft, .. } => {
                review_draft::submit(&draft, &mut gh, &mut gitlab).map(|()| None)
            }
        }
    })
}

impl Shell {
    fn draft_forge(&self) -> DraftForge {
        self.draft_forge.clone().unwrap_or_else(live_forge)
    }

    /// Reads the drafts still waiting on the user, for My tasks.
    pub(super) fn load_review_drafts(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.automation_state.service.clone() else {
            return;
        };
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations are busy".to_string())?
                .waiting_review_drafts()
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(drafts) => {
                    this.automation_state.review_drafts = drafts;
                    this.refresh_my_tasks(cx);
                    cx.notify();
                }
                Err(error) => tracing::warn!(%error, "draft reviews were not read"),
            });
        })
        .detach();
    }

    /// Leaves a finished review run's findings on `target` as a draft review,
    /// then settles the run.
    pub(super) fn leave_draft_review(
        &mut self,
        run: Running,
        target: ReviewTarget,
        reply: String,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.automation_state.service.clone() else {
            return;
        };
        let data_dir = locked(&self.store)
            .ok()
            .and_then(|store| store.data_dir().map(Path::to_path_buf));
        let forge = self.draft_forge();
        let run_id = run.run_id.clone();
        let work = cx.background_executor().spawn(async move {
            let data_dir = data_dir.ok_or("Gespan has no data folder")?;
            let draft = forge(DraftJob::Create {
                cwd: run.cwd.clone(),
                data_dir,
                target,
                reply,
            })?
            .ok_or("No draft review was left")?;
            service
                .lock()
                .map_err(|_| "Automations are busy".to_string())?
                .save_review_draft(
                    &run.run_id,
                    &run.automation_id,
                    &run.cwd,
                    &run.title,
                    &draft,
                    gespan_core::time::now_ms() as i64,
                )?;
            Ok::<_, String>(draft)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    this.finish_automation_run(run_id, "succeeded", None, cx);
                    this.load_review_drafts(cx);
                }
                Err(error) => {
                    tracing::warn!(%error, "a draft review was not left");
                    this.finish_automation_run(
                        run_id,
                        "failed",
                        Some(format!(
                            "The review is in its session, but Gespan could not leave it as a draft: {error}"
                        )),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Submits a waiting draft, or keeps it as a draft and stops asking.
    pub(super) fn decide_review_draft(
        &mut self,
        run_id: &str,
        submit: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.automation_state.service.clone() else {
            return;
        };
        let Some(saved) = self
            .automation_state
            .review_drafts
            .iter()
            .find(|d| d.run_id == run_id)
            .cloned()
        else {
            return;
        };
        if !self
            .automation_state
            .deciding_drafts
            .insert(run_id.to_string())
        {
            return;
        }
        let data_dir = locked(&self.store)
            .ok()
            .and_then(|store| store.data_dir().map(Path::to_path_buf));
        let forge = self.draft_forge();
        let work = cx.background_executor().spawn(async move {
            if submit {
                forge(DraftJob::Submit {
                    cwd: saved.cwd.clone(),
                    data_dir: data_dir.ok_or("Gespan has no data folder")?,
                    draft: saved.draft.clone(),
                })?;
            }
            service
                .lock()
                .map_err(|_| "Automations are busy".to_string())?
                .settle_review_draft(
                    &saved.run_id,
                    if submit { SUBMITTED } else { KEPT },
                    gespan_core::time::now_ms() as i64,
                )
        });
        let run_id = run_id.to_string();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.automation_state.deciding_drafts.remove(&run_id);
                if let Err(error) = result {
                    tracing::warn!(%error, "a draft review was not settled");
                    this.error = Some(if submit {
                        format!("Could not submit the review: {error}")
                    } else {
                        format!("Could not keep the review as a draft: {error}")
                    });
                }
                this.load_review_drafts(cx);
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub(crate) fn set_draft_forge_for_test(&mut self, forge: DraftForge) {
        self.draft_forge = Some(forge);
    }
}
