//! Flow, run and visit storage in `rebase.db`. Definitions and records are JSON
//! blobs with mirrored indexed columns, like `automations`.
use super::definition::{validate_flow, Flow};
use super::run::{FlowRun, FlowVisit};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// Exact DDL, pinned by `tests/shared_contracts.rs` so the Tauri twin can match it.
pub const FLOWS_DDL: &str = "CREATE TABLE IF NOT EXISTS flows (
  id TEXT PRIMARY KEY,
  definition_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
)";
pub const FLOW_RUNS_DDL: &str = "CREATE TABLE IF NOT EXISTS flow_runs (
  id TEXT PRIMARY KEY,
  flow_id TEXT NOT NULL REFERENCES flows(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  run_json TEXT NOT NULL
)";
pub const FLOW_VISITS_DDL: &str = "CREATE TABLE IF NOT EXISTS flow_visits (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES flow_runs(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  state_id TEXT NOT NULL,
  status TEXT NOT NULL,
  visit_json TEXT NOT NULL,
  UNIQUE (run_id, seq)
)";
pub const FLOW_INDEXES_DDL: [&str; 3] = [
    "CREATE INDEX IF NOT EXISTS flow_runs_history_idx ON flow_runs (flow_id, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS flow_runs_status_idx ON flow_runs (status)",
    "CREATE INDEX IF NOT EXISTS flow_visits_order_idx ON flow_visits (run_id, seq)",
];

const MAX_RUNS: usize = 50;
const ACTIVE: &str = "status IN ('running', 'waiting-approval')";

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    for ddl in [FLOWS_DDL, FLOW_RUNS_DDL, FLOW_VISITS_DDL]
        .into_iter()
        .chain(FLOW_INDEXES_DDL)
    {
        conn.execute_batch(&format!("{ddl};"))?;
    }
    Ok(())
}

fn err(e: impl ToString) -> String {
    e.to_string()
}

/// Reads `(id, json)` rows. A row whose JSON no longer parses is skipped and
/// logged, so one bad record cannot hide every other one.
fn readable<T: serde::de::DeserializeOwned>(
    st: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
    table: &str,
) -> Result<Vec<T>, String> {
    let rows = st
        .query_map(params, |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, raw) = row.map_err(err)?;
        match serde_json::from_str(&raw) {
            Ok(value) => out.push(value),
            Err(error) => eprintln!("rebase: skipped unreadable {table} row {id}: {error}"),
        }
    }
    Ok(out)
}

/// Own connection to `rebase.db`, like `automations::Service`.
pub struct FlowStore {
    conn: Connection,
}

impl FlowStore {
    pub fn open(db: &Path) -> Result<Self, String> {
        if let Some(parent) = db.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(err)?;
            }
        }
        let conn = Connection::open(db).map_err(err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(err)?;
        ensure_tables(&conn).map_err(err)?;
        Ok(Self { conn })
    }

    pub fn list(&self) -> Result<Vec<Flow>, String> {
        let mut st = self
            .conn
            .prepare("SELECT id, definition_json FROM flows ORDER BY updated_at DESC, id")
            .map_err(err)?;
        readable(&mut st, [], "flows")
    }

    /// Sets `created_at` on insert (kept on update) and always bumps `updated_at`.
    /// Refuses a Flow `validate_flow` reports errors for, so no path stores one.
    pub fn upsert(&self, flow: &Flow) -> Result<Flow, String> {
        let errors = validate_flow(flow).errors;
        if !errors.is_empty() {
            return Err(format!("Flow is invalid: {}", errors.join("; ")));
        }
        let now = now_millis();
        let previous: Option<String> = self
            .conn
            .query_row(
                "SELECT definition_json FROM flows WHERE id = ?1",
                [&flow.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let created_at = match previous {
            Some(raw) => serde_json::from_str::<Flow>(&raw).map_err(err)?.created_at,
            None if flow.created_at > 0 => flow.created_at,
            None => now,
        };
        let mut saved = flow.clone();
        saved.created_at = created_at;
        saved.updated_at = now;
        self.conn
            .execute(
                "INSERT INTO flows (id, definition_json, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET
                   definition_json = excluded.definition_json,
                   updated_at = excluded.updated_at",
                params![saved.id, serde_json::to_string(&saved).map_err(err)?, now],
            )
            .map_err(err)?;
        Ok(saved)
    }

    /// Cascades to runs and visits. Refused while a run is active.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let active: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM flow_runs WHERE flow_id = ?1 AND {ACTIVE}"),
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if active > 0 {
            return Err("Flow has an active run".into());
        }
        self.conn
            .execute("DELETE FROM flows WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }

    /// Newest first, at most 50.
    pub fn runs(&self, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>, String> {
        let limit = limit.min(MAX_RUNS) as i64;
        let mut st = self
            .conn
            .prepare(
                "SELECT id, run_json FROM flow_runs WHERE flow_id = ?1
                 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
            )
            .map_err(err)?;
        readable(&mut st, params![flow_id, limit], "flow_runs")
    }

    /// Runs that are `running` or `waiting-approval`, across all Flows.
    pub fn active_runs(&self) -> Result<Vec<FlowRun>, String> {
        let mut st = self
            .conn
            .prepare(&format!(
                "SELECT id, run_json FROM flow_runs WHERE {ACTIVE} ORDER BY created_at, rowid"
            ))
            .map_err(err)?;
        readable(&mut st, [], "flow_runs")
    }

    pub fn run(&self, id: &str) -> Result<Option<FlowRun>, String> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT run_json FROM flow_runs WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(err))
            .transpose()
    }

    /// Upsert by id. Fails if the Flow does not exist.
    pub fn save_run(&self, run: &FlowRun) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO flow_runs (id, flow_id, status, created_at, run_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                   status = excluded.status,
                   run_json = excluded.run_json",
                params![
                    run.id,
                    run.flow_id,
                    run.status.as_str(),
                    run.created_at,
                    serde_json::to_string(run).map_err(err)?
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    /// By `seq`.
    pub fn visits(&self, run_id: &str) -> Result<Vec<FlowVisit>, String> {
        let mut st = self
            .conn
            .prepare("SELECT id, visit_json FROM flow_visits WHERE run_id = ?1 ORDER BY seq")
            .map_err(err)?;
        readable(&mut st, [run_id], "flow_visits")
    }

    /// Upsert by id; `(run_id, seq)` is unique.
    pub fn save_visit(&self, visit: &FlowVisit) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO flow_visits (id, run_id, seq, state_id, status, visit_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                   state_id = excluded.state_id,
                   status = excluded.status,
                   visit_json = excluded.visit_json",
                params![
                    visit.id,
                    visit.run_id,
                    visit.seq,
                    visit.state_id,
                    visit.status.as_str(),
                    serde_json::to_string(visit).map_err(err)?
                ],
            )
            .map_err(err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::run::{FlowRun, FlowVisit, RunStatus, VisitStatus};
    use crate::flow::{starter_flows, Flow, FlowState, OutputValue};
    use std::collections::HashMap;

    fn store() -> FlowStore {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        ensure_tables(&conn).unwrap();
        FlowStore { conn }
    }

    fn flow(id: &str) -> Flow {
        let mut f = starter_flows().remove(0);
        f.id = id.into();
        f.cwd = "/tmp/project".into();
        f
    }

    fn run(id: &str, flow: &Flow, status: RunStatus, created_at: i64) -> FlowRun {
        FlowRun {
            id: id.into(),
            flow_id: flow.id.clone(),
            flow: flow.clone(),
            status,
            inputs: HashMap::from([("issue".to_string(), "42".to_string())]),
            worktree_cwd: None,
            current_state: Some(flow.initial.clone()),
            error: None,
            created_at,
            finished_at: None,
        }
    }

    fn visit(id: &str, run_id: &str, seq: u32) -> FlowVisit {
        FlowVisit {
            id: id.into(),
            run_id: run_id.into(),
            seq,
            state_id: "plan".into(),
            status: VisitStatus::Running,
            prompt: Some("go".into()),
            session_id: None,
            outputs: None,
            error: None,
            retried: false,
            note: None,
            started_at: 1,
            finished_at: None,
        }
    }

    #[test]
    fn upsert_then_list_round_trips_definition() {
        let s = store();
        let saved = s.upsert(&flow("f1")).unwrap();
        let listed = s.list().unwrap();
        assert_eq!(listed, vec![saved.clone()]);
        assert_eq!(listed[0].states, flow("f1").states);
        assert!(saved.created_at > 0 && saved.updated_at >= saved.created_at);
    }

    #[test]
    fn upsert_refuses_an_invalid_flow() {
        let s = store();
        let mut bad = flow("f1");
        let FlowState::Agent(first) = &mut bad.states[0] else {
            panic!("the starter begins with an agent");
        };
        first.transitions[0].to = "nowhere".into();
        let error = s.upsert(&bad).unwrap_err();
        assert!(error.starts_with("Flow is invalid: "), "{error}");
        assert!(error.contains("nowhere"), "{error}");
        assert!(s.list().unwrap().is_empty(), "nothing was stored");
        // An invalid edit of a stored Flow leaves the stored copy alone.
        let good = s.upsert(&flow("f1")).unwrap();
        bad.name = "Renamed".into();
        assert!(s.upsert(&bad).is_err());
        assert_eq!(s.list().unwrap(), vec![good]);
    }

    #[test]
    fn upsert_preserves_created_at_and_bumps_updated_at() {
        let s = store();
        let mut first = flow("f1");
        first.created_at = 5;
        let first = s.upsert(&first).unwrap();
        // A stale client copy must not clobber the stored created_at.
        let mut edit = first.clone();
        edit.created_at = 1;
        edit.updated_at = 1;
        edit.name = "Renamed".into();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = s.upsert(&edit).unwrap();
        assert_eq!(second.created_at, first.created_at);
        assert!(second.updated_at > first.updated_at);
        let listed = s.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], second);
        assert_eq!(listed[0].name, "Renamed");
    }

    #[test]
    fn delete_flow_cascades_runs_and_visits() {
        let s = store();
        let f = s.upsert(&flow("f1")).unwrap();
        s.save_run(&run("r1", &f, RunStatus::Succeeded, 1)).unwrap();
        s.save_visit(&visit("v1", "r1", 1)).unwrap();
        s.delete("f1").unwrap();
        assert!(s.list().unwrap().is_empty());
        assert!(s.run("r1").unwrap().is_none());
        assert!(s.visits("r1").unwrap().is_empty());
    }

    #[test]
    fn delete_refused_with_active_run() {
        for status in [RunStatus::Running, RunStatus::WaitingApproval] {
            let s = store();
            let f = s.upsert(&flow("f1")).unwrap();
            s.save_run(&run("r1", &f, RunStatus::Failed, 1)).unwrap();
            s.save_run(&run("r2", &f, status, 2)).unwrap();
            assert_eq!(s.delete("f1").unwrap_err(), "Flow has an active run");
            assert_eq!(s.list().unwrap().len(), 1);
            assert_eq!(s.runs("f1", 50).unwrap().len(), 2);
            // Once the run ends the Flow can go.
            s.save_run(&run("r2", &f, RunStatus::Cancelled, 2)).unwrap();
            s.delete("f1").unwrap();
        }
    }

    #[test]
    fn delete_unknown_flow_is_ok() {
        store().delete("nope").unwrap();
    }

    #[test]
    fn runs_listed_newest_first_and_capped_at_50() {
        let s = store();
        let f = s.upsert(&flow("f1")).unwrap();
        let other = s.upsert(&flow("f2")).unwrap();
        for i in 0..60 {
            s.save_run(&run(&format!("r{i}"), &f, RunStatus::Succeeded, i))
                .unwrap();
        }
        s.save_run(&run("other", &other, RunStatus::Succeeded, 100))
            .unwrap();
        let runs = s.runs("f1", 100).unwrap();
        assert_eq!(runs.len(), 50);
        assert_eq!(runs[0].id, "r59");
        assert_eq!(runs[49].id, "r10");
        let few = s.runs("f1", 3).unwrap();
        assert_eq!(
            few.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["r59", "r58", "r57"]
        );
    }

    #[test]
    fn active_runs_lists_only_running_and_waiting() {
        let s = store();
        let f = s.upsert(&flow("f1")).unwrap();
        for (id, status) in [
            ("a", RunStatus::Running),
            ("b", RunStatus::WaitingApproval),
            ("c", RunStatus::Succeeded),
            ("d", RunStatus::Failed),
            ("e", RunStatus::Cancelled),
        ] {
            s.save_run(&run(id, &f, status, 1)).unwrap();
        }
        let mut ids: Vec<_> = s.active_runs().unwrap().into_iter().map(|r| r.id).collect();
        ids.sort();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn save_run_updates_in_place_and_status_column_matches() {
        let s = store();
        let f = s.upsert(&flow("f1")).unwrap();
        let mut r = run("r1", &f, RunStatus::Running, 1);
        s.save_run(&r).unwrap();
        r.status = RunStatus::WaitingApproval;
        r.error = Some("x".into());
        s.save_run(&r).unwrap();
        assert_eq!(s.run("r1").unwrap().unwrap(), r);
        let col: String = s
            .conn
            .query_row("SELECT status FROM flow_runs WHERE id='r1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(col, "waiting-approval");
        assert!(s.run("missing").unwrap().is_none());
    }

    #[test]
    fn visit_seq_unique_per_run() {
        let s = store();
        let f = s.upsert(&flow("f1")).unwrap();
        s.save_run(&run("r1", &f, RunStatus::Running, 1)).unwrap();
        s.save_run(&run("r2", &f, RunStatus::Running, 1)).unwrap();
        s.save_visit(&visit("v1", "r1", 1)).unwrap();
        // Same run, same seq, different id: rejected.
        assert!(s.save_visit(&visit("v2", "r1", 1)).is_err());
        // Same seq in another run is fine.
        s.save_visit(&visit("v3", "r2", 1)).unwrap();
        // Same id updates in place.
        let mut v = visit("v1", "r1", 1);
        v.status = VisitStatus::Completed;
        v.outputs = Some(HashMap::from([("ok".to_string(), OutputValue::Bool(true))]));
        s.save_visit(&v).unwrap();
        assert_eq!(s.visits("r1").unwrap(), vec![v]);
    }

    #[test]
    fn visits_listed_in_seq_order() {
        let s = store();
        let f = s.upsert(&flow("f1")).unwrap();
        s.save_run(&run("r1", &f, RunStatus::Running, 1)).unwrap();
        for (id, seq) in [("c", 3), ("a", 1), ("b", 2)] {
            s.save_visit(&visit(id, "r1", seq)).unwrap();
        }
        let seqs: Vec<_> = s.visits("r1").unwrap().iter().map(|v| v.seq).collect();
        assert_eq!(seqs, [1, 2, 3]);
    }

    #[test]
    fn run_save_rejects_unknown_flow() {
        let s = store();
        let ghost = flow("ghost");
        assert!(s
            .save_run(&run("r1", &ghost, RunStatus::Running, 1))
            .is_err());
        assert!(s.run("r1").unwrap().is_none());
        assert!(s.save_visit(&visit("v1", "no-run", 1)).is_err());
    }

    #[test]
    fn listings_skip_a_row_whose_json_does_not_parse() {
        let s = store();
        let good = s.upsert(&flow("f1")).unwrap();
        s.conn
            .execute(
                "INSERT INTO flows (id, definition_json, updated_at) VALUES ('bad', '{', 0)",
                [],
            )
            .unwrap();
        assert_eq!(s.list().unwrap(), vec![good.clone()]);

        let running = run("r1", &good, RunStatus::Running, 1);
        s.save_run(&running).unwrap();
        s.conn
            .execute(
                "INSERT INTO flow_runs (id, flow_id, status, created_at, run_json)
                 VALUES ('r-bad', 'f1', 'running', 2, 'not json')",
                [],
            )
            .unwrap();
        assert_eq!(s.active_runs().unwrap(), vec![running.clone()]);
        assert_eq!(s.runs("f1", 10).unwrap(), vec![running]);

        let v1 = visit("v1", "r1", 1);
        s.save_visit(&v1).unwrap();
        s.conn
            .execute(
                "INSERT INTO flow_visits (id, run_id, seq, state_id, status, visit_json)
                 VALUES ('v-bad', 'r1', 2, 'plan', 'running', '[]')",
                [],
            )
            .unwrap();
        assert_eq!(s.visits("r1").unwrap(), vec![v1]);
    }

    #[test]
    fn ensure_tables_is_idempotent_alongside_other_native_tables() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        crate::automations::ensure_tables(&conn).unwrap();
        ensure_tables(&conn).unwrap();
        ensure_tables(&conn).unwrap();
        crate::automations::ensure_tables(&conn).unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for t in ["automations", "flows", "flow_runs", "flow_visits"] {
            assert!(names.iter().any(|n| n == t), "{t} in {names:?}");
        }
    }

    #[test]
    fn open_creates_tables_enforces_foreign_keys_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("rebase.db");
        let s = FlowStore::open(&path).unwrap();
        let f = s.upsert(&flow("f1")).unwrap();
        assert!(s
            .save_run(&run("r1", &flow("ghost"), RunStatus::Running, 1))
            .is_err());
        drop(s);
        assert_eq!(FlowStore::open(&path).unwrap().list().unwrap(), vec![f]);
    }
}
