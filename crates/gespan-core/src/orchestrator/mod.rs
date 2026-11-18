//! The orchestration core: how sessions learn about, start and message each
//! other: the roster of sibling sessions, the tree of sessions that started
//! one another, and the limits that keep that tree safe.
pub mod bridge;
pub mod mailbox;
pub mod policy;
pub mod repo;
pub mod roster;
pub mod start;
pub mod toolkit;
pub mod tree;

#[cfg(test)]
mod tests;
