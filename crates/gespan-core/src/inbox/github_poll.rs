//! Polling a repository's open pull requests without spending the user's
//! GitHub budget. GitHub counts 5,000 requests and 5,000 GraphQL points an
//! hour per user across every token, and a request that runs past 10 seconds
//! times out (502/504) and costs extra points. So:
//!
//! - One GraphQL query per repository asks for the review fields and check
//!   *counts* (`checkRunCountsByState`), never every check on every pull
//!   request, which is what made `gh pr list --json statusCheckRollup` time
//!   out on large repositories.
//! - Before that query, a conditional REST request (`If-None-Match`) asks
//!   whether the open pull requests changed. GitHub does not count a `304`,
//!   so an unchanged repository costs nothing. Check runs don't change a
//!   pull request's ETag, so while any check is still running the full query
//!   runs anyway, and at least every [`FULL_EVERY`].
//! - A timeout or rate limit backs the repository off (1, 2, 4, 8, then 15
//!   minutes) and keeps showing what it last had.
//! - The names of failed checks, for "Rust tests (Windows) failed", are asked
//!   only for your own pull requests with failures, once per update.
use super::github::{GitHubAssignee, GitHubLabel, GitHubWorkItem};
use super::review::{latest_reviews, CheckSummary, TtlCache};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

/// The longest an unchanged repository goes without a full query: check
/// runs don't move its ETag.
pub(crate) const FULL_EVERY: Duration = Duration::from_secs(10 * 60);
/// The longest a backed-off repository waits.
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

/// Open pull requests, newest update first, with what the list needs.
pub(crate) const OPEN_PRS_QUERY: &str = r#"query($owner: String!, $name: String!, $n: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: $n, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title url state isDraft createdAt updatedAt additions deletions reviewDecision
        author { login }
        labels(first: 10) { nodes { name color } }
        assignees(first: 10) { nodes { login } }
        reviewRequests(first: 10) { nodes { requestedReviewer {
          ... on User { login } ... on Bot { login } ... on Mannequin { login } ... on Team { name }
        } } }
        latestReviews(first: 10) { nodes { author { login } state submittedAt } }
        commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 1) {
          checkRunCountsByState { state count }
          statusContextCountsByState { state count }
        } } } } }
      }
    }
  }
}"#;

/// The failing checks of one pull request's head commit.
pub(crate) const FAILED_CHECKS_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 50) { nodes {
        ... on CheckRun { name conclusion }
        ... on StatusContext { context state }
      } } } } } }
    }
  }
}"#;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Login {
    login: String,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Nodes<T> {
    nodes: Vec<T>,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Reviewer {
    requested_reviewer: Option<ReviewerName>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct ReviewerName {
    login: String,
    name: String,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct LatestReview {
    author: Option<Login>,
    state: String,
    submitted_at: String,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct StateCount {
    state: String,
    count: u32,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Contexts {
    check_run_counts_by_state: Vec<StateCount>,
    status_context_counts_by_state: Vec<StateCount>,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Rollup {
    contexts: Contexts,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<Rollup>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct CommitNode {
    commit: Commit,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Pr {
    number: i64,
    title: String,
    url: String,
    state: String,
    is_draft: bool,
    created_at: String,
    updated_at: String,
    additions: i64,
    deletions: i64,
    review_decision: Option<String>,
    author: Option<Login>,
    labels: Nodes<GitHubLabelRow>,
    assignees: Nodes<Login>,
    review_requests: Nodes<Reviewer>,
    latest_reviews: Nodes<LatestReview>,
    commits: Nodes<CommitNode>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct GitHubLabelRow {
    name: String,
    color: String,
}

/// Check counts from GitHub's per-state tallies. Skipped and neutral runs
/// pass; cancelled, timed out and failed ones fail; stale runs were replaced
/// and don't count.
fn summary_from_counts(contexts: &Contexts) -> CheckSummary {
    let mut summary = CheckSummary::default();
    let mut add = |kind: &str, count: u32| {
        match kind {
            "passed" => summary.passed += count,
            "failed" => summary.failed += count,
            "pending" => summary.pending += count,
            _ => return,
        }
        summary.total += count;
    };
    for row in &contexts.check_run_counts_by_state {
        add(
            match row.state.as_str() {
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => "passed",
                "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
                | "ERROR" => "failed",
                "STALE" => "",
                _ => "pending",
            },
            row.count,
        );
    }
    for row in &contexts.status_context_counts_by_state {
        add(
            match row.state.as_str() {
                "SUCCESS" => "passed",
                "FAILURE" | "ERROR" => "failed",
                _ => "pending",
            },
            row.count,
        );
    }
    summary
}

/// `gh api graphql` output for [`OPEN_PRS_QUERY`], as list rows.
pub(crate) fn parse_open_prs(json: &str, repo: &str) -> Result<Vec<GitHubWorkItem>, String> {
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Repository {
        pull_requests: Nodes<Pr>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Data {
        repository: Option<Repository>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Response {
        data: Data,
        errors: Vec<serde_json::Value>,
    }
    let response: Response = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let Some(repository) = response.data.repository else {
        return Err(response
            .errors
            .first()
            .and_then(|e| e["message"].as_str())
            .unwrap_or("GitHub did not return the repository")
            .to_string());
    };
    Ok(repository
        .pull_requests
        .nodes
        .into_iter()
        .map(|pr| {
            let checks = pr
                .commits
                .nodes
                .into_iter()
                .next()
                .and_then(|node| node.commit.status_check_rollup)
                .map(|rollup| summary_from_counts(&rollup.contexts))
                .unwrap_or_default();
            GitHubWorkItem {
                kind: "pr".into(),
                number: pr.number,
                title: pr.title,
                url: pr.url,
                state: pr.state.to_lowercase(),
                state_reason: String::new(),
                created_at: pr.created_at,
                updated_at: pr.updated_at,
                labels: pr
                    .labels
                    .nodes
                    .into_iter()
                    .map(|l| GitHubLabel {
                        name: l.name,
                        color: l.color,
                    })
                    .collect(),
                assignees: pr
                    .assignees
                    .nodes
                    .into_iter()
                    .map(|a| GitHubAssignee {
                        avatar_url: super::github::github_avatar_url(&a.login),
                        login: a.login,
                    })
                    .collect(),
                draft: pr.is_draft,
                repo: repo.to_string(),
                author: pr.author.map(|a| a.login).unwrap_or_default(),
                review_decision: pr
                    .review_decision
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase(),
                reviews: latest_reviews(pr.latest_reviews.nodes.into_iter().map(|r| {
                    (
                        r.author.map(|a| a.login).unwrap_or_default(),
                        r.state,
                        r.submitted_at,
                    )
                })),
                requested_reviewers: pr
                    .review_requests
                    .nodes
                    .into_iter()
                    .filter_map(|r| r.requested_reviewer)
                    .map(|r| {
                        if r.login.trim().is_empty() {
                            r.name
                        } else {
                            r.login
                        }
                    })
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect(),
                viewer: String::new(),
                check_summary: checks,
                additions: pr.additions,
                deletions: pr.deletions,
            }
        })
        .collect())
}

/// The failing checks' names from [`FAILED_CHECKS_QUERY`]'s output.
pub(crate) fn parse_failed_checks(json: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let nodes = value
        .pointer(
            "/data/repository/pullRequest/commits/nodes/0/commit/statusCheckRollup/contexts/nodes",
        )
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for node in nodes {
        let (name, failed) = match (node["name"].as_str(), node["context"].as_str()) {
            (Some(name), _) => (
                name,
                matches!(
                    node["conclusion"].as_str().unwrap_or_default(),
                    "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
                ),
            ),
            (None, Some(context)) => (
                context,
                matches!(
                    node["state"].as_str().unwrap_or_default(),
                    "FAILURE" | "ERROR"
                ),
            ),
            _ => continue,
        };
        let name = name.trim();
        if failed && !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// What a conditional request said: its HTTP status and the ETag to send
/// next time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    pub status: u16,
    pub etag: Option<String>,
}

/// `gh api --include` output: the status line and the `ETag` header.
pub(crate) fn parse_probe(output: &str) -> Option<Probe> {
    let mut lines = output.lines();
    let status = lines
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()?;
    let etag = lines
        .take_while(|line| !line.trim().is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("etag")
                .then(|| value.trim().to_string())
        })
        .filter(|etag| !etag.is_empty());
    Some(Probe { status, etag })
}

/// A timeout or a rate limit: worth waiting out rather than asking again.
pub(crate) fn is_throttled(error: &str) -> bool {
    let error = error.to_lowercase();
    [
        "http 502",
        "http 503",
        "http 504",
        "gateway timeout",
        "bad gateway",
        "timed out",
        "rate limit",
        "abuse detection",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}

/// One repository's polling state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Gate {
    pub items: Option<Vec<GitHubWorkItem>>,
    pub etag: Option<String>,
    pub full_at: Option<Instant>,
    pub failures: u32,
    pub wait_until: Option<Instant>,
    pub last_error: Option<String>,
}

impl Gate {
    fn backoff(failures: u32) -> Duration {
        Duration::from_secs(60 << failures.saturating_sub(1).min(4)).min(MAX_BACKOFF)
    }
    /// A full query is due: nothing cached, too long since the last, or a
    /// cached pull request still has checks running.
    fn full_due(&self, now: Instant) -> bool {
        let Some(items) = &self.items else {
            return true;
        };
        self.full_at
            .is_none_or(|at| now.duration_since(at) >= FULL_EVERY)
            || items.iter().any(|item| item.check_summary.pending > 0)
    }
}

/// One poll of a repository: wait out a backoff, skip when GitHub says
/// nothing changed, else run the full query. `probe` sends the conditional
/// request with the ETag it is given; `full` runs the GraphQL query.
pub(crate) fn poll(
    gate: &mut Gate,
    now: Instant,
    mut probe: impl FnMut(Option<&str>) -> Result<Probe, String>,
    full: impl FnOnce() -> Result<Vec<GitHubWorkItem>, String>,
) -> Result<Vec<GitHubWorkItem>, String> {
    if gate.wait_until.is_some_and(|until| now < until) {
        return match &gate.items {
            Some(items) => Ok(items.clone()),
            None => Err(gate
                .last_error
                .clone()
                .unwrap_or_else(|| "GitHub asked to wait".into())),
        };
    }
    // Only an ETag GitHub gave for this same list can skip the query.
    if !gate.full_due(now) && gate.etag.is_some() {
        match probe(gate.etag.as_deref()) {
            Ok(Probe { status: 304, .. }) => {
                gate.failures = 0;
                gate.wait_until = None;
                return Ok(gate.items.clone().unwrap_or_default());
            }
            Ok(Probe { etag, .. }) => gate.etag = etag,
            Err(error) if is_throttled(&error) => return gate.throttled(now, error),
            // A probe that can't say runs the full query.
            Err(_) => {}
        }
    } else if gate.etag.is_none() {
        // The first ETag, so the next poll can skip.
        if let Ok(Probe { etag, .. }) = probe(None) {
            gate.etag = etag;
        }
    }
    match full() {
        Ok(items) => {
            gate.items = Some(items.clone());
            gate.full_at = Some(now);
            gate.failures = 0;
            gate.wait_until = None;
            gate.last_error = None;
            Ok(items)
        }
        Err(error) if is_throttled(&error) => gate.throttled(now, error),
        Err(error) => Err(error),
    }
}

impl Gate {
    fn throttled(&mut self, now: Instant, error: String) -> Result<Vec<GitHubWorkItem>, String> {
        self.failures += 1;
        self.wait_until = Some(now + Self::backoff(self.failures));
        self.last_error = Some(error.clone());
        match &self.items {
            Some(items) => Ok(items.clone()),
            None => Err(error),
        }
    }
}

/// Every repository's gate, by `gh` root and repository.
static GATES: Mutex<Option<HashMap<String, Gate>>> = Mutex::new(None);

/// The open pull requests of `repo`, polled as this module describes.
pub(crate) fn open_pull_requests(
    root: &Path,
    repo: &str,
    limit: u32,
) -> Result<Vec<GitHubWorkItem>, String> {
    let (owner, name) = super::github::split_github_repo(repo)?;
    let repo = format!("{owner}/{name}");
    let key = format!("{}|{}", root.to_string_lossy(), repo.to_lowercase());
    let mut gate = GATES
        .lock()
        .ok()
        .and_then(|gates| gates.as_ref().and_then(|g| g.get(&key)).cloned())
        .unwrap_or_default();
    let endpoint =
        format!("repos/{repo}/pulls?state=open&per_page=100&sort=updated&direction=desc");
    let result = poll(
        &mut gate,
        Instant::now(),
        |etag| {
            let header = etag.map(|etag| format!("If-None-Match: {etag}"));
            let mut args = vec!["api", "--include", "--method", "GET", &endpoint];
            if let Some(header) = &header {
                args.push("-H");
                args.push(header);
            }
            // A 304 is not a success to `gh`: read its headers either way.
            let output = super::run_gh_output(root, &args)?;
            parse_probe(&output).ok_or_else(|| "GitHub's answer had no status line".to_string())
        },
        || {
            let json = super::run_gh(
                root,
                &[
                    "api",
                    "graphql",
                    "-f",
                    &format!("query={OPEN_PRS_QUERY}"),
                    "-F",
                    &format!("owner={owner}"),
                    "-F",
                    &format!("name={name}"),
                    "-F",
                    &format!("n={}", limit.clamp(1, 100)),
                ],
                false,
            )?;
            parse_open_prs(&json, &repo)
        },
    );
    if let Ok(mut gates) = GATES.lock() {
        gates.get_or_insert_with(HashMap::new).insert(key, gate);
    }
    result
}

/// The names of `number`'s failed checks, once per update of it.
pub(crate) fn failed_check_names(
    root: &Path,
    repo: &str,
    number: i64,
    updated_at: &str,
) -> Vec<String> {
    static NAMES: TtlCache<Vec<String>> =
        TtlCache::new(Duration::from_secs(60 * 60), Duration::from_secs(5 * 60));
    let Ok((owner, name)) = super::github::split_github_repo(repo) else {
        return vec![];
    };
    NAMES
        .get(&format!("{repo}#{number}@{updated_at}"), || {
            let json = super::run_gh(
                root,
                &[
                    "api",
                    "graphql",
                    "-f",
                    &format!("query={FAILED_CHECKS_QUERY}"),
                    "-F",
                    &format!("owner={owner}"),
                    "-F",
                    &format!("name={name}"),
                    "-F",
                    &format!("number={number}"),
                ],
                false,
            )?;
            parse_failed_checks(&json)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: i64, pending: u32) -> GitHubWorkItem {
        GitHubWorkItem {
            kind: "pr".into(),
            number,
            title: String::new(),
            url: String::new(),
            state: "open".into(),
            state_reason: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            labels: vec![],
            assignees: vec![],
            draft: false,
            repo: "acme/web".into(),
            author: String::new(),
            review_decision: String::new(),
            reviews: vec![],
            requested_reviewers: vec![],
            viewer: String::new(),
            check_summary: CheckSummary {
                total: pending,
                pending,
                ..Default::default()
            },
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn the_list_reads_reviews_and_check_counts_from_one_query() {
        let json = r#"{"data":{"repository":{"pullRequests":{"nodes":[{
            "number":562,"title":"Toasty","url":"u","state":"OPEN","isDraft":false,
            "createdAt":"c","updatedAt":"u2","additions":318,"deletions":204,
            "reviewDecision":"CHANGES_REQUESTED","author":{"login":"devon"},
            "labels":{"nodes":[{"name":"ui","color":"fff"}]},
            "assignees":{"nodes":[{"login":"me"}]},
            "reviewRequests":{"nodes":[{"requestedReviewer":{"login":"me"}},{"requestedReviewer":{"name":"core"}},{"requestedReviewer":null}]},
            "latestReviews":{"nodes":[{"author":{"login":"mira"},"state":"CHANGES_REQUESTED","submittedAt":"t"}]},
            "commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{
              "checkRunCountsByState":[{"state":"SUCCESS","count":4},{"state":"SKIPPED","count":1},{"state":"FAILURE","count":1},{"state":"CANCELLED","count":1},{"state":"IN_PROGRESS","count":2},{"state":"STALE","count":3}],
              "statusContextCountsByState":[{"state":"PENDING","count":1},{"state":"ERROR","count":1}]}}}}]}
          },{
            "number":7,"title":"No checks","url":"u","state":"OPEN","isDraft":true,
            "author":null,"reviewDecision":null,"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}
          }]}}}}"#;
        let rows = parse_open_prs(json, "acme/web").unwrap();
        assert_eq!(rows.len(), 2);
        let first = &rows[0];
        assert_eq!((first.number, first.author.as_str()), (562, "devon"));
        assert_eq!(first.review_decision, "changes_requested");
        assert_eq!(
            first.requested_reviewers,
            vec!["me".to_string(), "core".to_string()]
        );
        assert_eq!(first.reviews[0].state, "changes_requested");
        assert_eq!((first.additions, first.deletions), (318, 204));
        assert_eq!(first.labels[0].name, "ui");
        assert_eq!(
            first.check_summary,
            CheckSummary {
                total: 11,
                passed: 5,
                failed: 3,
                pending: 3,
                failed_names: vec![],
            },
            "stale runs don't count; cancelled and errored ones fail"
        );
        assert_eq!(rows[1].check_summary, CheckSummary::default());
        assert!(rows[1].draft && rows[1].author.is_empty());
        let missing = r#"{"data":{"repository":null},"errors":[{"message":"Could not resolve to a Repository"}]}"#;
        assert_eq!(
            parse_open_prs(missing, "x/y").unwrap_err(),
            "Could not resolve to a Repository"
        );
    }

    #[test]
    fn the_query_asks_for_counts_not_every_check() {
        assert!(OPEN_PRS_QUERY.contains("checkRunCountsByState"));
        assert!(OPEN_PRS_QUERY.contains("contexts(first: 1)"));
        assert!(!OPEN_PRS_QUERY.contains("first: 100) { nodes { ... on CheckRun"));
    }

    #[test]
    fn failed_check_names_are_the_failing_ones_once_each() {
        let json = r#"{"data":{"repository":{"pullRequest":{"commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{"nodes":[
            {"name":"Lint","conclusion":"SUCCESS"},
            {"name":"Rust tests (Windows)","conclusion":"FAILURE"},
            {"name":"Rust tests (Windows)","conclusion":"FAILURE"},
            {"name":"Mobile","conclusion":"CANCELLED"},
            {"context":"ci/legacy","state":"ERROR"},
            {"context":"deploy","state":"PENDING"},
            {}
        ]}}}}]}}}}}"#;
        assert_eq!(
            parse_failed_checks(json).unwrap(),
            vec!["Rust tests (Windows)", "Mobile", "ci/legacy"]
        );
        assert_eq!(
            parse_failed_checks(r#"{"data":{}}"#).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_probe_reads_its_status_and_etag() {
        let out = "HTTP/2.0 304 Not Modified\nEtag: W/\"abc\"\nX-Ratelimit-Remaining: 4999\n\n";
        assert_eq!(
            parse_probe(out),
            Some(Probe {
                status: 304,
                etag: Some("W/\"abc\"".into())
            })
        );
        let ok = "HTTP/2.0 200 OK\nContent-Type: application/json\nETag: \"def\"\n\n[{\"etag\": \"not a header\"}]";
        assert_eq!(parse_probe(ok).unwrap().etag.as_deref(), Some("\"def\""));
        assert_eq!(parse_probe("not http"), None);
        assert_eq!(parse_probe("HTTP/1.1 200 OK\n\n").unwrap().etag, None);
    }

    #[test]
    fn timeouts_and_rate_limits_are_waited_out_but_other_errors_are_not() {
        for error in [
            "HTTP 504: 504 Gateway Timeout (https://api.github.com/graphql)",
            "HTTP 502: Bad Gateway",
            "read tcp: operation timed out",
            "API rate limit exceeded for user",
            "You have exceeded a secondary rate limit",
        ] {
            assert!(is_throttled(error), "{error}");
        }
        for error in [
            "Could not resolve to a Repository",
            "HTTP 404: Not Found",
            "token is invalid",
        ] {
            assert!(!is_throttled(error), "{error}");
        }
    }

    #[test]
    fn an_unchanged_repository_skips_the_query_until_checks_run_or_time_passes() {
        let now = Instant::now();
        let mut gate = Gate::default();
        let probes = std::cell::RefCell::new(Vec::new());
        let fulls = std::cell::Cell::new(0);
        let poll_with = |gate: &mut Gate, at: Instant, status: u16, items: Vec<GitHubWorkItem>| {
            let mut asked = None;
            let result = poll(
                gate,
                at,
                |etag| {
                    asked = Some(etag.map(str::to_string));
                    Ok(Probe {
                        status,
                        etag: Some("\"v2\"".into()),
                    })
                },
                || {
                    fulls.set(fulls.get() + 1);
                    Ok(items)
                },
            );
            probes.borrow_mut().push(asked);
            result
        };
        // First poll: an ETag is taken, then the full query.
        assert_eq!(
            poll_with(&mut gate, now, 200, vec![pr(1, 0)])
                .unwrap()
                .len(),
            1
        );
        assert_eq!(gate.etag.as_deref(), Some("\"v2\""));
        // Unchanged: a free 304, no query.
        let later = now + Duration::from_secs(120);
        assert_eq!(
            poll_with(&mut gate, later, 304, vec![]).unwrap()[0].number,
            1
        );
        assert_eq!(
            probes.borrow()[1],
            Some(Some("\"v2\"".into())),
            "the ETag is sent"
        );
        // Changed: the probe's 200 runs the query.
        assert_eq!(
            poll_with(&mut gate, later, 200, vec![pr(1, 0), pr(2, 2)])
                .unwrap()
                .len(),
            2
        );
        // A running check: the full query runs whatever the ETag says.
        poll_with(
            &mut gate,
            later + Duration::from_secs(1),
            304,
            vec![pr(2, 0)],
        )
        .unwrap();
        assert_eq!(gate.items.as_ref().unwrap()[0].number, 2);
        // Ten quiet minutes: the full query runs again.
        poll_with(
            &mut gate,
            later + FULL_EVERY + Duration::from_secs(5),
            304,
            vec![pr(3, 0)],
        )
        .unwrap();
        assert_eq!(gate.items.as_ref().unwrap()[0].number, 3);
        assert_eq!(fulls.get(), 4, "the 304 skipped one query");
    }

    #[test]
    fn a_timeout_backs_off_and_keeps_the_last_list() {
        let now = Instant::now();
        let mut gate = Gate::default();
        let ok = |gate: &mut Gate, at| {
            poll(
                gate,
                at,
                |_| {
                    Ok(Probe {
                        status: 200,
                        etag: None,
                    })
                },
                || Ok(vec![pr(1, 1)]),
            )
        };
        let timeout = |gate: &mut Gate, at| {
            poll(
                gate,
                at,
                |_| {
                    Ok(Probe {
                        status: 200,
                        etag: None,
                    })
                },
                || {
                    Err(
                        "HTTP 504: 504 Gateway Timeout (https://api.github.com/graphql)"
                            .to_string(),
                    )
                },
            )
        };
        // No list yet: the error shows.
        assert!(timeout(&mut gate, now).is_err());
        assert_eq!(gate.failures, 1);
        // While backed off nothing is asked, even a working query.
        let mut asked = false;
        let waiting = poll(
            &mut gate,
            now + Duration::from_secs(30),
            |_| {
                asked = true;
                Ok(Probe {
                    status: 200,
                    etag: None,
                })
            },
            || Ok(vec![pr(9, 0)]),
        );
        assert!(waiting.is_err() && !asked);
        // After the minute it works and resets.
        assert_eq!(
            ok(&mut gate, now + Duration::from_secs(61)).unwrap().len(),
            1
        );
        assert_eq!(gate.failures, 0);
        // A later timeout keeps the last list and waits longer each time.
        let t = now + Duration::from_secs(200);
        assert_eq!(timeout(&mut gate, t).unwrap()[0].number, 1);
        assert_eq!(gate.wait_until, Some(t + Duration::from_secs(60)));
        let t2 = t + Duration::from_secs(61);
        timeout(&mut gate, t2).unwrap();
        assert_eq!(gate.wait_until, Some(t2 + Duration::from_secs(120)));
        assert_eq!(Gate::backoff(9), MAX_BACKOFF);
        // Other errors are not waited out.
        let other = poll(
            &mut gate,
            t2 + MAX_BACKOFF,
            |_| {
                Ok(Probe {
                    status: 200,
                    etag: None,
                })
            },
            || Err("Could not resolve to a Repository".to_string()),
        );
        assert!(other.is_err());
    }
}
