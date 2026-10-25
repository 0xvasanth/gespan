//! Deny-listed fields inside text: a log line that never parsed as JSON, a
//! log tail cut mid-line, or JSON escaped inside another JSON string.

use std::sync::LazyLock;

use regex::Regex;

use crate::{DENY_LIST, REDACTED};

static DENIED_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i)(\\?")(?:{})\\?"\s*:\s*"#,
        DENY_LIST.join("|")
    ))
    .expect("denied key pattern")
});

/// Replaces the value of every deny-listed `"key": value` pair with
/// `"<redacted>"`. Text can't drop a field cleanly, so the key stays. A
/// value that is cut short runs to the end of its line.
pub(crate) fn blank_denied_fields(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pos = 0;
    while let Some(caps) = DENIED_KEY.captures_at(input, pos) {
        let key = caps.get(0).expect("whole match");
        let escaped = caps[1].starts_with('\\');
        let end = value_end(input.as_bytes(), key.end(), escaped);
        out.push_str(&input[pos..key.end()]);
        if escaped {
            out.push_str(&format!("\\\"{REDACTED}\\\""));
        } else {
            out.push_str(&format!("\"{REDACTED}\""));
        }
        pos = end;
    }
    out.push_str(&input[pos..]);
    out
}

/// Where the JSON value starting at `start` ends. In JSON-escaped text a
/// structural quote is `\"`, so it follows 1 (mod 4) backslashes; in plain
/// text it follows an even number.
///
/// A value that is cut short ends at the end of its line. JSON strings never
/// hold a raw newline, and JSONL puts one record per line, so the next line
/// is another record and must survive.
pub(crate) fn value_end(bytes: &[u8], start: usize, escaped: bool) -> usize {
    let structural = |i: usize| {
        let backslashes = bytes[..i].iter().rev().take_while(|b| **b == b'\\').count();
        if escaped {
            backslashes % 4 == 1
        } else {
            backslashes % 2 == 0
        }
    };
    let opens_string = |i: usize| {
        if escaped {
            bytes[i..].starts_with(b"\\\"")
        } else {
            bytes[i] == b'"'
        }
    };
    let line_end = || {
        bytes[start..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |offset| start + offset)
    };
    if start >= bytes.len() {
        return start;
    }
    if opens_string(start) {
        let first = start + if escaped { 2 } else { 1 };
        for (i, byte) in bytes.iter().enumerate().skip(first) {
            match byte {
                b'\n' => return i,
                b'"' if structural(i) => return i + 1,
                _ => {}
            }
        }
        return bytes.len();
    }
    if matches!(bytes[start], b'{' | b'[') {
        let mut depth = 0usize;
        let mut in_string = false;
        for (i, byte) in bytes.iter().enumerate().skip(start) {
            match byte {
                b'\n' if in_string => return line_end(),
                b'"' if structural(i) => in_string = !in_string,
                b'{' | b'[' if !in_string => depth += 1,
                b'}' | b']' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
        }
        return line_end();
    }
    (start..bytes.len())
        .find(|&i| {
            matches!(bytes[i], b',' | b'}' | b']' | b'"' | b'\\') || bytes[i].is_ascii_whitespace()
        })
        .unwrap_or(bytes.len())
}
