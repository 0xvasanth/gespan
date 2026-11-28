//! The service's own rules, the same on every host: what it refuses before
//! a request goes out, and how it orders and merges what comes back. A
//! scripted host stands in, so each rule is seen apart from any API.
use super::super::{host::PullRequestHost, *};
use std::sync::{Arc, Mutex};

/// A host that answers from fields and counts the calls that reach it.
#[derive(Clone, Default)]
struct Scripted {
    capabilities: Capabilities,
    prs: Vec<PullRequest>,
    threads: Vec<Thread>,
    commits: Vec<Commit>,
    reviews: Vec<Review>,
    checks: Vec<Check>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Scripted {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn called(&self, what: &str) {
        self.calls.lock().unwrap().push(what.to_owned());
    }
    fn service(&self) -> PullRequestService {
        PullRequestService::new(Box::new(self.clone()))
    }
}

fn comment(author: &str, body: &str, at: &str) -> Comment {
    Comment {
        id: format!("{author}-{at}"),
        author: author.into(),
        body: body.into(),
        created_at: at.into(),
        updated_at: at.into(),
        system: false,
    }
}

impl PullRequestHost for Scripted {
    fn kind(&self) -> HostKind {
        HostKind::GitHub
    }
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    fn current_user(&self) -> Result<String> {
        Ok("me".into())
    }
    fn repositories(&self) -> Result<Vec<Repository>> {
        Ok(Vec::new())
    }
    fn pull_requests(&self, _: &str, _: StateFilter) -> Result<Vec<PullRequest>> {
        self.called("pull_requests");
        Ok(self.prs.clone())
    }
    fn pull_request(&self, _: &PrRef) -> Result<PullRequest> {
        Ok(self.prs[0].clone())
    }
    fn review_requests(&self) -> Result<Vec<PullRequest>> {
        Ok(self.prs.clone())
    }
    fn threads(&self, _: &PrRef) -> Result<Vec<Thread>> {
        Ok(self.threads.clone())
    }
    fn add_comment(&self, _: &PrRef, body: &str) -> Result<Comment> {
        self.called(&format!("comment {body}"));
        Ok(comment("me", body, "2026-11-20T10:00:00Z"))
    }
    fn reply(&self, _: &PrRef, thread: &str, body: &str) -> Result<Comment> {
        self.called(&format!("reply {thread} {body}"));
        Ok(comment("me", body, "2026-11-20T10:00:00Z"))
    }
    fn edit_comment(&self, _: &PrRef, id: &str, body: &str) -> Result<Comment> {
        self.called(&format!("edit {id} {body}"));
        Ok(comment("me", body, "2026-11-20T10:00:00Z"))
    }
    fn resolve_thread(&self, _: &PrRef, thread: &str, resolved: bool) -> Result<()> {
        self.called(&format!("resolve {thread} {resolved}"));
        Ok(())
    }
    fn commits(&self, _: &PrRef) -> Result<Vec<Commit>> {
        Ok(self.commits.clone())
    }
    fn files(&self, _: &PrRef) -> Result<Vec<ChangedFile>> {
        Ok(Vec::new())
    }
    fn reviews(&self, _: &PrRef) -> Result<Vec<Review>> {
        Ok(self.reviews.clone())
    }
    fn submit_review(&self, _: &PrRef, review: &ReviewSubmission) -> Result<()> {
        self.called(&format!(
            "review {:?} {}",
            review.verdict,
            review.comments.len()
        ));
        Ok(())
    }
    fn checks(&self, _: &PrRef) -> Result<Vec<Check>> {
        Ok(self.checks.clone())
    }
}

fn pr() -> PrRef {
    PrRef::new("acme/web", 7)
}

fn line_comment(location: LineLocation, body: &str) -> DraftComment {
    DraftComment {
        location,
        body: body.into(),
    }
}

#[test]
fn empty_words_are_refused_before_the_host_hears_of_them() {
    let host = Scripted {
        capabilities: Capabilities {
            edit_comments: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let service = host.service();
    for result in [
        service.comment(&pr(), "   \n").map(|_| ()),
        service.reply(&pr(), "t1", "").map(|_| ()),
        service.reply(&pr(), " ", "words").map(|_| ()),
        service.edit_comment(&pr(), "c1", "\t").map(|_| ()),
        service.comment(&PrRef::new("", 7), "words").map(|_| ()),
        service
            .comment(&PrRef::new("acme/web", 0), "words")
            .map(|_| ()),
    ] {
        assert!(matches!(result, Err(HostError::Invalid(_))), "{result:?}");
    }
    assert!(host.calls().is_empty(), "{:?}", host.calls());
    // Words are sent trimmed.
    service.comment(&pr(), "  Looks good \n").unwrap();
    assert_eq!(host.calls(), ["comment Looks good"]);
}

#[test]
fn what_a_host_cant_do_is_refused_before_anything_is_sent() {
    let host = Scripted::default();
    let service = host.service();
    assert_eq!(
        service.edit_comment(&pr(), "c1", "new").unwrap_err(),
        HostError::Unsupported("editing comments")
    );
    assert_eq!(
        service.resolve_thread(&pr(), "t1", true).unwrap_err(),
        HostError::Unsupported("resolving threads")
    );
    let request_changes = ReviewSubmission {
        verdict: Verdict::RequestChanges,
        body: "Please fix".into(),
        comments: vec![],
    };
    assert_eq!(
        service.submit_review(&pr(), &request_changes).unwrap_err(),
        HostError::Unsupported("requesting changes")
    );
    let ranged = ReviewSubmission {
        verdict: Verdict::Comment,
        body: String::new(),
        comments: vec![line_comment(
            LineLocation::range("a.rs", Side::New, 3, 5),
            "Range",
        )],
    };
    assert_eq!(
        service.submit_review(&pr(), &ranged).unwrap_err(),
        HostError::Unsupported("comments on a range of lines")
    );
    assert!(host.calls().is_empty(), "{:?}", host.calls());
    // A one-line "range" is a single line, which every host takes.
    let single = ReviewSubmission {
        comments: vec![line_comment(
            LineLocation::range("a.rs", Side::New, 5, 5),
            "One",
        )],
        ..ranged
    };
    service.submit_review(&pr(), &single).unwrap();
    assert_eq!(host.calls(), ["review Comment 1"]);
}

#[test]
fn a_review_must_say_something_and_point_at_real_lines() {
    let host = Scripted {
        capabilities: Capabilities {
            line_ranges: true,
            request_changes: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let service = host.service();
    let bad = [
        ReviewSubmission::default(),
        ReviewSubmission {
            verdict: Verdict::RequestChanges,
            ..Default::default()
        },
        ReviewSubmission {
            comments: vec![line_comment(LineLocation::line("a.rs", Side::New, 3), "  ")],
            ..Default::default()
        },
        ReviewSubmission {
            comments: vec![line_comment(
                LineLocation::range("a.rs", Side::New, 9, 4),
                "Backwards",
            )],
            ..Default::default()
        },
        ReviewSubmission {
            comments: vec![line_comment(
                LineLocation::line("a.rs", Side::New, 0),
                "No line",
            )],
            ..Default::default()
        },
        ReviewSubmission {
            comments: vec![line_comment(
                LineLocation::line("", Side::New, 3),
                "No file",
            )],
            ..Default::default()
        },
    ];
    for review in &bad {
        assert!(
            matches!(
                service.submit_review(&pr(), review),
                Err(HostError::Invalid(_))
            ),
            "{review:?}"
        );
    }
    assert!(host.calls().is_empty());
    // Approving needs no words.
    service
        .submit_review(
            &pr(),
            &ReviewSubmission {
                verdict: Verdict::Approve,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(host.calls(), ["review Approve 0"]);
}

fn listed(number: u64, repo: &str, state: PrState, updated: &str) -> PullRequest {
    PullRequest {
        repo: repo.into(),
        number,
        title: format!("#{number}"),
        body: String::new(),
        state,
        draft: false,
        author: "dana".into(),
        source_branch: "x".into(),
        target_branch: "main".into(),
        url: String::new(),
        created_at: updated.into(),
        updated_at: updated.into(),
        head_sha: None,
        reviewers: vec![],
    }
}

#[test]
fn review_requests_are_open_once_each_newest_first() {
    let host = Scripted {
        prs: vec![
            listed(1, "acme/web", PrState::Open, "2026-11-20T09:00:00Z"),
            listed(2, "acme/web", PrState::Open, "2026-11-20T11:00:00.000Z"),
            listed(1, "acme/web", PrState::Open, "2026-11-20T09:00:00Z"),
            listed(1, "acme/api", PrState::Open, "2026-11-20T10:00:00Z"),
            listed(3, "acme/web", PrState::Merged, "2026-11-20T12:00:00Z"),
        ],
        ..Default::default()
    };
    let refs: Vec<(String, u64)> = host
        .service()
        .review_requests()
        .unwrap()
        .into_iter()
        .map(|p| (p.repo, p.number))
        .collect();
    assert_eq!(
        refs,
        [
            ("acme/web".into(), 2),
            ("acme/api".into(), 1),
            ("acme/web".into(), 1)
        ]
    );
    assert!(matches!(
        host.service().pull_requests(" ", StateFilter::Open),
        Err(HostError::Invalid(_))
    ));
    assert!(!host.calls().contains(&"pull_requests".to_owned()));
}

#[test]
fn the_conversation_reads_in_order_and_leaves_host_notes_out() {
    let mut system = comment("bot", "added 1 commit", "2026-11-20T09:00:00Z");
    system.system = true;
    let thread = |id: &str, comments: Vec<Comment>, path: Option<&str>| Thread {
        id: id.into(),
        location: path.map(|p| LineLocation::line(p, Side::New, 3)),
        resolved: None,
        comments,
    };
    let host = Scripted {
        threads: vec![
            thread(
                "late",
                vec![
                    comment("b", "second", "2026-11-20T12:00:00Z"),
                    comment("a", "first", "2026-11-20T11:00:00Z"),
                ],
                Some("src/a.rs"),
            ),
            thread("notes", vec![system.clone()], None),
            thread("empty", vec![], None),
            thread(
                "early",
                vec![system, comment("c", "hello", "2026-11-20T10:00:00Z")],
                None,
            ),
        ],
        ..Default::default()
    };
    let service = host.service();
    let threads = service.conversation(&pr()).unwrap();
    let seen: Vec<(&str, Vec<&str>)> = threads
        .iter()
        .map(|t| {
            (
                t.id.as_str(),
                t.comments.iter().map(|c| c.body.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [("early", vec!["hello"]), ("late", vec!["first", "second"])]
    );
    let on_file = service.file_threads(&pr(), "src/a.rs").unwrap();
    assert_eq!(on_file.len(), 1);
    assert_eq!(on_file[0].id, "late");
    assert!(service.file_threads(&pr(), "src/b.rs").unwrap().is_empty());
}

#[test]
fn verdicts_keep_each_reviewers_latest_stand_and_a_dismissal_clears_it() {
    let review = |author: &str, state: ReviewState, at: &str| Review {
        author: author.into(),
        state,
        body: String::new(),
        submitted_at: Some(at.into()),
    };
    let host = Scripted {
        reviews: vec![
            review("ann", ReviewState::Approved, "2026-11-20T12:00:00Z"),
            review("ann", ReviewState::ChangesRequested, "2026-11-20T10:00:00Z"),
            review("ann", ReviewState::Commented, "2026-11-20T13:00:00Z"),
            review("bob", ReviewState::ChangesRequested, "2026-11-20T09:00:00Z"),
            review("bob", ReviewState::Dismissed, "2026-11-20T11:00:00Z"),
            review("cy", ReviewState::ChangesRequested, "2026-11-20T09:30:00Z"),
        ],
        ..Default::default()
    };
    let verdicts: Vec<(String, ReviewState)> = host
        .service()
        .verdicts(&pr())
        .unwrap()
        .into_iter()
        .map(|r| (r.author, r.state))
        .collect();
    assert_eq!(
        verdicts,
        [
            ("cy".into(), ReviewState::ChangesRequested),
            ("ann".into(), ReviewState::Approved),
        ]
    );
}

#[test]
fn commits_read_oldest_first_and_checks_are_counted() {
    let commit = |sha: &str, at: &str| Commit {
        sha: sha.into(),
        title: sha.into(),
        message: sha.into(),
        author: "dana".into(),
        authored_at: at.into(),
        url: String::new(),
    };
    let check = |status: CheckStatus| Check {
        name: format!("{status:?}"),
        status,
        url: None,
    };
    let host = Scripted {
        commits: vec![
            commit("c", "2026-11-20T11:00:00Z"),
            commit("a", "2026-11-20T09:00:00+00:00"),
            commit("b", "2026-11-20T10:00:00.500Z"),
        ],
        checks: vec![
            check(CheckStatus::Success),
            check(CheckStatus::Success),
            check(CheckStatus::Failure),
            check(CheckStatus::Cancelled),
            check(CheckStatus::Pending),
            check(CheckStatus::Skipped),
        ],
        ..Default::default()
    };
    let service = host.service();
    let order: Vec<String> = service
        .commits(&pr())
        .unwrap()
        .into_iter()
        .map(|c| c.sha)
        .collect();
    assert_eq!(order, ["a", "b", "c"]);
    assert_eq!(
        service.check_summary(&pr()).unwrap(),
        CheckSummary {
            passed: 2,
            failed: 2,
            pending: 1,
            skipped: 1
        }
    );
}

#[test]
fn host_kinds_keep_the_ids_gespan_already_stores() {
    for kind in [HostKind::GitHub, HostKind::GitLab, HostKind::AzureDevOps] {
        assert_eq!(HostKind::from_id(kind.id()), Some(kind));
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            serde_json::json!(kind.id())
        );
    }
    assert_eq!(HostKind::from_id("bitbucket"), None);
}
