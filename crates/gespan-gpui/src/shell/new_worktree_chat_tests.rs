//! New chats started with "New worktree" each get a worktree of their own,
//! cut from the project's checkout, whichever worktree the window was showing
//! when they were created.
use super::*;
use crate::worktree_switcher::WorktreeFocus;
use gespan_core::{store::Store, worktree_management::default_root};
use gpui::{TestAppContext, WindowHandle};
use std::path::Path;

struct Fixture {
    window: WindowHandle<Shell>,
    repo: tempfile::TempDir,
    project: String,
    /// The existing worktree the window shows when the new chats start. Its
    /// branch has a commit the project's `main` does not.
    feature: String,
    /// The chat already working in `feature`.
    chat: i64,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(default_root(self.repo.path()));
    }
}
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
/// A project with an existing `feature` worktree, focused in the window, and
/// a chat that already works there.
fn fixture(cx: &mut TestAppContext) -> Fixture {
    let repo = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-b", "main"][..],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "user.name", "Fixture"],
        &["commit", "--allow-empty", "-m", "Initial"],
    ] {
        git(repo.path(), args);
    }
    let feature = gespan_core::git::create_named_worktree(repo.path(), "feature")
        .unwrap()
        .path;
    git(
        Path::new(&feature),
        &["commit", "--allow-empty", "-m", "Feature work"],
    );
    let project = repo.path().to_string_lossy().into_owned();
    let store = Store::open(Path::new(":memory:")).unwrap();
    let chat = store.create("kiro", &project).unwrap();
    store
        .save_agent_settings(
            chat.id,
            &gespan_core::agent::Settings {
                worktree_cwd: Some(feature.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    let mut state = Workspace::default();
    state.add_project(&project);
    state.open(chat.id);
    let saved = serde_json::to_string(&state).unwrap();
    let store = Arc::new(Mutex::new(store));
    let id = chat.id;
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(store, vec![chat], project.clone(), Some(saved), cx);
                shell.ensure_session(id, cx);
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    let focus = WorktreeFocus {
        path: feature.clone(),
        branch: Some("feature".into()),
    };
    window
        .update(cx, |shell, window, cx| {
            shell.select_worktree(Some(focus), window, cx);
            shell.open(id, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    Fixture {
        window,
        repo,
        project,
        feature,
        chat: id,
    }
}
impl Fixture {
    /// New chat, then "New worktree" in the composer's Workspace menu.
    /// Returns the draft's id.
    fn draft(&self, kind: Kind, cx: &mut TestAppContext) -> i64 {
        let before = self.chats(cx);
        self.window
            .update(cx, |shell, _, cx| shell.create(kind, cx))
            .unwrap();
        cx.run_until_parked();
        let id = self
            .chats(cx)
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("New chat made a chat");
        self.window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.workspace.active, Some(id), "the new chat is shown");
                shell.sessions[&id].update(cx, |view, cx| {
                    assert_eq!(view.summary.provider, kind.id());
                    assert_eq!(
                        view.settings.worktree_cwd.as_deref(),
                        Some(self.feature.as_str()),
                        "a new chat starts in the focused worktree"
                    );
                    view.menu_choice("workspace:new", cx);
                    assert!(view.settings.new_worktree);
                });
            })
            .unwrap();
        cx.run_until_parked();
        id
    }
    /// Sends `prompt` from the draft.
    fn send(&self, id: i64, prompt: &str, cx: &mut TestAppContext) {
        self.window
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].update(cx, |view, cx| {
                    // The worktree comes before the provider starts, for every
                    // provider; this one then stops without launching a CLI.
                    view.summary.provider = "fixture".into();
                    view.input
                        .update(cx, |input, cx| input.set_text(prompt.into(), cx));
                    view.send(cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
    }
    fn new_worktree_chat(&self, kind: Kind, prompt: &str, cx: &mut TestAppContext) -> i64 {
        let id = self.draft(kind, cx);
        self.send(id, prompt, cx);
        id
    }
    fn chats(&self, cx: &mut TestAppContext) -> Vec<i64> {
        self.window
            .update(cx, |shell, _, _| shell.sessions.keys().copied().collect())
            .unwrap()
    }
    /// The branch the composer's workspace bar shows beside its workspace.
    fn chip_branch(&self, id: i64, cx: &mut TestAppContext) -> Option<String> {
        self.window
            .update(cx, |shell, _, cx| {
                let view = shell.sessions[&id].read(cx);
                assert!(view.settings.new_worktree);
                view.workspace_branch()
            })
            .unwrap()
    }
    /// Where the chat works, as the chat, the shell and the store see it.
    fn working_copy(&self, id: i64, cx: &mut TestAppContext) -> String {
        self.window
            .update(cx, |shell, _, cx| {
                let view = shell.sessions[&id].read(cx);
                let stored = locked(&shell.store)
                    .unwrap()
                    .agent_settings(id)
                    .unwrap()
                    .unwrap_or_default();
                assert!(!view.settings.new_worktree && !stored.new_worktree);
                let cwd = view
                    .settings
                    .worktree_cwd
                    .clone()
                    .expect("the chat runs in a worktree");
                assert_eq!(stored.worktree_cwd.as_deref(), Some(cwd.as_str()));
                assert_eq!(shell.session_working_copy(id, cx), cwd);
                cwd
            })
            .unwrap()
    }
    fn head(&self, tree: &str) -> String {
        git(Path::new(tree), &["rev-parse", "HEAD"])
    }
    fn worktrees(&self) -> Vec<String> {
        git(self.repo.path(), &["worktree", "list", "--porcelain"])
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(str::to_owned)
            .collect()
    }
}
fn same(a: &str, b: &str) -> bool {
    gespan_core::files::path_key(a) == gespan_core::files::path_key(b)
}

#[gpui::test]
fn each_new_worktree_chat_gets_its_own_worktree_from_the_project(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut trees = Vec::new();
    for (kind, prompt) in [(Kind::Kiro, "First task"), (Kind::Claude, "Second task")] {
        let id = f.draft(kind, cx);
        assert_eq!(
            f.chip_branch(id, cx).as_deref(),
            Some("main"),
            "the composer shows the branch the worktree is cut from"
        );
        f.send(id, prompt, cx);
        let tree = f.working_copy(id, cx);
        assert!(!same(&tree, &f.feature), "{tree} is the existing worktree");
        assert!(!same(&tree, &f.project), "{tree} is the main checkout");
        assert_eq!(
            f.head(&tree),
            f.head(&f.project),
            "cut from the project, not from the focused worktree"
        );
        trees.push(tree);
    }
    assert!(!same(&trees[0], &trees[1]), "both chats share {}", trees[0]);
    assert_ne!(f.head(&f.feature), f.head(&f.project));
    assert_eq!(f.worktrees().len(), 4, "main, feature and one per chat");
}

#[gpui::test]
fn a_chosen_base_is_kept(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let id = f.draft(Kind::Kiro, cx);
    // The base the branch picker's GitPicked::Base stores.
    f.window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, _| {
                view.settings.worktree_base = Some("feature".into())
            })
        })
        .unwrap();
    assert_eq!(f.chip_branch(id, cx).as_deref(), Some("feature"));
    f.send(id, "Build on the feature", cx);
    let tree = f.working_copy(id, cx);
    assert!(!same(&tree, &f.feature));
    assert_eq!(f.head(&tree), f.head(&f.feature));
}

#[gpui::test]
fn a_chat_already_in_a_worktree_keeps_working_there(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let id = f.new_worktree_chat(Kind::Kiro, "First task", cx);
    let tree = f.working_copy(id, cx);
    let count = f.worktrees().len();
    // A follow-up in the new chat, and one in the chat that was already in
    // `feature`, both stay where they are.
    f.send(id, "Follow up", cx);
    assert_eq!(f.working_copy(id, cx), tree);
    f.send(f.chat, "Keep going", cx);
    f.window
        .update(cx, |shell, _, cx| {
            let view = shell.sessions[&f.chat].read(cx);
            assert_eq!(
                view.settings.worktree_cwd.as_deref(),
                Some(f.feature.as_str())
            );
        })
        .unwrap();
    assert_eq!(f.worktrees().len(), count, "no other worktree was made");
}

#[gpui::test]
fn choosing_new_worktree_before_the_draft_loads_still_cuts_from_the_project(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    // A draft made in the focused worktree, as New chat makes it, with
    // "New worktree" chosen before its first load lands.
    let id = f
        .window
        .update(cx, |shell, window, cx| {
            let summary = {
                let store = locked(&shell.store).unwrap();
                let summary = store.create("kiro", &f.project).unwrap();
                store
                    .save_agent_settings(
                        summary.id,
                        &gespan_core::agent::Settings {
                            worktree_cwd: Some(f.feature.clone()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                summary
            };
            let id = summary.id;
            shell.add(summary, cx);
            shell.open(id, window, cx);
            shell.sessions[&id].update(cx, |view, cx| view.menu_choice("workspace:new", cx));
            id
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(f.chip_branch(id, cx).as_deref(), Some("main"));
    f.send(id, "Start fresh", cx);
    let tree = f.working_copy(id, cx);
    assert!(!same(&tree, &f.feature));
    assert_eq!(f.head(&tree), f.head(&f.project));
}
