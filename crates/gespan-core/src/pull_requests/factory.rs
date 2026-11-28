use super::{azure_devops::AzureDevOps, github::GitHub, gitlab::GitLab, PullRequestHost};
use std::path::PathBuf;

/// The git hosts, in the order the Pull requests page asks them.
pub const GIT_HOSTS: [&str; 3] = ["github", "gitlab", "azuredevops"];

/// Hands out the [`PullRequestHost`] for a provider name.
#[derive(Clone, Debug)]
pub struct HostFactory {
    /// Where GitLab's and Azure DevOps's saved connections live.
    data_dir: PathBuf,
    /// The folder `gh` runs in when a call names no project.
    cwd: PathBuf,
}

impl HostFactory {
    pub fn new(data_dir: PathBuf, cwd: PathBuf) -> Self {
        Self { data_dir, cwd }
    }

    /// The host for `provider`, or `None` for anything that isn't a git
    /// host (Linear, Jira, Plane).
    pub fn host(&self, provider: &str) -> Option<Box<dyn PullRequestHost>> {
        Some(match provider {
            "github" => Box::new(GitHub::new(self.cwd.clone())),
            "gitlab" => Box::new(GitLab::new(self.data_dir.clone())),
            "azuredevops" => Box::new(AzureDevOps::new(self.data_dir.clone())),
            _ => return None,
        })
    }

    /// The host for `provider`, or a sentence saying it isn't one.
    pub fn require(&self, provider: &str) -> Result<Box<dyn PullRequestHost>, String> {
        self.host(provider)
            .ok_or_else(|| format!("{provider} is not a git host"))
    }
}
