//! Replying on the Overview, as the "Comment replies" canvas draws it: the
//! reply box opens under the comment you answer, not in the composer at the
//! bottom. A comment its host threads (a review thread, a GitLab discussion,
//! an Azure DevOps thread, a Linear comment) takes the reply in its thread;
//! one it can't (a GitHub general comment) gets a quote reply, a new comment
//! that quotes it and mentions its author. Several boxes can be open at once,
//! each with its own tone and Polish. A sent reply shows under its thread
//! while it goes, and keeps its error, Retry and Edit there if it fails.
//! Long threads fold their earlier replies, resolved threads fold to a line,
//! and runs of bot comments fold together.
use super::ui::action;
use super::*;
use crate::glass;
use gespan_core::inbox::polish::Tone;
use gpui::{
    AnyElement, FontWeight, Role, SharedString, Subscription, Window, div, prelude::*, px, rgb,
};
use std::collections::{HashMap, HashSet};

/// Replies past this many fold behind "Show N earlier replies".
pub(super) const SHOWN_REPLIES: usize = 3;

/// Where a reply to a comment goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ReplyTarget {
    /// Into the host's thread with this id.
    Thread(String),
    /// A new comment quoting `quoted` and mentioning `author`.
    Quote { author: String, quoted: String },
}

pub(super) struct ReplyBox {
    /// Names this box for Polish's answer.
    pub id: u64,
    pub input: Entity<Input>,
    pub target: ReplyTarget,
    /// Who is answered, for the box's label.
    pub author: String,
    pub tone: Tone,
    pub polishing: bool,
    pub polished: Option<(Tone, String)>,
    pub polish_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// A reply on its way, or one that didn't go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SentReply {
    /// The words as typed, for Edit.
    pub text: String,
    pub target: ReplyTarget,
    pub author: String,
    pub error: Option<String>,
    /// The host took it; it stays shown until the thread is read again.
    pub posted: bool,
}

#[derive(Default)]
pub(super) struct Replies {
    /// Open reply boxes, by the comment they answer.
    pub boxes: HashMap<String, ReplyBox>,
    /// Sent replies, by the comment they answer.
    pub sent: HashMap<String, Vec<SentReply>>,
    /// Folds opened: a long or resolved thread, or a run of bot comments, by
    /// its first comment's id.
    pub open: HashSet<String>,
    /// Threads being resolved or reopened.
    pub resolving: HashSet<String>,
    /// Why resolving a thread failed.
    pub errors: HashMap<String, String>,
    next_id: u64,
}

impl Replies {
    pub(super) fn clear(&mut self) {
        *self = Self {
            next_id: self.next_id,
            ..Default::default()
        };
    }
    /// The thread was read again: replies it now holds stop showing as sent.
    pub(super) fn thread_loaded(&mut self) {
        for sent in self.sent.values_mut() {
            sent.retain(|reply| !reply.posted);
        }
        self.sent.retain(|_, sent| !sent.is_empty());
    }
    /// What a thread shows besides its comments, for the list to tell when
    /// its row changed.
    pub(super) fn state_key(&self, root: &str) -> String {
        let sent = self.sent.get(root).map_or(0, Vec::len);
        let failed = self
            .sent
            .get(root)
            .map_or(0, |s| s.iter().filter(|r| r.error.is_some()).count());
        let boxed = self.boxes.get(root).map(|b| {
            (
                b.id,
                b.polishing,
                b.polished.is_some(),
                b.polish_error.is_some(),
            )
        });
        format!(
            "{boxed:?}:{sent}:{failed}:{}:{}:{}",
            self.open.contains(root),
            self.resolving.contains(root),
            self.errors.contains_key(root)
        )
    }
}

/// Where a reply to `comment` goes on `item`'s host.
pub(super) fn target(item: &Item, comment: &model::Comment) -> ReplyTarget {
    let threaded = match item.provider.as_str() {
        "linear" => return ReplyTarget::Thread(comment.id.clone()),
        "github" | "gitlab" => !comment.thread_id.is_empty(),
        "azuredevops" => item.kind == "pr" && !comment.thread_id.is_empty(),
        _ => false,
    };
    if threaded {
        ReplyTarget::Thread(comment.thread_id.clone())
    } else {
        ReplyTarget::Quote {
            author: comment.author.clone(),
            quoted: comment.body.clone(),
        }
    }
}

/// A comment folded with other bots' comments.
pub(super) fn is_bot_comment(comment: &model::Comment) -> bool {
    comment.replies.is_empty()
        && comment.path.is_empty()
        && (comment.author_bot || model::queue::is_bot(&comment.author))
}

impl Inbox {
    /// Opens the reply box under `comment`, or focuses it if it's open.
    pub(super) fn open_reply(
        &mut self,
        comment: &model::Comment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.item() else {
            return;
        };
        let root = comment.id.clone();
        if let Some(open) = self.replies.boxes.get(&root) {
            window.focus(&open.input.focus_handle(cx), cx);
            return;
        }
        let author = if comment.author.is_empty() {
            "ghost".to_owned()
        } else {
            comment.author.clone()
        };
        self.insert_reply_box(
            &root,
            target(&item, comment),
            author,
            String::new(),
            window,
            cx,
        );
    }

    fn insert_reply_box(
        &mut self,
        root: &str,
        target: ReplyTarget,
        author: String,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let quote = matches!(target, ReplyTarget::Quote { .. });
        let input = cx.new(|cx| {
            let mut input = Input::inbox_comment(cx);
            input.set_placeholder(
                if quote {
                    format!("Quote reply to {author}…")
                } else {
                    format!("Reply to {author}…")
                },
                cx,
            );
            input.set_text(text, cx);
            input
        });
        window.focus(&input.focus_handle(cx), cx);
        let key = root.to_owned();
        let changed_key = key.clone();
        let subscriptions = vec![
            cx.subscribe(&input, move |this, _, _: &input::Changed, cx| {
                if let Some(open) = this.replies.boxes.get_mut(&changed_key)
                    && open.polish_error.is_some()
                {
                    open.polish_error = None;
                    this.replies_changed(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&input, move |this, _, _: &input::Submitted, cx| {
                this.send_reply(&key, cx)
            }),
        ];
        self.replies.next_id += 1;
        let id = self.replies.next_id;
        self.replies.boxes.insert(
            root.to_owned(),
            ReplyBox {
                id,
                input,
                target,
                author,
                tone: self.review.tone,
                polishing: false,
                polished: None,
                polish_error: None,
                _subscriptions: subscriptions,
            },
        );
        self.replies_changed(cx);
    }

    pub(super) fn cancel_reply(&mut self, root: &str, cx: &mut Context<Self>) {
        if self.replies.boxes.remove(root).is_some() {
            self.replies_changed(cx);
        }
    }

    /// A reply box's or sent reply's look changed: its row is laid out
    /// again, and Files shows the same threads.
    pub(super) fn replies_changed(&mut self, cx: &mut Context<Self>) {
        self.activity_revision += 1;
        if let Some(thread) = &self.thread {
            let threads = super::remote_threads(self.item().as_ref(), thread, &self.replies);
            self.diff_view
                .update(cx, |view, cx| view.set_threads(threads, cx));
        }
        cx.notify();
    }

    /// A reply written in Files, into the thread `root` starts.
    pub(super) fn reply_from_files(&mut self, root: &str, text: &str, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        let Some(comment) = self
            .thread
            .as_ref()
            .and_then(|t| t.comments.iter().find(|c| c.id == root))
            .cloned()
        else {
            return;
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let reply = SentReply {
            text,
            target: target(&item, &comment),
            author: comment.author.clone(),
            error: None,
            posted: false,
        };
        self.replies
            .sent
            .entry(root.to_owned())
            .or_default()
            .push(reply.clone());
        self.post_reply(root.to_owned(), reply, cx);
        self.replies_changed(cx);
    }

    /// A failed reply went back in Files' box to be edited.
    pub(super) fn drop_reply(&mut self, root: &str, ix: usize, cx: &mut Context<Self>) {
        if let Some(sent) = self.replies.sent.get_mut(root)
            && ix < sent.len()
            && sent[ix].error.is_some()
        {
            sent.remove(ix);
            if sent.is_empty() {
                self.replies.sent.remove(root);
            }
            self.replies_changed(cx);
        }
    }

    /// Sends the box under `root`: into its thread, or as a quote reply.
    pub(super) fn send_reply(&mut self, root: &str, cx: &mut Context<Self>) {
        let Some(open) = self.replies.boxes.get(root) else {
            return;
        };
        let text = open.input.read(cx).content.trim().to_string();
        if text.is_empty() {
            return;
        }
        let reply = SentReply {
            text,
            target: open.target.clone(),
            author: open.author.clone(),
            error: None,
            posted: false,
        };
        self.replies.boxes.remove(root);
        self.replies
            .sent
            .entry(root.to_owned())
            .or_default()
            .push(reply.clone());
        self.post_reply(root.to_owned(), reply, cx);
        self.replies_changed(cx);
    }

    fn post_reply(&mut self, root: String, reply: SentReply, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        let (body, reply_to) = match &reply.target {
            ReplyTarget::Thread(thread) => (reply.text.clone(), thread.clone()),
            ReplyTarget::Quote { author, quoted } => (
                model::quote_reply(author, quoted, &reply.text),
                String::new(),
            ),
        };
        let key = item.key();
        let service = self.service.clone();
        let target = model::SelfTarget::of(&item);
        #[cfg(test)]
        let canned = self.canned_reply_result.clone();
        #[cfg(test)]
        self.posted_replies
            .borrow_mut()
            .push((body.clone(), reply_to.clone()));
        let task = cx.background_executor().spawn(async move {
            #[cfg(test)]
            if let Some(result) = canned {
                return result;
            }
            service.comment(&item, body, reply_to)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.selected.as_ref() != Some(&key) {
                    return;
                }
                let Some(sent) = this.replies.sent.get_mut(&root) else {
                    return;
                };
                let Some(entry) = sent
                    .iter_mut()
                    .find(|s| s.text == reply.text && !s.posted && s.error.is_none())
                else {
                    return;
                };
                match result {
                    Ok(_) => {
                        entry.posted = true;
                        this.self_activity
                            .record(target, gespan_core::time::now_ms());
                        this.load_detail(false, cx);
                    }
                    Err(error) => entry.error = Some(error),
                }
                this.replies_changed(cx);
            });
        })
        .detach();
    }

    /// Sends a failed reply again.
    pub(super) fn retry_reply(&mut self, root: &str, ix: usize, cx: &mut Context<Self>) {
        let Some(reply) = self
            .replies
            .sent
            .get_mut(root)
            .and_then(|sent| sent.get_mut(ix))
        else {
            return;
        };
        if reply.error.take().is_none() {
            return;
        }
        let reply = reply.clone();
        self.post_reply(root.to_owned(), reply, cx);
        self.replies_changed(cx);
    }

    /// Puts a failed reply's words back in its box to change them.
    pub(super) fn edit_reply(
        &mut self,
        root: &str,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sent) = self.replies.sent.get_mut(root) else {
            return;
        };
        if ix >= sent.len() || sent[ix].error.is_none() {
            return;
        }
        let reply = sent.remove(ix);
        if sent.is_empty() {
            self.replies.sent.remove(root);
        }
        self.replies.boxes.remove(root);
        self.insert_reply_box(root, reply.target, reply.author, reply.text, window, cx);
    }

    pub(super) fn set_reply_tone(&mut self, root: &str, tone: Tone, cx: &mut Context<Self>) {
        if let Some(open) = self.replies.boxes.get_mut(root) {
            open.tone = tone;
            self.replies_changed(cx);
        }
    }

    pub(super) fn polish_reply(&mut self, root: &str, cx: &mut Context<Self>) {
        let Some(open) = self.replies.boxes.get_mut(root) else {
            return;
        };
        let draft = open.input.read(cx).content.trim().to_string();
        if draft.is_empty() || open.polishing {
            return;
        }
        if self.review.polishing() {
            open.polish_error = Some("A rewrite is already on its way.".into());
            self.replies_changed(cx);
            return;
        }
        open.polishing = true;
        open.polished = None;
        open.polish_error = None;
        let (id, tone) = (open.id, open.tone);
        self.start_polish(review_bar::PolishTarget::Reply(id), draft, tone, cx);
        self.replies_changed(cx);
    }

    /// Polish's answer for the box it was asked from, if that box is open.
    pub(super) fn reply_polished(
        &mut self,
        id: u64,
        result: Result<(Tone, String), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.replies.boxes.values_mut().find(|b| b.id == id) else {
            return;
        };
        open.polishing = false;
        match result {
            Ok(polished) => open.polished = Some(polished),
            Err(error) => open.polish_error = Some(error),
        }
        self.replies_changed(cx);
    }

    pub(super) fn use_reply_polish(
        &mut self,
        root: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.replies.boxes.get_mut(root) else {
            return;
        };
        if let Some((_, text)) = open.polished.take() {
            open.input.update(cx, |input, cx| input.set_text(text, cx));
        }
        window.focus(&open.input.focus_handle(cx), cx);
        self.replies_changed(cx);
    }

    pub(super) fn keep_reply(&mut self, root: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.replies.boxes.get_mut(root) else {
            return;
        };
        open.polished = None;
        open.polish_error = None;
        window.focus(&open.input.focus_handle(cx), cx);
        self.replies_changed(cx);
    }

    /// Opens or closes a fold: a long or resolved thread, or a run of bots.
    pub(super) fn toggle_fold(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.replies.open.remove(key) {
            self.replies.open.insert(key.to_owned());
        }
        self.replies_changed(cx);
    }

    /// Resolves the thread `root` starts, or opens it again. It shows its
    /// new state at once and goes back if the host refuses.
    pub(super) fn resolve_thread(&mut self, root: &str, resolved: bool, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        let Some(thread_id) = self.set_resolved(root, resolved) else {
            return;
        };
        self.replies.resolving.insert(root.to_owned());
        self.replies.errors.remove(root);
        // Resolving folds the thread; opening it shows it whole.
        self.replies.open.remove(root);
        self.rebuild_activity();
        let key = item.key();
        let service = self.service.clone();
        #[cfg(test)]
        let canned = self.canned_reply_result.clone();
        let task = cx.background_executor().spawn(async move {
            #[cfg(test)]
            if let Some(result) = canned {
                return result.map(|_| ());
            }
            service.resolve_thread(&item, &thread_id, resolved)
        });
        let root = root.to_owned();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.replies.resolving.remove(&root);
                if this.selected.as_ref() != Some(&key) {
                    return;
                }
                if let Err(error) = result {
                    this.set_resolved(&root, !resolved);
                    this.replies.errors.insert(root.clone(), error);
                    this.rebuild_activity();
                }
                this.replies_changed(cx);
            });
        })
        .detach();
        self.replies_changed(cx);
    }

    /// Marks the thread `root` starts as `resolved` in what's shown; its
    /// host thread id, or None when there's no such resolvable thread.
    fn set_resolved(&mut self, root: &str, resolved: bool) -> Option<String> {
        let thread = self.thread.as_mut()?;
        let comment = thread
            .comments
            .iter_mut()
            .find(|c| c.id == root && c.resolvable && !c.thread_id.is_empty())?;
        comment.resolved = resolved;
        for reply in &mut comment.replies {
            reply.resolved = resolved;
        }
        Some(comment.thread_id.clone())
    }

    /// What follows a thread's comments: replies on their way or failed, the
    /// open reply box, and the thread's own actions.
    pub(super) fn thread_footer(
        &self,
        item: &Item,
        comment: &model::Comment,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = comment.id.clone();
        let sent = self.replies.sent.get(&root).cloned().unwrap_or_default();
        let open = self.replies.boxes.get(&root);
        let quote = matches!(target(item, comment), ReplyTarget::Quote { .. });
        let resolvable = comment.resolvable
            && !comment.thread_id.is_empty()
            && matches!(item.provider.as_str(), "github" | "gitlab" | "azuredevops");
        let resolving = self.replies.resolving.contains(&root);
        let mut footer = div()
            .debug_selector({
                let root = root.clone();
                move || format!("inbox-thread-footer-{root}")
            })
            .w_full()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(content(0.07))
            .flex()
            .flex_col()
            .gap(px(8.));
        for (ix, reply) in sent.into_iter().enumerate() {
            footer = footer.child(sent_reply(&root, ix, &reply, cx));
        }
        if let Some(open) = open {
            footer = footer.child(self.reply_box(&root, open, width, cx));
        }
        let reply_label = if quote { "Quote reply" } else { "Reply" };
        let error = self.replies.errors.get(&root).cloned();
        footer
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .when(open.is_none(), |d| {
                        let comment = comment.clone();
                        d.child(
                            action(format!("inbox-reply-{root}"), reply_label)
                                .debug_selector({
                                    let root = root.clone();
                                    move || format!("inbox-reply-{root}")
                                })
                                .h(px(26.))
                                .child(icon("arrow-turn-backward", 12.))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_reply(&comment, window, cx)
                                })),
                        )
                    })
                    .when(resolvable, |d| {
                        let resolved = comment.resolved;
                        let root = root.clone();
                        d.child(
                            action(
                                format!("inbox-resolve-{root}"),
                                if resolving {
                                    "Saving…"
                                } else if resolved {
                                    "Unresolve"
                                } else {
                                    "Resolve"
                                },
                            )
                            .debug_selector({
                                let root = root.clone();
                                move || format!("inbox-resolve-{root}")
                            })
                            .h(px(26.))
                            .when(!resolved, |d| d.child(icon("check", 12.)))
                            .when(resolving, |d| d.opacity(0.5))
                            .when(!resolving, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.resolve_thread(&root, !resolved, cx)
                                }))
                            }),
                        )
                    })
                    .when(quote, |d| {
                        d.child(
                            div()
                                .ml_auto()
                                .text_size(px(11.5))
                                .text_color(content(0.4))
                                .child(format!(
                                    "{} doesn't thread this comment",
                                    model::provider_label(&item.provider)
                                )),
                        )
                    }),
            )
            .when_some(error, |d, text| d.child(super::ui::error(text)))
            .into_any_element()
    }

    fn reply_box(
        &self,
        root: &str,
        open: &ReplyBox,
        _width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_text = !open.input.read(cx).content.trim().is_empty();
        let quote = matches!(open.target, ReplyTarget::Quote { .. });
        let violet = rgb(0xc0affc);
        let key = root.to_owned();
        let tones = glass::segmented()
            .id(SharedString::from(format!("inbox-reply-tone-{root}")))
            .role(Role::RadioGroup)
            .aria_label("Tone")
            .children(Tone::ALL.into_iter().map(|tone| {
                let on = open.tone == tone;
                let key = key.clone();
                let id = format!("inbox-reply-tone-{}-{root}", tone.label().to_lowercase());
                let selector = id.clone();
                glass::segment(SharedString::from(id), tone.label(), on)
                    .debug_selector(move || selector.clone())
                    .role(Role::RadioButton)
                    .aria_label(tone.label())
                    .aria_selected(on)
                    .h(px(22.))
                    .px(px(8.))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_reply_tone(&key, tone, cx)),
                    )
            }));
        let polished = open.polished.clone().map(|(tone, text)| {
            let (use_key, keep_key, again_key) = (key.clone(), key.clone(), key.clone());
            div()
                .debug_selector({
                    let root = root.to_owned();
                    move || format!("inbox-reply-polished-{root}")
                })
                .px(px(12.))
                .py(px(10.))
                .rounded(px(10.))
                .border_1()
                .border_color(gpui::Hsla::from(violet).opacity(0.3))
                .bg(gpui::Hsla::from(violet).opacity(0.07))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(violet)
                        .child(icon("sparkles", 12.).text_color(violet))
                        .child(format!("Polished · {}", tone.label().to_lowercase())),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(content(0.95))
                        .child(text),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            action(format!("inbox-reply-keep-{root}"), "Keep mine").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.keep_reply(&keep_key, window, cx)
                                }),
                            ),
                        )
                        .child(
                            action(format!("inbox-reply-again-{root}"), "Try again").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.polish_reply(&again_key, cx)
                                }),
                            ),
                        )
                        .child(
                            glass::button(
                                SharedString::from(format!("inbox-reply-use-{root}")),
                                "Use",
                                glass::Button::Primary,
                            )
                            .debug_selector({
                                let root = root.to_owned();
                                move || format!("inbox-reply-use-{root}")
                            })
                            .h(px(26.))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.use_reply_polish(&use_key, window, cx)
                                },
                            )),
                        ),
                )
        });
        let (cancel_key, polish_key, send_key) = (key.clone(), key.clone(), key.clone());
        div()
            .debug_selector({
                let root = root.to_owned();
                move || format!("inbox-reply-box-{root}")
            })
            .px(px(12.))
            .py(px(10.))
            .rounded(px(10.))
            .border_1()
            .border_color(accent_color().opacity(0.45))
            .bg(content(0.03))
            .flex()
            .flex_col()
            .gap(px(8.))
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_reply(&cancel_key, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .text_color(content(0.5))
                    .child(if quote {
                        format!("Quote reply to {} · posts as a new comment", open.author)
                    } else {
                        format!("Reply to {} · in this thread", open.author)
                    })
                    .child(div().ml_auto().child(if cfg!(target_os = "macos") {
                        "⌘↩ to send · Esc to cancel"
                    } else {
                        "Ctrl+↩ to send · Esc to cancel"
                    })),
            )
            .children(polished)
            .child(div().w_full().child(open.input.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(content(0.5))
                            .child("Tone"),
                    )
                    .child(tones)
                    .child(
                        glass::button(
                            SharedString::from(format!("inbox-reply-polish-{root}")),
                            if open.polishing {
                                "Polishing…"
                            } else {
                                "Polish"
                            },
                            glass::Button::Glass,
                        )
                        .debug_selector({
                            let root = root.to_owned();
                            move || format!("inbox-reply-polish-{root}")
                        })
                        .h(px(26.))
                        .px(px(9.))
                        .child(icon("sparkles", 12.).text_color(violet))
                        .when(!has_text || open.polishing, |d| d.opacity(0.45))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.polish_reply(&polish_key, cx)),
                        ),
                    )
                    .child(div().flex_1())
                    .child({
                        let key = key.clone();
                        action(format!("inbox-reply-cancel-{root}"), "Cancel").on_click(
                            cx.listener(move |this, _, _, cx| this.cancel_reply(&key, cx)),
                        )
                    })
                    .child(
                        glass::button(
                            SharedString::from(format!("inbox-reply-send-{root}")),
                            if quote { "Post comment" } else { "Reply" },
                            glass::Button::Primary,
                        )
                        .debug_selector({
                            let root = root.to_owned();
                            move || format!("inbox-reply-send-{root}")
                        })
                        .h(px(28.))
                        .when(!has_text, |d| d.opacity(0.4))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.send_reply(&send_key, cx)),
                        ),
                    ),
            )
            .when_some(open.polish_error.clone(), |d, text| {
                d.child(super::ui::error(text))
            })
            .into_any_element()
    }

    /// A resolved thread folded to one line: where it is, who started it
    /// and how long it is, with Show and Unresolve.
    pub(super) fn resolved_line(
        &self,
        item: &Item,
        comment: &model::Comment,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = comment.id.clone();
        let place = if comment.path.is_empty() {
            String::new()
        } else {
            format!(
                "{}{} · ",
                comment.path,
                comment.line.map(|l| format!(":{l}")).unwrap_or_default()
            )
        };
        let first = comment
            .body
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let count = 1 + comment.replies.len();
        let resolvable = matches!(item.provider.as_str(), "github" | "gitlab" | "azuredevops");
        let (show_key, reopen_key) = (root.clone(), root.clone());
        div()
            .debug_selector({
                let root = root.clone();
                move || format!("inbox-resolved-{root}")
            })
            .w_full()
            .min_w_0()
            .h(px(40.))
            .px(px(12.))
            .rounded(px(6.))
            .border_1()
            .border_color(content(0.08))
            .bg(content(0.02))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(content(0.55))
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(0x4ade80))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Resolved"),
            )
            .child(div().min_w_0().flex_1().truncate().child(format!(
                "{place}{}: {} · {count} {}",
                comment.author,
                first.trim(),
                if count == 1 { "comment" } else { "comments" }
            )))
            .child(
                action(format!("inbox-resolved-show-{root}"), "Show")
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_fold(&show_key, cx))),
            )
            .when(resolvable, |d| {
                d.child(
                    action(format!("inbox-resolved-reopen-{root}"), "Unresolve")
                        .h(px(26.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.resolve_thread(&reopen_key, false, cx)
                        })),
                )
            })
            .into_any_element()
    }

    /// A run of bot comments folded to one line.
    pub(super) fn bots_line(
        &self,
        item: &Item,
        comments: &[std::sync::Arc<model::Comment>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = comments[0].id.clone();
        let mut names: Vec<String> = Vec::new();
        for comment in comments {
            let name = comment.author.trim_start_matches("app/").to_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let count = comments.len();
        div()
            .debug_selector({
                let key = key.clone();
                move || format!("inbox-bots-{key}")
            })
            .w_full()
            .min_w_0()
            .h(px(40.))
            .px(px(12.))
            .rounded(px(6.))
            .border_1()
            .border_dashed()
            .border_color(content(0.14))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(12.))
            .text_color(content(0.6))
            .child(
                div()
                    .flex()
                    .children(comments.iter().take(3).enumerate().map(|(ix, c)| {
                        div().when(ix > 0, |d| d.ml(px(-6.))).child(avatar::image(
                            &item.provider,
                            &c.author,
                            &c.author_avatar_url,
                            20.,
                            window,
                            cx,
                        ))
                    })),
            )
            .child(div().min_w_0().flex_1().truncate().child(format!(
                "{count} bot {} · {}",
                if count == 1 { "comment" } else { "comments" },
                names.join(", ")
            )))
            .child(
                action(format!("inbox-bots-show-{key}"), "Show")
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_fold(&key, cx))),
            )
            .into_any_element()
    }
}

/// A sent reply under its thread: "Sending…", or why it didn't go with
/// Retry and Edit.
fn sent_reply(root: &str, ix: usize, reply: &SentReply, cx: &mut Context<Inbox>) -> AnyElement {
    let failed = reply.error.clone();
    let (retry_key, edit_key) = (root.to_owned(), root.to_owned());
    div()
        .debug_selector({
            let root = root.to_owned();
            move || format!("inbox-sent-reply-{root}-{ix}")
        })
        .pl(px(12.))
        .border_l_2()
        .border_color(content(0.1))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(content(0.5))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(1.))
                        .child("You"),
                )
                .child(if failed.is_some() {
                    "not sent"
                } else if reply.posted {
                    "sent"
                } else {
                    "Sending…"
                }),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(content(if failed.is_some() || !reply.posted {
                    0.6
                } else {
                    0.9
                }))
                .child(reply.text.clone()),
        )
        .when_some(failed, |d, error| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().min_w_0().child(super::ui::error(error)))
                    .child(
                        glass::button(
                            SharedString::from(format!("inbox-sent-retry-{root}-{ix}")),
                            "Retry",
                            glass::Button::Glass,
                        )
                        .debug_selector({
                            let root = root.to_owned();
                            move || format!("inbox-sent-retry-{root}-{ix}")
                        })
                        .h(px(26.))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.retry_reply(&retry_key, ix, cx)),
                        ),
                    )
                    .child(
                        action(format!("inbox-sent-edit-{root}-{ix}"), "Edit")
                            .debug_selector({
                                let root = root.to_owned();
                                move || format!("inbox-sent-edit-{root}-{ix}")
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_reply(&edit_key, ix, window, cx)
                            })),
                    ),
            )
        })
        .into_any_element()
}
