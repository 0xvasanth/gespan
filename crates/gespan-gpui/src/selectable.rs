//! Selectable text for every content surface: an I-beam over text, a drag
//! that selects across fragments, double and triple clicks that pick a word
//! and a line, and Cmd/Ctrl+C.
//!
//! A [`Selection`] holds anchors as row and byte positions, so selections
//! survive re-renders without keeping shaped text. Each painted fragment of
//! text records its layout for the frame; a drag hit-tests those layouts.
//! The transcript drives one [`Selection`] itself, because its list is
//! virtualised and copies from its rows. Every other surface uses a
//! [`SelectionScope`]: it numbers its text in render order and wraps its body
//! in [`SelectionScope::region`], which starts a drag anywhere inside the
//! body (from the nearest text), so gaps, padding and clickable rows no
//! longer swallow it.
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Element,
    ElementId, EntityId, FocusHandle, Global, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, SharedString, Styled, StyledText,
    TextLayout, Window, div, px, size,
};
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Position {
    pub(crate) row: (usize, usize),
    pub(crate) byte: usize,
}
#[derive(Clone, Copy)]
pub(crate) enum Granularity {
    Character,
    Word,
    Paragraph,
}
impl Granularity {
    fn from_clicks(count: usize) -> Self {
        match count {
            0 | 1 => Self::Character,
            2 => Self::Word,
            _ => Self::Paragraph,
        }
    }
}
pub(crate) struct Drag {
    origin: Point<Pixels>,
    anchor: Range<Position>,
    granularity: Granularity,
    pub(crate) moved: bool,
    /// Started off the text: focus waits for a real drag, so a plain click
    /// in a gap leaves the composer or list focused.
    focus_on_move: bool,
}
#[derive(Default)]
pub(crate) struct State {
    pub(crate) anchor: Option<Position>,
    pub(crate) head: Option<Position>,
    pub(crate) response: usize,
    pub(crate) quote: bool,
    pub(crate) drag: Option<Drag>,
    pub(crate) menu: Option<Point<Pixels>>,
    pub(crate) layouts: HashMap<((usize, usize), usize, usize), Fragment>,
    owner: Option<EntityId>,
    focus: Option<FocusHandle>,
    /// A [`SelectionScope`]'s next row number, and whether its region painted
    /// since, which starts the next render's numbering again from zero.
    next_row: usize,
    painted: bool,
}
pub(crate) struct Fragment {
    pub(crate) row: (usize, usize),
    pub(crate) response: usize,
    pub(crate) offset: usize,
    pub(crate) quote: bool,
    pub(crate) text: SharedString,
    pub(crate) row_text: SharedString,
    pub(crate) layout: TextLayout,
}
/// Set while the current mouse gesture has dragged out a selection, so a
/// clickable row under it does not also open on the mouse up.
#[derive(Default)]
struct DragSelected(bool);
impl Global for DragSelected {}
/// Whether the press that is ending dragged out a selection. Rows that open
/// on click check this, so a drag across them selects instead.
pub(crate) fn drag_selected(cx: &App) -> bool {
    cx.try_global::<DragSelected>().is_some_and(|g| g.0)
}
fn set_drag_selected(value: bool, cx: &mut App) {
    if drag_selected(cx) != value {
        cx.set_global(DragSelected(value));
    }
}
/// Forget the last gesture at the start of a new press. Registered in the
/// capture phase, so it runs before any row sees the press.
pub(crate) fn begin_gesture(cx: &mut App) {
    set_drag_selected(false, cx);
}
fn reset_gesture(window: &mut Window) {
    window.on_mouse_event(|_: &MouseDownEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            set_drag_selected(false, cx);
        }
    });
}

#[derive(Clone, Default)]
pub(crate) struct Selection(pub(crate) Rc<RefCell<State>>);
impl Selection {
    /// The view that repaints when the selection changes, and the focus that
    /// takes Cmd/Ctrl+C.
    pub(crate) fn bind(&self, owner: EntityId, focus: FocusHandle) {
        let mut state = self.0.borrow_mut();
        state.owner = Some(owner);
        state.focus = Some(focus);
    }
    pub(crate) fn hide_menu(&self) {
        let mut state = self.0.borrow_mut();
        state.menu = None;
        state.drag = None;
    }
    pub(crate) fn active(&self) -> bool {
        let state = self.0.borrow();
        state.drag.is_some() || state.menu.is_some() || state.anchor != state.head
    }
    pub(crate) fn dragging(&self) -> bool {
        self.0.borrow().drag.is_some()
    }
    pub(crate) fn clear(&self) {
        let mut state = self.0.borrow_mut();
        state.anchor = None;
        state.head = None;
        state.drag = None;
        state.menu = None;
        state.layouts.clear();
    }
    pub(crate) fn begin_frame(&self) {
        self.0.borrow_mut().layouts.clear();
    }
    pub(crate) fn range(
        &self,
        row: (usize, usize),
        response: usize,
        offset: usize,
        len: usize,
    ) -> Range<usize> {
        let s = self.0.borrow();
        let (Some(a), Some(b)) = (s.anchor, s.head) else {
            return 0..0;
        };
        if response != s.response {
            return 0..0;
        }
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        if row < a.row || row > b.row {
            return 0..0;
        }
        let start = if row == a.row {
            a.byte.saturating_sub(offset).min(len)
        } else {
            0
        };
        let end = if row == b.row {
            b.byte.saturating_sub(offset).min(len)
        } else {
            len
        };
        start.min(end)..end
    }
    fn notify(&self, cx: &mut App) {
        if let Some(owner) = self.0.borrow().owner {
            cx.notify(owner)
        }
    }
    fn focus(&self, window: &mut Window, cx: &mut App) {
        let focus = self.0.borrow().focus.clone();
        if let Some(focus) = focus {
            window.focus(&focus, cx)
        }
    }
    /// Starts a press on `range` of `row`: a caret for one click, the word for
    /// two and the line for three, extending the last anchor with shift.
    #[allow(clippy::too_many_arguments)]
    fn press(
        &self,
        row: (usize, usize),
        response: usize,
        quote: bool,
        range: Range<usize>,
        event: &MouseDownEvent,
        focus_on_move: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut state = self.0.borrow_mut();
        let start = Position {
            row,
            byte: range.start,
        };
        let end = Position {
            row,
            byte: range.end,
        };
        let extend = event.modifiers.shift && state.anchor.is_some() && state.response == response;
        let anchor = if extend {
            let anchor = state.anchor.unwrap();
            anchor..anchor
        } else {
            start..end
        };
        state.anchor = Some(anchor.start);
        state.head = Some(if start < anchor.start { start } else { end });
        state.response = response;
        state.quote = quote;
        state.drag = Some(Drag {
            origin: event.position,
            anchor,
            granularity: Granularity::from_clicks(event.click_count),
            moved: false,
            focus_on_move,
        });
        state.menu = None;
        drop(state);
        if !focus_on_move {
            self.focus(window, cx);
        }
        self.notify(cx);
    }
    fn nearest(
        &self,
        position: Point<Pixels>,
        response: Option<usize>,
    ) -> Option<(Fragment, usize)> {
        let state = self.0.borrow();
        let fragment = state
            .layouts
            .values()
            .filter(|f| response.is_none_or(|r| f.response == r))
            .min_by(|a, b| distance(a, position).total_cmp(&distance(b, position)))?;
        let byte = fragment
            .layout
            .index_for_position(position)
            .unwrap_or_else(|ix| ix);
        Some((
            Fragment {
                row: fragment.row,
                response: fragment.response,
                offset: fragment.offset,
                quote: fragment.quote,
                text: fragment.text.clone(),
                row_text: fragment.row_text.clone(),
                layout: fragment.layout.clone(),
            },
            fragment.offset + byte,
        ))
    }
    /// A press that landed beside the text, in a gap, the padding or a row's
    /// chrome: it anchors at the nearest text, so dragging from there selects.
    pub(crate) fn press_near(&self, event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
        if self.dragging() {
            return;
        }
        let Some((fragment, byte)) = self.nearest(event.position, None) else {
            self.clear();
            self.notify(cx);
            return;
        };
        let granularity = Granularity::from_clicks(event.click_count);
        let range = unit(&fragment.row_text, byte, granularity);
        self.press(
            fragment.row,
            fragment.response,
            fragment.quote,
            range,
            event,
            true,
            window,
            cx,
        );
    }
    /// Moves the head of a drag to the text nearest the pointer. Returns
    /// whether the selection changed.
    pub(crate) fn drag_to(
        &self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let mut state = self.0.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return false;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            state.drag = None;
            return false;
        }
        if !drag.moved {
            let delta = event.position - drag.origin;
            if delta.x.abs().max(delta.y.abs()) < px(3.) {
                return false;
            }
            drag.moved = true;
            let focus = drag.focus_on_move;
            drop(state);
            set_drag_selected(true, cx);
            if focus {
                self.focus(window, cx);
            }
            state = self.0.borrow_mut();
        }
        let drag = state.drag.as_ref().unwrap();
        let anchor = drag.anchor.clone();
        let granularity = drag.granularity;
        let response = state.response;
        drop(state);
        let Some((fragment, byte)) = self.nearest(event.position, Some(response)) else {
            return false;
        };
        let range = unit(&fragment.row_text, byte, granularity);
        let start = Position {
            row: fragment.row,
            byte: range.start,
        };
        let end = Position {
            row: fragment.row,
            byte: range.end,
        };
        let (anchor, head) = if start < anchor.start {
            (anchor.end, start)
        } else {
            (anchor.start, end)
        };
        let mut state = self.0.borrow_mut();
        if state.anchor != Some(anchor) || state.head != Some(head) {
            state.anchor = Some(anchor);
            state.head = Some(head);
            true
        } else {
            false
        }
    }
    /// Ends a drag. Returns the anchors when a press was in progress.
    pub(crate) fn finish(&self) -> Option<(Option<Position>, Option<Position>)> {
        let mut state = self.0.borrow_mut();
        state.drag.take()?;
        state.menu = None;
        Some((state.anchor, state.head))
    }
    /// The selected text, read from this frame's fragments in row order.
    /// Fragments of one row share its text, so a row is copied once.
    pub(crate) fn fragment_text(&self) -> Option<String> {
        let state = self.0.borrow();
        let (a, b) = (state.anchor?, state.head?);
        if a == b {
            return None;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let mut rows: Vec<_> = state
            .layouts
            .values()
            .filter(|f| f.response == state.response && f.row >= a.row && f.row <= b.row)
            .map(|f| (f.row, f.row_text.clone()))
            .collect();
        rows.sort_by_key(|(row, _)| *row);
        rows.dedup_by_key(|(row, _)| *row);
        let mut parts = Vec::new();
        for (row, text) in rows {
            let start = if row == a.row { snap(&text, a.byte) } else { 0 };
            let end = if row == b.row {
                snap(&text, b.byte)
            } else {
                text.len()
            };
            if start <= end {
                parts.push(text[start..end].to_owned())
            }
        }
        let text = parts.join("\n").trim().to_owned();
        (!text.is_empty()).then_some(text)
    }
}
#[cfg(test)]
impl Selection {
    /// Where the painted text holding `needle` sits, so tests can point the
    /// mouse at real text.
    pub(crate) fn bounds_for_test(&self, needle: &str) -> Option<Bounds<Pixels>> {
        let state = self.0.borrow();
        let fragment = state.layouts.values().find(|f| f.text.contains(needle))?;
        let start = fragment.text.find(needle)?;
        let layout = &fragment.layout;
        let a = layout.position_for_index(start)?;
        let b = layout.position_for_index(start + needle.len())?;
        Some(Bounds::new(
            a,
            size(
                if b.y == a.y {
                    b.x - a.x
                } else {
                    layout.bounds().right() - a.x
                },
                layout.line_height(),
            ),
        ))
    }
}
fn distance(f: &Fragment, position: Point<Pixels>) -> f32 {
    let bounds = f.layout.bounds();
    let dx = (bounds.left() - position.x)
        .max(px(0.))
        .max(position.x - bounds.right());
    let dy = (bounds.top() - position.y)
        .max(px(0.))
        .max(position.y - bounds.bottom());
    f32::from(dy) * 10000. + f32::from(dx)
}
pub(crate) fn snap(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .map(|(ix, _)| ix)
        .take_while(|ix| *ix <= byte)
        .last()
        .unwrap_or(0)
}
pub(crate) fn word(text: &str, byte: usize) -> Range<usize> {
    let at = if byte >= text.len() {
        text.grapheme_indices(true)
            .next_back()
            .map_or(0, |(ix, _)| ix)
    } else {
        snap(text, byte)
    };
    text.split_word_bound_indices()
        .find(|(ix, s)| *ix <= at && at < *ix + s.len())
        .map(|(ix, s)| ix..ix + s.len())
        .unwrap_or(at..at)
}
pub(crate) fn unit(text: &str, byte: usize, granularity: Granularity) -> Range<usize> {
    let byte = snap(text, byte);
    match granularity {
        Granularity::Character => byte..byte,
        Granularity::Word => word(text, byte),
        Granularity::Paragraph => {
            let start = text[..byte].rfind(['\n', '\t']).map_or(0, |ix| ix + 1);
            let end = text[byte..]
                .find(['\n', '\t'])
                .map_or(text.len(), |ix| byte + ix);
            start..end
        }
    }
}

/// One selectable piece of text: where it sits in its selection and how it
/// copies.
#[derive(Clone)]
pub(crate) struct TextSelection {
    pub(crate) state: Selection,
    pub(crate) row: (usize, usize),
    pub(crate) response: usize,
    pub(crate) offset: usize,
    pub(crate) quote: bool,
    pub(crate) row_text: Option<SharedString>,
    pub(crate) cursor: CursorStyle,
}
impl TextSelection {
    pub(crate) fn new(state: Selection, row: (usize, usize), response: usize, quote: bool) -> Self {
        Self {
            state,
            row,
            response,
            offset: 0,
            quote,
            row_text: None,
            cursor: CursorStyle::IBeam,
        }
    }
    pub(crate) fn with_text(mut self, text: SharedString) -> Self {
        self.row_text = Some(text);
        self
    }
    pub(crate) fn with_cursor(mut self, cursor: CursorStyle) -> Self {
        self.cursor = cursor;
        self
    }
    pub(crate) fn offset(&self, offset: usize) -> Self {
        let mut next = self.clone();
        next.offset += offset;
        next
    }
    pub(crate) fn has_selection(&self) -> bool {
        let state = self.state.0.borrow();
        state.anchor != state.head || state.drag.as_ref().is_some_and(|drag| drag.moved)
    }
    pub(crate) fn wrap(
        &self,
        text: SharedString,
        layout: TextLayout,
        child: AnyElement,
    ) -> AnyElement {
        let selection = self.clone();
        let click_layout = layout.clone();
        let click_text = text.clone();
        let row_text = self.row_text.clone().unwrap_or_else(|| text.clone());
        div()
            .relative()
            .cursor(self.cursor)
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                let byte = click_layout
                    .index_for_position(event.position)
                    .unwrap_or_else(|ix| ix);
                let byte = snap(&click_text, byte);
                let granularity = Granularity::from_clicks(event.click_count);
                let range = unit(&row_text, selection.offset + byte, granularity);
                selection.state.press(
                    selection.row,
                    selection.response,
                    selection.quote,
                    range,
                    event,
                    false,
                    window,
                    cx,
                );
            })
            .child(SelectedText {
                context: self.clone(),
                text,
                layout,
                child,
            })
            .into_any_element()
    }
}
struct SelectedText {
    context: TextSelection,
    text: SharedString,
    layout: TextLayout,
    child: AnyElement,
}
impl IntoElement for SelectedText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for SelectedText {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
        let context = &self.context;
        context.state.0.borrow_mut().layouts.insert(
            (context.row, context.response, context.offset),
            Fragment {
                row: context.row,
                response: context.response,
                offset: context.offset,
                quote: context.quote,
                text: self.text.clone(),
                row_text: context
                    .row_text
                    .clone()
                    .unwrap_or_else(|| self.text.clone()),
                layout: self.layout.clone(),
            },
        );
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let selected = self.context.state.range(
            self.context.row,
            self.context.response,
            self.context.offset,
            self.text.len(),
        );
        if !selected.is_empty() {
            let bounds = self.layout.bounds();
            let height = self.layout.line_height();
            if let (Some(a), Some(b)) = (
                self.layout.position_for_index(selected.start),
                self.layout.position_for_index(selected.end),
            ) {
                let mut y = a.y;
                while y <= b.y {
                    let left = if y == a.y { a.x } else { bounds.left() };
                    let right = if y == b.y { b.x } else { bounds.right() };
                    if right > left {
                        window.paint_quad(gpui::fill(
                            Bounds::new(gpui::point(left, y), size(right - left, height)),
                            crate::theme::text_selection(),
                        ))
                    }
                    y += height;
                }
            }
        }
        self.child.paint(window, cx);
    }
}

/// The selection of one view's content outside the transcript: Inbox and My
/// tasks details, Runs, Automations, Settings, documents and diffs.
#[derive(Clone)]
pub(crate) struct SelectionScope {
    selection: Selection,
    focus: FocusHandle,
}
impl SelectionScope {
    pub(crate) fn new<V: 'static>(cx: &mut Context<V>) -> Self {
        let selection = Selection::default();
        let focus = cx.focus_handle();
        selection.bind(cx.entity_id(), focus.clone());
        Self { selection, focus }
    }
    fn next_row(&self) -> (usize, usize) {
        let mut state = self.selection.0.borrow_mut();
        if std::mem::take(&mut state.painted) {
            state.next_row = 0;
        }
        state.next_row += 1;
        (0, state.next_row)
    }
    /// The selection for the next piece of text in render order, for
    /// markdown and code renderers that shape their own text.
    pub(crate) fn selection(&self) -> TextSelection {
        TextSelection::new(self.selection.clone(), self.next_row(), 0, false)
    }
    /// The selection for text with a stable place of its own, such as a row
    /// of a lazily painted document: `(block, row)` orders after the text
    /// numbered in render order and keeps its anchor while rows come and go.
    pub(crate) fn keyed(&self, (block, row): (usize, usize)) -> TextSelection {
        TextSelection::new(self.selection.clone(), (block + 1, row), 0, false)
    }
    /// Plain selectable text.
    pub(crate) fn text(&self, text: impl Into<SharedString>) -> AnyElement {
        let text = text.into();
        self.styled(text.clone(), StyledText::new(text))
    }
    /// Selectable text that is already styled, such as highlighted code.
    pub(crate) fn styled(&self, text: SharedString, styled: StyledText) -> AnyElement {
        let layout = styled.layout().clone();
        self.selection()
            .wrap(text, layout, styled.into_any_element())
    }
    /// Whether a press on this scope's text is in progress.
    pub(crate) fn pressed(&self) -> bool {
        self.selection.dragging()
    }
    #[cfg(test)]
    pub(crate) fn bounds_for_test(&self, needle: &str) -> Option<Bounds<Pixels>> {
        self.selection.bounds_for_test(needle)
    }
    /// Wraps the selectable body: a drag can start anywhere in it, the
    /// selection copies with Cmd/Ctrl+C and Escape clears it.
    pub(crate) fn region(
        &self,
        id: impl Into<ElementId>,
        child: impl IntoElement,
    ) -> gpui::Stateful<gpui::Div> {
        let copy = self.selection.clone();
        let escape = self.selection.clone();
        div()
            .id(id)
            .key_context("TranscriptSelection")
            .track_focus(&self.focus)
            .on_action(move |_: &crate::input::Copy, _, cx| {
                if let Some(text) = copy.fragment_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    cx.stop_propagation()
                }
            })
            .on_key_down(move |event, _, cx| {
                if event.keystroke.key == "escape" && escape.active() {
                    escape.clear();
                    escape.notify(cx);
                    cx.stop_propagation()
                }
            })
            .child(Region {
                selection: self.selection.clone(),
                child: child.into_any_element(),
            })
    }
}
impl gpui::Focusable for SelectionScope {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Tracks a drag across a scope's whole body, also outside it once started.
struct Region {
    selection: Selection,
    child: AnyElement,
}
impl IntoElement for Region {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Region {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.selection.begin_frame();
        self.child.prepaint(window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        reset_gesture(window);
        // Registered before the children, so the bubble phase reaches it
        // last: a press on text or a button that stops propagation wins.
        let press = self.selection.clone();
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && hitbox.is_hovered(window)
            {
                press.press_near(event, window, cx);
            }
        });
        self.child.paint(window, cx);
        self.selection.0.borrow_mut().painted = true;
        if self.selection.dragging() {
            window.set_window_cursor_style(CursorStyle::IBeam);
        }
        let drag = self.selection.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && drag.drag_to(event, window, cx) {
                drag.notify(cx);
            }
        });
        let finish = self.selection.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && finish.finish().is_some()
            {
                finish.notify(cx);
            }
        });
    }
}

/// Mouse gestures for selection tests, through the window's real dispatch.
#[cfg(test)]
pub(crate) mod test_support {
    use gpui::{
        CursorStyle, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
        VisualTestContext,
    };
    /// The cursor the window shows with the pointer at `at`.
    pub(crate) fn cursor_at(cx: &mut VisualTestContext, at: Point<Pixels>) -> Option<CursorStyle> {
        cx.simulate_mouse_move(at, None, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, _| window.cursor_style_for_test())
    }
    /// Presses `count` times in a row at `at`, as a double or triple click.
    pub(crate) fn click(cx: &mut VisualTestContext, at: Point<Pixels>, count: usize) {
        cx.simulate_mouse_move(at, None, Modifiers::default());
        for click_count in 1..=count {
            cx.simulate_event(MouseDownEvent {
                position: at,
                button: MouseButton::Left,
                click_count,
                first_mouse: false,
                modifiers: Modifiers::default(),
            });
            cx.simulate_event(MouseUpEvent {
                position: at,
                button: MouseButton::Left,
                click_count,
                modifiers: Modifiers::default(),
            });
        }
        cx.run_until_parked();
    }
    /// Presses at `from`, drags through the midpoint to `to` and releases.
    pub(crate) fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
        cx.simulate_mouse_move(from, None, Modifiers::default());
        cx.simulate_event(MouseDownEvent {
            position: from,
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
            modifiers: Modifiers::default(),
        });
        cx.run_until_parked();
        let mid = gpui::point((from.x + to.x) / 2., (from.y + to.y) / 2.);
        cx.simulate_mouse_move(mid, Some(MouseButton::Left), Modifiers::default());
        cx.run_until_parked();
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
        cx.run_until_parked();
        cx.simulate_event(MouseUpEvent {
            position: to,
            button: MouseButton::Left,
            click_count: 1,
            modifiers: Modifiers::default(),
        });
        cx.run_until_parked();
    }
    /// Copies with the platform's shortcut and returns the clipboard.
    pub(crate) fn copy(cx: &mut VisualTestContext) -> Option<String> {
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        });
        cx.run_until_parked();
        cx.read_from_clipboard().and_then(|item| item.text())
    }
}
