//! A chat's new worktree is named after the task it was started with, for
//! every provider, before any provider runs.
use super::*;
use std::path::Path;

struct Repo(tempfile::TempDir);
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(gespan_core::worktree_management::default_root(
            self.0.path(),
        ));
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

/// Runs one new-worktree turn of a `provider` chat whose CLI is not found,
/// and returns the worktree it made.
fn turn(store: &Database, project: &str, provider: Kind, prompt: &str) -> String {
    let summary = locked(store)
        .unwrap()
        .create(provider.id(), project)
        .unwrap();
    let id = summary.id;
    let (tx, rx) = async_channel::unbounded();
    let (_controls, controls) = std::sync::mpsc::channel();
    run_turn_using(
        TurnInput {
            summary,
            prompt: prompt.into(),
            settings: Settings {
                new_worktree: true,
                ..Default::default()
            },
        },
        controls,
        store.clone(),
        Arc::new(AtomicBool::new(false)),
        tx,
        |_| None,
    );
    let last = std::iter::from_fn(|| rx.try_recv().ok()).last().unwrap();
    assert!(last.finished, "{provider:?}");
    let saved = locked(store).unwrap().agent_settings(id).unwrap().unwrap();
    saved
        .worktree_cwd
        .unwrap_or_else(|| panic!("{provider:?} made no worktree: {}", last.status))
}

#[test]
fn every_provider_names_its_worktree_after_the_task() {
    let repo = Repo(tempfile::tempdir().unwrap());
    for args in [
        &["init", "-b", "main"][..],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "user.name", "Fixture"],
        &["commit", "--allow-empty", "-m", "Initial"],
    ] {
        git(repo.0.path(), args);
    }
    let project = repo.0.path().to_str().unwrap();
    let store: Database = Arc::new(Mutex::new(
        gespan_core::store::Store::open(Path::new(":memory:")).unwrap(),
    ));
    for (n, provider) in Kind::ALL.into_iter().enumerate() {
        let tree = turn(
            &store,
            project,
            provider,
            "Fix the login redirect\n\nIt loops.",
        );
        let name = match n {
            0 => "gespan/fix-the-login-redirect".to_owned(),
            n => format!("gespan/fix-the-login-redirect-{}", n + 1),
        };
        assert_eq!(
            git(Path::new(&tree), &["branch", "--show-current"]),
            name,
            "{provider:?}"
        );
        assert_eq!(
            Path::new(&tree).file_name().unwrap().to_string_lossy(),
            name.replace('/', "-"),
            "{provider:?}"
        );
    }
}
