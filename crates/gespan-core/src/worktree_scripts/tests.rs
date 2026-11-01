use super::test_host::Script;
use super::*;
use crate::git::{create_named_worktree, create_worktree};
use crate::worktree_management;

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
fn set_scripts(main: &Path, setup: &str, teardown: &str) {
    let scripts = Scripts {
        setup: setup.into(),
        teardown: teardown.into(),
    };
    save_scripts(main, &scripts).unwrap();
}
fn outcomes(host: &Script) -> Vec<Outcome> {
    host.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            Event::Finished { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}
fn leftovers(main: &Path) -> PathBuf {
    worktree_management::default_root(main)
}
/// Fails until a marker exists, leaving the marker: passes on the second run.
#[cfg(unix)]
const FAILS_ONCE: &str = "if [ -f marker ]; then echo second; else echo first; echo > marker; exit 4; fi";
#[cfg(windows)]
const FAILS_ONCE: &str =
    "@if exist marker (echo second) else (echo first& echo x> marker& exit 4)";
#[cfg(unix)]
const LONG: &str = "sleep 30";
#[cfg(windows)]
const LONG: &str = "ping -n 30 127.0.0.1 >nul";
/// Fails if another copy of itself is running in the same repository.
#[cfg(unix)]
const EXCLUSIVE: &str =
    "mkdir \"$GESPAN_PROJECT_ROOT/running\"; sleep 0.2; rmdir \"$GESPAN_PROJECT_ROOT/running\"";
#[cfg(windows)]
const EXCLUSIVE: &str = "@mkdir \"%GESPAN_PROJECT_ROOT%\\running\" || exit 1\r\n@ping -n 2 127.0.0.1 >nul\r\n@rmdir \"%GESPAN_PROJECT_ROOT%\\running\"";

#[test]
fn nothing_happens_without_scripts_or_included_files() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    let path = create_worktree(dir.path(), "main", 1).unwrap();
    assert!(path.is_dir());
    assert!(host.events.lock().unwrap().is_empty());
    assert_eq!(record(&path), None);
    worktree_management::remove(dir.path(), &path).unwrap();
    assert!(host.events.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir(leftovers(dir.path()));
}

#[test]
fn setup_finishes_in_the_new_worktree_before_creation_returns() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    set_scripts(dir.path(), "echo hello\necho done> setup-ran", "");
    let path = create_worktree(dir.path(), "main", 1).unwrap();
    assert!(path.join("setup-ran").is_file(), "it ran in the worktree");
    assert!(!dir.path().join("setup-ran").exists());
    assert_eq!(host.output(), ["hello"]);
    assert_eq!(outcomes(&host), [Outcome::Passed]);
    let saved = record(&path).unwrap();
    assert_eq!(saved.outcome, Some(Outcome::Passed));
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn the_script_is_told_where_it_is() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    #[cfg(unix)]
    let script = "echo \"$GESPAN_BRANCH\"\ncd \"$GESPAN_PROJECT_ROOT\" && pwd -P\ncd \"$GESPAN_WORKTREE_PATH\" && pwd -P";
    #[cfg(windows)]
    let script = "@echo %GESPAN_BRANCH%\r\n@echo %GESPAN_PROJECT_ROOT%\r\n@echo %GESPAN_WORKTREE_PATH%";
    set_scripts(dir.path(), script, "");
    let tree = create_named_worktree(dir.path(), "told").unwrap();
    let output = host.output();
    assert_eq!(output[0], "told");
    let same = |line: &str, path: &Path| {
        Path::new(line).canonicalize().unwrap() == path.canonicalize().unwrap()
    };
    assert!(same(&output[1], dir.path()), "{output:?}");
    assert!(same(&output[2], Path::new(&tree.path)), "{output:?}");
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn a_failure_with_nobody_to_ask_stops_and_keeps_the_worktree() {
    let dir = repo();
    set_scripts(dir.path(), "exit 3", "");
    let error = create_named_worktree(dir.path(), "unasked").unwrap_err();
    assert!(error.contains("code 3"), "{error}");
    let root = leftovers(dir.path());
    assert!(error.contains(&root.join("unasked").display().to_string()));
    assert!(root.join("unasked").is_dir());
    assert_eq!(
        record(&root.join("unasked")).unwrap().outcome,
        Some(Outcome::Failed("It exited with code 3.".into()))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn retry_runs_the_script_again() {
    let dir = repo();
    let host = Script::install(dir.path(), &[Decision::Retry]);
    set_scripts(dir.path(), FAILS_ONCE, "");
    let path = create_worktree(dir.path(), "main", 1).unwrap();
    assert_eq!(host.output(), ["first", "second"]);
    assert_eq!(outcomes(&host), [Outcome::Passed]);
    let events = host.events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e, Event::Retrying { .. })));
    assert!(path.is_dir());
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn start_anyway_goes_on_and_remembers_the_failure() {
    let dir = repo();
    let host = Script::install(dir.path(), &[Decision::Continue]);
    set_scripts(dir.path(), "exit 5", "");
    let path = create_worktree(dir.path(), "main", 1).unwrap();
    let failed = Outcome::Overridden("It exited with code 5.".into());
    assert_eq!(outcomes(&host), [failed.clone()]);
    assert_eq!(record(&path).unwrap().outcome, Some(failed));
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn cancel_refuses_the_worktree_but_leaves_it_in_place() {
    let dir = repo();
    let host = Script::install(dir.path(), &[Decision::Cancel]);
    set_scripts(dir.path(), "exit 5", "");
    let error = create_named_worktree(dir.path(), "cancelled").unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert_eq!(outcomes(&host), [Outcome::Cancelled]);
    assert!(leftovers(dir.path()).join("cancelled").is_dir());
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn a_running_script_can_be_stopped() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    set_scripts(dir.path(), LONG, "");
    let main = dir.path().to_path_buf();
    let began = Instant::now();
    let worker = std::thread::spawn(move || create_named_worktree(&main, "stopped"));
    while !worker.is_finished() {
        let id = host.events.lock().unwrap().iter().find_map(|e| match e {
            Event::Started(run) => Some(run.id),
            _ => None,
        });
        if let Some(id) = id {
            cancel(id);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(worker.join().unwrap().is_err());
    assert!(began.elapsed() < Duration::from_secs(20));
    assert_eq!(outcomes(&host), [Outcome::Cancelled]);
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[cfg(unix)]
#[test]
fn a_failing_line_stops_the_script() {
    let dir = repo();
    let host = Script::install(dir.path(), &[Decision::Continue]);
    set_scripts(dir.path(), "false\necho later", "");
    create_named_worktree(dir.path(), "stops").unwrap();
    assert!(host.output().is_empty());
    assert_eq!(
        outcomes(&host),
        [Outcome::Overridden("It exited with code 1.".into())]
    );
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[cfg(unix)]
#[test]
fn something_left_running_does_not_hold_up_the_worktree() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    set_scripts(dir.path(), "sleep 20 &\necho started", "");
    let began = Instant::now();
    create_named_worktree(dir.path(), "server").unwrap();
    assert!(began.elapsed() < Duration::from_secs(10));
    assert_eq!(host.output(), ["started"]);
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn copied_files_are_reported_even_without_a_script() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    std::fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();
    std::fs::write(dir.path().join(".worktreeinclude"), ".env\n").unwrap();
    std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
    let path = create_worktree(dir.path(), "main", 1).unwrap();
    assert_eq!(std::fs::read(path.join(".env")).unwrap(), b"SECRET=1");
    let events = host.events.lock().unwrap();
    let Event::Started(run) = &events[0] else {
        panic!("{events:?}");
    };
    assert!(!run.scripted);
    assert_eq!(run.copied.files, 1);
    assert_eq!(run.copied.names, [".env"]);
    assert_eq!(
        record(&path),
        Some(Record {
            copied: 1,
            outcome: None
        })
    );
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn four_worktrees_at_once_all_finish_without_overlapping() {
    let dir = repo();
    // No answers: a run that overlapped would fail and panic asking for one.
    let host = Script::install(dir.path(), &[]);
    set_scripts(dir.path(), EXCLUSIVE, "");
    let threads: Vec<_> = (0..4)
        .map(|n| {
            let main = dir.path().to_path_buf();
            std::thread::spawn(move || create_worktree(&main, "main", n))
        })
        .collect();
    for thread in threads {
        assert!(thread.join().unwrap().unwrap().is_dir());
    }
    assert_eq!(outcomes(&host), vec![Outcome::Passed; 4]);
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn turns_are_per_repository() {
    let (one, other) = (repo(), repo());
    let held = turn(one.path());
    drop(turn(other.path()));
    let path = one.path().to_path_buf();
    let (sender, waiting) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        drop(turn(&path));
        sender.send(()).unwrap();
    });
    assert!(waiting.recv_timeout(Duration::from_millis(150)).is_err());
    drop(held);
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    waiter.join().unwrap();
}

#[test]
fn a_question_does_not_hold_the_repositorys_turn() {
    struct Probe(PathBuf);
    impl Host for Probe {
        fn event(&self, _: Event) {}
        fn decide(&self, _: u64) -> Decision {
            // Would wait forever if the failed run still held it.
            drop(turn(&self.0));
            Decision::Cancel
        }
    }
    let dir = repo();
    test_host::put(dir.path(), Arc::new(Probe(dir.path().to_path_buf())));
    set_scripts(dir.path(), "exit 1", "");
    assert!(create_named_worktree(dir.path(), "asked").is_err());
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn teardown_runs_in_the_worktree_before_it_goes() {
    let dir = repo();
    let host = Script::install(dir.path(), &[]);
    let tree = create_named_worktree(dir.path(), "going").unwrap();
    let path = PathBuf::from(&tree.path);
    set_scripts(dir.path(), "", "echo bye");
    worktree_management::remove(dir.path(), &path).unwrap();
    assert_eq!(host.output(), ["bye"]);
    assert!(!path.exists());
    let _ = std::fs::remove_dir(leftovers(dir.path()));
}

#[test]
fn a_cancelled_teardown_keeps_the_worktree() {
    let dir = repo();
    let host = Script::install(dir.path(), &[Decision::Cancel, Decision::Cancel]);
    let tree = create_named_worktree(dir.path(), "kept").unwrap();
    let path = PathBuf::from(&tree.path);
    set_scripts(dir.path(), "", "exit 2");
    let error = worktree_management::remove(dir.path(), &path).unwrap_err();
    assert!(error.contains("kept"), "{error}");
    assert!(path.is_dir());
    assert!(worktree_management::remove_unused(dir.path(), &path).is_err());
    assert!(path.is_dir());
    assert_eq!(outcomes(&host), vec![Outcome::Cancelled; 2]);
    let _ = std::fs::remove_dir_all(leftovers(dir.path()));
}

#[test]
fn remove_anyway_goes_past_a_failed_teardown_and_clears_the_record() {
    let dir = repo();
    Script::install(dir.path(), &[Decision::Continue]);
    set_scripts(dir.path(), "echo up", "exit 2");
    let tree = create_named_worktree(dir.path(), "failing").unwrap();
    let path = PathBuf::from(&tree.path);
    assert!(record(&path).is_some());
    worktree_management::remove_unused(dir.path(), &path).unwrap();
    assert!(!path.exists());
    assert_eq!(record(&path), None);
    let _ = std::fs::remove_dir(leftovers(dir.path()));
}

#[test]
fn a_worktree_reaches_the_same_scripts_through_any_spelling() {
    let dir = repo();
    set_scripts(dir.path(), "echo one", "echo two");
    let spelled = dir.path().join(".");
    assert_eq!(scripts(&spelled).teardown, "echo two");
    set_scripts(dir.path(), "", "");
    assert_eq!(scripts(dir.path()), Scripts::default());
}

#[test]
fn scripts_survive_in_gespans_data_and_a_bad_file_reads_as_empty() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join(FILE);
    let mut store = Store {
        path: Some(path.clone()),
        data: Data::default(),
    };
    store.data.projects.insert(
        "project".into(),
        Scripts {
            setup: "bun install".into(),
            teardown: String::new(),
        },
    );
    store.save().unwrap();
    let mut read = Store {
        path: Some(path.clone()),
        data: Data::default(),
    };
    read.load();
    assert_eq!(read.data.projects["project"].setup, "bun install");
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 1);
    std::fs::write(&path, "not json").unwrap();
    read.load();
    assert!(read.data.projects.is_empty());
}

#[test]
fn output_splits_on_line_ends_and_progress_frames() {
    let mut lines = Vec::new();
    let input: &[u8] = b"one\r\ntwo\n10%\r20%\r\n\nlast";
    read_lines(input, &mut |line| lines.push(line));
    assert_eq!(lines, ["one", "two", "10%", "20%", "last"]);
}

#[test]
fn output_without_line_ends_is_capped_and_bad_bytes_survive() {
    let mut lines = Vec::new();
    let long = vec![b'a'; MAX_LINE * 9];
    read_lines(&long[..], &mut |line| lines.push(line));
    assert!(lines.len() >= 2);
    assert!(lines.iter().all(|line| line.chars().count() <= MAX_LINE));
    lines.clear();
    read_lines(&b"caf\xff\n"[..], &mut |line| lines.push(line));
    assert_eq!(lines, ["caf\u{fffd}"]);
}

#[test]
fn the_tail_keeps_the_latest_lines() {
    let mut tail = Tail::default();
    for n in 0..Tail::MAX + 3 {
        tail.push(n.to_string());
    }
    assert_eq!(tail.lines().count(), Tail::MAX);
    assert_eq!(tail.lines().next(), Some("3"));
}
