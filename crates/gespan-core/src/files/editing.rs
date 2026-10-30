//! Text editing commands for the code editor, ported from the Tauri editor's
//! CodeMirror setup: closeBrackets, bracketMatching, the tag matcher in
//! editorEditing.ts, indentOnInput, insertNewlineAndIndent, toggleComment,
//! moveLine, copyLine, deleteLine and selectMatchingBracket.
//!
//! Everything works on UTF-8 byte offsets into the buffer. Every command
//! returns at most one [`Edit`], so the editor applies it as one undo step.
use std::ops::Range;

/// Brackets closed for any file without its own list (CodeMirror's default).
const DEFAULT_BRACKETS: &[&str] = &["(", "[", "{", "'", "\""];
const JS_BRACKETS: &[&str] = &["(", "[", "{", "'", "\"", "`"];
const JSON_BRACKETS: &[&str] = &["[", "{", "\""];
const VUE_BRACKETS: &[&str] = &["{", "\""];
const PYTHON_BRACKETS: &[&str] = &["(", "[", "{", "'", "\"", "'''", "\"\"\""];
const TRIPLE_STRING_BRACKETS: &[&str] = &["(", "[", "{", "'", "\"", "\"\"\""];
const PYTHON_PREFIXES: &[&str] = &[
    "f", "fr", "rf", "r", "u", "b", "br", "rb", "F", "FR", "RF", "R", "U", "B", "BR", "RB",
];
const RUST_PREFIXES: &[&str] = &["b", "r", "br"];
const CPP_PREFIXES: &[&str] = &["L", "u", "U", "u8", "LR", "UR", "uR", "u8R", "R"];
const PHP_PREFIXES: &[&str] = &["b", "B"];
/// An opening bracket only closes when followed by nothing, whitespace or one
/// of these.
const CLOSE_BEFORE: &str = ")]}:;>";
/// Bracket matching gives up past this many bytes, like CodeMirror.
const MAX_SCAN: usize = 10_000;
/// Tag matching looks at most this far for the other tag.
const MAX_TAG_SCAN: usize = 200_000;
const C_LINE: Option<&str> = Some("//");
const C_BLOCK: Option<(&str, &str)> = Some(("/*", "*/"));
const MARKUP_BLOCK: Option<(&str, &str)> = Some(("<!--", "-->"));
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// How a new line's indentation is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indenter {
    /// Indent after `{`, `(` or `[`.
    Brackets,
    /// Brackets, plus a block after `:` and a dedent after `return`, `pass`…
    Python,
    /// Brackets, plus a nested mapping after `key:`.
    Yaml,
    /// Brackets, plus a nested element after an unclosed opening tag.
    Markup,
}

/// The editing rules for one file type, from the CodeMirror language data
/// the Tauri editor loads for that path.
#[derive(Clone, Copy, Debug)]
pub struct Language {
    pub brackets: &'static [&'static str],
    pub string_prefixes: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub indenter: Indenter,
    /// Highlight the matching HTML/XML/JSX tag name.
    pub tags: bool,
}

impl Language {
    const fn plain() -> Self {
        Self {
            brackets: DEFAULT_BRACKETS,
            string_prefixes: &[],
            line_comment: None,
            block_comment: None,
            indenter: Indenter::Brackets,
            tags: false,
        }
    }
    const fn c_like() -> Self {
        Self {
            line_comment: C_LINE,
            block_comment: C_BLOCK,
            ..Self::plain()
        }
    }
    const fn hash() -> Self {
        Self {
            line_comment: Some("#"),
            ..Self::plain()
        }
    }
    const fn markup() -> Self {
        Self {
            block_comment: MARKUP_BLOCK,
            indenter: Indenter::Markup,
            tags: true,
            ..Self::plain()
        }
    }
}

/// The editing rules for `path`, matched on its name the way the Tauri
/// editor picks a language (editorLanguage.ts).
pub fn language(path: &str) -> Language {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    let ext = name.rfind('.').map_or("", |at| &name[at + 1..]);
    match ext {
        "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" => Language {
            brackets: JS_BRACKETS,
            tags: matches!(ext, "jsx" | "tsx"),
            ..Language::c_like()
        },
        "json" => Language {
            brackets: JSON_BRACKETS,
            ..Language::plain()
        },
        // The legacy JSON mode keeps JavaScript's brackets but no comments.
        "jsonc" => Language {
            brackets: JS_BRACKETS,
            ..Language::plain()
        },
        "css" => Language {
            block_comment: C_BLOCK,
            ..Language::plain()
        },
        "scss" | "sass" | "less" => Language::c_like(),
        "html" | "htm" | "xml" | "svg" | "svelte" => Language::markup(),
        "vue" => Language {
            brackets: VUE_BRACKETS,
            ..Language::markup()
        },
        "md" | "mdx" | "markdown" => Language {
            block_comment: MARKUP_BLOCK,
            ..Language::plain()
        },
        "rs" => Language {
            string_prefixes: RUST_PREFIXES,
            ..Language::c_like()
        },
        "py" => Language {
            brackets: PYTHON_BRACKETS,
            string_prefixes: PYTHON_PREFIXES,
            indenter: Indenter::Python,
            ..Language::hash()
        },
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Language {
            string_prefixes: CPP_PREFIXES,
            ..Language::c_like()
        },
        "php" | "phtml" => Language {
            string_prefixes: PHP_PREFIXES,
            ..Language::c_like()
        },
        "kt" | "kts" | "scala" | "sc" => Language {
            brackets: TRIPLE_STRING_BRACKETS,
            ..Language::c_like()
        },
        "c" | "h" | "java" | "cs" | "go" | "dart" | "swift" | "m" | "mm" => Language::c_like(),
        "sql" => Language {
            brackets: JS_BRACKETS,
            line_comment: Some("--"),
            block_comment: C_BLOCK,
            ..Language::plain()
        },
        "yaml" | "yml" => Language {
            indenter: Indenter::Yaml,
            ..Language::hash()
        },
        "lua" => Language {
            line_comment: Some("--"),
            block_comment: Some(("--[[", "]]--")),
            ..Language::plain()
        },
        "ps1" | "psd1" | "psm1" => Language {
            block_comment: Some(("<#", "#>")),
            ..Language::hash()
        },
        "rb" | "rake" | "sh" | "bash" | "zsh" | "toml" | "r" | "pl" | "pm" => Language::hash(),
        _ if matches!(
            name.as_str(),
            "gemfile" | "rakefile" | ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile"
        ) || name == "dockerfile"
            || name.starts_with("dockerfile.") =>
        {
            Language::hash()
        }
        _ => Language::plain(),
    }
}

/// One replacement of `range` (in the old text) by `text`, after which the
/// selection is `selection` (in the new text). An empty range with empty text
/// only moves the selection and must not add an undo step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
    /// Where this edit inserted a closing bracket of its own, in the new
    /// text. Typing that closer later steps over it instead of doubling it.
    pub closer: Option<usize>,
}
impl Edit {
    fn new(range: Range<usize>, text: impl Into<String>, selection: Range<usize>) -> Self {
        Self {
            range,
            text: text.into(),
            selection,
            closer: None,
        }
    }
    fn caret(range: Range<usize>, text: impl Into<String>, at: usize) -> Self {
        Self::new(range, text, at..at)
    }
    /// Only moves the caret.
    pub fn is_move(&self) -> bool {
        self.range.is_empty() && self.text.is_empty()
    }
    /// The buffer after this edit.
    pub fn apply(&self, text: &str) -> String {
        format!(
            "{}{}{}",
            &text[..self.range.start],
            self.text,
            &text[self.range.end..]
        )
    }
}

fn closing(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        other => other,
    }
}
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn next_char(text: &str, at: usize) -> Option<char> {
    text[at..].chars().next()
}
fn prev_char(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |n| n + 1)
}
fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |n| at + n)
}
fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// The indentation step a buffer uses: a tab if its lines are tab-indented,
/// else the smallest even space indent it has (2 to 8), else two spaces (four
/// for Python). Odd indents are doc-comment continuations, not levels.
pub fn indent_unit(text: &str, language: &Language) -> String {
    let mut smallest = usize::MAX;
    for line in text.lines().take(2000) {
        let indent = leading_whitespace(line);
        if indent.len() == line.len() {
            continue;
        }
        if indent.starts_with('\t') {
            return "\t".into();
        }
        if indent.len() >= 2 && indent.len().is_multiple_of(2) {
            smallest = smallest.min(indent.len());
        }
    }
    if smallest <= 8 {
        return " ".repeat(smallest);
    }
    if language.indenter == Indenter::Python {
        "    ".into()
    } else {
        "  ".into()
    }
}

/// Typing `typed` over `selection`: auto-closing brackets and quotes,
/// wrapping a selection in them, stepping over a closer the editor inserted
/// (`auto_closed`), and re-indenting a line when its closer is typed.
/// `None` means a plain insert.
pub fn type_text(
    language: &Language,
    text: &str,
    selection: Range<usize>,
    typed: &str,
    unit: &str,
    auto_closed: &dyn Fn(usize) -> bool,
) -> Option<Edit> {
    let mut chars = typed.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let tokens = language.brackets;
    for token in tokens.iter().filter(|t| t.chars().count() == 1) {
        let open = token.chars().next().unwrap();
        let close = closing(open);
        if ch == open {
            return if open == close {
                let triple = tokens.contains(&token.repeat(3).as_str());
                type_quote(language, text, selection, open, triple, auto_closed)
            } else {
                type_open(text, selection, open, close)
            };
        }
        if ch == close
            && selection.is_empty()
            && next_char(text, selection.start) == Some(close)
            && auto_closed(selection.start)
        {
            let at = selection.start + close.len_utf8();
            return Some(Edit::caret(at..at, "", at));
        }
    }
    reindent_on_input(language, text, selection, ch, unit)
}

fn type_open(text: &str, selection: Range<usize>, open: char, close: char) -> Option<Edit> {
    let width = open.len_utf8();
    if !selection.is_empty() {
        let inner = &text[selection.clone()];
        let end = selection.end + width;
        return Some(Edit {
            closer: Some(end),
            ..Edit::new(
                selection.clone(),
                format!("{open}{inner}{close}"),
                selection.start + width..end,
            )
        });
    }
    let at = selection.start;
    match next_char(text, at) {
        Some(next) if !next.is_whitespace() && !CLOSE_BEFORE.contains(next) => None,
        _ => Some(Edit {
            closer: Some(at + width),
            ..Edit::caret(at..at, format!("{open}{close}"), at + width)
        }),
    }
}

fn type_quote(
    language: &Language,
    text: &str,
    selection: Range<usize>,
    quote: char,
    triple: bool,
    auto_closed: &dyn Fn(usize) -> bool,
) -> Option<Edit> {
    let width = quote.len_utf8();
    if !selection.is_empty() {
        let inner = &text[selection.clone()];
        let end = selection.end + width;
        return Some(Edit {
            closer: Some(end),
            ..Edit::new(
                selection.clone(),
                format!("{quote}{inner}{quote}"),
                selection.start + width..end,
            )
        });
    }
    let at = selection.start;
    let next = next_char(text, at);
    let pair = Edit {
        closer: Some(at + width),
        ..Edit::caret(at..at, format!("{quote}{quote}"), at + width)
    };
    if next == Some(quote) {
        if !auto_closed(at) {
            return None;
        }
        let three = quote.to_string().repeat(3);
        let step = if triple && text[at..].starts_with(&three) {
            3 * width
        } else {
            width
        };
        return Some(Edit::caret(at + step..at + step, "", at + step));
    }
    let two = quote.to_string().repeat(2);
    if triple
        && text[..at].ends_with(&two)
        && string_start(language, text, at - 2 * width).is_some()
    {
        return Some(Edit {
            closer: Some(at + width),
            ..Edit::caret(at..at, quote.to_string().repeat(4), at + width)
        });
    }
    if next.is_some_and(is_word) {
        return None;
    }
    (string_start(language, text, at).is_some() && !in_string(language, text, at)).then_some(pair)
}

/// Where a string opened at `at` would start: `at` after a non-word
/// character, or the start of a known prefix such as Python's `f`.
fn string_start(language: &Language, text: &str, at: usize) -> Option<usize> {
    if !prev_char(text, at).is_some_and(is_word) {
        return Some(at);
    }
    language.string_prefixes.iter().find_map(|prefix| {
        let start = at.checked_sub(prefix.len())?;
        (text.is_char_boundary(start)
            && &text[start..at] == *prefix
            && !prev_char(text, start).is_some_and(is_word))
        .then_some(start)
    })
}

/// Whether `at` is inside a string on its line, or right after one: quotes
/// typed there finish or extend it rather than opening a pair.
fn in_string(language: &Language, text: &str, at: usize) -> bool {
    let quotes: Vec<char> = language
        .brackets
        .iter()
        .filter_map(|t| {
            let mut chars = t.chars();
            let c = chars.next()?;
            (chars.next().is_none() && closing(c) == c).then_some(c)
        })
        .collect();
    let line = &text[line_start(text, at)..at];
    let mut open: Option<char> = None;
    let mut ended = false;
    let mut escaped = false;
    for (ix, c) in line.char_indices() {
        ended = false;
        if let Some(q) = open {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                open = None;
                ended = true;
            }
        } else if quotes.contains(&c) {
            open = Some(c);
        } else if language
            .line_comment
            .is_some_and(|token| line[ix..].starts_with(token))
        {
            return false;
        }
    }
    open.is_some() || ended
}

/// CodeMirror's indentOnInput: a closer typed as the first thing on a line
/// lines it up with its opener, and Python's `else:`, `elif …:`, `except…:`
/// and `finally:` drop back one level.
fn reindent_on_input(
    language: &Language,
    text: &str,
    selection: Range<usize>,
    ch: char,
    unit: &str,
) -> Option<Edit> {
    if !selection.is_empty() {
        return None;
    }
    let at = selection.start;
    let start = line_start(text, at);
    let before = &text[start..at];
    let indent = leading_whitespace(before);
    if matches!(ch, '}' | ']' | ')') && indent.len() == before.len() {
        let opener = unmatched_opener(text, start, ch)?;
        let target = leading_whitespace(&text[line_start(text, opener)..]);
        if target == indent {
            return None;
        }
        let new = format!("{target}{ch}");
        let caret = start + new.len();
        return Some(Edit::caret(start..at, new, caret));
    }
    if language.indenter == Indenter::Python && ch == ':' {
        let word = before.trim_start();
        let dedents = ["else", "finally"].contains(&word)
            || word.starts_with("elif ")
            || word == "except"
            || word.starts_with("except ");
        if !dedents {
            return None;
        }
        let previous = text[..start]
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())?;
        if leading_whitespace(previous).len() < indent.len() || !indent.ends_with(unit) {
            return None;
        }
        let target = &indent[..indent.len() - unit.len()];
        let new = format!("{target}{word}:");
        let caret = start + new.len();
        return Some(Edit::caret(start..at, new, caret));
    }
    None
}

/// The nearest `(`, `[` or `{` before `at` left open, for closer `close`.
fn unmatched_opener(text: &str, at: usize, close: char) -> Option<usize> {
    let floor = at.saturating_sub(MAX_SCAN);
    let mut depth = 0usize;
    for (ix, c) in text[..at].char_indices().rev() {
        if ix < floor {
            break;
        }
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    return (closing(c) == close).then_some(ix);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Backspace between an empty pair such as `(|)` or `"|"` deletes both.
pub fn delete_pair(language: &Language, text: &str, selection: Range<usize>) -> Option<Edit> {
    if !selection.is_empty() {
        return None;
    }
    let at = selection.start;
    let before = prev_char(text, at)?;
    let after = next_char(text, at)?;
    language
        .brackets
        .iter()
        .any(|t| t.chars().eq([before]) && closing(before) == after)
        .then(|| {
            let start = at - before.len_utf8();
            Edit::caret(start..at + after.len_utf8(), "", start)
        })
}

/// Enter: the new line keeps the indentation, gains a level after an opening
/// bracket (and `:` in Python and YAML, or an open tag in markup) and splits
/// an empty pair such as `{|}` over three lines. Whitespace around the caret
/// is not carried over.
pub fn newline(language: &Language, text: &str, selection: Range<usize>, unit: &str) -> Edit {
    let start = line_start(text, selection.start);
    let mut from = selection.start;
    let before = &text[start..from];
    let indent = leading_whitespace(before).to_owned();
    if indent.len() == before.len() {
        from = start;
    }
    let end = line_end(text, selection.end);
    let after = &text[selection.end..end];
    let to = selection.end + (after.len() - after.trim_start_matches([' ', '\t']).len());
    let next = text[to..end].chars().next();
    let content = before.trim_end();
    let last = content.chars().next_back();
    let deeper = format!("{indent}{unit}");
    let (inner, explode) = match last {
        Some(open @ ('{' | '(' | '[')) => (Some(deeper), next == Some(closing(open))),
        Some(':') if matches!(language.indenter, Indenter::Python | Indenter::Yaml) => {
            (Some(deeper), false)
        }
        _ if language.indenter == Indenter::Markup && open_tag_at_end(content) => {
            (Some(deeper), text[to..end].starts_with("</"))
        }
        _ if language.indenter == Indenter::Python && python_dedents(content) => (
            Some(
                indent
                    .strip_suffix(unit)
                    .unwrap_or(indent.as_str())
                    .to_owned(),
            ),
            false,
        ),
        _ => (None, false),
    };
    let line = inner.unwrap_or_else(|| indent.clone());
    let insert = if explode {
        format!("\n{line}\n{indent}")
    } else {
        format!("\n{line}")
    };
    let caret = from + 1 + line.len();
    Edit::caret(from..to, insert, caret)
}

fn python_dedents(content: &str) -> bool {
    let word = content
        .trim_start()
        .split(|c: char| !is_word(c))
        .next()
        .unwrap_or("");
    matches!(word, "return" | "pass" | "break" | "continue" | "raise")
}

/// Whether `content` ends with an opening tag such as `<div class="a">`
/// (not `<br>`, `<a/>` or `</p>`) that the line does not close.
fn open_tag_at_end(content: &str) -> bool {
    if !content.ends_with('>') || content.ends_with("/>") {
        return false;
    }
    let Some(lt) = content.rfind('<') else {
        return false;
    };
    let Some(tag) = parse_tag(content, lt) else {
        return false;
    };
    !tag.closing
        && !tag.self_closing
        && tag.end == content.len()
        && !VOID_TAGS.contains(&content[tag.name.clone()].to_ascii_lowercase().as_str())
}

/// The whole lines the selection touches, as `start..end` without the final
/// line break. A selection ending at the start of a line leaves that line out.
fn line_block(text: &str, selection: &Range<usize>) -> Range<usize> {
    let start = line_start(text, selection.start);
    let mut last = selection.end;
    if selection.end > selection.start && line_start(text, last) == last && last > start {
        last -= 1;
    }
    start..line_end(text, last)
}

/// Alt+↑ / Alt+↓: swap the selected lines with the line above or below,
/// carrying the selection. `None` at the top or bottom of the buffer.
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<Edit> {
    let block = line_block(text, &selection);
    let lines = &text[block.clone()];
    if up {
        if block.start == 0 {
            return None;
        }
        let above = line_start(text, block.start - 1);
        let other = &text[above..block.start - 1];
        let shift = other.len() + 1;
        Some(Edit::new(
            above..block.end,
            format!("{lines}\n{other}"),
            selection.start - shift..selection.end - shift,
        ))
    } else {
        if block.end == text.len() {
            return None;
        }
        let below = line_end(text, block.end + 1);
        let other = &text[block.end + 1..below];
        let shift = other.len() + 1;
        // A selection ending at the start of the line after the block can
        // only reach the end of the buffer.
        let clamp = |at: usize| (at + shift).min(text.len());
        Some(Edit::new(
            block.start..below,
            format!("{other}\n{lines}"),
            clamp(selection.start)..clamp(selection.end),
        ))
    }
}

/// Shift+Alt+↑ / Shift+Alt+↓: duplicate the selected lines. Down leaves the
/// selection on the lower copy and up on the upper one.
pub fn copy_lines(text: &str, selection: Range<usize>, up: bool) -> Edit {
    let block = line_block(text, &selection);
    let lines = &text[block.clone()];
    if up {
        Edit::new(block.end..block.end, format!("\n{lines}"), selection)
    } else {
        let shift = lines.len() + 1;
        Edit::new(
            block.start..block.start,
            format!("{lines}\n"),
            selection.start + shift..selection.end + shift,
        )
    }
}

/// ⌘⇧K: delete the selected lines with their line break. The caret keeps its
/// column on the line that takes their place.
pub fn delete_lines(text: &str, selection: Range<usize>) -> Option<Edit> {
    if text.is_empty() {
        return None;
    }
    let block = line_block(text, &selection);
    let column = selection.end.min(block.end) - line_start(text, selection.end.min(block.end));
    let (from, to) = if block.end < text.len() {
        (block.start, block.end + 1)
    } else {
        (block.start.saturating_sub(1), block.end)
    };
    let rest = format!("{}{}", &text[..from], &text[to..]);
    let target = line_start(&rest, from.min(rest.len()));
    let target_end = line_end(&rest, target);
    let mut caret = (target + column).min(target_end);
    while !rest.is_char_boundary(caret) {
        caret -= 1;
    }
    Some(Edit::caret(from..to, "", caret))
}

/// ⌘/: comment or uncomment the selected lines with the language's line
/// comment, or wrap them in its block comment when it has none.
pub fn toggle_comment(language: &Language, text: &str, selection: Range<usize>) -> Option<Edit> {
    if let Some(token) = language.line_comment {
        Some(toggle_line_comment(token, text, selection))
    } else {
        let (open, close) = language.block_comment?;
        Some(toggle_block_comment(open, close, text, selection))
    }
}

fn toggle_line_comment(token: &str, text: &str, selection: Range<usize>) -> Edit {
    let block = line_block(text, &selection);
    let lines: Vec<&str> = text[block.clone()].split('\n').collect();
    let single = lines.len() == 1;
    let min_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| leading_whitespace(line).len())
        .min();
    let commented = |line: &str| line[leading_whitespace(line).len()..].starts_with(token);
    let comment = lines
        .iter()
        .any(|line| !commented(line) && (!line.trim().is_empty() || single));
    // Each line's new text, where in the old text it changed, and by how much.
    let mut out: Vec<(String, usize, isize)> = Vec::with_capacity(lines.len());
    let mut offset = block.start;
    for line in &lines {
        let indent = leading_whitespace(line).len();
        let (changed, at, delta): (String, usize, isize) = if comment {
            if line.trim().is_empty() && !single {
                ((*line).to_owned(), 0, 0)
            } else {
                let at = min_indent.unwrap_or(indent).min(line.len());
                let insert = format!("{token} ");
                (
                    format!("{}{insert}{}", &line[..at], &line[at..]),
                    at,
                    insert.len() as isize,
                )
            }
        } else if commented(line) {
            let mut remove = token.len();
            if line[indent + remove..].starts_with(' ') {
                remove += 1;
            }
            (
                format!("{}{}", &line[..indent], &line[indent + remove..]),
                indent,
                -(remove as isize),
            )
        } else {
            ((*line).to_owned(), 0, 0)
        };
        out.push((changed, offset + at, delta));
        offset += line.len() + 1;
    }
    // A selection end moves by every insertion at or before it, and by the
    // part of every removal before it.
    let map = |pos: usize| {
        let mut moved = pos as isize;
        for (_, pivot, delta) in &out {
            if *delta > 0 && pos >= *pivot {
                moved += delta;
            } else if *delta < 0 && pos > *pivot {
                moved -= (pos - *pivot).min((-delta) as usize) as isize;
            }
        }
        moved as usize
    };
    let (start, end) = (map(selection.start), map(selection.end));
    let replaced = out
        .into_iter()
        .map(|(line, _, _)| line)
        .collect::<Vec<_>>()
        .join("\n");
    Edit::new(block, replaced, start..end)
}

fn toggle_block_comment(open: &str, close: &str, text: &str, selection: Range<usize>) -> Edit {
    let block = line_block(text, &selection);
    let from = block.start + leading_whitespace(&text[block.clone()]).len();
    let to = block.end;
    let body = &text[from..to];
    let trimmed = body.trim_end();
    if let Some(inner) = trimmed
        .strip_prefix(open)
        .and_then(|rest| rest.strip_suffix(close))
    {
        let lead = open.len() + usize::from(inner.starts_with(' '));
        let inner = inner.strip_prefix(' ').unwrap_or(inner);
        let inner = inner.strip_suffix(' ').unwrap_or(inner);
        let replaced = format!("{inner}{}", &body[trimmed.len()..]);
        let map = |pos: usize| {
            if pos <= from {
                pos
            } else {
                (pos.saturating_sub(lead)).clamp(from, from + inner.len())
            }
        };
        return Edit::new(from..to, replaced, map(selection.start)..map(selection.end));
    }
    let lead = open.len() + 1;
    let replaced = format!("{open} {body} {close}");
    // A caret at the end of the lines ends up after the closing token; a
    // selection keeps to the commented text.
    let caret_after = selection.is_empty() && selection.start >= to;
    let map = |pos: usize| {
        if pos < from {
            pos
        } else if caret_after {
            pos + lead + 1 + close.len()
        } else {
            pos + lead
        }
    };
    Edit::new(from..to, replaced, map(selection.start)..map(selection.end))
}

/// A bracket next to the caret and the one that matches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketMatch {
    /// The bracket beside the caret.
    pub at: usize,
    /// Its partner, or `None` when it is unmatched or closed by the wrong
    /// kind of bracket.
    pub other: Option<usize>,
}

/// The bracket before the caret, else the one after it, and its partner.
/// `class` sorts offsets into code, strings and comments: a bracket only
/// pairs with brackets of its own class, as CodeMirror's syntax-aware
/// matcher does.
pub fn match_bracket(text: &str, at: usize, class: &dyn Fn(usize) -> u8) -> Option<BracketMatch> {
    let before = prev_char(text, at)
        .filter(|c| "()[]{}".contains(*c))
        .map(|c| at - c.len_utf8());
    let after = next_char(text, at)
        .filter(|c| "()[]{}".contains(*c))
        .map(|_| at);
    let at = before.or(after)?;
    let bracket = text.as_bytes()[at] as char;
    let kind = class(at);
    let (forward, partner) = match bracket {
        '(' => (true, ')'),
        '[' => (true, ']'),
        '{' => (true, '}'),
        ')' => (false, '('),
        ']' => (false, '['),
        _ => (false, '{'),
    };
    let mut stack = Vec::new();
    let found = if forward {
        let end = (at + 1 + MAX_SCAN).min(text.len());
        scan(
            text.as_bytes()[at + 1..end]
                .iter()
                .enumerate()
                .map(|(ix, b)| (at + 1 + ix, *b as char)),
            bracket,
            partner,
            kind,
            class,
            &mut stack,
        )
    } else {
        let start = at.saturating_sub(MAX_SCAN);
        scan(
            text.as_bytes()[start..at]
                .iter()
                .enumerate()
                .rev()
                .map(|(ix, b)| (start + ix, *b as char)),
            bracket,
            partner,
            kind,
            class,
            &mut stack,
        )
    };
    Some(BracketMatch { at, other: found })
}

fn scan(
    chars: impl Iterator<Item = (usize, char)>,
    own: char,
    partner: char,
    kind: u8,
    class: &dyn Fn(usize) -> u8,
    stack: &mut Vec<char>,
) -> Option<usize> {
    let openers = if "([{".contains(own) { "([{" } else { ")]}" };
    for (ix, c) in chars {
        if !"()[]{}".contains(c) || class(ix) != kind {
            continue;
        }
        if openers.contains(c) {
            stack.push(c);
        } else if let Some(open) = stack.pop() {
            if mirror(open) != c {
                return None;
            }
        } else {
            return (c == partner).then_some(ix);
        }
    }
    None
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        _ => '{',
    }
}

/// Select from the bracket beside the caret through its partner, both
/// brackets included.
pub fn select_to_matching_bracket(
    text: &str,
    selection: Range<usize>,
    class: &dyn Fn(usize) -> u8,
) -> Option<Range<usize>> {
    let found = match_bracket(text, selection.end, class)
        .or_else(|| match_bracket(text, selection.start, class))?;
    let other = found.other?;
    let (a, b) = if other < found.at {
        (other, found.at)
    } else {
        (found.at, other)
    };
    Some(a.min(selection.start)..(b + 1).max(selection.end))
}

#[derive(Debug)]
struct Tag {
    name: Range<usize>,
    end: usize,
    closing: bool,
    self_closing: bool,
}

/// The tag starting at `lt` (`<name …>` or `</name>`), skipping quoted
/// attribute values.
fn parse_tag(text: &str, lt: usize) -> Option<Tag> {
    let bytes = text.as_bytes();
    let mut ix = lt + 1;
    let closing = bytes.get(ix) == Some(&b'/');
    if closing {
        ix += 1;
    }
    let name_start = ix;
    if !bytes.get(ix)?.is_ascii_alphabetic() {
        return None;
    }
    while ix < bytes.len()
        && (bytes[ix].is_ascii_alphanumeric() || matches!(bytes[ix], b'-' | b'_' | b':' | b'.'))
    {
        ix += 1;
    }
    let name = name_start..ix;
    let mut quote = None;
    while ix < bytes.len() {
        let b = bytes[ix];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'<' => return None,
            None if b == b'>' => {
                return Some(Tag {
                    name,
                    end: ix + 1,
                    closing,
                    self_closing: bytes[ix - 1] == b'/',
                });
            }
            None => {}
        }
        ix += 1;
    }
    None
}

/// With the caret in or beside a tag, the name ranges of that tag and the
/// tag that pairs with it, opening tag first.
pub fn match_tag(text: &str, at: usize) -> Option<(Range<usize>, Range<usize>)> {
    let line = line_start(text, at);
    let tag = text[line..at.min(text.len())]
        .rmatch_indices('<')
        .map(|(ix, _)| line + ix)
        .find_map(|lt| parse_tag(text, lt).filter(|t| t.end >= at).map(|t| (lt, t)))
        .or_else(|| {
            text[at..line_end(text, at)]
                .starts_with('<')
                .then(|| parse_tag(text, at).map(|t| (at, t)))
                .flatten()
        });
    let (lt, tag) = tag?;
    if tag.self_closing {
        return None;
    }
    let name = &text[tag.name.clone()];
    if !tag.closing && VOID_TAGS.contains(&name.to_ascii_lowercase().as_str()) {
        return None;
    }
    let mut depth = 0usize;
    if tag.closing {
        let floor = lt.saturating_sub(MAX_TAG_SCAN);
        for (ix, _) in text[floor..lt].rmatch_indices('<') {
            let ix = floor + ix;
            let Some(other) = parse_tag(text, ix) else {
                continue;
            };
            if &text[other.name.clone()] != name || other.self_closing {
                continue;
            }
            if other.closing {
                depth += 1;
            } else if depth == 0 {
                return Some((other.name, tag.name));
            } else {
                depth -= 1;
            }
        }
    } else {
        let ceiling = (tag.end + MAX_TAG_SCAN).min(text.len());
        let mut from = tag.end;
        while let Some(found) = text[from..ceiling].find('<') {
            let ix = from + found;
            from = ix + 1;
            let Some(other) = parse_tag(text, ix) else {
                continue;
            };
            if &text[other.name.clone()] != name || other.self_closing {
                continue;
            }
            if !other.closing {
                depth += 1;
            } else if depth == 0 {
                return Some((tag.name, other.name));
            } else {
                depth -= 1;
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "editing_tests.rs"]
mod tests;
