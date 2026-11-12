use super::*;
use crate::review_policy::{Forge, ReviewTarget};
use std::collections::VecDeque;

/// `git diff -M` of a real change set: a two-hunk edit, an added, a deleted
/// and a renamed-and-edited file, a removed line starting with `--`, a file
/// without a final newline and a path with a space.
const PR_DIFF: &str = include_str!("fixtures/pull_request.diff");
/// The same changes in the shape GitLab's `/merge_requests/:iid/diffs` returns.
const MR_DIFFS: &str = include_str!("fixtures/merge_request_diffs.json");

fn map() -> DiffMap {
    DiffMap::parse(PR_DIFF)
}

fn at(path: &str, side: Side, line: u32) -> Option<(Option<u32>, Option<u32>)> {
    map()
        .locate(path, side, line)
        .map(|a| (a.old_line, a.new_line))
}

// ------------------------------------------------------------ diff mapping

#[test]
fn a_two_hunk_edit_maps_added_removed_and_context_lines() {
    // Hunk 1 (@@ -1,8 +1,9 @@): line 3 replaced, a line added after 5.
    assert_eq!(at("src/lib.rs", Side::New, 3), Some((None, Some(3))));
    assert_eq!(at("src/lib.rs", Side::Old, 3), Some((Some(3), None)));
    assert_eq!(at("src/lib.rs", Side::New, 6), Some((None, Some(6))));
    // Context before and after the insertion shifts the new numbers.
    assert_eq!(at("src/lib.rs", Side::New, 5), Some((Some(5), Some(5))));
    assert_eq!(at("src/lib.rs", Side::New, 7), Some((Some(6), Some(7))));
    assert_eq!(at("src/lib.rs", Side::Old, 6), Some((Some(6), Some(7))));
    // Hunk 2 (@@ -27,13 +28,12 @@): old 30 removed, old 36 replaced.
    assert_eq!(at("src/lib.rs", Side::Old, 30), Some((Some(30), None)));
    assert_eq!(at("src/lib.rs", Side::New, 36), Some((None, Some(36))));
    assert_eq!(at("src/lib.rs", Side::New, 28), Some((Some(27), Some(28))));
    assert_eq!(at("src/lib.rs", Side::New, 39), Some((Some(39), Some(39))));
    // Around the removed old 30: new 30 is old 29, new 31 is old 31.
    assert_eq!(at("src/lib.rs", Side::New, 30), Some((Some(29), Some(30))));
    assert_eq!(at("src/lib.rs", Side::New, 31), Some((Some(31), Some(31))));
    assert_eq!(
        map().locate("src/lib.rs", Side::Old, 37).unwrap().new_line,
        Some(37)
    );
}

#[test]
fn lines_outside_every_hunk_are_not_commentable() {
    // Between the hunks, before the second and past the end.
    for line in [10, 15, 20, 26, 27, 41, 1000] {
        assert_eq!(at("src/lib.rs", Side::New, line), None, "new {line}");
    }
    assert_eq!(at("src/lib.rs", Side::Old, 9), None);
    assert_eq!(at("src/lib.rs", Side::Old, 0), None);
    // A file the pull request does not touch.
    assert_eq!(at("src/untouched.rs", Side::New, 1), None);
    assert_eq!(at("", Side::New, 1), None);
}

#[test]
fn a_removed_line_that_starts_with_two_dashes_is_not_a_file_header() {
    let m = map();
    let sql = m
        .files
        .iter()
        .find(|f| f.new_path == "docs/query.sql")
        .unwrap();
    assert_eq!(sql.old_path, "docs/query.sql");
    assert_eq!(at("docs/query.sql", Side::Old, 2), Some((Some(2), None)));
    assert_eq!(at("docs/query.sql", Side::New, 2), Some((Some(3), Some(2))));
    assert_eq!(at("docs/query.sql", Side::New, 3), Some((None, Some(3))));
    // The file after it was still read.
    assert!(m.files.iter().any(|f| f.new_path == "docs/with space.md"));
}

#[test]
fn a_renamed_file_is_found_by_either_name_and_keeps_both_paths() {
    let anchor = map().locate("src/after.rs", Side::New, 6).unwrap();
    assert_eq!(anchor.old_path, "src/before.rs");
    assert_eq!(anchor.new_path, "src/after.rs");
    assert_eq!((anchor.old_line, anchor.new_line), (None, Some(6)));
    let by_old_name = map().locate("src/before.rs", Side::Old, 6).unwrap();
    assert_eq!(by_old_name.new_path, "src/after.rs");
    // The hunk starts at line 3.
    assert_eq!(at("src/after.rs", Side::New, 2), None);
    assert_eq!(at("src/after.rs", Side::New, 3), Some((Some(3), Some(3))));
}

#[test]
fn added_and_deleted_files_have_one_side_only() {
    for line in 1..=3 {
        assert_eq!(at("src/new.rs", Side::New, line), Some((None, Some(line))));
        assert_eq!(at("src/new.rs", Side::Old, line), None);
    }
    let deleted = map().locate("src/old.rs", Side::Old, 2).unwrap();
    assert_eq!(
        (deleted.old_path.as_str(), deleted.new_path.as_str()),
        ("src/old.rs", "src/old.rs")
    );
    assert_eq!(at("src/old.rs", Side::New, 1), None);
}

#[test]
fn no_newline_markers_do_not_shift_line_numbers() {
    assert_eq!(at("notes.txt", Side::Old, 3), Some((Some(3), None)));
    assert_eq!(at("notes.txt", Side::New, 3), Some((None, Some(3))));
    assert_eq!(at("notes.txt", Side::New, 4), Some((None, Some(4))));
    assert_eq!(at("notes.txt", Side::New, 5), None);
    // The file after it still parses from its own header.
    assert_eq!(at("src/after.rs", Side::New, 6), Some((None, Some(6))));
}

#[test]
fn paths_with_spaces_and_the_ways_agents_write_paths_are_matched() {
    assert_eq!(
        at("docs/with space.md", Side::New, 2),
        Some((None, Some(2)))
    );
    for written in ["./src/lib.rs", "src\\lib.rs", " src/lib.rs ", "/src/lib.rs"] {
        assert!(map().locate(written, Side::New, 3).is_some(), "{written}");
    }
}

#[test]
fn windows_line_endings_parse_the_same() {
    let crlf = PR_DIFF.replace('\n', "\r\n");
    assert_eq!(DiffMap::parse(&crlf), map());
}

#[test]
fn gitlab_diffs_map_to_the_same_lines_as_the_git_diff() {
    let rows: Value = serde_json::from_str(MR_DIFFS).unwrap();
    let patch = crate::inbox::gitlab::patch_from_diffs(&rows).unwrap();
    let gitlab = DiffMap::parse(&patch);
    let mut want = map().files;
    let mut got = gitlab.files;
    want.sort_by(|a, b| a.new_path.cmp(&b.new_path));
    got.sort_by(|a, b| a.new_path.cmp(&b.new_path));
    assert_eq!(got, want);
}

// ------------------------------------------------------------ the reply

fn reply(json: &str) -> String {
    format!("Here is my review.\n\n```gespan-review\n{json}\n```\n")
}

#[test]
fn the_last_gespan_review_block_is_read() {
    let text = format!(
        "{}\nOn second thought:\n{}",
        reply(r#"{"summary":"first","comments":[]}"#),
        reply(
            r#"{"summary":"Verdict: fix","comments":[
                {"path":"./src/lib.rs","side":"new","line":3,"body":"Off by one"},
                {"path":"src/lib.rs","side":"LEFT","line":"30","body":"Why remove this?"},
                {"path":"src/lib.rs","side":"removed","line":31,"body":"x"},
                {"path":"src/lib.rs","line":0,"body":"No line"},
                {"path":"src/lib.rs","line":4,"body":"   "},
                {"path":"README.md","body":"General"}
            ]}"#
        )
    );
    let findings = parse_reply(&text);
    assert_eq!(findings.summary, "Verdict: fix");
    let got: Vec<(&str, Side, Option<u32>, &str)> = findings
        .comments
        .iter()
        .map(|f| (f.path.as_str(), f.side, f.line, f.body.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("src/lib.rs", Side::New, Some(3), "Off by one"),
            ("src/lib.rs", Side::Old, Some(30), "Why remove this?"),
            ("src/lib.rs", Side::Old, Some(31), "x"),
            ("src/lib.rs", Side::New, None, "No line"),
            ("README.md", Side::New, None, "General"),
        ]
    );
}

#[test]
fn a_reply_without_a_readable_block_becomes_the_summary() {
    let plain = "Verdict: pass. Nothing blocks.";
    assert_eq!(parse_reply(plain).summary, plain);
    assert!(parse_reply(plain).comments.is_empty());
    let broken = reply("{not json");
    let findings = parse_reply(&broken);
    assert_eq!(findings.summary, broken.trim());
    assert!(findings.comments.is_empty());
    // An unclosed block is not a block.
    let open = "Review\n```gespan-review\n{\"summary\":\"x\"}";
    assert_eq!(parse_reply(open).summary, open);
}

#[test]
fn an_empty_summary_falls_back_to_the_prose_around_the_block() {
    let text = format!("Verdict: pass{}", reply(r#"{"comments":[]}"#));
    assert!(parse_reply(&text).summary.starts_with("Verdict: pass"));
}

#[test]
fn a_longer_fence_and_long_bodies_are_handled() {
    let long = "x".repeat(MAX_BODY + 50);
    let text = format!(
        "````gespan-review\n{{\"summary\":\"s\",\"comments\":[{{\"path\":\"a\",\"line\":1,\"body\":\"{long}\"}}]}}\n````"
    );
    let findings = parse_reply(&text);
    assert_eq!(findings.comments[0].body.chars().count(), MAX_BODY + 1);
    assert!(findings.comments[0].body.ends_with('…'));
}

#[test]
fn slash_commands_in_anything_gespan_posts_are_made_inert() {
    let text = "/merge\n  /approve\nok /not-a-command\n```\n/inside code\n```\n/gespan review";
    assert_eq!(
        inert(text),
        "\\/merge\n  \\/approve\nok /not-a-command\n```\n/inside code\n```\n\\/gespan review"
    );
    // A reply steered by the pull request into quick actions or a new review
    // posts none of them.
    let findings = parse_reply(&reply(
        r#"{"summary":"/approve\n/merge","comments":[{"path":"src/lib.rs","line":3,"body":"/gespan review\n/close"}]}"#,
    ));
    assert_eq!(findings.summary, "\\/approve\n\\/merge");
    assert_eq!(findings.comments[0].body, "\\/gespan review\n\\/close");
    assert!(!crate::automations::is_review_command(
        &findings.comments[0].body
    ));
    // So does a reply without a block.
    assert_eq!(parse_reply("/merge").summary, "\\/merge");
}

// ------------------------------------------------------------ placement

fn finding(path: &str, side: Side, line: Option<u32>, body: &str) -> Finding {
    Finding {
        path: path.into(),
        side,
        line,
        body: body.into(),
    }
}

#[test]
fn findings_off_the_diff_move_into_the_summary_with_their_place() {
    let findings = Findings {
        summary: "Verdict: fix".into(),
        comments: vec![
            finding("src/lib.rs", Side::New, Some(3), "inline"),
            finding("src/lib.rs", Side::New, Some(15), "between hunks"),
            finding("src/lib.rs", Side::Old, Some(3), "old side"),
            finding("../../etc/passwd", Side::New, Some(1), "not in the PR"),
            finding("src/new.rs", Side::Old, Some(1), "wrong side"),
            finding("", Side::New, None, "general"),
        ],
    };
    let plan = Plan::new(findings, &map());
    let inline: Vec<_> = plan.inline.iter().map(|p| p.body.as_str()).collect();
    assert_eq!(inline, ["inline", "old side"]);
    let body = plan.body();
    assert!(
        body.starts_with("Verdict: fix\n\n**Notes on lines outside the diff**"),
        "{body}"
    );
    for want in [
        "- `src/lib.rs` line 15: between hunks",
        "- `../../etc/passwd` line 1: not in the PR",
        "- `src/new.rs` line 1 (old): wrong side",
        "- general",
    ] {
        assert!(body.contains(want), "missing {want}: {body}");
    }
}

#[test]
fn at_most_max_comments_go_inline() {
    let comments = (0..MAX_COMMENTS + 5)
        .map(|i| finding("src/new.rs", Side::New, Some(1), &format!("c{i}")))
        .collect();
    let plan = Plan::new(
        Findings {
            summary: "s".into(),
            comments,
        },
        &map(),
    );
    assert_eq!(plan.inline.len(), MAX_COMMENTS);
    assert_eq!(plan.general.len(), 5);
}

// ------------------------------------------------------------ GitHub

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

fn github() -> ReviewTarget {
    ReviewTarget {
        forge: Forge::GitHub,
        repo: "acme/app".into(),
        number: 7,
    }
}

/// A `gh` that answers from a script and records every call.
struct FakeGh {
    calls: Vec<(Vec<String>, Option<Value>)>,
    answers: VecDeque<Result<String, String>>,
}

impl FakeGh {
    fn new(answers: Vec<Result<&str, &str>>) -> Self {
        Self {
            calls: Vec::new(),
            answers: answers
                .into_iter()
                .map(|a| a.map(str::to_string).map_err(str::to_string))
                .collect(),
        }
    }
    fn run(&mut self, args: &[&str], input: Option<&str>) -> Result<String, String> {
        self.calls.push((
            args.iter().map(|a| a.to_string()).collect(),
            input.map(|i| serde_json::from_str(i).unwrap()),
        ));
        self.answers.pop_front().unwrap_or(Err("no answer".into()))
    }
    fn args(&self, i: usize) -> Vec<&str> {
        self.calls[i].0.iter().map(String::as_str).collect()
    }
}

fn create_github(gh: &mut FakeGh, findings: Findings) -> Result<DraftReview, String> {
    github_create(&github(), findings, &mut |args, input| gh.run(args, input))
}

fn sample_findings() -> Findings {
    parse_reply(&reply(
        r#"{"summary":"Verdict: fix","repo":"evil/other","number":99,"comments":[
            {"path":"src/lib.rs","side":"new","line":3,"body":"Added line"},
            {"path":"src/lib.rs","side":"old","line":30,"body":"Removed line"},
            {"path":"src/lib.rs","side":"new","line":5,"body":"Context line"},
            {"path":"src/after.rs","side":"new","line":6,"body":"Renamed file"},
            {"path":"src/lib.rs","side":"new","line":15,"body":"Off the diff"}
        ]}"#,
    ))
}

const REVIEW: &str = r#"{"id":42,"html_url":"https://github.com/acme/app/pull/7#pullrequestreview-42","state":"PENDING"}"#;

#[test]
fn github_leaves_a_pending_review_on_the_triggering_pull_request_only() {
    let mut gh = FakeGh::new(vec![Ok(HEAD), Ok(PR_DIFF), Ok(REVIEW)]);
    let draft = create_github(&mut gh, sample_findings()).unwrap();
    assert_eq!(gh.calls.len(), 3);
    assert_eq!(
        gh.args(0),
        [
            "pr",
            "view",
            "7",
            "--repo",
            "acme/app",
            "--json",
            "headRefOid",
            "--jq",
            ".headRefOid"
        ]
    );
    assert_eq!(gh.args(1), ["pr", "diff", "7", "--repo", "acme/app"]);
    // The agent's "repo"/"number" fields are ignored: the run's event decides.
    assert_eq!(
        gh.args(2),
        ["api", "--method", "POST", "repos/acme/app/pulls/7/reviews"]
    );
    let payload = gh.calls[2].1.clone().unwrap();
    // No event: GitHub keeps the review pending, visible only to its author.
    assert!(payload.get("event").is_none(), "{payload}");
    assert_eq!(payload["commit_id"], HEAD);
    assert_eq!(
        payload["comments"],
        json!([
            {"path": "src/lib.rs", "line": 3, "side": "RIGHT", "body": "Added line"},
            {"path": "src/lib.rs", "line": 30, "side": "LEFT", "body": "Removed line"},
            {"path": "src/lib.rs", "line": 5, "side": "RIGHT", "body": "Context line"},
            {"path": "src/after.rs", "line": 6, "side": "RIGHT", "body": "Renamed file"},
        ])
    );
    let body = payload["body"].as_str().unwrap();
    assert!(
        body.contains("- `src/lib.rs` line 15: Off the diff"),
        "{body}"
    );
    assert_eq!(draft.handle, Handle::Github { review_id: 42 });
    assert_eq!(
        draft.url,
        "https://github.com/acme/app/pull/7#pullrequestreview-42"
    );
    assert_eq!((draft.inline.len(), draft.outside), (4, 1));
    assert_eq!(draft.body, body);
}

#[test]
fn github_refusing_a_line_keeps_every_finding_in_the_summary() {
    let mut gh = FakeGh::new(vec![
        Ok(HEAD),
        Ok(PR_DIFF),
        Err("HTTP 422: Unprocessable Entity (Line could not be resolved)"),
        Ok(REVIEW),
    ]);
    let draft = create_github(&mut gh, sample_findings()).unwrap();
    assert_eq!(gh.calls.len(), 4);
    let retry = gh.calls[3].1.clone().unwrap();
    assert_eq!(retry["comments"], json!([]));
    let body = retry["body"].as_str().unwrap();
    for want in [
        "Added line",
        "Removed line",
        "line 30 (old)",
        "Renamed file",
        "Off the diff",
    ] {
        assert!(body.contains(want), "missing {want}: {body}");
    }
    assert_eq!((draft.inline.len(), draft.outside), (0, 5));
}

#[test]
fn github_with_a_pending_review_already_says_so_and_does_not_retry() {
    let mut gh = FakeGh::new(vec![
        Ok(HEAD),
        Ok(PR_DIFF),
        Err("HTTP 422: User can only have one pending review per pull request"),
    ]);
    let error = create_github(&mut gh, sample_findings()).unwrap_err();
    assert!(error.contains("already have a pending review"), "{error}");
    assert_eq!(gh.calls.len(), 3);
}

#[test]
fn github_without_a_diff_puts_every_finding_in_the_summary() {
    let mut gh = FakeGh::new(vec![Ok(HEAD), Err("diff too large"), Ok(REVIEW)]);
    let draft = create_github(&mut gh, sample_findings()).unwrap();
    assert_eq!(gh.calls[2].1.clone().unwrap()["comments"], json!([]));
    assert_eq!(draft.outside, 5);
}

#[test]
fn github_refuses_a_bad_head_or_a_gitlab_target_before_writing() {
    let mut gh = FakeGh::new(vec![Ok("not-a-sha; rm -rf /")]);
    assert!(create_github(&mut gh, sample_findings()).is_err());
    assert_eq!(gh.calls.len(), 1);
    let mut gh = FakeGh::new(vec![]);
    let gitlab = ReviewTarget {
        forge: Forge::GitLab,
        ..github()
    };
    assert!(github_create(&gitlab, sample_findings(), &mut |a, i| gh.run(a, i)).is_err());
    assert!(gh.calls.is_empty());
}

#[test]
fn github_submit_sends_the_pending_review_as_a_comment_review() {
    let mut gh = FakeGh::new(vec![Ok(HEAD), Ok(PR_DIFF), Ok(REVIEW), Ok("{}")]);
    let draft = create_github(&mut gh, sample_findings()).unwrap();
    let mut no_gitlab = || -> Result<Box<dyn GitlabHttp>, String> { panic!("GitLab not used") };
    submit(&draft, &mut |a, i| gh.run(a, i), &mut no_gitlab).unwrap();
    assert_eq!(
        gh.args(3),
        [
            "api",
            "--method",
            "POST",
            "repos/acme/app/pulls/7/reviews/42/events"
        ]
    );
    let payload = gh.calls[3].1.clone().unwrap();
    assert_eq!(payload["event"], "COMMENT");
    assert_eq!(payload["body"], draft.body);
}

// ------------------------------------------------------------ GitLab

fn gitlab() -> ReviewTarget {
    ReviewTarget {
        forge: Forge::GitLab,
        repo: "group/sub/app".into(),
        number: 12,
    }
}

const MR_PATH: &str = "/projects/group%2Fsub%2Fapp/merge_requests/12";

#[derive(Default)]
struct FakeGitlab {
    mr: Value,
    patch: Option<String>,
    /// Bodies whose `position.new_path` GitLab refuses.
    refuse_path: Option<String>,
    posts: Vec<(String, Value)>,
    puts: Vec<String>,
    gets: Vec<String>,
}

impl GitlabHttp for FakeGitlab {
    fn get(&mut self, path: &str) -> Result<Value, String> {
        self.gets.push(path.into());
        Ok(self.mr.clone())
    }
    fn post_json(&mut self, path: &str, body: &Value) -> Result<Value, String> {
        if let Some(refused) = &self.refuse_path {
            if body["position"]["new_path"] == refused.as_str() {
                return Err("400 Bad request - Note {:line_code=>[\"can't be blank\"]}".into());
            }
        }
        self.posts.push((path.into(), body.clone()));
        Ok(json!({ "id": 100 + self.posts.len() as u64 }))
    }
    fn put(&mut self, path: &str) -> Result<Value, String> {
        self.puts.push(path.into());
        Ok(json!({}))
    }
    fn mr_patch(&mut self, repo: &str, number: u64) -> Result<String, String> {
        assert_eq!((repo, number), ("group/sub/app", 12));
        self.patch.clone().ok_or_else(|| "no diff".to_string())
    }
}

fn gitlab_fake() -> FakeGitlab {
    let rows: Value = serde_json::from_str(MR_DIFFS).unwrap();
    FakeGitlab {
        mr: json!({
            "web_url": "https://gitlab.example/group/sub/app/-/merge_requests/12",
            "diff_refs": {"base_sha": "b1", "start_sha": "s1", "head_sha": "h1"}
        }),
        patch: Some(crate::inbox::gitlab::patch_from_diffs(&rows).unwrap()),
        ..Default::default()
    }
}

#[test]
fn gitlab_positions_each_draft_note_with_both_paths_and_the_right_lines() {
    let mut fake = gitlab_fake();
    let draft = gitlab_create(&gitlab(), sample_findings(), &mut fake).unwrap();
    assert_eq!(fake.gets, [MR_PATH]);
    let notes = format!("{MR_PATH}/draft_notes");
    assert!(fake.posts.iter().all(|(path, _)| *path == notes));
    let refs =
        json!({"position_type": "text", "base_sha": "b1", "start_sha": "s1", "head_sha": "h1"});
    let position = |extra: Value| {
        let mut p = refs.clone();
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let bodies: Vec<Value> = fake.posts.iter().map(|(_, b)| b.clone()).collect();
    assert_eq!(
        bodies[..4],
        [
            // Added: new_line only.
            json!({"note": "Added line", "position": position(json!({"old_path": "src/lib.rs", "new_path": "src/lib.rs", "new_line": 3}))}),
            // Removed: old_line only.
            json!({"note": "Removed line", "position": position(json!({"old_path": "src/lib.rs", "new_path": "src/lib.rs", "old_line": 30}))}),
            // Unchanged: both.
            json!({"note": "Context line", "position": position(json!({"old_path": "src/lib.rs", "new_path": "src/lib.rs", "old_line": 5, "new_line": 5}))}),
            // Renamed: old and new paths differ.
            json!({"note": "Renamed file", "position": position(json!({"old_path": "src/before.rs", "new_path": "src/after.rs", "new_line": 6}))}),
        ]
    );
    // The summary note has no position and carries the off-diff finding.
    let summary = &bodies[4];
    assert!(summary.get("position").is_none());
    assert!(summary["note"]
        .as_str()
        .unwrap()
        .contains("`src/lib.rs` line 15: Off the diff"));
    // Summary first, so it is published first.
    assert_eq!(
        draft.handle,
        Handle::Gitlab {
            note_ids: vec![105, 101, 102, 103, 104]
        }
    );
    assert_eq!(
        draft.url,
        "https://gitlab.example/group/sub/app/-/merge_requests/12"
    );
    assert_eq!((draft.inline.len(), draft.outside), (4, 1));
}

#[test]
fn gitlab_refusing_a_position_moves_that_finding_into_the_summary() {
    let mut fake = gitlab_fake();
    fake.refuse_path = Some("src/after.rs".into());
    let draft = gitlab_create(&gitlab(), sample_findings(), &mut fake).unwrap();
    assert_eq!(draft.inline.len(), 3);
    assert_eq!(draft.outside, 2);
    let summary = fake.posts.last().unwrap().1["note"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        summary.contains("`src/after.rs` line 6: Renamed file"),
        "{summary}"
    );
}

#[test]
fn gitlab_without_diff_refs_or_a_diff_leaves_one_summary_note() {
    for broken in [0, 1] {
        let mut fake = gitlab_fake();
        if broken == 0 {
            fake.mr = json!({"web_url": "u"});
        } else {
            fake.patch = None;
        }
        let draft = gitlab_create(&gitlab(), sample_findings(), &mut fake).unwrap();
        assert_eq!(fake.posts.len(), 1, "case {broken}");
        assert!(fake.posts[0].1.get("position").is_none());
        assert_eq!(draft.outside, 5);
    }
}

#[test]
fn gitlab_submit_publishes_only_gespans_own_notes() {
    let mut fake = gitlab_fake();
    let draft = gitlab_create(&gitlab(), sample_findings(), &mut fake).unwrap();
    let mut opened = Some(fake);
    let mut gh = FakeGh::new(vec![]);
    let mut open = || -> Result<Box<dyn GitlabHttp>, String> {
        Ok(Box::new(opened.take().expect("opened once")))
    };
    // Routed to GitLab, never to `gh`.
    submit(&draft, &mut |a, i| gh.run(a, i), &mut open).unwrap();
    assert!(gh.calls.is_empty());

    assert!(opened.is_none());
    // The exact calls, through a fake that keeps them.
    let mut fake = gitlab_fake();
    gitlab_submit(&draft, &[105, 101], &mut fake).unwrap();
    assert_eq!(
        fake.puts,
        [
            format!("{MR_PATH}/draft_notes/105/publish"),
            format!("{MR_PATH}/draft_notes/101/publish")
        ]
    );
}

#[test]
fn a_draft_whose_handle_does_not_match_its_forge_is_not_submitted() {
    let mut fake = gitlab_fake();
    let mut draft = gitlab_create(&gitlab(), sample_findings(), &mut fake).unwrap();
    draft.handle = Handle::Github { review_id: 1 };
    let mut gh = FakeGh::new(vec![]);
    let mut no_gitlab = || -> Result<Box<dyn GitlabHttp>, String> { panic!("not opened") };
    assert!(submit(&draft, &mut |a, i| gh.run(a, i), &mut no_gitlab).is_err());
    assert!(gh.calls.is_empty());
}

#[test]
fn drafts_round_trip_through_json() {
    let mut fake = gitlab_fake();
    let draft = gitlab_create(&gitlab(), sample_findings(), &mut fake).unwrap();
    let text = serde_json::to_string(&draft).unwrap();
    assert_eq!(serde_json::from_str::<DraftReview>(&text).unwrap(), draft);
}
