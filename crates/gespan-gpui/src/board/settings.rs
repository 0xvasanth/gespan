//! The board settings page: one page over the board with a section list on
//! the left. General, Columns, Labels and Priority live here; every write goes
//! through `Service` (settings writes are patches, so a toggle the other app
//! just set survives).
mod columns;
mod general;
mod labels;
mod priority;

use super::{BoardView, style::PALETTE};
use crate::{
    input::{self, Input},
    theme::*,
};
use gespan_core::board::{Service, extras::SettingsPatch};
use gpui::{
    AnyElement, Context, Entity, Focusable, FontWeight, Role, SharedString, Subscription, Window,
    div, prelude::*, px,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    General,
    Columns,
    Labels,
    Priority,
}

impl Section {
    const ALL: [(Section, &'static str, &'static str); 4] = [
        (Section::General, "general", "General"),
        (Section::Columns, "columns", "Columns"),
        (Section::Labels, "labels", "Labels"),
        (Section::Priority, "priority", "Priority"),
    ];
}

/// The one text field being edited on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Edit {
    BoardName,
    ColumnName(String),
    ColumnWip(String),
    LabelName(String),
    LabelNew,
    Priority(u8),
}

struct Editing {
    what: Edit,
    input: Entity<Input>,
    _submit: Subscription,
}

pub(crate) struct Page {
    section: Section,
    edit: Option<Editing>,
    /// The store's reason the last write was refused.
    error: Option<String>,
    /// A column being deleted while it still holds cards: "Move cards to…".
    moving: Option<String>,
    /// A label being merged into another, or asked to confirm its delete.
    merging: Option<String>,
    deleting_label: Option<String>,
}

impl Page {
    fn new(section: Section) -> Self {
        Self {
            section,
            edit: None,
            error: None,
            moving: None,
            merging: None,
            deleting_label: None,
        }
    }

    fn clear_prompts(&mut self) {
        self.edit = None;
        self.moving = None;
        self.merging = None;
        self.deleting_label = None;
    }
}

/// A clickable control with a stable id for assistive tech and tests.
pub(super) fn target(id: String) -> gpui::Stateful<gpui::Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .role(Role::Button)
}

pub(super) fn button(id: String, label: &str, on: bool) -> gpui::Stateful<gpui::Div> {
    target(id)
        .aria_label(label.to_string())
        .h(px(24.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(content(if on { 0.35 } else { 0.1 }))
        .when(on, |d| d.bg(content(0.08)))
        .text_size(px(12.))
        .text_color(content(if on { 0.95 } else { 0.65 }))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.07)))
        .child(label.to_string())
}

pub(super) fn swatch(id: String, hex: &str, on: bool) -> gpui::Stateful<gpui::Div> {
    target(id)
        .aria_label(format!("#{hex}"))
        .size(px(18.))
        .rounded_full()
        .border_2()
        .border_color(if on { content(0.9) } else { content(0.0) })
        .cursor_pointer()
        .child(
            div()
                .size_full()
                .rounded_full()
                .bg(super::style::label_color(hex)),
        )
}

pub(super) fn palette_hexes() -> impl Iterator<Item = String> {
    PALETTE.iter().map(|color| format!("{color:06x}"))
}

pub(super) fn heading(text: &'static str) -> gpui::Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(content(0.45))
        .child(text)
}

pub(super) fn card_row() -> gpui::Div {
    div()
        .px(px(10.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(content(0.08))
        .flex()
        .items_center()
        .gap(px(8.))
}

impl BoardView {
    /// Open the settings page on General.
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_board_id.is_none() {
            return;
        }
        self.switcher = None;
        self.menu = None;
        self.filter_open = false;
        self.sort_open = false;
        self.open_in_open = false;
        self.composer = None;
        window.focus(&self.focus, cx);
        self.settings = Some(Page::new(Section::General));
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            cx.notify();
        }
    }

    /// Escape on the settings page: the innermost prompt first, then the page.
    pub(super) fn settings_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(page) = &mut self.settings else {
            return false;
        };
        if page.edit.is_some()
            || page.moving.is_some()
            || page.merging.is_some()
            || page.deleting_label.is_some()
        {
            page.clear_prompts();
        } else {
            self.settings = None;
        }
        cx.notify();
        true
    }

    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if let Some(page) = &mut self.settings {
            page.section = section;
            page.clear_prompts();
            page.error = None;
        }
        cx.notify();
    }

    /// Run a write, show its refusal on the page, and reload either way.
    fn settings_write(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Service) -> Result<(), String>,
    ) -> bool {
        let result = self.service().and_then(action);
        let ok = result.is_ok();
        if let Some(page) = &mut self.settings {
            page.error = result.err();
            if ok {
                page.clear_prompts();
            }
        }
        self.reload(cx);
        ok
    }

    pub(super) fn patch_settings(&mut self, patch: SettingsPatch, cx: &mut Context<Self>) -> bool {
        let Some(board_id) = self.open_board_id.clone() else {
            return false;
        };
        self.settings_write(cx, |service| {
            service.update_settings(&board_id, &patch).map(drop)
        })
    }

    fn start_edit(
        &mut self,
        what: Edit,
        current: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| Input::field("Value", "Value", content(0.4), cx));
        input.update(cx, |input, cx| input.set_text(current.to_string(), cx));
        let submit = cx.subscribe_in(
            &input,
            window,
            |this, _, _: &input::Submitted, window, cx| this.commit_edit(window, cx),
        );
        window.focus(&input.focus_handle(cx), cx);
        if let Some(page) = &mut self.settings {
            page.clear_prompts();
            page.error = None;
            page.edit = Some(Editing {
                what,
                input,
                _submit: submit,
            });
        }
        cx.notify();
    }

    /// The field being edited, when it is `what`.
    pub(super) fn editing(&self, what: &Edit) -> Option<Entity<Input>> {
        let editing = self.settings.as_ref()?.edit.as_ref()?;
        (&editing.what == what).then(|| editing.input.clone())
    }

    /// A refused edit stays open to correct; an accepted one closes.
    fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.settings.as_mut().and_then(|page| page.edit.take()) else {
            return;
        };
        let text = editing.input.read(cx).content.trim().to_string();
        let board_id = self.open_board_id.clone();
        let ok = match editing.what.clone() {
            Edit::BoardName => match board_id {
                Some(id) if !text.is_empty() => {
                    self.settings_write(cx, |service| service.rename(&id, &text))
                }
                _ => true,
            },
            Edit::ColumnName(id) => {
                if text.is_empty() {
                    true
                } else {
                    let role = self.column(&id).and_then(|column| column.role.clone());
                    self.settings_write(cx, |service| {
                        service.column_update(&id, &text, role.as_deref())
                    })
                }
            }
            Edit::ColumnWip(id) => self.commit_wip(&id, &text, cx),
            Edit::LabelName(id) => {
                if text.is_empty() {
                    true
                } else {
                    self.settings_write(cx, |service| service.label_update(&id, Some(&text), None))
                }
            }
            Edit::LabelNew => match board_id {
                Some(board_id) if !text.is_empty() => {
                    let color = super::style::next_label_color(self.labels.len());
                    self.settings_write(cx, |service| {
                        service.create_label(&board_id, &text, &color).map(drop)
                    })
                }
                _ => true,
            },
            Edit::Priority(level) => self.commit_priority_name(level, &text, cx),
        };
        if ok {
            // The field is gone: keys (Escape) go to the board again.
            window.focus(&self.focus, cx);
        } else if let Some(page) = &mut self.settings {
            window.focus(&editing.input.focus_handle(cx), cx);
            page.edit = Some(editing);
        }
        cx.notify();
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.settings.as_ref()?;
        let nav: Vec<AnyElement> = Section::ALL
            .iter()
            .map(|&(section, key, label)| {
                let on = page.section == section;
                target(format!("settings-nav-{key}"))
                    .aria_label(label)
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .text_color(content(if on { 0.95 } else { 0.6 }))
                    .when(on, |d| d.bg(content(0.07)))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_section(section, cx)))
                    .into_any_element()
            })
            .collect();
        let body = match page.section {
            Section::General => self.render_general(cx),
            Section::Columns => self.render_columns(cx),
            Section::Labels => self.render_labels(cx),
            Section::Priority => self.render_priority(cx),
        };
        let error = page.error.clone().map(|error| {
            div()
                .id("settings-error")
                .debug_selector(|| "settings-error".into())
                .text_size(px(12.))
                .text_color(diff_foreground(false))
                .child(error)
        });
        Some(
            div()
                .id("board-settings")
                .debug_selector(|| "board-settings".into())
                .role(Role::Region)
                .aria_label("Board settings")
                .flex_1()
                .min_h_0()
                .flex()
                .child(
                    div()
                        .w(px(180.))
                        .flex_shrink_0()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .border_r_1()
                        .border_color(content(0.07))
                        .child(
                            target("settings-close".into())
                                .aria_label("Back to board")
                                .px(px(10.))
                                .py(px(6.))
                                .mb_2()
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(12.))
                                .text_color(content(0.6))
                                .cursor_pointer()
                                .hover(|s| s.bg(content(0.06)))
                                .child(icon("chevron-left", 12.))
                                .child("Back to board")
                                .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))),
                        )
                        .children(nav),
                )
                .child(
                    div()
                        .id("settings-body")
                        .flex_1()
                        .min_w_0()
                        .overflow_y_scroll()
                        .p_5()
                        .child(
                            div()
                                .max_w(px(640.))
                                .flex()
                                .flex_col()
                                .gap_3()
                                .children(error)
                                .child(body),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The text field for the edit in progress, or the clickable `text`.
    pub(super) fn editable_text(
        &self,
        id: String,
        what: Edit,
        text: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let initial = text.clone();
        self.editable_text_with(id, what, text, initial, cx)
    }

    /// Like `editable_text`, with the text the field starts from when it
    /// differs from what is shown.
    pub(super) fn editable_text_with(
        &self,
        id: String,
        what: Edit,
        text: String,
        initial: String,
        window_cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(input) = self.editing(&what) {
            return div()
                .id("settings-edit")
                .debug_selector(|| "settings-edit".into())
                .flex_1()
                .min_w_0()
                .h(px(26.))
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(content(0.14))
                .flex()
                .items_center()
                .child(div().flex_1().min_w_0().child(input))
                .into_any_element();
        }
        let current = initial;
        target(id)
            .aria_label(format!("Edit {text}"))
            .flex_1()
            .min_w_0()
            .truncate()
            .text_size(px(13.))
            .cursor_text()
            .child(text)
            .on_click(window_cx.listener(move |this, _, window, cx| {
                this.start_edit(what.clone(), &current, window, cx)
            }))
            .into_any_element()
    }
}
