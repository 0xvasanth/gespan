//! Chat text selects like text in a real shell: an I-beam over it, a drag
//! selects (also one that starts beside the text), double and triple clicks
//! pick a word and a line, and Cmd/Ctrl+C copies.
use super::*;
use crate::selectable::test_support::{click, copy, cursor_at, drag};
use gespan_core::store::Store;
use gespan_core::transcript::Row;
use gpui::{CursorStyle, TestAppContext, VisualTestContext, WindowHandle};

const CWD: &str = "/work/gespan";
const REPLY: &str =
    "The quick brown fox jumps over the lazy dog.\n\nSecond paragraph follows here.";

fn shell_with_chat(rows: Vec<Row>, cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let mut summary = store.create("claude", CWD).unwrap();
    summary.title = "Selection chat".into();
    summary.has_messages = true;
    let id = summary.id;
    let store = Arc::new(Mutex::new(store));
    cx.update(|cx| {
        crate::shell::bind_keys(cx);
        crate::input::bind(cx);
    });
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(store, vec![summary], CWD.into(), None, cx);
                shell.ensure_session(id, cx);
                shell
            })
        })
        .unwrap()
    });
    cx.simulate_window_resize(window.into(), gpui::size(px(1400.), px(900.)));
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            shell.show_session(id, w, cx);
            shell.sessions[&id].update(cx, |chat, cx| {
                chat.transcript.update(cx, |t, cx| {
                    t.patch(0, rows, cx);
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    (window, id)
}

fn chat(cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64) {
    shell_with_chat(
        vec![
            Row {
                role: "user".into(),
                text: "Please explain the fox".into(),
                first: true,
                sent_at: None,
            },
            Row {
                role: "assistant".into(),
                text: REPLY.into(),
                first: true,
                sent_at: None,
            },
        ],
        cx,
    )
}

fn bounds(
    window: WindowHandle<Shell>,
    id: i64,
    needle: &str,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id]
                .read(cx)
                .transcript
                .read(cx)
                .text_bounds_for_test(needle)
        })
        .unwrap()
        .unwrap_or_else(|| panic!("{needle:?} is painted as selectable text"))
}

fn selected(window: WindowHandle<Shell>, id: i64, cx: &mut VisualTestContext) -> Option<String> {
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id]
                .read(cx)
                .transcript
                .read(cx)
                .selected_text_for_test()
        })
        .unwrap()
}

fn left(b: Bounds<Pixels>) -> Point<Pixels> {
    gpui::point(b.left() + px(1.), b.center().y)
}
fn right(b: Bounds<Pixels>) -> Point<Pixels> {
    gpui::point(b.right() + px(2.), b.center().y)
}

#[gpui::test]
fn a_drag_across_a_reply_selects_it_and_the_shortcut_copies_exactly_that(cx: &mut TestAppContext) {
    let (window, id) = chat(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    let from = left(bounds(window, id, "quick", cx));
    let to = right(bounds(window, id, "lazy", cx));
    drag(cx, from, to);
    assert_eq!(
        selected(window, id, cx).as_deref(),
        Some("quick brown fox jumps over the lazy")
    );
    assert_eq!(
        copy(cx).as_deref(),
        Some("quick brown fox jumps over the lazy")
    );
    // Dragging backwards selects the same text.
    drag(cx, to, from);
    assert_eq!(
        selected(window, id, cx).as_deref(),
        Some("quick brown fox jumps over the lazy")
    );
}

#[gpui::test]
fn a_drag_that_starts_in_the_gap_between_paragraphs_still_selects(cx: &mut TestAppContext) {
    let (window, id) = chat(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    let first = bounds(window, id, "The quick", cx);
    let second = bounds(window, id, "Second paragraph follows here.", cx);
    // Just below the first paragraph's last line, in the 12px gap no glyph
    // covers: before, the press landed on nothing and the drag never began.
    let gap = gpui::point(first.left() + px(1.), first.bottom() + px(3.));
    assert!(gap.y < second.top(), "the press is in the gap");
    drag(cx, gap, right(second));
    let text = selected(window, id, cx).expect("a drag from the gap selects");
    assert!(text.contains("Second paragraph follows here"), "{text:?}");
    // A plain click in the gap clears it again and selects nothing.
    click(cx, gap, 1);
    assert_eq!(selected(window, id, cx), None);
}

#[gpui::test]
fn double_and_triple_clicks_select_a_word_and_its_line(cx: &mut TestAppContext) {
    let (window, id) = chat(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    let brown = bounds(window, id, "brown", cx).center();
    click(cx, brown, 2);
    assert_eq!(selected(window, id, cx).as_deref(), Some("brown"));
    click(cx, brown, 3);
    assert_eq!(
        selected(window, id, cx).as_deref(),
        Some("The quick brown fox jumps over the lazy dog.")
    );
    // The user's own message selects too.
    let user = bounds(window, id, "explain", cx).center();
    click(cx, user, 2);
    assert_eq!(selected(window, id, cx).as_deref(), Some("explain"));
}

#[gpui::test]
fn the_cursor_is_an_i_beam_over_chat_text_and_a_pointer_over_buttons(cx: &mut TestAppContext) {
    let (window, id) = chat(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    for needle in ["quick", "Please explain", "Second paragraph"] {
        let at = bounds(window, id, needle, cx).center();
        assert_eq!(cursor_at(cx, at), Some(CursorStyle::IBeam), "{needle}");
    }
    let toggle = cx
        .debug_bounds("toggle-right-panel")
        .expect("the title bar's panel toggle");
    assert_eq!(
        cursor_at(cx, toggle.center()),
        Some(CursorStyle::PointingHand)
    );
    // Dragging over a button selects nothing and keeps it unselectable.
    drag(
        cx,
        toggle.center(),
        toggle.center() + gpui::point(px(6.), px(0.)),
    );
    assert_eq!(selected(window, id, cx), None);
}
