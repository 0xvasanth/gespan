//! Review threads in Files, as the "Review threads in Files" canvas draws
//! them.
use super::super::review_threads::{PendingReply, RemoteComment, RemoteThread};
use super::super::*;
use super::{Segment, split_details};
use gespan_core::inbox::Side;
use std::{cell::RefCell, rc::Rc};

#[test]
fn details_blocks_fold_to_their_summary_and_keep_the_text_around_them() {
    let body = "Intro **bold**.\n\n<details>\n<summary>Why <b>this</b> was flagged</summary>\n\nBecause.\n</details>\n\nAfter.";
    assert_eq!(
        split_details(body),
        vec![
            Segment::Text("Intro **bold**.".into()),
            Segment::Details {
                summary: "Why this was flagged".into(),
                body: "Because.".into()
            },
            Segment::Text("After.".into()),
        ]
    );
    assert_eq!(
        split_details("<DETAILS open>no summary"),
        vec![Segment::Details {
            summary: "Details".into(),
            body: "no summary".into()
        }],
        "an unclosed block runs to the end; no summary reads Details"
    );
    assert_eq!(split_details("plain"), vec![Segment::Text("plain".into())]);
    assert_eq!(split_details("  "), vec![]);
}

fn comment(author: &str, body: &str) -> RemoteComment {
    RemoteComment {
        author: author.into(),
        when: "1h ago".into(),
        body: body.into(),
        ..Default::default()
    }
}

fn thread(root: &str, line: u32, resolved: bool, comments: Vec<RemoteComment>) -> RemoteThread {
    RemoteThread {
        root: root.into(),
        path: "CHANGELOG.md".into(),
        line,
        side: Side::New,
        resolved,
        resolvable: true,
        outdated: false,
        comments,
        pending: vec![],
    }
}

/// The review window with `threads`, drawn, and the events it raised.
fn with_threads(
    cx: &mut gpui::TestAppContext,
    threads: Vec<RemoteThread>,
) -> (
    gpui::WindowHandle<DiffView>,
    gpui::VisualTestContext,
    Rc<RefCell<Vec<String>>>,
    gpui::Subscription,
) {
    let (window, events, subscription) = super::super::tests::review_window_sized(cx, 1100.);
    window
        .update(cx, |view, _, cx| view.set_threads(threads, cx))
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    settle(&mut visual);
    (window, visual, events, subscription)
}

fn settle(visual: &mut gpui::VisualTestContext) {
    for _ in 0..3 {
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}
fn shown(visual: &mut gpui::VisualTestContext, selector: &str) -> bool {
    visual.debug_bounds(leak(selector.into())).is_some()
}
fn click(visual: &mut gpui::VisualTestContext, selector: &str) {
    let bounds = visual
        .debug_bounds(leak(selector.into()))
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), Default::default());
    settle(visual);
}

#[gpui::test]
fn a_thread_shows_markdown_folds_details_and_clamps_a_long_comment(cx: &mut gpui::TestAppContext) {
    let long = format!(
        "{}\n\n<details>\n<summary>Why this was flagged</summary>\n\nThe diff adds a file-wide disable.\n</details>",
        "Convention: eight files get file-wide lint disables. ".repeat(12)
    );
    let (window, mut visual, _, _s) = with_threads(
        cx,
        vec![thread(
            "t1",
            2,
            false,
            vec![
                comment("claude", &long),
                comment("cursor", "Fixed in 6 files"),
            ],
        )],
    );
    assert!(shown(&mut visual, "diff-remote-comment-t1:0"));
    assert!(
        shown(&mut visual, "diff-remote-comment-t1:1"),
        "the reply on the rail"
    );
    assert!(
        shown(&mut visual, "diff-details-t1:0:1"),
        "the <details> folds to its summary"
    );
    assert!(
        shown(&mut visual, "diff-comment-more-t1:0"),
        "a long comment clamps"
    );
    assert!(
        shown(&mut visual, "diff-open-threads-0"),
        "the file counts its open threads"
    );

    let clamped = visual
        .debug_bounds("diff-remote-comment-t1:0")
        .unwrap()
        .size
        .height;
    click(&mut visual, "diff-comment-more-t1:0");
    let whole = visual
        .debug_bounds("diff-remote-comment-t1:0")
        .unwrap()
        .size
        .height;
    assert!(
        whole > clamped,
        "Show more shows it whole: {clamped:?} → {whole:?}"
    );
    // Below the clamp, the summary opens its block.
    click(&mut visual, "diff-details-t1:0:1");
    assert!(
        window
            .update(&mut visual, |v, _, _| v.review.details.contains("t1:0:1"))
            .unwrap()
    );
    let opened = visual
        .debug_bounds("diff-remote-comment-t1:0")
        .unwrap()
        .size
        .height;
    assert!(
        opened > whole,
        "the block's text shows: {whole:?} → {opened:?}"
    );
}

#[gpui::test]
fn reply_opens_inside_the_thread_and_posts_now_not_as_a_draft(cx: &mut gpui::TestAppContext) {
    let (window, mut visual, events, _s) = with_threads(
        cx,
        vec![thread(
            "t1",
            2,
            false,
            vec![comment("marko", "Index this?")],
        )],
    );
    click(&mut visual, "diff-thread-reply-t1");
    let card = visual.debug_bounds("diff-remote-thread-0").unwrap();
    let editor = visual.debug_bounds("diff-review-editor").unwrap();
    assert!(
        card.contains(&editor.origin),
        "the box is inside the thread's card"
    );
    window
        .update(&mut visual, |view, _, cx| {
            let editor = view.review.editor.as_ref().unwrap();
            assert_eq!(editor.reply.as_deref(), Some("t1"));
            let input = editor.input.clone();
            input.update(cx, |input, cx| input.set_text("Done".into(), cx));
        })
        .unwrap();
    settle(&mut visual);
    click(&mut visual, "diff-review-save");
    assert_eq!(*events.borrow(), vec!["reply t1 Done".to_string()]);
    let (drafts, open) = window
        .update(&mut visual, |v, _, _| {
            (v.review.drafts.len(), v.review.editor.is_some())
        })
        .unwrap();
    assert_eq!((drafts, open), (0, false), "a reply isn't a review draft");
}

#[gpui::test]
fn resolved_threads_start_folded_and_resolve_folds_by_hand(cx: &mut gpui::TestAppContext) {
    let (window, mut visual, events, _s) = with_threads(
        cx,
        vec![
            thread("t1", 2, true, vec![comment("lee", "Guard the key")]),
            thread("t2", 3, false, vec![comment("marko", "Cache this?")]),
        ],
    );
    assert!(
        shown(&mut visual, "diff-thread-open-t1"),
        "resolved: one line"
    );
    assert!(!shown(&mut visual, "diff-thread-reply-t1"));
    assert!(shown(&mut visual, "diff-thread-unresolve-t1"));
    click(&mut visual, "diff-thread-open-t1");
    assert!(
        shown(&mut visual, "diff-thread-reply-t1"),
        "Show opens it whole"
    );

    click(&mut visual, "diff-thread-resolve-t2");
    assert_eq!(*events.borrow(), vec!["resolve t2 true".to_string()]);
    click(&mut visual, "diff-thread-fold-t2");
    assert!(shown(&mut visual, "diff-thread-open-t2"), "folded by hand");
    assert!(
        window
            .update(&mut visual, |v, _, _| v.review.closed.contains("t2"))
            .unwrap()
    );
}

#[gpui::test]
fn an_outdated_thread_sits_at_the_top_of_its_file(cx: &mut gpui::TestAppContext) {
    let mut outdated = thread(
        "t9",
        28,
        false,
        vec![comment("sam", "Keep them consistent")],
    );
    outdated.outdated = true;
    let (window, mut visual, _, _s) = with_threads(cx, vec![outdated.clone()]);
    assert!(shown(&mut visual, "diff-remote-thread-0"));
    let (anchor, first) = window
        .update(&mut visual, |v, _, _| {
            (
                super::thread_anchor(&v.files[0].blocks, &outdated),
                v.files[0].blocks.iter().enumerate().find_map(|(b, block)| {
                    block
                        .lines()
                        .iter()
                        .position(|l| {
                            matches!(l.kind, LineKind::Add | LineKind::Del | LineKind::Context)
                        })
                        .map(|i| (b, i))
                }),
            )
        })
        .unwrap();
    assert_eq!(anchor, first, "at the file's first line");
    // A current thread on a line the diff doesn't show isn't drawn.
    let mut away = thread("t8", 400, false, vec![comment("sam", "x")]);
    away.outdated = false;
    let anchor = window
        .update(&mut visual, |v, _, _| {
            super::thread_anchor(&v.files[0].blocks, &away)
        })
        .unwrap();
    assert_eq!(anchor, None);
}

#[gpui::test]
fn a_failed_reply_keeps_retry_and_edit_in_its_thread(cx: &mut gpui::TestAppContext) {
    let mut failed = thread("t1", 2, false, vec![comment("marko", "Index this?")]);
    failed.pending = vec![PendingReply {
        text: "Pushed".into(),
        error: Some("Could not reach GitHub".into()),
        posted: false,
    }];
    let (window, mut visual, events, _s) = with_threads(cx, vec![failed]);
    assert!(shown(&mut visual, "diff-pending-t1-0"));
    click(&mut visual, "diff-pending-retry-t1-0");
    click(&mut visual, "diff-pending-edit-t1-0");
    assert_eq!(
        *events.borrow(),
        vec!["retry t1 0".to_string(), "drop t1 0".to_string()]
    );
    let text = window
        .update(&mut visual, |v, _, cx| {
            let editor = v.review.editor.as_ref().unwrap();
            assert_eq!(editor.reply.as_deref(), Some("t1"));
            editor.input.read(cx).content.to_string()
        })
        .unwrap();
    assert_eq!(
        text, "Pushed",
        "Edit puts the words back in the thread's box"
    );
}
