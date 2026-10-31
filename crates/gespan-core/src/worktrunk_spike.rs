//! Spike for #295: keeps the calls a real wrapper would make reachable from the
//! app, so the size measurement counts them.
use std::path::Path;

use worktrunk::copy::copy_dir_recursive;
use worktrunk::git::Repository;
use worktrunk::progress::Progress;

pub fn merged(repo: &Path, branch: &str, target: &str) -> Result<Option<String>, String> {
    let repo = Repository::at(repo).map_err(|error| error.to_string())?;
    let snapshot = repo.capture_refs().map_err(|error| error.to_string())?;
    let (target, reason) = repo
        .integration_reason(&snapshot, branch, target)
        .map_err(|error| error.to_string())?;
    Ok(reason.map(|reason| format!("{} {target}", reason.description())))
}

pub fn copy(src: &Path, dest: &Path, root: &Path) -> Result<(usize, usize), String> {
    let progress = Progress::disabled();
    copy_dir_recursive(src, dest, Some(root), false, &progress)
        .map_err(|error| error.to_string())?;
    Ok(progress.copy_split())
}
