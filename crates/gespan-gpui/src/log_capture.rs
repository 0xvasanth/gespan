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
    tracing::subscriber::with_default(subscriber, body);
    String::from_utf8(out.0.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect()
}
