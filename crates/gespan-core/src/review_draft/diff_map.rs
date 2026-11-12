//! Which lines of a unified diff can carry an inline review comment, and on
//! which side.
//!
//! GitHub and GitLab only accept a comment on a line shown in the diff: an
//! added or removed line, or a context line inside a hunk. Anything else is
//! refused by the forge, so a finding is placed only when its line is here.

use serde::{Deserialize, Serialize};

/// The side of the diff a line is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The original file: removed lines, and context lines by their old number.
    Old,
    /// The changed file: added lines, and context lines by their new number.
    New,
}

/// One line shown in a hunk. Added lines have only `new`, removed lines only
/// `old`, and context lines both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub old: Option<u32>,
    pub new: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// The path before the change; the new path for an added file.
    pub old_path: String,
    /// The path after the change; the old path for a deleted file.
    pub new_path: String,
    pub lines: Vec<DiffLine>,
}

/// Where a comment goes: the file's two paths, the side, and both line
/// numbers the line has (GitLab wants both for a context line).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub old_path: String,
    pub new_path: String,
    pub side: Side,
    pub line: u32,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffMap {
    pub files: Vec<FileDiff>,
}

impl DiffMap {
    /// Reads `git diff` / `gh pr diff` output, or the patch Gespan builds
    /// from GitLab's diffs. Each hunk is read by its line counts, so a removed
    /// line that starts with `--` is never taken for a file header.
    pub fn parse(diff: &str) -> Self {
        let mut files: Vec<FileDiff> = Vec::new();
        let mut old_no = 0u32;
        let mut new_no = 0u32;
        let mut old_left = 0u32;
        let mut new_left = 0u32;
        for raw in diff.lines() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if old_left > 0 || new_left > 0 {
                if let Some(file) = files.last_mut() {
                    match line.as_bytes().first() {
                        Some(b'+') => {
                            file.lines.push(DiffLine {
                                old: None,
                                new: Some(new_no),
                            });
                            new_no += 1;
                            new_left = new_left.saturating_sub(1);
                            continue;
                        }
                        Some(b'-') => {
                            file.lines.push(DiffLine {
                                old: Some(old_no),
                                new: None,
                            });
                            old_no += 1;
                            old_left = old_left.saturating_sub(1);
                            continue;
                        }
                        // "\ No newline at end of file" belongs to the line before.
                        Some(b'\\') => continue,
                        // A context line; some tools strip its leading space
                        // when the line is blank.
                        Some(b' ') | None => {
                            file.lines.push(DiffLine {
                                old: Some(old_no),
                                new: Some(new_no),
                            });
                            old_no += 1;
                            new_no += 1;
                            old_left = old_left.saturating_sub(1);
                            new_left = new_left.saturating_sub(1);
                            continue;
                        }
                        // Anything else ends a hunk whose counts were wrong;
                        // the line is read as a header below.
                        Some(_) => {}
                    }
                }
                old_left = 0;
                new_left = 0;
            }
            if let Some(rest) = line.strip_prefix("diff --git ") {
                let (old, new) = git_header_paths(rest);
                files.push(FileDiff {
                    old_path: old,
                    new_path: new,
                    lines: Vec::new(),
                });
                continue;
            }
            let Some(file) = files.last_mut() else {
                continue;
            };
            if let Some(rest) = line.strip_prefix("--- ") {
                if let Some(path) = header_path(rest, "a/") {
                    file.old_path = path;
                }
            } else if let Some(rest) = line.strip_prefix("+++ ") {
                if let Some(path) = header_path(rest, "b/") {
                    file.new_path = path;
                }
            } else if let Some(rest) = line.strip_prefix("rename from ") {
                file.old_path = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("rename to ") {
                file.new_path = unquote(rest);
            } else if let Some((old_start, old_count, new_start, new_count)) = hunk_header(line) {
                old_no = old_start;
                new_no = new_start;
                old_left = old_count;
                new_left = new_count;
            }
        }
        for file in &mut files {
            if file.old_path.is_empty() {
                file.old_path = file.new_path.clone();
            }
            if file.new_path.is_empty() {
                file.new_path = file.old_path.clone();
            }
        }
        Self { files }
    }

    /// The anchor for `line` on `side` of `path`, when the diff shows it.
    /// `path` may be either name of a renamed file.
    pub fn locate(&self, path: &str, side: Side, line: u32) -> Option<Anchor> {
        let path = normalize_path(path);
        let file = self
            .files
            .iter()
            .find(|f| f.new_path == path)
            .or_else(|| self.files.iter().find(|f| f.old_path == path))?;
        let shown = file.lines.iter().find(|l| match side {
            Side::Old => l.old == Some(line),
            Side::New => l.new == Some(line),
        })?;
        Some(Anchor {
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            side,
            line,
            old_line: shown.old,
            new_line: shown.new,
        })
    }
}

/// A path as an agent may write it: `./src/x.rs`, `src\\x.rs` or ` src/x.rs `.
pub fn normalize_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    path.trim_start_matches('/').to_string()
}

/// `a/x b/y` from a `diff --git` line. Used only until `---`/`+++` or the
/// rename lines name the paths, which binary and mode-only changes lack.
fn git_header_paths(rest: &str) -> (String, String) {
    let rest = rest.trim();
    if rest.starts_with('"') {
        if let Some(words) = shlex::split(rest) {
            if let [a, b] = words.as_slice() {
                return (
                    a.strip_prefix("a/").unwrap_or(a).to_string(),
                    b.strip_prefix("b/").unwrap_or(b).to_string(),
                );
            }
        }
    }
    // Same path on both sides is the common case: "a/<p> b/<p>".
    if let Some(body) = rest.strip_prefix("a/") {
        let half = body.len().saturating_sub(2) / 2;
        if body.is_char_boundary(half) {
            let (left, right) = body.split_at(half);
            if let Some(right) = right.strip_prefix(" b/") {
                if left == right {
                    return (left.to_string(), right.to_string());
                }
            }
        }
        if let Some((left, right)) = body.split_once(" b/") {
            return (left.to_string(), right.to_string());
        }
    }
    (String::new(), String::new())
}

/// The path on a `---`/`+++` line, or `None` for `/dev/null`.
fn header_path(rest: &str, prefix: &str) -> Option<String> {
    // Git ends the path with a tab when it holds a space.
    let rest = rest.split('\t').next().unwrap_or(rest).trim();
    if rest == "/dev/null" {
        return None;
    }
    let path = unquote(rest);
    Some(path.strip_prefix(prefix).unwrap_or(&path).to_string())
}

/// Git quotes a path with unusual characters as a C string.
fn unquote(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        let mut out = Vec::new();
        let inner = &text.as_bytes()[1..text.len() - 1];
        let mut i = 0;
        while i < inner.len() {
            let byte = inner[i];
            if byte == b'\\' && i + 1 < inner.len() {
                let next = inner[i + 1];
                match next {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'0'..=b'7'
                        if i + 3 < inner.len()
                            && inner[i + 2].is_ascii_digit()
                            && inner[i + 3].is_ascii_digit() =>
                    {
                        let octal = std::str::from_utf8(&inner[i + 1..i + 4]).unwrap_or("0");
                        out.push(u8::from_str_radix(octal, 8).unwrap_or(b'?'));
                        i += 4;
                        continue;
                    }
                    other => out.push(other),
                }
                i += 2;
            } else {
                out.push(byte);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    } else {
        text.to_string()
    }
}

/// `@@ -a[,b] +c[,d] @@` as (a, b, c, d). A missing count is 1.
fn hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let range = |text: &str| -> Option<(u32, u32)> {
        match text.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    let (a, b) = range(old)?;
    let (c, d) = range(new)?;
    Some((a, b, c, d))
}
