//! Values of secret-named keys: `key=…`, `key: "…"`, `X-Api-Key: …`,
//! `"key": …` (plain or JSON-escaped) and `--key …`.
//!
//! Keys are found on their own and judged first; a value is only matched,
//! anchored at the end of its key, when the key is secret. A rejected key
//! costs no more than its own length, so scrubbing stays linear however many
//! counter keys (`tokens_used=…`) a line repeats.

use std::sync::LazyLock;

use regex::Regex;

use crate::fields::value_end;
use crate::patterns::{is_secret_key, is_strong_secret_key};
use crate::REDACTED;

/// Words that make a key a candidate. [`is_secret_key`] has the final say.
const MARKERS: &str = "pass|pwd|pw|secret|token|api[_-]?key|private[_-]?key|access[_-]?key|credential|authorization|cookie|sig";

static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?-u:\b)[A-Za-z0-9_-]*(?:{MARKERS})[A-Za-z0-9_-]*"
    ))
    .expect("key pattern")
});

/// After `"key`: the closing quote and the colon of a JSON pair.
static JSON_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(\\?)"\s*:\s*"#).expect("JSON separator pattern"));

/// After `key`: `=` and the value. An optional `Bearer`/`Basic`/`token`
/// scheme goes with it. A quoted value may hold escaped quotes and span
/// lines, and an unclosed quote runs to the end of the line. Anything up to
/// whitespace, `&` or a quote follows.
static EQUALS_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?i)^[ \t]*=[ \t]*(?:(?:bearer|basic|token)\s+)?"#,
        r#"(?:(?:\\"(?:[^"\\]|\\[^"])*\\"|"(?:[^"\\]|\\.)*"|'[^']*'|"[^"\n]*|'[^'\n]*)"#,
        r#"(?:[^\s&"\\]|\\[^"])*|(?:[^\s&"\\]|\\[^"])+)"#,
    ))
    .expect("equals value pattern")
});

/// After `key`: `: "…"`, as Rust's `Debug` and YAML write it. A quoted value
/// is never prose, so the colon is safe here.
static COLON_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^([ \t]*:[ \t]*)("(?:[^"\\]|\\.)*"|'[^']*')"#).expect("colon quoted pattern")
});

/// After `key`: the `:` (and an optional scheme) of an unquoted header
/// value. The value itself is scanned by hand in [`secret_value`].
static COLON_BARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)^([ \t]*:[ \t]*)((?:bearer|basic|token)[ \t]+)?"#)
        .expect("colon bare pattern")
});

/// How much of an unquoted header value decides whether it is a secret, so
/// a rejected one costs little however long it runs.
const DECIDE_WITHIN: usize = 512;

/// After `--key`: whitespace and the value.
static FLAG_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ \t]+[^\s-]\S*").expect("flag value pattern"));

/// Replaces the value of every secret-named key with `<redacted>`.
pub(crate) fn redact_secret_pairs(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(key) = KEY.find_at(input, from) {
        from = key.end();
        if !is_secret_key(key.as_str()) {
            continue;
        }
        if let Some((value_start, value_end, replacement)) = secret_value(input, key) {
            out.push_str(&input[copied..value_start]);
            out.push_str(&replacement);
            copied = value_end;
            from = value_end;
        }
    }
    out.push_str(&input[copied..]);
    out
}

/// Where the secret value after `key` starts and ends, and what replaces it.
fn secret_value(input: &str, key: regex::Match) -> Option<(usize, usize, String)> {
    let before = &input[..key.start()];
    let rest = &input[key.end()..];
    if before.ends_with('"') {
        if let Some(separator) = JSON_SEPARATOR.captures(rest) {
            let escaped = before.ends_with("\\\"");
            if escaped != separator[1].is_empty() {
                let start = key.end() + separator[0].len();
                let end = value_end(input.as_bytes(), start, escaped);
                if end > start {
                    let quote = if escaped { "\\\"" } else { "\"" };
                    return Some((start, end, format!("{quote}{REDACTED}{quote}")));
                }
            }
        }
    }
    if let Some(value) = EQUALS_VALUE.find(rest) {
        return Some((key.end(), key.end() + value.end(), format!("={REDACTED}")));
    }
    if let Some(caps) = COLON_QUOTED.captures(rest) {
        let quote = &caps[2][..1];
        let replacement = format!("{}{quote}{REDACTED}{quote}", &caps[1]);
        return Some((key.end(), key.end() + caps[0].len(), replacement));
    }
    if let Some(caps) = COLON_BARE.captures(rest) {
        let scheme = caps.get(2).map_or("", |scheme| scheme.as_str());
        let start = key.end() + caps[0].len();
        let ends_value = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';');
        // Decide on the first DECIDE_WITHIN bytes only, then replace the
        // whole value up to whitespace, a quote, `,` or `;`.
        let window = &input.as_bytes()[start..input.len().min(start + DECIDE_WITHIN)];
        let shown = window
            .iter()
            .position(|b| b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b',' | b';'))
            .unwrap_or(window.len());
        let value = &window[..shown];
        // "token: missing" stays; a strong name needs no digit once the value
        // is too long to be a word.
        let looks_secret = value.len() >= 8
            && (!scheme.is_empty()
                || value.iter().any(u8::is_ascii_digit)
                || (is_strong_secret_key(key.as_str()) && value.len() >= 16));
        if looks_secret {
            let end = input[start..]
                .find(ends_value)
                .map_or(input.len(), |offset| start + offset);
            let replacement = format!("{}{scheme}{REDACTED}", &caps[1]);
            return Some((key.end(), end, replacement));
        }
    }
    let dashes = before.len() - before.trim_end_matches('-').len();
    let flag_start = &before[..before.len() - dashes];
    if (1..=2).contains(&dashes)
        && (flag_start.is_empty() || flag_start.ends_with(char::is_whitespace))
    {
        if let Some(value) = FLAG_VALUE.find(rest) {
            return Some((key.end(), key.end() + value.end(), format!(" {REDACTED}")));
        }
    }
    None
}
