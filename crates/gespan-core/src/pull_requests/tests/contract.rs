//! One pull request, told three ways. GitHub, GitLab and Azure DevOps each
//! answer in their own JSON, order and timestamp style; through the service
//! all three read the same. This is the promise the layer makes, so a new
//! host proves itself here first.
//!
//! The pull request, on every host: "Add search" by dana from
//! feature/search into main, head abc123, review asked of me. lee leaves a
//! general comment at 10:00. At 11:00 I comment on lines 12 to 14 of
//! src/search.rs; dana replies at 12:00. dana pushed "Add search index" at
//! 09:00 and "Fix typo" at 09:30. I commented, then approved.
use super::{super::*, Fake};
use crate::pull_requests::{azure_devops::AzureDevOps, github::GitHub, gitlab::GitLab};
use serde_json::json;

const PR: u64 = 7;

fn pr() -> PrRef {
    PrRef::new("acme/web", PR)
}

fn github() -> PullRequestService {
    let fake = Fake::new();
    fake.get(
        "repos/acme/web/pulls/7",
        json!({
            "number": 7, "title": "Add search", "body": "Adds search.", "state": "open",
            "draft": false, "merged_at": null, "user": {"login": "dana"},
            "head": {"ref": "feature/search", "sha": "abc123"},
            "base": {"ref": "main", "repo": {"full_name": "acme/web"}},
            "html_url": "https://github.com/acme/web/pull/7",
            "created_at": "2026-11-20T08:00:00Z", "updated_at": "2026-11-20T12:00:00Z",
            "requested_reviewers": [{"login": "me"}]
        }),
    )
    .get(
        "repos/acme/web/issues/7/comments?per_page=100&page=1",
        json!([{
            "id": 1, "user": {"login": "lee"}, "body": "Looks good overall",
            "created_at": "2026-11-20T10:00:00Z", "updated_at": "2026-11-20T10:00:00Z"
        }]),
    )
    // The reply is listed before the comment it answers.
    .get(
        "repos/acme/web/pulls/7/comments?per_page=100&page=1",
        json!([
            {
                "id": 21, "in_reply_to_id": 20, "user": {"login": "dana"},
                "body": "Done in abc123", "path": "src/search.rs", "line": 14,
                "side": "RIGHT", "created_at": "2026-11-20T12:00:00Z",
                "updated_at": "2026-11-20T12:00:00Z"
            },
            {
                "id": 20, "user": {"login": "me"}, "body": "Can we index this?",
                "path": "src/search.rs", "line": 14, "start_line": 12, "side": "RIGHT",
                "created_at": "2026-11-20T11:00:00Z", "updated_at": "2026-11-20T11:00:00Z"
            }
        ]),
    )
    .get(
        "repos/acme/web/pulls/7/commits?per_page=100&page=1",
        json!([
            {"sha": "111", "author": {"login": "dana"}, "html_url": "u1",
             "commit": {"message": "Add search index\n\nUses a trie.",
                        "author": {"name": "Dana", "date": "2026-11-20T09:00:00Z"}}},
            {"sha": "abc123", "author": {"login": "dana"}, "html_url": "u2",
             "commit": {"message": "Fix typo",
                        "author": {"name": "Dana", "date": "2026-11-20T09:30:00Z"}}}
        ]),
    )
    .get(
        "repos/acme/web/pulls/7/reviews?per_page=100&page=1",
        json!([
            {"user": {"login": "me"}, "state": "COMMENTED", "body": "",
             "submitted_at": "2026-11-20T11:00:00Z"},
            {"user": {"login": "me"}, "state": "APPROVED", "body": "Ship it",
             "submitted_at": "2026-11-20T12:30:00Z"}
        ]),
    );
    PullRequestService::new(Box::new(GitHub::new(fake.transport())))
}

fn gitlab() -> PullRequestService {
    let fake = Fake::new();
    let mr = "projects/acme%2Fweb/merge_requests/7";
    fake.get(
        mr,
        json!({
            "iid": 7, "title": "Add search", "description": "Adds search.", "state": "opened",
            "draft": false, "author": {"username": "dana"},
            "source_branch": "feature/search", "target_branch": "main", "sha": "abc123",
            "web_url": "https://gitlab.com/acme/web/-/merge_requests/7",
            "references": {"full": "acme/web!7"},
            "created_at": "2026-11-20T08:00:00.000Z", "updated_at": "2026-11-20T12:00:00.000Z",
            "reviewers": [{"username": "me"}]
        }),
    )
    // GitLab writes its own notes into the conversation, and lists the
    // review thread first.
    .get(
        &format!("{mr}/discussions?per_page=100&page=1"),
        json!([
            {"id": "d-review", "individual_note": false, "notes": [
                {"id": 20, "author": {"username": "me"}, "body": "Can we index this?",
                 "created_at": "2026-11-20T11:00:00.000Z", "updated_at": "2026-11-20T11:00:00.000Z",
                 "system": false, "resolvable": true, "resolved": false,
                 "position": {"new_path": "src/search.rs", "old_path": "src/search.rs",
                              "new_line": 14, "old_line": null,
                              "line_range": {"start": {"new_line": 12}, "end": {"new_line": 14}}}},
                {"id": 21, "author": {"username": "dana"}, "body": "Done in abc123",
                 "created_at": "2026-11-20T12:00:00.000Z", "updated_at": "2026-11-20T12:00:00.000Z",
                 "system": false, "resolvable": true, "resolved": false}
            ]},
            {"id": "d-system", "individual_note": true, "notes": [
                {"id": 30, "author": {"username": "dana"}, "body": "added 1 commit",
                 "created_at": "2026-11-20T09:31:00.000Z", "updated_at": "2026-11-20T09:31:00.000Z",
                 "system": true}
            ]},
            {"id": "d-general", "individual_note": true, "notes": [
                {"id": 1, "author": {"username": "lee"}, "body": "Looks good overall",
                 "created_at": "2026-11-20T10:00:00.000Z", "updated_at": "2026-11-20T10:00:00.000Z",
                 "system": false}
            ]}
        ]),
    )
    // Newest first, as GitLab lists them.
    .get(
        &format!("{mr}/commits?per_page=100&page=1"),
        json!([
            {"id": "abc123", "title": "Fix typo", "message": "Fix typo",
             "author_name": "dana", "authored_date": "2026-11-20T09:30:00.000Z", "web_url": "u2"},
            {"id": "111", "title": "Add search index", "message": "Add search index\n\nUses a trie.",
             "author_name": "dana", "authored_date": "2026-11-20T09:00:00.000Z", "web_url": "u1"}
        ]),
    )
    .get(
        &format!("{mr}/approvals"),
        json!({"approved_by": [{"user": {"username": "me"}}]}),
    );
    PullRequestService::new(Box::new(GitLab::new(fake.transport())))
}

fn azure() -> PullRequestService {
    let fake = Fake::new();
    let pr = "acme/_apis/git/repositories/web/pullRequests/7";
    fake.get(
        &format!("{pr}?api-version=7.1"),
        json!({
            "pullRequestId": 7, "title": "Add search", "description": "Adds search.",
            "status": "active", "isDraft": false, "createdBy": {"uniqueName": "dana"},
            "sourceRefName": "refs/heads/feature/search", "targetRefName": "refs/heads/main",
            "creationDate": "2026-11-20T08:00:00.1234567Z",
            "lastMergeSourceCommit": {"commitId": "abc123"},
            "repository": {"name": "web", "project": {"name": "acme"}},
            "reviewers": [{"uniqueName": "me", "vote": 10}]
        }),
    )
    .get(
        &format!("{pr}/threads?api-version=7.1"),
        json!({"value": [
            {"id": 5, "status": "active",
             "threadContext": {"filePath": "/src/search.rs",
                               "rightFileStart": {"line": 12, "offset": 1},
                               "rightFileEnd": {"line": 14, "offset": 1}},
             "comments": [
                {"id": 2, "author": {"uniqueName": "dana"}, "content": "Done in abc123",
                 "publishedDate": "2026-11-20T12:00:00.000Z", "commentType": "text"},
                {"id": 1, "author": {"uniqueName": "me"}, "content": "Can we index this?",
                 "publishedDate": "2026-11-20T11:00:00.000Z", "commentType": "text"}
             ]},
            {"id": 6, "comments": [
                {"id": 1, "author": {"uniqueName": "dana"}, "content": "dana pushed 1 commit",
                 "publishedDate": "2026-11-20T09:31:00.000Z", "commentType": "system"}
            ]},
            {"id": 9, "isDeleted": true, "comments": [
                {"id": 1, "author": {"uniqueName": "lee"}, "content": "Removed",
                 "publishedDate": "2026-11-20T09:40:00.000Z", "commentType": "text"}
            ]},
            {"id": 4, "status": "active", "threadContext": null, "comments": [
                {"id": 1, "author": {"uniqueName": "lee"}, "content": "Looks good overall",
                 "publishedDate": "2026-11-20T10:00:00.000Z", "commentType": "text"}
            ]}
        ]}),
    )
    .get(
        &format!("{pr}/commits?api-version=7.1"),
        json!({"value": [
            {"commitId": "abc123", "comment": "Fix typo",
             "author": {"name": "dana", "date": "2026-11-20T09:30:00Z"}},
            {"commitId": "111", "comment": "Add search index\n\nUses a trie.",
             "author": {"name": "dana", "date": "2026-11-20T09:00:00Z"}}
        ]}),
    );
    PullRequestService::new(Box::new(AzureDevOps::new(
        fake.transport(),
        "https://dev.azure.com/acme-org",
    )))
}

/// A thread as read: where it is, and who said what.
type Talk = (Option<LineLocation>, Vec<(String, String)>);

/// What a reviewer sees of the pull request, without the ids and links each
/// host spells its own way.
#[derive(Debug, PartialEq)]
struct Seen {
    title: String,
    author: String,
    state: PrState,
    branches: (String, String),
    head: Option<String>,
    reviewers: Vec<String>,
    conversation: Vec<Talk>,
    commits: Vec<(String, String, String)>,
    verdicts: Vec<(String, ReviewState)>,
}

fn seen(service: &PullRequestService) -> Seen {
    let pr = service.pull_request(&pr()).unwrap();
    assert_eq!(pr.repo, "acme/web", "{:?}", service.kind());
    assert_eq!(pr.number, PR);
    Seen {
        title: pr.title,
        author: pr.author,
        state: pr.state,
        branches: (pr.source_branch, pr.target_branch),
        head: pr.head_sha,
        reviewers: pr.reviewers,
        conversation: service
            .conversation(&self::pr())
            .unwrap()
            .into_iter()
            .map(|t| {
                (
                    t.location,
                    t.comments.into_iter().map(|c| (c.author, c.body)).collect(),
                )
            })
            .collect(),
        commits: service
            .commits(&self::pr())
            .unwrap()
            .into_iter()
            .map(|c| (c.sha, c.title, c.author))
            .collect(),
        verdicts: service
            .verdicts(&self::pr())
            .unwrap()
            .into_iter()
            .map(|r| (r.author, r.state))
            .collect(),
    }
}

fn expected() -> Seen {
    let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
    Seen {
        title: "Add search".into(),
        author: "dana".into(),
        state: PrState::Open,
        branches: pair("feature/search", "main"),
        head: Some("abc123".into()),
        reviewers: vec!["me".into()],
        conversation: vec![
            (None, vec![pair("lee", "Looks good overall")]),
            (
                Some(LineLocation::range("src/search.rs", Side::New, 12, 14)),
                vec![
                    pair("me", "Can we index this?"),
                    pair("dana", "Done in abc123"),
                ],
            ),
        ],
        commits: vec![
            ("111".into(), "Add search index".into(), "dana".into()),
            ("abc123".into(), "Fix typo".into(), "dana".into()),
        ],
        verdicts: vec![("me".into(), ReviewState::Approved)],
    }
}

#[test]
fn github_reads_the_pull_request_like_every_host() {
    assert_eq!(seen(&github()), expected());
}

#[test]
fn gitlab_reads_the_pull_request_like_every_host() {
    assert_eq!(seen(&gitlab()), expected());
}

#[test]
fn azure_devops_reads_the_pull_request_like_every_host() {
    assert_eq!(seen(&azure()), expected());
}

/// Each host's thread and comment ids go back to it unchanged, so a reply
/// or an edit from the conversation reaches the right thread.
#[test]
fn thread_ids_from_the_conversation_reach_the_same_thread_again() {
    let review_thread = |service: PullRequestService| {
        service
            .conversation(&pr())
            .unwrap()
            .into_iter()
            .find(|t| t.location.is_some())
            .unwrap()
    };
    let github = review_thread(github());
    assert_eq!(github.id, "review:20");
    assert_eq!(github.comments[1].id, "review:21");
    let gitlab = review_thread(gitlab());
    assert_eq!(gitlab.id, "d-review");
    assert_eq!(gitlab.resolved, Some(false));
    assert_eq!(gitlab.comments[1].id, "21");
    let azure = review_thread(azure());
    assert_eq!(azure.id, "5");
    assert_eq!(azure.resolved, Some(false));
    assert_eq!(azure.comments[1].id, "5:2");
}
