//! Picks the host adapter and the way to reach it from one [`Connection`]:
//! the only place that knows both. Everything after it sees a
//! [`PullRequestService`].
use super::{
    azure_devops::AzureDevOps,
    error::{HostError, Result},
    github::GitHub,
    gitlab::GitLab,
    host::PullRequestHost,
    model::HostKind,
    service::PullRequestService,
    transport::{
        Auth, CliTransport, CommandRunner, HttpClient, SystemRunner, TokenTransport, Transport,
        UreqClient,
    },
};
use std::sync::Arc;

const GITHUB_API: &str = "https://api.github.com";

/// How to reach a code host: the host, and whether through its CLI or a
/// personal access token.
#[derive(Clone, PartialEq, Eq)]
pub enum Connection {
    /// GitHub through `gh`, signed in with `gh auth login`. `hostname` for
    /// GitHub Enterprise.
    GitHubCli { hostname: Option<String> },
    /// GitHub over HTTPS. `api_url` for GitHub Enterprise
    /// (`https://ghe.example.com/api/v3`); api.github.com otherwise.
    GitHubToken {
        api_url: Option<String>,
        token: String,
    },
    /// GitLab through `glab`, signed in with `glab auth login`.
    GitLabCli { hostname: Option<String> },
    /// GitLab over HTTPS: `url` is the instance (`https://gitlab.com`).
    GitLabToken { url: String, token: String },
    /// Azure DevOps over HTTPS: `organization_url` is
    /// `https://dev.azure.com/<organization>`.
    AzureDevOpsToken {
        organization_url: String,
        token: String,
    },
}

impl std::fmt::Debug for Connection {
    // Never print a token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHubCli { hostname } => write!(f, "GitHubCli({hostname:?})"),
            Self::GitHubToken { api_url, .. } => write!(f, "GitHubToken({api_url:?})"),
            Self::GitLabCli { hostname } => write!(f, "GitLabCli({hostname:?})"),
            Self::GitLabToken { url, .. } => write!(f, "GitLabToken({url})"),
            Self::AzureDevOpsToken {
                organization_url, ..
            } => write!(f, "AzureDevOpsToken({organization_url})"),
        }
    }
}

impl Connection {
    pub fn host(&self) -> HostKind {
        match self {
            Self::GitHubCli { .. } | Self::GitHubToken { .. } => HostKind::GitHub,
            Self::GitLabCli { .. } | Self::GitLabToken { .. } => HostKind::GitLab,
            Self::AzureDevOpsToken { .. } => HostKind::AzureDevOps,
        }
    }
}

fn token(token: &str) -> Result<String> {
    let token = token.trim();
    if token.is_empty() {
        return Err(HostError::Invalid("No access token given".into()));
    }
    Ok(token.to_owned())
}

fn https_url(url: &str, what: &str) -> Result<String> {
    let url = url.trim().trim_end_matches('/');
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(HostError::Invalid(format!(
            "{what} must start with https://"
        )));
    }
    Ok(url.to_owned())
}

/// Builds services. The processes and HTTPS it uses are injectable, so a
/// test can build the real thing with nothing leaving the machine.
pub struct HostFactory {
    runner: Arc<dyn CommandRunner>,
    http: Arc<dyn HttpClient>,
}

impl HostFactory {
    pub fn new(runner: Arc<dyn CommandRunner>, http: Arc<dyn HttpClient>) -> Self {
        Self { runner, http }
    }
    /// The real CLIs and real HTTPS.
    pub fn system() -> Self {
        Self::new(
            Arc::new(SystemRunner::default()),
            Arc::new(UreqClient::default()),
        )
    }

    /// The way requests reach the host for `connection`.
    pub fn transport(&self, connection: &Connection) -> Result<Box<dyn Transport>> {
        Ok(match connection {
            Connection::GitHubCli { hostname } => Box::new(CliTransport::new(
                "gh",
                hostname.clone(),
                self.runner.clone(),
            )),
            Connection::GitLabCli { hostname } => Box::new(CliTransport::new(
                "glab",
                hostname.clone(),
                self.runner.clone(),
            )),
            Connection::GitHubToken { api_url, token: t } => {
                let base = match api_url {
                    Some(url) if !url.trim().is_empty() => https_url(url, "The GitHub API URL")?,
                    _ => GITHUB_API.to_owned(),
                };
                Box::new(
                    TokenTransport::new(base, Auth::Bearer(token(t)?), self.http.clone())
                        .with_header("Accept", "application/vnd.github+json")
                        .with_header("X-GitHub-Api-Version", "2022-11-28"),
                )
            }
            Connection::GitLabToken { url, token: t } => Box::new(TokenTransport::new(
                format!("{}/api/v4", https_url(url, "The GitLab URL")?),
                Auth::PrivateToken(token(t)?),
                self.http.clone(),
            )),
            Connection::AzureDevOpsToken {
                organization_url,
                token: t,
            } => Box::new(TokenTransport::new(
                https_url(organization_url, "The Azure DevOps organization URL")?,
                Auth::BasicToken(token(t)?),
                self.http.clone(),
            )),
        })
    }

    /// The host adapter for `connection`, sending through its transport.
    pub fn host(&self, connection: &Connection) -> Result<Box<dyn PullRequestHost>> {
        let transport = self.transport(connection)?;
        Ok(match connection {
            Connection::GitHubCli { .. } | Connection::GitHubToken { .. } => {
                Box::new(GitHub::new(transport))
            }
            Connection::GitLabCli { .. } | Connection::GitLabToken { .. } => {
                Box::new(GitLab::new(transport))
            }
            Connection::AzureDevOpsToken {
                organization_url, ..
            } => Box::new(AzureDevOps::new(transport, organization_url)),
        })
    }

    pub fn service(&self, connection: &Connection) -> Result<PullRequestService> {
        Ok(PullRequestService::new(self.host(connection)?))
    }
}
