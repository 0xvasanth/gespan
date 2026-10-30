//! How the task drawer looks: gr8r's top bar, property grid, description,
//! subtasks and Comments/Activity tabs, in the board's quiet type scale.
use super::super::header::checked_row;
use super::super::markdown_toolbar::Format;
use super::{CardDetail, Choice, Picker, RelationKind, Tab, activity_text};
use crate::{
    board::{
        BoardView, LinkLabel,
        column::role_icon,
        menu_row, quick,
        style::{due_label, label_chip, priority_glyph},
    },
    popover::{Anchor, menu_popover},
    theme::*,
};
use gespan_core::board::{Card, extras::CardLink};
use gpui::{
    AnyElement, Context, Focusable, FontWeight, MouseButton, Role, SharedString, div, prelude::*,
    px,
};

const DRAWER_WIDTH: f32 = 440.;
/// The drawer body's side padding; only used before the first layout.
const DRAWER_PADDING: f32 = 20.;
/// Pickers open under the value column, past the property labels.
const PICKER_INDENT: f32 = 126.;
/// The comment field sends on the platform's secondary submit chord.
pub(crate) const SEND_HINT: &str = if cfg!(target_os = "macos") {
    "⌘Enter"
} else {
    "Ctrl+Enter"
};

/// Who wrote a comment: the name, the avatar's initial, and whether it is an
/// agent. Anything but `user` was written by an agent.
pub(crate) fn comment_author(author: &str) -> (&'static str, &'static str, bool) {
    if author == "user" {
        ("You", "Y", false)
    } else {
        ("Agent", "A", true)
    }
}

fn section(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(content(0.85))
        .child(label.into())
}

fn empty() -> AnyElement {
    div()
        .text_color(content(0.35))
        .child("Empty")
        .into_any_element()
}

fn quiet_button(id: &'static str, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(26.))
        .px_2()
        .rounded_md()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.7))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.06)).text_color(content(0.95)))
        .child(label)
}

impl BoardView {
    pub(in crate::board) fn render_card_detail(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let detail = self.card_detail.as_ref()?;
        let card = self.card(&detail.card_id)?.clone();
        let done = self.task_done(&card);
        let anchor = |key: &str| detail.anchors.get(key).cloned().unwrap_or_default();
        let choice = detail.choice.as_ref().map(|(which, menu, _)| {
            let key = match which {
                Choice::Status => "status".to_string(),
                Choice::Priority => "priority".to_string(),
                Choice::Due => "due".to_string(),
                Choice::Start => "start".to_string(),
                Choice::Subtask(index) => format!("subtask-{index}"),
                Choice::FieldDate(index) | Choice::FieldSelect(index) => format!("field-{index}"),
                Choice::LinkAdd => "links-add".to_string(),
            };
            menu_popover(menu, Anchor::Slot(anchor(&key)), cx)
                .solid()
                .into_any_element()
        });
        let full_page = detail.full_page;
        let top_bar = div()
            .h(px(PANE_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(content(0.07))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(icon("kanban", 12.))
                    .child("Board")
                    .child("/")
                    .child(
                        div()
                            .font_family("Menlo")
                            .child(format!("#{}", card.number)),
                    ),
            )
            .child(div().flex_1())
            .when(self.run_with_shown(), |d| {
                let card_id = card.id.clone();
                d.child(
                    // An icon, so the top bar's other actions keep their room.
                    icon_button("board-drawer-run-with", "Run with…", "play")
                        .debug_selector(|| "board-drawer-run-with".into())
                        .tab_index(0)
                        .focus_visible(|s| s.bg(content(0.06)))
                        .on_key_down(cx.listener({
                            let card_id = card_id.clone();
                            move |this, event: &gpui::KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    this.run_with(&card_id, window, cx);
                                }
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_with(&card_id, window, cx)
                        })),
                )
            })
            .child(
                quiet_button(
                    "board-drawer-full-page",
                    if full_page { "Drawer" } else { "Full page" },
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_full_page(cx))),
            )
            .child(
                quiet_button(
                    "board-drawer-complete",
                    if done { "Completed" } else { "Mark complete" },
                )
                .when(done, |d| d.text_color(diff_foreground(true)))
                .child(icon(if done { "check-circle" } else { "circle-dot" }, 13.))
                .on_click(cx.listener(|this, _, _, cx| this.mark_complete(cx))),
            )
            .child(
                icon_button("board-drawer-copy", "Copy reference", "link-01")
                    .debug_selector(|| "board-drawer-copy".into())
                    .on_click(cx.listener(|this, _, _, cx| this.copy_reference(cx))),
            )
            .child(
                div()
                    .id("board-drawer-more")
                    .debug_selector(|| "board-drawer-more".into())
                    .relative()
                    .role(Role::Button)
                    .aria_label("More")
                    .size(px(26.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child(anchor("more").probe())
                    .child(icon("more-horizontal", 13.).text_color(content(0.6)))
                    .on_click({
                        let anchor = anchor("more");
                        let card_id = card.id.clone();
                        cx.listener(move |this, _, window, cx| {
                            this.open_more_menu(
                                card_id.clone(),
                                Anchor::Slot(anchor.clone()),
                                window,
                                cx,
                            )
                        })
                    }),
            )
            .child(
                icon_button("board-drawer-close", "Close", "close")
                    .debug_selector(|| "board-drawer-close".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_card_detail(cx);
                    })),
            );
        let body = div()
            .id("board-drawer-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .child(
                div()
                    .w_full()
                    .when(full_page, |d| d.max_w(px(760.)).mx_auto())
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .id("board-drawer-title")
                            .debug_selector(|| "board-drawer-title".into())
                            .child(detail.title.clone()),
                    )
                    .child(self.render_properties(detail, &card, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(section("Description"))
                            .child(self.render_markdown_toolbar(cx))
                            .child(
                                div()
                                    .id("board-drawer-description")
                                    .debug_selector(|| "board-drawer-description".into())
                                    .min_h(px(72.))
                                    .px(px(8.))
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .hover(|s| s.bg(content(0.03)))
                                    .cursor_text()
                                    .on_click({
                                        let focus = detail.body.focus_handle(cx);
                                        move |_, window, cx| window.focus(&focus, cx)
                                    })
                                    .child(detail.body.clone()),
                            ),
                    )
                    .child(self.render_links(detail, &card, cx))
                    .child(self.render_subtasks(detail, cx))
                    .child(self.render_tabs(detail, cx)),
            );
        Some(
            div()
                .id("board-drawer")
                .debug_selector(|| "board-drawer".into())
                .track_focus(&self.drawer_focus)
                .role(Role::Dialog)
                .aria_label(format!("Task #{}", card.number))
                .h_full()
                .flex()
                .flex_col()
                .bg(pane_fill())
                .when(full_page, |d| d.flex_1().min_h_0().min_w_0())
                .when(!full_page, |d| {
                    d.w(px(DRAWER_WIDTH))
                        .flex_shrink_0()
                        .border_l_1()
                        .border_color(content(0.07))
                })
                .child(top_bar)
                .child(body)
                .children(choice)
                .into_any_element(),
        )
    }

    fn render_properties(
        &self,
        detail: &CardDetail,
        card: &Card,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let extras = self.extras.get(&card.id).cloned().unwrap_or_default();
        let column = self
            .detail
            .as_ref()
            .and_then(|d| d.columns.iter().find(|c| c.id == card.column_id).cloned());
        let anchor = |key: &str| detail.anchors.get(key).cloned().unwrap_or_default();
        let row = |key: &str, glyph: &'static str, label: &str| {
            let selector = format!("board-drawer-{key}");
            let label = SharedString::from(label.to_string());
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .relative()
                .role(Role::Button)
                .aria_label(label.clone())
                .min_h(px(32.))
                .px(px(6.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(content(0.04)))
                .child(anchor(key).probe())
                .child(
                    div()
                        .w(px(120.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(content(0.5))
                        .child(icon(glyph, 13.))
                        .child(label),
                )
        };
        let value = |child: AnyElement| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .text_size(px(13.))
                .child(child)
        };
        let now = gespan_core::time::now_ms() as i64;
        let status = column.as_ref().map_or_else(empty, |column| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(role_icon(column.role.as_deref()))
                .child(column.name.clone())
                .into_any_element()
        });
        let priority = if extras.priority == 0 {
            empty()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(priority_glyph(extras.priority))
                .child(self.priority_name(extras.priority))
                .into_any_element()
        };
        let due = extras.due_at.and_then(|at| due_label(at, now)).map_or_else(
            empty,
            |(text, overdue)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(overdue, |d| d.text_color(diff_foreground(false)))
                    .child(icon("calendar-03", 12.))
                    .child(text)
                    .into_any_element()
            },
        );
        let start = detail
            .start_at
            .and_then(|at| due_label(at, now))
            .map_or_else(empty, |(text, _)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("calendar-03", 12.))
                    .child(text)
                    .into_any_element()
            });
        let estimate = detail.estimate.map_or_else(empty, |minutes| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(icon("clock", 12.))
                .child(quick::format_estimate(minutes))
                .into_any_element()
        });
        let projects = if extras.projects.is_empty() {
            empty()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .children(extras.projects.iter().map(|path| {
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(content(0.06))
                        .text_size(px(12.))
                        .child(self.project_name(path))
                }))
                .into_any_element()
        };
        let labels: Vec<_> = extras
            .labels
            .iter()
            .filter_map(|id| self.labels.iter().find(|label| &label.id == id))
            .collect();
        let labels = if labels.is_empty() {
            empty()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .children(
                    labels
                        .iter()
                        .map(|label| label_chip(&label.name, &label.color)),
                )
                .into_any_element()
        };
        let linked = match (&card.link_kind, self.link_label(&card.id)) {
            (Some(kind), Some(LinkLabel::Found(name))) => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(icon(if kind == "pilot" { "bot" } else { "chatting" }, 12.))
                .child(div().min_w_0().truncate().child(name))
                .child(
                    div()
                        .id("board-open-linked")
                        .debug_selector(|| "board-open-linked".into())
                        .role(Role::Button)
                        .aria_label("Open linked work")
                        .px(px(6.))
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .text_color(accent_color())
                        .cursor_pointer()
                        .hover(|s| s.bg(content(0.06)))
                        .child("Open")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_linked(cx)
                        })),
                )
                .into_any_element(),
            (Some(_), _) => div()
                .id("board-link-missing")
                .debug_selector(|| "board-link-missing".into())
                .text_color(content(0.4))
                .child("Linked work no longer exists")
                .into_any_element(),
            (None, _) => empty(),
        };
        let card_id = card.id.clone();
        let ticket = match self.ticket_chip(
            format!("detail-{}", card.number),
            &card.id,
            false,
            false,
            cx,
        ) {
            Some(chip) => div()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(chip)
                .child(
                    quiet_button("board-ticket-refresh", "Refresh")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener({
                            let card_id = card_id.clone();
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.refresh_card_ticket(&card_id, true, true, cx)
                            }
                        })),
                )
                .child(
                    quiet_button("board-ticket-unlink", "Unlink")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener({
                            let card_id = card_id.clone();
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.unlink_ticket(&card_id, cx)
                            }
                        })),
                )
                .into_any_element(),
            None => div()
                .text_color(content(0.35))
                .child("Link a ticket…")
                .into_any_element(),
        };
        let picker = detail.picker;
        // Relations come in three groups, each with a row to add to it. The
        // remove buttons count through all three in order.
        let mut next_remove = 0usize;
        let mut relation_rows: Vec<Vec<AnyElement>> = Vec::new();
        for (key, label, kind) in [
            ("blocks", "Blocks", RelationKind::Blocks),
            ("blocked-by", "Blocked by", RelationKind::BlockedBy),
            ("relates", "Relates to", RelationKind::RelatesTo),
        ] {
            let mut chips: Vec<AnyElement> = Vec::new();
            for relation in &detail.relations {
                let other = match (kind, relation.kind.as_str()) {
                    (RelationKind::Blocks, "blocks") if relation.card_id == card.id => {
                        &relation.related_card_id
                    }
                    (RelationKind::BlockedBy, "blocks") if relation.related_card_id == card.id => {
                        &relation.card_id
                    }
                    (RelationKind::RelatesTo, "relates") => {
                        if relation.card_id == card.id {
                            &relation.related_card_id
                        } else {
                            &relation.card_id
                        }
                    }
                    _ => continue,
                };
                let index = next_remove;
                next_remove += 1;
                let name = self.card(other).map_or_else(
                    || "Unknown card".to_string(),
                    |c| format!("#{} {}", c.number, c.title),
                );
                let relation = relation.clone();
                let remove = format!("board-relation-remove-{index}");
                chips.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(content(0.06))
                        .text_size(px(12.))
                        .child(div().min_w_0().truncate().child(name))
                        .child(
                            div()
                                .id(SharedString::from(remove.clone()))
                                .debug_selector(move || remove.clone())
                                .role(Role::Button)
                                .aria_label("Remove relation")
                                .cursor_pointer()
                                .child(icon("close", 10.))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.unrelate(relation.clone(), cx)
                                })),
                        )
                        .into_any_element(),
                );
            }
            let shown = if chips.is_empty() {
                empty()
            } else {
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .children(chips)
                    .into_any_element()
            };
            let mut group = vec![
                row(key, "link-01", label)
                    .child(value(shown))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_picker(Picker::Relation(kind), window, cx)
                    }))
                    .into_any_element(),
            ];
            if picker == Some(Picker::Relation(kind)) {
                group.push(self.render_picker(detail, Picker::Relation(kind), card, cx));
            }
            relation_rows.push(group);
        }
        // One row per field the board has, in position order.
        let mut field_rows: Vec<Vec<AnyElement>> = Vec::new();
        for (index, field) in detail.fields.iter().enumerate() {
            let raw = detail.field_values.get(&field.id);
            let shown = match field.kind.as_str() {
                "checkbox" => {
                    let on = quick::field_checked(raw.map(String::as_str));
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .size(px(14.))
                                .rounded(px(3.))
                                .border_1()
                                .border_color(content(0.3))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(on, |d| {
                                    d.bg(accent_color().opacity(0.2))
                                        .child(icon("check", 10.).text_color(accent_color()))
                                }),
                        )
                        .child(if on { "Yes" } else { "No" })
                        .into_any_element()
                }
                kind => raw
                    .and_then(|raw| quick::field_text(kind, raw))
                    .map_or_else(empty, |text| {
                        div().min_w_0().truncate().child(text).into_any_element()
                    }),
            };
            let kind = field.kind.clone();
            let mut group = vec![
                row(&format!("field-{index}"), "sliders", &field.name)
                    .child(value(shown))
                    .on_click(cx.listener(move |this, _, window, cx| match kind.as_str() {
                        "checkbox" => this.toggle_checkbox_field(index, cx),
                        "date" => this.open_choice(Choice::FieldDate(index), window, cx),
                        "select" => this.open_choice(Choice::FieldSelect(index), window, cx),
                        _ => this.open_picker(Picker::Field(index), window, cx),
                    }))
                    .into_any_element(),
            ];
            if picker == Some(Picker::Field(index)) {
                group.push(self.render_picker(detail, Picker::Field(index), card, cx));
            }
            field_rows.push(group);
        }
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                row("status", "circle-dot", "Status")
                    .child(value(status))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_choice(Choice::Status, window, cx)
                    })),
            )
            .child(
                row("priority", "flag-02", "Priority")
                    .child(value(priority))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_choice(Choice::Priority, window, cx)
                    })),
            )
            .child(
                row("start", "calendar-03", "Start date")
                    .child(value(start))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_choice(Choice::Start, window, cx)
                    })),
            )
            .when(picker == Some(Picker::StartDate), |d| {
                d.child(self.render_picker(detail, Picker::StartDate, card, cx))
            })
            .child(
                row("due", "calendar-03", "Due date")
                    .child(value(due))
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open_choice(Choice::Due, window, cx)
                        }),
                    ),
            )
            .when(picker == Some(Picker::DueDate), |d| {
                d.child(self.render_picker(detail, Picker::DueDate, card, cx))
            })
            .child(
                row("estimate", "clock", "Estimate")
                    .child(value(estimate))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_picker(Picker::Estimate, window, cx)
                    })),
            )
            .when(picker == Some(Picker::Estimate), |d| {
                d.child(self.render_picker(detail, Picker::Estimate, card, cx))
            })
            .child(
                row("projects", "folder-01", "Projects")
                    .child(value(projects))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_picker(Picker::Projects, window, cx)
                    })),
            )
            .when(picker == Some(Picker::Projects), |d| {
                d.child(self.render_picker(detail, Picker::Projects, card, cx))
            })
            .child(
                row("labels", "tag-01", "Labels")
                    .child(value(labels))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_picker(Picker::Labels, window, cx)
                    })),
            )
            .when(picker == Some(Picker::Labels), |d| {
                d.child(self.render_picker(detail, Picker::Labels, card, cx))
            })
            .child(
                row("linked", "link-01", "Linked work")
                    .child(value(linked))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_picker(Picker::Link, window, cx)
                    })),
            )
            .when(picker == Some(Picker::Link), |d| {
                d.child(self.render_picker(detail, Picker::Link, card, cx))
            })
            .child(
                row("ticket", "link-01", "Ticket")
                    .child(value(ticket))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(crate::board::Event::LinkTicket(card_id.clone()))
                    })),
            )
            .children(relation_rows.into_iter().flatten())
            .children(field_rows.into_iter().flatten())
    }

    fn render_picker(
        &self,
        detail: &CardDetail,
        picker: Picker,
        card: &Card,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.picker_query(cx).to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let check = |on: bool| {
            div().size(px(12.)).flex_shrink_0().when(on, |d| {
                d.child(icon("check", 12.).text_color(accent_color()))
            })
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        match picker {
            Picker::Projects => {
                let chosen = self.card_projects(&card.id);
                let projects = self.project_choices();
                for (index, (path, name)) in projects.iter().enumerate() {
                    if !matches(name) {
                        continue;
                    }
                    let path = path.clone();
                    rows.push(
                        checked_row(
                            format!("board-pick-project-{index}").into(),
                            name.clone(),
                            chosen.contains(&path),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_project(path.clone(), cx)
                            }),
                        )
                        .into_any_element(),
                    );
                }
                if projects.is_empty() {
                    rows.push(
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .text_size(px(12.))
                            .text_color(content(0.4))
                            .child("No projects in this workspace")
                            .into_any_element(),
                    );
                }
            }
            Picker::Labels => {
                let chosen = self.card_labels(&card.id);
                let typed = self.picker_query(cx);
                for (index, label) in self.labels.iter().enumerate() {
                    if !matches(&label.name) {
                        continue;
                    }
                    let id = label.id.clone();
                    rows.push(
                        div()
                            .id(SharedString::from(format!("board-pick-label-{index}")))
                            .debug_selector({
                                let selector = format!("board-pick-label-{index}");
                                move || selector.clone()
                            })
                            .role(Role::MenuItem)
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.07)))
                            .child(check(chosen.contains(&label.id)))
                            .child(label_chip(&label.name, &label.color))
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.toggle_label(id.clone(), cx)
                                }),
                            )
                            .into_any_element(),
                    );
                }
                let exists = self
                    .labels
                    .iter()
                    .any(|label| label.name.eq_ignore_ascii_case(&typed));
                if !typed.is_empty() && !exists {
                    rows.push(
                        menu_row(
                            "board-create-label".into(),
                            format!("Create label “{typed}”"),
                        )
                        .text_color(accent_color())
                        .on_click(cx.listener(|this, _, _, cx| this.create_label(cx)))
                        .into_any_element(),
                    );
                }
            }
            Picker::Link => {
                for (index, target) in detail.link_targets.iter().enumerate() {
                    if !matches(&target.label) {
                        continue;
                    }
                    let glyph = if target.kind == "pilot" {
                        "bot"
                    } else {
                        "chatting"
                    };
                    let (kind, id) = (target.kind, target.id.clone());
                    rows.push(
                        menu_row(
                            format!("board-link-target-{index}").into(),
                            target.label.clone(),
                        )
                        .flex()
                        .gap_2()
                        .child(icon(glyph, 12.).text_color(content(0.5)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.link_selected(Some(kind), Some(id.clone()), cx)
                        }))
                        .into_any_element(),
                    );
                }
                if card.link_kind.is_some() {
                    rows.push(
                        menu_row("board-unlink".into(), "Unlink")
                            .text_color(content(0.6))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.link_selected(None, None, cx)),
                            )
                            .into_any_element(),
                    );
                }
            }
            Picker::Relation(kind) => {
                // Two cards have one relation at most, so a card already
                // linked to this one, in any way, is not offered again.
                let linked = |other: &str| {
                    detail
                        .relations
                        .iter()
                        .any(|r| r.card_id == other || r.related_card_id == other)
                };
                let mut others: Vec<_> = self
                    .detail
                    .iter()
                    .flat_map(|d| d.cards.iter())
                    .filter(|other| other.id != card.id && !linked(&other.id))
                    .filter(|other| matches(&format!("#{} {}", other.number, other.title)))
                    .collect();
                others.sort_by_key(|other| other.number);
                for (index, other) in others.into_iter().take(50).enumerate() {
                    let id = other.id.clone();
                    rows.push(
                        menu_row(
                            format!("board-pick-relation-{index}").into(),
                            format!("#{} {}", other.number, other.title),
                        )
                        .truncate()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.relate(kind, id.clone(), cx)),
                        )
                        .into_any_element(),
                    );
                }
            }
            Picker::AddLink(kind) => {
                for (index, target) in detail.link_targets.iter().enumerate() {
                    if target.kind != kind || !matches(&target.label) {
                        continue;
                    }
                    let glyph = if kind == "pilot" { "bot" } else { "chatting" };
                    let id = target.id.clone();
                    rows.push(
                        menu_row(
                            format!("board-add-link-target-{index}").into(),
                            target.label.clone(),
                        )
                        .flex()
                        .gap_2()
                        .child(icon(glyph, 12.).text_color(content(0.5)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_link_ref(kind, id.clone(), cx)
                        }))
                        .into_any_element(),
                    );
                }
            }
            Picker::Field(field_index) => {
                if let Some(field) = detail
                    .fields
                    .get(field_index)
                    .filter(|field| field.kind == "multi-select")
                {
                    let picks =
                        quick::field_picks(detail.field_values.get(&field.id).map(String::as_str));
                    for (n, option) in quick::field_options(field.options_json.as_deref())
                        .into_iter()
                        .enumerate()
                    {
                        let on = picks.contains(&option);
                        let chosen = option.clone();
                        rows.push(
                            checked_row(
                                format!("board-field-{field_index}-option-{n}").into(),
                                option,
                                on,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_multi_option(field_index, &chosen, cx)
                            }))
                            .into_any_element(),
                        );
                    }
                }
            }
            Picker::DueDate
            | Picker::StartDate
            | Picker::Estimate
            | Picker::PrUrl
            | Picker::WorktreePath => {}
        }
        // The picker floats over the rows below it instead of pushing them
        // down: a zero-height slot marks where it opens and how wide it may be.
        let slot = detail.anchors.get("picker").cloned().unwrap_or_default();
        let width = slot
            .get()
            .map_or(DRAWER_WIDTH - 2. * DRAWER_PADDING, |bounds| {
                f32::from(bounds.size.width)
            })
            - PICKER_INDENT;
        let panel = div()
            .id("board-picker")
            .debug_selector(|| "board-picker".into())
            .occlude()
            .ml(px(PICKER_INDENT))
            .w(px(width.max(160.)))
            .p(px(4.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.1))
            .bg(floating_fill())
            .shadow(vec![
                gpui::BoxShadow::new(px(0.), px(8.), gpui::rgba(0x00000040).into())
                    .blur_radius(px(16.))
                    .spread_radius(px(-4.)),
            ])
            .flex()
            .flex_col()
            .child(
                div()
                    .id("board-picker-query")
                    .debug_selector(|| "board-picker-query".into())
                    .h(px(26.))
                    .px_2()
                    .mb_1()
                    .rounded(px(6.))
                    .bg(content(0.04))
                    .flex()
                    .items_center()
                    .child(detail.query.clone()),
            )
            .children(detail.inline_error.clone().map(|error| {
                div()
                    .id("board-inline-error")
                    .debug_selector(|| "board-inline-error".into())
                    .px(px(8.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child(error)
            }))
            .child(
                div()
                    .id("board-picker-rows")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows),
            );
        // The slot takes no room: it gives back the gap its column puts
        // before it (the links list's, or the property rows').
        let gap = match picker {
            Picker::AddLink(_) | Picker::PrUrl | Picker::WorktreePath => 4.,
            _ => 2.,
        };
        div()
            .relative()
            .w_full()
            .h_0()
            .mt(px(-gap))
            .child(slot.layout_probe())
            .child(gpui::deferred(panel).with_priority(1))
            .into_any_element()
    }

    /// The card's links: kind and reference, with Open and Remove, then an
    /// Add menu. Tickets keep their own row above.
    fn render_links(
        &self,
        detail: &CardDetail,
        card: &Card,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = |kind: &str| match kind {
            "session" => "Session",
            "pilot" => "Pilot",
            "flow-run" => "Flow run",
            "pr" => "PR",
            "worktree" => "Worktree",
            _ => "",
        };
        // The primary link already shows in the Linked work row (ruling
        // C3-f), whether stored as role `primary` or as an extra row that
        // matches the card's link.
        let primary = (card.link_kind.as_deref(), card.link_id.as_deref());
        let rows = detail
            .links
            .iter()
            .filter(|link| super::shown_link(link, primary))
            .enumerate()
            .map(|(index, link)| {
                let openable =
                    matches!(link.kind.as_str(), "session" | "pilot" | "pr" | "worktree");
                let (row_id, open_id, remove_id) = (
                    format!("board-link-row-{index}"),
                    format!("board-link-open-{index}"),
                    format!("board-link-remove-{index}"),
                );
                let (open, remove) = (link.clone(), link.clone());
                let small = |id: String, text: &'static str| {
                    div()
                        .id(SharedString::from(id.clone()))
                        .debug_selector(move || id.clone())
                        .role(Role::Button)
                        .aria_label(text)
                        .px(px(6.))
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .text_color(content(0.6))
                        .cursor_pointer()
                        .hover(|s| s.bg(content(0.06)).text_color(content(0.95)))
                        .child(text)
                };
                div()
                    .id(SharedString::from(row_id.clone()))
                    .debug_selector(move || row_id.clone())
                    .min_h(px(28.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .child(
                        div()
                            .w(px(72.))
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(label(&link.kind)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.link_row_text(link)),
                    )
                    .when(openable, |d| {
                        d.child(small(open_id, "Open").text_color(accent_color()).on_click(
                            cx.listener(move |this, _, _, cx| this.open_link_row(open.clone(), cx)),
                        ))
                    })
                    .child(small(remove_id, "Remove").on_click(
                        cx.listener(move |this, _, _, cx| this.remove_link_row(remove.clone(), cx)),
                    ))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let picker = detail.picker;
        let adding = matches!(
            picker,
            Some(Picker::AddLink(_) | Picker::PrUrl | Picker::WorktreePath)
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(section("Links"))
                    .child(
                        div()
                            .id("board-drawer-links-add")
                            .debug_selector(|| "board-drawer-links-add".into())
                            .relative()
                            .role(Role::Button)
                            .aria_label("Add link")
                            .size(px(22.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.06)))
                            .child(
                                detail
                                    .anchors
                                    .get("links-add")
                                    .cloned()
                                    .unwrap_or_default()
                                    .probe(),
                            )
                            .child(icon("plus", 12.).text_color(content(0.6)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_choice(Choice::LinkAdd, window, cx)
                            })),
                    ),
            )
            .children(rows.is_empty().then(empty))
            .children(rows)
            .when(adding, |d| {
                d.child(self.render_picker(detail, picker.unwrap_or(Picker::PrUrl), card, cx))
            })
    }

    /// What a links-list row shows: a session's or Pilot's name, "No longer
    /// exists" for work that is gone, and the reference for a PR, worktree
    /// or flow run.
    pub(in crate::board) fn link_row_text(&self, link: &CardLink) -> String {
        match link.kind.as_str() {
            "session" | "pilot" => match self.link_target(&link.kind, &link.reference) {
                LinkLabel::Found(name) => name,
                LinkLabel::Missing => "No longer exists".into(),
            },
            _ => link.reference.clone(),
        }
    }

    fn render_markdown_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("board-md-toolbar")
            .debug_selector(|| "board-md-toolbar".into())
            .role(Role::Toolbar)
            .aria_label("Formatting")
            .flex()
            .items_center()
            .gap(px(2.))
            .children(Format::ALL.into_iter().map(|format| {
                let key = format!("board-md-{}", format.key());
                div()
                    .id(SharedString::from(key.clone()))
                    .debug_selector(move || key.clone())
                    .role(Role::Button)
                    .aria_label(format.label())
                    .h(px(24.))
                    .min_w(px(24.))
                    .px(px(6.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(content(0.6))
                    .when(format == Format::Bold, |d| d.font_weight(FontWeight::BOLD))
                    .when(format == Format::Italic, |d| d.italic())
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.08)))
                    .child(format.glyph())
                    // Keep the press from taking focus (and saving) before the
                    // transform reads the description's selection.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.format_description(format, window, cx)
                    }))
            }))
    }

    fn render_subtasks(&self, detail: &CardDetail, cx: &mut Context<Self>) -> impl IntoElement {
        let total = detail.subtasks.len();
        let done = detail.subtasks.iter().filter(|s| s.done).count();
        let share = if total == 0 {
            0.
        } else {
            done as f32 / total as f32
        };
        let adding = detail.subtask_input.clone();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(section("Subtasks"))
                    .when(total > 0, |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child(format!("{done}/{total}")),
                        )
                        .child(
                            div()
                                .w(px(100.))
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
                        )
                    }),
            )
            .children(detail.subtasks.iter().enumerate().map(|(index, subtask)| {
                let toggle = subtask.clone();
                let remove = subtask.id.clone();
                let group = format!("board-subtask-group-{index}");
                div()
                    .id(SharedString::from(format!("board-subtask-{index}")))
                    .debug_selector({
                        let selector = format!("board-subtask-{index}");
                        move || selector.clone()
                    })
                    .group(group.clone())
                    .role(Role::CheckBox)
                    .aria_label(subtask.title.clone())
                    .aria_toggled(if subtask.done {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .h(px(30.))
                    .px(px(6.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.04)))
                    .when_some(detail.subtask_focus.get(&subtask.id), |d, focus| {
                        d.track_focus(focus).tab_index(0)
                    })
                    .focus_visible(|s| s.bg(content(0.08)))
                    .on_key_down(cx.listener({
                        let id = subtask.id.clone();
                        move |this, event: &gpui::KeyDownEvent, _, cx| {
                            if let Some(by) = super::subtask_shift(&event.keystroke) {
                                cx.stop_propagation();
                                this.shift_subtask(&id, by, cx);
                            }
                        }
                    }))
                    .child(
                        div()
                            .size(px(14.))
                            .flex_shrink_0()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if subtask.done {
                                accent_color()
                            } else {
                                content(0.3)
                            })
                            .when(subtask.done, |d| d.bg(accent_color()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(subtask.done, |d| {
                                d.child(icon("check", 10.).text_color(white(0.95)))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(content(if subtask.done { 0.45 } else { 0.9 }))
                            .when(subtask.done, |d| d.line_through())
                            .child(subtask.title.clone()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("board-subtask-menu-{index}")))
                            .debug_selector({
                                let selector = format!("board-subtask-menu-{index}");
                                move || selector.clone()
                            })
                            .relative()
                            .role(Role::Button)
                            .aria_label("Subtask options")
                            .size(px(20.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.)
                            .group_hover(group.clone(), |s| s.opacity(1.))
                            .hover(|s| s.bg(content(0.08)))
                            .child(
                                detail
                                    .anchors
                                    .get(&format!("subtask-{index}"))
                                    .cloned()
                                    .unwrap_or_default()
                                    .probe(),
                            )
                            .child(icon("more-horizontal", 12.).text_color(content(0.5)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_choice(Choice::Subtask(index), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("board-subtask-delete-{index}")))
                            .debug_selector({
                                let selector = format!("board-subtask-delete-{index}");
                                move || selector.clone()
                            })
                            .role(Role::Button)
                            .aria_label("Delete subtask")
                            .size(px(20.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.)
                            .group_hover(group, |s| s.opacity(1.))
                            .hover(|s| s.bg(content(0.08)))
                            .child(icon("close", 10.).text_color(content(0.5)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.delete_subtask(remove.clone(), cx)
                            })),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_subtask(toggle.clone(), cx)),
                    )
            }))
            .child(match adding {
                Some(input) => div()
                    .h(px(30.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .child(input)
                    .into_any_element(),
                None => quiet_button("board-subtask-add", "Add subtask")
                    .child(icon("plus", 12.))
                    .on_click(cx.listener(|this, _, window, cx| this.start_subtask(window, cx)))
                    .into_any_element(),
            })
    }

    fn render_tabs(&self, detail: &CardDetail, cx: &mut Context<Self>) -> impl IntoElement {
        let activity_count = detail.activity_badge();
        let tab = |id: &'static str, label: &'static str, count: String, on: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .role(Role::Tab)
                .aria_selected(on)
                .pb(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(13.))
                .text_color(content(if on { 0.95 } else { 0.5 }))
                .when(on, |d| d.border_b_2().border_color(content(0.9)))
                .cursor_pointer()
                .child(label)
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(count),
                )
        };
        let body: AnyElement = match detail.tab {
            Tab::Comments => {
                let now = gespan_core::time::now_ms();
                let focus = detail.comment.focus_handle(cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when(detail.comments.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child("No comments yet."),
                        )
                    })
                    .children(detail.comments.iter().enumerate().map(|(index, comment)| {
                        let (name, initial, agent) = comment_author(&comment.author);
                        div()
                            .id(SharedString::from(format!("board-comment-{index}")))
                            .debug_selector({
                                let selector = format!("board-comment-{index}");
                                move || selector.clone()
                            })
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id(SharedString::from(format!("board-comment-avatar-{index}")))
                                    .debug_selector({
                                        let selector =
                                            format!("board-comment-avatar-{index}-{initial}");
                                        move || selector.clone()
                                    })
                                    .size(px(22.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::MEDIUM)
                                    // An agent wears the accent; you stay neutral.
                                    .when(agent, |d| d.bg(accent_color()).text_color(white(0.95)))
                                    .when(!agent, |d| d.bg(content(0.1)))
                                    .child(initial),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .text_size(px(12.))
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "board-comment-author-{index}"
                                                    )))
                                                    .debug_selector({
                                                        let selector = format!(
                                                            "board-comment-author-{index}-{name}"
                                                        );
                                                        move || selector.clone()
                                                    })
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(name),
                                            )
                                            .child(div().text_color(content(0.4)).child(
                                                gespan_core::time::relative(
                                                    comment.created_at.max(0) as u64,
                                                    now,
                                                ),
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(content(0.85))
                                            .child(comment.body.clone()),
                                    ),
                            )
                    }))
                    .child(
                        div()
                            .id("board-comment-input")
                            .debug_selector(|| "board-comment-input".into())
                            .min_h(px(64.))
                            .p(px(8.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(content(0.1))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .cursor_text()
                            .child(detail.comment.clone())
                            .on_click(move |_, window, cx| window.focus(&focus, cx))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(content(0.35))
                                            .child(SEND_HINT),
                                    )
                                    .child(
                                        div()
                                            .id("board-comment-send")
                                            .debug_selector(|| "board-comment-send".into())
                                            .role(Role::Button)
                                            .aria_label("Comment")
                                            .h(px(24.))
                                            .px(px(10.))
                                            .rounded_md()
                                            .flex()
                                            .items_center()
                                            .bg(accent_color())
                                            .text_color(white(0.95))
                                            .text_size(px(12.))
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.9))
                                            .child("Comment")
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.send_comment(cx)
                                            })),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            Tab::Activity => {
                let now = gespan_core::time::now_ms();
                match &detail.activity {
                    Some(Ok(rows)) if rows.is_empty() => div()
                        .text_size(px(12.))
                        .text_color(content(0.45))
                        .child("No activity yet")
                        .into_any_element(),
                    Some(Ok(rows)) => div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .children(rows.iter().map(|row| {
                            let when =
                                gespan_core::time::relative(row.created_at.max(0) as u64, now);
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.65))
                                .child(format!("{} · {when}", activity_text(row)))
                        }))
                        .into_any_element(),
                    Some(Err(_)) => div()
                        .id("board-activity-failed")
                        .debug_selector(|| "board-activity-failed".into())
                        .text_size(px(12.))
                        .text_color(diff_foreground(false))
                        .child("Couldn't load activity")
                        .into_any_element(),
                    None => div().into_any_element(),
                }
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .border_b_1()
                    .border_color(content(0.07))
                    .child(
                        tab(
                            "board-tab-comments",
                            "Comments",
                            detail.comments.len().to_string(),
                            detail.tab == Tab::Comments,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Comments, cx))),
                    )
                    .child(
                        tab(
                            "board-tab-activity",
                            "Activity",
                            activity_count,
                            detail.tab == Tab::Activity,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Activity, cx))),
                    ),
            )
            .child(body)
    }
}
