//! Native diagnostics: structured JSONL logs in the platform log folder,
//! crash files from the panic hook, and the local "Open logs folder" and
//! "Copy diagnostics" actions. Nothing here touches the network.
mod json;
mod paths;

pub use json::{filter, json_layer, JsonLayer};
pub use paths::{crash_dir, log_dir, Channel};
