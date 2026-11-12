//! The session-start block: what a session of a harness that reads no
//! decision file is told about the log, once, and again only after it changes.
use super::entry::{Entry, Kind as EntryKind};
use super::render::{render_block, START_BUDGET};
use super::scope::scope_for;
use super::service;
use crate::provider::Kind;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Whether this harness loads the project decision file on its own, so its
/// sessions get the log from the file instead of an injected block.
pub fn reads_decision_file(kind: Kind) -> bool {
    matches!(kind, Kind::Claude | Kind::Cursor | Kind::Kiro)
}

/// The block to inject into this session now, with its hash: `None` when the
/// log is off or closed, the harness reads the decision file itself and
/// Gespan's project file is in place, or `already` (the hash stored when the
/// block was last delivered) matches. With search by meaning on, the block
/// holds preferences and pinned decisions only. A checkout Gespan has not written yet
/// (a new worktree) gets the block, so no session goes without the log.
pub fn start_context(
    kind: Kind,
    workspace: &Path,
    already: Option<&str>,
) -> Option<(String, String)> {
    if !service::enabled()
        || (reads_decision_file(kind) && super::harness::project_file_ready(kind, workspace))
    {
        return None;
    }
    let scope = scope_for(workspace);
    // With search by meaning on, the start block holds the preferences and
    // the pinned decisions; the rest come per turn or through search.
    let entries = match service::with(|store| {
        let by_meaning = store.embedder().is_some();
        store.visible(&scope).map(|entries| {
            entries
                .into_iter()
                .filter(|e| !by_meaning || e.kind == EntryKind::Preference || e.pinned)
                .collect::<Vec<_>>()
        })
    }) {
        Ok(Ok(entries)) => entries,
        Ok(Err(error)) => {
            tracing::warn!(%error, "the decision log could not be read for the session start");
            return None;
        }
        Err(_) => return None,
    };
    let (prefs, decisions): (Vec<Entry>, Vec<Entry>) = entries
        .into_iter()
        .partition(|entry| entry.kind == EntryKind::Preference);
    let block = render_block(&prefs, &decisions, START_BUDGET).text;
    let hash: String = Sha256::digest(block.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (already != Some(hash.as_str())).then_some((block, hash))
}
