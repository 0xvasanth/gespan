//! The change an Edit or Write tool proposes, read from its own arguments
//! rather than from git, as src/integrations/harness/core/preview.ts builds
//! it: a few lines from the first change, with the whole edit's counts.
use super::{presentation, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_LINES: usize = 6;
const MAX_LINE_CHARS: usize = 120;
/// Where providers that send a diff outside the arguments keep its preview.
pub const DISPLAY_KEY: &str = "editPreview";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    Add,
    Del,
    Context,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// Absent for replacement excerpts, which do not start at line one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<usize>,
    pub kind: LineKind,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPreview {
    pub lines: Vec<Line>,
    #[serde(default)]
    pub additions: usize,
    #[serde(default)]
    pub deletions: usize,
    /// A Write's new contents, without the previous file to compare.
    #[serde(default)]
    pub content_only: bool,
}

impl EditPreview {
    /// Written content, or at least one changed line: something to show.
    pub fn shows_change(&self) -> bool {
        self.content_only || self.lines.iter().any(|line| line.kind != LineKind::Context)
    }

    /// The preview of replacing `old` with `new`, numbered in the new text.
    pub fn from_texts(old: Option<&str>, new: &str) -> Self {
        let old_lines = text_lines(old.unwrap_or(""));
        let new_lines = text_lines(new);
        let lines = if old_lines.is_empty() {
            new_lines
                .iter()
                .enumerate()
                .map(|(index, text)| line(Some(index + 1), LineKind::Add, text))
                .collect()
        } else {
            greedy_diff(&old_lines, &new_lines)
        };
        compact(lines)
    }

    /// A unified diff's first hunk lines; `None` when `text` is not a patch.
    pub fn from_patch(text: &str) -> Option<Self> {
        let trimmed = text.trim_start();
        if !(trimmed.starts_with("diff --git") || trimmed.starts_with("@@ ")) {
            return None;
        }
        let mut lines = Vec::new();
        let mut number = 0;
        for raw in text.replace("\r\n", "\n").split('\n') {
            if let Some(start) = hunk_start(raw) {
                number = start;
                continue;
            }
            if ["diff ", "index ", "--- ", "+++ ", "\\"]
                .iter()
                .any(|prefix| raw.starts_with(prefix))
            {
                continue;
            }
            if let Some(text) = raw.strip_prefix('+') {
                lines.push(line(Some(number), LineKind::Add, text));
                number += 1;
            } else if let Some(text) = raw.strip_prefix('-') {
                lines.push(line(Some(number), LineKind::Del, text));
            } else {
                let text = raw.strip_prefix(' ').unwrap_or(raw);
                lines.push(line(Some(number), LineKind::Context, text));
                number += 1;
            }
        }
        Some(compact(lines))
    }
}

pub(super) fn preview(call: &ToolCall) -> Option<EditPreview> {
    let action = call.presentation().action;
    if !matches!(action.as_str(), "Edit" | "Write") {
        return None;
    }
    let saved = call
        .display
        .as_ref()
        .and_then(|display| display.get(DISPLAY_KEY))
        .and_then(|value| serde_json::from_value::<EditPreview>(value.clone()).ok());
    if saved.is_some() {
        return saved;
    }
    let mut input = serde_json::from_str::<Value>(&call.input).unwrap_or(Value::Null);
    presentation::normalize_inputs(&mut input, 0);
    let mut records = Vec::new();
    presentation::inputs(&input, 0, &mut records);
    // MultiEdit's first edit stands for the call, as its first change does.
    if let Some(first) = input["edits"].as_array().and_then(|edits| edits.first()) {
        records.push(first);
    }
    for record in &records {
        for (old, new) in [
            ("old_string", "new_string"),
            ("oldText", "newText"),
            ("old_text", "new_text"),
            ("oldString", "newString"),
        ] {
            let (Some(before), Some(after)) = (record[old].as_str(), record[new].as_str()) else {
                continue;
            };
            let mut built = EditPreview::from_texts(Some(before), after);
            // Replacement strings are excerpts, not the file from line one.
            for line in &mut built.lines {
                line.number = None;
            }
            return Some(built);
        }
    }
    if action == "Write" {
        if let Some(content) = records.iter().find_map(|record| record["content"].as_str()) {
            // Write may replace an existing file. Without its previous
            // contents these are the written lines, not all additions.
            return Some(EditPreview {
                lines: text_lines(content)
                    .iter()
                    .take(MAX_LINES)
                    .enumerate()
                    .map(|(index, text)| line(Some(index + 1), LineKind::Context, text))
                    .collect(),
                content_only: true,
                ..EditPreview::default()
            });
        }
    }
    None
}

fn line(number: Option<usize>, kind: LineKind, text: &str) -> Line {
    let text = if text.chars().count() <= MAX_LINE_CHARS {
        text.to_owned()
    } else {
        let mut capped: String = text.chars().take(MAX_LINE_CHARS - 1).collect();
        capped.push('…');
        capped
    };
    Line { number, kind, text }
}

/// Counts for the whole change; lines from just before the first change.
fn compact(lines: Vec<Line>) -> EditPreview {
    let count = |kind| lines.iter().filter(|line| line.kind == kind).count();
    let (additions, deletions) = (count(LineKind::Add), count(LineKind::Del));
    let start = lines
        .iter()
        .position(|line| line.kind != LineKind::Context)
        .unwrap_or(0)
        .saturating_sub(1);
    EditPreview {
        lines: lines.into_iter().skip(start).take(MAX_LINES).collect(),
        additions,
        deletions,
        content_only: false,
    }
}

fn text_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

fn hunk_start(line: &str) -> Option<usize> {
    let rest = line.strip_prefix("@@ -")?;
    let new = rest.split_once(" +")?.1;
    new.split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Matching lines stay context; a short look-ahead resyncs after a change.
fn greedy_diff(old: &[&str], new: &[&str]) -> Vec<Line> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            out.push(line(Some(j + 1), LineKind::Context, old[i]));
            i += 1;
            j += 1;
            continue;
        }
        if let Some((si, sj)) = find_sync(old, new, i, j, 8) {
            while i < si {
                out.push(line(Some(i + 1), LineKind::Del, old[i]));
                i += 1;
            }
            while j < sj {
                out.push(line(Some(j + 1), LineKind::Add, new[j]));
                j += 1;
            }
            continue;
        }
        if i < old.len() {
            out.push(line(Some(i + 1), LineKind::Del, old[i]));
            i += 1;
        } else {
            out.push(line(Some(j + 1), LineKind::Add, new[j]));
            j += 1;
        }
    }
    out
}

fn find_sync(
    old: &[&str],
    new: &[&str],
    i: usize,
    j: usize,
    window: usize,
) -> Option<(usize, usize)> {
    for di in 0..=window {
        for dj in 0..=window {
            if di == 0 && dj == 0 {
                continue;
            }
            let (oi, nj) = (i + di, j + dj);
            if oi < old.len() && nj < new.len() && old[oi] == new[nj] {
                return Some((oi, nj));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
