//! The desktop schema's sessions as native ones: no imported sessions,
//! replacement IDs, native_* tables, or history conversion on open. The
//! tables are Toasty models (`store::schema`); statements run through
//! `store::db` on the Toasty database.
mod accounts;
pub(super) mod blocks;
pub(crate) mod claude_shell;
mod edit;
mod settings;
#[cfg(test)]
mod tests;
mod window;
pub use window::{HistoryPage, PAGE_TURNS};

use super::db::{args, Sql, SqlRow};
use super::schema::{
    InFlightSession, OrchestrationRun, OrchestrationSidebar, OrchestrationWorker, PilotBlock,
    PilotTranscript, PilotTurn, SessionRecord, SessionReminder, WorkspaceSnapshot,
};
use super::{Reminder, Session, Store};
use crate::{
    agent::Settings,
    context::Usage,
    transcript::{Row, Transcript},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};

pub(super) struct Shared {
    ids: RefCell<HashMap<i64, String>>,
    extras: RefCell<BTreeMap<String, String>>,
    /// The Settings database; `None` when read-only or in memory.
    extras_db: Option<crate::orm::Database>,
    snapshots: RefCell<HashMap<i64, HistoryRevision>>,
    /// The first block sequence of each indexed Pilot's row numbering.
    windows: RefCell<HashMap<i64, i64>>,
    queues: RefCell<HashMap<i64, String>>,
    account_source: RefCell<Option<crate::accounts::Book>>,
    preferences: std::sync::Arc<crate::desktop_preferences::DesktopPreferences>,
    pub(super) read_only: bool,
}

/// A revision guard, rather than another owned copy of every rich source block.
/// Keep this small guard even when a view releases its history: pending writes
/// must still refuse a source edit made by another window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HistoryRevision([u8; 32]);
impl HistoryRevision {
    fn of(blocks: &[Value]) -> Self {
        let mut hash = Self::start(blocks.len());
        for block in blocks {
            Self::push(&mut hash, block);
        }
        Self(hash.finalize().into())
    }
    fn start(len: usize) -> Sha256 {
        let mut hash = Sha256::new();
        hash.update((len as u64).to_le_bytes());
        hash
    }
    fn push(hash: &mut Sha256, block: &Value) {
        fn size(hash: &mut Sha256, len: usize) {
            hash.update((len as u64).to_le_bytes());
        }
        fn value(hash: &mut Sha256, item: &Value) {
            match item {
                Value::Null => hash.update([0]),
                Value::Bool(boolean) => hash.update([1, u8::from(*boolean)]),
                Value::Number(number) => {
                    hash.update([2]);
                    // serde_json treats floating negative zero as equal to
                    // positive zero, while retaining its distinction from an integer.
                    let number = if number.is_f64() && number.as_f64() == Some(0.0) {
                        "0.0".into()
                    } else {
                        number.to_string()
                    };
                    size(hash, number.len());
                    hash.update(number.as_bytes());
                }
                Value::String(text) => {
                    hash.update([3]);
                    size(hash, text.len());
                    hash.update(text.as_bytes());
                }
                Value::Array(values) => {
                    hash.update([4]);
                    size(hash, values.len());
                    for item in values {
                        value(hash, item);
                    }
                }
                Value::Object(fields) => {
                    hash.update([5]);
                    size(hash, fields.len());
                    // Be canonical even if another crate enables preserve_order.
                    let mut fields: Vec<_> = fields.iter().collect();
                    fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
                    for (key, item) in fields {
                        size(hash, key.len());
                        hash.update(key.as_bytes());
                        value(hash, item);
                    }
                }
            }
        }
        value(hash, block);
    }
    fn matches(&self, blocks: &[Value]) -> bool {
        *self == Self::of(blocks)
    }
}

/// The one row a single-row read found; none is an error, as it was for the
/// raw read this replaces.
fn one<T>(rows: Vec<T>) -> Result<T, String> {
    rows.into_iter()
        .next()
        .ok_or_else(|| "Query returned no rows".to_owned())
}
/// A session column's value, converted as the raw update converted it:
/// booleans are flags, numbers integers.
struct Column<'a> {
    name: &'a str,
    value: &'a Value,
}
impl Column<'_> {
    fn text(&self) -> Result<String, String> {
        match self.value {
            Value::Null => Err(format!("{} cannot be empty", self.name)),
            Value::String(text) => Ok(text.clone()),
            Value::Bool(flag) => Ok(i64::from(*flag).to_string()),
            Value::Number(number) => Ok(number.as_i64().unwrap_or(i64::MAX).to_string()),
            other => Ok(other.to_string()),
        }
    }
    fn optional_text(&self) -> Result<Option<String>, String> {
        match self.value {
            Value::Null => Ok(None),
            _ => self.text().map(Some),
        }
    }
    fn integer(&self) -> Result<i64, String> {
        match self.value {
            Value::Bool(flag) => Ok(i64::from(*flag)),
            Value::Number(number) => Ok(number.as_i64().unwrap_or(i64::MAX)),
            _ => Err(format!("{} must be a number", self.name)),
        }
    }
    fn optional_integer(&self) -> Result<Option<i64>, String> {
        match self.value {
            Value::Null => Ok(None),
            _ => self.integer().map(Some),
        }
    }
    fn flag(&self) -> Result<bool, String> {
        self.integer().map(|number| number != 0)
    }
}
fn parse<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}
/// Desktop app CLI requests name children `app-<caller>-<request>`.
/// Both parts can contain hyphens, so the longest existing caller wins.
fn started_caller<'a>(id: &'a str, ids: &HashSet<String>) -> Option<&'a str> {
    let rest = id.strip_prefix("app-")?;
    rest.match_indices('-')
        .rev()
        .map(|(index, _)| &rest[..index])
        .find(|candidate| ids.contains(*candidate))
}
pub(super) fn encode<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}
pub(crate) fn handle(id: &str) -> i64 {
    let bytes = Sha256::digest(id.as_bytes());
    (i64::from_le_bytes(bytes[..8].try_into().unwrap()) & ((1_i64 << 53) - 1)).max(1)
}
/// The columns `shared_session` reads, in its order: twelve of them, so a
/// query's own extra columns start at index 12.
const SESSION_COLUMNS: &str = "id,title,harness,cwd,provider_session_id,updated_at,\
    has_user_message,archived,pinned,linked_work_item_json,sidebar_hidden,created_at";
/// Clock slack between a prompt's send time and its run's start.
const START_SLACK_MS: u64 = 1_000;
/// A prompt's run start. No run starts before its prompt was sent: an earlier
/// start was copied from another turn and marks a steer, not a new turn.
fn turn_start(block: &Value) -> Option<u64> {
    block["startedAt"].as_u64().filter(|at| {
        block["sentAt"]
            .as_u64()
            .is_none_or(|sent| at.saturating_add(START_SLACK_MS) >= sent)
    })
}
/// Turn keys are row offsets, including invisible document/card metadata rows.
/// Never substitute the session's current model for missing historical provenance.
pub(super) fn project_turns(
    blocks: &[Value],
) -> Result<HashMap<usize, crate::agent::TurnInfo>, String> {
    let mut turns = HashMap::new();
    let mut offset = 0;
    let mut root_turn = None;
    for block in blocks {
        let rows = blocks::project(std::slice::from_ref(block))?;
        if let Some(prompt) = rows.iter().rposition(Row::is_prompt) {
            let model = &block["turnModel"];
            let duration = block["durationMs"].as_u64();
            let started_at = turn_start(block);
            let turn = offset + prompt + 1;
            if started_at.is_some() {
                root_turn = Some(turn);
            }
            let joined_turn =
                (block["role"] == "user" && block["sentAt"].is_number() && started_at.is_none())
                    .then_some(root_turn)
                    .flatten();
            let metrics = block
                .get("turnMetrics")
                .filter(|value| value.is_object())
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            if model.is_object()
                || duration.is_some()
                || started_at.is_some()
                || metrics.is_some()
                || block["providerTurnId"].is_string()
                || joined_turn.is_some()
            {
                turns.insert(
                    offset + prompt + 1,
                    crate::agent::TurnInfo {
                        joined_turn,
                        provider_turn_id: block["providerTurnId"].as_str().map(str::to_owned),
                        provider: model["harness"].as_str().unwrap_or("").into(),
                        model: model["name"].as_str().unwrap_or("").into(),
                        model_id: model["id"].as_str().map(str::to_owned),
                        duration_ms: duration.unwrap_or(0),
                        duration_known: duration.is_some(),
                        started_at,
                        completed_at: started_at
                            .and_then(|at| at.checked_add(duration.unwrap_or(0))),
                        metrics,
                    },
                );
            }
        }
        offset += rows.len();
    }
    Ok(turns)
}

/// Opens gespan.db (`orm_path`) through Toasty: adopts and migrates it, and
/// marks a `fresh` file (one that did not exist) as past every Tauri
/// migration, since an older Tauri build that opens it must not run them
/// again: they predate every column here. [`Store::open_gespan`] and the
/// decision log's store both open the file this way, so whichever opens it
/// first leaves it the same.
pub(crate) fn open_database(orm_path: &Path, fresh: bool) -> Result<crate::orm::Database, String> {
    let database = crate::orm::open(crate::orm::Kind::Gespan, orm_path)?;
    if fresh {
        database.run(|mut db| async move {
            super::schema::TauriMigration::create()
                .version(19)
                .applied_at(0)
                .exec(&mut db)
                .await
                .map(drop)
        })?;
    }
    Ok(database)
}

impl Store {
    /// Open the sessions, notes and reminders the desktop app saved. Gespan
    /// owns the file: one an older Tauri build left is adopted into the
    /// Toasty schema (`store::schema`), its rows kept and a backup written.
    pub fn open_gespan(path: &Path) -> Result<Self, String> {
        Self::open_shared(path, false, None)
    }
    pub fn read_gespan(path: &Path) -> Result<Self, String> {
        Self::open_shared(path, true, None)
    }
    #[cfg(test)]
    pub(crate) fn read_gespan_with_preferences_for_test(
        path: &Path,
        preferences: std::sync::Arc<crate::desktop_preferences::DesktopPreferences>,
    ) -> Result<Self, String> {
        Self::open_shared(path, true, Some(preferences))
    }
    #[cfg(test)]
    pub(crate) fn open_gespan_with_preferences_for_test(
        path: &Path,
        preferences: std::sync::Arc<crate::desktop_preferences::DesktopPreferences>,
    ) -> Result<Self, String> {
        Self::open_shared(path, false, Some(preferences))
    }
    /// A writable store adopts and migrates the file through Toasty (see
    /// `store::schema`); a brand-new file gets the current schema. A read-only
    /// store never changes the file: one Gespan has not adopted yet is read
    /// through an adopted copy.
    fn open_shared(
        path: &Path,
        read_only: bool,
        preferences: Option<std::sync::Arc<crate::desktop_preferences::DesktopPreferences>>,
    ) -> Result<Self, String> {
        let memory = path == Path::new(":memory:");
        let (database, snapshot) = if read_only {
            let database = crate::orm::open_read_only(crate::orm::Kind::Gespan, path)?;
            if database.is_current()? {
                (database, None)
            } else {
                let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
                let copy = dir.path().join("gespan.db");
                let target = copy.to_string_lossy().into_owned();
                database.run(|mut db| async move {
                    toasty::sql::statement("VACUUM INTO ?1")
                        .bind(target)
                        .exec(&mut db)
                        .await
                })?;
                drop(database);
                drop(crate::orm::open(crate::orm::Kind::Gespan, &copy)?);
                (
                    crate::orm::open_read_only(crate::orm::Kind::Gespan, &copy)?,
                    Some(dir),
                )
            }
        } else {
            let fresh = memory || !path.exists();
            let database = open_database(path, fresh)?;
            (database, None)
        };
        let (extras, extras_db) = settings::open(path, read_only)?;
        let store = Self {
            path: path.into(),
            shared: Shared {
                ids: RefCell::new(HashMap::new()),
                extras: RefCell::new(extras),
                extras_db,
                snapshots: RefCell::new(HashMap::new()),
                windows: RefCell::new(HashMap::new()),
                queues: RefCell::new(HashMap::new()),
                account_source: RefCell::new(None),
                preferences: preferences.unwrap_or_else(|| {
                    std::sync::Arc::new(
                        path.parent()
                            .filter(|_| !memory)
                            .map(crate::desktop_preferences::DesktopPreferences::load)
                            .unwrap_or_default(),
                    )
                }),
                read_only,
            },
            orm: database,
            _snapshot: snapshot,
        };
        // Register only identities here. No blocks_json is read at startup.
        for id in store.orm.run(|mut db| async move {
            SessionRecord::all()
                .select(SessionRecord::fields().id())
                .exec(&mut db)
                .await
        })? {
            store.shared_register(&id)?;
        }
        Ok(store)
    }
    /// Whether the session row `original` exists. Reads only its key.
    fn shared_exists(&self, original: &str) -> Result<bool, String> {
        let original = original.to_owned();
        self.orm()?.run(|mut db| async move {
            SessionRecord::filter_by_id(original)
                .select(SessionRecord::fields().id())
                .exec(&mut db)
                .await
                .map(|ids| !ids.is_empty())
        })
    }
    /// A read-only snapshot of desktop preferences, shared by this store's
    /// consumers. Reopening the store refreshes it without per-message WebKit I/O.
    pub fn desktop_preferences(&self) -> Option<&crate::desktop_preferences::DesktopPreferences> {
        Some(self.shared.preferences.as_ref())
    }

    pub(super) fn shared_search_reader(&self) -> Result<Self, String> {
        Self::open_shared(&self.path, true, Some(self.shared.preferences.clone()))
    }

    /// Resolve only a persisted identity; unlike CLI handle parsing this never
    /// treats a stale numeric-looking desktop ID as an existing session.
    pub fn resolve_existing_session_id(&self, id: &str) -> Option<i64> {
        let exists = self.shared_exists(id).ok()?;
        exists.then(|| self.shared_register(id).ok()).flatten()
    }
    /// UI handles are stable hashes; the database always retains its original IDs.
    pub fn persisted_session_id(&self, id: i64) -> Result<String, String> {
        if id < 0 {
            if let Some(remote) = self.remote_session(id)? {
                return Ok(format!(
                    "remote:{}:{}",
                    remote.project.environment_id,
                    remote.session_id.unwrap_or_else(|| id.to_string())
                ));
            }
        }
        self.shared
            .ids
            .borrow()
            .get(&id)
            .cloned()
            .ok_or_else(|| "Session no longer exists".into())
    }
    pub fn resolve_session_id(&self, id: &str) -> Result<i64, String> {
        if let Some((key, _)) = self
            .shared
            .ids
            .borrow()
            .iter()
            .find(|(_, value)| value.as_str() == id)
        {
            return Ok(*key);
        }
        if self.shared_exists(id)? {
            return self.shared_register(id);
        }
        id.parse::<i64>().map_err(|_| "Unknown session ID".into())
    }
    pub(super) fn shared_register(&self, id: &str) -> Result<i64, String> {
        let key = handle(id);
        let mut ids = self.shared.ids.borrow_mut();
        if ids.get(&key).is_some_and(|existing| existing != id) {
            return Err("Session identity collision; the database has not been modified".into());
        }
        ids.insert(key, id.into());
        Ok(key)
    }
    pub(super) fn shared_writable(&self) -> Result<(), String> {
        if self.shared.read_only {
            Err("This Gespan store is read-only".into())
        } else {
            Ok(())
        }
    }
    /// Where a shared store's SQL runs.
    pub(super) fn db(&self) -> Result<Sql<'_>, String> {
        self.sql()
    }
    /// One session's linked work item, read by its own row.
    pub(super) fn shared_work_item(
        &self,
        id: i64,
    ) -> Result<Option<crate::session_groups::WorkItem>, String> {
        let original = self.persisted_session_id(id)?;
        let raw = self
            .orm()?
            .run(|mut db| async move {
                SessionRecord::filter_by_id(original)
                    .select(SessionRecord::fields().linked_work_item_json())
                    .exec(&mut db)
                    .await
            })?
            .into_iter()
            .next()
            .ok_or("Session no longer exists")?;
        Ok(raw.and_then(|raw| parse(&raw).ok()))
    }
    /// Sets `fields` of one session row, each named by its column.
    pub(super) fn shared_patch(&self, id: i64, fields: &[(&str, Value)]) -> Result<(), String> {
        self.shared_writable()?;
        let id = self.persisted_session_id(id)?;
        if fields.is_empty() {
            return Ok(());
        }
        let mut update = SessionRecord::filter_by_id(id).update();
        for (name, value) in fields {
            let column = Column { name, value };
            update = match *name {
                "cwd" => update.cwd(column.text()?),
                "harness" => update.harness(column.text()?),
                "model" => update.model(column.text()?),
                "model_settings" => update.model_settings(column.text()?),
                "runtime_mode" => update.runtime_mode(column.text()?),
                "title" => update.title(column.text()?),
                "blocks_json" => update.blocks_json(column.text()?),
                "queued_messages_json" => update.queued_messages_json(column.text()?),
                "provider_session_id" => update.provider_session_id(column.optional_text()?),
                "branch" => update.branch(column.optional_text()?),
                "worktree_cwd" => update.worktree_cwd(column.optional_text()?),
                "linked_work_item_json" => update.linked_work_item_json(column.optional_text()?),
                "provider_account_id" => update.provider_account_id(column.optional_text()?),
                "automation_id" => update.automation_id(column.optional_text()?),
                "queue_status" => update.queue_status(column.optional_text()?),
                "inbox_ask" => update.inbox_ask(column.optional_text()?),
                "created_at" => update.created_at(column.integer()?),
                "updated_at" => update.updated_at(column.integer()?),
                "context_used" => update.context_used(column.optional_integer()?),
                "context_window" => update.context_window(column.optional_integer()?),
                "archived" => update.archived(column.flag()?),
                "has_user_message" => update.has_user_message(column.flag()?),
                "pinned" => update.pinned(column.flag()?),
                "worktree_removed" => update.worktree_removed(column.flag()?),
                "is_draft" => update.is_draft(column.flag()?),
                "sidebar_hidden" => update.sidebar_hidden(column.flag()?),
                other => return Err(format!("Unknown session column {other}")),
            };
        }
        self.orm()?
            .run(|mut db| async move { update.exec(&mut db).await })
    }
    pub(super) fn shared_setting(&self, key: &str) -> Result<Option<String>, String> {
        if let Some((id, message)) = key
            .strip_prefix("pilot-delivered:")
            .and_then(|v| v.split_once(':'))
        {
            if let Ok(id) = id.parse::<i64>() {
                if self.shared_delivered(id, message)? {
                    return Ok(Some("1".into()));
                }
            }
        }
        Ok(self.shared.extras.borrow().get(key).cloned())
    }
    pub(super) fn shared_save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.shared_writable()?;
        let shared = &self.shared;
        if shared
            .extras
            .borrow()
            .get(key)
            .is_some_and(|old| old == value)
        {
            return Ok(());
        }
        if let Some(database) = &shared.extras_db {
            settings::save(database, key, value)?;
        }
        // Publish only committed values. A failed save must remain retryable.
        shared.extras.borrow_mut().insert(key.into(), value.into());
        Ok(())
    }
    /// The session, its original ID, and whether it is hidden from the sidebar,
    /// from a row that starts with [`SESSION_COLUMNS`].
    fn shared_session(&self, row: &SqlRow) -> Result<(Session, String, bool), String> {
        let original: String = row.get(0)?;
        let id = self.shared_register(&original)?;
        Ok((
            Session {
                id,
                title: row.get(1)?,
                provider: row.get(2)?,
                cwd: row.get(3)?,
                provider_id: row.get(4)?,
                updated_at: row.get::<i64>(5)?.max(0) as u64,
                has_messages: row.get(6)?,
                archived: row.get(7)?,
                pinned: row.get(8)?,
                created_at: row.get::<i64>(11)?.max(0) as u64,
                work_item: row
                    .get::<Option<String>>(9)?
                    .and_then(|raw| parse(&raw).ok()),
            },
            original,
            row.get::<bool>(10)?,
        ))
    }
    /// Ask sessions with their raw context, which `shared_sessions` hides.
    pub(super) fn shared_inbox_asks(&self) -> Result<Vec<(Session, String)>, String> {
        let rows = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter(session.inbox_ask().is_some())
                .order_by((session.updated_at().desc(), session.id().asc()))
                .select((
                    (
                        session.id(),
                        session.title(),
                        session.harness(),
                        session.cwd(),
                    ),
                    (session.provider_session_id(), session.updated_at()),
                    (
                        session.has_user_message(),
                        session.archived(),
                        session.pinned(),
                    ),
                    (session.linked_work_item_json(), session.created_at()),
                    session.inbox_ask(),
                ))
                .exec(&mut db)
                .await
        })?;
        let mut result = Vec::new();
        for (
            (original, title, provider, cwd),
            (provider_id, updated_at),
            (has_messages, archived, pinned),
            (work_item, created_at),
            ask,
        ) in rows
        {
            let Some(ask) = ask else { continue };
            let session = Session {
                id: self.shared_register(&original)?,
                title,
                provider,
                cwd,
                provider_id,
                updated_at: updated_at.max(0) as u64,
                has_messages,
                archived,
                pinned,
                created_at: created_at.max(0) as u64,
                work_item: work_item.and_then(|raw| parse(&raw).ok()),
            };
            result.push((session, ask));
        }
        Ok(result)
    }
    pub(super) fn shared_sessions(&self) -> Result<Vec<Session>, String> {
        // Raw: the sidebar list must read only the covering index
        // (`INDEXED BY`), never a row with its blocks_json, and Toasty 0.11
        // cannot name an index.
        let rows = self.db()?.query(
            &format!(
                "SELECT {SESSION_COLUMNS} FROM sessions INDEXED BY sessions_cwd_cover_idx
                 WHERE id NOT IN (SELECT id FROM sessions WHERE inbox_ask IS NOT NULL)
                 AND id NOT IN (SELECT session_id FROM orchestration_workers)
                 ORDER BY updated_at DESC,id"
            ),
            args![],
        )?;
        let mut result = Vec::new();
        let mut hidden = HashSet::new();
        let mut originals = HashMap::new();
        for row in &rows {
            let (session, original, is_hidden) = self.shared_session(row)?;
            if is_hidden {
                hidden.insert(session.id);
            }
            originals.insert(session.id, original);
            result.push(session);
        }
        // Keep the source sidebar's compatibility exception for hidden
        // /operator launches. A Pilot's own hidden conversation cannot be a
        // listed caller. The UI also checks project/worktree and Pilot scope.
        let shown: HashMap<_, _> = result
            .iter()
            .filter(|s| !hidden.contains(&s.id) && s.has_messages)
            .map(|s| (originals[&s.id].clone(), s.cwd.clone()))
            .collect();
        let shown_ids = shown.keys().cloned().collect();
        result.retain(|s| {
            !hidden.contains(&s.id)
                || started_caller(&originals[&s.id], &shown_ids)
                    .is_some_and(|caller| shown[caller] == s.cwd)
        });
        Ok(result)
    }
    pub(super) fn shared_create(&self, provider: &str, cwd: &str) -> Result<Session, String> {
        self.shared_create_named(provider, cwd, &crate::accounts::request_id())
            .map(|(session, _)| session)
    }
    pub(super) fn shared_create_named(
        &self,
        provider: &str,
        cwd: &str,
        original: &str,
    ) -> Result<(Session, bool), String> {
        self.shared_writable()?;
        let label = crate::provider::Kind::parse(provider)
            .ok_or_else(|| format!("Unknown provider: {provider}"))?
            .label();
        let id = self.shared_register(original)?;
        if self.shared_exists(original)? {
            let existing = self
                .shared_sessions()?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or("The session created by this request was removed")?;
            if existing.provider != provider
                || crate::files::path_key(&existing.cwd) != crate::files::path_key(cwd)
            {
                return Err("Request ID was already used for another session".into());
            }
            return Ok((existing, false));
        }
        let now = crate::time::now_ms().min(i64::MAX as u64) as i64;
        let title = format!("New {label} session");
        // Toasty's DDL has no DEFAULT: every column Tauri's schema defaulted
        // gets that value here.
        let create = SessionRecord::create()
            .id(original)
            .cwd(cwd)
            .harness(provider)
            .model(format!("{provider}:default"))
            .model_settings("{}")
            .runtime_mode("supervised")
            .title(&title)
            .blocks_json("[]")
            .created_at(now)
            .updated_at(now)
            .archived(false)
            .has_user_message(false)
            .pinned(false)
            .worktree_removed(false)
            .is_draft(false)
            .queued_messages_json("[]")
            .sidebar_hidden(false);
        self.orm()?
            .run(|mut db| async move { create.exec(&mut db).await.map(drop) })?;
        Ok((
            Session {
                id,
                title,
                provider: provider.into(),
                cwd: cwd.into(),
                provider_id: None,
                has_messages: false,
                updated_at: now as u64,
                archived: false,
                pinned: false,
                created_at: now as u64,
                work_item: None,
            },
            true,
        ))
    }
    pub(super) fn shared_agent(&self, id: i64) -> Result<Option<Settings>, String> {
        let original = self.persisted_session_id(id)?;
        let raw = self
            .orm()?
            .run(|mut db| async move {
                let session = SessionRecord::fields();
                SessionRecord::filter_by_id(original)
                    .select((
                        (session.harness(), session.model(), session.model_settings()),
                        (session.runtime_mode(), session.worktree_cwd()),
                        (session.worktree_removed(), session.provider_account_id()),
                        (session.cwd(), session.provider_session_id()),
                        session.has_user_message(),
                    ))
                    .exec(&mut db)
                    .await
            })?
            .into_iter()
            .next();
        let Some((
            (provider, model, controls),
            (access, cwd),
            (removed, account),
            (project, provider_id),
            has_messages,
        )) = raw
        else {
            return Ok(None);
        };
        let mut controls: HashMap<String, String> = parse(&controls)?;
        let mut settings: Settings = self
            .shared_setting(&format!("agent:{id}"))?
            .map(|raw| parse(&raw))
            .transpose()?
            .unwrap_or_default();
        settings.model = super::gespan_import::native_model(&model, &provider);
        // A prior native launch may have created an untouched default blank.
        // Seed only that blank, never a saved or resumed conversation's model.
        let default_controls = controls.iter().all(|(key, value)| {
            (key == "fast" && value == "false") || (key == "serviceTier" && value == "default")
        });
        if settings.model.is_none()
            && default_controls
            && settings.revision == 0
            && !has_messages
            && provider_id.is_none()
        {
            if let Some(defaults) = self
                .desktop_preferences()
                .and_then(|preferences| preferences.session_defaults(&provider, &project))
            {
                settings.model = defaults.model;
                controls = defaults.model_settings;
            }
        }
        settings.access = crate::agent::AccessMode::from_id(&access);
        settings.effort = [
            "effort",
            "reasoning",
            "reasoningEffort",
            "reasoning_effort",
            "variant",
        ]
        .into_iter()
        .find_map(|key| controls.get(key).cloned());
        settings.fast = controls.get("fast").is_some_and(|v| v == "true");
        settings.service_tier = controls.get("serviceTier").cloned();
        settings.thinking = controls.get("thinking").and_then(|s| s.parse().ok());
        settings.model_settings = controls;
        settings.worktree_cwd = cwd;
        settings.worktree_removed = removed;
        settings.account.id = account.unwrap_or_else(|| crate::accounts::DEFAULT.into());
        // Desktop blocks are authoritative. Native-only legacy metadata remains
        // available, but cannot override a saved desktop turn's identity/time.
        let blocks = self.shared_blocks(id)?;
        settings.operator_enabled |= blocks.iter().any(|block| {
            block["role"] == "user"
                && block["draft"] != true
                && block["internal"] != true
                && (block["gespan"] == true
                    || ["pilot", "gespan"].into_iter().any(|command| {
                        crate::composer::consume_command(
                            block["text"].as_str().unwrap_or_default(),
                            command,
                        )
                        .is_some()
                    }))
        });
        for (turn, source) in project_turns(&blocks)? {
            let info = settings.turns.entry(turn).or_default();
            if !source.provider.is_empty() {
                info.provider = source.provider;
            }
            if !source.model.is_empty() {
                info.model = source.model;
            }
            if source.model_id.is_some() {
                info.model_id = source.model_id;
            }
            if source.provider_turn_id.is_some() {
                info.provider_turn_id = source.provider_turn_id;
            }
            if source.joined_turn.is_some() {
                info.joined_turn = source.joined_turn;
            }
            if source.duration_known {
                info.duration_ms = source.duration_ms;
                info.duration_known = true;
            }
            if source.started_at.is_some() {
                info.started_at = source.started_at;
                info.completed_at = source.completed_at;
            }
            if source.metrics.is_some() {
                info.metrics = source.metrics;
            }
        }
        Ok(Some(settings))
    }
    pub(super) fn shared_save_agent(&self, id: i64, settings: &Settings) -> Result<(), String> {
        self.shared_save_turns(id, &settings.turns)?;
        self.shared_save_agent_preferences(id, settings)
    }
    /// Preference-only updates for validated operations which already preserve
    /// their source transcript inside a surrounding transaction (rewind and
    /// worktree cleanup). These must not re-save or re-index turn receipts.
    fn shared_save_agent_preferences(&self, id: i64, settings: &Settings) -> Result<(), String> {
        let current = self.shared_agent(id)?.unwrap_or_default();
        let mut controls = current.model_settings;
        controls.extend(settings.model_settings.clone());
        let effort_key = [
            "effort",
            "reasoning",
            "reasoningEffort",
            "reasoning_effort",
            "variant",
        ]
        .into_iter()
        .find(|key| controls.contains_key(*key))
        .unwrap_or("effort");
        if let Some(effort) = &settings.effort {
            controls.insert(effort_key.into(), effort.clone());
        } else {
            controls.remove(effort_key);
        }
        controls.insert("fast".into(), settings.fast.to_string());
        if let Some(tier) = &settings.service_tier {
            controls.insert("serviceTier".into(), tier.clone());
        }
        if let Some(thinking) = settings.thinking {
            controls.insert("thinking".into(), thinking.to_string());
        }
        let mut fields = vec![
            ("model_settings", json!(encode(&controls)?)),
            ("runtime_mode", json!(settings.access.id())),
            ("worktree_cwd", json!(settings.worktree_cwd)),
            ("worktree_removed", json!(settings.worktree_removed)),
            ("provider_account_id", json!(settings.account.id)),
        ];
        let original = self.persisted_session_id(id)?;
        let row = one(self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter_by_id(original)
                .select((session.harness(), session.model()))
                .exec(&mut db)
                .await
        })?)?;
        let (provider, saved_model) = row;
        if settings.model != super::gespan_import::native_model(&saved_model, &provider) {
            let model = settings.model.as_deref().unwrap_or("default");
            let model = if provider == "claude" {
                model.strip_prefix("claude-").unwrap_or(model)
            } else {
                model
            };
            fields.push(("model", json!(format!("{provider}:{model}"))));
        }
        self.shared_patch(id, &fields)?;
        self.shared_save_setting(&format!("agent:{id}"), &encode(settings)?)
    }
    /// Add native turn receipts to their original desktop prompt objects.
    /// Unknown rich fields and existing provenance are retained verbatim.
    fn shared_save_turns(
        &self,
        id: i64,
        turns: &HashMap<usize, crate::agent::TurnInfo>,
    ) -> Result<(), String> {
        if turns.is_empty() {
            return Ok(());
        }
        self.shared_writable()?;
        let tx = self.db()?.begin()?;
        let mut blocks = self.shared_blocks(id)?;
        if self
            .shared
            .snapshots
            .borrow()
            .get(&id)
            .is_some_and(|revision| !revision.matches(&blocks))
        {
            return Err("This conversation changed in another window. Reopen it before saving; its data was preserved.".into());
        }
        let original = self.persisted_session_id(id)?;
        let row = one(self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter_by_id(original)
                .select((session.harness(), session.model()))
                .exec(&mut db)
                .await
        })?)?;
        let (session_provider, session_model) = row;
        let mut offset = 0;
        let mut changed = false;
        for block in &mut blocks {
            let rows = blocks::project(std::slice::from_ref(block))?;
            let turn = rows
                .iter()
                .rposition(Row::is_prompt)
                .map(|prompt| offset + prompt + 1);
            offset += rows.len();
            let Some(info) = turn.and_then(|turn| turns.get(&turn)) else {
                continue;
            };
            let mut fields = Vec::new();
            if !info.provider.is_empty() && (!info.model.is_empty() || info.model_id.is_some()) {
                let mut model = json!({"harness":info.provider});
                if !info.model.is_empty() {
                    model["name"] = json!(info.model);
                }
                if let Some(id) = &info.model_id {
                    model["id"] = json!(if info.provider == session_provider
                        && super::gespan_import::native_model(&session_model, &session_provider)
                            .as_ref()
                            == Some(id)
                    {
                        session_model.clone()
                    } else if id.starts_with(&format!("{}:", info.provider)) {
                        id.clone()
                    } else {
                        format!("{}:{id}", info.provider)
                    });
                }
                fields.push(("turnModel", model));
            }
            if info.has_duration() {
                fields.push(("durationMs", json!(info.duration_ms)));
            }
            if let Some(id) = &info.provider_turn_id {
                fields.push(("providerTurnId", json!(id)));
            }
            if let Some(at) = info.started_at {
                // A native root prompt is written with its send time before
                // its turn receipt arrives; the desktop schema calls it startedAt.
                // A message sent after that start is a steer, never its root.
                if block["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("gpui-"))
                    && block["startedAt"].is_null()
                    && block["sentAt"]
                        .as_u64()
                        .is_none_or(|sent| at.saturating_add(START_SLACK_MS) >= sent)
                {
                    block["startedAt"] = json!(at);
                    if block["sentAt"].as_u64() == Some(at) {
                        block.as_object_mut().unwrap().remove("sentAt");
                    }
                    changed = true;
                }
            }
            if let Some(metrics) = info.metrics.filter(|metrics| metrics.reported()) {
                let mut saved = json!({"inputTokens":metrics.input_tokens,
                    "outputTokens":metrics.output_tokens,"cacheReadTokens":metrics.cache_read_tokens,
                    "cacheWriteTokens":metrics.cache_write_tokens,"cacheHitPercent":metrics.cache_hit_percent});
                for (key, value) in [("costUsd", metrics.cost_usd), ("credits", metrics.credits)] {
                    if let Some(value) = value {
                        saved[key] = json!(value);
                    }
                }
                fields.push(("turnMetrics", saved));
            }
            for (key, value) in fields {
                if block[key].is_null() {
                    block[key] = value;
                    changed = true;
                } else if key == "turnModel" {
                    if let (Some(saved), Some(incoming)) =
                        (block[key].as_object_mut(), value.as_object())
                    {
                        for (name, value) in incoming {
                            if !saved.contains_key(name) {
                                saved.insert(name.clone(), value.clone());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
        if changed {
            self.shared_store_blocks(id, &blocks)?;
        }
        tx.commit()?;
        if changed {
            self.shared
                .snapshots
                .borrow_mut()
                .insert(id, HistoryRevision::of(&blocks));
        }
        Ok(())
    }
    pub(super) fn shared_context(&self, id: i64) -> Result<Option<Usage>, String> {
        let original = self.persisted_session_id(id)?;
        let (used, window) = one(self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter_by_id(original)
                .select((session.context_used(), session.context_window()))
                .exec(&mut db)
                .await
        })?)?;
        Ok(used.filter(|u| *u >= 0).map(|used| Usage {
            used: used as u64,
            window: window.filter(|w| *w > 0).map(|w| w as u64),
            percent: None,
        }))
    }
    /// The blocks whose rows this store numbers: an indexed Pilot's window, or
    /// the whole history of one stored as a blob.
    pub(super) fn shared_blocks(&self, id: i64) -> Result<Vec<Value>, String> {
        let original = self.persisted_session_id(id)?;
        if let Some(from) = self.shared_window(id, &original)? {
            self.shared_indexed_blocks(&original, from, i64::MAX)
        } else {
            let raw = one(self.orm()?.run(|mut db| async move {
                SessionRecord::filter_by_id(original)
                    .select(SessionRecord::fields().blocks_json())
                    .exec(&mut db)
                    .await
            })?)?;
            parse(&raw)
        }
    }
    pub(super) fn shared_load(&self, id: i64) -> Result<Transcript, String> {
        let original = self.persisted_session_id(id)?;
        if let Some(from) = self.shared_window(id, &original)? {
            // Project one source block at a time. A long Pilot must not retain
            // a second complete JSON history alongside its display rows.
            // The scalar count and the rows share one SQLite read snapshot:
            // one raw statement, as Toasty 0.11 cannot project a subquery.
            let records = self.db()?.query(
                "SELECT block_json,(SELECT count(*) FROM pilot_blocks WHERE session_id=?1 AND seq>=?2)
                 FROM pilot_blocks WHERE session_id=?1 AND seq>=?2 ORDER BY seq",
                args![&original, from],
            )?;
            let mut rows = Vec::new();
            let mut digest = HistoryRevision::start(0);
            let mut first = true;
            for record in records {
                if first {
                    digest = HistoryRevision::start(record.get::<i64>(1)? as usize);
                    first = false;
                }
                let raw: String = record.get(0)?;
                let block: Value = parse(&raw)?;
                HistoryRevision::push(&mut digest, &block);
                rows.extend(blocks::project(std::slice::from_ref(&block))?);
            }
            self.shared
                .snapshots
                .borrow_mut()
                .insert(id, HistoryRevision(digest.finalize().into()));
            self.restore_claude_shell(id, &mut rows);
            return Ok(Transcript { rows });
        }
        let blocks = self.shared_blocks(id)?;
        let mut rows = blocks::project(&blocks)?;
        self.restore_claude_shell(id, &mut rows);
        self.shared
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&blocks));
        Ok(Transcript { rows })
    }
    fn restore_claude_shell(&self, id: i64, rows: &mut [Row]) {
        if claude_shell::bare_ids(rows).is_empty() {
            return;
        }
        let Ok(original) = self.persisted_session_id(id) else {
            return;
        };
        let session = self.orm().and_then(|database| {
            database.run(|mut db| async move {
                let session = SessionRecord::fields();
                SessionRecord::filter(
                    session
                        .id()
                        .eq(original)
                        .and(session.harness().eq("claude")),
                )
                .select(session.provider_session_id())
                .exec(&mut db)
                .await
            })
        });
        let session = session.and_then(one);
        let (Ok(Some(session)), Ok(Some(settings))) = (session, self.shared_agent(id)) else {
            return;
        };
        // Stored settings carry only the account id; its folder is under our data.
        let Some(data) = self.data_dir() else {
            return;
        };
        let account = crate::accounts::Profile::new(&settings.account.id, data);
        if let Ok(home) = account.home(crate::provider::Kind::Claude) {
            claude_shell::restore(rows, &home, &session);
        }
    }
    pub(super) fn shared_store_blocks(&self, id: i64, blocks: &[Value]) -> Result<(), String> {
        let original = self.persisted_session_id(id)?;
        let (has_user, draft) = if let Some(from) = self.shared_window(id, &original)? {
            self.shared_store_indexed_blocks(&original, from, blocks)?;
            // Blocks before the window count too, and were never read.
            let session = original.clone();
            self.orm()?.run(|mut db| async move {
                let block = PilotBlock::fields();
                let user = block.session_id().eq(session).and(block.is_user().eq(true));
                let has_user = !PilotBlock::filter(user.clone())
                    .limit(1)
                    .select(block.seq())
                    .exec(&mut db)
                    .await?
                    .is_empty();
                let draft = !PilotBlock::filter(user.and(block.is_draft().eq(true)))
                    .limit(1)
                    .select(block.seq())
                    .exec(&mut db)
                    .await?
                    .is_empty();
                Ok((has_user, draft))
            })?
        } else {
            self.shared_patch(id, &[("blocks_json", json!(encode(&blocks)?))])?;
            (
                blocks.iter().any(|b| b["role"] == "user"),
                blocks
                    .iter()
                    .any(|b| b["role"] == "user" && b["draft"] == true),
            )
        };
        self.shared_patch(
            id,
            &[
                ("has_user_message", json!(has_user)),
                ("is_draft", json!(draft)),
            ],
        )
    }
    /// Replace the window's blocks, from sequence `from` on. Earlier blocks are
    /// outside every row index this store handed out, so they stay untouched.
    fn shared_store_indexed_blocks(
        &self,
        original: &str,
        from: i64,
        blocks: &[Value],
    ) -> Result<(), String> {
        let database = self.orm()?;
        let mut old = HashMap::new();
        // New blocks always land inside the window, after everything stored.
        let first = from.max(1);
        let session = original.to_owned();
        let (last, window) = database.run(|mut db| async move {
            let block = PilotBlock::fields();
            let last = PilotBlock::filter(block.session_id().eq(&session))
                .order_by(block.seq().desc())
                .limit(1)
                .select(block.seq())
                .exec(&mut db)
                .await?;
            let window =
                PilotBlock::filter(block.session_id().eq(&session).and(block.seq().ge(from)))
                    .order_by(block.seq().asc())
                    .select((block.seq(), block.block_id(), block.block_json()))
                    .exec(&mut db)
                    .await?;
            Ok((last, window))
        })?;
        let mut max_seq = last.into_iter().next().unwrap_or(0).max(first - 1);
        for (seq, id, raw) in window {
            old.insert(id, (seq, parse::<Value>(&raw)?));
        }
        let retained: HashSet<_> = blocks.iter().filter_map(|b| b["id"].as_str()).collect();
        if retained.len() != blocks.len() {
            return Err("Indexed conversation contains missing or duplicate block IDs".into());
        }
        let mut previous = 0;
        let mut appended = false;
        let mut resequence = false;
        for block in blocks {
            if let Some((seq, _)) = old.get(block["id"].as_str().unwrap()) {
                if appended || *seq <= previous {
                    resequence = true;
                }
                previous = *seq;
            } else {
                appended = true;
            }
        }
        if resequence {
            // An explicit insertion/reorder changes turn positions. Payloads
            // and block identities survive, in one enclosing transaction.
            let session = original.to_owned();
            database.run(|mut db| async move {
                let turn = PilotTurn::fields();
                PilotTurn::filter(turn.session_id().eq(&session).and(turn.seq().ge(from)))
                    .delete()
                    .exec(&mut db)
                    .await?;
                let block = PilotBlock::fields();
                PilotBlock::filter(block.session_id().eq(&session).and(block.seq().ge(from)))
                    .delete()
                    .exec(&mut db)
                    .await
            })?;
            old.clear();
            max_seq = first - 1;
        } else {
            for (id, (seq, _)) in &old {
                if !retained.contains(id.as_str()) {
                    let (session, id, seq) = (original.to_owned(), id.clone(), *seq);
                    database.run(|mut db| async move {
                        let turn = PilotTurn::fields();
                        PilotTurn::filter(turn.session_id().eq(&session).and(turn.seq().eq(seq)))
                            .delete()
                            .exec(&mut db)
                            .await?;
                        let block = PilotBlock::fields();
                        PilotBlock::filter(
                            block.session_id().eq(&session).and(block.block_id().eq(id)),
                        )
                        .delete()
                        .exec(&mut db)
                        .await
                    })?;
                }
            }
        }
        let mut after_handoff = false;
        for (ix, block) in blocks.iter().enumerate() {
            let id = block["id"].as_str().unwrap();
            let (seq, changed) = match old.get(id) {
                Some((seq, old)) => (*seq, old != block),
                None => {
                    max_seq += 1;
                    (max_seq, true)
                }
            };
            if changed {
                let search = [
                    block["text"].as_str(),
                    block["tool"]["title"].as_str(),
                    block["tool"]["detail"].as_str(),
                    block["image"]["name"].as_str(),
                    block["image"]["alt"].as_str(),
                    block["tool"]["preview"]["path"].as_str(),
                    block["tool"]["preview"]["query"].as_str(),
                    block["tool"]["preview"]["output"].as_str(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase();
                // A block keeps its id at its sequence: only its content changes.
                let upsert = PilotBlock::upsert_by_session_id_and_seq(original, seq)
                    .on_create(|created| created.block_id(id))
                    .block_json(encode(block)?)
                    .search_text(search)
                    .is_user(block["role"] == "user")
                    .is_draft(block["draft"] == true);
                database.run(|mut db| async move { upsert.exec(&mut db).await.map(drop) })?;
            }
            if ix == 0
                || after_handoff
                || (block["role"] == "user" && block["internal"] != true)
                || block.get("pilotHabit").is_some()
            {
                let turn = PilotTurn::upsert_by_session_id_and_seq(original, seq).or_ignore();
                database.run(|mut db| async move { turn.exec(&mut db).await.map(drop) })?;
            }
            after_handoff = block["role"] == "handoff";
        }
        Ok(())
    }
    pub(super) fn shared_write_rows(
        &self,
        id: i64,
        start: usize,
        rows: &[Row],
        message_id: Option<&str>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let tx = self.db()?.begin()?;
        let current = self.shared_blocks(id)?;
        if self
            .shared
            .snapshots
            .borrow()
            .get(&id)
            .is_some_and(|old| !old.matches(&current))
        {
            return Err("This conversation changed in another window. Reopen it before saving; its data was preserved.".into());
        }
        let mut next = blocks::merge(&current, start, rows)?;
        if let Some(message) = message_id {
            if let Some(prompt) = next
                .iter_mut()
                .rev()
                .find(|b| b["role"] == "user" && b["draft"] != true)
            {
                prompt["appRequestId"] = json!(message);
            }
            next.retain(|b| !(b["draft"] == true && b["id"].as_str() == Some(message)));
        }
        self.shared_store_blocks(id, &next)?;
        let mut next_queue = None;
        if let Some(message) = message_id {
            let raw = self.shared_queue(id)?;
            let mut queued: Vec<Value> = parse(&raw)?;
            queued.retain(|m| m["id"].as_str() != Some(message));
            let encoded = encode(&queued)?;
            self.shared_patch(id, &[("queued_messages_json", json!(encoded))])?;
            next_queue = Some(encoded);
        }

        if !rows.is_empty() {
            self.shared_patch(id, &[("updated_at", json!(crate::time::now_ms()))])?;
        }
        tx.commit()?;
        if let Some(queue) = next_queue {
            self.shared.queues.borrow_mut().insert(id, queue);
        }
        self.shared
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&next));
        // The source appRequestId is the durable, transactional receipt.
        Ok(())
    }
    pub(super) fn shared_trim_rows(&self, id: i64, from: usize) -> Result<(), String> {
        self.shared_writable()?;
        let tx = self.db()?.begin()?;
        let next = blocks::trim(&self.shared_blocks(id)?, from)?;
        self.shared_store_blocks(id, &next)?;
        tx.commit()?;
        self.shared
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&next));
        Ok(())
    }
    pub(super) fn shared_reset_provider_id_if(
        &self,
        id: i64,
        expected: Option<&str>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let original = self.persisted_session_id(id)?;
        let expected = expected.map(str::to_owned);
        self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            let current = match expected {
                Some(expected) => session.provider_session_id().eq(Some(expected)),
                None => session.provider_session_id().is_none(),
            };
            SessionRecord::filter(session.id().eq(original).and(current))
                .update()
                .provider_session_id(None::<String>)
                .exec(&mut db)
                .await
        })
    }
    pub(super) fn shared_change_provider(&self, id: i64, provider: &str) -> Result<(), String> {
        self.shared_writable()?;
        let original = self.persisted_session_id(id)?;
        let provider = provider.to_owned();
        self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter(
                session
                    .id()
                    .eq(original)
                    .and(session.harness().ne(&provider)),
            )
            .update()
            .harness(provider)
            .provider_session_id(None::<String>)
            .exec(&mut db)
            .await
        })
    }
    pub(super) fn shared_replace_title(
        &self,
        id: i64,
        expected: &str,
        title: &str,
    ) -> Result<bool, String> {
        self.shared_writable()?;
        // Raw: a compare-and-swap that must report whether it changed the
        // row, and Toasty 0.11's typed update returns no count.
        self.db()?
            .execute(
                "UPDATE sessions SET title=?3 WHERE id=?1 AND title=?2",
                args![self.persisted_session_id(id)?, expected, title],
            )
            .map(|n| n == 1)
    }
    /// Moves every session saved in `from` to `to`; only the folder changes.
    pub(super) fn shared_rebase_project(&self, from: &str, to: &str) -> Result<(), String> {
        self.shared_writable()?;
        let (from, to) = (from.to_owned(), to.to_owned());
        self.orm()?.run(|mut db| async move {
            SessionRecord::filter(SessionRecord::fields().cwd().eq(from))
                .update()
                .cwd(to)
                .exec(&mut db)
                .await
        })
    }
    pub(super) fn shared_worktrees(&self) -> Result<HashMap<i64, Option<String>>, String> {
        let rows = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::all()
                .select((session.id(), session.worktree_cwd()))
                .exec(&mut db)
                .await
        })?;
        let mut out = HashMap::new();
        for (id, path) in rows {
            out.insert(self.shared_register(&id)?, path);
        }
        Ok(out)
    }
    pub(super) fn shared_sidebar_hidden(&self) -> Result<HashSet<i64>, String> {
        let ids = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter(session.sidebar_hidden().eq(true))
                .select(session.id())
                .exec(&mut db)
                .await
        })?;
        ids.iter().map(|id| self.shared_register(id)).collect()
    }
    pub(super) fn shared_started_links(&self) -> Result<HashMap<i64, i64>, String> {
        let rows: Vec<(String, bool)> = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::all()
                .select((session.id(), session.sidebar_hidden()))
                .exec(&mut db)
                .await
        })?;
        let ids = rows.iter().map(|(id, _)| id.clone()).collect();
        let shown = rows
            .iter()
            .filter(|(_, hidden)| !hidden)
            .map(|(id, _)| id.clone())
            .collect();
        rows.iter()
            .filter_map(|(id, hidden)| {
                started_caller(id, if *hidden { &shown } else { &ids }).map(|caller| (id, caller))
            })
            .map(|(id, caller)| Ok((self.shared_register(id)?, self.shared_register(caller)?)))
            .collect()
    }
    pub(super) fn shared_pins(&self, ids: &[i64], pinned: bool) -> Result<(), String> {
        let tx = self.db()?.begin()?;
        for id in ids {
            self.shared_patch(*id, &[("pinned", json!(pinned))])?;
        }
        tx.commit()
    }
    pub(super) fn shared_settings_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(i64, String)>, String> {
        Ok(self
            .shared
            .extras
            .borrow()
            .iter()
            .filter_map(|(key, value)| {
                Some((key.strip_prefix(prefix)?.parse().ok()?, value.clone()))
            })
            .collect())
    }
    pub(super) fn shared_drafts(&self) -> Result<HashSet<i64>, String> {
        let ids = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter(session.is_draft().eq(true))
                .select(session.id())
                .exec(&mut db)
                .await
        })?;
        ids.iter().map(|id| self.shared_register(id)).collect()
    }
    pub(super) fn shared_groups(&self, cwd: &str) -> Result<crate::session_groups::Groups, String> {
        if let Some(raw) = self.shared_setting(&format!("groups:{cwd}"))? {
            return parse(&raw);
        }
        Ok(self
            .desktop_preferences()
            .and_then(|preferences| {
                preferences.groups(cwd, |id| self.resolve_existing_session_id(id))
            })
            .unwrap_or_default())
    }
    pub(super) fn shared_ui_state(&self) -> Result<Option<String>, String> {
        let saved = self.shared_setting("workspace")?;
        let mut workspace = saved.as_deref().map(parse::<Value>).transpose()?;
        {
            let raw = self
                .orm()?
                .run(|mut db| async move {
                    WorkspaceSnapshot::filter_by_id(1)
                        .select(WorkspaceSnapshot::fields().snapshot_json())
                        .exec(&mut db)
                        .await
                })?
                .into_iter()
                .next();
            if let Some(raw) = raw {
                let snapshot: Value = parse(&raw)?;
                if workspace.is_none() {
                    workspace = Some(
                        crate::desktop_preferences::DesktopPreferences::workspace_snapshot(
                            &snapshot,
                            |id| self.resolve_existing_session_id(id),
                        ),
                    );
                } else if let Some(saved) = workspace.as_mut().and_then(Value::as_object_mut) {
                    // Preserve the source snapshot alongside native layouts;
                    // unsupported panes can be recovered without rewriting it.
                    saved
                        .entry("desktop_workspace_snapshot")
                        .or_insert(snapshot);
                }
            }
        }
        let mut merged = workspace.clone().unwrap_or_else(|| json!({}));
        if let Some(preferences) = self.desktop_preferences() {
            preferences.apply_workspace(&mut merged);
        }
        if merged.as_object().is_some_and(|object| !object.is_empty()) {
            Ok(Some(merged.to_string()))
        } else {
            Ok(workspace.map(|value| value.to_string()))
        }
    }
    pub(super) fn shared_reminders(&self) -> Result<Vec<Reminder>, String> {
        let rows = self.orm()?.run(|mut db| async move {
            let reminder = SessionReminder::fields();
            SessionReminder::all()
                .order_by((reminder.due_at().asc(), reminder.session_id().asc()))
                .select((
                    reminder.session_id(),
                    reminder.due_at(),
                    reminder.fired_at(),
                ))
                .exec(&mut db)
                .await
        })?;
        let mut out = Vec::new();
        for (id, due, fired) in rows {
            out.push(Reminder {
                session_id: self.shared_register(&id)?,
                due_at: due.max(0) as u64,
                fired_at: fired.map(|at| at.max(0) as u64),
            });
        }
        Ok(out)
    }
    pub(super) fn shared_set_reminders(&self, ids: &[i64], due: u64) -> Result<(), String> {
        self.shared_writable()?;
        let tx = self.db()?.begin()?;
        for id in ids {
            let upsert = SessionReminder::upsert_by_session_id(self.persisted_session_id(*id)?)
                .due_at(due.min(i64::MAX as u64) as i64)
                .fired_at(None::<i64>);
            self.orm()?
                .run(|mut db| async move { upsert.exec(&mut db).await.map(drop) })?;
        }
        tx.commit()
    }
    pub(super) fn shared_snooze_reminder_if(
        &self,
        id: i64,
        expected_due: u64,
        new_due: u64,
    ) -> Result<bool, String> {
        self.shared_writable()?;
        // Raw: a conditional update that must report whether it matched, and
        // Toasty 0.11's typed update returns no count.
        self.db()?.execute("UPDATE session_reminders SET due_at=?3,fired_at=NULL WHERE session_id=?1 AND due_at=?2",
            args![self.persisted_session_id(id)?, expected_due.min(i64::MAX as u64) as i64, new_due.min(i64::MAX as u64) as i64]).map(|changed|changed==1)
    }
    pub(super) fn shared_clear_reminders(
        &self,
        ids: &[i64],
        expected: Option<u64>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        for id in ids {
            let original = self.persisted_session_id(*id)?;
            let expected = expected.map(|at| at.min(i64::MAX as u64) as i64);
            self.orm()?.run(|mut db| async move {
                let reminder = SessionReminder::fields();
                let mut which = reminder.session_id().eq(original);
                if let Some(due) = expected {
                    which = which.and(reminder.due_at().eq(due));
                }
                SessionReminder::filter(which).delete().exec(&mut db).await
            })?;
        }
        Ok(())
    }
    pub(super) fn shared_fire_reminders(
        &self,
        due: &[Reminder],
        now: u64,
    ) -> Result<Vec<Reminder>, String> {
        self.shared_writable()?;
        let tx = self.db()?.begin()?;
        let mut fired = Vec::new();
        for reminder in due {
            // Raw: each reminder fires once, so the update must report
            // whether it matched; Toasty 0.11's typed update returns no count.
            let changed=self.db()?.execute("UPDATE session_reminders SET fired_at=?2 WHERE session_id=?1 AND fired_at IS NULL AND due_at=?3",
                args![self.persisted_session_id(reminder.session_id)?, now.min(i64::MAX as u64) as i64, reminder.due_at.min(i64::MAX as u64) as i64])?;
            if changed == 1 {
                fired.push(reminder.clone());
            }
        }
        tx.commit()?;
        Ok(fired)
    }
    /// Removes the settings saved for session `id`: its agent, context,
    /// queue, delivery receipts, links and runs.
    fn shared_forget_settings(&self, id: i64) -> Result<(), String> {
        let exact = [
            "agent",
            "context",
            "follow-up",
            "started-by",
            "orchestration",
            "orchestration-run",
            "inbox-ask",
            "automation",
        ]
        .map(|prefix| format!("{prefix}:{id}"));
        let delivered = format!("pilot-delivered:{id}:");
        let keys: Vec<String> = self
            .shared
            .extras
            .borrow()
            .keys()
            .filter(|key| exact.contains(key) || key.starts_with(&delivered))
            .cloned()
            .collect();
        if keys.is_empty() {
            return Ok(());
        }
        if let Some(database) = &self.shared.extras_db {
            settings::remove(database, keys.clone())?;
        }
        let mut extras = self.shared.extras.borrow_mut();
        for key in keys {
            extras.remove(&key);
        }
        Ok(())
    }
    pub(super) fn shared_delete(&self, id: i64) -> Result<(), String> {
        self.shared_writable()?;
        let original = self.persisted_session_id(id)?;
        let tx = self.begin()?;
        // The cascade triggers (`store::schema::CONSTRAINTS`) would remove
        // most of these; deleting each explicitly keeps tables with no
        // foreign key (in-flight, orchestration) clean too.
        self.orm()?.run(|mut db| async move {
            let id = &original;
            SessionReminder::filter(SessionReminder::fields().session_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            PilotBlock::filter(PilotBlock::fields().session_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            PilotTurn::filter(PilotTurn::fields().session_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            PilotTranscript::filter(PilotTranscript::fields().session_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            InFlightSession::filter(InFlightSession::fields().session_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            OrchestrationWorker::filter(OrchestrationWorker::fields().session_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            OrchestrationSidebar::filter(OrchestrationSidebar::fields().lead_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            OrchestrationRun::filter(OrchestrationRun::fields().lead_id().eq(id))
                .delete()
                .exec(&mut db)
                .await?;
            SessionRecord::filter(SessionRecord::fields().id().eq(id))
                .delete()
                .exec(&mut db)
                .await
        })?;
        tx.commit()?;
        self.shared.snapshots.borrow_mut().remove(&id);
        self.shared_forget_settings(id)
    }
    pub(super) fn shared_detach_worktree(
        &self,
        path: &str,
        main: &str,
    ) -> Result<Vec<(Session, Settings)>, String> {
        let tx = self.db()?.begin()?;
        let mut out = Vec::new();
        for mut session in self.shared_sessions()? {
            let mut settings = self.shared_agent(session.id)?.unwrap_or_default();
            let execution = settings.worktree_cwd.as_deref().unwrap_or(&session.cwd);
            if !crate::files::is_equal_or_inside(execution, path) {
                continue;
            }
            settings.worktree_cwd = Some(execution.into());
            settings.worktree_removed = true;
            settings.revision += 1;
            settings.new_worktree = false;
            settings.worktree_base = None;
            if crate::files::is_equal_or_inside(&session.cwd, path) {
                session.cwd = main.into();
            }
            session.provider_id = None;
            self.shared_patch(
                session.id,
                &[
                    ("cwd", json!(session.cwd)),
                    ("provider_session_id", Value::Null),
                ],
            )?;
            self.shared_save_agent_preferences(session.id, &settings)?;
            out.push((session, settings));
        }
        tx.commit()?;
        Ok(out)
    }
    pub(super) fn shared_search(
        &self,
        query: &str,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(Vec<crate::search::Hit>, bool), String> {
        use std::sync::atomic::Ordering;
        if query.trim().is_empty() || cancel.load(Ordering::Acquire) {
            return Ok((Vec::new(), false));
        }
        // Cancellation is checked between sessions and rows.
        let result = (|| {
            let summaries = self.shared_sessions()?;
            let mut hits = Vec::new();
            let mut truncated = false;
            'sessions: for summary in summaries.iter().filter(|s| !s.archived && s.has_messages) {
                if cancel.load(Ordering::Acquire) {
                    return Ok((Vec::new(), false));
                }
                let rows = blocks::project(&self.shared_all_blocks(summary.id)?)?;
                let mut ix = 0;
                while ix < rows.len() {
                    if cancel.load(Ordering::Acquire) {
                        return Ok((Vec::new(), false));
                    }
                    let from = ix;
                    let mut text = String::new();
                    loop {
                        let body = &rows[ix].text;
                        let mut end = body
                            .len()
                            .min((2 * 1024 * 1024usize).saturating_sub(text.len()));
                        while !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        text.push_str(&body[..end]);
                        truncated |= end < body.len();
                        ix += 1;
                        if ix == rows.len() || rows[ix].first {
                            break;
                        }
                        if cancel.load(Ordering::Acquire) {
                            return Ok((Vec::new(), false));
                        }
                    }
                    if let Some(hit) = crate::search::message_hit(
                        summary,
                        from,
                        &rows[from].role,
                        &text,
                        query,
                        false,
                    ) {
                        hits.push(hit);
                    }
                    if hits.len() >= 400 {
                        truncated = true;
                        break 'sessions;
                    }
                }
            }
            hits.extend(crate::search::titles(&summaries, query));
            Ok((hits, truncated))
        })();
        if cancel.load(Ordering::Acquire) {
            Ok((Vec::new(), false))
        } else {
            result
        }
    }
    /// The session's queued messages, as stored.
    pub(super) fn shared_queue(&self, id: i64) -> Result<String, String> {
        let original = self.persisted_session_id(id)?;
        one(self.orm()?.run(|mut db| async move {
            SessionRecord::filter_by_id(original)
                .select(SessionRecord::fields().queued_messages_json())
                .exec(&mut db)
                .await
        })?)
    }
    pub(crate) fn shared_follow_ups(&self, id: i64) -> Result<crate::follow_up::State, String> {
        let original = self.persisted_session_id(id)?;
        let (raw, status) = one(self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter_by_id(original)
                .select((session.queued_messages_json(), session.queue_status()))
                .exec(&mut db)
                .await
        })?)?;
        let mut state: crate::follow_up::State = self
            .shared_setting(&format!("follow-up:{id}"))?
            .map(|raw| parse(&raw))
            .transpose()?
            .unwrap_or_default();
        self.shared.queues.borrow_mut().insert(id, raw.clone());
        state.messages = parse::<Vec<Value>>(&raw)?.iter().map(follow_up).collect();
        let blocks = self.shared_blocks(id)?;
        state.draft = None;
        if let Some((at, block)) = blocks.iter().enumerate().find(|(_, b)| b["draft"] == true) {
            let mut draft = follow_up(block);
            draft.at = blocks::project(&blocks[..at])?.len();
            state.draft = Some(draft);
        }
        state.paused = status.as_deref() == Some("paused") || !state.messages.is_empty();
        Ok(state)
    }
    pub(crate) fn shared_save_follow_ups(
        &self,
        id: i64,
        state: &crate::follow_up::State,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let previous: crate::follow_up::State = self
            .shared_setting(&format!("follow-up:{id}"))?
            .map(|raw| parse(&raw))
            .transpose()?
            .unwrap_or_default();
        if previous.revision > state.revision {
            return Ok(());
        }
        let tx = self.db()?.begin()?;
        let raw = self.shared_queue(id)?;
        if self
            .shared
            .queues
            .borrow()
            .get(&id)
            .is_some_and(|expected| expected != &raw)
        {
            return Err("The message queue changed in another window. Reopen it before saving; its data was preserved.".into());
        }
        let old: Vec<Value> = parse(&raw)?;
        let messages = state
            .messages
            .iter()
            .map(|m| {
                let mut value = old
                    .iter()
                    .find(|v| v["id"].as_str() == Some(&m.id))
                    .cloned()
                    .unwrap_or(json!({}));
                update_follow_up(&mut value, m);
                value
            })
            .collect::<Vec<_>>();
        self.shared_patch(
            id,
            &[
                ("queued_messages_json", json!(encode(&messages)?)),
                (
                    "queue_status",
                    json!(if state.paused { "paused" } else { "active" }),
                ),
            ],
        )?;
        let mut blocks = self.shared_blocks(id)?;
        if self
            .shared
            .snapshots
            .borrow()
            .get(&id)
            .is_some_and(|old| !old.matches(&blocks))
        {
            return Err("This conversation changed in another window. Reopen it before saving; its data was preserved.".into());
        }

        let old_draft = blocks.iter().find(|b| b["draft"] == true).cloned();
        blocks.retain(|b| b["draft"] != true);
        if let Some(draft) = &state.draft {
            let mut value = old_draft
                .filter(|v| v["id"].as_str() == Some(&draft.id))
                .unwrap_or(json!({"id":draft.id,"role":"user","draft":true}));
            update_follow_up(&mut value, draft);
            let mut at = blocks.len();
            let mut row = 0;
            for (ix, block) in blocks.iter().enumerate() {
                if row >= draft.at {
                    at = ix;
                    break;
                }
                row += blocks::project(std::slice::from_ref(block))?.len();
            }
            blocks.insert(at, value);
        }
        self.shared_store_blocks(id, &blocks)?;
        tx.commit()?;
        self.shared
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&blocks));
        self.shared
            .queues
            .borrow_mut()
            .insert(id, encode(&messages)?);
        self.shared_save_setting(&format!("follow-up:{id}"), &encode(state)?)
    }
}

fn follow_up(value: &Value) -> crate::follow_up::Message {
    crate::follow_up::Message {
        id: value["id"].as_str().unwrap_or("").into(),
        text: value["text"].as_str().unwrap_or("").into(),
        prepared_prompt: value["preparedPrompt"].as_str().map(str::to_owned),
        attachments: value["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["path"].as_str())
            .map(PathBuf::from)
            .collect(),
        mode: value["intent"].as_str().map(str::to_owned),
        error: value["error"].as_str().map(str::to_owned),
        ..Default::default()
    }
}
fn update_follow_up(value: &mut Value, message: &crate::follow_up::Message) {
    value["id"] = json!(message.id);
    value["text"] = json!(message.text);
    value["preparedPrompt"] = json!(message.prepared_prompt);
    if let Some(mode) = &message.mode {
        value["intent"] = json!(mode);
    }
    if let Some(error) = &message.error {
        value["error"] = json!(error);
    }
    if value["attachments"].is_null() {
        value["attachments"]=json!(message.attachments.iter().map(|p|json!({"id":p.to_string_lossy(),"path":p,"name":p.file_name().unwrap_or_default().to_string_lossy(),
            "mimeType":"application/octet-stream","kind":"file","size":std::fs::metadata(p).map(|m|m.len()).unwrap_or(0)})).collect::<Vec<_>>());
    }
}
