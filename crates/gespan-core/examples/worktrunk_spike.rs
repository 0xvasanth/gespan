//! Spike for #295, on a throwaway branch. Calls worktrunk from outside the crate
//! the way a desktop app would, and prints `SPIKE ...` lines. The parent process
//! captures the child's output, so anything without that prefix is worktrunk
//! writing to a stdio stream that a packaged app loses.
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use worktrunk::copy::{copy_dir_recursive, copy_leaf};
use worktrunk::git::Repository;
use worktrunk::progress::Progress;

macro_rules! report {
    ($($arg:tt)*) => { println!("SPIKE {}", format!($($arg)*)) };
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetConsoleWindow() -> *mut std::ffi::c_void;
}
#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn IsWindowVisible(window: *mut std::ffi::c_void) -> i32;
}

fn main() {
    if let Ok(real_git) = env::var("SPIKE_REAL_GIT") {
        return fake_git(&real_git);
    }
    if env::args().nth(1).as_deref() == Some("child") {
        return child();
    }
    if env::args().nth(1).as_deref() == Some("real") {
        return real_repo();
    }
    parent();
}

/// Stands in for `git` on Windows: records whether this process got a console
/// window, which is what a user would see flash, then runs the real git.
fn fake_git(real_git: &str) {
    #[cfg(windows)]
    {
        let (window, visible) = unsafe {
            let window = GetConsoleWindow();
            (window, IsWindowVisible(window))
        };
        let label = env::var("SPIKE_LABEL").unwrap_or_else(|_| "worktrunk".to_string());
        let args: Vec<String> = env::args().skip(1).collect();
        let line = format!(
            "{label}: console={} visible={} args={}\n",
            !window.is_null(),
            visible != 0,
            args.first().cloned().unwrap_or_default()
        );
        use std::io::Write;
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(env::var("SPIKE_PROBE_LOG").unwrap())
            .and_then(|mut file| file.write_all(line.as_bytes()));
    }
    let status = Command::new(real_git)
        .args(env::args().skip(1))
        .env_remove("SPIKE_REAL_GIT")
        .status()
        .unwrap();
    std::process::exit(status.code().unwrap_or(1));
}

/// Runs the child with an empty home and captured stdio, like a GUI app.
fn parent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .arg("child")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home.join("AppData"))
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    let probe_log = {
        use std::os::windows::process::CommandExt;
        let bin = temp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::copy(env::current_exe().unwrap(), bin.join("git.exe")).unwrap();
        let where_git = Command::new("where").arg("git").output().unwrap();
        let real_git = String::from_utf8(where_git.stdout).unwrap();
        let real_git = real_git.lines().next().unwrap().trim().to_string();
        let path = format!("{};{}", bin.display(), env::var("PATH").unwrap());
        let log = temp.path().join("probe.log");
        command
            .env("PATH", path)
            .env("SPIKE_REAL_GIT", &real_git)
            .env("SPIKE_REAL_GIT_FOR_CHILD", &real_git)
            .env("SPIKE_PROBE_LOG", &log)
            .creation_flags(DETACHED_PROCESS);
        log
    };
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stdout.lines().filter(|line| line.starts_with("SPIKE ")) {
        println!("{line}");
    }
    let noise: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with("SPIKE "))
        .collect();
    println!("SPIKE stdout noise lines: {} {noise:?}", noise.len());
    println!("SPIKE stderr bytes: {}", stderr.len());
    println!("--- child stderr\n{stderr}\n--- end child stderr");
    #[cfg(windows)]
    {
        let log = fs::read_to_string(&probe_log).unwrap_or_default();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for line in log.lines() {
            let key = line.split(" args=").next().unwrap().to_string();
            *counts.entry(key).or_default() += 1;
        }
        println!("SPIKE windows console probe: {counts:#?}");
    }
    assert!(output.status.success(), "child failed: {:?}", output.status);
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "spike")
        .env("GIT_AUTHOR_EMAIL", "spike@example.invalid")
        .env("GIT_COMMITTER_NAME", "spike")
        .env("GIT_COMMITTER_EMAIL", "spike@example.invalid")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.env("SPIKE_LABEL", "gespan-style-no-window");
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn lines(third: &str) -> String {
    format!("one\ntwo\n{third}\nfour\nfive\n")
}

fn build_repo(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    git(repo, &["config", "core.autocrlf", "false"]);
    commit(repo, "base.txt", "base\n", "base");
    commit(repo, "later.txt", &lines("three"), "later base");
    commit(repo, "reverted.txt", &lines("three"), "reverted base");

    git(repo, &["branch", "ancestor"]);

    let on_branch = |name: &str, steps: &[(&str, &str)]| {
        git(repo, &["checkout", "-q", "-b", name, "main"]);
        for (index, (file, text)) in steps.iter().enumerate() {
            commit(repo, file, text, &format!("{name} {index}"));
        }
        git(repo, &["checkout", "-q", "main"]);
    };
    on_branch("rebased", &[("r.txt", "1\n"), ("r.txt", "1\n2\n")]);
    on_branch("squashed", &[("s.txt", "1\n"), ("s.txt", "1\n2\n")]);
    on_branch(
        "squashed-later-same-file",
        &[("later.txt", &lines("draft")), ("later.txt", &lines("final"))],
    );
    on_branch(
        "squashed-then-reverted-conflict",
        &[("reverted.txt", &lines("kept-on-branch"))],
    );
    on_branch("squashed-then-reverted-clean", &[("clean.txt", "1\n")]);
    on_branch("self-reverted", &[("self.txt", "1\n")]);
    git(repo, &["checkout", "-q", "self-reverted"]);
    git(repo, &["revert", "--no-edit", "HEAD"]);
    git(repo, &["checkout", "-q", "main"]);
    on_branch("not-merged", &[("n.txt", "1\n")]);
    on_branch("half-merged", &[("h1.txt", "1\n"), ("h2.txt", "1\n")]);

    let squash = |branch: &str| {
        git(repo, &["merge", "-q", "--squash", branch]);
        git(repo, &["commit", "-q", "-m", &format!("squash {branch}")]);
    };
    commit(repo, "main.txt", "moved\n", "main moves");
    git(repo, &["cherry-pick", "rebased~1", "rebased"]);
    squash("squashed");
    squash("squashed-later-same-file");
    commit(repo, "later.txt", &lines("rewritten-on-main"), "later edit");
    squash("squashed-then-reverted-conflict");
    git(repo, &["revert", "--no-edit", "HEAD"]);
    commit(repo, "reverted.txt", &lines("something-else"), "rewrite");
    squash("squashed-then-reverted-clean");
    git(repo, &["revert", "--no-edit", "HEAD"]);
    git(repo, &["cherry-pick", "half-merged~1"]);
    git(repo, &["branch", "same-commit"]);
}

fn loose_objects(repo: &Path) -> String {
    let counts = git(repo, &["count-objects", "-v"]);
    let wanted = |key: &str| {
        counts
            .lines()
            .find_map(|line| line.strip_prefix(key).map(str::trim).map(str::to_string))
            .unwrap_or_default()
    };
    format!("loose={} packed={}", wanted("count:"), wanted("in-pack:"))
}

fn detection(repo_path: &Path) {
    let repo = Repository::at(repo_path).unwrap();
    let snapshot = repo.capture_refs().unwrap();
    for branch in [
        "same-commit",
        "ancestor",
        "rebased",
        "squashed",
        "squashed-later-same-file",
        "squashed-then-reverted-conflict",
        "squashed-then-reverted-clean",
        "self-reverted",
        "half-merged",
        "not-merged",
    ] {
        let started = Instant::now();
        let (target, reason) = repo.integration_reason(&snapshot, branch, "main").unwrap();
        report!(
            "integration {branch}: {reason:?} target={target} {}ms",
            started.elapsed().as_millis()
        );
    }

    // The snapshot pins what was checked, so a moved branch needs a new one.
    git(repo_path, &["branch", "moving", "main"]);
    let before = repo.capture_refs().unwrap();
    git(repo_path, &["checkout", "-q", "moving"]);
    commit(repo_path, "moved.txt", "1\n", "moved");
    git(repo_path, &["checkout", "-q", "main"]);
    let stale = repo.integration_reason(&before, "moving", "main").unwrap();
    let fresh_repo = Repository::at(repo_path).unwrap();
    let fresh_snapshot = fresh_repo.capture_refs().unwrap();
    let fresh = fresh_repo
        .integration_reason(&fresh_snapshot, "moving", "main")
        .unwrap();
    report!("moved branch: stale snapshot {:?}, fresh {:?}", stale.1, fresh.1);

    // A linked worktree with uncommitted work: the check only reads refs.
    let linked = repo_path.parent().unwrap().join("linked");
    git(
        repo_path,
        &["worktree", "add", "-q", "-b", "dirty-merged", linked.to_str().unwrap()],
    );
    fs::write(linked.join("uncommitted.txt"), "work\n").unwrap();
    let linked_repo = Repository::at(&linked).unwrap();
    let snapshot = linked_repo.capture_refs().unwrap();
    let (_, reason) = linked_repo
        .integration_reason(&snapshot, "dirty-merged", "main")
        .unwrap();
    report!("dirty linked worktree on merged branch: {reason:?}");
    report!(
        "missing branch: {:?}",
        linked_repo
            .integration_reason(&snapshot, "no-such-branch", "main")
            .map(|found| found.1)
            .map_err(|error| error.to_string())
    );
}

fn upstream(temp: &Path) {
    let origin = temp.join("origin.git");
    let clone = temp.join("clone");
    git(temp, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    build_small(&clone, &origin);
    let repo = Repository::at(&clone).unwrap();
    let snapshot = repo.capture_refs().unwrap();
    let (target, reason) = repo.integration_reason(&snapshot, "feature", "main").unwrap();
    report!("squash only on origin/main, local main behind: {reason:?} target={target}");
    let started = Instant::now();
    let default = repo.default_branch();
    report!(
        "default_branch: {default:?} {}ms",
        started.elapsed().as_millis()
    );
}

fn build_small(clone: &Path, origin: &Path) {
    fs::create_dir_all(clone).unwrap();
    git(clone, &["init", "-q", "-b", "main"]);
    git(clone, &["config", "commit.gpgsign", "false"]);
    commit(clone, "a.txt", "a\n", "a");
    git(clone, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(clone, &["push", "-q", "-u", "origin", "main"]);
    git(clone, &["checkout", "-q", "-b", "feature"]);
    commit(clone, "f.txt", "1\n", "f1");
    commit(clone, "f.txt", "1\n2\n", "f2");
    git(clone, &["checkout", "-q", "main"]);
    let main = git(clone, &["rev-parse", "main"]);
    git(clone, &["merge", "-q", "--squash", "feature"]);
    git(clone, &["commit", "-q", "-m", "squash"]);
    git(clone, &["push", "-q", "origin", "main"]);
    git(clone, &["reset", "-q", "--hard", &main]);
}

fn copying(temp: &Path) {
    let src = temp.join("copy-src");
    let dest_root = temp.join("copy-dest");
    fs::create_dir_all(src.join("pkg/deep/er")).unwrap();
    fs::create_dir_all(&dest_root).unwrap();
    for index in 0..500 {
        fs::write(src.join(format!("pkg/deep/f{index}.js")), vec![b'x'; 4096]).unwrap();
    }
    fs::write(src.join("pkg/big.bin"), vec![7u8; 64 * 1024 * 1024]).unwrap();
    fs::write(src.join("pkg/deep/er/tool"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        fs::set_permissions(
            src.join("pkg/deep/er/tool"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("big.bin", src.join("pkg/link")).unwrap();
        symlink("missing", src.join("pkg/broken")).unwrap();
    }

    let dest = dest_root.join("node_modules");
    let started = Instant::now();
    let progress = Progress::disabled();
    let skipped = copy_dir_recursive(&src, &dest, Some(&dest_root), false, &progress).unwrap();
    report!(
        "copy_dir_recursive: totals={:?} (reflinked, written)={:?} skipped={skipped} {}ms",
        progress.totals(),
        progress.copy_split(),
        started.elapsed().as_millis()
    );

    let again = Progress::disabled();
    let skipped = copy_dir_recursive(&src, &dest, Some(&dest_root), false, &again).unwrap();
    report!("re-run: totals={:?} skipped={skipped}", again.totals());

    fs::write(dest.join("pkg/big.bin"), "changed").unwrap();
    report!(
        "writing the copy leaves the source alone: {}",
        fs::metadata(src.join("pkg/big.bin")).unwrap().len() == 64 * 1024 * 1024
    );

    let leaf = copy_leaf(
        &src.join("pkg/deep/f1.js"),
        &dest_root.join("single.js"),
        Some(&dest_root),
        false,
    );
    report!("copy_leaf: {leaf:?}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dest.join("pkg/deep/er/tool")).unwrap();
        report!(
            "mode kept: {:o}, symlink kept: {}, broken symlink kept: {}",
            mode.permissions().mode() & 0o777,
            fs::symlink_metadata(dest.join("pkg/link")).unwrap().is_symlink(),
            fs::symlink_metadata(dest.join("pkg/broken")).unwrap().is_symlink()
        );
        let outside = temp.join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dest_root.join("escape")).unwrap();
        let escaped = copy_dir_recursive(
            &src,
            &dest_root.join("escape/x"),
            Some(&dest_root),
            false,
            &Progress::disabled(),
        );
        report!(
            "root guard on symlink escape: {:?}",
            escaped.map_err(|error| error.to_string())
        );
    }
}

#[cfg(unix)]
fn dispositions() -> Vec<(i32, usize)> {
    [libc::SIGCHLD, libc::SIGINT, libc::SIGTERM, libc::SIGPIPE, libc::SIGHUP]
        .into_iter()
        .map(|signal| unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal, std::ptr::null(), &mut action);
            (signal, action.sa_sigaction)
        })
        .collect()
}

#[cfg(unix)]
fn blocked_sigchld(repo_path: PathBuf) {
    let handle = std::thread::spawn(move || unsafe {
        let mut blocked: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGCHLD);
        libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, std::ptr::null_mut());
        let repo = Repository::at(&repo_path).unwrap();
        let snapshot = repo.capture_refs().unwrap();
        repo.integration_reason(&snapshot, "squashed", "main")
            .unwrap()
            .1
    });
    report!("SIGCHLD blocked on the calling thread: {:?}", handle.join().unwrap());
}

fn concurrent(repo_path: &Path) {
    let started = Instant::now();
    let threads: Vec<_> = (0..40)
        .map(|_| {
            let path = repo_path.to_path_buf();
            std::thread::spawn(move || {
                let repo = Repository::at(&path).unwrap();
                let snapshot = repo.capture_refs().unwrap();
                repo.integration_reason(&snapshot, "squashed-later-same-file", "main")
                    .unwrap()
                    .1
                    .is_some()
            })
        })
        .collect();
    let integrated = threads
        .into_iter()
        .filter(|_| true)
        .map(|thread| thread.join().unwrap())
        .filter(|found| *found)
        .count();
    report!(
        "40 concurrent checks from background threads: {integrated} integrated, {}ms",
        started.elapsed().as_millis()
    );
}

fn tree(path: &Path, base: &Path, into: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            tree(&path, base, into);
        } else {
            into.push(path.strip_prefix(base).unwrap().display().to_string());
        }
    }
}

fn child() {
    let temp = tempfile::tempdir().unwrap();
    let temp_path = dunce_like(temp.path());
    let repo_path = temp_path.join("repo");
    build_repo(&repo_path);

    let env_before: BTreeMap<_, _> = env::vars_os().collect();
    let cwd_before = env::current_dir().unwrap();
    #[cfg(unix)]
    let signals_before = dispositions();

    let log_path = temp_path.join("tracing.log");
    let log_file = fs::File::create(&log_path).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log_file))
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    report!("objects before checks: {}", loose_objects(&repo_path));
    detection(&repo_path);
    report!("objects after checks: {}", loose_objects(&repo_path));
    upstream(&temp_path);
    copying(&temp_path);
    concurrent(&repo_path);
    #[cfg(unix)]
    blocked_sigchld(repo_path.clone());

    report!(
        "process env unchanged: {}",
        env_before == env::vars_os().collect::<BTreeMap<_, _>>()
    );
    report!("cwd unchanged: {}", cwd_before == env::current_dir().unwrap());
    #[cfg(unix)]
    report!(
        "signal handlers unchanged: {} before={signals_before:?} after={:?}",
        signals_before == dispositions(),
        dispositions()
    );

    let mut wt_files = Vec::new();
    tree(&repo_path.join(".git/wt"), &repo_path, &mut wt_files);
    wt_files.sort();
    report!(
        ".git/wt files after reads: {} sample={:?}",
        wt_files.len(),
        wt_files.iter().take(3).collect::<Vec<_>>()
    );
    let mut home_files = Vec::new();
    let home = PathBuf::from(env::var_os("HOME").unwrap());
    tree(&home, &home, &mut home_files);
    report!("files created under home: {home_files:?}");

    // Broken worktrunk config, which Gespan users would not know about.
    let config = PathBuf::from(env::var_os("XDG_CONFIG_HOME").unwrap()).join("worktrunk");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("config.toml"), "not [ toml").unwrap();
    fs::create_dir_all(repo_path.join(".config")).unwrap();
    fs::write(repo_path.join(".config/wt.toml"), "not [ toml").unwrap();
    let repo = Repository::at(&repo_path).unwrap();
    let snapshot = repo.capture_refs().unwrap();
    let (_, reason) = repo.integration_reason(&snapshot, "squashed", "main").unwrap();
    report!("with broken worktrunk config: {reason:?}");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let real_git = env::var("SPIKE_REAL_GIT_FOR_CHILD").unwrap();
        // Controls through the probe, with and without the flag Gespan sets.
        let run = |label: &str, hidden: bool| {
            let mut command = Command::new("git");
            command
                .arg("--version")
                .env("SPIKE_LABEL", label)
                .env("SPIKE_REAL_GIT", &real_git)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if hidden {
                command.creation_flags(CREATE_NO_WINDOW);
            }
            command.status().unwrap();
        };
        run("control-no-flag", false);
        run("control-create-no-window", true);
    }

    let log = fs::read_to_string(&log_path).unwrap_or_default();
    let mut by_level: BTreeMap<&str, usize> = BTreeMap::new();
    for line in log.lines() {
        for level in ["TRACE", "DEBUG", " INFO", " WARN", "ERROR"] {
            if line.contains(level) {
                *by_level.entry(level).or_default() += 1;
                break;
            }
        }
    }
    report!("tracing events by level: {by_level:?}");
    for line in log.lines().take(4) {
        report!("tracing sample: {line}");
    }
    report!(
        "worktrunk bytes in tracing log: {}",
        fs::metadata(&log_path).unwrap().len()
    );
}

fn dunce_like(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .map(|path| {
            PathBuf::from(
                path.to_string_lossy()
                    .strip_prefix(r"\\?\")
                    .map(str::to_string)
                    .unwrap_or_else(|| path.to_string_lossy().to_string()),
            )
        })
        .unwrap()
}

/// `real <repo> <target> <branch>...`: cold and warm timing on a real history.
fn real_repo() {
    let args: Vec<String> = env::args().skip(2).collect();
    let repo = Repository::at(&args[0]).unwrap();
    for pass in ["cold", "warm"] {
        let repo = Repository::at(repo.discovery_path()).unwrap();
        let started = Instant::now();
        let snapshot = repo.capture_refs().unwrap();
        let captured = started.elapsed().as_millis();
        let mut found = BTreeMap::new();
        for branch in &args[2..] {
            let (_, reason) = repo.integration_reason(&snapshot, branch, &args[1]).unwrap();
            *found.entry(format!("{reason:?}")).or_insert(0) += 1;
        }
        report!(
            "{pass}: {} branches, capture {captured}ms, total {}ms, {found:?}",
            args.len() - 2,
            started.elapsed().as_millis()
        );
    }
}
