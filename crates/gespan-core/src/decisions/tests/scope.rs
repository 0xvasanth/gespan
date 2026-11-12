use crate::decisions::{normalize_remote, scope_for, shared_file};
use gespan_memory::Scope;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_empty(dir: &Path) {
    git(dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

#[test]
fn normalize_remote_unifies_spellings() {
    for spelling in [
        "git@github.com:O/R.git",
        "https://github.com/o/r",
        "https://user@github.com/o/r/",
        "ssh://git@github.com/o/r.git",
        "http://github.com/O/R.git/",
        "  https://GitHub.com/o/r.git  ",
    ] {
        assert_eq!(normalize_remote(spelling), "github.com/o/r", "{spelling}");
    }
}

#[test]
fn normalize_remote_drops_the_port() {
    assert_eq!(
        normalize_remote("ssh://git@git.example.com:2222/o/r.git"),
        "git.example.com/o/r"
    );
    assert_eq!(
        normalize_remote("https://git.example.com:8443/o/r"),
        "git.example.com/o/r"
    );
}

#[test]
fn normalize_remote_keeps_nested_groups() {
    assert_eq!(
        normalize_remote("git@gitlab.com:Group/Sub/Repo.git"),
        "gitlab.com/group/sub/repo"
    );
}

#[test]
fn normalize_remote_local_paths_are_lowercased_with_forward_slashes() {
    assert_eq!(normalize_remote("/Srv/Git/Repo.git/"), "/srv/git/repo");
    assert_eq!(normalize_remote(r"C:\Repos\Foo.git"), "c:/repos/foo");
    assert_eq!(
        normalize_remote("file:///Srv/Git/Repo.git"),
        "/srv/git/repo"
    );
}

#[test]
fn scope_for_repo_with_origin_is_repo() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:O/R.git"],
    );
    assert_eq!(scope_for(dir.path()), Scope::Repo("github.com/o/r".into()));
}

#[test]
fn scope_for_prefers_origin_over_other_remotes() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    git(dir.path(), &["remote", "add", "aaa", "https://h.com/a/a"]);
    git(
        dir.path(),
        &["remote", "add", "origin", "https://h.com/o/r"],
    );
    assert_eq!(scope_for(dir.path()), Scope::Repo("h.com/o/r".into()));
}

#[test]
fn scope_for_without_origin_uses_first_remote_alphabetically() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    git(dir.path(), &["remote", "add", "zeta", "https://h.com/z/z"]);
    git(dir.path(), &["remote", "add", "alpha", "https://h.com/a/a"]);
    assert_eq!(scope_for(dir.path()), Scope::Repo("h.com/a/a".into()));
}

#[test]
fn scope_for_repo_without_remote_is_project_toplevel() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    let sub = dir.path().join("a").join("b");
    std::fs::create_dir_all(&sub).unwrap();
    let Scope::Project(p) = scope_for(&sub) else {
        panic!("expected project scope");
    };
    assert_eq!(canon(Path::new(&p)), canon(dir.path()));
}

#[test]
fn scope_for_non_git_dir_is_project_path() {
    let dir = TempDir::new().unwrap();
    let Scope::Project(p) = scope_for(dir.path()) else {
        panic!("expected project scope");
    };
    assert_eq!(canon(Path::new(&p)), canon(dir.path()));
}

#[test]
fn worktree_and_main_checkout_share_scope_and_shared_file() {
    let dir = TempDir::new().unwrap();
    let main = dir.path().join("main");
    std::fs::create_dir(&main).unwrap();
    init_repo(&main);
    commit_empty(&main);
    git(
        &main,
        &["remote", "add", "origin", "https://github.com/o/r"],
    );
    let wt = dir.path().join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );
    let data = TempDir::new().unwrap();

    assert_eq!(scope_for(&wt), scope_for(&main));
    let (a, b) = (
        shared_file(&main, data.path()),
        shared_file(&wt, data.path()),
    );
    // The directory is created on first write, so compare the common dir.
    let common = |p: &Path| canon(p.parent().unwrap().parent().unwrap());
    assert_eq!(common(&a), common(&b));
    assert_eq!(a.parent().unwrap().file_name().unwrap(), "gespan");
    assert_eq!(a.file_name().unwrap(), "decisions.md");
    assert_eq!(a.file_name(), b.file_name());
    // Inside the main checkout's .git dir, not inside the worktree.
    assert_eq!(common(&a), canon(&main.join(".git")));
}

#[test]
fn worktree_without_remote_shares_the_shared_file_but_not_the_project_scope() {
    let dir = TempDir::new().unwrap();
    let main = dir.path().join("main");
    std::fs::create_dir(&main).unwrap();
    init_repo(&main);
    commit_empty(&main);
    let wt = dir.path().join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );
    let data = TempDir::new().unwrap();
    let (a, b) = (
        shared_file(&main, data.path()),
        shared_file(&wt, data.path()),
    );
    assert_eq!(a, b);
    // Each checkout is its own project when there is no remote to unify them.
    let (Scope::Project(pa), Scope::Project(pb)) = (scope_for(&main), scope_for(&wt)) else {
        panic!("expected project scopes");
    };
    assert_ne!(canon(Path::new(&pa)), canon(Path::new(&pb)));
}

#[test]
fn shared_file_outside_git_goes_to_data_dir() {
    let dir = TempDir::new().unwrap();
    let other = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let a = shared_file(dir.path(), data.path());
    assert_eq!(a.parent().unwrap(), data.path().join("decisions"));
    assert_eq!(a.extension().unwrap(), "md");
    let stem = a.file_stem().unwrap().to_str().unwrap();
    assert_eq!(stem.len(), 40);
    assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    // Stable for one path, different between paths.
    assert_eq!(a, shared_file(dir.path(), data.path()));
    assert_ne!(a, shared_file(other.path(), data.path()));
}
