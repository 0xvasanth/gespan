//! Claude Code keeps one JSONL file per session in
//! `<home>/projects/<folder>/<session id>.jsonl`. Sidechain files live in
//! subfolders and are skipped.
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use super::{title_of, Candidate, HEAD_LIMIT};
use crate::outside_turns::{parse_claude, OutsideTurn, TurnRole};
use crate::provider::Kind;

pub(super) fn list(home: &Path) -> Vec<Candidate> {
    let Ok(folders) = fs::read_dir(home.join("projects")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for folder in folders.flatten() {
        let Ok(files) = fs::read_dir(folder.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let is_session = path.extension().is_some_and(|e| e == "jsonl")
                && file.file_type().is_ok_and(|t| t.is_file());
            if !is_session {
                continue;
            }
            if let Some(candidate) = candidate(&path) {
                found.push(candidate);
            }
        }
    }
    found
}

/// The file's candidate, or `None` when it is unreadable or holds no real
/// user prompt in its head.
fn candidate(path: &Path) -> Option<Candidate> {
    let id = path.file_stem()?.to_str()?.to_string();
    let meta = fs::metadata(path).ok()?;
    let updated_at = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let mut head = Vec::new();
    File::open(path)
        .ok()?
        .take(HEAD_LIMIT)
        .read_to_end(&mut head)
        .ok()?;

    let turns = parse_claude(&head, None).turns;
    let title = turns
        .iter()
        .filter(|t| t.role == TurnRole::User)
        .map(|t| title_of(&t.text))
        .find(|t| !t.is_empty())?;
    let (cwd, branch) = cwd_and_branch(&head);
    Some(Candidate {
        kind: Kind::Claude,
        provider_id: id,
        cwd: cwd.unwrap_or_default(),
        title,
        branch,
        updated_at,
        messages: None,
        path: path.to_path_buf(),
    })
}

#[derive(Deserialize)]
struct Where {
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
}

/// The first `cwd` and the first `gitBranch` among the head's complete lines.
fn cwd_and_branch(head: &[u8]) -> (Option<String>, Option<String>) {
    let complete = head.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let (mut cwd, mut branch) = (None, None);
    for line in head[..complete].split(|b| *b == b'\n') {
        let Ok(line) = serde_json::from_slice::<Where>(line) else {
            continue;
        };
        cwd = cwd.or(line.cwd.filter(|s| !s.is_empty()));
        branch = branch.or(line.git_branch.filter(|s| !s.is_empty()));
        if cwd.is_some() && branch.is_some() {
            break;
        }
    }
    (cwd, branch)
}

pub(super) fn read(path: &Path) -> Result<Vec<OutsideTurn>, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(parse_claude(&bytes, None).turns)
}
