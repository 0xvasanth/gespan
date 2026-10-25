//! Replacing a known directory (home or a project root) wherever it appears.

use std::path::Path;

use regex::Regex;

/// One directory and the placeholder that replaces it.
#[derive(Debug)]
pub(crate) struct PathRule {
    pattern: Regex,
    placeholder: String,
    /// The configured path's length, so longer roots can be applied first.
    pub(crate) len: usize,
}

impl PathRule {
    /// `None` for a path that would match everything: empty or a bare root.
    pub(crate) fn new(path: &Path, placeholder: String) -> Option<Self> {
        let raw = path.to_string_lossy();
        let windows = raw.contains('\\') || is_drive(&raw);
        let separators: &[char] = if windows { &['\\', '/'] } else { &['/'] };
        let trimmed = raw.trim_end_matches(separators);
        if trimmed.is_empty() || (windows && is_drive(trimmed) && trimmed.len() == 2) {
            return None;
        }
        // Windows paths turn up with either separator, in any case, and with
        // doubled backslashes inside JSON-escaped text.
        let pattern = if windows {
            let parts: Vec<String> = trimmed
                .split(separators)
                .filter(|part| !part.is_empty())
                .map(regex::escape)
                .collect();
            format!(r"(?i){}", parts.join(r"[\\/]+"))
        } else {
            regex::escape(trimmed)
        };
        Some(Self {
            pattern: Regex::new(&pattern).expect("escaped path is a valid pattern"),
            placeholder,
            len: trimmed.len(),
        })
    }

    pub(crate) fn apply(&self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut last = 0;
        for found in self.pattern.find_iter(input) {
            // `/Users/al` must not eat `/Users/alice`.
            if input[found.end()..]
                .chars()
                .next()
                .is_some_and(continues_name)
            {
                continue;
            }
            out.push_str(&input[last..found.start()]);
            out.push_str(&self.placeholder);
            last = found.end();
        }
        out.push_str(&input[last..]);
        out
    }
}

fn is_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn continues_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}
