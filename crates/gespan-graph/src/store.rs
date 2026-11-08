//! `GraphStore`: one connection to `graph.db`.
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::error::GraphError;
use crate::key::{EdgeId, NodeId, NodeRef, Props, Scope, Source, TimeBasis};
use crate::kind::EdgeKind;
use crate::time::Timestamp;
use crate::{migrate, schema};

/// The graph database's file name. The caller chooses the directory.
pub const FILE_NAME: &str = "graph.db";

/// A node as a source saw it at `seen_at`.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeFact {
    pub node: NodeRef,
    pub label: String,
    pub props: Props,
    pub seen_at: Timestamp,
}

/// An edge as a source asserts it, with the caller's validity times.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeFact {
    pub kind: EdgeKind,
    pub src: NodeRef,
    pub dst: NodeRef,
    pub props: Props,
    pub valid_from: Timestamp,
    pub basis: TimeBasis,
    /// `Some` when the fact arrives already closed.
    pub valid_to: Option<Timestamp>,
    pub source: Source,
    pub scope: Scope,
}

/// What identifies an edge's open row: at most one row per key is open.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeKey {
    pub kind: EdgeKind,
    pub src: NodeRef,
    pub dst: NodeRef,
    pub source: Source,
    pub scope: Scope,
}

/// What `close_edge` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed(EdgeId),
    /// No open row had that key; nothing changed.
    NotOpen,
}

/// The knowledge graph store. Every call blocks: use it from a background
/// thread, never the UI thread or an async executor. `Send`, not `Sync`.
pub struct GraphStore {
    pub(crate) conn: Connection,
    path: PathBuf,
}

impl GraphStore {
    /// Opens (creating if needed) the graph database at `path` and brings it
    /// to the current schema. A file written by a newer build is refused
    /// with `NewerSchema` before anything is written to it.
    pub fn open(path: &Path) -> Result<GraphStore, GraphError> {
        #[cfg(unix)]
        create_private(path)?;
        let conn = Connection::open(path)?;
        let found = migrate::version(&conn)?;
        if found > schema::VERSION {
            return Err(GraphError::NewerSchema {
                found,
                supported: schema::VERSION,
            });
        }
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;",
        )?;
        migrate::run(&conn)?;
        Ok(GraphStore {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// The file this store opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The node's id, or `None` when the graph doesn't hold it.
    pub fn node(&self, node: &NodeRef) -> Result<Option<NodeId>, GraphError> {
        Ok(node_id(&self.conn, node)?)
    }

    /// Inserts the node, or merges the fact into it: label and props are
    /// replaced, `created_at` keeps the earliest `seen_at` and `updated_at`
    /// the latest. The rowid never changes, so the node's edges stay.
    pub fn upsert_node(&mut self, fact: &NodeFact) -> Result<NodeId, GraphError> {
        let props = fact.props.to_json()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.prepare_cached(
            "INSERT INTO graph_nodes (kind, key, label, props_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT (kind, key) DO UPDATE SET
               label = excluded.label,
               props_json = excluded.props_json,
               created_at = min(created_at, excluded.created_at),
               updated_at = max(updated_at, excluded.updated_at)
             WHERE label IS NOT excluded.label
                OR props_json IS NOT excluded.props_json
                OR excluded.created_at < created_at
                OR excluded.updated_at > updated_at",
        )?
        .execute(params![
            fact.node.kind.as_str(),
            fact.node.key.as_str(),
            fact.label,
            props,
            fact.seen_at.as_millis(),
        ])?;
        let id =
            node_id(&tx, &fact.node)?.ok_or_else(|| GraphError::MissingNode(fact.node.clone()))?;
        tx.commit()?;
        Ok(id)
    }

    /// Records an edge fact so that writing it again changes nothing:
    /// - open fact, no open row: inserted;
    /// - open fact, open row: props updated in place, `valid_from` kept,
    ///   except that a structural edge takes an earlier `valid_from` (and its
    ///   basis), since many owners assert it and the earliest is the truth;
    /// - closed fact, open row: the row is closed at `max(valid_to, valid_from)`;
    /// - closed fact, no open row: inserted closed unless the same closed row
    ///   already exists.
    ///
    /// The endpoints must exist; nothing is written on error.
    pub fn upsert_edge(&mut self, fact: &EdgeFact) -> Result<EdgeId, GraphError> {
        if !fact.kind.allows(fact.src.kind, fact.dst.kind) {
            return Err(GraphError::InvalidEndpoints {
                edge: fact.kind,
                src: fact.src.kind,
                dst: fact.dst.kind,
            });
        }
        if fact.src == fact.dst {
            return Err(GraphError::SelfLoop(fact.src.clone()));
        }
        let props = fact.props.to_json()?;
        let source = fact.source.as_str();
        let scope = fact.scope.as_str();
        let from = fact.valid_from.as_millis();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let src = node_id(&tx, &fact.src)?
            .ok_or_else(|| GraphError::MissingNode(fact.src.clone()))?
            .0;
        let dst = node_id(&tx, &fact.dst)?
            .ok_or_else(|| GraphError::MissingNode(fact.dst.clone()))?
            .0;
        let kind = fact.kind.as_str();
        let open: Option<(i64, i64, String)> = tx
            .prepare_cached(
                "SELECT id, valid_from, props_json FROM graph_edges
                 WHERE kind = ?1 AND src = ?2 AND dst = ?3 AND source = ?4 AND scope = ?5
                   AND valid_to IS NULL",
            )?
            .query_row(params![kind, src, dst, source, scope], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let insert = |valid_to: Option<i64>| -> rusqlite::Result<i64> {
            tx.prepare_cached(
                "INSERT INTO graph_edges
                   (src, dst, kind, props_json, valid_from, valid_from_basis, valid_to, source, scope, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?5)",
            )?
            .execute(params![src, dst, kind, props, from, fact.basis.as_str(), valid_to, source, scope])?;
            Ok(tx.last_insert_rowid())
        };
        let id = match (fact.valid_to, open) {
            (None, None) => insert(None)?,
            (None, Some((id, open_from, open_props))) => {
                if open_props != props {
                    tx.prepare_cached("UPDATE graph_edges SET props_json = ?2 WHERE id = ?1")?
                        .execute(params![id, props])?;
                }
                if fact.scope == Scope::Structural && from < open_from {
                    tx.prepare_cached(
                        "UPDATE graph_edges SET valid_from = ?2, valid_from_basis = ?3 WHERE id = ?1",
                    )?
                    .execute(params![id, from, fact.basis.as_str()])?;
                }
                id
            }
            (Some(to), Some((id, _, _))) => {
                tx.prepare_cached(
                    "UPDATE graph_edges SET valid_to = max(?2, valid_from) WHERE id = ?1",
                )?
                .execute(params![id, to.as_millis()])?;
                id
            }
            (Some(to), None) => {
                // Clamped like close_edge, so an inverted fact still converges.
                let to = to.as_millis().max(from);
                let existing: Option<i64> = tx
                    .prepare_cached(
                        "SELECT id FROM graph_edges
                         WHERE kind = ?1 AND src = ?2 AND dst = ?3 AND source = ?4 AND scope = ?5
                           AND valid_from = ?6 AND valid_to = ?7
                         ORDER BY id LIMIT 1",
                    )?
                    .query_row(params![kind, src, dst, source, scope, from, to], |r| {
                        r.get(0)
                    })
                    .optional()?;
                match existing {
                    Some(id) => id,
                    None => insert(Some(to))?,
                }
            }
        };
        tx.commit()?;
        Ok(EdgeId(id))
    }

    /// Closes the key's open row at `max(at, valid_from)`: the time is the
    /// caller's, clamped so a clock behind the fact keeps the row valid. A
    /// later `upsert_edge` of the same open fact starts a new row.
    pub fn close_edge(&mut self, key: &EdgeKey, at: Timestamp) -> Result<CloseOutcome, GraphError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (Some(src), Some(dst)) = (node_id(&tx, &key.src)?, node_id(&tx, &key.dst)?) else {
            return Ok(CloseOutcome::NotOpen);
        };
        let closed: Option<i64> = tx
            .prepare_cached(
                "UPDATE graph_edges SET valid_to = max(?6, valid_from)
                 WHERE kind = ?1 AND src = ?2 AND dst = ?3 AND source = ?4 AND scope = ?5
                   AND valid_to IS NULL
                 RETURNING id",
            )?
            .query_row(
                params![
                    key.kind.as_str(),
                    src.0,
                    dst.0,
                    key.source.as_str(),
                    key.scope.as_str(),
                    at.as_millis()
                ],
                |r| r.get(0),
            )
            .optional()?;
        tx.commit()?;
        Ok(closed.map_or(CloseOutcome::NotOpen, |id| CloseOutcome::Closed(EdgeId(id))))
    }

    /// Every node and edge as one JSON-array line each, sorted bytewise, so
    /// two graphs holding the same facts dump equal whatever their insert
    /// order or rowids. Leaves out rowids, edge `created_at`,
    /// `last_seen_run` and the bookkeeping tables.
    ///
    /// - nodes: `["N", kind, key, label, props, created_at, updated_at]`
    /// - edges: `["E", kind, src_kind, src_key, dst_kind, dst_key, source,
    ///   scope, valid_from, basis, valid_to|null, props]`
    pub fn canonical_dump(&self) -> Result<String, GraphError> {
        let mut lines = Vec::new();
        let mut nodes = self.conn.prepare_cached(
            "SELECT kind, key, label, props_json, created_at, updated_at FROM graph_nodes",
        )?;
        let mut rows = nodes.query([])?;
        while let Some(r) = rows.next()? {
            let props: serde_json::Value = serde_json::from_str(&r.get::<_, String>(3)?)?;
            let line = serde_json::json!([
                "N",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                props,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ]);
            lines.push(line.to_string());
        }
        let mut edges = self.conn.prepare_cached(
            "SELECT e.kind, s.kind, s.key, d.kind, d.key, e.source, e.scope,
                    e.valid_from, e.valid_from_basis, e.valid_to, e.props_json
             FROM graph_edges e
             JOIN graph_nodes s ON s.id = e.src
             JOIN graph_nodes d ON d.id = e.dst",
        )?;
        let mut rows = edges.query([])?;
        while let Some(r) = rows.next()? {
            let props: serde_json::Value = serde_json::from_str(&r.get::<_, String>(10)?)?;
            let line = serde_json::json!([
                "E",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<i64>>(9)?,
                props,
            ]);
            lines.push(line.to_string());
        }
        lines.sort();
        Ok(lines.join("\n"))
    }

    /// The raw connection, for tests and the seeded generator only.
    #[doc(hidden)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn node_id(conn: &Connection, node: &NodeRef) -> rusqlite::Result<Option<NodeId>> {
    conn.prepare_cached("SELECT id FROM graph_nodes WHERE kind = ?1 AND key = ?2")?
        .query_row(params![node.kind.as_str(), node.key.as_str()], |r| r.get(0))
        .optional()
        .map(|id| id.map(NodeId))
}

/// Creates the file owner-only before SQLite first opens it, as the
/// settings sidecar does.
#[cfg(unix)]
fn create_private(path: &Path) -> Result<(), GraphError> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}
