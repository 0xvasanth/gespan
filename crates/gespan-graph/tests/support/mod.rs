//! Shared test helpers: a store in a temporary directory and epoch-ms
//! fixture times.
#![allow(dead_code)]

use std::path::PathBuf;

use gespan_graph::{GraphStore, FILE_NAME};
use tempfile::TempDir;

/// 2026-10-31T00:00:00Z in epoch milliseconds, the base board-store's tests
/// use. Every fixture time is this plus an offset.
pub const BASE: i64 = 1_793_404_800_000;

/// A fresh store in its own temporary directory. Bind it as
/// `let (dir, store, path) = temp_store();` so the store drops before the
/// directory.
pub fn temp_store() -> (TempDir, GraphStore, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(FILE_NAME);
    let store = GraphStore::open(&path).expect("open graph store");
    (dir, store, path)
}

use gespan_graph::{
    BackfillSource, EdgeFact, EdgeId, EdgeKey, EdgeKind, NodeFact, NodeId, NodeKind, NodeRef,
    Props, Scope, Source, TimeBasis, Timestamp,
};

/// `BASE + offset` as a timestamp.
pub fn ts(offset: i64) -> Timestamp {
    Timestamp::from_millis(BASE + offset)
}

pub fn nref(kind: NodeKind, key: &str) -> NodeRef {
    NodeRef::new(kind, key).expect("non-empty key")
}

pub fn props(pairs: &[(&'static str, serde_json::Value)]) -> Props {
    Props(pairs.iter().cloned().collect())
}

/// Upserts a node labelled with its key, seen at `BASE + at`.
pub fn add_node(store: &mut GraphStore, kind: NodeKind, key: &str, at: i64) -> NodeId {
    store
        .upsert_node(&NodeFact {
            node: nref(kind, key),
            label: key.to_string(),
            props: Props::default(),
            seen_at: ts(at),
        })
        .expect("upsert node")
}

/// An open edge fact from the `blocks` backfill, scoped to its source node,
/// with native time `BASE + from` and no props.
pub fn fact(kind: EdgeKind, src: &NodeRef, dst: &NodeRef, from: i64) -> EdgeFact {
    EdgeFact {
        kind,
        src: src.clone(),
        dst: dst.clone(),
        props: Props::default(),
        valid_from: ts(from),
        basis: TimeBasis::Native,
        valid_to: None,
        source: Source::Backfill(BackfillSource::Blocks),
        scope: Scope::Owner(src.clone()),
    }
}

pub fn key_of(fact: &EdgeFact) -> EdgeKey {
    EdgeKey {
        kind: fact.kind,
        src: fact.src.clone(),
        dst: fact.dst.clone(),
        source: fact.source,
        scope: fact.scope.clone(),
    }
}

pub fn count(store: &GraphStore, table: &str) -> i64 {
    store
        .connection()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

/// `(valid_from, valid_to, basis, props_json)` of one edge row.
pub fn edge_row(store: &GraphStore, id: EdgeId) -> (i64, Option<i64>, String, String) {
    store
        .connection()
        .query_row(
            "SELECT valid_from, valid_to, valid_from_basis, props_json FROM graph_edges WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("edge row")
}
