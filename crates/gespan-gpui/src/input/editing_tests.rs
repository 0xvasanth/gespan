//! The editing commands driven through real keystrokes: each binding must
//! reach the editor (GPUI's keymap wins over key-down listeners) and each
//! command must undo in one step.
use super::*;
use gpui::{TestAppContext, VisualTestContext};

fn secondary() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}

/// An editor for `path` holding `marked`, with `|` as the caret or `«…»` as
/// the selection, focused and with an empty history.
fn editor<'a>(
    path: &str,
    marked: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Input>, &'a mut VisualTestContext) {
    cx.update(super::super::bind);
    let (text, selection) = if let Some(at) = marked.find('|') {
        (marked.replacen('|', "", 1), at..at)
    } else {
        let start = marked.find('«').unwrap();
        let text = marked.replacen('«', "", 1);
        let end = text.find('»').unwrap();
        (text.replacen('»', "", 1), start..end)
    };
    let path = path.to_owned();
    let (input, cx) = cx.add_window_view(move |_, cx| {
        let mut input = Input::editor(cx);
        input.set_editor_path(path);
        input.set_text(text, cx);
        input
    });
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            window.focus(&input.focus, cx);
            input.select_range(selection, cx);
            input.reset_editor_history();
        })
    });
    (input, cx)
}

/// The buffer with the caret or selection marked like `editor`'s input.
fn state(input: &Entity<Input>, cx: &mut VisualTestContext) -> String {
    input.read_with(cx, |input, _| {
        let s = input.selection.clone();
        let t = &input.content;
        if s.is_empty() {
            format!("{}|{}", &t[..s.start], &t[s.start..])
        } else {
            format!("{}«{}»{}", &t[..s.start], &t[s.clone()], &t[s.end..])
        }
    })
}

fn undo_steps(input: &Entity<Input>, cx: &mut VisualTestContext) -> usize {
    input.read_with(cx, |input, _| input.code.as_ref().unwrap().undo.len())
}

fn undo(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(&format!("{}-z", secondary()));
}

#[gpui::test]
fn typed_pairs_close_step_over_their_closer_and_delete_together(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.ts", "|", cx);
    cx.simulate_input("f(");
    assert_eq!(state(&input, cx), "f(|)");
    cx.simulate_input("x)");
    assert_eq!(
        state(&input, cx),
        "f(x)|",
        "the inserted closer is typed over"
    );
    cx.simulate_input(";a=[");
    assert_eq!(state(&input, cx), "f(x);a=[|]");
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        state(&input, cx),
        "f(x);a=|",
        "backspace removes both halves"
    );
    cx.simulate_input("`");
    assert_eq!(
        state(&input, cx),
        "f(x);a=`|`",
        "TypeScript pairs backticks"
    );
    undo(cx);
    assert_eq!(state(&input, cx), "f(x);a=|", "one undo removes the pair");
    cx.simulate_input("word");
    cx.simulate_keystrokes("home");
    cx.simulate_input("(");
    assert_eq!(
        state(&input, cx),
        "(|f(x);a=word",
        "no closer before a word character"
    );
}

#[gpui::test]
fn typing_a_quote_over_a_selection_wraps_it(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.py", "x = «name»", cx);
    cx.simulate_input("\"");
    assert_eq!(state(&input, cx), "x = \"«name»\"");
    assert_eq!(undo_steps(&input, cx), 1);
    undo(cx);
    assert_eq!(state(&input, cx), "x = «name»");
}

#[gpui::test]
fn enter_indents_by_language_and_splits_an_empty_pair(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.ts", "function f() {|}", cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(state(&input, cx), "function f() {\n  |\n}");
    cx.simulate_input("x");
    cx.simulate_keystrokes("enter");
    assert_eq!(state(&input, cx), "function f() {\n  x\n  |\n}");
    cx.simulate_input("}");
    assert_eq!(
        state(&input, cx),
        "function f() {\n  x\n}|\n}",
        "a closer typed on a blank line lines up with its opener"
    );
    undo(cx);
    assert_eq!(state(&input, cx), "function f() {\n  x\n  |\n}");

    let (input, cx) = editor("a.py", "def f():\n    if a:|", &mut cx.cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(state(&input, cx), "def f():\n    if a:\n        |");
    cx.simulate_input("pass");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        state(&input, cx),
        "def f():\n    if a:\n        pass\n    |",
        "pass ends the block"
    );
}

#[gpui::test]
fn toggle_comment_comments_and_uncomments_the_selected_lines(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.py", "«a = 1\n  b = 2»\n", cx);
    let toggle = format!("{}-/", secondary());
    cx.simulate_keystrokes(&toggle);
    assert_eq!(state(&input, cx), "# «a = 1\n#   b = 2»\n");
    assert_eq!(undo_steps(&input, cx), 1, "one edit for every line");
    cx.simulate_keystrokes(&toggle);
    assert_eq!(state(&input, cx), "«a = 1\n  b = 2»\n");
    undo(cx);
    undo(cx);
    assert_eq!(state(&input, cx), "«a = 1\n  b = 2»\n");

    let (input, cx) = editor("a.css", "a { b: c }|", &mut cx.cx);
    cx.simulate_keystrokes(&toggle);
    assert_eq!(
        state(&input, cx),
        "/* a { b: c } */|",
        "CSS uses block comments"
    );
}

#[gpui::test]
fn alt_arrows_move_and_copy_lines_and_stop_at_the_edges(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.rs", "one\nt|wo\nthree", cx);
    cx.simulate_keystrokes("alt-up");
    assert_eq!(state(&input, cx), "t|wo\none\nthree");
    cx.simulate_keystrokes("alt-up");
    assert_eq!(state(&input, cx), "t|wo\none\nthree", "already first");
    assert_eq!(undo_steps(&input, cx), 1, "a no-op adds no undo step");
    cx.simulate_keystrokes("alt-down alt-down");
    assert_eq!(state(&input, cx), "one\nthree\nt|wo");
    cx.simulate_keystrokes("alt-down");
    assert_eq!(state(&input, cx), "one\nthree\nt|wo", "already last");
    cx.simulate_keystrokes("shift-alt-down");
    assert_eq!(state(&input, cx), "one\nthree\ntwo\nt|wo");
    cx.simulate_keystrokes("shift-alt-up");
    assert_eq!(state(&input, cx), "one\nthree\ntwo\nt|wo\ntwo");
    undo(cx);
    assert_eq!(state(&input, cx), "one\nthree\ntwo\nt|wo");
    undo(cx);
    assert_eq!(state(&input, cx), "one\nthree\nt|wo");
}

#[gpui::test]
fn delete_line_removes_the_line_in_one_undoable_step(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.rs", "one\nt|wo\nthree", cx);
    let delete = format!("{}-shift-k", secondary());
    cx.simulate_keystrokes(&delete);
    assert_eq!(state(&input, cx), "one\nt|hree");
    cx.simulate_keystrokes(&delete);
    assert_eq!(
        state(&input, cx),
        "o|ne",
        "the last line takes its break with it"
    );
    cx.simulate_keystrokes(&delete);
    assert_eq!(state(&input, cx), "|");
    undo(cx);
    assert_eq!(state(&input, cx), "o|ne");
    undo(cx);
    undo(cx);
    assert_eq!(state(&input, cx), "one\nt|wo\nthree");
}

#[gpui::test]
fn select_to_matching_bracket_and_the_pair_is_highlighted(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.js", "f|(a, [b]);", cx);
    let highlighted = |input: &Entity<Input>, cx: &mut VisualTestContext| {
        input.read_with(cx, |i, _| {
            i.bracket_highlights()
                .into_iter()
                .map(|h| (h.range.start, h.matched))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(highlighted(&input, cx), [(1, true), (8, true)]);
    cx.simulate_keystrokes(&format!("{}-shift-\\", secondary()));
    assert_eq!(state(&input, cx), "f«(a, [b])»;");
    assert_eq!(undo_steps(&input, cx), 0, "selecting is not an edit");

    let (input, cx) = editor("a.js", "x = (]|", &mut cx.cx);
    assert_eq!(
        highlighted(&input, cx),
        [(5, false)],
        "a mismatched bracket is flagged"
    );
    let (input, cx) = editor("a.html", "<ul><l|i>x</li></ul>", &mut cx.cx);
    let names = input.read_with(cx, |i, _| {
        i.bracket_highlights()
            .into_iter()
            .map(|h| i.content[h.range].to_owned())
            .collect::<Vec<_>>()
    });
    assert_eq!(names, ["li", "li"], "the matching tag names");
}

#[gpui::test]
fn editing_shortcuts_follow_keybinding_overrides(cx: &mut TestAppContext) {
    let (input, cx) = editor("a.ts", "x|", cx);
    cx.update(|_, cx| {
        let mut prefs = crate::settings::Preferences::default();
        prefs
            .keybindings
            .insert("Editor: Toggle Comment".into(), Some("ctrl-alt-c".into()));
        crate::settings::apply_keybindings(&prefs, cx);
    });
    cx.simulate_keystrokes(&format!("{}-/", secondary()));
    assert_eq!(state(&input, cx), "x|", "the default no longer toggles");
    cx.simulate_keystrokes("ctrl-alt-c");
    assert_eq!(state(&input, cx), "// x|");
}
