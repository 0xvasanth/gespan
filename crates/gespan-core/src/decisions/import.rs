//! The one-time import of preferences the user already keeps in harness
//! files (their global CLAUDE.md, AGENTS.md): each top-level rule becomes a
//! pending preference, waiting for review in My tasks.
use super::entry::{derived_title, Kind, NewEntry, Scope, Source};
use super::files::{closes_fence, fence_opener, replace_section};
use super::harness::read_capped;
use super::store::DecisionStore;
use std::fs;
use std::path::{Path, PathBuf};

/// How many words of a rule go into its title after the heading.
const TITLE_WORDS: usize = 8;

/// A line's top-level list marker: `-`, `*`, `+` or `1.` / `1)` at column
/// zero, followed by a space. Returns the rule text after it.
fn bullet(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix(['-', '*', '+'])
        .or_else(|| {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            let after = &line[digits..];
            (digits > 0)
                .then(|| after.strip_prefix(['.', ')']))
                .flatten()
        })?
        .strip_prefix(' ')?;
    Some(rest.trim())
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some(rest.trim().trim_end_matches('#').trim())
}

/// One pending preference per top-level bullet or numbered rule of `text`
/// (Gespan's own section excluded), with nested bullets and wrapped lines
/// folded into their parent. The title is the nearest heading plus the first
/// words of the rule; the why names `file`.
pub fn candidates(text: &str, file: &Path) -> Vec<NewEntry> {
    let text = replace_section(text, "");
    let mut rules: Vec<(String, String)> = Vec::new();
    let mut heading_now = String::new();
    let mut fence: Option<(char, usize)> = None;
    // Whether the last rule can still take continuation lines.
    let mut open = false;
    for line in text.lines() {
        if let Some((ch, len)) = fence {
            if closes_fence(line, ch, len) {
                fence = None;
            }
            if open {
                if let Some((_, body)) = rules.last_mut() {
                    body.push('\n');
                    body.push_str(line.trim_end());
                }
            }
            continue;
        }
        if let Some(opener) = fence_opener(line) {
            fence = Some(opener);
            if open {
                if let Some((_, body)) = rules.last_mut() {
                    body.push('\n');
                    body.push_str(line.trim_end());
                }
            }
            continue;
        }
        if let Some(title) = heading(line) {
            heading_now = title.to_owned();
            open = false;
        } else if let Some(rule) = bullet(line) {
            open = true;
            if !rule.is_empty() {
                rules.push((heading_now.clone(), rule.to_owned()));
            } else {
                open = false;
            }
        } else if line.trim().is_empty() {
            // A blank line may sit between a rule and its nested items.
        } else if line.starts_with([' ', '\t']) {
            if open {
                if let Some((_, body)) = rules.last_mut() {
                    body.push('\n');
                    body.push_str(line.trim());
                }
            }
        } else {
            // Unindented prose ends the list.
            open = false;
        }
    }
    let why = format!("Imported from {}", file.display());
    rules
        .into_iter()
        .map(|(heading, body)| {
            let words: Vec<&str> = body
                .lines()
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .take(TITLE_WORDS)
                .collect();
            let lead = words.join(" ");
            let title = if heading.is_empty() {
                lead
            } else {
                format!("{heading}: {lead}")
            };
            NewEntry {
                kind: Kind::Preference,
                scope: Scope::Global,
                title: derived_title(&title),
                text: body,
                why: why.clone(),
                user_answer: None,
                source: Source {
                    import: Some(file.display().to_string()),
                    ..Source::default()
                },
            }
        })
        .collect()
}

/// Whether `path` may be read: a regular file, and when it is a symlink, one
/// that stays inside the folder it sits in.
fn safe_to_read(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return true;
    }
    let inside = path
        .parent()
        .and_then(|dir| fs::canonicalize(dir).ok())
        .zip(fs::canonicalize(path).ok())
        .is_some_and(|(dir, target)| target.starts_with(dir));
    inside
}

/// Proposes the preferences in `files` as pending entries that came from
/// them, skipping Gespan's own section, files that are missing, not regular,
/// linked out of their folder or over the size cap, and rules already
/// imported from the same file (whatever the user did with them). Returns
/// how many entries it added.
pub fn import_once(store: &DecisionStore, files: &[PathBuf], now_ms: i64) -> Result<usize, String> {
    let mut added = 0;
    for file in files {
        if !safe_to_read(file) {
            continue;
        }
        let Ok(Some(text)) = read_capped(file) else {
            continue;
        };
        for entry in candidates(&text, file) {
            let proposed = store
                .propose_import(entry, now_ms)
                .map_err(|e| e.to_string())?;
            added += usize::from(proposed.is_some());
        }
    }
    Ok(added)
}

/// The user's global rule files the import reads, found the way the harness
/// writer finds them: Claude's `CLAUDE.md`, Codex's `AGENTS.md` and OpenCode's
/// `AGENTS.md` when it exists. Each path once.
pub fn global_files(homes: super::harness::Homes) -> Vec<PathBuf> {
    use crate::provider::Kind;
    let mut files: Vec<PathBuf> = Vec::new();
    for (kind, name) in [
        (Kind::Claude, "CLAUDE.md"),
        (Kind::Codex, "AGENTS.md"),
        (Kind::OpenCode, "AGENTS.md"),
    ] {
        if let Some(path) = homes(kind).map(|home| home.join(name)) {
            if path.is_file() && !files.contains(&path) {
                files.push(path);
            }
        }
    }
    files
}
