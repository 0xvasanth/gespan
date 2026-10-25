//! The document model every description format converts through.
//!
//! [`normalize`] defines the canonical form. Each parser returns normalized
//! documents, so `parse(render(doc)) == doc` is the round-trip contract for
//! every format.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Doc {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    Paragraph(Vec<Inline>),
    /// `level` is 1 to 6.
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    BulletList(Vec<Vec<Block>>),
    OrderedList {
        start: u64,
        items: Vec<Vec<Block>>,
    },
    Code {
        language: Option<String>,
        code: String,
    },
    Quote(Vec<Block>),
    Rule,
    /// A raw HTML block written in Markdown.
    Html(String),
    Preserved(Preserved),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Inline {
    Text {
        text: String,
        marks: Marks,
    },
    HardBreak,
    Image {
        src: String,
        alt: String,
        link: Option<Link>,
    },
    /// Raw inline HTML written in Markdown.
    Html(String),
    Preserved(Preserved),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Marks {
    pub strong: bool,
    pub em: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    pub href: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Adf,
    Html,
}

/// Content the model can't represent, kept as it arrived: the compact JSON of
/// an ADF node, or the exact source of an HTML element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preserved {
    pub source: Source,
    pub raw: String,
}

/// The code-block language that carries a preserved block through Markdown.
pub(crate) const ADF_FENCE: &str = "gespan-adf";
pub(crate) const HTML_FENCE: &str = "gespan-html";

impl Source {
    pub(crate) fn fence(self) -> &'static str {
        match self {
            Source::Adf => ADF_FENCE,
            Source::Html => HTML_FENCE,
        }
    }

    pub(crate) fn from_fence(language: &str) -> Option<Source> {
        match language {
            ADF_FENCE => Some(Source::Adf),
            HTML_FENCE => Some(Source::Html),
            _ => None,
        }
    }
}

impl Marks {
    pub(crate) fn plain() -> Marks {
        Marks::default()
    }

    fn formatted(&self) -> bool {
        self.strong || self.em || self.strike
    }
}

impl Inline {
    pub(crate) fn text(text: &str, marks: Marks) -> Inline {
        Inline::Text {
            text: text.to_string(),
            marks,
        }
    }
}

impl Preserved {
    /// Readable text for the preserved content, shown next to it in
    /// Markdown. It's derived from `raw` every time, so editing it changes
    /// nothing.
    pub(crate) fn fallback_text(&self) -> String {
        let text = match self.source {
            Source::Adf => serde_json::from_str::<serde_json::Value>(&self.raw)
                .map(|node| adf_text(&node))
                .unwrap_or_default(),
            Source::Html => html_text(&self.raw),
        };
        collapse_whitespace(&text).trim().to_string()
    }
}

fn adf_text(node: &serde_json::Value) -> String {
    if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
        return text.to_string();
    }
    let attrs = node.get("attrs");
    for key in ["text", "shortName", "url", "title"] {
        if let Some(value) = attrs.and_then(|a| a.get(key)).and_then(|v| v.as_str()) {
            return value.to_string();
        }
    }
    let mut out = String::new();
    for child in node
        .get("content")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
    {
        let inline = child.get("type").and_then(|t| t.as_str()) == Some("text");
        if !inline && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&adf_text(child));
    }
    out
}

fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_html_entities(&out)
}

/// Decodes named (the common ones) and numeric character references.
/// Anything else is left as written.
pub(crate) fn decode_html_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest[1..]
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| {
                let name = &rest[1..1 + end];
                let c = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    _ => {
                        let code = if let Some(hex) =
                            name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
                        {
                            u32::from_str_radix(hex, 16).ok()
                        } else if let Some(dec) = name.strip_prefix('#') {
                            dec.parse().ok()
                        } else {
                            None
                        };
                        code.map(|code| {
                            char::from_u32(code)
                                .filter(|c| *c != '\0')
                                .unwrap_or('\u{FFFD}')
                        })
                    }
                };
                c.map(|c| (c, end + 2))
            });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Markdown can't number a list past this.
pub(crate) const MAX_LIST_START: u64 = 999_999_999;

/// Puts a document in canonical form; see the module docs.
pub(crate) fn normalize(doc: Doc) -> Doc {
    Doc {
        blocks: normalize_blocks(doc.blocks),
    }
}

fn normalize_blocks(blocks: Vec<Block>) -> Vec<Block> {
    blocks.into_iter().filter_map(normalize_block).collect()
}

fn normalize_items(items: Vec<Vec<Block>>) -> Vec<Vec<Block>> {
    items
        .into_iter()
        .map(normalize_blocks)
        .filter(|item| !item.is_empty())
        .collect()
}

fn normalize_block(block: Block) -> Option<Block> {
    match block {
        Block::Paragraph(content) => {
            let content = normalize_inlines(content, false);
            (!content.is_empty()).then_some(Block::Paragraph(content))
        }
        Block::Heading { level, content } => {
            let content = normalize_inlines(content, true);
            (!content.is_empty()).then_some(Block::Heading {
                level: level.clamp(1, 6),
                content,
            })
        }
        Block::BulletList(items) => {
            let items = normalize_items(items);
            (!items.is_empty()).then_some(Block::BulletList(items))
        }
        Block::OrderedList { start, items } => {
            let items = normalize_items(items);
            (!items.is_empty()).then_some(Block::OrderedList {
                start: start.min(MAX_LIST_START),
                items,
            })
        }
        Block::Code { language, code } => {
            let code = code
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .trim_end_matches('\n')
                .to_string();
            let language = language.and_then(|l| l.split_whitespace().next().map(str::to_string));
            Some(Block::Code { language, code })
        }
        Block::Quote(blocks) => {
            let blocks = normalize_blocks(blocks);
            (!blocks.is_empty()).then_some(Block::Quote(blocks))
        }
        Block::Rule => Some(Block::Rule),
        Block::Html(raw) => {
            let raw = raw.trim_end_matches(['\n', '\r']).to_string();
            (!raw.trim().is_empty()).then_some(Block::Html(raw))
        }
        Block::Preserved(preserved) => Some(Block::Preserved(preserved)),
    }
}

/// Every ASCII whitespace character becomes one space; NUL becomes U+FFFD.
pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let c = match c {
            '\0' => '\u{FFFD}',
            c if c.is_ascii_whitespace() => ' ',
            c => c,
        };
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out
}

fn normalize_inlines(content: Vec<Inline>, heading: bool) -> Vec<Inline> {
    let mut inlines: Vec<Inline> = content
        .into_iter()
        .map(|inline| match inline {
            Inline::Text { text, mut marks } => {
                if marks.code {
                    marks.strong = false;
                    marks.em = false;
                    marks.strike = false;
                }
                if let Some(link) = &mut marks.link {
                    link.title = link.title.as_deref().map(collapse_whitespace);
                }
                Inline::Text {
                    text: collapse_whitespace(&text),
                    marks,
                }
            }
            Inline::HardBreak if heading => Inline::text(" ", Marks::plain()),
            Inline::Image { src, alt, link } => {
                let alt = collapse_whitespace(&alt).trim().to_string();
                if src.is_empty() && alt.is_empty() {
                    // Nothing to show or link to.
                    Inline::text("", Marks::plain())
                } else {
                    Inline::Image { src, alt, link }
                }
            }
            other => other,
        })
        .collect();
    loop {
        let next = trim_edges(move_emphasis_spaces(merge_runs(inlines.clone())));
        if next == inlines {
            return next;
        }
        inlines = next;
    }
}

fn merge_runs(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match inline {
            Inline::Text { text, .. } if text.is_empty() => {}
            Inline::Text { text, marks } => match out.last_mut() {
                Some(Inline::Text {
                    text: previous,
                    marks: previous_marks,
                }) if *previous_marks == marks => {
                    // Both sides are already collapsed; only the join can
                    // hold two spaces.
                    let text = match previous.ends_with(' ') {
                        true => text.strip_prefix(' ').unwrap_or(&text),
                        false => &text,
                    };
                    previous.push_str(text);
                }
                _ => out.push(Inline::Text { text, marks }),
            },
            other => out.push(other),
        }
    }
    out
}

fn marks_of(inline: Option<&Inline>) -> Marks {
    match inline {
        Some(Inline::Text { marks, .. }) => marks.clone(),
        _ => Marks::plain(),
    }
}

/// `marks` without the formatting marks `other` lacks.
fn without_edges(marks: &Marks, other: &Marks) -> Marks {
    Marks {
        strong: marks.strong && other.strong,
        em: marks.em && other.em,
        strike: marks.strike && other.strike,
        ..marks.clone()
    }
}

/// A strong, em or strike span never starts or ends with a space.
fn move_emphasis_spaces(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for (i, inline) in inlines.iter().enumerate() {
        let Inline::Text { text, marks } = inline else {
            out.push(inline.clone());
            continue;
        };
        if !marks.formatted() {
            out.push(inline.clone());
            continue;
        }
        let before = marks_of(i.checked_sub(1).and_then(|p| inlines.get(p)));
        let after = marks_of(inlines.get(i + 1));
        let leading = without_edges(marks, &before);
        let trailing = without_edges(marks, &after);
        let mut body = text.as_str();
        let mut tail = None;
        if leading != *marks && body.starts_with(' ') {
            out.push(Inline::text(" ", leading));
            body = &body[1..];
        }
        if trailing != *marks && body.ends_with(' ') {
            body = &body[..body.len() - 1];
            tail = Some(Inline::text(" ", trailing));
        }
        out.push(Inline::text(body, marks.clone()));
        out.extend(tail);
    }
    out
}

fn trimmable(marks: &Marks) -> bool {
    !marks.code && marks.link.is_none()
}

/// Trims plain text at the block's edges and around hard breaks, and drops
/// hard breaks at the edges.
fn trim_edges(mut inlines: Vec<Inline>) -> Vec<Inline> {
    while matches!(inlines.first(), Some(Inline::HardBreak)) {
        inlines.remove(0);
    }
    while matches!(inlines.last(), Some(Inline::HardBreak)) {
        inlines.pop();
    }
    let len = inlines.len();
    for i in 0..len {
        let at_start = i == 0 || matches!(inlines[i - 1], Inline::HardBreak);
        let at_end = i + 1 == len || matches!(inlines[i + 1], Inline::HardBreak);
        if let Inline::Text { text, marks } = &mut inlines[i] {
            if trimmable(marks) {
                if at_start {
                    *text = text.trim_start_matches(' ').to_string();
                }
                if at_end {
                    *text = text.trim_end_matches(' ').to_string();
                }
            }
        }
    }
    inlines
}

#[cfg(test)]
mod tests {
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

    fn one(block: Block) -> Doc {
        normalize(Doc {
            blocks: vec![block],
        })
    }

    #[test]
    fn whitespace_collapses_in_text_but_not_code_blocks() {
        let doc = normalize(Doc {
            blocks: vec![
                para(vec![plain("a \t\n b\r\nc\0")]),
                Block::Code {
                    language: None,
                    code: "x  \ty\r\n\n".into(),
                },
            ],
        });
        assert_eq!(
            doc.blocks,
            vec![
                para(vec![plain("a b c\u{FFFD}")]),
                Block::Code {
                    language: None,
                    code: "x  \ty".into(),
                },
            ]
        );
    }

    #[test]
    fn code_drops_other_formatting_marks() {
        let marks = Marks {
            strong: true,
            em: true,
            strike: true,
            code: true,
            link: None,
        };
        let doc = one(para(vec![Inline::text("x", marks)]));
        let code = Marks {
            code: true,
            ..Marks::plain()
        };
        assert_eq!(doc.blocks, vec![para(vec![Inline::text("x", code)])]);
    }

    #[test]
    fn adjacent_runs_with_equal_marks_merge() {
        let doc = one(para(vec![
            plain("a"),
            plain(""),
            plain("b"),
            strong("c"),
            strong("d"),
        ]));
        assert_eq!(doc.blocks, vec![para(vec![plain("ab"), strong("cd")])]);
    }

    #[test]
    fn merged_code_runs_still_collapse_spaces() {
        let code = Marks {
            code: true,
            ..Marks::plain()
        };
        let doc = one(para(vec![
            plain("x"),
            Inline::text(" ", code.clone()),
            Inline::text(" ", code.clone()),
        ]));
        assert_eq!(
            doc.blocks,
            vec![para(vec![plain("x"), Inline::text(" ", code)])]
        );
    }

    #[test]
    fn emphasis_edges_move_spaces_outside() {
        let doc = one(para(vec![plain("x"), strong(" a "), plain("y")]));
        assert_eq!(
            doc.blocks,
            vec![para(vec![plain("x "), strong("a"), plain(" y")])]
        );
        // Strong continues across the runs, so only the em span's edge moves.
        let both = Marks {
            strong: true,
            em: true,
            ..Marks::plain()
        };
        let doc = one(para(vec![strong("a"), Inline::text(" b", both.clone())]));
        assert_eq!(
            doc.blocks,
            vec![para(vec![strong("a "), Inline::text("b", both)])]
        );
    }

    #[test]
    fn trims_plain_text_at_block_edges_and_around_hard_breaks() {
        let link = Marks {
            link: Some(Link {
                href: "https://x.test".into(),
                title: None,
            }),
            ..Marks::plain()
        };
        let doc = one(para(vec![
            Inline::HardBreak,
            plain(" a "),
            Inline::HardBreak,
            plain(" b"),
            Inline::text(" c ", link.clone()),
            Inline::HardBreak,
        ]));
        assert_eq!(
            doc.blocks,
            vec![para(vec![
                plain("a"),
                Inline::HardBreak,
                plain("b"),
                Inline::text(" c ", link),
            ])]
        );
    }

    #[test]
    fn drops_empty_blocks_recursively() {
        let doc = normalize(Doc {
            blocks: vec![
                para(vec![plain("  ")]),
                Block::Heading {
                    level: 2,
                    content: vec![],
                },
                Block::BulletList(vec![vec![para(vec![])], vec![]]),
                Block::Quote(vec![Block::OrderedList {
                    start: 3,
                    items: vec![vec![]],
                }]),
                para(vec![plain("kept")]),
            ],
        });
        assert_eq!(doc.blocks, vec![para(vec![plain("kept")])]);
    }

    #[test]
    fn heading_hard_break_becomes_space() {
        let doc = one(Block::Heading {
            level: 1,
            content: vec![plain("a"), Inline::HardBreak, plain("b")],
        });
        assert_eq!(
            doc.blocks,
            vec![Block::Heading {
                level: 1,
                content: vec![plain("a b")],
            }]
        );
    }

    #[test]
    fn code_languages_and_list_starts_are_canonical() {
        let doc = normalize(Doc {
            blocks: vec![
                Block::Code {
                    language: Some("gespan-adf extra".into()),
                    code: "{\"type\":\"panel\"}\r\n".into(),
                },
                Block::Code {
                    language: Some("  ".into()),
                    code: "x".into(),
                },
                Block::OrderedList {
                    start: u64::MAX,
                    items: vec![vec![Block::Html("<div>x</div>\n\n".into())]],
                },
            ],
        });
        assert_eq!(
            doc.blocks,
            vec![
                // Only Gespan's own Markdown marks content as kept.
                Block::Code {
                    language: Some("gespan-adf".into()),
                    code: "{\"type\":\"panel\"}".into(),
                },
                Block::Code {
                    language: None,
                    code: "x".into(),
                },
                Block::OrderedList {
                    start: 999_999_999,
                    items: vec![vec![Block::Html("<div>x</div>".into())]],
                },
            ]
        );
    }

    #[test]
    fn fallback_text_reads_preserved_content() {
        let fallback = |source, raw: &str| {
            Preserved {
                source,
                raw: raw.into(),
            }
            .fallback_text()
        };
        assert_eq!(
            fallback(
                Source::Adf,
                r#"{"type":"mention","attrs":{"id":"1","text":"@Alice"}}"#
            ),
            "@Alice"
        );
        assert_eq!(
            fallback(
                Source::Adf,
                r#"{"type":"panel","content":[{"type":"paragraph","content":[{"type":"text","text":"Heads "},{"type":"text","text":"up"}]},{"type":"paragraph","content":[{"type":"text","text":"twice"}]}]}"#
            ),
            "Heads up twice"
        );
        assert_eq!(fallback(Source::Adf, "not json"), "");
        assert_eq!(
            fallback(
                Source::Html,
                "<table><tr><td>a &amp; b</td><td>&#x1F600;&bogus;</td></tr></table>"
            ),
            "a & b\u{1F600}&bogus;"
        );
    }

    #[test]
    fn normalize_is_idempotent() {
        let doc = normalize(Doc {
            blocks: vec![para(vec![
                strong(" a "),
                Inline::text(
                    " b ",
                    Marks {
                        em: true,
                        strong: true,
                        ..Marks::plain()
                    },
                ),
                plain("  c"),
            ])],
        });
        assert_eq!(normalize(doc.clone()), doc);
    }
}
