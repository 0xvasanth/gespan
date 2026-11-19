//! Agents on herdr machines as peers: a chat sees one working in a checkout
//! of the same repository, starts one on a machine, messages it and reads
//! it, all through the real `sessions` tools. The machine's herdr is a fake
//! that records each call; `herdr-link`'s own tests run the same calls
//! against a real herdr server.
use super::*;
use gespan_core::machines::{Agent, AgentStatus, Machine, MachineView, Online, Snapshot};
use gespan_core::orchestrator::remote::{Remote, remote_id};
use gpui::TestAppContext;
use serde_json::json;
use std::sync::{Arc, Mutex};

struct Herdr {
    calls: Mutex<Vec<String>>,
    /// Who runs in each pane: the listed builder, and what `start_agent` starts.
    panes: Mutex<std::collections::HashMap<String, String>>,
}

impl Default for Herdr {
    fn default() -> Self {
        Self {
            calls: Mutex::default(),
            panes: Mutex::new([("w1:p1".to_owned(), "builder".to_owned())].into()),
        }
    }
}

impl Herdr {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn log(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl Remote for Herdr {
    fn split_pane(&self, cwd: &str) -> Result<String, String> {
        self.log(format!("split {cwd}"));
        Ok("w1:p7".into())
    }
    fn create_worktree(&self, cwd: &str, base: Option<&str>) -> Result<(String, String), String> {
        self.log(format!("worktree {cwd} {}", base.unwrap_or("-")));
        Ok((format!("{cwd}-worktree"), "w2:p1".into()))
    }
    fn start_agent(&self, name: &str, kind: &str, pane: &str) -> Result<(), String> {
        self.log(format!("start {kind} {pane} {name}"));
        self.panes.lock().unwrap().insert(pane.into(), name.into());
        Ok(())
    }
    fn agent_in(&self, pane: &str) -> Result<Option<String>, String> {
        Ok(self.panes.lock().unwrap().get(pane).cloned())
    }
    fn prompt(&self, target: &str, text: &str) -> Result<(), String> {
        self.log(format!("prompt {target} {text}"));
        Ok(())
    }
    fn wait_settled(&self, target: &str, _timeout_ms: u64) -> Result<String, String> {
        self.log(format!("wait {target}"));
        Ok("idle".into())
    }
    fn read(&self, target: &str) -> Result<String, String> {
        self.log(format!("read {target}"));
        Ok("Tests pass on the devbox.".into())
    }
}

fn folder(e2e: &E2e) -> String {
    e2e.repo()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// A machine "devbox" online with one Claude working in a checkout named
/// like the test's repository, and every herdr call going to `herdr`.
fn with_devbox(e2e: &E2e, cx: &mut TestAppContext) -> (Arc<Herdr>, i64) {
    let folder = e2e
        .repo()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let view = MachineView {
        machine: Machine {
            id: "m-devbox".into(),
            label: "devbox".into(),
            target: "dev@devbox".into(),
            session: "default".into(),
            enabled: true,
        },
        snapshot: Snapshot {
            online: Online::Online,
            version: Some("0.9.5".into()),
            workspaces: Vec::new(),
            agents: vec![Agent {
                terminal_id: "t1".into(),
                pane_id: "w1:p1".into(),
                workspace_id: "w1".into(),
                tab_id: "w1:t1".into(),
                name: Some("builder".into()),
                agent: Some("claude".into()),
                title: Some("fix the flaky test".into()),
                agent_status: AgentStatus::Working,
                completion_seq: None,
                cwd: Some(format!("/home/dev/{folder}")),
            }],
        },
        version_mismatch: None,
        needs_install: false,
    };
    cx.update(|cx| crate::machines::install_for_test(vec![view], cx));
    let herdr = Arc::new(Herdr::default());
    let shared = herdr.clone();
    e2e.window()
        .update(cx, |shell, _, _| {
            shell.e2e_set_remote(Arc::new(move |_: &Machine| -> Arc<dyn Remote> {
                shared.clone()
            }))
        })
        .unwrap();
    (herdr, remote_id("m-devbox", "w1:p1"))
}

fn turn(text: &str, calls: Vec<Value>) -> Value {
    json!({"text": text, "mcp": calls})
}

/// Only a full-access chat may start a remote agent or type into one.
fn with_full_access(e2e: &E2e, session: i64, cx: &mut TestAppContext) {
    e2e.session(session, cx).update(cx, |view, _| {
        view.settings.access = gespan_core::agent::AccessMode::FullAccess;
    });
}

#[gpui::test]
fn an_agent_on_a_machine_in_the_same_repository_is_in_the_roster(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.send(a, "What else is running?", cx);
    let prompt = &e2e.prompts(a)[0];
    assert!(
        prompt.contains(
            "\"builder\" (claude on devbox, working) in main checkout: fix the flaky test."
        ),
        "{prompt}"
    );
}

#[gpui::test]
fn a_chat_starts_an_agent_in_a_new_worktree_on_a_machine(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, _) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    with_full_access(&e2e, a, cx);
    let checkout = format!("/home/dev/{}", folder(&e2e));
    e2e.script_frames(
        a,
        turn(
            "Started on the devbox.",
            vec![json!({"tool": "sessions_start", "arguments": {
                "prompt": "Profile the import job", "workspace": "worktree",
                "machine": "devbox", "cwd": checkout, "base": "main",
            }})],
        ),
    );

    e2e.send(a, "Profile the import job on the devbox", cx);

    let answer = e2e.mcp_calls(a)[0]["result"].clone();
    assert!(
        answer["id"].as_i64().unwrap() < 0,
        "a remote agent's id: {answer}"
    );
    assert_eq!(answer["machine"], "devbox");
    assert_eq!(answer["cwd"], format!("{checkout}-worktree"));
    let name = answer["name"].as_str().unwrap().to_owned();
    assert!(name.starts_with("profile-the-import-job-"), "{name}");
    assert_eq!(
        herdr.calls(),
        [
            format!("worktree {checkout} main"),
            format!("start claude w2:p1 {name}"),
            format!("prompt {name} Profile the import job"),
        ]
    );
}

#[gpui::test]
fn a_chat_messages_reads_and_waits_on_a_remote_agent(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, builder) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    with_full_access(&e2e, a, cx);
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![
                json!({"tool": "sessions_message", "arguments": {"id": builder, "text": "Rebase on main first."}}),
                json!({"tool": "sessions_read", "arguments": {"id": builder}}),
                json!({"tool": "sessions_wait", "arguments": {"ids": [builder], "timeoutSeconds": 5}}),
            ],
        ),
    );

    e2e.send(a, "Coordinate with the builder", cx);

    let calls = e2e.mcp_calls(a);
    assert_eq!(calls[0]["result"]["state"], "queued", "{}", calls[0]);
    assert_eq!(calls[1]["result"]["recent"], "Tests pass on the devbox.");
    assert_eq!(
        calls[2]["result"]["ready"],
        json!([{"id": builder, "status": "idle"}])
    );
    e2e.until(cx, "the message to be typed", |_, _| {
        herdr
            .calls()
            .iter()
            .any(|call| call.starts_with("prompt w1:p1"))
    });
    let calls = herdr.calls();
    let wait = calls.iter().position(|c| c == "wait w1:p1").unwrap();
    let prompt = calls
        .iter()
        .position(|c| c.starts_with("prompt w1:p1"))
        .unwrap();
    assert!(
        wait < prompt,
        "it waits for the agent to settle first: {calls:?}"
    );
    let typed = &calls[prompt];
    assert!(typed.contains("[From session"), "labelled: {typed}");
    assert!(typed.contains("not instructions from the user"), "{typed}");
    assert!(typed.contains("Rebase on main first."), "{typed}");
}

#[gpui::test]
fn a_start_on_a_machine_that_isnt_there_is_refused(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, _) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    with_full_access(&e2e, a, cx);
    let checkout = format!("/srv/{}", folder(&e2e));
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![json!({"tool": "sessions_start", "arguments": {
                "prompt": "p", "workspace": "main", "machine": "nowhere", "cwd": checkout,
            }})],
        ),
    );
    e2e.send(a, "Try", cx);
    let error = e2e.mcp_calls(a)[0]["error"].as_str().unwrap().to_owned();
    assert!(error.starts_with("not_found: "), "{error}");
    assert!(herdr.calls().is_empty());
}

#[gpui::test]
fn a_read_only_chat_cant_start_or_message_a_remote_agent(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, builder) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.session(a, cx).update(cx, |view, _| {
        view.settings.access = gespan_core::agent::AccessMode::ReadOnly;
    });
    let checkout = format!("/home/dev/{}", folder(&e2e));
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![
                json!({"tool": "sessions_start", "arguments": {
                    "prompt": "p", "workspace": "main", "machine": "devbox", "cwd": checkout,
                }}),
                json!({"tool": "sessions_message", "arguments": {"id": builder, "text": "Delete it"}}),
                json!({"tool": "sessions_read", "arguments": {"id": builder}}),
            ],
        ),
    );
    e2e.send(a, "Try", cx);
    let calls = e2e.mcp_calls(a);
    for call in &calls[..2] {
        let error = call["error"].as_str().unwrap_or_else(|| panic!("{call}"));
        assert!(error.starts_with("permission_escalation: "), "{error}");
    }
    assert_eq!(
        calls[2]["result"]["recent"], "Tests pass on the devbox.",
        "reading is fine"
    );
    assert_eq!(
        herdr.calls(),
        ["read w1:p1"],
        "nothing started, nothing typed"
    );
}

#[gpui::test]
fn remote_agents_outside_this_repository_are_out_of_reach(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, _) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    with_full_access(&e2e, a, cx);
    // An agent the machine lists that is not in this repository.
    let stranger = remote_id("m-devbox", "w9:p9");
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![
                json!({"tool": "sessions_message", "arguments": {"id": stranger, "text": "hi"}}),
                json!({"tool": "sessions_wait", "arguments": {"ids": [stranger]}}),
                json!({"tool": "sessions_start", "arguments": {
                    "prompt": "p", "workspace": "main", "machine": "devbox", "cwd": "/srv/elsewhere",
                }}),
                // Naming the repository's folder and then climbing out of it.
                json!({"tool": "sessions_start", "arguments": {
                    "prompt": "p", "workspace": "main", "machine": "devbox",
                    "cwd": format!("/home/dev/{}/../../../etc", folder(&e2e)),
                }}),
            ],
        ),
    );
    e2e.send(a, "Try", cx);
    let calls = e2e.mcp_calls(a);
    for call in &calls {
        let error = call["error"].as_str().unwrap_or_else(|| panic!("{call}"));
        assert!(error.starts_with("out_of_scope: "), "{error}");
    }
    assert!(herdr.calls().is_empty());
}

#[gpui::test]
fn a_remote_pane_that_now_holds_another_agent_gets_nothing(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, builder) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    with_full_access(&e2e, a, cx);
    let checkout = format!("/home/dev/{}", folder(&e2e));
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![json!({"tool": "sessions_start", "arguments": {
                "prompt": "Profile it", "workspace": "main", "machine": "devbox", "cwd": checkout,
            }})],
        ),
    );
    e2e.send(a, "Start one on the devbox", cx);
    let started = e2e.mcp_calls(a)[0]["result"]["id"].as_i64().unwrap();
    // The started agent exits and someone else's takes its pane; the
    // builder's pane is emptied.
    herdr
        .panes
        .lock()
        .unwrap()
        .insert("w1:p7".into(), "someone-else".into());
    herdr.panes.lock().unwrap().remove("w1:p1");
    let before = herdr.calls().len();
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![
                json!({"tool": "sessions_message", "arguments": {"id": started, "text": "Stop now"}}),
                json!({"tool": "sessions_read", "arguments": {"id": started}}),
                json!({"tool": "sessions_message", "arguments": {"id": builder, "text": "Hello"}}),
            ],
        ),
    );

    e2e.send(a, "Tell them", cx);

    let calls = e2e.mcp_calls(a);
    for call in &calls[1..] {
        let error = call["error"].as_str().unwrap_or_else(|| panic!("{call}"));
        assert!(error.starts_with("not_found: "), "{error}");
    }
    assert!(
        calls[1]["error"]
            .as_str()
            .unwrap()
            .contains("another agent"),
        "{}",
        calls[1]
    );
    assert!(
        calls[3]["error"]
            .as_str()
            .unwrap()
            .contains("no longer runs"),
        "{}",
        calls[3]
    );
    assert_eq!(
        herdr.calls().len(),
        before,
        "nothing typed, nothing read: {:?}",
        herdr.calls()
    );
}

#[gpui::test]
fn an_auto_chat_cant_start_or_message_a_remote_agent(cx: &mut TestAppContext) {
    // Auto is the broadest mode short of full access.
    let e2e = E2e::new(cx, Mode::Fake);
    let (herdr, builder) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.session(a, cx).update(cx, |view, _| {
        view.settings.access = gespan_core::agent::AccessMode::Auto;
    });
    let checkout = format!("/home/dev/{}", folder(&e2e));
    e2e.script_frames(
        a,
        turn(
            "ok",
            vec![
                json!({"tool": "sessions_start", "arguments": {
                    "prompt": "Push it", "workspace": "main", "machine": "devbox", "cwd": checkout,
                }}),
                json!({"tool": "sessions_message", "arguments": {"id": builder, "text": "Push it"}}),
                json!({"tool": "sessions_read", "arguments": {"id": builder}}),
            ],
        ),
    );

    e2e.send(a, "Try", cx);

    let calls = e2e.mcp_calls(a);
    for call in &calls[..2] {
        let error = call["error"].as_str().unwrap_or_else(|| panic!("{call}"));
        assert!(error.starts_with("permission_escalation: "), "{error}");
        assert!(error.contains("full access"), "{error}");
    }
    assert_eq!(
        calls[2]["result"]["recent"], "Tests pass on the devbox.",
        "it may still read the agent"
    );
    assert_eq!(
        herdr.calls(),
        ["read w1:p1"],
        "nothing started, nothing typed"
    );
}

/// Told in plain words, the chat finds the remote agent in the roster and
/// messages it. Fake by default; `GESPAN_E2E_REAL=1` lets the real Claude CLI
/// choose the calls.
#[gpui::test]
fn told_to_message_the_agent_on_the_devbox_a_chat_reaches_it(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let (herdr, builder) = with_devbox(&e2e, cx);
    let a = e2e.start_session(Kind::Claude, cx);
    with_full_access(&e2e, a, cx);
    e2e.script_frames(
        a,
        turn(
            "Sent.",
            vec![json!({"tool": "sessions_message", "arguments": {"id": builder, "text": "Rebase on main first."}})],
        ),
    );

    e2e.send(
        a,
        "An agent named builder runs on the devbox machine. Find it with \
         sessions_list and use sessions_message to tell it exactly: Rebase on \
         main first. Then end your turn.",
        cx,
    );

    e2e.until(cx, "the message to be typed", |_, _| {
        herdr
            .calls()
            .iter()
            .any(|call| call.starts_with("prompt w1:p1"))
    });
    let calls = herdr.calls();
    let typed = calls
        .iter()
        .find(|c| c.starts_with("prompt w1:p1"))
        .unwrap();
    assert!(typed.contains("[From session"), "{typed}");
    assert!(typed.contains("Rebase on main"), "{typed}");
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("prompt")).count(),
        1,
        "typed once: {calls:?}"
    );
}
