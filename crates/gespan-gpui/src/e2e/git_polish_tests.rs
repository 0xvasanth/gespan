//! Polish on the Pull requests page: a review comment goes to a one-off agent
//! turn in the background, with the chosen tone, and its rewrite comes back
//! as an offer to Use, Try again or Keep mine. Nothing opens on screen: no Ask
//! panel and no session. Fake by default; `GESPAN_E2E_REAL=1` asks the real
//! `claude` and checks only that a rewrite came back.
use super::*;
use gespan_core::inbox::{Item, polish::Tone};
use gpui::TestAppContext;

fn pull_request(project: &Path) -> Item {
    Item {
        provider: "github".into(),
        kind: "pr".into(),
        repo: "acme/web".into(),
        number: 562,
        title: "Toasty: move the seen-state to typed queries".into(),
        url: "https://github.com/acme/web/pull/562".into(),
        state: "open".into(),
        author: "devon".into(),
        viewer: "me".into(),
        requested_reviewers: vec!["me".into()],
        project_path: project.to_string_lossy().into_owned(),
        ..Default::default()
    }
}

/// The Pull requests page on one pull request, with the background agent
/// scripted to answer `turns`.
fn open_git(e2e: &E2e, turns: Value, cx: &mut TestAppContext) -> Entity<crate::inbox::Inbox> {
    if e2e.mode() == Mode::Fake {
        e2e.script_background(turns, cx);
    }
    let item = pull_request(e2e.repo());
    let git = e2e
        .window()
        .update(cx, |shell, window, cx| {
            shell.e2e_open_git(vec![item], window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    git
}

#[gpui::test]
fn polish_rewrites_a_review_comment_in_the_background(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let rewrite = "Good catch, thanks. Could you add an index on key in this migration?";
    let git = open_git(
        &e2e,
        serde_json::json!([format!("Here it is:\n<polished>\n{rewrite}\n</polished>")]),
        cx,
    );
    let sessions = e2e.all_sessions(cx);

    git.update(cx, |git, cx| {
        git.polish_for_test("this is wrong, add the index", Tone::Friendly, cx)
    });
    e2e.until(cx, "the polished comment", |_, cx| {
        git.read_with(cx, |git, _| git.polish_outcome_for_test().is_some())
    });

    let (tone, text) = git
        .read_with(cx, |git, _| git.polish_outcome_for_test())
        .unwrap()
        .unwrap_or_else(|error| panic!("no rewrite: {error}"));
    assert_eq!(tone, Tone::Friendly);
    match e2e.mode() {
        Mode::Fake => assert_eq!(text, rewrite),
        Mode::Real => assert!(!text.trim().is_empty()),
    }
    // Nothing on screen: the panel stays shut and no session is made.
    assert!(!git.read_with(cx, |git, _| git.ask_open_for_test()));
    assert!(git.read_with(cx, |git, _| git.ask_conversation().is_none()));
    assert_eq!(e2e.all_sessions(cx), sessions, "Polish makes no session");

    if e2e.mode() == Mode::Fake {
        let prompts = e2e.background_prompts();
        assert_eq!(prompts.len(), 1, "one Polish, one turn: {prompts:?}");
        let prompt = &prompts[0];
        assert!(prompt.contains("Rewrite my review comment"), "{prompt}");
        assert!(
            prompt.contains("Friendly: warm"),
            "the tone travels: {prompt}"
        );
        assert!(prompt.contains("this is wrong, add the index"), "{prompt}");
        assert!(
            prompt.contains("acme/web#562")
                && prompt.contains("https://github.com/acme/web/pull/562"),
            "the prompt names the pull request: {prompt}"
        );
        let launches = e2e.background_launches();
        assert_eq!(launches.len(), 1);
        let argv: Vec<&str> = launches[0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let tools = argv.iter().position(|a| *a == "--tools").expect("{argv:?}");
        assert_eq!(argv[tools + 1], "", "no tools: {argv:?}");
        assert!(argv.contains(&"--no-session-persistence"), "{argv:?}");
    }

    let body = git.update(cx, |git, cx| git.use_polish_for_test(cx));
    assert_eq!(body, text, "Use puts the rewrite in the review comment");
    assert!(git.read_with(cx, |git, _| git.polish_outcome_for_test().is_none()));
}

/// An agent that answers without the tags leaves the draft alone and says so.
#[gpui::test]
fn a_reply_without_a_rewrite_keeps_the_draft_and_says_so(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let git = open_git(
        &e2e,
        serde_json::json!(["I would rather not rewrite that."]),
        cx,
    );
    git.update(cx, |git, cx| {
        git.polish_for_test("nit: rename this", Tone::Direct, cx)
    });
    e2e.until(cx, "Polish to finish", |_, cx| {
        git.read_with(cx, |git, _| git.polish_outcome_for_test().is_some())
    });
    assert_eq!(
        git.read_with(cx, |git, _| git.polish_outcome_for_test())
            .unwrap(),
        Err("The agent did not return a rewrite. Try again.".into())
    );
    let body = git.update(cx, |git, cx| git.use_polish_for_test(cx));
    assert_eq!(body, "nit: rename this", "the draft stays as written");
}

/// Polish from a comment box on a line runs the same way, and its rewrite
/// goes back to that box, not to the review bar.
#[gpui::test]
fn a_line_comment_is_polished_in_the_background_into_its_own_box(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let rewrite = "Could we add an index on key here?";
    let git = open_git(
        &e2e,
        serde_json::json!([format!("<polished>{rewrite}</polished>")]),
        cx,
    );
    git.update(cx, |git, cx| {
        git.polish_line_for_test("add an index", Tone::Suggestion, cx)
    });
    e2e.until(cx, "the line rewrite", |_, cx| {
        git.read_with(cx, |git, _| git.line_polish_outcome_for_test().is_some())
    });
    let (tone, text) = git
        .read_with(cx, |git, _| git.line_polish_outcome_for_test())
        .unwrap()
        .unwrap_or_else(|error| panic!("no rewrite: {error}"));
    assert_eq!(tone, Tone::Suggestion);
    match e2e.mode() {
        Mode::Fake => assert_eq!(text, rewrite),
        Mode::Real => assert!(!text.trim().is_empty()),
    }
    assert!(
        git.read_with(cx, |git, _| git.polish_outcome_for_test().is_none()),
        "the review bar's own Polish is untouched"
    );
    assert!(!git.read_with(cx, |git, _| git.ask_open_for_test()));
    if e2e.mode() == Mode::Fake {
        let prompts = e2e.background_prompts();
        assert!(prompts[0].contains("Suggestion:"), "{prompts:?}");
        assert!(prompts[0].contains("add an index"), "{prompts:?}");
    }
}

/// A rewrite still on its way when you open another pull request is dropped:
/// it never lands in the next pull request's comment.
#[gpui::test]
fn a_rewrite_for_a_pull_request_you_left_is_dropped(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    e2e.script_background(
        serde_json::json!([{"text": "<polished>late</polished>", "wait": true}]),
        cx,
    );
    let first = pull_request(e2e.repo());
    let mut second = first.clone();
    second.number = 563;
    second.url = "https://github.com/acme/web/pull/563".into();
    let git = e2e
        .window()
        .update(cx, |shell, window, cx| {
            shell.e2e_open_git(vec![first, second.clone()], window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    git.update(cx, |git, cx| {
        git.polish_for_test("first one", Tone::Direct, cx)
    });
    e2e.until(cx, "the background turn to start", |_, _| {
        !e2e.background_prompts().is_empty()
    });

    git.update(cx, |git, cx| git.select_for_test(second.key(), cx));
    e2e.release_background();
    // Give the late answer every chance to arrive.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(git.read_with(cx, |git, _| git.polish_outcome_for_test().is_none()));
    assert!(!git.read_with(cx, |git, _| git.polishing_for_test()));
}
