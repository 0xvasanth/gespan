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
