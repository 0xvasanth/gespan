//! Polish on the Pull requests page: a review comment goes to the pull request's Ask
//! agent with the chosen tone, and its rewrite comes back as an offer to
//! Use, Try again or Keep mine. Fake by default; `GESPAN_E2E_REAL=1` asks the
//! real `claude` and checks only that a rewrite came back.
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

#[gpui::test]
fn polish_rewrites_a_review_comment_through_the_ask_agent(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let rewrite = "Good catch, thanks. Could you add an index on key in this migration?";
    // The Ask conversation is a session the app starts itself.
    e2e.script_next_child(serde_json::json!([format!(
        "Here it is:\n<polished>\n{rewrite}\n</polished>"
    )]));
    let item = pull_request(e2e.repo());
    let git = e2e
        .window()
        .update(cx, |shell, window, cx| {
            // The fake agent stands in for Claude.
            shell.e2e_project_provider(Kind::Claude);
            shell.e2e_open_git(vec![item], window, cx)
        })
        .unwrap();
    cx.run_until_parked();

    git.update(cx, |git, cx| {
        git.polish_for_test("this is wrong, add the index", Tone::Friendly, cx)
    });
    e2e.until(cx, "the polished comment", |_, cx| {
        git.read_with(cx, |git, _| git.polish_outcome_for_test().is_some())
    });

    let outcome = git
        .read_with(cx, |git, _| git.polish_outcome_for_test())
        .unwrap();
    let (tone, text) = outcome.unwrap_or_else(|error| panic!("no rewrite: {error}"));
    assert_eq!(tone, Tone::Friendly);
    let session = git.read_with(cx, |git, cx| {
        git.ask_conversation()
            .expect("Polish opened the Ask panel on the pull request")
            .read(cx)
            .summary
            .id
    });
    let prompts = e2e.prompts(session);
    assert_eq!(prompts.len(), 1, "one Polish, one turn: {prompts:?}");
    let prompt = &prompts[0];
    assert!(prompt.contains("Rewrite my review comment"), "{prompt}");
    assert!(
        prompt.contains("Friendly: warm"),
        "the tone travels: {prompt}"
    );
    assert!(prompt.contains("this is wrong, add the index"), "{prompt}");
    assert!(
        prompt.contains("https://github.com/acme/web/pull/562"),
        "the Ask agent knows which pull request: {prompt}"
    );
    match e2e.mode() {
        Mode::Fake => assert_eq!(text, rewrite),
        Mode::Real => assert!(!text.trim().is_empty()),
    }

    let body = git.update(cx, |git, cx| git.use_polish_for_test(cx));
    assert_eq!(body, text, "Use puts the rewrite in the review comment");
    assert!(git.read_with(cx, |git, _| git.polish_outcome_for_test().is_none()));
}

/// An agent that answers without the tags leaves the draft alone and says so.
#[gpui::test]
fn a_reply_without_a_rewrite_keeps_the_draft_and_says_so(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    e2e.script_next_child(serde_json::json!(["I would rather not rewrite that."]));
    let item = pull_request(e2e.repo());
    let git = e2e
        .window()
        .update(cx, |shell, window, cx| {
            // The fake agent stands in for Claude.
            shell.e2e_project_provider(Kind::Claude);
            shell.e2e_open_git(vec![item], window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    git.update(cx, |git, cx| {
        git.polish_for_test("nit: rename this", Tone::Direct, cx)
    });
    e2e.until(cx, "Polish to finish", |_, cx| {
        git.read_with(cx, |git, _| git.polish_outcome_for_test().is_some())
    });
    let outcome = git
        .read_with(cx, |git, _| git.polish_outcome_for_test())
        .unwrap();
    assert_eq!(
        outcome,
        Err("The agent did not return a rewrite. Try again.".into())
    );
    let body = git.update(cx, |git, cx| git.use_polish_for_test(cx));
    assert_eq!(body, "nit: rename this", "the draft stays as written");
}
