//! Which project a workspace belongs to, and where its shared decision file lives.
//!
//! Worktrees and clones of one repository share decisions because the scope
//! comes from the remote, not from the checkout's path.
use gespan_memory::Scope;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Reduces a remote URL to `host/owner/repo`: lowercase, with no scheme, user,
/// `.git` suffix or trailing slash. A port stays, since another port can be
/// another server. Local paths keep their path, lowercased
/// with `/` separators.
pub fn normalize_remote(url: &str) -> String {
    let url = url.trim().to_lowercase().replace('\\', "/");
    let rest = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme == "file" {
            rest.to_owned()
        } else {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            format!("{}/{path}", host_of(authority))
        }
    } else if let Some((left, path)) = scp_like(&url) {
        format!("{}/{path}", host_of(left))
    } else {
        url
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    rest.trim_end_matches('/').to_owned()
}

/// `user@host:path` (git's scp-like syntax), but not a Windows drive like `c:/x`.
fn scp_like(url: &str) -> Option<(&str, &str)> {
    let colon = url.find(':')?;
    let (left, path) = (&url[..colon], &url[colon + 1..]);
    if left.contains('/') || left.len() < 2 {
        return None;
    }
    Some((left, path.trim_start_matches('/')))
}

/// The host of `user@host:port`, without the user.
fn host_of(authority: &str) -> &str {
    authority.rsplit_once('@').map_or(authority, |(_, h)| h)
}

/// The decision scope of a workspace. A remote alone must not merge scopes, so
/// `Repo` is the remote plus the first 12 hex digits of the root commit: a
/// clone shares it, an unrelated repository with the same URL does not.
/// Without a remote or a commit it is `Project` with the main checkout's path.
pub fn scope_for(workspace: &Path) -> Scope {
    if let (Some(url), Some(root)) = (crate::git::origin_url(workspace), root_commit(workspace)) {
        return Scope::Repo(format!("{}@{root}", normalize_remote(&url)));
    }
    Scope::Project(project_path(workspace))
}

/// The shared decision file: `<git common dir>/gespan/decisions.md`, so every
/// worktree of a repository reads one file. Outside git it lives in the data
/// dir, named by a hash of the project path.
pub fn shared_file(workspace: &Path, data_dir: &Path) -> PathBuf {
    if let Some(common) = git_common_dir(workspace) {
        return common.join("gespan").join("decisions.md");
    }
    let path = project_path(workspace);
    let digest = Sha256::digest(path.as_bytes());
    let name: String = digest[..20].iter().map(|b| format!("{b:02x}")).collect();
    data_dir.join("decisions").join(format!("{name}.md"))
}

/// The first 12 hex digits of the smallest root commit; `None` with no commits.
fn root_commit(workspace: &Path) -> Option<String> {
    let out = crate::git::git_probe(workspace, &["rev-list", "--max-parents=0", "HEAD"])?;
    let root = out.lines().map(str::trim).filter(|l| !l.is_empty()).min()?;
    Some(root.chars().take(12).collect())
}

fn git_common_dir(workspace: &Path) -> Option<PathBuf> {
    let out = crate::git::git_probe(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let out = out.trim();
    (!out.is_empty()).then(|| PathBuf::from(out))
}

/// The project's path: the main checkout of a repository (so its worktrees
/// agree), otherwise the workspace itself, canonicalized.
fn project_path(workspace: &Path) -> String {
    let path = main_checkout(workspace).unwrap_or_else(|| workspace.to_path_buf());
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    strip_verbatim(&path.to_string_lossy())
}

fn main_checkout(workspace: &Path) -> Option<PathBuf> {
    let common = git_common_dir(workspace)?;
    let bare = crate::git::git_probe(workspace, &["config", "--bool", "core.bare"])
        .is_some_and(|v| v.trim() == "true");
    if bare {
        return Some(common);
    }
    if common.file_name().is_some_and(|n| n == ".git") {
        return common.parent().map(Path::to_path_buf);
    }
    // A separate git dir: the checkout cannot be derived from it.
    let top = crate::git::git_probe(workspace, &["rev-parse", "--show-toplevel"])?;
    let top = top.trim();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

/// Windows canonicalization yields `\\?\C:\x` and `\\?\UNC\h\s`; drop the
/// verbatim prefix so one folder has one key and one hash.
pub(crate) fn strip_verbatim(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        path.to_owned()
    }
}
