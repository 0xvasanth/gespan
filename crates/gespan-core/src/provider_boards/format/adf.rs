//! Jira's Atlassian Document Format (ADF) to and from the document model.
//!
//! Only nodes, attributes and marks the model fully understands are
//! converted. Anything else (tables, panels, media, mentions, underline,
//! alignment, …) is kept whole as [`Preserved`] and written back as it came.

use serde_json::{json, Map, Value};

use super::doc::{normalize, Block, Doc, Inline, Link, Marks, Preserved, Source};

pub(crate) fn parse(adf: &Value) -> Doc {
    let blocks = match adf {
        Value::Null => Vec::new(),
        // Jira's v2 API returns descriptions as plain strings.
        Value::String(text) => vec![Block::Paragraph(vec![Inline::text(text, Marks::plain())])],
        Value::Object(node) if node.get("type").and_then(Value::as_str) == Some("doc") => {
            match node.get("content") {
                None => Vec::new(),
                Some(Value::Array(content)) => content.iter().map(block).collect(),
                Some(_) => vec![preserve_block(adf)],
            }
        }
        other => vec![block(other)],
    };
    normalize(Doc { blocks })
}

fn preserve(node: &Value) -> Preserved {
    Preserved {
        source: Source::Adf,
        raw: node.to_string(),
    }
}

fn preserve_block(node: &Value) -> Block {
    Block::Preserved(preserve(node))
}

/// A node's fields, when it has no keys beyond `allowed` and no attributes
/// beyond `attrs` (`localId` is always allowed: it's an id, not content).
fn fields<'a>(node: &'a Value, allowed: &[&str], attrs: &[&str]) -> Option<&'a Map<String, Value>> {
    let node = node.as_object()?;
    if node
        .keys()
        .any(|key| key != "type" && !allowed.contains(&key.as_str()))
    {
        return None;
    }
    match node.get("attrs") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            if map
                .keys()
                .any(|key| key != "localId" && !attrs.contains(&key.as_str()))
            {
                return None;
            }
        }
        Some(_) => return None,
    }
    Some(node)
}

fn attr<'a>(node: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    node.get("attrs")
        .and_then(|attrs| attrs.get(key))
        .filter(|v| !v.is_null())
}

/// `content` as an array; missing means empty.
fn content(node: &Map<String, Value>) -> Option<&[Value]> {
    match node.get("content") {
        None => Some(&[]),
        Some(Value::Array(content)) => Some(content),
        Some(_) => None,
    }
}

fn block(node: &Value) -> Block {
    try_block(node).unwrap_or_else(|| preserve_block(node))
}

fn try_block(node: &Value) -> Option<Block> {
    let kind = node.get("type")?.as_str()?;
    match kind {
        "paragraph" => {
            let node = fields(node, &["content", "attrs"], &[])?;
            Some(Block::Paragraph(inlines(content(node)?)))
        }
        "heading" => {
            let node = fields(node, &["content", "attrs"], &["level"])?;
            let level = attr(node, "level")?
                .as_u64()
                .filter(|l| (1..=6).contains(l))?;
            Some(Block::Heading {
                level: level as u8,
                content: inlines(content(node)?),
            })
        }
        "bulletList" => {
            let node = fields(node, &["content", "attrs"], &[])?;
            Some(Block::BulletList(items(content(node)?)?))
        }
        "orderedList" => {
            let node = fields(node, &["content", "attrs"], &["order"])?;
            let start = match attr(node, "order") {
                None => 1,
                Some(order) => order.as_u64()?,
            };
            Some(Block::OrderedList {
                start,
                items: items(content(node)?)?,
            })
        }
        "codeBlock" => {
            let node = fields(node, &["content", "attrs"], &["language"])?;
            let language = match attr(node, "language") {
                None => None,
                Some(language) => Some(language.as_str()?.to_string()),
            };
            let mut code = String::new();
            for text in content(node)? {
                let text = fields(text, &["text"], &[])?;
                (text.get("type")?.as_str()? == "text").then_some(())?;
                code.push_str(text.get("text")?.as_str()?);
            }
            Some(Block::Code { language, code })
        }
        "blockquote" => {
            let node = fields(node, &["content", "attrs"], &[])?;
            Some(Block::Quote(content(node)?.iter().map(block).collect()))
        }
        "rule" => {
            fields(node, &["attrs"], &[])?;
            Some(Block::Rule)
        }
        _ => None,
    }
}

/// List items; `None` when the list holds anything else.
fn items(content: &[Value]) -> Option<Vec<Vec<Block>>> {
    content
        .iter()
        .map(|item| {
            let item = fields(item, &["content", "attrs"], &[])?;
            (item.get("type")?.as_str()? == "listItem").then_some(())?;
            Some(self::content(item)?.iter().map(block).collect())
        })
        .collect()
}

fn inlines(content: &[Value]) -> Vec<Inline> {
    content
        .iter()
        .map(|node| try_inline(node).unwrap_or_else(|| Inline::Preserved(preserve(node))))
        .collect()
}

fn try_inline(node: &Value) -> Option<Inline> {
    match node.get("type")?.as_str()? {
        "text" => {
            let node = fields(node, &["text", "marks"], &[])?;
            let text = node.get("text")?.as_str()?;
            let mut marks = Marks::plain();
            for mark in node
                .get("marks")
                .map(|m| m.as_array())
                .unwrap_or(Some(&Vec::new()))?
            {
                match mark.get("type")?.as_str()? {
                    "strong" => marks.strong = true,
                    "em" => marks.em = true,
                    "strike" => marks.strike = true,
                    "code" => marks.code = true,
                    "link" => {
                        let mark = fields(mark, &["attrs"], &["href", "title"])?;
                        let title = match attr(mark, "title") {
                            None => None,
                            Some(title) => Some(title.as_str()?.to_string()),
                        };
                        marks.link = Some(Link {
                            href: attr(mark, "href")?.as_str()?.to_string(),
                            title: title.filter(|t| !t.is_empty()),
                        });
                        continue;
                    }
                    _ => return None,
                }
                fields(mark, &[], &[])?;
            }
            // ADF allows code only with a link; anything else would be
            // changed by normalizing.
            if marks.code && (marks.strong || marks.em || marks.strike) {
                return None;
            }
            Some(Inline::text(text, marks))
        }
        "hardBreak" => {
            fields(node, &["attrs"], &["text"])?;
            Some(Inline::HardBreak)
        }
        _ => None,
    }
}

pub(crate) fn render(doc: &Doc) -> Value {
    json!({
        "type": "doc",
        "version": 1,
        "content": render_blocks(&doc.blocks, false),
    })
}

/// `nested` is a list item or quote, where ADF allows only paragraphs, lists,
/// code blocks and media.
fn render_blocks(blocks: &[Block], nested: bool) -> Vec<Value> {
    blocks
        .iter()
        .flat_map(|block| render_block(block, nested))
        .collect()
}

fn code_block(language: Option<&str>, code: &str) -> Value {
    let mut node = json!({"type": "codeBlock"});
    if let Some(language) = language {
        node["attrs"] = json!({"language": language});
    }
    if !code.is_empty() {
        node["content"] = json!([{"type": "text", "text": code}]);
    }
    node
}

fn paragraph(content: Vec<Value>) -> Value {
    json!({"type": "paragraph", "content": content})
}

fn render_block(block: &Block, nested: bool) -> Vec<Value> {
    let node = match block {
        Block::Paragraph(content) => paragraph(render_inlines(content)),
        Block::Heading { level, content } if !nested => json!({
            "type": "heading",
            "attrs": {"level": level},
            "content": render_inlines(content),
        }),
        Block::Heading { content, .. } => {
            let bold: Vec<Inline> = content
                .iter()
                .map(|inline| match inline {
                    Inline::Text { text, marks } if !marks.code => Inline::text(
                        text,
                        Marks {
                            strong: true,
                            ..marks.clone()
                        },
                    ),
                    other => other.clone(),
                })
                .collect();
            paragraph(render_inlines(&bold))
        }
        Block::BulletList(items) => json!({"type": "bulletList", "content": render_items(items)}),
        Block::OrderedList { start, items } => json!({
            "type": "orderedList",
            "attrs": {"order": start},
            "content": render_items(items),
        }),
        Block::Code { language, code } => code_block(language.as_deref(), code),
        Block::Quote(blocks) if !nested => {
            json!({"type": "blockquote", "content": render_blocks(blocks, true)})
        }
        Block::Quote(blocks) => return render_blocks(blocks, true),
        Block::Rule if !nested => json!({"type": "rule"}),
        Block::Rule => paragraph(vec![json!({"type": "text", "text": "---"})]),
        Block::Html(raw) => code_block(Some(Source::Html.fence()), raw),
        Block::Preserved(preserved) => match preserved.source {
            Source::Adf => serde_json::from_str(&preserved.raw)
                .unwrap_or_else(|_| code_block(Some(Source::Adf.fence()), &preserved.raw)),
            Source::Html => code_block(Some(Source::Html.fence()), &preserved.raw),
        },
    };
    vec![node]
}

fn render_items(items: &[Vec<Block>]) -> Vec<Value> {
    items
        .iter()
        .map(|item| {
            let mut content = render_blocks(item, true);
            // A list item must start with a paragraph or a code block.
            let first = content
                .first()
                .and_then(|node| node.get("type"))
                .and_then(Value::as_str);
            if !matches!(first, Some("paragraph" | "codeBlock" | "mediaSingle")) {
                content.insert(0, paragraph(Vec::new()));
            }
            json!({"type": "listItem", "content": content})
        })
        .collect()
}

fn text_node(text: &str, marks: &Marks) -> Value {
    let mut list = Vec::new();
    for (on, kind) in [
        (marks.strong, "strong"),
        (marks.em, "em"),
        (marks.strike, "strike"),
        (marks.code, "code"),
    ] {
        if on {
            list.push(json!({"type": kind}));
        }
    }
    if let Some(link) = &marks.link {
        let mut attrs = json!({"href": link.href});
        if let Some(title) = &link.title {
            attrs["title"] = json!(title);
        }
        list.push(json!({"type": "link", "attrs": attrs}));
    }
    let mut node = json!({"type": "text", "text": text});
    if !list.is_empty() {
        node["marks"] = Value::Array(list);
    }
    node
}

fn render_inlines(content: &[Inline]) -> Vec<Value> {
    content
        .iter()
        .map(|inline| match inline {
            Inline::Text { text, marks } => text_node(text, marks),
            Inline::HardBreak => json!({"type": "hardBreak"}),
            Inline::Image { src, alt, .. } => {
                let text = if alt.is_empty() { src } else { alt };
                let marks = Marks {
                    link: Some(Link {
                        href: src.clone(),
                        title: None,
                    }),
                    ..Marks::plain()
                };
                text_node(text, &marks)
            }
            Inline::Html(raw) => text_node(raw, &Marks::plain()),
            Inline::Preserved(preserved) => match preserved.source {
                Source::Adf => serde_json::from_str(&preserved.raw)
                    .unwrap_or_else(|_| text_node(&preserved.raw, &Marks::plain())),
                Source::Html => text_node(&preserved.raw, &Marks::plain()),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::super::doc::{normalize, Block, Doc, Inline, Link, Marks};
    use super::super::markdown;
    use super::super::strategies::{doc, Profile};
    use super::super::{adf_to_markdown, markdown_to_adf};
    use super::*;

    fn plain(text: &str) -> Inline {
        Inline::text(text, Marks::plain())
    }

    fn adf(content: Value) -> Value {
        json!({"type": "doc", "version": 1, "content": content})
    }

    fn paragraph(text: &str) -> Value {
        json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
    }

    /// ADF → Markdown → ADF.
    fn through_markdown(value: &Value) -> Value {
        markdown_to_adf(&adf_to_markdown(value))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn adf_round_trips_any_document(doc in doc(Profile::Adf)) {
            let value = render(&doc);
            prop_assert_eq!(parse(&value), doc, "adf: {}", value);
        }

        #[test]
        fn adf_through_markdown_round_trips(doc in doc(Profile::Adf)) {
            let markdown = markdown::render(&parse(&render(&doc)));
            prop_assert_eq!(parse(&render(&markdown::parse(&markdown))), doc, "markdown:\n{}", markdown);
        }
    }

    #[test]
    fn renders_the_basic_constructs() {
        let value = markdown_to_adf(
            "## Steps\n\n1. Open **Settings**\n2. Click [Save](https://x.test \"T\")\n\n```rust\nfn main() {}\n```\n\n> _note_ `x`\n\n---",
        );
        assert_eq!(
            value,
            adf(json!([
                {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Steps"}]},
                {"type": "orderedList", "attrs": {"order": 1}, "content": [
                    {"type": "listItem", "content": [{"type": "paragraph", "content": [
                        {"type": "text", "text": "Open "},
                        {"type": "text", "text": "Settings", "marks": [{"type": "strong"}]},
                    ]}]},
                    {"type": "listItem", "content": [{"type": "paragraph", "content": [
                        {"type": "text", "text": "Click "},
                        {"type": "text", "text": "Save", "marks": [{"type": "link", "attrs": {"href": "https://x.test", "title": "T"}}]},
                    ]}]},
                ]},
                {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [{"type": "text", "text": "fn main() {}"}]},
                {"type": "blockquote", "content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "note", "marks": [{"type": "em"}]},
                    {"type": "text", "text": " "},
                    {"type": "text", "text": "x", "marks": [{"type": "code"}]},
                ]}]},
                {"type": "rule"},
            ]))
        );
        assert_eq!(through_markdown(&value), value);
    }

    #[test]
    fn unknown_block_node_is_preserved_verbatim() {
        let panel = json!({"type": "panel", "attrs": {"panelType": "warning"}, "content": [paragraph("Careful")]});
        let value = adf(json!([paragraph("before"), panel, paragraph("after")]));
        let markdown = adf_to_markdown(&value);
        assert!(
            markdown.contains("```gespan-adf\n{\"attrs\":{\"panelType\":\"warning\"}"),
            "{markdown}"
        );
        assert_eq!(through_markdown(&value), value);
    }

    #[test]
    fn unknown_inline_node_is_preserved_with_fallback() {
        let mention = json!({"type": "mention", "attrs": {"id": "5b10", "text": "@Alice", "accessLevel": ""}});
        let value = adf(json!([{"type": "paragraph", "content": [
            {"type": "text", "text": "cc "},
            mention,
            {"type": "text", "text": " please"},
        ]}]));
        let markdown = adf_to_markdown(&value);
        assert!(
            markdown.starts_with("cc <span data-gespan-adf="),
            "{markdown}"
        );
        assert!(markdown.ends_with(">@Alice</span> please"), "{markdown}");
        assert_eq!(through_markdown(&value), value);
    }

    #[test]
    fn text_with_unknown_mark_is_preserved_whole() {
        let underlined = json!({"type": "text", "text": "important", "marks": [{"type": "strong"}, {"type": "underline"}]});
        let value = adf(json!([{"type": "paragraph", "content": [underlined]}]));
        assert!(adf_to_markdown(&value).contains(">important</span>"));
        assert_eq!(through_markdown(&value), value);
    }

    #[test]
    fn block_marks_preserve_the_block() {
        let centred = json!({"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": "center"}}], "content": [{"type": "text", "text": "mid"}]});
        let value = adf(json!([centred]));
        assert!(adf_to_markdown(&value).starts_with("```gespan-adf"));
        assert_eq!(through_markdown(&value), value);
    }

    #[test]
    fn local_ids_are_ignored() {
        let value = adf(
            json!([{"type": "paragraph", "attrs": {"localId": "abc"}, "content": [{"type": "text", "text": "x"}]}]),
        );
        assert_eq!(
            parse(&value).blocks,
            vec![Block::Paragraph(vec![plain("x")])]
        );
    }

    #[test]
    fn list_item_starting_with_a_list_gets_an_empty_paragraph() {
        let doc = markdown::parse("- - inner");
        assert_eq!(
            render(&doc),
            adf(
                json!([{"type": "bulletList", "content": [{"type": "listItem", "content": [
                    {"type": "paragraph", "content": []},
                    {"type": "bulletList", "content": [{"type": "listItem", "content": [paragraph("inner")]}]},
                ]}]}])
            )
        );
        assert_eq!(parse(&render(&doc)), doc);
    }

    #[test]
    fn restricted_contexts_degrade_without_losing_text() {
        let value = markdown_to_adf("- # Title\n\n  ---\n\n  > quoted\n\n  <div>x</div>");
        assert_eq!(
            value,
            adf(
                json!([{"type": "bulletList", "content": [{"type": "listItem", "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "Title", "marks": [{"type": "strong"}]}]},
                    paragraph("---"),
                    paragraph("quoted"),
                    {"type": "codeBlock", "attrs": {"language": "gespan-html"}, "content": [{"type": "text", "text": "<div>x</div>"}]},
                ]}]}])
            )
        );
    }

    #[test]
    fn image_becomes_link_text() {
        let value = markdown_to_adf("![diagram](https://x.test/d.png) ![](https://x.test/e.png)");
        let link = |href: &str| json!([{"type": "link", "attrs": {"href": href}}]);
        assert_eq!(
            value,
            adf(json!([{"type": "paragraph", "content": [
                {"type": "text", "text": "diagram", "marks": link("https://x.test/d.png")},
                {"type": "text", "text": " "},
                {"type": "text", "text": "https://x.test/e.png", "marks": link("https://x.test/e.png")},
            ]}]))
        );
    }

    #[test]
    fn null_and_empty_docs_give_empty_markdown() {
        assert_eq!(adf_to_markdown(&Value::Null), "");
        assert_eq!(adf_to_markdown(&adf(json!([]))), "");
        assert_eq!(adf_to_markdown(&json!({"type": "doc", "version": 1})), "");
        assert_eq!(markdown_to_adf(""), adf(json!([])));
    }

    #[test]
    fn plain_string_descriptions_become_a_paragraph() {
        assert_eq!(
            adf_to_markdown(&json!("legacy *text*")),
            "legacy \\*text\\*"
        );
    }

    #[test]
    fn malformed_nodes_are_preserved_not_dropped() {
        let bad = [
            json!({"type": "heading", "attrs": {"level": 9}, "content": [{"type": "text", "text": "big"}]}),
            json!({"type": "bulletList", "content": [paragraph("not an item")]}),
            json!({"type": "codeBlock", "content": [{"type": "text", "text": "x", "marks": [{"type": "strong"}]}]}),
            json!({"type": "paragraph", "content": "oops"}),
            json!("a bare string"),
            json!({"no": "type"}),
        ];
        for node in bad {
            let value = adf(json!([node.clone()]));
            let doc = parse(&value);
            assert_eq!(doc.blocks.len(), 1, "{node}");
            assert!(
                matches!(doc.blocks[0], Block::Preserved(_)),
                "{node} gave {doc:?}"
            );
            assert_eq!(through_markdown(&value), value, "{node}");
        }
    }

    #[test]
    fn hard_breaks_and_code_with_links_survive() {
        let link = Some(Link {
            href: "https://x.test".into(),
            title: None,
        });
        let doc = normalize(Doc {
            blocks: vec![Block::Paragraph(vec![
                plain("a"),
                Inline::HardBreak,
                Inline::text(
                    "b",
                    Marks {
                        code: true,
                        link,
                        ..Marks::plain()
                    },
                ),
            ])],
        });
        assert_eq!(
            render(&doc),
            adf(json!([{"type": "paragraph", "content": [
                {"type": "text", "text": "a"},
                {"type": "hardBreak"},
                {"type": "text", "text": "b", "marks": [{"type": "code"}, {"type": "link", "attrs": {"href": "https://x.test"}}]},
            ]}]))
        );
        assert_eq!(parse(&render(&doc)), doc);
    }
}
