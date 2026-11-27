use crate::agent::AccessMode;
use crate::orchestrator::policy::Code;
use crate::orchestrator::start::{StartInput, WorkspaceChoice};
use crate::orchestrator::tree::Wake;
use serde_json::{json, Map, Value};

fn fields(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn parse(value: Value) -> Result<StartInput, crate::orchestrator::policy::Refusal> {
    StartInput::parse(&fields(value))
}

#[test]
fn the_smallest_start_defaults_the_rest() {
    let input = parse(json!({"prompt": "Write notes", "workspace": "worktree"})).unwrap();
    assert_eq!(input.prompt, "Write notes");
    assert_eq!(input.workspace, WorkspaceChoice::Worktree);
    assert_eq!(input.wake, Wake::Settled);
    assert_eq!(input.access, None);
    assert_eq!(input.harness, None);
    assert_eq!(input.request_id, None);
}

#[test]
fn every_field_is_read() {
    let input = parse(json!({
        "prompt": "Fix it", "workspace": "main", "harness": "codex", "model": "m",
        "wake": "always", "access": "read-only", "title": "Fixer", "requestId": "r-1",
    }))
    .unwrap();
    assert_eq!(input.workspace, WorkspaceChoice::Main);
    assert_eq!(input.harness.as_deref(), Some("codex"));
    assert_eq!(input.model.as_deref(), Some("m"));
    assert_eq!(input.wake, Wake::Always);
    assert_eq!(input.access, Some(AccessMode::ReadOnly));
    assert_eq!(input.title.as_deref(), Some("Fixer"));
    assert_eq!(input.request_id.as_deref(), Some("r-1"));
}

#[test]
fn a_worktree_can_name_its_base() {
    let input = parse(json!({"prompt": "p", "workspace": "worktree", "base": "develop"})).unwrap();
    assert_eq!(input.base.as_deref(), Some("develop"));
}

#[test]
fn an_existing_folder_must_be_absolute() {
    let absolute = std::env::temp_dir().join("checkout");
    let text = format!("existing:{}", absolute.display());
    let input = parse(json!({"prompt": "p", "workspace": text})).unwrap();
    assert_eq!(
        input.workspace,
        WorkspaceChoice::Existing(absolute.to_string_lossy().into_owned())
    );
    let refusal = parse(json!({"prompt": "p", "workspace": "existing:relative/dir"})).unwrap_err();
    assert_eq!(refusal.code, Code::InvalidInput);
    let refusal = parse(json!({"prompt": "p", "workspace": "existing:"})).unwrap_err();
    assert_eq!(refusal.code, Code::InvalidInput);
}

#[test]
fn bad_input_is_refused_as_invalid_input_naming_the_problem() {
    let cases = [
        (json!({"workspace": "main"}), "prompt"),
        (json!({"prompt": "   ", "workspace": "main"}), "prompt"),
        (json!({"prompt": "p"}), "workspace"),
        (
            json!({"prompt": "p", "workspace": "elsewhere"}),
            "workspace",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "wake": "sometimes"}),
            "wake",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "access": "root"}),
            "access",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "harness": "nope"}),
            "harness",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "base": "dev"}),
            "base",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "requestId": ""}),
            "requestId",
        ),
        (json!({"prompt": 3, "workspace": "main"}), "prompt"),
        (
            json!({"prompt": "p", "workspace": "main", "color": "red"}),
            "color",
        ),
    ];
    for (value, named) in cases {
        let refusal = parse(value.clone()).unwrap_err();
        assert_eq!(refusal.code, Code::InvalidInput, "{value}");
        assert!(refusal.message.contains(named), "{value}: {refusal}");
    }
}

#[test]
fn oversized_text_is_refused() {
    let long = "x".repeat(32_001);
    let refusal = parse(json!({"prompt": long, "workspace": "main"})).unwrap_err();
    assert_eq!(refusal.code, Code::InvalidInput);
    let refusal =
        parse(json!({"prompt": "p", "workspace": "main", "title": "t".repeat(161)})).unwrap_err();
    assert_eq!(refusal.code, Code::InvalidInput);
}

#[test]
fn a_null_optional_field_counts_as_absent() {
    let input =
        parse(json!({"prompt": "p", "workspace": "main", "model": null, "wake": null})).unwrap();
    assert_eq!(input.model, None);
    assert_eq!(input.wake, Wake::Settled);
}

#[test]
fn a_start_on_a_machine_names_it_and_the_checkout_there() {
    let input = parse(json!({
        "prompt": "Fix the build", "workspace": "worktree",
        "machine": "devbox", "cwd": "~/src/app", "base": "main",
    }))
    .unwrap();
    assert_eq!(input.machine.as_deref(), Some("devbox"));
    assert_eq!(input.cwd.as_deref(), Some("~/src/app"));
    let plain = parse(json!({"prompt": "p", "workspace": "main"})).unwrap();
    assert_eq!((plain.machine, plain.cwd), (None, None));
}

#[test]
fn a_machine_start_needs_its_checkout_and_no_local_path() {
    let cases = [
        (
            json!({"prompt": "p", "workspace": "main", "machine": "devbox"}),
            "cwd",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "machine": "devbox", "cwd": "src/app"}),
            "cwd",
        ),
        (
            json!({"prompt": "p", "workspace": "main", "cwd": "/srv/app"}),
            "machine",
        ),
        (
            json!({"prompt": "p", "workspace": "existing:/srv/app", "machine": "devbox", "cwd": "/srv/app"}),
            "workspace",
        ),
    ];
    for (value, named) in cases {
        let refusal = parse(value.clone()).unwrap_err();
        assert_eq!(refusal.code, Code::InvalidInput, "{value}");
        assert!(refusal.message.contains(named), "{value}: {refusal}");
    }
}
