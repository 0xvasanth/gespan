//! Replying on the Overview, as the "Comment replies" canvas draws it: the
//! box opens under the comment, threads take their reply and the rest get a
//! quote reply, several boxes stay open with their own words, a failed reply
//! keeps Retry and Edit on itself, and long threads, resolved threads and
//! runs of bots fold. No host is reached: what the host answers is the
//! test's.
use super::*;
use gespan_core::inbox::{Status, polish::Tone};
use gpui::{
    Bounds, Modifiers, TestAppContext, VisualTestContext, WindowBounds, WindowHandle,
    WindowOptions, point, px, size,
};
use std::path::PathBuf;

fn pull_request() -> Item {
    Item {
        provider: "github".into(),
        kind: "pr".into(),
        repo: "acme/web".into(),
        number: 7,
        state: "open".into(),
        title: "Preview environments".into(),
        author: "dana".into(),
        viewer: "me".into(),
        url: "https://github.com/acme/web/pull/7".into(),
        ..Default::default()
    }
}

fn comment(id: &str, author: &str, body: &str, minute: u32) -> model::Comment {
    model::Comment {
        id: id.into(),
        kind: "comment".into(),
        author: author.into(),
        body: body.into(),
        created_at: format!("2026-11-20T10:{minute:02}:00Z"),
        ..Default::default()
    }
}

/// marko's review thread on src/a.rs with dana's reply.
fn review_thread() -> model::Comment {
    model::Comment {
        kind: "review_comment".into(),
        path: "src/a.rs".into(),
        line: Some(4),
        thread_id: "PRRT_1".into(),
        resolvable: true,
        replies: vec![comment("r1-a", "dana", "Done", 6)],
        ..comment("r1", "marko", "Index this?", 5)
    }
}

/// lee's general comment: GitHub doesn't thread it.
fn general() -> model::Comment {
    comment(
        "c1",
        "lee",
        "Do previews get torn down?\nOtherwise Argo piles up.",
        1,
    )
}

/// The Pull requests page open on one pull request whose conversation is
/// `comments`; the host answers replies and resolves with `answer`.
fn open(
    cx: &mut TestAppContext,
    comments: Vec<model::Comment>,
    answer: Result<String, String>,
) -> (WindowHandle<Inbox>, VisualTestContext) {
    cx.update(|cx| cx.set_global(crate::settings::Preferences::default()));
    let item = pull_request();
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1300.), px(1400.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let store = std::sync::Arc::new(std::sync::Mutex::new(
                        gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap(),
                    ));
                    let mut inbox =
                        Inbox::new_inner(store, vec![], "/missing/git".into(), false, cx);
                    inbox.offline = Offline::Instant;
                    inbox.snapshot.connections.insert(
                        "github".into(),
                        Status {
                            connected: true,
                            ..Default::default()
                        },
                    );
                    inbox.snapshot.items = vec![item.clone()];
                    inbox.fetched = inbox.snapshot.clone();
                    inbox.canned_reply_result = Some(answer);
                    inbox
                })
            },
        )
        .unwrap()
    });
    cx.run_until_parked();
    window
        .update(cx, |inbox, _, cx| {
            inbox.select_for_test(item.key(), cx);
            let mut docs = Vec::new();
            for comment in &comments {
                for c in std::iter::once(comment).chain(&comment.replies) {
                    let body = c.body.clone();
                    docs.push((
                        c.clone(),
                        cx.new(|cx| document::InboxDocument::new(body, PathBuf::from("/repo"), cx)),
                    ));
                }
            }
            inbox.comments = docs;
            inbox.thread = Some(Thread {
                comments,
                ..Default::default()
            });
            inbox.rebuild_activity();
            cx.notify();
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    settle(&mut visual);
    (window, visual)
}

fn settle(visual: &mut VisualTestContext) {
    for _ in 0..3 {
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn shown(visual: &mut VisualTestContext, selector: &str) -> bool {
    visual.debug_bounds(leak(selector.to_string())).is_some()
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    let bounds = visual
        .debug_bounds(leak(selector.to_string()))
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), Modifiers::default());
    settle(visual);
}

fn type_reply(window: WindowHandle<Inbox>, visual: &mut VisualTestContext, root: &str, text: &str) {
    let text = text.to_owned();
    let root = root.to_owned();
    window
        .update(visual, move |inbox, _, cx| {
            let input = inbox.replies.boxes[&root].input.clone();
            input.update(cx, |input, cx| input.set_text(text, cx));
        })
        .unwrap();
    settle(visual);
}

fn posted(window: WindowHandle<Inbox>, visual: &mut VisualTestContext) -> Vec<(String, String)> {
    window
        .update(visual, |inbox, _, _| inbox.posted_replies.borrow().clone())
        .unwrap()
}

fn box_text(
    window: WindowHandle<Inbox>,
    visual: &mut VisualTestContext,
    root: &str,
) -> Option<String> {
    let root = root.to_owned();
    window
        .update(visual, move |inbox, _, cx| {
            inbox
                .replies
                .boxes
                .get(&root)
                .map(|b| b.input.read(cx).content.to_string())
        })
        .unwrap()
}

#[gpui::test]
fn reply_opens_under_the_thread_and_goes_into_it(cx: &mut TestAppContext) {
    let (window, mut visual) = open(cx, vec![general(), review_thread()], Ok("url".into()));
    assert!(shown(&mut visual, "inbox-thread-footer-r1"));
    assert!(!shown(&mut visual, "inbox-reply-box-r1"));

    click(&mut visual, "inbox-reply-r1");
    assert!(
        shown(&mut visual, "inbox-reply-box-r1"),
        "the box opens under the thread"
    );
    let footer = visual.debug_bounds("inbox-thread-footer-r1").unwrap();
    let reply_box = visual.debug_bounds("inbox-reply-box-r1").unwrap();
    assert!(
        footer.contains(&reply_box.origin),
        "inside the thread's card, not at the bottom"
    );
    let focused = window
        .update(&mut visual, |inbox, window, cx| {
            inbox.replies.boxes["r1"]
                .input
                .focus_handle(cx)
                .is_focused(window)
        })
        .unwrap();
    assert!(focused, "the reply box has the keyboard");

    type_reply(window, &mut visual, "r1", "Pushed the fix");
    click(&mut visual, "inbox-reply-send-r1");
    assert_eq!(
        posted(window, &mut visual),
        vec![("Pushed the fix".to_string(), "PRRT_1".to_string())]
    );
    assert!(
        !shown(&mut visual, "inbox-reply-box-r1"),
        "the box closes once sent"
    );
    assert!(
        shown(&mut visual, "inbox-sent-reply-r1-0"),
        "the reply shows under its thread"
    );
    let composer = window
        .update(&mut visual, |inbox, _, cx| {
            inbox.comment.read(cx).content.to_string()
        })
        .unwrap();
    assert!(
        composer.is_empty(),
        "the bottom composer is for new comments only"
    );
}

#[gpui::test]
fn a_comment_github_cant_thread_gets_a_quote_reply(cx: &mut TestAppContext) {
    let (window, mut visual) = open(cx, vec![general()], Ok("url".into()));
    click(&mut visual, "inbox-reply-c1");
    let target = window
        .update(&mut visual, |inbox, _, _| {
            inbox.replies.boxes["c1"].target.clone()
        })
        .unwrap();
    assert!(matches!(target, replies::ReplyTarget::Quote { .. }));
    // Nothing goes until there are words.
    click(&mut visual, "inbox-reply-send-c1");
    assert!(posted(window, &mut visual).is_empty());

    type_reply(window, &mut visual, "c1", "yes, on close");
    click(&mut visual, "inbox-reply-send-c1");
    assert_eq!(
        posted(window, &mut visual),
        vec![(
            model::quote_reply("lee", &general().body, "yes, on close"),
            String::new(),
        )],
        "a new comment that quotes lee and mentions them"
    );
}

#[gpui::test]
fn several_boxes_keep_their_words_and_a_failed_reply_keeps_retry_and_edit(cx: &mut TestAppContext) {
    let (window, mut visual) = open(
        cx,
        vec![general(), review_thread()],
        Err("GitHub is rate limiting requests".into()),
    );
    click(&mut visual, "inbox-reply-c1");
    type_reply(window, &mut visual, "c1", "first box");
    click(&mut visual, "inbox-reply-r1");
    type_reply(window, &mut visual, "r1", "second box");
    assert_eq!(
        box_text(window, &mut visual, "c1").as_deref(),
        Some("first box")
    );
    assert_eq!(
        box_text(window, &mut visual, "r1").as_deref(),
        Some("second box")
    );

    click(&mut visual, "inbox-reply-send-r1");
    assert!(
        shown(&mut visual, "inbox-sent-retry-r1-0"),
        "the error stays on the reply"
    );
    assert_eq!(
        box_text(window, &mut visual, "c1").as_deref(),
        Some("first box"),
        "the other box is untouched"
    );

    window
        .update(&mut visual, |inbox, _, _| {
            inbox.canned_reply_result = Some(Ok("url".into()))
        })
        .unwrap();
    click(&mut visual, "inbox-sent-retry-r1-0");
    assert_eq!(posted(window, &mut visual).len(), 2, "Retry sends it again");
    assert!(!shown(&mut visual, "inbox-sent-retry-r1-0"));

    // A failure, then Edit: the words go back in the box.
    window
        .update(&mut visual, |inbox, _, _| {
            inbox.canned_reply_result = Some(Err("Could not reach GitHub".into()))
        })
        .unwrap();
    click(&mut visual, "inbox-reply-send-c1");
    assert!(shown(&mut visual, "inbox-sent-edit-c1-0"));
    click(&mut visual, "inbox-sent-edit-c1-0");
    assert_eq!(
        box_text(window, &mut visual, "c1").as_deref(),
        Some("first box")
    );
    assert!(!shown(&mut visual, "inbox-sent-reply-c1-0"));
}

#[gpui::test]
fn cancel_closes_the_box_without_sending(cx: &mut TestAppContext) {
    let (window, mut visual) = open(cx, vec![review_thread()], Ok("url".into()));
    click(&mut visual, "inbox-reply-r1");
    type_reply(window, &mut visual, "r1", "never mind");
    click(&mut visual, "inbox-reply-cancel-r1");
    assert!(!shown(&mut visual, "inbox-reply-box-r1"));
    assert!(posted(window, &mut visual).is_empty());
}

#[gpui::test]
fn a_long_thread_folds_its_earlier_replies(cx: &mut TestAppContext) {
    let mut long = review_thread();
    long.replies = (0..5)
        .map(|ix| comment(&format!("r1-{ix}"), "dana", &format!("reply {ix}"), 10 + ix))
        .collect();
    let (window, mut visual) = open(cx, vec![long], Ok("url".into()));
    assert!(
        shown(&mut visual, "inbox-earlier-replies-r1"),
        "two of five fold"
    );
    click(&mut visual, "inbox-earlier-replies-r1");
    assert!(!shown(&mut visual, "inbox-earlier-replies-r1"));
    assert!(
        window
            .update(&mut visual, |inbox, _, _| inbox.replies.open.contains("r1"))
            .unwrap()
    );
    // Three or fewer replies never fold.
    let (_, mut short) = open(cx, vec![review_thread()], Ok("url".into()));
    assert!(!shown(&mut short, "inbox-earlier-replies-r1"));
}

#[gpui::test]
fn a_resolved_thread_folds_to_a_line_and_unresolve_goes_back_if_refused(cx: &mut TestAppContext) {
    let resolved = model::Comment {
        resolved: true,
        ..review_thread()
    };
    let (window, mut visual) = open(cx, vec![resolved], Err("Not allowed".into()));
    assert!(shown(&mut visual, "inbox-resolved-r1"));
    assert!(
        !shown(&mut visual, "inbox-thread-footer-r1"),
        "folded to one line"
    );

    click(&mut visual, "inbox-resolved-show-r1");
    assert!(
        shown(&mut visual, "inbox-thread-footer-r1"),
        "Show opens it whole"
    );
    assert!(shown(&mut visual, "inbox-resolve-r1"));

    let is_resolved = |window: WindowHandle<Inbox>, visual: &mut VisualTestContext| {
        window
            .update(visual, |inbox, _, _| {
                inbox.thread.as_ref().unwrap().comments[0].resolved
            })
            .unwrap()
    };
    click(&mut visual, "inbox-resolve-r1");
    assert!(
        is_resolved(window, &mut visual),
        "the host refused: it stays resolved"
    );
    let error = window
        .update(&mut visual, |inbox, _, _| {
            inbox.replies.errors.get("r1").cloned()
        })
        .unwrap();
    assert_eq!(error.as_deref(), Some("Not allowed"));

    window
        .update(&mut visual, |inbox, _, _| {
            inbox.canned_reply_result = Some(Ok(String::new()))
        })
        .unwrap();
    click(&mut visual, "inbox-resolved-reopen-r1");
    assert!(!is_resolved(window, &mut visual));
    assert!(
        shown(&mut visual, "inbox-thread-footer-r1"),
        "an open thread shows whole"
    );

    // Resolving folds it again.
    click(&mut visual, "inbox-resolve-r1");
    assert!(is_resolved(window, &mut visual));
    assert!(shown(&mut visual, "inbox-resolved-r1"));
}

#[gpui::test]
fn a_run_of_bot_comments_folds_to_one_line(cx: &mut TestAppContext) {
    let bot = |id: &str, author: &str, minute| model::Comment {
        author_bot: true,
        ..comment(id, author, "Preview image pushed", minute)
    };
    let (_, mut visual) = open(
        cx,
        vec![
            bot("b1", "github-actions", 1),
            bot("b2", "claude", 2),
            // A known bot login counts without the host's flag.
            comment("b3", "dependabot[bot]", "Bumped", 3),
            model::Comment {
                created_at: "2026-11-20T10:05:00Z".into(),
                ..general()
            },
            bot("b4", "github-actions", 9),
        ],
        Ok("url".into()),
    );
    assert!(
        shown(&mut visual, "inbox-bots-b1"),
        "three in a row fold together"
    );
    assert!(!shown(&mut visual, "inbox-thread-footer-b2"));
    assert!(
        shown(&mut visual, "inbox-thread-footer-c1"),
        "a person's comment stays"
    );
    assert!(
        shown(&mut visual, "inbox-bots-b4"),
        "a later bot run folds on its own"
    );

    click(&mut visual, "inbox-bots-show-b1");
    for id in ["b1", "b2", "b3"] {
        assert!(
            shown(&mut visual, &format!("inbox-thread-footer-{id}")),
            "{id} opens"
        );
    }
}

#[gpui::test]
fn polish_in_a_reply_box_offers_its_rewrite_to_that_box(cx: &mut TestAppContext) {
    let (window, mut visual) = open(cx, vec![general(), review_thread()], Ok("url".into()));
    click(&mut visual, "inbox-reply-r1");
    click(&mut visual, "inbox-reply-c1");
    type_reply(window, &mut visual, "r1", "this is wrong");
    click(&mut visual, "inbox-reply-tone-friendly-r1");
    let id = window
        .update(&mut visual, |inbox, _, _| {
            let open = &inbox.replies.boxes["r1"];
            assert_eq!(open.tone, Tone::Friendly);
            open.id
        })
        .unwrap();
    window
        .update(&mut visual, move |inbox, _, cx| {
            inbox.reply_polished(
                id,
                Ok((Tone::Friendly, "Thanks! Could you fix this?".into())),
                cx,
            )
        })
        .unwrap();
    settle(&mut visual);
    assert!(shown(&mut visual, "inbox-reply-polished-r1"));
    assert!(
        !shown(&mut visual, "inbox-reply-polished-c1"),
        "only the box that asked"
    );
    click(&mut visual, "inbox-reply-use-r1");
    assert_eq!(
        box_text(window, &mut visual, "r1").as_deref(),
        Some("Thanks! Could you fix this?")
    );
    assert!(!shown(&mut visual, "inbox-reply-polished-r1"));
}

/// Each host's comments go where that host threads them.
#[test]
fn replies_go_where_each_host_threads_them() {
    let item = |provider: &str, kind: &str| Item {
        provider: provider.into(),
        kind: kind.into(),
        ..Default::default()
    };
    let threaded = model::Comment {
        thread_id: "T1".into(),
        ..comment("x", "a", "body", 1)
    };
    let loose = comment("y", "a", "body", 1);
    use replies::{ReplyTarget, target};
    for provider in ["github", "gitlab", "azuredevops"] {
        assert_eq!(
            target(&item(provider, "pr"), &threaded),
            ReplyTarget::Thread("T1".into()),
            "{provider}"
        );
        assert!(matches!(
            target(&item(provider, "pr"), &loose),
            ReplyTarget::Quote { .. }
        ));
    }
    assert!(
        matches!(
            target(&item("azuredevops", "issue"), &threaded),
            ReplyTarget::Quote { .. }
        ),
        "a work item's comments aren't threads"
    );
    assert_eq!(
        target(&item("linear", "issue"), &loose),
        ReplyTarget::Thread("y".into()),
        "Linear replies to the comment itself"
    );
    assert!(matches!(
        target(&item("jira", "issue"), &threaded),
        ReplyTarget::Quote { .. }
    ));
}
