//! The release benchmark behind #385's "done when": 2-hop queries under
//! 50 ms (max) on a seeded 100k-node / 1M-edge power-law graph. Ignored; run
//! it with
//! `cargo test --release -p gespan-graph --test scale_100k -- --ignored --nocapture`.
mod support;

use std::time::{Duration, Instant};

use gespan_graph::{AsOf, Caps, EdgeFilter, GraphStore, Hops, NodeId};
use support::generate::{seeded_graph, Rng};
use support::{count, temp_store, ts};

const SEED: u64 = 0x6765_7370_616e;
const PINNED: u64 = 0xd121_938b_281d_538b;
const BUDGET: Duration = Duration::from_millis(50);

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// `(p50, p95, max)` of the random starts and the max of the hub starts.
fn time(
    store: &GraphStore,
    random: &[NodeId],
    hubs: &[NodeId],
    query: impl Fn(&GraphStore, NodeId) -> usize,
) -> (Duration, Duration, Duration, Duration) {
    let mut runs: Vec<Duration> = random
        .iter()
        .map(|&s| {
            let t = Instant::now();
            std::hint::black_box(query(store, s));
            t.elapsed()
        })
        .collect();
    let hub_max = hubs
        .iter()
        .map(|&s| {
            let t = Instant::now();
            assert!(query(store, s) > 0, "a hub start must find nodes");
            t.elapsed()
        })
        .max()
        .unwrap();
    runs.sort();
    let at = |q: f64| runs[((runs.len() - 1) as f64 * q).round() as usize];
    (at(0.50), at(0.95), *runs.last().unwrap(), hub_max)
}

#[test]
#[ignore = "release benchmark: 100k nodes / 1M edges"]
fn two_hop_under_50ms_at_100k_nodes_1m_edges() {
    let (_dir, store, _path) = temp_store();
    let loaded = Instant::now();
    let info = seeded_graph(&store, SEED, 100_000, 1_000_000, 5);
    let load = loaded.elapsed();
    assert_eq!(count(&store, "graph_nodes"), 100_000);
    assert_eq!(count(&store, "graph_edges"), 1_000_000);
    println!(
        "load {:.2} s, top hub degree {}, edge checksum {:#x}",
        load.as_secs_f64(),
        info.degrees[0],
        info.edge_checksum
    );
    assert!(
        info.degrees[0] >= 20_000,
        "top hub degree {}",
        info.degrees[0]
    );
    assert_eq!(
        info.edge_checksum, PINNED,
        "the generator changed: re-pin after checking why"
    );

    let mut rng = Rng::new(SEED);
    let random: Vec<NodeId> = (0..1_000)
        .map(|_| NodeId(1 + rng.below(100_000) as i64))
        .collect();
    let hubs = &info.hubs[..20];
    let two = Hops::new(2).unwrap();
    let mid = AsOf::At(ts(15_768_000_000));
    let rows = [
        (
            "neighborhood Now",
            time(&store, &random, hubs, |s, n| {
                s.neighborhood(n, Caps::DEFAULT, AsOf::Now).unwrap().len()
            }),
        ),
        (
            "neighborhood At(mid)",
            time(&store, &random, hubs, |s, n| {
                s.neighborhood(n, Caps::DEFAULT, mid).unwrap().len()
            }),
        ),
        (
            "reachable(Hops 2) Now",
            time(&store, &random, hubs, |s, n| {
                s.reachable(n, two, Caps::DEFAULT, AsOf::Now, &EdgeFilter::All)
                    .unwrap()
                    .len()
            }),
        ),
    ];
    println!("| query | p50 | p95 | max | hub max |");
    println!("|---|---|---|---|---|");
    for (name, (p50, p95, max, hub)) in &rows {
        println!(
            "| {name} | {:.3} | {:.3} | {:.3} | {:.3} |",
            ms(*p50),
            ms(*p95),
            ms(*max),
            ms(*hub)
        );
    }
    for (name, (_, _, max, hub)) in &rows {
        assert!(
            *max < BUDGET && *hub < BUDGET,
            "{name}: max {max:?}, hub max {hub:?}"
        );
    }
}
