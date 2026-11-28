//! What each host adapter sends: the endpoints and bodies its API wants,
//! and how it reads what's particular to it (paging, filters, line codes,
//! votes).
use super::{super::*, json, with_headers, Fake};
use crate::pull_requests::{
    azure_devops::AzureDevOps,
    github::GitHub,
    gitlab::{find_line, line_code, DiffLine, GitLab},
    host::PullRequestHost,
    transport::Method,
};
use serde_json::json;

fn pr() -> PrRef {
    PrRef::new("acme/web", 7)
}

fn review(verdict: Verdict, body: &str, comments: Vec<DraftComment>) -> ReviewSubmission {
    ReviewSubmission {
        verdict,
        body: body.into(),
        comments,
    }
}

fn draft(location: LineLocation, body: &str) -> DraftComment {
    DraftComment {
        location,
        body: body.into(),
    }
}

// GitHub

#[test]
fn github_sends_a_review_with_ranges_and_single_lines_in_one_request() {
    let fake = Fake::new();
    fake.on(
        Method::Post,
        "repos/acme/web/pulls/7/reviews",
        json(json!({"id": 1})),
    );
    GitHub::new(fake.transport())
        .submit_review(
            &pr(),
            &review(
                Verdict::RequestChanges,
                "A few things",
                vec![
                    draft(LineLocation::range("src/a.rs", Side::New, 12, 14), "Range"),
                    draft(LineLocation::line("src/b.rs", Side::Old, 3), "Deleted line"),
                ],
            ),
        )
        .unwrap();
    assert_eq!(
        fake.body(Method::Post, "repos/acme/web/pulls/7/reviews"),
        json!({
            "event": "REQUEST_CHANGES",
            "body": "A few things",
            "comments": [
                {"path": "src/a.rs", "body": "Range", "line": 14, "side": "RIGHT",
                 "start_line": 12, "start_side": "RIGHT"},
                {"path": "src/b.rs", "body": "Deleted line", "line": 3, "side": "LEFT"}
            ]
        })
    );
}

#[test]
fn github_replies_and_edits_go_to_the_api_the_comment_lives_in() {
    let fake = Fake::new();
    let row = json(json!({"id": 99, "user": {"login": "me"}, "body": "ok"}));
    fake.on(
        Method::Post,
        "repos/acme/web/pulls/7/comments/20/replies",
        row.clone(),
    )
    .on(
        Method::Post,
        "repos/acme/web/issues/7/comments",
        row.clone(),
    )
    .on(
        Method::Patch,
        "repos/acme/web/pulls/comments/21",
        row.clone(),
    )
    .on(Method::Patch, "repos/acme/web/issues/comments/1", row);
    let github = GitHub::new(fake.transport());
    assert_eq!(
        github.reply(&pr(), "review:20", "ok").unwrap().id,
        "review:99"
    );
    // General comments aren't threaded: a reply is a new comment.
    assert_eq!(github.reply(&pr(), "issue:1", "ok").unwrap().id, "issue:99");
    github
        .edit_comment(&pr(), "review:21", "new words")
        .unwrap();
    github.edit_comment(&pr(), "issue:1", "new words").unwrap();
    assert_eq!(
        fake.body(Method::Patch, "repos/acme/web/pulls/comments/21"),
        json!({"body": "new words"})
    );
    assert!(matches!(
        github.edit_comment(&pr(), "21", "x"),
        Err(HostError::Invalid(_))
    ));
    assert_eq!(
        github.resolve_thread(&pr(), "review:20", true),
        Err(HostError::Unsupported("resolving threads"))
    );
}

#[test]
fn github_follows_link_pages_and_tells_closed_from_merged() {
    let fake = Fake::new();
    let pr = |n: u64, merged: bool| {
        json!({"number": n, "state": "closed", "merged_at": if merged { json!("2026-11-01T00:00:00Z") } else { json!(null) },
               "user": {"login": "dana"}, "head": {"ref": "x"}, "base": {"ref": "main"},
               "updated_at": format!("2026-11-0{n}T00:00:00Z")})
    };
    fake.on(
        Method::Get,
        "repos/acme/web/pulls?state=closed&per_page=100&page=1",
        with_headers(
            json!([pr(1, true), pr(2, false)]),
            &[(
                "Link",
                "<https://api.github.com/repositories/1/pulls?page=2>; rel=\"next\"",
            )],
        ),
    )
    .get(
        "repos/acme/web/pulls?state=closed&per_page=100&page=2",
        json!([pr(3, true)]),
    );
    let github = GitHub::new(fake.transport());
    let merged: Vec<u64> = github
        .pull_requests("acme/web", StateFilter::Merged)
        .unwrap()
        .iter()
        .map(|p| p.number)
        .collect();
    assert_eq!(merged, vec![1, 3]);
    let closed = github
        .pull_requests("acme/web", StateFilter::Closed)
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].state, PrState::Closed);
    assert_eq!(fake.calls().len(), 4, "two pages each time, no third");
    assert!(matches!(
        github.pull_requests("not-a-repo", StateFilter::Open),
        Err(HostError::Invalid(_))
    ));
}

#[test]
fn github_checks_read_check_runs_and_commit_statuses_on_the_head_commit() {
    let fake = Fake::new();
    fake.get(
        "repos/acme/web/pulls/7",
        json!({"number": 7, "head": {"sha": "abc"}, "base": {}, "user": {}}),
    )
    .get(
        "repos/acme/web/commits/abc/check-runs?filter=latest&per_page=100&page=1",
        json!({"check_runs": [
            {"name": "test", "status": "completed", "conclusion": "success"},
            {"name": "lint", "status": "completed", "conclusion": "failure", "html_url": "l"},
            {"name": "build", "status": "in_progress", "conclusion": null},
            {"name": "docs", "status": "completed", "conclusion": "skipped"}
        ]}),
    )
    .get(
        "repos/acme/web/commits/abc/status",
        json!({"statuses": [{"context": "ci/legacy", "state": "error"}]}),
    );
    let checks = GitHub::new(fake.transport()).checks(&pr()).unwrap();
    let statuses: Vec<(&str, CheckStatus)> =
        checks.iter().map(|c| (c.name.as_str(), c.status)).collect();
    assert_eq!(
        statuses,
        vec![
            ("test", CheckStatus::Success),
            ("lint", CheckStatus::Failure),
            ("build", CheckStatus::Pending),
            ("docs", CheckStatus::Skipped),
            ("ci/legacy", CheckStatus::Failure),
        ]
    );
}

#[test]
fn github_review_requests_name_the_repository_from_search_results() {
    let fake = Fake::new();
    fake.get(
        "search/issues?q=is%3Apr+is%3Aopen+review-requested%3A%40me&per_page=100&page=1",
        json!({"items": [{
            "number": 5, "title": "Fix", "state": "open", "user": {"login": "dana"},
            "repository_url": "https://api.github.com/repos/acme/api"
        }]}),
    );
    let prs = GitHub::new(fake.transport()).review_requests().unwrap();
    assert_eq!(prs[0].repo, "acme/api");
    assert_eq!(prs[0].pr_ref(), PrRef::new("acme/api", 5));
}

// GitLab

const DIFF: &str = "@@ -10,4 +10,6 @@ fn search()\n context ten\n-old eleven\n+new eleven\n+new twelve\n context twelve\n+new fourteen\n context thirteen\n";

#[test]
fn gitlab_counts_diff_lines_as_gitlab_does() {
    // Context lines have both positions; an added line keeps the old
    // position of the next old line; a deleted one the next new line's.
    assert_eq!(
        find_line(DIFF, Side::New, 10),
        Some(DiffLine {
            kind: ' ',
            old: 10,
            new: 10
        })
    );
    assert_eq!(
        find_line(DIFF, Side::Old, 11),
        Some(DiffLine {
            kind: '-',
            old: 11,
            new: 11
        })
    );
    assert_eq!(
        find_line(DIFF, Side::New, 12),
        Some(DiffLine {
            kind: '+',
            old: 12,
            new: 12
        })
    );
    assert_eq!(
        find_line(DIFF, Side::New, 13),
        Some(DiffLine {
            kind: ' ',
            old: 12,
            new: 13
        })
    );
    assert_eq!(
        find_line(DIFF, Side::New, 14),
        Some(DiffLine {
            kind: '+',
            old: 13,
            new: 14
        })
    );
    assert_eq!(find_line(DIFF, Side::New, 99), None);
    use sha1::{Digest, Sha1};
    let path_sha: String = Sha1::digest(b"src/a.rs")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        line_code(
            "src/a.rs",
            DiffLine {
                kind: '+',
                old: 12,
                new: 13
            }
        ),
        format!("{path_sha}_12_13")
    );
}

fn gitlab_review_routes() -> Fake {
    let fake = Fake::new();
    let mr = "projects/acme%2Fweb/merge_requests/7";
    fake.get(
        mr,
        json!({"iid": 7, "diff_refs": {"base_sha": "b", "start_sha": "s", "head_sha": "h"}}),
    )
    .get(
        &format!("{mr}/diffs?per_page=100&page=1"),
        json!([{"old_path": "src/a.rs", "new_path": "src/a.rs", "diff": DIFF}]),
    )
    .on(
        Method::Post,
        &format!("{mr}/discussions"),
        json(json!({"id": "d"})),
    )
    .on(Method::Post, &format!("{mr}/notes"), json(json!({"id": 1})))
    .on(Method::Post, &format!("{mr}/approve"), json(json!({})));
    fake
}

#[test]
fn gitlab_sends_a_range_as_a_positioned_discussion_with_line_codes_then_approves() {
    let fake = gitlab_review_routes();
    let mr = "projects/acme%2Fweb/merge_requests/7";
    GitLab::new(fake.transport())
        .submit_review(
            &pr(),
            &review(
                Verdict::Approve,
                "LGTM",
                vec![draft(
                    LineLocation::range("src/a.rs", Side::New, 12, 14),
                    "Range",
                )],
            ),
        )
        .unwrap();
    let code = |old: u32, new: u32| {
        line_code(
            "src/a.rs",
            DiffLine {
                kind: '+',
                old,
                new,
            },
        )
    };
    assert_eq!(
        fake.body(Method::Post, &format!("{mr}/discussions")),
        json!({
            "body": "Range",
            "position": {
                "position_type": "text", "base_sha": "b", "start_sha": "s", "head_sha": "h",
                "old_path": "src/a.rs", "new_path": "src/a.rs", "new_line": 14,
                "line_range": {
                    "start": {"line_code": code(12, 12), "type": "new", "new_line": 12},
                    "end": {"line_code": code(13, 14), "type": "new", "new_line": 14}
                }
            }
        })
    );
    assert_eq!(
        fake.body(Method::Post, &format!("{mr}/notes")),
        json!({"body": "LGTM"})
    );
    let calls = fake.calls();
    assert_eq!(
        calls.last().unwrap(),
        &format!("POST {mr}/approve"),
        "the approval comes after the comments: {calls:?}"
    );
}

#[test]
fn gitlab_gives_an_unchanged_line_both_positions_and_refuses_a_line_outside_the_diff() {
    let fake = gitlab_review_routes();
    let mr = "projects/acme%2Fweb/merge_requests/7";
    let gitlab = GitLab::new(fake.transport());
    gitlab
        .submit_review(
            &pr(),
            &review(
                Verdict::Comment,
                "",
                vec![draft(
                    LineLocation::line("src/a.rs", Side::New, 13),
                    "Context",
                )],
            ),
        )
        .unwrap();
    let position = &fake.body(Method::Post, &format!("{mr}/discussions"))["position"];
    assert_eq!(position["old_line"], 12);
    assert_eq!(position["new_line"], 13);
    assert!(position.get("line_range").is_none());
    let error = gitlab
        .submit_review(
            &pr(),
            &review(
                Verdict::Comment,
                "",
                vec![draft(LineLocation::line("src/a.rs", Side::New, 99), "Far")],
            ),
        )
        .unwrap_err();
    assert!(
        matches!(error, HostError::Invalid(ref m) if m.contains("line 99")),
        "{error}"
    );
}

#[test]
fn gitlab_encodes_nested_project_paths_and_follows_next_page_headers() {
    let fake = Fake::new();
    let base = "projects/group%2Fsub%2Fweb/merge_requests?state=merged";
    fake.on(
        Method::Get,
        &format!("{base}&per_page=100&page=1"),
        with_headers(
            json!([{"iid": 1, "state": "merged"}]),
            &[("X-Next-Page", "2")],
        ),
    )
    .on(
        Method::Get,
        &format!("{base}&per_page=100&page=2"),
        with_headers(
            json!([{"iid": 2, "state": "merged"}]),
            &[("X-Next-Page", "")],
        ),
    );
    let prs = GitLab::new(fake.transport())
        .pull_requests("group/sub/web", StateFilter::Merged)
        .unwrap();
    assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![1, 2]);
    assert!(prs
        .iter()
        .all(|p| p.state == PrState::Merged && p.repo == "group/sub/web"));
}

#[test]
fn gitlab_replies_edits_and_resolves_in_its_discussion() {
    let fake = Fake::new();
    let mr = "projects/acme%2Fweb/merge_requests/7";
    let note = json(json!({"id": 5, "author": {"username": "me"}, "body": "ok"}));
    fake.on(
        Method::Post,
        &format!("{mr}/discussions/d-1/notes"),
        note.clone(),
    )
    .on(Method::Put, &format!("{mr}/notes/5"), note)
    .on(
        Method::Put,
        &format!("{mr}/discussions/d-1"),
        json(json!({})),
    );
    let gitlab = GitLab::new(fake.transport());
    assert_eq!(gitlab.reply(&pr(), "d-1", "ok").unwrap().id, "5");
    gitlab.edit_comment(&pr(), "5", "better").unwrap();
    gitlab.resolve_thread(&pr(), "d-1", true).unwrap();
    assert_eq!(
        fake.body(Method::Put, &format!("{mr}/notes/5")),
        json!({"body": "better"})
    );
    assert_eq!(
        fake.body(Method::Put, &format!("{mr}/discussions/d-1")),
        json!({"resolved": true})
    );
}

#[test]
fn gitlab_checks_are_the_latest_pipelines_jobs() {
    let fake = Fake::new();
    fake.get(
        "projects/acme%2Fweb/merge_requests/7/pipelines",
        json!([{"id": 42}, {"id": 41}]),
    )
    .get(
        "projects/acme%2Fweb/pipelines/42/jobs?per_page=100&page=1",
        json!([
            {"name": "test", "status": "success"},
            {"name": "lint", "status": "failed"},
            {"name": "deploy", "status": "manual"},
            {"name": "e2e", "status": "running"}
        ]),
    );
    let checks = GitLab::new(fake.transport()).checks(&pr()).unwrap();
    assert_eq!(
        checks.iter().map(|c| c.status).collect::<Vec<_>>(),
        vec![
            CheckStatus::Success,
            CheckStatus::Failure,
            CheckStatus::Skipped,
            CheckStatus::Pending
        ]
    );
}

// Azure DevOps

fn azure(fake: &Fake) -> AzureDevOps {
    AzureDevOps::new(fake.transport(), "https://dev.azure.com/acme-org")
}

const AZ_PR: &str = "acme/_apis/git/repositories/web/pullRequests/7";

#[test]
fn azure_devops_sends_line_threads_then_votes_as_the_signed_in_user() {
    let fake = Fake::new();
    fake.get(
        "_apis/connectionData",
        json!({"authenticatedUser": {"id": "u-1", "providerDisplayName": "Me"}}),
    )
    .on(
        Method::Post,
        &format!("{AZ_PR}/threads?api-version=7.1"),
        json(json!({"id": 3, "comments": [{"id": 1}]})),
    )
    .on(
        Method::Put,
        &format!("{AZ_PR}/reviewers/u-1?api-version=7.1"),
        json(json!({})),
    );
    azure(&fake)
        .submit_review(
            &pr(),
            &review(
                Verdict::RequestChanges,
                "",
                vec![draft(
                    LineLocation::range("src/a.rs", Side::Old, 3, 5),
                    "Range",
                )],
            ),
        )
        .unwrap();
    assert_eq!(
        fake.body(Method::Post, &format!("{AZ_PR}/threads?api-version=7.1")),
        json!({
            "comments": [{"parentCommentId": 0, "content": "Range", "commentType": 1}],
            "status": 1,
            "threadContext": {
                "filePath": "/src/a.rs",
                "leftFileStart": {"line": 3, "offset": 1},
                "leftFileEnd": {"line": 5, "offset": 1}
            }
        })
    );
    assert_eq!(
        fake.body(
            Method::Put,
            &format!("{AZ_PR}/reviewers/u-1?api-version=7.1")
        ),
        json!({"vote": -5}),
        "request changes is \"waiting for author\""
    );
}

#[test]
fn azure_devops_comment_reviews_cast_no_vote() {
    let fake = Fake::new();
    fake.on(
        Method::Post,
        &format!("{AZ_PR}/threads?api-version=7.1"),
        json(json!({"id": 3, "comments": [{"id": 1, "content": "Hi"}]})),
    );
    azure(&fake)
        .submit_review(&pr(), &review(Verdict::Comment, "Hi", vec![]))
        .unwrap();
    assert_eq!(
        fake.calls(),
        vec![format!("POST {AZ_PR}/threads?api-version=7.1")]
    );
}

#[test]
fn azure_devops_reads_files_from_the_latest_iteration_and_votes_as_reviews() {
    let fake = Fake::new();
    fake.get(
        &format!("{AZ_PR}/iterations?api-version=7.1"),
        json!({"value": [{"id": 1}, {"id": 3}, {"id": 2}]}),
    )
    .get(
        &format!("{AZ_PR}/iterations/3/changes?api-version=7.1"),
        json!({"changeEntries": [
            {"item": {"path": "/src/new.rs"}, "changeType": "add"},
            {"item": {"path": "/src/moved.rs"}, "changeType": "edit, rename", "originalPath": "/src/old.rs"},
            {"item": {"path": "/src", "gitObjectType": "tree"}, "changeType": "edit"},
            {"item": {"path": "/README.md"}, "changeType": "delete"}
        ]}),
    )
    .get(
        &format!("{AZ_PR}?api-version=7.1"),
        json!({"pullRequestId": 7, "reviewers": [
            {"uniqueName": "a", "vote": 10}, {"uniqueName": "b", "vote": 5},
            {"uniqueName": "c", "vote": 0}, {"uniqueName": "d", "vote": -10}
        ]}),
    );
    let azure = azure(&fake);
    let files = azure.files(&pr()).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.previous_path.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            ("src/new.rs", FileStatus::Added, None),
            ("src/moved.rs", FileStatus::Renamed, Some("src/old.rs")),
            ("README.md", FileStatus::Deleted, None),
        ]
    );
    assert!(files.iter().all(|f| f.patch.is_none()));
    let reviews = azure.reviews(&pr()).unwrap();
    assert_eq!(
        reviews
            .iter()
            .map(|r| (r.author.as_str(), r.state))
            .collect::<Vec<_>>(),
        vec![
            ("a", ReviewState::Approved),
            ("b", ReviewState::Approved),
            ("d", ReviewState::ChangesRequested),
        ]
    );
}

#[test]
fn azure_devops_escapes_project_names_and_links_to_the_web_pull_request() {
    let fake = Fake::new();
    fake.get(
        "My%20Project/_apis/git/repositories/web/pullRequests/7?api-version=7.1",
        json!({"pullRequestId": 7, "status": "completed",
               "repository": {"name": "web", "project": {"name": "My Project"}}}),
    );
    let pr = azure(&fake)
        .pull_request(&PrRef::new("My Project/web", 7))
        .unwrap();
    assert_eq!(pr.state, PrState::Merged);
    assert_eq!(pr.repo, "My Project/web");
    assert_eq!(
        pr.url,
        "https://dev.azure.com/acme-org/My%20Project/_git/web/pullrequest/7"
    );
}

#[test]
fn azure_devops_edits_and_resolves_through_the_thread() {
    let fake = Fake::new();
    fake.on(
        Method::Patch,
        &format!("{AZ_PR}/threads/5/comments/2?api-version=7.1"),
        json(json!({"id": 2, "content": "better"})),
    )
    .on(
        Method::Patch,
        &format!("{AZ_PR}/threads/5?api-version=7.1"),
        json(json!({})),
    );
    let azure = azure(&fake);
    assert_eq!(
        azure.edit_comment(&pr(), "5:2", "better").unwrap().id,
        "5:2"
    );
    azure.resolve_thread(&pr(), "5", true).unwrap();
    assert_eq!(
        fake.body(Method::Patch, &format!("{AZ_PR}/threads/5?api-version=7.1")),
        json!({"status": "fixed"})
    );
    assert!(matches!(
        azure.edit_comment(&pr(), "2", "x"),
        Err(HostError::Invalid(_))
    ));
}
