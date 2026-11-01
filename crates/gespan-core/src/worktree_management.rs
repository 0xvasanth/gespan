//! Settings worktree guards. Removal keeps the branch and commits, unless the
//! branch is confirmed merged and the user asked for it to go too.
use crate::branch_merged::{self, Merged};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
const NOT_MERGED: &str = "This branch isn’t merged, so it was kept.";
const BRANCH_MOVED: &str = "This branch moved after it was checked, so it was kept.";
const REMOVE_RETRIES: usize = 10;
const REMOVE_RETRY_DELAY: Duration = Duration::from_millis(200);
pub fn default_root(main: &Path) -> PathBuf {
    main.with_file_name(format!(
        "{}-worktrees",
        main.file_name().unwrap_or_default().to_string_lossy()
    ))
}
pub fn status(cwd: &Path, tree: &crate::git::Worktree) -> (bool, Option<bool>, usize) {
    let locked = crate::git::git(cwd, &["worktree", "list", "--porcelain", "-z"])
        .ok()
        .is_some_and(|s| {
            s.split("\0\0").any(|block| {
                block.starts_with(&format!("worktree {}\0", tree.path))
                    && block
                        .split('\0')
                        .any(|v| v == "locked" || v.starts_with("locked "))
            })
        });
    let dirty = crate::git::git(
        Path::new(&tree.path),
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .ok()
    .map(|s| !s.is_empty());
    let unpushed = crate::git::git(
        Path::new(&tree.path),
        &["rev-list", "--count", "HEAD", "--not", "--remotes"],
    )
    .ok()
    .and_then(|s| s.trim().parse().ok())
    .unwrap_or(0);
    (locked, dirty, unpushed)
}
pub fn check(cwd: &Path, path: &Path) -> Result<crate::git::Worktree, String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = crate::files::path_key(&canonical.to_string_lossy());
    let tree = crate::git::snapshot(cwd)?
        .worktrees
        .into_iter()
        .find(|t| {
            let canonical = Path::new(&t.path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&t.path));
            crate::files::path_key(&canonical.to_string_lossy()) == key
        })
        .ok_or("This path is not a registered worktree of this repository")?;
    if tree.main {
        return Err("The main working copy cannot be deleted".into());
    }
    if tree.branch.is_none() {
        return Err("Create a branch for this detached worktree before deleting it.".into());
    }
    if status(cwd, &tree).0 {
        return Err("This worktree is locked. Unlock it in Git before deleting it.".into());
    }
    Ok(tree)
}
pub fn remove(cwd: &Path, path: &Path) -> Result<(), String> {
    let _span = tracing::info_span!("worktree_remove").entered();
    crate::diagnostics::metrics::track("worktree.remove", remove_checked(cwd, path))
        .inspect_err(|error| tracing::warn!(%error, "could not remove a worktree"))
}
fn remove_checked(cwd: &Path, path: &Path) -> Result<(), String> {
    let tree = check(cwd, path)?;
    crate::provider::discard_worktree(Path::new(&tree.path));
    crate::git::git(cwd, &["worktree", "remove", "--force", "--", &tree.path]).map(|_| ())
}
/// How the worktree's branch reached the default branch, if it has. A check
/// that can't finish reads as not merged, which keeps the branch.
pub fn merged(cwd: &Path, tree: &crate::git::Worktree) -> Option<Merged> {
    let branch = tree.branch.as_deref()?;
    branch_merged::merged(cwd, branch)
        .inspect_err(|error| tracing::debug!(%error, "could not check a branch"))
        .ok()
        .flatten()
}
/// Deletes a merged local branch, if it is still at the commit that was checked
/// and nothing has it checked out.
pub fn delete_merged_branch(cwd: &Path, branch: &str, checked: &str) -> Result<(), String> {
    let _span = tracing::info_span!("branch_delete").entered();
    crate::diagnostics::metrics::track("branch.delete", delete_checked(cwd, branch, checked))
        .inspect_err(|error| tracing::warn!(%error, "could not delete a branch"))
}
fn delete_checked(cwd: &Path, branch: &str, checked: &str) -> Result<(), String> {
    let merged = branch_merged::merged(cwd, branch)?.ok_or(NOT_MERGED)?;
    if merged.branch_sha != checked {
        return Err(BRANCH_MOVED.into());
    }
    let open = crate::git::snapshot(cwd)?
        .worktrees
        .iter()
        .any(|tree| tree.branch.as_deref() == Some(branch));
    if open {
        return Err("This branch is checked out in a worktree, so it was kept.".into());
    }
    delete_ref(cwd, branch, checked)
}
/// Git refuses unless the branch is still at `checked`, which closes the gap
/// between the last check and the deletion.
fn delete_ref(cwd: &Path, branch: &str, checked: &str) -> Result<(), String> {
    let reference = format!("refs/heads/{branch}");
    crate::git::git(cwd, &["update-ref", "-d", &reference, checked])
        .map(|_| ())
        .map_err(|_| BRANCH_MOVED.to_owned())
}
/// Removes a worktree and its branch together, when both are finished: clean,
/// and merged. Anything less keeps both.
pub fn remove_finished(cwd: &Path, path: &Path) -> Result<(), String> {
    let tree = check(cwd, path)?;
    if status(cwd, &tree).1 != Some(false) {
        return Err("Files in this worktree have uncommitted changes.".into());
    }
    let merged = merged(cwd, &tree).ok_or(NOT_MERGED)?;
    let branch = tree.branch.as_deref().ok_or(NOT_MERGED)?;
    remove_unused(cwd, path)?;
    delete_merged_branch(cwd, branch, &merged.branch_sha)
}

/// Whether nothing but `session` works in the worktree at `path`.
pub fn only_used_by(path: &str, session: i64, users: &[(i64, String)]) -> bool {
    only_used_by_any(path, &[session], users)
}
/// Whether nothing but `sessions` works in the worktree at `path`.
pub fn only_used_by_any(path: &str, sessions: &[i64], users: &[(i64, String)]) -> bool {
    let real = |path: &str| {
        Path::new(path)
            .canonicalize()
            .map_or_else(|_| path.to_owned(), |p| p.to_string_lossy().into_owned())
    };
    let root = real(path);
    users.iter().all(|(id, cwd)| {
        sessions.contains(id) || !crate::files::is_equal_or_inside(&real(cwd), &root)
    })
}
/// The worktree at `path`, if removing `session` would leave it unused.
pub fn unused_after(
    cwd: &Path,
    path: &Path,
    session: i64,
    users: &[(i64, String)],
) -> Option<crate::git::Worktree> {
    check(cwd, path)
        .ok()
        .filter(|tree| only_used_by(&tree.path, session, users))
}
/// Removes a worktree nobody asked to discard: uncommitted changes keep it.
pub fn remove_unused(cwd: &Path, path: &Path) -> Result<(), String> {
    let _span = tracing::info_span!("worktree_remove").entered();
    crate::diagnostics::metrics::track("worktree.remove", remove_clean(cwd, path))
        .inspect_err(|error| tracing::warn!(%error, "could not remove a worktree"))
}
fn remove_clean(cwd: &Path, path: &Path) -> Result<(), String> {
    let tree = check(cwd, path)?;
    if status(cwd, &tree).1 != Some(false) {
        return Err("Files in this worktree have uncommitted changes.".into());
    }
    crate::provider::discard_worktree(Path::new(&tree.path));
    let remove = || crate::git::git(cwd, &["worktree", "remove", "--", &tree.path]).map(|_| ());
    // A discarded agent exits on its own thread. Windows refuses to delete a
    // folder while a process still runs in it.
    for _ in 0..REMOVE_RETRIES {
        if remove().is_ok() {
            return Ok(());
        }
        std::thread::sleep(REMOVE_RETRY_DELAY);
    }
    remove()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| crate::git::git(dir.path(), args).unwrap();
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        git(&["config", "user.name", "Fixture"]);
        std::fs::write(dir.path().join("file"), "before").unwrap();
        git(&["add", "file"]);
        git(&["commit", "-m", "Initial"]);
        dir
    }
    #[test]
    fn removal_guards_and_branch_retention() {
        let dir = repo();
        assert!(check(dir.path(), dir.path()).unwrap_err().contains("main"));
        let tree = crate::git::create_named_worktree(dir.path(), "settings-parity").unwrap();
        let path = Path::new(&tree.path);
        std::fs::write(path.join("untracked"), "discard in disposable test").unwrap();
        assert_eq!(status(dir.path(), &tree).1, Some(true));
        crate::git::git(dir.path(), &["worktree", "lock", "--", &tree.path]).unwrap();
        assert!(check(dir.path(), path).unwrap_err().contains("locked"));
        crate::git::git(dir.path(), &["worktree", "unlock", "--", &tree.path]).unwrap();
        remove(dir.path(), path).unwrap();
        assert!(!path.exists());
        assert!(crate::git::git(
            dir.path(),
            &["show-ref", "--verify", "refs/heads/settings-parity"]
        )
        .is_ok());
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn removals_are_counted() {
        use crate::diagnostics::metrics::{capture, ErrorKind};
        let dir = repo();
        let (refused, counts) = capture(|| remove(dir.path(), dir.path()));
        assert!(refused.is_err());
        let failures: Vec<_> = counts
            .iter()
            .filter(|c| c.name == "worktree.remove")
            .map(|c| (c.kind, c.count))
            .collect();
        assert_eq!(failures, [(Some(ErrorKind::Other), 1)]);
        let tree = crate::git::create_named_worktree(dir.path(), "counted-removal").unwrap();
        let (removed, counts) = capture(|| remove(dir.path(), Path::new(&tree.path)));
        removed.unwrap();
        assert!(counts
            .iter()
            .any(|c| c.name == "worktree.remove" && c.kind.is_none() && c.count == 1));
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn detached_commits_cannot_be_orphaned() {
        let dir = repo();
        let path = dir.path().join("detached");
        crate::git::git(
            dir.path(),
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().unwrap(),
                "HEAD",
            ],
        )
        .unwrap();
        assert!(remove(dir.path(), &path).unwrap_err().contains("detached"));
        crate::git::git(
            dir.path(),
            &["worktree", "remove", "--force", path.to_str().unwrap()],
        )
        .unwrap();
    }

    #[test]
    fn only_used_by_ignores_the_session_itself_and_other_folders() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "shared").unwrap();
        let elsewhere = dir.path().to_string_lossy().into_owned();
        let inside = format!("{}/src", tree.path);
        std::fs::create_dir(&inside).unwrap();
        let unused = [(1, tree.path.clone()), (2, elsewhere)];
        assert!(only_used_by(&tree.path, 1, &unused));
        assert!(!only_used_by(&tree.path, 2, &unused), "1 still works there");
        let nested = [(1, tree.path.clone()), (2, inside)];
        assert!(!only_used_by(&tree.path, 1, &nested), "a subfolder counts");
        assert!(only_used_by(&tree.path, 1, &[]));
        remove(dir.path(), Path::new(&tree.path)).unwrap();
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn a_run_owns_a_worktree_only_when_no_outside_session_uses_it() {
        let users = [
            (1, "/repo-tree".to_owned()),
            (2, "/repo-tree/sub".to_owned()),
        ];
        assert!(only_used_by_any("/repo-tree", &[1, 2], &users));
        assert!(!only_used_by_any("/repo-tree", &[1], &users));
    }
    #[test]
    fn only_used_by_sees_the_same_folder_through_another_spelling() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "spelled").unwrap();
        let spelled = format!("{}/../spelled", tree.path);
        assert!(!only_used_by(&tree.path, 1, &[(2, spelled)]));
        remove(dir.path(), Path::new(&tree.path)).unwrap();
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn unused_after_offers_only_a_removable_branch_worktree() {
        let dir = repo();
        let main = dir.path();
        assert!(unused_after(main, main, 1, &[]).is_none(), "the main copy");
        let tree = crate::git::create_named_worktree(main, "offered").unwrap();
        let path = Path::new(&tree.path);
        assert!(unused_after(main, path, 1, &[(1, tree.path.clone())]).is_some());
        assert!(unused_after(main, path, 1, &[(2, tree.path.clone())]).is_none());
        crate::git::git(main, &["worktree", "lock", "--", &tree.path]).unwrap();
        assert!(unused_after(main, path, 1, &[]).is_none(), "locked");
        crate::git::git(main, &["worktree", "unlock", "--", &tree.path]).unwrap();
        remove(main, path).unwrap();
        assert!(unused_after(main, path, 1, &[]).is_none(), "already gone");
        let detached = main.join("detached");
        let spot = detached.to_str().unwrap();
        crate::git::git(main, &["worktree", "add", "--detach", spot, "HEAD"]).unwrap();
        assert!(unused_after(main, &detached, 1, &[]).is_none(), "detached");
        crate::git::git(main, &["worktree", "remove", "--force", spot]).unwrap();
        let _ = std::fs::remove_dir(default_root(main));
    }
    #[test]
    fn remove_unused_keeps_uncommitted_work_and_the_branch() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "careful").unwrap();
        let path = Path::new(&tree.path);
        std::fs::write(path.join("file"), "edited").unwrap();
        let refused = remove_unused(dir.path(), path).unwrap_err();
        assert!(refused.contains("uncommitted"), "{refused}");
        crate::git::git(path, &["checkout", "--", "file"]).unwrap();
        std::fs::write(path.join("untracked"), "keep me").unwrap();
        assert!(remove_unused(dir.path(), path).is_err());
        assert!(path.join("untracked").exists());
        std::fs::remove_file(path.join("untracked")).unwrap();

        remove_unused(dir.path(), path).unwrap();
        assert!(!path.exists());
        crate::git::git(dir.path(), &["show-ref", "--verify", "refs/heads/careful"]).unwrap();
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn remove_unused_clears_ignored_files_and_is_counted() {
        use crate::diagnostics::metrics::capture;
        let dir = repo();
        std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        crate::git::git(dir.path(), &["add", ".gitignore"]).unwrap();
        crate::git::git(dir.path(), &["commit", "-m", "Ignore"]).unwrap();
        let tree = crate::git::create_named_worktree(dir.path(), "built").unwrap();
        let path = Path::new(&tree.path);
        std::fs::create_dir(path.join("build")).unwrap();
        std::fs::write(path.join("build/out"), "cache").unwrap();
        let (removed, counts) = capture(|| remove_unused(dir.path(), path));
        removed.unwrap();
        assert!(!path.exists());
        assert!(counts
            .iter()
            .any(|c| c.name == "worktree.remove" && c.kind.is_none()));
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }

    /// A worktree on `name`, with one commit that `main` then squash-merges.
    fn merged_tree(dir: &Path, name: &str) -> crate::git::Worktree {
        let git = |cwd: &Path, args: &[&str]| crate::git::git(cwd, args).unwrap();
        let tree = crate::git::create_named_worktree(dir, name).unwrap();
        let path = Path::new(&tree.path);
        std::fs::write(path.join(name), "work").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "Work"]);
        git(dir, &["merge", "--squash", name]);
        git(dir, &["commit", "-m", "Squash"]);
        tree
    }
    fn branch_exists(dir: &Path, name: &str) -> bool {
        let reference = format!("refs/heads/{name}");
        crate::git::git(dir, &["show-ref", "--verify", &reference]).is_ok()
    }
    #[test]
    fn a_finished_worktree_goes_with_its_branch_in_one_action() {
        let dir = repo();
        let tree = merged_tree(dir.path(), "finished");
        assert!(merged(dir.path(), &tree).is_some());
        remove_finished(dir.path(), Path::new(&tree.path)).unwrap();
        assert!(!Path::new(&tree.path).exists());
        assert!(!branch_exists(dir.path(), "finished"));
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn an_unfinished_worktree_keeps_everything() {
        let dir = repo();
        let main = dir.path();
        let tree = crate::git::create_named_worktree(main, "unfinished").unwrap();
        let path = Path::new(&tree.path);
        std::fs::write(path.join("new"), "work").unwrap();
        crate::git::git(path, &["add", "."]).unwrap();
        crate::git::git(path, &["commit", "-m", "Work"]).unwrap();
        assert!(merged(main, &tree).is_none());
        let refused = remove_finished(main, path).unwrap_err();
        assert_eq!(refused, NOT_MERGED);
        assert!(path.exists());
        assert!(branch_exists(main, "unfinished"));
        remove(main, path).unwrap();
        let _ = std::fs::remove_dir(default_root(main));
    }
    #[test]
    fn dirty_locked_and_detached_worktrees_are_refused() {
        let dir = repo();
        let main = dir.path();
        let tree = merged_tree(main, "guarded");
        let path = Path::new(&tree.path);
        std::fs::write(path.join("untracked"), "keep me").unwrap();
        let refused = remove_finished(main, path).unwrap_err();
        assert!(refused.contains("uncommitted"), "{refused}");
        std::fs::remove_file(path.join("untracked")).unwrap();
        crate::git::git(main, &["worktree", "lock", "--", &tree.path]).unwrap();
        let refused = remove_finished(main, path).unwrap_err();
        assert!(refused.contains("locked"), "{refused}");
        crate::git::git(main, &["worktree", "unlock", "--", &tree.path]).unwrap();
        crate::git::git(path, &["checkout", "--detach"]).unwrap();
        let refused = remove_finished(main, path).unwrap_err();
        assert!(refused.contains("detached"), "{refused}");
        assert!(path.exists());
        assert!(branch_exists(main, "guarded"));
        crate::git::git(main, &["worktree", "remove", "--force", "--", &tree.path]).unwrap();
        let _ = std::fs::remove_dir(default_root(main));
    }
    #[test]
    fn a_branch_that_moved_after_the_check_is_kept() {
        let dir = repo();
        let main = dir.path();
        let tree = merged_tree(main, "moving");
        let path = Path::new(&tree.path);
        let checked = merged(main, &tree).unwrap().branch_sha;
        std::fs::write(path.join("more"), "more work").unwrap();
        crate::git::git(path, &["add", "."]).unwrap();
        crate::git::git(path, &["commit", "-m", "More"]).unwrap();
        let moved = crate::git::git(main, &["rev-parse", "moving"]).unwrap();
        remove_unused(main, path).unwrap();
        // Still merged-looking checks can't hide the move: it is refused first
        // as unmerged, and Git itself refuses the stale commit.
        assert_eq!(
            delete_merged_branch(main, "moving", &checked),
            Err(NOT_MERGED.into())
        );
        assert_eq!(
            delete_ref(main, "moving", &checked),
            Err(BRANCH_MOVED.into())
        );
        assert_eq!(
            crate::git::git(main, &["rev-parse", "moving"]).unwrap(),
            moved
        );
        let _ = std::fs::remove_dir(default_root(main));
    }
    #[test]
    fn a_merged_branch_at_a_new_commit_is_kept_too() {
        let dir = repo();
        let main = dir.path();
        crate::git::git(main, &["branch", "stale"]).unwrap();
        let checked = crate::git::git(main, &["rev-parse", "stale"]).unwrap();
        std::fs::write(main.join("file"), "after").unwrap();
        crate::git::git(main, &["commit", "-am", "Move main"]).unwrap();
        crate::git::git(main, &["branch", "-f", "stale", "main"]).unwrap();
        assert_eq!(
            delete_merged_branch(main, "stale", checked.trim()),
            Err(BRANCH_MOVED.into())
        );
        assert!(branch_exists(main, "stale"));
    }
    #[test]
    fn a_checked_out_branch_is_kept() {
        let dir = repo();
        let main = dir.path();
        let tree = merged_tree(main, "open");
        let checked = merged(main, &tree).unwrap().branch_sha;
        let refused = delete_merged_branch(main, "open", &checked).unwrap_err();
        assert!(refused.contains("checked out"), "{refused}");
        assert!(branch_exists(main, "open"));
        remove(main, Path::new(&tree.path)).unwrap();
        let _ = std::fs::remove_dir(default_root(main));
    }
    #[test]
    fn a_new_worktree_gets_its_included_files() {
        let dir = repo();
        let main = dir.path();
        std::fs::write(main.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(main.join(".worktreeinclude"), ".env\n").unwrap();
        crate::git::git(main, &["add", "."]).unwrap();
        crate::git::git(main, &["commit", "-m", "Rules"]).unwrap();
        std::fs::write(main.join(".env"), "local").unwrap();
        let tree = crate::git::create_named_worktree(main, "seeded").unwrap();
        assert!(Path::new(&tree.path).join(".env").exists());
        let session = crate::git::create_worktree(main, "HEAD", 7).unwrap();
        assert!(session.join(".env").exists());
        remove(main, Path::new(&tree.path)).unwrap();
        remove(main, &session).unwrap();
        let _ = std::fs::remove_dir(default_root(main));
    }

    #[test]
    fn a_refused_worktree_removal_is_logged() {
        let dir = repo();
        let mut removed = Ok(());
        let logged = crate::diagnostics::test_capture(|| removed = remove(dir.path(), dir.path()));
        assert!(removed.is_err());
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert_eq!(logged[0]["level"], "warn");
        assert_eq!(logged[0]["span"], "worktree_remove");
        assert!(logged[0]["fields"]["error"]
            .as_str()
            .unwrap()
            .contains("main"));

        let tree = crate::git::create_named_worktree(dir.path(), "short-lived").unwrap();
        let quiet = crate::diagnostics::test_capture(|| {
            remove(dir.path(), Path::new(&tree.path)).unwrap();
        });
        assert!(quiet.is_empty(), "{quiet:?}");
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
}
