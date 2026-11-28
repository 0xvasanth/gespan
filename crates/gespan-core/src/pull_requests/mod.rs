//! Pull requests on any code host, in three layers:
//!
//! - **Transport**: how a request reaches the host. `gh api` or `glab api`
//!   with the account the user signed in with, or HTTPS with a personal
//!   access token ([`transport`]).
//! - **Host**: one adapter per host ([`PullRequestHost`]) that knows its
//!   REST endpoints and JSON and turns them into the shared [`model`]. It
//!   sends through whatever transport it's given, so GitHub reads the same
//!   over `gh` or a token.
//! - **Service**: what Gespan does with pull requests, the same on every
//!   host ([`PullRequestService`]): it checks requests before they go out and
//!   orders what comes back.
//!
//! [`HostFactory`] builds a service from one [`Connection`].
//!
//! Nothing in the app uses this yet: the Pull requests page still runs on
//! `crate::inbox`. This is built to replace that once it's proven.
mod azure_devops;
mod error;
mod factory;
mod github;
mod gitlab;
mod host;
pub mod model;
mod rest;
mod service;
pub mod transport;

pub use error::{HostError, Result};
pub use factory::{Connection, HostFactory};
pub use host::PullRequestHost;
pub use model::*;
pub use service::PullRequestService;

#[cfg(test)]
mod tests;
