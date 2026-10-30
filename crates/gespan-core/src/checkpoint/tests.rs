use super::*;

struct Fixture {
    dir: PathBuf,
    repo: PathBuf,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("gespan-turn-test-{}", unique_id()));
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init"]).unwrap();
        run_git(&repo, &["config", "user.name", "Test"]).unwrap();
        run_git(&repo, &["config", "user.email", "test@example.invalid"]).unwrap();
        run_git(&repo, &["config", "core.autocrlf", "false"]).unwrap();
        run_git(&repo, &["config", "core.safecrlf", "false"]).unwrap();
        run_git(&repo, &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(repo.join("a.txt"), "original\n").unwrap();
        std::fs::write(repo.join("delete.txt"), "remove me\n").unwrap();
        run_git(&repo, &["add", "."]).unwrap();
        run_git(&repo, &["commit", "-m", "initial"]).unwrap();
        Self {
            store: Store::new(dir.join("checkpoints")),
            dir,
            repo,
        }
    }
    fn cwd(&self) -> &str {
        self.repo.to_str().unwrap()
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.repo.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn records_shell_changes_and_preserves_the_index_head_and_saved_result() {
    let f = Fixture::new();
    f.write("a.txt", "user change\n");
    f.write("user.txt", "existing untracked work\n");
    run_git(&f.repo, &["add", "a.txt"]).unwrap();
    let index = std::fs::read(f.repo.join(".git/index")).unwrap();
    let head = run_git(&f.repo, &["rev-parse", "HEAD"]).unwrap();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    // No structured edit events: the actual file state is authoritative.
    f.write("a.txt", "user change\nagent change\n");
    f.write("new.txt", "one\ntwo\n");
    std::fs::remove_file(f.repo.join("delete.txt")).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    let status = f.store.status("session", f.cwd()).unwrap();
    assert_eq!(
        status
            .files
            .iter()
            .map(|file| file.relative.as_str())
            .collect::<Vec<_>>(),
        ["a.txt", "delete.txt", "new.txt"]
    );
    assert_eq!(
        (status.files[0].additions, status.files[0].deletions),
        (1, 0)
    );
    assert!(status.files.iter().all(|file| file.exact && file.undoable));
    assert_eq!(std::fs::read(f.repo.join(".git/index")).unwrap(), index);
    assert_eq!(run_git(&f.repo, &["rev-parse", "HEAD"]).unwrap(), head);

    f.write("a.txt", "a later editor change\n");
    // Reopening the store proves the review survives a restart and never reads
    // later contents as though they were this turn's result.
    let reopened = Store::new(f.dir.join("checkpoints"));
    let diff = reopened.file_diff("session", f.cwd(), "a.txt").unwrap();
    assert_eq!(diff.original, "user change\n");
    assert_eq!(diff.current, "user change\nagent change\n");
    let status = reopened.status("session", f.cwd()).unwrap();
    assert!(status.files[0].exact);
    assert!(!status.files[0].undoable);
    assert!(reopened.undo("session", f.cwd(), None).is_err());
    assert!(f.repo.join("new.txt").exists());
    assert!(!f.repo.join("delete.txt").exists());
}

#[test]
fn each_turn_has_a_fresh_baseline_and_noop_turns_clear_the_old_card() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "first turn\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "external edit between turns\n");
    f.store.begin_turn("session", f.cwd(), "second").unwrap();
    f.write("a.txt", "external edit between turns\nsecond turn\n");
    f.store.finish_turn("session", f.cwd(), "second").unwrap();
    let diff = f.store.file_diff("session", f.cwd(), "a.txt").unwrap();
    assert_eq!(diff.original, "external edit between turns\n");
    assert_eq!(diff.current, "external edit between turns\nsecond turn\n");
    let status = f.store.status("session", f.cwd()).unwrap();
    assert_eq!(
        (status.files[0].additions, status.files[0].deletions),
        (1, 0)
    );
    f.store.begin_turn("session", f.cwd(), "third").unwrap();
    f.store.finish_turn("session", f.cwd(), "third").unwrap();
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    assert!(f.dir.join("checkpoints/session/turns/first.json").exists());
    assert!(run_git(
        &f.repo,
        &["rev-parse", &turn_ref("session", "first", "after")]
    )
    .is_ok());
}

#[test]
fn continuing_an_unfinished_turn_preserves_its_baseline_and_edit_evidence() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.store.begin_turn("other", f.cwd(), "other").unwrap();
    f.write("a.txt", "session edit\n");
    f.store
        .capture("session", f.cwd(), &["a.txt".into()])
        .unwrap();
    f.write("foreign.txt", "other session\n");
    // The UI reloads before it can finish the first checkpoint, then continues.
    let reopened = Store::new(f.dir.join("checkpoints"));
    reopened.begin_turn("session", f.cwd(), "continue").unwrap();
    reopened
        .finish_turn("session", f.cwd(), "continue")
        .unwrap();
    let status = reopened.status("session", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "a.txt");
    assert!(!status.files[0].undoable);
    let diff = reopened.file_diff("session", f.cwd(), "a.txt").unwrap();
    assert_eq!(diff.original, "original\n");
    assert_eq!(diff.current, "session edit\n");
}

#[test]
fn recovers_edits_when_an_older_version_replaced_an_unfinished_baseline() {
    for committed in [false, true] {
        let f = Fixture::new();
        f.store.begin_turn("session", f.cwd(), "first").unwrap();
        f.write("a.txt", "session edit\n");
        f.write("foreign.txt", "other session\n");
        // Simulate the old Continue path losing the active metadata, but
        // leaving its original Git-backed baseline reference intact.
        let mut review = read_review(&f.store.session_dir("session"))
            .unwrap()
            .unwrap();
        review.active = None;
        write_review(&f.store.session_dir("session"), &review).unwrap();
        if committed {
            run_git(&f.repo, &["add", "a.txt"]).unwrap();
            run_git(&f.repo, &["commit", "-m", "accept edit"]).unwrap();
        }
        f.store.begin_turn("session", f.cwd(), "continue").unwrap();
        f.store.finish_turn("session", f.cwd(), "continue").unwrap();
        assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
        f.store
            .recover_orphaned_turn("session", f.cwd(), &["removed-fixture.txt".into()])
            .unwrap();
        assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
        f.store
            .recover_orphaned_turn("session", f.cwd(), &["a.txt".into()])
            .unwrap();
        let reopened = Store::new(f.dir.join("checkpoints"));
        let status = reopened.status("session", f.cwd()).unwrap();
        if committed {
            assert!(status.files.is_empty());
        } else {
            assert_eq!(status.files.len(), 1);
            assert_eq!(status.files[0].relative, "a.txt");
            assert!(!status.files[0].undoable);
            let diff = reopened.file_diff("session", f.cwd(), "a.txt").unwrap();
            assert_eq!(diff.original, "original\n");
            assert_eq!(diff.current, "session edit\n");
        }
    }
}

#[test]
fn overlapping_turns_keep_exact_diffs_but_cannot_restore_shared_work() {
    let f = Fixture::new();
    f.write("a.txt", "top\n\nbottom\n");
    f.store.begin_turn("a", f.cwd(), "first").unwrap();
    f.store.begin_turn("b", f.cwd(), "second").unwrap();
    f.write("a.txt", "top edited by a\n\nbottom edited by b\n");
    for id in ["a", "b"] {
        f.store.capture(id, f.cwd(), &["a.txt".into()]).unwrap();
    }
    f.store.finish_turn("a", f.cwd(), "first").unwrap();
    f.store.finish_turn("b", f.cwd(), "second").unwrap();
    for id in ["a", "b"] {
        let status = f.store.status(id, f.cwd()).unwrap();
        assert_eq!(
            (status.files[0].additions, status.files[0].deletions),
            (2, 2)
        );
        assert!(status.files[0].exact);
        assert!(!status.files[0].undoable);
        assert!(f.store.undo(id, f.cwd(), None).is_err());
        assert_eq!(
            f.store.file_diff(id, f.cwd(), "a.txt").unwrap().current,
            "top edited by a\n\nbottom edited by b\n"
        );
    }
}

#[test]
fn abandoned_turns_from_exited_processes_do_not_share_later_turns() {
    let f = Fixture::new();
    f.store.begin_turn("crashed", f.cwd(), "first").unwrap();
    f.store.begin_turn("legacy", f.cwd(), "second").unwrap();
    let crashed = f.store.session_dir("crashed");
    let mut review = read_review(&crashed).unwrap().unwrap();
    review
        .active
        .as_mut()
        .unwrap()
        .owner
        .as_mut()
        .unwrap()
        .launch = "exited".into();
    write_review(&crashed, &review).unwrap();
    let legacy = f.store.session_dir("legacy");
    let mut review = read_review(&legacy).unwrap().unwrap();
    review.active.as_mut().unwrap().owner = None;
    write_review(&legacy, &review).unwrap();

    f.store.begin_turn("live", f.cwd(), "third").unwrap();
    f.write("a.txt", "edited from the shell\n");
    f.store.finish_turn("live", f.cwd(), "third").unwrap();
    let status = f.store.status("live", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "a.txt");
    assert!(status.files[0].undoable);
}

#[test]
fn shared_turns_only_review_files_with_their_own_successful_edit_events() {
    let f = Fixture::new();
    f.store.begin_turn("reader", f.cwd(), "first").unwrap();
    f.store.begin_turn("writer", f.cwd(), "second").unwrap();
    f.write("a.txt", "edited by the writer\n");
    f.store
        .capture("writer", f.cwd(), &["a.txt".into()])
        .unwrap();
    // The reader's temporary test fixture is removed before it finishes.
    f.write("temporary.txt", "temporary test\n");
    f.store
        .capture("reader", f.cwd(), &["temporary.txt".into()])
        .unwrap();
    std::fs::remove_file(f.repo.join("temporary.txt")).unwrap();
    f.store.finish_turn("reader", f.cwd(), "first").unwrap();
    f.store.finish_turn("writer", f.cwd(), "second").unwrap();

    let reopened = Store::new(f.dir.join("checkpoints"));
    assert!(reopened.status("reader", f.cwd()).unwrap().files.is_empty());
    let status = reopened.status("writer", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "a.txt");
    assert!(!status.files[0].undoable);
    // The complete workspace recording is retained separately from attribution.
    assert_eq!(
        reopened
            .file_diff("reader", f.cwd(), "a.txt")
            .unwrap()
            .current,
        "edited by the writer\n"
    );
}

#[test]
fn shared_turns_with_different_edits_do_not_claim_each_others_files() {
    let f = Fixture::new();
    f.store.begin_turn("a", f.cwd(), "first").unwrap();
    f.store.begin_turn("b", f.cwd(), "second").unwrap();
    f.write("a.txt", "edited by a\n");
    // Windows strips trailing spaces from file names, so only the leading
    // space can be exercised there.
    let literal = if cfg!(windows) {
        " literal name .txt"
    } else {
        " literal name .txt "
    };
    f.write(literal, "edited by b\n");
    f.store.capture("a", f.cwd(), &["a.txt".into()]).unwrap();
    f.store
        .capture(
            "b",
            f.cwd(),
            &[f.repo.join(literal).to_string_lossy().into()],
        )
        .unwrap();
    f.store.finish_turn("a", f.cwd(), "first").unwrap();
    f.store.finish_turn("b", f.cwd(), "second").unwrap();
    for (id, relative) in [("a", "a.txt"), ("b", literal)] {
        let status = f.store.status(id, f.cwd()).unwrap();
        assert_eq!(status.files.len(), 1);
        assert_eq!(status.files[0].relative, relative);
    }
}

#[test]
fn old_shared_recordings_without_edit_evidence_do_not_claim_workspace_changes() {
    let f = Fixture::new();
    f.store.begin_turn("reader", f.cwd(), "first").unwrap();
    f.store.begin_turn("writer", f.cwd(), "second").unwrap();
    f.write("a.txt", "foreign change\n");
    f.store.finish_turn("reader", f.cwd(), "first").unwrap();
    let path = f.dir.join("checkpoints/reader/turn-review.json");
    let mut review: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    review["latest"].as_object_mut().unwrap().remove("edited");
    std::fs::write(path, serde_json::to_vec(&review).unwrap()).unwrap();
    let reopened = Store::new(f.dir.join("checkpoints"));
    assert!(reopened.status("reader", f.cwd()).unwrap().files.is_empty());
}

#[test]
fn nested_turns_share_work_but_sibling_path_prefixes_do_not() {
    let f = Fixture::new();
    f.write("nested/a.txt", "child baseline\n");
    f.write("nested-other/a.txt", "sibling baseline\n");
    let child = f.repo.join("nested");
    let sibling = f.repo.join("nested-other");
    let child_cwd = child.to_str().unwrap();
    let sibling_cwd = sibling.to_str().unwrap();

    f.store.begin_turn("child", child_cwd, "first").unwrap();
    f.store.begin_turn("sibling", sibling_cwd, "first").unwrap();
    f.write("nested/a.txt", "independent child change\n");
    f.store.finish_turn("child", child_cwd, "first").unwrap();
    let status = f.store.status("child", child_cwd).unwrap();
    assert_eq!(status.files.len(), 1);
    assert!(status.files[0].undoable);
    f.store
        .finish_turn("sibling", sibling_cwd, "first")
        .unwrap();

    f.store.begin_turn("parent", f.cwd(), "second").unwrap();
    f.store.begin_turn("child", child_cwd, "second").unwrap();
    f.write("nested/a.txt", "shared parent and child change\n");
    f.store
        .capture("parent", f.cwd(), &["nested/a.txt".into()])
        .unwrap();
    f.store
        .capture("child", child_cwd, &["a.txt".into()])
        .unwrap();
    f.store.finish_turn("parent", f.cwd(), "second").unwrap();
    f.store.finish_turn("child", child_cwd, "second").unwrap();
    for (id, cwd) in [("parent", f.cwd()), ("child", child_cwd)] {
        let status = f.store.status(id, cwd).unwrap();
        assert_eq!(status.files.len(), 1);
        assert!(status.files[0].exact);
        assert!(!status.files[0].undoable);
        assert!(f.store.undo(id, cwd, None).is_err());
    }
}

#[test]
fn undo_restores_only_recorded_files_and_keep_retains_history() {
    let f = Fixture::new();
    f.write("user.txt", "user baseline\n");
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "agent\n");
    f.write("user.txt", "user baseline\nagent addition\n");
    f.write("new.txt", "created\n");
    std::fs::remove_file(f.repo.join("delete.txt")).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    f.write("unrelated.txt", "later work\n");
    let index = std::fs::read(f.repo.join(".git/index")).unwrap();
    f.store.keep("session", f.cwd(), Some("a.txt")).unwrap();
    f.store.undo("session", f.cwd(), None).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "agent\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.repo.join("user.txt")).unwrap(),
        "user baseline\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.repo.join("delete.txt")).unwrap(),
        "remove me\n"
    );
    assert!(f.repo.join("unrelated.txt").exists());
    assert!(!f.repo.join("new.txt").exists());
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    assert_eq!(std::fs::read(f.repo.join(".git/index")).unwrap(), index);
    assert!(f.dir.join("checkpoints/session/turns/first.json").exists());
    f.store.forget("session").unwrap();
    assert!(run_git(
        &f.repo,
        &["for-each-ref", "refs/gespan/checkpoints/session/"]
    )
    .unwrap()
    .is_empty());
}

#[test]
fn failures_and_incomplete_turns_never_fall_back_to_the_previous_result() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "first\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    f.store.begin_turn("session", f.cwd(), "second").unwrap();
    assert!(f
        .store
        .status("session", f.cwd())
        .unwrap_err()
        .contains("without a final"));
    // Stale completion from a cancelled turn must not settle a newer turn.
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    assert!(f.store.status("session", f.cwd()).is_err());
    run_git(&f.repo, &["config", "core.sparseCheckout", "true"]).unwrap();
    assert!(f.store.finish_turn("session", f.cwd(), "second").is_err());
    assert!(f
        .store
        .status("session", f.cwd())
        .unwrap_err()
        .contains("sparse"));
    run_git(&f.repo, &["config", "core.sparseCheckout", "false"]).unwrap();
    f.store.begin_turn("session", f.cwd(), "third").unwrap();
    f.store.finish_turn("session", f.cwd(), "third").unwrap();
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
}

#[test]
fn records_empty_binary_and_unusual_paths_without_the_old_file_limit() {
    let f = Fixture::new();
    f.write(".gitignore", "ignored/\n");
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("ignored/file.txt", "ignored\n");
    for index in 0..501 {
        f.write(&format!("new/{index}.txt"), "");
    }
    f.write("binary.bin", "\0binary");
    let path = if cfg!(windows) {
        "café name.txt"
    } else {
        "café\tname\n.txt"
    };
    f.write(path, "new\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    let status = f.store.status("session", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 503);
    assert!(status
        .files
        .iter()
        .any(|file| file.relative == path && file.additions == 1));
    assert!(
        f.store
            .file_diff("session", f.cwd(), "binary.bin")
            .unwrap()
            .binary
    );
    assert_eq!(
        f.store.file_diff("session", f.cwd(), path).unwrap().current,
        "new\n"
    );
}

#[test]
fn commits_during_the_turn_clear_review_but_preserve_the_recorded_diff() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "committed during the turn\n");
    run_git(&f.repo, &["add", "."]).unwrap();
    run_git(&f.repo, &["commit", "-m", "agent changes"]).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    assert!(run_git(&f.repo, &["status", "--porcelain"])
        .unwrap()
        .is_empty());
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    let diff = f.store.file_diff("session", f.cwd(), "a.txt").unwrap();
    assert_eq!(diff.original, "original\n");
    assert_eq!(diff.current, "committed during the turn\n");
}

#[test]
fn commits_after_the_turn_clear_modified_created_and_deleted_files() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "agent change\n");
    f.write("new.txt", "new file\n");
    std::fs::remove_file(f.repo.join("delete.txt")).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    assert_eq!(f.store.status("session", f.cwd()).unwrap().files.len(), 3);

    // The commit can include further edits made after the recorded turn.
    f.write("a.txt", "agent change\nuser addition\n");
    run_git(&f.repo, &["add", "-A"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "accept changes"]).unwrap();
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    assert_eq!(
        f.store
            .file_diff("session", f.cwd(), "a.txt")
            .unwrap()
            .current,
        "agent change\n"
    );
    assert!(f.dir.join("checkpoints/session/turns/first.json").exists());
}

#[test]
fn unrelated_and_partial_commits_preserve_pending_files_and_exact_counts() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "agent change\n");
    f.write("new.txt", "new file\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();

    f.write("unrelated.txt", "other work\n");
    run_git(&f.repo, &["add", "unrelated.txt"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "unrelated work"]).unwrap();
    assert_eq!(f.store.status("session", f.cwd()).unwrap().files.len(), 2);

    run_git(&f.repo, &["add", "a.txt"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "accept one file"]).unwrap();
    let status = f.store.status("session", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "new.txt");
    assert_eq!(
        (status.files[0].additions, status.files[0].deletions),
        (1, 0)
    );
    assert!(status.files[0].exact);
    assert!(!status.files[0].undoable);
}

#[test]
fn committing_only_part_of_a_files_changes_keeps_it_pending() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "first change\nsecond change\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "first change\n");
    run_git(&f.repo, &["add", "a.txt"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "accept part of the file"]).unwrap();
    f.write("a.txt", "first change\nsecond change\n");
    let status = f.store.status("session", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "a.txt");
    assert_eq!(
        (status.files[0].additions, status.files[0].deletions),
        (2, 1)
    );
}

#[test]
fn committed_results_do_not_reappear_after_later_edits_or_restarts() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "agent change\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    run_git(&f.repo, &["add", "a.txt"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "accept changes"]).unwrap();
    // A later edit must not keep the already committed turn's card alive.
    f.write("a.txt", "later editor change\n");
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());

    run_git(&f.repo, &["add", "a.txt"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "later work"]).unwrap();
    f.write("a.txt", "another editor change\n");
    let reopened = Store::new(f.dir.join("checkpoints"));
    assert!(reopened
        .status("session", f.cwd())
        .unwrap()
        .files
        .is_empty());
    assert_eq!(
        reopened
            .file_diff("session", f.cwd(), "a.txt")
            .unwrap()
            .current,
        "agent change\n"
    );
}

#[test]
fn presentation_status_never_writes_the_recording_index_head_or_dirty_files() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "agent change\n");
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    assert_eq!(
        f.store
            .status_readonly("session", f.cwd())
            .unwrap()
            .files
            .len(),
        1
    );
    run_git(&f.repo, &["add", "a.txt"]).unwrap();
    run_git(&f.repo, &["commit", "-m", "accept changes"]).unwrap();
    f.write("a.txt", "later user edit\n");
    f.write("user.txt", "untracked user work\n");
    let review = f.store.session_dir("session").join("turn-review.json");
    let recording = std::fs::read(&review).unwrap();
    let index = std::fs::read(f.repo.join(".git/index")).unwrap();
    let head = run_git(&f.repo, &["rev-parse", "HEAD"]).unwrap();
    assert!(f
        .store
        .status_readonly("session", f.cwd())
        .unwrap()
        .files
        .is_empty());
    assert_eq!(std::fs::read(&review).unwrap(), recording);
    assert_eq!(std::fs::read(f.repo.join(".git/index")).unwrap(), index);
    assert_eq!(run_git(&f.repo, &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "later user edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.repo.join("user.txt")).unwrap(),
        "untracked user work\n"
    );
    // The ordinary review read still durably dismisses accepted paths.
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    assert_ne!(std::fs::read(&review).unwrap(), recording);
}

#[test]
fn presentation_status_reads_the_latest_native_or_original_desktop_session_id_in_place() {
    let f = Fixture::new();
    let desktop = "44f70dd0-6ad2-47f0-baea-aebad5f36d2c";
    let aliases = vec!["42".to_string(), desktop.to_string()];
    f.store.begin_turn(desktop, f.cwd(), "desktop").unwrap();
    f.write("a.txt", "desktop result\n");
    f.store.finish_turn(desktop, f.cwd(), "desktop").unwrap();
    assert_eq!(
        f.store
            .status_readonly_aliases(&aliases, f.cwd())
            .unwrap()
            .files[0]
            .relative,
        "a.txt"
    );
    assert!(!f.store.session_dir("42").exists());
    f.store.begin_turn("42", f.cwd(), "native").unwrap();
    f.write("delete.txt", "native result\n");
    f.store.finish_turn("42", f.cwd(), "native").unwrap();
    let set_time = |session: &str, seconds| {
        std::fs::File::options()
            .write(true)
            .open(f.store.session_dir(session).join("turn-review.json"))
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
            )
            .unwrap();
    };
    set_time(desktop, 1_000);
    set_time("42", 2_000);
    assert_eq!(
        f.store
            .status_readonly_aliases(&aliases, f.cwd())
            .unwrap()
            .files[0]
            .relative,
        "delete.txt"
    );
    set_time(desktop, 3_000);
    assert_eq!(
        f.store
            .status_readonly_aliases(&aliases, f.cwd())
            .unwrap()
            .files[0]
            .relative,
        "a.txt"
    );
}

#[test]
fn commits_clear_review_in_nested_workspaces_and_repositories_without_head() {
    let f = Fixture::new();
    let nested = f.repo.join("nested");
    let unborn = f.dir.join("unborn");
    std::fs::create_dir(&nested).unwrap();
    std::fs::create_dir(&unborn).unwrap();
    run_git(&unborn, &["init"]).unwrap();
    run_git(&unborn, &["config", "user.name", "Test"]).unwrap();
    run_git(&unborn, &["config", "user.email", "test@example.invalid"]).unwrap();

    for (session, root) in [("nested", nested), ("unborn", unborn)] {
        let cwd = root.to_str().unwrap();
        let pending = "café pending.txt";
        f.store.begin_turn(session, cwd, "first").unwrap();
        std::fs::write(root.join("committed.txt"), "accepted\n").unwrap();
        std::fs::write(root.join(pending), "pending\n").unwrap();
        f.store.finish_turn(session, cwd, "first").unwrap();
        run_git(&root, &["add", "committed.txt"]).unwrap();
        run_git(&root, &["commit", "-m", "accept one file"]).unwrap();
        let status = f.store.status(session, cwd).unwrap();
        assert_eq!(status.files.len(), 1);
        assert_eq!(status.files[0].relative, pending);
        run_git(&root, &["add", pending]).unwrap();
        run_git(&root, &["commit", "-m", "accept remaining work"]).unwrap();
        assert!(f.store.status(session, cwd).unwrap().files.is_empty());
    }
}

#[test]
fn linked_worktrees_have_independent_recordings_and_indexes() {
    let f = Fixture::new();
    let linked = f.dir.join("linked");
    run_git(
        &f.repo,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    )
    .unwrap();
    let cwd = linked.to_str().unwrap();
    let git_dir = run_git(&linked, &["rev-parse", "--absolute-git-dir"]).unwrap();
    let index_path = PathBuf::from(String::from_utf8(git_dir).unwrap().trim()).join("index");
    let index = std::fs::read(&index_path).unwrap();
    f.store.begin_turn("main", f.cwd(), "first").unwrap();
    f.store.begin_turn("linked", cwd, "second").unwrap();
    f.write("a.txt", "main change\n");
    std::fs::write(linked.join("delete.txt"), "linked change\n").unwrap();
    f.store.finish_turn("main", f.cwd(), "first").unwrap();
    f.store.finish_turn("linked", cwd, "second").unwrap();
    assert_eq!(
        f.store.status("main", f.cwd()).unwrap().files[0].relative,
        "a.txt"
    );
    let status = f.store.status("linked", cwd).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "delete.txt");
    assert!(status.files[0].undoable);
    assert_eq!(std::fs::read(&index_path).unwrap(), index);
    f.store.undo("linked", cwd, None).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "main change\n"
    );
}

#[cfg(unix)]
#[test]
fn preserves_literal_paths_and_records_modes_and_symlinks_safely() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    run_git(&f.repo, &["config", "core.fileMode", "true"]).unwrap();
    let path = " literal => name .txt ";
    f.write(path, "baseline\n");
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write(path, "changed\n");
    std::fs::set_permissions(f.repo.join("a.txt"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(f.repo.join("delete.txt"), f.repo.join("renamed.txt")).unwrap();
    symlink("a.txt", f.repo.join("link.txt")).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    let status = f.store.status("session", f.cwd()).unwrap();
    assert_eq!(status.files.len(), 5);
    let link = status
        .files
        .iter()
        .find(|file| file.relative == "link.txt")
        .unwrap();
    assert!(link.exact);
    assert!(!link.undoable);
    let diff = f.store.file_diff("session", f.cwd(), path).unwrap();
    assert_eq!(diff.original, "baseline\n");
    assert_eq!(diff.current, "changed\n");
    f.store.undo("session", f.cwd(), Some(path)).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.repo.join(path)).unwrap(),
        "baseline\n"
    );
    f.store.undo("session", f.cwd(), Some("a.txt")).unwrap();
    assert_eq!(file_mode(&f.repo.join("a.txt")).unwrap() & 0o111, 0);
    for path in ["../a.txt", "/a.txt", "a/../a.txt", "./a.txt", ".git/config"] {
        assert!(f.store.file_diff("session", f.cwd(), path).is_err());
    }
}

#[test]
fn checkpoints_scope_nested_workspaces_and_repositories_without_head() {
    let f = Fixture::new();
    f.write("nested/a.txt", "nested\n");
    let cwd = f.repo.join("nested");
    let cwd = cwd.to_str().unwrap();
    f.store.begin_turn("nested", cwd, "first").unwrap();
    f.write("nested/a.txt", "nested\nchanged\n");
    f.write("a.txt", "outside scope\n");
    f.store.finish_turn("nested", cwd, "first").unwrap();
    let status = f.store.status("nested", cwd).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].relative, "a.txt");
    assert_eq!(
        f.store.file_diff("nested", cwd, "a.txt").unwrap().current,
        "nested\nchanged\n"
    );
    f.store.undo("nested", cwd, None).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "outside scope\n"
    );

    let unborn = f.dir.join("unborn");
    std::fs::create_dir(&unborn).unwrap();
    run_git(&unborn, &["init"]).unwrap();
    let cwd = unborn.to_str().unwrap();
    f.store.begin_turn("unborn", cwd, "first").unwrap();
    std::fs::write(unborn.join("new.txt"), "new\n").unwrap();
    f.store.finish_turn("unborn", cwd, "first").unwrap();
    assert_eq!(f.store.status("unborn", cwd).unwrap().files[0].additions, 1);
    f.store.undo("unborn", cwd, None).unwrap();
    assert!(!unborn.join("new.txt").exists());
}

#[test]
fn a_git_failure_without_output_names_the_command_and_its_exit() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q"]).unwrap();
    // Windows git would otherwise warn about line endings on stderr.
    run_git(dir.path(), &["config", "core.autocrlf", "false"]).unwrap();
    run_git(dir.path(), &["config", "core.safecrlf", "false"]).unwrap();
    // `diff --quiet` reports a difference by exit code alone.
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    run_git(dir.path(), &["add", "a.txt"]).unwrap();
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    let error = run_git(dir.path(), &["diff", "--quiet"]).unwrap_err();
    assert!(error.starts_with("git diff --quiet failed ("), "{error}");
    assert!(error.contains('1'), "{error}");
    // Git's own message still wins when it has one.
    let error = run_git(dir.path(), &["cat-file", "-p", "no-such-object"]).unwrap_err();
    assert!(error.contains("no-such-object"), "{error}");
}

fn relatives(status: &CheckpointStatus) -> Vec<&str> {
    status
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect()
}

/// One recorded turn that changed a.txt, created new.txt and deleted delete.txt.
fn turn_with_three_changes(f: &Fixture) {
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    f.write("a.txt", "agent\n");
    f.write("new.txt", "created\n");
    std::fs::remove_file(f.repo.join("delete.txt")).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
}

#[test]
fn undo_one_file_restores_only_that_file_for_changed_added_and_deleted_files() {
    let f = Fixture::new();
    turn_with_three_changes(&f);

    let status = f.store.undo("session", f.cwd(), Some("a.txt")).unwrap();
    assert_eq!(relatives(&status), ["delete.txt", "new.txt"]);
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "original\n"
    );
    assert!(f.repo.join("new.txt").exists());
    assert!(!f.repo.join("delete.txt").exists());

    let status = f.store.undo("session", f.cwd(), Some("new.txt")).unwrap();
    assert_eq!(relatives(&status), ["delete.txt"]);
    assert!(!f.repo.join("new.txt").exists());

    let status = f
        .store
        .undo("session", f.cwd(), Some("delete.txt"))
        .unwrap();
    assert!(status.files.is_empty());
    assert_eq!(
        std::fs::read_to_string(f.repo.join("delete.txt")).unwrap(),
        "remove me\n"
    );
}

#[test]
fn keep_one_file_leaves_it_on_disk_and_the_rest_pending() {
    let f = Fixture::new();
    turn_with_three_changes(&f);

    for (kept, left) in [
        ("a.txt", vec!["delete.txt", "new.txt"]),
        ("new.txt", vec!["delete.txt"]),
        ("delete.txt", vec![]),
    ] {
        let status = f.store.keep("session", f.cwd(), Some(kept)).unwrap();
        assert_eq!(relatives(&status), left, "after keeping {kept}");
    }
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "agent\n"
    );
    assert!(f.repo.join("new.txt").exists());
    assert!(!f.repo.join("delete.txt").exists());
    // Kept files stay accepted, so Undo all has nothing left to restore.
    assert!(f
        .store
        .undo("session", f.cwd(), None)
        .unwrap()
        .files
        .is_empty());
    assert_eq!(
        std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
        "agent\n"
    );
}

/// A folder without Git, and the absolute paths a provider's edit tools name.
struct Folder {
    _temp: tempfile::TempDir,
    root: PathBuf,
    store: Store,
}

impl Folder {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("folder");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("gone.txt"), "old\n").unwrap();
        Self {
            store: Store::new(temp.path().join("checkpoints")),
            root,
            _temp: temp,
        }
    }
    fn cwd(&self) -> &str {
        self.root.to_str().unwrap()
    }
    fn path(&self, relative: &str) -> String {
        relative
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part))
            .to_string_lossy()
            .into_owned()
    }
    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.path(relative)).ok()
    }
    /// A structured edit: snapshot before it starts, record its result after.
    fn edit(&self, relative: &str, contents: Option<&str>) {
        let paths = [self.path(relative)];
        self.store.prepare("session", self.cwd(), &paths).unwrap();
        match contents {
            Some(text) => std::fs::write(self.path(relative), text).unwrap(),
            None => std::fs::remove_file(self.path(relative)).unwrap(),
        }
        self.store
            .capture_edit("session", self.cwd(), &paths)
            .unwrap();
    }
    /// One turn that changed src/a.txt, created new.txt and deleted gone.txt.
    fn three_changes(&self) {
        self.store
            .begin_turn("session", self.cwd(), "turn")
            .unwrap();
        self.edit("src/a.txt", Some("one\nTWO\nthree\n"));
        self.edit("new.txt", Some("fresh\n"));
        self.edit("gone.txt", None);
        self.store
            .finish_turn("session", self.cwd(), "turn")
            .unwrap();
    }
}

#[test]
fn folders_without_git_record_structured_edits_with_exact_counts_and_diffs() {
    let f = Folder::new();
    f.three_changes();
    // A shell write without edit events is not this session's change.
    std::fs::write(f.path("shell.txt"), "untracked\n").unwrap();

    let status = f.store.status("session", f.cwd()).unwrap();
    let files: Vec<_> = status
        .files
        .iter()
        .map(|file| {
            (
                file.relative.as_str(),
                file.status.as_str(),
                file.additions,
                file.deletions,
                file.exact && file.undoable,
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            ("gone.txt", "deleted", 0, 1, true),
            ("new.txt", "added", 1, 0, true),
            ("src/a.txt", "modified", 2, 1, true),
        ]
    );
    let diff = f.store.file_diff("session", f.cwd(), "src/a.txt").unwrap();
    assert_eq!(
        (diff.original.as_str(), diff.current.as_str()),
        ("one\ntwo\n", "one\nTWO\nthree\n")
    );
    // A later turn keeps the session's recording, as the desktop app does.
    f.store.begin_turn("session", f.cwd(), "next").unwrap();
    assert_eq!(f.store.status("session", f.cwd()).unwrap().files.len(), 3);
}

#[test]
fn folders_without_git_undo_and_keep_one_file_at_a_time() {
    let f = Folder::new();
    f.three_changes();

    let status = f.store.undo("session", f.cwd(), Some("src/a.txt")).unwrap();
    assert_eq!(relatives(&status), ["gone.txt", "new.txt"]);
    assert_eq!(f.read("src/a.txt").as_deref(), Some("one\ntwo\n"));

    let status = f.store.keep("session", f.cwd(), Some("new.txt")).unwrap();
    assert_eq!(relatives(&status), ["gone.txt"]);
    assert_eq!(f.read("new.txt").as_deref(), Some("fresh\n"));

    let status = f.store.undo("session", f.cwd(), Some("gone.txt")).unwrap();
    assert!(status.files.is_empty());
    assert_eq!(f.read("gone.txt").as_deref(), Some("old\n"));
    // The kept file is untouched by a later Undo all.
    f.store.undo("session", f.cwd(), None).unwrap();
    assert_eq!(f.read("new.txt").as_deref(), Some("fresh\n"));
}

#[test]
fn folders_without_git_undo_a_created_file_and_keep_a_deleted_one() {
    let f = Folder::new();
    f.three_changes();
    let status = f.store.undo("session", f.cwd(), Some("new.txt")).unwrap();
    assert_eq!(relatives(&status), ["gone.txt", "src/a.txt"]);
    assert_eq!(f.read("new.txt"), None);
    let status = f.store.keep("session", f.cwd(), Some("gone.txt")).unwrap();
    assert_eq!(relatives(&status), ["src/a.txt"]);
    assert_eq!(f.read("gone.txt"), None);
}

#[test]
fn folders_without_git_refuse_to_undo_a_file_changed_after_the_edit() {
    let f = Folder::new();
    f.three_changes();
    std::fs::write(f.path("src/a.txt"), "someone else\n").unwrap();

    let status = f.store.status("session", f.cwd()).unwrap();
    let file = status
        .files
        .iter()
        .find(|file| file.relative == "src/a.txt")
        .unwrap();
    assert!(file.exact && !file.undoable);
    let error = f
        .store
        .undo("session", f.cwd(), Some("src/a.txt"))
        .unwrap_err();
    assert!(error.contains("changed outside this session"), "{error}");
    assert!(f.store.undo("session", f.cwd(), None).is_err());
    assert_eq!(f.read("src/a.txt").as_deref(), Some("someone else\n"));
    assert_eq!(f.read("new.txt").as_deref(), Some("fresh\n"));
    // Keep still accepts it.
    let status = f.store.keep("session", f.cwd(), Some("src/a.txt")).unwrap();
    assert_eq!(relatives(&status), ["gone.txt", "new.txt"]);
}

#[test]
fn folders_without_git_undo_all_and_forget_clear_the_recording() {
    let f = Folder::new();
    f.three_changes();
    assert!(f
        .store
        .undo("session", f.cwd(), None)
        .unwrap()
        .files
        .is_empty());
    assert_eq!(f.read("src/a.txt").as_deref(), Some("one\ntwo\n"));
    assert_eq!(f.read("new.txt"), None);
    assert_eq!(f.read("gone.txt").as_deref(), Some("old\n"));
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());

    f.three_changes();
    assert_eq!(f.store.status("session", f.cwd()).unwrap().files.len(), 3);
    f.store.forget("session").unwrap();
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    assert_eq!(f.read("new.txt").as_deref(), Some("fresh\n"));
}

#[test]
fn folders_without_git_need_an_edit_start_and_ignore_other_folders() {
    let f = Folder::new();
    f.store.begin_turn("session", f.cwd(), "turn").unwrap();
    // A completion without its start event has no safe baseline.
    std::fs::write(f.path("src/a.txt"), "changed\n").unwrap();
    f.store
        .capture_edit("session", f.cwd(), &[f.path("src/a.txt")])
        .unwrap();
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    // Paths outside the folder are never recorded.
    let outside = tempfile::tempdir().unwrap();
    let stray = outside.path().join("x.txt").to_string_lossy().into_owned();
    f.store
        .prepare("session", f.cwd(), std::slice::from_ref(&stray))
        .unwrap();
    f.store.capture_edit("session", f.cwd(), &[stray]).unwrap();
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
    // The same session in another folder starts a fresh recording.
    let other = tempfile::tempdir().unwrap();
    let other_cwd = other.path().to_str().unwrap();
    f.store.begin_turn("session", other_cwd, "turn").unwrap();
    assert!(f
        .store
        .status("session", other_cwd)
        .unwrap()
        .files
        .is_empty());
    assert!(f.store.status("session", f.cwd()).unwrap().files.is_empty());
}

#[test]
fn folders_without_git_do_not_offer_undo_for_another_sessions_file() {
    let f = Folder::new();
    f.three_changes();
    f.store.begin_turn("other", f.cwd(), "turn").unwrap();
    let paths = [f.path("src/a.txt")];
    f.store.prepare("other", f.cwd(), &paths).unwrap();
    f.store.capture_edit("other", f.cwd(), &paths).unwrap();
    let status = f.store.status("session", f.cwd()).unwrap();
    let file = status
        .files
        .iter()
        .find(|file| file.relative == "src/a.txt")
        .unwrap();
    assert!(!file.undoable);
    assert!(status
        .files
        .iter()
        .filter(|file| file.relative != "src/a.txt")
        .all(|file| file.undoable));
}

#[test]
fn git_turns_ignore_folder_edit_events() {
    let f = Fixture::new();
    f.store.begin_turn("session", f.cwd(), "first").unwrap();
    let path = f.repo.join("a.txt").to_string_lossy().into_owned();
    f.store
        .prepare("session", f.cwd(), std::slice::from_ref(&path))
        .unwrap();
    f.write("a.txt", "agent\n");
    f.store.capture_edit("session", f.cwd(), &[path]).unwrap();
    f.store.finish_turn("session", f.cwd(), "first").unwrap();
    assert!(!f
        .dir
        .join("checkpoints")
        .join("session")
        .join("manifest.json")
        .exists());
    assert_eq!(
        relatives(&f.store.status("session", f.cwd()).unwrap()),
        ["a.txt"]
    );
}
