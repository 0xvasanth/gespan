//! The one-time import of preferences the user already keeps in harness
//! files (their global CLAUDE.md, AGENTS.md): each top-level rule becomes a
//! pending preference, waiting for review in My tasks.
use super::entry::{derived_title, Kind, NewEntry, Scope, Source};
use super::files::{closes_fence, fence_opener, owned_regular_file, replace_section};
use super::harness::read_capped;
use super::store::DecisionStore;
use crate::accounts;
use std::fs;
use std::path::{Path, PathBuf};

/// How many words of a rule go into its title after the heading.
const TITLE_WORDS: usize = 8;

/// The most rules taken from one file; the rest are skipped (and logged), so
/// a huge file cannot flood My tasks.
pub const MAX_RULES_PER_FILE: usize = 200;

/// A list marker found at the start of a line.
struct Marker<'a> {
    /// Spaces before the marker.
    indent: usize,
    /// Where a nested item's marker would have to start to belong to it.
    content: usize,
    /// The text after the marker.
    rest: &'a str,
}

/// A line's list marker: `-`, `*`, `+` or `1.` / `1)` (up to three digits)
/// after at most three spaces, followed by a space. Four or more spaces make
/// it plain indented text. A line of only dashes, stars and underscores
/// after the marker (`* * *`) is a horizontal rule, not a list item.
fn bullet(line: &str) -> Option<Marker<'_>> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let (width, after) = if let Some(after) = body.strip_prefix(['-', '*', '+']) {
        (1, after)
    } else {
        let digits = body.chars().take_while(char::is_ascii_digit).count();
        ((1..=3).contains(&digits)).then_some(())?;
        (digits + 1, body[digits..].strip_prefix(['.', ')'])?)
    };
    let rest = after.strip_prefix(' ')?.trim();
    let divider = !rest.is_empty() && rest.chars().all(|c| matches!(c, '-' | '*' | '_' | ' '));
    (!divider).then_some(Marker {
        indent,
        content: indent + width + 1,
        rest,
    })
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
    candidates_from(text, file, accounts::home().as_deref())
}

/// `candidates` with the home folder named, so the file is shown as `~/...`.
fn candidates_from(text: &str, file: &Path, home: Option<&Path>) -> Vec<NewEntry> {
    let text = replace_section(text, "");
    let mut rules: Vec<(String, String)> = Vec::new();
    let mut heading_now = String::new();
    let mut fence: Option<(char, usize)> = None;
    // Whether the last rule can still take continuation lines.
    let mut open = false;
    // Where the open rule's text starts, in columns.
    let mut content = 0;
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
        } else if let Some(marker) = bullet(line).filter(|m| !(open && m.indent >= content)) {
            // A bullet starting where the open rule's text starts is nested
            // in it; anywhere left of that it is a rule of its own.
            open = !marker.rest.is_empty();
            content = marker.content;
            if open {
                rules.push((heading_now.clone(), marker.rest.to_owned()));
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
    if rules.len() > MAX_RULES_PER_FILE {
        tracing::warn!(
            file = %file.display(),
            skipped = rules.len() - MAX_RULES_PER_FILE,
            "too many rules to import; the rest were skipped"
        );
    }
    let shown = tilde(file, home);
    let why = format!("Imported from {shown}");
    rules
        .into_iter()
        .take(MAX_RULES_PER_FILE)
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
                    import: Some(shown.clone()),
                    ..Source::default()
                },
            }
        })
        .collect()
}

/// `path` with the home folder written as `~`, so the user's account name
/// never lands in stored text. The same rule names the source of every
/// entry, so what an import matches on stays stable.
fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Whether `path` may be read: a regular file, and when it is a symlink,
/// one that stays inside the folder it sits in or ends at a regular file
/// the user owns (a global CLAUDE.md kept in a dotfiles repository), the
/// same files the harness writer follows.
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
    inside || owned_regular_file(path).is_ok()
}

/// Proposes the preferences in `files` as pending entries that came from
/// them, skipping Gespan's own section, files that are missing, not regular,
/// linked out of their folder or over the size cap, and rules already
/// imported from the same file (whatever the user did with them). Returns
/// how many entries it added.
pub fn import_once(store: &DecisionStore, files: &[PathBuf], now_ms: i64) -> Result<usize, String> {
    import_once_from(store, files, now_ms, accounts::home().as_deref())
}

/// [`import_once`] with the home folder named (tests give their own).
pub fn import_once_from(
    store: &DecisionStore,
    files: &[PathBuf],
    now_ms: i64,
    home: Option<&Path>,
) -> Result<usize, String> {
    let mut added = 0;
    for file in files {
        if !safe_to_read(file) {
            continue;
        }
        let Ok(Some(text)) = read_capped(file) else {
            continue;
        };
        for entry in candidates_from(&text, file, home) {
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
