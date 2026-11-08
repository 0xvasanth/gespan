mod support;

use gespan_graph::{
    AsOf, Caps, EdgeFilter, EdgeKind as E, GraphError, GraphStore, Hit, Hops, NodeId,
    NodeKind as N, SessionId, Timestamp,
};
use support::generate::{seeded_graph, Rng};
use support::oracle::Oracle;
use support::*;

const INERT: Caps = Caps {
    per_start: u32::MAX,
    per_hop: u32::MAX,
    total: u32::MAX,
};
const SMALL: Caps = Caps {
    per_start: 3,
    per_hop: 2,
    total: 5,
};
const YEAR: i64 = 31_536_000_000;

fn probes() -> [AsOf; 4] {
    [
        AsOf::Now,
        AsOf::At(ts(YEAR / 4)),
        AsOf::At(ts(YEAR / 2)),
        AsOf::At(ts(YEAR * 3 / 4)),
    ]
}

fn tuples(hits: &[Hit]) -> Vec<(i64, u8, E, i64)> {
    hits.iter()
        .map(|h| (h.node.0, h.hops, h.via, h.valid_from.as_millis()))
        .collect()
}

/// A seeded 2k-node / 20k-edge store, its oracle, and 50 starts (the top hub
/// first, then PRNG picks).
fn seeded(seed: u64) -> (tempfile::TempDir, GraphStore, Oracle, Vec<NodeId>) {
    let (dir, store, _path) = temp_store();
    let info = seeded_graph(&store, seed, 2_000, 20_000, 10);
    let oracle = Oracle::load(store.connection());
    let mut rng = Rng::new(seed ^ 0xa5a5);
    let mut starts = vec![info.hubs[0]];
    starts.extend((0..49).map(|_| NodeId(1 + rng.below(2_000) as i64)));
    (dir, store, oracle, starts)
}

#[test]
fn neighborhood_matches_oracle() {
    let mut compared = 0;
    for seed in 1..=5 {
        let (_dir, store, oracle, starts) = seeded(seed);
        for &start in &starts {
            for caps in [INERT, Caps::DEFAULT, SMALL] {
                for as_of in probes() {
                    let got = store.neighborhood(start, caps, as_of).unwrap();
                    let want = oracle.neighborhood(start.0, caps, as_of);
                    assert_eq!(
                        tuples(&got),
                        want,
                        "seed {seed} start {start:?} {caps:?} {as_of:?}"
                    );
                    compared += usize::from(!want.is_empty());
                }
            }
        }
    }
    assert!(
        compared > 2_000,
        "most probes must return hits to prove anything: {compared}"
    );
}

#[test]
fn inert_caps_equal_petgraph_k_hop() {
    for seed in [1, 2] {
        let (_dir, store, oracle, starts) = seeded(seed);
        for &start in starts.iter().take(20) {
            for as_of in [AsOf::Now, AsOf::At(ts(YEAR / 2))] {
                let hits = store.neighborhood(start, INERT, as_of).unwrap();
                let got: std::collections::BTreeMap<i64, u8> =
                    hits.iter().map(|h| (h.node.0, h.hops)).collect();
                assert_eq!(
                    got,
                    oracle.k_hop(start.0, 2, as_of, &EdgeFilter::All),
                    "seed {seed} start {start:?}"
                );
            }
        }
    }
}

/// Session hub H and files f1..f6 in that id order, H's edges written
/// newest-neighbour-last so EdgeId order disagrees with neighbour order.
fn hub_store() -> (tempfile::TempDir, GraphStore, Vec<NodeId>) {
    let (dir, mut store, _path) = temp_store();
    let mut ids = vec![add_node(&mut store, N::Session, "H", 0)];
    for i in 1..=6 {
        ids.push(add_node(&mut store, N::File, &format!("f{i}"), 0));
    }
    ids.push(add_node(&mut store, N::Session, "S2", 0));
    let h = nref(N::Session, "H");
    for (file, from) in [
        ("f4", 30),
        ("f3", 30),
        ("f2", 30),
        ("f1", 10),
        ("f5", 20),
        ("f6", 5),
    ] {
        store
            .upsert_edge(&fact(E::Touched, &h, &nref(N::File, file), from))
            .unwrap();
    }
    // S2 reaches f5 with an edge older than H's, so f5's best edge leads back
    // to H.
    store
        .upsert_edge(&fact(
            E::Touched,
            &nref(N::Session, "S2"),
            &nref(N::File, "f5"),
            1,
        ))
        .unwrap();
    (dir, store, ids)
}

#[test]
fn tie_break_and_caps_on_a_hand_built_hub() {
    let (_dir, store, ids) = hub_store();
    let (h, f2, f3, f4, f5, s2) = (ids[0], ids[2], ids[3], ids[4], ids[5], ids[7]);
    let at = |o: i64| BASE + o;

    // per_start 4 keeps the three BASE+30 ties (by neighbour id, not edge id)
    // and BASE+20; f1 and f6 fall outside the cut. f5's one hop-2 slot is
    // spent on the edge back to H, so S2 is not reached.
    let hits = store
        .neighborhood(
            h,
            Caps {
                per_start: 4,
                per_hop: 1,
                total: 10,
            },
            AsOf::Now,
        )
        .unwrap();
    assert_eq!(
        tuples(&hits),
        vec![
            (f2.0, 1, E::Touched, at(30)),
            (f3.0, 1, E::Touched, at(30)),
            (f4.0, 1, E::Touched, at(30)),
            (f5.0, 1, E::Touched, at(20)),
        ]
    );
    assert_eq!(
        (hits[0].kind, hits[0].key.as_str(), hits[0].label.as_str()),
        (N::File, "f2", "f2")
    );

    // A second hop-2 slot reaches S2 through f5.
    let hits = store
        .neighborhood(
            h,
            Caps {
                per_start: 4,
                per_hop: 2,
                total: 10,
            },
            AsOf::Now,
        )
        .unwrap();
    assert_eq!(tuples(&hits).last(), Some(&(s2.0, 2, E::Touched, at(1))));
    assert_eq!(hits.len(), 5);

    // total cuts the ordered list.
    let hits = store
        .neighborhood(
            h,
            Caps {
                per_start: 4,
                per_hop: 2,
                total: 2,
            },
            AsOf::Now,
        )
        .unwrap();
    assert_eq!(
        tuples(&hits),
        vec![(f2.0, 1, E::Touched, at(30)), (f3.0, 1, E::Touched, at(30))]
    );
}

#[test]
fn open_edge_beats_one_closed_after_t_whatever_the_insert_order() {
    for closed_first in [true, false] {
        let (_dir, mut store, _path) = temp_store();
        let h = add_node(&mut store, N::Session, "H", 0);
        let f = add_node(&mut store, N::File, "f", 0);
        let s2 = add_node(&mut store, N::Session, "S2", 0);
        let fr = nref(N::File, "f");
        // The same pair of equal-time edges from H (hop 1) and from S2 (hop 2):
        // `touched` closed after t, `read` open.
        for session in ["H", "S2"] {
            let sr = nref(N::Session, session);
            let mut closed = fact(E::Touched, &sr, &fr, 10);
            closed.valid_to = Some(ts(50));
            let open = fact(E::Read, &sr, &fr, 10);
            let order = if closed_first {
                [closed, open]
            } else {
                [open, closed]
            };
            for e in &order {
                store.upsert_edge(e).unwrap();
            }
        }
        let hits = store
            .neighborhood(h, Caps::DEFAULT, AsOf::At(ts(20)))
            .unwrap();
        assert_eq!(
            tuples(&hits),
            vec![(f.0, 1, E::Read, BASE + 10), (s2.0, 2, E::Read, BASE + 10)],
            "closed first: {closed_first}"
        );
        let oracle = Oracle::load(store.connection());
        assert_eq!(
            oracle.neighborhood(h.0, Caps::DEFAULT, AsOf::At(ts(20))),
            tuples(&hits)
        );
    }
}

#[test]
fn as_of_is_half_open() {
    let (_dir, mut store, _path) = temp_store();
    let s = add_node(&mut store, N::Session, "S", 0);
    let a = add_node(&mut store, N::Branch, "a", 0);
    let b = add_node(&mut store, N::Branch, "b", 0);
    let sr = nref(N::Session, "S");
    let mut on_a = fact(E::OnBranch, &sr, &nref(N::Branch, "a"), 10);
    on_a.valid_to = Some(ts(20));
    store.upsert_edge(&on_a).unwrap();
    store
        .upsert_edge(&fact(E::OnBranch, &sr, &nref(N::Branch, "b"), 20))
        .unwrap();

    let nodes = |as_of| -> Vec<NodeId> {
        store
            .neighborhood(s, Caps::DEFAULT, as_of)
            .unwrap()
            .iter()
            .map(|h| h.node)
            .collect()
    };
    assert_eq!(nodes(AsOf::At(ts(9))), vec![]);
    assert_eq!(nodes(AsOf::At(ts(19))), vec![a]);
    assert_eq!(nodes(AsOf::At(ts(20))), vec![b]);
    assert_eq!(nodes(AsOf::Now), vec![b]);
}

#[test]
fn as_of_picks_the_right_history_row() {
    let (_dir, mut store, _path) = temp_store();
    let s = add_node(&mut store, N::Session, "S", 0);
    add_node(&mut store, N::Branch, "main", 0);
    let on = fact(
        E::OnBranch,
        &nref(N::Session, "S"),
        &nref(N::Branch, "main"),
        10,
    );
    store.upsert_edge(&on).unwrap();
    store.close_edge(&key_of(&on), ts(20)).unwrap();
    store
        .upsert_edge(&fact(
            E::OnBranch,
            &nref(N::Session, "S"),
            &nref(N::Branch, "main"),
            30,
        ))
        .unwrap();

    let from = |t| -> Vec<i64> {
        store
            .neighborhood(s, Caps::DEFAULT, AsOf::At(ts(t)))
            .unwrap()
            .iter()
            .map(|h| h.valid_from.as_millis())
            .collect()
    };
    assert_eq!(from(15), vec![BASE + 10]);
    assert_eq!(from(25), Vec::<i64>::new());
    assert_eq!(from(35), vec![BASE + 30]);
}

#[test]
fn bad_inputs() {
    assert!(matches!(Hops::new(0), Err(GraphError::InvalidHops(0))));
    assert!(matches!(Hops::new(5), Err(GraphError::InvalidHops(5))));
    assert_eq!(Hops::new(4).unwrap().get(), 4);

    let (_dir, mut store, _path) = temp_store();
    assert_eq!(store.node(&nref(N::Session, "nobody")).unwrap(), None);
    let lonely = add_node(&mut store, N::Session, "lonely", 0);
    assert!(store
        .neighborhood(lonely, Caps::DEFAULT, AsOf::Now)
        .unwrap()
        .is_empty());

    let s = add_node(&mut store, N::Session, "S", 0);
    add_node(&mut store, N::File, "f", 0);
    store
        .upsert_edge(&fact(
            E::Touched,
            &nref(N::Session, "S"),
            &nref(N::File, "f"),
            0,
        ))
        .unwrap();
    assert_eq!(
        store
            .neighborhood(s, Caps::DEFAULT, AsOf::Now)
            .unwrap()
            .len(),
        1
    );
    let zero = Caps {
        per_start: 0,
        ..Caps::DEFAULT
    };
    assert!(store.neighborhood(s, zero, AsOf::Now).unwrap().is_empty());
    let none = Caps {
        total: 0,
        ..Caps::DEFAULT
    };
    assert!(store.neighborhood(s, none, AsOf::Now).unwrap().is_empty());

    // A stale id (its node was deleted) is an empty result, not an error.
    store.delete_session(&SessionId("S".into())).unwrap();
    assert_eq!(
        store.neighborhood(s, Caps::DEFAULT, AsOf::Now).unwrap(),
        vec![]
    );
    let _ = Timestamp::from_millis(0);
}

fn reached(r: &[gespan_graph::Reached]) -> Vec<(i64, u8)> {
    r.iter().map(|x| (x.node.0, x.hops)).collect()
}

fn hops(h: u8) -> Hops {
    Hops::new(h).unwrap()
}

#[test]
fn reachable_matches_oracle() {
    use std::collections::BTreeSet;
    let filters = [
        EdgeFilter::All,
        EdgeFilter::Only(BTreeSet::from([E::InProject])),
        EdgeFilter::Only(BTreeSet::from([E::Touched, E::Read])),
    ];
    let mut compared = 0;
    for seed in 1..=3 {
        let (_dir, store, oracle, starts) = seeded(seed);
        for (i, &start) in starts.iter().enumerate() {
            for caps in [INERT, Caps::DEFAULT, SMALL] {
                // Wide walks cover most of a 2k-node graph and dominate the
                // run time, so they sample the starts: inert caps take a third
                // of them up to Hops 2 (inert_reachable_equals_petgraph_k_hop
                // covers inert Hops 3-4 against petgraph), and DEFAULT at
                // Hops 4 takes every fifth, the top hub included.
                if caps == INERT && i % 3 != 0 {
                    continue;
                }
                let deepest = match caps {
                    INERT => 2,
                    Caps::DEFAULT if i % 5 != 0 => 3,
                    _ => 4,
                };
                for h in 1..=deepest {
                    for filter in &filters {
                        for as_of in [AsOf::Now, AsOf::At(ts(YEAR / 2))] {
                            let got = store
                                .reachable(start, hops(h), caps, as_of, filter)
                                .unwrap();
                            let want = oracle.reachable(start.0, h, caps, as_of, filter);
                            assert_eq!(reached(&got), want, "seed {seed} start {start:?} hops {h} {caps:?} {filter:?} {as_of:?}");
                            compared += usize::from(!want.is_empty());
                        }
                    }
                }
            }
        }
    }
    assert!(
        compared > 2_000,
        "most probes must return nodes to prove anything: {compared}"
    );
}

#[test]
fn inert_reachable_equals_petgraph_k_hop() {
    for seed in [1, 2] {
        let (_dir, store, oracle, starts) = seeded(seed);
        for &start in starts.iter().take(10) {
            for h in 1..=4 {
                for as_of in [AsOf::Now, AsOf::At(ts(YEAR / 2))] {
                    let got = store
                        .reachable(start, hops(h), INERT, as_of, &EdgeFilter::All)
                        .unwrap();
                    let got: std::collections::BTreeMap<i64, u8> =
                        reached(&got).into_iter().collect();
                    assert_eq!(
                        got,
                        oracle.k_hop(start.0, h, as_of, &EdgeFilter::All),
                        "seed {seed} start {start:?} hops {h}"
                    );
                }
            }
        }
    }
}

/// Eight raw edges on nodes 1..=7 with known ids, from the CTE spike.
/// From 1: 1-2 (100), 1-3 (90), 1-4 (80). From 2: 2-6 (50) and 2-5 (50), a
/// tie the neighbour id breaks. From 3: 3-7 (40), and 3-5 (40) closed at 60.
/// 6-4 (30).
fn spike_store() -> (tempfile::TempDir, GraphStore) {
    let (dir, store, _path) = temp_store();
    let c = store.connection();
    for i in 1..=7 {
        c.execute(
            "INSERT INTO graph_nodes (id, kind, key, label, created_at, updated_at) VALUES (?1, 'file', 'n' || ?1, 'n' || ?1, 0, 0)",
            [i],
        )
        .unwrap();
    }
    for (id, s, d, from, to) in [
        (1, 1, 2, 100, None),
        (2, 1, 3, 90, None),
        (3, 1, 4, 80, None),
        (4, 2, 6, 50, None),
        (5, 3, 7, 40, None),
        (6, 3, 5, 40, Some(60)),
        (7, 6, 4, 30, None),
        (8, 2, 5, 50, None),
    ] {
        c.execute(
            "INSERT INTO graph_edges (id, src, dst, kind, valid_from, valid_from_basis, valid_to, source, scope, created_at)
             VALUES (?1, ?2, ?3, 'touched', ?4, 'native', ?5, 'backfill:blocks', 'structural', ?4)",
            rusqlite::params![id, s, d, from, to],
        )
        .unwrap();
    }
    (dir, store)
}

#[test]
fn reachable_caps_and_ties_on_a_hand_built_graph() {
    let (_dir, store) = spike_store();
    let run = |per_start, per_hop, h| {
        let caps = Caps {
            per_start,
            per_hop,
            total: 100,
        };
        reached(
            &store
                .reachable(NodeId(1), hops(h), caps, AsOf::Now, &EdgeFilter::All)
                .unwrap(),
        )
    };
    // Each hop-1 node's single slot is its edge back to the start.
    assert_eq!(run(2, 1, 2), vec![(2, 1), (3, 1)]);
    // 2's 50-tie goes to neighbour 5 over 6; 3's closed edge to 5 doesn't
    // count; 5 and 7 only lead back to depth-1 nodes.
    assert_eq!(run(2, 2, 3), vec![(2, 1), (3, 1), (5, 2), (7, 2)]);
    assert_eq!(
        run(2, 2, 1),
        vec![(2, 1), (3, 1)],
        "Hops 1 is the seed alone"
    );
    // total cuts by (hops, node).
    let caps = Caps {
        per_start: 2,
        per_hop: 2,
        total: 3,
    };
    let got = store
        .reachable(NodeId(1), hops(3), caps, AsOf::Now, &EdgeFilter::All)
        .unwrap();
    assert_eq!(reached(&got), vec![(2, 1), (3, 1), (5, 2)]);
    assert_eq!(
        (got[2].kind, got[2].key.as_str(), got[2].label.as_str()),
        (N::File, "n5", "n5")
    );
}

#[test]
fn reachable_never_reexpands_the_start() {
    let (_dir, mut store, _path) = temp_store();
    let h = add_node(&mut store, N::Session, "H", 0);
    let mut files = Vec::new();
    for (i, from) in [50, 40, 30, 20, 10].into_iter().enumerate() {
        let key = format!("f{}", i + 1);
        files.push(add_node(&mut store, N::File, &key, 0));
        store
            .upsert_edge(&fact(
                E::Touched,
                &nref(N::Session, "H"),
                &nref(N::File, &key),
                from,
            ))
            .unwrap();
    }
    let s2 = add_node(&mut store, N::Session, "S2", 0);
    store
        .upsert_edge(&fact(
            E::Touched,
            &nref(N::Session, "S2"),
            &nref(N::File, "f2"),
            5,
        ))
        .unwrap();

    // Hop 1 keeps f1 and f2. Depth 2 reaches H again (and S2); H must not be
    // expanded with per_hop 20, or f3..f5 would come back as hops 3.
    let caps = Caps {
        per_start: 2,
        per_hop: 20,
        total: 100,
    };
    let got = store
        .reachable(h, hops(3), caps, AsOf::Now, &EdgeFilter::All)
        .unwrap();
    assert_eq!(
        reached(&got),
        vec![(files[0].0, 1), (files[1].0, 1), (s2.0, 2)]
    );
    let oracle = Oracle::load(store.connection());
    assert_eq!(
        oracle.reachable(h.0, 3, caps, AsOf::Now, &EdgeFilter::All),
        reached(&got)
    );
}

#[test]
fn reachable_as_of_and_bad_inputs() {
    let (_dir, mut store, _path) = temp_store();
    let s = add_node(&mut store, N::Session, "S", 0);
    let a = add_node(&mut store, N::Branch, "a", 0);
    let b = add_node(&mut store, N::Branch, "b", 0);
    let sr = nref(N::Session, "S");
    let mut on_a = fact(E::OnBranch, &sr, &nref(N::Branch, "a"), 10);
    on_a.valid_to = Some(ts(20));
    store.upsert_edge(&on_a).unwrap();
    store
        .upsert_edge(&fact(E::OnBranch, &sr, &nref(N::Branch, "b"), 20))
        .unwrap();
    // Half-open, and the same answer neighborhood gives (as_of_is_half_open).
    let at = |as_of| {
        reached(
            &store
                .reachable(s, hops(2), Caps::DEFAULT, as_of, &EdgeFilter::All)
                .unwrap(),
        )
    };
    assert_eq!(at(AsOf::At(ts(9))), vec![]);
    assert_eq!(at(AsOf::At(ts(19))), vec![(a.0, 1)]);
    assert_eq!(at(AsOf::At(ts(20))), vec![(b.0, 1)]);
    assert_eq!(at(AsOf::Now), vec![(b.0, 1)]);
    let only = EdgeFilter::Only(std::collections::BTreeSet::from([E::Touched]));
    assert!(store
        .reachable(s, hops(2), Caps::DEFAULT, AsOf::Now, &only)
        .unwrap()
        .is_empty());
    let zero = Caps {
        per_start: 0,
        ..Caps::DEFAULT
    };
    assert!(store
        .reachable(s, hops(2), zero, AsOf::Now, &EdgeFilter::All)
        .unwrap()
        .is_empty());

    let lonely = add_node(&mut store, N::Session, "lonely", 0);
    assert!(store
        .reachable(lonely, hops(4), Caps::DEFAULT, AsOf::Now, &EdgeFilter::All)
        .unwrap()
        .is_empty());
    store.delete_session(&SessionId("S".into())).unwrap();
    assert!(store
        .reachable(s, hops(4), Caps::DEFAULT, AsOf::Now, &EdgeFilter::All)
        .unwrap()
        .is_empty());
}
