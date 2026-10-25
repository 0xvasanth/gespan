use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Upper bound on terms handed to the recognizer.
pub const MAX_TERMS: usize = 230;
/// Shorter words are common speech and need no biasing.
pub const MIN_TERM_LEN: usize = 4;

const MAX_TRACKED_FILES: usize = 2000;

pub struct VocabularySources<'a> {
    pub project_root: Option<&'a Path>,
    pub branches: &'a [String],
    pub known_names: &'a [String],
    pub user_words: &'a [String],
}

/// Local branch names of the repo at `project_root`; empty when git fails.
pub fn local_branches(project_root: &Path) -> Vec<String> {
    git_output(
        project_root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .map(|out| {
        out.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

/// Builds the term list: user words first (never truncated away), then
/// project folders, tracked file stems, branch segments and known names.
pub fn pick_terms(src: &VocabularySources) -> Vec<String> {
    let mut candidates: Vec<String> = src.user_words.to_vec();

    if let Some(root) = src.project_root {
        candidates.extend(top_level_folders(root));
        candidates.extend(tracked_file_stems(root));
    }
    for branch in src.branches {
        candidates.extend(branch.split('/').map(String::from));
    }
    candidates.extend(src.known_names.iter().cloned());

    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for raw in candidates {
        let term = raw.trim();
        if term.chars().count() < MIN_TERM_LEN || term.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if seen.insert(term.to_lowercase()) {
            terms.push(term.to_string());
            if terms.len() == MAX_TERMS {
                break;
            }
        }
    }
    terms
}

fn top_level_folders(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

fn tracked_file_stems(root: &Path) -> Vec<String> {
    let Some(out) = git_output(root, &["ls-files", "-z"]) else {
        return Vec::new();
    };
    out.split('\0')
        .filter(|p| !p.is_empty())
        .take(MAX_TRACKED_FILES)
        .filter_map(|p| Path::new(p).file_stem()?.to_str().map(String::from))
        .filter(|stem| !stem.starts_with('.'))
        .collect()
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
