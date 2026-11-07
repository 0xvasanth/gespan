use super::super::toolkit::{Toolkit, Waits};
use super::super::{detect, set_personal, Personal};
use crate::tool_servers::{self, Mount, ACTIONS, ACTIONS_READ};
use mcp_hub::{Grant, Toolkit as _};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RUN: Toolkit = Toolkit { may_run: true };
const READ: Toolkit = Toolkit { may_run: false };

/// Long enough for a loaded CI runner to finish a quick action, but a run
/// still going returns at once.
const SHORT: Waits = Waits {
    run: Duration::from_secs(30),
    background: Duration::from_millis(300),
    output: Duration::from_millis(300),
};

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn gespan(dir: &Path, tasks: &str) {
    write(
        dir,
        ".gespan/tasks.json",
        &format!(r#"{{ "version": "2.0.0", "tasks": [{tasks}] }}"#),
    );
}

/// A grant no session refreshed, so the project is the workspace itself.
fn grant(session: i64, workspace: &Path, toolkit: &str) -> Grant {
    Grant {
        session,
        workspace: workspace.to_path_buf(),
        toolkits: BTreeSet::from([toolkit.to_owned()]),
        servers: BTreeSet::new(),
    }
}

fn names(toolkit: &Toolkit, grant: &Grant) -> Vec<String> {
    toolkit
        .tools(grant)
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

fn lines(report: &Value) -> Vec<String> {
    report["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().trim().to_owned())
        .collect()
}

fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap()
}

/// A command that sleeps for about `seconds`.
fn sleeper(seconds: u32) -> String {
    if cfg!(windows) {
        format!("ping -n {} 127.0.0.1 >NUL", seconds + 1)
    } else {
        format!("sleep {seconds}")
    }
}

#[test]
fn a_read_only_session_is_offered_list_only_and_run_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Mark", "command": "echo x > ran.txt" }"#,
    );
    assert_eq!(READ.name(), ACTIONS_READ);
    assert_eq!(RUN.name(), ACTIONS);
    let read = grant(7101, root, ACTIONS_READ);
    assert_eq!(names(&READ, &read), ["actions_list"]);
    assert_eq!(
        names(&RUN, &grant(7101, root, ACTIONS)),
        [
            "actions_list",
            "actions_run",
            "actions_output",
            "actions_stop"
        ]
    );
    for (tool, input) in [
        (
            "actions_run",
            json!({"id": "gespan:Mark", "command": "echo x > ran.txt"}),
        ),
        ("actions_output", json!({"runId": "run-1"})),
        ("actions_stop", json!({"runId": "run-1"})),
        ("actions_anything", json!({})),
    ] {
        assert_eq!(
            READ.call(&read, tool, input).unwrap_err(),
            "Read-only sessions can list actions but not run them.",
            "{tool}"
        );
    }
    assert!(!root.join("ran.txt").exists(), "nothing ran");
    let listed = READ.call(&read, "actions_list", json!({})).unwrap();
    assert_eq!(listed["actions"][0]["id"], "gespan:Mark");
}

#[test]
fn the_hub_keeps_a_read_only_session_off_the_run_tools() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Mark", "command": "echo x > ran.txt" }"#,
    );
    let connection = tool_servers::test_connection(&Mount {
        session: 7102,
        worktree: root.to_path_buf(),
        toolkits: BTreeSet::from([ACTIONS_READ.to_owned()]),
        project: root.display().to_string(),
        ..Mount::default()
    });
    let listed = mcp_hub::call_tool(connection.clone(), "actions_list", json!({})).unwrap();
    assert_eq!(listed["actions"][0]["id"], "gespan:Mark");
    let refused = mcp_hub::call_tool(
        connection,
        "actions_run",
        json!({"id": "gespan:Mark", "command": "echo x > ran.txt"}),
    );
    assert!(
        refused.unwrap_err().contains("isn't available"),
        "never offered, so never reached"
    );
    assert!(!root.join("ran.txt").exists());
}

#[test]
fn list_shape_has_ids_and_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Test", "command": "echo hi", "detail": "The suite" },
           { "label": "Dev", "command": "echo dev", "isBackground": true },
           { "label": "Secret", "command": "echo shh", "hide": true },
           { "label": "Open", "command": "code ${file}" }"#,
    );
    write(
        root,
        "package.json",
        r#"{ "scripts": { "lint": "eslint ." } }"#,
    );
    write(root, ".vscode/tasks.json", "{ not json");
    let listed = RUN
        .call(&grant(7103, root, ACTIONS), "actions_list", json!({}))
        .unwrap();
    let cwd = detect(&root.display().to_string(), root).actions[0]
        .cwd
        .display()
        .to_string();
    let actions = listed["actions"].as_array().unwrap();
    let ids: Vec<&str> = actions.iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "gespan:Test",
            "gespan:Dev",
            "gespan:Open",
            "package.json:lint"
        ],
        "a hidden action isn't listed"
    );
    assert_eq!(
        actions[0],
        json!({
            "id": "gespan:Test",
            "label": "Test",
            "detail": "The suite",
            "source": ".gespan/tasks.json",
            "command": "echo hi",
            "cwd": cwd,
            "background": false,
            "blocked": null,
            "env": [],
        })
    );
    assert_eq!(actions[1]["background"], true);
    assert_eq!(
        actions[2]["blocked"],
        "Uses ${file}, which Gespan can't fill in"
    );
    assert_eq!(actions[3]["source"], "package.json");
    assert_eq!(listed["errors"][0]["file"], ".vscode/tasks.json");
    assert!(listed["errors"][0]["reason"].is_string());
}

#[test]
fn run_returns_exit_code_and_output() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Hi", "command": "echo hi" },
           { "label": "Fail", "command": "echo oops && exit 3" }"#,
    );
    // Through the hub, as an agent calls it.
    let connection = tool_servers::test_connection(&Mount {
        session: 7104,
        worktree: root.to_path_buf(),
        toolkits: BTreeSet::from([ACTIONS.to_owned()]),
        project: root.display().to_string(),
        ..Mount::default()
    });
    let report = mcp_hub::call_tool(
        connection.clone(),
        "actions_run",
        json!({"id": "gespan:Hi", "command": "echo hi"}),
    )
    .unwrap();
    assert_eq!(report["status"], "exited", "{report}");
    assert_eq!(report["exitCode"], 0);
    assert_eq!(lines(&report), ["hi"]);
    assert!(report["runId"].as_str().unwrap().starts_with("run-"));
    let failed = mcp_hub::call_tool(
        connection,
        "actions_run",
        json!({"id": "gespan:Fail", "command": "echo oops && exit 3"}),
    )
    .unwrap();
    assert_eq!(failed["status"], "exited", "{failed}");
    assert_eq!(
        failed["exitCode"], 3,
        "a failing exit is a report, not an error"
    );
    assert_eq!(lines(&failed), ["oops"]);
}

#[test]
fn run_uses_the_sessions_checkout() {
    let main = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let print = if cfg!(windows) { "cd" } else { "pwd" };
    for dir in [main.path(), worktree.path()] {
        gespan(
            dir,
            &format!(r#"{{ "label": "Where", "command": "{print}" }}"#),
        );
    }
    // The session's project is the main checkout; it works in its worktree.
    let mount = Mount {
        session: 7105,
        worktree: worktree.path().to_path_buf(),
        toolkits: BTreeSet::from([ACTIONS.to_owned()]),
        project: main.path().display().to_string(),
        ..Mount::default()
    };
    tool_servers::refresh(&mount);
    let report = RUN
        .call(
            &grant(7105, worktree.path(), ACTIONS),
            "actions_run",
            json!({"id": "gespan:Where", "command": print}),
        )
        .unwrap();
    assert_eq!(report["status"], "exited", "{report}");
    let printed = lines(&report);
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert_eq!(real(Path::new(&printed[0])), real(worktree.path()));
}

#[test]
fn an_unknown_id_is_refused_with_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    gespan(dir.path(), r#"{ "label": "Hi", "command": "echo hi" }"#);
    let refused = RUN.call(
        &grant(7106, dir.path(), ACTIONS),
        "actions_run",
        json!({"id": "gespan:Nope", "command": "echo hi"}),
    );
    assert_eq!(
        refused.unwrap_err(),
        "Unknown action: gespan:Nope. Call actions_list for the ids."
    );
}

#[test]
fn a_blocked_action_is_refused_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Open", "command": "echo x > ran.txt ${file}" }"#,
    );
    let refused = RUN.call(
        &grant(7107, root, ACTIONS),
        "actions_run",
        json!({"id": "gespan:Open", "command": "echo x > ran.txt ${file}"}),
    );
    assert_eq!(
        refused.unwrap_err(),
        "Open can't run: Uses ${file}, which Gespan can't fill in"
    );
    assert!(!root.join("ran.txt").exists(), "nothing ran");
}

#[test]
fn a_background_action_returns_while_running_and_stops_on_request() {
    let dir = tempfile::tempdir().unwrap();
    gespan(
        dir.path(),
        &format!(
            r#"{{ "label": "Serve", "command": "echo up && {}", "isBackground": true }}"#,
            sleeper(60)
        ),
    );
    let grant = grant(7108, dir.path(), ACTIONS);
    let listed = RUN.call(&grant, "actions_list", json!({})).unwrap();
    let command = listed["actions"][0]["command"].clone();
    let started = Instant::now();
    let report = RUN
        .call_waiting(
            &grant,
            "actions_run",
            json!({"id": "gespan:Serve", "command": command}),
            &SHORT,
        )
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(report["status"], "running", "{report}");
    let run_id = report["runId"].as_str().unwrap().to_owned();
    let later = RUN
        .call_waiting(&grant, "actions_output", json!({"runId": run_id}), &SHORT)
        .unwrap();
    assert_eq!(later["status"], "running", "{later}");
    let stopped = RUN
        .call(&grant, "actions_stop", json!({"runId": run_id}))
        .unwrap();
    assert_eq!(stopped["status"], "stopped", "{stopped}");
}

#[test]
fn personal_actions_come_from_the_sessions_project_or_else_its_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let personal = |id: &str| Personal {
        id: id.into(),
        name: id.into(),
        command: format!("echo {id}"),
        icon: "play".into(),
    };
    let project = format!("toolkit-project-{}", root.display());
    set_personal(&project, vec![personal("by-project")]);
    set_personal(&root.display().to_string(), vec![personal("by-workspace")]);
    let ids = |grant: &Grant| -> Vec<String> {
        let listed = RUN.call(grant, "actions_list", Value::Null).unwrap();
        listed["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|action| action["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        ids(&grant(7109, root, ACTIONS)),
        ["personal:by-workspace"],
        "a session Gespan hasn't mounted keys by its workspace"
    );
    tool_servers::refresh(&Mount {
        session: 7110,
        worktree: root.to_path_buf(),
        toolkits: BTreeSet::from([ACTIONS.to_owned()]),
        project: project.clone(),
        ..Mount::default()
    });
    assert_eq!(ids(&grant(7110, root, ACTIONS)), ["personal:by-project"]);
    tool_servers::refresh(&Mount {
        session: 7112,
        worktree: root.to_path_buf(),
        ..Mount::default()
    });
    assert_eq!(
        ids(&grant(7112, root, ACTIONS)),
        ["personal:by-workspace"],
        "a mount with no project keys by the workspace too"
    );
    set_personal(&project, Vec::new());
    set_personal(&root.display().to_string(), Vec::new());
}

#[test]
fn arguments_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let grant = grant(7111, dir.path(), ACTIONS);
    for (tool, input, error) in [
        (
            "actions_run",
            json!("gespan:Hi"),
            "Arguments must be an object",
        ),
        ("actions_run", json!({}), "id is required"),
        ("actions_run", json!({"id": 3}), "id must be a string"),
        (
            "actions_run",
            json!({"id": "gespan:Hi"}),
            "command is required",
        ),
        (
            "actions_run",
            json!({"id": "gespan:Hi", "command": ["echo"]}),
            "command must be a string",
        ),
        (
            "actions_run",
            json!({"id": "gespan:Hi", "command": "echo hi", "waitMs": 1}),
            "Unknown argument: waitMs",
        ),
        (
            "actions_list",
            json!({"all": true}),
            "Unknown argument: all",
        ),
        ("actions_output", json!({}), "runId is required"),
        (
            "actions_stop",
            json!({"id": "run-1"}),
            "Unknown argument: id",
        ),
        (
            "actions_output",
            json!({"runId": "run-none"}),
            "No run run-none. Finished runs are kept only for a while.",
        ),
        ("actions_nothing", json!({}), "Unknown actions tool"),
    ] {
        assert_eq!(
            RUN.call(&grant, tool, input.clone()).unwrap_err(),
            error,
            "{tool} {input}"
        );
    }
}

#[test]
fn a_changed_command_is_refused_until_listed_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Mark", "command": "echo x > ran.txt" }"#,
    );
    let grant = grant(7113, root, ACTIONS);
    let approved = json!({"id": "gespan:Mark", "command": "echo x > ran.txt"});
    // The repository's file changed after the agent listed it.
    gespan(
        root,
        r#"{ "label": "Mark", "command": "echo y > ran.txt" }"#,
    );
    assert_eq!(
        RUN.call(&grant, "actions_run", approved).unwrap_err(),
        "The command for gespan:Mark is now `echo y > ran.txt`; call actions_list again."
    );
    assert!(!root.join("ran.txt").exists(), "nothing ran");
    let report = RUN
        .call(
            &grant,
            "actions_run",
            json!({"id": "gespan:Mark", "command": "echo y > ran.txt"}),
        )
        .unwrap();
    assert_eq!(report["exitCode"], 0, "{report}");
    assert_eq!(
        std::fs::read_to_string(root.join("ran.txt"))
            .unwrap()
            .trim(),
        "y"
    );
}

#[test]
fn a_run_is_out_of_reach_from_another_checkout() {
    let ours = tempfile::tempdir().unwrap();
    let theirs = tempfile::tempdir().unwrap();
    gespan(
        ours.path(),
        &format!(
            r#"{{ "label": "Serve", "command": "{}", "isBackground": true }}"#,
            sleeper(60)
        ),
    );
    let mine = grant(7114, ours.path(), ACTIONS);
    let other = grant(7115, theirs.path(), ACTIONS);
    let listed = RUN.call(&mine, "actions_list", json!({})).unwrap();
    let command = listed["actions"][0]["command"].clone();
    let report = RUN
        .call_waiting(
            &mine,
            "actions_run",
            json!({"id": "gespan:Serve", "command": command}),
            &SHORT,
        )
        .unwrap();
    let run_id = report["runId"].as_str().unwrap().to_owned();
    let unknown = |id: &str| format!("No run {id}. Finished runs are kept only for a while.");
    for tool in ["actions_output", "actions_stop"] {
        assert_eq!(
            RUN.call_waiting(&other, tool, json!({"runId": run_id}), &SHORT)
                .unwrap_err(),
            unknown(&run_id),
            "{tool}: the same as an id that doesn't exist"
        );
        assert_eq!(
            RUN.call_waiting(&other, tool, json!({"runId": "run-0"}), &SHORT)
                .unwrap_err(),
            unknown("run-0")
        );
    }
    let still = RUN
        .call_waiting(&mine, "actions_output", json!({"runId": run_id}), &SHORT)
        .unwrap();
    assert_eq!(still["status"], "running", "{still}");
    let stopped = RUN
        .call(&mine, "actions_stop", json!({"runId": run_id}))
        .unwrap();
    assert_eq!(stopped["status"], "stopped", "{stopped}");
}

#[test]
fn tools_tell_the_agent_which_ones_change_things() {
    let dir = tempfile::tempdir().unwrap();
    let tools = RUN.tools(&grant(7116, dir.path(), ACTIONS));
    let annotations =
        |name: &str| tools.iter().find(|tool| tool["name"] == name).unwrap()["annotations"].clone();
    assert_eq!(annotations("actions_list")["readOnlyHint"], true);
    assert_eq!(annotations("actions_output")["readOnlyHint"], true);
    assert_eq!(annotations("actions_run")["destructiveHint"], true);
    assert_eq!(annotations("actions_stop")["destructiveHint"], true);
    let run = tools
        .iter()
        .find(|tool| tool["name"] == "actions_run")
        .unwrap();
    assert_eq!(run["inputSchema"]["required"], json!(["id", "command"]));
    assert!(run["inputSchema"]["properties"]["command"]["description"]
        .as_str()
        .unwrap()
        .contains("exactly as actions_list returned it"));
}

#[test]
fn a_listed_action_shows_its_env_names_but_never_the_values() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gespan(
        root,
        r#"{ "label": "Serve", "command": "echo serve",
             "options": { "env": { "A_PORT": "3000", "B_TOKEN": "s3cret" } } },
           { "label": "Plain", "command": "echo plain" }"#,
    );
    let listed = RUN
        .call(&grant(7121, root, ACTIONS), "actions_list", json!({}))
        .unwrap();
    let actions = listed["actions"].as_array().unwrap();
    assert_eq!(actions[0]["env"], json!(["A_PORT", "B_TOKEN"]));
    assert_eq!(actions[1]["env"], json!([]));
    assert!(!listed.to_string().contains("s3cret"));
}
