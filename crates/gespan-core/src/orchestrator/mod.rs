//! The orchestration core: how sessions learn about, start and message each
//! other. Milestone 1 holds the roster of sibling sessions.
pub mod bridge;
pub mod repo;
pub mod toolkit;

#[cfg(test)]
mod tests;
