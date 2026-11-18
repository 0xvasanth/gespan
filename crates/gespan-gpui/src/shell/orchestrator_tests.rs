//! Sessions can list and read their siblings: other sessions in the same
//! repository, in its main checkout or any of its worktrees.
use super::orchestrator::live_status;
use super::*;
use gespan_core::orchestrator::{
    bridge::{Action, Request, Scope},
    roster::Status,
};
use gespan_core::{store::Store, transcript::Row, worktree_management::default_root};
use gpui::{TestAppContext, WindowHandle};
use serde_json::{Value, json};
use std::path::Path;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-b", "main"][..],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "user.name", "Fixture"],
        &["config", "commit.gpgsign", "false"],
    ] {
        git(repo.path(), args);
    }
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    std::fs::write(repo.path().join("src/a.rs"), "a\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-q", "-m", "Initial"]);
    repo
}

fn rows(pairs: &[(&str, &str)]) -> Vec<Row> {
    let mut transcript = gespan_core::transcript::Transcript::default();
    for (role, text) in pairs {
        transcript.begin(role);
        transcript.append(text);
    }
    transcript.rows
}

fn edit_row(path: &str) -> String {
    json!({
        "id": format!("edit-{path}"),
        "name": "Edit",
        "input": json!({"file_path": path, "old_string": "a", "new_string": "b"}).to_string(),
        "output": "",
        "state": "completed",
    })
    .to_string()
}

struct Fixture {
    window: WindowHandle<Shell>,
    repo: tempfile::TempDir,
    _other: tempfile::TempDir,
    /// On the main checkout.
    a: i64,
    /// On the `feature` worktree; asked to fix the sidebar and edited `src/a.rs`.
    b: i64,
    /// In another repository.
    c: i64,
    archived: i64,
    remote: i64,
    /// A new chat nothing was ever sent from.
    blank: i64,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(default_root(self.repo.path()));
    }
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    let repo = repo();
    let other = self::repo();
    let feature = gespan_core::git::create_named_worktree(repo.path(), "feature")
        .unwrap()
        .path;
    std::fs::write(Path::new(&feature).join("src/a.rs"), "changed\n").unwrap();
    let project = repo.path().to_string_lossy().into_owned();
    let other_project = other.path().to_string_lossy().into_owned();
    let mut store = Store::open(Path::new(":memory:")).unwrap();
    let a = store.create("claude", &project).unwrap();
    let b = store.create("kiro", &project).unwrap();
    let c = store.create("claude", &other_project).unwrap();
    let archived = store.create("claude", &project).unwrap();
    let blank = store.create("claude", &project).unwrap();
    store
        .write_rows(
            a.id,
            0,
            &rows(&[("user", "Write the docs"), ("assistant", "On it.")]),
        )
        .unwrap();
    store
        .write_rows(
            c.id,
            0,
            &rows(&[("user", "Bump the version"), ("assistant", "Done.")]),
        )
        .unwrap();
    store
        .save_agent_settings(
            b.id,
            &gespan_core::agent::Settings {
                worktree_cwd: Some(feature.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .write_rows(
            b.id,
            0,
            &rows(&[
                ("user", "Fix the sidebar flicker\nIt flashes on resize."),
                ("tool", &edit_row("src/a.rs")),
                ("assistant", "Done."),
            ]),
        )
        .unwrap();
    store.set_archived(archived.id, true).unwrap();
    let mut sessions = store.sessions().unwrap();
    // A remote project's chat, as the sidebar lists it.
    let remote = gespan_core::store::Session {
        id: c.id + 1_000,
        cwd: "remote://box/repo".into(),
        ..a.clone()
    };
    sessions.push(remote.clone());
    let mut state = Workspace::default();
    state.add_project(&project);
    state.open(a.id);
    let saved = serde_json::to_string(&state).unwrap();
    let store = Arc::new(Mutex::new(store));
    let ids = (a.id, b.id, c.id, archived.id, remote.id, blank.id);
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(store, sessions, project.clone(), Some(saved), cx);
                shell.ensure_session(ids.0, cx);
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    Fixture {
        window,
        repo,
        _other: other,
        a: ids.0,
        b: ids.1,
        c: ids.2,
        archived: ids.3,
        remote: ids.4,
        blank: ids.5,
    }
}

impl Fixture {
    /// Asks the shell as `session` would through its `sessions` tools.
    fn ask(&self, session: i64, action: Action, cx: &mut TestAppContext) -> Result<Value, String> {
        let (reply, answer) = std::sync::mpsc::channel();
        self.window
            .update(cx, |shell, _, cx| {
                shell.answer_sessions(
                    Request {
                        session,
                        action,
                        reply,
                    },
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        answer.try_recv().expect("the shell answered")
    }
    fn list(&self, session: i64, scope: Scope, cx: &mut TestAppContext) -> Value {
        self.ask(session, Action::List { scope }, cx).unwrap()
    }
}

fn ids(list: &Value, key: &str) -> Vec<i64> {
    list[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn the_status_follows_hold_then_work_then_approval() {
    assert_eq!(live_status(true, true, true), Status::Held);
    assert_eq!(live_status(false, true, true), Status::WaitingOnUser);
    assert_eq!(live_status(false, true, false), Status::Working);
    assert_eq!(live_status(false, false, false), Status::Idle);
}

#[gpui::test]
fn list_from_a_names_b_with_branch_goal_and_files(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let list = f.list(f.a, Scope::Repo, cx);
    assert_eq!(ids(&list, "sessions"), [f.b], "{list:#}");
    let b = &list["sessions"][0];
    assert_eq!(
        b["workspace"],
        json!({"kind":"worktree","branch":"feature"})
    );
    assert_eq!(b["status"], "idle");
    assert_eq!(b["harness"], "kiro");
    assert_eq!(b["goal"], "Fix the sidebar flicker");
    assert_eq!(b["files"], json!(["src/a.rs"]));
}

#[gpui::test]
fn b_sees_a_on_the_main_checkout(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let list = f.list(f.b, Scope::Repo, cx);
    assert_eq!(ids(&list, "sessions"), [f.a], "{list:#}");
    assert_eq!(list["sessions"][0]["workspace"], json!({"kind":"main"}));
}

#[gpui::test]
fn a_session_in_another_repo_is_only_listed_with_scope_all(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let repo = f.list(f.a, Scope::Repo, cx);
    assert!(!ids(&repo, "sessions").contains(&f.c));
    assert!(repo.get("elsewhere").is_none());
    let all = f.list(f.a, Scope::All, cx);
    assert_eq!(ids(&all, "sessions"), [f.b]);
    assert_eq!(ids(&all, "elsewhere"), [f.c], "{all:#}");
}

#[gpui::test]
fn archived_and_remote_sessions_are_left_out(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let all = f.list(f.a, Scope::All, cx);
    for key in ["sessions", "elsewhere"] {
        let listed = ids(&all, key);
        assert!(!listed.contains(&f.archived), "{key}: {all:#}");
        assert!(!listed.contains(&f.remote), "{key}: {all:#}");
    }
}

#[gpui::test]
fn read_returns_bs_recent_exchanges_without_operator(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let page = f
        .ask(
            f.a,
            Action::Read {
                id: f.b,
                before: None,
            },
            cx,
        )
        .unwrap();
    assert_eq!(page["sessionId"], f.b);
    assert!(
        page.to_string().contains("Fix the sidebar flicker"),
        "{page:#}"
    );
}

#[gpui::test]
fn archived_and_remote_sessions_are_not_readable(cx: &mut TestAppContext) {
    let f = fixture(cx);
    for id in [f.archived, f.remote, 999_999] {
        let error = f
            .ask(f.a, Action::Read { id, before: None }, cx)
            .unwrap_err();
        assert_eq!(error, "That session isn't readable here.");
    }
}

#[gpui::test]
fn facts_are_learned_in_the_background_for_the_roster(cx: &mut TestAppContext) {
    let f = fixture(cx);
    // Opening A asked for its roster; what B is doing was read off the UI
    // thread and kept, so the next turn's roster needs no git.
    let facts = f
        .window
        .update(cx, |shell, _, _| shell.session_facts.get(&f.b).cloned())
        .unwrap()
        .expect("B's facts were learned");
    assert_eq!(facts.goal, "Fix the sidebar flicker");
    assert_eq!(facts.files, ["src/a.rs"]);
    assert_eq!(facts.branch.as_deref(), Some("feature"));
}

#[gpui::test]
fn a_finished_turn_refreshes_its_files(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.list(f.a, Scope::Repo, cx);
    // B's change lands; its file is no longer uncommitted.
    let feature = f
        .window
        .update(cx, |shell, _, _| shell.session_worktrees[&f.b].clone())
        .unwrap()
        .unwrap();
    git(Path::new(&feature), &["commit", "-qam", "Fix"]);
    f.window
        .update(cx, |shell, _, cx| shell.refresh_session_facts(f.b, cx))
        .unwrap();
    cx.run_until_parked();
    let list = f.list(f.a, Scope::Repo, cx);
    assert_eq!(list["sessions"][0]["files"], json!([]));
}

#[test]
fn every_local_turn_mounts_the_sessions_toolkit() {
    use crate::runtime::sessions_toolkit;
    assert_eq!(
        sessions_toolkit(false),
        Some(gespan_core::tool_servers::SESSIONS)
    );
    assert_eq!(sessions_toolkit(true), None);
}

#[gpui::test]
fn a_session_in_another_repo_is_not_readable(cx: &mut TestAppContext) {
    let f = fixture(cx);
    // Listed with scope all, but its transcript stays with its own repo.
    f.list(f.a, Scope::All, cx);
    let error = f
        .ask(
            f.a,
            Action::Read {
                id: f.c,
                before: None,
            },
            cx,
        )
        .unwrap_err();
    assert_eq!(error, "That session isn't readable here.");
}

#[gpui::test]
fn elsewhere_names_the_project_folder_not_its_path(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let all = f.list(f.a, Scope::All, cx);
    let c = &all["elsewhere"][0];
    assert!(c.get("cwd").is_none(), "{all:#}");
    let folder = f._other.path().file_name().unwrap().to_string_lossy();
    assert_eq!(c["project"], folder.as_ref());
}

#[gpui::test]
fn a_blank_chat_is_not_a_sibling(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let all = f.list(f.a, Scope::All, cx);
    for key in ["sessions", "elsewhere"] {
        assert!(!ids(&all, key).contains(&f.blank), "{key}: {all:#}");
    }
    // A blank chat still sees the sessions at work around it.
    assert_eq!(
        ids(&f.list(f.blank, Scope::Repo, cx), "sessions"),
        [f.b, f.a]
    );
}
