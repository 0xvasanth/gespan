//! My tasks' history: each request that waited on the user with the user's
//! answer, and the project decisions saved from those answers.
use super::Store;
use crate::tasks::{Kind, SavedDecision, Status, TaskRecord};

/// The most past tasks the Answered tab lists.
pub const PAST_TASKS_LIMIT: usize = 500;

/// A request that waited on the user, with the answer once there is one.
#[derive(Debug, toasty::Model)]
#[table = "my_tasks"]
#[index(name = "my_tasks_closed_idx", status, closed_at)]
#[index(name = "my_tasks_decision_idx", decision_id)]
pub(crate) struct MyTask {
    #[key]
    key: String,
    kind: String,
    project: String,
    origin: String,
    source_label: String,
    headline: String,
    request_json: String,
    asked_at: i64,
    status: String,
    answer: Option<String>,
    answer_json: Option<String>,
    closed_at: Option<i64>,
    decision_id: Option<i64>,
}

/// A project decision saved from the answers that point at it.
#[derive(Debug, toasty::Model)]
#[table = "task_decisions"]
pub(crate) struct TaskDecision {
    #[key]
    #[auto]
    id: i64,
    project: String,
    statement: String,
    why: String,
    created_at: i64,
    updated_at: i64,
    archived_at: Option<i64>,
}

/// The hand-made tables older builds created, for adoption tests.
#[cfg(test)]
pub(crate) const LEGACY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS task_decisions (
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
     CREATE INDEX IF NOT EXISTS my_tasks_decision_idx ON my_tasks (decision_id);";

/// Adoption's legacy step. These tables only ever had the one hand-made
/// shape above, so there is nothing to bring up to date.
pub(crate) fn legacy_upgrade(_conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    Ok(())
}

fn ms(at: u64) -> i64 {
    i64::try_from(at).unwrap_or(i64::MAX)
}

fn at(ms: i64) -> u64 {
    ms.max(0) as u64
}

impl MyTask {
    fn record(self) -> Option<TaskRecord> {
        let json = |text: &str| serde_json::from_str(text).ok();
        Some(TaskRecord {
            kind: Kind::from_id(&self.kind)?,
            status: Status::from_id(&self.status)?,
            request: json(&self.request_json).unwrap_or_default(),
            answer_value: self.answer_json.as_deref().and_then(json),
            key: self.key,
            project: self.project,
            origin: self.origin,
            source_label: self.source_label,
            headline: self.headline,
            asked_at: at(self.asked_at),
            answer: self.answer,
            closed_at: self.closed_at.map(at),
            decision: self.decision_id,
        })
    }
}

fn records(rows: Vec<MyTask>) -> Vec<TaskRecord> {
    rows.into_iter().filter_map(MyTask::record).collect()
}

impl Store {
    /// Keeps a request that started waiting. One already answered is left
    /// as it is; one withdrawn is open again, since it is waiting again (a
    /// Pilot's choices outlive a restart).
    pub fn open_task(&self, task: &TaskRecord) -> Result<(), String> {
        // A conditional upsert (`ON CONFLICT ... WHERE`), which Toasty 0.11's
        // typed upsert cannot express: raw SQL on the same database.
        let insert = toasty::sql::statement(
            "INSERT INTO my_tasks (key, kind, project, origin, source_label,
               headline, request_json, asked_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open')
             ON CONFLICT(key) DO UPDATE SET status = 'open', closed_at = NULL
               WHERE status = 'withdrawn'",
        )
        .bind(task.key.clone())
        .bind(task.kind.id())
        .bind(task.project.clone())
        .bind(task.origin.clone())
        .bind(task.source_label.clone())
        .bind(task.headline.clone())
        .bind(task.request.to_string())
        .bind(ms(task.asked_at));
        self.orm()?
            .run(|mut db| async move { insert.exec(&mut db).await })?;
        Ok(())
    }

    /// Keeps the user's answer with its request. The request may not have
    /// been kept yet (answered within a refresh), so it is written whole;
    /// one already kept keeps the time it started waiting.
    pub fn answer_task(&self, task: &TaskRecord) -> Result<(), String> {
        let task = task.clone();
        self.orm()?.transaction(|tx| {
            Box::pin(async move {
                let answer_json = task.answer_value.as_ref().map(|value| value.to_string());
                let closed_at = Some(ms(task.closed_at.unwrap_or(task.asked_at)));
                match MyTask::filter_by_key(&task.key).first().exec(tx).await? {
                    Some(mut kept) => {
                        kept.update()
                            .status("answered")
                            .answer(task.answer)
                            .answer_json(answer_json)
                            .closed_at(closed_at)
                            .exec(tx)
                            .await?
                    }
                    None => {
                        MyTask::create()
                            .key(task.key)
                            .kind(task.kind.id())
                            .project(task.project)
                            .origin(task.origin)
                            .source_label(task.source_label)
                            .headline(task.headline)
                            .request_json(task.request.to_string())
                            .asked_at(ms(task.asked_at))
                            .status("answered")
                            .answer(task.answer)
                            .answer_json(answer_json)
                            .closed_at(closed_at)
                            .exec(tx)
                            .await?;
                    }
                }
                Ok(())
            })
        })
    }

    /// Marks requests that stopped waiting with no answer. An answered one
    /// stays answered.
    pub fn withdraw_tasks(&self, keys: &[String], at: u64) -> Result<(), String> {
        let keys = keys.to_vec();
        self.orm()?.transaction(|tx| {
            Box::pin(async move {
                let task = MyTask::fields();
                for key in keys {
                    MyTask::filter(task.key().eq(key).and(task.status().eq("open")))
                        .update()
                        .status("withdrawn")
                        .closed_at(Some(ms(at)))
                        .exec(tx)
                        .await?;
                }
                Ok(())
            })
        })
    }

    /// Requests left open by an earlier run of Gespan: their agents are gone.
    pub fn withdraw_tasks_before(&self, at: u64) -> Result<usize, String> {
        // Raw: the caller needs the count, and Toasty 0.11's typed update
        // returns none.
        let withdraw = toasty::sql::statement(
            "UPDATE my_tasks SET status = 'withdrawn', closed_at = ?1
             WHERE status = 'open' AND asked_at < ?1",
        )
        .bind(ms(at));
        let count = self
            .orm()?
            .run(|mut db| async move { withdraw.exec(&mut db).await })?;
        Ok(count as usize)
    }

    pub fn task(&self, key: &str) -> Result<Option<TaskRecord>, String> {
        let row = self
            .orm()?
            .run(|mut db| async move { MyTask::filter_by_key(key).first().exec(&mut db).await })?;
        Ok(row.and_then(MyTask::record))
    }

    /// Answered and withdrawn requests, most recently closed first.
    pub fn past_tasks(&self) -> Result<Vec<TaskRecord>, String> {
        let task = MyTask::fields();
        let rows = self.orm()?.run(|mut db| async move {
            MyTask::filter(task.status().ne("open"))
                .order_by((task.closed_at().desc(), task.asked_at().desc()))
                .limit(PAST_TASKS_LIMIT)
                .exec(&mut db)
                .await
        })?;
        Ok(records(rows))
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
        let (project, statement, why) = (project.to_owned(), statement.to_owned(), why.trim());
        let (why, sources) = (why.to_owned(), sources.to_vec());
        self.orm()?.transaction(|tx| {
            Box::pin(async move {
                let decision = TaskDecision::create()
                    .project(project)
                    .statement(statement)
                    .why(why)
                    .created_at(ms(at))
                    .updated_at(ms(at))
                    .exec(tx)
                    .await?;
                for key in sources {
                    MyTask::filter_by_key(key)
                        .update()
                        .decision_id(Some(decision.id))
                        .exec(tx)
                        .await?;
                }
                Ok(decision.id)
            })
        })
    }

    pub fn edit_decision(&self, id: i64, statement: &str, at: u64) -> Result<(), String> {
        let statement = statement.trim();
        if statement.is_empty() {
            return Err("A decision needs a statement".into());
        }
        self.orm()?.run(|mut db| async move {
            TaskDecision::filter_by_id(id)
                .update()
                .statement(statement)
                .updated_at(ms(at))
                .exec(&mut db)
                .await
        })
    }

    /// Takes a decision off the list; its answers stay in the history.
    pub fn archive_decision(&self, id: i64, at: u64) -> Result<(), String> {
        self.orm()?.run(|mut db| async move {
            TaskDecision::filter_by_id(id)
                .update()
                .archived_at(Some(ms(at)))
                .exec(&mut db)
                .await
        })
    }

    /// Decisions not archived, most recently changed first, each with the
    /// answers it came from.
    pub fn decisions(&self) -> Result<Vec<SavedDecision>, String> {
        self.orm()?.run(|mut db| async move {
            let decision = TaskDecision::fields();
            let task = MyTask::fields();
            let rows = TaskDecision::filter(decision.archived_at().is_none())
                .order_by((decision.updated_at().desc(), decision.id().desc()))
                .exec(&mut db)
                .await?;
            let mut decisions = Vec::with_capacity(rows.len());
            for row in rows {
                let sources = MyTask::filter(task.decision_id().eq(Some(row.id)))
                    .order_by(task.closed_at().desc())
                    .exec(&mut db)
                    .await?;
                decisions.push(SavedDecision {
                    id: row.id,
                    project: row.project,
                    statement: row.statement,
                    why: row.why,
                    created_at: at(row.created_at),
                    updated_at: at(row.updated_at),
                    sources: records(sources),
                });
            }
            Ok(decisions)
        })
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
