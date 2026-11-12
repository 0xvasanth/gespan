//! A seeded power-law graph loaded straight into `graph.db`.
use gespan_graph::{GraphStore, NodeId};
use rusqlite::params;

use super::BASE;

/// xorshift64*: small, seeded, and the same on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

pub struct GenInfo {
    /// Every node with an edge, by degree (open and closed) descending, then id.
    pub hubs: Vec<NodeId>,
    /// `degrees[i]` is `hubs[i]`'s degree.
    pub degrees: Vec<u64>,
    /// FNV-1a over `(src, dst, kind, valid_from, valid_to)` in id order.
    pub edge_checksum: u64,
}

const YEAR_MS: u64 = 31_536_000_000;

/// Fills an empty store: node ids `1..=nodes` (1% projects, 30% sessions, the
/// rest files) and exactly `edges` edges: 80% session→file `touched`, 10%
/// session→project and 10% file→project `in_project`, with file and project
/// endpoints drawn as `lo + floor(n·u⁴)` so
/// a few files and projects become hubs. `valid_from` takes one of 1,000
/// values in a year, so ties are common; `closed_pct`% of edges are closed.
pub fn seeded_graph(
    store: &GraphStore,
    seed: u64,
    nodes: usize,
    edges: usize,
    closed_pct: u32,
) -> GenInfo {
    let conn = store.connection();
    let nprojects = (nodes / 100).max(1);
    let nsessions = (nodes * 30 / 100).max(1);
    let nfiles = nodes - nprojects - nsessions;
    let (project_lo, session_lo, file_lo) = (1, 1 + nprojects, 1 + nprojects + nsessions);

    let indexes: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT name, sql FROM sqlite_master WHERE type = 'index' AND tbl_name = 'graph_edges' AND name <> 'graph_edges_open_uq' AND sql IS NOT NULL")
            .unwrap();
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert_eq!(indexes.len(), 6);

    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    for (name, _) in &indexes {
        conn.execute_batch(&format!("DROP INDEX {name}")).unwrap();
    }
    {
        let mut node = conn
            .prepare("INSERT INTO graph_nodes (id, kind, key, label, props_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?3, '{}', ?4, ?4)")
            .unwrap();
        for i in 1..=nodes {
            let kind = if i < session_lo {
                "project"
            } else if i < file_lo {
                "session"
            } else {
                "file"
            };
            node.execute(params![i as i64, kind, format!("{kind}-{i}"), BASE])
                .unwrap();
        }
        let mut rng = Rng::new(seed);
        let mut edge = conn
            .prepare(
                "INSERT OR IGNORE INTO graph_edges
                   (src, dst, kind, props_json, valid_from, valid_from_basis, valid_to, source, scope, created_at)
                 VALUES (?1, ?2, ?3, '{}', ?4, 'native', ?5, 'backfill:blocks', ?6, ?4)",
            )
            .unwrap();
        let mut written = 0;
        while written < edges {
            let session = session_lo + rng.below(nsessions as u64) as usize;
            let file = |rng: &mut Rng| file_lo + ((nfiles as f64) * rng.unit().powi(4)) as usize;
            let project =
                |rng: &mut Rng| project_lo + ((nprojects as f64) * rng.unit().powi(4)) as usize;
            // Files get edges in both directions (in from sessions, out to
            // projects), so expand's out/in merge is exercised.
            let (src, dst, kind, scope) = match rng.below(10) {
                0..=7 => (
                    session,
                    file(&mut rng),
                    "touched",
                    format!("session:session-{session}"),
                ),
                8 => (
                    session,
                    project(&mut rng),
                    "in_project",
                    format!("session:session-{session}"),
                ),
                _ => (
                    file(&mut rng),
                    project(&mut rng),
                    "in_project",
                    "structural".to_string(),
                ),
            };
            let from = BASE + (rng.below(1000) * (YEAR_MS / 1000)) as i64;
            let to = (rng.below(100) < u64::from(closed_pct))
                .then(|| from + 1 + rng.below(86_400_000) as i64);
            written += edge
                .execute(params![src as i64, dst as i64, kind, from, to, scope])
                .unwrap();
        }
    }
    for (_, sql) in &indexes {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch("COMMIT; ANALYZE;").unwrap();

    let mut checksum: u64 = 0xcbf2_9ce4_8422_2325;
    let mut fnv = |bytes: &[u8]| {
        for b in bytes {
            checksum ^= u64::from(*b);
            checksum = checksum.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    {
        let mut stmt = conn
            .prepare("SELECT src, dst, kind, valid_from, valid_to FROM graph_edges ORDER BY id")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            fnv(&r.get::<_, i64>(0).unwrap().to_le_bytes());
            fnv(&r.get::<_, i64>(1).unwrap().to_le_bytes());
            fnv(r.get::<_, String>(2).unwrap().as_bytes());
            fnv(&r.get::<_, i64>(3).unwrap().to_le_bytes());
            fnv(&r
                .get::<_, Option<i64>>(4)
                .unwrap()
                .unwrap_or(-1)
                .to_le_bytes());
        }
    }
    let mut stmt = conn
        .prepare(
            "SELECT n, count(*) AS d FROM (SELECT src AS n FROM graph_edges UNION ALL SELECT dst FROM graph_edges)
             GROUP BY n ORDER BY d DESC, n",
        )
        .unwrap();
    let ranked: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    GenInfo {
        hubs: ranked.iter().map(|(n, _)| NodeId(*n)).collect(),
        degrees: ranked.iter().map(|(_, d)| *d as u64).collect(),
        edge_checksum: checksum,
    }
}
