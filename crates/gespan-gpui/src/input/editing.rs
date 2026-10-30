//! The code editor's editing commands: auto-closing pairs, Enter indentation,
//! comment toggling, line moves and bracket/tag matching. The text rules live
//! in `gespan_core::files::editing`; this applies each result as one edit, so
//! one undo reverts it.
use super::*;
use gespan_core::files::editing::{self, Edit, Language};

actions!(
    code_editor,
    [
        ToggleComment,
        MoveLineUp,
        MoveLineDown,
        CopyLineUp,
        CopyLineDown,
        DeleteLine,
        SelectToMatchingBracket
    ]
);

pub(super) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-/", ToggleComment, Some("CodeEditor")),
        KeyBinding::new("alt-up", MoveLineUp, Some("CodeEditor")),
        KeyBinding::new("alt-down", MoveLineDown, Some("CodeEditor")),
        KeyBinding::new("shift-alt-up", CopyLineUp, Some("CodeEditor")),
        KeyBinding::new("shift-alt-down", CopyLineDown, Some("CodeEditor")),
        KeyBinding::new("secondary-shift-k", DeleteLine, Some("CodeEditor")),
        KeyBinding::new(
            "secondary-shift-\\",
            SelectToMatchingBracket,
            Some("CodeEditor"),
        ),
    ]);
}

/// A bracket or tag name next to the caret, and whether it has a partner.
pub(super) struct Highlight {
    pub range: Range<usize>,
    pub matched: bool,
}

impl Input {
    fn code_language(&self) -> Option<Language> {
        self.code.as_ref().map(|code| editing::language(&code.path))
    }
    /// Sorts offsets into code (0), strings (1) and comments (2) from the
    /// last analysis, so brackets only pair within their own kind.
    fn bracket_class(&self) -> impl Fn(usize) -> u8 + '_ {
        let tokens = self
            .code
            .as_ref()
            .map_or(&[][..], |code| &code.analysis.tokens[..]);
        move |at| {
            let end = tokens.partition_point(|t| t.from <= at);
            tokens[..end]
                .iter()
                .rev()
                .take(4)
                .find(|t| at < t.to)
                .map_or(0, |t| {
                    if t.kind.contains("syntax-string") {
                        1
                    } else if t.kind.contains("syntax-comment") {
                        2
                    } else {
                        0
                    }
                })
        }
    }
    /// Applies one command result as a single undo step. An edit that only
    /// moves the caret leaves the history alone.
    fn apply_code_edit(&mut self, edit: Edit, typing: bool, cx: &mut Context<Self>) -> bool {
        if self.read_only || self.is_composing() {
            return false;
        }
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        if edit.is_move() {
            self.selection = self.clip_range(edit.selection);
            self.reversed = false;
            self.reveal_code();
            cx.notify();
            return true;
        }
        let range = self.clip_range(edit.range.clone());
        if range != edit.range || !self.accept_code_edit(&range, &edit.text, cx) {
            return false;
        }
        if let Some(limit) = self.max_chars {
            let length = self.content[..range.start].encode_utf16().count()
                + edit.text.encode_utf16().count()
                + self.content[range.end..].encode_utf16().count();
            if length > limit {
                return false;
            }
        }
        self.record_undo();
        self.map_code_folds(&range, edit.text.len());
        self.content = edit.apply(&self.content).into();
        self.code_changed();
        self.selection = self.clip_range(edit.selection);
        self.reversed = false;
        if let Some(code) = &mut self.code {
            code.auto_closed.extend(edit.closer);
            if !typing {
                code.suppress_completion = true;
            }
        }
        cx.emit(Changed);
        cx.notify();
        true
    }
    /// Typed text that opens, wraps or steps over a pair, or re-indents a
    /// closer. Returns whether it was handled here.
    pub(super) fn type_code(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(language) = self.code_language() else {
            return false;
        };
        if self.read_only || self.is_composing() {
            return false;
        }
        let range = self.replacement_range(range);
        if range != self.selection {
            return false;
        }
        let unit = editing::indent_unit(&self.content, &language);
        let closed = self.code.as_ref().map_or(&[][..], |c| &c.auto_closed[..]);
        let edit = editing::type_text(&language, &self.content, range, text, &unit, &|at| {
            closed.contains(&at)
        });
        edit.is_some_and(|edit| self.apply_code_edit(edit, true, cx))
    }
    /// Enter in the editor: language-aware indentation.
    pub(super) fn code_newline(&mut self, cx: &mut Context<Self>) {
        let Some(language) = self.code_language() else {
            return;
        };
        let unit = editing::indent_unit(&self.content, &language);
        let selection = self.clip_range(self.selection.clone());
        let edit = editing::newline(&language, &self.content, selection, &unit);
        self.apply_code_edit(edit, true, cx);
    }
    /// Backspace inside an empty pair deletes both halves.
    pub(super) fn code_delete_pair(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(language) = self.code_language() else {
            return false;
        };
        let selection = self.clip_range(self.selection.clone());
        editing::delete_pair(&language, &self.content, selection)
            .is_some_and(|edit| self.apply_code_edit(edit, true, cx))
    }
    /// Forgets inserted closers once the caret leaves their line, as
    /// CodeMirror does.
    pub(super) fn keep_closers_on_caret_line(&mut self) {
        let at = self.cursor();
        let Some(code) = &mut self.code else {
            return;
        };
        if code.auto_closed.is_empty() {
            return;
        }
        let start = self.content[..at].rfind('\n').map_or(0, |n| n + 1);
        let end = self.content[at..]
            .find('\n')
            .map_or(self.content.len(), |n| at + n);
        code.auto_closed.retain(|p| (start..=end).contains(p));
    }
    fn run_code_command(
        &mut self,
        cx: &mut Context<Self>,
        command: impl FnOnce(&Language, &str, Range<usize>) -> Option<Edit>,
    ) {
        let Some(language) = self.code_language() else {
            cx.propagate();
            return;
        };
        if self.is_composing() {
            return;
        }
        let selection = self.clip_range(self.selection.clone());
        if let Some(edit) = command(&language, &self.content, selection) {
            self.clear_completion();
            self.apply_code_edit(edit, false, cx);
        }
    }
    pub(super) fn toggle_comment(
        &mut self,
        _: &ToggleComment,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_code_command(cx, editing::toggle_comment);
    }
    pub(super) fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.run_code_command(cx, |_, text, s| editing::move_lines(text, s, true));
    }
    pub(super) fn move_line_down(
        &mut self,
        _: &MoveLineDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_code_command(cx, |_, text, s| editing::move_lines(text, s, false));
    }
    pub(super) fn copy_line_up(&mut self, _: &CopyLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.run_code_command(cx, |_, text, s| Some(editing::copy_lines(text, s, true)));
    }
    pub(super) fn copy_line_down(
        &mut self,
        _: &CopyLineDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_code_command(cx, |_, text, s| Some(editing::copy_lines(text, s, false)));
    }
    pub(super) fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        self.run_code_command(cx, |_, text, s| editing::delete_lines(text, s));
    }
    pub(super) fn select_to_matching_bracket(
        &mut self,
        _: &SelectToMatchingBracket,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.code.is_none() {
            cx.propagate();
            return;
        }
        let selection = self.clip_range(self.selection.clone());
        let found = {
            let class = self.bracket_class();
            editing::select_to_matching_bracket(&self.content, selection, &class)
        };
        if let Some(range) = found {
            self.select_range(range, cx);
        }
    }
    /// The bracket or tag pair beside the caret, for painting.
    pub(super) fn bracket_highlights(&self) -> Vec<Highlight> {
        let Some(language) = self.code_language() else {
            return Vec::new();
        };
        if !self.selection.is_empty() {
            return Vec::new();
        }
        let at = self.cursor();
        let class = self.bracket_class();
        let mut out = Vec::new();
        if let Some(found) = editing::match_bracket(&self.content, at, &class) {
            let matched = found.other.is_some();
            out.push(Highlight {
                range: found.at..found.at + 1,
                matched,
            });
            if let Some(other) = found.other {
                out.push(Highlight {
                    range: other..other + 1,
                    matched,
                });
            }
        }
        if language.tags
            && let Some((open, close)) = editing::match_tag(&self.content, at)
        {
            out.push(Highlight {
                range: open,
                matched: true,
            });
            out.push(Highlight {
                range: close,
                matched: true,
            });
        }
        out
    }
}

#[cfg(test)]
#[path = "editing_tests.rs"]
mod tests;
