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
