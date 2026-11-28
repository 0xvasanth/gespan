//! Which project folder reviews each repository.
//!
//! A repository is reviewed in exactly one local project: the one whose git
//! remote points at it. When two open projects point at the same repository
//! (two clones), the user picks one in Settings › Git; until then the repo is
//! not reviewed, so a review never runs in the wrong checkout. A repository
//! added by hand has no folder and can't be reviewed until it has one.

use crate::inbox::{queue::repo_key, ProjectRepo};
use std::collections::BTreeMap;

/// One repository and the folders that could review it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoFolder {
    /// [`repo_key`]: `github:acme/app`.
    pub repo: String,
    /// Every open project folder whose remote points here, sorted, once each.
    pub folders: Vec<String>,
    /// The one that reviews it: the only folder, or the user's pick among
    /// several. `None` while there is none or the pick is still to make.
    pub folder: Option<String>,
}
impl RepoFolder {
    /// More than one folder and no pick yet.
    pub fn needs_choice(&self) -> bool {
        self.folders.len() > 1 && self.folder.is_none()
    }
}

/// Groups `repos` by repository. `chosen` maps a [`repo_key`] to the folder
/// the user picked; a pick that is no longer one of the repo's folders is
/// ignored.
pub fn repo_folders(repos: &[ProjectRepo], chosen: &BTreeMap<String, String>) -> Vec<RepoFolder> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for repo in repos {
        if repo.repo.trim().is_empty() {
            continue;
        }
        let folders = grouped
            .entry(repo_key(&repo.provider, &repo.repo))
            .or_default();
        let path = repo.project_path.trim();
        if path.is_empty() {
            continue;
        }
        let key = crate::files::path_key(path);
        if !folders
            .iter()
            .any(|folder| crate::files::path_key(folder) == key)
        {
            folders.push(path.to_string());
        }
    }
    grouped
        .into_iter()
        .map(|(repo, mut folders)| {
            folders.sort_by_key(|folder| crate::files::path_key(folder));
            let folder = match folders.as_slice() {
                [only] => Some(only.clone()),
                _ => chosen.get(&repo).and_then(|pick| {
                    let pick = crate::files::path_key(pick);
                    folders
                        .iter()
                        .find(|folder| crate::files::path_key(folder) == pick)
                        .cloned()
                }),
            };
            RepoFolder {
                repo,
                folders,
                folder,
            }
        })
        .collect()
}

/// How many repositories still wait for the user to pick their folder.
pub fn waiting_for_pick(rows: &[RepoFolder]) -> usize {
    rows.iter().filter(|row| row.folders.len() > 1).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(provider: &str, name: &str, path: &str) -> ProjectRepo {
        ProjectRepo {
            provider: provider.into(),
            repo: name.into(),
            project_path: path.into(),
        }
    }

    #[test]
    fn a_repo_with_one_project_reviews_there() {
        let rows = repo_folders(&[repo("github", "acme/app", "/work/app")], &BTreeMap::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].repo, "github:acme/app");
        assert_eq!(rows[0].folder.as_deref(), Some("/work/app"));
        assert!(!rows[0].needs_choice());
    }

    #[test]
    fn two_clones_wait_for_the_user_to_pick_one() {
        let repos = [
            repo("github", "langfuse/langfuse", "/work/langfuse-fork"),
            repo("github", "Langfuse/Langfuse", "/work/langfuse"),
        ];
        let rows = repo_folders(&repos, &BTreeMap::new());
        assert_eq!(rows.len(), 1, "the same repo in any case is one repo");
        assert_eq!(
            rows[0].folders,
            vec!["/work/langfuse", "/work/langfuse-fork"]
        );
        assert_eq!(rows[0].folder, None);
        assert!(rows[0].needs_choice());

        let picked = BTreeMap::from([(
            "github:langfuse/langfuse".to_string(),
            "/work/langfuse-fork".to_string(),
        )]);
        let rows = repo_folders(&repos, &picked);
        assert_eq!(rows[0].folder.as_deref(), Some("/work/langfuse-fork"));
        assert!(!rows[0].needs_choice());
    }

    #[test]
    fn a_pick_that_is_no_longer_open_asks_again() {
        let repos = [
            repo("github", "acme/app", "/work/a"),
            repo("github", "acme/app", "/work/b"),
        ];
        let stale = BTreeMap::from([("github:acme/app".to_string(), "/work/gone".to_string())]);
        assert!(repo_folders(&repos, &stale)[0].needs_choice());
    }

    #[test]
    fn a_stale_pick_never_overrides_the_only_folder() {
        let stale = BTreeMap::from([("github:acme/app".to_string(), "/work/old".to_string())]);
        let rows = repo_folders(&[repo("github", "acme/app", "/work/new")], &stale);
        assert_eq!(rows[0].folder.as_deref(), Some("/work/new"));
    }

    #[test]
    fn the_same_folder_listed_twice_is_one_folder() {
        let repos = [
            repo("github", "acme/app", "/work/app"),
            repo("github", "acme/app", "/work/app/"),
        ];
        let rows = repo_folders(&repos, &BTreeMap::new());
        assert_eq!(rows[0].folders.len(), 1, "{:?}", rows[0].folders);
        assert!(rows[0].folder.is_some());
    }

    #[test]
    fn a_repo_added_by_hand_has_no_folder_to_review_in() {
        let rows = repo_folders(&[repo("github", "acme/web", "")], &BTreeMap::new());
        assert!(rows[0].folders.is_empty());
        assert_eq!(rows[0].folder, None);
        assert!(!rows[0].needs_choice(), "nothing to choose between");
    }

    #[test]
    fn the_same_name_on_two_hosts_is_two_repos() {
        let rows = repo_folders(
            &[
                repo("github", "acme/app", "/work/gh"),
                repo("gitlab", "acme/app", "/work/gl"),
            ],
            &BTreeMap::new(),
        );
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.folder.is_some()));
    }
}
