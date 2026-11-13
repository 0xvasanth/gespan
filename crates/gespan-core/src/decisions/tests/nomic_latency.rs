//! How long Nomic takes to embed one user message on this machine. Run by
//! hand, never in CI: it downloads the model (about 137 MB) into
//! `GESPAN_NOMIC_TEST_MODELS`, a folder you choose.
//!
//! ```sh
//! GESPAN_NOMIC_TEST_MODELS=/tmp/models cargo test --manifest-path crates/gespan-gpui/Cargo.toml \
//!   --release -p gespan-core --features nomic --lib nomic_latency -- --ignored --nocapture
//! ```
use crate::decisions::embed::{runtime_version, Embedder, Nomic, DIMS, NOMIC_MODEL};
use std::time::Instant;

#[test]
#[ignore = "downloads the Nomic model and measures this machine"]
fn nomic_embeds_a_300_char_message_fast_enough() {
    let root = std::env::var_os("GESPAN_NOMIC_TEST_MODELS")
        .expect("set GESPAN_NOMIC_TEST_MODELS to a folder for the model");
    let models = gespan_models::ModelStore::new(root.into());
    models
        .download(&NOMIC_MODEL, &gespan_models::CancelToken::new(), |_| {})
        .unwrap();
    let nomic = Nomic::load(&models.dir(&NOMIC_MODEL)).unwrap();
    let message = "Can you make the retry logic in the sync worker back off \
        exponentially and give up after five attempts? Also log each failure with \
        the request id so we can trace it, and keep the existing timeout of thirty \
        seconds. Don't change the public API of the client, other crates use it. Add a test for the give-up path too.";
    let message = &message[..300];
    // The first call pays for session warm-up; a turn after launch would too,
    // so it is reported but not counted.
    let first = Instant::now();
    assert_eq!(nomic.embed_query(message).unwrap().len(), DIMS);
    let warmup = first.elapsed();
    let mut times: Vec<f64> = (0..50)
        .map(|_| {
            let t = Instant::now();
            nomic.embed_query(message).unwrap();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    let p = |q: f64| times[((times.len() as f64 * q).ceil() as usize).saturating_sub(1)];
    println!(
        "nomic on onnxruntime {} ({} chars): warm-up {:.1} ms, p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        runtime_version(),
        message.len(),
        warmup.as_secs_f64() * 1000.0,
        p(0.50),
        p(0.95),
        times[times.len() - 1]
    );
}

/// Calibrates `Nomic::max_distance`: cosine distances from a search query
/// to decision-style entries it is about, and to ones it is not about.
#[test]
#[ignore = "downloads the Nomic model; prints distances for calibration"]
fn nomic_distances_for_related_and_unrelated_decisions() {
    let root = std::env::var_os("GESPAN_NOMIC_TEST_MODELS")
        .expect("set GESPAN_NOMIC_TEST_MODELS to a folder for the model");
    let models = gespan_models::ModelStore::new(root.into());
    models
        .download(&NOMIC_MODEL, &gespan_models::CancelToken::new(), |_| {})
        .unwrap();
    let nomic = Nomic::load(&models.dir(&NOMIC_MODEL)).unwrap();
    let doc = |t: &str| nomic.embed_document(t).unwrap();
    let query = |t: &str| nomic.embed_query(t).unwrap();
    let dist = |a: &[f32], b: &[f32]| 1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    // (query, an entry it is about)
    let related = [
        ("how should we retry failed requests", "Retries\nRetry network calls three times with exponential backoff\nFlaky APIs"),
        ("rebase or merge?", "Git history\nRebase feature branches onto main instead of merging\nLinear history is easier to bisect"),
        ("tabs vs spaces", "Indentation\nIndent with four spaces, never tabs\nMatches rustfmt"),
        ("which database for local data", "Storage\nKeep local data in SQLite through Toasty\nOne file, easy backups"),
        ("error handling style", "Errors\nUse anyhow in binaries and thiserror in libraries\nTyped errors for callers"),
        ("where do tests go", "Tests\nPut unit tests in a tests module next to the code\nKeeps them close to what they cover"),
        ("logging format", "Logs\nWrite structured JSONL logs with tracing\nMachine readable"),
        ("naming branches", "Branches\nName branches after the ticket id\nEasy to trace"),
        ("dark mode colors", "Theme\nDefine colours as tokens and redefine them for dark mode\nOne place to change"),
        ("commit message style", "Commits\nWrite imperative commit subjects under 72 characters\nReadable history"),
        ("can I use unwrap", "Panics\nNever unwrap in production code; return errors\nCrashes lose user work"),
        ("how to name tests", "Test names\nName tests after the behaviour they check\nFailures read as sentences"),
    ];
    let unrelated_docs: Vec<&str> = related.iter().map(|(_, d)| *d).collect();
    let mut near = Vec::new();
    let mut far = Vec::new();
    for (i, (q, _)) in related.iter().enumerate() {
        let qv = query(q);
        for (j, d) in unrelated_docs.iter().enumerate() {
            let d = dist(&qv, &doc(d));
            if i == j {
                near.push(d)
            } else {
                far.push(d)
            }
        }
    }
    near.sort_by(f32::total_cmp);
    far.sort_by(f32::total_cmp);
    println!(
        "related   ({}): min {:.3} median {:.3} max {:.3}",
        near.len(),
        near[0],
        near[near.len() / 2],
        near[near.len() - 1]
    );
    println!(
        "unrelated ({}): min {:.3} p5 {:.3} median {:.3}",
        far.len(),
        far[0],
        far[far.len() / 20],
        far[far.len() / 2]
    );
    println!("related: {near:.3?}");
    println!("unrelated lowest 15: {:.3?}", &far[..15]);
}
