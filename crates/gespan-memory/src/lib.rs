//! The decision log's store: user preferences and project decisions that
//! agents propose and the user approves.
//!
//! Times always come from the caller, never the clock, so every operation is
//! deterministic under test.

mod entry;
mod error;
mod schema;
mod store;

pub use entry::{
    Config, Edit, Entry, Flag, Kind, NewEntry, Scope, Source, Status, MAX_FIELD_CHARS,
};
pub use error::Error;
pub use store::Store;
