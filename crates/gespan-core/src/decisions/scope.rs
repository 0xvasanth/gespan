//! Which project a workspace belongs to, and where its shared decision file lives.
//!
//! Worktrees and clones of one repository share decisions because the scope
//! comes from the remote, not from the checkout's path.
use gespan_memory::Scope;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Reduces a remote URL to `host/owner/repo`: lowercase, with no scheme, user,
/// port, `.git` suffix or trailing slash. Local paths keep their path, lowercased
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

/// The host of `user@host:port`, without the user or the port.
fn host_of(authority: &str) -> &str {
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    host.split_once(':').map_or(host, |(h, _)| h)
}

/// The decision scope of a workspace: its remote (`origin`, else the first
/// remote) as `Repo`, otherwise `Project` with the canonical checkout path.
pub fn scope_for(workspace: &Path) -> Scope {
    match crate::git::origin_url(workspace) {
        Some(url) => Scope::Repo(normalize_remote(&url)),
        None => Scope::Project(project_path(workspace)),
    }
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

fn git_common_dir(workspace: &Path) -> Option<PathBuf> {
    let out = crate::git::git(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let out = out.trim();
    (!out.is_empty()).then(|| PathBuf::from(out))
}

/// The checkout's top level inside a repository, otherwise the canonical path.
fn project_path(workspace: &Path) -> String {
    let top = crate::git::git(workspace, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    let path = match top {
        Some(top) => PathBuf::from(top),
        None => workspace.to_path_buf(),
    };
    std::fs::canonicalize(&path)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}
