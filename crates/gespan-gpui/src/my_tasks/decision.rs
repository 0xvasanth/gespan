//! A decision on the card: the agent's text and why, what the user said, any
//! flag, and the review controls. A proposal offers Approve, Edit, Always
//! include and Reject, with "Replaces:" choices when it reads like active
//! entries; a flag offers "Replace with…" and Dismiss.
use super::card::answer;
use super::*;
use crate::glass::{self, Button};
use gespan_core::decisions::Edit;
use gespan_core::decisions::review::shown_hash;
use gpui::{AnyElement, FontWeight, Role, SharedString, px};

/// A line of the card's text: `label` muted, then `text`.
fn line(selector: &'static str, label: Option<&'static str>, text: &str) -> AnyElement {
    div()
        .debug_selector(move || selector.into())
        .text_size(px(13.))
        .line_height(px(20.))
        .text_color(content(0.8))
        .when_some(label, |d, label| {
            d.child(
                div()
                    .text_color(content(0.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
        })
        .child(text.to_owned())
        .into_any_element()
}

fn toggled(on: bool) -> gpui::Toggled {
    if on {
        gpui::Toggled::True
    } else {
        gpui::Toggled::False
    }
}

/// One edit field, in the glass field style, growing with its text.
fn field(selector: &'static str, input: &Entity<Input>) -> AnyElement {
    glass::field()
        .debug_selector(move || selector.into())
        .h_auto()
        .min_h(px(34.))
        .py(px(6.))
        .child(div().flex_1().min_w_0().child(input.clone()))
        .into_any_element()
}

impl MyTasks {
    /// The wording in the edit fields.
    fn edited(&self, cx: &gpui::App) -> Edit {
        let read = |input: &Entity<Input>| input.read(cx).content.to_string();
        Edit {
            title: read(&self.decision_title),
            text: read(&self.decision_text),
            why: read(&self.decision_why),
        }
    }

    /// Opens the edit fields on `entry`'s wording.
    fn start_editing(&mut self, entry: &Entry, cx: &mut Context<Self>) {
        for (input, text) in [
            (&self.decision_title, &entry.title),
            (&self.decision_text, &entry.text),
            (&self.decision_why, &entry.why),
        ] {
            input.update(cx, |input, cx| input.set_text(text.clone(), cx));
        }
        self.decision_editing = true;
        cx.notify();
    }

    /// Shows why the review of the task `key` failed, on its card.
    pub(crate) fn set_review_error(&mut self, key: String, error: String, cx: &mut Context<Self>) {
        self.review_error = Some((key, error));
        cx.notify();
    }

    /// A review that went through: the card starts fresh on the next task.
    pub(crate) fn review_done(&mut self, cx: &mut Context<Self>) {
        self.review_error = None;
        self.decision_editing = false;
        self.decision_pinned = false;
        self.decision_replaces.clear();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn decision_replaces(&self) -> &[i64] {
        &self.decision_replaces
    }

    #[cfg(test)]
    pub(crate) fn decision_pinned(&self) -> bool {
        self.decision_pinned
    }

    fn review_button(
        id: &'static str,
        label: &'static str,
        weight: Button,
        cx: &mut Context<Self>,
        click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        answer(id, id.into(), label, weight)
            .on_click(cx.listener(move |this, _, _, cx| click(this, cx)))
            .into_any_element()
    }

    /// The card's text and answers for `task`, a [`Body::Decision`].
    pub(super) fn decision_card(
        &mut self,
        task: &TaskItem,
        body: &mut Vec<AnyElement>,
        buttons: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let Body::Decision {
            entry,
            flag,
            similar,
        } = &task.body
        else {
            return;
        };
        let (key, flag) = (task.key.as_str(), flag.as_ref());
        let id = entry.id;
        body.push(line("my-tasks-review-text", None, &entry.text));
        body.push(line("my-tasks-review-why", Some("Why: "), &entry.why));
        if let Some(said) = entry.user_answer.as_deref() {
            body.push(line("my-tasks-review-answer", Some("You said: "), said));
        }
        if let Some(flag) = flag {
            body.push(line(
                "my-tasks-review-flag",
                Some("An agent disagrees: "),
                &flag.reason,
            ));
        }
        if self.decision_editing {
            body.push(field("my-tasks-review-edit-title", &self.decision_title));
            body.push(field("my-tasks-review-edit-text", &self.decision_text));
            body.push(field("my-tasks-review-edit-why", &self.decision_why));
        }
        if let Some((_, error)) = self.review_error.as_ref().filter(|(k, _)| k == key) {
            body.push(
                div()
                    .debug_selector(|| "my-tasks-review-error".into())
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xff6b6b))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        let key = key.to_owned();
        if let Some(flag) = flag {
            let flag = flag.id;
            if self.decision_editing {
                let key = key.clone();
                buttons.push(Self::review_button(
                    "my-tasks-review-save-replacement",
                    "Save replacement",
                    Button::Primary,
                    cx,
                    move |this, cx| {
                        let new = this.edited(cx);
                        cx.emit(Event::DecisionReview {
                            key: key.clone(),
                            id,
                            review: Review::ReplaceFromFlag { flag, new },
                        })
                    },
                ));
                buttons.push(Self::review_button(
                    "my-tasks-review-cancel",
                    "Cancel",
                    Button::Ghost,
                    cx,
                    |this, cx| {
                        this.decision_editing = false;
                        cx.notify();
                    },
                ));
            } else {
                let shown = entry.clone();
                buttons.push(Self::review_button(
                    "my-tasks-review-replace",
                    "Replace with\u{2026}",
                    Button::Primary,
                    cx,
                    move |this, cx| this.start_editing(&shown, cx),
                ));
                buttons.push(Self::review_button(
                    "my-tasks-review-dismiss",
                    "Dismiss",
                    Button::Ghost,
                    cx,
                    move |_, cx| {
                        cx.emit(Event::DecisionReview {
                            key: key.clone(),
                            id,
                            review: Review::DismissFlag { flag },
                        })
                    },
                ));
            }
            return;
        }
        if !similar.is_empty() {
            body.push(self.replaces(similar, cx));
        }
        // What the card shows now: an agent's later revision is refused.
        let shown = shown_hash(entry);
        {
            let (key, shown) = (key.clone(), shown.clone());
            buttons.push(Self::review_button(
                "my-tasks-review-approve",
                "Approve",
                Button::Primary,
                cx,
                move |this, cx| {
                    let edit = this.decision_editing.then(|| this.edited(cx));
                    cx.emit(Event::DecisionReview {
                        key: key.clone(),
                        id,
                        review: Review::Approve {
                            shown: shown.clone(),
                            edit,
                            pinned: this.decision_pinned,
                            replaces: this.decision_replaces.clone(),
                        },
                    })
                },
            ));
        }
        let wording = entry.clone();
        buttons.push(Self::review_button(
            "my-tasks-review-edit",
            if self.decision_editing {
                "Keep wording"
            } else {
                "Edit"
            },
            Button::Glass,
            cx,
            move |this, cx| {
                if this.decision_editing {
                    this.decision_editing = false;
                    cx.notify();
                } else {
                    this.start_editing(&wording, cx);
                }
            },
        ));
        let pinned = self.decision_pinned;
        buttons.push(
            answer(
                "my-tasks-review-pin",
                "my-tasks-review-pin".into(),
                "Always include",
                Button::Glass,
            )
            .role(Role::CheckBox)
            .aria_toggled(toggled(pinned))
            .when(pinned, |d| d.child(icon("check", 12.)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.decision_pinned = !this.decision_pinned;
                cx.notify();
            }))
            .into_any_element(),
        );
        buttons.push(Self::review_button(
            "my-tasks-review-reject",
            "Reject",
            Button::Ghost,
            cx,
            move |_, cx| {
                cx.emit(Event::DecisionReview {
                    key: key.clone(),
                    id,
                    review: Review::Reject {
                        shown: shown.clone(),
                    },
                })
            },
        ));
    }

    /// "Replaces:" and a checkbox per similar active entry, unticked until
    /// the user ticks it.
    fn replaces(&self, similar: &[Entry], cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        for entry in similar {
            let old = entry.id;
            let ticked = self.decision_replaces.contains(&old);
            let selector = format!("my-tasks-review-replaces-{old}");
            rows.push(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(Role::CheckBox)
                    .aria_toggled(toggled(ticked))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .child(
                        div()
                            .size(px(14.))
                            .flex_shrink_0()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(glass::edge())
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(ticked, |d| d.child(icon("check", 10.))),
                    )
                    .child(format!("{} {}", entry.label(), entry.title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(at) = this.decision_replaces.iter().position(|id| *id == old) {
                            this.decision_replaces.remove(at);
                        } else {
                            this.decision_replaces.push(old);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        div()
            .debug_selector(|| "my-tasks-review-replaces".into())
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child("Replaces:"),
            )
            .children(rows)
            .into_any_element()
    }
}
