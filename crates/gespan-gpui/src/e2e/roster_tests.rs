//! Sessions learn about their siblings at the start of a turn: the other
//! sessions working in the same repository, in its main checkout or any of
//! its worktrees. Driven end to end, from the composer to the agent process.
use super::*;
use gpui::TestAppContext;
use serde_json::json;

const HEADER: &str = "Other sessions in this repository";
const B_TASK: &str = "Fix the sidebar flicker";

/// An Edit tool call on `path` and its result, as Claude streams them.
fn edit_frames(path: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": {
                "id": "fake-edit", "role": "assistant", "model": "fake",
                "content": [{
                    "type": "tool_use", "id": "toolu_edit", "name": "Edit",
                    "input": {"file_path": path, "old_string": "a", "new_string": "b"},
                }],
                "usage": {"input_tokens": 1, "output_tokens": 1},
            },
        }),
        json!({
            "type": "user", "parent_tool_use_id": null,
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_edit", "content": "Edited"},
            ]},
        }),
    ]
}

/// A repository holding `src/a.rs`, with session B in a new worktree of it
/// that was asked to fix the sidebar and edited `src/a.rs` there.
fn with_b_in_a_worktree(e2e: &E2e, cx: &mut TestAppContext) -> i64 {
    std::fs::create_dir_all(e2e.repo().join("src")).unwrap();
    std::fs::write(e2e.repo().join("src/a.rs"), "a\n").unwrap();
    git(e2e.repo(), &["add", "."]);
    git(e2e.repo(), &["commit", "-qm", "Add a.rs"]);
    let b = e2e.start_session(Kind::Claude, cx);
    e2e.session(b, cx)
        .update(cx, |view, cx| view.menu_choice("workspace:new", cx));
    cx.run_until_parked();
    e2e.script_frames(
        b,
        json!({"text": "Fixed.", "frames": edit_frames("src/a.rs")}),
    );
    e2e.send(b, &format!("{B_TASK}\nOnly reply OK; change nothing."), cx);
    // What the edit left behind in B's worktree. B carries on, and what
    // its turns changed is learned when its next turn ends.
    let worktree = b_worktree(e2e, b, cx);
    std::fs::write(Path::new(&worktree).join("src/a.rs"), "b\n").unwrap();
    e2e.send(b, "Only reply OK.", cx);
    b
}

fn b_worktree(e2e: &E2e, b: i64, cx: &mut TestAppContext) -> String {
    e2e.session(b, cx)
        .read_with(cx, |view, _| view.settings.worktree_cwd.clone())
        .expect("B works in a worktree of its own")
}

/// B's goal, branch and file reach A's agent on A's next turn, while A's chat
/// shows only what was typed.
#[gpui::test]
fn a_sibling_in_a_worktree_reaches_the_next_prompt(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let b = with_b_in_a_worktree(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_reply(a, "B is fixing the sidebar flicker.");
    let question = "In one short sentence: what is the other session in this \
                    repository working on? Answer only from what you were told; use no tools.";

    let reply = e2e.send(a, question, cx);

    let prompt = &e2e.prompts(a)[0];
    assert!(prompt.starts_with(HEADER), "{prompt}");
    assert!(prompt.contains(B_TASK), "{prompt}");
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(b_worktree(&e2e, b, cx))
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    assert!(!branch.is_empty());
    assert!(
        prompt.contains(&format!("in worktree {branch}")),
        "{prompt}"
    );
    assert!(
        prompt.ends_with(question),
        "the typed text follows: {prompt}"
    );
    assert_eq!(e2e.transcript(a, cx)[0].text, question);
    match e2e.mode() {
        Mode::Fake => assert!(prompt.contains("Changing: src/a.rs"), "{prompt}"),
        // A real model must have read the roster to know this.
        Mode::Real => assert!(reply.to_lowercase().contains("sidebar"), "{reply}"),
    }
}

/// When A changes the same file B holds, A is warned.
#[gpui::test]
fn the_same_file_in_two_checkouts_is_flagged(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    with_b_in_a_worktree(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        json!({"text": "Edited.", "frames": edit_frames("src/a.rs")}),
    );
    // The edit's result on disk, in place when A's turn ends.
    std::fs::write(e2e.repo().join("src/a.rs"), "tidy\n").unwrap();
    e2e.send(a, "Tidy a.rs", cx);

    e2e.send(a, "Carry on", cx);

    let prompt = e2e.prompts(a).pop().unwrap();
    assert!(
        prompt
            .lines()
            .any(|line| line.starts_with('⚠') && line.contains("src/a.rs")),
        "{prompt}"
    );
}

#[gpui::test]
fn an_unchanged_roster_is_not_sent_again(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    with_b_in_a_worktree(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);

    e2e.send(a, "one", cx);
    e2e.send(a, "two", cx);

    let prompts = e2e.prompts(a);
    assert!(prompts[0].starts_with(HEADER), "{prompts:?}");
    assert_eq!(prompts[1], "two", "nothing changed, so only the message");
}

#[gpui::test]
fn a_new_sibling_sends_the_roster_again(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    with_b_in_a_worktree(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.send(a, "one", cx);
    let c = e2e.start_session(Kind::Claude, cx);
    e2e.send(c, "Write the release notes", cx);

    e2e.send(a, "two", cx);

    let prompt = e2e.prompts(a).pop().unwrap();
    assert!(prompt.starts_with(HEADER), "{prompt}");
    assert!(prompt.contains("Write the release notes"), "{prompt}");
    assert!(prompt.ends_with("two"), "{prompt}");
}

#[gpui::test]
fn a_session_alone_gets_no_roster(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);

    e2e.send(a, "hi", cx);
    e2e.send(a, "again", cx);

    assert_eq!(e2e.prompts(a), ["hi", "again"]);
}
