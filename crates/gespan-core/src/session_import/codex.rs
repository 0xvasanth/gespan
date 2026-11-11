//! Codex keeps one rollout per session, in
//! `<home>/sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl` and,
//! once archived, under `<home>/archived_sessions`, possibly compressed as
//! `.jsonl.zst`.
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Deserialize;

use super::{title_of, Candidate, HEAD_LIMIT};
use crate::accounts::transfer::codex::{rollout_matches, validate_thread_id};
use crate::outside_turns::{parse_codex, OutsideTurn, TurnRole};
use crate::provider::Kind;

pub(super) fn list(home: &Path) -> Vec<Candidate> {
    let mut files = Vec::new();
    for root in ["sessions", "archived_sessions"] {
        walk(&home.join(root), &mut files);
    }
    // Rollouts' heads are read side by side.
    files
        .par_iter()
        .filter_map(|path| candidate(path))
        .collect()
}

/// Collects every file under `dir`. Symlinks are not followed.
fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, files);
        } else if kind.is_file() {
            files.push(path);
        }
    }
}

/// The session id in a rollout's file name, or `None` when the name is not
/// a rollout's or the id is not a valid thread id.
fn id_from_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name
        .strip_suffix(".jsonl")
        .or_else(|| name.strip_suffix(".jsonl.zst"))?;
    let rest = stem.strip_prefix("rollout-")?;
    // `<yyyy-mm-ddThh-mm-ss>-<id>`; a name without that timestamp keeps
    // everything after its first dash.
    let timestamp = rest.as_bytes().get(..19).is_some_and(|t| {
        t.iter().enumerate().all(|(i, b)| match i {
            4 | 7 | 13 | 16 => *b == b'-',
            10 => *b == b'T',
            _ => b.is_ascii_digit(),
        })
    });
    let id = if timestamp {
        rest.get(20..)?
    } else {
        rest.split_once('-')?.1
    };
    validate_thread_id(id).ok()?;
    rollout_matches(path, id).then(|| id.to_string())
}

fn is_compressed(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "zst")
}

/// Up to `limit` bytes of the rollout, decompressed.
fn read_up_to(path: &Path, limit: Option<u64>) -> std::io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let reader: Box<dyn Read> = if is_compressed(path) {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut bytes = Vec::new();
    match limit {
        Some(limit) => reader.take(limit).read_to_end(&mut bytes)?,
        None => {
            let mut reader = reader;
            reader.read_to_end(&mut bytes)?
        }
    };
    Ok(bytes)
}

/// The file's candidate, or `None` when its name is not a rollout's, it is
/// unreadable, or its head holds no user message.
fn candidate(path: &Path) -> Option<Candidate> {
    let id = id_from_name(path)?;
    let updated_at = fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let head = read_up_to(path, Some(HEAD_LIMIT)).ok()?;

    let title = parse_codex(&head)
        .turns
        .iter()
        .filter(|t| t.role == TurnRole::User)
        .map(|t| title_of(&t.text))
        .find(|t| !t.is_empty())?;
    let (cwd, branch) = cwd_and_branch(&head);
    Some(Candidate {
        kind: Kind::Codex,
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
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<Meta>,
}

#[derive(Deserialize)]
struct Meta {
    cwd: Option<String>,
    git: Option<Git>,
}

#[derive(Deserialize)]
struct Git {
    branch: Option<String>,
}

/// The cwd and git branch of the first `session_meta` among the head's
/// complete lines.
fn cwd_and_branch(head: &[u8]) -> (Option<String>, Option<String>) {
    let complete = head.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    for line in head[..complete].split(|b| *b == b'\n') {
        let Ok(line) = serde_json::from_slice::<Line>(line) else {
            continue;
        };
        if line.kind.as_deref() != Some("session_meta") {
            continue;
        }
        let Some(meta) = line.payload else {
            continue;
        };
        let cwd = meta.cwd.filter(|s| !s.is_empty());
        let branch = meta.git.and_then(|g| g.branch).filter(|s| !s.is_empty());
        return (cwd, branch);
    }
    (None, None)
}

pub(super) fn read(path: &Path) -> Result<Vec<OutsideTurn>, String> {
    let bytes = read_up_to(path, None).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(parse_codex(&bytes).turns)
}
