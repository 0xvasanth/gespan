//! How the task drawer looks: gr8r's top bar, property grid, description,
//! subtasks and Comments/Activity tabs, in the board's quiet type scale.
use super::{CardDetail, Choice, Picker, Tab, activity_text};
use crate::{
    board::{
        BoardView, LinkLabel,
        column::role_icon,
        menu_row,
        style::{due_label, label_chip, priority_glyph},
    },
    popover::{Anchor, menu_popover},
    theme::*,
};
use gpui::{
    AnyElement, Context, Focusable, FontWeight, MouseButton, Role, SharedString, div, prelude::*,
    px,
};
use rebase_core::board::{Card, extras::PRIORITIES};

const DRAWER_WIDTH: f32 = 440.;
/// The comment field sends on the platform's secondary submit chord.
pub(crate) const SEND_HINT: &str = if cfg!(target_os = "macos") {
    "⌘Enter"
} else {
    "Ctrl+Enter"
};

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
                Choice::Status => "status",
                Choice::Priority => "priority",
                Choice::Due => "due",
                Choice::More => "more",
            };
            menu_popover(menu, Anchor::Slot(anchor(key)), cx).into_any_element()
        });
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
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_choice(Choice::More, window, cx)
                    })),
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
            .child(self.render_subtasks(detail, cx))
            .child(self.render_tabs(detail, cx));
        Some(
            div()
                .id("board-drawer")
                .debug_selector(|| "board-drawer".into())
                .role(Role::Dialog)
                .aria_label(format!("Task #{}", card.number))
                .w(px(DRAWER_WIDTH))
                .h_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .bg(pane_fill())
                .border_l_1()
                .border_color(content(0.07))
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
        let row = |key: &'static str, glyph: &'static str, label: &'static str| {
            let selector = format!("board-drawer-{key}");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .relative()
                .role(Role::Button)
                .aria_label(label)
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
        let now = rebase_core::time::now_ms() as i64;
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
                .child(PRIORITIES[extras.priority as usize])
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
        let picker = detail.picker;
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
                        menu_row(format!("board-pick-project-{index}").into(), name.clone())
                            .flex()
                            .gap_2()
                            .child(check(chosen.contains(&path)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_project(path.clone(), cx)
                            }))
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
            Picker::DueDate => {}
        }
        div()
            .id("board-picker")
            .debug_selector(|| "board-picker".into())
            .ml(px(126.))
            .mb_1()
            .p(px(4.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.1))
            .bg(content(0.02))
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
            .child(
                div()
                    .id("board-picker-rows")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element()
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
                    .h(px(30.))
                    .px(px(6.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.04)))
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
                            .id(SharedString::from(format!("board-subtask-delete-{index}")))
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
                let now = rebase_core::time::now_ms();
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
                                    .size(px(22.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(content(0.1))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(10.))
                                    .child("Y"),
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
                                                div().font_weight(FontWeight::MEDIUM).child("You"),
                                            )
                                            .child(div().text_color(content(0.4)).child(
                                                rebase_core::time::relative(
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
                let now = rebase_core::time::now_ms();
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
                                rebase_core::time::relative(row.created_at.max(0) as u64, now);
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
