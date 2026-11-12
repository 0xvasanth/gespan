//! The user's review of the log, from My tasks: approving or rejecting a
//! proposal, and answering an agent's flag on an approved entry.
use super::entry::{Edit, Entry, NewEntry, ShownHash, Source, Status};
use super::service;
use super::store::DecisionStore;

/// What the user decided about one entry.
#[derive(Clone, Debug)]
pub enum Review {
    /// Approve a proposal as `shown`, optionally reworded and pinned
    /// ("Always include"), replacing the active entries the user ticked.
    Approve {
        shown: ShownHash,
        edit: Option<Edit>,
        pinned: bool,
        replaces: Vec<i64>,
    },
    /// Reject a proposal as `shown`.
    Reject { shown: ShownHash },
    /// Answer a flag on an active entry with new wording that replaces it.
    ReplaceFromFlag { flag: i64, new: Edit },
    /// Keep the entry as it is and close the flag.
    DismissFlag { flag: i64 },
}

/// The fingerprint of what a card shows of `entry`: its title, text, why
/// and user answer.
pub fn shown_hash(entry: &Entry) -> ShownHash {
    ShownHash::of(
        &entry.title,
        &entry.text,
        &entry.why,
        entry.user_answer.as_deref(),
    )
}

/// How many keyword matches are read before keeping the same-scope active
/// ones: search also returns global and pending entries.
const SEARCH_WINDOW: usize = 50;

/// Active entries in `entry`'s own scope (never global or another
/// project's) that read like it, best match first, at most `limit`: what
/// approving it might replace. Keyword search over title and text for now;
/// Task 10 swaps in vectors.
pub fn similar_active(store: &DecisionStore, entry: &Entry, limit: usize) -> Vec<Entry> {
    let query = format!("{} {}", entry.title, entry.text);
    let found = store
        .search(&entry.scope, &query, SEARCH_WINDOW)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "the decision log could not search for similar entries");
            Vec::new()
        });
    found
        .into_iter()
        .filter(|e| e.id != entry.id && e.status == Status::Active && e.scope == entry.scope)
        .take(limit)
        .collect()
}

/// Applies the user's review of entry `id`, then has the harness files
/// rewritten (through the hook the app registers). Nothing is rewritten when
/// the review fails.
pub fn apply(store: &DecisionStore, id: i64, review: Review, now_ms: i64) -> Result<(), String> {
    match review {
        Review::Approve {
            shown,
            edit,
            pinned,
            replaces,
        } => {
            store
                .approve_if_unchanged(id, &shown, edit, pinned, &replaces, now_ms)
                .map_err(|e| e.to_string())?;
        }
        Review::Reject { shown } => store
            .reject_if_unchanged(id, &shown, now_ms)
            .map_err(|e| e.to_string())?,
        Review::ReplaceFromFlag { flag, new } => replace_from_flag(store, id, flag, new, now_ms)?,
        Review::DismissFlag { flag } => {
            open_flag_on(store, id, flag)?;
            store.dismiss_flag(flag).map_err(|e| e.to_string())?;
        }
    }
    service::regenerate();
    Ok(())
}

/// The entry `flag` is on, when the flag is open and on entry `id`.
fn open_flag_on(store: &DecisionStore, id: i64, flag: i64) -> Result<Entry, String> {
    store
        .open_flags()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(f, _)| f.id == flag && f.entry == id)
        .map(|(_, entry)| entry)
        .ok_or_else(|| format!("Flag {flag} isn't open on entry {id}."))
}

/// Writes the user's replacement as a new entry of the flagged one's kind,
/// scope and pin, approves it in the flagged one's place, then closes the
/// flag. Its wording is cleaned like any entry's.
fn replace_from_flag(
    store: &DecisionStore,
    id: i64,
    flag: i64,
    new: Edit,
    now_ms: i64,
) -> Result<(), String> {
    let old = open_flag_on(store, id, flag)?;
    if old.status != Status::Active {
        return Err(format!(
            "{} isn't approved, so it can't be replaced.",
            old.label()
        ));
    }
    let successor = store
        .propose(
            NewEntry {
                kind: old.kind,
                scope: old.scope.clone(),
                title: new.title,
                text: new.text,
                why: new.why,
                user_answer: None,
                source: Source::default(),
            },
            now_ms,
        )
        .map_err(|e| e.to_string())?;
    if let Err(error) = store.approve(successor.id, None, old.pinned, &[id], now_ms) {
        // Leave nothing half-done waiting for review.
        if let Err(cleanup) = store.reject(successor.id, now_ms) {
            tracing::warn!(%cleanup, "a replacement that failed was left pending");
        }
        return Err(error.to_string());
    }
    store.dismiss_flag(flag).map_err(|e| e.to_string())
}
