//! Working-copy operations used by the native composer. Git protects dirty files;
//! these operations never force a checkout, stash, or discard changes.
mod remote;
use crate::diagnostics::metrics;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Deserialize)]
pub struct Branch {
    pub name: String,
    pub remote: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: String,
    #[serde(alias = "isMain")]
    pub main: bool,
    pub missing: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub current: Option<String>,
    pub detached: bool,
    pub linked: bool,
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
}
/// The Git subcommand in `args`, past any global options such as `-c x=y`.
pub(crate) fn subcommand<'a>(args: &[&'a str]) -> &'a str {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "-c" | "-C" => {
                args.next();
            }
            option if option.starts_with('-') => {}
            subcommand => return subcommand,
        }
    }
    ""
}

pub(crate) fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    if crate::files::remote::is_remote(cwd) {
        return Err("This isn’t available for projects on another machine yet.".into());
    }
    let _span = tracing::info_span!("git", op = subcommand(args)).entered();
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    crate::process::configure_child(&mut command);
    let out = command.output().map_err(|e| {
        tracing::error!(error = %e, "could not run Git");
        metrics::failure("git.command", metrics::ErrorKind::of_spawn(&e));
        format!("Could not run Git: {e}")
    })?;
    if !out.status.success() {
        // Callers often probe and expect a failure, so this is only `debug`;
        // the operations they belong to log their own failures.
        tracing::debug!(status = out.status.code(), "git exited with an error");
        metrics::failure("git.command", metrics::ErrorKind::ExitStatus);
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    String::from_utf8(out.stdout).map_err(|e| {
        metrics::failure("git.command", metrics::ErrorKind::Parse);
        e.to_string()
    })
}
pub fn snapshot(cwd: &Path) -> Result<Snapshot, String> {
    if crate::files::remote::is_remote(cwd) {
        return remote::snapshot(cwd);
    }
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let current = git(cwd, &["symbolic-ref", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_owned());
    let detached = current.is_none();
    let current = current.or_else(|| {
        git(cwd, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_owned())
    });
    let trees = worktrees(cwd)?;
    let canonical = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_owned());
    let root = std::fs::canonicalize(root.trim()).unwrap_or(canonical);
    let linked = trees
        .iter()
        .any(|t| !t.main && std::fs::canonicalize(&t.path).ok().as_ref() == Some(&root));
    let branches = git(
        cwd,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)",
            "refs/heads/",
            "refs/remotes/",
        ],
    )?
    .lines()
    .filter_map(|line| {
        if let Some(name) = line.strip_prefix("refs/heads/") {
            Some(Branch {
                name: name.into(),
                remote: None,
            })
        } else {
            let (remote, name) = line.strip_prefix("refs/remotes/")?.split_once('/')?;
            (name != "HEAD").then(|| Branch {
                name: name.into(),
                remote: Some(remote.into()),
            })
        }
    })
    .collect();
    Ok(Snapshot {
        current,
        detached,
        linked,
        branches,
        worktrees: trees,
    })
}

/// Listing working copies does not require reading every local/remote branch.
pub fn worktrees(cwd: &Path) -> Result<Vec<Worktree>, String> {
    if crate::files::remote::is_remote(cwd) {
        return remote::worktrees(cwd);
    }
    let mut trees = Vec::new();
    let mut tree = Worktree::default();
    for field in git(cwd, &["worktree", "list", "--porcelain", "-z"])?.split('\0') {
        if field.is_empty() {
            if !tree.path.is_empty() {
                tree.main = trees.is_empty();
                tree.missing = !Path::new(&tree.path).is_dir();
                trees.push(std::mem::take(&mut tree));
            }
        } else if let Some(path) = field.strip_prefix("worktree ") {
            tree.path = path.into();
        } else if let Some(branch) = field.strip_prefix("branch refs/heads/") {
            tree.branch = Some(branch.into());
        } else if let Some(head) = field.strip_prefix("HEAD ") {
            tree.head = head.into();
        }
    }
    Ok(trees)
}
fn valid_branch(cwd: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with(['-', '@']) {
        return Err("Enter a valid branch name".into());
    }
    git(cwd, &["check-ref-format", "--branch", name]).map(|_| ())
}
pub fn checkout(cwd: &Path, branch: &Branch) -> Result<(), String> {
    if crate::files::remote::is_remote(cwd) {
        return remote::checkout(cwd, branch);
    }
    valid_branch(cwd, &branch.name)?;
    if let Some(remote) = &branch.remote {
        let reference = format!("{remote}/{}", branch.name);
        git(
            cwd,
            &["switch", "--track", "-c", &branch.name, "--", &reference],
        )?;
    } else {
        git(cwd, &["switch", "--", &branch.name])?;
    }
    Ok(())
}
pub fn create_branch(cwd: &Path, name: &str) -> Result<(), String> {
    if crate::files::remote::is_remote(cwd) {
        return remote::create_branch(cwd, name);
    }
    valid_branch(cwd, name)?;
    git(cwd, &["switch", "-c", name])?;
    Ok(())
}
/// Legacy branch prefixes, still recognised so existing worktrees keep working.
const LEGACY_BRANCH_PREFIXES: [&str; 1] = ["mc/"];
const APP_BRANCH_PREFIX: &str = "gespan/";

fn has_app_prefix(name: &str, suffix: &str) -> bool {
    std::iter::once(APP_BRANCH_PREFIX)
        .chain(LEGACY_BRANCH_PREFIXES)
        .any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(suffix))
        })
}

/// Branches Gespan created itself (current `gespan/` or legacy `mc/`).
pub fn is_app_branch(name: &str) -> bool {
    has_app_prefix(name, "")
}

/// Orchestration worker branches (`gespan/orch-…` or legacy `mc/orch-…`).
pub fn is_orchestration_branch(name: &str) -> bool {
    has_app_prefix(name, "orch-")
}

/// A short placeholder, like Tauri's temporaryWorktreeBranchName, until
/// [`rename_app_branch`] names it after the session's first message. The
/// session id keeps two sessions created in the same millisecond apart.
fn native_session_branch(session: i64, stamp: u128) -> String {
    format!(
        "{APP_BRANCH_PREFIX}{}{}",
        base36(stamp),
        base36(session.unsigned_abs().into())
    )
}

fn base36(mut value: u128) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit((value % 36) as u32, 36).unwrap_or('0'));
        value /= 36;
        if value == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

fn same_tree(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}

/// The branch the worktree at `worktree` is on, if it is one of `root`'s.
pub fn worktree_branch(root: &Path, worktree: &Path) -> Option<String> {
    worktrees(root)
        .ok()?
        .into_iter()
        .find(|tree| same_tree(Path::new(&tree.path), worktree))
        .and_then(|tree| tree.branch)
}

/// Renames the branch of the app-created worktree at `worktree` from the
/// placeholder `from` to `gespan/<fragment>`, as Tauri's renameWorktreeBranch
/// does once the first message names the work. A name already taken gets
/// `-2`, `-3`… Branches the user created, or renamed since `from` was read,
/// are never touched. Returns the new branch.
pub fn rename_app_branch(
    root: &Path,
    worktree: &Path,
    from: &str,
    fragment: &str,
) -> Result<String, String> {
    let fragment = crate::scm::text::sanitize_branch_fragment(
        fragment
            .trim()
            .trim_start_matches(APP_BRANCH_PREFIX)
            .trim_start_matches(LEGACY_BRANCH_PREFIXES[0]),
    );
    if fragment.is_empty() {
        return Err("Enter a valid branch name".into());
    }
    let trees = worktrees(root)?;
    let tree = trees
        .iter()
        .find(|tree| same_tree(Path::new(&tree.path), worktree))
        .ok_or("This path is not a registered worktree of this repository")?;
    if tree.main {
        return Err("The main working copy cannot be renamed here".into());
    }
    let current = tree.branch.as_deref().ok_or("The worktree is detached")?;
    if !is_app_branch(current) {
        return Err("Only automatically created worktree branches can be renamed".into());
    }
    if current != from {
        return Err("The branch was renamed while its name was being generated".into());
    }
    let taken = |name: &str| {
        git(
            root,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{name}"),
            ],
        )
        .is_ok()
    };
    let wanted = format!("{APP_BRANCH_PREFIX}{fragment}");
    if current == wanted {
        return Ok(wanted);
    }
    for name in std::iter::once(wanted.clone()).chain((2..=20).map(|n| format!("{wanted}-{n}"))) {
        if name == current {
            return Ok(name);
        }
        if taken(&name) {
            continue;
        }
        valid_branch(root, &name)?;
        match git(Path::new(&tree.path), &["branch", "-m", current, &name]) {
            Ok(_) => return Ok(name),
            // Another session took it between the check and the rename.
            Err(_) if taken(&name) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(format!("{wanted} and its numbered variants are taken"))
}

pub fn create_worktree(cwd: &Path, base: &str, session: i64) -> Result<PathBuf, String> {
    let _span = tracing::info_span!("worktree_create", session_id = session).entered();
    metrics::track(
        "worktree.create",
        create_session_worktree(cwd, base, session),
    )
    .inspect_err(|error| tracing::warn!(%error, "could not create a worktree"))
}
fn create_session_worktree(cwd: &Path, base: &str, session: i64) -> Result<PathBuf, String> {
    if crate::files::remote::is_remote(cwd) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        return remote::create_worktree(
            cwd,
            &native_session_branch(session, stamp),
            Some(base),
            false,
        )
        .map(|tree| PathBuf::from(tree.path));
    }
    let state = snapshot(cwd)?;
    let main = state.worktrees.first().ok_or("No working copies found")?;
    let main = Path::new(&main.path);
    let parent = main.with_file_name(format!(
        "{}-worktrees",
        main.file_name().unwrap_or_default().to_string_lossy()
    ));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let name = native_session_branch(session, stamp);
    let path = parent.join(name.replace('/', "-"));
    let commit = git(
        cwd,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    )?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    git(
        cwd,
        &[
            "worktree",
            "add",
            "--no-track",
            "-b",
            &name,
            "--",
            &path.to_string_lossy(),
            commit.trim(),
        ],
    )?;
    Ok(path)
}

/// SidebarWorktreeSwitcher: create a named copy from the selected checkout's
/// HEAD, alongside the main project's other worktrees. Existing files and
/// branches are never replaced.
pub fn create_named_worktree(cwd: &Path, branch: &str) -> Result<Worktree, String> {
    create_project_worktree(cwd, branch, None, false)
}
pub fn create_project_worktree(
    cwd: &Path,
    branch: &str,
    base: Option<&str>,
    existing: bool,
) -> Result<Worktree, String> {
    let _span = tracing::info_span!("worktree_create", existing).entered();
    metrics::track(
        "worktree.create",
        create_branch_worktree(cwd, branch, base, existing),
    )
    .inspect_err(|error| tracing::warn!(%error, "could not create a worktree"))
}
fn create_branch_worktree(
    cwd: &Path,
    branch: &str,
    base: Option<&str>,
    existing: bool,
) -> Result<Worktree, String> {
    if crate::files::remote::is_remote(cwd) {
        return remote::create_worktree(cwd, branch.trim(), base, existing);
    }
    let branch = branch.trim();
    valid_branch(cwd, branch)?;
    let state = snapshot(cwd)?;
    if state
        .worktrees
        .iter()
        .any(|tree| tree.branch.as_deref() == Some(branch))
    {
        return Err("This branch already has a working copy. Select it from the picker.".into());
    }
    let main = Path::new(
        &state
            .worktrees
            .first()
            .ok_or("No working copies found")?
            .path,
    );
    let parent = main.with_file_name(format!(
        "{}-worktrees",
        main.file_name().unwrap_or_default().to_string_lossy()
    ));
    let slug: String = branch
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = parent.join(slug);
    if path.exists() {
        return Err(format!(
            "{} already exists. Choose another branch name.",
            path.display()
        ));
    }
    let reference = if existing {
        format!("refs/heads/{branch}")
    } else {
        base.unwrap_or("HEAD").into()
    };
    let commit = git(
        cwd,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if existing {
        git(
            cwd,
            &["worktree", "add", "--", &path.to_string_lossy(), branch],
        )?;
    } else {
        git(
            cwd,
            &[
                "worktree",
                "add",
                "--no-track",
                "-b",
                branch,
                "--",
                &path.to_string_lossy(),
                commit.trim(),
            ],
        )?;
    }
    snapshot(cwd)?
        .worktrees
        .into_iter()
        .find(|tree| Path::new(&tree.path) == path)
        .ok_or_else(|| {
            "Worktree created, but could not be found. Refresh the working copies.".into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::metrics::{capture, ErrorKind};

    fn counted(
        counts: Vec<crate::diagnostics::metrics::Count>,
    ) -> Vec<(String, Option<ErrorKind>, u64)> {
        counts
            .into_iter()
            .map(|c| (c.name, c.kind, c.count))
            .collect()
    }

    #[test]
    fn failed_git_commands_are_counted_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let (result, counts) = capture(|| git(dir.path(), &["rev-parse", "--show-toplevel"]));
        assert!(result.is_err());
        assert_eq!(
            counted(counts),
            [("git.command".into(), Some(ErrorKind::ExitStatus), 1)]
        );
        let (result, counts) = capture(|| git(dir.path(), &["--version"]));
        assert!(result.is_ok());
        assert!(counts.is_empty(), "successful git commands are not counted");
    }

    #[test]
    fn worktree_create_outside_a_repository_counts_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (session, counts) = capture(|| create_worktree(dir.path(), "main", 1));
        assert!(session.is_err());
        let counts = counted(counts);
        assert!(
            counts.contains(&("worktree.create".into(), Some(ErrorKind::NotFound), 1)),
            "{counts:?}"
        );
        let (named, counts) = capture(|| create_named_worktree(dir.path(), "feature"));
        assert!(named.is_err());
        assert!(counted(counts)
            .iter()
            .any(|(name, kind, _)| name == "worktree.create" && kind.is_some()));
    }

    #[test]
    fn worktree_create_in_a_repository_counts_a_success() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| git(dir.path(), args).unwrap();
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "fixture@example.invalid"]);
        run(&["config", "user.name", "Fixture"]);
        std::fs::write(dir.path().join("file"), "x").unwrap();
        run(&["add", "file"]);
        run(&["commit", "-m", "Initial"]);
        let (tree, counts) = capture(|| create_named_worktree(dir.path(), "counted"));
        let tree = tree.unwrap();
        assert!(counted(counts).contains(&("worktree.create".into(), None, 1)));
        run(&["worktree", "remove", "--force", "--", &tree.path]);
        let _ = std::fs::remove_dir(Path::new(&tree.path).parent().unwrap());
    }
    #[test]
    fn native_session_worktrees_use_gespan_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        git(&cwd, &["init", "-b", "main"]).unwrap();
        std::fs::write(cwd.join("file"), "x\n").unwrap();
        git(&cwd, &["add", "file"]).unwrap();
        git(
            &cwd,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "init",
            ],
        )
        .unwrap();
        let path = create_worktree(&cwd, "main", 7).unwrap();
        let folder = path.file_name().unwrap().to_string_lossy().into_owned();
        // A short placeholder, not `native-<session>-<milliseconds>`.
        assert!(
            folder.starts_with("gespan-") && !folder.contains("native"),
            "{folder}"
        );
        assert!(folder.len() <= "gespan-".len() + 12, "{folder}");
        let branch = worktrees(&cwd)
            .unwrap()
            .into_iter()
            .find_map(|tree| tree.branch.filter(|b| b.starts_with("gespan/")));
        assert!(branch.as_deref().is_some_and(is_app_branch));
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        git(&cwd, &["init", "-b", "main"]).unwrap();
        std::fs::write(cwd.join("file"), "x\n").unwrap();
        git(&cwd, &["add", "file"]).unwrap();
        git(
            &cwd,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "init",
            ],
        )
        .unwrap();
        (temp, cwd)
    }

    fn branch_of(cwd: &Path, tree: &Path) -> Option<String> {
        worktrees(cwd)
            .unwrap()
            .into_iter()
            .find(|t| same_tree(Path::new(&t.path), tree))
            .and_then(|t| t.branch)
    }

    #[test]
    fn placeholder_branches_are_short_and_distinct_per_session() {
        let a = native_session_branch(7, 1_792_829_478_472);
        let b = native_session_branch(8, 1_792_829_478_472);
        assert_ne!(a, b);
        assert!(
            a.starts_with("gespan/") && a.len() <= "gespan/".len() + 12,
            "{a}"
        );
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }

    /// Renames from whatever the worktree is on now, as naming reads first.
    fn rename(cwd: &Path, tree: &Path, fragment: &str) -> Result<String, String> {
        let from = branch_of(cwd, tree).unwrap_or_default();
        rename_app_branch(cwd, tree, &from, fragment)
    }

    #[test]
    fn a_branch_renamed_while_naming_ran_is_left_alone() {
        let (_temp, cwd) = repo();
        let tree = create_worktree(&cwd, "main", 7).unwrap();
        let placeholder = branch_of(&cwd, &tree).unwrap();
        // The user renames it, still under the app's prefix, before the name arrives.
        git(&tree, &["branch", "-m", "gespan/my-own-name"]).unwrap();
        assert!(rename_app_branch(&cwd, &tree, &placeholder, "generated name").is_err());
        assert_eq!(
            branch_of(&cwd, &tree).as_deref(),
            Some("gespan/my-own-name")
        );
    }

    #[test]
    fn an_app_worktree_takes_the_name_of_its_work() {
        let (_temp, cwd) = repo();
        let tree = create_worktree(&cwd, "main", 7).unwrap();
        assert_eq!(
            rename(&cwd, &tree, "Open in Editor button").unwrap(),
            "gespan/open-in-editor-button"
        );
        assert_eq!(
            branch_of(&cwd, &tree).as_deref(),
            Some("gespan/open-in-editor-button")
        );
        // Already named: a repeat is a no-op, and a model's own prefix is not doubled.
        assert_eq!(
            rename(&cwd, &tree, "gespan/open-in-editor-button").unwrap(),
            "gespan/open-in-editor-button"
        );
    }

    #[test]
    fn a_taken_name_gets_the_next_number() {
        let (_temp, cwd) = repo();
        let first = create_worktree(&cwd, "main", 7).unwrap();
        let second = create_worktree(&cwd, "main", 8).unwrap();
        rename(&cwd, &first, "fix login").unwrap();
        assert_eq!(
            rename(&cwd, &second, "fix login").unwrap(),
            "gespan/fix-login-2"
        );
        assert_eq!(branch_of(&cwd, &first).as_deref(), Some("gespan/fix-login"));
    }

    #[test]
    #[ignore = "manual integration check: asks the installed Claude CLI for a name"]
    fn a_live_provider_names_a_new_worktree_after_its_message() {
        let (_temp, cwd) = repo();
        let tree = create_worktree(&cwd, "main", 7).unwrap();
        let placeholder = branch_of(&cwd, &tree).unwrap();
        let fragment = crate::session_title::generate_branch_name(
            crate::provider::Kind::Claude,
            &tree,
            "Add an Open in editor button to the session's title bar",
            Default::default(),
            Default::default(),
        )
        .expect("Claude suggested a branch name");
        let branch = rename_app_branch(&cwd, &tree, &placeholder, &fragment).unwrap();
        println!("named: {branch}");
        assert!(branch.starts_with("gespan/") && branch.len() > "gespan/".len());
        assert!(branch.contains("editor"), "{branch}");
        assert_eq!(branch_of(&cwd, &tree).as_deref(), Some(branch.as_str()));
    }

    #[test]
    fn branches_the_user_owns_are_never_renamed() {
        let (_temp, cwd) = repo();
        let tree = create_named_worktree(&cwd, "my-feature").unwrap();
        let path = PathBuf::from(&tree.path);
        assert!(rename(&cwd, &path, "something else").is_err());
        assert_eq!(branch_of(&cwd, &path).as_deref(), Some("my-feature"));
        assert!(rename(&cwd, &cwd, "main work").is_err(), "main copy");
        let app = create_worktree(&cwd, "main", 9).unwrap();
        assert!(rename(&cwd, &app, " ... ").is_err(), "nothing usable");
    }
    #[test]
    fn app_and_orchestration_branches_accept_legacy_prefix() {
        assert!(is_app_branch("gespan/x") && is_app_branch("mc/12345678"));
        assert!(!is_app_branch("feature/x") && !is_app_branch("mcx/y"));
        assert!(is_orchestration_branch("mc/orch-x"));
        assert!(is_orchestration_branch("gespan/orch-x"));
        assert!(!is_orchestration_branch("feature/x"));
        assert!(!is_orchestration_branch("gespan/other"));
    }
    #[test]
    fn sidebar_named_copies_preserve_dirty_files_and_refuse_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        git(&cwd, &["init", "-b", "main"]).unwrap();
        git(&cwd, &["config", "core.autocrlf", "false"]).unwrap();
        std::fs::write(cwd.join("file"), "committed\n").unwrap();
        git(&cwd, &["add", "file"]).unwrap();
        git(
            &cwd,
            &[
                "-c",
                "user.name=Native test",
                "-c",
                "user.email=native@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "Initial",
            ],
        )
        .unwrap();
        std::fs::write(cwd.join("file"), "unsaved checkout changes\n").unwrap();
        let tree = create_named_worktree(&cwd, "feature/sidebar").unwrap();
        assert_eq!(tree.branch.as_deref(), Some("feature/sidebar"));
        assert_eq!(
            Path::new(&tree.path).file_name().unwrap(),
            "feature-sidebar"
        );
        assert_eq!(
            std::fs::read_to_string(cwd.join("file")).unwrap(),
            "unsaved checkout changes\n"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&tree.path).join("file")).unwrap(),
            "committed\n"
        );
        assert!(create_named_worktree(&cwd, "feature/sidebar")
            .unwrap_err()
            .contains("already has a working copy"));
        assert!(create_named_worktree(&cwd, "-invalid").is_err());
        let collision = temp.path().join("project-worktrees/collision");
        std::fs::create_dir(&collision).unwrap();
        std::fs::write(collision.join("keep"), "keep").unwrap();
        assert!(create_named_worktree(&cwd, "collision")
            .unwrap_err()
            .contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(collision.join("keep")).unwrap(),
            "keep"
        );
    }
    #[test]
    fn working_copy_operations_preserve_dirty_files_and_track_remotes() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        git(&cwd, &["init", "-b", "main"]).unwrap();
        git(&cwd, &["config", "core.autocrlf", "false"]).unwrap();
        git(&cwd, &["config", "user.name", "Native review"]).unwrap();
        git(&cwd, &["config", "user.email", "native@example.invalid"]).unwrap();
        std::fs::write(cwd.join("file"), "main\n").unwrap();
        git(&cwd, &["add", "file"]).unwrap();
        git(
            &cwd,
            &["-c", "commit.gpgsign=false", "commit", "-m", "Initial"],
        )
        .unwrap();
        create_branch(&cwd, "feature").unwrap();
        std::fs::write(cwd.join("file"), "feature\n").unwrap();
        git(&cwd, &["add", "file"]).unwrap();
        git(
            &cwd,
            &["-c", "commit.gpgsign=false", "commit", "-m", "Feature"],
        )
        .unwrap();
        checkout(
            &cwd,
            &Branch {
                name: "main".into(),
                remote: None,
            },
        )
        .unwrap();
        std::fs::write(cwd.join("file"), "my unsaved work\n").unwrap();
        assert!(checkout(
            &cwd,
            &Branch {
                name: "feature".into(),
                remote: None
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(cwd.join("file")).unwrap(),
            "my unsaved work\n"
        );
        assert_eq!(snapshot(&cwd).unwrap().current.as_deref(), Some("main"));
        let tree = create_worktree(&cwd, "feature", 7).unwrap();
        let state = snapshot(&tree).unwrap();
        assert!(state.linked);
        assert_eq!(state.worktrees.len(), 2);
        assert_eq!(
            std::fs::read_to_string(tree.join("file")).unwrap(),
            "feature\n"
        );
        git(&cwd, &["remote", "add", "origin", "/unused"]).unwrap();
        git(
            &cwd,
            &["update-ref", "refs/remotes/origin/remote-feature", "HEAD"],
        )
        .unwrap();
        checkout(
            &cwd,
            &Branch {
                name: "remote-feature".into(),
                remote: Some("origin".into()),
            },
        )
        .unwrap();
        assert_eq!(
            snapshot(&cwd).unwrap().current.as_deref(),
            Some("remote-feature")
        );
        assert_eq!(
            git(&cwd, &["rev-parse", "--abbrev-ref", "@{upstream}"])
                .unwrap()
                .trim(),
            "origin/remote-feature"
        );
        assert!(create_branch(&cwd, "--orphan").is_err());
    }

    #[test]
    fn a_failing_git_command_logs_only_at_debug() {
        let (_temp, cwd) = repo();
        let probe = [
            "-c",
            "core.quotePath=false",
            "show-ref",
            "--verify",
            "refs/heads/nope",
        ];
        let quiet = crate::diagnostics::test_capture(|| assert!(git(&cwd, &probe).is_err()));
        assert!(quiet.is_empty(), "{quiet:?}");
        let logged = crate::diagnostics::test_capture_with("gespan_core::git=debug", || {
            assert!(git(&cwd, &probe).is_err());
        });
        assert_eq!(logged.len(), 1, "{logged:?}");
        let line = &logged[0];
        assert_eq!(line["level"], "debug");
        assert_eq!(line["span"], "git");
        // The subcommand only: never its arguments, which can name branches or paths.
        assert_eq!(line["fields"]["op"], "show-ref");
        assert!(line["fields"]["status"].is_number());
        assert!(!line.to_string().contains("refs/heads/nope"), "{line}");
    }

    #[test]
    fn a_failed_worktree_create_is_logged_with_its_session() {
        let (_temp, cwd) = repo();
        let mut created = Ok(PathBuf::new());
        let logged = crate::diagnostics::test_capture(|| {
            created = create_worktree(&cwd, "no-such-base", 7);
        });
        let error = created.unwrap_err();
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert_eq!(logged[0]["level"], "warn");
        assert_eq!(logged[0]["span"], "worktree_create");
        assert_eq!(logged[0]["session_id"], 7);
        assert_eq!(logged[0]["fields"], serde_json::json!({"error": error}));
    }

    #[test]
    fn a_failed_named_worktree_create_is_logged() {
        let (_temp, cwd) = repo();
        create_named_worktree(&cwd, "taken").unwrap();
        let mut created = Ok(None);
        let logged = crate::diagnostics::test_capture(|| {
            created = create_named_worktree(&cwd, "taken").map(Some);
        });
        assert!(created.is_err());
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert_eq!(logged[0]["level"], "warn");
        assert_eq!(logged[0]["span"], "worktree_create");
        // The branch name reaches the log only inside Git's error text, never as a field.
        let fields = logged[0]["fields"].as_object().unwrap();
        let mut keys: Vec<_> = fields.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["error", "existing"], "{fields:?}");
        assert_eq!(fields["existing"], false);
    }
}
