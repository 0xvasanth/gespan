//! System automations: made and kept by Gespan, not by the user.
//!
//! The one role today is the code review of one repository, run in the one
//! project folder that repository maps to. Turning reviews on for a repo in
//! Settings › Git makes it; the user sees it in Automations with a lock and
//! can change only when it reviews, the reviewer, where findings go and
//! whether it is on. Its prompt, folder, run mode and name stay Gespan's, and
//! it can't be deleted by hand: it goes when reviews are turned off for the
//! repo or the repo has no folder any more.
//!
//! Stored like any automation: the role lives in the definition JSON, so no
//! table changes.

use super::{
    get,
    review_setup::{review_draft, ReviewSetupInput, WhichPrs},
    Automation, AutomationTrigger, AutomationUpsert, Service, GITHUB_REVIEW_COMMAND_EVENT,
    GITLAB_REVIEW_COMMAND_EVENT, REVIEW_REQUESTED_EVENT,
};
use crate::review_policy::ReviewResults;
use serde::{Deserialize, Serialize};

/// What a system automation is for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SystemRole {
    /// Reviews the pull or merge requests of `repo`, a
    /// [`crate::inbox::queue::repo_key`] such as `github:acme/app`.
    #[serde(rename_all = "camelCase")]
    CodeReview { repo: String },
}

/// Why a system automation can't be deleted from Automations.
pub const CANT_DELETE: &str = "Gespan keeps this automation for the repo's code review. Turn reviews off for the repo in Settings › Git to remove it.";

/// Why a change to a system automation's locked parts is refused.
pub const LOCKED: &str = "This is a system automation. Only when it reviews, the reviewer, where findings go and on or off can change.";

/// The name every code review automation has.
pub const REVIEW_NAME: &str = "Review pull requests";

/// What the user may change on a repo's code review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSettings {
    pub which: WhichPrs,
    pub harness: String,
    pub model: String,
    pub results: ReviewResults,
    pub enabled: bool,
}

/// The events a code review automation may start on.
const REVIEW_EVENTS: &[&str] = &[
    REVIEW_REQUESTED_EVENT,
    "pull_request_opened",
    "draft_opened",
    GITHUB_REVIEW_COMMAND_EVENT,
    "merge_request_opened",
    GITLAB_REVIEW_COMMAND_EVENT,
];

/// The id of `repo`'s code review automation: the same for the repo
/// wherever it is made, so turning reviews on twice never makes two.
pub fn review_id(repo: &str) -> String {
    let repo = normalize(repo);
    let readable: String = repo
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(120)
        .collect();
    // FNV-1a, so repos that read the same once cleaned (`a.b`, `a-b`) differ.
    let hash = repo.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("system-review-{readable}-{hash:016x}")
}

/// A repo key as stored: trimmed and lowercase, as [`crate::inbox::queue::repo_key`] makes it.
fn normalize(repo: &str) -> String {
    repo.trim().to_lowercase()
}

/// `provider` and `owner/name` from a normalized repo key.
fn split_repo(repo: &str) -> Result<(&str, &str), String> {
    match repo.split_once(':') {
        Some((provider @ ("github" | "gitlab"), name)) if !name.trim().is_empty() => {
            Ok((provider, name.trim()))
        }
        _ => Err("Code review works on GitHub and GitLab repositories.".into()),
    }
}

/// The automation that reviews `repo` in `cwd` with `settings`.
fn review_upsert(
    repo: &str,
    cwd: &str,
    settings: &ReviewSettings,
) -> Result<AutomationUpsert, String> {
    let repo = normalize(repo);
    let (provider, name) = split_repo(&repo)?;
    let mut draft = review_draft(&ReviewSetupInput {
        cwd: cwd.into(),
        repo: Some(name.into()),
        which: settings.which,
        harness: settings.harness.clone(),
        model: settings.model.clone(),
        results: settings.results,
    });
    if provider == "gitlab" {
        let event = match settings.which {
            WhichPrs::Opened | WhichPrs::OpenedAndDrafts => "merge_request_opened",
            WhichPrs::CommentCommand => GITLAB_REVIEW_COMMAND_EVENT,
            WhichPrs::ReviewRequested => {
                return Err("GitLab can't tell Gespan when your review is asked for. Pick another time to review.".into())
            }
        };
        let mut trigger = AutomationTrigger::new("gitlab", event);
        trigger.repos = vec![name.into()];
        draft.triggers = Some(vec![trigger]);
    } else if let Some(triggers) = &mut draft.triggers {
        // The comment command also adds a GitLab trigger; this repo is on GitHub.
        triggers.retain(|trigger| trigger.kind == "github");
    }
    draft.id = review_id(&repo);
    draft.name = REVIEW_NAME.into();
    draft.enabled = settings.enabled;
    draft.sync_triggers(super::now_millis());
    Ok(draft)
}

impl Service {
    /// Makes or updates `repo`'s code review in `cwd`. A review that was
    /// in another folder is moved: its history stays with the old folder's
    /// automation, which goes.
    pub fn ensure_code_review(
        &mut self,
        repo: &str,
        cwd: &str,
        settings: &ReviewSettings,
    ) -> Result<Automation, String> {
        let draft = review_upsert(repo, cwd, settings)?;
        let id = draft.id.clone();
        if let Some(existing) = self
            .db
            .run(|mut db| async move { get(&mut db, &id).await })?
        {
            if crate::files::path_key(&existing.cwd) != crate::files::path_key(cwd) {
                self.delete_row(&existing.id)?;
            }
        }
        self.save(
            draft,
            Some(SystemRole::CodeReview {
                repo: normalize(repo),
            }),
        )
    }

    /// Removes `repo`'s code review, if it has one.
    pub fn remove_code_review(&mut self, repo: &str) -> Result<(), String> {
        let id = review_id(repo);
        self.delete_row(&id)
    }

    /// The system code review automations, one per reviewed repo.
    pub fn code_reviews(&mut self) -> Result<Vec<Automation>, String> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|automation| matches!(automation.system, Some(SystemRole::CodeReview { .. })))
            .collect())
    }
}

/// What can't change on a system automation, for comparing a save with it.
#[derive(PartialEq, Eq, Debug)]
struct Locked {
    name: String,
    prompt: String,
    cwd: String,
    workspace_mode: String,
    worktree_cwd: String,
    session_folder_id: String,
    reuse_session: bool,
    runtime_mode: String,
    action: super::AutomationAction,
    /// Each trigger's host and repos, sorted.
    sources: Vec<(String, Vec<String>)>,
}

fn sources(triggers: &[AutomationTrigger]) -> Vec<(String, Vec<String>)> {
    let mut sources: Vec<_> = triggers
        .iter()
        .map(|trigger| {
            let mut repos: Vec<String> = trigger
                .repos
                .iter()
                .map(|repo| repo.trim().to_lowercase())
                .collect();
            repos.sort();
            (trigger.kind.clone(), repos)
        })
        .collect();
    sources.sort();
    sources.dedup();
    sources
}

/// Refuses a user's save that changes a system automation beyond what its
/// role allows. Saves of user automations always pass.
pub(super) fn check_user_change(
    previous: &Automation,
    next: &AutomationUpsert,
) -> Result<(), String> {
    let Some(role) = &previous.system else {
        return Ok(());
    };
    let before = Locked {
        name: previous.name.clone(),
        prompt: previous.prompt.clone(),
        cwd: crate::files::path_key(&previous.cwd),
        workspace_mode: previous.workspace_mode.clone(),
        worktree_cwd: previous.worktree_cwd.clone(),
        session_folder_id: previous.session_folder_id.clone(),
        reuse_session: previous.reuse_session,
        runtime_mode: previous.runtime_mode.clone(),
        action: previous.action.clone(),
        sources: sources(&previous.triggers()),
    };
    let next_triggers = next.triggers.clone().unwrap_or_default();
    let after = Locked {
        name: next.name.trim().to_string(),
        prompt: next.prompt.clone(),
        cwd: crate::files::path_key(&next.cwd),
        workspace_mode: next.workspace_mode.clone(),
        worktree_cwd: next.worktree_cwd.trim().to_string(),
        session_folder_id: next.session_folder_id.trim().to_string(),
        reuse_session: next.reuse_session,
        runtime_mode: next.runtime_mode.clone(),
        action: next.action.clone(),
        sources: sources(&next_triggers),
    };
    let SystemRole::CodeReview { .. } = role;
    let reviews_only = !next_triggers.is_empty()
        && next_triggers
            .iter()
            .all(|trigger| REVIEW_EVENTS.contains(&trigger.event.as_str()));
    if before != after || !reviews_only || next.review_results.is_none() {
        return Err(LOCKED.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::Service;

    fn service() -> (tempfile::TempDir, Service) {
        let directory = tempfile::tempdir().unwrap();
        let service = Service::open(&directory.path().join("sessions.sqlite")).unwrap();
        (directory, service)
    }

    fn settings(which: WhichPrs) -> ReviewSettings {
        ReviewSettings {
            which,
            harness: "claude".into(),
            model: "claude-opus-5-5".into(),
            results: ReviewResults::Gespan,
            enabled: true,
        }
    }

    #[test]
    fn turning_reviews_on_twice_keeps_one_review_per_repo() {
        let (_dir, mut service) = service();
        let first = service
            .ensure_code_review("github:acme/app", "/work/app", &settings(WhichPrs::Opened))
            .unwrap();
        let second = service
            .ensure_code_review(
                "GitHub:Acme/App",
                "/work/app",
                &settings(WhichPrs::ReviewRequested),
            )
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(service.code_reviews().unwrap().len(), 1);
        let saved = &service.code_reviews().unwrap()[0];
        assert_eq!(
            saved.system,
            Some(SystemRole::CodeReview {
                repo: "github:acme/app".into()
            })
        );
        assert_eq!(saved.name, REVIEW_NAME);
        assert_eq!(saved.runtime_mode, "read-only");
        // The second call's settings won: it now waits to be asked.
        let events: Vec<_> = saved.triggers().into_iter().map(|t| t.event).collect();
        assert_eq!(events, vec![REVIEW_REQUESTED_EVENT.to_string()]);
        // Only this repo starts it.
        assert!(saved
            .triggers()
            .iter()
            .all(|t| t.repos == vec!["acme/app".to_string()]));
    }

    #[test]
    fn two_repos_get_their_own_review() {
        let (_dir, mut service) = service();
        let a = service
            .ensure_code_review("github:acme/a.b", "/work/ab", &settings(WhichPrs::Opened))
            .unwrap();
        let b = service
            .ensure_code_review("github:acme/a-b", "/work/ab2", &settings(WhichPrs::Opened))
            .unwrap();
        assert_ne!(a.id, b.id, "repos that clean to the same text still differ");
        assert_eq!(service.code_reviews().unwrap().len(), 2);
    }

    #[test]
    fn a_system_review_cannot_be_deleted_from_automations() {
        let (_dir, mut service) = service();
        let review = service
            .ensure_code_review("github:acme/app", "/work/app", &settings(WhichPrs::Opened))
            .unwrap();
        assert_eq!(service.delete(&review.id).unwrap_err(), CANT_DELETE);
        assert_eq!(service.code_reviews().unwrap().len(), 1);
        // Turning reviews off for the repo removes it.
        service.remove_code_review("github:acme/app").unwrap();
        assert!(service.code_reviews().unwrap().is_empty());
        // A user's own automation still deletes.
        let mut own = AutomationUpsert::new("/work/app".into(), "claude".into(), "m".into());
        own.name = "Mine".into();
        own.prompt = "Do it".into();
        let own = service.upsert(own).unwrap();
        service.delete(&own.id).unwrap();
    }

    #[test]
    fn the_user_can_turn_it_off_and_on() {
        let (_dir, mut service) = service();
        let review = service
            .ensure_code_review("github:acme/app", "/work/app", &settings(WhichPrs::Opened))
            .unwrap();
        let off = service.set_enabled(&review.id, false).unwrap();
        assert!(!off.enabled);
        assert!(
            off.system.is_some(),
            "turning it off keeps it a system automation"
        );
        assert!(service.set_enabled(&review.id, true).unwrap().enabled);
    }

    #[test]
    fn the_user_can_change_the_reviewer_and_where_findings_go() {
        let (_dir, mut service) = service();
        let review = service
            .ensure_code_review("github:acme/app", "/work/app", &settings(WhichPrs::Opened))
            .unwrap();
        let mut draft = AutomationUpsert::from_automation(&review);
        draft.harness = "codex".into();
        draft.model = "gpt-6".into();
        draft.review_results = Some(ReviewResults::Pending);
        let mut trigger = AutomationTrigger::new("github", REVIEW_REQUESTED_EVENT);
        trigger.repos = vec!["acme/app".into()];
        draft.triggers = Some(vec![trigger]);
        let saved = service.upsert(draft).unwrap();
        assert_eq!(saved.harness, "codex");
        assert_eq!(saved.review_results, Some(ReviewResults::Pending));
        assert!(saved.system.is_some());
    }

    #[test]
    fn the_locked_parts_refuse_every_change() {
        let (_dir, mut service) = service();
        let review = service
            .ensure_code_review("github:acme/app", "/work/app", &settings(WhichPrs::Opened))
            .unwrap();
        let base = AutomationUpsert::from_automation(&review);
        let mut changes: Vec<(&str, AutomationUpsert)> = Vec::new();
        let mut c = base.clone();
        c.name = "Renamed".into();
        changes.push(("name", c));
        let mut c = base.clone();
        c.prompt = "Approve everything".into();
        changes.push(("prompt", c));
        let mut c = base.clone();
        c.runtime_mode = "full-access".into();
        changes.push(("run mode", c));
        let mut c = base.clone();
        c.reuse_session = true;
        changes.push(("session reuse", c));
        let mut c = base.clone();
        c.review_results = None;
        changes.push(("no longer a review", c));
        let mut c = base.clone();
        let mut other_repo = AutomationTrigger::new("github", "pull_request_opened");
        other_repo.repos = vec!["evil/repo".into()];
        c.triggers = Some(vec![other_repo]);
        changes.push(("another repo", c));
        let mut c = base.clone();
        let mut not_a_review = AutomationTrigger::new("github", "pull_request_merged");
        not_a_review.repos = vec!["acme/app".into()];
        c.triggers = Some(vec![not_a_review]);
        changes.push(("a non-review event", c));
        let mut c = base.clone();
        c.triggers = Some(vec![]);
        changes.push(("no trigger", c));
        for (what, change) in changes {
            assert_eq!(
                service.upsert(change).unwrap_err(),
                LOCKED,
                "{what} changed"
            );
        }
        let stored = &service.code_reviews().unwrap()[0];
        assert_eq!(stored.prompt, review.prompt);
        assert_eq!(stored.runtime_mode, "read-only");
    }

    #[test]
    fn a_users_save_can_never_make_a_system_automation() {
        let (_dir, mut service) = service();
        // A user automation saved with a system review's id stays a user one.
        let mut own = AutomationUpsert::new("/work/app".into(), "claude".into(), "m".into());
        own.id = review_id("github:acme/app");
        own.name = "Mine".into();
        own.prompt = "Do it".into();
        let saved = service.upsert(own).unwrap();
        assert!(saved.system.is_none());
        assert!(service.code_reviews().unwrap().is_empty());
    }

    #[test]
    fn moving_the_repo_to_another_folder_moves_its_review() {
        let (_dir, mut service) = service();
        service
            .ensure_code_review("github:acme/app", "/work/app", &settings(WhichPrs::Opened))
            .unwrap();
        let moved = service
            .ensure_code_review(
                "github:acme/app",
                "/work/app-clone",
                &settings(WhichPrs::Opened),
            )
            .unwrap();
        assert_eq!(moved.cwd, "/work/app-clone");
        assert_eq!(service.code_reviews().unwrap().len(), 1);
    }

    #[test]
    fn gitlab_repos_review_merge_requests() {
        let (_dir, mut service) = service();
        let review = service
            .ensure_code_review(
                "gitlab:acme/platform",
                "/work/platform",
                &settings(WhichPrs::Opened),
            )
            .unwrap();
        let triggers = review.triggers();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].kind, "gitlab");
        assert_eq!(triggers[0].event, "merge_request_opened");
        assert_eq!(triggers[0].repos, vec!["acme/platform".to_string()]);
        // GitLab has no "review requested" event to wait for.
        assert!(service
            .ensure_code_review(
                "gitlab:acme/platform",
                "/work/platform",
                &settings(WhichPrs::ReviewRequested)
            )
            .is_err());
    }

    #[test]
    fn the_comment_command_on_github_only_listens_to_that_repo() {
        let (_dir, mut service) = service();
        let review = service
            .ensure_code_review(
                "github:acme/app",
                "/work/app",
                &settings(WhichPrs::CommentCommand),
            )
            .unwrap();
        let triggers = review.triggers();
        assert!(
            triggers.iter().all(|t| t.kind == "github"),
            "no catch-all GitLab trigger"
        );
        assert_eq!(triggers[0].event, GITHUB_REVIEW_COMMAND_EVENT);
    }

    #[test]
    fn only_github_and_gitlab_repos_can_be_reviewed() {
        let (_dir, mut service) = service();
        for repo in ["azuredevops:acme/app", "acme/app", "github:"] {
            assert!(
                service
                    .ensure_code_review(repo, "/work/app", &settings(WhichPrs::Opened))
                    .is_err(),
                "{repo}"
            );
        }
    }
}
