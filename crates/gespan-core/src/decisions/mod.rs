//! The decision log: preferences and project decisions shared by every agent.

pub mod scope;
pub mod service;
pub(crate) mod toolkit;

pub use scope::{normalize_remote, scope_for, shared_file};
pub use service::{enabled, init, set_enabled};

#[cfg(test)]
mod tests;
