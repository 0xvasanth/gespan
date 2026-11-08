# gespan-graph

Gespan's knowledge graph: projects, repos, worktrees, branches, sessions,
pilots, automation runs, files, PRs, tickets, cards, people, notes and
memories, and the edges between them. It lives in its own `graph.db` next to
`gespan.db`, so wiping it means deleting one file and its `-wal`/`-shm`.

This crate is Stage 1a (#385): the schema, the store and the queries. Nothing
opens or writes `graph.db` outside tests yet. The backfill from `gespan.db`
and reconciliation are #386, and live updates are #131/#132.

## Rules

- **Times are epoch milliseconds**, and the store never reads the clock.
  Every `valid_from`, `valid_to` and `seen_at` is the caller's, so writing the
  same facts twice leaves the same graph (`canonical_dump` is equal).
- **Every query is capped.** `neighborhood` (2 hops, app-side) and
  `reachable` (1–4 hops, a recursive CTE) both take `Caps`; there is no
  uncapped method. Both rank a node's incident edges by `(valid_from DESC,
  neighbour id, open first, edge id)`. Both can read the graph as of a past
  instant (`AsOf::At`, half-open: `valid_from <= t < valid_to`).
- **Threading.** Every call blocks. Call from a background thread, never the
  UI thread or an async executor. `GraphStore` is `Send`, not `Sync`.
- **Privacy.** Keys and labels are stored verbatim; callers redact them.
  On Unix the file is created `0600`.
- **Deleting.** `delete_session`/`delete_anchor` remove the anchor, every
  edge it asserted (by scope, even where it isn't an endpoint), its
  `graph_scopes` rows and the orphans left behind.

## Benchmark

The per-PR test (`tests/scale.rs`) times a 10k-node / 100k-edge graph in a
debug build. The release benchmark behind the "under 50 ms" target is
ignored by default:

```sh
cargo test --release -p gespan-graph --test scale_100k -- --ignored --nocapture
```

It seeds 100,000 nodes and 1,000,000 power-law edges (top hub about 28k
edges), then prints p50/p95/max over 1,000 random starts and the max over the
20 biggest hubs.
