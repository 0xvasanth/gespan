use crate::decisions::files::{
    ensure_excluded, remove_excluded, replace_section, user_edited_section, write_atomic, BEGIN,
    END,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn section(body: &str) -> String {
    format!("{BEGIN}\n{body}\n{END}\n")
}

// Trailing spaces, tabs, unicode, a code fence and no final newline.
const USER_TEXT: &str =
    "# Notes  \n\ttabbed line\t\nünïcode ✓ 日本語\n```\nfenced\n```\nlast line, no newline";

#[test]
fn appends_section_when_missing_and_keeps_user_text_byte_for_byte() {
    let out = replace_section(USER_TEXT, "- D-1 use sqlite");
    assert!(out.starts_with(USER_TEXT));
    assert_eq!(
        out,
        format!("{USER_TEXT}\n\n{}", section("- D-1 use sqlite"))
    );
    // A file that already ends in a newline gets exactly one blank line.
    let out = replace_section("hello\n", "x");
    assert_eq!(out, format!("hello\n\n{}", section("x")));
    // An empty file gets just the section.
    assert_eq!(replace_section("", "x"), section("x"));
}

#[test]
fn replaces_only_between_markers() {
    let before = "intro  \n\n";
    let after = "\noutro\t\nno newline";
    let existing = format!("{before}{}{after}", section("old\nbody"));
    let out = replace_section(&existing, "new body");
    assert_eq!(out, format!("{before}{}{after}", section("new body")));
    // Re-applying the same body is a no-op.
    assert_eq!(replace_section(&out, "new body"), out);
}

#[test]
fn empty_body_removes_section_and_its_blank_line_only() {
    let with = replace_section(USER_TEXT, "body");
    assert_eq!(replace_section(&with, ""), format!("{USER_TEXT}\n"));

    let with = replace_section("a\n", "body");
    assert_eq!(replace_section(&with, ""), "a\n");

    // Text after the section is untouched; a blank line the user added
    // after it stays.
    let existing = format!("a\n\n{}\nb\n", section("body"));
    assert_eq!(replace_section(&existing, ""), "a\n\nb\n");

    // Nothing to remove: unchanged, byte for byte.
    assert_eq!(replace_section(USER_TEXT, ""), USER_TEXT);
    // A section alone in the file leaves an empty file.
    assert_eq!(replace_section(&section("x"), ""), "");
}

#[test]
fn begin_without_end_keeps_everything_and_appends_one_new_section() {
    let existing = format!("top\n{BEGIN}\nuser wrote this after a damaged marker\n");
    let out = replace_section(&existing, "fresh");
    assert!(out.starts_with(&existing));
    assert_eq!(out, format!("{existing}\n{}", section("fresh")));
    assert_eq!(out.matches(BEGIN).count(), 2);
    assert_eq!(out.matches(&format!("{END}\n")).count(), 1);
    // The orphan stays orphaned on the next pass: still one complete section.
    let again = replace_section(&out, "fresh2");
    assert!(again.starts_with(&existing));
    assert!(again.ends_with(&section("fresh2")));
    assert_eq!(again.matches(END).count(), 1);
    // Removing keeps the user's orphan line.
    assert_eq!(replace_section(&out, ""), existing);
}

#[test]
fn end_without_begin_is_user_text() {
    let existing = format!("a\n{END}\nb\n");
    let out = replace_section(&existing, "x");
    assert!(out.starts_with(&existing));
    assert!(out.ends_with(&section("x")));
}

#[test]
fn two_sections_collapse_to_the_first() {
    let existing = format!("a\n\n{}mid\n\n{}tail\n", section("one"), section("two"));
    let out = replace_section(&existing, "new");
    assert_eq!(out, format!("a\n\n{}mid\ntail\n", section("new")));
    assert_eq!(out.matches(BEGIN).count(), 1);
    assert_eq!(replace_section(&existing, ""), "a\nmid\ntail\n");
}

#[test]
fn markers_inside_a_code_fence_are_ignored_when_a_real_section_exists() {
    let example = format!("doc\n```\n{BEGIN}\nexample\n{END}\n```\nafter\n");
    let existing = format!("{example}\n{}", section("old"));
    let out = replace_section(&existing, "real");
    assert_eq!(out, format!("{example}\n{}", section("real")));
    assert_eq!(replace_section(&out, ""), example);
    // A marker with extra text on its line is not a marker.
    let inline = format!("see {BEGIN} here\n");
    assert!(replace_section(&inline, "x").starts_with(&inline));
    assert_eq!(replace_section(&inline, "x").matches(END).count(), 1);
}

#[test]
fn the_only_fenced_pair_is_taken_as_ours_and_stays_stable() {
    // Accepted trade-off: a lone fenced example of the exact markers.
    let existing = format!("doc\n```\n{BEGIN}\nexample\n{END}\n```\nafter\n");
    let r1 = replace_section(&existing, "real");
    assert_eq!(r1.matches(BEGIN).count(), 1);
    assert_eq!(replace_section(&r1, "real"), r1);
}

#[test]
fn markers_tolerate_surrounding_whitespace() {
    let existing = format!("  {BEGIN}\t\nold\n {END}  \nrest\n");
    let out = replace_section(&existing, "new");
    assert!(out.contains("new"));
    assert!(!out.contains("old"));
    assert!(out.ends_with("rest\n"));
    assert_eq!(out.matches(BEGIN).count(), 1);
}

#[test]
fn crlf_files_stay_crlf() {
    let existing = "line one\r\nline two\r\n";
    let out = replace_section(existing, "a\nb");
    assert_eq!(
        out,
        format!("line one\r\nline two\r\n\r\n{BEGIN}\r\na\r\nb\r\n{END}\r\n")
    );
    assert!(!out.replace("\r\n", "").contains('\n'));
    let replaced = replace_section(&out, "c");
    assert_eq!(
        replaced,
        format!("line one\r\nline two\r\n\r\n{BEGIN}\r\nc\r\n{END}\r\n")
    );
    assert_eq!(replace_section(&replaced, ""), existing);
    // No trailing newline in a CRLF file: the added newline is CRLF.
    let out = replace_section("x\r\ny", "z");
    assert!(out.starts_with("x\r\ny\r\n\r\n"));
}

#[test]
fn user_edited_section_detects_change() {
    let written = replace_section("user\n", "line 1\nline 2");
    assert!(!user_edited_section(&written, &written));
    // Edits outside the section are not edits of the section.
    let outside = format!("added by user\n{written}");
    assert!(!user_edited_section(&outside, &written));
    // An edit inside is.
    let edited = written.replace("line 2", "line 2 (mine)");
    assert!(user_edited_section(&edited, &written));
    // Line-ending conversion alone is not an edit.
    assert!(!user_edited_section(
        &written.replace('\n', "\r\n"),
        &written
    ));
    // A file with no section has nothing of ours to lose.
    assert!(!user_edited_section("plain\n", &written));
}

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
    git(dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
}

fn exclude_lines(common: &Path) -> Vec<String> {
    fs::read_to_string(common.join("info").join("exclude"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn ensure_excluded_is_idempotent_and_writes_to_common_dir_from_a_worktree() {
    let tmp = TempDir::new().unwrap();
    let main = tmp.path().join("main");
    fs::create_dir(&main).unwrap();
    init_repo(&main);
    let wt: PathBuf = tmp.path().join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );

    let file = wt.join("sub").join("CLAUDE.md");
    fs::create_dir(wt.join("sub")).unwrap();
    fs::write(&file, "x").unwrap();
    // Pre-existing user entries stay, including one with no final newline.
    let info = main.join(".git").join("info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("exclude"), "# mine\n*.log").unwrap();

    ensure_excluded(&wt, &file).unwrap();
    ensure_excluded(&wt, &file).unwrap();
    assert_eq!(
        exclude_lines(&main.join(".git")),
        ["# mine", "*.log", "/sub/CLAUDE.md"]
    );
    // Nothing was written into the worktree's private git dir.
    assert!(!main
        .join(".git")
        .join("worktrees")
        .join("wt")
        .join("info")
        .join("exclude")
        .exists());
    // Git really ignores it.
    let status = Command::new("git")
        .arg("-C")
        .arg(&wt)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&status.stdout).contains("CLAUDE.md"));

    // A second file is appended; removing the first leaves everything else.
    let other = wt.join("AGENTS.md");
    ensure_excluded(&wt, &other).unwrap();
    remove_excluded(&wt, &file).unwrap();
    remove_excluded(&wt, &file).unwrap();
    assert_eq!(
        exclude_lines(&main.join(".git")),
        ["# mine", "*.log", "/AGENTS.md"]
    );
}

#[test]
fn ensure_excluded_creates_info_dir_in_a_plain_repo() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let info = tmp.path().join(".git").join("info");
    let _ = fs::remove_dir_all(&info);
    ensure_excluded(tmp.path(), &tmp.path().join("CLAUDE.md")).unwrap();
    assert_eq!(exclude_lines(&tmp.path().join(".git")), ["/CLAUDE.md"]);
}

#[test]
fn ensure_excluded_outside_git_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("CLAUDE.md");
    ensure_excluded(tmp.path(), &file).unwrap();
    remove_excluded(tmp.path(), &file).unwrap();
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn ensure_excluded_for_a_file_outside_the_work_tree_is_a_no_op() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    init_repo(&repo);
    let elsewhere = tmp.path().join("elsewhere.md");
    let before = exclude_lines(&repo.join(".git"));
    ensure_excluded(&repo, &elsewhere).unwrap();
    remove_excluded(&repo, &elsewhere).unwrap();
    assert_eq!(exclude_lines(&repo.join(".git")), before);
}

#[test]
fn write_atomic_replaces_content_and_leaves_no_temp_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("out.md");
    write_atomic(&path, "first", tmp.path()).unwrap();
    write_atomic(&path, "second ✓", tmp.path()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "second ✓");
    let names: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["out.md"]);
    // Missing parent directories are created.
    let nested = tmp.path().join("a").join("b").join("f.md");
    write_atomic(&nested, "n", tmp.path()).unwrap();
    assert_eq!(fs::read_to_string(nested).unwrap(), "n");
}

#[cfg(unix)]
#[test]
fn write_atomic_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("script.md");
    fs::write(&path, "x").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    write_atomic(&path, "y", tmp.path()).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[cfg(unix)]
#[test]
fn write_atomic_never_leaves_partial_file() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("CLAUDE.md");
    fs::write(&path, "original").unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let running_as_root = fs::File::create(tmp.path().join("probe")).is_ok();
    let result = write_atomic(&path, "replacement", tmp.path());
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755)).unwrap();
    if running_as_root {
        return; // Read-only directories do not stop root.
    }
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    let names: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["CLAUDE.md"]);
}

#[test]
fn write_atomic_failure_keeps_original_when_rename_fails() {
    // The target is a non-empty directory, so the final rename must fail.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("target");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("child"), "x").unwrap();
    assert!(write_atomic(&path, "data", tmp.path()).is_err());
    assert!(path.join("child").exists());
    let names: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["target"], "temp file cleaned up");
}

#[test]
fn a_stray_unclosed_fence_does_not_hide_the_section_or_duplicate_it() {
    let stray = "intro  \n```\nunclosed example\t\n\n";
    let first = replace_section(stray, "one");
    let second = replace_section(&first, "two");
    let third = replace_section(&second, "two");
    assert_eq!(second.matches(BEGIN).count(), 1);
    assert_eq!(third, second);
    assert!(second.starts_with(stray));
    assert_eq!(replace_section(&second, ""), stray);
}

#[test]
fn tilde_and_longer_fences_hide_markers_until_a_matching_closer() {
    let real = section("old");
    let tilde = format!("~~~\n{BEGIN}\nx\n{END}\n~~~\n");
    let existing = format!("{tilde}\n{real}");
    let out = replace_section(&existing, "new");
    assert_eq!(out, format!("{tilde}\n{}", section("new")));
    assert_eq!(replace_section(&out, ""), tilde);

    // ``` inside a ```` block does not close it; ~~~ does not close ```.
    let long = format!("````\n```\n{BEGIN}\nx\n{END}\n```\n````\n```\n~~~\n{END}\n```\n");
    let existing = format!("{long}\n{real}");
    let out = replace_section(&existing, "new");
    assert_eq!(out, format!("{long}\n{}", section("new")));
    assert_eq!(replace_section(&out, ""), long);
}

#[test]
fn an_orphan_begin_does_not_capture_a_fenced_example_when_a_real_section_exists() {
    let user = format!("{BEGIN}\nuser\n```\n{END}\n```\nafter\n");
    let existing = format!("{user}\n{}", section("old"));
    let out = replace_section(&existing, "fresh");
    assert_eq!(out, format!("{user}\n{}", section("fresh")));
    assert_eq!(replace_section(&out, ""), user);
}

#[cfg(unix)]
fn symlink_or_skip(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(not(unix))]
fn symlink_or_skip(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[test]
fn write_atomic_writes_through_a_symlink_and_keeps_the_link() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("AGENTS.md");
    let mid = tmp.path().join("mid.md");
    let link = tmp.path().join("CLAUDE.md");
    fs::write(&real, "old").unwrap();
    if !symlink_or_skip(Path::new("AGENTS.md"), &mid)
        || !symlink_or_skip(Path::new("mid.md"), &link)
    {
        return; // Symlinks unavailable (unprivileged Windows).
    }
    write_atomic(&link, "new", tmp.path()).unwrap();
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(&mid).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_to_string(&real).unwrap(), "new");
    assert_eq!(fs::read_to_string(&link).unwrap(), "new");
}

#[test]
fn ensure_excluded_lists_a_symlink_under_its_own_name() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let real = tmp.path().join("AGENTS.md");
    let link = tmp.path().join("CLAUDE.md");
    fs::write(&real, "x").unwrap();
    if !symlink_or_skip(Path::new("AGENTS.md"), &link) {
        return;
    }
    let before = exclude_lines(&tmp.path().join(".git"));
    ensure_excluded(tmp.path(), &link).unwrap();
    let mut expected = before.clone();
    expected.push("/CLAUDE.md".to_owned());
    assert_eq!(exclude_lines(&tmp.path().join(".git")), expected);
    remove_excluded(tmp.path(), &link).unwrap();
    assert_eq!(exclude_lines(&tmp.path().join(".git")), before);
}

#[test]
fn ensure_excluded_keeps_crlf_endings() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let exclude = tmp.path().join(".git").join("info").join("exclude");
    fs::write(&exclude, "# mine\r\n*.log").unwrap();
    ensure_excluded(tmp.path(), &tmp.path().join("CLAUDE.md")).unwrap();
    assert_eq!(
        fs::read_to_string(&exclude).unwrap(),
        "# mine\r\n*.log\r\n/CLAUDE.md\r\n"
    );
}

#[test]
fn a_stray_fence_before_the_section_and_a_balanced_block_after_it_stays_idempotent() {
    let user_head = "intro\n```
stray opener\n";
    let user_tail = "\nlater\n```\ncode\n```\nend\n";
    let existing = format!("{user_head}\n{}{user_tail}", section("old"));
    let first = replace_section(&existing, "new");
    assert_eq!(first.matches(BEGIN).count(), 1);
    assert_eq!(first, format!("{user_head}\n{}{user_tail}", section("new")));
    assert_eq!(replace_section(&first, "new"), first);
    let second = replace_section(&first, "newer");
    assert_eq!(second.matches(BEGIN).count(), 1);
    assert!(second.starts_with(user_head) && second.ends_with(user_tail));
    // Also when Gespan appended it itself.
    let appended = replace_section(&format!("{user_head}{user_tail}"), "x");
    assert_eq!(replace_section(&appended, "x"), appended);
    assert_eq!(replace_section(&appended, "").matches(BEGIN).count(), 0);
}

#[test]
fn regeneration_is_idempotent_for_generated_messy_inputs() {
    let pool = [
        "text  ",
        "",
        "```",
        "~~~",
        "````",
        "```rust",
        BEGIN,
        END,
        "\tindented\t",
        "ünï ✓",
    ];
    let example = format!("{BEGIN}\nexample\n{END}");
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for case in 0..400 {
        let n = next() % 9;
        let mut lines: Vec<String> = (0..n)
            .map(|_| {
                if next() % 7 == 0 {
                    example.clone()
                } else {
                    pool[next() % pool.len()].to_owned()
                }
            })
            .collect();
        if next() % 3 == 0 {
            lines.push(String::new());
        }
        let mut x = lines.join("\n");
        if next() % 2 == 0 {
            x.push('\n');
        }
        let r1 = replace_section(&x, "body");
        let r2 = replace_section(&r1, "body");
        assert_eq!(r2, r1, "case {case}: not idempotent for {x:?}");
        let r3 = replace_section(&r1, "other");
        assert_eq!(
            r3.matches(BEGIN).count(),
            r1.matches(BEGIN).count(),
            "case {case}: section count grew for {x:?}"
        );
        // When the input holds nothing we would treat as ours, adding and
        // removing a section returns the user's bytes (a missing final
        // newline gains one).
        if replace_section(&x, "") == x {
            let back = replace_section(&r1, "");
            let ok = back == x || (!x.ends_with('\n') && back == format!("{x}\n"));
            assert!(ok, "case {case}: {x:?} came back as {back:?}");
        }
    }
}

#[test]
fn write_atomic_on_a_symlink_loop_errors_and_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a.md");
    let b = tmp.path().join("b.md");
    if !symlink_or_skip(Path::new("b.md"), &a) || !symlink_or_skip(Path::new("a.md"), &b) {
        return;
    }
    assert!(write_atomic(&a, "data", tmp.path()).is_err());
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
}

#[test]
fn write_atomic_refuses_a_link_that_escapes_the_allowed_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("authorized_keys");
    fs::write(&secret, "keep me").unwrap();
    let link = root.join("CLAUDE.local.md");
    if !symlink_or_skip(&secret, &link) {
        return;
    }
    let err = write_atomic(&link, "gespan", &root).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("CLAUDE.local.md"));
    assert_eq!(fs::read_to_string(&secret).unwrap(), "keep me");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
}

#[test]
fn write_atomic_refuses_a_dangling_link_that_escapes_and_creates_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    let link = root.join("AGENTS.md");
    if !symlink_or_skip(&outside.join("new").join("f.md"), &link) {
        return;
    }
    assert!(write_atomic(&link, "x", &root).is_err());
    assert!(!outside.exists(), "no directory created outside the root");
    // Dangling but escaping through an existing outside directory.
    fs::create_dir_all(&outside).unwrap();
    let link2 = root.join("B.md");
    if symlink_or_skip(&outside.join("f.md"), &link2) {
        assert!(write_atomic(&link2, "x", &root).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }
}

#[test]
fn write_atomic_follows_a_link_that_stays_inside_the_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs").join("real.md"), "old").unwrap();
    let link = root.join("CLAUDE.md");
    if !symlink_or_skip(&Path::new("docs").join("real.md"), &link) {
        return;
    }
    write_atomic(&link, "new", &root).unwrap();
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_to_string(root.join("docs").join("real.md")).unwrap(),
        "new"
    );
}

#[test]
fn write_atomic_refuses_a_plain_path_outside_the_allowed_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let outside = tmp.path().join("elsewhere").join("f.md");
    assert!(write_atomic(&outside, "x", &root).is_err());
    assert!(!tmp.path().join("elsewhere").exists());
}

#[test]
fn many_stray_fences_are_scanned_in_linear_passes() {
    let mut text = "```\n".repeat(10_000);
    text.push_str("tail");
    let out = replace_section(&text, "body");
    assert!(out.starts_with(&text));
    assert_eq!(replace_section(&out, "body"), out);
    assert_eq!(replace_section(&out, "").trim_end(), text.trim_end());
}

#[test]
fn begin_fence_end_fence_counterexample_is_idempotent() {
    let x = format!("```\n{BEGIN}\n```\n{END}\n```\n");
    let r1 = replace_section(&x, "body");
    assert_eq!(r1.matches(BEGIN).count(), 1);
    let r2 = replace_section(&r1, "body");
    assert_eq!(r2, r1);
}

#[test]
fn regeneration_is_idempotent_for_every_short_sequence_of_fences_and_markers() {
    let alphabet = ["```", "~~~", BEGIN, END, "text"];
    let bodies = ["b", "b\n```\nc\n```"];
    let mut checked = 0usize;
    for len in 0..=6usize {
        let total = alphabet.len().pow(len as u32);
        for mut code in 0..total {
            let mut lines = Vec::with_capacity(len);
            for _ in 0..len {
                lines.push(alphabet[code % alphabet.len()]);
                code /= alphabet.len();
            }
            for trailing_newline in [true, false] {
                let mut x = lines.join("\n");
                if trailing_newline && !x.is_empty() {
                    x.push('\n');
                }
                for body in bodies {
                    let r1 = replace_section(&x, body);
                    assert_eq!(
                        replace_section(&r1, body),
                        r1,
                        "not idempotent: {x:?} / {body:?}"
                    );
                    let r3 = replace_section(&r1, "other");
                    assert_eq!(
                        r3.matches(BEGIN).count(),
                        r1.matches(BEGIN).count(),
                        "section count changed: {x:?} / {body:?}"
                    );
                    if replace_section(&x, "") == x {
                        let back = replace_section(&r1, "");
                        assert!(
                            back == x || (!x.ends_with('\n') && back == format!("{x}\n")),
                            "removal did not restore {x:?} / {body:?}: {back:?}"
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 50_000);
}

#[test]
fn write_atomic_refuses_parent_dir_components_and_creates_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let sneaky = root
        .join("missing")
        .join("..")
        .join("..")
        .join("outside")
        .join("f.md");
    let err = write_atomic(&sneaky, "x", &root).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(!root.join("missing").exists());
    assert!(!tmp.path().join("outside").exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}
