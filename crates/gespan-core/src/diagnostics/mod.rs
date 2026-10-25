//! Native diagnostics: structured JSONL logs in the platform log folder,
//! crash files from the panic hook, and the local "Open logs folder" and
//! "Copy diagnostics" actions. Nothing here touches the network.
mod json;
mod paths;
mod rotation;

pub use json::{filter, json_layer, JsonLayer};
pub use paths::{crash_dir, log_dir, Channel};
pub use rotation::{file_name, prune, Clock, DailyFile, SystemClock, MAX_BYTES, MAX_DAYS};
