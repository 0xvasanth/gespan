//! One local MCP endpoint shared by many agent sessions. Each session reaches
//! it over streamable HTTP on loopback, with a bearer token of its own.
//!
//! The crate depends on nothing of its host's. The host names the server,
//! registers each capability as a [`Toolkit`], and mints a token per agent, so
//! a new capability needs no listener, token scheme or process of its own.
mod endpoint;
mod grants;
mod relay;
mod session;
mod toolkit;

pub use endpoint::{Config, Hub};
pub use grants::{Grant, Refusal, Token};
pub use relay::{call_tool, relay, Connection};
pub use toolkit::{text_result, Toolkit};
