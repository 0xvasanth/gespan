//! Flows in Rebase Native. `rebase_core::flow` owns the rules; this is the
//! host that runs each agent visit as a background session.
mod host;

pub(crate) use host::{Host, Probe, SessionDriver, Settled};
