use super::*;

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}
fn git(root: &Path, args: &[&str]) {
    run_git(root, args).unwrap();
}
fn repo() -> Repo {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    git(&root, &["config", "user.name", "Fixture"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), "a\n").unwrap();
    std::fs::write(root.join("src/gone.txt"), "gone\n").unwrap();
    std::fs::write(root.join("docs.txt"), "docs\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "Initial"]);
    Repo { _dir: dir, root }
}
fn read(root: impl AsRef<Path>, path: &str) -> String {
    std::fs::read_to_string(root.as_ref().join(path)).unwrap()
}
fn all() -> Vec<String> {
    vec![".".to_owned()]
}

#[test]
fn a_worker_starts_with_the_leads_uncommitted_contents() {
    let repo = repo();
    std::fs::write(repo.root.join("src/a.txt"), "edited\n").unwrap();
    std::fs::write(repo.root.join("src/new.txt"), "untracked\n").unwrap();
    std::fs::remove_file(repo.root.join("docs.txt")).unwrap();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    assert_eq!(workspace.branch, "gespan/orch-t1");
    assert_eq!(read(&workspace.path, "src/a.txt"), "edited\n");
    assert_eq!(read(&workspace.path, "src/new.txt"), "untracked\n");
    assert!(!Path::new(&workspace.path).join("docs.txt").exists());
    assert!(unchanged(&workspace, "t1").unwrap(), "seeding is not work");
}

#[test]
fn setup_sees_the_seeded_files_and_what_it_writes_is_not_the_workers_delta() {
    let repo = repo();
    std::fs::write(repo.root.join("docs.txt"), "edited\n").unwrap();
    let scripts = crate::worktree_scripts::Scripts {
        setup: "echo setup>> docs.txt".into(),
        teardown: String::new(),
    };
    crate::worktree_scripts::save_scripts(&repo.root, &scripts).unwrap();
    let workspace = prepare(&repo.root, "setup").unwrap().unwrap();
    let seen = read(&workspace.path, "docs.txt");
    assert!(seen.starts_with("edited"), "{seen}");
    assert!(seen.trim_end().ends_with("setup"), "{seen}");
    assert!(unchanged(&workspace, "setup").unwrap());
    let integrated = integrate(&repo.root, &workspace, "setup", &all()).unwrap();
    assert!(integrated.files.is_empty());
    assert_eq!(read(&repo.root, "docs.txt"), "edited\n");
    remove(&repo.root, &workspace, "setup").unwrap();
}

#[test]
fn a_task_can_try_again_after_its_setup_fails() {
    let repo = repo();
    let scripts = |setup: &str| crate::worktree_scripts::Scripts {
        setup: setup.into(),
        teardown: String::new(),
    };
    crate::worktree_scripts::save_scripts(&repo.root, &scripts("exit 2")).unwrap();
    let error = prepare(&repo.root, "again").unwrap_err();
    assert!(error.contains("Setup script failed"), "{error}");
    assert_eq!(crate::git::worktrees(&repo.root).unwrap().len(), 1);
    crate::worktree_scripts::save_scripts(&repo.root, &scripts("echo ok")).unwrap();
    let workspace = prepare(&repo.root, "again").unwrap().unwrap();
    remove(&repo.root, &workspace, "again").unwrap();
}

#[test]
fn a_retry_with_no_setup_does_not_inherit_the_failed_record() {
    let repo = repo();
    let scripts = crate::worktree_scripts::Scripts {
        setup: "exit 2".into(),
        teardown: String::new(),
    };
    crate::worktree_scripts::save_scripts(&repo.root, &scripts).unwrap();
    prepare(&repo.root, "record").unwrap_err();
    crate::worktree_scripts::save_scripts(&repo.root, &Default::default()).unwrap();
    let workspace = prepare(&repo.root, "record").unwrap().unwrap();
    let record = crate::worktree_scripts::record(Path::new(&workspace.path));
    assert_eq!(record, None);
    remove(&repo.root, &workspace, "record").unwrap();
}

#[test]
#[cfg(unix)]
fn a_task_can_try_again_after_seeding_fails() {
    let repo = repo();
    let link = repo.root.join("link");
    std::os::unix::fs::symlink("docs.txt", &link).unwrap();
    let error = prepare(&repo.root, "seed").unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
    assert_eq!(crate::git::worktrees(&repo.root).unwrap().len(), 1);
    std::fs::remove_file(&link).unwrap();
    let workspace = prepare(&repo.root, "seed").unwrap().unwrap();
    remove(&repo.root, &workspace, "seed").unwrap();
}

#[test]
fn a_call_that_loses_the_race_leaves_the_winners_checkout() {
    let repo = repo();
    // Held so both calls get past their scan before either can make anything.
    let held = crate::worktree_scripts::turn(&repo.root);
    let calls: Vec<_> = (0..2)
        .map(|_| {
            let root = repo.root.clone();
            std::thread::spawn(move || prepare(&root, "race"))
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(held);
    let (won, lost): (Vec<_>, Vec<_>) = calls
        .into_iter()
        .map(|call| call.join().unwrap())
        .partition(Result::is_ok);
    assert_eq!((won.len(), lost.len()), (1, 1), "{lost:?}");
    let workspace = won[0].clone().unwrap().unwrap();
    assert!(Path::new(&workspace.path).is_dir());
    assert_eq!(crate::git::worktrees(&repo.root).unwrap().len(), 2);
    remove(&repo.root, &workspace, "race").unwrap();
}

#[test]
fn integrate_applies_edits_additions_and_deletions_to_the_lead() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    let worker = Path::new(&workspace.path);
    std::fs::write(worker.join("src/a.txt"), "changed\n").unwrap();
    std::fs::write(worker.join("src/b.txt"), "added\n").unwrap();
    std::fs::remove_file(worker.join("src/gone.txt")).unwrap();
    assert!(!unchanged(&workspace, "t1").unwrap());
    let result = integrate(&repo.root, &workspace, "t1", &["src".to_owned()]).unwrap();
    assert_eq!(result.files, ["src/a.txt", "src/b.txt", "src/gone.txt"]);
    assert_eq!(result.already_applied, 0);
    assert_eq!(read(&repo.root, "src/a.txt"), "changed\n");
    assert_eq!(read(&repo.root, "src/b.txt"), "added\n");
    assert!(!repo.root.join("src/gone.txt").exists());
}

#[test]
fn integrating_twice_is_safe_and_finishes_a_partial_attempt() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    let worker = Path::new(&workspace.path);
    std::fs::write(worker.join("src/a.txt"), "changed\n").unwrap();
    std::fs::write(worker.join("src/b.txt"), "added\n").unwrap();
    // An earlier attempt stopped after writing one of the two files.
    std::fs::write(repo.root.join("src/a.txt"), "changed\n").unwrap();
    let result = integrate(&repo.root, &workspace, "t1", &all()).unwrap();
    assert_eq!(result.already_applied, 1);
    assert_eq!(read(&repo.root, "src/b.txt"), "added\n");
    let again = integrate(&repo.root, &workspace, "t1", &all()).unwrap();
    assert_eq!(again.already_applied, 2);
}

#[test]
fn a_file_the_lead_changed_is_never_overwritten_and_nothing_is_half_applied() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    let worker = Path::new(&workspace.path);
    std::fs::write(worker.join("docs.txt"), "worker docs\n").unwrap();
    std::fs::write(worker.join("src/a.txt"), "worker\n").unwrap();
    std::fs::write(repo.root.join("src/a.txt"), "lead's own edit\n").unwrap();
    let error = integrate(&repo.root, &workspace, "t1", &all()).unwrap_err();
    assert!(error.contains("the lead checkout changed"), "{error}");
    assert!(error.contains("kept"), "{error}");
    assert_eq!(read(&repo.root, "src/a.txt"), "lead's own edit\n");
    assert_eq!(
        read(&repo.root, "docs.txt"),
        "docs\n",
        "checked before writing"
    );
}

#[test]
fn a_change_outside_the_tasks_files_is_refused() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    std::fs::write(Path::new(&workspace.path).join("docs.txt"), "out\n").unwrap();
    let error = integrate(&repo.root, &workspace, "t1", &["src".to_owned()]).unwrap_err();
    assert!(error.contains("outside this task's files"), "{error}");
    assert_eq!(read(&repo.root, "docs.txt"), "docs\n");
}

#[test]
fn a_moved_branch_stops_integration() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    std::fs::write(Path::new(&workspace.path).join("src/a.txt"), "w\n").unwrap();
    std::fs::write(repo.root.join("other.txt"), "x\n").unwrap();
    git(&repo.root, &["add", "other.txt"]);
    git(&repo.root, &["commit", "-m", "Moved"]);
    let error = integrate(&repo.root, &workspace, "t1", &all()).unwrap_err();
    assert!(error.contains("branch moved"), "{error}");
}

#[test]
fn binary_contents_survive_the_round_trip() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    let bytes = vec![0u8, 159, 146, 150, 255, 10, 13, 10];
    std::fs::write(Path::new(&workspace.path).join("src/blob.bin"), &bytes).unwrap();
    integrate(&repo.root, &workspace, "t1", &all()).unwrap();
    assert_eq!(
        std::fs::read(repo.root.join("src/blob.bin")).unwrap(),
        bytes
    );
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_target_path_is_refused() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    let worker = Path::new(&workspace.path);
    std::fs::create_dir(worker.join("linked")).unwrap();
    std::fs::write(worker.join("linked/file.txt"), "w\n").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.root.join("linked")).unwrap();
    let error = integrate(&repo.root, &workspace, "t1", &all()).unwrap_err();
    assert!(error.contains("symbolic link"), "{error}");
    assert!(!outside.path().join("file.txt").exists());
}

#[cfg(unix)]
#[test]
fn an_executable_stays_executable() {
    use std::os::unix::fs::PermissionsExt;
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    let script = Path::new(&workspace.path).join("src/run.sh");
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    crate::test_exec::make_executable(&script, 0o755);
    integrate(&repo.root, &workspace, "t1", &all()).unwrap();
    let mode = std::fs::metadata(repo.root.join("src/run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[test]
fn remove_clears_the_checkout_branch_and_snapshots() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    remove(&repo.root, &workspace, "t1").unwrap();
    assert!(!Path::new(&workspace.path).exists());
    assert!(run_git(
        &repo.root,
        &["rev-parse", "--verify", "refs/heads/gespan/orch-t1"]
    )
    .is_err());
    assert!(run_git(
        &repo.root,
        &["rev-parse", "--verify", "refs/gespan/orch/t1/base"]
    )
    .is_err());
    remove(&repo.root, &workspace, "t1").unwrap();
}

#[test]
fn a_branch_that_is_not_an_orchestration_branch_is_never_deleted() {
    let repo = repo();
    let workspace = prepare(&repo.root, "t1").unwrap().unwrap();
    git(&repo.root, &["branch", "keep-me"]);
    let mine = Workspace {
        path: workspace.path,
        branch: "keep-me".into(),
    };
    remove(&repo.root, &mine, "t1").unwrap();
    assert!(run_git(&repo.root, &["rev-parse", "--verify", "refs/heads/keep-me"]).is_ok());
}

#[test]
fn prepare_finds_a_crashed_runs_checkout_again() {
    let repo = repo();
    let first = prepare(&repo.root, "t1").unwrap().unwrap();
    std::fs::write(Path::new(&first.path).join("src/a.txt"), "kept\n").unwrap();
    let second = prepare(&repo.root, "t1").unwrap().unwrap();
    assert_eq!(second.path, first.path);
    assert_eq!(read(&second.path, "src/a.txt"), "kept\n");
}

#[test]
fn a_folder_without_git_or_commits_shares_the_leads_checkout() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(prepare(dir.path(), "t1").unwrap(), None);
    git(dir.path(), &["init", "-b", "main"]);
    assert_eq!(prepare(dir.path(), "t1").unwrap(), None);
}
