//! InboxMiniCard.tsx and NoteMiniCard.tsx: what a new message starts from,
//! shown above the composer until it is sent or removed.
use super::*;
use gespan_core::{inbox::Item, notes::Note};

#[derive(Clone, Debug)]
pub(crate) enum ComposerCard {
    Inbox { item: Box<Item>, prompt: String },
    Note(Box<Note>),
}

impl ComposerCard {
    fn placeholder(&self) -> &'static str {
        match self {
            Self::Inbox { .. } => "Add a note, or send to start…",
            Self::Note(_) => "Add a message, or send…",
        }
    }
    /// The Inbox item's prompt with the typed note after it (composeInboxMessage).
    fn inbox_message(prompt: &str, text: &str) -> String {
        match (prompt.trim(), text.trim()) {
            (prompt, "") => prompt.to_owned(),
            ("", note) => note.to_owned(),
            (prompt, note) => format!("{prompt}\n\n{note}"),
        }
    }
}

fn kind_label(item: &Item) -> &'static str {
    match (item.kind.as_str(), item.provider.as_str()) {
        ("pr", "gitlab") => "Merge request",
        ("pr", _) => "Pull request",
        _ => "Issue",
    }
}

fn provider_label(item: &Item) -> &'static str {
    match item.provider.as_str() {
        "linear" => "Linear",
        "jira" => "Jira",
        "gitlab" => "GitLab",
        "azuredevops" => "ADO",
        _ => "GitHub",
    }
}

impl Conversation {
    pub(crate) fn set_composer_card(&mut self, card: ComposerCard, cx: &mut Context<Self>) {
        let hint = card.placeholder();
        self.input.update(cx, |input, cx| {
            if self.composer_card.is_none() {
                self.composer_placeholder = input.placeholder().clone();
            }
            input.set_placeholder(hint, cx);
        });
        self.composer_card = Some(card);
        cx.emit(SessionChanged);
        cx.notify();
    }
    pub(super) fn clear_composer_card(&mut self, cx: &mut Context<Self>) {
        if self.composer_card.take().is_some() {
            let hint = self.composer_placeholder.clone();
            self.input
                .update(cx, |input, cx| input.set_placeholder(hint, cx));
            cx.notify();
        }
    }
    /// Turn the typed text into the message. An Inbox card becomes the message
    /// itself; a note stays out of it, as a card beside the prompt.
    pub(super) fn apply_composer_card(&mut self, text: String, cx: &mut Context<Self>) -> String {
        let card = self.composer_card.clone();
        self.clear_composer_card(cx);
        match card {
            None => text,
            Some(ComposerCard::Inbox { prompt, .. }) => ComposerCard::inbox_message(&prompt, &text),
            Some(ComposerCard::Note(note)) => {
                let typed = text.trim().to_owned();
                if self.is_remote() {
                    return gespan_core::notes::compose_message(&note, &typed);
                }
                // A message that waits in the queue goes through @note/, which
                // is resolved when it is finally sent.
                if self.busy
                    || self.stopping
                    || self.settings.pending_provider_switch.is_some()
                    || crate::hold::active(cx)
                    || !self.follow_ups.state.messages.is_empty()
                {
                    return format!("{typed} @note/{}", note.slug).trim().to_owned();
                }
                self.pending_note = Some(note);
                if typed.is_empty() {
                    "Use this note.".to_owned()
                } else {
                    typed
                }
            }
        }
    }
    pub(super) fn composer_card_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.composer_card.as_ref()?;
        let (top, title, source, labels, url, name) = match card {
            ComposerCard::Inbox { item, .. } => {
                let reference = item.reference();
                let kind = kind_label(item);
                let source = if matches!(item.provider.as_str(), "linear" | "jira")
                    && !item.team_name.is_empty()
                {
                    item.team_name.clone()
                } else {
                    item.repo.clone()
                };
                (
                    format!("{kind} · {reference}"),
                    if item.title.trim().is_empty() {
                        reference.clone()
                    } else {
                        item.title.trim().to_owned()
                    },
                    source,
                    item.labels
                        .iter()
                        .take(2)
                        .map(|label| label.name.clone())
                        .collect::<Vec<_>>(),
                    Some(item.url.trim().to_owned()).filter(|url| !url.is_empty()),
                    format!("{kind} {reference}"),
                )
            }
            ComposerCard::Note(note) => {
                let title = if note.title.trim().is_empty() {
                    "Untitled".to_owned()
                } else {
                    note.title.clone()
                };
                (
                    if note.slug.is_empty() {
                        "Note".to_owned()
                    } else {
                        format!("Note · {}", note.slug)
                    },
                    title.clone(),
                    note.project_name().unwrap_or_default(),
                    Vec::new(),
                    None,
                    format!("note {title}"),
                )
            }
        };
        let tracker = match card {
            ComposerCard::Inbox { item, .. } => Some(provider_label(item)),
            ComposerCard::Note(_) => None,
        };
        let glyph = match card {
            ComposerCard::Inbox { item, .. } if item.kind == "pr" => "git-pull-request",
            ComposerCard::Inbox { .. } => "circle-dot",
            ComposerCard::Note(_) => "note",
        };
        let detail = !source.is_empty() || !labels.is_empty();
        Some(
            div()
                .id("composer-card")
                .debug_selector(|| "composer-card".into())
                .role(Role::Group)
                .aria_label(top.clone())
                .relative()
                .mx_3()
                .mt_2()
                .px(px(10.))
                .py_2()
                .pr_8()
                .rounded_md()
                .border_1()
                .border_color(content(0.1))
                .bg(content(0.06))
                .when_some(url, |d, url| {
                    let label = tracker.unwrap_or("GitHub");
                    d.cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .tooltip(move |_, cx| {
                            cx.new(|_| {
                                HintTooltip(if label == "GitHub" {
                                    "Open in GitHub"
                                } else {
                                    "Open in tracker"
                                })
                            })
                            .into()
                        })
                })
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(glyph, 12.).text_color(content(0.45)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(content(0.5))
                                .child(top),
                        ),
                )
                .child(
                    div()
                        .mt_1()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(content(1.))
                        .child(title),
                )
                .when(detail, |d| {
                    d.child(
                        div()
                            .mt_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(div().min_w_0().flex_1().truncate().child(source))
                            .children(labels.into_iter().map(|label| {
                                div()
                                    .flex_shrink_0()
                                    .px_1()
                                    .rounded(px(4.))
                                    .bg(content(0.08))
                                    .child(label)
                            })),
                    )
                })
                .child(
                    div()
                        .id("composer-card-remove")
                        .debug_selector(|| "composer-card-remove".into())
                        .role(Role::Button)
                        .aria_label(format!("Remove {name}"))
                        .absolute()
                        .right(px(6.))
                        .top(px(6.))
                        .size(px(20.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_color(content(0.4))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon("close", 12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.clear_composer_card(cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
