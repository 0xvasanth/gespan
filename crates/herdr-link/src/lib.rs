//! A client for herdr's public JSON-lines API. It names no host application,
//! so it can be used by anything that wants to talk to a herdr server.

mod client;
mod ssh;
mod transport;
mod types;

pub use client::*;
pub use ssh::*;
pub use transport::*;
pub use types::*;
