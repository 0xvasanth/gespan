//! Pull requests on any git host, behind one trait.
//!
//! [`PullRequestHost`] is everything Gespan does with a git host's pull
//! requests and issues: connect, list, read a discussion, reply, resolve,
//! diff, check, review and act. Each host implements it over its own API:
//! GitHub through `gh` (on a remote project's machine when the project is
//! there), GitLab and Azure DevOps over HTTPS with the token saved in
//! Settings. [`HostFactory`] hands out the host for a provider name, so the
//! rest of the app never matches on a provider to reach one.
mod azure_devops;
mod factory;
mod github;
mod gitlab;
mod host;

pub use factory::{HostFactory, GIT_HOSTS};
#[cfg(test)]
pub(crate) use github::search_for as github_search;
pub use github::REVIEW_REQUESTED_SEARCH;
pub(crate) use github::{assigned_issues as github_assigned_issues, call as github_call};
pub use host::{MergedPullRequest, OpenPullRequest, PullRequestHost};

#[cfg(test)]
mod tests;
