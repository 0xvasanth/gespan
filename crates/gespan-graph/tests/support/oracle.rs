//! An independent model of spec §6's queries over petgraph, for checking
//! the store's SQL against.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use gespan_graph::{AsOf, Caps, EdgeFilter, EdgeKind};
use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use petgraph::Direction;
use rusqlite::Connection;

#[derive(Clone, Copy, Debug)]
pub struct EdgeW {
    pub id: i64,
    pub kind: EdgeKind,
    pub valid_from: i64,
    pub valid_to: Option<i64>,
}

/// One incident edge as `expand` sees it.
#[derive(Clone, Copy, Debug)]
struct Cand {
    id: i64,
    nb: i64,
    kind: EdgeKind,
    valid_from: i64,
    valid_to: Option<i64>,
}

impl Cand {
    /// Spec §6's ranking: `(valid_from DESC, neighbour, valid_to NULL first, id)`.
    fn rank(&self) -> (Reverse<i64>, i64, Option<i64>, i64) {
        (Reverse(self.valid_from), self.nb, self.valid_to, self.id)
    }

    /// The `via` choice among edges reaching one node (spec §12b):
    /// `(valid_from DESC, valid_to NULL first, id)`.
    fn via_rank(&self) -> (Reverse<i64>, Option<i64>, i64) {
        (Reverse(self.valid_from), self.valid_to, self.id)
    }
}

pub struct Oracle {
    graph: StableGraph<i64, EdgeW>,
    index: HashMap<i64, NodeIndex>,
}

fn qualifies(e: &EdgeW, as_of: AsOf) -> bool {
    match as_of {
        AsOf::Now => e.valid_to.is_none(),
        AsOf::At(t) => {
            let t = t.as_millis();
            e.valid_from <= t && e.valid_to.is_none_or(|to| to > t)
        }
    }
}

fn passes(e: &EdgeW, filter: &EdgeFilter) -> bool {
    match filter {
        EdgeFilter::All => true,
        EdgeFilter::Only(kinds) => kinds.contains(&e.kind),
    }
}

impl Oracle {
    /// Every node and edge row of the store, open and closed.
    pub fn load(conn: &Connection) -> Oracle {
        let mut graph = StableGraph::new();
        let mut index = HashMap::new();
        let mut nodes = conn
            .prepare("SELECT id FROM graph_nodes ORDER BY id")
            .unwrap();
        for id in nodes.query_map([], |r| r.get::<_, i64>(0)).unwrap() {
            let id = id.unwrap();
            index.insert(id, graph.add_node(id));
        }
        let mut edges = conn
            .prepare("SELECT id, src, dst, kind, valid_from, valid_to FROM graph_edges ORDER BY id")
            .unwrap();
        let rows = edges
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (id, src, dst, kind, valid_from, valid_to) = row.unwrap();
            let w = EdgeW {
                id,
                kind: kind.parse().unwrap(),
                valid_from,
                valid_to,
            };
            graph.add_edge(index[&src], index[&dst], w);
        }
        Oracle { graph, index }
    }

    /// The `k` best qualifying edges incident to `n`, in either direction.
    fn expand(&self, n: i64, k: u32, as_of: AsOf, filter: &EdgeFilter) -> Vec<Cand> {
        let Some(&ix) = self.index.get(&n) else {
            return Vec::new();
        };
        let mut cands = Vec::new();
        for dir in [Direction::Outgoing, Direction::Incoming] {
            for e in self.graph.edges_directed(ix, dir) {
                let w = e.weight();
                if !qualifies(w, as_of) || !passes(w, filter) {
                    continue;
                }
                let other = if dir == Direction::Outgoing {
                    e.target()
                } else {
                    e.source()
                };
                cands.push(Cand {
                    id: w.id,
                    nb: self.graph[other],
                    kind: w.kind,
                    valid_from: w.valid_from,
                    valid_to: w.valid_to,
                });
            }
        }
        cands.sort_by_key(Cand::rank);
        cands.truncate(k as usize);
        cands
    }

    /// Spec §6's `neighborhood`: `(node, hops, via, valid_from)` in hit order.
    pub fn neighborhood(
        &self,
        start: i64,
        caps: Caps,
        as_of: AsOf,
    ) -> Vec<(i64, u8, EdgeKind, i64)> {
        let mut best: BTreeMap<i64, (u8, Cand)> = BTreeMap::new();
        let offer = |hops: u8, c: Cand, best: &mut BTreeMap<i64, (u8, Cand)>| {
            if c.nb == start {
                return;
            }
            match best.get(&c.nb) {
                Some((h, _)) if *h < hops => {}
                Some((h, old)) if *h == hops && old.via_rank() <= c.via_rank() => {}
                _ => {
                    best.insert(c.nb, (hops, c));
                }
            }
        };
        let first = self.expand(start, caps.per_start, as_of, &EdgeFilter::All);
        for c in &first {
            offer(1, *c, &mut best);
        }
        let hop1: BTreeSet<i64> = first.iter().map(|c| c.nb).filter(|n| *n != start).collect();
        for m in hop1 {
            for c in self.expand(m, caps.per_hop, as_of, &EdgeFilter::All) {
                offer(2, c, &mut best);
            }
        }
        let mut hits: Vec<(i64, u8, EdgeKind, i64)> = best
            .into_iter()
            .map(|(n, (h, c))| (n, h, c.kind, c.valid_from))
            .collect();
        hits.sort_by_key(|(n, _, _, from)| (Reverse(*from), *n));
        hits.truncate(caps.total as usize);
        hits
    }

    /// Spec §6's `reachable` CTE: per-depth sets, where every node at a depth
    /// below `hops` is expanded except the start (spec §12b), and each node
    /// keeps its smallest depth. `(node, hops)` ordered by hops then node.
    pub fn reachable(
        &self,
        start: i64,
        hops: u8,
        caps: Caps,
        as_of: AsOf,
        filter: &EdgeFilter,
    ) -> Vec<(i64, u8)> {
        let mut levels: Vec<BTreeSet<i64>> = Vec::new();
        levels.push(
            self.expand(start, caps.per_start, as_of, filter)
                .iter()
                .map(|c| c.nb)
                .collect(),
        );
        for _ in 1..hops {
            let mut next = BTreeSet::new();
            for &n in levels.last().unwrap() {
                if n == start {
                    continue;
                }
                next.extend(
                    self.expand(n, caps.per_hop, as_of, filter)
                        .iter()
                        .map(|c| c.nb),
                );
            }
            levels.push(next);
        }
        let mut depth: BTreeMap<i64, u8> = BTreeMap::new();
        for (i, level) in levels.iter().enumerate() {
            for &n in level {
                if n != start {
                    depth.entry(n).or_insert(i as u8 + 1);
                }
            }
        }
        let mut out: Vec<(i64, u8)> = depth.into_iter().collect();
        out.sort_by_key(|(n, d)| (*d, *n));
        out.truncate(caps.total as usize);
        out
    }

    /// Plain k-hop distances from petgraph's `dijkstra` over the undirected
    /// view of qualifying edges, start excluded: a library check on the model
    /// above when caps are inert.
    pub fn k_hop(&self, start: i64, h: u8, as_of: AsOf, filter: &EdgeFilter) -> BTreeMap<i64, u8> {
        let mut un: petgraph::graph::UnGraph<i64, ()> = petgraph::graph::UnGraph::default();
        let mut ix = HashMap::new();
        for n in self.graph.node_indices() {
            ix.insert(self.graph[n], un.add_node(self.graph[n]));
        }
        for e in self.graph.edge_references() {
            if qualifies(e.weight(), as_of) && passes(e.weight(), filter) {
                un.add_edge(ix[&self.graph[e.source()]], ix[&self.graph[e.target()]], ());
            }
        }
        let Some(&s) = ix.get(&start) else {
            return BTreeMap::new();
        };
        petgraph::algo::dijkstra(&un, s, None, |_| 1u32)
            .into_iter()
            .filter(|(n, d)| *n != s && *d <= u32::from(h))
            .map(|(n, d)| (un[n], d as u8))
            .collect()
    }
}
