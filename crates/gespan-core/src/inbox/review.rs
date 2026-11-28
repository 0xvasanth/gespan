//! Code review on the Pull requests page: what a pull or merge request's reviews and
//! checks say, and the host calls a reviewer makes (submit a review, rerun
//! failed checks, find repositories, review requests outside your repos).
//!
//! GitHub, GitLab and Azure DevOps fill the same shapes, so the Pull requests page
//! groups and words every host's requests the same way.
use super::{Item, Service};
use crate::pull_requests::GIT_HOSTS;
use serde::{Deserialize, Serialize};

use std::{
    collections::HashMap,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

/// One reviewer's latest review of a pull request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Review {
    pub login: String,
    /// `approved`, `changes_requested`, `commented` or `dismissed`.
    pub state: String,
}

/// How a pull request's head commit checks stand, counted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CheckSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    /// Queued or running.
    pub pending: u32,
    /// The failed checks' names, as the host shows them.
    pub failed_names: Vec<String>,
}

impl CheckSummary {
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Counts checks by how they ended. [`Outcome::Ignored`] ones (a
    /// superseded run, a policy that does not apply) are left out.
    pub(crate) fn tally(checks: impl IntoIterator<Item = (String, Outcome)>) -> Self {
        let mut summary = Self::default();
        for (name, outcome) in checks {
            match outcome {
                Outcome::Passed => summary.passed += 1,
                Outcome::Failed => {
                    summary.failed += 1;
                    let name = name.trim();
                    if !name.is_empty() && !summary.failed_names.iter().any(|n| n == name) {
                        summary.failed_names.push(name.to_string());
                    }
                }
                Outcome::Pending => summary.pending += 1,
                Outcome::Ignored => continue,
            }
            summary.total += 1;
        }
        summary
    }
}

/// How one check counts in a [`CheckSummary`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Succeeded, or was skipped or neutral: nothing for anyone to do.
    Passed,
    Failed,
    /// Queued or running.
    Pending,
    Ignored,
}

/// What the reviewer is told once a review is sent. GitLab has no review
/// that requests changes, so there the approval is removed and the comment
/// posted, and the notice says so.
pub fn review_notice(provider: &str, verdict: Verdict) -> &'static str {
    match (provider, verdict) {
        ("gitlab", Verdict::RequestChanges) => {
            "GitLab can't request changes here: your approval was removed and your comment posted."
        }
        (_, Verdict::Comment) => "Review comment posted.",
        (_, Verdict::RequestChanges) => "Changes requested.",
        (_, Verdict::Approve) => "Approved.",
    }
}

/// Which side of a diff a line comment sits on: the new file (added or
/// unchanged lines) or the old one (deleted lines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    New,
    Old,
}

/// One line of a pull request's diff, as each host addresses it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LineRef {
    /// The line's number on its side: what GitHub and Azure DevOps take.
    pub line: u32,
    pub side: Side,
    /// Its numbers in the old and new file, where it has one (a context line
    /// has both): what GitLab's position takes.
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    /// Where it sits in the old and new file even when it is not there (an
    /// added line's place in the old file): GitLab's line codes count these.
    pub old_pos: u32,
    pub new_pos: u32,
}

impl LineRef {
    /// A line from its old and new numbers and diff positions: a deleted
    /// line (no new number) is on the old side.
    pub fn new(old_line: Option<u32>, new_line: Option<u32>, old_pos: u32, new_pos: u32) -> Self {
        let (line, side) = match (new_line, old_line) {
            (Some(new), _) => (new, Side::New),
            (None, Some(old)) => (old, Side::Old),
            (None, None) => (0, Side::New),
        };
        Self {
            line,
            side,
            old_line,
            new_line,
            old_pos,
            new_pos,
        }
    }
}

/// A comment on a line, or a range of lines, of a pull request's diff, sent
/// with a review so it lands there instead of in the general discussion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LineComment {
    pub path: String,
    /// The last line it is on.
    #[serde(flatten)]
    pub end: LineRef,
    /// The first line, when it spans several.
    pub start: Option<LineRef>,
    pub body: String,
}

impl LineComment {
    /// "line 12", or "lines 10 to 12" for a range.
    pub fn lines_label(&self) -> String {
        match &self.start {
            Some(start) if start.line != self.end.line || start.side != self.end.side => {
                format!("lines {} to {}", start.line, self.end.line)
            }
            _ => format!("line {}", self.end.line),
        }
    }
    /// The first line when it is a real range: one line is not one.
    pub fn range_start(&self) -> Option<&LineRef> {
        self.start
            .as_ref()
            .filter(|start| start.line != self.end.line || start.side != self.end.side)
    }
}

/// What a submitted review says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Comment,
    RequestChanges,
    Approve,
}

impl Verdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::RequestChanges => "request_changes",
            Self::Approve => "approve",
        }
    }
}

/// A repository the connected account can reach, from "Add a repo".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RepoHit {
    /// `github`, `gitlab` or `azuredevops`.
    pub provider: String,
    /// The host's own spelling, the same as [`Item::repo`]: `owner/name`, a
    /// GitLab path with groups, or Azure DevOps `project/repository` (the
    /// organization is the connected one).
    pub repo: String,
    pub description: String,
}

/// The review states the Pull requests page knows, from any host's spelling. A review
/// still pending (not submitted) has none.
pub(crate) fn review_state(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "approved" => Some("approved"),
        "changes_requested" | "requested_changes" => Some("changes_requested"),
        "commented" | "reviewed" => Some("commented"),
        "dismissed" => Some("dismissed"),
        _ => None,
    }
}

/// Each reviewer's latest review, from `(login, state, submitted_at)` rows
/// in any order. A later comment does not take back an earlier approval or
/// request for changes, as on GitHub, but a dismissal does. Logins compare
/// without case; the first spelling seen is kept.
pub(crate) fn latest_reviews(
    rows: impl IntoIterator<Item = (String, String, String)>,
) -> Vec<Review> {
    let mut rows: Vec<(String, String, String)> = rows
        .into_iter()
        .filter(|(login, state, _)| !login.trim().is_empty() && review_state(state).is_some())
        .collect();
    // Stable: rows without a time keep their order among themselves.
    rows.sort_by(|a, b| a.2.cmp(&b.2));
    let mut out: Vec<Review> = Vec::new();
    for (login, state, _) in rows {
        let state = review_state(&state).unwrap_or("commented");
        let login = login.trim();
        match out
            .iter_mut()
            .find(|review| review.login.eq_ignore_ascii_case(login))
        {
            Some(review) => {
                let decided = matches!(review.state.as_str(), "approved" | "changes_requested");
                if !(state == "commented" && decided) {
                    review.state = state.into();
                }
            }
            None => out.push(Review {
                login: login.into(),
                state: state.into(),
            }),
        }
    }
    out
}

/// Rows of `items` outside `known` (`(provider, repo)`, without case),
/// marked as other repositories' review requests.
pub(crate) fn outside_known(items: Vec<Item>, known: &[(String, String)]) -> Vec<Item> {
    items
        .into_iter()
        .filter(|item| {
            !known.iter().any(|(provider, repo)| {
                provider.trim().eq_ignore_ascii_case(item.provider.trim())
                    && repo.trim().eq_ignore_ascii_case(item.repo.trim())
            })
        })
        .map(|mut item| {
            item.other_repo = true;
            item
        })
        .collect()
}

/// Runs `work` on every item with at most `workers` threads at once, so a
/// list's per-request lookups don't run one after another.
pub(crate) fn for_each_bounded<T: Send>(items: Vec<T>, workers: usize, work: impl Fn(T) + Sync) {
    if items.len() <= 1 || workers <= 1 {
        items.into_iter().for_each(work);
        return;
    }
    let queue = Mutex::new(items.into_iter());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let next = queue.lock().ok().and_then(|mut queue| queue.next());
                match next {
                    Some(item) => work(item),
                    None => break,
                }
            });
        }
    });
}

type CacheEntries<V> = HashMap<String, (Instant, Result<V, String>)>;

/// A small per-process cache of host answers, such as who is signed in.
/// Failures are kept for a shorter while than answers, so a host that can't
/// say is not asked again on every poll, yet recovers soon.
pub(crate) struct TtlCache<V> {
    entries: Mutex<Option<CacheEntries<V>>>,
    ok_for: Duration,
    err_for: Duration,
}

impl<V: Clone> TtlCache<V> {
    pub(crate) const fn new(ok_for: Duration, err_for: Duration) -> Self {
        Self {
            entries: Mutex::new(None),
            ok_for,
            err_for,
        }
    }

    pub(crate) fn get(
        &self,
        key: &str,
        fetch: impl FnOnce() -> Result<V, String>,
    ) -> Result<V, String> {
        if let Ok(entries) = self.entries.lock() {
            if let Some((at, value)) = entries.as_ref().and_then(|e| e.get(key)) {
                let ttl = if value.is_ok() {
                    self.ok_for
                } else {
                    self.err_for
                };
                if at.elapsed() < ttl {
                    return value.clone();
                }
            }
        }
        let value = fetch();
        if let Ok(mut entries) = self.entries.lock() {
            let entries = entries.get_or_insert_with(HashMap::new);
            // Old keys (a merge request's previous update) are dropped as
            // the cache grows, so it stays small.
            if entries.len() > 2_000 {
                let longest = self.ok_for.max(self.err_for);
                entries.retain(|_, (at, _)| at.elapsed() < longest);
            }
            entries.insert(key.to_string(), (Instant::now(), value.clone()));
        }
        value
    }
}

/// GitHub turns down a request for changes or a comment without words.
/// Approving needs no words; requesting changes needs a summary; a comment
/// review needs a summary or at least one line comment.
fn check_review_body(verdict: Verdict, body: &str, comments: &[LineComment]) -> Result<(), String> {
    let empty = body.trim().is_empty();
    if comments.iter().any(|c| {
        c.body.trim().is_empty()
            || c.path.trim().is_empty()
            || c.end.line == 0
            || c.start.as_ref().is_some_and(|start| start.line == 0)
    }) {
        return Err("A line comment is missing its text or line.".into());
    }
    if (verdict == Verdict::RequestChanges && empty)
        || (verdict == Verdict::Comment && empty && comments.is_empty())
    {
        return Err(match verdict {
            Verdict::RequestChanges => "Say what needs to change before requesting changes.",
            _ => "Write a comment before sending it.",
        }
        .into());
    }
    Ok(())
}

fn known_repo(known: &[(String, String)], provider: &str, repo: &str) -> bool {
    known.iter().any(|(p, r)| {
        p.trim().eq_ignore_ascii_case(provider) && r.trim().eq_ignore_ascii_case(repo.trim())
    })
}

impl Service {
    /// Submits a review on a pull or merge request. `body` may be empty for
    /// an approval; a request for changes or a comment needs one.
    ///
    /// GitLab has no public API for "Request changes", so there it removes
    /// your approval (if any) and posts `body` as a note. Azure DevOps sets
    /// your vote (10 approve, -5 waiting for author) and posts `body` as a
    /// thread.
    pub fn submit_review(
        &self,
        item: &Item,
        verdict: Verdict,
        body: &str,
        comments: &[LineComment],
    ) -> Result<String, String> {
        self.send_review(item, verdict, body, comments)
            .map(|()| review_notice(&item.provider, verdict).to_string())
    }

    fn send_review(
        &self,
        item: &Item,
        verdict: Verdict,
        body: &str,
        comments: &[LineComment],
    ) -> Result<(), String> {
        if item.kind != "pr" {
            return Err("Only pull requests can be reviewed".into());
        }
        check_review_body(verdict, body, comments)?;
        match self.hosts().host(&item.provider) {
            Some(host) => host.submit_review(item, verdict, body, comments),
            None => Err("Reviews are not available for this host".into()),
        }
    }

    /// Reruns a GitLab or Azure DevOps merge request's failed pipeline jobs,
    /// and says what was started. GitHub uses [`Service::rerun_failed`] with
    /// the workflow runs its checks name.
    pub fn rerun_failed_pipeline(&self, item: &Item) -> Result<String, String> {
        if item.kind != "pr" {
            return Err("Only pull requests have checks to rerun".into());
        }
        match self.hosts().host(&item.provider) {
            Some(host) => host.rerun_failed(item),
            None => Err("Rerun GitHub checks with their workflow runs".into()),
        }
    }

    /// The open pull requests of a repository added by hand in Settings ›
    /// Git, which no open project points at. Rows have no `project_path`.
    pub fn list_added_repo(&self, provider: &str, repo: &str) -> Result<Vec<Item>, String> {
        let rows = self
            .hosts()
            .host(provider)
            .ok_or_else(|| format!("{provider} repositories can't be added"))?
            .repo_pull_requests(repo)?;
        Ok(rows
            .into_iter()
            .map(|mut item| {
                item.provider = provider.into();
                item.project_path = String::new();
                item
            })
            .collect())
    }

    /// One search per connected host for open pull requests that ask for
    /// your review anywhere you have access, minus the repositories in
    /// `known` (`(provider, repo)`, compared without case). Rows come back
    /// with `other_repo` set. Fails only when every connected host failed.
    pub fn other_review_requests(&self, known: &[(String, String)]) -> Result<Vec<Item>, String> {
        let mut items = Vec::new();
        let mut first_error = None;
        let mut any_ok = false;
        for (_, result) in self.other_review_requests_by_host(known) {
            match result {
                Ok(rows) => {
                    any_ok = true;
                    items.extend(rows);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) if !any_ok => Err(error),
            _ => Ok(items),
        }
    }

    /// [`Service::other_review_requests`] per host, so one host's failure
    /// can be shown beside the others' rows. Hosts that are not connected
    /// are left out.
    pub(crate) fn other_review_requests_by_host(&self, known: &[(String, String)]) -> HostRows {
        let mut out = Vec::new();
        for provider in GIT_HOSTS {
            let Some(host) = self.hosts().host(provider) else {
                continue;
            };
            if !host.status().is_ok_and(|status| status.connected) {
                continue;
            }
            let rows = host.review_requests();
            let rows = rows.map(|rows: Vec<Item>| {
                let rows = rows
                    .into_iter()
                    .map(|mut item| {
                        item.provider = provider.into();
                        item.project_path = String::new();
                        item
                    })
                    .collect();
                outside_known(rows, known)
            });
            out.push((provider, rows));
        }
        out
    }

    /// Repositories the connected accounts can reach whose name matches
    /// `query`, for "Add a repo". At most 20 per host. Fails only when
    /// every connected host failed.
    pub fn search_repos(&self, query: &str) -> Result<Vec<RepoHit>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        let mut first_error = None;
        let mut any_ok = false;
        for provider in GIT_HOSTS {
            let Some(host) = self.hosts().host(provider) else {
                continue;
            };
            if !host.status().is_ok_and(|status| status.connected) {
                continue;
            }
            let found = host.search_repos(query);
            match found {
                Ok(found) => {
                    any_ok = true;
                    hits.extend(found.into_iter().map(|(repo, description)| RepoHit {
                        provider: provider.into(),
                        repo,
                        description,
                    }));
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) if !any_ok => Err(error),
            _ => Ok(hits),
        }
    }
}

/// Each host's rows, or why it could not list them.
pub(crate) type HostRows = Vec<(&'static str, Result<Vec<Item>, String>)>;

/// Searches every host for review requests outside `known`.
pub(crate) type OtherSearch<'a> = &'a dyn Fn(&[(String, String)]) -> HostRows;

/// Merges hand-added repositories and other repositories' review requests
/// into a project snapshot. See [`Service::list_git`].
pub(crate) fn merge_git(
    snapshot: &mut super::Snapshot,
    added: &[(String, String)],
    mut list_added: impl FnMut(&str, &str) -> Result<Vec<Item>, String>,
    other: Option<OtherSearch<'_>>,
) {
    let mut known: Vec<(String, String)> = snapshot
        .repos
        .iter()
        .map(|r| (r.provider.clone(), r.repo.clone()))
        .chain(
            snapshot
                .items
                .iter()
                .filter(|item| !item.repo.is_empty())
                .map(|item| (item.provider.clone(), item.repo.clone())),
        )
        .collect();
    for (provider, repo) in added {
        let (provider, repo) = (provider.trim(), repo.trim());
        if provider.is_empty() || repo.is_empty() || known_repo(&known, provider, repo) {
            continue;
        }
        known.push((provider.to_string(), repo.to_string()));
        snapshot.repos.push(super::ProjectRepo {
            provider: provider.into(),
            repo: repo.into(),
            project_path: String::new(),
        });
        let scope = super::repo_scope(provider, Path::new(""), repo, "pr");
        match list_added(provider, repo) {
            Ok(rows) => {
                for item in rows {
                    snapshot
                        .item_scopes
                        .entry(item.key())
                        .or_insert_with(|| scope.clone());
                    snapshot.items.push(item);
                }
                snapshot.fetched.insert(scope);
            }
            Err(error) => {
                snapshot.repo_errors.insert(scope, error);
            }
        }
    }
    if let Some(other) = other {
        for (provider, result) in other(&known) {
            match result {
                Ok(rows) => snapshot.items.extend(rows),
                Err(error) => {
                    snapshot
                        .repo_errors
                        .insert(format!("{provider}:other-review-requests"), error);
                }
            }
        }
    }
    snapshot.items.sort_by(|a, b| {
        b.updated_ms()
            .cmp(&a.updated_ms())
            .then_with(|| a.title.cmp(&b.title))
    });
    let mut seen = std::collections::HashSet::new();
    snapshot.items.retain(|item| seen.insert(item.key()));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn the_review_notice_says_when_gitlab_could_not_request_changes() {
        assert_eq!(
            review_notice("github", Verdict::RequestChanges),
            "Changes requested."
        );
        assert_eq!(review_notice("azuredevops", Verdict::Approve), "Approved.");
        assert_eq!(
            review_notice("gitlab", Verdict::Comment),
            "Review comment posted."
        );
        assert!(review_notice("gitlab", Verdict::RequestChanges).contains("approval was removed"));
    }

    use super::*;
    use crate::inbox::{ProjectRepo, Snapshot};

    fn row(login: &str, state: &str, at: &str) -> (String, String, String) {
        (login.into(), state.into(), at.into())
    }

    fn pr(provider: &str, repo: &str, number: i64) -> Item {
        Item {
            provider: provider.into(),
            kind: "pr".into(),
            repo: repo.into(),
            number,
            updated_at: format!("2026-11-{:02}T00:00:00Z", number.clamp(1, 28)),
            ..Default::default()
        }
    }

    #[test]
    fn no_checks_is_an_empty_summary() {
        let summary = CheckSummary::tally(Vec::new());
        assert!(summary.is_empty());
        assert_eq!(summary, CheckSummary::default());
    }

    #[test]
    fn ignored_checks_are_not_counted_and_failed_names_are_unique() {
        let summary = CheckSummary::tally([
            ("lint".to_string(), Outcome::Passed),
            ("Windows tests".to_string(), Outcome::Failed),
            ("Windows tests".to_string(), Outcome::Failed),
            ("stale".to_string(), Outcome::Ignored),
            ("build".to_string(), Outcome::Pending),
            ("  ".to_string(), Outcome::Failed),
        ]);
        assert_eq!(
            (
                summary.total,
                summary.passed,
                summary.failed,
                summary.pending
            ),
            (5, 1, 3, 1)
        );
        assert_eq!(summary.failed_names, ["Windows tests"]);
    }

    #[test]
    fn the_latest_review_per_reviewer_wins_without_case() {
        let reviews = latest_reviews([
            row("mira", "CHANGES_REQUESTED", "2026-11-02T10:00:00Z"),
            row("cubic", "APPROVED", "2026-11-01T10:00:00Z"),
            row("Mira", "APPROVED", "2026-11-03T10:00:00Z"),
            row("ghost", "PENDING", "2026-11-04T10:00:00Z"),
            row("", "APPROVED", "2026-11-04T10:00:00Z"),
        ]);
        assert_eq!(
            reviews,
            [
                Review {
                    login: "cubic".into(),
                    state: "approved".into()
                },
                Review {
                    login: "mira".into(),
                    state: "approved".into()
                },
            ]
        );
    }

    #[test]
    fn a_later_comment_keeps_a_verdict_but_a_dismissal_replaces_it() {
        let reviews = latest_reviews([
            row("mira", "APPROVED", "2026-11-01T00:00:00Z"),
            row("mira", "COMMENTED", "2026-11-02T00:00:00Z"),
            row("jo", "CHANGES_REQUESTED", "2026-11-01T00:00:00Z"),
            row("jo", "DISMISSED", "2026-11-02T00:00:00Z"),
            row("al", "COMMENTED", "2026-11-01T00:00:00Z"),
        ]);
        let states: Vec<(&str, &str)> = reviews
            .iter()
            .map(|r| (r.login.as_str(), r.state.as_str()))
            .collect();
        assert_eq!(
            states,
            [
                ("mira", "approved"),
                ("jo", "dismissed"),
                ("al", "commented")
            ]
        );
    }

    #[test]
    fn other_repo_filtering_ignores_case_and_marks_the_rest() {
        let known = vec![("GitHub".to_string(), " Acme/App ".to_string())];
        let rows = outside_known(
            vec![
                pr("github", "acme/app", 1),
                pr("gitlab", "acme/app", 2),
                pr("github", "acme/other", 3),
            ],
            &known,
        );
        let left: Vec<(&str, &str, bool)> = rows
            .iter()
            .map(|i| (i.provider.as_str(), i.repo.as_str(), i.other_repo))
            .collect();
        assert_eq!(
            left,
            [("gitlab", "acme/app", true), ("github", "acme/other", true)]
        );
    }

    #[test]
    fn requests_for_changes_and_comments_need_words() {
        assert!(check_review_body(Verdict::Approve, "", &[]).is_ok());
        assert!(check_review_body(Verdict::RequestChanges, "  ", &[])
            .unwrap_err()
            .contains("needs to change"));
        assert!(check_review_body(Verdict::Comment, "", &[]).is_err());
        assert!(check_review_body(Verdict::Comment, "nice", &[]).is_ok());
        // Line comments are enough for a comment review, never for changes.
        let line = LineComment {
            path: "src/lib.rs".into(),
            end: LineRef::new(Some(12), Some(12), 12, 12),
            start: None,
            body: "add the index".into(),
        };
        assert!(check_review_body(Verdict::Comment, "", std::slice::from_ref(&line)).is_ok());
        assert!(check_review_body(Verdict::Approve, "", std::slice::from_ref(&line)).is_ok());
        assert!(
            check_review_body(Verdict::RequestChanges, "", std::slice::from_ref(&line)).is_err()
        );
        for broken in [
            LineComment {
                body: " ".into(),
                ..line.clone()
            },
            LineComment {
                end: LineRef::default(),
                ..line.clone()
            },
            LineComment {
                start: Some(LineRef::default()),
                ..line.clone()
            },
            LineComment {
                path: String::new(),
                ..line.clone()
            },
        ] {
            assert!(check_review_body(Verdict::Approve, "ok", &[broken]).is_err());
        }
    }

    #[test]
    fn a_line_knows_its_side_from_the_numbers_it_has() {
        let added = LineRef::new(None, Some(10), 9, 10);
        assert_eq!((added.line, added.side), (10, Side::New));
        let deleted = LineRef::new(Some(7), None, 7, 6);
        assert_eq!((deleted.line, deleted.side), (7, Side::Old));
        let context = LineRef::new(Some(4), Some(5), 4, 5);
        assert_eq!((context.line, context.side), (5, Side::New));
        let one = LineComment {
            end: added.clone(),
            start: Some(added.clone()),
            ..Default::default()
        };
        assert_eq!(one.range_start(), None, "a range of one line is one line");
        assert_eq!(one.lines_label(), "line 10");
        let range = LineComment {
            end: added,
            start: Some(context),
            ..Default::default()
        };
        assert_eq!(range.lines_label(), "lines 5 to 10");
    }

    fn sample() -> (LineComment, LineComment, LineComment) {
        let new = LineComment {
            path: "docs/hackathon.md".into(),
            end: LineRef::new(None, Some(100), 98, 100),
            start: None,
            body: " test comment ".into(),
        };
        let old = LineComment {
            path: "src/old.rs".into(),
            end: LineRef::new(Some(7), None, 7, 6),
            start: None,
            body: "why remove this?".into(),
        };
        // Lines 10 to 12 of the new file: a context line, then two added.
        let range = LineComment {
            path: "CHANGELOG.md".into(),
            start: Some(LineRef::new(Some(9), Some(10), 9, 10)),
            end: LineRef::new(None, Some(12), 10, 12),
            body: "Say what changed.".into(),
        };
        (new, old, range)
    }

    #[test]
    fn line_comments_land_on_their_lines_on_github() {
        let (new, old, range) = sample();
        let github = crate::inbox::github::github_review_payload(
            Verdict::Comment,
            "",
            &[new.clone(), old.clone(), range],
        );
        assert_eq!(
            github,
            json!({
                "event": "COMMENT",
                "comments": [
                    {"path": "docs/hackathon.md", "line": 100, "side": "RIGHT", "body": "test comment"},
                    {"path": "src/old.rs", "line": 7, "side": "LEFT", "body": "why remove this?"},
                    {"path": "CHANGELOG.md", "start_line": 10, "start_side": "RIGHT",
                     "line": 12, "side": "RIGHT", "body": "Say what changed."},
                ],
            }),
            "no summary is sent when there is none"
        );
        assert_eq!(
            crate::inbox::github::github_review_payload(Verdict::RequestChanges, " fix it ", &[])
                ["body"],
            "fix it"
        );
    }

    #[test]
    fn line_comments_land_on_their_lines_on_gitlab() {
        let (new, old, range) = sample();
        let refs = json!({"base_sha": "b", "start_sha": "s", "head_sha": "h"});
        let gitlab = crate::inbox::gitlab::gitlab_line_discussion(&refs, &new).unwrap();
        assert_eq!(gitlab["body"], "test comment");
        assert_eq!(gitlab["position"]["new_line"], 100);
        assert!(gitlab["position"].get("old_line").is_none());
        assert!(gitlab["position"].get("line_range").is_none());
        assert_eq!(gitlab["position"]["head_sha"], "h");
        let gitlab_old = crate::inbox::gitlab::gitlab_line_discussion(&refs, &old).unwrap();
        assert_eq!(gitlab_old["position"]["old_line"], 7);
        assert!(gitlab_old["position"].get("new_line").is_none());
        let multi = crate::inbox::gitlab::gitlab_line_discussion(&refs, &range).unwrap();
        let lines = &multi["position"]["line_range"];
        assert_eq!(multi["position"]["new_line"], 12);
        assert_eq!(
            lines["start"]["type"],
            serde_json::Value::Null,
            "a context line has no type"
        );
        assert_eq!(lines["start"]["old_line"], 9);
        assert_eq!(lines["start"]["new_line"], 10);
        assert_eq!(lines["end"]["type"], "new");
        assert_eq!(lines["end"]["old_line"], serde_json::Value::Null);
        assert!(lines["start"]["line_code"]
            .as_str()
            .unwrap()
            .ends_with("_9_10"));
        assert!(lines["end"]["line_code"]
            .as_str()
            .unwrap()
            .ends_with("_10_12"));
        assert_eq!(
            lines["start"]["line_code"]
                .as_str()
                .unwrap()
                .split('_')
                .next()
                .unwrap()
                .len(),
            40,
            "a SHA-1 of the path"
        );
        // A context line on its own carries both numbers.
        let context = LineComment {
            end: LineRef::new(Some(9), Some(10), 9, 10),
            ..range.clone()
        };
        let one = crate::inbox::gitlab::gitlab_line_discussion(
            &refs,
            &LineComment {
                start: None,
                ..context
            },
        )
        .unwrap();
        assert_eq!(
            (
                one["position"]["old_line"].clone(),
                one["position"]["new_line"].clone()
            ),
            (json!(9), json!(10))
        );
        assert!(crate::inbox::gitlab::gitlab_line_discussion(&json!({}), &new).is_err());
    }

    #[test]
    fn line_comments_land_on_their_lines_on_azure_devops() {
        let (new, old, range) = sample();
        let ado = crate::inbox::azure_devops::azure_line_thread(&new);
        assert_eq!(ado["threadContext"]["filePath"], "/docs/hackathon.md");
        assert_eq!(
            ado["threadContext"]["rightFileStart"],
            json!({"line": 100, "offset": 1})
        );
        assert!(ado["threadContext"].get("leftFileStart").is_none());
        let ado_old = crate::inbox::azure_devops::azure_line_thread(&old);
        assert_eq!(
            ado_old["threadContext"]["leftFileEnd"],
            json!({"line": 7, "offset": 1})
        );
        assert_eq!(ado_old["comments"][0]["content"], "why remove this?");
        let ado_range = crate::inbox::azure_devops::azure_line_thread(&range);
        assert_eq!(
            ado_range["threadContext"]["rightFileStart"],
            json!({"line": 10, "offset": 1})
        );
        assert_eq!(
            ado_range["threadContext"]["rightFileEnd"],
            json!({"line": 12, "offset": 1})
        );
    }

    #[test]
    fn list_git_skips_added_repos_a_project_covers_and_dedupes_rows() {
        let mut snapshot = Snapshot {
            items: vec![pr("github", "acme/app", 1)],
            repos: vec![ProjectRepo {
                provider: "github".into(),
                repo: "acme/app".into(),
                project_path: "/code/app".into(),
            }],
            ..Default::default()
        };
        let mut asked = Vec::new();
        let other = |known: &[(String, String)]| {
            // Known holds the project's repo and every added one.
            assert!(known_repo(known, "github", "ACME/app"));
            assert!(known_repo(known, "gitlab", "grp/lib"));
            assert!(known_repo(known, "github", "acme/broken"));
            vec![
                (
                    "github",
                    Ok(outside_known(vec![pr("github", "else/where", 9)], known)),
                ),
                ("gitlab", Err("GitLab is down".to_string())),
            ]
        };
        merge_git(
            &mut snapshot,
            &[
                ("github".into(), "Acme/App".into()),
                ("gitlab".into(), "grp/lib".into()),
                ("gitlab".into(), "GRP/lib".into()),
                ("github".into(), "acme/broken".into()),
            ],
            |provider, repo| {
                asked.push(format!("{provider}:{repo}"));
                match repo {
                    "acme/broken" => Err("Not found".into()),
                    _ => Ok(vec![pr(provider, repo, 4), pr(provider, repo, 4)]),
                }
            },
            Some(&other),
        );
        // The covered repo and the repeated spelling are not fetched again.
        assert_eq!(asked, ["gitlab:grp/lib", "github:acme/broken"]);
        let keys: Vec<String> = snapshot.items.iter().map(Item::key).collect();
        assert_eq!(
            keys,
            [
                "github:else/where:pr:9",
                "gitlab:grp/lib:pr:4",
                "github:acme/app:pr:1"
            ]
        );
        assert!(snapshot.items[0].other_repo);
        assert!(!snapshot.items[1].other_repo);
        assert_eq!(
            snapshot
                .repos
                .iter()
                .map(|r| (r.repo.as_str(), r.project_path.as_str()))
                .collect::<Vec<_>>(),
            [
                ("acme/app", "/code/app"),
                ("grp/lib", ""),
                ("acme/broken", "")
            ]
        );
        let errors: Vec<&String> = snapshot.repo_errors.keys().collect();
        assert_eq!(
            errors,
            [
                &crate::inbox::repo_scope("github", Path::new(""), "acme/broken", "pr"),
                &"gitlab:other-review-requests".to_string()
            ]
        );
        assert!(snapshot.fetched.contains(&crate::inbox::repo_scope(
            "gitlab",
            Path::new(""),
            "grp/lib",
            "pr"
        )));
    }

    #[test]
    fn bounded_workers_visit_every_item_once() {
        let seen = Mutex::new(Vec::new());
        for_each_bounded((0..25).collect(), 4, |n: i32| seen.lock().unwrap().push(n));
        let mut seen = seen.into_inner().unwrap();
        seen.sort();
        assert_eq!(seen, (0..25).collect::<Vec<_>>());
    }

    #[test]
    fn the_cache_answers_again_without_asking_until_it_expires() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60), Duration::ZERO);
        let mut calls = 0;
        for _ in 0..3 {
            let value = cache.get("k", || {
                calls += 1;
                Ok("mira".into())
            });
            assert_eq!(value.unwrap(), "mira");
        }
        assert_eq!(calls, 1);
        // A failure kept for no time is asked again.
        for _ in 0..2 {
            assert!(cache
                .get("e", || {
                    calls += 1;
                    Err("down".into())
                })
                .is_err());
        }
        assert_eq!(calls, 3);
    }
}
