//! Native diagnostics: structured JSONL logs in the platform log folder,
//! crash files from the panic hook, and the local "Open logs folder" and
//! "Copy diagnostics" actions. Nothing here touches the network.
mod actions;
mod bundle;
mod collect;
mod fallback;
mod json;
mod panic;
mod paths;
mod recent;
mod rotation;

pub use actions::{open_logs_folder, summary, summary_for};
pub use bundle::{
    build, default_consent, Bundle, Destination, Entry, Report, Selection, Sources, Stamp, System,
    MAX_BUNDLE_BYTES,
};
pub use collect::{
    build_channel, collect, environment_json, installed_providers, locale_from, log_tail,
    redactor_for, running_log_dir, system, Environment, TAIL_LINES,
};
pub use fallback::{
    downloads_dir, issue_url, remove_crashes, save_bundle, CUT_NOTE, ISSUE_FORM, MAX_URL_BYTES,
};
pub use json::{filter, json_layer, JsonLayer};
pub use panic::{install_panic_hook, panic_message, write_crash};
pub use paths::{crash_dir, log_dir, Channel};
pub use recent::{note_error_event, recent_error_events};
pub use rotation::{file_name, prune, Clock, DailyFile, SystemClock, MAX_BYTES, MAX_DAYS};

use std::{
    any::Any,
    panic::UnwindSafe,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_log::AsLog;
use tracing_subscriber::layer::SubscriberExt;

pub struct Config {
    pub log_dir: PathBuf,
    pub version: String,
    pub channel: Channel,
}

/// Flushes the log writer when dropped. `main` keeps it for the life of the
/// process; `std::process::exit` skips destructors, so exit paths call
/// [`flush`] themselves.
#[must_use = "dropping the guard stops file logging"]
pub struct Guard(());

impl Drop for Guard {
    fn drop(&mut self) {
        flush();
    }
}

/// The running configuration, for the "Copy diagnostics" summary.
static STARTED: OnceLock<Config> = OnceLock::new();
/// The non-blocking writer's guard, kept for the life of the process.
static WRITER: Mutex<Option<WorkerGuard>> = Mutex::new(None);

/// Human-readable lines, with colour only on a terminal.
fn readable_layer<S, W>(writer: W) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + 'static,
{
    tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
}

/// Starts file logging (filtered by `GESPAN_LOG`), bridges the `log` crate,
/// and installs the panic hook. Prunes old logs first. Call it once.
pub fn init(config: Config) -> Result<Guard, String> {
    if STARTED.get().is_some() {
        return Err("Diagnostics are already running.".into());
    }
    let file = DailyFile::new(config.log_dir.clone(), SystemClock, MAX_DAYS, MAX_BYTES)
        .map_err(|error| format!("Cannot write logs to {}: {error}", config.log_dir.display()))?;
    let (writer, worker) = tracing_appender::non_blocking(file);
    let (env_filter, filter_error) = filter(std::env::var("GESPAN_LOG").ok().as_deref());
    // Development runs also print readable lines, as `eprintln!` did.
    let stderr = (config.channel == Channel::Dev).then(|| readable_layer(std::io::stderr));
    // `log` stops below this level itself, so filtered-out `log::debug!`
    // calls in dependencies never reach the subscriber.
    let log_level = env_filter
        .max_level_hint()
        .map_or(log::LevelFilter::Trace, |level| level.as_log());
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(json_layer(writer, &config.version))
        .with(stderr);
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| format!("Cannot start logging: {error}"))?;
    let bridged = tracing_log::LogTracer::builder()
        .with_max_level(log_level)
        .init();
    *WRITER.lock().unwrap_or_else(|e| e.into_inner()) = Some(worker);
    install_panic_hook(crash_dir(&config.log_dir), config.version.clone());
    if let Some(error) = filter_error {
        tracing::warn!(%error, "GESPAN_LOG is not a valid filter; using info");
    }
    if let Err(error) = bridged {
        tracing::warn!(%error, "log crate messages are not captured");
    }
    tracing::info!(channel = ?config.channel, "diagnostics started");
    let _ = STARTED.set(config);
    Ok(Guard(()))
}

/// Writes out buffered log lines and stops file logging. Safe to call more
/// than once; later events are dropped instead of blocking.
pub fn flush() {
    let worker = WRITER.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(worker);
}

/// `std::panic::catch_unwind` for code that is expected to panic on bad input.
/// The panic is logged at `warn` and returned as its message; it is not a
/// crash, so the panic hook writes no crash file for it.
pub fn catch_unwind<T>(what: &str, f: impl FnOnce() -> T + UnwindSafe) -> Result<T, String> {
    panic::expecting_panics(|| std::panic::catch_unwind(f)).map_err(|payload| {
        log_caught_panic(what, &*payload);
        panic_message(&*payload)
    })
}

/// What was caught is logged at `warn`, not dropped.
fn log_caught_panic(what: &str, payload: &(dyn Any + Send)) {
    tracing::warn!(what, panic = %panic_message(payload), "caught a panic");
}

/// Captures this thread's log lines while `body` runs, for other modules' tests.
#[cfg(test)]
pub(crate) fn test_capture(body: impl FnOnce()) -> Vec<serde_json::Value> {
    json::tests::capture(None, body).lines()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_lines_show_a_log_record_s_own_target() {
        use tracing_subscriber::layer::SubscriberExt;
        let out = json::tests::Capture::default();
        let writer = out.clone();
        let subscriber =
            tracing_subscriber::registry().with(readable_layer(move || writer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            let record = log::Record::builder()
                .level(log::Level::Warn)
                .target("gpui::window")
                .args(format_args!("from log"))
                .build();
            tracing_log::format_trace(&record).unwrap();
        });
        let text = out.raw();
        assert!(text.contains("gpui::window"), "{text}");
        assert!(!text.contains("log.target"), "{text}");
    }

    #[test]
    fn caught_panic_logs_at_warn() {
        let payload = std::panic::catch_unwind(|| panic!("bad footer")).unwrap_err();
        let out = json::tests::capture(None, || {
            log_caught_panic("webview2 localStorage", &*payload)
        });
        let lines = out.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["level"], "warn");
        assert_eq!(lines[0]["fields"]["what"], "webview2 localStorage");
        assert_eq!(lines[0]["fields"]["panic"], "bad footer");
    }
}
