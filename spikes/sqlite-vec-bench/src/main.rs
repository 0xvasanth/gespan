//! Spike: how well does SQLite + sqlite-vec hold Nomic embeddings for Gespan
//! memory and the knowledge graph?
//!
//! Subcommands:
//!   embed-text CORPUS EMB_DIR MODEL_CACHE
//!       Embed docs.jsonl / queries.jsonl with nomic-embed-text-v1.5 (ONNX,
//!       int8) and write docs.text.f32 / queries.text.f32.
//!   bench CORPUS EMB_DIR DB_DIR MODEL SIZES [QUERIES]
//!       MODEL is `text` or `code`; SIZES like 17016,100000,1000000. Real
//!       vectors fill the first rows; the rest are noisy copies (distractors).
//!       Prints one JSON line per (config, size).
//!   compare CORPUS EMB_DIR
//!       Prompt->reply retrieval quality of both models on the code subset,
//!       exact search, no SQLite.
//!
//! Vector files are `u32 n, u32 dim, n*dim f32` little-endian.

use anyhow::{bail, Context, Result};
use rand::prelude::*;
use rand_distr::Normal;
use rayon::prelude::*;
use rusqlite::{ffi::sqlite3_auto_extension, params, Connection};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const K: usize = 10;
const RESCORE_POOL: usize = 100;

#[derive(Deserialize, Clone)]
struct Doc {
    project: String,
    text: String,
}

#[derive(Deserialize, Clone)]
struct Query {
    project: String,
    text: String,
    target: usize,
}

fn read_jsonl<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<Vec<T>> {
    let f = File::open(path).with_context(|| path.display().to_string())?;
    BufReader::new(f)
        .lines()
        .map(|l| Ok(serde_json::from_str(&l?)?))
        .collect()
}

struct Matrix {
    dim: usize,
    rows: Vec<Vec<f32>>,
}

fn write_matrix(path: &Path, rows: &[Vec<f32>]) -> Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(&(rows.len() as u32).to_le_bytes())?;
    w.write_all(&(rows[0].len() as u32).to_le_bytes())?;
    for r in rows {
        for x in r {
            w.write_all(&x.to_le_bytes())?;
        }
    }
    Ok(())
}

fn read_matrix(path: &Path) -> Result<Matrix> {
    let mut buf = Vec::new();
    File::open(path)
        .with_context(|| path.display().to_string())?
        .read_to_end(&mut buf)?;
    let n = u32::from_le_bytes(buf[0..4].try_into()?) as usize;
    let dim = u32::from_le_bytes(buf[4..8].try_into()?) as usize;
    let floats: Vec<f32> = buf[8..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    if floats.len() != n * dim {
        bail!("{}: expected {n}x{dim}", path.display());
    }
    Ok(Matrix {
        dim,
        rows: floats.chunks(dim).map(<[f32]>::to_vec).collect(),
    })
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Nomic v1.5 Matryoshka recipe: layer norm over the full vector, truncate,
/// renormalize. Layer norm is scale-invariant, so it is fine to apply it to
/// the already-normalized model output.
fn matryoshka(v: &[f32], dim: usize) -> Vec<f32> {
    let n = v.len() as f32;
    let mean = v.iter().sum::<f32>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    let sd = (var + 1e-5).sqrt();
    let mut out: Vec<f32> = v[..dim].iter().map(|x| (x - mean) / sd).collect();
    normalize(&mut out);
    out
}

fn f32_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn int8_blob(v: &[f32], scale: f32) -> Vec<u8> {
    v.iter()
        .map(|x| ((x * scale).round().clamp(-127.0, 127.0) as i8) as u8)
        .collect()
}

fn bit_blob(v: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; v.len().div_ceil(8)];
    for (i, x) in v.iter().enumerate() {
        if *x > 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

// ---------------------------------------------------------------- embed-text

fn embed_text(corpus: &Path, emb: &Path, cache: &Path) -> Result<()> {
    use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
    let docs: Vec<Doc> = read_jsonl(&corpus.join("docs.jsonl"))?;
    let queries: Vec<Query> = read_jsonl(&corpus.join("queries.jsonl"))?;
    let mut model = TextEmbedding::try_new(
        TextInitOptions::new(EmbeddingModel::NomicEmbedTextV15Q)
            .with_cache_dir(cache.to_path_buf())
            .with_show_download_progress(true),
    )?;
    std::fs::create_dir_all(emb)?;
    for (name, texts, prefix) in [
        (
            "queries.text.f32",
            queries.iter().map(|q| q.text.clone()).collect::<Vec<_>>(),
            "search_query: ",
        ),
        (
            "docs.text.f32",
            docs.iter().map(|d| d.text.clone()).collect(),
            "search_document: ",
        ),
    ] {
        let started = Instant::now();
        let mut rows = Vec::with_capacity(texts.len());
        for (i, batch) in texts.chunks(512).enumerate() {
            // The int8 model quantizes activations per call, so fastembed
            // refuses batches: embed one text per call, as an app would.
            for t in batch {
                let mut out = model.embed(vec![format!("{prefix}{t}")], None)?;
                let mut v = out.pop().context("no embedding")?;
                normalize(&mut v);
                rows.push(v);
            }
            eprintln!(
                "{name}: {}/{} ({:.0}/s)",
                i * 512 + batch.len(),
                texts.len(),
                rows.len() as f64 / started.elapsed().as_secs_f64()
            );
        }
        write_matrix(&emb.join(name), &rows)?;
        println!(
            "{{\"embedded\":\"{name}\",\"n\":{},\"secs\":{:.1},\"per_sec\":{:.1}}}",
            rows.len(),
            started.elapsed().as_secs_f64(),
            rows.len() as f64 / started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- bench

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    F32,
    Int8,
    Bit,
}

#[derive(Clone, Debug)]
struct Config {
    name: String,
    kind: Kind,
    dim: usize,
    /// Matryoshka-truncate to `dim` (text model only).
    mrl: bool,
    /// Rescore the bit candidates with the full float vectors.
    rescore: bool,
    /// vec0 partition key on project; queries filter to the query's project.
    partition: bool,
}

fn configs(model: &str, full: usize) -> Vec<Config> {
    let c = |name: &str, kind, dim, mrl, rescore, partition| Config {
        name: name.into(),
        kind,
        dim,
        mrl,
        rescore,
        partition,
    };
    let mut v = vec![
        c(&format!("f32_{full}"), Kind::F32, full, false, false, false),
        c(&format!("int8_{full}"), Kind::Int8, full, false, false, false),
        c(&format!("bit_{full}"), Kind::Bit, full, false, false, false),
        c(&format!("bit_{full}+rescore"), Kind::Bit, full, false, true, false),
    ];
    if model == "text" {
        v.extend([
            c("f32_512_mrl", Kind::F32, 512, true, false, false),
            c("f32_256_mrl", Kind::F32, 256, true, false, false),
            c("int8_256_mrl", Kind::Int8, 256, true, false, false),
            c("f32_256_mrl+project", Kind::F32, 256, true, false, true),
        ]);
    }
    v
}

fn represent(cfg: &Config, v: &[f32]) -> Vec<f32> {
    if cfg.mrl {
        matryoshka(v, cfg.dim)
    } else {
        v.to_vec()
    }
}

/// Grow the real set to `size` rows with noisy copies (cosine ~0.85 to their
/// source), so larger sizes keep a realistic distribution of directions.
fn augment(real: &[Vec<f32>], real_projects: &[String], size: usize) -> (Vec<Vec<f32>>, Vec<String>) {
    let dim = real[0].len();
    let sigma = (0.384f32 / dim as f32).sqrt();
    let extra: Vec<(Vec<f32>, String)> = (real.len()..size)
        .into_par_iter()
        .map(|i| {
            let mut rng = StdRng::seed_from_u64(i as u64);
            let noise = Normal::new(0.0, sigma).unwrap();
            let src = i % real.len();
            let mut v: Vec<f32> = real[src].iter().map(|x| x + noise.sample(&mut rng)).collect();
            normalize(&mut v);
            let p = real_projects[rng.gen_range(0..real_projects.len())].clone();
            (v, p)
        })
        .collect();
    let mut rows = real.to_vec();
    let mut projects = real_projects.to_vec();
    for (v, p) in extra {
        rows.push(v);
        projects.push(p);
    }
    (rows, projects)
}

/// Exact top-k by cosine over full vectors, optionally within one project.
fn exact_topk(rows: &[Vec<f32>], projects: &[String], q: &[f32], project: Option<&str>) -> Vec<usize> {
    let mut scored: Vec<(f32, usize)> = rows
        .par_iter()
        .enumerate()
        .filter(|(i, _)| project.is_none_or(|p| projects[*i] == p))
        .map(|(i, r)| (dot(r, q), i))
        .collect();
    let k = K.min(scored.len());
    if k == 0 {
        return vec![];
    }
    scored.select_nth_unstable_by(k - 1, |a, b| b.0.total_cmp(&a.0));
    scored.truncate(k);
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, i)| i).collect()
}

fn open_db(path: &Path) -> Result<Connection> {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let conn = Connection::open(path)?;
    // Same settings Gespan's stores use.
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

fn col_type(cfg: &Config) -> String {
    match cfg.kind {
        Kind::F32 => format!("float[{}]", cfg.dim),
        Kind::Int8 => format!("int8[{}]", cfg.dim),
        Kind::Bit => format!("bit[{}]", cfg.dim),
    }
}

fn wrap(cfg: &Config, param: &str) -> String {
    match cfg.kind {
        Kind::F32 => param.into(),
        Kind::Int8 => format!("vec_int8({param})"),
        Kind::Bit => format!("vec_bit({param})"),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

#[allow(clippy::too_many_arguments)]
fn run_config(
    cfg: &Config,
    db_dir: &Path,
    model: &str,
    rows: &[Vec<f32>],
    projects: &[String],
    queries: &[Query],
    qvecs: &[Vec<f32>],
    truth: &[Vec<usize>],
) -> Result<serde_json::Value> {
    let path = db_dir.join(format!("{model}-{}-{}.db", cfg.name.replace('+', "_"), rows.len()));
    let conn = open_db(&path)?;
    let partition = if cfg.partition { "project text partition key, " } else { "" };
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE vec USING vec0({partition}embedding {});",
        col_type(cfg)
    ))?;
    if cfg.rescore {
        conn.execute_batch("CREATE TABLE full_vec (id INTEGER PRIMARY KEY, v BLOB NOT NULL);")?;
    }

    // int8 scale: map the largest component magnitude to 127.
    let reps: Vec<Vec<f32>> = rows.par_iter().map(|v| represent(cfg, v)).collect();
    let scale = 127.0
        / reps
            .par_iter()
            .map(|v| v.iter().fold(0f32, |m, x| m.max(x.abs())))
            .reduce(|| 0.0, f32::max);
    let blob = |v: &[f32]| match cfg.kind {
        Kind::F32 => f32_blob(v),
        Kind::Int8 => int8_blob(v, scale),
        Kind::Bit => bit_blob(v),
    };

    let started = Instant::now();
    {
        let tx = conn.unchecked_transaction()?;
        let sql = if cfg.partition {
            format!("INSERT INTO vec(rowid, project, embedding) VALUES (?1, ?2, {})", wrap(cfg, "?3"))
        } else {
            format!("INSERT INTO vec(rowid, embedding) VALUES (?1, {})", wrap(cfg, "?2"))
        };
        let mut ins = tx.prepare(&sql)?;
        let mut ins_full = tx.prepare("INSERT INTO full_vec(id, v) VALUES (?1, ?2)").ok();
        for (i, v) in reps.iter().enumerate() {
            if cfg.partition {
                ins.execute(params![i as i64, projects[i], blob(v)])?;
            } else {
                ins.execute(params![i as i64, blob(v)])?;
            }
            if cfg.rescore {
                ins_full.as_mut().unwrap().execute(params![i as i64, f32_blob(&rows[i])])?;
            }
        }
        drop(ins);
        drop(ins_full);
        tx.commit()?;
    }
    let insert_secs = started.elapsed().as_secs_f64();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let bytes = std::fs::metadata(&path)?.len();

    let sql = if cfg.rescore {
        format!(
            "WITH c AS (SELECT rowid FROM vec WHERE embedding MATCH {} AND k = {RESCORE_POOL})
             SELECT c.rowid FROM c JOIN full_vec f ON f.id = c.rowid
             ORDER BY vec_distance_l2(f.v, ?2) LIMIT {K}",
            wrap(cfg, "?1")
        )
    } else if cfg.partition {
        format!(
            "SELECT rowid FROM vec WHERE embedding MATCH {} AND k = {K} AND project = ?2",
            wrap(cfg, "?1")
        )
    } else {
        format!("SELECT rowid FROM vec WHERE embedding MATCH {} AND k = {K}", wrap(cfg, "?1"))
    };
    let mut stmt = conn.prepare(&sql)?;
    let mut search = |qi: usize| -> Result<Vec<usize>> {
        let qrep = blob(&represent(cfg, &qvecs[qi]));
        let ids = if cfg.rescore {
            stmt.query_map(params![qrep, f32_blob(&qvecs[qi])], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else if cfg.partition {
            stmt.query_map(params![qrep, queries[qi].project], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![qrep], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(ids.into_iter().map(|i| i as usize).collect())
    };

    // Warm pass (page cache), then the timed pass.
    for qi in 0..qvecs.len() {
        search(qi)?;
    }
    let mut lat = Vec::new();
    let (mut recall, mut hits, mut rr) = (0f64, 0usize, 0f64);
    for qi in 0..qvecs.len() {
        let t = Instant::now();
        let got = search(qi)?;
        lat.push(t.elapsed().as_secs_f64() * 1000.0);
        let want = &truth[qi];
        if !want.is_empty() {
            recall += got.iter().filter(|g| want.contains(g)).count() as f64 / want.len() as f64;
        }
        if let Some(pos) = got.iter().position(|g| *g == queries[qi].target) {
            hits += 1;
            rr += 1.0 / (pos + 1) as f64;
        }
    }
    lat.sort_by(f64::total_cmp);
    let nq = qvecs.len() as f64;
    drop(stmt);
    drop(conn);
    for p in [path.clone(), path.with_extension("db-wal"), path.with_extension("db-shm")] {
        let _ = std::fs::remove_file(p);
    }
    Ok(serde_json::json!({
        "model": model,
        "config": cfg.name,
        "n": rows.len(),
        "db_mb": bytes as f64 / 1e6,
        "bytes_per_vec": bytes as f64 / rows.len() as f64,
        "insert_per_sec": rows.len() as f64 / insert_secs,
        "p50_ms": percentile(&lat, 0.5),
        "p95_ms": percentile(&lat, 0.95),
        "recall_at_10": recall / nq,
        "hit_at_10": hits as f64 / nq,
        "mrr_at_10": rr / nq,
        "queries": qvecs.len(),
    }))
}

fn bench(corpus: &Path, emb: &Path, db_dir: &Path, model: &str, sizes: &[usize], max_q: usize, only: Option<&str>) -> Result<()> {
    std::fs::create_dir_all(db_dir)?;
    let all_docs: Vec<Doc> = read_jsonl(&corpus.join("docs.jsonl"))?;
    let all_queries: Vec<Query> = read_jsonl(&corpus.join("queries.jsonl"))?;
    let dm = read_matrix(&emb.join(format!("docs.{model}.f32")))?;
    let qm = read_matrix(&emb.join(format!("queries.{model}.f32")))?;

    // The code model only embeds a subset: docs.code.ids / queries.code.ids
    // map its rows back to corpus ids.
    let (docs, queries): (Vec<Doc>, Vec<Query>) = if model == "code" {
        let ids = |name: &str| -> Result<Vec<usize>> {
            Ok(std::fs::read_to_string(emb.join(name))?
                .lines()
                .map(|l| l.parse().unwrap())
                .collect())
        };
        let dids = ids("docs.code.ids")?;
        let remap: HashMap<usize, usize> = dids.iter().enumerate().map(|(i, d)| (*d, i)).collect();
        let docs = dids.iter().map(|d| all_docs[*d].clone()).collect();
        let queries = ids("queries.code.ids")?
            .iter()
            .map(|q| {
                let mut q = all_queries[*q].clone();
                q.target = remap[&q.target];
                q
            })
            .collect();
        (docs, queries)
    } else {
        (all_docs, all_queries)
    };
    assert_eq!(docs.len(), dm.rows.len());
    assert_eq!(queries.len(), qm.rows.len());
    let queries: Vec<Query> = queries.into_iter().take(max_q).collect();
    let qvecs: Vec<Vec<f32>> = qm.rows.into_iter().take(max_q).collect();
    let real_projects: Vec<String> = docs.iter().map(|d| d.project.clone()).collect();

    for &size in sizes {
        let (rows, projects) = augment(&dm.rows, &real_projects, size.max(dm.rows.len()));
        let truth_all: Vec<Vec<usize>> = qvecs.iter().map(|q| exact_topk(&rows, &projects, q, None)).collect();
        let truth_proj: Vec<Vec<usize>> = qvecs
            .iter()
            .zip(&queries)
            .map(|(q, qq)| exact_topk(&rows, &projects, q, Some(&qq.project)))
            .collect();
        for cfg in configs(model, dm.dim) {
            if only.is_some_and(|o| !o.split(',').any(|n| n == cfg.name)) {
                continue;
            }
            let truth = if cfg.partition { &truth_proj } else { &truth_all };
            let out = run_config(&cfg, db_dir, model, &rows, &projects, &queries, &qvecs, truth)?;
            println!("{out}");
            std::io::stdout().flush()?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- compare

fn compare(corpus: &Path, emb: &Path) -> Result<()> {
    let all_queries: Vec<Query> = read_jsonl(&corpus.join("queries.jsonl"))?;
    let ids = |name: &str| -> Result<Vec<usize>> {
        Ok(std::fs::read_to_string(emb.join(name))?
            .lines()
            .map(|l| l.parse().unwrap())
            .collect())
    };
    let dids = ids("docs.code.ids")?;
    let qids = ids("queries.code.ids")?;
    let text_d = read_matrix(&emb.join("docs.text.f32"))?;
    let text_q = read_matrix(&emb.join("queries.text.f32"))?;
    let code_d = read_matrix(&emb.join("docs.code.f32"))?;
    let code_q = read_matrix(&emb.join("queries.code.f32"))?;

    let score = |name: &str, docs: Vec<Vec<f32>>, qs: Vec<Vec<f32>>| {
        let none: Vec<String> = vec![String::new(); docs.len()];
        let (mut hits, mut rr) = (0usize, 0f64);
        for (qi, q) in qs.iter().enumerate() {
            let target = all_queries[qids[qi]].target;
            let got = exact_topk(&docs, &none, q, None);
            if let Some(pos) = got.iter().position(|g| dids[*g] == target) {
                hits += 1;
                rr += 1.0 / (pos + 1) as f64;
            }
        }
        println!(
            "{}",
            serde_json::json!({"compare": name, "docs": docs.len(), "queries": qs.len(),
                "hit_at_10": hits as f64 / qs.len() as f64, "mrr_at_10": rr / qs.len() as f64})
        );
    };
    let pick = |m: &Matrix, ix: &[usize]| ix.iter().map(|i| m.rows[*i].clone()).collect::<Vec<_>>();
    score("text_768", pick(&text_d, &dids), pick(&text_q, &qids));
    score(
        "text_256_mrl",
        pick(&text_d, &dids).iter().map(|v| matryoshka(v, 256)).collect(),
        pick(&text_q, &qids).iter().map(|v| matryoshka(v, 256)).collect(),
    );
    score("code_3584", code_d.rows, code_q.rows);
    Ok(())
}

fn main() -> Result<()> {
    unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));
    }
    let a: Vec<String> = std::env::args().collect();
    let p = |i: usize| PathBuf::from(&a[i]);
    match a.get(1).map(String::as_str) {
        Some("embed-text") => embed_text(&p(2), &p(3), &p(4)),
        Some("bench") => {
            let sizes: Vec<usize> = a[6].split(',').map(|s| s.parse().unwrap()).collect();
            let max_q = a.get(7).map_or(200, |s| s.parse().unwrap());
            bench(&p(2), &p(3), &p(4), &a[5], &sizes, max_q, a.get(8).map(String::as_str))
        }
        Some("compare") => compare(&p(2), &p(3)),
        _ => bail!("usage: see the header of src/main.rs"),
    }
}
