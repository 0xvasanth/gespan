//! The "Review pull requests" automation the two-click setup creates.
//!
//! Pure: builds the draft and finds an existing one. Saving goes through
//! `Service::upsert` like any other automation.

use super::{Automation, AutomationAction, AutomationTrigger, AutomationUpsert};
use crate::review_policy::ReviewResults;

/// What the reviewer is told to look for. Where the review goes is added at
/// launch from `review_policy::instruction`, not here.
pub const REVIEW_PROMPT: &str = "Review this pull request.\n\nCheck for:\n- Correctness bugs and regressions\n- Missing tests for the changed behavior\n- Security or data-loss risks\n- API / contract breakage\n\nBlockers first, then suggestions. Do not nitpick formatting. If the change is good, say so briefly and note residual risk.";

/// Which pull requests start a review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhichPrs {
    ReviewRequested,
    Opened,
    OpenedAndDrafts,
}

#[derive(Clone, Debug)]
pub struct ReviewSetupInput {
    pub cwd: String,
    /// `None` means any repo.
    pub repo: Option<String>,
    pub which: WhichPrs,
    pub harness: String,
    pub model: String,
    pub results: ReviewResults,
}

pub fn review_draft(input: &ReviewSetupInput) -> AutomationUpsert {
    let events: &[&str] = match input.which {
        WhichPrs::ReviewRequested => &["pull_request_review_requested"],
        WhichPrs::Opened => &["pull_request_opened"],
        WhichPrs::OpenedAndDrafts => &["pull_request_opened", "draft_opened"],
    };
    let triggers = events
        .iter()
        .map(|event| {
            let mut trigger = AutomationTrigger::new("github", event);
            trigger.repos = input.repo.iter().cloned().collect();
            trigger
        })
        .collect();
    let mut draft = AutomationUpsert::new(
        input.cwd.clone(),
        input.harness.clone(),
        input.model.clone(),
    );
    draft.name = "Review pull requests".into();
    draft.prompt = REVIEW_PROMPT.into();
    draft.workspace_mode = "current".into();
    draft.reuse_session = false;
    draft.runtime_mode = "read-only".into();
    draft.action = AutomationAction::Session;
    draft.review_results = Some(input.results);
    draft.triggers = Some(triggers);
    draft.sync_triggers(super::now_millis());
    draft
}

/// The review automation already set up for this project, if any.
pub fn find_review_automation<'a>(
    automations: &'a [Automation],
    cwd: &str,
) -> Option<&'a Automation> {
    automations
        .iter()
        .find(|automation| automation.cwd == cwd && automation.review_results.is_some())
}

#[cfg(test)]
mod tests {
    use super::super::{now_millis, write_automation, Service};
    use super::*;

    fn input(which: WhichPrs, results: ReviewResults, repo: Option<&str>) -> ReviewSetupInput {
        ReviewSetupInput {
            cwd: "/repo".into(),
            repo: repo.map(str::to_string),
            which,
            harness: "claude".into(),
            model: "model".into(),
            results,
        }
    }

    #[test]
    fn review_draft_maps_every_choice() {
        for which in [
            WhichPrs::ReviewRequested,
            WhichPrs::Opened,
            WhichPrs::OpenedAndDrafts,
        ] {
            for results in [
                ReviewResults::Gespan,
                ReviewResults::Draft,
                ReviewResults::Post,
            ] {
                for repo in [Some("acme/app"), None] {
                    let draft = review_draft(&input(which, results, repo));
                    assert_eq!(draft.name, "Review pull requests");
                    assert_eq!(draft.workspace_mode, "current");
                    assert!(!draft.reuse_session);
                    assert_eq!(draft.runtime_mode, "read-only");
                    assert_eq!(draft.action, AutomationAction::Session);
                    assert_eq!(draft.review_results, Some(results));
                    assert_eq!(draft.prompt, REVIEW_PROMPT);
                    // Fits a review request as well as a newly opened PR.
                    assert!(draft.prompt.starts_with("Review this pull request.\n"));
                    assert!(!draft.prompt.contains("newly opened"));
                    let expected: &[&str] = match which {
                        WhichPrs::ReviewRequested => &["pull_request_review_requested"],
                        WhichPrs::Opened => &["pull_request_opened"],
                        WhichPrs::OpenedAndDrafts => &["pull_request_opened", "draft_opened"],
                    };
                    let triggers = draft.triggers.unwrap();
                    let events: Vec<&str> = triggers.iter().map(|t| t.event.as_str()).collect();
                    assert_eq!(events, expected);
                    for trigger in &triggers {
                        assert_eq!(trigger.kind, "github");
                        let want: Vec<String> = repo.iter().map(|r| r.to_string()).collect();
                        assert_eq!(trigger.repos, want);
                    }
                }
            }
        }
    }

    #[test]
    fn review_draft_saves_and_reloads() {
        let directory = tempfile::tempdir().unwrap();
        let mut service = Service::open(&directory.path().join("sessions.sqlite")).unwrap();
        let draft = review_draft(&input(
            WhichPrs::Opened,
            ReviewResults::Draft,
            Some("acme/app"),
        ));
        let saved = service.upsert(draft).unwrap();
        let listed = service.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, saved.id);
        assert_eq!(listed[0].review_results, Some(ReviewResults::Draft));
        assert_eq!(listed[0].runtime_mode, "read-only");
    }

    #[test]
    fn automations_without_review_results_load_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let mut service = Service::open(&directory.path().join("sessions.sqlite")).unwrap();
        let mut draft = AutomationUpsert::new("/repo".into(), "codex".into(), "model".into());
        draft.name = "Old".into();
        draft.prompt = "Do it".into();
        let saved = service.upsert(draft).unwrap();
        // Rewrite the stored definition the way an older build wrote it.
        let raw: String = service
            .conn
            .query_row(
                "SELECT definition_json FROM automations WHERE id = ?1",
                [&saved.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!raw.contains("reviewResults"));
        let loaded = service.list().unwrap();
        assert_eq!(loaded[0].review_results, None);
        assert_eq!(loaded[0].runtime_mode, "auto");
        // Saving it again must not invent the field either.
        let mut again = loaded[0].clone();
        again.updated_at = now_millis();
        write_automation(&service.conn, &again).unwrap();
        let raw: String = service
            .conn
            .query_row(
                "SELECT definition_json FROM automations WHERE id = ?1",
                [&saved.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!raw.contains("reviewResults"));
    }

    #[test]
    fn find_review_automation_matches_project_and_kind() {
        let directory = tempfile::tempdir().unwrap();
        let mut service = Service::open(&directory.path().join("sessions.sqlite")).unwrap();
        let mut review_a = review_draft(&input(WhichPrs::Opened, ReviewResults::Gespan, None));
        review_a.cwd = "/a".into();
        let mut plain_a = AutomationUpsert::new("/a".into(), "codex".into(), "model".into());
        plain_a.name = "Plain".into();
        plain_a.prompt = "x".into();
        let mut review_b = review_draft(&input(WhichPrs::Opened, ReviewResults::Post, None));
        review_b.cwd = "/b".into();
        let review_a = service.upsert(review_a).unwrap();
        service.upsert(plain_a).unwrap();
        service.upsert(review_b).unwrap();
        let all = service.list().unwrap();
        let found = find_review_automation(&all, "/a").unwrap();
        assert_eq!(found.id, review_a.id);
        assert!(find_review_automation(&all, "/c").is_none());
    }
}
