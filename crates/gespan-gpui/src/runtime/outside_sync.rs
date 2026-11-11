//! Turns taken in the agent's own CLI (`claude --resume`, `codex resume`),
//! picked up before Gespan sends so the chat shows them and the agent
//! resumes with them. Best effort: nothing here ever fails a turn.
use super::{Database, Session, locked};
use gespan_core::outside_turns::{self, Checkpoint, OutsideTurn, TurnRole};
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
    /// an account whose home can't be found, or a Pilot's Codex, whose
    /// rollouts live under `codex-pilots` rather than the account's home.
    pub(super) fn of(
        store: &Database,
        summary: &Session,
        account: &str,
        pilot: bool,
    ) -> Option<Self> {
        if summary.cwd.starts_with("remote://") {
            return None;
        }
        let kind = Kind::parse(&summary.provider)?;
        if !matches!(kind, Kind::Claude | Kind::Codex) || (pilot && kind == Kind::Codex) {
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
    /// Picked-up turns could not be saved: no checkpoint may move this turn,
    /// so the next send reads them again.
    pub(super) held: bool,
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
    before_turn_with(
        store,
        target,
        turn,
        transcript,
        Effects {
            save_rows: |store: &Database, session, start, rows: &[Row]| {
                locked(store)?.write_rows(session, start, rows)
            },
            forget: provider::forget_session,
            still_writing: provider::agent_still_writing,
        },
    )
}

/// What `before_turn` does outside the session file, passed in so tests can
/// watch it.
pub(super) struct Effects<S, F, W> {
    /// Saves the picked-up rows.
    pub(super) save_rows: S,
    /// Drops the session's warm agent.
    pub(super) forget: F,
    /// Whether Gespan's own agent for the session may still be writing.
    pub(super) still_writing: W,
}

pub(super) fn before_turn_with<S, F, W>(
    store: &Database,
    target: &Target,
    turn: &Turn,
    transcript: &mut Transcript,
    effects: Effects<S, F, W>,
) -> Synced
where
    S: FnOnce(&Database, i64, usize, &[Row]) -> Result<(), String>,
    F: FnOnce(Kind, &str, &str, &Path),
    W: FnOnce(Kind, &str, &str, &Path) -> bool,
{
    let Effects {
        save_rows,
        forget,
        still_writing,
    } = effects;
    let stored = stored_checkpoint(store, turn.session);
    if still_writing(target.kind, turn.provider_id, turn.account, turn.cwd) {
        // Stop left it finishing its turn, or Hold parked it with background
        // work: what it wrote is Gespan's own, and dropping it would end
        // that work.
        if let Some(checkpoint) = outside_turns::checkpoint_at_end(
            target.kind,
            &target.home,
            turn.provider_id,
            stored.as_ref(),
        ) {
            save_checkpoint(store, turn.session, &checkpoint);
        }
        tracing::info!("terminal sync skipped: Gespan's own agent is still writing");
        return Synced::default();
    }
    let own = OwnTurn::last(&transcript.rows);
    let mut sync = outside_turns::sync(
        target.kind,
        &target.home,
        turn.provider_id,
        stored.as_ref(),
        own.prompt.as_deref(),
    );
    if !sync.turns.is_empty() {
        // An agent can flush its last entries for Gespan's own turn after
        // reporting it done, past the checkpoint taken then.
        let picked = sync.turns.len();
        sync.turns = own.without_echoes(std::mem::take(&mut sync.turns));
        if sync.turns.is_empty() {
            tracing::info!(picked, "terminal sync read only Gespan's own last turn");
            sync.grew = false;
        }
    }
    let mut synced = Synced::default();
    if sync.grew {
        // The warm agent's context is stale, and its next write could fork
        // the session file.
        forget(target.kind, turn.provider_id, turn.account, turn.cwd);
        synced.forgot = true;
    }
    if !sync.turns.is_empty() {
        let old = transcript.rows.len();
        outside_turns::append_rows(transcript, &sync.turns);
        if let Err(error) = save_rows(store, turn.session, old, &transcript.rows[old..]) {
            // Keep the old checkpoint, and skip this turn's own, so the next
            // send reads these turns again. That read also holds Gespan's own
            // turn from this send, after them; it is dropped as an echo, but
            // the terminal turns then show after it rather than before.
            tracing::warn!(%error, "could not save terminal turns");
            transcript.rows.truncate(old);
            synced.held = true;
            return synced;
        }
        tracing::info!(turns = sync.turns.len(), "picked up terminal turns");
        synced.picked = Some(old);
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
    let known = stored_checkpoint(store, session);
    if let Some(checkpoint) =
        outside_turns::checkpoint_at_end(target.kind, &target.home, provider_id, known.as_ref())
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

/// Gespan's own latest turn: the prompt and the reply texts it shows.
#[derive(Debug, Default)]
struct OwnTurn {
    prompt: Option<String>,
    replies: Vec<String>,
}

impl OwnTurn {
    fn last(rows: &[Row]) -> Self {
        let Some(start) = rows.iter().rposition(|row| row.role == "user" && row.first) else {
            return Self::default();
        };
        let mut groups: Vec<(&str, String)> = Vec::new();
        for row in &rows[start..] {
            match groups.last_mut() {
                Some((role, text)) if !row.first && *role == row.role => text.push_str(&row.text),
                _ => groups.push((&row.role, row.text.clone())),
            }
        }
        let mut groups = groups.into_iter();
        Self {
            prompt: groups.next().map(|(_, text)| text),
            replies: groups
                .filter(|(role, _)| *role == "assistant")
                .map(|(_, text)| text.trim().to_owned())
                .collect(),
        }
    }

    fn echoes(&self, turn: &OutsideTurn) -> bool {
        let text = turn.text.trim();
        match turn.role {
            TurnRole::User => self.prompt.as_deref().map(str::trim) == Some(text),
            TurnRole::Assistant => self.replies.iter().any(|reply| reply == text),
        }
    }

    /// Drops this turn where it starts or ends `turns`: its prompt, if read,
    /// and the replies after it. A matching prompt that got a different reply
    /// is the user asking again in the terminal, and stays.
    fn without_echoes(&self, mut turns: Vec<OutsideTurn>) -> Vec<OutsideTurn> {
        let lead = self.echo_len(&turns);
        turns.drain(..lead);
        if let Some(last_prompt) = turns.iter().rposition(|t| t.role == TurnRole::User)
            && self.echo_len(&turns[last_prompt..]) == turns.len() - last_prompt
        {
            turns.truncate(last_prompt);
        }
        turns
    }

    /// How many turns at the start of `turns` repeat this one.
    fn echo_len(&self, turns: &[OutsideTurn]) -> usize {
        let prompt = usize::from(
            turns
                .first()
                .is_some_and(|t| t.role == TurnRole::User && self.echoes(t)),
        );
        let replies = turns[prompt..]
            .iter()
            .take_while(|t| t.role == TurnRole::Assistant && self.echoes(t))
            .count();
        let end = prompt + replies;
        let complete = end == turns.len() || turns[end].role == TurnRole::User;
        if prompt == 1 && !complete {
            // Asked again and answered differently: not an echo.
            return 0;
        }
        end
    }
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

    fn said(role: TurnRole, text: &str) -> OutsideTurn {
        OutsideTurn {
            role,
            text: text.into(),
            sent_at: None,
        }
    }

    #[test]
    fn own_turn_joins_continuation_rows_and_collects_every_reply() {
        let rows = [
            row("user", "first", true),
            row("assistant", "old reply", true),
            row("user", "a long ", true),
            row("user", "prompt", false),
            row("attachments", "[]", true),
            row("assistant", "Looking", true),
            row("tool", "{}", true),
            row("assistant", "Done ", true),
            row("assistant", "here", false),
        ];
        let own = OwnTurn::last(&rows);
        assert_eq!(own.prompt.as_deref(), Some("a long prompt"));
        assert_eq!(own.replies, ["Looking", "Done here"]);
        assert!(
            OwnTurn::last(&[row("assistant", "hi", true)])
                .prompt
                .is_none()
        );
    }

    #[test]
    fn echoes_of_the_own_turn_are_dropped_at_either_end_only() {
        use TurnRole::{Assistant, User};
        let own = OwnTurn {
            prompt: Some("Fix it".into()),
            replies: vec!["Done".into()],
        };
        let terminal = [said(User, "ran it"), said(Assistant, "ok")];
        // Flushed late: before the terminal turn.
        let turns = [
            &[said(User, " Fix it "), said(Assistant, "Done")][..],
            &terminal,
        ]
        .concat();
        assert_eq!(own.without_echoes(turns), terminal);
        // Only the reply flushed late.
        let turns = [&[said(Assistant, "Done")][..], &terminal].concat();
        assert_eq!(own.without_echoes(turns), terminal);
        // Read again after it, when a failed save held the checkpoint back.
        let turns = [
            &terminal[..],
            &[said(User, "Fix it"), said(Assistant, "Done")],
        ]
        .concat();
        assert_eq!(own.without_echoes(turns), terminal);
        // Nothing but the echo.
        assert!(own.without_echoes(vec![said(User, "Fix it")]).is_empty());
        // Asked again in the terminal and answered differently: kept.
        let again = vec![
            said(User, "Fix it"),
            said(Assistant, "Fixed it differently"),
        ];
        assert_eq!(own.without_echoes(again.clone()), again);
        // A repeat in the middle is the user's own.
        let middle = [
            &terminal[..],
            &[said(User, "Fix it"), said(Assistant, "Done")],
            &terminal,
        ]
        .concat();
        assert_eq!(own.without_echoes(middle.clone()), middle);
    }
}
