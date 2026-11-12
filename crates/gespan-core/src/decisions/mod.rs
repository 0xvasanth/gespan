//! The decision log: preferences and project decisions shared by every agent.

pub mod scope;

pub use scope::{normalize_remote, scope_for, shared_file};

#[cfg(test)]
mod tests;
