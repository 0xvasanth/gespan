use crate::orchestrator::repo::{checkout_root, current_branch, repo_key, session_files};
use std::{fs, path::Path, process::Command};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one commit holding `src/a.rs` and `src/b.rs`.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    fs::write(root.join("src/b.rs"), "b\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
    dir
}

fn edits(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn main_and_worktree_share_a_repo_key() {
    let main = repo();
    let holder = tempfile::tempdir().unwrap();
    let worktree = holder.path().join("feature");
    git(
        main.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            worktree.to_str().unwrap(),
        ],
    );
    let key = repo_key(main.path());
    assert!(key.is_some());
    assert_eq!(repo_key(&worktree), key);
}

#[test]
fn a_subdirectory_has_its_repo_key() {
    let main = repo();
    let sub = main.path().join("pkg").join("a");
    fs::create_dir_all(&sub).unwrap();
    assert!(repo_key(main.path()).is_some());
    assert_eq!(repo_key(&sub), repo_key(main.path()));
}

#[test]
fn a_plain_folder_has_no_repo_key() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(repo_key(plain.path()), None);
}

#[test]
fn a_missing_folder_has_no_repo_key() {
    let main = repo();
    assert_eq!(repo_key(&main.path().join("gone")), None);
}

#[test]
fn the_checkout_root_of_a_subdirectory_is_the_checkout() {
    let main = repo();
    let sub = main.path().join("src");
    let root = checkout_root(&sub).unwrap();
    assert_eq!(
        fs::canonicalize(root).unwrap(),
        fs::canonicalize(main.path()).unwrap()
    );
}

#[test]
fn files_keep_only_edits_still_dirty() {
    let main = repo();
    let root = main.path();
    fs::write(root.join("src/a.rs"), "a changed\n").unwrap();
    fs::write(root.join("src/b.rs"), "b changed\n").unwrap();
    git(root, &["commit", "-q", "-am", "b landed"]);
    fs::write(root.join("src/a.rs"), "a changed again\n").unwrap();
    fs::write(root.join("new.txt"), "new\n").unwrap();
    let files = session_files(&edits(&["src/a.rs", "src/b.rs", "new.txt"]), root);
    assert_eq!(files, ["new.txt", "src/a.rs"]);
}

#[test]
fn files_the_session_never_edited_are_not_its_own() {
    let main = repo();
    let root = main.path();
    fs::write(root.join("src/a.rs"), "someone else\n").unwrap();
    fs::write(root.join("src/b.rs"), "mine\n").unwrap();
    assert_eq!(session_files(&edits(&["src/b.rs"]), root), ["src/b.rs"]);
}

#[test]
fn absolute_edit_paths_become_relative() {
    let main = repo();
    let root = main.path();
    fs::write(root.join("src/a.rs"), "changed\n").unwrap();
    let absolute = root.join("src").join("a.rs").to_string_lossy().into_owned();
    assert_eq!(session_files(&[absolute], root), ["src/a.rs"]);
}

#[test]
fn edits_outside_the_checkout_are_dropped() {
    let main = repo();
    let root = main.path();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(root.join("src/a.rs"), "changed\n").unwrap();
    let outside = elsewhere.path().join("x.rs").to_string_lossy().into_owned();
    assert!(session_files(&[outside], root).is_empty());
}

#[test]
fn a_renamed_file_counts_under_its_new_name() {
    let main = repo();
    let root = main.path();
    git(root, &["mv", "src/a.rs", "src/c.rs"]);
    assert_eq!(session_files(&edits(&["src/c.rs"]), root), ["src/c.rs"]);
}

#[test]
fn a_folder_that_is_not_a_repository_has_no_files() {
    let plain = tempfile::tempdir().unwrap();
    fs::write(plain.path().join("a.rs"), "a\n").unwrap();
    assert!(session_files(&edits(&["a.rs"]), plain.path()).is_empty());
}

#[test]
fn a_worktree_names_its_branch() {
    let main = repo();
    let holder = tempfile::tempdir().unwrap();
    let worktree = holder.path().join("feature");
    git(
        main.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            worktree.to_str().unwrap(),
        ],
    );
    assert_eq!(current_branch(&worktree).as_deref(), Some("feature"));
}

#[test]
fn a_detached_checkout_has_no_branch() {
    let main = repo();
    git(main.path(), &["checkout", "-q", "--detach"]);
    assert_eq!(current_branch(main.path()), None);
}
