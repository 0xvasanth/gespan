//! Native diagnostics: structured JSONL logs in the platform log folder,
//! crash files from the panic hook, and the local "Open logs folder" and
//! "Copy diagnostics" actions. Nothing here touches the network.
mod paths;

pub use paths::{Channel, crash_dir, log_dir};
