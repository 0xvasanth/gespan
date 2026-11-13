//! The session-start block: what a session of a harness that reads no
//! decision file is told about the log, once, and again only after it changes.
//! And, with search by meaning on, each turn's related block.
use super::entry::{Entry, Kind as EntryKind, Scope};
use super::render::{render_block, render_ranked, START_BUDGET, TURN_BUDGET};
use super::scope::cached_scope;
use super::service;
use crate::diagnostics::metrics::{self, ErrorKind};
use crate::provider::Kind;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

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
    let scope = cached_scope(workspace);
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

/// Counted each time a turn is sent without its related block because the
/// lookup was not ready in time (or could not start).
const SKIPPED: &str = "decisions.related.skipped";

/// At most this many per-turn lookups run at once in the process. A lookup
/// still running when its turn's deadline passed keeps its slot until it
/// ends, so a stuck embedder holds these few threads and no more: later
/// turns go without the block instead of starting another.
pub const MAX_RELATED_WORKERS: usize = 2;

/// The most entries a related block lists before the budget trims it.
const RELATED_LIMIT: usize = 8;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// A slot among [`MAX_RELATED_WORKERS`], given back when dropped.
struct Slot;

impl Slot {
    fn take() -> Option<Slot> {
        // A compare-exchange loop rather than `fetch_update`, which newer
        // toolchains deprecate under a name older ones don't have.
        let mut n = IN_FLIGHT.load(Ordering::SeqCst);
        while n < MAX_RELATED_WORKERS {
            match IN_FLIGHT.compare_exchange_weak(n, n + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Slot),
                Err(now) => n = now,
            }
        }
        None
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Per-turn lookups running now, finished turns' late ones included.
pub fn related_in_flight() -> usize {
    IN_FLIGHT.load(Ordering::SeqCst)
}

/// The decisions related to `user_message`, for the turn about to be sent,
/// with search by meaning on. `None` when the log is off, search by meaning
/// is off, nothing related is left once what the session start already
/// gave (preferences and pinned decisions) is taken out, or the lookup is
/// not ready within `deadline`. When the turn goes without it for want of
/// time or room, `decisions.related.skipped` is counted, by kind:
/// - `Other`: the cap is full ([`MAX_RELATED_WORKERS`] lookups running);
/// - `Timeout`: the lookup was past the deadline;
/// - `Panic`: the worker panicked;
/// - `SpawnFailed`: the worker thread could not start.
///
/// Nothing related is not a skip and is not counted. The block opens with
/// [`RELATED_HEADING`](super::render::RELATED_HEADING), not the standing
/// instruction, and lists entries in search order
/// ([`render_ranked`](super::render::render_ranked)).
///
/// `scope` is the session's project scope, worked out once per turn by the
/// caller ([`cached_scope`](super::scope::cached_scope)). The lookup (hybrid
/// search with its relevance cut-off) runs on its own thread; this waits for
/// it at most `deadline`, so the turn is never held longer.
pub fn related_context(scope: &Scope, user_message: &str, deadline: Duration) -> Option<String> {
    if !service::enabled() || !matches!(service::with(|s| s.embedder().is_some()), Ok(true)) {
        return None;
    }
    let Some(slot) = Slot::take() else {
        metrics::failure(SKIPPED, ErrorKind::Other);
        return None;
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let scope = scope.clone();
    let message = user_message.to_owned();
    let spawned = std::thread::Builder::new()
        .name("decisions-related".into())
        .spawn(move || {
            let _slot = slot;
            let _ = tx.send(related_block(&scope, &message));
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "the related decisions lookup could not start");
        metrics::failure(SKIPPED, ErrorKind::SpawnFailed);
        return None;
    }
    match rx.recv_timeout(deadline) {
        Ok(block) => block,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            metrics::failure(SKIPPED, ErrorKind::Timeout);
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            metrics::failure(SKIPPED, ErrorKind::Panic);
            None
        }
    }
}

/// The lookup itself, on the worker thread.
fn related_block(scope: &Scope, message: &str) -> Option<String> {
    let found = service::with(|store| {
        if store.embedder().is_none() {
            return Ok(Vec::new());
        }
        store.search(scope, message, RELATED_LIMIT)
    });
    let entries = match found {
        Ok(Ok(entries)) => entries,
        Ok(Err(error)) => {
            tracing::warn!(%error, "the decision log could not be searched for this turn");
            return None;
        }
        Err(_) => return None,
    };
    // Preferences and pinned decisions are in the session-start block (or
    // the project file) already.
    let related: Vec<Entry> = entries
        .into_iter()
        .filter(|e| e.kind != EntryKind::Preference && !e.pinned)
        .collect();
    // Search order is relevance order: kept, so the budget trims the weakest.
    (!related.is_empty()).then(|| render_ranked(&related, TURN_BUDGET).text)
}
