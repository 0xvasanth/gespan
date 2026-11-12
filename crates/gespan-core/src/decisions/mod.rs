//! The decision log: preferences and project decisions shared by every agent.

pub mod context;
pub mod entry;
pub mod files;
pub mod harness;
pub mod render;
pub mod scope;
pub mod service;
pub mod store;
pub(crate) mod toolkit;

pub use context::{reads_decision_file, start_context};
pub use entry::{
    Config, Edit, Entry, Error, Flag, Kind, NewEntry, Scope, Source, Status, MAX_FIELD_CHARS,
};
pub use render::{render_block, Block, STANDING_INSTRUCTION, START_BUDGET, TURN_BUDGET};
pub use scope::{normalize_remote, scope_for, shared_file};
pub use service::{enabled, init, set_enabled};
pub use store::DecisionStore;

#[cfg(test)]
mod tests;
