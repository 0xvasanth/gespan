//! Select the rendered transcript, retaining byte anchors instead of shaped
//! history. The selection engine is shared with every content surface in
//! [`crate::selectable`]; the transcript adds response scoping, copying from
//! its rows (its list is virtualised) and the selection menu.
use super::*;
#[cfg(test)]
pub(crate) use crate::selectable::Position;
pub(super) use crate::selectable::Selection;
pub(crate) use crate::selectable::TextSelection;
use crate::selectable::snap;
use gpui::{AnyElement, MouseButton, Point};

const MENU_DELAY: std::time::Duration = std::time::Duration::from_millis(300);
/// The top centre of the selection's first line, as React anchors to the
/// range's first client rect rather than the pointer.
fn menu_anchor(state: &crate::selectable::State) -> Option<Point<Pixels>> {
    let (a, b) = (state.anchor?, state.head?);
    let (start, end) = if a < b { (a, b) } else { (b, a) };
    let fragment = state.layouts.values().find(|f| {
        f.response == state.response
            && f.row == start.row
            && f.offset <= start.byte
            && start.byte < f.offset + f.text.len()
    })?;
    let top = fragment
        .layout
        .position_for_index(start.byte - fragment.offset)?;
    let right = (end.row == start.row && end.byte <= fragment.offset + fragment.text.len())
        .then(|| {
            fragment
                .layout
                .position_for_index(end.byte - fragment.offset)
        })
        .flatten()
        .filter(|p| p.y == top.y)
        .map_or(fragment.layout.bounds().right(), |p| p.x);
    Some(gpui::point((top.x + right) / 2., top.y))
}
impl TranscriptView {
    pub(super) fn selection_turn(&self) -> Option<usize> {
        let state = self.selection.0.borrow();
        state.menu?;
        self.messages
            .iter()
            .find(|m| m.start == state.response)
            .map(|m| m.turn)
    }
    /// The selection context for one rendered row of `response`. Documents
    /// select and copy like chat, but "Add to chat/notes" are conversation
    /// actions, so `quote` is dropped there and their menu offers Copy alone.
    pub(super) fn text_selection(
        &self,
        row: (usize, usize),
        response: usize,
        quote: bool,
        _cx: &Context<Self>,
    ) -> Option<TextSelection> {
        Some(TextSelection::new(
            self.selection.clone(),
            row,
            response,
            quote && !self.document,
        ))
    }
    pub(super) fn move_selection(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.drag_to(event, window, cx) {
            cx.notify()
        }
    }
    pub(super) fn finish_selection(
        &mut self,
        event: &gpui::MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((anchor, head)) = self.selection.finish() else {
            return;
        };
        cx.notify();
        if anchor == head {
            return;
        }
        // Wait out a quick select-and-go before offering the menu, and drop
        // it if the selection moved on in the meantime.
        let fallback = event.position;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MENU_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let mut state = this.selection.0.borrow_mut();
                if state.drag.is_some() || state.anchor != anchor || state.head != head {
                    return;
                }
                state.menu = Some(menu_anchor(&state).unwrap_or(fallback));
                drop(state);
                cx.notify();
            });
        })
        .detach();
    }
    /// Where the painted text holding `needle` sits, so tests can point the
    /// mouse at real message text.
    #[cfg(test)]
    pub(crate) fn text_bounds_for_test(&self, needle: &str) -> Option<Bounds<Pixels>> {
        self.selection.bounds_for_test(needle)
    }
    #[cfg(test)]
    pub(crate) fn selected_text_for_test(&self) -> Option<String> {
        self.selected_text()
    }
    pub(super) fn selected_text(&self) -> Option<String> {
        let state = self.selection.0.borrow();
        let (a, b) = (state.anchor?, state.head?);
        if a == b {
            return None;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let mut parts = Vec::new();
        for row in self.rows.iter().filter(|row| {
            row.key >= a.row
                && row.key <= b.row
                && self.messages[row.message].start == state.response
        }) {
            let text = match &row.body {
                Body::Markdown(md) => crate::markdown::selection_text(md),
                Body::Plain(text) => text.to_string(),
                Body::Interjection(note) => note.body().to_owned(),
                // Tool rows and other chrome with selectable text copy what
                // they painted this frame.
                _ => match state.layouts.values().find(|f| f.row == row.key) {
                    Some(fragment) => fragment.row_text.to_string(),
                    None => continue,
                },
            };
            let start = if row.key == a.row {
                snap(&text, a.byte)
            } else {
                0
            };
            let end = if row.key == b.row {
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
    pub(super) fn copy_selection(
        &mut self,
        _: &crate::input::Copy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.stop_propagation()
        }
    }
    pub(super) fn selection_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.selection.0.borrow();
        let position = state.menu?;
        let quote = state.quote;
        let response = state.response;
        drop(state);
        let text = self.selected_text()?;
        let copy = text.clone();
        let note = text.clone();
        let turn = self
            .messages
            .iter()
            .find(|m| m.start == response)
            .map_or(response, |m| m.turn);
        let button = |id: &'static str, label: &'static str, glyph: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label)
                .flex()
                .items_center()
                .gap_2()
                .h(px(32.))
                .px_2p5()
                .rounded(px(8.))
                .text_size(px(13.))
                .hover(|s| s.bg(content(0.05)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(icon(glyph, 14.))
                .child(label)
        };
        let panel = div()
            .id("transcript-selection-actions")
            .role(Role::Toolbar)
            .aria_label("Selected text actions")
            .p_1()
            .min_w(px(144.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                // Every action closes the menu; the copied text stays
                // highlighted until the next press, as a text field leaves it.
                button("selection-copy", "Copy", "copy").on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                        this.selection.hide_menu();
                        cx.notify()
                    },
                )),
            )
            .when(quote, |d| {
                d.child(
                    button("selection-chat", "Add to chat", "comment-add").on_click(cx.listener(
                        move |this, _, window, cx| {
                            if this.pilot {
                                cx.emit(PilotResponseAction::Quote {
                                    text: text.clone(),
                                    window: window.window_handle(),
                                })
                            } else {
                                cx.emit(ResponseRequest::QuoteSelection { text: text.clone() })
                            }
                            this.selection.clear();
                            cx.notify();
                        },
                    )),
                )
            })
            .when(quote, |d| {
                d.child(
                    button("selection-note", "Add to notes", "response-note").on_click(
                        cx.listener(move |this, _, _, cx| {
                            if this.note_pending.insert(turn) {
                                cx.emit(ResponseRequest::SaveNote {
                                    turn,
                                    text: note.clone(),
                                    selection: true,
                                });
                            }
                            this.selection.hide_menu();
                            cx.notify()
                        }),
                    ),
                )
            })
            .when_some(
                self.note_feedback
                    .get(&turn)
                    .and_then(|result| result.as_ref().err()),
                |d, error| {
                    d.child(
                        div()
                            .id("selection-note-error")
                            .role(Role::Alert)
                            .max_w(px(280.))
                            .px_2p5()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(rgb(0xf87171))
                            .child(format!("Could not save note: {error}")),
                    )
                },
            );
        let weak = cx.weak_entity();
        Some(
            popover("transcript-selection", Anchor::Point(position))
                .allow_outside_press()
                .side(Side::Top)
                .align(crate::popover::Align::Center)
                .child(panel)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.selection.0.borrow_mut().menu = None;
                        cx.notify()
                    });
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selectable::word;
    use gpui::TestAppContext;
    #[gpui::test]
    fn rendered_selection_crosses_markdown_and_code_without_chrome_or_other_responses(
        cx: &mut TestAppContext,
    ) {
        let view = cx.new(|_| TranscriptView::new());
        view.update(cx,|view,cx|{
            view.patch(0,vec![Row{role:"assistant".into(),text:"Hello **café**.\n\n```rs\nlet x = 1;\n```\n\nTail".into(),first:true,sent_at:None},Row{role:"user".into(),text:"Next".into(),first:true,sent_at:None},Row{role:"assistant".into(),text:"Other response".into(),first:true,sent_at:None}],cx);
            let first=view.rows.iter().find(|row|matches!(&row.body,Body::Markdown(md) if crate::markdown::selection_text(md)=="Hello café.")).unwrap().key;
            let code=view.rows.iter().find(|row|matches!(&row.body,Body::Markdown(md) if matches!(md.part,Part::Code{..}))).unwrap().key;
            {let mut selection=view.selection.0.borrow_mut();selection.response=0;selection.quote=true;selection.anchor=Some(Position{row:first,byte:6});selection.head=Some(Position{row:code,byte:10});}
            assert_eq!(view.selected_text().as_deref(),Some("café.\nlet x = 1;"));
            {let mut selection=view.selection.0.borrow_mut();let anchor=selection.anchor;selection.anchor=selection.head;selection.head=anchor;}
            assert_eq!(view.selected_text().as_deref(),Some("café.\nlet x = 1;"));
            view.selection.clear();assert!(view.selected_text().is_none());
        });
    }
    #[test]
    fn selection_ranges_are_direction_independent_and_response_scoped() {
        let selection = Selection::default();
        {
            let mut state = selection.0.borrow_mut();
            state.response = 5;
            state.anchor = Some(Position {
                row: (5, 2),
                byte: 8,
            });
            state.head = Some(Position {
                row: (5, 0),
                byte: 3,
            });
        }
        assert_eq!(selection.range((5, 0), 5, 0, 10), 3..10);
        assert_eq!(selection.range((5, 1), 5, 0, 20), 0..20);
        assert_eq!(selection.range((5, 2), 5, 4, 20), 0..4);
        assert_eq!(selection.range((5, 1), 6, 0, 20), 0..0);
    }
    #[test]
    fn unicode_anchors_do_not_split_composed_graphemes() {
        assert_eq!(snap("e\u{301}🙂", 2), 0);
        assert_eq!(snap("e\u{301}🙂", 4), 3);
        assert_eq!(word("hello café", 7), 6..11);
    }
    #[gpui::test]
    fn documents_are_selectable_and_copy_without_chat_actions(cx: &mut TestAppContext) {
        let view =
            cx.new(|cx| TranscriptView::document("# QA note\n\nSelect **this** text.".into(), cx));
        view.update(cx, |view, cx| {
            let row = view
                .rows
                .iter()
                .find(|row| matches!(&row.body, Body::Markdown(md) if crate::markdown::selection_text(md) == "Select this text."))
                .unwrap();
            let (key, response) = (row.key, view.messages[row.message].start);
            let selection = view.text_selection(key, response, true, cx).expect("documents select");
            assert!(!selection.quote, "documents offer Copy alone");
            {
                let mut state = view.selection.0.borrow_mut();
                state.response = response;
                state.anchor = Some(Position { row: key, byte: 7 });
                state.head = Some(Position { row: key, byte: 11 });
                state.menu = Some(gpui::point(px(10.), px(10.)));
            }
            assert_eq!(view.selected_text().as_deref(), Some("this"));
            assert!(view.selection_menu(cx).is_some());
        });
    }
    fn divider(custom_type: &str, text: &str) -> Row {
        Row {
            role: "interjection".into(),
            text: serde_json::to_string(&gespan_core::transcript::Interjection {
                text: text.into(),
                custom_type: custom_type.into(),
                severity: None,
            })
            .unwrap(),
            first: true,
            sent_at: None,
        }
    }
    /// A divider shows only its label, which is not selectable text, so a
    /// selection across it copies the rows around it and not the agent's
    /// name kept in the imported divider's raw text.
    #[gpui::test]
    fn copying_across_a_divider_copies_none_of_its_raw_text(cx: &mut TestAppContext) {
        for (custom_type, text) in [
            (gespan_core::session_import::IMPORTED_TYPE, "Claude Code"),
            (gespan_core::outside_turns::MARKER_TYPE, ""),
        ] {
            let view = cx.new(|_| TranscriptView::new());
            view.update(cx, |view, cx| {
                let plain = |role: &str, text: &str| Row {
                    role: role.into(),
                    text: text.into(),
                    first: true,
                    sent_at: None,
                };
                view.patch(
                    0,
                    vec![
                        plain("assistant", "Earlier reply"),
                        divider(custom_type, text),
                        plain("user", "Fix it"),
                        plain("assistant", "Fixed."),
                    ],
                    cx,
                );
                let key = |text: &str| {
                    view.rows
                        .iter()
                        .find(|row| match &row.body {
                            Body::Markdown(md) => crate::markdown::selection_text(md) == text,
                            Body::Plain(plain) => plain.as_ref() == text,
                            _ => false,
                        })
                        .unwrap()
                        .key
                };
                let (before, after) = (key("Earlier reply"), key("Fix it"));
                let divider = view
                    .rows
                    .iter()
                    .find(|row| matches!(row.body, Body::Interjection(_)))
                    .unwrap();
                let response = view.messages[divider.message].start;
                {
                    let mut state = view.selection.0.borrow_mut();
                    state.response = response;
                    state.anchor = Some(Position {
                        row: before,
                        byte: 0,
                    });
                    state.head = Some(Position {
                        row: after,
                        byte: 6,
                    });
                }
                assert_eq!(
                    view.selected_text(),
                    None,
                    "{custom_type} divider copies nothing"
                );
                view.selection.0.borrow_mut().response = view
                    .messages
                    .iter()
                    .find(|m| m.role == "user")
                    .unwrap()
                    .start;
                assert_eq!(view.selected_text().as_deref(), Some("Fix it"));
            });
        }
    }

    /// Pilot chat is the transcript in Pilot mode: its messages select and
    /// copy the same way.
    #[gpui::test]
    fn pilot_chat_messages_select_with_a_drag_and_copy(cx: &mut TestAppContext) {
        use crate::selectable::test_support::{copy, cursor_at, drag};
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            crate::input::bind(cx);
        });
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = TranscriptView::new();
                    view.pilot_mode("cat", 0xe86b5f, "PilotCat");
                    view.patch(
                        0,
                        vec![
                            Row {
                                role: "user".into(),
                                text: "What shipped today?".into(),
                                first: true,
                                sent_at: None,
                            },
                            Row {
                                role: "assistant".into(),
                                text: "Two fixes shipped to main this morning.".into(),
                                first: true,
                                sent_at: None,
                            },
                        ],
                        cx,
                    );
                    view
                })
            })
            .unwrap()
        });
        let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let bounds = |needle: &str, cx: &mut gpui::VisualTestContext| {
            window
                .update(cx, |view, _, _| view.text_bounds_for_test(needle))
                .unwrap()
                .unwrap_or_else(|| panic!("{needle:?} is selectable"))
        };
        let fixes = bounds("fixes", cx);
        let main = bounds("main", cx);
        assert_eq!(
            cursor_at(cx, fixes.center()),
            Some(gpui::CursorStyle::IBeam)
        );
        drag(
            cx,
            gpui::point(fixes.left() + px(1.), fixes.center().y),
            gpui::point(main.right() + px(2.), main.center().y),
        );
        assert_eq!(copy(cx).as_deref(), Some("fixes shipped to main"));
        let asked = bounds("shipped today", cx);
        drag(
            cx,
            gpui::point(asked.left() + px(1.), asked.center().y),
            gpui::point(asked.right() + px(2.), asked.center().y),
        );
        assert_eq!(copy(cx).as_deref(), Some("shipped today"));
    }

    /// A transcript with one exchange, Pilot or not, and its events.
    fn menu_chat(
        pilot: bool,
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<TranscriptView>,
        std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    ) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            crate::input::bind(cx);
        });
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (chat, pilot_seen) = (seen.clone(), seen.clone());
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = TranscriptView::new();
                    if pilot {
                        view.pilot_mode("cat", 0xe86b5f, "PilotCat");
                    }
                    view.patch(
                        0,
                        vec![
                            Row {
                                role: "user".into(),
                                text: "What shipped today?".into(),
                                first: true,
                                sent_at: None,
                            },
                            Row {
                                role: "assistant".into(),
                                text: "Two fixes shipped to main this morning.".into(),
                                first: true,
                                sent_at: None,
                            },
                        ],
                        cx,
                    );
                    cx.subscribe_self(move |_, event: &ResponseRequest, _| {
                        chat.borrow_mut().push(match event {
                            ResponseRequest::QuoteSelection { text } => format!("quote:{text}"),
                            ResponseRequest::SaveNote { text, .. } => format!("note:{text}"),
                            _ => "other".into(),
                        })
                    })
                    .detach();
                    cx.subscribe_self(move |_, event: &PilotResponseAction, _| {
                        if let PilotResponseAction::Quote { text, .. } = event {
                            pilot_seen.borrow_mut().push(format!("pilot-quote:{text}"))
                        }
                    })
                    .detach();
                    view
                })
            })
            .unwrap()
        });
        (window, seen)
    }
    /// Drags across `from`..`to` and waits out the menu's delay.
    fn select_for_menu(
        window: gpui::WindowHandle<TranscriptView>,
        from: &str,
        to: &str,
        cx: &mut gpui::VisualTestContext,
    ) {
        let bounds = |needle: &str, cx: &mut gpui::VisualTestContext| {
            window
                .update(cx, |view, _, _| view.text_bounds_for_test(needle))
                .unwrap()
                .unwrap_or_else(|| panic!("{needle:?} is selectable"))
        };
        let (a, b) = (bounds(from, cx), bounds(to, cx));
        crate::selectable::test_support::drag(
            cx,
            gpui::point(a.left() + px(1.), a.center().y),
            gpui::point(b.right() + px(2.), b.center().y),
        );
        cx.executor().advance_clock(MENU_DELAY * 2);
        cx.run_until_parked();
        let state = window
            .update(cx, |view, _, _| {
                let st = view.selection.0.borrow();
                format!(
                    "{:?} {:?} drag={} menu={:?} text={:?}",
                    st.anchor,
                    st.head,
                    st.drag.is_some(),
                    st.menu,
                    view.selected_text()
                )
            })
            .unwrap();
        assert!(menu_open(cx), "the menu opens over the selection: {state}");
    }
    fn menu_open(cx: &mut gpui::VisualTestContext) -> bool {
        cx.debug_bounds("selection-copy").is_some()
    }
    fn press(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let at = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
        crate::selectable::test_support::click(cx, at, 1);
    }

    #[gpui::test]
    fn copy_closes_the_menu_and_copies(cx: &mut TestAppContext) {
        for pilot in [false, true] {
            let (window, _) = menu_chat(pilot, cx);
            let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            cx.run_until_parked();
            select_for_menu(window, "fixes", "main", cx);
            press(cx, "selection-copy");
            assert!(!menu_open(cx), "pilot={pilot}: Copy closes the menu");
            assert_eq!(
                cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
                Some("fixes shipped to main")
            );
        }
    }

    #[gpui::test]
    fn add_to_chat_closes_the_menu_and_quotes(cx: &mut TestAppContext) {
        for pilot in [false, true] {
            let (window, seen) = menu_chat(pilot, cx);
            let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            cx.run_until_parked();
            select_for_menu(window, "fixes", "main", cx);
            press(cx, "selection-chat");
            assert!(!menu_open(cx), "pilot={pilot}: Add to chat closes the menu");
            let expected = if pilot {
                "pilot-quote:fixes shipped to main"
            } else {
                "quote:fixes shipped to main"
            };
            assert_eq!(seen.borrow().as_slice(), [expected.to_string()]);
        }
    }

    #[gpui::test]
    fn add_to_notes_closes_the_menu(cx: &mut TestAppContext) {
        let (window, seen) = menu_chat(false, cx);
        let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        select_for_menu(window, "fixes", "main", cx);
        press(cx, "selection-note");
        assert!(!menu_open(cx));
        assert_eq!(
            seen.borrow().as_slice(),
            ["note:fixes shipped to main".to_string()]
        );
    }

    #[gpui::test]
    fn a_click_elsewhere_escape_and_a_cleared_selection_close_the_menu(cx: &mut TestAppContext) {
        for pilot in [false, true] {
            let (window, _) = menu_chat(pilot, cx);
            let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            cx.run_until_parked();
            // A click elsewhere: on other text in the chat.
            select_for_menu(window, "fixes", "main", cx);
            let other = window
                .update(cx, |view, _, _| view.text_bounds_for_test("shipped today"))
                .unwrap()
                .unwrap();
            crate::selectable::test_support::click(cx, other.center(), 1);
            cx.executor().advance_clock(MENU_DELAY * 2);
            cx.run_until_parked();
            assert!(!menu_open(cx), "pilot={pilot}: a click elsewhere closes it");
            // Escape.
            select_for_menu(window, "fixes", "main", cx);
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            assert!(!menu_open(cx), "pilot={pilot}: Escape closes it");
            // The selection clearing, as a scroll or resize clears it.
            select_for_menu(window, "fixes", "main", cx);
            window
                .update(cx, |view, _, cx| {
                    view.selection.clear();
                    cx.notify()
                })
                .unwrap();
            cx.run_until_parked();
            assert!(
                !menu_open(cx),
                "pilot={pilot}: a cleared selection closes it"
            );
        }
    }

    #[gpui::test]
    fn repeated_select_then_act_cycles_open_and_close_the_menu_each_time(cx: &mut TestAppContext) {
        for pilot in [false, true] {
            let (window, seen) = menu_chat(pilot, cx);
            let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            cx.run_until_parked();
            for round in 0..4 {
                let (from, to) = if round % 2 == 0 {
                    ("fixes", "main")
                } else {
                    ("Two", "fixes")
                };
                select_for_menu(window, from, to, cx);
                press(
                    cx,
                    if round % 2 == 0 {
                        "selection-copy"
                    } else {
                        "selection-chat"
                    },
                );
                // Nothing reopens it once the delay has passed again.
                cx.executor().advance_clock(MENU_DELAY * 2);
                cx.run_until_parked();
                assert!(!menu_open(cx), "pilot={pilot} round {round}");
            }
            assert_eq!(
                seen.borrow().len(),
                2,
                "pilot={pilot}: one quote per Add to chat"
            );
        }
    }
}
