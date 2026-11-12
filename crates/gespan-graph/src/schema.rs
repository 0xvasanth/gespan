//! The graph tables as Toasty models, their migrations and the constraints
//! Toasty's DDL cannot express.
//!
//! The migrations live with every other Gespan database's, in
//! `crates/gespan-core/migrations/graph/`, generated from [`models`] by
//! gespan-core's `toasty-migrate graph <name>`. gespan-core registers these
//! models and constraints as its `Graph` database kind, so its drift test
//! fails when a model changes without a migration.
//!
//! Toasty 0.11 writes no `CHECK`, `DEFAULT`, `REFERENCES`, partial or
//! descending index, so [`constraints`] adds them on every open:
//! - the kind `CHECK` lists, built from `NodeKind::ALL` and `EdgeKind::ALL`
//!   so the two can't drift, and the other checks, as triggers that abort;
//! - `graph_edges.src`/`dst` referencing `graph_nodes(id)` with `ON DELETE
//!   CASCADE`, as triggers;
//! - the partial (`WHERE valid_to IS NULL`) and `valid_from DESC` indexes.
//!   The `_all` indexes lead with the endpoint and then `valid_from DESC`, so
//!   a capped as-of read walks a hub's edges newest first and stops after
//!   `k`.
//!
//! `last_seen_run`, `graph_scopes`, `graph_run_scopes` and `graph_runs` are
//! #386's: created now so it needs no migration, and not written by 1a
//! (except that deleting an anchor deletes its scope rows).
use std::sync::LazyLock;

use crate::kind::{EdgeKind, NodeKind};

#[derive(Debug, toasty::Model)]
#[table = "graph_nodes"]
#[unique(name = "graph_nodes_kind_key", kind, key)]
pub(crate) struct GraphNode {
    #[key]
    #[auto]
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) key: String,
    pub(crate) label: String,
    #[default("{}".to_string())]
    pub(crate) props_json: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) last_seen_run: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[table = "graph_edges"]
#[index(name = "graph_edges_scope", scope)]
pub(crate) struct GraphEdge {
    #[key]
    #[auto]
    pub(crate) id: i64,
    pub(crate) src: i64,
    pub(crate) dst: i64,
    pub(crate) kind: String,
    #[default("{}".to_string())]
    pub(crate) props_json: String,
    pub(crate) valid_from: i64,
    pub(crate) valid_from_basis: String,
    pub(crate) valid_to: Option<i64>,
    pub(crate) source: String,
    pub(crate) scope: String,
    pub(crate) created_at: i64,
    pub(crate) last_seen_run: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[table = "graph_scopes"]
#[key(source, scope)]
pub(crate) struct GraphScope {
    source: String,
    scope: String,
    observed_updated_at: i64,
    observed_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "graph_run_scopes"]
#[key(run, source, scope)]
pub(crate) struct GraphRunScope {
    run: i64,
    source: String,
    scope: String,
    close_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "graph_runs"]
pub(crate) struct GraphRun {
    #[key]
    #[auto]
    id: i64,
    started_at: i64,
    finished_at: Option<i64>,
    observed_at: i64,
    status: String,
    report_json: Option<String>,
}

/// Every graph table, for gespan-core's `Graph` kind and the store.
pub fn models() -> toasty::ModelSet {
    toasty::models!(GraphNode, GraphEdge, GraphScope, GraphRunScope, GraphRun)
}

/// The graph migrations, embedded from gespan-core's `migrations/graph/`.
pub fn migrations() -> toasty::migration::MigrationSet {
    toasty::embed_migrations!("../gespan-core/migrations/graph")
}

fn quoted(names: impl Iterator<Item = &'static str>) -> String {
    names
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A `CHECK (condition)` on `table` as two triggers, one per insert and one
/// per update, that abort the statement (SQLite reports it as a constraint
/// violation). Like a `CHECK`, a NULL condition passes.
fn check(table: &str, name: &str, condition: &str) -> [String; 2] {
    ["INSERT", "UPDATE"].map(|event| {
        format!(
            "CREATE TRIGGER IF NOT EXISTS {name}_{lower} BEFORE {event} ON {table}
             WHEN NOT ({condition})
             BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: {name}'); END",
            lower = event.to_lowercase(),
        )
    })
}

/// `graph_edges.<column>` must name a node, like `REFERENCES graph_nodes(id)`.
fn reference(column: &str) -> [String; 2] {
    ["INSERT", "UPDATE"].map(|event| {
        format!(
            "CREATE TRIGGER IF NOT EXISTS graph_edges_{column}_fk_{lower} BEFORE {event} ON graph_edges
             WHEN NOT EXISTS (SELECT 1 FROM graph_nodes WHERE id = NEW.{column})
             BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed: graph_edges.{column}'); END",
            lower = event.to_lowercase(),
        )
    })
}

fn build() -> Vec<String> {
    let nodes = quoted(NodeKind::ALL.into_iter().map(NodeKind::as_str));
    let edges = quoted(EdgeKind::ALL.into_iter().map(EdgeKind::as_str));
    let mut sql = Vec::new();
    sql.extend(check(
        "graph_nodes",
        "graph_nodes_kind",
        &format!("NEW.kind IN ({nodes})"),
    ));
    sql.extend(check(
        "graph_edges",
        "graph_edges_kind",
        &format!("NEW.kind IN ({edges})"),
    ));
    sql.extend(check(
        "graph_edges",
        "graph_edges_basis",
        "NEW.valid_from_basis IN ('native', 'owner_created', 'observed_by')",
    ));
    sql.extend(check(
        "graph_edges",
        "graph_edges_valid",
        "NEW.valid_to IS NULL OR NEW.valid_to >= NEW.valid_from",
    ));
    sql.extend(check(
        "graph_edges",
        "graph_edges_no_loop",
        "NEW.src <> NEW.dst",
    ));
    sql.extend(check(
        "graph_runs",
        "graph_runs_status",
        "NEW.status IN ('running', 'completed', 'cancelled', 'failed')",
    ));
    sql.extend(reference("src"));
    sql.extend(reference("dst"));
    sql.push(
        "CREATE TRIGGER IF NOT EXISTS graph_nodes_cascade AFTER DELETE ON graph_nodes
         BEGIN
           DELETE FROM graph_edges WHERE src = OLD.id;
           DELETE FROM graph_edges WHERE dst = OLD.id;
         END"
        .into(),
    );
    sql.extend(
        [
            "CREATE UNIQUE INDEX IF NOT EXISTS graph_edges_open_uq ON graph_edges (kind, src, dst, source, scope) WHERE valid_to IS NULL",
            "CREATE INDEX IF NOT EXISTS graph_edges_scope_open ON graph_edges (source, scope) WHERE valid_to IS NULL",
            "CREATE INDEX IF NOT EXISTS graph_edges_out_open ON graph_edges (src, valid_from DESC, dst) WHERE valid_to IS NULL",
            "CREATE INDEX IF NOT EXISTS graph_edges_in_open ON graph_edges (dst, valid_from DESC, src) WHERE valid_to IS NULL",
            "CREATE INDEX IF NOT EXISTS graph_edges_out_all ON graph_edges (src, valid_from DESC, dst, valid_to)",
            "CREATE INDEX IF NOT EXISTS graph_edges_in_all ON graph_edges (dst, valid_from DESC, src, valid_to)",
        ]
        .map(String::from),
    );
    sql
}

/// What the graph tables need beyond Toasty's DDL, run after the migrations
/// on every open. Each statement is idempotent; to change one, add it under
/// a new name and drop the old one here.
pub fn constraints() -> &'static [&'static str] {
    static SQL: LazyLock<Vec<String>> = LazyLock::new(build);
    static REFS: LazyLock<Vec<&'static str>> =
        LazyLock::new(|| SQL.iter().map(String::as_str).collect());
    &REFS
}
