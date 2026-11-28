//! The factory, and what each host does with a call it can or can't make.
//! GitHub's calls are answered by a fake remote host, so nothing reaches
//! the network; GitLab and Azure DevOps run against an empty data folder,
//! where they have no connection.
use super::{HostFactory, GIT_HOSTS};
use crate::files::remote::TEST_HOST;
use crate::inbox::{Filters, Item, Service, Verdict};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

/// Its own machine: remote logins are cached per machine across tests.
const REMOTE: &str = "remote://pr-hosts/home/me/web";

fn factory(data_dir: &Path) -> HostFactory {
    HostFactory::new(
        data_dir.to_path_buf(),
        PathBuf::from("/nonexistent/checkout"),
    )
}

fn item(provider: &str, kind: &str) -> Item {
    Item {
        provider: provider.into(),
        kind: kind.into(),
        repo: "acme/web".into(),
        number: 42,
        project_path: REMOTE.into(),
        ..Default::default()
    }
}

type Calls = Rc<RefCell<Vec<(String, Value)>>>;

/// A remote machine whose host answers every `git_github_*` command, and
/// what it was asked.
fn remote_host(answer: impl Fn(&str) -> Value + 'static) -> Calls {
    let calls = Calls::default();
    let seen = calls.clone();
    TEST_HOST.set(Some(Box::new(move |environment, params| {
        assert_eq!(environment, "pr-hosts");
        let command = params["command"].as_str().unwrap().to_owned();
        seen.borrow_mut()
            .push((command.clone(), params["args"].clone()));
        Ok(answer(&command))
    })));
    calls
}

#[test]
fn the_factory_hands_out_each_git_host_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = factory(dir.path());
    for provider in GIT_HOSTS {
        assert_eq!(hosts.host(provider).unwrap().provider(), provider);
    }
    for other in ["linear", "jira", "plane", "", "GitHub"] {
        assert!(hosts.host(other).is_none(), "{other:?} is not a git host");
    }
    assert_eq!(
        hosts.require("jira").err().unwrap(),
        "jira is not a git host"
    );
}

#[test]
fn github_reaches_a_remote_project_through_its_machine() {
    let calls = remote_host(|command| match command {
        "git_github_work_item_details" => json!({"body": "Hi", "author": "dana"}),
        "git_github_work_item_thread" => json!({"comments": [], "commits": []}),
        "git_github_work_item_comment" => json!("https://github.com/acme/web/pull/42#c1"),
        "git_github_pr_diff" => {
            json!({"additions": 1, "deletions": 0, "files": [], "patch": "", "truncated": false})
        }
        "git_github_pr_checks" => json!({"headOid": "abc", "checks": []}),
        "git_github_pr_review" => Value::Null,
        other => panic!("unexpected {other}"),
    });
    let dir = tempfile::tempdir().unwrap();
    let github = factory(dir.path()).require("github").unwrap();
    let pr = item("github", "pr");
    assert_eq!(github.details(&pr).unwrap().body, "Hi");
    github.thread(&pr).unwrap();
    assert_eq!(
        github
            .comment(&pr, "Thanks".into(), "PRRC_1".into())
            .unwrap(),
        "https://github.com/acme/web/pull/42#c1"
    );
    assert_eq!(github.diff(&pr, true).unwrap().additions, 1);
    assert_eq!(github.checks(&pr).unwrap().head_oid, "abc");
    github
        .submit_review(&pr, Verdict::Approve, "LGTM", &[])
        .unwrap();

    let calls = calls.borrow();
    let asked: Vec<&str> = calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(
        asked,
        [
            "git_github_work_item_details",
            "git_github_work_item_thread",
            "git_github_work_item_comment",
            "git_github_pr_diff",
            "git_github_pr_checks",
            "git_github_pr_review",
        ]
    );
    for (command, args) in calls.iter() {
        assert_eq!(
            args["cwd"], "/home/me/web",
            "{command} runs in the project, on its machine"
        );
    }
    assert_eq!(
        calls[2].1["inReplyTo"], "PRRC_1",
        "the reply keeps its target"
    );
    assert_eq!(calls[3].1["fullContext"], true);
    assert_eq!(calls[5].1["verdict"], "approve");
}

#[test]
fn a_remote_pull_request_list_names_the_machine_s_account() {
    remote_host(|command| match command {
        "git_github_work_items" => json!([{
            "kind": "pr", "repo": "acme/web", "number": 7, "title": "Remote",
            "url": "", "state": "open", "createdAt": "", "updatedAt": "",
        }]),
        "git_github_login" => json!("remote-user"),
        other => panic!("unexpected {other}"),
    });
    let dir = tempfile::tempdir().unwrap();
    let github = factory(dir.path()).require("github").unwrap();
    let rows = github
        .work_items(
            Path::new(REMOTE),
            "acme/web",
            "pr",
            &Filters::default(),
            "open",
            40,
        )
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].viewer, "remote-user");

    let review = Filters {
        review_requested: true,
        ..Default::default()
    };
    let issues = github
        .work_items(Path::new(REMOTE), "acme/web", "issue", &review, "open", 40)
        .unwrap();
    assert!(issues.is_empty(), "review requests are never issues");
}

#[test]
fn what_a_host_cant_do_fails_with_a_sentence() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = factory(dir.path());
    let github = hosts.require("github").unwrap();
    assert_eq!(
        github
            .resolve_thread(&item("github", "issue"), "PRRT_1", true)
            .unwrap_err(),
        "This host has no resolving"
    );
    assert_eq!(
        github
            .resolve_thread(&item("github", "pr"), "PRRT_1", true)
            .unwrap_err(),
        "Resolve this thread on GitHub: the project is on another machine"
    );
    assert!(
        github
            .current_user(Path::new(REMOTE))
            .unwrap_err()
            .contains("is on another machine"),
        "automations read a remote project's comments only"
    );
    assert!(github.assigned("pr", 10).is_none(), "GitHub lists per repo");

    let gitlab = hosts.require("gitlab").unwrap();
    let mr = item("gitlab", "pr");
    assert_eq!(
        gitlab.checks(&mr).unwrap_err(),
        "Checks are not available for this host"
    );
    assert_eq!(
        gitlab.action(&mr, "merge").unwrap_err(),
        "This host can't do that from Gespan yet"
    );
    assert!(gitlab
        .open_pull_requests(Path::new("/tmp"), "acme/web", false, 5)
        .is_err());

    let azure = hosts.require("azuredevops").unwrap();
    let ado_pr = item("azuredevops", "pr");
    assert_eq!(
        azure
            .resolve_thread(&item("azuredevops", "issue"), "1", true)
            .unwrap_err(),
        "This host has no resolving"
    );
    assert!(azure.permission(&ado_pr, "dana").is_err());
}

#[test]
fn hosts_without_a_saved_connection_say_so_and_list_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = factory(dir.path());
    for provider in ["gitlab", "azuredevops"] {
        let host = hosts.require(provider).unwrap();
        let status = host.status().unwrap();
        assert!(!status.connected, "{provider} has no token saved");
        assert!(
            host.assigned("pr", 10).unwrap().is_err(),
            "{provider} can't list without a token"
        );
    }
}

#[test]
fn the_service_reaches_git_hosts_only_through_the_factory() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service {
        data_dir: dir.path().to_path_buf(),
        cwd: PathBuf::from("/nonexistent/checkout"),
    };
    let linear = Item {
        provider: "linear".into(),
        kind: "linear".into(),
        ..Default::default()
    };
    assert_eq!(
        service.resolve_thread(&linear, "t", true).unwrap_err(),
        "This host has no resolving"
    );
    assert_eq!(
        service.diff(&linear).unwrap_err(),
        "This issue has no pull request diff"
    );
    let jira = Item {
        provider: "jira".into(),
        kind: "pr".into(),
        ..Default::default()
    };
    assert_eq!(
        service
            .submit_review(&jira, Verdict::Approve, "ok", &[])
            .unwrap_err(),
        "Reviews are not available for this host"
    );
    assert_eq!(
        service.list_added_repo("linear", "acme/web").unwrap_err(),
        "linear repositories can't be added"
    );
    assert!(
        service.status("gitlab").is_ok_and(|s| !s.connected),
        "status asks the host"
    );
}

#[test]
fn a_link_in_a_remote_session_is_read_by_its_machine() {
    let calls = remote_host(|command| match command {
        "git_github_work_item" => json!({
            "kind": "pr", "repo": "acme/web", "number": 9, "title": "Linked",
            "url": "", "state": "open", "createdAt": "", "updatedAt": "",
        }),
        "git_github_work_item_details" => json!({"body": "Fixes the cache", "author": "dana"}),
        other => panic!("unexpected {other}"),
    });
    let dir = tempfile::tempdir().unwrap();
    let github = factory(dir.path()).require("github").unwrap();
    let item = github
        .linked_item(Path::new(REMOTE), "acme/web", "pr", 9)
        .unwrap();
    assert_eq!(
        (item.title.as_str(), item.provider.as_str()),
        ("Linked", "github")
    );
    let details = github
        .linked_details(Path::new(REMOTE), "acme/web", "pr", 9)
        .unwrap();
    assert_eq!(details.body, "Fixes the cache");
    let calls = calls.borrow();
    assert_eq!(calls.len(), 2);
    for (_, args) in calls.iter() {
        assert_eq!(
            (
                args["repo"].as_str(),
                args["kind"].as_str(),
                args["number"].as_i64()
            ),
            (Some("acme/web"), Some("pr"), Some(9))
        );
    }
}

#[test]
fn only_github_previews_links() {
    let dir = tempfile::tempdir().unwrap();
    for provider in ["gitlab", "azuredevops"] {
        let host = factory(dir.path()).require(provider).unwrap();
        assert_eq!(
            host.linked_item(Path::new("/tmp"), "acme/web", "pr", 1)
                .unwrap_err(),
            "This host's links can't be previewed yet"
        );
        assert!(host
            .linked_details(Path::new("/tmp"), "acme/web", "pr", 1)
            .is_err());
    }
}
