use serde_json::Value;
/// Reject misspelled fields and ambiguous types before any app mutation.
pub fn validate(action: &str, input: &Value) -> Result<(), String> {
    let fields=match action{
        "models.list"|"soul.read"|"habits.list"=>"",
        "sessions.list"|"folders.list"|"worktrees.list"=>"project",
        "sessions.read"=>"sessionId project before limit maxChars",
        "sessions.start"=>"prompt project harness model modelSettings runtimeMode title draft reveal workspaceMode worktreeBase worktreeCwd notifyOnComplete",
        "sessions.send"=>"sessionId prompt project notifyOnComplete",
        "sessions.draft"=>"sessionId prompt project",
        "sessions.stop"|"sessions.archive"|"sessions.delete"=>"sessionId project",
        "worktrees.create"=>"branch base existing project",
        "folders.move"=>"sessionId folderId newFolderName project",
        "notes.list"=>"limit offset","notes.read"=>"id","notes.write"=>"id title body tags",
        "artifacts.list"=>"kind limit offset","artifacts.read"=>"id","artifacts.write"=>"id kind title body summary",
        "soul.update"=>"text expectedHash","memory.read"=>"topic","memory.search"=>"query since",
        "memory.add"=>"fact topic until","memory.replace"=>"find fact topic until","memory.remove"=>"find topic",
        "habits.add"=>"name instructions schedule","habits.update"=>"id name instructions schedule enabled","habits.run"|"habits.remove"=>"id",
        "chat.card"=>"type repo number note sessionId options name instructions schedule",
        "flows.list"=>"","flows.read"|"flows.delete"=>"flowId","flows.save"=>"flow","flows.start"=>"flowId goal task inputs",
        "flows.runs"=>"goal","flows.run"|"flows.cancel"=>"runId","flows.editRun"=>"runId flow note","flows.resume"=>"runId stateId",
        _=>return Err("Unknown app action. Run app help".into()),
    };
    for (key, value) in input.as_object().ok_or("Input must be an object")? {
        if !fields.split_whitespace().any(|f| f == key) {
            return Err(format!("{action} does not take {key}"));
        }
        if ["draft", "reveal", "notifyOnComplete", "existing", "enabled"].contains(&key.as_str())
            && !value.is_boolean()
        {
            return Err(format!("{key} must be true or false"));
        }
        if ["before", "limit", "offset", "maxChars", "number"].contains(&key.as_str())
            && value.as_u64().is_none()
        {
            return Err(format!("{key} must be a non-negative integer"));
        }
        if ![
            "draft",
            "reveal",
            "notifyOnComplete",
            "existing",
            "enabled",
            "before",
            "limit",
            "offset",
            "maxChars",
            "number",
            "id",
            "sessionId",
            "modelSettings",
            "schedule",
            "tags",
            "options",
            "flow",
            "inputs",
        ]
        .contains(&key.as_str())
            && !value.is_string()
        {
            return Err(format!("{key} must be text"));
        }
    }
    if input.get("runtimeMode").is_some_and(|v| {
        !matches!(
            v.as_str(),
            Some("read-only" | "supervised" | "auto-accept-edits" | "auto" | "full-access")
        )
    }) {
        return Err("Unknown runtimeMode".into());
    }
    if input
        .get("workspaceMode")
        .is_some_and(|v| !matches!(v.as_str(), Some("current" | "checkout" | "worktree")))
    {
        return Err("workspaceMode must be current or worktree".into());
    }
    if input.get("flow").is_some_and(|v| !v.is_object()) {
        return Err("flow must be a Flow object".into());
    }
    if input.get("inputs").is_some_and(|v| {
        v.as_object()
            .is_none_or(|o| o.values().any(|v| !v.is_string()))
    }) {
        return Err("inputs must be an object of text values".into());
    }
    if input.get("tags").is_some_and(|v| {
        v.as_array().is_none_or(|a| {
            a.len() > 32 || a.iter().any(|v| v.as_str().is_none_or(|s| s.len() > 160))
        })
    }) {
        return Err("tags must be an array of up to 32 short strings".into());
    }
    Ok(())
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
}
