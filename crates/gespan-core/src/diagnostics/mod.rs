//! Native diagnostics: structured JSONL logs in the platform log folder,
//! crash files from the panic hook, and the local "Open logs folder" and
//! "Copy diagnostics" actions. Nothing here touches the network.
mod actions;
mod json;
mod panic;
mod paths;
mod rotation;

pub use actions::{open_logs_folder, summary, summary_for};
pub use json::{filter, json_layer, JsonLayer};
pub use panic::{install_panic_hook, panic_message, write_crash};
pub use paths::{crash_dir, log_dir, Channel};
pub use rotation::{file_name, prune, Clock, DailyFile, SystemClock, MAX_BYTES, MAX_DAYS};

use std::{
    any::Any,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use tracing_appender::non_blocking::WorkerGuard;
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
    let stderr = (config.channel == Channel::Dev).then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
    });
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(json_layer(writer, &config.version))
        .with(stderr);
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| format!("Cannot start logging: {error}"))?;
    let bridged = tracing_log::LogTracer::init();
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

/// For `catch_unwind` sites: what was caught is logged at `warn`, not dropped.
pub fn log_caught_panic(what: &str, payload: &(dyn Any + Send)) {
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
