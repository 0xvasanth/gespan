use super::*;

/// `|` marks the caret; `«…»` marks a selection.
fn parse(marked: &str) -> (String, Range<usize>) {
    if let Some(at) = marked.find('|') {
        let text = marked.replacen('|', "", 1);
        return (text, at..at);
    }
    let start = marked.find('«').expect("caret or selection");
    let text = marked.replacen('«', "", 1);
    let end = text.find('»').expect("selection end");
    (text.replacen('»', "", 1), start..end)
}
fn render(text: &str, selection: &Range<usize>) -> String {
    if selection.is_empty() {
        format!("{}|{}", &text[..selection.start], &text[selection.start..])
    } else {
        format!(
            "{}«{}»{}",
            &text[..selection.start],
            &text[selection.clone()],
            &text[selection.end..]
        )
    }
}
fn applied(text: &str, edit: &Edit) -> String {
    render(&edit.apply(text), &edit.selection)
}
fn never(_: usize) -> bool {
    false
}
/// Types `typed` the way the editor does: the command's edit, or a plain
/// insert when it has none.
fn typing(path: &str, marked: &str, typed: &str, closed: &dyn Fn(usize) -> bool) -> String {
    let (text, selection) = parse(marked);
    let lang = language(path);
    match type_text(&lang, &text, selection.clone(), typed, "  ", closed) {
        Some(edit) => applied(&text, &edit),
        None => {
            let at = selection.start + typed.len();
            render(
                &format!(
                    "{}{typed}{}",
                    &text[..selection.start],
                    &text[selection.end..]
                ),
                &(at..at),
            )
        }
    }
}
fn typed(path: &str, marked: &str, ch: &str) -> String {
    typing(path, marked, ch, &never)
}

#[test]
fn each_default_pair_auto_closes_and_records_its_closer() {
    for (open, close) in [("(", ")"), ("[", "]"), ("{", "}"), ("'", "'"), ("\"", "\"")] {
        let (text, selection) = parse("x = |");
        let edit = type_text(&language("a.rs"), &text, selection, open, "  ", &never).unwrap();
        assert_eq!(applied(&text, &edit), format!("x = {open}|{close}"));
        assert_eq!(edit.closer, Some(5), "the inserted closer is tracked");
    }
}

#[test]
fn language_bracket_lists_follow_the_tauri_language_data() {
    assert_eq!(typed("a.ts", "x = |", "`"), "x = `|`");
    assert_eq!(typed("a.sql", "select |", "`"), "select `|`");
    assert_eq!(
        typed("a.rs", "x = |", "`"),
        "x = `|",
        "Rust does not pair backticks"
    );
    assert_eq!(
        typed("a.json", "|", "("),
        "(|",
        "JSON only pairs brackets, braces and double quotes"
    );
    assert_eq!(typed("a.json", "|", "'"), "'|");
    assert_eq!(typed("a.json", "|", "["), "[|]");
    assert_eq!(typed("a.vue", "|", "("), "(|");
    assert_eq!(typed("a.vue", "|", "{"), "{|}");
}

#[test]
fn opening_brackets_only_close_before_space_end_or_a_closer() {
    assert_eq!(typed("a.js", "f|", "("), "f(|)");
    assert_eq!(typed("a.js", "f| x", "("), "f(|) x", "whitespace after");
    assert_eq!(typed("a.js", "[|]", "("), "[(|)]");
    assert_eq!(typed("a.js", "a|;", "("), "a(|);");
    assert_eq!(
        typed("a.js", "|word", "("),
        "(|word",
        "no pair before a word"
    );
    assert_eq!(typed("a.js", "|.x", "["), "[|.x");
}

#[test]
fn quotes_do_not_pair_beside_word_characters_or_inside_strings() {
    assert_eq!(typed("a.js", "don|", "'"), "don'|", "after a word");
    assert_eq!(
        typed("a.js", "x = |abc", "\""),
        "x = \"|abc",
        "before a word"
    );
    assert_eq!(
        typed("a.js", "x = \"abc|", "\""),
        "x = \"abc\"|",
        "closing an open string"
    );
    assert_eq!(
        typed("a.js", "x = \"a\"|", "\""),
        "x = \"a\"\"|",
        "right after a string"
    );
    assert_eq!(
        typed("a.js", "x = \"it|", "'"),
        "x = \"it'|",
        "inside another string"
    );
    assert_eq!(typed("a.js", "x = (|)", "\""), "x = (\"|\")");
    assert_eq!(
        typed("a.py", "x = 1  # it|", "'"),
        "x = 1  # it'|",
        "word character before it"
    );
}

#[test]
fn string_prefixes_let_quotes_pair_after_them() {
    assert_eq!(typed("a.py", "x = f|", "\""), "x = f\"|\"");
    assert_eq!(typed("a.py", "x = rb|", "'"), "x = rb'|'");
    assert_eq!(
        typed("a.py", "x = elf|", "\""),
        "x = elf\"|",
        "not a prefix"
    );
    assert_eq!(typed("a.rs", "let s = r|", "\""), "let s = r\"|\"");
    assert_eq!(
        typed("a.js", "x = f|", "\""),
        "x = f\"|",
        "JS has no prefixes"
    );
}

#[test]
fn typing_over_an_inserted_closer_steps_over_it() {
    let closed_at = |pos: usize| move |at: usize| at == pos;
    assert_eq!(typing("a.js", "f(|)", ")", &closed_at(2)), "f()|");
    assert_eq!(typing("a.js", "[1|]", "]", &closed_at(2)), "[1]|");
    assert_eq!(
        typing("a.js", "x = \"ab|\"", "\"", &closed_at(7)),
        "x = \"ab\"|"
    );
    // A closer the user typed (not tracked) is not stepped over.
    assert_eq!(typing("a.js", "f(|)", ")", &never), "f()|)");
    assert_eq!(typing("a.js", "x = \"|\"", "\"", &never), "x = \"\"|\"");
}

#[test]
fn python_triple_quotes_open_a_docstring() {
    let closed = |at: usize| at == 1;
    assert_eq!(typed("a.py", "|", "\""), "\"|\"");
    assert_eq!(typing("a.py", "\"|\"", "\"", &closed), "\"\"|");
    assert_eq!(typed("a.py", "\"\"|", "\""), "\"\"\"|\"\"\"");
    assert_eq!(
        typing("a.py", "\"\"\"doc|\"\"\"", "\"", &|at| at == 6),
        "\"\"\"doc\"\"\"|",
        "steps over all three closing quotes"
    );
    assert_eq!(
        typed("a.js", "\"\"|", "\""),
        "\"\"\"|",
        "only where triple strings exist"
    );
}

#[test]
fn a_selection_is_wrapped_and_stays_selected() {
    assert_eq!(typed("a.js", "x = «a + b»;", "("), "x = («a + b»);");
    assert_eq!(typed("a.js", "«word»", "\""), "\"«word»\"");
    assert_eq!(
        typed("a.json", "«word»", "("),
        "(|",
        "not a pair here: replaced"
    );
}

#[test]
fn backspace_deletes_an_empty_pair() {
    let lang = language("a.ts");
    for marked in ["(|)", "[|]", "{|}", "'|'", "\"|\"", "`|`"] {
        let (text, selection) = parse(marked);
        let edit = delete_pair(&lang, &text, selection).unwrap();
        assert_eq!(applied(&text, &edit), "|", "{marked}");
    }
    for marked in ["(|]", "(x|)", "(|x)", "|()", "«()»"] {
        let (text, selection) = parse(marked);
        assert!(delete_pair(&lang, &text, selection).is_none(), "{marked}");
    }
    let (text, selection) = parse("(|)");
    assert!(delete_pair(&language("a.json"), &text, selection).is_none());
}

#[test]
fn a_closer_typed_on_a_blank_line_lines_up_with_its_opener() {
    assert_eq!(
        typed("a.js", "if (a) {\n  x();\n  |", "}"),
        "if (a) {\n  x();\n}|"
    );
    assert_eq!(
        typed("a.js", "  call(\n    a,\n    |", ")"),
        "  call(\n    a,\n  )|"
    );
    assert_eq!(
        typed("a.js", "{\n  x |", "}"),
        "{\n  x }|",
        "not first on the line"
    );
    assert_eq!(typed("a.js", "{\n|", "}"), "{\n}|", "already lined up");
    let (text, selection) = parse("{\n  |");
    let edit = type_text(&language("a.js"), &text, selection, "}", "  ", &never).unwrap();
    assert_eq!(edit.range, 2..4, "one replacement of the indent and closer");
}

#[test]
fn python_else_and_friends_dedent_when_their_colon_is_typed() {
    let unit = |path: &str, marked: &str| {
        let (text, selection) = parse(marked);
        type_text(&language(path), &text, selection, ":", "    ", &never)
            .map(|edit| applied(&text, &edit))
    };
    assert_eq!(
        unit("a.py", "if a:\n    b()\n    else|"),
        Some("if a:\n    b()\nelse:|".into())
    );
    assert_eq!(
        unit("a.py", "try:\n    b()\n    except ValueError|"),
        Some("try:\n    b()\nexcept ValueError:|".into())
    );
    assert_eq!(
        unit("a.py", "if a:\n    if b:\n        c()\n        elif d|"),
        Some("if a:\n    if b:\n        c()\n    elif d:|".into())
    );
    assert_eq!(
        unit("a.py", "if a:\n    b()\nelse|"),
        None,
        "already dedented"
    );
    assert_eq!(unit("a.py", "x = {\n    'k'|"), None);
    assert_eq!(unit("a.js", "    else|"), None);
}

fn enter(path: &str, marked: &str, unit: &str) -> String {
    let (text, selection) = parse(marked);
    applied(&text, &newline(&language(path), &text, selection, unit))
}

#[test]
fn enter_keeps_raises_and_splits_indentation() {
    assert_eq!(enter("a.js", "  x();|", "  "), "  x();\n  |");
    assert_eq!(enter("a.js", "  if (a) {|", "  "), "  if (a) {\n    |");
    assert_eq!(enter("a.js", "f(|)", "  "), "f(\n  |\n)");
    assert_eq!(enter("a.js", "x = [|]", "  "), "x = [\n  |\n]");
    assert_eq!(enter("a.js", "  a {|}", "\t"), "  a {\n  \t|\n  }");
    assert_eq!(enter("a.js", "f(a,|", "  "), "f(a,|".replace('|', "\n|"));
    assert_eq!(
        enter("a.js", "{  |  x", "  "),
        "{  \n  |x",
        "whitespace around the caret is dropped"
    );
    assert_eq!(
        enter("a.js", "    |", "  "),
        "\n    |",
        "a blank line keeps no trailing indent"
    );
    assert_eq!(enter("a.js", "a «sel» b", "  "), "a \n|b");
}

#[test]
fn enter_follows_python_yaml_and_markup_blocks() {
    assert_eq!(enter("a.py", "def f():|", "    "), "def f():\n    |");
    assert_eq!(enter("a.py", "    return x|", "    "), "    return x\n|");
    assert_eq!(
        enter("a.py", "        pass|", "    "),
        "        pass\n    |"
    );
    assert_eq!(
        enter("a.py", "    returned = 1|", "    "),
        "    returned = 1\n    |"
    );
    assert_eq!(
        enter("a.js", "label:|", "  "),
        "label:\n|",
        "only Python and YAML"
    );
    assert_eq!(enter("a.yaml", "key:|", "  "), "key:\n  |");
    assert_eq!(
        enter("a.html", "<div class=\"a\">|", "  "),
        "<div class=\"a\">\n  |"
    );
    assert_eq!(enter("a.html", "<ul>|</ul>", "  "), "<ul>\n  |\n</ul>");
    assert_eq!(enter("a.html", "<br>|", "  "), "<br>\n|");
    assert_eq!(enter("a.html", "<p>x</p>|", "  "), "<p>x</p>\n|");
    assert_eq!(enter("a.html", "<img />|", "  "), "<img />\n|");
}

#[test]
fn indent_unit_comes_from_the_buffer() {
    let js = language("a.js");
    assert_eq!(indent_unit("a\n    b\n        c\n", &js), "    ");
    assert_eq!(indent_unit("a\n\tb\n", &js), "\t");
    assert_eq!(indent_unit("/**\n * doc\n */\nx\n", &js), "  ");
    assert_eq!(indent_unit("", &language("a.py")), "    ");
}

fn comment(path: &str, marked: &str) -> Option<String> {
    let (text, selection) = parse(marked);
    toggle_comment(&language(path), &text, selection).map(|edit| applied(&text, &edit))
}

#[test]
fn toggle_comment_uses_each_languages_tokens() {
    assert_eq!(comment("a.ts", "x|"), Some("// x|".into()));
    assert_eq!(comment("a.py", "x|"), Some("# x|".into()));
    assert_eq!(comment("a.sql", "x|"), Some("-- x|".into()));
    assert_eq!(comment("a.lua", "x|"), Some("-- x|".into()));
    assert_eq!(comment("Dockerfile", "RUN x|"), Some("# RUN x|".into()));
    assert_eq!(
        comment("a.css", "a { b: c }|"),
        Some("/* a { b: c } */|".into())
    );
    assert_eq!(
        comment("a.html", "  <p>x</p>|"),
        Some("  <!-- <p>x</p> -->|".into())
    );
    assert_eq!(comment("a.json", "x|"), None, "JSON has no comments");
}

#[test]
fn toggle_line_comment_over_mixed_indentation_round_trips() {
    let source = "«function f() {\n    a();\n\n  b();\n}»";
    let on = comment("a.js", source).unwrap();
    assert_eq!(
        on, "// «function f() {\n//     a();\n\n//   b();\n// }»",
        "comments at the smallest indent and skips blank lines"
    );
    let off = comment("a.js", &on).unwrap();
    assert_eq!(off, source);

    let nested = "  «if (a) {\n      b();\n  }»";
    let on = comment("a.js", nested).unwrap();
    assert_eq!(on, "  // «if (a) {\n  //     b();\n  // }»");
    assert_eq!(comment("a.js", &on).unwrap(), nested);
}

#[test]
fn toggle_line_comment_comments_all_when_any_line_is_plain() {
    assert_eq!(comment("a.js", "«// a\nb»").unwrap(), "// «// a\n// b»");
    assert_eq!(
        comment("a.js", "«  //a\n  // b»").unwrap(),
        "«  a\n  b»",
        "the space after the token goes only when present"
    );
    assert_eq!(
        comment("a.py", "x = 1|\ny = 2").unwrap(),
        "# x = 1|\ny = 2",
        "a caret comments only its own line"
    );
    assert_eq!(
        comment("a.py", "«a\n»b").unwrap(),
        "# «a\n»b",
        "a selection ending at a line start leaves that line alone"
    );
    assert_eq!(comment("a.js", "  |").unwrap(), "  // |");
}

#[test]
fn toggle_block_comment_wraps_and_unwraps_whole_lines() {
    let on = comment("a.css", "  «a {\n    b: c;\n  }»").unwrap();
    assert_eq!(
        on, "  /* «a {\n    b: c;\n  }» */",
        "the selection stays on the commented text"
    );
    assert_eq!(comment("a.css", &on).unwrap(), "  «a {\n    b: c;\n  }»");
    assert_eq!(comment("a.md", "<!-- x| -->").unwrap(), "x|");
}

fn lines(marked: &str, f: impl Fn(&str, Range<usize>) -> Option<Edit>) -> Option<String> {
    let (text, selection) = parse(marked);
    f(&text, selection).map(|edit| applied(&text, &edit))
}

#[test]
fn move_lines_swaps_with_neighbours_and_stops_at_the_edges() {
    let up = |t: &str, s| move_lines(t, s, true);
    let down = |t: &str, s| move_lines(t, s, false);
    assert_eq!(lines("a\nb|c\nd", up), Some("b|c\na\nd".into()));
    assert_eq!(lines("a\nb|c\nd", down), Some("a\nd\nb|c".into()));
    assert_eq!(lines("a|\nb", up), None, "first line");
    assert_eq!(lines("a\nb|", down), None, "last line");
    assert_eq!(lines("a\n«b\nc»\nd", up), Some("«b\nc»\na\nd".into()));
    assert_eq!(lines("a\n«b\nc»\nd", down), Some("a\nd\n«b\nc»".into()));
    assert_eq!(
        lines("a\n«b\n»c", down),
        Some("a\nc\n«b»".into()),
        "a selection ending at a line start moves only the lines before it"
    );
    assert_eq!(lines("x\n|", up), Some("|\nx".into()), "an empty last line");
}

#[test]
fn copy_lines_duplicates_above_or_below_including_edges() {
    let up = |t: &str, s| Some(copy_lines(t, s, true));
    let down = |t: &str, s| Some(copy_lines(t, s, false));
    assert_eq!(lines("a|b\nc", up), Some("a|b\nab\nc".into()));
    assert_eq!(lines("a|b\nc", down), Some("ab\na|b\nc".into()));
    assert_eq!(lines("a\nc|", down), Some("a\nc\nc|".into()), "last line");
    assert_eq!(lines("a|", up), Some("a|\na".into()), "only line");
    assert_eq!(lines("«a\nb»", down), Some("a\nb\n«a\nb»".into()));
}

#[test]
fn delete_lines_removes_whole_lines_at_every_position() {
    let del = |t: &str, s| delete_lines(t, s);
    assert_eq!(lines("one\nt|wo\nthree", del), Some("one\nt|hree".into()));
    assert_eq!(lines("on|e\ntwo", del), Some("tw|o".into()), "first line");
    assert_eq!(lines("one\ntw|o", del), Some("on|e".into()), "last line");
    assert_eq!(
        lines("a\nlong|line", del),
        Some("a|".into()),
        "column clamps"
    );
    assert_eq!(lines("on|ly", del), Some("|".into()), "only line");
    assert_eq!(lines("|", del), None, "empty buffer");
    assert_eq!(
        lines("a\n«b\nc»\nd", del),
        Some("a\nd|".into()),
        "the caret keeps the selection head's column"
    );
}

fn code(_: usize) -> u8 {
    0
}

#[test]
fn brackets_match_across_nesting_in_both_directions() {
    let text = "f(a[1], {b: (2)})";
    let at = |m: Option<BracketMatch>| m.map(|m| (m.at, m.other));
    assert_eq!(
        at(match_bracket(text, 2, &code)),
        Some((1, Some(16))),
        "after ("
    );
    assert_eq!(
        at(match_bracket(text, 1, &code)),
        Some((1, Some(16))),
        "before ("
    );
    assert_eq!(
        at(match_bracket(text, 17, &code)),
        Some((16, Some(1))),
        "after )"
    );
    assert_eq!(
        at(match_bracket(text, 8, &code)),
        Some((8, Some(15))),
        "brace before caret"
    );
    assert_eq!(
        at(match_bracket(text, 5, &code)),
        Some((5, Some(3))),
        "] after caret"
    );
    assert_eq!(at(match_bracket("x", 1, &code)), None);
    assert_eq!(
        at(match_bracket("(]", 0, &code)),
        Some((0, None)),
        "mismatched closer"
    );
    assert_eq!(
        at(match_bracket("((", 2, &code)),
        Some((1, None)),
        "unclosed"
    );
}

#[test]
fn brackets_in_strings_and_comments_only_pair_with_their_own_kind() {
    let text = "f(\")\", x) // (";
    // 0 code, 1 string, 2 comment.
    let class = |at: usize| match at {
        2..=4 => 1,
        10.. => 2,
        _ => 0,
    };
    let m = match_bracket(text, 1, &class).unwrap();
    assert_eq!(m.other, Some(8), "skips the ) inside the string");
    let m = match_bracket(text, 14, &class).unwrap();
    assert_eq!(
        (m.at, m.other),
        (13, None),
        "the comment's ( stays unmatched"
    );
    let m = match_bracket("\"(a)\"", 1, &|at| u8::from((0..5).contains(&at))).unwrap();
    assert_eq!(m.other, Some(3), "brackets inside one string still pair");
}

#[test]
fn select_to_matching_bracket_covers_both_brackets() {
    let text = "x = f(a, [b]);";
    assert_eq!(select_to_matching_bracket(text, 5..5, &code), Some(5..13));
    assert_eq!(select_to_matching_bracket(text, 13..13, &code), Some(5..13));
    assert_eq!(select_to_matching_bracket(text, 9..9, &code), Some(9..12));
    assert_eq!(select_to_matching_bracket(text, 12..12, &code), Some(9..12));
    assert_eq!(select_to_matching_bracket(text, 1..1, &code), None);
    assert_eq!(select_to_matching_bracket("(a", 0..0, &code), None);
}

#[test]
fn tags_match_their_partner_across_nesting() {
    let text = "<div><div class=\"a>\">x</div><br><img/></div>";
    let name = |r: Range<usize>| (r.start, &text[r]);
    let pair = |at| match_tag(text, at).map(|(a, b)| (name(a), name(b)));
    let outer = Some(((1, "div"), (40, "div")));
    assert_eq!(pair(2), outer, "in the opening name");
    assert_eq!(pair(5), outer, "right after the opening tag");
    assert_eq!(pair(43), outer, "in the closing tag");
    assert_eq!(pair(10), Some(((6, "div"), (24, "div"))), "in attributes");
    assert_eq!(
        pair(26),
        Some(((6, "div"), (24, "div"))),
        "in the inner closing tag"
    );
    assert_eq!(pair(30), None, "void element");
    assert_eq!(pair(35), None, "self-closing element");
    assert_eq!(match_tag("<p>abc</p>", 4), None, "text between tags");
    assert_eq!(match_tag("<p>unclosed", 1), None);
}

#[test]
fn language_detection_matches_paths_and_names() {
    assert!(language("src/App.tsx").tags);
    assert!(!language("src/app.ts").tags);
    assert_eq!(language("C:\\x\\Main.PY").indenter, Indenter::Python);
    assert_eq!(language("Gemfile").line_comment, Some("#"));
    assert_eq!(language("notes.txt").line_comment, None);
    assert_eq!(language("a.lua").block_comment, Some(("--[[", "]]--")));
}
