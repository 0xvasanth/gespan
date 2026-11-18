use crate::orchestrator::bridge::{Action, Scope, Server, UNAVAILABLE};
use crate::orchestrator::toolkit::Toolkit;
use mcp_hub::Toolkit as _;
use serde_json::json;

/// Routes are process-wide, so each test uses its own session ids.
fn grant(session: i64) -> mcp_hub::Grant {
    mcp_hub::Grant {
        session,
        workspace: "/work".into(),
        toolkits: [crate::tool_servers::SESSIONS.to_owned()].into(),
        servers: Default::default(),
    }
}

#[test]
fn the_toolkit_lists_reading_and_starting_tools() {
    assert_eq!(Toolkit.name(), crate::tool_servers::SESSIONS);
    let tools = Toolkit.tools(&grant(9_001));
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "sessions_list",
            "sessions_read",
            "sessions_start",
            "sessions_cancel",
            "sessions_message",
            "sessions_wait"
        ]
    );
    for tool in &tools {
        let reads = ["sessions_list", "sessions_read", "sessions_wait"]
            .contains(&tool["name"].as_str().unwrap());
        assert_eq!(tool["annotations"]["readOnlyHint"], reads, "{tool}");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{tool}");
    }
    let start = &tools[2]["inputSchema"];
    assert_eq!(start["required"], json!(["prompt", "workspace"]));
    let properties: Vec<&String> = start["properties"].as_object().unwrap().keys().collect();
    for field in crate::orchestrator::start::FIELDS {
        assert!(
            properties.iter().any(|p| p == field),
            "{field} is described"
        );
    }
}

#[test]
fn an_unknown_field_is_refused() {
    let error = Toolkit
        .call(
            &grant(9_002),
            "sessions_list",
            json!({"scope":"repo","x":1}),
        )
        .unwrap_err();
    assert!(error.contains("\"x\""), "{error}");
    let error = Toolkit
        .call(&grant(9_002), "sessions_read", json!({"id":3,"after":"c"}))
        .unwrap_err();
    assert!(error.contains("\"after\""), "{error}");
}

#[test]
fn a_bad_scope_is_refused() {
    for scope in [json!("mine"), json!(1)] {
        let error = Toolkit
            .call(&grant(9_003), "sessions_list", json!({"scope":scope}))
            .unwrap_err();
        assert!(error.contains("repo") && error.contains("all"), "{error}");
    }
}

#[test]
fn read_needs_an_integer_id() {
    for input in [json!({"id":"7"}), json!({"id":7.5}), json!({})] {
        let error = Toolkit
            .call(&grant(9_004), "sessions_read", input.clone())
            .unwrap_err();
        assert!(error.contains("id"), "{input}: {error}");
    }
    let error = Toolkit
        .call(&grant(9_004), "sessions_read", json!({"id":7,"before":3}))
        .unwrap_err();
    assert!(error.contains("before"), "{error}");
}

#[test]
fn arguments_must_be_an_object() {
    assert!(Toolkit
        .call(&grant(9_005), "sessions_list", json!([]))
        .is_err());
    assert!(Toolkit
        .call(&grant(9_005), "sessions_other", json!({}))
        .is_err());
}

#[test]
fn without_a_route_the_call_says_unavailable() {
    assert_eq!(
        Toolkit
            .call(&grant(9_006), "sessions_list", json!({}))
            .unwrap_err(),
        UNAVAILABLE
    );
}

#[test]
fn a_call_reaches_the_app_as_its_session() {
    let (server, rx) = Server::start();
    server.serve(42);
    server.serve(42);
    let worker = std::thread::spawn(|| Toolkit.call(&grant(42), "sessions_list", json!({})));
    let request = rx.recv_blocking().unwrap();
    assert_eq!(request.session, 42);
    assert_eq!(request.action, Action::List { scope: Scope::Repo });
    request.reply.send(Ok(json!({"sessions":[]}))).unwrap();
    assert_eq!(worker.join().unwrap().unwrap(), json!({"sessions":[]}));
}

#[test]
fn a_read_carries_its_id_and_cursor() {
    let (server, rx) = Server::start();
    server.serve(43);
    let worker = std::thread::spawn(|| {
        Toolkit.call(&grant(43), "sessions_read", json!({"id":7,"before":"c-1"}))
    });
    let request = rx.recv_blocking().unwrap();
    assert_eq!(request.session, 43);
    assert_eq!(
        request.action,
        Action::Read {
            id: 7,
            before: Some("c-1".into())
        }
    );
    request
        .reply
        .send(Err("That session isn't readable here.".into()))
        .unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap_err(),
        "That session isn't readable here."
    );
    let worker =
        std::thread::spawn(|| Toolkit.call(&grant(43), "sessions_list", json!({"scope":"all"})));
    let request = rx.recv_blocking().unwrap();
    assert_eq!(request.action, Action::List { scope: Scope::All });
    request.reply.send(Ok(json!({}))).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn dropping_the_server_ends_its_routes() {
    let (server, rx) = Server::start();
    server.serve(44);
    drop(server);
    assert_eq!(
        Toolkit
            .call(&grant(44), "sessions_list", json!({}))
            .unwrap_err(),
        UNAVAILABLE
    );
    drop(rx);
}

#[test]
fn a_closed_receiver_says_unavailable() {
    let (server, rx) = Server::start();
    server.serve(45);
    drop(rx);
    assert_eq!(
        Toolkit
            .call(&grant(45), "sessions_list", json!({}))
            .unwrap_err(),
        UNAVAILABLE
    );
}

#[test]
fn a_start_reaches_the_app_parsed() {
    let (server, rx) = Server::start();
    server.serve(46);
    let worker = std::thread::spawn(|| {
        Toolkit.call(
            &grant(46),
            "sessions_start",
            json!({"prompt":"Write notes","workspace":"worktree","requestId":"r1"}),
        )
    });
    let request = rx.recv_blocking().unwrap();
    assert_eq!(request.session, 46);
    let Action::Start { input } = &request.action else {
        panic!("{:?}", request.action)
    };
    assert_eq!(input.prompt, "Write notes");
    assert_eq!(input.request_id.as_deref(), Some("r1"));
    request.reply.send(Ok(json!({"id": 7}))).unwrap();
    assert_eq!(worker.join().unwrap().unwrap(), json!({"id": 7}));
}

#[test]
fn a_bad_start_is_refused_before_the_app_sees_it() {
    // No route: a call that reached dispatch would say unavailable instead.
    let error = Toolkit
        .call(
            &grant(9_007),
            "sessions_start",
            json!({"prompt":"p","workspace":"main","x":1}),
        )
        .unwrap_err();
    assert!(error.starts_with("invalid_input: "), "{error}");
    let error = Toolkit
        .call(&grant(9_007), "sessions_start", json!({"workspace":"main"}))
        .unwrap_err();
    assert!(error.starts_with("invalid_input: "), "{error}");
}

#[test]
fn a_cancel_carries_its_target() {
    let (server, rx) = Server::start();
    server.serve(47);
    let worker =
        std::thread::spawn(|| Toolkit.call(&grant(47), "sessions_cancel", json!({"id": 9})));
    let request = rx.recv_blocking().unwrap();
    assert_eq!(request.action, Action::Cancel { id: 9 });
    request.reply.send(Ok(json!({"stopped":[9]}))).unwrap();
    worker.join().unwrap().unwrap();
    for input in [json!({}), json!({"id":"9"}), json!({"id":9,"all":true})] {
        let error = Toolkit
            .call(&grant(9_008), "sessions_cancel", input.clone())
            .unwrap_err();
        assert!(error.starts_with("invalid_input: "), "{input}: {error}");
    }
}

#[test]
fn a_message_carries_its_target_text_and_request() {
    let (server, rx) = Server::start();
    server.serve(48);
    let worker = std::thread::spawn(|| {
        Toolkit.call(
            &grant(48),
            "sessions_message",
            json!({"id": 5, "text": "Rebase on main", "requestId": "m1"}),
        )
    });
    let request = rx.recv_blocking().unwrap();
    assert_eq!(
        request.action,
        Action::Message {
            id: 5,
            text: "Rebase on main".into(),
            request_id: Some("m1".into())
        }
    );
    request.reply.send(Ok(json!({"state": "queued"}))).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn a_bad_message_is_refused_before_the_app_sees_it() {
    for input in [
        json!({"text": "hi"}),
        json!({"id": 5}),
        json!({"id": 5, "text": "   "}),
        json!({"id": 5, "text": "x".repeat(32_001)}),
        json!({"id": 5, "text": "hi", "cc": [6]}),
        json!({"id": 5, "text": "hi", "requestId": ""}),
    ] {
        let error = Toolkit
            .call(&grant(9_009), "sessions_message", input.clone())
            .unwrap_err();
        assert!(error.starts_with("invalid_input: "), "{input}: {error}");
    }
}

#[test]
fn a_wait_is_bounded() {
    let (server, rx) = Server::start();
    server.serve(49);
    let worker = std::thread::spawn(|| {
        Toolkit.call(
            &grant(49),
            "sessions_wait",
            json!({"ids": [3, 4], "timeoutSeconds": 2}),
        )
    });
    let request = rx.recv_blocking().unwrap();
    assert_eq!(
        request.action,
        Action::Wait {
            ids: vec![3, 4],
            timeout_seconds: 2
        }
    );
    request.reply.send(Ok(json!({"timedOut": true}))).unwrap();
    worker.join().unwrap().unwrap();
    let worker =
        std::thread::spawn(|| Toolkit.call(&grant(49), "sessions_wait", json!({"ids": [3]})));
    let request = rx.recv_blocking().unwrap();
    assert_eq!(
        request.action,
        Action::Wait {
            ids: vec![3],
            timeout_seconds: 20
        },
        "20 seconds by default"
    );
    request.reply.send(Ok(json!({}))).unwrap();
    worker.join().unwrap().unwrap();
    for input in [
        json!({"ids": []}),
        json!({"ids": [1], "timeoutSeconds": 26}),
        json!({"ids": [1], "timeoutSeconds": -1}),
        json!({"ids": ["1"]}),
        json!({"ids": (0..21).collect::<Vec<_>>()}),
        json!({"ids": [1], "forever": true}),
    ] {
        let error = Toolkit
            .call(&grant(9_010), "sessions_wait", input.clone())
            .unwrap_err();
        assert!(error.starts_with("invalid_input: "), "{input}: {error}");
    }
}
