//! Random documents for the round-trip properties, one profile per format.

use proptest::prelude::*;

use super::doc::{normalize, Block, Doc, Inline, Link, Marks, Preserved, Source};

/// Which format the documents must be representable in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    /// Everything the model holds.
    Markdown,
    /// No images or raw HTML, and ADF's nesting limits.
    Adf,
    /// No raw Markdown HTML and no ADF-preserved content.
    Html,
}

/// ADF nodes in the compact, key-sorted form `serde_json` writes.
pub(crate) const ADF_BLOCKS: &[&str] = &[
    r#"{"attrs":{"panelType":"info"},"content":[{"content":[{"text":"Heads up","type":"text"}],"type":"paragraph"}],"type":"panel"}"#,
    r#"{"attrs":{"isNumberColumnEnabled":false,"layout":"default"},"content":[{"content":[{"content":[{"content":[{"text":"cell","type":"text"}],"type":"paragraph"}],"type":"tableCell"}],"type":"tableRow"}],"type":"table"}"#,
    r#"{"content":[{"attrs":{"collection":"","id":"f3a1","type":"file"},"type":"media"}],"type":"mediaSingle"}"#,
];
pub(crate) const ADF_INLINES: &[&str] = &[
    r#"{"attrs":{"id":"5b10a2844c20165700ede21g","text":"@Alice"},"type":"mention"}"#,
    r#"{"attrs":{"shortName":":tada:","text":"🎉"},"type":"emoji"}"#,
    r#"{"marks":[{"type":"underline"}],"text":"under","type":"text"}"#,
];
pub(crate) const HTML_BLOCKS: &[&str] = &[
    "<table><tbody><tr><td><p>cell</p></td></tr></tbody></table>",
    r#"<image-component src="https://x.test/a.png" id="i1"></image-component>"#,
    r#"<ul data-type="taskList"><li data-checked="true" data-type="taskItem"><p>done</p></li></ul>"#,
];
pub(crate) const HTML_INLINES: &[&str] = &[
    r#"<mention-component entity_identifier="u1" entity_name="user_mention"></mention-component>"#,
    "<u>under</u>",
    r#"<span style="color: red">red &amp; bold</span>"#,
];
const MARKDOWN_HTML_INLINES: &[&str] = &["<kbd>", "</kbd>", "<sup>"];
const MARKDOWN_HTML_BLOCKS: &[&str] = &[
    "<div>\nhi\n</div>",
    "<details><summary>More</summary></details>",
];

fn text() -> impl Strategy<Value = String> {
    "[a-z *_`\\[\\]<>#+=.!~\\\\&|()1é\"-]{1,8}"
}

fn link() -> impl Strategy<Value = Option<Link>> {
    prop_oneof![
        3 => Just(None),
        1 => (
            prop::sample::select(vec![
                "https://x.test/a",
                "https://x.test/a b",
                "https://x.test/(p)?q=1&amp;r=2",
                "mailto:a@x.test",
                "/relative\\path",
            ]),
            prop::option::of(prop::sample::select(vec!["Title", "say \"hi\""])),
        )
            .prop_map(|(href, title)| Some(Link {
                href: href.into(),
                title: title.map(str::to_string),
            })),
    ]
}

fn preserved(
    source: Source,
    snippets: &'static [&'static str],
) -> impl Strategy<Value = Preserved> {
    prop::sample::select(snippets).prop_map(move |raw| Preserved {
        source,
        raw: raw.to_string(),
    })
}

fn inline(profile: Profile) -> BoxedStrategy<Inline> {
    let text = (text(), any::<[bool; 4]>(), link()).prop_map(
        |(text, [strong, em, strike, code], link)| Inline::Text {
            text,
            marks: Marks {
                strong,
                em,
                strike,
                code,
                link,
            },
        },
    );
    let image = (
        prop::sample::select(vec!["https://x.test/i.png", "a b.png"]),
        "[a-z *\\]!]{0,6}",
        link(),
    )
        .prop_map(|(src, alt, link)| Inline::Image {
            src: src.into(),
            alt,
            link,
        });
    let html = prop::sample::select(MARKDOWN_HTML_INLINES).prop_map(|raw| Inline::Html(raw.into()));
    let adf = preserved(Source::Adf, ADF_INLINES).prop_map(Inline::Preserved);
    let html_preserved = preserved(Source::Html, HTML_INLINES).prop_map(Inline::Preserved);
    match profile {
        Profile::Markdown => prop_oneof![
            8 => text,
            1 => Just(Inline::HardBreak),
            1 => image,
            1 => html,
            1 => adf,
            1 => html_preserved,
        ]
        .boxed(),
        Profile::Adf => prop_oneof![8 => text, 1 => Just(Inline::HardBreak), 1 => adf].boxed(),
        Profile::Html => prop_oneof![
            8 => text,
            1 => Just(Inline::HardBreak),
            1 => image,
            1 => html_preserved,
        ]
        .boxed(),
    }
}

fn inlines(profile: Profile) -> impl Strategy<Value = Vec<Inline>> {
    prop::collection::vec(inline(profile), 1..7)
}

/// A lone HTML tag at the start of a line is an HTML block in CommonMark, so
/// raw inline HTML never opens a line. Runs after normalizing, which trims.
fn guard_line_start_html(blocks: &mut [Block]) {
    for block in blocks {
        match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => {
                let mut i = 0;
                while i < content.len() {
                    let line_start = i == 0 || matches!(content[i - 1], Inline::HardBreak);
                    if line_start && matches!(content[i], Inline::Html(_)) {
                        content.insert(i, Inline::text("x", Marks::plain()));
                    }
                    i += 1;
                }
            }
            Block::BulletList(items) | Block::OrderedList { items, .. } => items
                .iter_mut()
                .for_each(|item| guard_line_start_html(item)),
            Block::Quote(blocks) => guard_line_start_html(blocks),
            _ => {}
        }
    }
}

fn leaf(profile: Profile) -> BoxedStrategy<Block> {
    let paragraph = inlines(profile).prop_map(Block::Paragraph);
    let heading =
        (1u8..=6, inlines(profile)).prop_map(|(level, content)| Block::Heading { level, content });
    let code = (
        prop::option::of(prop::sample::select(vec![
            "rust",
            "c++",
            "js extra",
            "",
            "gespan-html",
            "gespan\\-adf",
            "gespan&#45;html",
        ])),
        "[a-z `~\n]{0,12}",
    )
        .prop_map(|(language, code)| Block::Code {
            language: language.map(str::to_string),
            code,
        });
    let mut options: Vec<(u32, BoxedStrategy<Block>)> = vec![
        (6, paragraph.boxed()),
        (2, heading.boxed()),
        (2, code.boxed()),
        (1, Just(Block::Rule).boxed()),
    ];
    if profile != Profile::Html {
        options.push((
            1,
            preserved(Source::Adf, ADF_BLOCKS)
                .prop_map(Block::Preserved)
                .boxed(),
        ));
    }
    if profile != Profile::Adf {
        options.push((
            1,
            preserved(Source::Html, HTML_BLOCKS)
                .prop_map(Block::Preserved)
                .boxed(),
        ));
    }
    if profile == Profile::Markdown {
        options.push((
            1,
            prop::sample::select(MARKDOWN_HTML_BLOCKS)
                .prop_map(|raw| Block::Html(raw.into()))
                .boxed(),
        ));
    }
    prop::strategy::Union::new_weighted(options).boxed()
}

fn block(profile: Profile) -> impl Strategy<Value = Block> {
    leaf(profile).prop_recursive(3, 24, 4, move |inner| {
        let items = prop::collection::vec(prop::collection::vec(inner.clone(), 1..3), 1..4);
        prop_oneof![
            items.clone().prop_map(Block::BulletList),
            (prop_oneof![0u64..12, Just(999_999_998)], items)
                .prop_map(|(start, items)| Block::OrderedList { start, items }),
            prop::collection::vec(inner, 1..3).prop_map(Block::Quote),
        ]
    })
}

/// ADF allows only paragraphs, lists, code and preserved nodes inside list
/// items and quotes, and no list inside a quote's quote.
fn adf_restrict(blocks: Vec<Block>, nested: bool) -> Vec<Block> {
    blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::BulletList(items) => Some(Block::BulletList(
                items
                    .into_iter()
                    .map(|item| adf_restrict(item, true))
                    .collect(),
            )),
            Block::OrderedList { start, items } => Some(Block::OrderedList {
                start,
                items: items
                    .into_iter()
                    .map(|item| adf_restrict(item, true))
                    .collect(),
            }),
            Block::Quote(blocks) if !nested => Some(Block::Quote(adf_restrict(blocks, true))),
            Block::Quote(_) | Block::Heading { .. } | Block::Rule if nested => None,
            other => Some(other),
        })
        .collect()
}

/// A normalized document representable in `profile`'s format.
pub(crate) fn doc(profile: Profile) -> impl Strategy<Value = Doc> {
    prop::collection::vec(block(profile), 0..6).prop_map(move |blocks| {
        let blocks = if profile == Profile::Adf {
            adf_restrict(blocks, false)
        } else {
            blocks
        };
        let mut doc = normalize(Doc { blocks });
        guard_line_start_html(&mut doc.blocks);
        normalize(doc)
    })
}
