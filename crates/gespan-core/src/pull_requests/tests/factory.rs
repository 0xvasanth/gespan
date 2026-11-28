//! The factory: one connection in, the right host behind the right
//! transport out. The same GitHub adapter reads the same answer over `gh`
//! and over a token.
use super::{super::*, FakeHttp, FakeRunner};
use crate::pull_requests::transport::ApiResponse;
use std::sync::Arc;

const PULLS: &str = r#"[{"number": 3, "title": "Speed up", "state": "open",
    "user": {"login": "dana"}, "head": {"ref": "fast"}, "base": {"ref": "main"},
    "updated_at": "2026-11-20T10:00:00Z"}]"#;

fn factory(runner: &FakeRunner, http: &FakeHttp) -> HostFactory {
    HostFactory::new(Arc::new(runner.clone()), Arc::new(http.clone()))
}

#[test]
fn github_over_gh_and_over_a_token_reads_the_same_pull_requests() {
    let runner = FakeRunner::printing(true, &format!("HTTP/2.0 200 OK\n\n{PULLS}"), "");
    let http = FakeHttp::answering(ApiResponse::ok(PULLS));
    let factory = factory(&runner, &http);
    let over_cli = factory
        .service(&Connection::GitHubCli { hostname: None })
        .unwrap()
        .pull_requests("acme/web", StateFilter::Open)
        .unwrap();
    let over_token = factory
        .service(&Connection::GitHubToken {
            api_url: None,
            token: "tok".into(),
        })
        .unwrap()
        .pull_requests("acme/web", StateFilter::Open)
        .unwrap();
    assert_eq!(over_cli, over_token);
    assert_eq!(over_cli[0].title, "Speed up");
    assert_eq!(runner.runs()[0].0, "gh");
    assert_eq!(
        http.url(0),
        "https://api.github.com/repos/acme/web/pulls?state=open&per_page=100&page=1"
    );
    assert_eq!(
        http.header(0, "Accept").as_deref(),
        Some("application/vnd.github+json")
    );
    assert_eq!(
        http.header(0, "Authorization").as_deref(),
        Some("Bearer tok")
    );
}

#[test]
fn each_connection_builds_its_host_behind_its_transport() {
    let runner = FakeRunner::printing(
        true,
        "HTTP/2.0 200 OK\n\n{\"username\": \"me\", \"login\": \"me\"}",
        "",
    );
    let http = FakeHttp::answering(ApiResponse::ok(
        r#"{"username": "me", "login": "me",
        "authenticatedUser": {"id": "u", "providerDisplayName": "me"}}"#,
    ));
    let factory = factory(&runner, &http);
    let cases = [
        (Connection::GitHubCli { hostname: None }, HostKind::GitHub),
        (
            Connection::GitHubToken {
                api_url: Some("https://ghe.example.com/api/v3/".into()),
                token: "t".into(),
            },
            HostKind::GitHub,
        ),
        (
            Connection::GitLabCli {
                hostname: Some("gitlab.example.com".into()),
            },
            HostKind::GitLab,
        ),
        (
            Connection::GitLabToken {
                url: "https://gitlab.example.com".into(),
                token: "t".into(),
            },
            HostKind::GitLab,
        ),
        (
            Connection::AzureDevOpsToken {
                organization_url: "https://dev.azure.com/acme".into(),
                token: "t".into(),
            },
            HostKind::AzureDevOps,
        ),
    ];
    for (connection, kind) in cases {
        assert_eq!(connection.host(), kind);
        let service = factory.service(&connection).unwrap();
        assert_eq!(service.kind(), kind, "{connection:?}");
        assert_eq!(service.current_user().unwrap(), "me", "{connection:?}");
    }
    let programs: Vec<String> = runner.runs().into_iter().map(|r| r.0).collect();
    assert_eq!(programs, ["gh", "glab"]);
    let urls: Vec<String> = (0..3).map(|i| http.url(i)).collect();
    assert_eq!(
        urls,
        [
            "https://ghe.example.com/api/v3/user",
            "https://gitlab.example.com/api/v4/user",
            "https://dev.azure.com/acme/_apis/connectionData",
        ]
    );
    // The CLI's host name reaches the CLI.
    assert!(runner.runs()[1]
        .1
        .contains(&"gitlab.example.com".to_owned()));
}

#[test]
fn a_connection_without_a_token_or_an_https_url_is_refused_before_anything_is_sent() {
    let runner = FakeRunner::default();
    let http = FakeHttp::default();
    let factory = factory(&runner, &http);
    for connection in [
        Connection::GitHubToken {
            api_url: None,
            token: "  ".into(),
        },
        Connection::GitLabToken {
            url: "gitlab.com".into(),
            token: "t".into(),
        },
        Connection::AzureDevOpsToken {
            organization_url: "https://dev.azure.com/acme".into(),
            token: String::new(),
        },
    ] {
        assert!(
            matches!(factory.service(&connection), Err(HostError::Invalid(_))),
            "{connection:?}"
        );
    }
    assert!(http.sent.lock().unwrap().is_empty());
}

#[test]
fn a_connection_never_prints_its_token() {
    let printed = format!(
        "{:?} {:?}",
        Connection::GitHubToken {
            api_url: None,
            token: "ghp_secret".into()
        },
        Connection::AzureDevOpsToken {
            organization_url: "https://dev.azure.com/a".into(),
            token: "az_secret".into()
        },
    );
    assert!(!printed.contains("secret"), "{printed}");
}
