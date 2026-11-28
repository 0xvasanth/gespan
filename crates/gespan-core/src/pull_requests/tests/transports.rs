//! The two ways of connecting: a host's CLI and a token over HTTPS. Both
//! must send the same request and read errors the same way.
use super::{super::*, FakeHttp, FakeRunner};
use crate::pull_requests::transport::{
    check, parse_included, ApiRequest, ApiResponse, Auth, CliTransport, Method, TokenTransport,
    Transport,
};
use serde_json::json;
use std::sync::Arc;

fn status(status: u16, headers: &[(&str, &str)], body: &str) -> ApiResponse {
    ApiResponse {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.into(),
    }
}

#[test]
fn every_host_error_reads_the_same_whatever_carried_it() {
    assert!(check(status(204, &[], "")).is_ok());
    assert_eq!(
        check(status(401, &[], r#"{"message":"Bad credentials"}"#)),
        Err(HostError::Unauthorized("Bad credentials".into()))
    );
    assert_eq!(
        check(status(
            403,
            &[("X-RateLimit-Remaining", "0")],
            r#"{"message":"API rate limit exceeded"}"#
        )),
        Err(HostError::RateLimited {
            retry_after_secs: None
        })
    );
    assert_eq!(
        check(status(429, &[("Retry-After", "30")], "")),
        Err(HostError::RateLimited {
            retry_after_secs: Some(30)
        })
    );
    assert_eq!(
        check(status(403, &[], r#"{"message":"Resource not accessible"}"#)),
        Err(HostError::Unauthorized("Resource not accessible".into()))
    );
    // GitLab says `error`, Azure DevOps says `message`.
    assert_eq!(
        check(status(404, &[], r#"{"error":"404 Project Not Found"}"#)),
        Err(HostError::NotFound("404 Project Not Found".into()))
    );
    assert_eq!(
        check(status(422, &[], r#"{"message":"Validation Failed"}"#)),
        Err(HostError::Http {
            status: 422,
            message: "Validation Failed".into()
        })
    );
    assert_eq!(
        check(status(502, &[], "<html>Bad gateway</html>")),
        Err(HostError::Http {
            status: 502,
            message: "<html>Bad gateway</html>".into()
        })
    );
}

#[test]
fn the_cli_runs_the_hosts_api_command_with_the_body_on_stdin() {
    let runner = FakeRunner::printing(
        true,
        "HTTP/2.0 201 Created\r\nContent-Type: application/json\r\n\r\n{\"id\":1}",
        "",
    );
    let gh = CliTransport::new(
        "gh",
        Some("ghe.example.com".into()),
        Arc::new(runner.clone()),
    );
    let response = gh
        .send(&ApiRequest::with_body(
            Method::Post,
            "repos/acme/web/issues/7/comments",
            json!({"body": "Hi"}),
        ))
        .unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.header("content-type"), Some("application/json"));
    assert_eq!(
        response.json::<serde_json::Value>().unwrap(),
        json!({"id": 1})
    );
    let runs = runner.runs();
    assert_eq!(runs.len(), 1);
    let (program, args, stdin) = &runs[0];
    assert_eq!(program, "gh");
    assert_eq!(
        args,
        &[
            "api",
            "--method",
            "POST",
            "--include",
            "--hostname",
            "ghe.example.com",
            "repos/acme/web/issues/7/comments",
            "--input",
            "-"
        ]
    );
    assert_eq!(stdin.as_deref(), Some(r#"{"body":"Hi"}"#));

    let glab = CliTransport::new("glab", None, Arc::new(runner.clone()));
    glab.send(&ApiRequest::get("projects/acme%2Fweb")).unwrap();
    let (program, args, stdin) = &runner.runs()[1];
    assert_eq!(program, "glab");
    assert_eq!(
        args,
        &["api", "--method", "GET", "--include", "projects/acme%2Fweb"]
    );
    assert_eq!(stdin, &None, "a GET sends no body");
}

#[test]
fn a_cli_error_answer_is_read_from_its_status_and_a_missing_login_from_stderr() {
    // gh exits with an error for a 404 but still prints the answer.
    let runner = FakeRunner::printing(
        false,
        "HTTP/2.0 404 Not Found\nContent-Type: application/json\n\n{\"message\":\"Not Found\"}",
        "gh: Not Found (HTTP 404)",
    );
    let gh = CliTransport::new("gh", None, Arc::new(runner));
    assert_eq!(
        gh.send(&ApiRequest::get("repos/acme/gone")),
        Err(HostError::NotFound("Not Found".into()))
    );
    let runner = FakeRunner::printing(
        false,
        "",
        "To get started with GitHub CLI, please run:  gh auth login",
    );
    let gh = CliTransport::new("gh", None, Arc::new(runner));
    assert!(matches!(
        gh.send(&ApiRequest::get("user")),
        Err(HostError::Unauthorized(_))
    ));
    let runner = FakeRunner::printing(false, "", "");
    let glab = CliTransport::new("glab", None, Arc::new(runner));
    assert_eq!(
        glab.send(&ApiRequest::get("user")),
        Err(HostError::Transport("glab printed nothing".into()))
    );
}

#[test]
fn an_included_answer_keeps_blank_lines_inside_its_body() {
    let response =
        parse_included("HTTP/1.1 200 OK\r\nLink: <x>; rel=\"next\"\r\n\r\n{\"a\":\"one\n\ntwo\"}")
            .unwrap();
    assert_eq!(response.status, 200);
    assert!(response.has_next_link());
    assert_eq!(response.body, "{\"a\":\"one\n\ntwo\"}");
    assert_eq!(parse_included("not an answer"), None);
}

#[test]
fn each_token_signs_requests_the_way_its_host_expects() {
    let http = FakeHttp::answering(ApiResponse::ok("{}"));
    let send = |auth: Auth, base: &str| {
        TokenTransport::new(base, auth, Arc::new(http.clone()))
            .send(&ApiRequest::get("/user"))
            .unwrap();
    };
    send(Auth::Bearer("gh-tok".into()), "https://api.github.com/");
    send(
        Auth::PrivateToken("gl-tok".into()),
        "https://gitlab.example.com/api/v4",
    );
    send(
        Auth::BasicToken("az-tok".into()),
        "https://dev.azure.com/acme",
    );
    assert_eq!(http.url(0), "https://api.github.com/user");
    assert_eq!(
        http.header(0, "Authorization").as_deref(),
        Some("Bearer gh-tok")
    );
    assert_eq!(http.url(1), "https://gitlab.example.com/api/v4/user");
    assert_eq!(http.header(1, "PRIVATE-TOKEN").as_deref(), Some("gl-tok"));
    assert_eq!(http.header(1, "Authorization"), None);
    // Basic with an empty user: base64(":az-tok").
    assert_eq!(http.url(2), "https://dev.azure.com/acme/user");
    assert_eq!(
        http.header(2, "Authorization").as_deref(),
        Some("Basic OmF6LXRvaw==")
    );
    assert_eq!(http.header(0, "Content-Type"), None, "a GET has no body");
    assert_eq!(format!("{:?}", Auth::Bearer("secret".into())), "Bearer(…)");
}

#[test]
fn a_token_request_with_a_body_says_it_is_json_and_maps_errors() {
    let http = FakeHttp::answering(status(401, &[], r#"{"message":"401 Unauthorized"}"#));
    let transport = TokenTransport::new(
        "https://gitlab.com/api/v4",
        Auth::PrivateToken("t".into()),
        Arc::new(http.clone()),
    )
    .with_header("Accept", "application/vnd.test+json");
    let error = transport
        .send(&ApiRequest::with_body(
            Method::Put,
            "projects/1",
            json!({"a": 1}),
        ))
        .unwrap_err();
    assert_eq!(error, HostError::Unauthorized("401 Unauthorized".into()));
    assert_eq!(
        http.header(0, "Content-Type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        http.header(0, "Accept").as_deref(),
        Some("application/vnd.test+json")
    );
    let sent = http.sent.lock().unwrap();
    assert_eq!(sent[0].0, Method::Put);
    assert_eq!(sent[0].3.as_deref(), Some(r#"{"a":1}"#));
}
