//! The Board app view, over the same `board*` tables the Tauri app uses.
use crate::theme::*;
use gpui::{App, Context, EventEmitter, FocusHandle, Focusable, Role, Window, div, prelude::*, px};
use std::path::PathBuf;

pub(crate) enum Event {
    Close,
}

pub(crate) struct BoardView {
    #[allow(dead_code)]
    db_path: PathBuf,
    focus: FocusHandle,
}

impl EventEmitter<Event> for BoardView {}
impl Focusable for BoardView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BoardView {
    pub(crate) fn new(db_path: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            db_path,
            focus: cx.focus_handle(),
        }
    }
}

impl gpui::Render for BoardView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("board-view")
            .debug_selector(|| "board-view".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(Event::Close);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(content(0.5))
                    .child("No boards yet"),
            )
            .child(
                div()
                    .id("board-create")
                    .debug_selector(|| "board-create".into())
                    .role(Role::Button)
                    .aria_label("Create board")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.1))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child("Create board"),
            )
    }
}
