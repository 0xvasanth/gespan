//! One board card, laid out like gr8r's: label chips, the title, subtask
//! progress, then a quiet meta row (number, priority, due date, comments, link,
//! last change). Cards drag between steps; "…" and Enter open Move to….
use super::{
    BoardView, DraggedCard,
    link_status::{ChecksState, LinkChip, LinkState},
    quick::Face,
    style::{due_label, label_chip, priority_glyph},
};
use crate::{popover::Anchor, theme::*};
use gespan_core::board::Card;
use gpui::{
    AnyElement, Context, MouseButton, MouseDownEvent, Role, SharedString, Stateful, div,
    prelude::*, px,
};

/// A control on the card face that opens a quick-edit menu. Like "…", it stops
/// the press and the click, so it never starts a drag or opens the card.
fn face_control(
    control: Stateful<gpui::Div>,
    card_id: &str,
    face: Face,
    cx: &mut Context<BoardView>,
) -> Stateful<gpui::Div> {
    let card_id = card_id.to_string();
    control
        .role(Role::Button)
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.open_face_menu(card_id.clone(), face, window, cx)
        }))
}

impl BoardView {
    pub(super) fn render_card(
        &self,
        card: &Card,
        done: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        self.rendered_cards.set(self.rendered_cards.get() + 1);
        let number = card.number;
        let display = &self.display;
        let face = &display.face;
        let compact = display.density == gespan_core::board::extras::Density::Compact;
        let extras = self.extras.get(&card.id).cloned().unwrap_or_default();
        let now = gespan_core::time::now_ms();
        let labels: Vec<_> = extras
            .labels
            .iter()
            .filter_map(|id| self.labels.iter().find(|label| &label.id == id))
            .collect();
        let due = extras
            .due_at
            .filter(|_| face.due)
            .and_then(|at| due_label(at, now as i64))
            .map(|(text, overdue)| {
                let control = div()
                    .id(("board-card-due", number as u64))
                    .debug_selector(move || format!("board-card-due-{number}"))
                    .aria_label("Change due date")
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(content(0.08)))
                    .when(overdue && !done, |d| d.text_color(diff_foreground(false)))
                    .child(icon("calendar-03", 11.))
                    .child(text);
                face_control(control, &card.id, Face::Due, cx)
            });
        let updated = gespan_core::time::relative(card.updated_at.max(0) as u64, now);
        let anchor = self.card_anchors.get(&card.id).cloned().unwrap_or_default();
        let lifted = self.dragged_card.as_ref() == Some(&card.id);
        let held = self
            .kb_drag
            .as_ref()
            .is_some_and(|drag| drag.card_id == card.id);
        let group = format!("board-card-group-{number}");
        let payload = DraggedCard {
            card_id: card.id.clone(),
            number,
            title: card.title.clone(),
        };
        let weak = cx.weak_entity();
        let menu_id = card.id.clone();
        div()
            .id(("board-card", number as u64))
            .debug_selector(move || format!("board-card-{number}"))
            .group(group.clone())
            .relative()
            .role(Role::ListItem)
            .aria_label(card.title.clone())
            .flex_shrink_0()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .px(px(12.))
            .py(px(if compact { 5. } else { 10. }))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.07))
            .bg(pane_fill())
            .hover(|s| s.border_color(content(0.14)))
            .when(lifted, |d| d.opacity(0.35))
            .when(held, |d| d.border_color(accent_color()).bg(content(0.08)))
            .when(self.is_selected(&card.id), |d| {
                d.border_color(accent_color())
                    .bg(accent_color().opacity(0.14))
            })
            .flex()
            .flex_col()
            .gap(px(if compact { 2. } else { 6. }))
            .cursor_grab()
            .on_drag(payload, move |payload, _, _, cx| {
                let card_id = payload.card_id.clone();
                let _ = weak.update(cx, |this, cx| {
                    this.begin_card_drag(card_id);
                    cx.notify();
                });
                cx.new(|_| payload.clone())
            })
            .when_some(self.card_focus.get(&card.id), |d, focus| {
                d.track_focus(focus).tab_index(0)
            })
            .focus_visible(|s| s.border_color(accent_color()))
            .on_key_down(cx.listener({
                let id = card.id.clone();
                move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if this.card_key(&id, event, window, cx) {
                        cx.stop_propagation();
                    }
                }
            }))
            .on_click(cx.listener({
                let id = card.id.clone();
                move |this, event, window, cx| this.card_click(&id, event, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let id = card.id.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_more_menu(id.clone(), Anchor::Point(event.position), window, cx)
                    }
                }),
            )
            .child(anchor.probe())
            .when(face.labels && !labels.is_empty(), |d| {
                let row = div()
                    .id(("board-card-labels", number as u64))
                    .debug_selector(move || format!("board-card-labels-{number}"))
                    .aria_label("Change labels")
                    .mr(px(20.))
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .children(
                        labels
                            .iter()
                            .map(|label| label_chip(&label.name, &label.color)),
                    );
                d.child(
                    div()
                        .flex()
                        .child(face_control(row, &card.id, Face::Labels, cx)),
                )
            })
            .child(
                div()
                    .pr(px(20.))
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .when(done, |d| {
                        d.child(
                            div()
                                .pt(px(2.))
                                .child(icon("check-circle", 13.).text_color(diff_foreground(true))),
                        )
                    })
                    .children(self.blocked_lock(&card.id, number))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .line_clamp(2)
                            .text_ellipsis()
                            .text_color(content(if done { 0.6 } else { 0.92 }))
                            .child(card.title.clone()),
                    ),
            )
            .when(face.subtasks && extras.subtasks_total > 0, |d| {
                let share = extras.subtasks_done as f32 / extras.subtasks_total as f32;
                d.child(
                    div()
                        .id(("board-card-subtasks", number as u64))
                        .debug_selector(move || format!("board-card-subtasks-{number}"))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(icon("check-list", 11.))
                        .child(format!(
                            "{}/{}",
                            extras.subtasks_done, extras.subtasks_total
                        ))
                        .child(
                            div()
                                .flex_1()
                                .h(px(3.))
                                .rounded_full()
                                .bg(content(0.08))
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(share))
                                        .rounded_full()
                                        .bg(accent_color()),
                                ),
                        ),
                )
            })
            .children(self.field_chips(card, number))
            .children(self.agent_field_chips(&card.id, number))
            .children(self.link_chip_row(&card.id, number))
            .when_some(
                self.ticket_chip(format!("card-{number}"), &card.id, false, true, cx),
                |d, chip| d.child(div().flex().min_w_0().child(chip)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .when(face.number, |d| {
                        d.child(div().font_family("Menlo").child(format!("#{number}")))
                    })
                    .when(face.priority && extras.priority > 0, |d| {
                        let glyph = div()
                            .id(("board-card-priority", number as u64))
                            .debug_selector(move || format!("board-card-priority-{number}"))
                            .aria_label("Change priority")
                            .aria_description(self.priority_name(extras.priority))
                            .px(px(2.))
                            .rounded(px(4.))
                            .hover(|s| s.bg(content(0.08)))
                            .child(priority_glyph(extras.priority));
                        d.child(face_control(glyph, &card.id, Face::Priority, cx))
                    })
                    // A faint target, shown on hover, for a card with no priority.
                    .when(face.priority && extras.priority == 0, |d| {
                        let target = div()
                            .id(("board-card-priority-target", number as u64))
                            .debug_selector(move || format!("board-card-priority-target-{number}"))
                            .aria_label("Set priority")
                            .px(px(2.))
                            .rounded(px(4.))
                            .opacity(0.)
                            .group_hover(group.clone(), |s| s.opacity(0.6))
                            .hover(|s| s.bg(content(0.08)))
                            .child(priority_glyph(0));
                        d.child(face_control(target, &card.id, Face::Priority, cx))
                    })
                    .children(due)
                    .when(face.comments && extras.comments > 0, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(3.))
                                .child(icon("message-square", 11.))
                                .child(extras.comments.to_string()),
                        )
                    })
                    .child(div().flex_1())
                    .child(div().text_color(content(0.35)).child(updated)),
            )
            .child(
                div()
                    .id(("board-card-menu", number as u64))
                    .debug_selector(move || format!("board-card-menu-{number}"))
                    .absolute()
                    .top(px(6.))
                    .right(px(6.))
                    .size(px(22.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(Role::Button)
                    .aria_label("Card options")
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .hover(|s| s.bg(content(0.08)))
                    .cursor_pointer()
                    .child(icon("more-horizontal", 12.).text_color(content(0.6)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_card_actions(menu_id.clone(), window, cx)
                    })),
            )
            .into_any_element()
    }
}

impl BoardView {
    /// One chip per custom field the face shows and the card has a value for.
    /// A field deleted since is skipped without a word.
    fn field_chips(&self, card: &Card, number: i64) -> Vec<AnyElement> {
        let Some(row) = self.index.get(&card.id) else {
            return Vec::new();
        };
        self.display
            .face
            .fields
            .iter()
            .enumerate()
            .filter_map(|(at, id)| {
                let field = self.fields.iter().find(|field| &field.id == id)?;
                let text = super::quick::field_text(&field.kind, row.fields.get(id)?)?;
                Some(
                    div()
                        .id(("board-card-field", number as u64 * 100 + at as u64))
                        .debug_selector(move || format!("board-card-field-{number}-{at}"))
                        .flex()
                        .gap(px(4.))
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .child(div().text_color(content(0.4)).child(field.name.clone()))
                        .child(div().min_w_0().truncate().child(text))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

/// How a chip's state reads and colours.
fn state_color(state: LinkState) -> gpui::Hsla {
    match state {
        LinkState::Running | LinkState::Open => accent_color(),
        LinkState::Waiting => gpui::hsla(42. / 360., 0.78, 0.55, 1.),
        LinkState::Failed | LinkState::Closed => diff_foreground(false),
        LinkState::Completed | LinkState::Merged => diff_foreground(true),
        LinkState::Idle | LinkState::Unknown | LinkState::Missing => content(0.4),
    }
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "pilot" => "bot",
        "pr" => "git-pull-request",
        "worktree" => "git-branch",
        _ => "chatting",
    }
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "pilot" => "Pilot",
        "pr" => "Pull request",
        "worktree" => "Worktree",
        _ => "Session",
    }
}

impl BoardView {
    /// A lock on a card some live, unfinished card blocks.
    fn blocked_lock(&self, card_id: &str, number: i64) -> Option<AnyElement> {
        let blockers = self.blocked.get(card_id).filter(|b| !b.is_empty())?;
        let text = format!(
            "Blocked by {}",
            blockers
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        Some(
            div()
                .id(("board-card-blocked", number as u64))
                .debug_selector(move || format!("board-card-blocked-{number}"))
                .role(Role::Image)
                .aria_label(text)
                .pt(px(2.))
                .child(icon("square-lock", 13.).text_color(diff_foreground(false)))
                .into_any_element(),
        )
    }

    /// One chip per link, when the face shows link status.
    fn link_chip_row(&self, card_id: &str, number: i64) -> Option<AnyElement> {
        let chips = self.chips.get(card_id).filter(|chips| !chips.is_empty())?;
        if !self.display.face.link_status {
            return None;
        }
        Some(
            div()
                .id(("board-card-links", number as u64))
                .debug_selector(move || format!("board-card-links-{number}"))
                .role(Role::List)
                .aria_label("Linked work")
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .children(
                    chips
                        .iter()
                        .enumerate()
                        .map(|(at, chip)| link_chip(chip, number, at)),
                )
                .into_any_element(),
        )
    }

    /// The agent fields the face shows that have a value. Cost has no
    /// source yet, so it never shows.
    fn agent_field_chips(&self, card_id: &str, number: i64) -> Vec<AnyElement> {
        let Some(values) = self.agent.get(card_id) else {
            return Vec::new();
        };
        self.display
            .face
            .agent_fields
            .iter()
            .filter(|field| field.as_str() != "cost")
            .filter_map(|field| {
                let value = values.get(field)?.to_string();
                let key = field.clone();
                Some(
                    div()
                        .id(SharedString::from(format!(
                            "board-card-agent-{field}-{number}"
                        )))
                        .debug_selector(move || format!("board-card-agent-{key}-{number}"))
                        .flex()
                        .gap(px(4.))
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .child(
                            div()
                                .text_color(content(0.4))
                                .child(super::agent_fields::label(field)),
                        )
                        .child(div().min_w_0().truncate().child(value))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

fn link_chip(chip: &LinkChip, number: i64, at: usize) -> AnyElement {
    let text = match chip.state {
        LinkState::Missing => "Linked work no longer exists".to_string(),
        _ => chip.label.clone(),
    };
    let checks = chip.checks.map(|checks| {
        let (glyph, ok) = match checks {
            ChecksState::Passing => ("✓", Some(true)),
            ChecksState::Failing => ("✕", Some(false)),
            ChecksState::Pending => ("•", None),
        };
        div()
            .text_color(ok.map_or(content(0.5), diff_foreground))
            .child(glyph)
    });
    let diff = chip.diff.map(|(added, deleted)| {
        div()
            .flex()
            .gap(px(3.))
            .child(
                div()
                    .text_color(diff_foreground(true))
                    .child(format!("+{added}")),
            )
            .child(
                div()
                    .text_color(diff_foreground(false))
                    .child(format!("−{deleted}")),
            )
    });
    let state = chip.state;
    div()
        .id(SharedString::from(format!("board-card-link-{number}-{at}")))
        .debug_selector(move || format!("board-card-link-{number}-{at}"))
        .role(Role::ListItem)
        .aria_label(format!("{} {}", kind_name(&chip.kind), text))
        .aria_description(chip.status_text())
        .max_w(px(200.))
        .min_w_0()
        .h(px(18.))
        .px(px(6.))
        .rounded(px(4.))
        .bg(content(0.05))
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.))
        .text_color(content(if state == LinkState::Missing {
            0.4
        } else {
            0.65
        }))
        .child(icon(kind_icon(&chip.kind), 11.))
        .child(div().min_w_0().truncate().child(text))
        .when(
            state != LinkState::Missing && chip.kind != "worktree",
            |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_color(state_color(state))
                        .child(state.text()),
                )
            },
        )
        .children(checks)
        .children(diff)
        .into_any_element()
}
