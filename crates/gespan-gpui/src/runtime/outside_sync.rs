//! Turns taken in the agent's own CLI (`claude --resume`, `codex resume`),
//! picked up before Gespan sends so the chat shows them and the agent
//! resumes with them. Best effort: nothing here ever fails a turn.
use super::{Database, Session, locked};
use gespan_core::outside_turns::{self, Checkpoint};
use gespan_core::provider::{self, Kind};
use gespan_core::transcript::{Row, Transcript};
use std::path::{Path, PathBuf};

/// The agent file a session syncs against: its CLI and the account's home.
pub(super) struct Target {
    pub(super) kind: Kind,
    pub(super) home: PathBuf,
}

impl Target {
    /// `None` for remote projects, agents without a readable session file,
    /// or an account whose home can't be found.
    pub(super) fn of(store: &Database, summary: &Session, account: &str) -> Option<Self> {
        if summary.cwd.starts_with("remote://") {
            return None;
        }
        let kind = Kind::parse(&summary.provider)?;
        if !matches!(kind, Kind::Claude | Kind::Codex) {
            return None;
        }
        let data = match locked(store) {
            Ok(store) => store.data_dir().map(Path::to_path_buf),
            Err(error) => {
                tracing::warn!(%error, "terminal sync skipped: store unavailable");
                return None;
            }
        };
        let profile = match &data {
            Some(data) => gespan_core::accounts::Profile::new(account, data),
            None if account == gespan_core::accounts::DEFAULT => Default::default(),
            None => return None,
        };
        match profile.home(kind) {
            Ok(home) => Some(Self { kind, home }),
            Err(error) => {
                tracing::warn!(%error, "terminal sync skipped: no agent home");
                None
            }
        }
    }
}

/// What a pre-turn sync did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Synced {
    /// The first picked-up row, when terminal turns were appended.
    pub(super) picked: Option<usize>,
    /// The warm agent was dropped because the file grew from outside.
    pub(super) forgot: bool,
}

/// The session about to send, as its warm agent is keyed.
pub(super) struct Turn<'a> {
    pub(super) session: i64,
    pub(super) provider_id: &'a str,
    pub(super) account: &'a str,
    pub(super) cwd: &'a Path,
}

/// Appends what the agent's CLI added since the last checkpoint to
/// `transcript` (and the store), behind one "From terminal" marker.
pub(super) fn before_turn(
    store: &Database,
    target: &Target,
    turn: &Turn,
    transcript: &mut Transcript,
) -> Synced {
    let stored = stored_checkpoint(store, turn.session);
    let last_prompt = last_prompt(&transcript.rows);
    let sync = outside_turns::sync(
        target.kind,
        &target.home,
        turn.provider_id,
        stored.as_ref(),
        last_prompt.as_deref(),
    );
    let mut synced = Synced::default();
    if !sync.turns.is_empty() {
        let old = transcript.rows.len();
        outside_turns::append_rows(transcript, &sync.turns);
        let saved = locked(store)
            .and_then(|mut store| store.write_rows(turn.session, old, &transcript.rows[old..]));
        if let Err(error) = saved {
            // Unsaved, they are left for the next send to pick up again.
            tracing::warn!(%error, "could not save terminal turns");
            transcript.rows.truncate(old);
            return synced;
        }
        tracing::info!(turns = sync.turns.len(), "picked up terminal turns");
        synced.picked = Some(old);
    }
    if sync.grew {
        // The warm agent's context is stale, and its next write could fork
        // the session file.
        provider::forget_session(target.kind, turn.provider_id, turn.account, turn.cwd);
        synced.forgot = true;
    }
    // No checkpoint means no file or a failed read: keep what we had.
    if let Some(checkpoint) = &sync.checkpoint {
        save_checkpoint(store, turn.session, checkpoint);
    }
    synced
}

/// Moves the checkpoint past the turn Gespan just ran, so it is never read
/// back as outside.
pub(super) fn after_turn(store: &Database, target: &Target, session: i64, provider_id: &str) {
    if let Some(checkpoint) =
        outside_turns::checkpoint_at_end(target.kind, &target.home, provider_id)
    {
        save_checkpoint(store, session, &checkpoint);
    }
}

fn stored_checkpoint(store: &Database, session: i64) -> Option<Checkpoint> {
    let value = locked(store)
        .and_then(|store| store.setting(&outside_turns::checkpoint_key(session)))
        .inspect_err(|error| tracing::warn!(%error, "could not read the sync checkpoint"))
        .ok()??;
    serde_json::from_str(&value)
        .inspect_err(|error| tracing::warn!(%error, "ignoring an unreadable sync checkpoint"))
        .ok()
}

fn save_checkpoint(store: &Database, session: i64, checkpoint: &Checkpoint) {
    let saved = serde_json::to_string(checkpoint)
        .map_err(|e| e.to_string())
        .and_then(|value| {
            locked(store)?.save_setting(&outside_turns::checkpoint_key(session), &value)
        });
    if let Err(error) = saved {
        tracing::warn!(%error, "could not save the sync checkpoint");
    }
}

/// The text of the last user prompt, across its continuation rows.
fn last_prompt(rows: &[Row]) -> Option<String> {
    let start = rows
        .iter()
        .rposition(|row| row.role == "user" && row.first)?;
    Some(
        rows[start..]
            .iter()
            .enumerate()
            .take_while(|(ix, row)| row.role == "user" && (*ix == 0 || !row.first))
            .map(|(_, row)| row.text.as_str())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, text: &str, first: bool) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first,
            sent_at: None,
        }
    }

    #[test]
    fn last_prompt_joins_continuation_rows_of_the_last_user_message() {
        let rows = [
            row("user", "first", true),
            row("assistant", "reply", true),
            row("user", "a long ", true),
            row("user", "prompt", false),
            row("attachments", "[]", true),
            row("assistant", "done", true),
        ];
        assert_eq!(last_prompt(&rows).as_deref(), Some("a long prompt"));
    }

    #[test]
    fn last_prompt_stops_at_the_next_message() {
        let rows = [row("user", "one", true), row("user", "two", true)];
        assert_eq!(last_prompt(&rows).as_deref(), Some("two"));
        assert_eq!(last_prompt(&[row("assistant", "hi", true)]), None);
    }
}
