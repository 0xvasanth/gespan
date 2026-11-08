//! The Gespan knowledge graph: projects, sessions, files, tickets, cards and
//! the edges between them, in its own `graph.db`.
//!
//! - **Times are epoch milliseconds** ([`Timestamp`]). Nothing guesses units,
//!   and the store never reads the clock: every `valid_from`, `valid_to` and
//!   `seen_at` comes from the caller, so rerunning the same facts leaves the
//!   same graph.
//! - **Threading.** Every call is synchronous and blocking. Call it from a
//!   background thread, never the UI thread or an async executor.
//!   [`GraphStore`] is `Send`, not `Sync`.
//! - **No cancellation yet.** Every operation here is bounded: writes are one
//!   short transaction each, and queries are capped by [`Caps`]. A
//!   `CancelToken` arrives with the backfill (#386), the first open-ended
//!   operation.
//! - **Privacy.** Labels and keys are stored as given; callers redact them.
//!   Nothing opens or writes `graph.db` outside tests until #386/#132.
mod error;
mod key;
mod kind;
pub mod migrate;
pub mod schema;
mod store;
mod time;

pub use error::GraphError;
pub use key::{
    BackfillSource, EdgeId, NodeId, NodeKey, NodeRef, Props, Scope, SessionId, Source, TimeBasis,
};
pub use kind::{EdgeKind, NodeKind};
pub use store::{CloseOutcome, EdgeFact, EdgeKey, GraphStore, NodeFact, FILE_NAME};
pub use time::Timestamp;
