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
/// Where a rewrite goes: the review bar's comment, or the comment box open
/// on a line in Files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PolishTarget {
    Bar,
    Line,
}

struct Pending {
    target: PolishTarget,
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
    /// "Submit review" is open: the summary, its tone and the verdict.
    pub(super) composing: bool,
    pub(super) verdict: Option<Verdict>,
    /// What the last line Polish handed the diff, for tests.
    #[cfg(test)]
    pub(super) line_polish_for_test: Option<Result<(Tone, String), String>>,
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
    /// Polish from a comment box in Files, and what it handed back.
    pub(crate) fn polish_line_for_test(&mut self, draft: &str, tone: Tone, cx: &mut Context<Self>) {
        self.set_tab(Tab::Code, cx);
        self.polish_line(draft.to_string(), tone, cx);
    }
    pub(crate) fn line_polish_outcome_for_test(&self) -> Option<Result<(Tone, String), String>> {
        self.review.line_polish_for_test.clone()
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
            self.review.error = Some(
                if verdict == Verdict::Comment {
                    "Add a comment on a line, or a summary, to comment."
                } else {
                    "Say in the summary what needs to change."
                }
                .into(),
            );
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
                        this.review.composing = false;
                        this.review.verdict = None;
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
        let tone = self.review.tone;
        self.start_polish(PolishTarget::Bar, draft, tone, cx);
    }

    /// Polish asked from a comment box on a line in Files.
    pub(super) fn polish_line(&mut self, draft: String, tone: Tone, cx: &mut Context<Self>) {
        if self.review.pending.is_some() {
            self.diff_view.update(cx, |view, cx| {
                view.polish_result(Err("A rewrite is already on its way.".into()), cx)
            });
            return;
        }
        self.start_polish(PolishTarget::Line, draft.trim().to_string(), tone, cx);
    }

    fn start_polish(
        &mut self,
        target: PolishTarget,
        draft: String,
        tone: Tone,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.item() else {
            return;
        };
        if draft.is_empty() || self.review.pending.is_some() {
            return;
        }
        if target == PolishTarget::Bar {
            self.review.polished = None;
            self.review.polish_error = None;
        }
        self.review.pending = Some(Pending {
            target,
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
                let target = pending.target;
                self.review.pending = None;
                self.finish_polish(
                    target,
                    Err(
                        "Ask has a message you haven't sent. Send or clear it, then Polish again."
                            .into(),
                    ),
                    cx,
                );
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
        let target = pending.target;
        self.review.pending = None;
        let reply = view.read(cx).last_reply(cx);
        let result = polish::extract(&reply)
            .map(|text| (tone, text))
            .ok_or_else(|| "The agent did not return a rewrite. Try again.".to_string());
        self.finish_polish(target, result, cx);
    }

    /// Hands a rewrite, or why there is none, to whoever asked.
    fn finish_polish(
        &mut self,
        target: PolishTarget,
        result: Result<(Tone, String), String>,
        cx: &mut Context<Self>,
    ) {
        match target {
            PolishTarget::Bar => match result {
                Ok(polished) => self.review.polished = Some(polished),
                Err(error) => self.review.polish_error = Some(error),
            },
            PolishTarget::Line => {
                #[cfg(test)]
                {
                    self.review.line_polish_for_test = Some(result.clone());
                }
                self.diff_view
                    .update(cx, |view, cx| view.polish_result(result, cx))
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

    /// "Submit review": opens the panel with the summary and the verdict.
    pub(super) fn open_submit(&mut self, cx: &mut Context<Self>) {
        self.review.composing = true;
        self.review.error = None;
        self.review.notice = None;
        if self.review.verdict.is_none() {
            self.review.verdict = Some(Verdict::Comment);
        }
        self.focus_review = true;
        cx.notify();
    }
    fn close_submit(&mut self, cx: &mut Context<Self>) {
        self.review.composing = false;
        self.review.error = None;
        cx.notify();
    }

    /// The review bar: what is drafted and "Submit review". The panel it
    /// opens takes an optional summary (with its tone and Polish), the
    /// verdict, and sends it all with the line comments.
    pub(super) fn review_bar_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.review_open() {
            return None;
        }
        let review = &self.review;
        let mine = self.item().is_some_and(|item| queue::is_mine(&item));
        let pending = self.diff_view.read(cx).drafts().to_vec();
        let count = pending.len();
        let drafted = if count == 0 {
            "No comments yet · use the + beside a line".to_string()
        } else {
            format!(
                "Your review · {count} comment{} drafted",
                if count == 1 { "" } else { "s" }
            )
        };
        let status = review
            .error
            .clone()
            .or_else(|| review.polish_error.clone())
            .map(|text| (text, true))
            .or_else(|| review.notice.clone().map(|text| (text, false)));
        let status_line = |status: Option<(String, bool)>| {
            status.map(|(text, bad)| {
                div()
                    .debug_selector(|| "git-review-status".into())
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(if bad {
                        rgb(0xfda4af).into()
                    } else {
                        content(0.62)
                    })
                    .child(text)
            })
        };
        if !review.composing {
            return Some(
                glass::card_strong()
                    .debug_selector(|| "git-review-bar".into())
                    .flex_shrink_0()
                    .mx(px(28.))
                    .mt(px(10.))
                    .mb(px(18.))
                    .px(px(14.))
                    .py(px(8.))
                    .rounded(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .debug_selector(|| "git-review-drafted".into())
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(content(if count > 0 { 0.9 } else { 0.55 }))
                            .child(drafted),
                    )
                    .child(div().flex_1().min_w_0().children(status_line(status)))
                    .child(
                        glass::button(
                            "git-review-open",
                            "Submit review",
                            if count > 0 {
                                glass::Button::Primary
                            } else {
                                glass::Button::Glass
                            },
                        )
                        .debug_selector(|| "git-review-open".into())
                        .role(Role::Button)
                        .aria_label("Submit review")
                        .h(px(30.))
                        .child(icon("chevron-up", 12.))
                        .on_click(cx.listener(|this, _, _, cx| this.open_submit(cx))),
                    )
                    .into_any_element(),
            );
        }

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
        let polishing = review.polishing();
        let polish_button = glass::button(
            "git-polish",
            if polishing { "Polishing…" } else { "Polish" },
            glass::Button::Glass,
        )
        .debug_selector(|| "git-polish".into())
        .role(Role::Button)
        .aria_label("Polish")
        .h(px(28.))
        .px(px(10.))
        .child(icon("sparkles", 13.).text_color(rgb(0xc0affc)))
        .when(!has_text || polishing, |d| d.opacity(0.45))
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
        let drafts = (!pending.is_empty()).then(|| {
            div()
                .debug_selector(|| "git-review-drafts".into())
                .flex()
                .flex_col()
                .gap(px(4.))
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
        let chosen = review.verdict.unwrap_or(Verdict::Comment);
        let mut verdicts = vec![(
            Verdict::Comment,
            "Comment",
            "Feedback without approving or blocking.",
        )];
        if !mine {
            verdicts.push((Verdict::Approve, "Approve", "These changes can merge."));
            verdicts.push((
                Verdict::RequestChanges,
                "Request changes",
                "Must be addressed before merging.",
            ));
        }
        let verdict_rows = div()
            .id("git-review-verdicts")
            .role(Role::RadioGroup)
            .aria_label("Verdict")
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(verdicts.into_iter().map(|(verdict, label, hint)| {
                let on = verdict == chosen;
                let id = match verdict {
                    Verdict::Comment => "git-review-verdict-comment",
                    Verdict::Approve => "git-review-verdict-approve",
                    Verdict::RequestChanges => "git-review-verdict-request-changes",
                };
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(on)
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.05)))
                    .when(on, |d| d.bg(content(0.07)))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded_full()
                            .border_1()
                            .border_color(if on { accent_color() } else { content(0.35) })
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(on, |d| {
                                d.child(div().size(px(6.)).rounded_full().bg(accent_color()))
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.95))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(hint),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.review.verdict = Some(verdict);
                        this.review.error = None;
                        cx.notify();
                    }))
            }));
        let sending = review.submitting.is_some();
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
                .child(
                    glass::card_strong()
                        .debug_selector(|| "git-review-panel".into())
                        .px(px(14.))
                        .py(px(12.))
                        .rounded(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key == "escape" {
                                this.close_submit(cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(content(1.))
                                        .child("Finish your review"),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(content(0.55))
                                        .child(drafted.clone()),
                                ),
                        )
                        .children(drafts)
                        .child(
                            div()
                                .debug_selector(|| "git-review-body".into())
                                .min_h(px(56.))
                                .max_h(px(140.))
                                .px(px(10.))
                                .py(px(6.))
                                .rounded(px(8.))
                                .border_1()
                                .border_color(content(0.1))
                                .bg(content(0.03))
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
                                .child(polish_button),
                        )
                        .child(verdict_rows)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().flex_1().min_w_0().children(status_line(status)))
                                .child(
                                    glass::button(
                                        "git-review-cancel",
                                        "Cancel",
                                        glass::Button::Ghost,
                                    )
                                    .debug_selector(|| "git-review-cancel".into())
                                    .role(Role::Button)
                                    .h(px(30.))
                                    .on_click(cx.listener(|this, _, _, cx| this.close_submit(cx))),
                                )
                                .child(
                                    glass::button(
                                        "git-review-submit",
                                        if sending {
                                            "Sending…"
                                        } else {
                                            "Submit review"
                                        },
                                        glass::Button::Primary,
                                    )
                                    .debug_selector(|| "git-review-submit".into())
                                    .role(Role::Button)
                                    .aria_label("Submit review")
                                    .h(px(30.))
                                    .when(sending, |d| d.opacity(0.5))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| this.submit_review(chosen, cx),
                                    )),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}
