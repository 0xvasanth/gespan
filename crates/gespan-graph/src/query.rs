//! Capped graph queries. Every query takes [`Caps`]: uncapped starts at a
//! hub took 230–520 ms in the research, capped ones about a millisecond.
//!
//! **Ranking.** Both queries pick a node's `k` best incident edges, in either
//! direction, by `(valid_from DESC, neighbour id, valid_to with open first,
//! edge id)`. Edges back to the start or to nodes already found count toward
//! `k` and are then dropped, so a cap is "k edges", not "k new nodes".
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection};

use crate::error::GraphError;
use crate::key::{EdgeId, NodeId, NodeKey};
use crate::kind::{EdgeKind, NodeKind};
use crate::store::GraphStore;
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
fn expand(conn: &Connection, node: i64, k: u32, as_of: AsOf) -> Result<Vec<Cand>, GraphError> {
    let mut cands = Vec::new();
    if k == 0 {
        return Ok(cands);
    }
    let (out, inn) = match as_of {
        AsOf::Now => (OUT_NOW, IN_NOW),
        AsOf::At(_) => (OUT_AT, IN_AT),
    };
    for sql in [out, inn] {
        let mut stmt = conn.prepare_cached(sql)?;
        let mut rows = match as_of {
            AsOf::Now => stmt.query(params![node, limit(k)])?,
            AsOf::At(t) => stmt.query(params![node, limit(k), t.as_millis()])?,
        };
        while let Some(r) = rows.next()? {
            cands.push(Cand {
                edge: EdgeId(r.get(0)?),
                nb: r.get(1)?,
                kind: r.get::<_, String>(2)?.parse()?,
                valid_from: r.get(3)?,
                valid_to: r.get(4)?,
            });
        }
    }
    cands.sort_by_key(Cand::rank);
    cands.truncate(k as usize);
    Ok(cands)
}

/// `(kind, key, label)` for each id, in one statement.
fn labels(
    conn: &Connection,
    ids: impl Iterator<Item = i64>,
) -> Result<BTreeMap<i64, (NodeKind, NodeKey, String)>, GraphError> {
    let ids = serde_json::to_string(&ids.collect::<Vec<_>>())?;
    let mut stmt = conn.prepare_cached(
        "SELECT id, kind, key, label FROM graph_nodes WHERE id IN (SELECT value FROM json_each(?1))",
    )?;
    let mut rows = stmt.query([ids])?;
    let mut out = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let kind: NodeKind = r.get::<_, String>(1)?.parse()?;
        out.insert(
            r.get(0)?,
            (kind, NodeKey::new(r.get::<_, String>(2)?)?, r.get(3)?),
        );
    }
    Ok(out)
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
        let conn = &self.conn;
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
        let first = expand(conn, start.0, caps.per_start, as_of)?;
        for c in &first {
            offer(&mut best, 1, *c);
        }
        let hop1: BTreeSet<i64> = first.iter().map(|c| c.nb).collect();
        for m in hop1 {
            for c in expand(conn, m, caps.per_hop, as_of)? {
                offer(&mut best, 2, c);
            }
        }
        let mut found: Vec<(i64, u8, Cand)> =
            best.into_iter().map(|(n, (h, c))| (n, h, c)).collect();
        found.sort_by_key(|(n, _, c)| (Reverse(c.valid_from), *n));
        found.truncate(caps.total as usize);
        let mut names = labels(conn, found.iter().map(|(n, _, _)| *n))?;
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
}
