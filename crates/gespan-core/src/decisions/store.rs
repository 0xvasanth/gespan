//! The decision log's store. Its entries are rows of `task_decisions` in
//! gespan.db, the table My tasks saves its project decisions to, so the two
//! are one list. [`DecisionStore`] has its own Toasty handle on the file
//! (the app's `Store` keeps another); SQLite's write-ahead log and the busy
//! timeout every ORM connection sets let the two share it.
//!
//! Times always come from the caller, never the clock, so every operation is
//! deterministic under test.
use std::path::Path;

use super::entry::{
    Config, Edit, Entry, Error, Flag, Kind, NewEntry, Scope, ShownHash, Source, Status,
};
use crate::store::db::{args, Sql, SqlRow};
use crate::store::my_tasks::TaskDecision;

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
///
/// Task 10's `vec0` table for semantic search goes here too, the same way.
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
}

impl DecisionStore {
    /// Opens gespan.db at `path` the way the app's store does (adopting and
    /// migrating it, and marking a new file as past the Tauri migrations), so
    /// it does not matter which of the two opens the file first. `:memory:`
    /// opens a private in-memory database.
    pub fn open(path: &Path, config: Config) -> Result<DecisionStore, Error> {
        let fresh = crate::orm::is_memory(path) || !path.exists();
        let db = crate::store::shared::open_database(path, fresh)?;
        Ok(DecisionStore { db, config })
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
        let row = self.db.run(|mut db| async move {
            TaskDecision::create()
                .project(project)
                .statement(text)
                .why(why)
                .created_at(now_ms)
                .updated_at(now_ms)
                .archived_at(None)
                .kind(kind)
                .status(Status::Pending.as_str())
                .scope(scope)
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
                .exec(&mut db)
                .await
        })?;
        self.load(row.id)?.ok_or(Error::NotFound(row.id))
    }

    /// Proposes an imported entry unless its source already has one with the
    /// same text, whatever became of it (rejected and archived rows count, so
    /// a rejected import never comes back). `None` when it was already there.
    pub fn propose_import(&self, e: NewEntry, now_ms: i64) -> Result<Option<Entry>, Error> {
        let src = e.source.import.as_deref().map(|v| self.clean(v));
        let text = self.clean(&e.text);
        let seen = self.sql().query(
            "SELECT 1 FROM task_decisions WHERE import_src = ?1 AND statement = ?2 LIMIT 1",
            args![src.unwrap_or_default(), text],
        )?;
        if !seen.is_empty() {
            return Ok(None);
        }
        self.propose(e, now_ms).map(Some)
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
                    .exec(tx)
                    .await?;
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
                update.exec(tx).await?;
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
                    .scope(old.scope)
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
        let n = self.sql().execute(
            "UPDATE task_decisions SET title = ?1, statement = ?2, why = ?3, user_answer = ?4, \
             question_ref = ?5, user_turn = ?6 \
             WHERE id = ?7 AND status = 'pending'",
            args![
                self.clean(&edit.title),
                self.clean(&edit.text),
                self.clean(&edit.why),
                user_answer.map(|a| self.clean(a)),
                self.clean_source(&source.question_ref),
                self.clean_source(&source.user_turn),
                id
            ],
        )?;
        if n == 0 {
            return Err(self.missing_or(id, Error::NotPending(id))?);
        }
        self.load(id)?.ok_or(Error::NotFound(id))
    }

    pub fn reject(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        let n = self.sql().execute(
            "UPDATE task_decisions SET status = 'rejected', decided_at = ?1, updated_at = ?1 \
             WHERE id = ?2 AND status = 'pending'",
            args![now_ms, id],
        )?;
        if n == 0 {
            return Err(self.missing_or(id, Error::NotPending(id))?);
        }
        Ok(())
    }

    /// Archives an active entry. It leaves My tasks' Decisions list too,
    /// which reads `archived_at`.
    pub fn archive(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        let n = self.sql().execute(
            "UPDATE task_decisions SET status = 'archived', decided_at = ?1, \
             archived_at = ?1, updated_at = ?1 \
             WHERE id = ?2 AND status = 'active'",
            args![now_ms, id],
        )?;
        if n == 0 {
            return Err(self.missing_or(id, Error::NotActive(id))?);
        }
        Ok(())
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

    /// Keyword search over title, text and why, best match first. Sees exactly
    /// what `visible` sees (same filter), so a rejected, replaced or
    /// archived entry, or another project's entry, is never returned.
    ///
    /// The query is arbitrary text. Every word becomes a quoted phrase, so
    /// FTS5 operators and syntax in it are plain words and cannot make the
    /// search fail. Words are OR-ed, and bm25 ranks entries matching more of
    /// them (or matching them more often) higher. A query with no usable
    /// words returns nothing.
    pub fn search(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
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
