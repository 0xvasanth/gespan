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

/// One version of a Flow: a numbered snapshot of a definition that was
/// live, or the one draft being edited before it is saved as the next.
/// `flows` keeps the live definition, so running a Flow reads nothing here.
#[derive(Debug, toasty::Model)]
#[table = "flow_versions"]
#[unique(name = "flow_versions_number_idx", flow_id, version)]
pub(crate) struct FlowVersionRow {
    /// `<flow id>:v<n>`, or `<flow id>:draft`.
    #[key]
    id: String,
    flow_id: String,
    version: i64,
    /// `"saved"` or `"draft"`.
    status: String,
    definition_json: String,
    created_at: i64,
}

/// A Flow version as the Pipeline tab lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowVersion {
    pub flow_id: String,
    /// v1, v2…; a draft carries the number it will be saved as.
    pub version: i64,
    pub draft: bool,
    pub flow: Flow,
    pub created_at: i64,
}

const SAVED: &str = "saved";
const DRAFT: &str = "draft";

fn version_id(flow_id: &str, version: i64) -> String {
    format!("{flow_id}:v{version}")
}
fn draft_id(flow_id: &str) -> String {
    format!("{flow_id}:draft")
}

/// The definition without its timestamps, to tell whether a save changed it.
fn definition_of(flow: &Flow) -> Result<serde_json::Value, toasty::Error> {
    let mut flow = flow.clone();
    (flow.created_at, flow.updated_at) = (0, 0);
    serde_json::to_value(&flow).map_err(json_error)
}

/// The newest saved version of `flow_id`, if any.
async fn latest_saved(
    tx: &mut toasty::Transaction<'_>,
    flow_id: &str,
) -> Result<Option<FlowVersionRow>, toasty::Error> {
    let version = FlowVersionRow::fields();
    FlowVersionRow::filter(
        version
            .flow_id()
            .eq(flow_id)
            .and(version.status().eq(SAVED.to_owned())),
    )
    .order_by(version.version().desc())
    .first()
    .exec(tx)
    .await
}

/// Records `flow` as the next saved version unless the newest already has
/// the same definition.
async fn record_version(
    tx: &mut toasty::Transaction<'_>,
    flow: &Flow,
    at: i64,
) -> Result<(), toasty::Error> {
    let latest = latest_saved(tx, &flow.id).await?;
    let next = match &latest {
        Some(row) => {
            let same = serde_json::from_str::<Flow>(&row.definition_json)
                .ok()
                .map(|old| definition_of(&old))
                .transpose()?
                == Some(definition_of(flow)?);
            if same {
                return Ok(());
            }
            row.version + 1
        }
        None => 1,
    };
    FlowVersionRow::create()
        .id(version_id(&flow.id, next))
        .flow_id(flow.id.clone())
        .version(next)
        .status(SAVED.to_owned())
        .definition_json(serde_json::to_string(flow).map_err(json_error)?)
        .created_at(at)
        .exec(tx)
        .await?;
    Ok(())
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
        let store = Self {
            db: orm::open(Kind::Gespan, db)?,
        };
        store.backfill_versions()?;
        Ok(store)
    }

    /// Flows saved before versions existed become v1, exactly as they are,
    /// dated when they were last saved. Runs on every open and does nothing
    /// once each Flow has a version.
    fn backfill_versions(&self) -> Result<(), String> {
        self.db.transaction(|tx| {
            Box::pin(async move {
                let flows = FlowRow::all().exec(tx).await?;
                for row in flows {
                    if latest_saved(tx, &row.id).await?.is_some() {
                        continue;
                    }
                    // A definition that no longer parses stays as it is;
                    // `list` already skips it.
                    if serde_json::from_str::<Flow>(&row.definition_json).is_err() {
                        continue;
                    }
                    FlowVersionRow::create()
                        .id(version_id(&row.id, 1))
                        .flow_id(row.id.clone())
                        .version(1)
                        .status(SAVED.to_owned())
                        .definition_json(row.definition_json.clone())
                        .created_at(row.updated_at)
                        .exec(tx)
                        .await?;
                }
                Ok(())
            })
        })
    }

    /// `flow_id`'s versions, the draft first, then newest saved first.
    pub fn versions(&self, flow_id: &str) -> Result<Vec<FlowVersion>, String> {
        let rows = self.db.run(|mut db| async move {
            let version = FlowVersionRow::fields();
            FlowVersionRow::filter(version.flow_id().eq(flow_id))
                .order_by(version.version().desc())
                .exec(&mut db)
                .await
        })?;
        // A draft is stored as version 0; it will be saved as the next one.
        let next = rows
            .iter()
            .filter(|row| row.status == SAVED)
            .map(|row| row.version)
            .max()
            .unwrap_or(0)
            + 1;
        let mut versions: Vec<FlowVersion> = rows
            .into_iter()
            .filter_map(
                |row| match serde_json::from_str::<Flow>(&row.definition_json) {
                    Ok(flow) => Some(FlowVersion {
                        flow_id: row.flow_id,
                        version: if row.status == DRAFT {
                            next
                        } else {
                            row.version
                        },
                        draft: row.status == DRAFT,
                        flow,
                        created_at: row.created_at,
                    }),
                    Err(error) => {
                        tracing::warn!(id = %row.id, %error, "skipped an unreadable flow version");
                        None
                    }
                },
            )
            .collect();
        versions.sort_by_key(|v| (!v.draft, std::cmp::Reverse(v.version)));
        Ok(versions)
    }

    /// The draft being edited for `flow_id`, if any.
    pub fn draft(&self, flow_id: &str) -> Result<Option<FlowVersion>, String> {
        Ok(self.versions(flow_id)?.into_iter().find(|v| v.draft))
    }

    /// Keeps `flow` as `flow.id`'s draft, numbered as the next version. The
    /// live definition is untouched until [`Self::publish_draft`]. The Flow
    /// must exist, and the draft must be valid.
    pub fn save_draft(&self, flow: &Flow) -> Result<FlowVersion, String> {
        let errors = validate_flow(flow).errors;
        if !errors.is_empty() {
            return Err(format!("Flow is invalid: {}", errors.join("; ")));
        }
        let now = now_millis();
        let draft = flow.clone();
        self.db.transaction(|tx| {
            Box::pin(async move {
                let Some(live) = FlowRow::filter_by_id(&draft.id).first().exec(tx).await? else {
                    return Ok(Err("Flow not found".to_owned()));
                };
                let next = latest_saved(tx, &draft.id)
                    .await?
                    .map_or(1, |row| row.version + 1);
                let mut stored = draft.clone();
                // A draft keeps the Flow's identity and creation time.
                stored.created_at = serde_json::from_str::<Flow>(&live.definition_json)
                    .map(|f| f.created_at)
                    .unwrap_or(now);
                stored.updated_at = now;
                let raw = serde_json::to_string(&stored).map_err(json_error)?;
                match FlowVersionRow::filter_by_id(draft_id(&draft.id))
                    .first()
                    .exec(tx)
                    .await?
                {
                    Some(mut row) => {
                        row.update()
                            .definition_json(raw)
                            .created_at(now)
                            .exec(tx)
                            .await?
                    }
                    None => {
                        FlowVersionRow::create()
                            .id(draft_id(&draft.id))
                            .flow_id(draft.id.clone())
                            // Unnumbered until saved; see `versions`.
                            .version(0)
                            .status(DRAFT.to_owned())
                            .definition_json(raw)
                            .created_at(now)
                            .exec(tx)
                            .await?;
                    }
                }
                Ok(Ok(FlowVersion {
                    flow_id: stored.id.clone(),
                    version: next,
                    draft: true,
                    flow: stored,
                    created_at: now,
                }))
            })
        })?
    }

    /// Makes `flow_id`'s draft the live definition, saved as its version.
    pub fn publish_draft(&self, flow_id: &str) -> Result<Flow, String> {
        let draft = self.draft(flow_id)?.ok_or("No draft to save")?;
        let saved = self.upsert(&draft.flow)?;
        self.discard_draft(flow_id)?;
        Ok(saved)
    }

    /// Drops `flow_id`'s draft; the live definition stays.
    pub fn discard_draft(&self, flow_id: &str) -> Result<(), String> {
        let id = draft_id(flow_id);
        self.db.run(|mut db| async move {
            FlowVersionRow::filter_by_id(&id)
                .delete()
                .exec(&mut db)
                .await
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
                // Every change to the live definition is the next version.
                record_version(tx, &saved, now).await?;
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
                // Versions are only written here, so this store keeps them
                // with their Flow rather than a trigger.
                let version = FlowVersionRow::fields();
                FlowVersionRow::filter(version.flow_id().eq(&id))
                    .delete()
                    .exec(tx)
                    .await?;
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
        // An adopted Flow arrives as its v1, exactly as it was.
        let versions = s.versions("f1").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!((versions[0].version, versions[0].draft), (1, false));
        assert_eq!(versions[0].flow, f);
        assert_eq!(versions[0].created_at, 5, "dated when it was last saved");
        // And deleting the Flow still takes its runs and their visits.
        s.delete("f1").unwrap();
        assert_eq!(texts(&s, "SELECT id FROM flow_runs"), Vec::<String>::new());
        assert_eq!(
            texts(&s, "SELECT id FROM flow_visits"),
            Vec::<String>::new()
        );
    }

    /// A Flow saved by a build without versions, on a file already on
    /// Toasty: written straight to `flows`, as that build did.
    fn old_flow_row(path: &Path, f: &Flow, updated_at: i64) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO flows (id, definition_json, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![f.id, serde_json::to_string(f).unwrap(), updated_at],
        )
        .unwrap();
    }

    #[test]
    fn existing_pipelines_become_v1_exactly_as_they_were_and_keep_their_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        // The file exists with today's tables; its Flows predate versions.
        drop(FlowStore::open(&path).unwrap());
        let (a, mut b) = (flow("a"), flow("b"));
        b.name = "Release".into();
        old_flow_row(&path, &a, 100);
        old_flow_row(&path, &b, 200);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let r = run("r1", &b, RunStatus::Succeeded, 150);
        conn.execute(
            "INSERT INTO flow_runs (id, flow_id, status, created_at, run_json) VALUES ('r1', 'b', 'succeeded', 150, ?1)",
            [serde_json::to_string(&r).unwrap()],
        )
        .unwrap();
        drop(conn);

        let s = FlowStore::open(&path).unwrap();
        for (f, at) in [(&a, 100), (&b, 200)] {
            let versions = s.versions(&f.id).unwrap();
            assert_eq!(versions.len(), 1, "{}", f.id);
            assert_eq!((versions[0].version, versions[0].draft), (1, false));
            assert_eq!(&versions[0].flow, f, "the definition is untouched");
            assert_eq!(versions[0].created_at, at);
        }
        // Live definitions and runs are as they were.
        assert_eq!(s.list().unwrap().len(), 2);
        assert_eq!(s.runs("b", 10).unwrap(), vec![r]);
        drop(s);
        // Opening again adds nothing.
        let s = FlowStore::open(&path).unwrap();
        assert_eq!(s.versions("a").unwrap().len(), 1);
        assert_eq!(
            texts(&s, "SELECT id FROM flow_versions ORDER BY id"),
            ["a:v1", "b:v1"]
        );
    }

    #[test]
    fn an_unreadable_old_flow_is_left_alone_by_the_backfill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        drop(FlowStore::open(&path).unwrap());
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO flows (id, definition_json, updated_at) VALUES ('bad', '{not json', 1)",
            [],
        )
        .unwrap();
        drop(conn);
        let s = FlowStore::open(&path).expect("one bad row does not stop the store");
        assert!(s.versions("bad").unwrap().is_empty());
    }

    #[test]
    fn each_change_to_a_flow_is_its_next_version_and_a_resave_is_not() {
        let s = store();
        let mut f = flow("f");
        s.upsert(&f).unwrap();
        s.upsert(&f).unwrap();
        assert_eq!(
            s.versions("f").unwrap().len(),
            1,
            "the same definition again"
        );
        f.name = "Renamed".into();
        s.upsert(&f).unwrap();
        let versions = s.versions("f").unwrap();
        let numbers: Vec<_> = versions.iter().map(|v| v.version).collect();
        assert_eq!(numbers, [2, 1], "newest first");
        assert_eq!(versions[0].flow.name, "Renamed");
        assert_ne!(versions[1].flow.name, "Renamed");
    }

    #[test]
    fn a_draft_stays_apart_from_the_live_flow_until_saved_as_the_next_version() {
        let s = store();
        let f = flow("f");
        s.upsert(&f).unwrap();
        let mut edited = f.clone();
        edited.description = "Only build Linux when main moved".into();
        let draft = s.save_draft(&edited).unwrap();
        assert_eq!((draft.version, draft.draft), (2, true));
        // Running reads the live Flow, which has not changed.
        assert_eq!(s.list().unwrap()[0].description, f.description);
        let versions = s.versions("f").unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|v| (v.version, v.draft))
                .collect::<Vec<_>>(),
            [(2, true), (1, false)],
            "the draft leads"
        );
        // Editing the draft again keeps one draft.
        edited.description = "Only build Linux when main has new commits".into();
        s.save_draft(&edited).unwrap();
        assert_eq!(s.versions("f").unwrap().len(), 2);
        let live = s.publish_draft("f").unwrap();
        assert_eq!(live.description, edited.description);
        assert_eq!(s.list().unwrap()[0].description, edited.description);
        let versions = s.versions("f").unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|v| (v.version, v.draft))
                .collect::<Vec<_>>(),
            [(2, false), (1, false)]
        );
        assert!(s.draft("f").unwrap().is_none());
    }

    #[test]
    fn a_discarded_draft_leaves_the_live_flow_and_its_versions() {
        let s = store();
        let f = flow("f");
        s.upsert(&f).unwrap();
        let mut edited = f.clone();
        edited.name = "Draft name".into();
        s.save_draft(&edited).unwrap();
        s.discard_draft("f").unwrap();
        assert!(s.draft("f").unwrap().is_none());
        assert_eq!(s.versions("f").unwrap().len(), 1);
        assert_eq!(s.list().unwrap()[0].name, f.name);
        assert!(s.publish_draft("f").is_err(), "nothing to save");
    }

    #[test]
    fn a_draft_needs_a_valid_definition_and_an_existing_flow() {
        let s = store();
        assert_eq!(s.save_draft(&flow("ghost")).unwrap_err(), "Flow not found");
        let f = flow("f");
        s.upsert(&f).unwrap();
        let mut broken = f.clone();
        broken.initial = "missing-state".into();
        assert!(s
            .save_draft(&broken)
            .unwrap_err()
            .starts_with("Flow is invalid"));
        assert!(s.draft("f").unwrap().is_none());
    }

    #[test]
    fn deleting_a_flow_deletes_its_versions_and_draft() {
        let s = store();
        let f = flow("f");
        s.upsert(&f).unwrap();
        let mut edited = f.clone();
        edited.name = "Next".into();
        s.save_draft(&edited).unwrap();
        s.delete("f").unwrap();
        assert_eq!(
            texts(&s, "SELECT id FROM flow_versions"),
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
