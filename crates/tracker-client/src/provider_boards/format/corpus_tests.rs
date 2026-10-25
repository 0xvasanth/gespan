//! Round trips over realistic descriptions from each tracker.
//!
//! The Jira fixtures are written in the form the converter produces (marks in
//! `strong, em, strike, code, link` order, no `localId` on converted nodes),
//! so a round trip must give them back exactly.

use serde_json::Value;

use super::doc::{Block, Doc, Inline};
use super::{
    adf, adf_to_markdown, html, markdown, markdown_to_adf, markdown_to_plane_html,
    plane_html_to_markdown,
};

const JIRA: &[(&str, &str)] = &[
    ("bug-report", include_str!("fixtures/jira/bug-report.json")),
    (
        "feature-request",
        include_str!("fixtures/jira/feature-request.json"),
    ),
    (
        "media-and-cards",
        include_str!("fixtures/jira/media-and-cards.json"),
    ),
    ("empty", include_str!("fixtures/jira/empty.json")),
];
const PLANE: &[(&str, &str)] = &[
    (
        "editor-output",
        include_str!("fixtures/plane/editor-output.html"),
    ),
    ("empty", include_str!("fixtures/plane/empty.html")),
];
const MARKDOWN: &[(&str, &str)] = &[
    ("agent-pr", include_str!("fixtures/markdown/agent-pr.md")),
    ("bug-note", include_str!("fixtures/markdown/bug-note.md")),
];

fn jira(name: &str) -> Value {
    let (_, text) = JIRA
        .iter()
        .find(|(n, _)| *n == name)
        .expect("fixture exists");
    serde_json::from_str(text).expect("fixture is JSON")
}

fn preserved_raws(doc: &Doc) -> Vec<String> {
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for block in blocks {
            match block {
                Block::Preserved(preserved) => out.push(preserved.raw.clone()),
                Block::Paragraph(content) | Block::Heading { content, .. } => {
                    for inline in content {
                        if let Inline::Preserved(preserved) = inline {
                            out.push(preserved.raw.clone());
                        }
                    }
                }
                Block::BulletList(items) | Block::OrderedList { items, .. } => {
                    items.iter().for_each(|item| walk(item, out))
                }
                Block::Quote(blocks) => walk(blocks, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    out
}

#[test]
fn jira_descriptions_round_trip_exactly() {
    for (name, text) in JIRA {
        let original: Value = serde_json::from_str(text).expect("fixture is JSON");
        let markdown = adf_to_markdown(&original);
        let back = markdown_to_adf(&markdown);
        assert_eq!(back, original, "{name}: markdown was\n{markdown}");
        assert_eq!(
            adf_to_markdown(&back),
            markdown,
            "{name}: markdown is stable"
        );
    }
}

#[test]
fn jira_bug_report_reads_as_markdown() {
    let markdown = adf_to_markdown(&jira("bug-report"));
    for expected in [
        "## Summary\n\nSaving a board with more than 50 cards fails with a **500** from the API. <span data-gespan-adf=",
        ">@Alice Moreau</span> first saw it on <span data-gespan-adf=",
        ">PROD</span>.",
        "## Steps to reproduce\n\n1. Open a board with _more than 50_ cards\n2. Drag any card to `Done`\n3. Click **Save**\\\n   Nothing happens for \\~10 s",
        "```rust\nError: request failed (500)\n    at save_board (board.rs:120)\n    at <async block>\n```",
        "```gespan-adf\n{\"attrs\":{\"isNumberColumnEnabled\":false",
        "See the [logs](https://logs.x.test/run/42) and ~~the old workaround~~ <span data-gespan-adf=",
    ] {
        assert!(markdown.contains(expected), "missing {expected:?} in\n{markdown}");
    }
}

#[test]
fn jira_unsupported_nodes_are_kept_in_place() {
    let original = jira("media-and-cards");
    let kinds = |value: &Value| -> Vec<String> {
        value["content"]
            .as_array()
            .expect("doc content")
            .iter()
            .map(|node| node["type"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    let back = markdown_to_adf(&adf_to_markdown(&original));
    assert_eq!(
        kinds(&back),
        [
            "paragraph",
            "mediaSingle",
            "paragraph",
            "expand",
            "paragraph",
            "taskList"
        ]
    );
    // Seven nodes the model can't hold, each carried through unchanged.
    assert_eq!(preserved_raws(&adf::parse(&original)).len(), 7);
}

#[test]
fn plane_descriptions_round_trip() {
    for (name, original) in PLANE {
        let doc = html::parse(original);
        let markdown = plane_html_to_markdown(original);
        let back = markdown_to_plane_html(&markdown);
        assert_eq!(html::parse(&back), doc, "{name}: markdown was\n{markdown}");
        assert_eq!(
            plane_html_to_markdown(&back),
            markdown,
            "{name}: markdown is stable"
        );
        assert_eq!(
            markdown_to_plane_html(&plane_html_to_markdown(&back)),
            back,
            "{name}: html is stable"
        );
        for raw in preserved_raws(&doc) {
            assert!(
                original.contains(&raw),
                "{name}: {raw} is a slice of the original"
            );
            assert!(
                back.contains(&raw),
                "{name}: {raw} is written back unchanged"
            );
        }
    }
}

#[test]
fn plane_editor_output_reads_as_markdown() {
    let (_, original) = PLANE[0];
    let markdown = plane_html_to_markdown(original);
    for expected in [
        "## Context\n\nThe sync job **drops labels** when a work item moves to `Done`. Reported by <span data-gespan-html=",
        "- Happens on _self-hosted_ only\n- Since [v0.24](https://github.com/makeplane/plane/releases), with ~~no~~ a workaround:\n\n  1. re-add the label",
        "```python\nfor item in work_items:\n    if item.labels and item.state == \"done\":\n        sync(item)\n```",
        "> \"We lost every label on 40 tickets.\"\\\n> — a customer\n\n---",
        "```gespan-html\n<ul data-type=\"taskList\"",
        "```gespan-html\n<p class=\"editor-paragraph-block\" style=\"text-align: center\">Centered note</p>\n```",
    ] {
        assert!(markdown.contains(expected), "missing {expected:?} in\n{markdown}");
    }
    assert_eq!(plane_html_to_markdown(PLANE[1].1), "");
}

#[test]
fn markdown_descriptions_survive_both_trackers() {
    for (name, original) in MARKDOWN {
        let doc = markdown::parse(original);
        assert_eq!(
            adf::parse(&markdown_to_adf(original)),
            doc,
            "{name} through Jira"
        );
        assert_eq!(
            html::parse(&markdown_to_plane_html(original)),
            doc,
            "{name} through Plane"
        );
        let canonical = markdown::render(&doc);
        assert_eq!(
            markdown::render(&markdown::parse(&canonical)),
            canonical,
            "{name} is stable"
        );
        assert_eq!(
            adf_to_markdown(&markdown_to_adf(&canonical)),
            canonical,
            "{name} via Jira text"
        );
        assert_eq!(
            plane_html_to_markdown(&markdown_to_plane_html(&canonical)),
            canonical,
            "{name} via Plane text"
        );
    }
}
