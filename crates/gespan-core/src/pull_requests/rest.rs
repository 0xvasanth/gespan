//! Small helpers the host adapters share: paging and reading JSON fields.
use super::{
    error::Result,
    transport::{ApiRequest, Transport},
};
use serde_json::Value;

/// The most pages a list follows, 100 items each.
pub(crate) const MAX_PAGES: u32 = 10;

/// `path` with `query` appended after `?` or `&`.
pub(crate) fn with_query(path: &str, query: &str) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}{query}")
}

/// Every item of a paged list, asking `page=1, 2, …` with 100 a page while
/// the host says there is more: GitHub with a `Link: rel="next"` header,
/// GitLab with `X-Next-Page`. `items` picks the list out of each page.
pub(crate) fn paged(
    transport: &dyn Transport,
    path: &str,
    items: impl Fn(Value) -> Vec<Value>,
) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for page in 1..=MAX_PAGES {
        let response = transport.send(&ApiRequest::get(with_query(
            path,
            &format!("per_page=100&page={page}"),
        )))?;
        let more = response.has_next_link()
            || response
                .header("x-next-page")
                .is_some_and(|next| !next.trim().is_empty());
        out.extend(items(response.json()?));
        if !more {
            break;
        }
    }
    Ok(out)
}

pub(crate) fn array(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

/// A string field, or "" when it's missing or null.
pub(crate) fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// A string field that is present and not empty.
pub(crate) fn some_text(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

pub(crate) fn number(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// A number or a numeric string, as ids sometimes are.
pub(crate) fn id(value: &Value) -> String {
    match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

pub(crate) fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or_default().trim().to_owned()
}

/// One URL path segment, every byte but the unreserved ones escaped:
/// `group/sub/project` becomes `group%2Fsub%2Fproject`, `My Project`
/// becomes `My%20Project`.
pub(crate) fn encode(component: &str) -> String {
    let mut out = String::with_capacity(component.len());
    for byte in component.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
