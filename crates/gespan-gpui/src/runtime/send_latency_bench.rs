//! First-response latency through the real send path, against the real
//! provider CLIs. Manual: it needs a signed-in `claude` or `kiro-cli` and
//! spends a few tiny prompts. From the repository root:
//!
//! ```sh
//! BENCH_KIND=claude BENCH_N=5 BENCH_CWD=/path/to/a/git/project \
//!   cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui \
//!   --profile native-dev -- --ignored send_latency_bench --nocapture
//! ```
//!
//! Each of the `BENCH_N` sessions sends three turns: a cold start, a second
//! turn, and a third that reuses the kept process. Times are milliseconds
//! from `run_turn`, read from the `gespan::send` debug events. Delivery ends
//! when the view's update channel receives the first text, before painting.
use super::*;
use std::sync::OnceLock;
use std::time::Instant;
use tracing_subscriber::layer::SubscriberExt;

static MARKS: OnceLock<Mutex<Vec<(Instant, String)>>> = OnceLock::new();

struct Marks;
struct Stage(String);
impl tracing::field::Visit for Stage {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_str(field, &format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "stage" {
            self.0.insert_str(0, value);
        } else {
            self.0.push_str(&format!(" {}={value}", field.name()));
        }
    }
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Marks {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != "gespan::send" {
            return;
        }
        let mut stage = Stage(String::new());
        event.record(&mut stage);
        if let Some(marks) = MARKS.get() {
            marks.lock().unwrap().push((Instant::now(), stage.0));
        }
    }
}

/// One turn's marks, in milliseconds from its start.
fn one_turn(store: &Database, session: i64) -> Vec<(f64, String)> {
    MARKS.get().unwrap().lock().unwrap().clear();
    let summary = locked(store)
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|s| s.id == session)
        .unwrap();
    let settings = locked(store)
        .unwrap()
        .agent_settings(session)
        .unwrap()
        .unwrap_or_default();
    let (tx, rx) = async_channel::bounded(8);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker_store = store.clone();
    let start = Instant::now();
    let worker = std::thread::spawn(move || {
        run_turn(
            summary,
            "Reply with exactly the word: ok".into(),
            settings,
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            tx,
        );
    });
    let mut delivered = None;
    loop {
        let update = rx.recv_blocking().expect("runtime closed early");
        if delivered.is_none()
            && update
                .rows
                .iter()
                .any(|r| r.role == "assistant" && !r.text.is_empty())
        {
            delivered = Some(Instant::now());
        }
        if update.finished {
            assert!(update.succeeded, "turn failed: {}", update.status);
            break;
        }
    }
    worker.join().unwrap();
    let ms = |at: Instant| at.duration_since(start).as_secs_f64() * 1000.0;
    let mut marks: Vec<_> = MARKS
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .iter()
        .map(|(at, stage)| (ms(*at), stage.clone()))
        .collect();
    if let Some(at) = delivered {
        marks.push((ms(at), "view.first_text".into()));
    }
    marks
}

fn at(marks: &[(f64, String)], stages: &[&str]) -> Option<f64> {
    marks
        .iter()
        .find(|(_, s)| {
            stages
                .iter()
                .any(|stage| s == stage || s.starts_with(&format!("{stage} ")))
        })
        .map(|(t, _)| *t)
}

type Timeline = [(f64, String)];
type Metric = fn(&Timeline) -> Option<f64>;

const PROMPT: [&str; 2] = ["claude.prompt_written", "acp.send method=session/prompt"];

fn report(label: &str, runs: &[Vec<(f64, String)>]) {
    let metrics: [(&str, Metric); 6] = [
        ("CLI process started", |m| at(m, &["provider.spawned"])),
        ("baseline recorded", |m| at(m, &["checkpoint.recorded"])),
        ("prompt written to the CLI", |m| at(m, &PROMPT)),
        ("first text parsed", |m| at(m, &["provider.first_text"])),
        ("first text delivered", |m| at(m, &["view.first_text"])),
        ("parse to delivery", |m| {
            Some(at(m, &["view.first_text"])? - at(m, &["provider.first_text"])?)
        }),
    ];
    eprintln!("== {label} (n={})", runs.len());
    for (name, metric) in metrics {
        let mut v: Vec<f64> = runs.iter().filter_map(|m| metric(m)).collect();
        if v.is_empty() {
            continue;
        }
        v.sort_by(f64::total_cmp);
        let median = v[v.len() / 2];
        let p90 = v[(v.len() * 9).div_ceil(10) - 1];
        eprintln!("  {name:26} median {median:8.1}  p90 {p90:8.1}");
    }
}

#[test]
#[ignore = "manual benchmark against real provider CLIs"]
fn send_latency_bench() {
    MARKS.get_or_init(Default::default);
    let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Marks));
    let kind = std::env::var("BENCH_KIND").unwrap_or("claude".into());
    let n: usize = std::env::var("BENCH_N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(5);
    let cwd = std::env::var("BENCH_CWD").expect("BENCH_CWD names a git project");
    let data = tempfile::tempdir().unwrap();
    gespan_core::tool_servers::configure(data.path());
    gespan_core::checkpoint::init(data.path().join("checkpoints"));
    gespan_core::provider::configure_idle_agents(8);
    let store: Database = Arc::new(Mutex::new(
        Store::open_gespan(&data.path().join("gespan.db")).unwrap(),
    ));
    let verbose = std::env::var_os("BENCH_VERBOSE").is_some();
    let mut turns: [Vec<Vec<(f64, String)>>; 3] = Default::default();
    for i in 0..n {
        let session = locked(&store).unwrap().create(&kind, &cwd).unwrap().id;
        gespan_core::provider::keep_idle_agents(vec![session]);
        for (turn, label) in ["cold", "second", "warm"].iter().enumerate() {
            let marks = gespan_core::process::owned_by(session, || one_turn(&store, session));
            if verbose {
                for (t, stage) in &marks {
                    eprintln!("{label}{i} {t:9.1} {stage}");
                }
            }
            turns[turn].push(marks);
        }
    }
    for (label, runs) in ["cold", "second", "warm"].iter().zip(&turns) {
        report(&format!("{kind} {label}"), runs);
    }
}
