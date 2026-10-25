//! Description converters: Markdown ↔ Jira ADF and Markdown ↔ Plane HTML.
//!
//! Every format is parsed into one small document model ([`doc::Doc`]) and
//! rendered out of it. Content the model can't hold (tables, panels,
//! mentions, styled text, …) is kept as the original ADF node or HTML element
//! and written back unchanged, so a round trip never drops it.
mod doc;
mod markdown;
#[cfg(test)]
mod strategies;
