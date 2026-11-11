use super::actions;
use serde_json::Value;
/// Reject unknown actions, misspelled fields, values of the wrong kind and
/// missing required fields before any app mutation. Every rule a field has
/// comes from `actions`.
pub fn validate(action: &str, input: &Value) -> Result<(), String> {
    let spec = actions::find(action).ok_or_else(|| unknown_action(action))?;
    let fields = input.as_object().ok_or("Input must be an object")?;
    for (key, value) in fields {
        let field = spec
            .field(key)
            .ok_or_else(|| format!("{action} does not take {key}"))?;
        if let Some(error) = field.ty.check(key, value) {
            return Err(error);
        }
    }
    if input["draft"] == true && input["notifyOnComplete"] == true {
        return Err("An unsent draft cannot send a completion notification".into());
    }
    if input.get("besideSessionId").is_some()
        && !matches!(input["placement"].as_str(), Some("right" | "down"))
    {
        return Err("besideSessionId requires placement right or down".into());
    }
    let missing: Vec<&str> = spec
        .fields
        .iter()
        .filter(|field| field.required && !fields.contains_key(field.name))
        .map(|field| field.name)
        .collect();
    if !missing.is_empty() {
        return Err(format!("{action} needs {}", missing.join(", ")));
    }
    Ok(())
}

/// Names the action meant, or the actions in the same namespace.
pub(crate) fn unknown_action(action: &str) -> String {
    if let Some(meant) = actions::suggest(action) {
        return format!("Unknown app action {action}. Did you mean {meant}?");
    }
    let namespace = action.split('.').next().unwrap_or_default();
    let siblings: Vec<&str> = actions::all()
        .iter()
        .map(|a| a.name)
        .filter(|name| name.split('.').next() == Some(namespace))
        .collect();
    if siblings.is_empty() {
        format!("Unknown app action {action}. Call the app_help tool to list every action.")
    } else {
        format!(
            "Unknown app action {action}. {namespace} actions are {}.",
            siblings.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn validates_before_mutation() {
        assert!(validate("sessions.start", &json!({"prompt":"x","draft":"false"})).is_err());
        assert!(validate("sessions.delete", &json!({"sessionId":1,"force":true})).is_err());
        assert!(validate("sessions.start", &json!({"runtimeMode":"unsafe"})).is_err());
        assert!(validate(
            "sessions.start",
            &json!({"prompt":"x","project":"/code","draft":true})
        )
        .is_ok());
    }
    #[test]
    fn api_validate_rejects_unknown_flows_keys() {
        for (action, input) in [
            ("flows.list", json!({"owner":"p2"})),
            ("flows.save", json!({"flow":{},"owner":"p2"})),
            ("flows.read", json!({"id":"f1"})),
            (
                "flows.start",
                json!({"flowId":"f1","goal":"g","task":"t","tag":{}}),
            ),
            ("flows.runs", json!({"owner":"p2"})),
            ("flows.run", json!({"runId":"r1","limit":5})),
            (
                "flows.editRun",
                json!({"runId":"r1","flow":{},"note":"n","force":true}),
            ),
            ("flows.cancel", json!({"runId":"r1","flowId":"f1"})),
            (
                "flows.resume",
                json!({"runId":"r1","stateId":"s","inputs":{}}),
            ),
            ("flows.delete", json!({"flowId":"f1","runId":"r1"})),
            ("flows.approve", json!({"runId":"r1"})),
        ] {
            assert!(validate(action, &input).is_err(), "{action} {input}");
        }
        assert_eq!(
            validate("flows.save", &json!({"flow":"{}"})),
            Err("flow must be a Flow object".into())
        );
        assert_eq!(
            validate("flows.start", &json!({"flowId":"f1","inputs":{"task":1}})),
            Err("inputs must be an object of text values".into())
        );
        assert_eq!(
            validate("flows.start", &json!({"flowId":"f1","inputs":["task"]})),
            Err("inputs must be an object of text values".into())
        );
        assert_eq!(
            validate("flows.read", &json!({"flowId":7})),
            Err("flowId must be text".into())
        );
        for (action, input) in [
            ("flows.list", json!({})),
            ("flows.read", json!({"flowId":"f1"})),
            ("flows.save", json!({"flow":{"name":"x"}})),
            ("flows.delete", json!({"flowId":"f1"})),
            (
                "flows.start",
                json!({"flowId":"f1","goal":"g","task":"t","inputs":{"task":"x"}}),
            ),
            ("flows.runs", json!({"goal":"g"})),
            ("flows.run", json!({"runId":"r1"})),
            ("flows.editRun", json!({"runId":"r1","flow":{},"note":"n"})),
            ("flows.cancel", json!({"runId":"r1"})),
            ("flows.resume", json!({"runId":"r1","stateId":"build"})),
        ] {
            assert_eq!(validate(action, &input), Ok(()), "{action} {input}");
        }
    }
    #[test]
    fn every_missing_required_field_is_named_at_once() {
        assert_eq!(
            validate("flows.start", &json!({"flowId":"f1"})),
            Err("flows.start needs goal, task".into())
        );
        assert_eq!(
            validate("soul.update", &json!({})),
            Err("soul.update needs text, expectedHash".into())
        );
        assert_eq!(
            validate(
                "habits.add",
                &json!({"name":"x","instructions":"y","schedule":"daily"})
            ),
            Err("schedule must be an object".into())
        );
        // A wrong value is named before what's missing.
        assert_eq!(
            validate("sessions.send", &json!({"sessionId":true})),
            Err("sessionId must be an ID".into())
        );
    }
    #[test]
    fn an_unknown_action_names_the_one_meant_or_its_namespace() {
        assert_eq!(
            validate("memory_add", &json!({})),
            Err("Unknown app action memory_add. Did you mean memory.add?".into())
        );
        assert_eq!(
            validate("soul.write", &json!({})),
            Err("Unknown app action soul.write. soul actions are soul.read, soul.update.".into())
        );
        assert_eq!(
            validate("deploy", &json!({})),
            Err("Unknown app action deploy. Call the app_help tool to list every action.".into())
        );
    }
}
