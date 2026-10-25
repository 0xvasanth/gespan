//! Structured log fields and report JSON.

use serde_json::Value;

use crate::patterns::is_secret_key;
use crate::{Redactor, REDACTED};

/// Fields that hold user content. They are dropped, whatever they contain;
/// logging code names anything sensitive with one of these so it never leaves.
pub const DENY_LIST: [&str; 8] = [
    "prompt",
    "content",
    "diff",
    "output",
    "body",
    "env",
    "audio",
    "transcript",
];

impl Redactor {
    /// Drops deny-listed fields at any depth, blanks secret-named fields, and
    /// scrubs every remaining key and string with [`Redactor::redact_text`].
    pub fn redact_value(&self, value: Value) -> Value {
        match value {
            Value::String(text) => match embedded_json(&text) {
                Some(inner) => Value::String(self.redact_value(inner).to_string()),
                None => Value::String(self.redact_text(&text)),
            },
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.redact_value(item))
                    .collect(),
            ),
            Value::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .filter(|(key, _)| !is_denied(key))
                    .map(|(key, value)| {
                        let value = if is_secret_key(&key) {
                            Value::String(REDACTED.to_string())
                        } else {
                            self.redact_value(value)
                        };
                        (self.redact_text(&key), value)
                    })
                    .collect(),
            ),
            other => other,
        }
    }
}

/// A string that is itself a JSON object or array, such as a response body
/// logged as text, so its fields get the same treatment.
fn embedded_json(text: &str) -> Option<Value> {
    let trimmed = text.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    serde_json::from_str(text)
        .ok()
        .filter(|value: &Value| value.is_object() || value.is_array())
}

fn is_denied(key: &str) -> bool {
    DENY_LIST
        .iter()
        .any(|denied| key.eq_ignore_ascii_case(denied))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{Config, Redactor};

    fn redactor() -> Redactor {
        Redactor::new(Config {
            home: Some("/Users/alice".into()),
            ..Config::default()
        })
    }

    #[test]
    fn deny_listed_fields_are_dropped() {
        let input = json!({
            "level": "error",
            "prompt": "fix it", "content": "x", "diff": "+a", "output": "ok",
            "body": {"nested": true}, "env": {"HOME": "/Users/alice"},
            "audio": [1, 2], "transcript": "hello", "Prompt": "mixed case",
        });
        assert_eq!(redactor().redact_value(input), json!({"level": "error"}));
    }

    #[test]
    fn similar_names_survive() {
        let input = json!({"prompt_id": 7, "contents": "toc", "output_kind": "text"});
        assert_eq!(redactor().redact_value(input.clone()), input);
    }

    #[test]
    fn deny_listed_fields_inside_arrays_are_dropped() {
        let input = json!({"turns": [{"prompt": "x", "id": 1}, {"fields": {"diff": "y", "n": 2}}]});
        assert_eq!(
            redactor().redact_value(input),
            json!({"turns": [{"id": 1}, {"fields": {"n": 2}}]})
        );
    }

    #[test]
    fn secret_named_fields_are_blanked_whatever_their_type() {
        let input =
            json!({"api_key": 42, "token": "t", "GITHUB_TOKEN": {"a": 1}, "tokens_used": 9});
        assert_eq!(
            redactor().redact_value(input),
            json!({"api_key": "<redacted>", "token": "<redacted>", "GITHUB_TOKEN": "<redacted>", "tokens_used": 9})
        );
    }

    #[test]
    fn keys_and_strings_are_scrubbed() {
        let input = json!({
            "/Users/alice/notes.md": "seen by bob@x.io",
            "list": ["/Users/alice/a", "Bearer abcdefgh12345678"],
        });
        assert_eq!(
            redactor().redact_value(input),
            json!({"~/notes.md": "seen by <email>", "list": ["~/a", "Bearer <redacted>"]})
        );
    }

    #[test]
    fn json_inside_a_string_is_scrubbed_as_json() {
        let input = json!({"response": "{\"prompt\":\"hi\",\"ok\":true}", "note": "{not json"});
        assert_eq!(
            redactor().redact_value(input),
            json!({"response": "{\"ok\":true}", "note": "{not json"})
        );
    }

    #[test]
    fn numbers_bools_and_null_are_untouched() {
        let input = json!([1, 2.5, true, null, {"n": -3}]);
        assert_eq!(redactor().redact_value(input.clone()), input);
    }
}
