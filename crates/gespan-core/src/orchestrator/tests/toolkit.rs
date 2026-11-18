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
fn the_toolkit_lists_two_read_only_tools() {
    assert_eq!(Toolkit.name(), crate::tool_servers::SESSIONS);
    let tools = Toolkit.tools(&grant(9_001));
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["sessions_list", "sessions_read"]);
    for tool in &tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{tool}");
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
