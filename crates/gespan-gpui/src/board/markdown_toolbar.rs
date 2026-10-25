//! The description's Markdown toolbar. Each button is a pure text transform:
//! `(text, selection)` in, `(new text, new selection)` out, with byte offsets
//! on char boundaries. The stored text stays plain Markdown.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Bold,
    Italic,
    Heading,
    Bullets,
    Numbered,
    Link,
    Code,
}

impl Format {
    pub(crate) const ALL: [Format; 7] = [
        Format::Bold,
        Format::Italic,
        Format::Heading,
        Format::Bullets,
        Format::Numbered,
        Format::Link,
        Format::Code,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Format::Bold => "Bold",
            Format::Italic => "Italic",
            Format::Heading => "Heading",
            Format::Bullets => "Bulleted list",
            Format::Numbered => "Numbered list",
            Format::Link => "Link",
            Format::Code => "Code",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Format::Bold => "bold",
            Format::Italic => "italic",
            Format::Heading => "heading",
            Format::Bullets => "bullets",
            Format::Numbered => "numbered",
            Format::Link => "link",
            Format::Code => "code",
        }
    }

    /// What the button shows.
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Format::Bold => "B",
            Format::Italic => "I",
            Format::Heading => "H",
            Format::Bullets => "•",
            Format::Numbered => "1.",
            Format::Link => "Link",
            Format::Code => "</>",
        }
    }
}

/// Apply `format` to `selection` (clamped to the text and to char boundaries).
pub(crate) fn apply(format: Format, text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let selection = clamp(text, selection);
    match format {
        Format::Bold => wrap(text, selection, "**", "**"),
        Format::Italic => wrap(text, selection, "_", "_"),
        Format::Code => code(text, selection),
        Format::Link => link(text, selection),
        Format::Heading => heading(text, selection),
        Format::Bullets => prefix_lines(text, selection, |_| "- ".to_string()),
        Format::Numbered => prefix_lines(text, selection, |n| format!("{}. ", n + 1)),
    }
}

fn clamp(text: &str, range: Range<usize>) -> Range<usize> {
    let fix = |mut offset: usize| {
        offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    };
    let (a, b) = (fix(range.start), fix(range.end));
    a.min(b)..a.max(b)
}

/// The selection without its trailing newlines, which stay outside any marker.
fn trim_newlines(text: &str, selection: Range<usize>) -> Range<usize> {
    let mut end = selection.end;
    while end > selection.start && text.as_bytes()[end - 1] == b'\n' {
        end -= 1;
    }
    selection.start..end
}

fn splice(text: &str, body: Range<usize>, open: &str, close: &str) -> String {
    format!(
        "{}{open}{}{close}{}",
        &text[..body.start],
        &text[body.clone()],
        &text[body.end..]
    )
}

fn wrap(text: &str, selection: Range<usize>, open: &str, close: &str) -> (String, Range<usize>) {
    let body = trim_newlines(text, selection);
    let start = body.start + open.len();
    let end = start + body.len();
    (splice(text, body, open, close), start..end)
}

fn code(text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let body = trim_newlines(text, selection);
    if !text[body.clone()].contains('\n') {
        return wrap(text, body, "`", "`");
    }
    // The fences sit on lines of their own: a selection that starts or ends
    // mid-line gets a line break before the opening or after the closing one.
    let at_line_start = body.start == 0 || text.as_bytes()[body.start - 1] == b'\n';
    let at_line_end = text[body.end..].is_empty() || text[body.end..].starts_with('\n');
    let open = if at_line_start { "```\n" } else { "\n```\n" };
    let close = if at_line_end { "\n```" } else { "\n```\n" };
    let start = body.start + open.len();
    let end = start + body.len();
    (splice(text, body, open, close), start..end)
}

fn link(text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let body = trim_newlines(text, selection);
    if body.is_empty() {
        // Nothing to link: `[]()` with the cursor between the brackets.
        let at = body.start + 1;
        return (splice(text, body, "[]()", ""), at..at);
    }
    let url_start = body.end + "](".len() + 1;
    let url = url_start..url_start + "url".len();
    (splice(text, body, "[", "](url)"), url)
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn heading(text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let start = line_start(text, selection.start);
    let mut new = String::with_capacity(text.len() + 3);
    new.push_str(&text[..start]);
    new.push_str("## ");
    new.push_str(&text[start..]);
    let shift = 3;
    (new, selection.start + shift..selection.end + shift)
}

/// Prefix every line the selection touches. A selection that ends with a
/// newline does not touch the line after it.
fn prefix_lines(
    text: &str,
    selection: Range<usize>,
    prefix: impl Fn(usize) -> String,
) -> (String, Range<usize>) {
    let body = trim_newlines(text, selection.clone());
    let first = line_start(text, body.start);
    let last_end = text[body.end..]
        .find('\n')
        .map_or(text.len(), |i| body.end + i);
    let mut block = String::new();
    for (n, line) in text[first..last_end].split('\n').enumerate() {
        if n > 0 {
            block.push('\n');
        }
        block.push_str(&prefix(n));
        block.push_str(line);
    }
    let new = format!("{}{block}{}", &text[..first], &text[last_end..]);
    if selection.is_empty() {
        let at = selection.start + prefix(0).len();
        (new, at..at)
    } else {
        (new, first..first + block.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(format: Format, text: &str, sel: Range<usize>) -> (String, String) {
        let (new, range) = apply(format, text, sel);
        (new.clone(), new[range].to_string())
    }

    fn cursor(format: Format, text: &str, at: usize) -> (String, Range<usize>) {
        apply(format, text, at..at)
    }

    #[test]
    fn bold_wraps_a_selection_and_selects_the_inside() {
        assert_eq!(
            run(Format::Bold, "make it loud", 8..12),
            ("make it **loud**".into(), "loud".into())
        );
    }

    #[test]
    fn bold_with_no_selection_puts_the_cursor_between_the_markers() {
        assert_eq!(cursor(Format::Bold, "ab", 1), ("a****b".into(), 3..3));
        assert_eq!(cursor(Format::Bold, "", 0), ("****".into(), 2..2));
    }

    #[test]
    fn italic_wraps_in_underscores() {
        assert_eq!(
            run(Format::Italic, "a soft word", 2..6),
            ("a _soft_ word".into(), "soft".into())
        );
        assert_eq!(cursor(Format::Italic, "x", 1), ("x__".into(), 2..2));
    }

    #[test]
    fn wrapping_keeps_a_trailing_newline_outside_the_markers() {
        assert_eq!(
            run(Format::Bold, "one\ntwo\n", 0..4),
            ("**one**\ntwo\n".into(), "one".into())
        );
    }

    #[test]
    fn wrapping_a_multi_line_selection_wraps_it_whole() {
        assert_eq!(
            run(Format::Italic, "a\nb", 0..3),
            ("_a\nb_".into(), "a\nb".into())
        );
    }

    #[test]
    fn wrapping_at_the_end_of_the_text() {
        assert_eq!(
            run(Format::Bold, "end word", 4..8),
            ("end **word**".into(), "word".into())
        );
        assert_eq!(cursor(Format::Bold, "end", 3), ("end****".into(), 5..5));
    }

    #[test]
    fn multi_byte_text_stays_on_char_boundaries() {
        // "é" is two bytes; an offset inside it clamps back to its start.
        assert_eq!(
            run(Format::Bold, "é ü", 0..2),
            ("**é** ü".into(), "é".into())
        );
        assert_eq!(cursor(Format::Bold, "é", 1), ("****é".into(), 2..2));
    }

    #[test]
    fn heading_prefixes_the_cursor_line() {
        assert_eq!(
            cursor(Format::Heading, "Title", 0),
            ("## Title".into(), 3..3)
        );
        assert_eq!(
            cursor(Format::Heading, "a\nsecond\nc", 4),
            ("a\n## second\nc".into(), 7..7)
        );
        assert_eq!(cursor(Format::Heading, "", 0), ("## ".into(), 3..3));
    }

    #[test]
    fn heading_on_a_selection_prefixes_only_its_first_line() {
        assert_eq!(
            apply(Format::Heading, "ab\ncd", 1..5),
            ("## ab\ncd".into(), 4..8)
        );
    }

    #[test]
    fn heading_at_the_end_of_the_text() {
        assert_eq!(cursor(Format::Heading, "a\nb", 3), ("a\n## b".into(), 6..6));
    }

    #[test]
    fn bullets_prefix_each_selected_line() {
        assert_eq!(
            apply(Format::Bullets, "a\nb\nc", 0..5),
            ("- a\n- b\n- c".into(), 0..11)
        );
        // Only the lines the selection touches.
        assert_eq!(
            apply(Format::Bullets, "a\nb\nc", 2..3),
            ("a\n- b\nc".into(), 2..5)
        );
    }

    #[test]
    fn bullets_with_no_selection_prefix_the_cursor_line() {
        assert_eq!(
            cursor(Format::Bullets, "a\nbc", 3),
            ("a\n- bc".into(), 5..5)
        );
        assert_eq!(cursor(Format::Bullets, "", 0), ("- ".into(), 2..2));
    }

    #[test]
    fn a_selection_ending_in_a_newline_leaves_the_next_line_alone() {
        assert_eq!(
            apply(Format::Bullets, "a\nb\nc", 0..4),
            ("- a\n- b\nc".into(), 0..7)
        );
        assert_eq!(
            apply(Format::Numbered, "a\nb\n", 0..4),
            ("1. a\n2. b\n".into(), 0..9)
        );
    }

    #[test]
    fn numbered_lists_count_from_one() {
        assert_eq!(
            apply(Format::Numbered, "a\nb\nc", 0..5),
            ("1. a\n2. b\n3. c".into(), 0..14)
        );
        assert_eq!(cursor(Format::Numbered, "", 0), ("1. ".into(), 3..3));
    }

    #[test]
    fn numbered_at_the_end_of_the_text() {
        assert_eq!(
            apply(Format::Numbered, "x\nlast", 2..6),
            ("x\n1. last".into(), 2..9)
        );
    }

    #[test]
    fn link_turns_the_selection_into_text_and_selects_the_url() {
        assert_eq!(
            run(Format::Link, "see docs now", 4..8),
            ("see [docs](url) now".into(), "url".into())
        );
    }

    #[test]
    fn link_with_no_selection_puts_the_cursor_inside_the_brackets() {
        assert_eq!(cursor(Format::Link, "ab", 2), ("ab[]()".into(), 3..3));
    }

    #[test]
    fn link_at_the_end_and_with_a_trailing_newline() {
        assert_eq!(
            run(Format::Link, "go site", 3..7),
            ("go [site](url)".into(), "url".into())
        );
        assert_eq!(
            run(Format::Link, "site\n", 0..5),
            ("[site](url)\n".into(), "url".into())
        );
    }

    #[test]
    fn code_wraps_one_line_in_backticks() {
        assert_eq!(
            run(Format::Code, "run ls now", 4..6),
            ("run `ls` now".into(), "ls".into())
        );
        assert_eq!(cursor(Format::Code, "a", 1), ("a``".into(), 2..2));
    }

    #[test]
    fn code_fences_several_lines() {
        assert_eq!(
            run(Format::Code, "a\nb", 0..3),
            ("```\na\nb\n```".into(), "a\nb".into())
        );
    }

    #[test]
    fn code_fence_keeps_a_trailing_newline_outside() {
        assert_eq!(
            run(Format::Code, "a\nb\nrest", 0..4),
            ("```\na\nb\n```\nrest".into(), "a\nb".into())
        );
        // One line plus its newline is still one line.
        assert_eq!(
            run(Format::Code, "a\nrest", 0..2),
            ("`a`\nrest".into(), "a".into())
        );
    }

    #[test]
    fn code_fence_at_the_end_of_the_text() {
        assert_eq!(
            run(Format::Code, "x\na\nb", 2..5),
            ("x\n```\na\nb\n```".into(), "a\nb".into())
        );
    }

    #[test]
    fn code_fence_from_the_middle_of_a_line_starts_on_its_own_line() {
        assert_eq!(
            run(Format::Code, "foo bar\nbaz", 4..11),
            ("foo \n```\nbar\nbaz\n```".into(), "bar\nbaz".into())
        );
    }

    #[test]
    fn code_fence_ending_mid_line_closes_on_its_own_line() {
        assert_eq!(
            run(Format::Code, "foo\nbar baz", 0..7),
            ("```\nfoo\nbar\n```\n baz".into(), "foo\nbar".into())
        );
    }

    #[test]
    fn code_fence_starting_and_ending_mid_line() {
        assert_eq!(
            run(Format::Code, "foo bar\nbaz qux", 4..11),
            ("foo \n```\nbar\nbaz\n```\n qux".into(), "bar\nbaz".into())
        );
    }

    #[test]
    fn non_ascii_text_formats_on_char_boundaries() {
        // Heading: the prefix goes before the line, offsets shift by 3 bytes.
        assert_eq!(
            apply(Format::Heading, "ça\nüber", 4..8),
            ("ça\n## über".into(), 7..11)
        );
        // Bullets: only the lines the selection touches.
        assert_eq!(
            apply(Format::Bullets, "é\nü\nñ", 3..5),
            ("é\n- ü\nñ".into(), 3..7)
        );
        // Link: the selected word keeps its accents and `url` is selected.
        assert_eq!(
            run(Format::Link, "voir été ici", 5..10),
            ("voir [été](url) ici".into(), "url".into())
        );
        // Code: one line and several lines.
        assert_eq!(
            run(Format::Code, "lancer ñandú", 7..14),
            ("lancer `ñandú`".into(), "ñandú".into())
        );
        assert_eq!(
            run(Format::Code, "é ü\nñ", 3..8),
            ("é \n```\nü\nñ\n```".into(), "ü\nñ".into())
        );
    }

    #[test]
    fn an_out_of_range_selection_is_clamped() {
        assert_eq!(cursor(Format::Bold, "ab", 99), ("ab****".into(), 4..4));
    }
}
