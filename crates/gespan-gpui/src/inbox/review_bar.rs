//! The review bar under a pull request's files: a comment, its tone, Polish,
//! and Comment / Request changes / Approve.
//!
//! Polish asks the pull request's Ask agent for a rewrite in the chosen tone.
//! The Ask panel opens on that conversation, so the turn is visible and can be
//! stopped like any other; the rewrite comes back as a card to Use, Try again
//! or Keep mine.
use super::*;
use crate::conversation::Conversation;
use crate::glass;
use gespan_core::inbox::{
    Verdict,
    polish::{self, Tone},
    queue,
};
use gpui::{AnyElement, FontWeight, Role, SharedString, Subscription, div, prelude::*, px, rgb};

/// A Polish turn: sent once the Ask conversation is hosted, answered once
/// the turn it started has finished.
struct Pending {
    key: String,
    prompt: String,
    tone: Tone,
    sent: bool,
    /// The conversation was busy with this turn at some point.
    started: bool,
    _observe: Option<Subscription>,
}

#[derive(Default)]
pub(crate) struct ReviewBar {
    pub(super) tone: Tone,
    pending: Option<Pending>,
    /// The rewrite on offer, and the tone it was written in.
    pub(super) polished: Option<(Tone, String)>,
    pub(super) polish_error: Option<String>,
    pub(super) submitting: Option<Verdict>,
    pub(super) error: Option<String>,
    pub(super) notice: Option<String>,
}
impl ReviewBar {
    pub(super) fn reset(&mut self) {
        let tone = self.tone;
        *self = Self {
            tone,
            ..Default::default()
        };
    }
    /// Typing makes an offered rewrite stale.
    pub(super) fn draft_changed(&mut self) {
        self.error = None;
        self.notice = None;
    }
    pub(super) fn polishing(&self) -> bool {
        self.pending.is_some()
    }
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Comment => "Comment",
        Verdict::RequestChanges => "Request changes",
        Verdict::Approve => "Approve",
    }
}

#[cfg(test)]
impl Inbox {
    /// Files, a draft in `tone`, then Polish.
    pub(crate) fn polish_for_test(&mut self, draft: &str, tone: Tone, cx: &mut Context<Self>) {
        self.set_tab(Tab::Code, cx);
        self.review_body
            .update(cx, |input, cx| input.set_text(draft.to_string(), cx));
        self.review.tone = tone;
        self.polish(cx);
    }
    pub(crate) fn polish_outcome_for_test(&self) -> Option<Result<(Tone, String), String>> {
        self.review
            .polished
            .clone()
            .map(Ok)
            .or_else(|| self.review.polish_error.clone().map(Err))
    }
    pub(crate) fn use_polish_for_test(&mut self, cx: &mut Context<Self>) -> String {
        self.use_polish(cx);
        self.review_body.read(cx).content.to_string()
    }
}

impl Inbox {
    /// An open pull request on a host that takes reviews. Your own gets the
    /// bar too, with Comment only: hosts don't let you approve your own.
    pub(super) fn review_open(&self) -> bool {
        self.item().is_some_and(|item| {
            item.kind == "pr"
                && matches!(item.provider.as_str(), "github" | "gitlab" | "azuredevops")
                && !item.state.eq_ignore_ascii_case("merged")
                && !item.state.eq_ignore_ascii_case("closed")
        })
    }

    pub(super) fn submit_review(&mut self, verdict: Verdict, cx: &mut Context<Self>) {
        if self.review.submitting.is_some() {
            return;
        }
        let Some(item) = self.item() else {
            return;
        };
        let body = self.review_body.read(cx).content.trim().to_string();
        let drafts = self.diff_view.read(cx).drafts().to_vec();
        let words_needed = match verdict {
            Verdict::Approve => false,
            Verdict::RequestChanges => true,
            Verdict::Comment => drafts.is_empty(),
        };
        if body.is_empty() && words_needed {
            self.review.error = Some(format!(
                "Write a comment to {}.",
                if verdict == Verdict::Comment {
                    "comment"
                } else {
                    "request changes"
                }
            ));
            cx.notify();
            return;
        }
        self.review.submitting = Some(verdict);
        self.review.error = None;
        self.review.notice = None;
        let key = item.key();
        let service = self.service.clone();
        let target = gespan_core::inbox::SelfTarget::of(&item);
        #[cfg(test)]
        let offline = self.offline != Offline::Live;
        #[cfg(not(test))]
        let offline = false;
        #[cfg(test)]
        let reviews = self.submitted_reviews.clone();
        let task = cx.background_executor().spawn(async move {
            if offline {
                #[cfg(test)]
                reviews
                    .lock()
                    .unwrap()
                    .push((item.key(), verdict, body, drafts));
                return Ok(
                    gespan_core::inbox::review::review_notice(&item.provider, verdict).to_string(),
                );
            }
            service.submit_review(&item, verdict, &body, &drafts)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.review.submitting = None;
                if this.selected.as_ref() != Some(&key) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(notice) => {
                        this.self_activity
                            .record(target, gespan_core::time::now_ms());
                        this.review_body
                            .update(cx, |input, cx| input.set_text(String::new(), cx));
                        this.review.polished = None;
                        this.diff_view
                            .update(cx, |view, cx| view.set_drafts(vec![], cx));
                        this.review.notice = Some(notice);
                        this.load_detail(false, cx);
                        this.refresh(cx);
                    }
                    Err(error) => this.review.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Polish: asks the Ask agent for the draft in the chosen tone.
    pub(super) fn polish(&mut self, cx: &mut Context<Self>) {
        let draft = self.review_body.read(cx).content.trim().to_string();
        let Some(item) = self.item() else {
            return;
        };
        if draft.is_empty() || self.review.pending.is_some() {
            return;
        }
        let tone = self.review.tone;
        self.review.polished = None;
        self.review.polish_error = None;
        self.review.pending = Some(Pending {
            key: model::ask::key(&item),
            prompt: polish::prompt(&draft, tone),
            tone,
            sent: false,
            started: false,
            _observe: None,
        });
        self.open_ask(cx);
        self.send_polish(cx);
        cx.notify();
    }

    /// Sends a waiting Polish turn once its conversation is on screen.
    pub(super) fn send_polish(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.ask_conversation() else {
            return;
        };
        let Some(hosted) = self.ask_hosted_key() else {
            return;
        };
        let Some(pending) = self.review.pending.as_mut() else {
            return;
        };
        if pending.key != hosted {
            return;
        }
        if pending._observe.is_none() {
            pending._observe =
                Some(cx.observe(&view, |this, view, cx| this.watch_polish(view, cx)));
        }
        self.watch_polish(view, cx);
    }

    /// Sends the Polish turn once the conversation can take it (its history
    /// loaded, no turn running), then waits for that turn to end.
    fn watch_polish(&mut self, view: Entity<Conversation>, cx: &mut Context<Self>) {
        let Some(pending) = self.review.pending.as_mut() else {
            return;
        };
        if !pending.sent {
            // Words in the Ask composer are the user's: never send over them.
            if !view.read(cx).input.read(cx).content.trim().is_empty() {
                self.review.pending = None;
                self.review.polish_error = Some(
                    "Ask has a message you haven't sent. Send or clear it, then Polish again."
                        .into(),
                );
                cx.notify();
                return;
            }
            if !view.read(cx).ready_for_external(cx) {
                return;
            }
            pending.sent = true;
            let prompt = pending.prompt.clone();
            view.update(cx, |conversation, cx| {
                conversation.send_worker_turn(prompt, cx)
            });
            return;
        }
        if view.read(cx).external_submission_pending() {
            pending.started = true;
            return;
        }
        if !pending.started {
            return;
        }
        let tone = pending.tone;
        self.review.pending = None;
        let reply = view.read(cx).last_reply(cx);
        match polish::extract(&reply) {
            Some(text) => self.review.polished = Some((tone, text)),
            None => {
                self.review.polish_error =
                    Some("The agent did not return a rewrite. Try again.".into())
            }
        }
        cx.notify();
    }

    pub(super) fn use_polish(&mut self, cx: &mut Context<Self>) {
        if let Some((_, text)) = self.review.polished.take() {
            self.review_body
                .update(cx, |input, cx| input.set_text(text, cx));
            cx.notify();
        }
    }

    pub(super) fn review_bar_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.review_open() {
            return None;
        }
        let review = &self.review;
        let has_text = !self.review_body.read(cx).content.trim().is_empty();
        let tones = glass::segmented()
            .id("git-review-tone")
            .debug_selector(|| "git-review-tone".into())
            .role(Role::RadioGroup)
            .aria_label("Tone")
            .children(Tone::ALL.into_iter().map(|tone| {
                let on = review.tone == tone;
                glass::segment(
                    SharedString::from(format!("git-tone-{}", tone.label().to_lowercase())),
                    tone.label(),
                    on,
                )
                .debug_selector(move || format!("git-tone-{}", tone.label().to_lowercase()))
                .role(Role::RadioButton)
                .aria_label(tone.label())
                .aria_selected(on)
                .h(px(24.))
                .px(px(9.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.review.tone = tone;
                    cx.notify();
                }))
            }));
        let polish_button = glass::button(
            "git-polish",
            if review.polishing() {
                "Polishing…"
            } else {
                "Polish"
            },
            glass::Button::Glass,
        )
        .debug_selector(|| "git-polish".into())
        .role(Role::Button)
        .aria_label("Polish")
        .h(px(28.))
        .px(px(10.))
        .child(icon("sparkles", 13.).text_color(rgb(0xc0affc)))
        .when(!has_text || review.polishing(), |d| d.opacity(0.45))
        .on_click(cx.listener(|this, _, _, cx| this.polish(cx)));
        let suggestion = review.polished.clone().map(|(tone, text)| {
            glass::card()
                .debug_selector(|| "git-polished".into())
                .px(px(14.))
                .py(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0xc0affc))
                        .child(icon("sparkles", 12.).text_color(rgb(0xc0affc)))
                        .child(format!("Polished · {}", tone.label().to_lowercase())),
                )
                .child(
                    div()
                        .debug_selector(|| "git-polished-text".into())
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .text_color(content(0.95))
                        .child(text),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            glass::button("git-polish-keep", "Keep mine", glass::Button::Ghost)
                                .debug_selector(|| "git-polish-keep".into())
                                .role(Role::Button)
                                .h(px(28.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review.polished = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            glass::button("git-polish-again", "Try again", glass::Button::Glass)
                                .debug_selector(|| "git-polish-again".into())
                                .role(Role::Button)
                                .h(px(28.))
                                .on_click(cx.listener(|this, _, _, cx| this.polish(cx))),
                        )
                        .child(
                            glass::button("git-polish-use", "Use", glass::Button::Primary)
                                .debug_selector(|| "git-polish-use".into())
                                .role(Role::Button)
                                .h(px(28.))
                                .on_click(cx.listener(|this, _, _, cx| this.use_polish(cx))),
                        ),
                )
        });
        let verdict = |id: &'static str, verdict: Verdict, kind: glass::Button| {
            let label = if review.submitting == Some(verdict) {
                "Sending…"
            } else {
                verdict_label(verdict)
            };
            glass::button(id, label, kind)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(verdict_label(verdict))
                .h(px(30.))
                .when(review.submitting.is_some(), |d| d.opacity(0.5))
                .on_click(cx.listener(move |this, _, _, cx| this.submit_review(verdict, cx)))
        };
        let mine = self.item().is_some_and(|item| queue::is_mine(&item));
        let pending = self.diff_view.read(cx).drafts().to_vec();
        let drafts = (!pending.is_empty()).then(|| {
            let count = pending.len();
            glass::card()
                .debug_selector(|| "git-review-drafts".into())
                .px(px(14.))
                .py(px(10.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(content(0.75))
                        .child(format!(
                            "Your review · {count} comment{} drafted",
                            if count == 1 { "" } else { "s" }
                        )),
                )
                .children(pending.iter().enumerate().map(|(ix, draft)| {
                    div()
                        .debug_selector(move || format!("git-review-draft-{ix}"))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex_none()
                                .font_family(crate::fonts::monospace())
                                .text_size(px(11.))
                                .text_color(content(0.55))
                                .child(format!("{} · {}", draft.path, draft.lines_label())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(content(0.9))
                                .child(draft.body.clone()),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("git-review-draft-remove-{ix}")))
                                .debug_selector(move || format!("git-review-draft-remove-{ix}"))
                                .role(Role::Button)
                                .aria_label("Remove this comment")
                                .cursor_pointer()
                                .child(icon("x", 12.).text_color(content(0.5)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.diff_view
                                        .update(cx, |view, cx| view.delete_draft(ix, cx));
                                    cx.notify();
                                })),
                        )
                }))
        });
        let status = review
            .error
            .clone()
            .or_else(|| review.polish_error.clone())
            .map(|text| (text, true))
            .or_else(|| review.notice.clone().map(|text| (text, false)));
        Some(
            div()
                .debug_selector(|| "git-review-bar".into())
                .flex_shrink_0()
                .px(px(28.))
                .pt(px(10.))
                .pb(px(18.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(suggestion)
                .children(drafts)
                .child(
                    glass::card_strong()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .debug_selector(|| "git-review-body".into())
                                .min_h(px(40.))
                                .max_h(px(140.))
                                .child(self.review_body.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(content(0.55))
                                        .child("Tone"),
                                )
                                .child(tones)
                                .child(polish_button)
                                .child(div().flex_1())
                                .child(verdict(
                                    "git-review-comment",
                                    Verdict::Comment,
                                    glass::Button::Ghost,
                                ))
                                .when(!mine, |d| {
                                    d.child(verdict(
                                        "git-review-request-changes",
                                        Verdict::RequestChanges,
                                        glass::Button::Glass,
                                    ))
                                    .child(verdict(
                                        "git-review-approve",
                                        Verdict::Approve,
                                        glass::Button::Primary,
                                    ))
                                }),
                        ),
                )
                .when_some(status, |d, (text, bad)| {
                    d.child(
                        div()
                            .debug_selector(|| "git-review-status".into())
                            .text_size(px(12.))
                            .text_color(if bad {
                                rgb(0xfda4af).into()
                            } else {
                                content(0.62)
                            })
                            .child(text),
                    )
                })
                .into_any_element(),
        )
    }
}
