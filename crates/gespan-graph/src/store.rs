//! `GraphStore`: `graph.db` through Toasty, one connection.
//!
//! Typed queries go through the models in [`crate::schema`]; what Toasty
//! cannot express stays raw SQL on the same connection (`toasty::sql`): the
//! upsert with its min/max merge, `RETURNING`, the partial-index predicate
//! `valid_to IS NULL`, joins, counting deletes and the queries in
//! [`crate::query`].
use std::cell::OnceCell;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use rusqlite::Connection;
use toasty::schema::db::Type;
use toasty::stmt::Value;
use toasty::Executor;
use toasty_core::driver::operation::TransactionMode;

use crate::db;
use crate::error::GraphError;
use crate::key::{scope_of, EdgeId, NodeId, NodeRef, Props, Scope, SessionId, Source, TimeBasis};
use crate::kind::{EdgeKind, NodeKind};
use crate::schema::{GraphNode, GraphScope};
use crate::time::Timestamp;

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

/// What a delete removed, orphans included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeleteReport {
    pub nodes_removed: u64,
    pub edges_removed: u64,
}

/// The knowledge graph store. Every call blocks: use it from a background
/// thread, never the UI thread or an async executor. `Send`, not `Sync`:
///
/// ```
/// fn send<T: Send>() {}
/// send::<gespan_graph::GraphStore>();
/// ```
///
/// ```compile_fail
/// fn sync<T: Sync>() {}
/// sync::<gespan_graph::GraphStore>();
/// ```
pub struct GraphStore {
    /// A plain connection for tests and the seeded generator, opened on
    /// first use (see [`GraphStore::connection`]). It is also what keeps the
    /// store `!Sync`. Declared first, so it closes first.
    raw: OnceCell<Connection>,
    handle: db::Handle,
    path: PathBuf,
}

/// A store transaction's work, as [`GraphStore::write`] takes it.
type TxFuture<'t, T> = Pin<Box<dyn Future<Output = Result<T, GraphError>> + Send + 't>>;

/// The columns of one raw-SQL row, read in order.
pub(crate) struct Row(std::vec::IntoIter<Value>);

impl Row {
    fn next(&mut self) -> Result<Value, GraphError> {
        self.0
            .next()
            .ok_or_else(|| toasty::Error::invalid_result("a graph row has too few columns").into())
    }

    pub(crate) fn i64(&mut self) -> Result<i64, GraphError> {
        Ok(i64::try_from(self.next()?)?)
    }

    pub(crate) fn opt_i64(&mut self) -> Result<Option<i64>, GraphError> {
        match self.next()? {
            Value::Null => Ok(None),
            value => Ok(Some(i64::try_from(value)?)),
        }
    }

    pub(crate) fn string(&mut self) -> Result<String, GraphError> {
        Ok(String::try_from(self.next()?)?)
    }
}

/// Runs a raw query and returns its rows.
pub(crate) async fn query(
    ex: &mut dyn Executor,
    sql: toasty::sql::Query,
) -> Result<Vec<Row>, GraphError> {
    Ok(sql
        .exec(ex)
        .await?
        .into_iter()
        .map(|row| Row(row.into_record().fields.into_iter()))
        .collect())
}

/// `value` as a nullable INTEGER parameter: a NULL needs its type spelled out.
fn int(value: Option<i64>) -> (Value, Type) {
    (value.into(), Type::Integer(8))
}

impl GraphStore {
    /// Opens (creating if needed) the graph database at `path` and applies
    /// its pending migrations. A file with migrations this build does not
    /// know (written by a newer build) is refused with `UnknownMigrations`
    /// before anything is written to it.
    pub fn open(path: &Path) -> Result<GraphStore, GraphError> {
        #[cfg(unix)]
        create_private(path)?;
        let handle = db::open(path)?;
        Ok(GraphStore {
            raw: OnceCell::new(),
            handle,
            path: path.to_path_buf(),
        })
    }

    /// The file this store opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs `work` on the database and waits for it.
    pub(crate) fn read<T, F, Fut>(&self, work: F) -> Result<T, GraphError>
    where
        F: FnOnce(toasty::Db) -> Fut,
        Fut: Future<Output = Result<T, GraphError>> + Send,
        T: Send,
    {
        db::run(work(self.handle.db().clone()))
    }

    /// Runs `work` in one transaction that takes the write lock up front
    /// (`BEGIN IMMEDIATE`). It commits on `Ok` and rolls back otherwise, so
    /// nothing is written on error.
    fn write<T, F>(&mut self, work: F) -> Result<T, GraphError>
    where
        F: for<'c, 't> FnOnce(&'c mut toasty::Transaction<'t>) -> TxFuture<'c, T> + Send,
        T: Send,
    {
        let mut db = self.handle.db().clone();
        db::run(async move {
            let mut tx = db
                .transaction_builder()
                .mode(TransactionMode::Immediate)
                .begin()
                .await?;
            match work(&mut tx).await {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await?;
                    Err(error)
                }
            }
        })
    }

    /// The node's id, or `None` when the graph doesn't hold it.
    pub fn node(&self, node: &NodeRef) -> Result<Option<NodeId>, GraphError> {
        self.read(|mut db| async move { node_id(&mut db, node).await })
    }

    /// Inserts the node, or merges the fact into it: label and props are
    /// replaced, `created_at` keeps the earliest `seen_at` and `updated_at`
    /// the latest. The rowid never changes, so the node's edges stay.
    pub fn upsert_node(&mut self, fact: &NodeFact) -> Result<NodeId, GraphError> {
        let props = fact.props.to_json()?;
        // A conditional upsert with a min/max merge: raw SQL.
        let upsert = toasty::sql::statement(
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
        )
        .bind(fact.node.kind.as_str())
        .bind(fact.node.key.as_str())
        .bind(fact.label.as_str())
        .bind(props)
        .bind(fact.seen_at.as_millis());
        let node = fact.node.clone();
        self.write(|tx| {
            Box::pin(async move {
                upsert.exec(tx).await?;
                node_id(tx, &node)
                    .await?
                    .ok_or(GraphError::MissingNode(node))
            })
        })
    }

    /// Records an edge fact so that writing it again changes nothing:
    /// - open fact, no open row: inserted;
    /// - open fact, open row: props updated in place, `valid_from` kept,
    ///   except that a structural edge takes an earlier `valid_from` (and its
    ///   basis), since many owners assert it and the earliest is the truth;
    /// - closed fact, open row: the row is closed at `max(valid_to, valid_from)`;
    /// - closed fact, no open row: inserted closed unless a closed row with
    ///   the key already covers it (starts no later, ends at the same time).
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
        let fact = fact.clone();
        self.write(|tx| Box::pin(async move { upsert_edge_in(tx, &fact, props).await }))
    }

    /// Closes the key's open row at `max(at, valid_from)`: the time is the
    /// caller's, clamped so a clock behind the fact keeps the row valid. A
    /// later `upsert_edge` of the same open fact starts a new row.
    pub fn close_edge(&mut self, key: &EdgeKey, at: Timestamp) -> Result<CloseOutcome, GraphError> {
        let key = key.clone();
        self.write(|tx| {
            Box::pin(async move {
                let (Some(src), Some(dst)) =
                    (node_id(tx, &key.src).await?, node_id(tx, &key.dst).await?)
                else {
                    return Ok(CloseOutcome::NotOpen);
                };
                // An update that returns the row it changed: raw SQL.
                let closed = query(
                    tx,
                    toasty::sql::query(
                        "UPDATE graph_edges SET valid_to = max(?6, valid_from)
                         WHERE kind = ?1 AND src = ?2 AND dst = ?3 AND source = ?4 AND scope = ?5
                           AND valid_to IS NULL
                         RETURNING id",
                    )
                    .bind(key.kind.as_str())
                    .bind(src.0)
                    .bind(dst.0)
                    .bind(key.source.as_str())
                    .bind(key.scope.as_str())
                    .bind(at.as_millis()),
                )
                .await?;
                Ok(match closed.into_iter().next() {
                    Some(mut row) => CloseOutcome::Closed(EdgeId(row.i64()?)),
                    None => CloseOutcome::NotOpen,
                })
            })
        })
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
        let (nodes, edges) = self.read(|mut db| async move {
            let nodes = GraphNode::all().exec(&mut db).await?;
            // A join: raw SQL.
            let edges = query(
                &mut db,
                toasty::sql::query(
                    "SELECT e.kind, s.kind, s.key, d.kind, d.key, e.source, e.scope,
                            e.valid_from, e.valid_from_basis, e.valid_to, e.props_json
                     FROM graph_edges e
                     JOIN graph_nodes s ON s.id = e.src
                     JOIN graph_nodes d ON d.id = e.dst",
                ),
            )
            .await?;
            Ok((nodes, edges))
        })?;
        let mut lines = Vec::with_capacity(nodes.len() + edges.len());
        for node in nodes {
            let props: serde_json::Value = serde_json::from_str(&node.props_json)?;
            let line = serde_json::json!([
                "N",
                node.kind,
                node.key,
                node.label,
                props,
                node.created_at,
                node.updated_at,
            ]);
            lines.push(line.to_string());
        }
        for mut r in edges {
            let kind = r.string()?;
            let (src_kind, src_key, dst_kind, dst_key) =
                (r.string()?, r.string()?, r.string()?, r.string()?);
            let (source, scope) = (r.string()?, r.string()?);
            let (valid_from, basis, valid_to) = (r.i64()?, r.string()?, r.opt_i64()?);
            let props: serde_json::Value = serde_json::from_str(&r.string()?)?;
            let line = serde_json::json!([
                "E", kind, src_kind, src_key, dst_kind, dst_key, source, scope, valid_from, basis,
                valid_to, props,
            ]);
            lines.push(line.to_string());
        }
        lines.sort();
        Ok(lines.join("\n"))
    }

    /// Deletes an anchor (a session, pilot, automation run, card, note or
    /// memory) in one transaction:
    /// 1. every edge it asserted (`scope = "<kind>:<key>"`), open or closed,
    ///    even where it is not an endpoint and even if the node itself is not
    ///    in the graph;
    /// 2. the node, and with it every edge touching it;
    /// 3. its `graph_scopes` rows;
    /// 4. the orphans this leaves (see [`GraphStore::gc_orphans`]).
    ///
    /// An unknown anchor with no scoped edges is `Ok` with zero counts.
    pub fn delete_anchor(&mut self, node: &NodeRef) -> Result<DeleteReport, GraphError> {
        if !node.kind.is_anchor() {
            return Err(GraphError::NotAnchor(node.kind));
        }
        let scope = scope_of(node);
        let node = node.clone();
        self.write(|tx| {
            Box::pin(async move {
                // The cascade doesn't report its changes, so the report is
                // the difference of the totals.
                let before = totals(tx).await?;
                toasty::sql::statement("DELETE FROM graph_edges WHERE scope = ?1")
                    .bind(scope.as_str())
                    .exec(tx)
                    .await?;
                toasty::sql::statement("DELETE FROM graph_nodes WHERE kind = ?1 AND key = ?2")
                    .bind(node.kind.as_str())
                    .bind(node.key.as_str())
                    .exec(tx)
                    .await?;
                let f = GraphScope::fields();
                GraphScope::filter(f.scope().eq(scope))
                    .delete()
                    .exec(tx)
                    .await?;
                gc_in(tx).await?;
                let after = totals(tx).await?;
                Ok(DeleteReport {
                    nodes_removed: before.0.saturating_sub(after.0),
                    edges_removed: before.1.saturating_sub(after.1),
                })
            })
        })
    }

    /// `delete_anchor` for a persisted session.
    pub fn delete_session(&mut self, id: &SessionId) -> Result<DeleteReport, GraphError> {
        self.delete_anchor(&NodeRef::session(id)?)
    }

    /// Removes derived nodes no anchor reaches, and returns how many went:
    /// 1. derived nodes (other than repos) with no edge, open or closed, to an
    ///    anchor node;
    /// 2. then repos with no `of_repo` edge from a surviving PR or ticket.
    ///
    /// So a file only one session touched goes with that session even though
    /// it is still `in_project`, while a shared file and its project stay.
    /// Running it twice removes nothing the second time.
    ///
    /// Known limit: people only join derived nodes (`authored`/`assigned` to
    /// PRs and tickets), so pass 1 always collects them. Stage 1 writes none;
    /// whoever first writes people must give them a pass like repos'.
    pub fn gc_orphans(&mut self) -> Result<u64, GraphError> {
        self.write(|tx| Box::pin(async move { gc_in(tx).await }))
    }

    /// Closes the database and removes `graph.db` with its `-wal` and
    /// `-shm` files. If closing fails nothing is deleted.
    pub fn wipe(self) -> Result<(), GraphError> {
        let GraphStore { raw, handle, path } = self;
        if let Some(conn) = raw.into_inner() {
            conn.close().map_err(|(_, e)| e)?;
        }
        handle.close()?;
        GraphStore::wipe_files(&path)
    }

    /// Removes the database at `path` with its `-wal` and `-shm` files,
    /// without opening it. Files that are already absent are fine; any other
    /// failure (such as a file still open elsewhere on Windows) is an error.
    /// The WAL goes first: a failure part-way leaves the database whole, never
    /// a stray WAL that a new `graph.db` could replay.
    pub fn wipe_files(path: &Path) -> Result<(), GraphError> {
        for suffix in ["-wal", "-shm", ""] {
            let mut file = path.as_os_str().to_owned();
            file.push(suffix);
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// A plain SQLite connection to the same file (five second busy
    /// timeout), for tests and the seeded generator only. The store itself
    /// goes through Toasty.
    #[doc(hidden)]
    pub fn connection(&self) -> &Connection {
        self.raw.get_or_init(|| {
            let conn = Connection::open(&self.path).expect("open graph.db");
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .expect("busy timeout");
            conn
        })
    }
}

/// `upsert_edge` inside its transaction.
async fn upsert_edge_in(
    tx: &mut dyn Executor,
    fact: &EdgeFact,
    props: String,
) -> Result<EdgeId, GraphError> {
    let source = fact.source.as_str();
    let scope = fact.scope.as_str();
    let from = fact.valid_from.as_millis();
    let src = node_id(tx, &fact.src)
        .await?
        .ok_or_else(|| GraphError::MissingNode(fact.src.clone()))?
        .0;
    let dst = node_id(tx, &fact.dst)
        .await?
        .ok_or_else(|| GraphError::MissingNode(fact.dst.clone()))?
        .0;
    let kind = fact.kind.as_str();
    let basis = fact.basis.as_str();
    // `valid_to IS NULL` is the partial unique index's predicate: raw SQL.
    let open = query(
        tx,
        toasty::sql::query(
            "SELECT id, valid_from, props_json FROM graph_edges
             WHERE kind = ?1 AND src = ?2 AND dst = ?3 AND source = ?4 AND scope = ?5
               AND valid_to IS NULL",
        )
        .bind(kind)
        .bind(src)
        .bind(dst)
        .bind(source.as_str())
        .bind(scope.as_str()),
    )
    .await?
    .into_iter()
    .next()
    .map(|mut r| Ok::<_, GraphError>((r.i64()?, r.i64()?, r.string()?)))
    .transpose()?;
    let insert = |valid_to: Option<i64>| {
        let (valid_to, ty) = int(valid_to);
        toasty::sql::query(
            "INSERT INTO graph_edges
               (src, dst, kind, props_json, valid_from, valid_from_basis, valid_to, source, scope, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?5)
             RETURNING id",
        )
        .bind(src)
        .bind(dst)
        .bind(kind)
        .bind(props.as_str())
        .bind(from)
        .bind(basis)
        .bind_typed(valid_to, ty)
        .bind(source.as_str())
        .bind(scope.as_str())
    };
    let inserted = |rows: Vec<Row>| -> Result<i64, GraphError> {
        rows.into_iter()
            .next()
            .ok_or_else(|| toasty::Error::invalid_result("an insert returned no id"))?
            .i64()
    };
    let id = match (fact.valid_to, open) {
        (None, None) => inserted(query(tx, insert(None)).await?)?,
        (None, Some((id, open_from, open_props))) => {
            if open_props != props {
                toasty::sql::statement("UPDATE graph_edges SET props_json = ?2 WHERE id = ?1")
                    .bind(id)
                    .bind(props.as_str())
                    .exec(tx)
                    .await?;
            }
            if fact.scope == Scope::Structural && from < open_from {
                toasty::sql::statement(
                    "UPDATE graph_edges SET valid_from = ?2, valid_from_basis = ?3 WHERE id = ?1",
                )
                .bind(id)
                .bind(from)
                .bind(basis)
                .exec(tx)
                .await?;
            }
            id
        }
        (Some(to), Some((id, _, _))) => {
            toasty::sql::statement(
                "UPDATE graph_edges SET valid_to = max(?2, valid_from) WHERE id = ?1",
            )
            .bind(id)
            .bind(to.as_millis())
            .exec(tx)
            .await?;
            id
        }
        (Some(to), None) => {
            // Already recorded when a closed row with this key covers the
            // fact: it starts no later and ends where closing an open row
            // with this fact would have ended it (`max(to, valid_from)`).
            // That is the row an earlier write of the fact closed or
            // inserted, even when the open row kept an earlier start.
            let to = to.as_millis();
            let existing = query(
                tx,
                toasty::sql::query(
                    "SELECT id FROM graph_edges
                     WHERE kind = ?1 AND src = ?2 AND dst = ?3 AND source = ?4 AND scope = ?5
                       AND valid_to IS NOT NULL
                       AND valid_from <= ?6 AND valid_to = max(?7, valid_from)
                     ORDER BY id LIMIT 1",
                )
                .bind(kind)
                .bind(src)
                .bind(dst)
                .bind(source.as_str())
                .bind(scope.as_str())
                .bind(from)
                .bind(to),
            )
            .await?;
            match existing.into_iter().next() {
                Some(mut row) => row.i64()?,
                // Clamped like close_edge, so an inverted fact still converges.
                None => inserted(query(tx, insert(Some(to.max(from)))).await?)?,
            }
        }
    };
    Ok(EdgeId(id))
}

/// `(nodes, edges)` row counts.
async fn totals(ex: &mut dyn Executor) -> Result<(u64, u64), GraphError> {
    let mut row = query(
        ex,
        toasty::sql::query(
            "SELECT (SELECT count(*) FROM graph_nodes), (SELECT count(*) FROM graph_edges)",
        ),
    )
    .await?
    .into_iter()
    .next()
    .ok_or_else(|| toasty::Error::invalid_result("no totals"))?;
    Ok((row.i64()?.unsigned_abs(), row.i64()?.unsigned_abs()))
}

fn kind_list(keep: impl Fn(NodeKind) -> bool) -> String {
    NodeKind::ALL
        .into_iter()
        .filter(|k| keep(*k))
        .map(|k| format!("'{}'", k.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `gc_orphans`' two passes inside the caller's transaction. The `OR` of
/// "edge out to an anchor / edge in from an anchor" is two `EXISTS` clauses
/// so each probes its own index.
async fn gc_in(ex: &mut dyn Executor) -> Result<u64, GraphError> {
    let anchors = kind_list(NodeKind::is_anchor);
    let derived = kind_list(|k| !k.is_anchor() && k != NodeKind::Repo);
    let first = toasty::sql::statement(format!(
        "DELETE FROM graph_nodes
         WHERE kind IN ({derived})
           AND NOT EXISTS (SELECT 1 FROM graph_edges e JOIN graph_nodes a ON a.id = e.dst
                           WHERE e.src = graph_nodes.id AND a.kind IN ({anchors}))
           AND NOT EXISTS (SELECT 1 FROM graph_edges e JOIN graph_nodes a ON a.id = e.src
                           WHERE e.dst = graph_nodes.id AND a.kind IN ({anchors}))"
    ))
    .exec(ex)
    .await?;
    let repos = toasty::sql::statement(
        "DELETE FROM graph_nodes
         WHERE kind = 'repo'
           AND NOT EXISTS (SELECT 1 FROM graph_edges e JOIN graph_nodes p ON p.id = e.src
                           WHERE e.dst = graph_nodes.id AND e.kind = 'of_repo'
                             AND p.kind IN ('pr', 'ticket'))",
    )
    .exec(ex)
    .await?;
    Ok(first + repos)
}

async fn node_id(ex: &mut dyn Executor, node: &NodeRef) -> Result<Option<NodeId>, GraphError> {
    let f = GraphNode::fields();
    let found = GraphNode::filter(
        f.kind()
            .eq(node.kind.as_str())
            .and(f.key().eq(node.key.as_str())),
    )
    .first()
    .exec(ex)
    .await?;
    Ok(found.map(|node| NodeId(node.id)))
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
