//! The orchestration core's tables: who started whom, the receipts that make
//! a repeated `sessions_start` harmless, the event log, and the outbox of
//! effects still to apply, all of which survive a restart.
use super::Store;
use crate::orchestrator::tree::{Link, Origin, Tree, Wake};
use serde_json::Value;

/// One child and the session that started it.
#[derive(Debug, toasty::Model)]
#[table = "session_links"]
#[index(name = "session_links_parent_idx", parent_session_id)]
pub(crate) struct SessionLink {
    #[key]
    child_session_id: String,
    parent_session_id: String,
    root_session_id: String,
    depth: i64,
    origin: String,
    wake: String,
    created_at: i64,
}

/// The answer to one command, kept so the same request returns it again.
#[derive(Debug, toasty::Model)]
#[table = "orch_commands"]
#[index(name = "orch_commands_caller_idx", caller_session_id)]
pub(crate) struct OrchCommand {
    #[key]
    command_id: String,
    caller_session_id: String,
    kind: String,
    result_json: String,
    created_at: i64,
}

/// What happened, in order. Append-only.
#[derive(Debug, toasty::Model)]
#[table = "orch_events"]
#[index(name = "orch_events_session_idx", session_id)]
pub(crate) struct OrchEvent {
    #[key]
    #[auto]
    seq: i64,
    at: i64,
    kind: String,
    session_id: String,
    payload_json: String,
    command_id: Option<String>,
}

/// An effect still to apply to a live session, retried after a restart.
#[derive(Debug, toasty::Model)]
#[table = "orch_effects"]
#[index(name = "orch_effects_state_idx", state)]
pub(crate) struct OrchEffect {
    #[key]
    effect_id: String,
    kind: String,
    session_id: String,
    payload_json: String,
    state: String,
    attempts: i64,
    last_error: Option<String>,
    created_at: i64,
}

/// Run on every open (see `Kind::constraints`): the foreign keys and cascades
/// Toasty's DDL cannot express.
pub(crate) const CONSTRAINTS: &[&str] = &[
    // session_links.child_session_id and parent_session_id REFERENCES sessions(id).
    "CREATE TRIGGER IF NOT EXISTS session_links_sessions_insert BEFORE INSERT ON session_links
       WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.child_session_id)
         OR NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.parent_session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    "CREATE TRIGGER IF NOT EXISTS session_links_sessions_update
       BEFORE UPDATE OF child_session_id, parent_session_id ON session_links
       WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.child_session_id)
         OR NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.parent_session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    // session_links.depth: CHECK (depth BETWEEN 1 AND 2).
    "CREATE TRIGGER IF NOT EXISTS session_links_depth_insert BEFORE INSERT ON session_links
       WHEN NEW.depth < 1 OR NEW.depth > 2
       BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: depth between 1 and 2'); END",
    "CREATE TRIGGER IF NOT EXISTS session_links_depth_update
       BEFORE UPDATE OF depth ON session_links WHEN NEW.depth < 1 OR NEW.depth > 2
       BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: depth between 1 and 2'); END",
    // orch_effects.session_id REFERENCES sessions(id).
    "CREATE TRIGGER IF NOT EXISTS orch_effects_session_insert BEFORE INSERT ON orch_effects
       WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    // ON DELETE CASCADE. `Store::delete` re-roots a deleted parent's
    // children first; this only catches what deletes a session another way.
    "CREATE TRIGGER IF NOT EXISTS orchestrator_sessions_delete_cascade AFTER DELETE ON sessions
       BEGIN
         DELETE FROM session_links WHERE child_session_id = OLD.id OR parent_session_id = OLD.id;
         DELETE FROM orch_effects WHERE session_id = OLD.id;
         DELETE FROM orch_commands WHERE caller_session_id = OLD.id;
       END",
];

/// An effect waiting in the outbox.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingEffect {
    pub id: String,
    pub kind: String,
    pub session: i64,
    pub payload: Value,
    pub attempts: i64,
}

fn now() -> i64 {
    crate::time::now_ms().min(i64::MAX as u64) as i64
}

impl Store {
    /// Every link whose sessions still exist here.
    pub fn session_links(&self) -> Result<Tree, String> {
        let rows = self
            .orm()?
            .run(|mut db| async move { SessionLink::all().exec(&mut db).await })?;
        Ok(Tree::new(rows.into_iter().filter_map(|row| {
            Some(Link {
                child: self.resolve_existing_session_id(&row.child_session_id)?,
                parent: self.resolve_existing_session_id(&row.parent_session_id)?,
                root: self.resolve_existing_session_id(&row.root_session_id)?,
                depth: u8::try_from(row.depth).ok()?,
                origin: Origin::from_id(&row.origin)?,
                wake: Wake::from_id(&row.wake)?,
            })
        })))
    }

    /// Records who started `link.child`. Writing it again replaces it.
    pub fn save_session_link(&self, link: &Link) -> Result<(), String> {
        self.shared_writable()?;
        let upsert =
            SessionLink::upsert_by_child_session_id(self.persisted_session_id(link.child)?)
                .parent_session_id(self.persisted_session_id(link.parent)?)
                .root_session_id(self.persisted_session_id(link.root)?)
                .depth(i64::from(link.depth))
                .origin(link.origin.id())
                .wake(link.wake.id())
                .created_at(now());
        self.orm()?
            .run(|mut db| async move { upsert.exec(&mut db).await.map(drop) })
    }

    /// Before `id` is deleted: its link goes, and its children become roots
    /// with everything below them re-rooted.
    pub(super) fn unlink_session(&self, id: i64) -> Result<(), String> {
        let mut tree = self.session_links()?;
        let removal = tree.remove(id);
        let removed = removal
            .removed
            .iter()
            .map(|child| self.persisted_session_id(*child))
            .collect::<Result<Vec<_>, _>>()?;
        let updated = removal
            .updated
            .iter()
            .map(|link| {
                Ok::<_, String>((
                    self.persisted_session_id(link.child)?,
                    self.persisted_session_id(link.root)?,
                    i64::from(link.depth),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let original = self.persisted_session_id(id)?;
        self.orm()?.run(|mut db| async move {
            for child in removed {
                SessionLink::filter_by_child_session_id(child)
                    .delete()
                    .exec(&mut db)
                    .await?;
            }
            for (child, root, depth) in updated {
                SessionLink::filter_by_child_session_id(child)
                    .update()
                    .root_session_id(root)
                    .depth(depth)
                    .exec(&mut db)
                    .await?;
            }
            OrchEffect::filter(OrchEffect::fields().session_id().eq(&original))
                .delete()
                .exec(&mut db)
                .await?;
            OrchCommand::filter(OrchCommand::fields().caller_session_id().eq(&original))
                .delete()
                .exec(&mut db)
                .await
        })
    }

    /// The saved answer to `command_id`, if it ran before.
    pub fn command_receipt(&self, command_id: &str) -> Result<Option<Value>, String> {
        let id = command_id.to_owned();
        let row = self.orm()?.run(|mut db| async move {
            OrchCommand::filter_by_command_id(id)
                .first()
                .exec(&mut db)
                .await
        })?;
        Ok(row.and_then(|row| serde_json::from_str(&row.result_json).ok()))
    }

    /// Saves a command's answer and appends its event, in one transaction.
    /// Returns the answer already saved when the command ran before, which
    /// the caller must return instead of its own.
    pub fn save_command_receipt(
        &self,
        command_id: &str,
        caller: i64,
        kind: &str,
        session: i64,
        result: &Value,
    ) -> Result<Option<Value>, String> {
        self.shared_writable()?;
        let (id, caller, session) = (
            command_id.to_owned(),
            self.persisted_session_id(caller)?,
            self.persisted_session_id(session)?,
        );
        let (kind, json) = (kind.to_owned(), result.to_string());
        let earlier = self.orm()?.transaction(move |tx| {
            Box::pin(async move {
                if let Some(row) = OrchCommand::filter_by_command_id(&id)
                    .first()
                    .exec(&mut *tx)
                    .await?
                {
                    return Ok(Some(row.result_json));
                }
                let at = now();
                OrchCommand::create()
                    .command_id(&id)
                    .caller_session_id(&caller)
                    .kind(&kind)
                    .result_json(&json)
                    .created_at(at)
                    .exec(&mut *tx)
                    .await?;
                OrchEvent::create()
                    .at(at)
                    .kind(&kind)
                    .session_id(&session)
                    .payload_json(&json)
                    .command_id(Some(id.clone()))
                    .exec(&mut *tx)
                    .await?;
                Ok(None)
            })
        })?;
        Ok(earlier.and_then(|json| serde_json::from_str(&json).ok()))
    }

    /// Appends one event to the log.
    pub fn append_orch_event(
        &self,
        kind: &str,
        session: i64,
        payload: &Value,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let create = OrchEvent::create()
            .at(now())
            .kind(kind)
            .session_id(self.persisted_session_id(session)?)
            .payload_json(payload.to_string())
            .command_id(None::<String>);
        self.orm()?
            .run(|mut db| async move { create.exec(&mut db).await.map(drop) })
    }

    /// `session`'s events, oldest first: `(kind, payload)`.
    pub fn orch_events(&self, session: i64) -> Result<Vec<(String, Value)>, String> {
        let id = self.persisted_session_id(session)?;
        let rows = self.orm()?.run(|mut db| async move {
            OrchEvent::filter(OrchEvent::fields().session_id().eq(id))
                .order_by(OrchEvent::fields().seq().asc())
                .exec(&mut db)
                .await
        })?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let payload = serde_json::from_str(&row.payload_json).unwrap_or(Value::Null);
                (row.kind, payload)
            })
            .collect())
    }

    /// Adds an effect to the outbox. `false` when `effect_id` is already
    /// there, so applying the same effect twice is impossible.
    pub fn queue_orch_effect(
        &self,
        effect_id: &str,
        kind: &str,
        session: i64,
        payload: &Value,
    ) -> Result<bool, String> {
        self.shared_writable()?;
        let (id, kind, json) = (effect_id.to_owned(), kind.to_owned(), payload.to_string());
        let session = self.persisted_session_id(session)?;
        self.orm()?.transaction(move |tx| {
            Box::pin(async move {
                if OrchEffect::filter_by_effect_id(&id)
                    .first()
                    .exec(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Ok(false);
                }
                OrchEffect::create()
                    .effect_id(&id)
                    .kind(&kind)
                    .session_id(&session)
                    .payload_json(&json)
                    .state("pending")
                    .attempts(0)
                    .last_error(None::<String>)
                    .created_at(now())
                    .exec(&mut *tx)
                    .await?;
                Ok(true)
            })
        })
    }

    /// Effects not yet applied, oldest first, optionally of one kind.
    pub fn pending_orch_effects(&self, kind: Option<&str>) -> Result<Vec<PendingEffect>, String> {
        let rows = self.orm()?.run(|mut db| async move {
            OrchEffect::filter(OrchEffect::fields().state().eq("pending"))
                .order_by(OrchEffect::fields().created_at().asc())
                .exec(&mut db)
                .await
        })?;
        Ok(rows
            .into_iter()
            .filter(|row| kind.is_none_or(|kind| row.kind == kind))
            .filter_map(|row| {
                Some(PendingEffect {
                    session: self.resolve_existing_session_id(&row.session_id)?,
                    payload: serde_json::from_str(&row.payload_json).ok()?,
                    id: row.effect_id,
                    kind: row.kind,
                    attempts: row.attempts,
                })
            })
            .collect())
    }

    /// Marks an effect applied, or counts a failed attempt. After
    /// `MAX_ATTEMPTS` failures it stops being retried.
    pub fn finish_orch_effect(&self, effect_id: &str, error: Option<&str>) -> Result<(), String> {
        const MAX_ATTEMPTS: i64 = 5;
        self.shared_writable()?;
        let (id, error) = (effect_id.to_owned(), error.map(str::to_owned));
        self.orm()?.transaction(move |tx| {
            Box::pin(async move {
                let Some(row) = OrchEffect::filter_by_effect_id(&id)
                    .first()
                    .exec(&mut *tx)
                    .await?
                else {
                    return Ok(());
                };
                let attempts = row.attempts + 1;
                let state = match &error {
                    None => "done",
                    Some(_) if attempts >= MAX_ATTEMPTS => "failed",
                    Some(_) => "pending",
                };
                OrchEffect::filter_by_effect_id(&id)
                    .update()
                    .state(state)
                    .attempts(attempts)
                    .last_error(error)
                    .exec(&mut *tx)
                    .await
            })
        })
    }
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod tests;
