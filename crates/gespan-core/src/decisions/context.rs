//! The session-start block: what a session of a harness that reads no
//! decision file is told about the log, once, and again only after it changes.
use super::scope::scope_for;
use super::service;
use crate::provider::Kind;
use gespan_memory::{render_block, Entry, Kind as EntryKind, START_BUDGET};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Whether this harness loads the project decision file on its own, so its
/// sessions get the log from the file instead of an injected block.
pub fn reads_decision_file(kind: Kind) -> bool {
    matches!(kind, Kind::Claude | Kind::Cursor | Kind::Kiro)
}

/// The block to inject into this session now, with its hash: `None` when the
/// log is off or closed, the harness reads the decision file itself, or
/// `already` (the hash stored when the block was last delivered) matches.
pub fn start_context(
    kind: Kind,
    workspace: &Path,
    already: Option<&str>,
) -> Option<(String, String)> {
    if !service::enabled() || reads_decision_file(kind) {
        return None;
    }
    let scope = scope_for(workspace);
    let entries = match service::with(|store| store.visible(&scope)) {
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
