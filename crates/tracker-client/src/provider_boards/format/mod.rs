//! Description converters: Markdown ↔ Jira ADF and Markdown ↔ Plane HTML.
//!
//! Every format is parsed into one small document model ([`doc::Doc`]) and
//! rendered out of it. Content the model can't hold (tables, panels,
//! mentions, styled text, …) is kept as the original ADF node or HTML element
//! and written back unchanged, so a round trip never drops it. In Markdown it
//! shows as a `gespan-adf` / `gespan-html` code block, or as a
//! `<span data-gespan-…>` around readable text; both are read-only.
mod adf;
#[cfg(test)]
mod corpus_tests;
mod doc;
mod html;
mod markdown;
#[cfg(test)]
mod strategies;

use serde_json::Value;

/// Markdown to a Jira description (an ADF `doc`).
pub fn markdown_to_adf(markdown: &str) -> Value {
    adf::render(&markdown::parse(markdown))
}

/// Markdown to an ADF comment: like [`markdown_to_adf`], but each single
/// newline outside code blocks is a line break.
pub fn markdown_to_adf_comment(markdown: &str) -> Value {
    adf::render(&markdown::parse_comment(markdown))
}

/// A Jira description (an ADF `doc`, or `null`) to Markdown.
///
/// Whitespace in text is normalized as Markdown reads it (runs of spaces
/// collapse, edge spaces drop), so the first write-back can change it once;
/// after that the round trip is stable.
pub fn adf_to_markdown(adf: &Value) -> String {
    markdown::render(&adf::parse(adf))
}

/// Markdown to a Plane `description_html`.
///
/// Not sanitized: raw HTML in the Markdown (including `<script>` or `on…`
/// attributes) and kept Plane elements are written as they are, so this relies
/// on Plane sanitizing on its side. A caller writing text that didn't come
/// from Plane or the user must sanitize it first.
pub fn markdown_to_plane_html(markdown: &str) -> String {
    html::render(&markdown::parse(markdown))
}

/// Markdown to a Plane comment: like [`markdown_to_plane_html`], but each
/// single newline outside code blocks is a line break. Same sanitizing caveat.
pub fn markdown_to_plane_comment_html(markdown: &str) -> String {
    html::render(&markdown::parse_comment(markdown))
}

/// A Plane `description_html` to Markdown.
pub fn plane_html_to_markdown(html: &str) -> String {
    markdown::render(&html::parse(html))
}
