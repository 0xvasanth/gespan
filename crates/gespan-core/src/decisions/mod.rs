//! The decision log: preferences and project decisions shared by every agent.

pub mod context;
pub mod files;
pub mod harness;
pub mod scope;
pub mod service;
pub(crate) mod toolkit;

pub use context::{reads_decision_file, start_context};
pub use scope::{normalize_remote, scope_for, shared_file};
pub use service::{enabled, init, set_enabled};

#[cfg(test)]
mod tests;
