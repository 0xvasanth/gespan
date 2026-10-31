//! Whether a branch's work already reached the default branch, including when
//! it was rebased or squash-merged. The checks are ported from worktrunk (see
//! NOTICE), and run through Gespan's own Git handling.
//!
//! Every check reads commit ids, so its answer is about the commit it read. A
//! caller that acts on the answer has to act on `Merged::branch_sha`.
use crate::git::{git, git_answer};
use std::{collections::HashSet, path::Path};

/// Past this many commits on the target, the patch-id scan costs seconds and
/// the answer stays "not merged".
const PATCH_ID_SCAN_MAX_COMMITS: usize = 500;
/// The whole diff has to read the same however the two sides detect renames.
const NO_RENAMES: &str = "--no-renames";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    SameCommit,
    Ancestor,
    NoChanges,
    SameFiles,
    MergeAddsNothing,
    /// One commit on the target made the branch's whole change. A later commit
    /// may have reverted it, so this is weaker evidence than the others.
    SameChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    pub how: How,
    /// The branch it was found in, such as `main` or `origin/main`.
    pub target: String,
    pub branch_sha: String,
}
impl Merged {
    pub fn reason(&self) -> String {
        let target = &self.target;
        match self.how {
            How::SameCommit => format!("Same commit as {target}"),
            How::Ancestor => format!("All its commits are on {target}"),
            How::NoChanges => "It has no file changes".to_owned(),
            How::SameFiles => format!("Its files match {target}"),
            How::MergeAddsNothing => format!("Merging it into {target} would change nothing"),
            How::SameChange => format!("A commit on {target} made the same change"),
        }
    }
}

/// The branch finished work lands on, for a repository that has one.
pub fn default_target(cwd: &Path) -> Option<String> {
    let exists = |name: &str| {
        matches!(
            git_answer(
                cwd,
                &["show-ref", "--verify", "--quiet", &format!("refs/{name}")],
                &[],
                None
            ),
            Ok((Some(0), _))
        )
    };
    let (code, head) = git_answer(
        cwd,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        &[],
        None,
    )
    .ok()?;
    if code == Some(0) {
        let head = String::from_utf8_lossy(&head);
        if let Some(remote) = head.trim().strip_prefix("refs/remotes/") {
            let local = remote.split_once('/').map_or(remote, |(_, name)| name);
            return Some(if exists(&format!("heads/{local}")) {
                local.to_owned()
            } else {
                remote.to_owned()
            });
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|name| exists(&format!("heads/{name}")))
        .map(str::to_owned)
}

/// How `branch` reached the default branch. `None` when it hasn't, or when the
/// repository has no default branch to compare with.
pub fn merged(cwd: &Path, branch: &str) -> Result<Option<Merged>, String> {
    let Some(target) = default_target(cwd) else {
        return Ok(None);
    };
    merged_into(cwd, branch, &target)
}

pub fn merged_into(cwd: &Path, branch: &str, target: &str) -> Result<Option<Merged>, String> {
    if branch == target {
        return Ok(None);
    }
    let branch_sha = commit(cwd, &format!("refs/heads/{branch}"))?;
    for candidate in targets(cwd, target)? {
        if let Some(how) = against(cwd, &branch_sha, &candidate)? {
            return Ok(Some(Merged {
                how,
                target: candidate,
                branch_sha,
            }));
        }
    }
    Ok(None)
}

/// A branch can be merged upstream while local `main` is behind, so whichever
/// side has more gets checked, and both when they diverged.
fn targets(cwd: &Path, target: &str) -> Result<Vec<String>, String> {
    let reference = format!("refs/heads/{target}");
    let name = git(
        cwd,
        &["for-each-ref", "--format=%(upstream:short)", &reference],
    )?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Ok(vec![target.to_owned()]);
    }
    let local = commit(cwd, target)?;
    let remote = commit(cwd, &name)?;
    Ok(if local == remote || is_ancestor(cwd, &remote, &local)? {
        vec![target.to_owned()]
    } else if is_ancestor(cwd, &local, &remote)? {
        vec![name]
    } else {
        vec![target.to_owned(), name]
    })
}

/// Cheapest check first, stopping at the first that answers.
fn against(cwd: &Path, branch: &str, target: &str) -> Result<Option<How>, String> {
    let target_sha = commit(cwd, target)?;
    if branch == target_sha {
        return Ok(Some(How::SameCommit));
    }
    if is_ancestor(cwd, branch, &target_sha)? {
        return Ok(Some(How::Ancestor));
    }
    let Some(base) = merge_base(cwd, branch, &target_sha)? else {
        return Ok(None);
    };
    let branch_tree = tree(cwd, branch)?;
    if branch_tree == tree(cwd, &base)? {
        return Ok(Some(How::NoChanges));
    }
    let target_tree = tree(cwd, &target_sha)?;
    if branch_tree == target_tree {
        return Ok(Some(How::SameFiles));
    }
    Ok(match merge_tree(cwd, &target_sha, branch)? {
        Some(merged) if merged == target_tree => Some(How::MergeAddsNothing),
        Some(_) => None,
        // Conflicts: later commits touched the same lines.
        None => same_change(cwd, branch, &target_sha, &base)?.then_some(How::SameChange),
    })
}

fn commit(cwd: &Path, name: &str) -> Result<String, String> {
    let name = format!("{name}^{{commit}}");
    Ok(
        git(cwd, &["rev-parse", "--verify", "--end-of-options", &name])?
            .trim()
            .to_owned(),
    )
}
fn tree(cwd: &Path, commit: &str) -> Result<String, String> {
    let name = format!("{commit}^{{tree}}");
    Ok(git(cwd, &["rev-parse", "--verify", &name])?
        .trim()
        .to_owned())
}
fn is_ancestor(cwd: &Path, ancestor: &str, of: &str) -> Result<bool, String> {
    let args = ["merge-base", "--is-ancestor", ancestor, of];
    match git_answer(cwd, &args, &[], None)?.0 {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err("Git could not compare these commits.".into()),
    }
}
fn merge_base(cwd: &Path, a: &str, b: &str) -> Result<Option<String>, String> {
    let (code, out) = git_answer(cwd, &["merge-base", a, b], &[], None)?;
    match code {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out).trim().to_owned())),
        Some(1) => Ok(None),
        _ => Err("Git could not compare these commits.".into()),
    }
}

/// The tree a merge would make, or `None` when it conflicts. The merge writes
/// objects, so they go to a folder of their own and never reach the repository.
fn merge_tree(cwd: &Path, target: &str, branch: &str) -> Result<Option<String>, String> {
    let common = git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let objects = Path::new(common.trim()).join("objects");
    let scratch = tempfile::tempdir().map_err(|error| error.to_string())?;
    let envs = [
        ("GIT_OBJECT_DIRECTORY", scratch.path()),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects.as_path()),
    ];
    let args = ["merge-tree", "--write-tree", target, branch];
    let (code, out) = git_answer(cwd, &args, &envs, None)?;
    match code {
        Some(0) => Ok(String::from_utf8_lossy(&out)
            .lines()
            .next()
            .map(|tree| tree.trim().to_owned())),
        Some(1) => Ok(None),
        // Older than Git 2.38, for one.
        _ => Err("This version of Git could not simulate the merge.".into()),
    }
}

/// Whether one commit on the target has the branch's whole diff. This is what a
/// squash merge leaves behind once later commits changed the same lines.
fn same_change(cwd: &Path, branch: &str, target: &str, base: &str) -> Result<bool, String> {
    let range = format!("{base}..{target}");
    let commits = git(cwd, &["rev-list", "--no-merges", &range])?;
    if commits.lines().count() > PATCH_ID_SCAN_MAX_COMMITS {
        return Ok(false);
    }
    // Plumbing on both sides, so diff settings can't change the ids.
    let branch_diff = bytes(
        cwd,
        &["diff-tree", "-p", NO_RENAMES, "--no-ext-diff", base, branch],
        None,
    )?;
    let target_diffs = bytes(
        cwd,
        &["diff-tree", "--stdin", "-p", NO_RENAMES, "--no-ext-diff"],
        Some(commits.as_bytes()),
    )?;
    let wanted = patch_ids(cwd, &branch_diff)?;
    let found = patch_ids(cwd, &target_diffs)?;
    Ok(!wanted.is_empty() && wanted.is_subset(&found))
}
fn bytes(cwd: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    match git_answer(cwd, args, &[], input)? {
        (Some(0), out) => Ok(out),
        _ => Err("Git could not read these commits.".into()),
    }
}
fn patch_ids(cwd: &Path, diff: &[u8]) -> Result<HashSet<String>, String> {
    let out = bytes(cwd, &["patch-id", "--stable"], Some(diff))?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(dir: &Path, args: &[&str]) -> String {
        git(dir, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
    }
    fn commit_file(dir: &Path, file: &str, text: &str) {
        fs::write(dir.join(file), text).unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-qm", file]);
    }
    fn lines(third: &str) -> String {
        format!("one\ntwo\n{third}\nfour\nfive\n")
    }
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        run(path, &["init", "-qb", "main"]);
        run(path, &["config", "user.name", "Test"]);
        run(path, &["config", "user.email", "test@example.invalid"]);
        run(path, &["config", "commit.gpgsign", "false"]);
        run(path, &["config", "core.autocrlf", "false"]);
        commit_file(path, "shared.txt", &lines("three"));
        dir
    }
    fn branch(path: &Path, name: &str, files: &[(&str, &str)]) {
        run(path, &["checkout", "-qb", name, "main"]);
        for (file, text) in files {
            commit_file(path, file, text);
        }
        run(path, &["checkout", "-q", "main"]);
    }
    fn squash(path: &Path, name: &str) {
        run(path, &["merge", "-q", "--squash", name]);
        run(path, &["commit", "-qm", "Squash"]);
    }
    fn how(path: &Path, name: &str) -> Option<How> {
        merged(path, name).unwrap().map(|merged| merged.how)
    }

    #[test]
    fn same_commit_and_ancestor() {
        let dir = repo();
        let path = dir.path();
        run(path, &["branch", "old"]);
        commit_file(path, "next.txt", "1\n");
        run(path, &["branch", "same"]);
        assert_eq!(how(path, "same"), Some(How::SameCommit));
        assert_eq!(how(path, "old"), Some(How::Ancestor));
    }
    #[test]
    fn a_rebased_branch_counts_as_merged() {
        let dir = repo();
        let path = dir.path();
        branch(path, "rebased", &[("r.txt", "1\n"), ("r.txt", "1\n2\n")]);
        commit_file(path, "main.txt", "moved\n");
        run(path, &["cherry-pick", "rebased~1", "rebased"]);
        assert_eq!(how(path, "rebased"), Some(How::MergeAddsNothing));
    }
    #[test]
    fn a_squash_merged_branch_counts_as_merged() {
        let dir = repo();
        let path = dir.path();
        branch(path, "squashed", &[("s.txt", "1\n"), ("s.txt", "1\n2\n")]);
        commit_file(path, "main.txt", "moved\n");
        squash(path, "squashed");
        commit_file(path, "after.txt", "later\n");
        assert_eq!(how(path, "squashed"), Some(How::MergeAddsNothing));
    }
    #[test]
    fn a_squash_with_later_changes_to_the_same_lines_counts_as_merged() {
        let dir = repo();
        let path = dir.path();
        let (draft, last) = (lines("draft"), lines("last"));
        branch(
            path,
            "later",
            &[
                ("shared.txt", draft.as_str()),
                ("shared.txt", last.as_str()),
            ],
        );
        squash(path, "later");
        commit_file(path, "shared.txt", &lines("rewritten"));
        assert_eq!(how(path, "later"), Some(How::SameChange));
    }
    #[test]
    fn identical_files_on_the_target_count_as_merged() {
        let dir = repo();
        let path = dir.path();
        branch(path, "copied", &[("c.txt", "1\n")]);
        fs::write(path.join("c.txt"), "1\n").unwrap();
        run(path, &["add", "."]);
        run(path, &["commit", "-qm", "The same file, from elsewhere"]);
        assert_eq!(how(path, "copied"), Some(How::SameFiles));
    }
    #[test]
    fn a_branch_that_undid_itself_has_no_changes() {
        let dir = repo();
        let path = dir.path();
        branch(path, "undone", &[("u.txt", "1\n")]);
        run(path, &["checkout", "-q", "undone"]);
        run(path, &["revert", "--no-edit", "HEAD"]);
        run(path, &["checkout", "-q", "main"]);
        commit_file(path, "main.txt", "moved\n");
        assert_eq!(how(path, "undone"), Some(How::NoChanges));
    }
    #[test]
    fn unfinished_work_is_not_merged() {
        let dir = repo();
        let path = dir.path();
        branch(path, "open", &[("n.txt", "1\n")]);
        branch(path, "half", &[("h1.txt", "1\n"), ("h2.txt", "1\n")]);
        commit_file(path, "main.txt", "moved\n");
        run(path, &["cherry-pick", "half~1"]);
        assert_eq!(how(path, "open"), None);
        assert_eq!(how(path, "half"), None, "only one of two commits landed");
    }
    #[test]
    fn a_squash_reverted_cleanly_is_not_merged() {
        let dir = repo();
        let path = dir.path();
        branch(path, "reverted", &[("c.txt", "1\n")]);
        squash(path, "reverted");
        run(path, &["revert", "--no-edit", "HEAD"]);
        assert_eq!(how(path, "reverted"), None);
    }
    #[test]
    fn a_conflicting_squash_that_was_reverted_reads_as_the_weakest_evidence() {
        let dir = repo();
        let path = dir.path();
        let kept = lines("kept");
        branch(path, "gone", &[("shared.txt", kept.as_str())]);
        squash(path, "gone");
        run(path, &["revert", "--no-edit", "HEAD"]);
        commit_file(path, "shared.txt", &lines("other"));
        assert_eq!(how(path, "gone"), Some(How::SameChange));
    }
    #[test]
    fn a_branch_with_no_shared_history_is_not_merged() {
        let dir = repo();
        let path = dir.path();
        run(path, &["checkout", "-q", "--orphan", "island"]);
        commit_file(path, "island.txt", "1\n");
        run(path, &["checkout", "-qf", "main"]);
        assert_eq!(how(path, "island"), None);
    }
    #[test]
    fn the_default_branch_is_never_merged_into_itself() {
        let dir = repo();
        assert_eq!(how(dir.path(), "main"), None);
    }
    #[test]
    fn a_squash_only_on_the_upstream_counts_and_names_it() {
        let dir = repo();
        let path = dir.path();
        let origin = tempfile::tempdir().unwrap();
        run(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
        let url = origin.path().to_string_lossy().into_owned();
        run(path, &["remote", "add", "origin", &url]);
        run(path, &["push", "-qu", "origin", "main"]);
        branch(path, "feature", &[("f.txt", "1\n"), ("f.txt", "1\n2\n")]);
        let behind = run(path, &["rev-parse", "main"]);
        squash(path, "feature");
        run(path, &["push", "-q", "origin", "main"]);
        run(path, &["reset", "-q", "--hard", behind.trim()]);
        let found = merged(path, "feature").unwrap().unwrap();
        assert_eq!(found.target, "origin/main");
        assert_eq!(found.how, How::SameFiles);
        assert_eq!(found.reason(), "Its files match origin/main");
    }
    #[test]
    fn origin_head_chooses_the_target() {
        let dir = repo();
        let path = dir.path();
        run(path, &["branch", "-m", "main", "trunk"]);
        assert_eq!(default_target(path), None);
        run(
            path,
            &["remote", "add", "origin", "https://example.invalid/r.git"],
        );
        run(path, &["update-ref", "refs/remotes/origin/trunk", "trunk"]);
        run(
            path,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
        );
        assert_eq!(default_target(path).as_deref(), Some("trunk"));
    }
    #[test]
    fn checks_leave_nothing_in_the_repository() {
        let dir = repo();
        let path = dir.path();
        branch(path, "open", &[("o.txt", "1\n")]);
        commit_file(path, "main.txt", "moved\n");
        assert_eq!(how(path, "open"), None);
        let merged = merge_tree(path, "main", "open").unwrap().unwrap();
        let args = ["cat-file", "-e", merged.as_str()];
        let (code, _) = git_answer(path, &args, &[], None).unwrap();
        assert_ne!(code, Some(0), "the simulated merge reached the repository");
        assert!(!path.join(".git/wt").exists());
    }
    #[test]
    fn a_moved_branch_is_read_again() {
        let dir = repo();
        let path = dir.path();
        run(path, &["branch", "moving"]);
        let first = merged(path, "moving").unwrap().unwrap();
        run(path, &["checkout", "-q", "moving"]);
        commit_file(path, "new.txt", "1\n");
        run(path, &["checkout", "-q", "main"]);
        assert_eq!(how(path, "moving"), None);
        assert_ne!(first.branch_sha, run(path, &["rev-parse", "moving"]).trim());
    }
    #[test]
    fn patch_id_scan_stops_at_the_cap() {
        let dir = repo();
        let path = dir.path();
        let kept = lines("kept");
        branch(path, "big", &[("shared.txt", kept.as_str())]);
        squash(path, "big");
        commit_file(path, "shared.txt", &lines("other"));
        let base = run(path, &["merge-base", "main", "big"]);
        let target = run(path, &["rev-parse", "main"]);
        assert!(same_change(path, "big", target.trim(), base.trim()).unwrap());
        let tree = run(path, &["rev-parse", "main^{tree}"]);
        let mut head = target.trim().to_owned();
        for index in 0..=PATCH_ID_SCAN_MAX_COMMITS {
            let args = ["commit-tree", tree.trim(), "-p", &head, "-m"];
            let message = index.to_string();
            let next = run(path, &[&args[..], &[&message]].concat());
            head = next.trim().to_owned();
        }
        assert!(!same_change(path, "big", &head, base.trim()).unwrap());
    }
    #[test]
    fn a_remote_project_gets_an_error_and_no_answer() {
        assert!(merged_into(Path::new("remote://host/repo"), "feature", "main").is_err());
    }
}
