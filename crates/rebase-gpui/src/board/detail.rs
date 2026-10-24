//! A card's detail panel: title and notes, its link, its activity and delete.
use super::{BoardView, Event, LinkLabel, menu_row, turns};
use crate::{
    input::{self, Input},
    runtime::locked,
    theme::*,
};
use gpui::{
    AnyElement, Context, Entity, Focusable, FontWeight, Role, SharedString, Subscription, Window,
    div, prelude::*, px,
};
use rebase_core::board::Activity;

const ACTIVITY_LIMIT: usize = 50;

pub(crate) struct CardDetail {
    pub card_id: String,
    title: Entity<Input>,
    body: Entity<Input>,
    pub activity: Option<Result<Vec<Activity>, String>>,
    picker: bool,
    _subscriptions: Vec<Subscription>,
}

/// One activity row as the timeline reads it, before its relative time.
pub(crate) fn activity_text(row: &Activity) -> String {
    let what = match row.kind.as_str() {
        "moved" => format!(
            "Moved from {} to {}",
            row.from_column.as_deref().unwrap_or("?"),
            row.to_column.as_deref().unwrap_or("?")
        ),
        kind => {
            let mut chars = kind.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    };
    format!("{what} · {}", row.actor)
}

/// Something a card can link to: link kind, link id, what to call it.
struct LinkTarget {
    kind: &'static str,
    id: String,
    label: String,
}

impl BoardView {
    pub(crate) fn open_card_detail(
        &mut self,
        card_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dragging {
            return;
        }
        self.close_card_detail(cx);
        let Some(card) = self.card(&card_id).cloned() else {
            return;
        };
        let title = cx.new(|cx| Input::field("Card title", "Card title", content(0.4), cx));
        title.update(cx, |input, cx| input.set_text(card.title.clone(), cx));
        let body = cx.new(|cx| {
            let mut input = Input::commit_message(cx);
            input.set_placeholder("Notes", cx);
            input
        });
        body.update(cx, |input, cx| input.set_text(card.body.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&title, |this, _, _: &input::Submitted, cx| {
                this.save_card_details(cx)
            }),
            cx.on_blur(&title.focus_handle(cx), window, |this, _, cx| {
                this.save_card_details(cx)
            }),
            cx.on_blur(&body.focus_handle(cx), window, |this, _, cx| {
                this.save_card_details(cx)
            }),
        ];
        let activity = self.load_activity(&card.board_id, &card_id);
        self.card_detail = Some(CardDetail {
            card_id,
            title,
            body,
            activity: Some(activity),
            picker: false,
            _subscriptions: subscriptions,
        });
        self.menu = None;
        cx.notify();
    }

    /// Close the panel, keeping its edits. True when one was open.
    pub(crate) fn close_card_detail(&mut self, cx: &mut Context<Self>) -> bool {
        if self.card_detail.is_none() {
            return false;
        }
        self.save_card_details(cx);
        self.card_detail = None;
        cx.notify();
        true
    }

    fn card(&self, card_id: &str) -> Option<&rebase_core::board::Card> {
        self.detail
            .as_ref()?
            .cards
            .iter()
            .find(|card| card.id == card_id)
    }

    fn load_activity(&mut self, board_id: &str, card_id: &str) -> Result<Vec<Activity>, String> {
        self.service()?
            .activity(board_id, Some(card_id), ACTIVITY_LIMIT)
    }

    fn save_card_details(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.card_detail else {
            return;
        };
        let title = detail.title.read(cx).content.trim().to_string();
        let body = detail.body.read(cx).content.to_string();
        let card_id = detail.card_id.clone();
        let Some(card) = self.card(&card_id) else {
            return;
        };
        if title.is_empty() || (title == card.title && body == card.body) {
            return;
        }
        self.write(cx, |service| service.card_update(&card_id, &title, &body));
        self.refresh_activity();
    }

    fn refresh_activity(&mut self) {
        let Some(card_id) = self.card_detail.as_ref().map(|d| d.card_id.clone()) else {
            return;
        };
        let Some(board_id) = self.card(&card_id).map(|card| card.board_id.clone()) else {
            return;
        };
        let activity = self.load_activity(&board_id, &card_id);
        if let Some(detail) = &mut self.card_detail {
            detail.activity = Some(activity);
        }
    }

    fn link_targets(&self, cx: &Context<Self>) -> Vec<LinkTarget> {
        let mut targets = Vec::new();
        if let Ok(store) = locked(&self.store) {
            let mut sessions = store.sessions().unwrap_or_default();
            sessions.retain(|session| !session.archived);
            sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
            for session in sessions.into_iter().take(50) {
                if let Ok(id) = store.persisted_session_id(session.id) {
                    let label = if session.title.trim().is_empty() {
                        "Untitled session".to_string()
                    } else {
                        session.title
                    };
                    targets.push(LinkTarget {
                        kind: "session",
                        id,
                        label,
                    });
                }
            }
        }
        targets.extend(turns::pilots(cx).into_iter().map(|(id, name)| LinkTarget {
            kind: "pilot",
            id,
            label: name,
        }));
        targets
    }

    fn link_selected(
        &mut self,
        kind: Option<&'static str>,
        id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(card_id) = self.card_detail.as_ref().map(|d| d.card_id.clone()) else {
            return;
        };
        if let Some(detail) = &mut self.card_detail {
            detail.picker = false;
        }
        self.write(cx, |service| {
            service.card_link(&card_id, kind, id.as_deref())
        });
        self.refresh_activity();
    }

    fn open_linked(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self
            .card_detail
            .as_ref()
            .and_then(|detail| self.card(&detail.card_id))
            .cloned()
        else {
            return;
        };
        match (card.link_kind.as_deref(), card.link_id) {
            (Some("session"), Some(id)) => {
                let native = locked(&self.store)
                    .ok()
                    .and_then(|store| store.resolve_existing_session_id(&id));
                match native {
                    Some(native) => cx.emit(Event::OpenSession(native)),
                    None => {
                        self.error = Some("That session no longer exists".into());
                        cx.notify();
                    }
                }
            }
            (Some("pilot"), Some(id)) => cx.emit(Event::OpenPilot(id)),
            _ => {}
        }
    }

    fn delete_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self
            .card_detail
            .as_ref()
            .and_then(|detail| self.card(&detail.card_id))
            .cloned()
        else {
            return;
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Delete “{}”?", card.title),
            Some("Its notes and activity are removed."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    this.card_detail = None;
                    this.write(cx, |service| service.card_delete(&card.id));
                });
            }
        })
        .detach();
    }

    pub(super) fn render_card_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.card_detail.as_ref()?;
        let card = self.card(&detail.card_id)?.clone();
        let label = self.link_label(&card.id);
        let mut panel = div()
            .id("board-card-detail")
            .debug_selector(|| "board-card-detail".into())
            .role(Role::Dialog)
            .aria_label(format!("Card #{}", card.number))
            .occlude()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(420.))
            .bg(pane_fill())
            .border_l_1()
            .border_color(content(0.1))
            .shadow_lg()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(PANE_HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(content(0.07))
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(format!("#{}", card.number)),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button("board-card-detail-close", "Close", "close")
                            .debug_selector(|| "board-card-detail-close".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_card_detail(cx);
                            })),
                    ),
            );
        let mut body = div()
            .id("board-card-detail-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .child(section("Title").child(detail.title.clone()))
            .child(
                section("Notes").child(
                    div()
                        .min_h(px(80.))
                        .p(px(8.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(content(0.1))
                        .child(detail.body.clone()),
                ),
            );
        let mut link = section("Linked work");
        link = match (&card.link_kind, label) {
            (Some(_), Some(LinkLabel::Found(name))) => link.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .child(name),
                    )
                    .child(
                        text_button("board-open-linked", "Open linked work")
                            .on_click(cx.listener(|this, _, _, cx| this.open_linked(cx))),
                    ),
            ),
            (Some(_), _) => link.child(
                div()
                    .id("board-link-missing")
                    .debug_selector(|| "board-link-missing".into())
                    .text_size(px(13.))
                    .text_color(content(0.45))
                    .child("Linked work no longer exists"),
            ),
            (None, _) => link.child(
                div()
                    .text_size(px(13.))
                    .text_color(content(0.45))
                    .child("Not linked"),
            ),
        };
        link =
            link.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        text_button(
                            "board-link-picker",
                            if card.link_kind.is_some() {
                                "Change link"
                            } else {
                                "Link…"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(detail) = &mut this.card_detail {
                                detail.picker = !detail.picker;
                            }
                            cx.notify();
                        })),
                    )
                    .when(card.link_kind.is_some(), |d| {
                        d.child(text_button("board-unlink", "Unlink").on_click(
                            cx.listener(|this, _, _, cx| this.link_selected(None, None, cx)),
                        ))
                    }),
            );
        if detail.picker {
            let targets = self.link_targets(cx);
            link = link.child(
                div()
                    .id("board-link-targets")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(content(0.1))
                    .when(targets.is_empty(), |d| {
                        d.child(
                            div()
                                .p_2()
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child("No sessions or Pilots to link"),
                        )
                    })
                    .children(targets.into_iter().enumerate().map(|(index, target)| {
                        let glyph = if target.kind == "pilot" {
                            "bot"
                        } else {
                            "chatting"
                        };
                        let (kind, id) = (target.kind, target.id.clone());
                        menu_row(format!("board-link-target-{index}").into(), target.label)
                            .flex()
                            .gap_2()
                            .child(icon(glyph, 12.).text_color(content(0.5)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.link_selected(Some(kind), Some(id.clone()), cx)
                            }))
                    })),
            );
        }
        body = body.child(link);
        let activity = match &detail.activity {
            Some(Ok(rows)) if rows.is_empty() => section("Activity").child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child("No activity yet"),
            ),
            Some(Ok(rows)) => {
                let now = rebase_core::time::now_ms();
                section("Activity").children(rows.iter().map(|row| {
                    let when = rebase_core::time::relative(row.created_at.max(0) as u64, now);
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.7))
                        .child(format!("{} · {when}", activity_text(row)))
                }))
            }
            Some(Err(_)) => section("Activity").child(
                div()
                    .id("board-activity-failed")
                    .debug_selector(|| "board-activity-failed".into())
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child("Couldn't load activity"),
            ),
            None => section("Activity"),
        };
        body = body.child(activity).child(
            text_button("board-card-delete", "Delete card")
                .text_color(diff_foreground(false))
                .on_click(cx.listener(|this, _, window, cx| this.delete_card(window, cx))),
        );
        panel = panel.child(body);
        Some(panel.into_any_element())
    }
}

fn section(label: &'static str) -> gpui::Div {
    div().flex().flex_col().gap_1().child(
        div()
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(content(0.45))
            .child(label),
    )
}

fn text_button(id: &'static str, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .role(Role::Button)
        .aria_label(label.clone())
        .px_2()
        .py_1()
        .rounded_md()
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.06)))
        .child(label)
}
