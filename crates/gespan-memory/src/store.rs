use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::entry::{
    Config, Edit, Entry, Flag, Kind, NewEntry, Scope, Source, Status, MAX_FIELD_CHARS,
};
use crate::{schema, Error};

/// `entries` columns in the order `row_to_entry` reads them, with the open
/// flag count last.
pub(crate) const COLS: &str =
    "e.id, e.kind, e.scope, e.status, e.title, e.text, e.why, e.user_answer, \
    e.session, e.question_ref, e.user_turn, e.import_src, e.pinned, e.supersedes, \
    e.replaced_by, e.created_ms, e.decided_ms, \
    (SELECT COUNT(*) FROM flags f WHERE f.entry = e.id AND f.dismissed = 0)";

const HAS_USER_SOURCE: &str =
    "((e.question_ref IS NOT NULL AND e.question_ref <> '') OR (e.user_turn IS NOT NULL AND e.user_turn <> ''))";

/// What an agent may see in `scope` (bound as `?1`): active entries, plus
/// pending ones that carry a user source, for the scope and for global.
/// `visible` and `search` both filter with this, so they cannot drift apart.
pub(crate) fn visible_filter() -> String {
    format!(
        "(e.status = 'active' OR (e.status = 'pending' AND {HAS_USER_SOURCE})) \
         AND e.scope IN (?1, 'global')"
    )
}

pub struct Store {
    conn: Mutex<Connection>,
    config: Config,
}

impl Store {
    pub fn open(path: &Path, config: Config) -> Result<Store, Error> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        schema::migrate(&conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
            config,
        })
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Redact, then cap at `MAX_FIELD_CHARS` characters (not bytes).
    fn clean(&self, s: &str) -> String {
        (self.config.redact)(s)
            .chars()
            .take(MAX_FIELD_CHARS)
            .collect()
    }

    pub fn propose(&self, e: NewEntry, now_ms: i64) -> Result<Entry, Error> {
        if e.why.trim().is_empty() {
            return Err(Error::MissingWhy);
        }
        let title = self.clean(&e.title);
        let text = self.clean(&e.text);
        let why = self.clean(&e.why);
        let answer = e.user_answer.as_deref().map(|a| self.clean(a));
        // Source text is user-supplied free text too: redact and cap it, then
        // treat what is left empty as absent.
        let source = |s: &Option<String>| {
            s.as_deref()
                .map(|v| self.clean(v))
                .filter(|v| !v.is_empty())
        };
        let conn = self.conn();
        conn.execute(
            "INSERT INTO entries (kind, scope, status, title, text, why, user_answer, session, \
             question_ref, user_turn, import_src, created_ms) \
             VALUES (?1, ?2, 'pending', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                e.kind.as_str(),
                e.scope.to_string(),
                title,
                text,
                why,
                answer,
                e.source.session,
                source(&e.source.question_ref),
                source(&e.source.user_turn),
                source(&e.source.import),
                now_ms
            ],
        )?;
        let id = conn.last_insert_rowid();
        load(&conn, id)?.ok_or(Error::NotFound(id))
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
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let current = load(&tx, id)?.ok_or(Error::NotFound(id))?;
        if current.status != Status::Pending {
            return Err(Error::NotPending(id));
        }
        for &old in replaces {
            if old == id {
                return Err(Error::SelfReplace(id));
            }
            let prev = load(&tx, old)?.ok_or(Error::NotFound(old))?;
            if prev.status != Status::Active {
                return Err(Error::NotActive(old));
            }
        }
        for &old in replaces {
            tx.execute(
                "UPDATE entries SET status = 'replaced', replaced_by = ?1, decided_ms = ?2 \
                 WHERE id = ?3",
                params![id, now_ms, old],
            )?;
        }
        tx.execute(
            "UPDATE entries SET status = 'active', pinned = ?1, decided_ms = ?2, supersedes = ?3 \
             WHERE id = ?4",
            params![pinned, now_ms, replaces.first(), id],
        )?;
        if let Some(ed) = edit {
            tx.execute(
                "UPDATE entries SET title = ?1, text = ?2, why = ?3 WHERE id = ?4",
                params![ed.title, ed.text, ed.why, id],
            )?;
        }
        let out = load(&tx, id)?.ok_or(Error::NotFound(id))?;
        tx.commit()?;
        Ok(out)
    }

    /// Rewrite a pending entry's title, text and why in place (redacted and
    /// capped like a new one). Anything but a pending entry is refused.
    pub fn revise_pending(&self, id: i64, edit: Edit) -> Result<Entry, Error> {
        if edit.why.trim().is_empty() {
            return Err(Error::MissingWhy);
        }
        let (title, text, why) = (
            self.clean(&edit.title),
            self.clean(&edit.text),
            self.clean(&edit.why),
        );
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE entries SET title = ?1, text = ?2, why = ?3 \
             WHERE id = ?4 AND status = 'pending'",
            params![title, text, why, id],
        )?;
        if n == 0 {
            return Err(missing_or(&conn, id, Error::NotPending(id))?);
        }
        load(&conn, id)?.ok_or(Error::NotFound(id))
    }

    pub fn reject(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE entries SET status = 'rejected', decided_ms = ?1 \
             WHERE id = ?2 AND status = 'pending'",
            params![now_ms, id],
        )?;
        if n == 0 {
            return Err(missing_or(&conn, id, Error::NotPending(id))?);
        }
        Ok(())
    }

    pub fn archive(&self, id: i64, now_ms: i64) -> Result<(), Error> {
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE entries SET status = 'archived', decided_ms = ?1 \
             WHERE id = ?2 AND status = 'active'",
            params![now_ms, id],
        )?;
        if n == 0 {
            return Err(missing_or(&conn, id, Error::NotActive(id))?);
        }
        Ok(())
    }

    pub fn flag(&self, id: i64, session: i64, reason: &str, now_ms: i64) -> Result<Flag, Error> {
        let reason = self.clean(reason);
        let conn = self.conn();
        if load(&conn, id)?.is_none() {
            return Err(Error::NotFound(id));
        }
        conn.execute(
            "INSERT INTO flags (entry, session, reason, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![id, session, reason, now_ms],
        )?;
        Ok(Flag {
            id: conn.last_insert_rowid(),
            entry: id,
            session,
            reason,
            created_ms: now_ms,
        })
    }

    pub fn dismiss_flag(&self, flag: i64) -> Result<(), Error> {
        let n = self
            .conn()
            .execute("UPDATE flags SET dismissed = 1 WHERE id = ?1", [flag])?;
        if n == 0 {
            return Err(Error::FlagNotFound(flag));
        }
        Ok(())
    }

    /// The entry and the entries it replaced (directly or through earlier
    /// replacements), newest first.
    pub fn get(&self, id: i64) -> Result<Option<(Entry, Vec<Entry>)>, Error> {
        let conn = self.conn();
        let Some(entry) = load(&conn, id)? else {
            return Ok(None);
        };
        let mut history: Vec<Entry> = Vec::new();
        let mut frontier = vec![id];
        while let Some(next) = frontier.pop() {
            let sql = format!("SELECT {COLS} FROM entries e WHERE e.replaced_by = ?1");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([next], row_to_entry)?;
            for r in rows {
                let e = r??;
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
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, entry, session, reason, created_ms FROM flags \
             WHERE dismissed = 0 ORDER BY created_ms, id",
        )?;
        let flags = stmt
            .query_map([], |r| {
                Ok(Flag {
                    id: r.get(0)?,
                    entry: r.get(1)?,
                    session: r.get(2)?,
                    reason: r.get(3)?,
                    created_ms: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(flags.len());
        for f in flags {
            let e = load(&conn, f.entry)?.ok_or(Error::NotFound(f.entry))?;
            out.push((f, e));
        }
        Ok(out)
    }

    fn query(&self, filter: &str, scope: Option<&Scope>) -> Result<Vec<Entry>, Error> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {COLS} FROM entries e WHERE {filter} \
             ORDER BY COALESCE(e.decided_ms, e.created_ms) DESC, e.id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = match scope {
            Some(s) => stmt.query_map([s.to_string()], row_to_entry)?,
            None => stmt.query_map([], row_to_entry)?,
        };
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }
}

/// Distinguish "no such entry" from the more specific failure.
fn missing_or(conn: &Connection, id: i64, otherwise: Error) -> Result<Error, Error> {
    Ok(if load(conn, id)?.is_none() {
        Error::NotFound(id)
    } else {
        otherwise
    })
}

fn load(conn: &Connection, id: i64) -> Result<Option<Entry>, Error> {
    let sql = format!("SELECT {COLS} FROM entries e WHERE e.id = ?1");
    match conn.query_row(&sql, [id], row_to_entry).optional()? {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

/// The outer error is SQLite's; the inner is a stored value this build does
/// not recognise.
pub(crate) fn row_to_entry(r: &Row<'_>) -> rusqlite::Result<Result<Entry, Error>> {
    let kind: String = r.get(1)?;
    let scope: String = r.get(2)?;
    let status: String = r.get(3)?;
    let entry = (|| {
        Ok::<_, rusqlite::Error>(Entry {
            id: r.get(0)?,
            kind: Kind::Preference,
            scope: Scope::Global,
            status: Status::Pending,
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
            open_flags: r.get(17)?,
        })
    })()?;
    Ok((|| {
        Ok(Entry {
            kind: Kind::parse(&kind)?,
            scope: Scope::parse(&scope)?,
            status: Status::parse(&status)?,
            ..entry
        })
    })())
}
