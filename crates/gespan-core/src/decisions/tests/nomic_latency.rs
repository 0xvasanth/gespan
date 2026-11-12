//! How long Nomic takes to embed one user message on this machine. Run by
//! hand, never in CI: it downloads the model (about 137 MB) into
//! `GESPAN_NOMIC_TEST_MODELS`, a folder you choose.
//!
//! ```sh
//! GESPAN_NOMIC_TEST_MODELS=/tmp/models cargo test --manifest-path crates/gespan-gpui/Cargo.toml \
//!   --release -p gespan-core --features nomic --lib nomic_latency -- --ignored --nocapture
//! ```
use crate::decisions::embed::{Embedder, Nomic, DIMS, NOMIC_MODEL};
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
        "nomic {} chars: warm-up {:.1} ms, p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        message.len(),
        warmup.as_secs_f64() * 1000.0,
        p(0.50),
        p(0.95),
        times[times.len() - 1]
    );
}
