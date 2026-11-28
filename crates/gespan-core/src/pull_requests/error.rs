//! What can go wrong talking to a code host, the same for every host and
//! every way of connecting to it.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The token or CLI login was refused (HTTP 401, or 403 without a rate
    /// limit).
    Unauthorized(String),
    /// The repository, pull request, thread or comment doesn't exist, or
    /// isn't visible to this account.
    NotFound(String),
    /// The host asked us to slow down. `retry_after_secs` when it said how
    /// long.
    RateLimited { retry_after_secs: Option<u64> },
    /// This host can't do it, such as GitLab requesting changes.
    Unsupported(&'static str),
    /// The request was refused before it was sent: an empty comment, a bad
    /// line range, a missing setting.
    Invalid(String),
    /// Any other HTTP error, with the host's own message.
    Http { status: u16, message: String },
    /// The request never got an answer: no network, no CLI, a timeout.
    Transport(String),
    /// The answer wasn't the shape this host's API documents.
    Parse(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(message) => write!(f, "Not signed in or not allowed: {message}"),
            Self::NotFound(what) => write!(f, "Not found: {what}"),
            Self::RateLimited {
                retry_after_secs: Some(secs),
            } => write!(f, "Rate limited; try again in {secs}s"),
            Self::RateLimited { .. } => write!(f, "Rate limited; try again later"),
            Self::Unsupported(what) => write!(f, "This host doesn't support {what}"),
            Self::Invalid(message) => f.write_str(message),
            Self::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            Self::Transport(message) => write!(f, "Couldn't reach the host: {message}"),
            Self::Parse(message) => write!(f, "Unexpected answer from the host: {message}"),
        }
    }
}

impl std::error::Error for HostError {}

pub type Result<T> = std::result::Result<T, HostError>;
