//! Events on Inbox items that automations wait for: comments on pull and
//! merge requests, failed checks, review requests, merges and assignments.
//!
//! The Inbox lists items, not what happens on them. On each refresh,
//! [`watch_events`] asks the provider about the listed items in the projects
//! an automation watches, and turns what it finds into items carrying an
//! [`Occurrence`]. Those go through the same claim ledger as other events, so
//! each one starts a run once.
use super::{Checks, Comment, Item, Occurrence, Service};
use crate::automations::{
    Watch, CHECK_FAILED_EVENT, GITHUB_COMMENT_EVENT, GITLAB_COMMENT_EVENT, ISSUE_ASSIGNED_EVENT,
    MERGED_EVENT, REVIEW_REQUESTED_EVENT,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The most of a comment an automation is given.
pub const MAX_OCCURRENCE_TEXT: usize = 20_000;
/// Open pull requests looked at per repository for checks and review requests.
const OPEN_LIMIT: u32 = 50;
/// Recently merged pull requests looked at per repository.
const MERGED_LIMIT: u32 = 20;
/// Assigned issues looked at per repository.
const ASSIGNED_LIMIT: u32 = 30;

/// What a provider is asked while watching. [`LiveEvents`] asks the real one;
/// tests answer from memory.
pub trait EventSource {
    /// The connected account on `provider`, as seen from `project`.
    fn login(&mut self, provider: &str, project: &str) -> Result<String, String>;
    /// Every comment on the pull or merge request `item`, replies included.
    fn comments(&mut self, item: &Item) -> Result<Vec<Comment>, String>;
    /// Open pull requests in `repo` with their head commit and checks.
    /// `review_requested` keeps only those awaiting the account's review.
    fn open_pull_requests(
        &mut self,
        project: &str,
        repo: &str,
        review_requested: bool,
    ) -> Result<Vec<(Item, Checks)>, String>;
    /// Recently merged pull requests in `repo`, with when and by whom.
    fn merged_pull_requests(
        &mut self,
        project: &str,
        repo: &str,
    ) -> Result<Vec<(Item, String, String)>, String>;
    /// Open issues in `repo` assigned to the account.
    fn assigned_issues(&mut self, project: &str, repo: &str) -> Result<Vec<Item>, String>;
}

/// What one watch produced: event items to claim, the pull requests whose
/// comments were read (by key, with their update time then), and what failed.
#[derive(Debug, Default)]
pub struct Watched {
    pub items: Vec<Item>,
    pub read: Vec<(String, i64)>,
    pub errors: Vec<String>,
}

/// Looks for new events on `listed` for `watches`. Comments are read only
/// for open pull requests that changed since `read` last saw them; the other
/// events need one or two list calls per watched repository. A failed call
/// is reported in `errors` and tried again next time.
pub fn watch_events(
    watches: &[Watch],
    listed: &[Item],
    read: &HashMap<String, i64>,
    source: &mut impl EventSource,
) -> Watched {
    let mut out = Watched::default();
    let mut logins: HashMap<(String, String), Result<String, String>> = HashMap::new();
    let mut login =
        |source: &mut dyn FnMut() -> Result<String, String>, provider: &str, project: &str| {
            logins
                .entry((provider.to_string(), project.to_string()))
                .or_insert_with(source)
                .clone()
        };
    let watched = |provider: &str, event: &str, item: &Item| {
        let project = crate::files::path_key(&item.project_path);
        watches
            .iter()
            .filter(|watch| {
                watch.provider == provider
                    && watch.event == event
                    && crate::files::path_key(&watch.cwd) == project
            })
            .map(|watch| watch.since)
            .min()
    };

    // Comments, per changed open pull or merge request.
    for item in listed {
        if item.occurrence.is_some()
            || item.kind != "pr"
            || item.number <= 0
            || item.repo.trim().is_empty()
            || !matches!(item.status(), "Open" | "Draft")
        {
            continue;
        }
        let event = match item.provider.as_str() {
            "github" => GITHUB_COMMENT_EVENT,
            "gitlab" => GITLAB_COMMENT_EVENT,
            _ => continue,
        };
        let Some(since) = watched(&item.provider, event, item) else {
            continue;
        };
        let updated = item.updated_ms();
        if updated != 0 && updated < since {
            continue;
        }
        if read.get(&item.key()).is_some_and(|seen| updated <= *seen) {
            continue;
        }
        let me = match login(
            &mut || source.login(&item.provider, &item.project_path),
            &item.provider,
            &item.project_path,
        ) {
            Ok(me) => me,
            Err(error) => {
                out.errors.push(error);
                continue;
            }
        };
        match source.comments(item) {
            Ok(comments) => {
                out.items.extend(comment_items(item, &comments, &me));
                out.read.push((item.key(), updated));
            }
            Err(error) => out.errors.push(error),
        }
    }

    // The rest, per watched GitHub repository the Inbox lists.
    let mut repos = Vec::new();
    let mut seen = HashSet::new();
    for item in listed {
        if item.provider == "github"
            && item.occurrence.is_none()
            && !item.repo.trim().is_empty()
            && seen.insert((
                crate::files::path_key(&item.project_path),
                item.repo.trim().to_lowercase(),
            ))
        {
            repos.push(item);
        }
    }
    for listed in repos {
        let (project, repo) = (listed.project_path.as_str(), listed.repo.trim());
        let wants = |event| watched("github", event, listed).is_some();
        let here = |mut item: Item| {
            item.provider = "github".into();
            item.project_path = project.to_string();
            item
        };
        if wants(CHECK_FAILED_EVENT) {
            match source.open_pull_requests(project, repo, false) {
                Ok(prs) => {
                    for (pr, checks) in prs {
                        out.items.extend(check_failed_items(&here(pr), &checks));
                    }
                }
                Err(error) => out.errors.push(error),
            }
        }
        if wants(REVIEW_REQUESTED_EVENT) {
            match source.open_pull_requests(project, repo, true) {
                Ok(prs) => {
                    for (pr, checks) in prs {
                        out.items
                            .push(review_requested_item(&here(pr), &checks.head_oid));
                    }
                }
                Err(error) => out.errors.push(error),
            }
        }
        if wants(MERGED_EVENT) {
            match source.merged_pull_requests(project, repo) {
                Ok(prs) => {
                    for (pr, at, by) in prs {
                        out.items.push(merged_item(&here(pr), &at, &by));
                    }
                }
                Err(error) => out.errors.push(error),
            }
        }
        if wants(ISSUE_ASSIGNED_EVENT) {
            match login(&mut || source.login("github", project), "github", project) {
                Ok(me) => match source.assigned_issues(project, repo) {
                    Ok(issues) => {
                        for issue in issues {
                            out.items.push(assigned_item(&here(issue), &me));
                        }
                    }
                    Err(error) => out.errors.push(error),
                },
                Err(error) => out.errors.push(error),
            }
        }
    }
    out
}

fn occurred(source: &Item, at: &str, url: &str, occurrence: Occurrence) -> Item {
    Item {
        provider: source.provider.clone(),
        kind: source.kind.clone(),
        id: occurrence.id.clone(),
        number: source.number,
        title: source.title.clone(),
        url: if url.trim().is_empty() {
            source.url.clone()
        } else {
            url.trim().to_string()
        },
        state: source.state.clone(),
        draft: source.draft,
        created_at: at.to_string(),
        updated_at: at.to_string(),
        repo: source.repo.clone(),
        project_path: source.project_path.clone(),
        occurrence: Some(occurrence),
        ..Default::default()
    }
}

/// The relations to a repository that come with write access.
pub fn can_write(association: &str) -> bool {
    matches!(
        association.trim().to_ascii_uppercase().as_str(),
        "OWNER" | "MEMBER" | "COLLABORATOR"
    )
}

fn same_account(author: &str, me: &str) -> bool {
    let me = me.trim().trim_start_matches('@');
    !me.is_empty() && author.trim().eq_ignore_ascii_case(me)
}

/// The comments on pull or merge request `pr` as event items, oldest first,
/// replies after the comment they answer. `me` is the connected account.
pub fn comment_items(pr: &Item, comments: &[Comment], me: &str) -> Vec<Item> {
    let event = if pr.provider == "gitlab" {
        GITLAB_COMMENT_EVENT
    } else {
        GITHUB_COMMENT_EVENT
    };
    let mut flat = Vec::new();
    for comment in comments {
        flat.push(comment);
        flat.extend(comment.replies.iter());
    }
    flat.into_iter()
        .filter(|comment| !comment.id.trim().is_empty())
        .filter_map(|comment| {
            let mut body = comment.body.trim().to_string();
            if body.is_empty() && comment.kind == "review" {
                body = match comment.state.to_ascii_uppercase().as_str() {
                    "APPROVED" => "Approved these changes.".into(),
                    "CHANGES_REQUESTED" => "Requested changes.".into(),
                    _ => String::new(),
                };
            }
            if body.is_empty() {
                return None;
            }
            if !comment.path.trim().is_empty() {
                let line = comment
                    .line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default();
                body = format!("On {}{line}:\n{body}", comment.path.trim());
            }
            Some(occurred(
                pr,
                &comment.created_at,
                &comment.url,
                Occurrence {
                    event: event.into(),
                    id: comment.id.trim().into(),
                    author: comment.author.trim().into(),
                    body: body.chars().take(MAX_OCCURRENCE_TEXT).collect(),
                    mine: same_account(&comment.author, me),
                    trusted: can_write(&comment.author_association),
                },
            ))
        })
        .collect()
}

/// One event per failing check on `pr`'s head commit. A check that fails
/// again after a new push is a new event; one still failing is not.
pub fn check_failed_items(pr: &Item, checks: &Checks) -> Vec<Item> {
    let head = checks.head_oid.trim();
    let short: String = head.chars().take(7).collect();
    checks
        .checks
        .iter()
        .filter(|check| check.state == "fail" && !check.name.trim().is_empty())
        .map(|check| {
            let name = check.name.trim();
            let workflow = check.workflow.trim();
            let label = if workflow.is_empty() || workflow == name {
                name.to_string()
            } else {
                format!("{workflow} / {name}")
            };
            let at = check
                .completed_at
                .as_deref()
                .or(check.started_at.as_deref())
                .filter(|at| !at.trim().is_empty())
                .unwrap_or(&pr.updated_at);
            occurred(
                pr,
                at,
                check.url.as_deref().unwrap_or_default(),
                Occurrence {
                    event: CHECK_FAILED_EVENT.into(),
                    id: format!("{head}:{label}"),
                    author: String::new(),
                    body: if short.is_empty() {
                        format!("Check \"{label}\" failed.")
                    } else {
                        format!("Check \"{label}\" failed on commit {short}.")
                    },
                    mine: false,
                    trusted: true,
                },
            )
        })
        .collect()
}

/// `pr` awaits the account's review. A request still open after new commits
/// are pushed is asked again: there is something new to review.
pub fn review_requested_item(pr: &Item, head: &str) -> Item {
    occurred(
        pr,
        &pr.updated_at,
        "",
        Occurrence {
            event: REVIEW_REQUESTED_EVENT.into(),
            id: if head.trim().is_empty() {
                "review".into()
            } else {
                head.trim().into()
            },
            author: String::new(),
            body: "Your review was requested.".into(),
            mine: false,
            trusted: true,
        },
    )
}

/// `pr` was merged at `at` by `by`. Merging your own pull request is the
/// usual end of one, not an agent waking itself, so it always counts.
pub fn merged_item(pr: &Item, at: &str, by: &str) -> Item {
    let at = if at.trim().is_empty() {
        &pr.updated_at
    } else {
        at
    };
    occurred(
        pr,
        at,
        "",
        Occurrence {
            event: MERGED_EVENT.into(),
            id: "merged".into(),
            author: by.trim().into(),
            body: if by.trim().is_empty() {
                "Merged.".into()
            } else {
                format!("Merged by @{}.", by.trim())
            },
            mine: false,
            trusted: true,
        },
    )
}

/// `issue` is assigned to `me`. GitHub's list says nothing about when, so
/// its last update stands in.
pub fn assigned_item(issue: &Item, me: &str) -> Item {
    occurred(
        issue,
        &issue.updated_at,
        "",
        Occurrence {
            event: ISSUE_ASSIGNED_EVENT.into(),
            id: me.trim().trim_start_matches('@').to_lowercase(),
            author: String::new(),
            body: format!("Assigned to @{}.", me.trim().trim_start_matches('@')),
            mine: false,
            trusted: true,
        },
    )
}

/// Asks the real providers, through the Inbox's own calls. GitHub's list
/// calls run on this machine's `gh`; a project on another machine is
/// watched for comments only, and only once its account is known.
pub struct LiveEvents {
    service: Service,
    /// GitLab access levels already looked up, by repository and username.
    access: HashMap<(String, String), i64>,
}

impl LiveEvents {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            access: HashMap::new(),
        }
    }
    fn local(project: &str) -> Result<&Path, String> {
        let path = Path::new(project);
        if crate::files::remote::is_remote(path) {
            return Err(format!(
                "{project} is on another machine: only its comments are watched"
            ));
        }
        Ok(path)
    }
}

impl LiveEvents {
    fn mark_gitlab_access(&mut self, repo: &str, comment: &mut Comment) -> Result<(), String> {
        let author = comment.author.trim().to_lowercase();
        if author.is_empty() {
            return Ok(());
        }
        let key = (repo.to_lowercase(), author.clone());
        let level = match self.access.get(&key) {
            Some(level) => *level,
            None => {
                let level = super::gitlab::gitlab_member_access(
                    self.service.data_dir.clone(),
                    repo.to_string(),
                    author,
                )?;
                self.access.insert(key, level);
                level
            }
        };
        comment.author_association = gitlab_association(level).into();
        Ok(())
    }
}

/// A GitLab access level as GitHub would name the relation: Developer (30)
/// and above can push.
pub fn gitlab_association(level: i64) -> &'static str {
    if level >= 30 {
        "MEMBER"
    } else {
        "NONE"
    }
}

impl EventSource for LiveEvents {
    fn login(&mut self, provider: &str, project: &str) -> Result<String, String> {
        match provider {
            "gitlab" => super::gitlab::gitlab_current_user(self.service.data_dir.clone()),
            _ => super::github::git_github_login_for(&super::gh_root(Self::local(project)?)),
        }
    }
    fn comments(&mut self, item: &Item) -> Result<Vec<Comment>, String> {
        let mut comments = self.service.thread(item)?.comments;
        // GitHub says how each author relates to the repository; GitLab's
        // comments do not, so their authors' access is looked up.
        if item.provider == "gitlab" {
            for comment in &mut comments {
                self.mark_gitlab_access(&item.repo, comment)?;
                for reply in &mut comment.replies {
                    self.mark_gitlab_access(&item.repo, reply)?;
                }
            }
        }
        Ok(comments)
    }
    fn open_pull_requests(
        &mut self,
        project: &str,
        repo: &str,
        review_requested: bool,
    ) -> Result<Vec<(Item, Checks)>, String> {
        let root = super::gh_root(Self::local(project)?);
        let search = if review_requested {
            "review-requested:@me"
        } else {
            ""
        };
        super::github::git_github_pr_states_for(&root, repo, "open", search, OPEN_LIMIT)?
            .into_iter()
            .map(|pr| {
                Ok((
                    super::convert(pr.item)?,
                    Checks {
                        head_oid: pr.head_oid,
                        checks: super::convert(pr.checks)?,
                    },
                ))
            })
            .collect()
    }
    fn merged_pull_requests(
        &mut self,
        project: &str,
        repo: &str,
    ) -> Result<Vec<(Item, String, String)>, String> {
        let root = super::gh_root(Self::local(project)?);
        super::github::git_github_pr_states_for(&root, repo, "merged", "", MERGED_LIMIT)?
            .into_iter()
            .map(|pr| Ok((super::convert(pr.item)?, pr.merged_at, pr.merged_by)))
            .collect()
    }
    fn assigned_issues(&mut self, project: &str, repo: &str) -> Result<Vec<Item>, String> {
        let root = super::gh_root(Self::local(project)?);
        super::convert(super::github::git_github_work_items_for(
            &root,
            repo,
            "issue",
            true,
            "open",
            "",
            ASSIGNED_LIMIT,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::Check;

    fn pr(provider: &str, number: i64, updated: &str) -> Item {
        Item {
            provider: provider.into(),
            kind: "pr".into(),
            number,
            title: format!("PR {number}"),
            url: format!("https://example.com/acme/app/pull/{number}"),
            state: "open".into(),
            updated_at: updated.into(),
            repo: "acme/app".into(),
            project_path: "/repo".into(),
            ..Default::default()
        }
    }
    fn said(id: &str, kind: &str, author: &str, body: &str) -> Comment {
        Comment {
            id: id.into(),
            kind: kind.into(),
            author: author.into(),
            author_association: "MEMBER".into(),
            body: body.into(),
            created_at: "2026-11-11T10:00:00Z".into(),
            url: format!("https://example.com/c/{id}"),
            ..Default::default()
        }
    }
    fn watch(provider: &str, event: &str, since: i64) -> Watch {
        Watch {
            provider: provider.into(),
            event: event.into(),
            cwd: "/repo".into(),
            since,
        }
    }

    #[test]
    fn every_kind_of_comment_and_reply_is_its_own_event() {
        let mut thread = said("t1", "review_comment", "alice", "Rename this");
        thread.path = "src/login.rs".into();
        thread.line = Some(42);
        thread.replies = vec![said("t2", "review_comment", "bob", "Agreed")];
        let mut approved = said("r1", "review", "carol", "");
        approved.state = "APPROVED".into();
        let mut silent = said("r2", "review", "carol", "  ");
        silent.state = "COMMENTED".into();
        let mut stranger = said("c9", "comment", "mallory", "Hi");
        stranger.author_association = "NONE".into();
        let comments = [
            said("c1", "comment", "alice", "Looks good"),
            thread,
            approved,
            silent,
            said("", "comment", "alice", "No id"),
            said("c2", "comment", "Gespan-Bot", "Pushed a fix"),
            stranger,
        ];
        let items = comment_items(&pr("github", 7, ""), &comments, "@gespan-bot");
        let seen: Vec<_> = items
            .iter()
            .map(|item| {
                let o = item.occurrence.as_ref().unwrap();
                (
                    o.id.as_str(),
                    o.author.as_str(),
                    o.body.as_str(),
                    o.mine,
                    o.trusted,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("c1", "alice", "Looks good", false, true),
                (
                    "t1",
                    "alice",
                    "On src/login.rs:42:\nRename this",
                    false,
                    true
                ),
                ("t2", "bob", "Agreed", false, true),
                ("r1", "carol", "Approved these changes.", false, true),
                ("c2", "Gespan-Bot", "Pushed a fix", true, true),
                ("c9", "mallory", "Hi", false, false),
            ]
        );
        let first = &items[0];
        assert_eq!(
            first.occurrence.as_ref().unwrap().event,
            GITHUB_COMMENT_EVENT
        );
        assert_eq!(first.kind, "pr");
        assert_eq!(first.url, "https://example.com/c/c1");
        assert_eq!(first.created_at, "2026-11-11T10:00:00Z");
        assert_eq!(first.project_path, "/repo");
        assert_eq!(first.key(), "github:acme/app:pr:7:pull_request_comment:c1");
        assert_ne!(first.key(), pr("github", 7, "").key());
        // On GitLab the same comment raises GitLab's event.
        let gitlab = comment_items(&pr("gitlab", 7, ""), &comments[..1], "");
        assert_eq!(
            gitlab[0].occurrence.as_ref().unwrap().event,
            GITLAB_COMMENT_EVENT
        );
        // A comment without a link points at the pull request; a huge one is cut.
        let mut long = said(
            "c3",
            "comment",
            "alice",
            &"x".repeat(MAX_OCCURRENCE_TEXT * 2),
        );
        long.url.clear();
        let item = comment_items(&pr("github", 7, ""), &[long], "").remove(0);
        assert_eq!(item.url, "https://example.com/acme/app/pull/7");
        assert_eq!(
            item.occurrence.unwrap().body.chars().count(),
            MAX_OCCURRENCE_TEXT
        );
        // Event items survive being saved for a retry; old ones have none.
        let saved: Item = serde_json::from_str(&serde_json::to_string(first).unwrap()).unwrap();
        assert_eq!(&saved, first);
        assert_eq!(serde_json::from_str::<Item>("{}").unwrap().occurrence, None);
        assert!(!serde_json::to_string(&pr("github", 7, ""))
            .unwrap()
            .contains("occurrence"));
    }

    #[test]
    fn only_failing_checks_raise_events_named_by_workflow_and_commit() {
        let checks = Checks {
            head_oid: "abc1234def".into(),
            checks: vec![
                Check {
                    name: "test".into(),
                    workflow: "CI".into(),
                    state: "fail".into(),
                    completed_at: Some("2026-11-11T09:05:00Z".into()),
                    ..Default::default()
                },
                Check {
                    name: "deploy/preview".into(),
                    state: "fail".into(),
                    started_at: Some("2026-11-11T09:02:00Z".into()),
                    url: Some("https://deploy.example/1".into()),
                    ..Default::default()
                },
                Check {
                    name: "lint".into(),
                    state: "pending".into(),
                    ..Default::default()
                },
                Check {
                    name: "build".into(),
                    state: "pass".into(),
                    ..Default::default()
                },
            ],
        };
        let items = check_failed_items(&pr("github", 7, "2026-11-11T08:00:00Z"), &checks);
        let seen: Vec<_> = items
            .iter()
            .map(|item| {
                let o = item.occurrence.as_ref().unwrap();
                (
                    o.id.as_str(),
                    o.body.as_str(),
                    item.created_at.as_str(),
                    item.url.as_str(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (
                    "abc1234def:CI / test",
                    "Check \"CI / test\" failed on commit abc1234.",
                    "2026-11-11T09:05:00Z",
                    "https://example.com/acme/app/pull/7"
                ),
                (
                    "abc1234def:deploy/preview",
                    "Check \"deploy/preview\" failed on commit abc1234.",
                    "2026-11-11T09:02:00Z",
                    "https://deploy.example/1"
                ),
            ]
        );
    }

    /// Answers from memory and records what it was asked.
    #[derive(Default)]
    struct Fake {
        comments: HashMap<i64, Result<Vec<Comment>, String>>,
        open: Vec<(Item, Checks)>,
        review: Vec<(Item, Checks)>,
        merged: Vec<(Item, String, String)>,
        assigned: Vec<Item>,
        asked: Vec<String>,
    }
    impl EventSource for Fake {
        fn login(&mut self, provider: &str, project: &str) -> Result<String, String> {
            self.asked.push(format!("login {provider} {project}"));
            Ok("gespan-bot".into())
        }
        fn comments(&mut self, item: &Item) -> Result<Vec<Comment>, String> {
            self.asked
                .push(format!("comments {} {}", item.provider, item.number));
            self.comments
                .get(&item.number)
                .cloned()
                .unwrap_or(Ok(Vec::new()))
        }
        fn open_pull_requests(
            &mut self,
            project: &str,
            repo: &str,
            review_requested: bool,
        ) -> Result<Vec<(Item, Checks)>, String> {
            self.asked
                .push(format!("open {project} {repo} review={review_requested}"));
            Ok(if review_requested {
                self.review.clone()
            } else {
                self.open.clone()
            })
        }
        fn merged_pull_requests(
            &mut self,
            project: &str,
            repo: &str,
        ) -> Result<Vec<(Item, String, String)>, String> {
            self.asked.push(format!("merged {project} {repo}"));
            Ok(self.merged.clone())
        }
        fn assigned_issues(&mut self, project: &str, repo: &str) -> Result<Vec<Item>, String> {
            self.asked.push(format!("assigned {project} {repo}"));
            Ok(self.assigned.clone())
        }
    }

    #[test]
    fn comments_are_read_only_for_changed_open_pull_requests_and_failures_are_retried() {
        let since = crate::inbox::timestamp_ms("2026-11-11T09:00:00Z");
        let watches = [
            watch("github", GITHUB_COMMENT_EVENT, since),
            watch("gitlab", GITLAB_COMMENT_EVENT, since),
        ];
        let mut merged = pr("github", 4, "2026-11-11T10:00:00Z");
        merged.state = "merged".into();
        let mut elsewhere = pr("github", 5, "2026-11-11T10:00:00Z");
        elsewhere.project_path = "/other".into();
        let mut issue = pr("github", 6, "2026-11-11T10:00:00Z");
        issue.kind = "issue".into();
        let listed = vec![
            pr("github", 1, "2026-11-11T10:00:00Z"),
            pr("github", 2, "2026-11-11T10:00:00Z"),
            pr("gitlab", 3, "2026-11-11T10:00:00Z"),
            merged,
            elsewhere,
            issue,
            pr("github", 7, "2026-11-11T08:00:00Z"),
        ];
        let mut fake = Fake::default();
        fake.comments
            .insert(1, Ok(vec![said("a", "comment", "alice", "Hi")]));
        fake.comments.insert(2, Err("GitHub is unreachable".into()));
        fake.comments
            .insert(3, Ok(vec![said("b", "comment", "alice", "Hallo")]));
        let mut read = HashMap::new();
        let out = watch_events(&watches, &listed, &read, &mut fake);
        assert_eq!(
            out.items.iter().map(Item::key).collect::<Vec<_>>(),
            [
                "github:acme/app:pr:1:pull_request_comment:a",
                "gitlab:acme/app:pr:3:merge_request_comment:b"
            ]
        );
        assert_eq!(out.errors, ["GitHub is unreachable"]);
        assert_eq!(
            out.read
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            [listed[0].key(), listed[2].key()]
        );
        assert_eq!(
            fake.asked,
            [
                "login github /repo",
                "comments github 1",
                "comments github 2",
                "login gitlab /repo",
                "comments gitlab 3"
            ],
            "the account is looked up once per forge and project"
        );
        // Nothing else is watched, so no list calls were made.
        read.extend(out.read);

        // Next refresh: only the one that failed is asked again, until
        // another one changes.
        let mut fake = Fake::default();
        watch_events(&watches, &listed, &read, &mut fake);
        assert_eq!(fake.asked, ["login github /repo", "comments github 2"]);
        let mut changed = listed.clone();
        changed[0] = pr("github", 1, "2026-11-11T11:00:00Z");
        let mut fake = Fake::default();
        watch_events(&watches, &changed, &read, &mut fake);
        assert_eq!(
            fake.asked,
            [
                "login github /repo",
                "comments github 1",
                "comments github 2"
            ]
        );
        // With nobody watching, nothing is asked at all.
        let mut fake = Fake::default();
        let out = watch_events(&[], &listed, &HashMap::new(), &mut fake);
        assert!(out.items.is_empty() && fake.asked.is_empty());
    }

    #[test]
    fn list_events_are_asked_once_per_watched_repository_and_land_in_its_project() {
        let watches = [
            watch("github", CHECK_FAILED_EVENT, 0),
            watch("github", REVIEW_REQUESTED_EVENT, 0),
            watch("github", MERGED_EVENT, 0),
            watch("github", ISSUE_ASSIGNED_EVENT, 0),
        ];
        // The list gives the repository; the items from gh carry no project.
        let bare = |number: i64, kind: &str| Item {
            kind: kind.into(),
            number,
            title: format!("Item {number}"),
            updated_at: "2026-11-11T10:00:00Z".into(),
            repo: "acme/app".into(),
            ..Default::default()
        };
        let failing = Checks {
            head_oid: "abc".into(),
            checks: vec![Check {
                name: "test".into(),
                state: "fail".into(),
                ..Default::default()
            }],
        };
        let mut fake = Fake {
            open: vec![(bare(1, "pr"), failing.clone())],
            review: vec![(bare(2, "pr"), failing)],
            merged: vec![(bare(3, "pr"), "2026-11-11T10:00:00Z".into(), "alice".into())],
            assigned: vec![bare(4, "issue")],
            ..Default::default()
        };
        let mut other = pr("gitlab", 9, "2026-11-11T10:00:00Z");
        other.repo = "acme/gl".into();
        let listed = vec![
            pr("github", 10, "2026-11-11T10:00:00Z"),
            pr("github", 11, "2026-11-11T10:00:00Z"),
            other,
        ];
        let out = watch_events(&watches, &listed, &HashMap::new(), &mut fake);
        assert_eq!(
            fake.asked,
            [
                "open /repo acme/app review=false",
                "open /repo acme/app review=true",
                "merged /repo acme/app",
                "login github /repo",
                "assigned /repo acme/app"
            ],
            "one repository, asked once, however many of its items are listed"
        );
        let seen: Vec<_> = out
            .items
            .iter()
            .map(|item| {
                (
                    item.provider.as_str(),
                    item.project_path.as_str(),
                    item.number,
                    item.occurrence.as_ref().unwrap().event.as_str(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("github", "/repo", 1, CHECK_FAILED_EVENT),
                ("github", "/repo", 2, REVIEW_REQUESTED_EVENT),
                ("github", "/repo", 3, MERGED_EVENT),
                ("github", "/repo", 4, ISSUE_ASSIGNED_EVENT),
            ]
        );
        // A project nobody watches for these is not asked.
        let mut fake = Fake::default();
        let mut elsewhere = pr("github", 10, "2026-11-11T10:00:00Z");
        elsewhere.project_path = "/other".into();
        watch_events(&watches, &[elsewhere], &HashMap::new(), &mut fake);
        assert!(fake.asked.is_empty());
    }

    #[test]
    fn a_project_on_another_machine_is_not_asked_through_this_machines_gh() {
        let mut live = LiveEvents::new(Service {
            data_dir: std::env::temp_dir(),
            cwd: std::env::temp_dir(),
        });
        let error = live
            .open_pull_requests("remote://box/work/app", "acme/app", false)
            .unwrap_err();
        assert!(error.contains("on another machine"), "{error}");
        assert!(live.login("github", "remote://box/work/app").is_err());
    }
}
