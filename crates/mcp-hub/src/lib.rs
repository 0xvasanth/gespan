//! One local MCP endpoint shared by many agent sessions. Each session reaches
//! it over streamable HTTP on loopback, with a bearer token of its own.
//!
//! The crate depends on nothing of its host's. The host names the server,
//! registers each capability as a [`Toolkit`], and mints a token per agent, so
//! a new capability needs no listener, token scheme or process of its own.
// Logging, roots and sampling are deprecated for protocol 2026-07-28, and every
// agent and server on an older version still uses them.
#![allow(deprecated)]

mod endpoint;
mod grants;
mod oauth;
mod pool;
mod relay;
mod remote;
mod router;
mod session;
mod toolkit;
mod upstream;

pub use endpoint::{Config, Hub};
pub use grants::{Grant, Refusal, Token};
pub use oauth::{safe_for_tokens, sign_in, TokenStore};
pub use pool::{NoObserver, Observer, Running, ServerError, ServerSpec, Sharing};
pub use relay::{call_tool, relay, relay_lines, Connection};
pub use remote::connect;
pub use toolkit::{text_result, Toolkit};
pub use upstream::{Spawned, Spawner};
