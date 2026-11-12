//! The decision log: preferences and project decisions shared by every agent.

pub mod context;
pub mod embed;
pub mod entry;
pub mod files;
pub mod harness;
pub mod import;
pub mod render;
pub mod review;
pub mod scope;
pub mod service;
pub mod store;
pub(crate) mod toolkit;
pub(crate) mod vector;

pub use context::{reads_decision_file, related_context, start_context};
pub use embed::{Embedder, HashEmbedder};
pub use entry::{
    Config, Edit, Entry, Error, Flag, Kind, NewEntry, Scope, ShownHash, Source, Status,
    MAX_FIELD_CHARS,
};
pub use render::{render_block, Block, STANDING_INSTRUCTION, START_BUDGET, TURN_BUDGET};
pub use review::Review;
pub use scope::{normalize_remote, scope_for, shared_file};
pub use service::{enabled, init, set_enabled};
pub use store::DecisionStore;

#[cfg(test)]
mod tests;
