//! "Do X in a new worktree": a chat's agent calls `sessions_start` over the
//! real MCP endpoint, the app starts a child session that runs on its own,
//! and the child's result comes back to the chat as a new turn. Driven end to
//! end, from the agent's tool call to the child's process and back.
use super::*;
use gpui::TestAppContext;
use serde_json::json;

fn start_call(arguments: Value) -> Value {
    json!({"tool": "sessions_start", "arguments": arguments})
}

/// A turn whose agent makes these calls to Gespan's tools, then says `text`.
fn calling(text: &str, calls: Vec<Value>) -> Value {
    json!({"text": text, "mcp": calls})
}

/// The id `sessions_start` answered with in the `n`th call `id` made.
fn started(e2e: &E2e, id: i64, n: usize) -> i64 {
    let calls = e2e.mcp_calls(id);
    let call = calls
        .get(n)
        .unwrap_or_else(|| panic!("call {n} of {calls:?}"));
    call["result"]["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("sessions_start failed: {call}"))
}

/// The error a call answered with.
fn error_of(e2e: &E2e, id: i64, n: usize) -> String {
    let calls = e2e.mcp_calls(id);
    calls[n]["error"]
        .as_str()
        .unwrap_or_else(|| panic!("call {n} succeeded: {}", calls[n]))
        .to_owned()
}

fn idle_after(e2e: &E2e, id: i64, prompts: usize, cx: &mut TestAppContext) {
    e2e.until(cx, &format!("session {id}'s turn {prompts}"), |e2e, cx| {
        e2e.prompts(id).len() >= prompts && !e2e.busy(id, cx)
    });
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap()
}

#[gpui::test]
fn a_child_in_a_new_worktree_runs_and_reports_back(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "Started a helper.",
            vec![start_call(json!({
                "prompt": "Write the release notes\nKeep them short.",
                "workspace": "worktree",
            }))],
        ),
    );
    e2e.script_next_child(json!(["Release notes written."]));

    e2e.send(a, "Write the release notes in a new worktree", cx);

    let b = started(&e2e, a, 0);
    let answer = &e2e.mcp_calls(a)[0]["result"];
    assert_eq!(answer["workspace"], "worktree", "{answer}");
    assert_eq!(answer["depth"], 1);
    assert_eq!(answer["title"], "Write the release notes");
    assert_eq!(answer["access"], "supervised", "the parent's mode");
    // The child ran its own turn, in a worktree of the same repository.
    idle_after(&e2e, b, 1, cx);
    assert!(
        e2e.prompts(b)[0].ends_with("Write the release notes\nKeep them short."),
        "{:?}",
        e2e.prompts(b)
    );
    let cwd = PathBuf::from(e2e.launches(b)[0]["cwd"].as_str().unwrap());
    assert_ne!(canonical(&cwd), canonical(e2e.repo()));
    assert_eq!(
        gespan_core::orchestrator::repo::repo_key(&cwd),
        gespan_core::orchestrator::repo::repo_key(e2e.repo()),
        "a worktree of the chat's repository"
    );
    let link = e2e
        .window()
        .update(cx, |shell, _, _| shell.e2e_link(b))
        .unwrap()
        .expect("linked to its parent");
    assert_eq!((link.parent, link.root, link.depth), (a, a, 1));
    e2e.until(cx, "the child's row under its parent", |e2e, cx| {
        e2e.window()
            .update(cx, |shell, _, cx| shell.e2e_child_rows(cx))
            .unwrap()
            .contains(&(b, 1))
    });
    // Its result arrives in the chat as a new turn, labelled with the child.
    idle_after(&e2e, a, 2, cx);
    let report = &e2e.prompts(a)[1];
    assert!(
        report.contains("[From session \"Write the release notes\""),
        "{report}"
    );
    assert!(report.contains("finished"), "{report}");
    assert!(report.contains("Release notes written."), "{report}");
    // The roster on that turn says who started the child.
    let a_title = e2e
        .session(a, cx)
        .read_with(cx, |view, _| view.summary.title.clone());
    assert!(
        report.contains(&format!("started by \"{a_title}\"")),
        "{report}"
    );
    assert_eq!(e2e.prompts(a).len(), 2, "one report, sent once");
}

#[gpui::test]
fn a_child_in_the_main_checkout_works_in_the_repository(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![start_call(json!({"prompt": "Tidy", "workspace": "main"}))],
        ),
    );
    e2e.send(a, "Tidy in the main repo", cx);
    let b = started(&e2e, a, 0);
    assert_eq!(e2e.mcp_calls(a)[0]["result"]["workspace"], "main");
    idle_after(&e2e, b, 1, cx);
    let cwd = PathBuf::from(e2e.launches(b)[0]["cwd"].as_str().unwrap());
    assert_eq!(canonical(&cwd), canonical(e2e.repo()));
}

#[gpui::test]
fn grandchildren_are_the_deepest(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let deeper = |prompt: &str| {
        json!([calling(
            "ok",
            vec![start_call(
                json!({"prompt": prompt, "workspace": "main", "wake": "never"})
            )]
        )])
    };
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![start_call(
                json!({"prompt": "level one", "workspace": "main", "wake": "never"}),
            )],
        ),
    );
    e2e.script_next_child(deeper("level two"));
    e2e.script_next_child(deeper("level three"));

    e2e.send(a, "Go deep", cx);

    let b = started(&e2e, a, 0);
    idle_after(&e2e, b, 1, cx);
    let c = started(&e2e, b, 0);
    idle_after(&e2e, c, 1, cx);
    let refused = error_of(&e2e, c, 0);
    assert!(refused.starts_with("depth_limit: "), "{refused}");
    let mut link = |id| {
        e2e.window()
            .update(cx, |shell, _, _| shell.e2e_link(id))
            .unwrap()
            .unwrap()
    };
    assert_eq!((link(c).parent, link(c).root, link(c).depth), (b, a, 2));
}

#[gpui::test]
fn five_running_children_are_the_most(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let start = |n: usize| {
        start_call(json!({"prompt": format!("job {n}"), "workspace": "main", "wake": "never"}))
    };
    e2e.script_frames(a, calling("ok", (0..6).map(start).collect()));
    for _ in 0..6 {
        e2e.script_next_child(json!([{"text": "done", "wait": true}]));
    }

    e2e.send(a, "Start six jobs", cx);

    let children: Vec<i64> = (0..5).map(|n| started(&e2e, a, n)).collect();
    let refused = error_of(&e2e, a, 5);
    assert!(refused.starts_with("fanout_limit: "), "{refused}");
    // One finishing makes room for one more.
    e2e.release(children[0]);
    idle_after(&e2e, children[0], 1, cx);
    e2e.script_frames(a, calling("ok", vec![start(7)]));
    e2e.send(a, "One more", cx);
    let more = started(&e2e, a, 6);
    for id in children.iter().skip(1).chain([&more]) {
        e2e.release(*id);
    }
}

#[gpui::test]
fn a_child_never_gets_broader_permissions(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![
                start_call(json!({"prompt": "p", "workspace": "main", "access": "full-access"})),
                start_call(json!({"prompt": "look", "workspace": "main", "access": "read-only", "wake": "never"})),
            ],
        ),
    );
    let before = e2e.all_sessions(cx).len();
    e2e.send(a, "Start one", cx);
    let refused = error_of(&e2e, a, 0);
    assert!(refused.starts_with("permission_escalation: "), "{refused}");
    let b = started(&e2e, a, 1);
    assert_eq!(e2e.mcp_calls(a)[1]["result"]["access"], "read-only");
    assert_eq!(
        e2e.all_sessions(cx).len(),
        before + 1,
        "the refused one made nothing"
    );
    idle_after(&e2e, b, 1, cx);
    let argv = e2e.launches(b)[0]["argv"].to_string();
    assert!(
        argv.contains("\"plan\""),
        "read-only runs in plan mode: {argv}"
    );
}

#[gpui::test]
fn the_same_request_starts_one_session(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    let once = start_call(json!({
        "prompt": "Once", "workspace": "main", "requestId": "only-once", "wake": "never",
    }));
    e2e.script_frames(a, calling("ok", vec![once.clone(), once.clone()]));
    e2e.script_frames(a, calling("ok", vec![once]));
    let before = e2e.all_sessions(cx).len();

    e2e.send(a, "Start it", cx);
    e2e.send(a, "Start it again", cx);

    let b = started(&e2e, a, 0);
    assert_eq!(started(&e2e, a, 1), b);
    assert_eq!(started(&e2e, a, 2), b, "a later turn's retry too");
    assert_eq!(e2e.all_sessions(cx).len(), before + 1);
    idle_after(&e2e, b, 1, cx);
    assert_eq!(e2e.prompts(b).len(), 1, "its prompt ran once");
}

#[gpui::test]
fn bad_input_starts_nothing(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![
                start_call(json!({"prompt": "p", "workspace": "main", "colour": "red"})),
                start_call(json!({"prompt": "p", "workspace": "nowhere"})),
                start_call(json!({"prompt": "p", "workspace": format!(
                    "existing:{}",
                    std::env::temp_dir().join("definitely-not-a-worktree").display()
                )})),
            ],
        ),
    );
    let before = e2e.all_sessions(cx).len();
    e2e.send(a, "Start badly", cx);
    assert!(error_of(&e2e, a, 0).starts_with("invalid_input: "));
    assert!(error_of(&e2e, a, 1).starts_with("invalid_input: "));
    let missing = error_of(&e2e, a, 2);
    assert!(
        missing.contains("unavailable"),
        "not a worktree of this project: {missing}"
    );
    assert_eq!(e2e.all_sessions(cx).len(), before);
}

#[gpui::test]
fn cancel_stops_a_child_and_its_children(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![start_call(json!({"prompt": "b", "workspace": "main"}))],
        ),
    );
    // B starts C, then both keep working until stopped.
    e2e.script_next_child(json!([{
        "text": "b done", "wait": true,
        "mcp": [start_call(json!({"prompt": "c", "workspace": "main"}))],
    }]));
    e2e.script_next_child(json!([{"text": "c done", "wait": true}]));
    e2e.send(a, "Start b", cx);
    let b = started(&e2e, a, 0);
    e2e.until(cx, "B to start C", |e2e, _| !e2e.mcp_calls(b).is_empty());
    let c = started(&e2e, b, 0);
    e2e.until(cx, "C to be working", |e2e, cx| e2e.busy(c, cx));
    // Someone else's session is out of reach, and so is the caller itself.
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![
                json!({"tool": "sessions_cancel", "arguments": {"id": a}}),
                json!({"tool": "sessions_cancel", "arguments": {"id": b}}),
            ],
        ),
    );

    e2e.send(a, "Stop b", cx);

    assert!(error_of(&e2e, a, 1).starts_with("self_target: "));
    assert_eq!(
        e2e.mcp_calls(a)[2]["result"]["stopped"],
        json!([c, b]),
        "leaves first"
    );
    e2e.until(cx, "B and C to stop", |e2e, cx| {
        !e2e.busy(b, cx) && !e2e.busy(c, cx)
    });
    // Stopped by its parent, so nothing is reported back with wake settled.
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(e2e.prompts(a).len(), 2, "no report: {:?}", e2e.prompts(a));
}

#[gpui::test]
fn a_session_can_only_cancel_what_it_started(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![start_call(
                json!({"prompt": "b", "workspace": "main", "wake": "never"}),
            )],
        ),
    );
    e2e.script_next_child(json!([calling(
        "ok",
        vec![json!({"tool": "sessions_cancel", "arguments": {"id": 0}})]
    )]));
    e2e.send(a, "Start b", cx);
    let b = started(&e2e, a, 0);
    idle_after(&e2e, b, 1, cx);
    assert!(error_of(&e2e, b, 0).starts_with("out_of_scope: "));
    // A chat nobody started can't reach a sibling chat either.
    let other = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        other,
        calling(
            "ok",
            vec![json!({"tool": "sessions_cancel", "arguments": {"id": b}})],
        ),
    );
    e2e.send(other, "Stop b", cx);
    assert!(error_of(&e2e, other, 0).starts_with("out_of_scope: "));
}

#[gpui::test]
fn wake_decides_which_stops_are_reported(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![
                start_call(json!({"prompt": "quiet", "workspace": "main", "wake": "never"})),
                start_call(json!({"prompt": "breaks", "workspace": "main", "wake": "settled"})),
            ],
        ),
    );
    e2e.script_next_child(json!(["quiet done"]));
    e2e.script_next_child(json!([{"text": "", "error": true}]));

    e2e.send(a, "Start two", cx);

    let quiet = started(&e2e, a, 0);
    let breaks = started(&e2e, a, 1);
    idle_after(&e2e, quiet, 1, cx);
    idle_after(&e2e, breaks, 1, cx);
    idle_after(&e2e, a, 2, cx);
    let report = &e2e.prompts(a)[1];
    assert!(
        report.contains("\"breaks\"") && report.contains("failed"),
        "{report}"
    );
    assert!(
        !report.contains("[From session \"quiet\""),
        "wake never sends nothing: {report}"
    );
}

#[gpui::test]
fn reports_that_arrive_while_the_chat_works_come_as_one_turn(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    // A starts two quick children, then keeps working until released.
    e2e.script_frames(
        a,
        json!({
            "text": "ok", "wait": true,
            "mcp": [
                start_call(json!({"prompt": "first", "workspace": "main"})),
                start_call(json!({"prompt": "second", "workspace": "main"})),
            ],
        }),
    );
    e2e.script_next_child(json!(["first result"]));
    e2e.script_next_child(json!(["second result"]));
    let view = e2e.session(a, cx);
    view.update(cx, |view, cx| {
        view.input
            .update(cx, |input, cx| input.set_text("Start two".into(), cx));
        view.send_typed(cx);
    });
    e2e.until(cx, "both children", |e2e, _| e2e.mcp_calls(a).len() == 2);
    let (first, second) = (started(&e2e, a, 0), started(&e2e, a, 1));
    idle_after(&e2e, first, 1, cx);
    idle_after(&e2e, second, 1, cx);
    assert_eq!(e2e.prompts(a).len(), 1, "A is still working");

    e2e.release(a);

    idle_after(&e2e, a, 2, cx);
    let report = &e2e.prompts(a)[1];
    assert!(
        report.contains("first result") && report.contains("second result"),
        "{report}"
    );
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(e2e.prompts(a).len(), 2, "one turn for both");
}

fn watched(e2e: &E2e, id: i64, cx: &mut TestAppContext) -> Vec<i64> {
    use gespan_core::watching::Source;
    e2e.session(id, cx).read_with(cx, |view, _| {
        view.watches()
            .into_iter()
            .filter_map(|watch| match watch.source {
                Source::Session(child) => Some(child),
                _ => None,
            })
            .collect()
    })
}

#[gpui::test]
fn the_watching_chip_lists_running_children_until_the_user_stops_waiting(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![
                start_call(json!({"prompt": "long job", "workspace": "main"})),
                start_call(json!({"prompt": "silent", "workspace": "main", "wake": "never"})),
            ],
        ),
    );
    e2e.script_next_child(json!([{"text": "long done", "wait": true}]));
    e2e.script_next_child(json!([{"text": "silent done", "wait": true}]));
    e2e.send(a, "Start a long job", cx);
    let (b, silent) = (started(&e2e, a, 0), started(&e2e, a, 1));
    e2e.until(cx, "B on A's chip", |e2e, cx| watched(e2e, a, cx) == [b]);
    assert!(
        !watched(&e2e, a, cx).contains(&silent),
        "wake never isn't awaited"
    );

    // The user stops waiting from the chip: B runs on, but won't report.
    e2e.session(a, cx).update(cx, |_, cx| {
        cx.emit(crate::conversation::StopWatching(b));
    });
    e2e.until(cx, "the chip to empty", |e2e, cx| {
        watched(e2e, a, cx).is_empty()
    });
    assert!(e2e.busy(b, cx), "still running");
    e2e.release(b);
    e2e.release(silent);
    idle_after(&e2e, b, 1, cx);
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(e2e.prompts(a).len(), 1, "no report after stop waiting");
}

#[gpui::test]
fn a_report_saved_before_a_restart_is_delivered_once(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.send(a, "hello", cx);
    // What a window that closed with a report still undelivered left behind.
    let helper = e2e.start_session(Kind::Claude, cx);
    {
        use gespan_core::orchestrator::mailbox::{Kind as MailKind, Mail, State};
        let store = e2e.store(cx);
        let store = locked(&store).unwrap();
        assert!(
            store
                .enqueue_mail(&Mail {
                    id: "report:earlier".into(),
                    to: a,
                    from: Some(helper),
                    from_title: "Earlier helper".into(),
                    kind: MailKind::Report,
                    text: "It finished.\n\nDid the earlier thing.".into(),
                    hops: 1,
                    created_ms: 1,
                    state: State::Queued,
                    held: None,
                    released: false,
                    sender_access: None,
                })
                .unwrap()
        );
    }

    e2e.window()
        .update(cx, |shell, _, cx| shell.e2e_resume_wake_reports(cx))
        .unwrap();

    idle_after(&e2e, a, 2, cx);
    let report = &e2e.prompts(a)[1];
    assert!(
        report.contains(&format!("[From session \"Earlier helper\" (id {helper})]")),
        "{report}"
    );
    assert!(report.contains("Did the earlier thing."), "{report}");
    // Delivered means done: starting again sends nothing more.
    e2e.window()
        .update(cx, |shell, _, cx| shell.e2e_resume_wake_reports(cx))
        .unwrap();
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(e2e.prompts(a).len(), 2);
    let store = e2e.store(cx);
    assert!(
        locked(&store)
            .unwrap()
            .pending_orch_effects(None)
            .unwrap()
            .is_empty()
    );
}

#[gpui::test]
fn deleting_a_parent_leaves_its_child_a_chat_of_its_own(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![start_call(
                json!({"prompt": "orphan", "workspace": "main", "wake": "never"}),
            )],
        ),
    );
    e2e.send(a, "Start one", cx);
    let b = started(&e2e, a, 0);
    idle_after(&e2e, b, 1, cx);
    // Delete needs A's pane closed, as the sidebar's Delete arranges.
    e2e.window()
        .update(cx, |shell, window, cx| shell.e2e_delete(a, window, cx))
        .unwrap();
    e2e.until(cx, "A to be deleted", |e2e, cx| {
        e2e.window()
            .update(cx, |shell, _, _| shell.e2e_link(b))
            .unwrap()
            .is_none()
    });
    let store = e2e.store(cx);
    assert!(
        locked(&store)
            .unwrap()
            .session_links()
            .unwrap()
            .link(b)
            .is_none()
    );
}

#[gpui::test]
fn the_sidebar_stops_what_a_chat_started(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![start_call(json!({"prompt": "b", "workspace": "main"}))],
        ),
    );
    e2e.script_next_child(json!([{
        "text": "b done", "wait": true,
        "mcp": [start_call(json!({"prompt": "c", "workspace": "main"}))],
    }]));
    e2e.script_next_child(json!([{"text": "c done", "wait": true}]));
    let menu = |e2e: &E2e, cx: &mut TestAppContext| {
        e2e.window()
            .update(cx, |shell, _, cx| shell.e2e_menu(a, cx))
            .unwrap()
    };
    assert!(
        !menu(&e2e, cx).iter().any(|(id, _)| id == "stop-started"),
        "nothing to stop yet"
    );
    e2e.send(a, "Start b", cx);
    let b = started(&e2e, a, 0);
    e2e.until(cx, "B to start C", |e2e, _| !e2e.mcp_calls(b).is_empty());
    let c = started(&e2e, b, 0);
    e2e.until(cx, "C to work", |e2e, cx| e2e.busy(c, cx));
    let item = menu(&e2e, cx)
        .into_iter()
        .find(|(id, _)| id == "stop-started")
        .expect("offered while they run");
    assert_eq!(item.1, "Stop the 2 sessions it started");

    e2e.window()
        .update(cx, |shell, window, cx| {
            shell.e2e_run_menu(a, "stop-started", window, cx)
        })
        .unwrap();

    e2e.until(cx, "B and C to stop", |e2e, cx| {
        !e2e.busy(b, cx) && !e2e.busy(c, cx)
    });
    assert!(!menu(&e2e, cx).iter().any(|(id, _)| id == "stop-started"));
}

#[gpui::test]
fn a_plan_turn_starts_only_read_only_sessions(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::Fake);
    let a = e2e.start_session(Kind::Claude, cx);
    e2e.session(a, cx)
        .update(cx, |view, _| view.settings.plan = true);
    e2e.script_frames(
        a,
        calling(
            "ok",
            vec![
                start_call(json!({"prompt": "look", "workspace": "main", "wake": "never"})),
                start_call(json!({"prompt": "edit", "workspace": "main", "access": "supervised"})),
            ],
        ),
    );
    e2e.send(a, "Plan it", cx);
    assert_eq!(
        e2e.mcp_calls(a)[0]["result"]["access"],
        "read-only",
        "a plan turn's child is read-only though the composer says supervised"
    );
    let refused = error_of(&e2e, a, 1);
    assert!(refused.starts_with("permission_escalation: "), "{refused}");
}
