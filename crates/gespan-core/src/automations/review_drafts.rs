//! Draft reviews Gespan left on a pull or merge request, kept until the user
//! submits them or keeps them as drafts.

use super::{ReviewDraftRow, Service};
use crate::review_draft::DraftReview;

/// Waiting for the user to submit it or keep it as a draft.
pub const WAITING: &str = "waiting";
/// Submitted on the forge.
pub const SUBMITTED: &str = "submitted";
/// Left as a draft on the forge; Gespan no longer asks.
pub const KEPT: &str = "kept";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedDraft {
    pub run_id: String,
    pub automation_id: String,
    pub cwd: String,
    pub title: String,
    pub draft: DraftReview,
    pub status: String,
    pub created_at: i64,
}

impl Service {
    /// Records the draft a run left. Saving the same run again replaces it.
    pub fn save_review_draft(
        &mut self,
        run_id: &str,
        automation_id: &str,
        cwd: &str,
        title: &str,
        draft: &DraftReview,
        now: i64,
    ) -> Result<(), String> {
        let json = serde_json::to_string(draft).map_err(|e| e.to_string())?;
        let (run_id, automation_id, cwd, title) = (
            run_id.to_owned(),
            automation_id.to_owned(),
            cwd.to_owned(),
            title.to_owned(),
        );
        // Saving the same run again replaces it.
        self.db.run(|mut db| async move {
            ReviewDraftRow::upsert_by_run_id(run_id)
                .automation_id(automation_id)
                .cwd(cwd)
                .title(title)
                .draft_json(json)
                .status(WAITING)
                .created_at(now)
                .updated_at(now)
                .exec(&mut db)
                .await
                .map(drop)
        })
    }

    /// Drafts still waiting on the user, oldest first. A row that no longer
    /// reads is skipped rather than hiding the others.
    pub fn waiting_review_drafts(&mut self) -> Result<Vec<SavedDraft>, String> {
        let rows = self.db.run(|mut db| async move {
            let row = ReviewDraftRow::fields();
            ReviewDraftRow::filter(row.status().eq(WAITING))
                .exec(&mut db)
                .await
        })?;
        let mut rows: Vec<_> = rows
            .into_iter()
            .map(|r| {
                (
                    r.run_id,
                    r.automation_id,
                    r.cwd,
                    r.title,
                    r.draft_json,
                    r.status,
                    r.created_at,
                )
            })
            .collect();
        rows.sort_by(|a, b| (a.6, &a.0).cmp(&(b.6, &b.0)));
        let mut out = Vec::new();
        for (run_id, automation_id, cwd, title, json, status, created_at) in rows {
            match serde_json::from_str(&json) {
                Ok(draft) => out.push(SavedDraft {
                    run_id,
                    automation_id,
                    cwd,
                    title,
                    draft,
                    status,
                    created_at,
                }),
                Err(error) => tracing::warn!(%error, %run_id, "a saved draft review did not read"),
            }
        }
        Ok(out)
    }

    /// Settles a waiting draft as [`SUBMITTED`] or [`KEPT`]. Returns false
    /// when it was not waiting, so a second click does nothing twice.
    pub fn settle_review_draft(
        &mut self,
        run_id: &str,
        status: &str,
        now: i64,
    ) -> Result<bool, String> {
        if status != SUBMITTED && status != KEPT {
            return Err(format!("Unknown draft review status {status}"));
        }
        let (run_id, status) = (run_id.to_owned(), status.to_owned());
        // Only a waiting draft changes; the transaction holds the write lock,
        // so the row read here is the row updated.
        self.db.transaction(|tx| {
            Box::pin(async move {
                let row = ReviewDraftRow::fields();
                let waiting =
                    ReviewDraftRow::filter(row.run_id().eq(&run_id).and(row.status().eq(WAITING)))
                        .first()
                        .exec(&mut *tx)
                        .await?;
                if waiting.is_none() {
                    return Ok(false);
                }
                ReviewDraftRow::filter_by_run_id(&run_id)
                    .update()
                    .status(status)
                    .updated_at(now)
                    .exec(&mut *tx)
                    .await?;
                Ok(true)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{now_millis, AutomationUpsert};
    use super::*;
    use crate::review_draft::diff_map::Side;
    use crate::review_draft::{Handle, ShownComment};
    use crate::review_policy::Forge;

    fn draft(number: u64) -> DraftReview {
        DraftReview {
            forge: Forge::GitHub,
            repo: "acme/app".into(),
            number,
            url: format!("https://github.com/acme/app/pull/{number}"),
            handle: Handle::Github { review_id: number },
            body: "Verdict: fix".into(),
            inline: vec![ShownComment {
                path: "src/lib.rs".into(),
                side: Side::New,
                line: 3,
                body: "Off by one".into(),
            }],
            outside: 1,
        }
    }

    fn service() -> (tempfile::TempDir, Service, String) {
        let directory = tempfile::tempdir().unwrap();
        let mut service = Service::open(&directory.path().join("sessions.sqlite")).unwrap();
        let mut upsert = AutomationUpsert::new("/repo".into(), "claude".into(), "m".into());
        upsert.name = "Review".into();
        upsert.prompt = "Review".into();
        let id = service.upsert(upsert).unwrap().id;
        (directory, service, id)
    }

    #[test]
    fn a_draft_waits_until_settled_once_and_survives_reopening() {
        let (directory, mut service, automation) = service();
        service
            .save_review_draft("run-2", &automation, "/repo", "acme/app#8", &draft(8), 20)
            .unwrap();
        service
            .save_review_draft("run-1", &automation, "/repo", "acme/app#7", &draft(7), 10)
            .unwrap();
        drop(service);
        let mut service = Service::open(&directory.path().join("sessions.sqlite")).unwrap();
        let waiting = service.waiting_review_drafts().unwrap();
        assert_eq!(
            waiting
                .iter()
                .map(|d| d.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-1", "run-2"]
        );
        assert_eq!(waiting[0].draft, draft(7));
        assert!(service
            .settle_review_draft("run-1", SUBMITTED, now_millis())
            .unwrap());
        // A second answer to the same draft does nothing.
        assert!(!service
            .settle_review_draft("run-1", KEPT, now_millis())
            .unwrap());
        assert!(service
            .settle_review_draft("run-2", KEPT, now_millis())
            .unwrap());
        assert!(service.waiting_review_drafts().unwrap().is_empty());
        assert!(service.settle_review_draft("run-2", "deleted", 0).is_err());
    }

    #[test]
    fn a_draft_for_an_automation_that_does_not_exist_is_refused() {
        let (_directory, mut service, _automation) = service();
        let error = service
            .save_review_draft(
                "run-9",
                "no-such-automation",
                "/repo",
                "acme/app#9",
                &draft(9),
                5,
            )
            .unwrap_err();
        assert!(error.contains("FOREIGN KEY"), "{error}");
        assert!(service.waiting_review_drafts().unwrap().is_empty());
    }

    #[test]
    fn deleting_the_automation_drops_its_drafts() {
        let (_directory, mut service, automation) = service();
        service
            .save_review_draft("run-1", &automation, "/repo", "t", &draft(7), 1)
            .unwrap();
        service.delete(&automation).unwrap();
        assert!(service.waiting_review_drafts().unwrap().is_empty());
    }
}
