//! One JSON object per line, in the shared diagnostics log schema:
//! `{ ts, level, target, message, app, version, session_id?, span?, fields, event_id }`.
use serde_json::{Map, Value};
use std::io::Write;
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    Event, Subscriber,
};
use tracing_log::NormalizeEvent;
use tracing_subscriber::{
    filter::LevelFilter, fmt::MakeWriter, layer::Context, registry::LookupSpan, EnvFilter, Layer,
};

pub struct JsonLayer<W> {
    writer: W,
    version: String,
}

pub fn json_layer<W>(writer: W, version: &str) -> JsonLayer<W>
where
    W: for<'a> MakeWriter<'a> + 'static,
{
    JsonLayer {
        writer,
        version: version.to_owned(),
    }
}

/// The `GESPAN_LOG` filter. Unless it sets a global level of its own (`warn`,
/// `debug`, …), everything else stays at `info`: `gespan_core::git=debug`
/// raises git without silencing the rest, panics included. A value that does
/// not parse falls back to `info` and returns the parse error, so a typo
/// never turns logging off.
pub fn filter(spec: Option<&str>) -> (EnvFilter, Option<String>) {
    let builder = || EnvFilter::builder().with_default_directive(LevelFilter::INFO.into());
    let spec = spec.unwrap_or_default();
    let global = spec
        .split(',')
        .map(str::trim)
        // `""` parses as ERROR, but EnvFilter skips empty pieces.
        .filter(|directive| !directive.is_empty())
        .any(|directive| directive.parse::<LevelFilter>().is_ok());
    let spec = if global || spec.trim().is_empty() {
        spec.to_owned()
    } else {
        format!("info,{spec}")
    };
    match builder().parse(&spec) {
        Ok(filter) => (filter, None),
        Err(error) => (builder().parse_lossy(""), Some(error.to_string())),
    }
}

/// A span's fields, kept in its extensions until an event inside it.
struct SpanFields(Map<String, Value>);

#[derive(Default)]
struct Fields {
    message: Option<String>,
    values: Map<String, Value>,
}
impl Fields {
    fn put(&mut self, field: &Field, value: Value) {
        // `log` records carry their target, module, file and line as
        // `log.*` fields; the target is restored from them instead.
        if field.name().starts_with("log.") {
            return;
        }
        if field.name() == "message" {
            self.message = Some(match value {
                Value::String(text) => text,
                other => other.to_string(),
            });
        } else {
            self.values.insert(field.name().to_owned(), value);
        }
    }
}
impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.into());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, value.into());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, value.to_string().into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, format!("{value:?}").into());
    }
}

impl<S, W> Layer<S> for JsonLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + 'static,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(fields.values));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        values.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            if let Some(stored) = span.extensions_mut().get_mut::<SpanFields>() {
                stored.0.extend(fields.values);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut merged = Map::new();
        let mut path = Vec::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                path.push(span.name());
                if let Some(stored) = span.extensions().get::<SpanFields>() {
                    merged.extend(stored.0.clone());
                }
            }
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        merged.extend(fields.values);
        let normalized = event.normalized_metadata();
        let metadata = normalized.as_ref().unwrap_or_else(|| event.metadata());
        let mut line = Map::new();
        line.insert(
            "ts".into(),
            chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                .into(),
        );
        line.insert(
            "level".into(),
            metadata.level().as_str().to_ascii_lowercase().into(),
        );
        line.insert("target".into(), metadata.target().into());
        line.insert("message".into(), fields.message.unwrap_or_default().into());
        line.insert("app".into(), "native".into());
        line.insert("version".into(), self.version.clone().into());
        if let Some(session) = merged.remove("session_id") {
            line.insert("session_id".into(), session);
        }
        if !path.is_empty() {
            line.insert("span".into(), path.join(":").into());
        }
        line.insert("fields".into(), Value::Object(merged));
        let event_id = uuid::Uuid::new_v4().to_string();
        if *metadata.level() == tracing::Level::ERROR {
            super::note_error_event(event_id.clone());
        }
        line.insert("event_id".into(), event_id.into());
        // serde_json escapes newlines, so a record is always exactly one line.
        let mut bytes = Value::Object(line).to_string().into_bytes();
        bytes.push(b'\n');
        let _ = self.writer.make_writer_for(metadata).write_all(&bytes);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    /// An in-memory writer: what the layer writes, line by line.
    #[derive(Clone, Default)]
    pub(crate) struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Capture {
        pub(crate) fn lines(&self) -> Vec<Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
                .collect()
        }
        pub(crate) fn raw(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }
    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Runs `body` with only this layer installed, filtered by `spec`.
    pub(crate) fn capture(spec: Option<&str>, body: impl FnOnce()) -> Capture {
        let out = Capture::default();
        let subscriber = tracing_subscriber::registry()
            .with(json_layer(out.clone(), "1.2.3").with_filter(filter(spec).0));
        // With a single registered dispatcher, tracing-core computes a new
        // callsite's interest from the registering thread's default only, so
        // a parallel test with no subscriber would disable it for this one.
        // A second, permanent dispatcher makes it consult every live one.
        static ANCHOR: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
        ANCHOR.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        tracing::subscriber::with_default(subscriber, body);
        out
    }

    #[test]
    fn captures_event_fields() {
        let out = capture(None, || {
            tracing::warn!(
                provider = "codex",
                attempts = 3,
                ok = false,
                "agent start failed"
            );
        });
        let lines = out.lines();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line["level"], "warn");
        assert_eq!(line["message"], "agent start failed");
        assert_eq!(line["target"], module_path!());
        assert_eq!(line["app"], "native");
        assert_eq!(line["version"], "1.2.3");
        assert_eq!(
            line["fields"],
            serde_json::json!({"provider": "codex", "attempts": 3, "ok": false})
        );
        uuid::Uuid::parse_str(line["event_id"].as_str().unwrap()).unwrap();
        chrono::DateTime::parse_from_rfc3339(line["ts"].as_str().unwrap()).unwrap();
        assert!(line.get("span").is_none());
        assert!(line.get("session_id").is_none());
    }

    #[test]
    fn event_ids_are_unique() {
        let out = capture(None, || {
            tracing::info!("one");
            tracing::info!("two");
        });
        let lines = out.lines();
        assert_ne!(lines[0]["event_id"], lines[1]["event_id"]);
    }

    #[test]
    fn span_fields_and_path() {
        let out = capture(None, || {
            let worktree = tracing::info_span!("worktree", session_id = 7, attempt = 1);
            let _w = worktree.enter();
            let git = tracing::info_span!("git", action = "fetch");
            let _g = git.enter();
            git.record("action", "pull");
            tracing::error!(attempt = 2, "git failed");
        });
        let line = &out.lines()[0];
        assert_eq!(line["span"], "worktree:git");
        assert_eq!(line["session_id"], 7);
        // Recorded later than creation, and the event's own field wins.
        assert_eq!(
            line["fields"],
            serde_json::json!({"action": "pull", "attempt": 2})
        );
    }

    #[test]
    fn one_line_per_event_even_with_newlines() {
        let out = capture(None, || {
            tracing::info!(detail = "a\nb", "first\nline");
            tracing::info!("second");
        });
        assert_eq!(out.raw().lines().count(), 2);
        assert_eq!(out.lines()[0]["message"], "first\nline");
        assert_eq!(out.lines()[0]["fields"]["detail"], "a\nb");
    }

    #[test]
    fn errors_and_debug_values_are_strings() {
        let error = std::io::Error::other("disk full");
        let out = capture(None, || {
            tracing::error!(error = &error as &dyn std::error::Error, path = ?"/tmp/x", "write failed");
        });
        let fields = &out.lines()[0]["fields"];
        assert_eq!(fields["error"], "disk full");
        assert_eq!(fields["path"], "\"/tmp/x\"");
    }

    #[test]
    fn log_crate_records_keep_their_own_target() {
        let out = capture(None, || {
            let record = log::Record::builder()
                .level(log::Level::Warn)
                .target("gpui::window")
                .module_path(Some("gpui::window"))
                .file(Some("src/window.rs"))
                .line(Some(13))
                .args(format_args!("from log"))
                .build();
            tracing_log::format_trace(&record).unwrap();
        });
        let line = &out.lines()[0];
        assert_eq!(line["target"], "gpui::window");
        assert_eq!(line["level"], "warn");
        assert_eq!(line["message"], "from log");
        // The bridge's bookkeeping fields are not the event's own fields.
        assert_eq!(line["fields"], serde_json::json!({}));
    }

    #[test]
    fn filter_respects_gespan_log() {
        let out = capture(Some("info,gespan_core::git=debug"), || {
            tracing::debug!(target: "gespan_core::git", "git debug");
            tracing::debug!(target: "gespan_core::store", "store debug");
            tracing::info!(target: "gespan_core::store", "store info");
        });
        let messages: Vec<_> = out.lines().iter().map(|l| l["message"].clone()).collect();
        assert_eq!(messages, ["git debug", "store info"]);
    }

    #[test]
    fn a_target_only_filter_keeps_info_elsewhere() {
        let out = capture(Some("gespan_core::git=debug"), || {
            tracing::debug!(target: "gespan_core::git", "git debug");
            tracing::debug!(target: "gespan_core::store", "store debug");
            tracing::info!(target: "gespan_core::store", "store info");
            tracing::error!(target: "gespan_core::diagnostics", kind = "panic", "panic");
        });
        let messages: Vec<_> = out.lines().iter().map(|l| l["message"].clone()).collect();
        assert_eq!(messages, ["git debug", "store info", "panic"]);
    }

    #[test]
    fn empty_pieces_are_not_a_global_level() {
        for spec in [
            "gespan_core::git=debug,",
            ",gespan_core::git=debug",
            "gespan_core::git=debug,,",
        ] {
            let out = capture(Some(spec), || {
                tracing::debug!(target: "gespan_core::git", "git debug");
                tracing::info!(target: "gespan_core::store", "store info");
                tracing::error!(target: "gespan_core::diagnostics", kind = "panic", "panic");
            });
            let messages: Vec<_> = out.lines().iter().map(|l| l["message"].clone()).collect();
            assert_eq!(messages, ["git debug", "store info", "panic"], "{spec:?}");
        }
    }

    #[test]
    fn an_explicit_global_level_is_kept() {
        let out = capture(Some("warn,gespan_core::git=debug"), || {
            tracing::debug!(target: "gespan_core::git", "git debug");
            tracing::info!(target: "gespan_core::store", "store info");
            tracing::warn!(target: "gespan_core::store", "store warn");
        });
        let messages: Vec<_> = out.lines().iter().map(|l| l["message"].clone()).collect();
        assert_eq!(messages, ["git debug", "store warn"]);
        let quiet = capture(Some("error"), || {
            tracing::warn!("hidden");
            tracing::error!("shown");
        });
        assert_eq!(quiet.lines().len(), 1);
    }

    #[test]
    fn filter_defaults_to_info() {
        let out = capture(None, || {
            tracing::debug!("hidden");
            tracing::info!("shown");
        });
        assert_eq!(out.lines().len(), 1);
        assert_eq!(filter(Some("")).1, None);
    }

    #[test]
    fn invalid_filter_falls_back_to_info() {
        assert!(filter(Some("gespan_core=[bad")).1.is_some());
        let out = capture(Some("gespan_core=[bad"), || {
            tracing::debug!("hidden");
            tracing::info!("shown");
        });
        let messages: Vec<_> = out.lines().iter().map(|l| l["message"].clone()).collect();
        assert_eq!(messages, ["shown"]);
    }
}
