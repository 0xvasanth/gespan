//! My tasks' history: each request that waited on the user with the user's
//! answer, and the project decisions saved from those answers.
use super::Store;
use crate::tasks::{Kind, SavedDecision, Status, TaskRecord};
use rusqlite::{params, Connection, OptionalExtension, Row};

/// The most past tasks the Answered tab lists.
pub const PAST_TASKS_LIMIT: usize = 500;

pub fn ensure_my_task_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_decisions (
           id INTEGER PRIMARY KEY,
           project TEXT NOT NULL,
           statement TEXT NOT NULL,
           why TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           archived_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS my_tasks (
           key TEXT PRIMARY KEY,
           kind TEXT NOT NULL,
           project TEXT NOT NULL,
           origin TEXT NOT NULL,
           source_label TEXT NOT NULL,
           headline TEXT NOT NULL,
           request_json TEXT NOT NULL,
           asked_at INTEGER NOT NULL,
           status TEXT NOT NULL,
           answer TEXT,
           answer_json TEXT,
           closed_at INTEGER,
           decision_id INTEGER REFERENCES task_decisions(id)
         );
         CREATE INDEX IF NOT EXISTS my_tasks_closed_idx ON my_tasks (status, closed_at DESC);
         CREATE INDEX IF NOT EXISTS my_tasks_decision_idx ON my_tasks (decision_id);",
    )
}

fn ms(at: u64) -> i64 {
    i64::try_from(at).unwrap_or(i64::MAX)
}

const COLUMNS: &str = "key, kind, project, origin, source_label, headline, request_json,
    asked_at, status, answer, answer_json, closed_at, decision_id";

fn record(row: &Row<'_>) -> rusqlite::Result<Option<TaskRecord>> {
    let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
    let (Some(kind), Some(status)) = (
        Kind::from_id(&row.get::<_, String>(1)?),
        Status::from_id(&row.get::<_, String>(8)?),
    ) else {
        return Ok(None);
    };
    Ok(Some(TaskRecord {
        key: row.get(0)?,
        kind,
        project: row.get(2)?,
        origin: row.get(3)?,
        source_label: row.get(4)?,
        headline: row.get(5)?,
        request: json(Some(row.get(6)?)).unwrap_or_default(),
        asked_at: row.get::<_, i64>(7)?.max(0) as u64,
        status,
        answer: row.get(9)?,
        answer_value: json(row.get(10)?),
        closed_at: row.get::<_, Option<i64>>(11)?.map(|at| at.max(0) as u64),
        decision: row.get(12)?,
    }))
}

impl Store {
    fn my_task_tables(&self) -> Result<&Connection, String> {
        ensure_my_task_tables(&self.conn).map_err(|e| e.to_string())?;
        Ok(&self.conn)
    }

    /// Keeps a request that started waiting. One already answered is left
    /// as it is; one withdrawn is open again, since it is waiting again (a
    /// Pilot's choices outlive a restart).
    pub fn open_task(&self, task: &TaskRecord) -> Result<(), String> {
        self.my_task_tables()?
            .execute(
                "INSERT INTO my_tasks (key, kind, project, origin, source_label,
                   headline, request_json, asked_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open')
                 ON CONFLICT(key) DO UPDATE SET status = 'open', closed_at = NULL
                   WHERE status = 'withdrawn'",
                params![
                    task.key,
                    task.kind.id(),
                    task.project,
                    task.origin,
                    task.source_label,
                    task.headline,
                    task.request.to_string(),
                    ms(task.asked_at),
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Keeps the user's answer with its request. The request may not have
    /// been kept yet (answered within a refresh), so it is written whole;
    /// one already kept keeps the time it started waiting.
    pub fn answer_task(&self, task: &TaskRecord) -> Result<(), String> {
        self.my_task_tables()?
            .execute(
                "INSERT INTO my_tasks (key, kind, project, origin, source_label, headline,
                   request_json, asked_at, status, answer, answer_json, closed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'answered', ?9, ?10, ?11)
                 ON CONFLICT(key) DO UPDATE SET status = 'answered',
                   answer = excluded.answer, answer_json = excluded.answer_json,
                   closed_at = excluded.closed_at",
                params![
                    task.key,
                    task.kind.id(),
                    task.project,
                    task.origin,
                    task.source_label,
                    task.headline,
                    task.request.to_string(),
                    ms(task.asked_at),
                    task.answer,
                    task.answer_value.as_ref().map(|value| value.to_string()),
                    ms(task.closed_at.unwrap_or(task.asked_at)),
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Marks requests that stopped waiting with no answer. An answered one
    /// stays answered.
    pub fn withdraw_tasks(&self, keys: &[String], at: u64) -> Result<(), String> {
        let conn = self.my_task_tables()?;
        let tx = super::write_transaction(conn).map_err(|e| e.to_string())?;
        for key in keys {
            tx.execute(
                "UPDATE my_tasks SET status = 'withdrawn', closed_at = ?2
                 WHERE key = ?1 AND status = 'open'",
                params![key, ms(at)],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Requests left open by an earlier run of Gespan: their agents are gone.
    pub fn withdraw_tasks_before(&self, at: u64) -> Result<usize, String> {
        self.my_task_tables()?
            .execute(
                "UPDATE my_tasks SET status = 'withdrawn', closed_at = ?1
                 WHERE status = 'open' AND asked_at < ?1",
                params![ms(at)],
            )
            .map_err(|e| e.to_string())
    }

    pub fn task(&self, key: &str) -> Result<Option<TaskRecord>, String> {
        self.my_task_tables()?
            .query_row(
                &format!("SELECT {COLUMNS} FROM my_tasks WHERE key = ?1"),
                [key],
                record,
            )
            .optional()
            .map(Option::flatten)
            .map_err(|e| e.to_string())
    }

    /// Answered and withdrawn requests, most recently closed first.
    pub fn past_tasks(&self) -> Result<Vec<TaskRecord>, String> {
        let conn = self.my_task_tables()?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM my_tasks WHERE status != 'open'
                 ORDER BY closed_at DESC, asked_at DESC LIMIT ?1"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([PAST_TASKS_LIMIT as i64], record)
            .map_err(|e| e.to_string())?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.extend(row.map_err(|e| e.to_string())?);
        }
        Ok(tasks)
    }

    /// Saves a decision for `project` from the answers under `sources`.
    pub fn save_decision(
        &self,
        project: &str,
        statement: &str,
        why: &str,
        sources: &[String],
        at: u64,
    ) -> Result<i64, String> {
        let statement = statement.trim();
        if statement.is_empty() {
            return Err("A decision needs a statement".into());
        }
        let conn = self.my_task_tables()?;
        let tx = super::write_transaction(conn).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO task_decisions (project, statement, why, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![project, statement, why.trim(), ms(at)],
        )
        .map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        for key in sources {
            tx.execute(
                "UPDATE my_tasks SET decision_id = ?1 WHERE key = ?2",
                params![id, key],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn edit_decision(&self, id: i64, statement: &str, at: u64) -> Result<(), String> {
        let statement = statement.trim();
        if statement.is_empty() {
            return Err("A decision needs a statement".into());
        }
        self.my_task_tables()?
            .execute(
                "UPDATE task_decisions SET statement = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, statement, ms(at)],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Takes a decision off the list; its answers stay in the history.
    pub fn archive_decision(&self, id: i64, at: u64) -> Result<(), String> {
        self.my_task_tables()?
            .execute(
                "UPDATE task_decisions SET archived_at = ?2 WHERE id = ?1",
                params![id, ms(at)],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Decisions not archived, most recently changed first, each with the
    /// answers it came from.
    pub fn decisions(&self) -> Result<Vec<SavedDecision>, String> {
        let conn = self.my_task_tables()?;
        let mut statement = conn
            .prepare(
                "SELECT id, project, statement, why, created_at, updated_at FROM task_decisions
                 WHERE archived_at IS NULL ORDER BY updated_at DESC, id DESC",
            )
            .map_err(|e| e.to_string())?;
        let mut decisions = statement
            .query_map([], |row| {
                Ok(SavedDecision {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    statement: row.get(2)?,
                    why: row.get(3)?,
                    created_at: row.get::<_, i64>(4)?.max(0) as u64,
                    updated_at: row.get::<_, i64>(5)?.max(0) as u64,
                    sources: Vec::new(),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut sources = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM my_tasks WHERE decision_id = ?1
                 ORDER BY closed_at DESC"
            ))
            .map_err(|e| e.to_string())?;
        for decision in &mut decisions {
            for row in sources
                .query_map([decision.id], record)
                .map_err(|e| e.to_string())?
            {
                decision.sources.extend(row.map_err(|e| e.to_string())?);
            }
        }
        Ok(decisions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn store() -> Store {
        Store::open(Path::new(":memory:")).unwrap()
    }

    fn question(key: &str, asked_at: u64) -> TaskRecord {
        TaskRecord::open(
            key,
            Kind::Question,
            "/work/gespan",
            "session:7",
            "Fix fsid",
            "How should fsidCount be kept?",
            json!({"items": [{"question": "How should fsidCount be kept?",
                "options": ["Recount at read time", "Carry K's counter"]}]}),
            asked_at,
        )
    }

    #[test]
    fn an_answer_is_kept_with_its_whole_request() {
        let store = store();
        store.open_task(&question("question:7:q1", 100)).unwrap();
        assert!(store.past_tasks().unwrap().is_empty(), "open is not past");
        let answered = question("question:7:q1", 999).answered(
            "Carry K's counter",
            json!([["Carry K's counter"]]),
            250,
        );
        store.answer_task(&answered).unwrap();
        let [past] = &store.past_tasks().unwrap()[..] else {
            panic!("one past task");
        };
        assert_eq!(past.status, Status::Answered);
        assert_eq!(past.answer.as_deref(), Some("Carry K's counter"));
        assert_eq!(past.answer_value, Some(json!([["Carry K's counter"]])));
        assert_eq!(past.request["items"][0]["options"][1], "Carry K's counter");
        // The wait started when it was first kept, not when it was answered.
        assert_eq!(past.asked_at, 100);
        assert_eq!(past.closed_at, Some(250));
    }

    #[test]
    fn an_answer_given_before_the_request_was_kept_is_kept_whole() {
        let store = store();
        let answered = question("question:7:q2", 100).answered("Recount", json!([]), 120);
        store.answer_task(&answered).unwrap();
        assert_eq!(store.task("question:7:q2").unwrap(), Some(answered));
    }

    #[test]
    fn a_request_seen_again_keeps_its_answer() {
        let store = store();
        let answered = question("question:7:q1", 100).answered("Recount", json!([]), 120);
        store.answer_task(&answered).unwrap();
        store.open_task(&question("question:7:q1", 300)).unwrap();
        assert_eq!(
            store.task("question:7:q1").unwrap().unwrap().status,
            Status::Answered
        );
    }

    #[test]
    fn a_withdrawn_request_seen_waiting_again_is_open() {
        let store = store();
        store.open_task(&question("choice:7:4", 100)).unwrap();
        store.withdraw_tasks_before(200).unwrap();
        store.open_task(&question("choice:7:4", 100)).unwrap();
        let task = store.task("choice:7:4").unwrap().unwrap();
        assert_eq!((task.status, task.closed_at), (Status::Open, None));
    }

    #[test]
    fn only_requests_still_open_are_withdrawn() {
        let store = store();
        store.open_task(&question("question:7:open", 100)).unwrap();
        store
            .answer_task(&question("question:7:done", 100).answered("Yes", json!([]), 110))
            .unwrap();
        store
            .withdraw_tasks(&["question:7:open".into(), "question:7:done".into()], 200)
            .unwrap();
        let status = |key| store.task(key).unwrap().unwrap().status;
        assert_eq!(status("question:7:open"), Status::Withdrawn);
        assert_eq!(status("question:7:done"), Status::Answered);
        // An answer that lands after the request was withdrawn still counts.
        store
            .answer_task(&question("question:7:open", 100).answered("Late", json!([]), 210))
            .unwrap();
        assert_eq!(status("question:7:open"), Status::Answered);
    }

    #[test]
    fn requests_left_open_by_an_earlier_run_are_withdrawn_at_launch() {
        let store = store();
        store.open_task(&question("question:7:old", 100)).unwrap();
        store.open_task(&question("question:7:new", 500)).unwrap();
        assert_eq!(store.withdraw_tasks_before(400).unwrap(), 1);
        assert_eq!(
            store.task("question:7:old").unwrap().unwrap().status,
            Status::Withdrawn
        );
        assert_eq!(
            store.task("question:7:new").unwrap().unwrap().status,
            Status::Open
        );
    }

    #[test]
    fn past_tasks_are_listed_most_recently_closed_first() {
        let store = store();
        for (key, closed) in [
            ("question:7:a", 300),
            ("question:7:b", 100),
            ("question:7:c", 200),
        ] {
            store
                .answer_task(&question(key, 50).answered("x", json!([]), closed))
                .unwrap();
        }
        let keys: Vec<_> = store
            .past_tasks()
            .unwrap()
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(keys, ["question:7:a", "question:7:c", "question:7:b"]);
    }

    #[test]
    fn a_decision_keeps_the_answers_it_came_from() {
        let store = store();
        let answered = question("question:7:q1", 100).answered("Carry K's counter", json!([]), 120);
        assert!(answered.can_remember());
        store.answer_task(&answered).unwrap();
        let id = store
            .save_decision(
                "/work/gespan",
                "  Carry K's fsidCount counter on every write. ",
                "",
                &["question:7:q1".into()],
                130,
            )
            .unwrap();
        let [decision] = &store.decisions().unwrap()[..] else {
            panic!("one decision");
        };
        assert_eq!(decision.id, id);
        assert_eq!(
            decision.statement,
            "Carry K's fsidCount counter on every write."
        );
        assert_eq!(decision.sources.len(), 1);
        assert_eq!(
            decision.sources[0].answer.as_deref(),
            Some("Carry K's counter")
        );
        // Saved once: the answer no longer offers to be remembered.
        let kept = store.task("question:7:q1").unwrap().unwrap();
        assert_eq!(kept.decision, Some(id));
        assert!(!kept.can_remember());
    }

    #[test]
    fn an_empty_decision_is_refused() {
        let store = store();
        assert!(store.save_decision("/p", "   ", "", &[], 1).is_err());
        let id = store.save_decision("/p", "Keep it", "", &[], 1).unwrap();
        assert!(store.edit_decision(id, " ", 2).is_err());
        assert_eq!(store.decisions().unwrap()[0].statement, "Keep it");
    }

    #[test]
    fn an_edited_decision_moves_to_the_top_and_an_archived_one_leaves() {
        let store = store();
        let first = store.save_decision("/p", "First", "", &[], 1).unwrap();
        let second = store.save_decision("/p", "Second", "", &[], 2).unwrap();
        store.edit_decision(first, "First, edited", 3).unwrap();
        let statements: Vec<_> = store
            .decisions()
            .unwrap()
            .into_iter()
            .map(|d| d.statement)
            .collect();
        assert_eq!(statements, ["First, edited", "Second"]);
        store.archive_decision(second, 4).unwrap();
        assert_eq!(store.decisions().unwrap().len(), 1);
    }

    #[test]
    fn tasks_and_decisions_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        {
            let store = Store::open(&path).unwrap();
            store
                .answer_task(&question("question:7:q1", 100).answered("Recount", json!([]), 120))
                .unwrap();
            store
                .save_decision(
                    "/work/gespan",
                    "Recount",
                    "",
                    &["question:7:q1".into()],
                    130,
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.past_tasks().unwrap().len(), 1);
        assert_eq!(store.decisions().unwrap()[0].sources.len(), 1);
    }

    #[test]
    fn a_suggested_decision_joins_the_question_and_the_answer() {
        let answered = question("question:7:q1", 1).answered("Carry K's counter", json!([]), 2);
        assert_eq!(
            answered.suggested_statement(),
            "How should fsidCount be kept: Carry K's counter"
        );
        let approval = TaskRecord::open(
            "a",
            Kind::Approval,
            "/p",
            "",
            "",
            "Allow Bash?",
            json!({}),
            1,
        )
        .answered("Allowed once", json!("allow"), 2);
        assert!(!approval.can_remember(), "approvals are not decisions");
    }
}
