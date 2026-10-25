//! Description converters: Markdown ↔ Jira ADF and Markdown ↔ Plane HTML.
//!
//! Every format is parsed into one small document model ([`doc::Doc`]) and
//! rendered out of it. Content the model can't hold (tables, panels,
//! mentions, styled text, …) is kept as the original ADF node or HTML element
//! and written back unchanged, so a round trip never drops it. In Markdown it
//! shows as a `gespan-adf` / `gespan-html` code block, or as a
//! `<span data-gespan-…>` around readable text; both are read-only.
mod adf;
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

/// A Jira description (an ADF `doc`, or `null`) to Markdown.
pub fn adf_to_markdown(adf: &Value) -> String {
    markdown::render(&adf::parse(adf))
}

/// Markdown to a Plane `description_html`.
pub fn markdown_to_plane_html(markdown: &str) -> String {
    html::render(&markdown::parse(markdown))
}

/// A Plane `description_html` to Markdown.
pub fn plane_html_to_markdown(html: &str) -> String {
    markdown::render(&html::parse(html))
}
