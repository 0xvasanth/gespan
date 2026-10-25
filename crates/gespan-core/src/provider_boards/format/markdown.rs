//! Markdown (CommonMark plus `~~strike~~`) to and from the document model.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::doc::{
    decode_html_entities, normalize, Block, Doc, Inline, Link, Marks, Preserved, Source,
    MAX_LIST_START,
};

pub(crate) fn parse(markdown: &str) -> Doc {
    let mut builder = Builder::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH) {
        builder.event(event);
    }
    normalize(Doc {
        blocks: builder.finish(),
    })
}

enum Frame {
    Container(Vec<Block>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    /// A tight list item holds its text without a paragraph, so `inlines`
    /// collects it until the next block.
    Item {
        blocks: Vec<Block>,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Code(Option<String>, String),
    Html(String),
    Image {
        src: String,
        alt: String,
        link: Option<Link>,
    },
}

/// Container frames (quotes, lists, items) deeper than this are flattened
/// into their parent, so hostile input can't exhaust the stack.
pub(crate) const MAX_DEPTH: usize = 64;

struct Builder {
    frames: Vec<Frame>,
    strong: usize,
    em: usize,
    strike: usize,
    links: Vec<Link>,
    skip: Option<Skip>,
    /// Container starts ignored past [`MAX_DEPTH`], whose ends are ignored too.
    overflow: usize,
}

/// The read-only text of a preserved span, held back until its `</span>`.
/// If the span never closes, the text is put back, less the fallback.
struct Skip {
    /// Open `<span>`s, counting the preserved one.
    depth: usize,
    fallback: String,
    held: Vec<Event<'static>>,
}

impl Builder {
    fn new() -> Builder {
        Builder {
            frames: vec![Frame::Container(Vec::new())],
            strong: 0,
            em: 0,
            strike: 0,
            links: Vec::new(),
            skip: None,
            overflow: 0,
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.release_skip();
        while self.frames.len() > 1 {
            self.close_frame();
        }
        match self.frames.pop() {
            Some(Frame::Container(blocks)) => blocks,
            _ => Vec::new(),
        }
    }

    fn event(&mut self, event: Event) {
        if let Some(skip) = &mut self.skip {
            let held = match &event {
                // Another preserved span means this one was never closed.
                Event::InlineHtml(html) if preserved_span(html.trim()).is_some() => false,
                Event::InlineHtml(html) => {
                    let tag = html.trim().to_ascii_lowercase();
                    if tag.starts_with("<span") {
                        skip.depth += 1;
                    } else if tag == "</span>" {
                        skip.depth -= 1;
                        if skip.depth == 0 {
                            self.skip = None;
                            return;
                        }
                    }
                    true
                }
                Event::Text(_)
                | Event::Code(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Start(
                    Tag::Emphasis
                    | Tag::Strong
                    | Tag::Strikethrough
                    | Tag::Link { .. }
                    | Tag::Image { .. },
                )
                | Event::End(
                    TagEnd::Emphasis
                    | TagEnd::Strong
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Image,
                ) => true,
                _ => false,
            };
            if held {
                skip.held.push(event.into_static());
                return;
            }
            self.release_skip();
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text, false),
            Event::Code(text) => self.text(&text, true),
            Event::SoftBreak => self.text(" ", false),
            Event::HardBreak => self.push_inline(Inline::HardBreak),
            Event::InlineHtml(html) => self.inline_html(&html),
            Event::Html(html) => match self.frames.last_mut() {
                Some(Frame::Html(raw)) => raw.push_str(&html),
                _ => self.push_inline(Inline::Html(html.to_string())),
            },
            Event::Rule => self.push_block(Block::Rule),
            _ => {}
        }
    }

    /// Puts back what an unclosed preserved span held, less its fallback.
    fn release_skip(&mut self) {
        let Some(skip) = self.skip.take() else {
            return;
        };
        let mut held = skip.held.into_iter().peekable();
        let mut leading = String::new();
        while let Some(Event::Text(text)) = held.peek() {
            leading.push_str(text);
            held.next();
        }
        let rest = leading
            .strip_prefix(skip.fallback.as_str())
            .unwrap_or(&leading);
        if !rest.is_empty() {
            self.text(rest, false);
        }
        for event in held {
            self.event(event);
        }
    }

    fn start(&mut self, tag: Tag) {
        let container = matches!(tag, Tag::BlockQuote(_) | Tag::List(_) | Tag::Item);
        // A list needs room for its item as well.
        let room = if matches!(tag, Tag::List(_)) { 2 } else { 1 };
        if container && (self.overflow > 0 || self.frames.len() + room > MAX_DEPTH) {
            self.overflow += 1;
            return;
        }
        match tag {
            Tag::Paragraph => self.open_block(Frame::Paragraph(Vec::new())),
            Tag::Heading { level, .. } => self.open_block(Frame::Heading(level as u8, Vec::new())),
            Tag::BlockQuote(_) => self.open_block(Frame::Quote(Vec::new())),
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => Some(info.to_string()),
                    CodeBlockKind::Indented => None,
                };
                self.open_block(Frame::Code(language, String::new()));
            }
            Tag::HtmlBlock => self.open_block(Frame::Html(String::new())),
            Tag::List(start) => self.open_block(Frame::List {
                start,
                items: Vec::new(),
            }),
            Tag::Item => self.open_block(Frame::Item {
                blocks: Vec::new(),
                inlines: Vec::new(),
            }),
            Tag::Emphasis => self.em += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link {
                dest_url, title, ..
            } => self.links.push(Link {
                href: dest_url.to_string(),
                title: (!title.is_empty()).then(|| title.to_string()),
            }),
            Tag::Image { dest_url, .. } => self.frames.push(Frame::Image {
                src: dest_url.to_string(),
                alt: String::new(),
                link: self.links.last().cloned(),
            }),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        if self.overflow > 0
            && matches!(tag, TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item)
        {
            self.overflow -= 1;
            return;
        }
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::Image => {
                if matches!(tag, TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) {
                    // Emphasis never spans blocks; this also ends unclosed
                    // `<strong>`-style tags.
                    self.strong = 0;
                    self.em = 0;
                    self.strike = 0;
                }
                self.close_frame();
            }
            TagEnd::Emphasis => self.em = self.em.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            _ => {}
        }
    }

    fn close_frame(&mut self) {
        self.flush_item();
        let Some(frame) = self.frames.pop() else {
            return;
        };
        match frame {
            Frame::Paragraph(content) => self.push_block(Block::Paragraph(content)),
            Frame::Heading(level, content) => self.push_block(Block::Heading { level, content }),
            Frame::Quote(blocks) => self.push_block(Block::Quote(blocks)),
            // Only Gespan's own exact fence marks kept content; a tracker's
            // code block in that language stays code (see `fenced_info`).
            Frame::Code(language, code) => {
                let source = language
                    .as_deref()
                    .map(str::trim)
                    .and_then(Source::from_fence);
                self.push_block(match source {
                    Some(source) => Block::Preserved(Preserved {
                        source,
                        raw: code.trim_end_matches('\n').to_string(),
                    }),
                    None => Block::Code { language, code },
                });
            }
            Frame::Html(raw) => self.push_block(Block::Html(raw)),
            Frame::List { start, items } => self.push_block(match start {
                Some(start) => Block::OrderedList { start, items },
                None => Block::BulletList(items),
            }),
            Frame::Item { blocks, .. } => {
                if let Some(Frame::List { items, .. }) = self.frames.last_mut() {
                    items.push(blocks);
                }
            }
            Frame::Image { src, alt, link } => self.push_inline(Inline::Image { src, alt, link }),
            Frame::Container(blocks) => {
                // Only the root is a container; put it back.
                self.frames.push(Frame::Container(blocks));
            }
        }
    }

    fn open_block(&mut self, frame: Frame) {
        self.flush_item();
        self.frames.push(frame);
    }

    fn flush_item(&mut self) {
        if let Some(Frame::Item { blocks, inlines }) = self.frames.last_mut() {
            if !inlines.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(inlines)));
            }
        }
    }

    fn push_block(&mut self, block: Block) {
        self.flush_item();
        if let Some(Frame::Container(blocks) | Frame::Quote(blocks) | Frame::Item { blocks, .. }) =
            self.frames.last_mut()
        {
            blocks.push(block);
        }
    }

    fn push_inline(&mut self, inline: Inline) {
        match self.frames.last_mut() {
            Some(Frame::Paragraph(content) | Frame::Heading(_, content)) => content.push(inline),
            Some(Frame::Item { inlines, .. }) => inlines.push(inline),
            // A tight item flattened past MAX_DEPTH: its text joins a
            // paragraph in the parent.
            Some(Frame::Container(blocks) | Frame::Quote(blocks)) => match blocks.last_mut() {
                Some(Block::Paragraph(content)) => content.push(inline),
                _ => blocks.push(Block::Paragraph(vec![inline])),
            },
            Some(Frame::Image { alt, .. }) => {
                if let Inline::Text { text, .. } = inline {
                    alt.push_str(&text);
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str, code: bool) {
        match self.frames.last_mut() {
            Some(Frame::Code(_, body) | Frame::Html(body)) => body.push_str(text),
            Some(Frame::Image { alt, .. }) => alt.push_str(text),
            _ => {
                let marks = Marks {
                    strong: self.strong > 0,
                    em: self.em > 0,
                    strike: self.strike > 0,
                    code,
                    link: self.links.last().cloned(),
                };
                self.push_inline(Inline::text(text, marks));
            }
        }
    }

    fn inline_html(&mut self, html: &str) {
        let tag = html.trim();
        match tag.to_ascii_lowercase().as_str() {
            "<strong>" | "<b>" => self.strong += 1,
            "</strong>" | "</b>" => self.strong = self.strong.saturating_sub(1),
            "<em>" | "<i>" => self.em += 1,
            "</em>" | "</i>" => self.em = self.em.saturating_sub(1),
            "<del>" | "<s>" | "<strike>" => self.strike += 1,
            "</del>" | "</s>" | "</strike>" => self.strike = self.strike.saturating_sub(1),
            _ => match preserved_span(tag) {
                Some(preserved) => {
                    let fallback = preserved.fallback_text();
                    self.push_inline(Inline::Preserved(preserved));
                    self.skip = Some(Skip {
                        depth: 1,
                        fallback,
                        held: Vec::new(),
                    });
                }
                None => self.push_inline(Inline::Html(html.to_string())),
            },
        }
    }
}

fn source_name(source: Source) -> &'static str {
    match source {
        Source::Adf => "adf",
        Source::Html => "html",
    }
}

/// Reads `<span data-gespan-adf="…">` (or `-html`), as [`preserved_span_html`]
/// writes it.
fn preserved_span(tag: &str) -> Option<Preserved> {
    let rest = tag.strip_prefix("<span data-gespan-")?;
    let (source, rest) = [Source::Adf, Source::Html].into_iter().find_map(|source| {
        rest.strip_prefix(source_name(source))
            .and_then(|rest| rest.strip_prefix("=\""))
            .map(|rest| (source, rest))
    })?;
    let value = rest.strip_suffix("\">")?;
    (!value.contains('"')).then(|| Preserved {
        source,
        raw: decode_html_entities(value),
    })
}

/// The read-only form of preserved inline content, shared with the HTML
/// renderer.
pub(crate) fn preserved_span_html(preserved: &Preserved, fallback: &str) -> String {
    let mut value = String::new();
    for c in preserved.raw.chars() {
        match c {
            '&' => value.push_str("&amp;"),
            '"' => value.push_str("&quot;"),
            '<' => value.push_str("&lt;"),
            '>' => value.push_str("&gt;"),
            '\n' => value.push_str("&#10;"),
            c => value.push(c),
        }
    }
    format!(
        "<span data-gespan-{}=\"{value}\">{fallback}</span>",
        source_name(preserved.source)
    )
}

pub(crate) fn render(doc: &Doc) -> String {
    render_blocks(&doc.blocks).join("\n")
}

fn render_blocks(blocks: &[Block]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut previous: Option<&Block> = None;
    // Two lists in a row merge unless their markers differ.
    let mut bullet_alt = false;
    let mut ordered_alt = false;
    for block in blocks {
        match (previous, block) {
            (Some(Block::BulletList(_)), Block::BulletList(_)) => bullet_alt = !bullet_alt,
            (_, Block::BulletList(_)) => bullet_alt = false,
            (Some(Block::OrderedList { .. }), Block::OrderedList { .. }) => {
                ordered_alt = !ordered_alt
            }
            (_, Block::OrderedList { .. }) => ordered_alt = false,
            _ => {}
        }
        if previous.is_some() {
            lines.push(String::new());
        }
        lines.extend(render_block(block, bullet_alt, ordered_alt));
        previous = Some(block);
    }
    lines
}

fn render_block(block: &Block, bullet_alt: bool, ordered_alt: bool) -> Vec<String> {
    match block {
        Block::Paragraph(content) => render_inlines(content, false)
            .split('\n')
            .map(str::to_string)
            .collect(),
        Block::Heading { level, content } => vec![format!(
            "{} {}",
            "#".repeat(usize::from(*level)),
            render_inlines(content, true)
        )],
        Block::BulletList(items) => {
            let marker = if bullet_alt { "* " } else { "- " };
            items
                .iter()
                .flat_map(|item| list_item(render_blocks(item), marker))
                .collect()
        }
        Block::OrderedList { start, items } => {
            let delimiter = if ordered_alt { ')' } else { '.' };
            items
                .iter()
                .enumerate()
                .flat_map(|(i, item)| {
                    let number = start
                        .checked_add(i as u64)
                        .filter(|n| *n <= MAX_LIST_START)
                        .unwrap_or(*start);
                    list_item(render_blocks(item), &format!("{number}{delimiter} "))
                })
                .collect()
        }
        Block::Code { language, code } => fenced(&fenced_info(language.as_deref()), code),
        Block::Quote(blocks) => render_blocks(blocks)
            .into_iter()
            .map(|line| {
                if line.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {line}")
                }
            })
            .collect(),
        Block::Rule => vec!["---".to_string()],
        Block::Html(raw) => raw.split('\n').map(str::to_string).collect(),
        Block::Preserved(preserved) => fenced(preserved.source.fence(), &preserved.raw),
    }
}

/// A code block's info string. A real code block whose language is a kept-content
/// fence gets a second word, so it never reads back as kept content.
fn fenced_info(language: Option<&str>) -> String {
    match language {
        Some(language) if Source::from_fence(language).is_some() => format!("{language} code"),
        Some(language) => language.to_string(),
        None => String::new(),
    }
}

fn list_item(mut lines: Vec<String>, marker: &str) -> Vec<String> {
    let indent = " ".repeat(marker.len());
    for (i, line) in lines.iter_mut().enumerate() {
        if i == 0 {
            // `- ---` is a rule, not an item holding one.
            let first = if line == "---" { "___" } else { line.as_str() };
            *line = format!("{marker}{first}");
        } else if !line.is_empty() {
            *line = format!("{indent}{line}");
        }
    }
    lines
}

fn longest_run(text: &str, target: char) -> usize {
    let (mut longest, mut current) = (0, 0);
    for c in text.chars() {
        current = if c == target { current + 1 } else { 0 };
        longest = longest.max(current);
    }
    longest
}

fn fenced(info: &str, body: &str) -> Vec<String> {
    let fence_char = if info.contains('`') { '~' } else { '`' };
    let fence = fence_char
        .to_string()
        .repeat((longest_run(body, fence_char) + 1).max(3));
    let mut lines = vec![format!("{fence}{info}")];
    if !body.is_empty() {
        lines.extend(body.split('\n').map(str::to_string));
    }
    lines.push(fence);
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Strong,
    Strike,
    Em,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Open {
    Link(Link),
    Format(Format),
}

enum Piece {
    /// Plain text, escaped in [`resolve`].
    Text(String),
    Raw(String),
    /// A format mark's opening or closing, paired by id.
    Delimiter {
        format: Format,
        open: bool,
        pair: usize,
    },
    Break,
}

/// The marks a run needs open, outermost first.
fn wanted(marks: &Marks) -> Vec<Open> {
    let mut wanted = Vec::new();
    if let Some(link) = &marks.link {
        wanted.push(Open::Link(link.clone()));
    }
    for (on, format) in [
        (marks.strong, Format::Strong),
        (marks.strike, Format::Strike),
        (marks.em, Format::Em),
    ] {
        if on && !marks.code {
            wanted.push(Open::Format(format));
        }
    }
    wanted
}

struct Inlines {
    pieces: Vec<Piece>,
    stack: Vec<(Open, usize)>,
    next_pair: usize,
}

impl Inlines {
    /// Closes what `wanted` lacks (and everything opened after it), then
    /// opens what's missing.
    fn set(&mut self, wanted: &[Open]) {
        let keep = self
            .stack
            .iter()
            .take_while(|(open, _)| wanted.contains(open))
            .count();
        while self.stack.len() > keep {
            let (open, pair) = self.stack.pop().expect("stack is longer than keep");
            self.pieces.push(match open {
                Open::Link(link) => Piece::Raw(format!(
                    "]({})",
                    destination(&link.href, link.title.as_deref())
                )),
                Open::Format(format) => Piece::Delimiter {
                    format,
                    open: false,
                    pair,
                },
            });
        }
        for open in wanted {
            if self.stack.iter().any(|(o, _)| o == open) {
                continue;
            }
            let pair = self.next_pair;
            self.next_pair += 1;
            self.pieces.push(match open {
                Open::Link(_) => Piece::Raw("[".to_string()),
                Open::Format(format) => Piece::Delimiter {
                    format: *format,
                    open: true,
                    pair,
                },
            });
            self.stack.push((open.clone(), pair));
        }
    }
}

fn render_inlines(content: &[Inline], heading: bool) -> String {
    let mut out = Inlines {
        pieces: Vec::new(),
        stack: Vec::new(),
        next_pair: 0,
    };
    for inline in content {
        match inline {
            Inline::Text { text, marks } if is_autolink(text, marks) => {
                out.set(&[]);
                out.pieces.push(Piece::Raw(format!("<{text}>")));
            }
            Inline::Text { text, marks } => {
                out.set(&wanted(marks));
                out.pieces.push(if marks.code {
                    Piece::Raw(code_span(text))
                } else {
                    Piece::Text(text.clone())
                });
            }
            Inline::HardBreak => {
                out.set(&[]);
                out.pieces.push(Piece::Break);
            }
            Inline::Image { src, alt, link } => {
                let wanted: Vec<Open> = link.iter().cloned().map(Open::Link).collect();
                out.set(&wanted);
                out.pieces.push(Piece::Raw(format!(
                    "![{}]({})",
                    escape_text(alt, false, false),
                    destination(src, None)
                )));
            }
            Inline::Html(raw) => out.pieces.push(Piece::Raw(raw.clone())),
            Inline::Preserved(preserved) => {
                let fallback = escape_text(&preserved.fallback_text(), false, false);
                out.pieces
                    .push(Piece::Raw(preserved_span_html(preserved, &fallback)));
            }
        }
    }
    out.set(&[]);
    resolve(out.pieces, heading)
}

fn is_autolink(text: &str, marks: &Marks) -> bool {
    let Some(link) = &marks.link else {
        return false;
    };
    if marks.strong
        || marks.em
        || marks.strike
        || marks.code
        || link.title.is_some()
        || link.href != text
    {
        return false;
    }
    let Some((scheme, rest)) = text.split_once(':') else {
        return false;
    };
    let scheme_ok = (2..=32).contains(&scheme.len())
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    scheme_ok
        && !rest.is_empty()
        && !rest.chars().any(|c| {
            c.is_ascii_whitespace() || c.is_ascii_control() || matches!(c, '<' | '>' | '\\' | '&')
        })
}

fn code_span(code: &str) -> String {
    let fence = "`".repeat(longest_run(code, '`') + 1);
    let pad = !code.chars().all(|c| c == ' ')
        && (code.starts_with(['`', ' ']) || code.ends_with(['`', ' ']));
    if pad {
        format!("{fence} {code} {fence}")
    } else {
        format!("{fence}{code}{fence}")
    }
}

/// `&` only starts a character reference before a letter, digit or `#`.
fn starts_reference(rest: &str) -> bool {
    rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '#')
}

fn destination(href: &str, title: Option<&str>) -> String {
    let angle = href.is_empty()
        || href.starts_with('<')
        || href.chars().any(|c| c == ' ' || c.is_ascii_control());
    let mut out = String::new();
    if angle {
        out.push('<');
    }
    for (i, c) in href.char_indices() {
        let escape = match c {
            '\\' => true,
            '<' | '>' => angle,
            '(' | ')' => !angle,
            '&' => starts_reference(&href[i + 1..]),
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    if angle {
        out.push('>');
    }
    if let Some(title) = title {
        out.push_str(" \"");
        for (i, c) in title.char_indices() {
            if matches!(c, '\\' | '"') || (c == '&' && starts_reference(&title[i + 1..])) {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
    }
    out
}

/// Escapes text so it parses back as the same text. `line_start` covers block
/// markers; `heading_end` an ATX closing `#`.
fn escape_text(text: &str, line_start: bool, heading_end: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    // At the start of a line, `1.` or `1)` would start a list.
    let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    let list_number =
        line_start && (1..=9).contains(&digits) && matches!(chars.get(digits), Some('.' | ')'));
    let mut out = String::with_capacity(text.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        let last = i + 1 == chars.len();
        let escape = match c {
            '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '<' | '>' => true,
            '&' => chars
                .get(i + 1)
                .is_some_and(|n| n.is_ascii_alphanumeric() || *n == '#'),
            '!' => last,
            '#' if (i == 0 && line_start) || (last && heading_end) => true,
            '-' | '+' | '=' => i == 0 && line_start,
            '.' | ')' => list_number && i == digits,
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Punct,
    Alnum,
    Other,
}

fn class(c: Option<char>) -> Class {
    match c {
        None => Class::Space,
        Some(c) if c.is_ascii_whitespace() => Class::Space,
        Some(c) if c.is_ascii_punctuation() => Class::Punct,
        Some(c) if c.is_alphanumeric() => Class::Alnum,
        Some(_) => Class::Other,
    }
}

/// A conservative reading of CommonMark's flanking rules: true only when the
/// delimiter is sure to open (or close) here.
fn delimiter_parses(format: Format, open: bool, before: Class, after: Class) -> bool {
    use Class::{Alnum, Punct, Space};
    let (inside, outside) = if open {
        (after, before)
    } else {
        (before, after)
    };
    match format {
        // `*` and `~` runs may sit inside a word.
        Format::Strong | Format::Strike => {
            inside == Alnum || (inside == Punct && matches!(outside, Space | Punct))
        }
        // `_` must also have a space or punctuation outside.
        Format::Em => matches!(inside, Alnum | Punct) && matches!(outside, Space | Punct),
    }
}

fn resolve(pieces: Vec<Piece>, heading: bool) -> String {
    // Escape text first: its first and last characters decide the delimiters.
    let mut line_start = true;
    let count = pieces.len();
    let texts: Vec<Option<String>> = pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| match piece {
            Piece::Text(text) => {
                let escaped = escape_text(text, line_start, heading && i + 1 == count);
                line_start = false;
                Some(escaped)
            }
            Piece::Break => {
                line_start = true;
                None
            }
            _ => {
                line_start = false;
                None
            }
        })
        .collect();
    let edge = |i: usize, first: bool| -> Option<char> {
        match &pieces[i] {
            Piece::Text(_) => {
                let text = texts[i].as_deref().unwrap_or_default();
                if first {
                    text.chars().next()
                } else {
                    text.chars().last()
                }
            }
            Piece::Raw(raw) => {
                if first {
                    raw.chars().next()
                } else {
                    raw.chars().last()
                }
            }
            // Both forms of a delimiter are ASCII punctuation.
            Piece::Delimiter { .. } => Some('*'),
            Piece::Break => None,
        }
    };
    let mut broken = std::collections::HashSet::new();
    for (i, piece) in pieces.iter().enumerate() {
        if let Piece::Delimiter { format, open, pair } = piece {
            let before = class(i.checked_sub(1).and_then(|p| edge(p, false)));
            let after = class(if i + 1 < count {
                edge(i + 1, true)
            } else {
                None
            });
            if !delimiter_parses(*format, *open, before, after) {
                broken.insert(*pair);
            }
        }
    }
    let mut out = String::new();
    for (i, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(_) => out.push_str(texts[i].as_deref().unwrap_or_default()),
            Piece::Raw(raw) => out.push_str(raw),
            Piece::Break => out.push_str("\\\n"),
            Piece::Delimiter { format, open, pair } => {
                let (delimiter, tag) = match format {
                    Format::Strong => ("**", "strong"),
                    Format::Strike => ("~~", "del"),
                    Format::Em => ("_", "em"),
                };
                if !broken.contains(pair) {
                    out.push_str(delimiter);
                } else if *open {
                    out.push_str(&format!("<{tag}>"));
                } else {
                    out.push_str(&format!("</{tag}>"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::doc::{normalize, Block, Doc, Inline, Link, Marks, Preserved, Source};
    use super::super::strategies::{doc, Profile, ADF_INLINES};
    use super::*;

    fn plain(text: &str) -> Inline {
        Inline::text(text, Marks::plain())
    }

    fn strong(text: &str) -> Inline {
        Inline::text(
            text,
            Marks {
                strong: true,
                ..Marks::plain()
            },
        )
    }

    fn para(content: Vec<Inline>) -> Block {
        Block::Paragraph(content)
    }

    fn round_trip(doc: &Doc) -> Doc {
        parse(&render(doc))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn markdown_round_trips_any_document(doc in doc(Profile::Markdown)) {
            let markdown = render(&doc);
            prop_assert_eq!(parse(&markdown), doc, "markdown:\n{}", markdown);
        }

        #[test]
        fn markdown_render_is_stable(doc in doc(Profile::Markdown)) {
            let markdown = render(&doc);
            prop_assert_eq!(render(&parse(&markdown)), markdown);
        }
    }

    #[test]
    fn parses_the_basic_constructs() {
        let doc = parse(
            "# Title\n\nSome **bold**, _em_, ~~gone~~ and `code` with a [link](https://x.test \"T\").\n\n\
             - one\n- two\n  1. nested\n\n```rust\nfn main() {}\n```\n\n> quoted\n\n---\n",
        );
        let link = Marks {
            link: Some(Link {
                href: "https://x.test".into(),
                title: Some("T".into()),
            }),
            ..Marks::plain()
        };
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    content: vec![plain("Title")],
                },
                para(vec![
                    plain("Some "),
                    strong("bold"),
                    plain(", "),
                    Inline::text(
                        "em",
                        Marks {
                            em: true,
                            ..Marks::plain()
                        }
                    ),
                    plain(", "),
                    Inline::text(
                        "gone",
                        Marks {
                            strike: true,
                            ..Marks::plain()
                        }
                    ),
                    plain(" and "),
                    Inline::text(
                        "code",
                        Marks {
                            code: true,
                            ..Marks::plain()
                        }
                    ),
                    plain(" with a "),
                    Inline::text("link", link),
                    plain("."),
                ]),
                Block::BulletList(vec![
                    vec![para(vec![plain("one")])],
                    vec![
                        para(vec![plain("two")]),
                        Block::OrderedList {
                            start: 1,
                            items: vec![vec![para(vec![plain("nested")])]],
                        },
                    ],
                ]),
                Block::Code {
                    language: Some("rust".into()),
                    code: "fn main() {}".into(),
                },
                Block::Quote(vec![para(vec![plain("quoted")])]),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn punctuation_stays_plain_text() {
        for text in [
            "#123 *not* 1. [x] <b> \\ _a_ ~~s~~ `c` &amp; ![i](j)",
            "1. looks like a list",
            "- also a list",
            "+ and this",
            "===",
            "# heading?",
            "2024) a year",
            "ends with #",
        ] {
            let doc = normalize(Doc {
                blocks: vec![para(vec![plain(text)])],
            });
            assert_eq!(
                round_trip(&doc),
                doc,
                "{text:?} rendered as {:?}",
                render(&doc)
            );
        }
        let heading = normalize(Doc {
            blocks: vec![Block::Heading {
                level: 2,
                content: vec![plain("Ends with #")],
            }],
        });
        assert_eq!(round_trip(&heading), heading);
    }

    #[test]
    fn emphasis_falls_back_to_tags_when_flanking_fails() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![strong("\"a\""), plain("b")])],
        });
        let markdown = render(&doc);
        assert_eq!(markdown, "<strong>\"a\"</strong>b");
        assert_eq!(parse(&markdown), doc);

        let em = Marks {
            em: true,
            ..Marks::plain()
        };
        let intraword = normalize(Doc {
            blocks: vec![para(vec![
                plain("snake"),
                Inline::text("case", em),
                plain("d"),
            ])],
        });
        assert_eq!(render(&intraword), "snake<em>case</em>d");
        assert_eq!(round_trip(&intraword), intraword);
    }

    #[test]
    fn delimiters_are_used_when_they_parse() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![plain("a "), strong("b"), plain(" c")])],
        });
        assert_eq!(render(&doc), "a **b** c");
    }

    #[test]
    fn adjacent_lists_stay_separate() {
        let item = |text: &str| vec![para(vec![plain(text)])];
        let doc = normalize(Doc {
            blocks: vec![
                Block::BulletList(vec![item("a")]),
                Block::BulletList(vec![item("b")]),
                Block::OrderedList {
                    start: 1,
                    items: vec![item("c")],
                },
                Block::OrderedList {
                    start: 5,
                    items: vec![item("d")],
                },
            ],
        });
        assert_eq!(render(&doc), "- a\n\n* b\n\n1. c\n\n5) d");
        assert_eq!(round_trip(&doc), doc);
    }

    #[test]
    fn code_span_with_backticks_and_edge_spaces() {
        let code = Marks {
            code: true,
            ..Marks::plain()
        };
        for text in ["a`b", "`edge", " lead", "trail ", " ", "``"] {
            let doc = normalize(Doc {
                blocks: vec![para(vec![plain("x "), Inline::text(text, code.clone())])],
            });
            assert_eq!(
                round_trip(&doc),
                doc,
                "{text:?} rendered as {:?}",
                render(&doc)
            );
        }
    }

    #[test]
    fn fence_longer_than_code_backticks() {
        let doc = normalize(Doc {
            blocks: vec![Block::Code {
                language: Some("md".into()),
                code: "```\ninner\n````".into(),
            }],
        });
        assert_eq!(render(&doc), "`````md\n```\ninner\n````\n`````");
        assert_eq!(round_trip(&doc), doc);
    }

    #[test]
    fn tight_and_loose_lists_parse_alike() {
        assert_eq!(parse("- a\n- b"), parse("- a\n\n- b"));
    }

    #[test]
    fn soft_breaks_become_spaces() {
        assert_eq!(parse("one\ntwo").blocks, vec![para(vec![plain("one two")])]);
        assert_eq!(
            parse("one\\\ntwo").blocks,
            vec![para(vec![plain("one"), Inline::HardBreak, plain("two")])]
        );
    }

    #[test]
    fn autolink_when_text_is_the_url() {
        let url = "https://x.test/a?b=1";
        let doc = normalize(Doc {
            blocks: vec![para(vec![Inline::text(
                url,
                Marks {
                    link: Some(Link {
                        href: url.into(),
                        title: None,
                    }),
                    ..Marks::plain()
                },
            )])],
        });
        assert_eq!(render(&doc), format!("<{url}>"));
        assert_eq!(round_trip(&doc), doc);
    }

    fn mention() -> Preserved {
        Preserved {
            source: Source::Adf,
            raw: ADF_INLINES[0].into(),
        }
    }

    #[test]
    fn preserved_inline_renders_as_a_read_only_span() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![plain("ping "), Inline::Preserved(mention())])],
        });
        let markdown = render(&doc);
        assert!(
            markdown.starts_with("ping <span data-gespan-adf=\"{&quot;attrs&quot;:"),
            "{markdown}"
        );
        assert!(markdown.ends_with("\">@Alice</span>"), "{markdown}");
        assert_eq!(parse(&markdown), doc);
    }

    #[test]
    fn preserved_inline_survives_edited_fallback() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![
                plain("ping "),
                Inline::Preserved(mention()),
                plain(" now"),
            ])],
        });
        let edited = render(&doc).replace(">@Alice<", ">@Bob *was* here<");
        assert_eq!(parse(&edited), doc);
    }

    #[test]
    fn preserved_inline_survives_missing_close_tag() {
        let doc = normalize(Doc {
            blocks: vec![
                para(vec![plain("ping "), Inline::Preserved(mention())]),
                para(vec![plain("next")]),
            ],
        });
        let broken = render(&doc).replace("</span>", "");
        assert_eq!(parse(&broken), doc);
    }

    #[test]
    fn missing_close_tag_keeps_the_text_after_the_span() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![
                plain("cc "),
                Inline::Preserved(mention()),
                plain(" please review the "),
                strong("PR"),
                plain(" and "),
                Inline::Preserved(mention()),
                plain(" today"),
            ])],
        });
        let broken = render(&doc).replacen("</span>", "", 1);
        assert_eq!(parse(&broken), doc, "{broken}");
        let both = render(&doc).replace("</span>", "");
        assert_eq!(parse(&both), doc, "{both}");
    }

    #[test]
    fn edited_fallback_with_markup_is_ignored() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![
                plain("ping "),
                Inline::Preserved(mention()),
                plain(" now"),
            ])],
        });
        let edited = render(&doc).replace(">@Alice<", ">@Alice and [Bob](https://x.test) `too`<");
        assert_eq!(parse(&edited), doc);
    }

    #[test]
    fn deep_nesting_is_flattened_not_fatal() {
        for markdown in [
            format!("{} x", ">".repeat(10_000)),
            format!("{}x", "- ".repeat(5_000)),
        ] {
            let doc = parse(&markdown);
            assert_eq!(render(&doc).matches('x').count(), 1);
            assert_eq!(parse(&render(&doc)), doc);
        }
    }

    #[test]
    fn only_an_exact_gespan_fence_is_kept_content() {
        let code = |language: &str| Block::Code {
            language: Some(language.into()),
            code: "<b>x</b>".into(),
        };
        let doc = normalize(Doc {
            blocks: vec![code("gespan-html")],
        });
        assert_eq!(render(&doc), "```gespan-html code\n<b>x</b>\n```");
        assert_eq!(round_trip(&doc), doc);
        assert_eq!(
            parse("```gespan-html extra\n<b>x</b>\n```").blocks,
            vec![code("gespan-html")]
        );
    }

    #[test]
    fn preserved_block_is_a_gespan_fence() {
        let doc = normalize(Doc {
            blocks: vec![Block::Preserved(Preserved {
                source: Source::Html,
                raw: "<table><tr><td>```</td></tr></table>".into(),
            })],
        });
        let markdown = render(&doc);
        assert_eq!(
            markdown,
            "````gespan-html\n<table><tr><td>```</td></tr></table>\n````"
        );
        assert_eq!(parse(&markdown), doc);
    }

    #[test]
    fn raw_html_written_in_markdown_is_kept() {
        let doc = parse("<details>\n<summary>More</summary>\n</details>\n\nPress <kbd>K</kbd>");
        assert_eq!(
            doc.blocks,
            vec![
                Block::Html("<details>\n<summary>More</summary>\n</details>".into()),
                para(vec![
                    plain("Press "),
                    Inline::Html("<kbd>".into()),
                    plain("K"),
                    Inline::Html("</kbd>".into()),
                ]),
            ]
        );
        assert_eq!(round_trip(&doc), doc);
    }
}
