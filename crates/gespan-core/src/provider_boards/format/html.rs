//! Plane's `description_html` to and from the document model.
//!
//! Plane's editor writes simple, well-formed HTML, but descriptions can also
//! come from the API or older clients, so the parser is lenient: stray end
//! tags are ignored, unclosed tags close at their parent's end, and a `<` that
//! doesn't start a tag is text. Elements or attributes the model doesn't know
//! are kept as their exact source.

use super::doc::{
    decode_html_entities, normalize, Block, Doc, Inline, Link, Marks, Preserved, Source,
};
use super::markdown::{preserved_span_html, MAX_DEPTH};

pub(crate) fn parse(html: &str) -> Doc {
    // HTML reads CR and CRLF as LF before anything else.
    let html = html.replace("\r\n", "\n").replace('\r', "\n");
    let root = Tree::build(&html);
    normalize(Doc {
        blocks: blocks(&root, &html),
    })
}

enum Node {
    Element(Element),
    Text(String),
}

struct Element {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    /// The element's source is `html[start..end]`.
    start: usize,
    end: usize,
    /// Without an end tag, an element's source stops after its last content.
    /// A raw-text element (`<script>`) also gets its end tag added, since its
    /// content would otherwise run on into whatever follows.
    missing_end_tag: bool,
    content_end: usize,
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
/// Elements whose content is text up to the matching end tag.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];
const INLINE: &[&str] = &[
    "a",
    "abbr",
    "b",
    "bdi",
    "bdo",
    "br",
    "cite",
    "code",
    "data",
    "del",
    "dfn",
    "em",
    "i",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "mark",
    "mention-component",
    "q",
    "s",
    "samp",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "time",
    "u",
    "var",
    "wbr",
];
/// Attributes that change nothing we keep, allowed on any element.
const IGNORED_ATTRS: &[&str] = &["class", "data-tight", "rel", "spellcheck", "target"];

struct Tree;

impl Tree {
    fn build(html: &str) -> Element {
        let mut stack = vec![Element {
            tag: String::new(),
            attrs: Vec::new(),
            children: Vec::new(),
            start: 0,
            end: html.len(),
            missing_end_tag: false,
            content_end: 0,
        }];
        let bytes = html.as_bytes();
        let mut i = 0;
        let mut text_start = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                i += 1;
                continue;
            }
            let Some(token) = tag_at(html, i) else {
                i += 1;
                continue;
            };
            push_text(&mut stack, &html[text_start..i], i);
            match token {
                Token::Skip { end } => i = end,
                Token::End { name, end } => {
                    if let Some(depth) = stack.iter().rposition(|e| e.tag == name) {
                        if depth > 0 {
                            while stack.len() > depth + 1 {
                                close(&mut stack, None);
                            }
                            close(&mut stack, Some(end));
                        }
                    }
                    i = end;
                }
                Token::Start {
                    name,
                    attrs,
                    end,
                    self_closing,
                } => {
                    // A block start ends an open paragraph, and a list item
                    // ends the previous one, as browsers do.
                    let top = stack.last().map(|e| e.tag.as_str()).unwrap_or_default();
                    if (top == "p" && !INLINE.contains(&name.as_str()))
                        || (top == "li" && name == "li")
                    {
                        close(&mut stack, None);
                    }
                    let mut element = Element {
                        tag: name,
                        attrs,
                        children: Vec::new(),
                        start: i,
                        end,
                        missing_end_tag: false,
                        content_end: end,
                    };
                    i = end;
                    if RAW_TEXT.contains(&element.tag.as_str()) && !self_closing {
                        let closing = format!("</{}", element.tag);
                        let content_end = find_ci(html, end, &closing).unwrap_or(html.len());
                        element
                            .children
                            .push(Node::Text(html[end..content_end].to_string()));
                        match html[content_end..].find('>') {
                            Some(gt) => i = content_end + gt + 1,
                            None => {
                                i = html.len();
                                element.missing_end_tag = true;
                            }
                        }
                        element.end = i;
                        push_node(&mut stack, Node::Element(element), i);
                    } else if self_closing
                        || VOID.contains(&element.tag.as_str())
                        || stack.len() >= MAX_DEPTH
                    {
                        // Past MAX_DEPTH an element is left empty and its
                        // content joins the parent, so recursion stays bounded.
                        push_node(&mut stack, Node::Element(element), end);
                    } else {
                        stack.push(element);
                    }
                }
            }
            text_start = i;
        }
        push_text(&mut stack, &html[text_start..], html.len());
        while stack.len() > 1 {
            close(&mut stack, None);
        }
        stack.pop().expect("the root is never popped")
    }
}

/// Adds a child that ends at `end` in the source.
fn push_node(stack: &mut [Element], node: Node, end: usize) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
        parent.content_end = end;
    }
}

fn push_text(stack: &mut [Element], text: &str, end: usize) {
    if !text.is_empty() {
        push_node(stack, Node::Text(decode_html_entities(text)), end);
    }
}

/// Closes the innermost element, at its end tag's end, or implicitly.
fn close(stack: &mut Vec<Element>, end_tag: Option<usize>) {
    if let Some(mut element) = stack.pop() {
        match end_tag {
            Some(end) => element.end = end,
            None => element.end = element.content_end,
        }
        let end = element.end;
        push_node(stack, Node::Element(element), end);
    }
}

/// Finds an ASCII `needle` from `from`, ignoring ASCII case, without copying.
fn find_ci(html: &str, from: usize, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    html.as_bytes()[from..]
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|at| from + at)
}

enum Token {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        end: usize,
        self_closing: bool,
    },
    End {
        name: String,
        end: usize,
    },
    /// A comment, doctype or processing instruction.
    Skip {
        end: usize,
    },
}

/// The tag starting at `html[at] == '<'`, or `None` when it's just text.
fn tag_at(html: &str, at: usize) -> Option<Token> {
    let rest = &html[at..];
    if let Some(comment) = rest.strip_prefix("<!--") {
        let end = comment.find("-->").map_or(html.len(), |e| at + 4 + e + 3);
        return Some(Token::Skip { end });
    }
    if rest.starts_with("<!") || rest.starts_with("<?") {
        let end = rest.find('>').map_or(html.len(), |e| at + e + 1);
        return Some(Token::Skip { end });
    }
    let (closing, name_at) = if rest.starts_with("</") {
        (true, 2)
    } else {
        (false, 1)
    };
    let name_len = rest[name_at..]
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
        .unwrap_or(rest.len() - name_at);
    if name_len == 0 || !rest[name_at..].starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name = rest[name_at..name_at + name_len].to_ascii_lowercase();
    let bytes = rest.as_bytes();
    let mut i = name_at + name_len;
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i)? {
            b'>' => {
                let end = at + i + 1;
                return Some(if closing {
                    Token::End { name, end }
                } else {
                    Token::Start {
                        name,
                        attrs,
                        end,
                        self_closing: false,
                    }
                });
            }
            b'/' if bytes.get(i + 1) == Some(&b'>') => {
                let end = at + i + 2;
                return Some(if closing {
                    Token::End { name, end }
                } else {
                    Token::Start {
                        name,
                        attrs,
                        end,
                        self_closing: true,
                    }
                });
            }
            b'/' => i += 1,
            // A `<` inside a tag means it was text, or the tag would swallow
            // the one after it (quoted values are checked the same way).
            b'<' => return None,
            _ => {
                let start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b'=' | b'>' | b'/' | b'<')
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                let key = rest[start..i].to_ascii_lowercase();
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                let mut value = String::new();
                if bytes.get(i) == Some(&b'=') {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    match bytes.get(i)? {
                        quote @ (b'"' | b'\'') => {
                            let close = rest[i + 1..].find(*quote as char)?;
                            // Serializers escape `<` in values. A raw one
                            // means this isn't a tag, and stopping here keeps
                            // every attempt short of the next `<`.
                            if rest[i + 1..i + 1 + close].contains('<') {
                                return None;
                            }
                            value = decode_html_entities(&rest[i + 1..i + 1 + close]);
                            i += close + 2;
                        }
                        _ => {
                            let start = i;
                            while i < bytes.len()
                                && bytes[i] != b'>'
                                && !bytes[i].is_ascii_whitespace()
                            {
                                if bytes[i] == b'<' {
                                    return None;
                                }
                                i += 1;
                            }
                            value = decode_html_entities(&rest[start..i]);
                        }
                    }
                }
                attrs.push((key, value));
            }
        }
    }
}

impl Element {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// True when every attribute is ignorable or in `known`.
    fn attrs_within(&self, known: &[&str]) -> bool {
        self.attrs
            .iter()
            .all(|(k, _)| IGNORED_ATTRS.contains(&k.as_str()) || known.contains(&k.as_str()))
    }

    fn text_content(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                Node::Text(text) => out.push_str(text),
                Node::Element(element) => out.push_str(&element.text_content()),
            }
        }
        out
    }

    /// The exact source; see [`Element::missing_end_tag`].
    fn preserved(&self, html: &str) -> Preserved {
        let mut raw = html[self.start..self.end].to_string();
        if self.missing_end_tag {
            raw.push_str(&format!("</{}>", self.tag));
        }
        Preserved {
            source: Source::Html,
            raw,
        }
    }
}

fn is_inline(node: &Node) -> bool {
    match node {
        Node::Text(_) => true,
        Node::Element(element) => INLINE.contains(&element.tag.as_str()),
    }
}

/// Block content: runs of inline nodes become paragraphs.
fn blocks(parent: &Element, html: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut run: Vec<&Node> = Vec::new();
    for child in &parent.children {
        if is_inline(child) {
            run.push(child);
            continue;
        }
        if !run.is_empty() {
            out.push(Block::Paragraph(inlines(
                &std::mem::take(&mut run),
                &Marks::plain(),
                html,
            )));
        }
        if let Node::Element(element) = child {
            out.push(
                block(element, html).unwrap_or_else(|| Block::Preserved(element.preserved(html))),
            );
        }
    }
    if !run.is_empty() {
        out.push(Block::Paragraph(inlines(&run, &Marks::plain(), html)));
    }
    out
}

fn block(element: &Element, html: &str) -> Option<Block> {
    let children: Vec<&Node> = element.children.iter().collect();
    match element.tag.as_str() {
        "p" if element.attrs_within(&[]) => {
            Some(Block::Paragraph(inlines(&children, &Marks::plain(), html)))
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if element.attrs_within(&[]) => {
            Some(Block::Heading {
                level: element.tag[1..].parse().ok()?,
                content: inlines(&children, &Marks::plain(), html),
            })
        }
        "ul" if element.attrs_within(&[]) => Some(Block::BulletList(items(element, html)?)),
        "ol" if element.attrs_within(&["start"]) => Some(Block::OrderedList {
            start: match element.attr("start") {
                None => 1,
                Some(start) => start.trim().parse().ok()?,
            },
            items: items(element, html)?,
        }),
        "pre" if element.attrs_within(&[]) => {
            let mut language = None;
            let mut code = element.text_content();
            if let [Node::Element(inner)] = element.children.as_slice() {
                if inner.tag == "code" {
                    if !inner.attrs_within(&[]) {
                        return None;
                    }
                    language = inner.attr("class").and_then(|class| {
                        class
                            .split_whitespace()
                            .find_map(|c| c.strip_prefix("language-"))
                            .map(str::to_string)
                    });
                    code = inner.text_content();
                }
            }
            Some(Block::Code { language, code })
        }
        "blockquote" if element.attrs_within(&[]) => Some(Block::Quote(blocks(element, html))),
        "hr" if element.attrs_within(&[]) => Some(Block::Rule),
        _ => None,
    }
}

/// List items; `None` when the list holds anything else.
fn items(list: &Element, html: &str) -> Option<Vec<Vec<Block>>> {
    let mut items = Vec::new();
    for child in &list.children {
        match child {
            Node::Text(text) if text.trim().is_empty() => {}
            Node::Element(item) if item.tag == "li" && item.attrs_within(&[]) => {
                items.push(blocks(item, html))
            }
            _ => return None,
        }
    }
    Some(items)
}

fn inlines(nodes: &[&Node], marks: &Marks, html: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            Node::Text(text) => out.push(Inline::text(text, marks.clone())),
            Node::Element(element) => inline(element, marks, html, &mut out),
        }
    }
    out
}

/// True when the model can hold the element and everything inside it.
fn representable(element: &Element) -> bool {
    let known = match element.tag.as_str() {
        "strong" | "b" | "em" | "i" | "s" | "del" | "strike" | "code" | "br" => {
            element.attrs_within(&[])
        }
        "a" => element.attrs_within(&["href", "title"]) && element.attr("href").is_some(),
        "img" => element.attrs_within(&["src", "alt"]) && element.attr("src").is_some(),
        _ => false,
    };
    known
        && (element.tag == "code"
            || element.children.iter().all(|child| match child {
                Node::Text(_) => true,
                Node::Element(inner) => representable(inner),
            }))
}

fn inline(element: &Element, marks: &Marks, html: &str, out: &mut Vec<Inline>) {
    // A link or bold around something unknown is kept whole, marks and all.
    if !representable(element) {
        out.push(Inline::Preserved(element.preserved(html)));
        return;
    }
    let children: Vec<&Node> = element.children.iter().collect();
    let mut inner = marks.clone();
    let known = match element.tag.as_str() {
        "strong" | "b" if element.attrs_within(&[]) => {
            inner.strong = true;
            true
        }
        "em" | "i" if element.attrs_within(&[]) => {
            inner.em = true;
            true
        }
        "s" | "del" | "strike" if element.attrs_within(&[]) => {
            inner.strike = true;
            true
        }
        "code" if element.attrs_within(&[]) => {
            inner.code = true;
            out.push(Inline::text(&element.text_content(), inner));
            return;
        }
        "a" if element.attrs_within(&["href", "title"]) => match element.attr("href") {
            Some(href) => {
                inner.link = Some(Link {
                    href: href.to_string(),
                    title: element
                        .attr("title")
                        .filter(|t| !t.is_empty())
                        .map(str::to_string),
                });
                true
            }
            None => false,
        },
        "br" if element.attrs_within(&[]) => {
            out.push(Inline::HardBreak);
            return;
        }
        "img" if element.attrs_within(&["src", "alt"]) => match element.attr("src") {
            Some(src) => {
                out.push(Inline::Image {
                    src: src.to_string(),
                    alt: element.attr("alt").unwrap_or_default().to_string(),
                    link: marks.link.clone(),
                });
                return;
            }
            None => false,
        },
        _ => false,
    };
    if known {
        out.extend(inlines(&children, &inner, html));
    } else {
        out.push(Inline::Preserved(element.preserved(html)));
    }
}

pub(crate) fn render(doc: &Doc) -> String {
    let mut out = String::new();
    render_blocks(&doc.blocks, &mut out);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

fn render_blocks(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Paragraph(content) => {
                out.push_str("<p>");
                render_inlines(content, out);
                out.push_str("</p>");
            }
            Block::Heading { level, content } => {
                out.push_str(&format!("<h{level}>"));
                render_inlines(content, out);
                out.push_str(&format!("</h{level}>"));
            }
            Block::BulletList(items) => {
                out.push_str("<ul>");
                render_items(items, out);
                out.push_str("</ul>");
            }
            Block::OrderedList { start, items } => {
                if *start == 1 {
                    out.push_str("<ol>");
                } else {
                    out.push_str(&format!("<ol start=\"{start}\">"));
                }
                render_items(items, out);
                out.push_str("</ol>");
            }
            Block::Code { language, code } => render_code(language.as_deref(), code, out),
            Block::Quote(blocks) => {
                out.push_str("<blockquote>");
                render_blocks(blocks, out);
                out.push_str("</blockquote>");
            }
            Block::Rule => out.push_str("<hr>"),
            Block::Html(raw) => out.push_str(raw),
            Block::Preserved(preserved) => match preserved.source {
                Source::Html => out.push_str(&preserved.raw),
                Source::Adf => render_code(Some(Source::Adf.fence()), &preserved.raw, out),
            },
        }
    }
}

fn render_code(language: Option<&str>, code: &str, out: &mut String) {
    match language {
        Some(language) => out.push_str(&format!(
            "<pre><code class=\"language-{}\">",
            escape_attr(language)
        )),
        None => out.push_str("<pre><code>"),
    }
    out.push_str(&escape(code));
    out.push_str("</code></pre>");
}

fn render_items(items: &[Vec<Block>], out: &mut String) {
    for item in items {
        out.push_str("<li>");
        render_blocks(item, out);
        out.push_str("</li>");
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Open {
    Link(Link),
    Strong,
    Strike,
    Em,
}

impl Open {
    fn tags(&self) -> (String, &'static str) {
        match self {
            Open::Link(link) => {
                let mut open = format!("<a href=\"{}\"", escape_attr(&link.href));
                if let Some(title) = &link.title {
                    open.push_str(&format!(" title=\"{}\"", escape_attr(title)));
                }
                open.push('>');
                (open, "</a>")
            }
            Open::Strong => ("<strong>".into(), "</strong>"),
            Open::Strike => ("<s>".into(), "</s>"),
            Open::Em => ("<em>".into(), "</em>"),
        }
    }
}

/// The elements a run needs open, outermost first.
fn wanted(marks: &Marks) -> Vec<Open> {
    let mut wanted: Vec<Open> = marks.link.iter().cloned().map(Open::Link).collect();
    if !marks.code {
        for (on, open) in [
            (marks.strong, Open::Strong),
            (marks.strike, Open::Strike),
            (marks.em, Open::Em),
        ] {
            if on {
                wanted.push(open);
            }
        }
    }
    wanted
}

fn set(stack: &mut Vec<Open>, wanted: &[Open], out: &mut String) {
    let keep = stack
        .iter()
        .take_while(|open| wanted.contains(open))
        .count();
    while stack.len() > keep {
        if let Some(open) = stack.pop() {
            out.push_str(open.tags().1);
        }
    }
    for open in wanted {
        if !stack.contains(open) {
            out.push_str(&open.tags().0);
            stack.push(open.clone());
        }
    }
}

fn render_inlines(content: &[Inline], out: &mut String) {
    let mut stack = Vec::new();
    for inline in content {
        match inline {
            Inline::Text { text, marks } => {
                set(&mut stack, &wanted(marks), out);
                if marks.code {
                    out.push_str(&format!("<code>{}</code>", escape(text)));
                } else {
                    out.push_str(&escape(text));
                }
            }
            Inline::HardBreak => out.push_str("<br>"),
            Inline::Image { src, alt, link } => {
                let wanted: Vec<Open> = link.iter().cloned().map(Open::Link).collect();
                set(&mut stack, &wanted, out);
                out.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\">",
                    escape_attr(src),
                    escape_attr(alt)
                ));
            }
            // Kept content carries no marks of its own, and inside a known
            // element it would make that whole element kept.
            Inline::Html(raw) => {
                set(&mut stack, &[], out);
                out.push_str(raw);
            }
            Inline::Preserved(preserved) => match preserved.source {
                Source::Html => {
                    set(&mut stack, &[], out);
                    out.push_str(&preserved.raw);
                }
                Source::Adf => out.push_str(&preserved_span_html(
                    preserved,
                    &escape(&preserved.fallback_text()),
                )),
            },
        }
    }
    set(&mut stack, &[], out);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::doc::{Block, Inline, Link, Marks, Preserved, Source};
    use super::super::markdown;
    use super::super::strategies::{doc, Profile};
    use super::super::{markdown_to_plane_html, plane_html_to_markdown};
    use super::*;

    fn plain(text: &str) -> Inline {
        Inline::text(text, Marks::plain())
    }

    fn para(content: Vec<Inline>) -> Block {
        Block::Paragraph(content)
    }

    fn preserved_html(raw: &str) -> Preserved {
        Preserved {
            source: Source::Html,
            raw: raw.into(),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn html_round_trips_any_document(doc in doc(Profile::Html)) {
            let html = render(&doc);
            prop_assert_eq!(parse(&html), doc, "html: {}", html);
        }

        #[test]
        fn any_input_converts_and_settles(
            html in "(<[/!?]?[a-z]{0,3}( [a-z]{1,3}(=(\"[^\"]{0,3}\"|'[^']{0,3}'|[a-zé]{0,3}))?)*/?>?|&[a-z#0-9]{0,4};?|[a-zé😀 <>&\"'=/-]){0,40}",
        ) {
            // Tag soup with literal `<` inside a kept element can read
            // differently once written back, but one more round settles it,
            // so a sync never keeps rewriting the description.
            let sync = |html: &str| markdown_to_plane_html(&plane_html_to_markdown(html));
            let once = sync(&html);
            let twice = sync(&once);
            prop_assert_eq!(sync(&twice), twice);
        }

        #[test]
        fn html_through_markdown_round_trips(doc in doc(Profile::Html)) {
            let markdown = markdown::render(&parse(&render(&doc)));
            prop_assert_eq!(parse(&render(&markdown::parse(&markdown))), doc, "markdown:\n{}", markdown);
        }
    }

    #[test]
    fn renders_the_basic_constructs() {
        let html = markdown_to_plane_html(
            "## Steps\n\n3. Open **Settings** & _more_\n\n- [Save](https://x.test?a=1&b=\"2\" \"T\")\n\n```rust\nif a < b {}\n```\n\n> ~~old~~ `x`\\\nnext\n\n---\n\n![a \"b\"](https://x.test/i.png)",
        );
        assert_eq!(
            html,
            "<h2>Steps</h2>\
             <ol start=\"3\"><li><p>Open <strong>Settings</strong> &amp; <em>more</em></p></li></ol>\
             <ul><li><p><a href=\"https://x.test?a=1&amp;b=&quot;2&quot;\" title=\"T\">Save</a></p></li></ul>\
             <pre><code class=\"language-rust\">if a &lt; b {}</code></pre>\
             <blockquote><p><s>old</s> <code>x</code><br>next</p></blockquote>\
             <hr>\
             <p><img src=\"https://x.test/i.png\" alt=\"a &quot;b&quot;\"></p>"
        );
        assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(&html)), html);
    }

    #[test]
    fn plane_editor_classes_are_ignored() {
        let html = concat!(
            r#"<h1 class="editor-heading-block">Title</h1>"#,
            r#"<p class="editor-paragraph-block">Run <code class="rounded bg-custom-background-80 font-mono" spellcheck="false">make</code> "#,
            r#"then <a target="_blank" rel="noopener noreferrer nofollow" class="text-custom-primary-300 underline" href="https://x.test">docs</a></p>"#,
            r#"<ul class="list-disc pl-7 space-y-[--list-spacing-y] tight" data-tight="true"><li class="not-prose space-y-2"><p class="editor-paragraph-block">one</p></li></ul>"#,
            r#"<pre class=""><code class="language-typescript">const a = 1;</code></pre>"#,
        );
        assert_eq!(
            plane_html_to_markdown(html),
            "# Title\n\nRun `make` then [docs](https://x.test)\n\n- one\n\n```typescript\nconst a = 1;\n```"
        );
    }

    #[test]
    fn task_list_and_table_are_preserved_verbatim() {
        let task_list = r#"<ul data-type="taskList"><li data-checked="false" data-type="taskItem"><label><input type="checkbox"><span></span></label><div><p>todo</p></div></li></ul>"#;
        let table = r#"<table><tbody><tr><th><p>Env</p></th></tr><tr><td><p>prod</p></td></tr></tbody></table>"#;
        let html = format!("<p>before</p>{task_list}{table}<p>after</p>");
        let doc = parse(&html);
        assert_eq!(
            doc.blocks,
            vec![
                para(vec![plain("before")]),
                Block::Preserved(preserved_html(task_list)),
                Block::Preserved(preserved_html(table)),
                para(vec![plain("after")]),
            ]
        );
        assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(&html)), html);
    }

    #[test]
    fn mention_component_is_preserved_inline() {
        let mention = r#"<mention-component entity_identifier="9f2c" entity_name="user_mention"></mention-component>"#;
        let html = format!("<p>cc {mention} please</p>");
        assert_eq!(
            parse(&html).blocks,
            vec![para(vec![
                plain("cc "),
                Inline::Preserved(preserved_html(mention)),
                plain(" please"),
            ])]
        );
        assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(&html)), html);
    }

    #[test]
    fn styled_paragraph_is_preserved() {
        let html = r#"<p style="text-align: center">mid</p>"#;
        assert_eq!(
            parse(html).blocks,
            vec![Block::Preserved(preserved_html(html))]
        );
        assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(html)), html);
    }

    #[test]
    fn entities_decode_and_reencode() {
        let html = "<p>&amp; &lt;b&gt; &nbsp;x &#39; &#x1F600; &bogus; a&b</p>";
        assert_eq!(
            parse(html).blocks,
            vec![para(vec![plain("& <b> \u{a0}x ' \u{1F600} &bogus; a&b")])]
        );
        assert_eq!(
            render(&parse(html)),
            "<p>&amp; &lt;b&gt; \u{a0}x ' \u{1F600} &amp;bogus; a&amp;b</p>"
        );
    }

    #[test]
    fn whitespace_between_blocks_is_ignored() {
        let html = "\n<p>a</p>\n  <p>b <strong>c</strong> d</p>\n";
        assert_eq!(
            parse(html).blocks,
            vec![
                para(vec![plain("a")]),
                para(vec![
                    plain("b "),
                    Inline::text(
                        "c",
                        Marks {
                            strong: true,
                            ..Marks::plain()
                        }
                    ),
                    plain(" d"),
                ]),
            ]
        );
    }

    #[test]
    fn empty_plane_description_is_empty() {
        assert_eq!(plane_html_to_markdown("<p></p>"), "");
        assert_eq!(plane_html_to_markdown(""), "");
        assert_eq!(markdown_to_plane_html(""), "");
    }

    #[test]
    fn link_and_image_attributes_round_trip() {
        let link = Some(Link {
            href: "https://x.test/a b".into(),
            title: Some("say \"hi\"".into()),
        });
        let html = r#"<p><a href="https://x.test/a b" title="say &quot;hi&quot;"><img src="i.png" alt="pic">text</a></p>"#;
        assert_eq!(
            parse(html).blocks,
            vec![para(vec![
                Inline::Image {
                    src: "i.png".into(),
                    alt: "pic".into(),
                    link: link.clone(),
                },
                Inline::text(
                    "text",
                    Marks {
                        link,
                        ..Marks::plain()
                    }
                ),
            ])]
        );
        assert_eq!(render(&parse(html)), html);
    }

    #[test]
    fn malformed_html_does_not_panic_or_drop_text() {
        let cases = [
            ("<p>a <strong>bold", "a **bold**"),
            ("<p>one</p></p></div><p>two</p>", "one\n\ntwo"),
            ("<p>a < b and c > d</p>", "a \\< b and c \\> d"),
            ("<p>first<p>second</p>", "first\n\nsecond"),
            ("<ul><li>one<li>two</ul>", "- one\n- two"),
            ("<p>x <!-- note --> y</p>", "x y"),
            ("<p>half <a href=\"x\"", "half \\<a href=\"x\""),
            ("plain text", "plain text"),
            ("<p>a <b<c d</p>", "a \\<b\\<c d"),
            ("<p>x <i a=<y z</i></p>", "x \\<i a=\\<y z"),
            // Serializers escape `<` in attribute values; a raw one means
            // this isn't a tag, which also keeps the scan linear.
            (
                "<p>q <a title=\"<b>\" href=\"u\">t</a></p>",
                "q \\<a title=\"**\" href=\"u\"\\>t**",
            ),
        ];
        for (html, markdown) in cases {
            assert_eq!(plane_html_to_markdown(html), markdown, "{html}");
        }
    }

    #[test]
    fn carriage_returns_read_as_newlines() {
        // HTML treats CR and CRLF as LF before parsing; Markdown does too.
        let html = "<table title='a\r\nb\rc'></table>";
        let expected = preserved_html("<table title='a\nb\nc'></table>");
        assert_eq!(parse(html).blocks, vec![Block::Preserved(expected)]);
        assert_eq!(
            markdown_to_plane_html(&plane_html_to_markdown(html)),
            "<table title='a\nb\nc'></table>"
        );
    }

    #[test]
    fn unclosed_elements_end_at_their_last_content() {
        let raw = |html: &str| match parse(html).blocks.as_slice() {
            [Block::Paragraph(content)] => match content.as_slice() {
                [Inline::Preserved(preserved)] => preserved.raw.clone(),
                other => panic!("{html}: {other:?}"),
            },
            [Block::Preserved(preserved)] => preserved.raw.clone(),
            other => panic!("{html}: {other:?}"),
        };
        assert_eq!(raw("<p><u>abc"), "<u>abc");
        assert_eq!(raw("<p><u>abc</p>"), "<u>abc");
        // A trailing comment or bogus tag isn't content, and kept it would
        // swallow whatever follows the element when it's written back.
        assert_eq!(raw("<s a><!"), "<s a>");
        assert_eq!(raw("<p><s a>x<!-- c"), "<s a>x");
        assert_eq!(raw("<script>x"), "<script>x</script>");
        assert_eq!(raw("<table><tr><td>cell"), "<table><tr><td>cell");
        for html in ["<p><u>abc", "<s a><!", "<script>x", "<table><tr><td>cell"] {
            let doc = parse(html);
            assert_eq!(parse(&render(&doc)), doc, "{html}");
        }
    }

    #[test]
    fn unknown_inline_inside_a_known_one_keeps_the_outer_marks() {
        for (html, kept) in [
            (
                r#"<p>see <a href="https://x.test/doc"><u>the doc</u></a> now</p>"#,
                r#"<a href="https://x.test/doc"><u>the doc</u></a>"#,
            ),
            (
                "<p>see <strong>very <em><u>x</u></em></strong> now</p>",
                "<strong>very <em><u>x</u></em></strong>",
            ),
        ] {
            assert_eq!(
                parse(html).blocks,
                vec![para(vec![
                    plain("see "),
                    Inline::Preserved(preserved_html(kept)),
                    plain(" now"),
                ])],
                "{html}"
            );
            assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(html)), html);
        }
    }

    #[test]
    fn deep_nesting_is_flattened_not_fatal() {
        for html in [
            format!("{}x", "<blockquote>".repeat(10_000)),
            format!("<p>{}x</p>", "<b>".repeat(10_000)),
            format!("{}x", "<ul><li>".repeat(5_000)),
        ] {
            let markdown = plane_html_to_markdown(&html);
            assert_eq!(markdown.matches('x').count(), 1, "{}", &html[..40]);
            let back = markdown_to_plane_html(&markdown);
            assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(&back)), back);
        }
    }

    #[test]
    fn code_that_looks_like_kept_content_stays_inert() {
        // Escaped text in a tracker must never come back as live markup.
        let html = r#"<pre><code class="language-gespan-html">&lt;img src=x onerror=alert(1)&gt;</code></pre>"#;
        assert_eq!(
            parse(html).blocks,
            vec![Block::Code {
                language: Some("gespan-html".into()),
                code: "<img src=x onerror=alert(1)>".into(),
            }]
        );
        assert_eq!(markdown_to_plane_html(&plane_html_to_markdown(html)), html);
        // Nor may a span from Plane pose as kept Jira content.
        let span = r#"<span data-gespan-adf="{&quot;type&quot;:&quot;rule&quot;}">x</span>"#;
        let html = format!("<p>{span}</p>");
        assert_eq!(
            parse(&html).blocks,
            vec![para(vec![Inline::Preserved(preserved_html(span))])]
        );
    }

    #[test]
    fn many_raw_text_elements_parse_in_linear_time() {
        let html = "<style>a</style>".repeat(60_000);
        let started = std::time::Instant::now();
        let doc = parse(&html);
        assert_eq!(doc.blocks.len(), 60_000);
        assert!(
            started.elapsed().as_secs() < 5,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn unclosed_quoted_attributes_parse_in_linear_time() {
        // Every `<` sits inside a quoted value, and no `>` closes the tag.
        let html = format!("<p>x <a {}</p>", "c=\"<a \" ".repeat(40_000));
        let started = std::time::Instant::now();
        let markdown = plane_html_to_markdown(&html);
        assert!(
            markdown.starts_with("x \\<a c=\"\\<a \""),
            "{}",
            &markdown[..40]
        );
        assert!(
            started.elapsed().as_secs() < 5,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn many_adjacent_runs_merge_in_linear_time() {
        let html = format!("<p>{}</p>", "<strong>a b </strong>".repeat(60_000));
        let started = std::time::Instant::now();
        let doc = parse(&html);
        assert_eq!(doc.blocks.len(), 1);
        assert!(
            started.elapsed().as_secs() < 5,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn script_and_unknown_blocks_are_kept_whole() {
        let html = "<script>if (a < b) alert(1)</script><div class=\"x\"><p>in div</p></div>";
        assert_eq!(
            parse(html).blocks,
            vec![
                Block::Preserved(preserved_html("<script>if (a < b) alert(1)</script>")),
                Block::Preserved(preserved_html("<div class=\"x\"><p>in div</p></div>")),
            ]
        );
    }
}
