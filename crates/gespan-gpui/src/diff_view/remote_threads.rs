//! The pull request's own review threads in Files, as the "Review threads
//! in Files" canvas draws them: each sits under its line as a card with a
//! header (where, how many, open or resolved, fold), its comments with
//! avatars and Markdown (a `<details>` block folds to its summary, a long
//! comment clamps with Show more), replies on a rail, and a footer with
//! Reply and Resolve. A reply opens the same box as a review comment, Tone
//! and Polish included, inside the thread, and posts at once rather than
//! with the review. Resolved threads start folded; an outdated thread sits
//! at the top of its file and says where it was.
use super::review_threads::{At, PendingReply, RemoteComment, RemoteThread};
use super::*;
use crate::markdown::{self, MarkdownRow};
use crate::markdown_model;
use gpui::{AnyElement, Div, FontWeight, Role, SharedString};
use std::collections::{HashMap, HashSet};

/// A comment body to draw: Markdown, or a `<details>` block folded to its
/// summary.
#[derive(Clone, PartialEq)]
pub(crate) enum BodyPart {
    Rows(Vec<MarkdownRow>),
    Details {
        summary: String,
        rows: Vec<MarkdownRow>,
    },
}

/// A comment's text in pieces: Markdown, and each `<details>` block's
/// summary and inside, so the block folds instead of showing its tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    Text(String),
    Details { summary: String, body: String },
}

/// Splits `<details><summary>…</summary>…</details>` out of a comment. A
/// block left open runs to the end; one without a summary is "Details".
pub(crate) fn split_details(body: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(open) = lower.find("<details") else {
            break;
        };
        let before = &rest[..open];
        if !before.trim().is_empty() {
            out.push(Segment::Text(before.trim().to_owned()));
        }
        let after_open = match lower[open..].find('>') {
            Some(close) => open + close + 1,
            None => rest.len(),
        };
        let inner_end = lower[after_open..]
            .find("</details>")
            .map(|end| after_open + end);
        let inner = &rest[after_open..inner_end.unwrap_or(rest.len())];
        let inner_lower = inner.to_ascii_lowercase();
        let (summary, body) = match (
            inner_lower.find("<summary>"),
            inner_lower.find("</summary>"),
        ) {
            (Some(start), Some(end)) if start < end => (
                inner[start + "<summary>".len()..end].trim().to_owned(),
                inner[end + "</summary>".len()..].trim().to_owned(),
            ),
            _ => ("Details".to_owned(), inner.trim().to_owned()),
        };
        out.push(Segment::Details {
            summary: if summary.is_empty() {
                "Details".into()
            } else {
                strip_tags(&summary)
            },
            body,
        });
        rest = match inner_end {
            Some(end) => &rest[end + "</details>".len()..],
            None => "",
        };
    }
    if !rest.trim().is_empty() {
        out.push(Segment::Text(rest.trim().to_owned()));
    }
    out
}

/// Text without HTML tags, for a summary line.
fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for ch in text.chars() {
        match ch {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

fn rows(markdown: &str) -> Vec<MarkdownRow> {
    markdown_model::parse_full(markdown)
        .blocks
        .iter()
        .flat_map(|block| markdown::rows(&block.block))
        .collect()
}

/// Every comment's body, parsed once, by `root:comment`.
pub(super) fn parse_bodies(threads: &[RemoteThread]) -> HashMap<String, Vec<BodyPart>> {
    let mut out = HashMap::new();
    for thread in threads {
        for (cix, comment) in thread.comments.iter().enumerate() {
            let parts = split_details(&comment.body)
                .into_iter()
                .map(|segment| match segment {
                    Segment::Text(text) => BodyPart::Rows(rows(&text)),
                    Segment::Details { summary, body } => BodyPart::Details {
                        summary,
                        rows: rows(&body),
                    },
                })
                .collect();
            out.insert(format!("{}:{cix}", thread.root), parts);
        }
    }
    out
}

/// A comment past this many characters (outside `<details>`) clamps.
const LONG: usize = 480;
/// Links in a thread: Show more, as the design draws them.
const LINK: gpui::Rgba = gpui::Rgba {
    r: 0x8c as f32 / 255.,
    g: 0xc3 as f32 / 255.,
    b: 0xff as f32 / 255.,
    a: 1.,
};
/// The clamped height of a long comment.
const CLAMP: f32 = 168.;
const MD_LINE: f32 = 24.;
/// Pixels a character of comment text takes, kept generous so a guess runs
/// tall rather than clipping.
const CHAR: f32 = 7.4;

/// Where a remote thread sits: on its line, or, when its line changed
/// since it was written, at the first line of its file. A current thread on
/// a line the diff doesn't show isn't drawn.
pub(super) fn thread_anchor(blocks: &[Block], thread: &RemoteThread) -> Option<At> {
    if !thread.outdated {
        return super::review_threads::line_at(blocks, thread.side, thread.line);
    }
    blocks.iter().enumerate().find_map(|(b, block)| {
        block
            .lines()
            .iter()
            .position(|l| matches!(l.kind, LineKind::Add | LineKind::Del | LineKind::Context))
            .map(|i| (b, i))
    })
}

/// The text in a comment outside its `<details>` blocks.
fn plain_len(body: &str) -> usize {
    split_details(body)
        .iter()
        .map(|s| match s {
            Segment::Text(text) => text.chars().count(),
            Segment::Details { .. } => 0,
        })
        .sum()
}

impl DiffView {
    /// A thread is folded: resolved and not opened, or folded by hand.
    pub(super) fn thread_folded(&self, thread: &RemoteThread) -> bool {
        (thread.resolved && !self.review.opened.contains(&thread.root))
            || self.review.closed.contains(&thread.root)
    }
    fn text_chars(&self, inset: f32) -> f32 {
        let width = self.viewport.get().map_or(900., |b| b.size.width.as_f32());
        let gutter = self.gutter_width();
        let column = width.min(gutter + 900.) - gutter - 36. - 28. - inset;
        (column / CHAR).max(20.)
    }
    fn text_height(&self, text: &str, inset: f32) -> f32 {
        let chars = self.text_chars(inset);
        text.lines()
            .map(|l| ((l.chars().count() as f32) / chars).ceil().max(1.))
            .sum::<f32>()
            .max(1.)
            * MD_LINE
    }
    fn comment_height(&self, thread: &RemoteThread, cix: usize) -> f32 {
        let comment = &thread.comments[cix];
        let key = format!("{}:{cix}", thread.root);
        let inset = if cix == 0 { 0. } else { 28. };
        let mut body = 0.;
        for (pix, segment) in split_details(&comment.body).iter().enumerate() {
            body += match segment {
                Segment::Text(text) => self.text_height(text, inset) + 8.,
                Segment::Details { body, .. } => {
                    34. + if self.review.details.contains(&format!("{key}:{pix}")) {
                        self.text_height(body, inset + 12.) + 8.
                    } else {
                        0.
                    }
                }
            };
        }
        let long = plain_len(&comment.body) > LONG;
        if long && !self.review.more.contains(&key) {
            body = body.min(CLAMP) + 26.;
        } else if long {
            body += 26.;
        }
        28. + body + 8.
    }
    /// A thread card's height, as near as it can be told before it's drawn.
    pub(super) fn remote_height(&self, ix: usize) -> f32 {
        let thread = &self.review.threads[ix];
        if self.thread_folded(thread) {
            return 40.;
        }
        let comments: f32 = (0..thread.comments.len())
            .map(|cix| self.comment_height(thread, cix))
            .sum();
        let pending: f32 = thread
            .pending
            .iter()
            .map(|p| {
                24. + self.text_height(&p.text, 18.) + if p.error.is_some() { 34. } else { 0. }
            })
            .sum();
        let editor = self
            .review
            .editor
            .as_ref()
            .filter(|e| e.reply.as_deref() == Some(thread.root.as_str()))
            .map_or(0., |e| e.height(self.review.narrow) + 12.);
        38. + comments + pending + editor + 42. + 4.
    }

    /// The number of open remote threads on `path`.
    pub(super) fn open_threads(&self, path: &str) -> usize {
        self.review
            .threads
            .iter()
            .filter(|t| t.path == path && !t.resolved)
            .count()
    }
    /// A line's remote threads: how many, and whether all are resolved.
    pub(super) fn line_threads(&self, file: usize, at: At) -> Option<bool> {
        let threads = self.remote_at(file, at);
        (!threads.is_empty()).then(|| threads.iter().all(|ix| self.review.threads[*ix].resolved))
    }

    /// Opens the reply box inside the thread `root` starts, with `text`.
    pub(crate) fn open_reply(
        &mut self,
        root: &str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.review.threads.iter().find(|t| t.root == root) else {
            return;
        };
        let Some(file) = self.files.iter().position(|f| f.path == thread.path) else {
            return;
        };
        let Some(at) = thread_anchor(&self.files[file].blocks, thread) else {
            return;
        };
        let author = thread
            .comments
            .last()
            .map(|c| c.author.clone())
            .unwrap_or_default();
        // An open thread shows its box.
        self.review.closed.remove(root);
        if thread.resolved {
            self.review.opened.insert(root.to_owned());
        }
        self.open_editor(
            super::review_threads::Pick {
                file,
                anchor: at,
                head: at,
            },
            None,
            text,
            window,
            cx,
        );
        if let Some(editor) = self.review.editor.as_mut() {
            editor.reply = Some(root.to_owned());
            editor.input.update(cx, |input, cx| {
                input.set_placeholder(format!("Reply to {author}…"), cx)
            });
        }
        self.rebuild(cx);
    }

    fn toggle_thread(&mut self, root: &str, cx: &mut Context<Self>) {
        let Some(thread) = self.review.threads.iter().find(|t| t.root == root) else {
            return;
        };
        let folded = self.thread_folded(thread);
        let resolved = thread.resolved;
        let root = root.to_owned();
        match (folded, resolved) {
            (true, true) => {
                self.review.closed.remove(&root);
                self.review.opened.insert(root);
            }
            (true, false) => {
                self.review.closed.remove(&root);
            }
            (false, true) => {
                self.review.opened.remove(&root);
            }
            (false, false) => {
                self.review.closed.insert(root);
            }
        }
        self.rebuild(cx);
    }
    fn toggle_key(set: &mut HashSet<String>, key: String) {
        if !set.remove(&key) {
            set.insert(key);
        }
    }

    /// One remote thread's card under its line.
    pub(super) fn remote_card(
        &self,
        file: usize,
        at: At,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread = &self.review.threads[ix];
        let root = thread.root.clone();
        let siblings = self.remote_at(file, at);
        let position = siblings.iter().position(|s| *s == ix).unwrap_or(0) + 1;
        let first = thread.comments.first();
        if self.thread_folded(thread) {
            return self.folded_card(ix, thread, cx);
        }
        let count = thread.comments.len();
        let header_text = if thread.outdated {
            format!("Was on line {}", thread.line)
        } else if siblings.len() > 1 {
            format!(
                "Line {} · thread {position} of {}",
                thread.line,
                siblings.len()
            )
        } else {
            format!(
                "Line {} · {count} {}",
                thread.line,
                if count == 1 { "comment" } else { "comments" }
            )
        };
        let toggle_root = root.clone();
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(36.))
            .border_b_1()
            .border_color(content(0.07))
            .text_size(px(12.))
            .text_color(content(0.6))
            .child(icon("message-square", 12.).text_color(content(0.55)))
            .when(thread.outdated, |d| {
                d.child(chip("Outdated", rgb(0xf5dc84).into()))
            })
            .child(header_text)
            .child(if thread.resolved {
                chip("Resolved", rgb(0x6ee7b7).into())
            } else {
                chip("Open", content(0.6))
            })
            .child(div().flex_1())
            .child(
                div()
                    .id(SharedString::from(format!("diff-thread-fold-{root}")))
                    .debug_selector({
                        let root = root.clone();
                        move || format!("diff-thread-fold-{root}")
                    })
                    .role(Role::Button)
                    .aria_label("Fold thread")
                    .size(px(24.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.08)))
                    .child(icon("chevron-up", 13.))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_thread(&toggle_root, cx)),
                    ),
            );
        let mut card = div()
            .debug_selector(move || format!("diff-remote-thread-{ix}"))
            .flex_none()
            .rounded(px(12.))
            .bg(content(0.045))
            .border_1()
            .border_color(content(0.12))
            .flex()
            .flex_col()
            .child(header);
        if let Some(first) = first {
            card = card.child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .child(self.comment_view(thread, 0, first, window, cx)),
            );
        }
        if thread.comments.len() > 1 || !thread.pending.is_empty() || self.replying(&root) {
            let mut rail = div()
                .mx(px(14.))
                .mb(px(10.))
                .ml(px(24.))
                .pl(px(14.))
                .border_l_2()
                .border_color(content(0.14))
                .flex()
                .flex_col()
                .gap(px(10.));
            for (cix, comment) in thread.comments.iter().enumerate().skip(1) {
                rail = rail.child(self.comment_view(thread, cix, comment, window, cx));
            }
            for (pix, pending) in thread.pending.iter().enumerate() {
                rail = rail.child(self.pending_view(&root, pix, pending, cx));
            }
            card = card.child(rail);
        }
        if self.replying(&root)
            && let Some(editor_box) = self.editor_box(cx)
        {
            card = card.child(div().mx(px(12.)).mb(px(10.)).child(editor_box));
        }
        let (reply_root, resolve_root) = (root.clone(), root.clone());
        let resolved = thread.resolved;
        card.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(12.))
                .h(px(40.))
                .border_t_1()
                .border_color(content(0.07))
                .when(!self.replying(&root), |d| {
                    d.child(
                        small_action(format!("diff-thread-reply-{root}"), "Reply").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.open_reply(&reply_root, String::new(), window, cx)
                            }),
                        ),
                    )
                })
                .when(thread.resolvable, |d| {
                    d.child(
                        small_action(
                            format!("diff-thread-resolve-{root}"),
                            if resolved { "Unresolve" } else { "Resolve" },
                        )
                        .gap(px(6.))
                        .when(!resolved, |d| d.child(icon("check", 12.)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(DiffEvent::Resolve {
                                root: resolve_root.clone(),
                                resolved: !resolved,
                            })
                        })),
                    )
                })
                .child(
                    div()
                        .ml_auto()
                        .text_size(px(11.5))
                        .text_color(content(0.4))
                        .child("Replies post now, not with your review"),
                ),
        )
        .into_any_element()
    }

    fn replying(&self, root: &str) -> bool {
        self.review
            .editor
            .as_ref()
            .is_some_and(|e| e.reply.as_deref() == Some(root))
    }

    /// A folded thread: one line with its first comment, Open, and for a
    /// resolved one Unresolve.
    fn folded_card(&self, ix: usize, thread: &RemoteThread, cx: &mut Context<Self>) -> AnyElement {
        let root = thread.root.clone();
        let first = thread.comments.first().cloned().unwrap_or_default();
        let line = first
            .body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('<'))
            .unwrap_or("")
            .to_owned();
        let count = thread.comments.len();
        let (open_root, resolve_root) = (root.clone(), root.clone());
        div()
            .debug_selector(move || format!("diff-remote-thread-{ix}"))
            .flex_none()
            .h(px(40.))
            .px(px(12.))
            .rounded(px(10.))
            .bg(content(0.03))
            .border_1()
            .border_color(content(0.09))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(content(0.6))
            .when(thread.resolved, |d| {
                d.child(chip("Resolved", rgb(0x6ee7b7).into()))
            })
            .when(!thread.resolved, |d| {
                d.child(icon("message-square", 12.).text_color(content(0.55)))
            })
            .child(div().min_w_0().flex_1().truncate().child(format!(
                "{}: {line} · {count} {}",
                first.author,
                if count == 1 { "comment" } else { "comments" }
            )))
            .child(
                small_action(
                    format!("diff-thread-open-{root}"),
                    if thread.resolved { "Show" } else { "Open" },
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_thread(&open_root, cx))),
            )
            .when(thread.resolved && thread.resolvable, |d| {
                d.child(
                    small_action(format!("diff-thread-unresolve-{root}"), "Unresolve").on_click(
                        cx.listener(move |_, _, _, cx| {
                            cx.emit(DiffEvent::Resolve {
                                root: resolve_root.clone(),
                                resolved: false,
                            })
                        }),
                    ),
                )
            })
            .into_any_element()
    }

    /// One comment: who, when, and its Markdown, clamped when long.
    fn comment_view(
        &self,
        thread: &RemoteThread,
        cix: usize,
        comment: &RemoteComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("{}:{cix}", thread.root);
        let long = plain_len(&comment.body) > LONG;
        let clamped = long && !self.review.more.contains(&key);
        let parts = self.review.bodies.get(&key).cloned().unwrap_or_default();
        let mut body = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(clamped, |d| d.max_h(px(CLAMP)).overflow_hidden());
        for (pix, part) in parts.into_iter().enumerate() {
            body = body.child(match part {
                BodyPart::Rows(rows) => markdown_rows(&rows, &format!("{key}:{pix}")),
                BodyPart::Details { summary, rows } => {
                    let details_key = format!("{key}:{pix}");
                    let open = self.review.details.contains(&details_key);
                    let toggle = details_key.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .id(SharedString::from(format!("diff-details-{details_key}")))
                                .debug_selector({
                                    let details_key = details_key.clone();
                                    move || format!("diff-details-{details_key}")
                                })
                                .role(Role::Button)
                                .aria_label(summary.clone())
                                .h(px(28.))
                                .px(px(10.))
                                .rounded(px(8.))
                                .bg(gpui::black().opacity(0.22))
                                .border_1()
                                .border_color(content(0.08))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(12.5))
                                .text_color(content(0.85))
                                .cursor_pointer()
                                .hover(|s| s.bg(gpui::black().opacity(0.32)))
                                .child(icon(
                                    if open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    11.,
                                ))
                                .child(summary)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    Self::toggle_key(&mut this.review.details, toggle.clone());
                                    // An opened block shows: under the clamp it
                                    // would open out of sight.
                                    if this.review.details.contains(&toggle) {
                                        this.review.more.insert(comment_key(&toggle));
                                    }
                                    this.rebuild(cx);
                                })),
                        )
                        .when(open, |d| {
                            d.child(div().pl(px(12.)).child(markdown_rows(&rows, &details_key)))
                        })
                        .into_any_element()
                }
            });
        }
        let more_key = key.clone();
        div()
            .debug_selector({
                let key = key.clone();
                move || format!("diff-remote-comment-{key}")
            })
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(22.))
                    .text_size(px(12.))
                    .child(crate::inbox::avatar_image(
                        "",
                        &comment.author,
                        &comment.avatar_url,
                        20.,
                        window,
                        cx,
                    ))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(0.95))
                            .child(comment.author.clone()),
                    )
                    .when(comment.bot, |d| d.child(chip("bot", content(0.6))))
                    .child(div().text_color(content(0.45)).child(comment.when.clone())),
            )
            .child(body)
            .when(long, |d| {
                let id = format!("diff-comment-more-{key}");
                d.child(
                    div().flex().child(
                        div()
                            .id(SharedString::from(id.clone()))
                            .debug_selector(move || id.clone())
                            .role(Role::Button)
                            .text_size(px(12.))
                            .text_color(LINK)
                            .cursor_pointer()
                            .hover(|s| s.underline())
                            .child(if clamped { "Show more" } else { "Show less" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                Self::toggle_key(&mut this.review.more, more_key.clone());
                                this.rebuild(cx);
                            })),
                    ),
                )
            })
            .into_any_element()
    }

    /// A reply sent from here: on its way, sent, or failed with Retry and
    /// Edit.
    fn pending_view(
        &self,
        root: &str,
        ix: usize,
        pending: &PendingReply,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (retry_root, edit_root, text) =
            (root.to_owned(), root.to_owned(), pending.text.clone());
        div()
            .debug_selector({
                let root = root.to_owned();
                move || format!("diff-pending-{root}-{ix}")
            })
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child("You"),
                    )
                    .child(if pending.error.is_some() {
                        "not sent"
                    } else if pending.posted {
                        "sent"
                    } else {
                        "Sending…"
                    }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(content(if pending.posted { 0.9 } else { 0.6 }))
                    .child(pending.text.clone()),
            )
            .when_some(pending.error.clone(), |d, error| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .text_color(rgb(0xfda4af))
                                .child(error),
                        )
                        .child(
                            small_action(format!("diff-pending-retry-{root}-{ix}"), "Retry")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(DiffEvent::RetryReply {
                                        root: retry_root.clone(),
                                        ix,
                                    })
                                })),
                        )
                        .child(
                            small_action(format!("diff-pending-edit-{root}-{ix}"), "Edit")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.emit(DiffEvent::DropReply {
                                        root: edit_root.clone(),
                                        ix,
                                    });
                                    this.open_reply(&edit_root, text.clone(), window, cx);
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

fn markdown_rows(rows: &[MarkdownRow], key: &str) -> AnyElement {
    let now = std::time::Instant::now();
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .children(rows.iter().enumerate().map(|(ix, row)| {
            let id = SharedString::from(format!("diff-md-{key}-{ix}"));
            // Markdown leaves a fenced block for its caller to draw.
            let body = match &row.part {
                markdown::Part::Code { fence, lines } => {
                    let selector = format!("diff-md-code-{key}-{ix}");
                    div()
                        .id(id)
                        .debug_selector(move || selector.clone())
                        .w_full()
                        .min_w_0()
                        .overflow_x_scroll()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(content(0.1))
                        .bg(gpui::black().opacity(0.22))
                        .py(px(6.))
                        .text_size(px(12.))
                        .child(markdown::code_lines(fence, lines.clone(), None, "", false))
                        .into_any_element()
                }
                _ => markdown::body(row, id, markdown::Tone::Normal, None, now),
            };
            div()
                .w_full()
                .min_w_0()
                .pt(px(if ix == 0 { 0. } else { row.gap }))
                .child(body)
        }))
        .into_any_element()
}

fn chip(label: &'static str, ink: Hsla) -> Div {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(8.))
        .rounded_full()
        .flex()
        .items_center()
        .text_size(px(11.))
        .text_color(ink)
        .bg(ink.opacity(0.1))
        .border_1()
        .border_color(ink.opacity(0.22))
        .child(label)
}

/// The comment a `<details>` block belongs to: `root:comment:part` →
/// `root:comment`.
pub(super) fn comment_key(details_key: &str) -> String {
    details_key
        .rsplit_once(':')
        .map_or(details_key, |(comment, _)| comment)
        .to_owned()
}

fn small_action(id: String, label: &'static str) -> gpui::Stateful<Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .role(Role::Button)
        .aria_label(label)
        .h(px(26.))
        .px(px(9.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(content(0.7))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        .child(label)
}

#[cfg(test)]
#[path = "remote_threads_tests.rs"]
mod tests;
