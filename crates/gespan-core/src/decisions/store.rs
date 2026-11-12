//! The decision log's store. Its entries are rows of `task_decisions` in
//! gespan.db, the table My tasks saves its project decisions to, so the two
//! are one list. [`DecisionStore`] has its own Toasty handle on the file
//! (the app's `Store` keeps another); SQLite's write-ahead log and the busy
//! timeout every ORM connection sets let the two share it.
//!
//! Times always come from the caller, never the clock, so every operation is
//! deterministic under test.
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use super::embed::{Embedder, DIMS};
use super::entry::{
    Config, Edit, Entry, Error, Flag, Kind, NewEntry, Scope, ShownHash, Source, Status,
};
use super::vector::{self, Vector};
use crate::store::db::{self, args, Sql, SqlRow};
use crate::store::my_tasks::TaskDecision;

/// Reciprocal rank fusion's `k`: how much a list's top ranks outweigh its
/// lower ones when keyword and vector results are merged.
pub const RRF_K: f64 = 60.0;

/// How many keyword and vector hits each go into a hybrid search before
/// they are fused and cut to the caller's limit.
const HYBRID_POOL: usize = 50;

/// sqlite-vec refuses a nearest-neighbour `k` above this.
const MAX_KNN: usize = 4096;

/// An agent's disagreement with an active entry, for the user to review.
#[derive(Debug, toasty::Model)]
#[table = "decision_flags"]
#[index(name = "decision_flags_entry_idx", entry, dismissed)]
pub(crate) struct DecisionFlag {
    #[key]
    #[auto]
    pub(crate) id: i64,
    /// The flagged entry's id in `task_decisions`.
    pub(crate) entry: i64,
    pub(crate) session: i64,
    pub(crate) reason: String,
    pub(crate) created_at: i64,
    pub(crate) dismissed: bool,
}

/// What the gespan kind runs on every open (`orm::Kind::constraints`):
/// keyword search over the log, as an external-content FTS5 index on
/// `task_decisions` kept in step by triggers. Every statement is idempotent.
/// The last one indexes rows the triggers never saw (rows older builds left,
/// or a file adopted before the index existed): it rebuilds only when the
/// index and the table hold different numbers of rows.
/// The `vec0` table for search by meaning is `vector::CONSTRAINTS`, run
/// right after these.
pub(crate) const CONSTRAINTS: &[&str] = &[
    "CREATE VIRTUAL TABLE IF NOT EXISTS task_decisions_fts USING fts5(
       title, statement, why, content='task_decisions', content_rowid='id'
     )",
    "CREATE TRIGGER IF NOT EXISTS task_decisions_fts_ai AFTER INSERT ON task_decisions BEGIN
       INSERT INTO task_decisions_fts(rowid, title, statement, why)
       VALUES (new.id, new.title, new.statement, new.why);
     END",
    "CREATE TRIGGER IF NOT EXISTS task_decisions_fts_ad AFTER DELETE ON task_decisions BEGIN
       INSERT INTO task_decisions_fts(task_decisions_fts, rowid, title, statement, why)
       VALUES ('delete', old.id, old.title, old.statement, old.why);
     END",
    "CREATE TRIGGER IF NOT EXISTS task_decisions_fts_au
       AFTER UPDATE OF title, statement, why ON task_decisions BEGIN
       INSERT INTO task_decisions_fts(task_decisions_fts, rowid, title, statement, why)
       VALUES ('delete', old.id, old.title, old.statement, old.why);
       INSERT INTO task_decisions_fts(rowid, title, statement, why)
       VALUES (new.id, new.title, new.statement, new.why);
     END",
    "INSERT INTO task_decisions_fts(task_decisions_fts) SELECT 'rebuild'
     WHERE (SELECT COUNT(*) FROM task_decisions_fts_docsize)
        <> (SELECT COUNT(*) FROM task_decisions)",
];

/// `task_decisions` columns in the order `row_to_entry` reads them, with the
/// open flag count last.
const COLS: &str = "e.id, e.kind, e.scope, e.status, e.title, e.statement, e.why, \
    e.user_answer, e.session, e.question_ref, e.user_turn, e.import_src, e.pinned, \
    e.supersedes, e.replaced_by, e.created_at, e.decided_at, \
    (SELECT COUNT(*) FROM decision_flags f WHERE f.entry = e.id AND f.dismissed = 0)";

const HAS_USER_SOURCE: &str = "((e.question_ref IS NOT NULL AND e.question_ref <> '') \
     OR (e.user_turn IS NOT NULL AND e.user_turn <> ''))";

/// What an agent may see in `scope` (bound as `?1`): active entries, plus
/// pending ones that carry a user source, for the scope and for global.
/// `visible` and `search` both filter with this, so they cannot drift apart.
fn visible_filter() -> String {
    format!(
        "(e.status = 'active' OR (e.status = 'pending' AND {HAS_USER_SOURCE})) \
         AND e.scope IN (?1, 'global')"
    )
}

/// The decision log over `task_decisions` and `decision_flags`.
pub struct DecisionStore {
    db: crate::orm::Database,
    config: Config,
    /// Set while "Find related decisions by meaning" is on: entries get
    /// vectors as they are written, and search is hybrid.
    embedder: RwLock<Option<Arc<dyn Embedder>>>,
    /// One re-embedding pass at a time.
    reembedding: Mutex<()>,
}

impl DecisionStore {
    /// Opens gespan.db at `path` the way the app's store does (adopting and
    /// migrating it, and marking a new file as past the Tauri migrations), so
    /// it does not matter which of the two opens the file first. `:memory:`
    /// opens a private in-memory database.
    pub fn open(path: &Path, config: Config) -> Result<DecisionStore, Error> {
        let fresh = crate::orm::is_memory(path) || !path.exists();
        let db = crate::store::shared::open_database(path, fresh)?;
        Ok(DecisionStore {
            db,
            config,
            embedder: RwLock::new(None),
            reembedding: Mutex::new(()),
        })
    }

    /// The embedder search by meaning uses, when it is on.
    pub fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        self.embedder
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Turns search by meaning on with `embedder`, or off with `None`.
    ///
    /// On: the model is recorded next to the vectors; if another model (or
    /// none) made the stored ones, they are all deleted in that same
    /// transaction. Then every active or pending entry whose vector is
    /// missing or stale is embedded ([`reembed`](Self::reembed)), so this
    /// can take a while: call it off the UI thread. Off: entries are no
    /// longer embedded and search is keyword only; stored vectors are kept
    /// for when it comes back on with the same model.
    pub fn set_embedder(&self, embedder: Option<Arc<dyn Embedder>>) -> Result<(), Error> {
        let Some(embedder) = embedder else {
            *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Ok(());
        };
        if embedder.dims() != DIMS {
            return Err(Error::Embed(format!(
                "{} makes {}-dimension vectors; the log stores {DIMS}",
                embedder.model_id(),
                embedder.dims()
            )));
        }
        // Held across the switch, so a pass for the old model can't write
        // its vectors after the table was emptied for the new one.
        let _one_pass = self.reembedding.lock().unwrap_or_else(|e| e.into_inner());
        let model = embedder.model_id().to_string();
        let dims = embedder.dims();
        self.db.transaction(move |tx| {
            Box::pin(async move { vector::adopt_model(tx, &model, dims).await })
        })?;
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = Some(embedder);
        drop(_one_pass);
        self.reembed().map(drop)
    }

    /// Brings the vectors in line with the entries: embeds each active or
    /// pending entry whose vector is missing, from another model, of other
    /// text or in another scope, and deletes the vectors of entries that
    /// can no longer be shown (or no longer exist). Each entry's write is
    /// its own transaction, which re-reads the entry and skips it if it
    /// changed while it was being embedded (the next pass catches it).
    /// Returns how many entries were embedded; does nothing while search by
    /// meaning is off. Embeds one entry at a time: call it off the UI thread.
    pub fn reembed(&self) -> Result<usize, Error> {
        let _one_pass = self.reembedding.lock().unwrap_or_else(|e| e.into_inner());
        let Some(embedder) = self.embedder() else {
            return Ok(0);
        };
        let model = embedder.model_id().to_string();
        let stored: std::collections::HashMap<i64, (String, String)> = self
            .sql()
            .query(
                "SELECT rowid, scope, fingerprint FROM task_decisions_vec",
                args![],
            )?
            .iter()
            .map(|r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
            .collect::<Result<_, String>>()?;
        let rows = self.sql().query(
            "SELECT id, scope, title, statement, why FROM task_decisions              WHERE status IN ('active', 'pending') ORDER BY id",
            args![],
        )?;
        let mut keep = std::collections::HashSet::new();
        let mut embedded = 0;
        for row in rows {
            let (id, scope): (i64, String) = (row.get(0)?, row.get(1)?);
            let (title, text, why): (String, String, String) =
                (row.get(2)?, row.get(3)?, row.get(4)?);
            keep.insert(id);
            let fingerprint = vector::fingerprint(&model, &title, &text, &why);
            if stored.get(&id) == Some(&(scope.clone(), fingerprint.clone())) {
                continue;
            }
            let embedding = embedder.embed_document(&vector::document(&title, &text, &why))?;
            let v = Vector {
                embedding,
                fingerprint,
            };
            let model = model.clone();
            let wrote = self.db.transaction(move |tx| {
                Box::pin(async move {
                    let Some(now) = TaskDecision::filter_by_id(id)
                        .first()
                        .exec(&mut *tx)
                        .await?
                    else {
                        return Ok(false);
                    };
                    let live = matches!(
                        Status::parse(&now.status),
                        Ok(Status::Active | Status::Pending)
                    );
                    if !live
                        || now.scope != scope
                        || vector::fingerprint(&model, &now.title, &now.statement, &now.why)
                            != v.fingerprint
                    {
                        return Ok(false);
                    }
                    vector::put(tx, id, &scope, &v).await?;
                    Ok(true)
                })
            })?;
            embedded += usize::from(wrote);
        }
        let stale: Vec<i64> = stored.into_keys().filter(|id| !keep.contains(id)).collect();
        if !stale.is_empty() {
            self.db
                .transaction(move |tx| Box::pin(async move { vector::remove(tx, &stale).await }))?;
        }
        Ok(embedded)
    }

    /// The vector for an entry with this title, text and why, when search by
    /// meaning is on. An embedder failure is logged and gives `None`: the
    /// entry is written without one, and the next [`reembed`](Self::reembed)
    /// adds it.
    fn vector_for(&self, title: &str, text: &str, why: &str) -> Option<Vector> {
        let embedder = self.embedder()?;
        match embedder.embed_document(&vector::document(title, text, why)) {
            Ok(embedding) => Some(Vector {
                embedding,
                fingerprint: vector::fingerprint(embedder.model_id(), title, text, why),
            }),
            Err(error) => {
                tracing::warn!(%error, "a decision log entry could not be embedded");
                None
            }
        }
    }

    fn sql(&self) -> Sql<'_> {
        Sql::Orm(&self.db)
    }

    /// Redact, then cap (`Config::clean`, which My tasks uses too).
    fn clean(&self, s: &str) -> String {
        self.config.clean(s)
    }

    /// Source text is user-supplied free text too: redact and cap it, then
    /// treat what is left empty as absent.
    fn clean_source(&self, s: &Option<String>) -> Option<String> {
        s.as_deref()
            .map(|v| self.clean(v))
            .filter(|v| !v.is_empty())
    }

    /// Moves My tasks decisions that are still keyed `project:<their
    /// project>` (rows migration 0001 gave that scope, or ones saved without
    /// git) into the scope `scope_of` gives their project: a repo scope for a
    /// repository with a remote, the main checkout for a worktree, a
    /// canonical path. `scope_of` (which runs git) is called for each
    /// distinct project outside any transaction. A row whose scope already
    /// differs is never touched, so running it again changes nothing.
    /// Returns how many rows moved.
    pub fn rescope_project_rows(&self, scope_of: impl Fn(&Path) -> Scope) -> Result<usize, Error> {
        let projects = self.sql().query(
            "SELECT DISTINCT project FROM task_decisions WHERE scope = 'project:' || project",
            args![],
        )?;
        let mut moved = 0;
        for row in projects {
            let project: String = row.get(0)?;
            let scope = scope_of(Path::new(&project)).to_string();
            if scope == format!("project:{project}") {
                continue;
            }
            moved += self.sql().execute(
                "UPDATE task_decisions SET scope = ?1 \
                 WHERE project = ?2 AND scope = 'project:' || ?2",
                args![scope, project],
            )?;
        }
        Ok(moved)
    }

    pub fn propose(&self, e: NewEntry, now_ms: i64) -> Result<Entry, Error> {
        if e.why.trim().is_empty() {
            return Err(Error::MissingWhy);
        }
        let title = self.clean(&e.title);
        let text = self.clean(&e.text);
        let why = self.clean(&e.why);
        let answer = e.user_answer.as_deref().map(|a| self.clean(a));
        let question_ref = self.clean_source(&e.source.question_ref);
        let user_turn = self.clean_source(&e.source.user_turn);
        let import = self.clean_source(&e.source.import);
        let project = super::entry::project_column(&e.scope);
        let (kind, scope) = (e.kind.as_str(), e.scope.to_string());
        let session = e.source.session;
        let v = self.vector_for(&title, &text, &why);
        // The entry and its vector are one transaction: neither is ever
        // written without the other.
        let id = self.db.transaction(move |tx| {
            Box::pin(async move {
                let row = TaskDecision::create()
                    .project(project)
                    .statement(text)
                    .why(why)
                    .created_at(now_ms)
                    .updated_at(now_ms)
                    .archived_at(None)
                    .kind(kind)
                    .status(Status::Pending.as_str())
                    .scope(scope.clone())
                    .title(title)
                    .user_answer(answer)
                    .session(session)
                    .question_ref(question_ref)
                    .user_turn(user_turn)
                    .import_src(import)
                    .pinned(false)
                    .supersedes(None)
                    .replaced_by(None)
                    .decided_at(None)
                    .exec(&mut *tx)
                    .await?;
                if let Some(v) = v {
                    vector::put(tx, row.id, &scope, &v).await?;
                }
                Ok(row.id)
            })
        })?;
        self.load(id)?.ok_or(Error::NotFound(id))
    }

    /// Proposes an imported entry unless its source already has one with the
    /// same text, whatever became of it (rejected and archived rows count, so
    /// a rejected import never comes back). `None` when it was already there.
    pub fn propose_import(&self, e: NewEntry, now_ms: i64) -> Result<Option<Entry>, Error> {
        if e.why.trim().is_empty() {
            return Err(Error::MissingWhy);
        }
        let src = self.clean_source(&e.source.import);
        let (title, text, why) = (
            self.clean(&e.title),
            self.clean(&e.text),
            self.clean(&e.why),
        );
        let (kind, scope) = (e.kind.as_str(), e.scope.to_string());
        let project = super::entry::project_column(&e.scope);
        let v = self.vector_for(&title, &text, &why);
        // One immediate transaction holds the check and the insert (and the
        // entry's vector), so two imports racing cannot both find the rule
        // missing.
        let id = self.db.transaction(move |tx| {
            Box::pin(async move {
                let f = TaskDecision::fields();
                let seen = TaskDecision::filter(
                    f.import_src()
                        .eq(src.clone())
                        .and(f.statement().eq(text.clone())),
                )
                .first()
                .exec(&mut *tx)
                .await?;
                if seen.is_some() {
                    return Ok(None);
                }
                let row = TaskDecision::create()
                    .project(project)
                    .statement(text)
                    .why(why)
                    .created_at(now_ms)
                    .updated_at(now_ms)
                    .archived_at(None)
                    .kind(kind)
                    .status(Status::Pending.as_str())
                    .scope(scope.clone())
                    .title(title)
                    .user_answer(None)
                    .session(None)
                    .question_ref(None)
                    .user_turn(None)
                    .import_src(src)
                    .pinned(false)
                    .supersedes(None)
                    .replaced_by(None)
                    .decided_at(None)
                    .exec(&mut *tx)
                    .await?;
                if let Some(v) = v {
                    vector::put(tx, row.id, &scope, &v).await?;
                }
                Ok(Some(row.id))
            })
        })?;
        id.map(|id| self.load(id)?.ok_or(Error::NotFound(id)))
            .transpose()
    }

    /// Approve a pending entry, optionally edited, optionally replacing active
    /// entries. All in one transaction.
    pub fn approve(
        &self,
        id: i64,
        edit: Option<Edit>,
        pinned: bool,
        replaces: &[i64],
        now_ms: i64,
    ) -> Result<Entry, Error> {
        self.approve_checked(id, None, edit, pinned, replaces, now_ms)
    }

    /// [`approve`](Self::approve), but only while the proposal still reads
    /// as `shown` (its title, text, why and user answer). The check and the
    /// approval are one transaction, so an agent's `revise_pending` can't
    /// slip in between; a changed proposal is refused with
    /// [`Error::Changed`] and nothing is written.
    pub fn approve_if_unchanged(
        &self,
        id: i64,
        shown: &ShownHash,
        edit: Option<Edit>,
        pinned: bool,
        replaces: &[i64],
        now_ms: i64,
    ) -> Result<Entry, Error> {
        self.approve_checked(id, Some(shown.clone()), edit, pinned, replaces, now_ms)
    }

    /// [`reject`](Self::reject), but only while the proposal still reads as
    /// `shown`; see [`approve_if_unchanged`](Self::approve_if_unchanged).
    pub fn reject_if_unchanged(
        &self,
        id: i64,
        shown: &ShownHash,
        now_ms: i64,
    ) -> Result<(), Error> {
        let shown = shown.clone();
        let refused = self.db.transaction(move |tx| {
            Box::pin(async move {
                let Some(current) = TaskDecision::filter_by_id(id).first().exec(tx).await? else {
                    return Ok(Some(Error::NotFound(id)));
                };
                if Status::parse(&current.status).ok() != Some(Status::Pending) {
                    return Ok(Some(Error::NotPending(id)));
                }
                if shown_now(&current) != shown {
                    return Ok(Some(Error::Changed));
                }
                let mut current = current;
                current
                    .update()
                    .status(Status::Rejected.as_str())
                    .decided_at(Some(now_ms))
                    .updated_at(now_ms)
                    .exec(&mut *tx)
                    .await?;
                vector::remove(tx, &[id]).await?;
                Ok(None)
            })
        })?;
        refused.map_or(Ok(()), Err)
    }

    fn approve_checked(
        &self,
        id: i64,
        shown: Option<ShownHash>,
        edit: Option<Edit>,
        pinned: bool,
        replaces: &[i64],
        now_ms: i64,
    ) -> Result<Entry, Error> {
        let edit = match edit {
            Some(ed) => {
                if ed.why.trim().is_empty() {
                    return Err(Error::MissingWhy);
                }
                Some(Edit {
                    title: self.clean(&ed.title),
                    text: self.clean(&ed.text),
                    why: self.clean(&ed.why),
                })
            }
            None => None,
        };
        // An edit changes what the entry says, so its vector is made again.
        let v = edit
            .as_ref()
            .and_then(|ed| self.vector_for(&ed.title, &ed.text, &ed.why));
        let replaces = replaces.to_vec();
        // The transaction's own error type is Toasty's, so a refusal is
        // carried out as the value and returned after the rollback.
        let refused = self.db.transaction(move |tx| {
            Box::pin(async move {
                let status = |row: &TaskDecision| Status::parse(&row.status).ok();
                let Some(current) = TaskDecision::filter_by_id(id).first().exec(tx).await? else {
                    return Ok(Some(Error::NotFound(id)));
                };
                if status(&current) != Some(Status::Pending) {
                    return Ok(Some(Error::NotPending(id)));
                }
                if shown.is_some_and(|shown| shown_now(&current) != shown) {
                    return Ok(Some(Error::Changed));
                }
                if let Some(refusal) = refuse_replacing(tx, &replaces, Some(id)).await? {
                    return Ok(Some(refusal));
                }
                mark_replaced(tx, &replaces, id, now_ms).await?;
                let scope = current.scope.clone();
                let mut current = current;
                let mut update = current
                    .update()
                    .status(Status::Active.as_str())
                    .pinned(pinned)
                    .decided_at(Some(now_ms))
                    .updated_at(now_ms)
                    .supersedes(replaces.first().copied());
                if let Some(ed) = edit {
                    update = update.title(ed.title).statement(ed.text).why(ed.why);
                }
                update.exec(&mut *tx).await?;
                if let Some(v) = v {
                    vector::put(tx, id, &scope, &v).await?;
                }
                Ok(None)
            })
        })?;
        if let Some(error) = refused {
            return Err(error);
        }
        self.load(id)?.ok_or(Error::NotFound(id))
    }

    /// Answers an agent's `flag` on active entry `id` with the user's own
    /// wording: one transaction checks the flag is open and on `id` and that
    /// `id` is active, writes the cleaned wording as an active entry of the
    /// same kind, scope, project and pin, marks `id` replaced by it, and
    /// closes every open flag on `id`. Any refusal or failure leaves all of
    /// it unwritten. Returns the new entry.
    pub fn replace_from_flag(
        &self,
        id: i64,
        flag: i64,
        new: Edit,
        now_ms: i64,
    ) -> Result<Entry, Error> {
        if new.why.trim().is_empty() {
            return Err(Error::MissingWhy);
        }
        let new = Edit {
            title: self.clean(&new.title),
            text: self.clean(&new.text),
            why: self.clean(&new.why),
        };
        let v = self.vector_for(&new.title, &new.text, &new.why);
        let outcome = self.db.transaction(move |tx| {
            Box::pin(async move {
                match DecisionFlag::filter_by_id(flag).first().exec(tx).await? {
                    Some(open) if open.entry == id && !open.dismissed => {}
                    _ => return Ok(Err(Error::FlagNotFound(flag))),
                }
                if let Some(refusal) = refuse_replacing(tx, &[id], None).await? {
                    return Ok(Err(refusal));
                }
                let Some(old) = TaskDecision::filter_by_id(id).first().exec(tx).await? else {
                    return Ok(Err(Error::NotFound(id)));
                };
                let successor = TaskDecision::create()
                    .project(old.project)
                    .statement(new.text)
                    .why(new.why)
                    .created_at(now_ms)
                    .updated_at(now_ms)
                    .archived_at(None)
                    .kind(old.kind)
                    .status(Status::Active.as_str())
                    .scope(old.scope.clone())
                    .title(new.title)
                    .user_answer(None)
                    .session(None)
                    .question_ref(None)
                    .user_turn(None)
                    .import_src(None)
                    .pinned(old.pinned)
                    .supersedes(Some(id))
                    .replaced_by(None)
                    .decided_at(Some(now_ms))
                    .exec(tx)
                    .await?;
                mark_replaced(tx, &[id], successor.id, now_ms).await?;
                if let Some(v) = v {
                    vector::put(tx, successor.id, &old.scope, &v).await?;
                }
                Ok(Ok(successor.id))
            })
        })?;
        let successor = outcome?;
        self.load(successor)?.ok_or(Error::NotFound(successor))
    }

    /// Rewrite a pending entry's title, text and why in place (redacted and
    /// capped like a new one), and replace its user answer and user source
    /// (`question_ref`, `user_turn`) with the given ones in the same update,
    /// so an old attestation never sits beside new text. The proposing
    /// session and import marker are kept. Anything but a pending entry is
    /// refused.
    pub fn revise_pending(
        &self,
        id: i64,
        edit: Edit,
        user_answer: Option<&str>,
        source: &Source,
    ) -> Result<Entry, Error> {
        if edit.why.trim().is_empty() {
            return Err(Error::MissingWhy);
        }
        let (title, text, why) = (
            self.clean(&edit.title),
            self.clean(&edit.text),
            self.clean(&edit.why),
        );
        let v = self.vector_for(&title, &text, &why);
        let update = db::statement(
            "UPDATE task_decisions SET title = ?1, statement = ?2, why = ?3, user_answer = ?4, \
             question_ref = ?5, user_turn = ?6 \
             WHERE id = ?7 AND status = 'pending'",
            args![
                title,
                text,
                why,
                user_answer.map(|a| self.clean(a)),
                self.clean_source(&source.question_ref),
                self.clean_source(&source.user_turn),
                id
            ],
        );
        let n = self.db.transaction(move |tx| {
            Box::pin(async move {
                let n = update.exec(&mut *tx).await?;
                if let (1.., Some(v)) = (n, v) {
                    let row = TaskDecision::filter_by_id(id)
                        .first()
                        .exec(&mut *tx)
                        .await?;
                    if let Some(row) = row {
                        vector::put(tx, id, &row.scope, &v).await?;
                    }
                }
                Ok(n)
            })
        })?;
        if n == 0 {
            return Err(self.missing_or(id, Error::NotPending(id))?);
        }
        self.load(id)?.ok_or(Error::NotFound(id))
    }

    pub fn reject(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        let n = self.leave_with_vector(
            "UPDATE task_decisions SET status = 'rejected', decided_at = ?1, updated_at = ?1 \
             WHERE id = ?2 AND status = 'pending'",
            id,
            now_ms,
        )?;
        if n == 0 {
            return Err(self.missing_or(id, Error::NotPending(id))?);
        }
        Ok(())
    }

    /// Archives an active entry. It leaves My tasks' Decisions list too,
    /// which reads `archived_at`.
    pub fn archive(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        let n = self.leave_with_vector(
            "UPDATE task_decisions SET status = 'archived', decided_at = ?1, \
             archived_at = ?1, updated_at = ?1 \
             WHERE id = ?2 AND status = 'active'",
            id,
            now_ms,
        )?;
        if n == 0 {
            return Err(self.missing_or(id, Error::NotActive(id))?);
        }
        Ok(())
    }

    /// Runs `update` (bound `?1` = `now_ms`, `?2` = `id`), which takes entry
    /// `id` out of what can be shown, and deletes its vector in the same
    /// transaction. Returns how many rows `update` changed.
    fn leave_with_vector(&self, update: &str, id: i64, now_ms: i64) -> Result<u64, Error> {
        let update = db::statement(update, args![now_ms, id]);
        Ok(self.db.transaction(move |tx| {
            Box::pin(async move {
                let n = update.exec(&mut *tx).await?;
                if n > 0 {
                    vector::remove(tx, &[id]).await?;
                }
                Ok(n)
            })
        })?)
    }

    pub fn flag(&self, id: i64, session: i64, reason: &str, now_ms: i64) -> Result<Flag, Error> {
        let reason = self.clean(reason);
        if self.load(id)?.is_none() {
            return Err(Error::NotFound(id));
        }
        let kept = reason.clone();
        let row = self.db.run(|mut db| async move {
            DecisionFlag::create()
                .entry(id)
                .session(session)
                .reason(kept)
                .created_at(now_ms)
                .dismissed(false)
                .exec(&mut db)
                .await
        })?;
        Ok(Flag {
            id: row.id,
            entry: id,
            session,
            reason,
            created_ms: now_ms,
        })
    }

    pub fn dismiss_flag(&self, flag: i64) -> Result<(), Error> {
        let n = self.sql().execute(
            "UPDATE decision_flags SET dismissed = 1 WHERE id = ?1",
            args![flag],
        )?;
        if n == 0 {
            return Err(Error::FlagNotFound(flag));
        }
        Ok(())
    }

    /// The entry and the entries it replaced (directly or through earlier
    /// replacements), newest first.
    pub fn get(&self, id: i64) -> Result<Option<(Entry, Vec<Entry>)>, Error> {
        let Some(entry) = self.load(id)? else {
            return Ok(None);
        };
        let mut history: Vec<Entry> = Vec::new();
        let mut frontier = vec![id];
        while let Some(next) = frontier.pop() {
            let sql = format!("SELECT {COLS} FROM task_decisions e WHERE e.replaced_by = ?1");
            for row in self.sql().query(&sql, args![next])? {
                let e = row_to_entry(&row)?;
                frontier.push(e.id);
                history.push(e);
            }
        }
        history.sort_by(|a, b| {
            b.decided_ms
                .cmp(&a.decided_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(Some((entry, history)))
    }

    /// Active entries, plus pending ones that carry a user source, for the
    /// scope and for global.
    pub fn visible(&self, scope: &Scope) -> Result<Vec<Entry>, Error> {
        self.query(&visible_filter(), Some(scope))
    }

    /// Active entries for the scope and for global, newest decided first.
    pub fn active(&self, scope: &Scope) -> Result<Vec<Entry>, Error> {
        self.query(
            "e.status = 'active' AND e.scope IN (?1, 'global')",
            Some(scope),
        )
    }

    pub fn pending(&self) -> Result<Vec<Entry>, Error> {
        self.query("e.status = 'pending'", None)
    }

    /// Undismissed flags with their entries, oldest first.
    pub fn open_flags(&self) -> Result<Vec<(Flag, Entry)>, Error> {
        let rows = self.sql().query(
            "SELECT id, entry, session, reason, created_at FROM decision_flags \
             WHERE dismissed = 0 ORDER BY created_at, id",
            args![],
        )?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let flag = Flag {
                id: row.get(0)?,
                entry: row.get(1)?,
                session: row.get(2)?,
                reason: row.get(3)?,
                created_ms: row.get(4)?,
            };
            let e = self.load(flag.entry)?.ok_or(Error::NotFound(flag.entry))?;
            out.push((flag, e));
        }
        Ok(out)
    }

    /// Search over title, text and why, best match first. Sees exactly what
    /// `visible` sees (same filter), so a rejected, replaced or archived
    /// entry, or another project's entry, is never returned.
    ///
    /// With search by meaning off it is keyword search. On, it is hybrid:
    /// the best [`HYBRID_POOL`] keyword hits and the best as many entries by
    /// meaning ([`related`](Self::related)) are merged by reciprocal rank
    /// fusion with k = [`RRF_K`]. If the query can't be embedded, it falls
    /// back to keywords.
    ///
    /// The query is arbitrary text. Every word becomes a quoted phrase, so
    /// FTS5 operators and syntax in it are plain words and cannot make the
    /// search fail. Words are OR-ed, and bm25 ranks entries matching more of
    /// them (or matching them more often) higher. A query with no usable
    /// words returns nothing.
    pub fn search(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
        let Some(embedder) = self.embedder() else {
            return self.keyword(scope, query, limit);
        };
        if match_expr(query).is_none() {
            return Ok(Vec::new());
        }
        let by_meaning = match embedder.embed_query(query) {
            Ok(v) => self.related(scope, &v, HYBRID_POOL)?,
            Err(error) => {
                tracing::warn!(%error, "the decision log could not embed a search");
                return self.keyword(scope, query, limit);
            }
        };
        let by_words = self.keyword(scope, query, HYBRID_POOL)?;
        let ids = |list: &[Entry]| list.iter().map(|e| e.id).collect::<Vec<_>>();
        let order = vector::fuse(&[ids(&by_words), ids(&by_meaning)], RRF_K);
        let mut entries: std::collections::HashMap<i64, Entry> = by_words
            .into_iter()
            .chain(by_meaning)
            .map(|e| (e.id, e))
            .collect();
        Ok(order
            .into_iter()
            .filter_map(|id| entries.remove(&id))
            .take(limit)
            .collect())
    }

    /// The `k` entries nearest `query_vec` by meaning that `visible` would
    /// show for `scope`, nearest first. Each nearest-neighbour query reads
    /// one partition of the vector table: `scope`'s, then (for a project
    /// scope) `global`'s, the preferences every scope sees. No other
    /// scope's vectors are read. Empty while search by meaning is off.
    pub fn related(&self, scope: &Scope, query_vec: &[f32], k: usize) -> Result<Vec<Entry>, Error> {
        if k == 0 || self.embedder().is_none() {
            return Ok(Vec::new());
        }
        if query_vec.len() != DIMS {
            return Err(Error::Embed(format!(
                "a {}-dimension query vector; the log stores {DIMS}",
                query_vec.len()
            )));
        }
        let scope_text = scope.to_string();
        let mut partitions = vec![scope_text.clone()];
        if scope_text != "global" {
            partitions.push("global".into());
        }
        // Rows that can't be shown lose their vectors as they leave, so a
        // pool a little past `k` is enough; the filter below is the rule.
        let pool = i64::try_from(k.saturating_mul(2).clamp(64, MAX_KNN)).unwrap_or(64);
        let limit = i64::try_from(k).unwrap_or(i64::MAX);
        let sql = format!(
            "WITH hits AS (SELECT rowid AS id, distance FROM task_decisions_vec \
               WHERE embedding MATCH ?2 AND k = ?3 AND scope = ?4) \
             SELECT {COLS}, hits.distance FROM hits JOIN task_decisions e ON e.id = hits.id \
             WHERE {} AND e.scope = ?4 ORDER BY hits.distance, e.id DESC LIMIT ?5",
            visible_filter()
        );
        let blob = vector::blob(query_vec);
        let mut found: Vec<(f64, Entry)> = Vec::new();
        for partition in partitions {
            let rows = self.sql().query(
                &sql,
                args![scope_text.clone(), blob.clone(), pool, partition, limit],
            )?;
            for row in rows {
                found.push((row.get(18)?, row_to_entry(&row)?));
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.id.cmp(&a.1.id)));
        Ok(found.into_iter().take(k).map(|(_, e)| e).collect())
    }

    /// Keyword search: what [`search`](Self::search) is with search by
    /// meaning off.
    fn keyword(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
        let Some(m) = match_expr(query) else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {COLS} FROM task_decisions_fts \
             JOIN task_decisions e ON e.id = task_decisions_fts.rowid \
             WHERE task_decisions_fts MATCH ?2 AND {} \
             ORDER BY bm25(task_decisions_fts), e.id DESC LIMIT ?3",
            visible_filter()
        );
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.sql()
            .query(&sql, args![scope.to_string(), m, limit])?
            .iter()
            .map(row_to_entry)
            .collect()
    }

    fn query(&self, filter: &str, scope: Option<&Scope>) -> Result<Vec<Entry>, Error> {
        let sql = format!(
            "SELECT {COLS} FROM task_decisions e WHERE {filter} \
             ORDER BY COALESCE(e.decided_at, e.created_at) DESC, e.id DESC"
        );
        let args = match scope {
            Some(s) => args![s.to_string()],
            None => args![],
        };
        self.sql()
            .query(&sql, args)?
            .iter()
            .map(row_to_entry)
            .collect()
    }

    /// Distinguish "no such entry" from the more specific failure.
    fn missing_or(&self, id: i64, otherwise: Error) -> Result<Error, Error> {
        Ok(if self.load(id)?.is_none() {
            Error::NotFound(id)
        } else {
            otherwise
        })
    }

    fn load(&self, id: i64) -> Result<Option<Entry>, Error> {
        let sql = format!("SELECT {COLS} FROM task_decisions e WHERE e.id = ?1");
        self.sql()
            .optional(&sql, args![id])?
            .map(|row| row_to_entry(&row))
            .transpose()
    }
}

/// Why `olds` can't be replaced (by entry `by`, when it exists yet): each
/// must exist and be active, and none may be `by` itself. Reads only, so a
/// refusal leaves the transaction with nothing written.
async fn refuse_replacing(
    tx: &mut toasty::Transaction<'_>,
    olds: &[i64],
    by: Option<i64>,
) -> toasty::Result<Option<Error>> {
    for &old in olds {
        if Some(old) == by {
            return Ok(Some(Error::SelfReplace(old)));
        }
        match TaskDecision::filter_by_id(old)
            .first()
            .exec(&mut *tx)
            .await?
        {
            None => return Ok(Some(Error::NotFound(old))),
            Some(prev) if Status::parse(&prev.status).ok() != Some(Status::Active) => {
                return Ok(Some(Error::NotActive(old)))
            }
            Some(_) => {}
        }
    }
    Ok(None)
}

/// Marks each of `olds` replaced by entry `by`, and closes the open flags on
/// them: a flag on wording that no longer applies has nothing left to decide.
async fn mark_replaced(
    tx: &mut toasty::Transaction<'_>,
    olds: &[i64],
    by: i64,
    now_ms: i64,
) -> toasty::Result<()> {
    let flag = DecisionFlag::fields();
    for &old in olds {
        TaskDecision::filter_by_id(old)
            .update()
            .status(Status::Replaced.as_str())
            .replaced_by(Some(by))
            .decided_at(Some(now_ms))
            .updated_at(now_ms)
            .exec(&mut *tx)
            .await?;
        DecisionFlag::filter(flag.entry().eq(old).and(flag.dismissed().eq(false)))
            .update()
            .dismissed(true)
            .exec(&mut *tx)
            .await?;
    }
    // A replaced entry is never shown again, so it is never searched for.
    vector::remove(tx, olds).await?;
    Ok(())
}

/// The fingerprint of what a review card would show of `row` now.
fn shown_now(row: &TaskDecision) -> ShownHash {
    ShownHash::of(
        &row.title,
        &row.statement,
        &row.why,
        row.user_answer.as_deref(),
    )
}

fn row_to_entry(r: &SqlRow) -> Result<Entry, Error> {
    let open_flags: i64 = r.get(17)?;
    Ok(Entry {
        id: r.get(0)?,
        kind: Kind::parse(&r.get::<String>(1)?)?,
        scope: Scope::parse(&r.get::<String>(2)?)?,
        status: Status::parse(&r.get::<String>(3)?)?,
        title: r.get(4)?,
        text: r.get(5)?,
        why: r.get(6)?,
        user_answer: r.get(7)?,
        source: Source {
            session: r.get(8)?,
            question_ref: r.get(9)?,
            user_turn: r.get(10)?,
            import: r.get(11)?,
        },
        pinned: r.get(12)?,
        supersedes: r.get(13)?,
        replaced_by: r.get(14)?,
        created_ms: r.get(15)?,
        decided_ms: r.get(16)?,
        open_flags: u32::try_from(open_flags).unwrap_or(u32::MAX),
    })
}

/// An FTS5 MATCH expression for free text: each whitespace-separated word as
/// a quoted phrase, OR-ed. Words with no letter or digit (`*`, `-`, `"`) have
/// no tokens to match, so they are dropped. `None` when nothing is left.
fn match_expr(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" OR "))
    }
}
