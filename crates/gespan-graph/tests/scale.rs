//! Runs on every PR in a debug build: a seeded 10k-node / 100k-edge
//! power-law graph, timed, plus the query plans the speed depends on.
mod support;

use std::time::{Duration, Instant};

use gespan_graph::query::{IN_AT, IN_NOW, OUT_AT, OUT_NOW};
use gespan_graph::{AsOf, Caps, EdgeFilter, Hops, NodeId};
use rusqlite::Connection;
use support::generate::{seeded_graph, Rng};
use support::{count, temp_store};

const BUDGET: Duration = Duration::from_millis(50);

fn plan(conn: &Connection, sql: &str, at: bool) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let detail = |r: &rusqlite::Row<'_>| r.get::<_, String>(3);
    let rows: Vec<String> = if at {
        let rows = stmt.query_map(rusqlite::params![1, 50, 0], detail).unwrap();
        rows.map(Result::unwrap).collect()
    } else {
        let rows = stmt.query_map(rusqlite::params![1, 50], detail).unwrap();
        rows.map(Result::unwrap).collect()
    };
    rows.join(" | ")
}

#[test]
fn seeded_10k_two_hop_is_fast_and_uses_the_indexes() {
    let (_dir, store, _path) = temp_store();
    let info = seeded_graph(&store, 7, 10_000, 100_000, 5);
    assert_eq!(count(&store, "graph_nodes"), 10_000);
    assert_eq!(count(&store, "graph_edges"), 100_000);
    {
        let (_d, again, _p) = temp_store();
        assert_eq!(
            seeded_graph(&again, 7, 10_000, 100_000, 5).edge_checksum,
            info.edge_checksum
        );
    }

    let mut rng = Rng::new(7);
    let mut starts: Vec<NodeId> = (0..200)
        .map(|_| NodeId(1 + rng.below(10_000) as i64))
        .collect();
    starts.push(info.hubs[0]);
    let two = Hops::new(2).unwrap();
    let (mut near_max, mut reach_max) = (Duration::ZERO, Duration::ZERO);
    // Each query's time is its fastest of three runs, so one stall on a
    // shared CI runner can't fail the budget; a slow plan is slow every time.
    let fastest = |run: &mut dyn FnMut()| {
        (0..3)
            .map(|_| {
                let t = Instant::now();
                run();
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    for &start in &starts {
        let mut hits = Vec::new();
        near_max = near_max.max(fastest(&mut || {
            hits = store.neighborhood(start, Caps::DEFAULT, AsOf::Now).unwrap();
        }));
        let mut reached = Vec::new();
        reach_max = reach_max.max(fastest(&mut || {
            reached = store
                .reachable(start, two, Caps::DEFAULT, AsOf::Now, &EdgeFilter::All)
                .unwrap();
        }));
        if start == info.hubs[0] {
            assert_eq!(hits.len(), 200, "the hub fills the total cap");
            assert_eq!(reached.len(), 200);
        }
    }
    eprintln!("10k/100k debug: neighborhood max {near_max:?}, reachable(2) max {reach_max:?}, hub degree {}", info.degrees[0]);
    assert!(near_max < BUDGET, "neighborhood max {near_max:?}");
    assert!(reach_max < BUDGET, "reachable max {reach_max:?}");

    // The per-node statements walk an index in ranking order and stop at k.
    // Dropping `valid_to` from the ORDER BY brings a temp B-tree back on
    // the as-of ones.
    let conn = store.connection();
    for (sql, at, index) in [
        (OUT_NOW, false, "graph_edges_out_open"),
        (IN_NOW, false, "graph_edges_in_open"),
        (OUT_AT, true, "graph_edges_out_all"),
        (IN_AT, true, "graph_edges_in_all"),
    ] {
        let p = plan(conn, sql, at);
        assert!(p.contains(index), "{index}: {p}");
        assert!(!p.contains("TEMP B-TREE"), "{index}: {p}");
    }
}
