//! Review comments written in the diff, the way GitHub and Paseo do them:
//! the "+" beside a line starts a comment there; dragging from it across
//! lines, or shift-clicking another line's "+", picks a range. The comment
//! box opens inline under the last picked line, and saved drafts stay there
//! as pending cards with Edit and Delete until the review is sent.
//!
//! Only a pull request's Files turns this on ([`DiffView::review_comments`]);
//! elsewhere a line comment is text for the chat.
use super::*;
use gespan_core::inbox::{LineComment, LineRef};
use gpui::Entity;

/// A line by where it sits in its file's diff: block, then line.
pub(super) type At = (usize, usize);

/// Lines being picked with the mouse, or the range the open box is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pick {
    pub file: usize,
    pub anchor: At,
    pub head: At,
}
impl Pick {
    fn range(&self) -> (At, At) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
    pub(super) fn holds(&self, file: usize, at: At) -> bool {
        let (start, end) = self.range();
        self.file == file && start <= at && at <= end
    }
}

pub(super) struct Editor {
    pub pick: Pick,
    /// The draft being edited, or `None` for a new one.
    pub editing: Option<usize>,
    pub input: Entity<Input>,
    _events: Vec<Subscription>,
}

#[derive(Default)]
pub(super) struct Review {
    pub drafts: Vec<LineComment>,
    /// A drag from a "+" in progress.
    pub picking: Option<Pick>,
    pub editor: Option<Editor>,
}

const PAD: f32 = 8.;
const GAP: f32 = 6.;
const EDITOR: f32 = 150.;
const CARD: f32 = 58.;
const CARD_LINE: f32 = 18.;
/// Characters a card's line holds before it wraps, for its height.
const CARD_CHARS: usize = 100;

fn card_height(body: &str) -> f32 {
    let lines: usize = body
        .lines()
        .map(|line| line.chars().count().div_ceil(CARD_CHARS).max(1))
        .sum::<usize>()
        .max(1);
    CARD + CARD_LINE * (lines.saturating_sub(1)) as f32
}

/// Where a line sits in the old and new file, even when it is in only one:
/// an added line's old position is the next old line, as GitLab counts.
pub(super) fn line_ref(blocks: &[Block], at: At) -> Option<LineRef> {
    let lines = blocks.get(at.0)?.lines();
    let line = lines.get(at.1)?;
    if !matches!(line.kind, LineKind::Add | LineKind::Del | LineKind::Context) {
        return None;
    }
    let before =
        |pick: fn(&Line) -> Option<u32>| lines[..at.1].iter().rev().find_map(pick).map(|n| n + 1);
    let after = |pick: fn(&Line) -> Option<u32>| lines[at.1..].iter().find_map(pick);
    let old_pos = line
        .old_number
        .or_else(|| before(|l| l.old_number))
        .or_else(|| after(|l| l.old_number))
        .unwrap_or(1);
    let new_pos = line
        .new_number
        .or_else(|| before(|l| l.new_number))
        .or_else(|| after(|l| l.new_number))
        .unwrap_or(1);
    Some(LineRef::new(
        line.old_number,
        line.new_number,
        old_pos,
        new_pos,
    ))
}

/// The line a draft ends on, in a file's diff.
fn draft_at(blocks: &[Block], draft: &LineComment) -> Option<At> {
    blocks.iter().enumerate().find_map(|(b, block)| {
        block.lines().iter().enumerate().find_map(|(i, line)| {
            let number = match draft.end.side {
                gespan_core::inbox::Side::New => line.new_number,
                gespan_core::inbox::Side::Old if line.kind == LineKind::Del => line.old_number,
                gespan_core::inbox::Side::Old => None,
            };
            (number == Some(draft.end.line)).then_some((b, i))
        })
    })
}

impl DiffView {
    /// The drafts written in this diff, in the order they were added.
    pub(crate) fn drafts(&self) -> &[LineComment] {
        &self.review.drafts
    }
    /// Replaces the drafts (another pull request, or after sending).
    pub(crate) fn set_drafts(&mut self, drafts: Vec<LineComment>, cx: &mut Context<Self>) {
        self.review.drafts = drafts;
        self.review.editor = None;
        self.review.picking = None;
        self.rebuild(cx);
    }

    /// What sits under a line: the drafts that end there, and whether the
    /// open box does.
    pub(super) fn thread_at(&self, file: usize, at: At) -> (Vec<usize>, bool) {
        let Some(diff) = self.files.get(file) else {
            return (vec![], false);
        };
        let drafts = self
            .review
            .drafts
            .iter()
            .enumerate()
            .filter(|(_, draft)| draft.path == diff.path)
            .filter(|(_, draft)| draft_at(&diff.blocks, draft) == Some(at))
            .map(|(ix, _)| ix)
            .collect();
        let editor = self
            .review
            .editor
            .as_ref()
            .is_some_and(|editor| editor.pick.file == file && editor.pick.range().1 == at);
        (drafts, editor)
    }
    /// A thread row's height, or `None` when nothing sits under the line.
    pub(super) fn thread_height(&self, file: usize, at: At) -> Option<f32> {
        let (drafts, editor) = self.thread_at(file, at);
        let editing = self.review.editor.as_ref().and_then(|e| e.editing);
        let cards: Vec<f32> = drafts
            .iter()
            .filter(|ix| Some(**ix) != editing)
            .map(|ix| card_height(&self.review.drafts[*ix].body))
            .collect();
        let blocks = cards.len() + usize::from(editor);
        if blocks == 0 {
            return None;
        }
        Some(
            cards.iter().sum::<f32>()
                + if editor { EDITOR } else { 0. }
                + GAP * (blocks - 1) as f32
                + PAD * 2.,
        )
    }
    /// Lines with a draft on them light their number, as a hint the
    /// thread under them is theirs.
    pub(super) fn has_draft(&self, file: usize, at: At) -> bool {
        let Some(diff) = self.files.get(file) else {
            return false;
        };
        self.review.drafts.iter().any(|draft| {
            draft.path == diff.path && {
                let end = draft_at(&diff.blocks, draft);
                let start = draft.range_start().and_then(|start| {
                    draft_at(
                        &diff.blocks,
                        &LineComment {
                            end: start.clone(),
                            ..Default::default()
                        },
                    )
                });
                match (start, end) {
                    (Some(start), Some(end)) => start <= at && at <= end,
                    (None, Some(end)) => end == at,
                    _ => false,
                }
            }
        })
    }
    /// Picked: being dragged over, or under the open box.
    pub(super) fn picked(&self, file: usize, at: At) -> bool {
        self.review.picking.is_some_and(|pick| pick.holds(file, at))
            || self
                .review
                .editor
                .as_ref()
                .is_some_and(|editor| editor.pick.holds(file, at))
    }

    /// The "+" pressed: a new pick, or with shift the open box's range
    /// stretched to this line.
    pub(super) fn press_line(&mut self, file: usize, at: At, shift: bool, cx: &mut Context<Self>) {
        if shift
            && let Some(editor) = self.review.editor.as_mut()
            && editor.pick.file == file
            && editor.editing.is_none()
        {
            editor.pick.head = at;
            self.rebuild(cx);
            return;
        }
        self.review.picking = Some(Pick {
            file,
            anchor: at,
            head: at,
        });
        cx.notify();
    }
    /// Dragging over a line of the same file while a pick is held.
    pub(super) fn drag_over(&mut self, file: usize, at: At, cx: &mut Context<Self>) {
        if let Some(pick) = self.review.picking.as_mut()
            && pick.file == file
            && pick.head != at
        {
            pick.head = at;
            cx.notify();
        }
    }
    /// Released: the box opens on what was picked.
    pub(super) fn release_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pick) = self.review.picking.take() else {
            return;
        };
        self.open_editor(pick, None, String::new(), window, cx);
    }

    fn open_editor(
        &mut self,
        pick: Pick,
        editing: Option<usize>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = Input::commit_message(cx);
            input.set_placeholder("Leave a comment…", cx);
            input.set_text(text, cx);
            input
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        let events = vec![
            cx.subscribe(&input, |_, _, _: &input::Changed, cx| cx.notify()),
            cx.subscribe_in(
                &input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.save_editor(window, cx),
            ),
        ];
        self.review.editor = Some(Editor {
            pick,
            editing,
            input,
            _events: events,
        });
        self.rebuild(cx);
    }
    pub(super) fn edit_draft(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.review.drafts.get(ix).cloned() else {
            return;
        };
        let Some(file) = self.files.iter().position(|f| f.path == draft.path) else {
            return;
        };
        let Some(end) = draft_at(&self.files[file].blocks, &draft) else {
            return;
        };
        let start = draft
            .range_start()
            .and_then(|start| {
                draft_at(
                    &self.files[file].blocks,
                    &LineComment {
                        end: start.clone(),
                        ..Default::default()
                    },
                )
            })
            .unwrap_or(end);
        let pick = Pick {
            file,
            anchor: start,
            head: end,
        };
        self.open_editor(pick, Some(ix), draft.body, window, cx);
    }
    pub(crate) fn delete_draft(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.review.drafts.len() {
            self.review.drafts.remove(ix);
            if let Some(editor) = self.review.editor.as_mut() {
                editor.editing = match editor.editing {
                    Some(e) if e == ix => None,
                    Some(e) if e > ix => Some(e - 1),
                    other => other,
                };
            }
            cx.emit(DiffEvent::DraftsChanged);
            self.rebuild(cx);
        }
    }
    pub(super) fn cancel_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.editor.take().is_some() {
            window.focus(&self.focus, cx);
            self.rebuild(cx);
        }
    }
    /// Saves the box as a draft on its lines: a new one, or the one edited.
    pub(super) fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.review.editor.as_ref() else {
            return;
        };
        let body = editor.input.read(cx).content.trim().to_string();
        if body.is_empty() {
            return;
        }
        let pick = editor.pick;
        let editing = editor.editing;
        let Some(file) = self.files.get(pick.file) else {
            return;
        };
        let (start, end) = pick.range();
        let (Some(start), Some(end)) = (line_ref(&file.blocks, start), line_ref(&file.blocks, end))
        else {
            return;
        };
        let comment = LineComment {
            path: file.path.clone(),
            start: (start != end).then_some(start),
            end,
            body,
        };
        match editing.and_then(|ix| self.review.drafts.get_mut(ix)) {
            Some(draft) => *draft = comment,
            None => self.review.drafts.push(comment),
        }
        self.review.editor = None;
        window.focus(&self.focus, cx);
        cx.emit(DiffEvent::DraftsChanged);
        self.rebuild(cx);
    }

    /// The row under a line: its pending drafts, then the box when it is open
    /// there.
    pub(super) fn thread_row(
        &mut self,
        file: usize,
        at: At,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (drafts, editor_here) = self.thread_at(file, at);
        let editing = self.review.editor.as_ref().and_then(|e| e.editing);
        let gutter = self.gutter_width();
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .py(px(PAD))
            .pl(px(gutter + 12.))
            .pr(px(24.))
            .max_w(px(gutter + 760.));
        for ix in drafts {
            if Some(ix) == editing {
                continue;
            }
            let draft = &self.review.drafts[ix];
            column = column.child(
                div()
                    .debug_selector(move || format!("diff-draft-{ix}"))
                    .h(px(card_height(&draft.body)))
                    .flex_none()
                    .overflow_hidden()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .bg(content(0.05))
                    .border_1()
                    .border_color(content(0.1))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(11.5))
                            .text_color(content(0.6))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(0.9))
                                    .child("You"),
                            )
                            .child(
                                div()
                                    .px(px(6.))
                                    .rounded_full()
                                    .bg(rgb(0xe8c547).into_hsla_opacity(0.18))
                                    .text_color(rgb(0xe8c547))
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Pending"),
                            )
                            .child(draft.lines_label())
                            .child(div().flex_1())
                            .child(
                                tool_button_owned(
                                    format!("diff-draft-edit-{ix}"),
                                    "Edit comment",
                                    "pencil-line",
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| this.edit_draft(ix, window, cx),
                                )),
                            )
                            .child(
                                tool_button_owned(
                                    format!("diff-draft-delete-{ix}"),
                                    "Delete comment",
                                    "trash",
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.delete_draft(ix, cx)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(CARD_LINE))
                            .text_color(content(0.92))
                            .child(draft.body.clone()),
                    ),
            );
        }
        if editor_here && let Some(editor) = self.review.editor.as_ref() {
            let diff = &self.files[editor.pick.file];
            let (start, end) = editor.pick.range();
            let label = match (line_ref(&diff.blocks, start), line_ref(&diff.blocks, end)) {
                (Some(start), Some(end)) if start != end => {
                    format!("Comment on lines {} to {}", start.line, end.line)
                }
                (_, Some(end)) => format!("Comment on line {}", end.line),
                _ => "Comment".into(),
            };
            let can_save = !editor.input.read(cx).content.trim().is_empty();
            let editing = editor.editing.is_some();
            column = column.child(
                div()
                    .debug_selector(|| "diff-review-editor".into())
                    .h(px(EDITOR))
                    .flex_none()
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .bg(crate::theme::background_base().opacity(0.9))
                    .border_1()
                    .border_color(accent_color().opacity(0.45))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.cancel_editor(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .font_family(crate::fonts::monospace())
                            .text_size(px(11.))
                            .text_color(content(0.55))
                            .child(format!("{} · {label}", diff.path)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(content(0.1))
                            .bg(content(0.03))
                            .text_size(px(13.))
                            .child(editor.input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(10.5))
                                    .text_color(content(0.4))
                                    .child(if cfg!(target_os = "macos") {
                                        "⌘↩ to save · Esc to cancel"
                                    } else {
                                        "Ctrl+↩ to save · Esc to cancel"
                                    }),
                            )
                            .child(
                                div()
                                    .id("diff-review-cancel")
                                    .debug_selector(|| "diff-review-cancel".into())
                                    .role(Role::Button)
                                    .aria_label("Cancel")
                                    .h(px(28.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .text_size(px(12.))
                                    .text_color(content(0.7))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(content(0.08)))
                                    .child("Cancel")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_editor(window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .id("diff-review-save")
                                    .debug_selector(|| "diff-review-save".into())
                                    .role(Role::Button)
                                    .aria_label(if editing {
                                        "Save comment"
                                    } else {
                                        "Add review comment"
                                    })
                                    .h(px(28.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded_md()
                                    .bg(content(1.))
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(crate::theme::background_base())
                                    .when(!can_save, |d| d.opacity(0.4))
                                    .child(
                                        icon("comment-add", 14.)
                                            .text_color(crate::theme::background_base()),
                                    )
                                    .child(if editing {
                                        "Save"
                                    } else {
                                        "Add review comment"
                                    })
                                    .when(can_save, |d| {
                                        d.cursor_pointer().on_click(cx.listener(
                                            |this, _, window, cx| this.save_editor(window, cx),
                                        ))
                                    }),
                            ),
                    ),
            );
        }
        div()
            .id(("diff-thread", at.0 * 100_000 + at.1))
            .debug_selector(move || format!("diff-thread-{file}-{}-{}", at.0, at.1))
            .h(px(height))
            .w_full()
            .overflow_hidden()
            .border_t_1()
            .border_b_1()
            .border_color(content(0.06))
            .bg(content(0.02))
            .child(column)
            .into_any_element()
    }
}

/// A small icon button with an id made at run time.
fn tool_button_owned(
    id: String,
    label: &'static str,
    glyph: &'static str,
) -> gpui::Stateful<gpui::Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .role(Role::Button)
        .aria_label(label)
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.1)))
        .child(icon(glyph, 12.).text_color(content(0.65)))
}

trait Opacity {
    fn into_hsla_opacity(self, alpha: f32) -> Hsla;
}
impl Opacity for gpui::Rgba {
    fn into_hsla_opacity(self, alpha: f32) -> Hsla {
        Hsla::from(self).opacity(alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::inbox::Side;

    fn lines(spec: &[(LineKind, Option<u32>, Option<u32>)]) -> Vec<Block> {
        vec![Block::Hunk {
            chunk: None,
            lines: spec
                .iter()
                .map(|(kind, old, new)| Line {
                    kind: *kind,
                    text: String::new(),
                    old_number: *old,
                    new_number: *new,
                    chunk: None,
                })
                .collect(),
        }]
    }

    #[test]
    fn lines_know_their_place_in_both_files_as_gitlab_counts() {
        let blocks = lines(&[
            (LineKind::Context, Some(9), Some(10)),
            (LineKind::Del, Some(10), None),
            (LineKind::Add, None, Some(11)),
            (LineKind::Add, None, Some(12)),
            (LineKind::Context, Some(11), Some(13)),
        ]);
        let at = |i| line_ref(&blocks, (0, i)).unwrap();
        assert_eq!(
            (at(0).line, at(0).side, at(0).old_pos, at(0).new_pos),
            (10, Side::New, 9, 10)
        );
        assert_eq!(
            (at(1).line, at(1).side, at(1).old_pos, at(1).new_pos),
            (10, Side::Old, 10, 11)
        );
        // An added line sits before the next old line.
        assert_eq!((at(2).old_pos, at(2).new_pos), (11, 11));
        assert_eq!((at(3).old_pos, at(3).new_pos), (11, 12));
        assert_eq!(line_ref(&blocks, (0, 9)), None);
        assert_eq!(line_ref(&blocks, (3, 0)), None);
    }

    #[test]
    fn a_pick_holds_its_range_whichever_way_it_was_dragged() {
        let up = Pick {
            file: 1,
            anchor: (0, 5),
            head: (0, 2),
        };
        assert!(up.holds(1, (0, 2)) && up.holds(1, (0, 4)) && up.holds(1, (0, 5)));
        assert!(!up.holds(1, (0, 6)) && !up.holds(0, (0, 3)));
        assert_eq!(up.range(), ((0, 2), (0, 5)));
    }

    #[test]
    fn card_heights_grow_with_their_text() {
        assert_eq!(card_height("short"), CARD);
        assert_eq!(card_height("one\ntwo"), CARD + CARD_LINE);
        assert_eq!(
            card_height(&"x".repeat(CARD_CHARS * 2 + 1)),
            CARD + CARD_LINE * 2.
        );
    }
}
