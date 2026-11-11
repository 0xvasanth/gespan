//! Sessions an agent kept in its own folder, found on disk so they can be
//! brought into Gespan. Nothing here writes to an agent's files.
use std::path::{Path, PathBuf};

use crate::outside_turns::OutsideTurn;
use crate::provider::Kind;

pub use placement::{already_imported, place, Placement, Skip};

mod claude;
mod codex;
mod placement;
#[cfg(test)]
mod tests;

/// Bytes read from the start of each file when listing, so a session of
/// hundreds of megabytes lists as fast as a small one.
pub(crate) const HEAD_LIMIT: u64 = 1 << 20;

/// One session found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    /// The agent's own session id.
    pub provider_id: String,
    /// The folder the session ran in.
    pub cwd: String,
    /// The first user prompt, first line, trimmed.
    pub title: String,
    pub branch: Option<String>,
    /// File modification time, in Unix milliseconds.
    pub updated_at: u64,
    /// User and assistant turns; `None` until counted.
    pub messages: Option<u32>,
    /// The session file or folder.
    pub path: PathBuf,
}

/// Sessions of `kind` under its `home`, newest first.
pub fn list(kind: Kind, home: &Path) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = match kind {
        Kind::Claude => claude::list(home),
        Kind::Codex => codex::list(home),
        _ => Vec::new(),
    };
    found.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    found
}

/// Every visible turn of the session, oldest first.
pub fn read_turns(candidate: &Candidate) -> Result<Vec<OutsideTurn>, String> {
    match candidate.kind {
        Kind::Claude => claude::read(&candidate.path),
        Kind::Codex => codex::read(&candidate.path),
        _ => Err("not supported".into()),
    }
}

/// How many user and assistant turns the session holds.
pub fn count_messages(candidate: &Candidate) -> Option<u32> {
    let turns = match candidate.kind {
        Kind::Claude => claude::read(&candidate.path).ok()?,
        Kind::Codex => codex::read(&candidate.path).ok()?,
        _ => return None,
    };
    u32::try_from(turns.len()).ok()
}

/// A prompt's first line, trimmed.
pub(crate) fn title_of(prompt: &str) -> String {
    prompt.lines().next().unwrap_or("").trim().to_string()
}
