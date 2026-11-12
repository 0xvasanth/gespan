//! Capped graph queries. Every query takes [`Caps`]: uncapped starts at a
//! hub took 230–520 ms in the research, capped ones about a millisecond.
//!
//! **Ranking.** Both queries pick a node's `k` best incident edges, in either
//! direction, by `(valid_from DESC, neighbour id, valid_to with open first,
//! edge id)`. Edges back to the start or to nodes already found count toward
//! `k` and are then dropped, so a cap is "k edges", not "k new nodes".
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use toasty::Executor;

use crate::error::GraphError;
use crate::key::{EdgeId, NodeId, NodeKey};
use crate::kind::{EdgeKind, NodeKind};
use crate::schema::GraphNode;
use crate::store::{query, GraphStore};
use crate::time::Timestamp;

/// Per-node fan-out caps. `per_start` edges are expanded at the start,
/// `per_hop` at each later node, and at most `total` results come back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub per_start: u32,
    pub per_hop: u32,
    pub total: u32,
}

impl Caps {
    pub const DEFAULT: Caps = Caps {
        per_start: 50,
        per_hop: 20,
        total: 200,
    };
}

/// A hop count in `1..=4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hops(u8);

impl Hops {
    pub fn new(hops: u8) -> Result<Hops, GraphError> {
        if (1..=4).contains(&hops) {
            Ok(Hops(hops))
        } else {
            Err(GraphError::InvalidHops(hops))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// Which edges count: the open ones, or those valid at an instant. An edge
/// is valid at `t` when `valid_from <= t < valid_to` (half-open).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsOf {
    Now,
    At(Timestamp),
}

/// Which edge kinds a walk may follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeFilter {
    All,
    Only(BTreeSet<EdgeKind>),
}

/// A node found by [`GraphStore::neighborhood`], with the best edge that
/// reached it at its smallest hop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub node: NodeId,
    pub kind: NodeKind,
    pub key: NodeKey,
    pub label: String,
    pub hops: u8,
    pub via: EdgeKind,
    pub valid_from: Timestamp,
}

/// A node found by [`GraphStore::reachable`] at its smallest hop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reached {
    pub node: NodeId,
    pub kind: NodeKind,
    pub key: NodeKey,
    pub label: String,
    pub hops: u8,
}

// The per-node statements. `valid_to` sits before `id` in the ORDER BY so
// the `_all` indexes `(src|dst, valid_from DESC, other, valid_to)` serve the
// whole order: without it the as-of statements sort in a temp B-tree. Under
// `Now` every `valid_to` is NULL, so the term changes nothing.
//
// Raw SQL, though Toasty 0.11 could express them: a neighbourhood runs up to
// `2 + 2·per_start` of these, and Toasty plans every typed statement when it
// runs. The typed version measured three to four times slower and put
// `neighborhood` over the scale test's budget.
#[doc(hidden)]
pub const OUT_NOW: &str = "SELECT id, dst, kind, valid_from, valid_to FROM graph_edges
 WHERE src = ?1 AND valid_to IS NULL
 ORDER BY valid_from DESC, dst, valid_to, id LIMIT ?2";
#[doc(hidden)]
pub const IN_NOW: &str = "SELECT id, src, kind, valid_from, valid_to FROM graph_edges
 WHERE dst = ?1 AND valid_to IS NULL
 ORDER BY valid_from DESC, src, valid_to, id LIMIT ?2";
#[doc(hidden)]
pub const OUT_AT: &str = "SELECT id, dst, kind, valid_from, valid_to FROM graph_edges
 WHERE src = ?1 AND valid_from <= ?3 AND (valid_to IS NULL OR valid_to > ?3)
 ORDER BY valid_from DESC, dst, valid_to, id LIMIT ?2";
#[doc(hidden)]
pub const IN_AT: &str = "SELECT id, src, kind, valid_from, valid_to FROM graph_edges
 WHERE dst = ?1 AND valid_from <= ?3 AND (valid_to IS NULL OR valid_to > ?3)
 ORDER BY valid_from DESC, src, valid_to, id LIMIT ?2";

/// One incident edge, seen from the node being expanded.
#[derive(Clone, Copy, Debug)]
struct Cand {
    edge: EdgeId,
    nb: i64,
    kind: EdgeKind,
    valid_from: i64,
    valid_to: Option<i64>,
}

impl Cand {
    fn rank(&self) -> (Reverse<i64>, i64, Option<i64>, i64) {
        (
            Reverse(self.valid_from),
            self.nb,
            self.valid_to,
            self.edge.0,
        )
    }

    /// Among edges reaching the same node: the expand key without the
    /// (equal) neighbour.
    fn via_rank(&self) -> (Reverse<i64>, Option<i64>, i64) {
        (Reverse(self.valid_from), self.valid_to, self.edge.0)
    }
}

fn limit(k: u32) -> i64 {
    i64::from(k)
}

/// The `k` best qualifying edges incident to `node`: the top `k` of each
/// direction, merged and cut to `k`, which is exact because the top `k` of a
/// union lies inside the union of the two top-`k` lists.
async fn expand(
    ex: &mut dyn Executor,
    node: i64,
    k: u32,
    as_of: AsOf,
) -> Result<Vec<Cand>, GraphError> {
    let mut cands = Vec::new();
    if k == 0 {
        return Ok(cands);
    }
    let (out, inn) = match as_of {
        AsOf::Now => (OUT_NOW, IN_NOW),
        AsOf::At(_) => (OUT_AT, IN_AT),
    };
    for sql in [out, inn] {
        let mut statement = toasty::sql::query(sql).bind(node).bind(limit(k));
        if let AsOf::At(t) = as_of {
            statement = statement.bind(t.as_millis());
        }
        for mut r in query(ex, statement).await? {
            cands.push(Cand {
                edge: EdgeId(r.i64()?),
                nb: r.i64()?,
                kind: r.string()?.parse()?,
                valid_from: r.i64()?,
                valid_to: r.opt_i64()?,
            });
        }
    }
    cands.sort_by_key(Cand::rank);
    cands.truncate(k as usize);
    Ok(cands)
}

/// `(kind, key, label)` for each id, in one query.
async fn labels(
    ex: &mut dyn Executor,
    ids: impl Iterator<Item = i64>,
) -> Result<BTreeMap<i64, (NodeKind, NodeKey, String)>, GraphError> {
    let f = GraphNode::fields();
    let nodes = GraphNode::filter(f.id().in_list(ids.collect::<Vec<_>>()))
        .exec(ex)
        .await?;
    let mut out = BTreeMap::new();
    for node in nodes {
        let kind: NodeKind = node.kind.parse()?;
        out.insert(node.id, (kind, NodeKey::new(node.key)?, node.label));
    }
    Ok(out)
}

/// The ids of the `k` best qualifying edges incident to `node` (a column or
/// parameter), as a subquery: the same ranking as [`expand`].
fn top_k(node: &str, k: &str, at: bool, only: bool) -> String {
    let valid = if at {
        format!("valid_from <= {T} AND (valid_to IS NULL OR valid_to > {T})")
    } else {
        "valid_to IS NULL".to_string()
    };
    let kinds = if only {
        format!(
            " AND kind IN (SELECT value FROM json_each({}))",
            kinds_param(at)
        )
    } else {
        String::new()
    };
    format!(
        "SELECT id FROM (
           SELECT * FROM (SELECT id, dst AS nb, valid_from, valid_to FROM graph_edges
                          WHERE src = {node} AND {valid}{kinds}
                          ORDER BY valid_from DESC, dst, valid_to, id LIMIT {k})
           UNION ALL
           SELECT * FROM (SELECT id, src AS nb, valid_from, valid_to FROM graph_edges
                          WHERE dst = {node} AND {valid}{kinds}
                          ORDER BY valid_from DESC, src, valid_to, id LIMIT {k}))
         ORDER BY valid_from DESC, nb, valid_to, id LIMIT {k}"
    )
}

// `reachable`'s numbered parameters. `?6` (the as-of time) is there only for
// `At`, and the kinds take the next number, since SQLite counts parameters up
// to the highest one used and every one must be bound.
const START: &str = "?1";
const PER_START: &str = "?2";
const PER_HOP: &str = "?3";
const HOPS: &str = "?4";
const TOTAL: &str = "?5";
const T: &str = "?6";

fn kinds_param(at: bool) -> &'static str {
    if at {
        "?7"
    } else {
        "?6"
    }
}

/// One of the four fixed `reachable` statements (`{Now, At} × {All, Only}`).
/// Raw SQL: a recursive CTE, which Toasty 0.11 cannot express.
///
/// Hop 1 is seeded in a non-recursive term with `:per_start`; the recursive
/// term expands only depths `1..hops` with `:per_hop`. Each `LIMIT` is one
/// bound parameter because SQLite does not resolve `walk` columns in a
/// `LIMIT`. The start is never expanded again (`walk.node <> :start`):
/// otherwise a hop-1 node's edge back to it would put it at depth 2 and
/// expand it with `per_hop`, reporting direct neighbours at hops 3.
fn reachable_sql(at: bool, only: bool) -> String {
    let seed = top_k(START, PER_START, at, only);
    let step = top_k("walk.node", PER_HOP, at, only);
    format!(
        "WITH RECURSIVE walk(node, depth) AS (
           SELECT {START}, 0
           UNION
           SELECT CASE WHEN e.src = {START} THEN e.dst ELSE e.src END, 1
             FROM graph_edges e WHERE e.id IN ({seed})
           UNION
           SELECT CASE WHEN e.src = walk.node THEN e.dst ELSE e.src END, walk.depth + 1
             FROM walk JOIN graph_edges e ON e.id IN ({step})
            WHERE walk.depth >= 1 AND walk.depth < {HOPS} AND walk.node <> {START})
         SELECT w.node, MIN(w.depth) AS d, n.kind, n.key, n.label
           FROM walk w JOIN graph_nodes n ON n.id = w.node
          WHERE w.node <> {START}
          GROUP BY w.node
          ORDER BY d, w.node
          LIMIT {TOTAL}"
    )
}

impl GraphStore {
    /// The start's capped 2-hop neighbourhood (spec §6):
    /// 1. hop 1 expands the start with `per_start`;
    /// 2. hop 2 expands each hop-1 node with `per_hop`;
    /// 3. a node keeps its smallest hop, and its `via`/`valid_from` come from
    ///    the best edge that reached it there, by `(valid_from DESC, open
    ///    first, edge id)`;
    /// 4. the start is never a hit; hits are ordered by `(valid_from DESC,
    ///    node)` and cut to `total`.
    ///
    /// Reads at most `2·per_start + 2·per_start·per_hop` index entries (2,100
    /// with [`Caps::DEFAULT`]). An unknown or deleted start gives no hits.
    pub fn neighborhood(
        &self,
        start: NodeId,
        caps: Caps,
        as_of: AsOf,
    ) -> Result<Vec<Hit>, GraphError> {
        let mut best: BTreeMap<i64, (u8, Cand)> = BTreeMap::new();
        let offer = |best: &mut BTreeMap<i64, (u8, Cand)>, hops: u8, c: Cand| {
            if c.nb == start.0 {
                return;
            }
            let better = match best.get(&c.nb) {
                None => true,
                Some((h, old)) => hops < *h || (hops == *h && c.via_rank() < old.via_rank()),
            };
            if better {
                best.insert(c.nb, (hops, c));
            }
        };
        // One wait for the whole query; its statements run back to back on
        // the one connection.
        let (found, mut names) = self.read(|mut db| async move {
            let first = expand(&mut db, start.0, caps.per_start, as_of).await?;
            for c in &first {
                offer(&mut best, 1, *c);
            }
            let hop1: BTreeSet<i64> = first.iter().map(|c| c.nb).collect();
            for m in hop1 {
                for c in expand(&mut db, m, caps.per_hop, as_of).await? {
                    offer(&mut best, 2, c);
                }
            }
            let mut found: Vec<(i64, u8, Cand)> =
                best.into_iter().map(|(n, (h, c))| (n, h, c)).collect();
            found.sort_by_key(|(n, _, c)| (Reverse(c.valid_from), *n));
            found.truncate(caps.total as usize);
            let names = labels(&mut db, found.iter().map(|(n, _, _)| *n)).await?;
            Ok((found, names))
        })?;
        let mut hits = Vec::with_capacity(found.len());
        for (n, hops, c) in found {
            // A node deleted under us between statements has no label; skip it.
            if let Some((kind, key, label)) = names.remove(&n) {
                hits.push(Hit {
                    node: NodeId(n),
                    kind,
                    key,
                    label,
                    hops,
                    via: c.kind,
                    valid_from: Timestamp::from_millis(c.valid_from),
                });
            }
        }
        Ok(hits)
    }

    /// Every node within `hops` of the start along capped expansions, at its
    /// smallest hop, ordered by `(hops, node)` and cut to `total`. One
    /// recursive CTE (spec §6): hop 1 takes the start's `per_start` best
    /// edges, later hops each node's `per_hop` best, with the same ranking as
    /// [`GraphStore::neighborhood`]. The start is never expanded after hop 1
    /// and never returned. The shape was verified on the bundled SQLite
    /// 3.53.2.
    ///
    /// Reads at most `2·per_start·per_hop^(hops−1)` index entries per depth:
    /// 800k at `Hops 4` with [`Caps::DEFAULT`], so keep `hops` small on hubs.
    pub fn reachable(
        &self,
        start: NodeId,
        hops: Hops,
        caps: Caps,
        as_of: AsOf,
        filter: &EdgeFilter,
    ) -> Result<Vec<Reached>, GraphError> {
        let (at, t) = match as_of {
            AsOf::Now => (false, None),
            AsOf::At(t) => (true, Some(t.as_millis())),
        };
        let kinds = match filter {
            EdgeFilter::All => None,
            EdgeFilter::Only(kinds) => Some(serde_json::to_string(
                &kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            )?),
        };
        let mut statement = toasty::sql::query(reachable_sql(at, kinds.is_some()))
            .bind(start.0)
            .bind(limit(caps.per_start))
            .bind(limit(caps.per_hop))
            .bind(i64::from(hops.get()))
            .bind(limit(caps.total));
        if let Some(t) = t {
            statement = statement.bind(t);
        }
        if let Some(kinds) = kinds {
            statement = statement.bind(kinds);
        }
        let rows = self.read(|mut db| async move { query(&mut db, statement).await })?;
        let mut out = Vec::with_capacity(rows.len());
        for mut r in rows {
            out.push(Reached {
                node: NodeId(r.i64()?),
                hops: u8::try_from(r.i64()?).unwrap_or(u8::MAX),
                kind: r.string()?.parse()?,
                key: NodeKey::new(r.string()?)?,
                label: r.string()?,
            });
        }
        Ok(out)
    }
}
