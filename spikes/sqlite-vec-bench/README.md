# Spike: SQLite + sqlite-vec for Gespan memory and the knowledge graph

Question: can the SQLite we already ship hold the embeddings for memory and
the decision/knowledge graph, or do we need another database? Run on
2026-11-12, Apple M2 Max, sqlite-vec 0.1.9 statically linked into
`rusqlite` 0.40.2 (bundled), WAL on, the same settings as Gespan's stores.

**Answer: yes, SQLite works, as long as every query is scoped (partition key)
or uses binary vectors with a float rescore. Plain exact float search is too
slow past ~20k vectors.**

## Method

- **Corpus.** 17,016 assistant-reply chunks (≤1,200 chars) from 280 projects
  of local Claude Code transcripts, plus 835 queries. Each query is a user
  prompt, and its "right answer" is the first chunk of the reply to it. The
  task: given the prompt, find the reply. That is a stand-in for memory
  recall and needs no hand labels. The text never leaves the machine and is
  not in this folder: `results/` holds numbers only.
- **Models.**
  - `nomic-embed-text-v1.5`: ONNX int8, 768 dims, run via fastembed on CPU.
  - `nomic-embed-code`: 7B, GGUF Q4_K_M, 3584 dims, run via Ollama on Metal,
    on a 5k-doc subset with 300 queries.
- **Scale.** Sizes above the real 17k (or 5k for the code model) are filled
  with noisy copies of real vectors (cosine ≈ 0.85 to their source) acting
  as distractors. Correct answers are always real docs.
- **Metrics.**
  - `recall@10`: overlap with exact float search on the full-size vectors.
  - `hit@10`: the right reply is in the top 10.
  - `MRR`: mean reciprocal rank of the right reply.
  - Latency: warm page cache, 200 queries, k = 10.

## Results: nomic-embed-text-v1.5

| config | n | MB | B/vec | p50 ms | p95 ms | recall@10 | hit@10 |
|---|---:|---:|---:|---:|---:|---:|---:|
| f32 768 (exact) | 17k | 54 | 3171 | 36 | 56 | 1.000 | 0.340 |
| f32 768 (exact) | 100k | 311 | 3109 | 138 | 210 | 1.000 | 0.340 |
| f32 768 (exact) | 1M | 3101 | 3101 | 1396 | 1814 | 1.000 | 0.340 |
| int8 768 | 100k | 79 | 794 | 88 | 113 | 0.983 | 0.335 |
| bit 768 alone | 100k | 12 | 120 | 3.2 | 3.6 | 0.446 | 0.250 |
| **bit 768 + f32 rescore (top 100)** | 100k | 423 | 4226 | **11.8** | 12.3 | 0.850 | 0.330 |
| **bit 768 + f32 rescore** | 1M | 4226 | 4226 | **126** | 225 | 0.844 | 0.330 |
| f32 512 Matryoshka | 100k | 208 | 2082 | 89 | 134 | 0.788 | 0.320 |
| f32 256 Matryoshka | 100k | 105 | 1052 | 40 | 59 | 0.626 | 0.305 |
| int8 256 Matryoshka | 100k | 28 | 280 | 22 | 30 | 0.624 | 0.310 |
| **f32 256 + `project` partition key** | 100k | 356 | 3561 | **1.5** | 6.6 | 0.749* | 0.470* |
| **f32 256 + `project` partition key** | 1M | 1255 | 1255 | **14.5** | 75 | 0.730* | 0.465* |

\* Filtered to the query's project, so not comparable with the unfiltered
rows: it measures a scoped memory query, which is how Gespan would ask.

Full data for 17k / 100k / 250k / 1M is in `results/bench.jsonl`.

## Results: nomic-embed-code vs nomic-embed-text-v1.5

Same 5k docs and 300 queries, exact search (`results/compare.jsonl`):

| model | dims | hit@10 | MRR@10 | embed speed |
|---|---:|---:|---:|---|
| nomic-embed-code (Q4_K_M) | 3584 | **0.537** | 0.357 | 2.0 docs/s (Metal GPU) |
| nomic-embed-text-v1.5 | 768 | 0.437 | 0.305 | 6.9 docs/s (CPU, one per call) |
| nomic-embed-text-v1.5 Matryoshka | 256 | 0.413 | 0.293 | same |

In SQLite at 50k vectors, the code model's 3584 dims cost the following
(`results/bench.jsonl`):

| config | B/vec | p50 ms | recall@10 |
|---|---:|---:|---:|
| f32 exact | 14.4 KB | 395 | 1.000 |
| int8 | 3.6 KB | 237 | 0.986 |
| bit + f32 rescore | 16.9 KB | **10.6** | **0.990** |

## Findings

1. **Exact float search scales linearly and gets slow:** 36 ms at 17k,
   138 ms at 100k, 1.4 s at 1M (768 dims). It's fine for one small scope,
   not for searching everything.
2. **Partitioning by scope is the biggest lever.** A `project text partition
   key` column gives 1.5 ms at 100k and 14.5 ms at 1M. Gespan's memory and
   graph queries are naturally scoped (project, pilot, session), so this
   matches how we'd query. Caveat: vec0 allocates storage in chunks per
   partition, so with 280 partitions and only 17k rows the file was 17 KB per
   vector. Set `chunk_size` lower when partitions are small.
3. **Binary vectors + float rescore is the right default for unscoped
   search.** It's 12× faster than exact at 100k and 11× at 1M, with almost
   the same task quality (hit@10 0.330 vs 0.340). It works even better on
   the code model (recall 0.99). The cost is storage: bits plus the full
   float vector. An untested variant, bits + int8 rescore, would cut that to
   about 0.9 KB per vector.
4. **int8 is nearly free quality-wise** (recall ≥ 0.98, 4× smaller), but
   it's only 1.6× faster. Use it for storage, not speed.
5. **Matryoshka 256 saves space but costs quality.** Agreement with the
   768-dim results drops to 0.62, and hit@10 drops 10% relative. Don't use it
   as the main index.
6. **nomic-embed-code finds 23% more right replies in the top 10** than
   text v1.5, even on this conversational corpus. But it's a 4.4 GB, 7B
   model that needs the GPU, and its vectors are 4.7× larger. It isn't a
   default we can ship to every user; it could be an opt-in "high quality"
   mode on capable Macs.
7. **Embedding speed, not the database, is the bottleneck.** 7 docs/s on CPU
   is too slow for a backfill, and 17k docs took 41 minutes. This is runtime
   setup: the dynamically quantized model can't batch. Try the fp16/fp32
   ONNX model with batching, or CoreML/Metal, before choosing a model.
8. Inserts were never a problem (8k–200k rows/s inside one transaction).

## What this means for the design

- Keep SQLite. Add sqlite-vec next to `graph.db`. The extension links
  statically through `rusqlite`, so there's nothing extra to install.
- Store vectors in a `vec0` table keyed by graph node id, with `scope` (or
  `project`) as a partition key. Keep the model id and dims in a sidecar
  table so we can re-embed when the model changes.
- Run scoped queries first, and fall back to global bit + rescore.
- Revisit an approximate-index database (libSQL DiskANN, LanceDB) only if a
  single unscoped search must cover well over 1M vectors in under 50 ms.

## Not covered

- Cold cache (first query after launch), Windows/Linux, mobile.
- Hybrid FTS5 + vector ranking.
- bit + int8 rescore.
- Concurrent writes during search.
- Real vectors beyond 17k: the larger sizes use synthetic distractors.

## Reproduce

```bash
cargo build --release
python3 -I scripts/build_corpus.py ~/.claude/projects "$OUT/corpus" 50000 1000
./target/release/sqlite-vec-bench embed-text "$OUT/corpus" "$OUT/emb" "$OUT/models"
ollama pull hf.co/nomic-ai/nomic-embed-code-GGUF:Q4_K_M
python3 -I scripts/embed_code.py "$OUT/corpus" "$OUT/emb" 5000 300
./target/release/sqlite-vec-bench compare "$OUT/corpus" "$OUT/emb"
./target/release/sqlite-vec-bench bench "$OUT/corpus" "$OUT/emb" "$OUT/db" text 17016,100000,250000 200
./target/release/sqlite-vec-bench bench "$OUT/corpus" "$OUT/emb" "$OUT/db" code 5000,50000 200
```

`$OUT` must be outside the repo: the corpus holds private transcript text.
