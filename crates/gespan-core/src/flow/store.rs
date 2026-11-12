//! Flow, run and visit storage in `gespan.db`. Definitions and records are JSON
//! blobs with mirrored indexed columns, like `automations`.
use super::definition::{validate_flow, Flow};
use super::run::{FlowRun, FlowVisit};
use crate::orm::{self, Kind};
use std::path::Path;

/// A stored Flow definition.
#[derive(Debug, toasty::Model)]
#[table = "flows"]
pub(crate) struct FlowRow {
    #[key]
    id: String,
    definition_json: String,
    updated_at: i64,
}

/// A stored run. `flow_id` points at `flows`; [`CONSTRAINTS`] keeps it there.
#[derive(Debug, toasty::Model)]
#[table = "flow_runs"]
#[index(name = "flow_runs_history_idx", flow_id, created_at)]
#[index(name = "flow_runs_status_idx", status)]
pub(crate) struct FlowRunRow {
    #[key]
    id: String,
    flow_id: String,
    status: String,
    created_at: i64,
    run_json: String,
}

/// A stored visit, unique by `(run_id, seq)`. `run_id` points at `flow_runs`.
#[derive(Debug, toasty::Model)]
#[table = "flow_visits"]
#[unique(name = "flow_visits_order_idx", run_id, seq)]
pub(crate) struct FlowVisitRow {
    #[key]
    id: String,
    run_id: String,
    seq: i64,
    state_id: String,
    status: String,
    visit_json: String,
}

/// The foreign keys the hand-made tables declared, which Toasty's DDL
/// cannot: a run needs its Flow and a visit its run, and deleting a Flow
/// deletes its runs and their visits (`ON DELETE CASCADE`).
pub(crate) const CONSTRAINTS: &[&str] = &[
    "CREATE TRIGGER IF NOT EXISTS flow_runs_need_their_flow
     BEFORE INSERT ON flow_runs
     WHEN NOT EXISTS (SELECT 1 FROM flows WHERE id = NEW.flow_id)
     BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    "CREATE TRIGGER IF NOT EXISTS flow_visits_need_their_run
     BEFORE INSERT ON flow_visits
     WHEN NOT EXISTS (SELECT 1 FROM flow_runs WHERE id = NEW.run_id)
     BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    "CREATE TRIGGER IF NOT EXISTS flows_delete_their_runs
     AFTER DELETE ON flows
     BEGIN DELETE FROM flow_runs WHERE flow_id = OLD.id; END",
    "CREATE TRIGGER IF NOT EXISTS flow_runs_delete_their_visits
     AFTER DELETE ON flow_runs
     BEGIN DELETE FROM flow_visits WHERE run_id = OLD.id; END",
];

/// The hand-made tables older builds created, for adoption tests.
#[cfg(test)]
pub(crate) const LEGACY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS flows (
  id TEXT PRIMARY KEY,
  definition_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS flow_runs (
  id TEXT PRIMARY KEY,
  flow_id TEXT NOT NULL REFERENCES flows(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  run_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS flow_visits (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES flow_runs(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  state_id TEXT NOT NULL,
  status TEXT NOT NULL,
  visit_json TEXT NOT NULL,
  UNIQUE (run_id, seq)
);
CREATE INDEX IF NOT EXISTS flow_runs_history_idx ON flow_runs (flow_id, created_at DESC);
CREATE INDEX IF NOT EXISTS flow_runs_status_idx ON flow_runs (status);
CREATE INDEX IF NOT EXISTS flow_visits_order_idx ON flow_visits (run_id, seq);";

const MAX_RUNS: usize = 50;
const ACTIVE: &str = "status IN ('running', 'waiting-approval')";

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn json_error(error: serde_json::Error) -> toasty::Error {
    toasty::Error::from_args(format_args!("{error}"))
}

/// `(id, json)` pairs from a raw query selecting those two text columns.
fn text_pairs(rows: Vec<toasty::stmt::Value>) -> Vec<(String, String)> {
    rows.into_iter()
        .filter_map(|row| {
            let mut fields = row.into_record().fields.into_iter();
            match (fields.next(), fields.next()) {
                (Some(toasty::stmt::Value::String(id)), Some(toasty::stmt::Value::String(raw))) => {
                    Some((id, raw))
                }
                _ => None,
            }
        })
        .collect()
}

/// Parses `(id, json)` rows. A row whose JSON no longer parses is skipped
/// and logged, so one bad record cannot hide every other one.
fn readable<T: serde::de::DeserializeOwned>(
    rows: impl IntoIterator<Item = (String, String)>,
    table: &str,
) -> Vec<T> {
    let mut out = Vec::new();
    for (id, raw) in rows {
        match serde_json::from_str(&raw) {
            Ok(value) => out.push(value),
            Err(error) => tracing::warn!(table, id = %id, %error, "skipped an unreadable row"),
        }
    }
    out
}

/// Its own Toasty database on `gespan.db`, like `automations::Service`.
pub struct FlowStore {
    db: orm::Database,
}

impl FlowStore {
    pub fn open(db: &Path) -> Result<Self, String> {
        Ok(Self {
            db: orm::open(Kind::Gespan, db)?,
        })
    }

    pub fn list(&self) -> Result<Vec<Flow>, String> {
        let rows = self.db.run(|mut db| async move {
            let flow = FlowRow::fields();
            FlowRow::all()
                .order_by((flow.updated_at().desc(), flow.id().asc()))
                .exec(&mut db)
                .await
        })?;
        Ok(readable(
            rows.into_iter().map(|row| (row.id, row.definition_json)),
            "flows",
        ))
    }

    /// Flows a Pilot owns; user Flows and other Pilots' Flows are excluded.
    pub fn owned(&self, owner: &str) -> Result<Vec<Flow>, String> {
        let mut flows = self.list()?;
        flows.retain(|f| f.owner.as_deref() == Some(owner));
        Ok(flows)
    }

    pub fn owned_flow(&self, owner: &str, id: &str) -> Result<Option<Flow>, String> {
        Ok(self.owned(owner)?.into_iter().find(|f| f.id == id))
    }

    /// A run and its visits, only when the run's snapshot is owned by `owner`.
    pub fn owned_run(
        &self,
        owner: &str,
        run_id: &str,
    ) -> Result<Option<(FlowRun, Vec<FlowVisit>)>, String> {
        let Some(run) = self.run(run_id)? else {
            return Ok(None);
        };
        if run.flow.owner.as_deref() != Some(owner) {
            return Ok(None);
        }
        let visits = self.visits(run_id)?;
        Ok(Some((run, visits)))
    }

    /// Newest first across the owner's Flows (each scanned to its newest 50
    /// runs), optionally only runs tagged with `goal`.
    pub fn owned_runs(
        &self,
        owner: &str,
        goal: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FlowRun>, String> {
        let mut runs = Vec::new();
        for flow in self.owned(owner)? {
            runs.extend(self.runs(&flow.id, MAX_RUNS)?);
        }
        if let Some(goal) = goal {
            runs.retain(|r| r.tag.as_ref().is_some_and(|t| t.goal == goal));
        }
        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        runs.truncate(limit);
        Ok(runs)
    }

    /// Sets `created_at` on insert (kept on update) and always bumps `updated_at`.
    /// Refuses a Flow `validate_flow` reports errors for, so no path stores one.
    pub fn upsert(&self, flow: &Flow) -> Result<Flow, String> {
        let errors = validate_flow(flow).errors;
        if !errors.is_empty() {
            return Err(format!("Flow is invalid: {}", errors.join("; ")));
        }
        let now = now_millis();
        let mut saved = flow.clone();
        self.db.transaction(|tx| {
            Box::pin(async move {
                let previous = FlowRow::filter_by_id(&saved.id).first().exec(tx).await?;
                saved.created_at = match &previous {
                    Some(row) => {
                        serde_json::from_str::<Flow>(&row.definition_json)
                            .map_err(json_error)?
                            .created_at
                    }
                    None if saved.created_at > 0 => saved.created_at,
                    None => now,
                };
                saved.updated_at = now;
                let raw = serde_json::to_string(&saved).map_err(json_error)?;
                match previous {
                    Some(mut row) => {
                        row.update()
                            .definition_json(raw)
                            .updated_at(now)
                            .exec(tx)
                            .await?
                    }
                    None => {
                        FlowRow::create()
                            .id(saved.id.clone())
                            .definition_json(raw)
                            .updated_at(now)
                            .exec(tx)
                            .await?;
                    }
                }
                Ok(saved)
            })
        })
    }

    /// Cascades to runs and visits. Refused while a run is active.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let id = id.to_owned();
        self.db.transaction(|tx| {
            Box::pin(async move {
                let run = FlowRunRow::fields();
                let active = FlowRunRow::filter(
                    run.flow_id().eq(&id).and(
                        run.status()
                            .in_list(["running".to_owned(), "waiting-approval".to_owned()]),
                    ),
                )
                .first()
                .exec(tx)
                .await?;
                if active.is_some() {
                    return Ok(Err("Flow has an active run".to_owned()));
                }
                FlowRow::filter_by_id(&id).delete().exec(tx).await?;
                Ok(Ok(()))
            })
        })?
    }

    /// Newest first, at most 50. Runs made in the same millisecond come
    /// newest-saved first, which needs `rowid`. A Toasty model cannot name
    /// it: raw SQL.
    pub fn runs(&self, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>, String> {
        let query = toasty::sql::query(
            "SELECT id, run_json FROM flow_runs WHERE flow_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )
        .bind(flow_id.to_owned())
        .bind(limit.min(MAX_RUNS) as i64);
        let rows = self
            .db
            .run(|mut db| async move { query.exec(&mut db).await })?;
        Ok(readable(text_pairs(rows), "flow_runs"))
    }

    /// Runs that are `running` or `waiting-approval`, across all Flows, in
    /// the order they were saved. That order needs `rowid`, which a Toasty
    /// model cannot name: raw SQL.
    pub fn active_runs(&self) -> Result<Vec<FlowRun>, String> {
        let query = toasty::sql::query(format!(
            "SELECT id, run_json FROM flow_runs WHERE {ACTIVE} ORDER BY created_at, rowid"
        ));
        let rows = self
            .db
            .run(|mut db| async move { query.exec(&mut db).await })?;
        Ok(readable(text_pairs(rows), "flow_runs"))
    }

    pub fn run(&self, id: &str) -> Result<Option<FlowRun>, String> {
        let row = self.db.run(|mut db| async move {
            FlowRunRow::filter_by_id(id).first().exec(&mut db).await
        })?;
        row.map(|row| serde_json::from_str(&row.run_json).map_err(|e| e.to_string()))
            .transpose()
    }

    /// Upsert by id. Fails if the Flow does not exist.
    pub fn save_run(&self, run: &FlowRun) -> Result<(), String> {
        let raw = serde_json::to_string(run).map_err(|e| e.to_string())?;
        let run = run.clone();
        self.db.transaction(|tx| {
            Box::pin(async move {
                let status = run.status.as_str().to_owned();
                match FlowRunRow::filter_by_id(&run.id).first().exec(tx).await? {
                    // The Flow and creation time stay as first saved.
                    Some(mut row) => row.update().status(status).run_json(raw).exec(tx).await?,
                    None => {
                        FlowRunRow::create()
                            .id(run.id)
                            .flow_id(run.flow_id)
                            .status(status)
                            .created_at(run.created_at)
                            .run_json(raw)
                            .exec(tx)
                            .await?;
                    }
                }
                Ok(())
            })
        })
    }

    /// By `seq`.
    pub fn visits(&self, run_id: &str) -> Result<Vec<FlowVisit>, String> {
        let rows = self.db.run(|mut db| async move {
            let visit = FlowVisitRow::fields();
            FlowVisitRow::filter(visit.run_id().eq(run_id))
                .order_by(visit.seq().asc())
                .exec(&mut db)
                .await
        })?;
        Ok(readable(
            rows.into_iter().map(|row| (row.id, row.visit_json)),
            "flow_visits",
        ))
    }

    /// Upsert by id; `(run_id, seq)` is unique.
    pub fn save_visit(&self, visit: &FlowVisit) -> Result<(), String> {
        let raw = serde_json::to_string(visit).map_err(|e| e.to_string())?;
        let visit = visit.clone();
        self.db.transaction(|tx| {
            Box::pin(async move {
                let status = visit.status.as_str().to_owned();
                match FlowVisitRow::filter_by_id(&visit.id)
                    .first()
                    .exec(tx)
                    .await?
                {
                    // The run and its place in it stay as first saved.
                    Some(mut row) => {
                        row.update()
                            .state_id(visit.state_id)
                            .status(status)
                            .visit_json(raw)
                            .exec(tx)
                            .await?
                    }
                    None => {
                        FlowVisitRow::create()
                            .id(visit.id)
                            .run_id(visit.run_id)
                            .seq(i64::from(visit.seq))
                            .state_id(visit.state_id)
                            .status(status)
                            .visit_json(raw)
                            .exec(tx)
                            .await?;
                    }
                }
                Ok(())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::run::{FlowRun, FlowVisit, RunStatus, VisitStatus};
    use crate::flow::{starter_flows, Flow, FlowState, OutputValue};
    use std::collections::HashMap;

    fn store() -> FlowStore {
        FlowStore::open(Path::new(":memory:")).unwrap()
    }

    /// Runs raw SQL on the store's database, as another writer could.
    fn execute(s: &FlowStore, sql: &'static str) {
        s.db.run(|mut db| async move { toasty::sql::statement(sql).exec(&mut db).await })
            .unwrap();
    }

    /// The first column of `sql`'s rows, as text.
    fn texts(s: &FlowStore, sql: &'static str) -> Vec<String> {
        s.db.run(|mut db| async move { toasty::sql::query(sql).exec(&mut db).await })
            .unwrap()
            .into_iter()
            .map(|row| row.into_record().fields[0].as_string_unwrap().to_owned())
            .collect()
    }

    fn flow(id: &str) -> Flow {
        let mut f = starter_flows().remove(0);
        f.id = id.into();
        f.cwd = "/tmp/project".into();
        f
    }

    /// A one-state Flow a Pilot may own (no approval states).
    fn pilot_flow(id: &str, owner: &str) -> Flow {
        let mut f = flow(id);
        f.states.truncate(1);
        let FlowState::Agent(only) = &mut f.states[0] else {
            panic!("the starter begins with an agent");
        };
        only.transitions = vec![crate::flow::Transition {
            to: crate::flow::SUCCEEDED.into(),
            when: None,
        }];
        f.initial = only.id.clone();
        f.owner = Some(owner.into());
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
            tag: None,
            edits: vec![],
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
    fn owned_helpers_return_only_that_pilots_flows_and_runs() {
        use crate::flow::run::RunTag;
        let s = store();
        let p1 = pilot_flow("p1-flow", "p1");
        let p1b = pilot_flow("p1-other", "p1");
        let p2 = pilot_flow("p2-flow", "p2");
        let user = flow("user-flow");
        for f in [&p1, &p1b, &p2, &user] {
            s.upsert(f).unwrap();
        }
        let tagged = |id: &str, f: &Flow, goal: &str, at: i64| {
            let mut r = run(id, f, RunStatus::Succeeded, at);
            r.tag = Some(RunTag {
                goal: goal.into(),
                task: "t".into(),
            });
            r
        };
        s.save_run(&tagged("r1", &p1, "g1", 10)).unwrap();
        s.save_run(&tagged("r2", &p1b, "g2", 30)).unwrap();
        s.save_run(&tagged("r3", &p1, "g1", 20)).unwrap();
        s.save_run(&tagged("r4", &p2, "g1", 40)).unwrap();
        s.save_run(&run("r5", &user, RunStatus::Succeeded, 50))
            .unwrap();
        s.save_visit(&visit("v1", "r1", 1)).unwrap();

        let mut ids: Vec<String> = s.owned("p1").unwrap().into_iter().map(|f| f.id).collect();
        ids.sort();
        assert_eq!(ids, ["p1-flow", "p1-other"]);
        assert!(s.owned("nobody").unwrap().is_empty());

        assert_eq!(
            s.owned_flow("p1", "p1-flow").unwrap().unwrap().id,
            "p1-flow"
        );
        assert_eq!(s.owned_flow("p1", "p2-flow").unwrap(), None);
        assert_eq!(s.owned_flow("p1", "user-flow").unwrap(), None);
        assert_eq!(s.owned_flow("p1", "missing").unwrap(), None);

        let (r, visits) = s.owned_run("p1", "r1").unwrap().unwrap();
        assert_eq!(r.id, "r1");
        assert_eq!(visits.len(), 1);
        assert!(s.owned_run("p1", "r4").unwrap().is_none());
        assert!(s.owned_run("p1", "r5").unwrap().is_none());
        assert!(s.owned_run("p1", "missing").unwrap().is_none());

        let all: Vec<String> = s
            .owned_runs("p1", None, 50)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(all, ["r2", "r3", "r1"]);
        let g1: Vec<String> = s
            .owned_runs("p1", Some("g1"), 50)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(g1, ["r3", "r1"]);
        assert_eq!(s.owned_runs("p1", None, 2).unwrap().len(), 2);
        assert!(s.owned_runs("p1", Some("none"), 50).unwrap().is_empty());
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
        assert_eq!(
            texts(&s, "SELECT status FROM flow_runs WHERE id='r1'"),
            ["waiting-approval"]
        );
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
        execute(
            &s,
            "INSERT INTO flows (id, definition_json, updated_at) VALUES ('bad', '{', 0)",
        );
        let mut listed = Ok(Vec::new());
        let logged = crate::diagnostics::test_capture(|| listed = s.list());
        assert_eq!(listed.unwrap(), vec![good.clone()]);
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0]["level"], "warn");
        assert_eq!(logged[0]["message"], "skipped an unreadable row");
        assert_eq!(logged[0]["fields"]["table"], "flows");
        assert_eq!(logged[0]["fields"]["id"], "bad");
        assert!(!logged[0]["fields"]["error"].as_str().unwrap().is_empty());

        let running = run("r1", &good, RunStatus::Running, 1);
        s.save_run(&running).unwrap();
        execute(
            &s,
            "INSERT INTO flow_runs (id, flow_id, status, created_at, run_json)
             VALUES ('r-bad', 'f1', 'running', 2, 'not json')",
        );
        assert_eq!(s.active_runs().unwrap(), vec![running.clone()]);
        assert_eq!(s.runs("f1", 10).unwrap(), vec![running]);

        let v1 = visit("v1", "r1", 1);
        s.save_visit(&v1).unwrap();
        execute(
            &s,
            "INSERT INTO flow_visits (id, run_id, seq, state_id, status, visit_json)
             VALUES ('v-bad', 'r1', 2, 'plan', 'running', '[]')",
        );
        assert_eq!(s.visits("r1").unwrap(), vec![v1]);
    }

    #[test]
    fn flows_and_automations_share_one_file_through_their_own_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let flows = FlowStore::open(&path).unwrap();
        let mut automations = crate::automations::Service::open(&path).unwrap();
        let again = FlowStore::open(&path).unwrap();
        let f = flows.upsert(&flow("f1")).unwrap();
        let mut draft = crate::automations::AutomationUpsert::new(
            "/repo".into(),
            "codex".into(),
            "model".into(),
        );
        draft.name = "Nightly".into();
        draft.prompt = "Report".into();
        let automation = automations.upsert(draft).unwrap();
        again
            .save_run(&run("r1", &f, RunStatus::Running, 1))
            .unwrap();
        assert_eq!(flows.run("r1").unwrap().unwrap().id, "r1");
        assert_eq!(automations.list().unwrap()[0].id, automation.id);
        let names = texts(
            &flows,
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
        );
        for t in [
            "automation_event_claims",
            "automation_runs",
            "automations",
            "flow_runs",
            "flow_visits",
            "flows",
            "my_tasks",
        ] {
            assert!(names.iter().any(|n| n == t), "{t} in {names:?}");
        }
    }

    #[test]
    fn a_hand_made_flows_file_keeps_its_rows_and_its_foreign_keys_when_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let f = flow("f1");
        let r = run("r1", &f, RunStatus::Succeeded, 7);
        let v = visit("v1", "r1", 1);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(LEGACY_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO flows VALUES (?1, ?2, 5)",
            rusqlite::params![f.id, serde_json::to_string(&f).unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO flow_runs VALUES ('r1', 'f1', 'succeeded', 7, ?1)",
            [serde_json::to_string(&r).unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO flow_visits VALUES ('v1', 'r1', 1, 'plan', 'running', ?1)",
            [serde_json::to_string(&v).unwrap()],
        )
        .unwrap();
        drop(conn);

        let s = FlowStore::open(&path).unwrap();
        assert_eq!(s.list().unwrap(), vec![f.clone()]);
        assert_eq!(s.runs("f1", 10).unwrap(), vec![r]);
        assert_eq!(s.visits("r1").unwrap(), vec![v]);
        // The adopted tables still refuse orphans and duplicate places.
        assert!(s
            .save_run(&run("r2", &flow("ghost"), RunStatus::Running, 1))
            .is_err());
        assert!(s.save_visit(&visit("v2", "r1", 1)).is_err());
        // And deleting the Flow still takes its runs and their visits.
        s.delete("f1").unwrap();
        assert_eq!(texts(&s, "SELECT id FROM flow_runs"), Vec::<String>::new());
        assert_eq!(
            texts(&s, "SELECT id FROM flow_visits"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_raw_insert_cannot_add_a_visit_to_a_missing_run_either() {
        // The foreign keys are triggers on the tables, not checks in this
        // store, so any writer on the file meets them.
        let s = store();
        let refused = s.db.run(|mut db| async move {
            toasty::sql::statement(
                "INSERT INTO flow_visits (id, run_id, seq, state_id, status, visit_json)
                 VALUES ('v', 'no-run', 1, 'plan', 'running', '{}')",
            )
            .exec(&mut db)
            .await
        });
        assert!(refused.unwrap_err().contains("FOREIGN KEY"));
    }

    #[test]
    fn open_creates_tables_enforces_foreign_keys_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("gespan.db");
        let s = FlowStore::open(&path).unwrap();
        let f = s.upsert(&flow("f1")).unwrap();
        assert!(s
            .save_run(&run("r1", &flow("ghost"), RunStatus::Running, 1))
            .is_err());
        drop(s);
        assert_eq!(FlowStore::open(&path).unwrap().list().unwrap(), vec![f]);
    }
}
