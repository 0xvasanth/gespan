//! The header's board switcher: open, create, rename and delete boards. The
//! board last opened on purpose is remembered in the `board-open` setting.
use super::{
    BoardView,
    header::{checked_row, presses},
    menu_row,
};
use crate::{
    input::{self, Input},
    popover::{Anchor, popover},
    runtime::locked,
    theme::*,
};
use gespan_core::board::extras::TemplateInfo;
use gpui::{
    AnyElement, Context, Entity, Focusable, FontWeight, Role, SharedString, Subscription, Window,
    div, prelude::*, px,
};

pub(super) const SETTING: &str = "board-open";

pub(super) enum Mode {
    List,
    New(Entity<Input>),
    Rename(Entity<Input>),
    /// Delete was pressed once; the next press does it.
    ConfirmDelete,
}

pub(super) struct Switcher {
    mode: Mode,
    /// The store's reason a name was refused, shown in the menu.
    error: Option<String>,
    /// New board: the templates on offer and the one picked (Blank first).
    templates: Vec<TemplateInfo>,
    template: String,
    /// A board this menu made but could not open (the drawer's edits would
    /// not save): its id and name. Enter with that name opens it again
    /// instead of making a second one.
    created: Option<(String, String)>,
    _submit: Option<Subscription>,
}

const BLANK: &str = "blank";

impl Switcher {
    fn list() -> Self {
        Self {
            mode: Mode::List,
            error: None,
            templates: Vec::new(),
            template: BLANK.into(),
            created: None,
            _submit: None,
        }
    }
}

impl BoardView {
    fn open_board_name(&self) -> String {
        self.boards
            .iter()
            .find(|board| Some(&board.id) == self.open_board_id.as_ref())
            .map_or_else(|| "Board".to_string(), |board| board.name.clone())
    }

    fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.filter_open = false;
        self.sort_open = false;
        self.menu = None;
        self.switcher = match self.switcher {
            Some(_) => None,
            None => Some(Switcher::list()),
        };
        cx.notify();
    }

    /// Show the board list, opening it when it is closed.
    pub(crate) fn open_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_none() {
            self.toggle_switcher(window, cx);
        } else {
            window.focus(&self.focus, cx);
        }
    }

    /// Open a board and remember it. The search and filter go (labels belong
    /// to a board), as do the drawer and any open menu. False when the drawer
    /// could not save its edits, in which case nothing changes.
    pub(crate) fn open_board(&mut self, id: String, cx: &mut Context<Self>) -> bool {
        if self.open_board_id.as_deref() != Some(id.as_str()) {
            if self.card_detail.is_some() {
                self.close_card_detail(cx);
                if self.card_detail.is_some() {
                    return false;
                }
            }
            // A delete held by the toast is carried out before leaving.
            self.commit_pending(cx);
            self.remember_board(&id);
            self.selected_board_id = Some(id);
            self.reset_for_board(cx);
            self.reload(cx);
        }
        self.switcher = None;
        cx.notify();
        true
    }

    fn remember_board(&mut self, id: &str) {
        if let Err(error) = locked(&self.store).and_then(|db| db.save_setting(SETTING, id)) {
            self.error = Some(error);
        }
    }

    fn reset_for_board(&mut self, cx: &mut Context<Self>) {
        self.reset_view(cx);
        self.settings = None;
        self.default_view_applied = None;
        self.filter_open = false;
        self.sort_open = false;
        self.card_menu = None;
        self.menu = None;
        self.composer = None;
        self.new_card = None;
        self.renaming = None;
    }

    /// Make a board and open it. False when the name is blank or refused; the
    /// store's reason shows in the switcher.
    /// Make a board from a template (`blank` is today's four columns) and
    /// open it.
    pub(crate) fn create_board_from(
        &mut self,
        name: &str,
        template: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        // The board was made by an earlier Enter that couldn't open it:
        // open that one, never a duplicate.
        if let Some((id, _)) = self
            .switcher
            .as_ref()
            .and_then(|switcher| switcher.created.clone())
            .filter(|(_, made)| made == name)
        {
            return self.open_made_board(id, name, cx);
        }
        let made = self.service().and_then(|service| {
            if template == BLANK {
                service.create(name)
            } else {
                service.create_from_template(name, template)
            }
        });
        match made {
            Ok(board) => self.open_made_board(board.id, name, cx),
            Err(error) => {
                self.switcher_error(error, cx);
                false
            }
        }
    }

    /// Open a board the switcher just made. When the drawer's edits won't
    /// save, the board stays made but closed: the name stays in the field,
    /// the menu says why, and the next Enter tries to open it again.
    fn open_made_board(&mut self, id: String, name: &str, cx: &mut Context<Self>) -> bool {
        if self.open_board(id.clone(), cx) {
            return true;
        }
        // The new board is in the list now, but nothing else changed.
        if let Ok(boards) = self.service().and_then(|service| service.list()) {
            self.boards = boards;
        }
        let reason = self
            .error
            .clone()
            .unwrap_or_else(|| "the open card's edits weren't saved".into());
        if let Some(switcher) = &mut self.switcher {
            switcher.created = Some((id, name.to_string()));
        }
        self.switcher_error(
            format!("Created \"{name}\" but couldn't open it: {reason}"),
            cx,
        );
        false
    }

    fn rename_open_board(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let (name, Some(id)) = (name.trim(), self.open_board_id.clone()) else {
            return false;
        };
        if name.is_empty() {
            return false;
        }
        match self.service().and_then(|service| service.rename(&id, name)) {
            Ok(()) => {
                self.switcher = None;
                self.reload(cx);
                true
            }
            Err(error) => {
                self.switcher_error(error, cx);
                false
            }
        }
    }

    /// Delete the open board and everything on it. Another board opens; if
    /// none is left, `load` makes a fresh default one.
    fn delete_open_board(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.open_board_id.clone() else {
            return;
        };
        self.commit_pending(cx);
        if let Err(error) = self.service().and_then(|service| service.delete(&id)) {
            self.switcher_error(error, cx);
            return;
        }
        self.selected_board_id = None;
        self.card_detail = None;
        self.reset_for_board(cx);
        self.switcher = None;
        self.reload(cx);
        if let Some(next) = self.open_board_id.clone() {
            self.remember_board(&next);
        }
        cx.notify();
    }

    fn switcher_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(switcher) = &mut self.switcher {
            switcher.error = Some(error);
        } else {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn start_board_name(&mut self, renaming: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| Input::field("Board name", "Board name", content(0.4), cx));
        if renaming {
            let current = self.open_board_name();
            input.update(cx, |input, cx| input.set_text(current, cx));
        }
        let submit = cx.subscribe(&input, move |this, input, _: &input::Submitted, cx| {
            let name = input.read(cx).content.to_string();
            if renaming {
                this.rename_open_board(&name, cx);
            } else {
                let template = this
                    .switcher
                    .as_ref()
                    .map_or_else(|| BLANK.to_string(), |switcher| switcher.template.clone());
                this.create_board_from(&name, &template, cx);
            }
        });
        let (templates, error) = if renaming {
            (Vec::new(), None)
        } else {
            match self.service().and_then(|service| service.templates()) {
                Ok(templates) => (templates, None),
                // Blank needs no stored template, so it is still offered
                // (and created by Enter) without the list.
                Err(error) => (
                    vec![TemplateInfo {
                        id: BLANK.into(),
                        name: "Blank".into(),
                        built_in: true,
                    }],
                    Some(error),
                ),
            }
        };
        window.focus(&input.focus_handle(cx), cx);
        self.switcher = Some(Switcher {
            mode: if renaming {
                Mode::Rename(input)
            } else {
                Mode::New(input)
            },
            error,
            templates,
            template: BLANK.into(),
            created: None,
            _submit: Some(submit),
        });
        cx.notify();
    }

    /// "No boards yet": Create board names one and picks its template.
    pub(super) fn render_no_boards(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .debug_selector(|| "board-no-boards".into())
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(content(0.6))
                    .child("No boards yet"),
            )
            .child(
                crate::modal::primary_button("board-create-first", "Create board")
                    .debug_selector(|| "board-create-first".into())
                    .child(self.switcher_anchor.probe())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.start_board_name(false, window, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_switcher_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.open_board_name();
        let open = self.switcher.is_some();
        div()
            .id("board-switcher")
            .debug_selector(|| "board-switcher".into())
            .relative()
            .role(Role::Button)
            .aria_label(format!("Board: {name}"))
            .h(px(26.))
            .px_2()
            .rounded_md()
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(content(0.06)))
            .when(open, |d| d.bg(content(0.05)))
            .track_focus(&self.switcher_focus)
            .tab_index(0)
            .focus_visible(|s| s.border_1().border_color(accent_color()))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if presses(event) {
                    cx.stop_propagation();
                    this.toggle_switcher(window, cx);
                }
            }))
            .child(self.switcher_anchor.probe())
            .child(icon("kanban", 14.).text_color(content(0.55)))
            .child(
                div()
                    .max_w(px(200.))
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .child(icon("chevron-down", 11.).text_color(content(0.45)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_switcher(window, cx)))
    }

    pub(super) fn render_switcher(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let switcher = self.switcher.as_ref()?;
        let weak = cx.weak_entity();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, board) in self.boards.iter().enumerate() {
            let on = Some(&board.id) == self.open_board_id.as_ref();
            let id = board.id.clone();
            rows.push(
                checked_row(
                    format!("board-switch-{index}").into(),
                    board.name.clone(),
                    on,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_board(id.clone(), cx);
                }))
                .into_any_element(),
            );
        }
        if !rows.is_empty() {
            rows.push(div().h(px(1.)).my_1().bg(content(0.07)).into_any_element());
        }
        match &switcher.mode {
            Mode::New(input) => {
                rows.push(name_field("board-new-name", input));
                for template in &switcher.templates {
                    let on = switcher.template == template.id;
                    let id = template.id.clone();
                    let field = input.clone();
                    rows.push(
                        checked_row(
                            format!("board-template-{}", template.id).into(),
                            template.name.clone(),
                            on,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(switcher) = &mut this.switcher {
                                switcher.template = id.clone();
                            }
                            // Enter still creates: the name field keeps focus.
                            window.focus(&field.focus_handle(cx), cx);
                            cx.notify();
                        }))
                        .into_any_element(),
                    );
                }
            }
            Mode::Rename(input) => rows.push(name_field("board-rename-name", input)),
            Mode::List => {
                rows.push(
                    menu_row("board-settings-open".into(), "Board settings")
                        .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)))
                        .into_any_element(),
                );
                rows.push(
                    menu_row("board-new".into(), "New board")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_board_name(false, window, cx)
                        }))
                        .into_any_element(),
                );
                rows.push(
                    menu_row("board-rename".into(), "Rename board")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_board_name(true, window, cx)
                        }))
                        .into_any_element(),
                );
                rows.push(
                    menu_row("board-delete".into(), "Delete board")
                        .text_color(diff_foreground(false))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(switcher) = &mut this.switcher {
                                switcher.mode = Mode::ConfirmDelete;
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            Mode::ConfirmDelete => {
                let label = format!("Delete \"{}\" and its cards", self.open_board_name());
                rows.push(
                    menu_row("board-delete-confirm".into(), label)
                        .text_color(diff_foreground(false))
                        .on_click(cx.listener(|this, _, _, cx| this.delete_open_board(cx)))
                        .into_any_element(),
                );
            }
        }
        if let Some(error) = &switcher.error {
            rows.push(
                div()
                    .id("board-switcher-error")
                    .debug_selector(|| "board-switcher-error".into())
                    .px(px(8.))
                    .py(px(4.))
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child(SharedString::from(error.clone()))
                    .into_any_element(),
            );
        }
        Some(
            popover(
                "board-switcher-menu",
                Anchor::Slot(self.switcher_anchor.clone()),
            )
            .solid()
            .width(260.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.switcher = None;
                    cx.notify();
                });
            })
            .child(
                div()
                    .id("board-switcher-list")
                    .debug_selector(|| "board-switcher-list".into())
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
impl BoardView {
    /// The text in the switcher's new or rename name field, if one is open.
    pub(super) fn switcher_name_text(&self, cx: &gpui::App) -> Option<String> {
        match &self.switcher.as_ref()?.mode {
            Mode::New(input) | Mode::Rename(input) => Some(input.read(cx).content.to_string()),
            _ => None,
        }
    }

    /// The reason the switcher shows, if any.
    pub(super) fn switcher_error_text(&self) -> Option<String> {
        self.switcher.as_ref()?.error.clone()
    }
}

fn name_field(id: &'static str, input: &Entity<Input>) -> AnyElement {
    div()
        .id(id)
        .debug_selector(|| id.into())
        .mx(px(4.))
        .my(px(2.))
        .h(px(26.))
        .px_2()
        .rounded_md()
        .border_1()
        .border_color(content(0.12))
        .flex()
        .items_center()
        .child(div().flex_1().min_w_0().child(input.clone()))
        .into_any_element()
}
