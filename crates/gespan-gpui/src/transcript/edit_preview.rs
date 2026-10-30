//! ToolDiffPreview.tsx and FilePreview.tsx: hovering an Edit or Write file
//! chip shows the change the tool itself proposed, independent of git.
use super::*;
use crate::popover::Align;
use crate::right_panel::{DIFF_ADD, DIFF_DEL, format_integer, tabular};
use gespan_core::tool::edit_preview::{EditPreview, LineKind};
use gpui::{AnyElement, Hsla, Task};
use std::time::Duration;

pub(super) const OPEN_DELAY: Duration = Duration::from_millis(300);
pub(super) const CLOSE_DELAY: Duration = Duration::from_millis(180);
const WIDTH: f32 = 460.;
/// `--color-diff-add` and `--color-diff-del`, the solid marker hues.
const ADD_MARK: u32 = 0x10b981;
const DEL_MARK: u32 = 0xf43f5e;

#[derive(Default)]
pub(super) struct Previews {
    open: Option<Open>,
    anchors: HashMap<String, AnchorSlot>,
    opening: Option<(String, Task<()>)>,
    closing: Option<Task<()>>,
    /// What the pointer is over. Entering one target can be reported before
    /// leaving the other, so a late leave never closes what is now hovered.
    over_chip: Option<String>,
    over_card: bool,
}

#[derive(Clone)]
pub(super) struct Open {
    pub key: String,
    pub label: String,
    pub path: Option<String>,
    pub state: ToolState,
    pub preview: EditPreview,
}

impl Open {
    pub(super) fn description(&self) -> &'static str {
        match self.state {
            ToolState::Running => "Proposed changes",
            ToolState::Failed | ToolState::Cancelled => "Attempted changes · tool did not complete",
            ToolState::Completed if self.preview.content_only => {
                "Written content · previous contents unavailable"
            }
            ToolState::Completed => "Change preview",
        }
    }
}

impl TranscriptView {
    pub(super) fn edit_preview_anchor(&mut self, key: &str) -> AnchorSlot {
        self.edit_previews
            .anchors
            .entry(key.to_owned())
            .or_default()
            .clone()
    }
    pub(super) fn hover_edit_chip(&mut self, open: Open, hovered: bool, cx: &mut Context<Self>) {
        let previews = &mut self.edit_previews;
        if !hovered {
            if previews.over_chip.as_deref() == Some(open.key.as_str()) {
                previews.over_chip = None;
            }
            if previews
                .opening
                .as_ref()
                .is_some_and(|(key, _)| *key == open.key)
            {
                previews.opening = None;
            }
            self.close_edit_preview_after_delay(cx);
            return;
        }
        previews.over_chip = Some(open.key.clone());
        previews.closing = None;
        if previews
            .open
            .as_ref()
            .is_some_and(|shown| shown.key == open.key)
            || previews
                .opening
                .as_ref()
                .is_some_and(|(key, _)| *key == open.key)
        {
            return;
        }
        let key = open.key.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OPEN_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.edit_previews.opening = None;
                this.edit_previews.open = Some(open);
                cx.notify();
            });
        });
        previews.opening = Some((key, task));
    }
    fn hover_edit_card(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.edit_previews.over_card = hovered;
        if hovered {
            self.edit_previews.closing = None;
        } else {
            self.close_edit_preview_after_delay(cx);
        }
    }
    fn close_edit_preview_after_delay(&mut self, cx: &mut Context<Self>) {
        if self.edit_previews.open.is_none() {
            return;
        }
        self.edit_previews.closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let previews = &this.edit_previews;
                let still_over = previews.over_card
                    || previews.over_chip.as_ref().is_some_and(|key| {
                        previews.open.as_ref().is_some_and(|open| open.key == *key)
                    });
                if still_over {
                    this.edit_previews.closing = None;
                } else {
                    this.hide_edit_preview(cx);
                }
            });
        }));
    }
    pub(super) fn hide_edit_preview(&mut self, cx: &mut Context<Self>) {
        let previews = &mut self.edit_previews;
        previews.opening = None;
        previews.closing = None;
        previews.over_card = false;
        if previews.open.take().is_some() {
            cx.notify();
        }
    }
    #[cfg(test)]
    pub(super) fn open_edit_preview(&self) -> Option<&Open> {
        self.edit_previews.open.as_ref()
    }

    /// The open chip's popover, below it.
    pub(super) fn edit_preview_card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.edit_previews.open.clone()?;
        let anchor = self.edit_previews.anchors.get(&open.key)?.clone();
        let preview = &open.preview;
        let mono = crate::fonts::monospace();
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(content(0.1))
            .text_size(px(11.))
            .text_color(content(0.5))
            .child(div().min_w_0().flex_1().child(open.description()))
            .child(
                div()
                    .id("edit-preview-close")
                    .role(Role::Button)
                    .aria_label("Close preview")
                    .flex_shrink_0()
                    .rounded(px(4.))
                    .p(px(2.))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                    .child(icon("close", 12.))
                    .on_click(cx.listener(|this, _, _, cx| this.hide_edit_preview(cx))),
            );
        let counts = (preview.additions > 0 || preview.deletions > 0).then(|| {
            div()
                .flex_shrink_0()
                .flex()
                .gap_1()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_features(tabular())
                .when(preview.additions > 0, |d| {
                    d.child(
                        div()
                            .text_color(rgb(DIFF_ADD))
                            .child(format!("+{}", format_integer(preview.additions as i64))),
                    )
                })
                .when(preview.deletions > 0, |d| {
                    d.child(
                        div()
                            .text_color(rgb(DIFF_DEL))
                            .child(format!("-{}", format_integer(preview.deletions as i64))),
                    )
                })
        });
        let file = div()
            .flex()
            .items_center()
            .gap_2()
            .px(px(10.))
            .py_2()
            .child(crate::activity::file_icon(&open.label, 16.))
            .child({
                let name = div()
                    .id("edit-preview-file")
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .font_family(mono)
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(content(0.85))
                    .child(open.label.clone());
                match open.path.clone() {
                    Some(path) => name
                        .role(Role::Link)
                        .aria_label(format!("Open {path}"))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(0x7dd3fc)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.hide_edit_preview(cx);
                            window.dispatch_action(
                                Box::new(crate::file_editor::OpenPath(path.clone())),
                                cx,
                            );
                        })),
                    None => name,
                }
            })
            .children(counts);
        let lines = div()
            .id("edit-preview-lines")
            .debug_selector(|| "edit-preview-lines".into())
            .aria_label("Preview lines")
            .max_h(px(180.))
            .overflow_scroll()
            .border_t_1()
            .border_color(content(0.1))
            .font_family(mono)
            .text_size(px(12.))
            .line_height(px(18.))
            .when(preview.content_only && preview.lines.is_empty(), |d| {
                d.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(content(0.5))
                        .child("Empty file"),
                )
            })
            .children(preview.lines.iter().map(|line| {
                let (bg, bar, mark) = match line.kind {
                    LineKind::Add => (
                        Some(Hsla::from(rgb(ADD_MARK)).opacity(0.15)),
                        Some(ADD_MARK),
                        "+",
                    ),
                    LineKind::Del => (
                        Some(Hsla::from(rgb(DEL_MARK)).opacity(0.15)),
                        Some(DEL_MARK),
                        "−",
                    ),
                    LineKind::Context => (None, None, " "),
                };
                div()
                    .relative()
                    .flex()
                    .items_baseline()
                    .when_some(bg, |d, bg| d.bg(bg))
                    .when_some(bar, |d, bar| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .w(px(2.))
                                .bg(rgb(bar)),
                        )
                    })
                    .child(
                        div()
                            .w(px(28.))
                            .flex_shrink_0()
                            .pr_1()
                            .text_right()
                            .text_size(px(10.))
                            .text_color(content(0.35))
                            .child(line.number.map_or_else(|| " ".into(), |n| n.to_string())),
                    )
                    .child(
                        div()
                            .w(px(12.))
                            .flex_shrink_0()
                            .text_center()
                            .text_size(px(10.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(match line.kind {
                                LineKind::Add => rgb(DIFF_ADD).into(),
                                LineKind::Del => rgb(DIFF_DEL).into(),
                                LineKind::Context => content(0.),
                            })
                            .child(mark),
                    )
                    .child(
                        div()
                            .pr_3()
                            .whitespace_nowrap()
                            .text_color(content(0.8))
                            .child(line.text.clone()),
                    )
            }));
        let card = div()
            .id("edit-preview")
            .debug_selector(|| "edit-preview".into())
            .role(Role::Dialog)
            .aria_label(format!("{}: {}", open.description(), open.label))
            .min_w_0()
            .flex()
            .flex_col()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.hover_edit_card(*hovered, cx)))
            .child(header)
            .child(file)
            .when(preview.content_only || !preview.lines.is_empty(), |d| {
                d.child(lines)
            });
        let dismiss = cx.weak_entity();
        Some(
            popover("edit-preview-popover", Anchor::Slot(anchor))
                .side(Side::Bottom)
                .align(Align::Start)
                .gap(4.)
                .width(WIDTH)
                .max_height(280.)
                .solid()
                .allow_outside_press()
                .on_dismiss(move |_, cx| {
                    let _ = dismiss.update(cx, |this, cx| this.hide_edit_preview(cx));
                })
                .child(card)
                .into_any_element(),
        )
    }
}
