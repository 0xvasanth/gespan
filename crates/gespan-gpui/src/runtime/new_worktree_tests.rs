//! A turn asked for a new worktree makes one before its provider starts, the
//! same way for every provider.
use super::*;
use std::path::{Path, PathBuf};

struct Repo(tempfile::TempDir);
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(gespan_core::worktree_management::default_root(
            self.0.path(),
        ));
    }
}
impl Repo {
    fn new() -> Self {
        let repo = Self(tempfile::tempdir().unwrap());
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.email", "fixture@example.invalid"],
            &["config", "user.name", "Fixture"],
            &["commit", "--allow-empty", "-m", "Initial"],
        ] {
            repo.git(args);
        }
        repo
    }
    fn path(&self) -> &str {
        self.0.path().to_str().unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(self.0.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
    fn worktrees(&self) -> usize {
        self.git(&["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count()
    }
}

/// Runs one turn of a new `provider` chat with `settings`, whose CLI is not
/// found, and returns where the chat ended up working.
fn turn(store: &Database, project: &str, provider: Kind, settings: Settings) -> Option<String> {
    let summary = locked(store)
        .unwrap()
        .create(provider.id(), project)
        .unwrap();
    turn_in(store, summary, settings)
}

/// The same, as a turn of the existing chat `summary`.
fn turn_in(
    store: &Database,
    summary: gespan_core::store::Session,
    settings: Settings,
) -> Option<String> {
    let provider = summary.provider.clone();
    let id = summary.id;
    let (tx, rx) = async_channel::unbounded();
    let (_controls, controls) = std::sync::mpsc::channel();
    run_turn_using(
        TurnInput {
            summary,
            prompt: "Start".into(),
            settings,
        },
        controls,
        store.clone(),
        Arc::new(AtomicBool::new(false)),
        tx,
        |_| None,
    );
    let last = std::iter::from_fn(|| rx.try_recv().ok()).last().unwrap();
    assert!(
        last.finished && !last.succeeded,
        "{provider:?}: {}",
        last.status
    );
    let saved = locked(store).unwrap().agent_settings(id).unwrap().unwrap();
    // The last update names only a worktree this turn made.
    if let Some(made) = &last.worktree_cwd {
        assert!(
            !saved.new_worktree,
            "{provider:?} still asks for a worktree"
        );
        assert_eq!(saved.worktree_cwd.as_ref(), Some(made));
    }
    saved.worktree_cwd
}

fn store() -> Database {
    Arc::new(Mutex::new(
        gespan_core::store::Store::open(Path::new(":memory:")).unwrap(),
    ))
}

#[test]
fn every_provider_gets_a_new_worktree_per_chat() {
    let repo = Repo::new();
    let store = store();
    let mut trees: Vec<PathBuf> = Vec::new();
    for provider in Kind::ALL {
        for _ in 0..2 {
            let tree = turn(
                &store,
                repo.path(),
                provider,
                Settings {
                    new_worktree: true,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|| panic!("{provider:?} made no worktree"));
            let tree = PathBuf::from(tree);
            assert!(tree.is_dir(), "{provider:?}: {}", tree.display());
            assert!(
                !trees.contains(&tree),
                "{provider:?} reused {}",
                tree.display()
            );
            trees.push(tree);
        }
    }
    assert_eq!(repo.worktrees(), 1 + 2 * Kind::ALL.len());
}

#[test]
fn a_chat_keeps_its_new_worktree_for_later_turns() {
    let repo = Repo::new();
    let store = store();
    let summary = locked(&store)
        .unwrap()
        .create(Kind::Kiro.id(), repo.path())
        .unwrap();
    let id = summary.id;
    let made = turn_in(
        &store,
        summary.clone(),
        Settings {
            new_worktree: true,
            ..Default::default()
        },
    )
    .expect("the first turn makes a worktree");
    // Later turns of the same chat run with what it saved, and a stale
    // request beside its worktree is that chat reopened, not a new one.
    for stale in [false, true] {
        let mut settings = locked(&store).unwrap().agent_settings(id).unwrap().unwrap();
        settings.new_worktree |= stale;
        let tree = turn_in(&store, summary.clone(), settings);
        assert_eq!(
            tree.as_deref(),
            Some(made.as_str()),
            "stale request: {stale}"
        );
    }
    assert_eq!(repo.worktrees(), 2, "main and the chat's one worktree");
}
