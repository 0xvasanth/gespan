//! The decision log's store. Its entries are rows of `task_decisions` in
//! gespan.db, the table My tasks saves its project decisions to, so the two
//! are one list. [`DecisionStore`] has its own Toasty handle on the file
//! (the app's `Store` keeps another); SQLite's write-ahead log and the busy
//! timeout every ORM connection sets let the two share it.
//!
//! Times always come from the caller, never the clock, so every operation is
//! deterministic under test.
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use toasty::stmt::Expr;
use toasty::Executor;

use super::embed::{Embedder, DIMS};
use super::entry::{
    Config, Edit, Entry, Error, Flag, Kind, NewEntry, Scope, ShownHash, Source, Status,
};
use super::vector::{self, Embedding, Vector};
use crate::store::db::{args, Sql};
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
/// raw SQL, since an FTS5 virtual table and its triggers are beyond what
/// Toasty can model.
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

/// How many ids go into one `IN` list, well under SQLite's bound-parameter
/// limit.
const IN_CHUNK: usize = 500;

/// What an agent may see in `scope`: active entries, plus pending ones that
/// carry a user source (a non-empty `question_ref` or `user_turn`), for the
/// scope and for global. `visible`, `search` and `nearest` all filter with
/// this, so they cannot drift apart.
fn visible_filter(scope: &str) -> Expr<bool> {
    let e = TaskDecision::fields();
    let user_source = e
        .question_ref()
        .is_some()
        .and(e.question_ref().ne(Some(String::new())))
        .or(e
            .user_turn()
            .is_some()
            .and(e.user_turn().ne(Some(String::new()))));
    e.status()
        .eq(Status::Active.as_str())
        .or(e.status().eq(Status::Pending.as_str()).and(user_source))
        .and(e.scope().in_list([scope.to_string(), "global".to_string()]))
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
        self.switch_embedder(embedder.clone())?;
        self.reembed_or_turn_off(&embedder).map(drop)
    }

    /// The first half of turning search by meaning on: records `embedder`'s
    /// model (deleting another model's vectors in the same transaction) and
    /// makes it the one new writes use. Quick; [`reembed_or_turn_off`]
    /// (Self::reembed_or_turn_off) is the slow half.
    pub fn switch_embedder(&self, embedder: Arc<dyn Embedder>) -> Result<(), Error> {
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
        Ok(())
    }

    /// The pass after [`switch_embedder`](Self::switch_embedder):
    /// [`reembed`](Self::reembed). If vectors can't be written, `embedder`
    /// is turned back off (unless another took its place), so proposals,
    /// whose vector is written with them, keep working. The error is logged
    /// and returned.
    pub fn reembed_or_turn_off(&self, embedder: &Arc<dyn Embedder>) -> Result<usize, Error> {
        self.reembed().inspect_err(|error| {
            tracing::warn!(%error, "search by meaning could not embed the log; it stays off");
            let mut slot = self.embedder.write().unwrap_or_else(|e| e.into_inner());
            if slot.as_ref().is_some_and(|now| Arc::ptr_eq(now, embedder)) {
                *slot = None;
            }
        })
    }

    /// Brings the vectors in line with the entries: embeds each active or
    /// pending entry whose vector is missing, from another model, of other
    /// text or in another scope, and deletes the vectors of entries that
    /// can no longer be shown (or no longer exist). Each entry's write is
    /// its own transaction, which re-reads the entry and skips it if it
    /// changed while it was being embedded (the next pass catches it). An
    /// entry the embedder fails on is logged, loses any old vector and is
    /// skipped; the pass goes on. The pass stops when search by meaning is
    /// switched off or to another embedder meanwhile. Returns how many
    /// entries were embedded; does nothing while search by meaning is off.
    /// Embeds one entry at a time: call it off the UI thread.
    pub fn reembed(&self) -> Result<usize, Error> {
        let _one_pass = self.reembedding.lock().unwrap_or_else(|e| e.into_inner());
        let Some(embedder) = self.embedder() else {
            return Ok(0);
        };
        let model = embedder.model_id().to_string();
        let stored = vector::stored(&self.db)?;
        let rows = self.db.run(|mut db| async move {
            let e = TaskDecision::fields();
            TaskDecision::filter(
                e.status()
                    .in_list([Status::Active.as_str(), Status::Pending.as_str()]),
            )
            .order_by(e.id().asc())
            .exec(&mut db)
            .await
        })?;
        let mut keep = std::collections::HashSet::new();
        let mut embedded = 0;
        for row in rows {
            let still_current = self
                .embedder()
                .is_some_and(|now| Arc::ptr_eq(&now, &embedder));
            if !still_current {
                return Ok(embedded);
            }
            let TaskDecision {
                id,
                scope,
                title,
                statement: text,
                why,
                ..
            } = row;
            let fingerprint = vector::fingerprint(&model, &title, &text, &why);
            if stored.get(&id) == Some(&(scope.clone(), fingerprint.clone())) {
                keep.insert(id);
                continue;
            }
            let embedding = match embedder.embed_document(&vector::document(&title, &text, &why)) {
                Ok(embedding) => embedding,
                Err(error) => {
                    // Not kept: its old vector (of other text) goes below.
                    tracing::warn!(id, %error, "a decision log entry could not be embedded");
                    continue;
                }
            };
            keep.insert(id);
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
    /// meaning is on. An embedder failure is logged and gives
    /// [`Embedding::Failed`]: the entry is written without a vector (an old
    /// one, of other text, is removed), and the next
    /// [`reembed`](Self::reembed) adds it.
    fn vector_for(&self, title: &str, text: &str, why: &str) -> Embedding {
        let Some(embedder) = self.embedder() else {
            return Embedding::Off;
        };
        match embedder.embed_document(&vector::document(title, text, why)) {
            Ok(embedding) => Embedding::Vector(Vector {
                embedding,
                fingerprint: vector::fingerprint(embedder.model_id(), title, text, why),
            }),
            Err(error) => {
                tracing::warn!(%error, "a decision log entry could not be embedded");
                Embedding::Failed
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
        let rows = self.db.run(|mut db| async move {
            TaskDecision::filter(TaskDecision::fields().scope().starts_with("project:"))
                .exec(&mut db)
                .await
        })?;
        let projects: BTreeSet<String> = rows
            .into_iter()
            .filter(|row| row.scope == format!("project:{}", row.project))
            .map(|row| row.project)
            .collect();
        let mut moved = 0;
        for project in projects {
            let scope = scope_of(Path::new(&project)).to_string();
            let keyed = format!("project:{project}");
            if scope == keyed {
                continue;
            }
            // The count and the update are one transaction, so the count is
            // what the update changed.
            moved += self.db.transaction(move |tx| {
                Box::pin(async move {
                    let e = TaskDecision::fields();
                    let still_keyed = || {
                        TaskDecision::filter(
                            e.project()
                                .eq(project.clone())
                                .and(e.scope().eq(keyed.clone())),
                        )
                    };
                    let n = still_keyed().count().exec(&mut *tx).await?;
                    if n > 0 {
                        still_keyed().update().scope(scope).exec(&mut *tx).await?;
                    }
                    Ok(usize::try_from(n).unwrap_or(usize::MAX))
                })
            })?;
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
                v.apply(tx, row.id, &scope).await?;
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
                v.apply(tx, row.id, &scope).await?;
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
        let v = edit.as_ref().map_or(Embedding::Off, |ed| {
            self.vector_for(&ed.title, &ed.text, &ed.why)
        });
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
                v.apply(tx, id, &scope).await?;
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
                v.apply(tx, successor.id, &old.scope).await?;
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
        let answer = user_answer.map(|a| self.clean(a));
        let question_ref = self.clean_source(&source.question_ref);
        let user_turn = self.clean_source(&source.user_turn);
        // A compare-and-set: the transaction holds the write lock, so the
        // pending row read here is the row updated.
        let revised = self.db.transaction(move |tx| {
            Box::pin(async move {
                let Some(mut row) = in_status(tx, id, Status::Pending).await? else {
                    return Ok(false);
                };
                row.update()
                    .title(title)
                    .statement(text)
                    .why(why)
                    .user_answer(answer)
                    .question_ref(question_ref)
                    .user_turn(user_turn)
                    .exec(&mut *tx)
                    .await?;
                v.apply(tx, id, &row.scope).await?;
                Ok(true)
            })
        })?;
        if !revised {
            return Err(self.missing_or(id, Error::NotPending(id))?);
        }
        self.load(id)?.ok_or(Error::NotFound(id))
    }

    pub fn reject(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        if !self.leave_with_vector(id, Status::Pending, Status::Rejected, now_ms)? {
            return Err(self.missing_or(id, Error::NotPending(id))?);
        }
        Ok(())
    }

    /// Archives an active entry. It leaves My tasks' Decisions list too,
    /// which reads `archived_at`.
    pub fn archive(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        if !self.leave_with_vector(id, Status::Active, Status::Archived, now_ms)? {
            return Err(self.missing_or(id, Error::NotActive(id))?);
        }
        Ok(())
    }

    /// Moves entry `id` from status `from` to `to` (rejected or archived),
    /// out of what can be shown, stamping it decided (and archived, for
    /// `to` archived) at `now_ms`, and deletes its vector in the same
    /// transaction. False, with nothing written, when `id` is not in `from`.
    fn leave_with_vector(
        &self,
        id: i64,
        from: Status,
        to: Status,
        now_ms: i64,
    ) -> Result<bool, Error> {
        // A compare-and-set: the transaction holds the write lock, so the row
        // read in `from` is the row updated.
        Ok(self.db.transaction(move |tx| {
            Box::pin(async move {
                let Some(mut row) = in_status(tx, id, from).await? else {
                    return Ok(false);
                };
                let mut update = row
                    .update()
                    .status(to.as_str())
                    .decided_at(Some(now_ms))
                    .updated_at(now_ms);
                if to == Status::Archived {
                    update = update.archived_at(Some(now_ms));
                }
                update.exec(&mut *tx).await?;
                vector::remove(tx, &[id]).await?;
                Ok(true)
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
        // The read and the update are one transaction, so a flag found here
        // is the flag dismissed.
        let found = self.db.transaction(move |tx| {
            Box::pin(async move {
                if DecisionFlag::filter_by_id(flag)
                    .first()
                    .exec(&mut *tx)
                    .await?
                    .is_none()
                {
                    return Ok(false);
                }
                DecisionFlag::filter_by_id(flag)
                    .update()
                    .dismissed(true)
                    .exec(&mut *tx)
                    .await?;
                Ok(true)
            })
        })?;
        if !found {
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
            let replaced = TaskDecision::fields().replaced_by().eq(Some(next));
            for e in self.entries(replaced)? {
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
        self.newest_first(visible_filter(&scope.to_string()))
    }

    /// Active entries for the scope and for global, newest decided first.
    pub fn active(&self, scope: &Scope) -> Result<Vec<Entry>, Error> {
        let e = TaskDecision::fields();
        self.newest_first(
            e.status()
                .eq(Status::Active.as_str())
                .and(e.scope().in_list([scope.to_string(), "global".to_string()])),
        )
    }

    pub fn pending(&self) -> Result<Vec<Entry>, Error> {
        self.newest_first(TaskDecision::fields().status().eq(Status::Pending.as_str()))
    }

    /// Undismissed flags with their entries, oldest first.
    pub fn open_flags(&self) -> Result<Vec<(Flag, Entry)>, Error> {
        let rows = self.db.run(|mut db| async move {
            let f = DecisionFlag::fields();
            DecisionFlag::filter(f.dismissed().eq(false))
                .order_by((f.created_at().asc(), f.id().asc()))
                .exec(&mut db)
                .await
        })?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let flag = Flag {
                id: row.id,
                entry: row.entry,
                session: row.session,
                reason: row.reason,
                created_ms: row.created_at,
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

    /// The entries related to `query_vec`: the [`nearest`](Self::nearest)
    /// `k` within the embedder's [`max_distance`](Embedder::max_distance),
    /// nearest first. Hybrid search keeps keyword hits whatever their
    /// distance, and adds only these by meaning.
    pub fn related(&self, scope: &Scope, query_vec: &[f32], k: usize) -> Result<Vec<Entry>, Error> {
        let Some(embedder) = self.embedder() else {
            return Ok(Vec::new());
        };
        let max = f64::from(embedder.max_distance());
        Ok(self
            .nearest(scope, query_vec, k)?
            .into_iter()
            .take_while(|(_, distance)| *distance <= max)
            .map(|(e, _)| e)
            .collect())
    }

    /// The `k` entries nearest `query_vec` by meaning that `visible` would
    /// show for `scope`, nearest first, with their cosine distances, however
    /// far. Each nearest-neighbour query reads one partition of the vector
    /// table: `scope`'s, then (for a project scope) `global`'s, the
    /// preferences every scope sees. No other scope's vectors are read.
    /// Empty while search by meaning is off.
    pub fn nearest(
        &self,
        scope: &Scope,
        query_vec: &[f32],
        k: usize,
    ) -> Result<Vec<(Entry, f64)>, Error> {
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
        let blob = vector::blob(query_vec);
        let mut found: Vec<(Entry, f64)> = Vec::new();
        for partition in partitions {
            // Entries that can't be shown lose their vectors as they leave,
            // but hidden pending ones keep theirs: when the filter drops
            // enough of the pool to leave fewer than `k`, ask again for a
            // wider one, while the partition holds more than was read.
            let size = vector::partition_size(&self.db, &partition)?;
            let mut pool = k.saturating_mul(2).clamp(64, MAX_KNN);
            let shown = loop {
                let hits = vector::knn(&self.db, &partition, &blob, pool)?;
                // The hits `visible` would show for `scope` that are in this
                // partition, nearest first (newest first on a tie), cut to `k`.
                let in_partition = visible_filter(&scope_text)
                    .and(TaskDecision::fields().scope().eq(partition.clone()));
                let shown = self.ranked(&hits, in_partition, k)?;
                let read_all = i64::try_from(pool).unwrap_or(i64::MAX) >= size;
                if shown.len() >= k || read_all || pool >= MAX_KNN {
                    break shown;
                }
                pool = pool.saturating_mul(8).min(MAX_KNN);
            };
            found.extend(shown);
        }
        found.sort_by(|a, b| a.1.total_cmp(&b.1).then(b.0.id.cmp(&a.0.id)));
        found.truncate(k);
        Ok(found)
    }

    /// Keyword search: what [`search`](Self::search) is with search by
    /// meaning off.
    fn keyword(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
        let Some(m) = match_expr(query) else {
            return Ok(Vec::new());
        };
        // FTS5's MATCH and bm25 on a virtual table, which Toasty cannot
        // model: raw SQL for the matching ids and their ranks. Every match is
        // read, because the visibility filter below must run before the
        // limit, as it did when this was one joined query.
        let hits: Vec<(i64, f64)> = self
            .sql()
            .query(
                "SELECT rowid, bm25(task_decisions_fts) FROM task_decisions_fts \
                 WHERE task_decisions_fts MATCH ?1",
                args![m],
            )?
            .iter()
            .map(|r| Ok((r.get(0)?, r.get(1)?)))
            .collect::<Result<_, String>>()?;
        Ok(self
            .ranked(&hits, visible_filter(&scope.to_string()), limit)?
            .into_iter()
            .map(|(e, _)| e)
            .collect())
    }

    /// The entries of `hits` (ids with a rank, lower is better) that match
    /// `filter`, best rank first and the newest (larger id) first on a tie,
    /// cut to `limit`, with their ranks.
    fn ranked(
        &self,
        hits: &[(i64, f64)],
        filter: Expr<bool>,
        limit: usize,
    ) -> Result<Vec<(Entry, f64)>, Error> {
        let mut by_id: HashMap<i64, Entry> = HashMap::new();
        for chunk in hits.chunks(IN_CHUNK) {
            let ids: Vec<i64> = chunk.iter().map(|(id, _)| *id).collect();
            let filter = filter.clone().and(TaskDecision::fields().id().in_list(ids));
            by_id.extend(self.entries(filter)?.into_iter().map(|e| (e.id, e)));
        }
        let mut out: Vec<(Entry, f64)> = hits
            .iter()
            .filter_map(|&(id, rank)| by_id.remove(&id).map(|e| (e, rank)))
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(b.0.id.cmp(&a.0.id)));
        out.truncate(limit);
        Ok(out)
    }

    /// The entries matching `filter`, most recently decided (or, undecided,
    /// created) first, then the newest first.
    fn newest_first(&self, filter: Expr<bool>) -> Result<Vec<Entry>, Error> {
        let mut entries = self.entries(filter)?;
        entries.sort_by(|a, b| {
            b.decided_ms
                .unwrap_or(b.created_ms)
                .cmp(&a.decided_ms.unwrap_or(a.created_ms))
                .then(b.id.cmp(&a.id))
        });
        Ok(entries)
    }

    /// The entries matching `filter`, with their open flag counts, in no
    /// particular order.
    fn entries(&self, filter: Expr<bool>) -> Result<Vec<Entry>, Error> {
        let (rows, flags) = self.db.run(|mut db| async move {
            let rows = TaskDecision::filter(filter).exec(&mut db).await?;
            let flags = open_flag_counts(&mut db).await?;
            Ok((rows, flags))
        })?;
        rows.into_iter()
            .map(|row| {
                let open = flags.get(&row.id).copied().unwrap_or(0);
                entry_of(row, open)
            })
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
        let found = self.db.run(|mut db| async move {
            let Some(row) = TaskDecision::filter_by_id(id).first().exec(&mut db).await? else {
                return Ok(None);
            };
            let f = DecisionFlag::fields();
            let open = DecisionFlag::filter(f.entry().eq(id).and(f.dismissed().eq(false)))
                .count()
                .exec(&mut db)
                .await?;
            Ok(Some((row, open)))
        })?;
        found
            .map(|(row, open)| entry_of(row, u32::try_from(open).unwrap_or(u32::MAX)))
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

/// Entry `id` while its status is `status`.
async fn in_status(
    tx: &mut toasty::Transaction<'_>,
    id: i64,
    status: Status,
) -> toasty::Result<Option<TaskDecision>> {
    let e = TaskDecision::fields();
    TaskDecision::filter(e.id().eq(id).and(e.status().eq(status.as_str())))
        .first()
        .exec(&mut *tx)
        .await
}

/// Each entry's number of undismissed flags. Open flags are few: the user
/// answers or dismisses them.
async fn open_flag_counts(ex: &mut dyn Executor) -> toasty::Result<HashMap<i64, u32>> {
    let mut counts: HashMap<i64, u32> = HashMap::new();
    for flag in DecisionFlag::filter(DecisionFlag::fields().dismissed().eq(false))
        .exec(ex)
        .await?
    {
        let n = counts.entry(flag.entry).or_default();
        *n = n.saturating_add(1);
    }
    Ok(counts)
}

fn entry_of(row: TaskDecision, open_flags: u32) -> Result<Entry, Error> {
    Ok(Entry {
        id: row.id,
        kind: Kind::parse(&row.kind)?,
        scope: Scope::parse(&row.scope)?,
        status: Status::parse(&row.status)?,
        title: row.title,
        text: row.statement,
        why: row.why,
        user_answer: row.user_answer,
        source: Source {
            session: row.session,
            question_ref: row.question_ref,
            user_turn: row.user_turn,
            import: row.import_src,
        },
        pinned: row.pinned,
        supersedes: row.supersedes,
        replaced_by: row.replaced_by,
        created_ms: row.created_at,
        decided_ms: row.decided_at,
        open_flags,
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
