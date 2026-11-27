use super::*;
use std::cell::RefCell;

fn session(id: i64, kind: &str, repo: &str, number: u64) -> Session {
    Session {
        id,
        title: format!("session {id}"),
        provider: "claude".into(),
        cwd: "/tmp".into(),
        provider_id: None,
        has_messages: true,
        updated_at: 0,
        created_at: 0,
        archived: false,
        pinned: false,
        work_item: Some(WorkItem::Github(GithubWorkItem {
            kind: kind.into(),
            repo: repo.into(),
            number,
            url: format!("https://github.com/{repo}/pull/{number}"),
        })),
    }
}

fn node(updated: &str) -> Value {
    serde_json::json!({"issueOrPullRequest": {
        "title": "Fix it", "state": "OPEN", "merged": false, "updatedAt": updated
    }})
}

fn finished(success: bool, stdout: &str, stderr: &str) -> Result<Finished, String> {
    Ok(Finished {
        success,
        stdout: stdout.into(),
        stderr: stderr.into(),
    })
}

/// A fake gh: answers GraphQL with `graphql` and every REST call with
/// `rest`, and keeps each call's endpoint (or "graphql").
struct Fake<G, R> {
    calls: RefCell<Vec<String>>,
    graphql: G,
    rest: R,
}
impl<G, R> Fake<G, R>
where
    G: Fn() -> Result<Finished, String>,
    R: Fn(&str) -> Result<Finished, String>,
{
    fn new(graphql: G, rest: R) -> Self {
        Self {
            calls: RefCell::default(),
            graphql,
            rest,
        }
    }
    fn run(&self, args: &[String]) -> Result<Finished, String> {
        if args[1] == "graphql" {
            self.calls.borrow_mut().push("graphql".into());
            (self.graphql)()
        } else {
            let endpoint = args[3].clone();
            self.calls.borrow_mut().push(endpoint.clone());
            (self.rest)(&endpoint)
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

const UPDATED: &str = "2026-11-27T05:00:00Z";
const NOT_FOUND: &str = "gh: Could not resolve to an issue or pull request with the number of 999.";
const RATE_LIMITED: &str = "gh: API rate limit exceeded for user ID 1. (HTTP 403)";

/// The bug that spent 5,000 REST requests in under half an hour: one linked
/// item GitHub couldn't find made gh exit with an error, the whole GraphQL
/// answer was dropped, and every item was asked for again over REST, every
/// minute.
#[test]
fn one_missing_item_keeps_the_others_and_asks_nothing_item_by_item() {
    let data = serde_json::json!({
        "data": {"i0": node(UPDATED), "i1": {"issueOrPullRequest": null}},
        "errors": [{"type": "NOT_FOUND", "message": NOT_FOUND}],
    })
    .to_string();
    let fake = Fake::new(
        || finished(false, &data, NOT_FOUND),
        |_| finished(true, "[]", ""),
    );
    let sessions = vec![
        session(1, "pr", "owner/found", 7),
        session(2, "issue", "owner/gone", 999),
    ];

    let fetched = fetch(sessions, &HashMap::new(), Details::new(), &|a| fake.run(a));

    let cards = fetched.cards.expect("the found item still makes a card");
    assert_eq!(cards.keys().copied().collect::<Vec<_>>(), vec![1]);
    assert_eq!(cards[&1].title, "Fix it");
    assert!(!fetched.failed);
    assert_eq!(
        fake.calls(),
        vec![
            "graphql",
            "repos/owner/found/issues/7/comments?per_page=100",
            "repos/owner/found/pulls/7/reviews?per_page=100",
            "repos/owner/found/pulls/7/comments?per_page=100",
            "repos/owner/found/pulls/7/commits?per_page=100",
        ],
        "only the changed item's detail feeds, never a summary per item"
    );
}

#[test]
fn a_poll_github_refuses_keeps_the_last_cards_and_asks_nothing_else() {
    let fake = Fake::new(
        || finished(false, "", RATE_LIMITED),
        |_| finished(true, "[]", ""),
    );
    let previous: Details = HashMap::from([(
        "repos/a/b/issues/1/comments?per_page=100".into(),
        (5, Value::Null),
    )]);
    let sessions = vec![session(1, "pr", "a/b", 1), session(2, "issue", "a/c", 2)];

    let fetched = fetch(sessions, &HashMap::new(), previous.clone(), &|a| {
        fake.run(a)
    });

    assert!(fetched.cards.is_none(), "the last poll's cards stay");
    assert!(fetched.failed, "the next poll waits");
    assert_eq!(fetched.details, previous, "cached feeds survive");
    assert_eq!(fake.calls(), vec!["graphql"]);
}

#[test]
fn a_timed_out_graphql_request_is_a_failed_poll() {
    let fake = Fake::new(
        || Err("CLI command timed out".into()),
        |_| finished(true, "[]", ""),
    );
    let fetched = fetch(
        vec![session(1, "pr", "a/b", 1)],
        &HashMap::new(),
        Details::new(),
        &|a| fake.run(a),
    );
    assert!(fetched.cards.is_none());
    assert!(fetched.failed);
    assert_eq!(fake.calls(), vec!["graphql"]);
}

#[test]
fn a_rate_limited_detail_feed_stops_the_poll_there_and_waits() {
    let data = serde_json::json!({"data": {"i0": node(UPDATED), "i1": node(UPDATED)}}).to_string();
    let fake = Fake::new(
        || finished(true, &data, ""),
        |_| finished(false, "", RATE_LIMITED),
    );
    let sessions = vec![session(1, "pr", "a/b", 1), session(2, "pr", "a/c", 2)];

    let fetched = fetch(sessions, &HashMap::new(), Details::new(), &|a| fake.run(a));

    assert_eq!(
        fake.calls().len(),
        2,
        "GraphQL, then one feed, then nothing"
    );
    assert!(fetched.failed);
    let cards = fetched.cards.expect("both items still show as updated");
    assert_eq!(cards.len(), 2);
    assert!(cards.values().all(|c| c.details_error));
    assert!(
        fetched.details.is_empty(),
        "a rate-limited feed is asked for again after the wait"
    );
}

#[test]
fn a_feed_that_is_gone_is_not_asked_for_again_until_the_item_changes() {
    let updated = RefCell::new(UPDATED.to_owned());
    let graphql = || {
        finished(
            true,
            &serde_json::json!({"data": {"i0": node(&updated.borrow())}}).to_string(),
            "",
        )
    };
    let fake = Fake::new(graphql, |_| finished(false, "", "gh: Not Found (HTTP 404)"));
    let poll = |details: Details| {
        fetch(
            vec![session(1, "issue", "a/b", 1)],
            &HashMap::new(),
            details,
            &|a| fake.run(a),
        )
    };

    let first = poll(Details::new());
    assert!(!first.failed, "a missing feed isn't a reason to wait");
    assert!(first.cards.unwrap()[&1].details_error);
    assert_eq!(fake.calls().len(), 2);

    let second = poll(first.details);
    assert_eq!(fake.calls().len(), 3, "only GraphQL the second time");
    assert!(second.cards.unwrap()[&1].details_error);

    *updated.borrow_mut() = "2026-11-27T06:00:00Z".into();
    poll(second.details);
    assert_eq!(
        fake.calls().len(),
        5,
        "the changed item's feed is asked again"
    );
}

#[test]
fn no_linked_github_items_means_no_calls() {
    let fake = Fake::new(|| finished(true, "{}", ""), |_| finished(true, "[]", ""));
    let mut archived = session(1, "pr", "a/b", 1);
    archived.archived = true;
    let mut unsafe_slug = session(2, "pr", "a/b c", 2);
    unsafe_slug.work_item = Some(WorkItem::Github(GithubWorkItem {
        kind: "pr".into(),
        repo: "a/b\"){x}".into(),
        number: 2,
        url: String::new(),
    }));

    let fetched = fetch(
        vec![archived, unsafe_slug],
        &HashMap::new(),
        Details::new(),
        &|a| fake.run(a),
    );

    assert!(fake.calls().is_empty());
    assert_eq!(fetched.cards, Some(HashMap::new()));
    assert!(!fetched.failed);
}

#[test]
fn failed_polls_wait_2_4_8_then_15_minutes_and_a_good_poll_resets() {
    let start = Instant::now();
    let minutes = |m: u64| start + Duration::from_secs(m * 60);
    let mut pace = Pace::default();
    assert!(pace.ready(start));

    pace.record(true, start);
    assert!(!pace.ready(minutes(1)));
    assert!(pace.ready(minutes(2)));

    let mut waits = Vec::new();
    for _ in 0..4 {
        pace.record(true, start);
        waits.push((pace.retry_at.unwrap() - start).as_secs() / 60);
    }
    assert_eq!(waits, vec![4, 8, 15, 15]);

    pace.record(false, start);
    assert_eq!(pace, Pace::default());
    assert!(pace.ready(start));
}
