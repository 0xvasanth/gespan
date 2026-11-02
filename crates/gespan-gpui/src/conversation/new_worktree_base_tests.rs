//! A new worktree is cut from the base the user chose, or from the project
//! checkout's HEAD, never from whatever branch the draft was last showing.
use super::*;
use gespan_core::{store::Store, worktree_management::default_root};
use gpui::{TestAppContext, WindowHandle};
use std::sync::Mutex;

struct Repo {
    dir: tempfile::TempDir,
    feature: String,
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(default_root(self.dir.path()));
    }
}
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
/// A project on `main` with a `feature` worktree one commit ahead.
fn repo() -> Repo {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-b", "main"][..],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "user.name", "Fixture"],
        &["commit", "--allow-empty", "-m", "Initial"],
    ] {
        git(dir.path(), args);
    }
    let feature = gespan_core::git::create_named_worktree(dir.path(), "feature")
        .unwrap()
        .path;
    git(
        Path::new(&feature),
        &["commit", "--allow-empty", "-m", "Feature work"],
    );
    Repo { dir, feature }
}
/// A draft that starts in `feature`, as New chat makes it there.
fn draft(repo: &Repo, cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let project = repo.dir.path().to_string_lossy().into_owned();
    let store = Store::open(Path::new(":memory:")).unwrap();
    let mut summary = store.create("kiro", &project).unwrap();
    store
        .save_agent_settings(
            summary.id,
            &Settings {
                worktree_cwd: Some(repo.feature.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    // The worktree comes before the provider starts; this one then stops
    // without launching a CLI.
    summary.provider = "fixture".into();
    let store = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        input::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                // As the shell does when it shows the chat.
                let mut view = Conversation::new(summary, store, cx);
                view.load(cx);
                view
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.loaded);
            assert_eq!(view.branch.as_deref(), Some("feature"));
        })
        .unwrap();
    window
}
/// Sends from the draft while it still holds the `feature` branch it showed
/// before "New worktree", as a late load or refresh can leave it. Returns the
/// new worktree.
fn send_with_stale_branch(window: WindowHandle<Conversation>, cx: &mut TestAppContext) -> String {
    window
        .update(cx, |view, _, cx| {
            view.branch = Some("feature".into());
            view.input
                .update(cx, |input, cx| input.set_text("Start fresh".into(), cx));
            view.send(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(!view.settings.new_worktree);
            view.settings.worktree_cwd.clone().unwrap()
        })
        .unwrap()
}

#[gpui::test]
fn without_a_chosen_base_a_new_worktree_starts_from_the_project(cx: &mut TestAppContext) {
    let repo = repo();
    let window = draft(&repo, cx);
    window
        .update(cx, |view, _, cx| view.menu_choice("workspace:new", cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.workspace_branch().as_deref(), Some("main"));
        })
        .unwrap();
    let tree = send_with_stale_branch(window, cx);
    assert_ne!(tree, repo.feature);
    assert_eq!(
        git(Path::new(&tree), &["rev-parse", "HEAD"]),
        git(repo.dir.path(), &["rev-parse", "main"]),
        "cut from the project, not from the branch the draft showed"
    );
}

#[gpui::test]
fn a_chosen_base_is_what_the_new_worktree_starts_from(cx: &mut TestAppContext) {
    let repo = repo();
    let window = draft(&repo, cx);
    window
        .update(cx, |view, _, cx| {
            view.menu_choice("workspace:new", cx);
            view.settings.worktree_base = Some("feature".into());
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.workspace_branch().as_deref(), Some("feature"));
        })
        .unwrap();
    let tree = send_with_stale_branch(window, cx);
    assert_eq!(
        git(Path::new(&tree), &["rev-parse", "HEAD"]),
        git(repo.dir.path(), &["rev-parse", "feature"])
    );
}
