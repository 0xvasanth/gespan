//! Schema v1. The kind `CHECK` lists are built from `NodeKind::ALL` and
//! `EdgeKind::ALL`, so the two can't drift.
use crate::kind::{EdgeKind, NodeKind};

/// The schema version this build writes.
pub const VERSION: i64 = 1;

fn quoted(names: impl Iterator<Item = &'static str>) -> String {
    names
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The v1 DDL. `last_seen_run`, `graph_scopes`, `graph_run_scopes` and
/// `graph_runs` are #386's: created here so it needs no migration, and not
/// written by 1a (except that deleting an anchor deletes its scope rows).
///
/// The `_all` indexes lead with the endpoint and then `valid_from DESC`, so a
/// capped as-of read walks a hub's edges newest first and stops after `k`.
pub fn ddl() -> String {
    let nodes = quoted(NodeKind::ALL.into_iter().map(NodeKind::as_str));
    let edges = quoted(EdgeKind::ALL.into_iter().map(EdgeKind::as_str));
    format!(
        "CREATE TABLE graph_schema (version INTEGER NOT NULL);
CREATE TABLE graph_nodes (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ({nodes})),
  key TEXT NOT NULL,
  label TEXT NOT NULL,
  props_json TEXT NOT NULL DEFAULT '{{}}',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_seen_run INTEGER,
  UNIQUE (kind, key)
);
CREATE TABLE graph_edges (
  id INTEGER PRIMARY KEY,
  src INTEGER NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
  dst INTEGER NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ({edges})),
  props_json TEXT NOT NULL DEFAULT '{{}}',
  valid_from INTEGER NOT NULL,
  valid_from_basis TEXT NOT NULL CHECK (valid_from_basis IN ('native', 'owner_created', 'observed_by')),
  valid_to INTEGER,
  source TEXT NOT NULL,
  scope TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_seen_run INTEGER,
  CHECK (valid_to IS NULL OR valid_to >= valid_from),
  CHECK (src <> dst)
);
CREATE UNIQUE INDEX graph_edges_open_uq ON graph_edges (kind, src, dst, source, scope) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_scope_open ON graph_edges (source, scope) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_scope ON graph_edges (scope);
CREATE INDEX graph_edges_out_open ON graph_edges (src, valid_from DESC, dst) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_in_open ON graph_edges (dst, valid_from DESC, src) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_out_all ON graph_edges (src, valid_from DESC, dst, valid_to);
CREATE INDEX graph_edges_in_all ON graph_edges (dst, valid_from DESC, src, valid_to);
CREATE TABLE graph_scopes (
  source TEXT NOT NULL,
  scope TEXT NOT NULL,
  observed_updated_at INTEGER NOT NULL,
  observed_at INTEGER NOT NULL,
  PRIMARY KEY (source, scope)
);
CREATE TABLE graph_run_scopes (
  run INTEGER NOT NULL,
  source TEXT NOT NULL,
  scope TEXT NOT NULL,
  close_at INTEGER NOT NULL,
  PRIMARY KEY (run, source, scope)
);
CREATE TABLE graph_runs (
  id INTEGER PRIMARY KEY,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  observed_at INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'cancelled', 'failed')),
  report_json TEXT
);
"
    )
}
