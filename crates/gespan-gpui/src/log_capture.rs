//! Captures the log lines a test body writes, in the shape the log file gets.
use std::{
    io::Write,
    sync::{Arc, Mutex},
};
use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt};

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);
impl Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> MakeWriter<'a> for Lines {
    type Writer = Lines;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Runs `body` on this thread with the JSONL layer installed, at every level,
/// and returns one value per line written.
pub(crate) fn capture(body: impl FnOnce()) -> Vec<serde_json::Value> {
    let out = Lines::default();
    let subscriber = tracing_subscriber::registry()
        .with(gespan_core::diagnostics::json_layer(out.clone(), "test"));
    // With a single registered dispatcher, tracing-core computes a new
    // callsite's interest from the registering thread's default only, so a
    // parallel test with no subscriber would disable it for this one. A
    // second, permanent dispatcher makes it consult every live one.
    static ANCHOR: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    ANCHOR.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    tracing::subscriber::with_default(subscriber, body);
    String::from_utf8(out.0.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect()
}

#[cfg(test)]
mod tests {
    fn emit() {
        tracing::warn!("seen by the capture");
    }

    #[test]
    fn a_callsite_first_hit_on_a_thread_with_no_subscriber_is_still_captured() {
        let lines = super::capture(|| {
            // Another test's thread, logging with no subscriber, reaches the
            // callsite first.
            std::thread::spawn(emit).join().unwrap();
            emit();
        });
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["message"], "seen by the capture");
    }
}
