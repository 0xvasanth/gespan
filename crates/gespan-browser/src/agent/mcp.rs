//! Maps an action result onto an MCP `CallToolResult`. The host that serves
//! the `browser_*` tools lives outside this crate; this is the one place that
//! decides how a result is shaped for the model.

use serde_json::{json, Value};

use crate::protocol::{ActionKind, ActionResult, Outcome, Reason};

/// Map one action result onto an MCP `CallToolResult`.
///
/// - done: the message and the compact envelope; snapshot and read put their
///   text first, screenshot adds an image block, and the page snapshot of
///   an action follows the message as its own text block.
/// - blocked, failed, unknown: `isError`, with the reason and whether a retry
///   can help spelled out so the model need not parse JSON.
pub fn tool_result(kind: Option<ActionKind>, result: &ActionResult) -> Value {
    let mut content = Vec::new();
    let mut envelope = result.clone();
    if result.outcome == Outcome::Done {
        match kind {
            Some(
                ActionKind::Snapshot | ActionKind::Read | ActionKind::Console | ActionKind::Network,
            ) => {
                if let Some(text) = data_text(result.data.as_ref()) {
                    content.push(json!({"type": "text", "text": text}));
                    envelope.data = data_without(result.data.as_ref(), &TEXT_KEYS);
                }
            }
            Some(ActionKind::Screenshot) => {
                if let Some((image, mime)) = data_image(result.data.as_ref()) {
                    content.push(json!({"type": "image", "data": image, "mimeType": mime}));
                    envelope.data = data_without(result.data.as_ref(), &IMAGE_KEYS);
                }
            }
            // An action's page snapshot follows the summary as its own text.
            _ => {
                if let Some(text) = data_snapshot(result.data.as_ref()) {
                    content.push(json!({"type": "text", "text": text}));
                    envelope.data = data_without(result.data.as_ref(), &["snapshot"]);
                }
            }
        }
        let summary = format!("{}\n{}", result.message, compact(&envelope));
        // The text the agent asked for leads; the envelope follows it. An
        // image goes after the text that explains it.
        match kind {
            Some(
                ActionKind::Snapshot | ActionKind::Read | ActionKind::Console | ActionKind::Network,
            ) => content.push(json!({"type": "text", "text": summary})),
            _ => content.insert(0, json!({"type": "text", "text": summary})),
        }
        return json!({"content": content, "isError": false});
    }
    let reason = result.reason.map(Reason::as_str).unwrap_or("none");
    let advice = match (result.outcome, result.retryable) {
        (Outcome::Unknown, _) => format!(
            "The action may have happened: take a snapshot before retrying, and retry only with request_id {}.",
            result.request_id
        ),
        (_, true) => "Retrying can help once the cause is gone.".to_string(),
        (_, false) => "Retrying the same call will not help.".to_string(),
    };
    let text = format!(
        "{}\noutcome: {}; reason: {}; retryable: {}. {}\n{}",
        result.message,
        result.outcome.as_str(),
        reason,
        result.retryable,
        advice,
        compact(&envelope)
    );
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

/// The envelope on one line, without the message (already the first line of
/// the text) and without empty parts.
fn compact(result: &ActionResult) -> String {
    let mut value = serde_json::to_value(result).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, value| match key.as_str() {
            "message" => false,
            "data" | "reason" => !value.is_null(),
            "observed" => value
                .as_object()
                .is_some_and(|o| o.iter().any(|(_, v)| v != &json!(false))),
            _ => true,
        });
    }
    value.to_string()
}

/// Text payload of a snapshot or read result: `data.text`, or `data` itself
/// when the semantic layer returns a bare string.
pub fn data_text(data: Option<&Value>) -> Option<&str> {
    let data = data?;
    data.as_str().or_else(|| {
        ["text", "snapshot", "content"]
            .iter()
            .find_map(|key| data.get(key).and_then(Value::as_str))
    })
}

/// The page snapshot an action (not `snapshot` itself) returned.
pub fn data_snapshot(data: Option<&Value>) -> Option<&str> {
    data?.get("snapshot").and_then(Value::as_str)
}

/// Base64 image of a screenshot result and its media type.
pub fn data_image(data: Option<&Value>) -> Option<(&str, String)> {
    let data = data?;
    let image = data.as_str().or_else(|| {
        ["image", "data", "base64"]
            .iter()
            .find_map(|key| data.get(key).and_then(Value::as_str))
    })?;
    let declared = ["mime_type", "mimeType", "format"]
        .iter()
        .find_map(|key| data.get(key).and_then(Value::as_str))
        .unwrap_or("png");
    let mime = match declared.trim_start_matches("image/") {
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Some((image, mime.to_string()))
}

/// `data` with the keys [`data_text`] / [`data_image`] read removed, so the
/// envelope that travels next to the extracted content does not repeat it.
pub fn data_without(data: Option<&Value>, keys: &[&str]) -> Option<Value> {
    match data? {
        Value::String(_) => None,
        Value::Object(map) => {
            let rest: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(key, _)| !keys.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            (!rest.is_empty()).then_some(Value::Object(rest))
        }
        other => Some(other.clone()),
    }
}

pub const TEXT_KEYS: [&str; 3] = ["text", "snapshot", "content"];
pub const IMAGE_KEYS: [&str; 3] = ["image", "data", "base64"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_are_found_in_canonical_and_bare_shapes() {
        let snapshot = json!({"text": "- button \"Go\" [ref=e1]", "version": 3});
        assert_eq!(data_text(Some(&snapshot)), Some("- button \"Go\" [ref=e1]"));
        assert_eq!(
            data_without(Some(&snapshot), &TEXT_KEYS),
            Some(json!({"version": 3}))
        );
        assert_eq!(data_text(Some(&json!("bare"))), Some("bare"));
        assert_eq!(data_without(Some(&json!("bare")), &TEXT_KEYS), None);
        assert_eq!(data_text(Some(&json!({"tabs": []}))), None);

        let shot = json!({"image": "iVBO", "mime_type": "image/jpeg", "width": 10});
        assert_eq!(
            data_image(Some(&shot)),
            Some(("iVBO", "image/jpeg".to_string()))
        );
        assert_eq!(
            data_image(Some(&json!({"data": "AAAA", "format": "png"}))),
            Some(("AAAA", "image/png".to_string()))
        );
        assert_eq!(
            data_without(Some(&shot), &IMAGE_KEYS),
            Some(json!({"mime_type": "image/jpeg", "width": 10}))
        );
    }

    #[test]
    fn a_done_snapshot_leads_with_its_text_and_an_action_snapshot_follows_the_summary() {
        let mut snapshot = ActionResult::from_reason("req_1", Reason::InvalidRequest);
        snapshot.outcome = Outcome::Done;
        snapshot.reason = None;
        snapshot.message = "Snapshot taken.".into();
        snapshot.data = Some(json!({"text": "- button \"Go\" [ref=e1]", "version": 3}));
        let result = tool_result(Some(ActionKind::Snapshot), &snapshot);
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "- button \"Go\" [ref=e1]");
        assert!(result["content"][1]["text"]
            .as_str()
            .unwrap()
            .starts_with("Snapshot taken."));

        snapshot.data = Some(json!({"snapshot": "page text"}));
        let result = tool_result(Some(ActionKind::Click), &snapshot);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Snapshot taken."));
        assert_eq!(result["content"][1]["text"], "page text");
    }

    #[test]
    fn a_refusal_is_an_error_naming_the_reason_and_whether_a_retry_can_help() {
        let result = tool_result(
            Some(ActionKind::Click),
            &ActionResult::from_reason("req_2", Reason::NotGranted),
        );
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("reason: not_granted"), "{text}");
        assert!(text.contains("retryable:"), "{text}");
    }
}
