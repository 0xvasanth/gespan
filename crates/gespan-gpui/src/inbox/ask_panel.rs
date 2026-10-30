//! Ask: a chat about the selected item, beside its detail. The panel is only a
//! host for a Conversation the shell owns, so a turn keeps running after the
//! panel closes or moves to another item.
use super::*;
use crate::conversation::Conversation;
use gpui::{
    AnyElement, CursorStyle, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Role, Window, canvas, div, prelude::*,
};

const DEFAULT_WIDTH: f32 = 440.;
const MIN_WIDTH: f32 = 360.;
/// Kept free for the list and the detail view.
const RESERVED_WIDTH: f32 = 650.;
/// Below this the panel takes over the view, as the Tauri app's does.
const OVERLAY_BELOW: f32 = 1100.;

struct Hosted {
    key: String,
    session: i64,
    view: Entity<Conversation>,
}

pub(super) struct AskPanel {
    open: bool,
    width: f32,
    /// Pointer and width where a drag on the left edge began.
    drag: Option<(f32, f32)>,
    /// The conversation asked of the shell, which `hosted` then answers.
    requested: Option<String>,
    hosted: Option<Hosted>,
    error: Option<String>,
    focus_composer: bool,
}

impl Default for AskPanel {
    fn default() -> Self {
        Self {
            open: false,
            width: DEFAULT_WIDTH,
            drag: None,
            requested: None,
            hosted: None,
            error: None,
            focus_composer: false,
        }
    }
}

fn clamp_width(width: f32, viewport: f32) -> f32 {
    width.clamp(MIN_WIDTH, (viewport - RESERVED_WIDTH).max(MIN_WIDTH))
}

impl Inbox {
    pub(super) fn open_ask(&mut self, cx: &mut Context<Self>) {
        self.ask.open = true;
        self.ask.focus_composer = true;
        self.sync_ask(cx);
        cx.notify();
    }
    fn close_ask(&mut self, cx: &mut Context<Self>) {
        self.ask.open = false;
        self.ask.requested = None;
        self.ask.hosted = None;
        self.ask.error = None;
        self.ask.drag = None;
        cx.emit(Event::AskClosed);
        cx.notify();
    }
    /// Keeps the open panel on the selected item's conversation.
    pub(super) fn sync_ask(&mut self, cx: &mut Context<Self>) {
        if !self.ask.open {
            return;
        }
        let Some(mut item) = self.item() else {
            return;
        };
        let key = model::ask::key(&item);
        if self.ask.requested.as_ref() == Some(&key) {
            return;
        }
        // The description is about to land; a new session wants it saved.
        if model::ask::wants_description(&item) && self.details.is_none() && self.detail_loading {
            self.ask.hosted = None;
            return;
        }
        if item.project_path.is_empty() {
            item.project_path = self.start_project.to_string_lossy().into_owned();
        }
        self.ask.requested = Some(key);
        self.ask.hosted = None;
        self.ask.error = None;
        let description = model::ask::wants_description(&item)
            .then(|| self.details.as_ref().map(|details| details.body.clone()))
            .flatten();
        cx.emit(Event::Ask {
            item: Box::new(item),
            description,
        });
        cx.notify();
    }
    fn restart_ask(&mut self, cx: &mut Context<Self>) {
        if let Some(hosted) = self.ask.hosted.take() {
            cx.emit(Event::RestartAsk {
                key: hosted.key,
                session: hosted.session,
            });
        } else if self.ask.error.take().is_some() {
            self.ask.requested = None;
            self.sync_ask(cx);
        }
        self.ask.focus_composer = true;
        cx.notify();
    }
    /// The shell answers with the conversation it owns. A late answer for an
    /// item no longer selected is dropped; its conversation carries on.
    pub(crate) fn host_ask(
        &mut self,
        key: String,
        session: i64,
        view: Entity<Conversation>,
        cx: &mut Context<Self>,
    ) {
        if self.ask.requested.as_ref() == Some(&key) {
            self.ask.hosted = Some(Hosted { key, session, view });
            cx.notify();
        }
    }
    pub(crate) fn ask_failed(&mut self, key: String, error: String, cx: &mut Context<Self>) {
        if self.ask.requested.as_ref() == Some(&key) {
            self.ask.hosted = None;
            self.ask.error = Some(error);
            cx.notify();
        }
    }
    /// The conversation on screen while the inbox is.
    pub(crate) fn shown_ask_session(&self) -> Option<i64> {
        self.ask
            .hosted
            .as_ref()
            .filter(|_| self.ask.open)
            .map(|hosted| hosted.session)
    }
    /// Brings up the item a notification's Ask conversation belongs to.
    pub(crate) fn reveal_ask(&mut self, key: &str, cx: &mut Context<Self>) {
        let item = self
            .snapshot
            .items
            .iter()
            .find(|item| model::ask::key(item) == key)
            .map(Item::key);
        if let Some(item) = item {
            self.select(item, cx);
            self.open_ask(cx);
        }
    }
    fn drag_ask(&mut self, x: f32, viewport: f32, cx: &mut Context<Self>) {
        if let Some((start, width)) = self.ask.drag {
            // The panel grows leftward, away from the window's edge.
            self.ask.width = clamp_width(width + start - x, viewport);
            cx.notify();
        }
    }

    pub(super) fn ask_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.ask.open {
            return None;
        }
        let item = self.item()?;
        if let Some(hosted) = self.ask.hosted.as_ref().filter(|_| self.ask.focus_composer) {
            self.ask.focus_composer = false;
            let focus = hosted.view.read(cx).input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        let viewport = f32::from(window.viewport_size().width);
        let overlay = viewport < OVERLAY_BELOW;
        let reference = item.reference();
        let inbox = cx.entity().downgrade();
        let waiting = self.ask.hosted.is_none() && self.ask.error.is_none();
        let panel = div()
            .id("inbox-ask-panel")
            .debug_selector(|| "inbox-ask-panel".into())
            .role(Role::Region)
            .aria_label(format!("Ask about {reference}"))
            .relative()
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(content(0.07))
            .when(overlay, |d| {
                d.absolute().top_0().left_0().size_full().bg(shell_fill())
            })
            .when(!overlay, |d| d.w(px(clamp_width(self.ask.width, viewport))))
            // Esc belongs to this chat, not to the inbox around it.
            .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("Ask · {reference}")),
                    )
                    .child(
                        icon_button_enabled(
                            "inbox-ask-restart",
                            "Restart conversation",
                            "rotate-ccw",
                            !waiting,
                        )
                        .debug_selector(|| "inbox-ask-restart".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !waiting {
                                this.restart_ask(cx);
                            }
                        })),
                    )
                    .child(
                        icon_button("inbox-ask-close", "Close panel", "panel-right")
                            .debug_selector(|| "inbox-ask-close".into())
                            .on_click(cx.listener(|this, _, _, cx| this.close_ask(cx))),
                    ),
            )
            .when_some(self.ask.error.clone(), |d, text| {
                d.child(
                    div()
                        .p(px(12.))
                        .text_size(px(12.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(text),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .children(self.ask.hosted.as_ref().map(|hosted| hosted.view.clone())),
            )
            .when(!overlay, |d| {
                d.child(
                    div()
                        .id("inbox-ask-resize")
                        .debug_selector(|| "inbox-ask-resize".into())
                        .role(Role::Splitter)
                        .aria_label("Resize discussion")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-4.))
                        .w(px(8.))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if event.click_count == 2 {
                                    this.ask.drag = None;
                                    this.ask.width = DEFAULT_WIDTH;
                                } else {
                                    let width = clamp_width(this.ask.width, viewport);
                                    this.ask.drag = Some((f32::from(event.position.x), width));
                                }
                                cx.notify();
                            }),
                        ),
                )
            })
            // The drag follows the pointer anywhere in the window, including
            // over the chat and the rails around the inbox.
            .when(self.ask.drag.is_some(), |d| {
                d.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let moved = inbox.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, _, window, cx| {
                                let viewport = f32::from(window.viewport_size().width);
                                let x = f32::from(event.position.x);
                                let _ = moved.update(cx, |this, cx| this.drag_ask(x, viewport, cx));
                            });
                            let ended = inbox.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                                let _ = ended.update(cx, |this, cx| {
                                    if this.ask.drag.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            });
        Some(panel.into_any_element())
    }
}

#[cfg(test)]
impl Inbox {
    pub(crate) fn ask_width(&self) -> f32 {
        self.ask.width
    }
    pub(crate) fn ask_open(&self) -> bool {
        self.ask.open
    }
    pub(crate) fn ask_conversation(&self) -> Option<Entity<Conversation>> {
        self.ask.hosted.as_ref().map(|hosted| hosted.view.clone())
    }
    pub(crate) fn ask_error(&self) -> Option<String> {
        self.ask.error.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_width_stays_between_the_minimum_and_what_the_window_leaves() {
        assert_eq!(clamp_width(100., 1600.), MIN_WIDTH);
        assert_eq!(clamp_width(500., 1600.), 500.);
        assert_eq!(clamp_width(5000., 1600.), 950.);
        // A window too small for both still shows the whole minimum.
        assert_eq!(clamp_width(500., 800.), MIN_WIDTH);
    }
}
