//! How agents reach the browser module.
//!
//! - [`tools`]: tool names, schemas and descriptions; the descriptions come
//!   from `instructions.md`.
//! - [`mcp`]: how an action result is shaped into an MCP tool result.
//!
//! The server that exposes these tools is Gespan's MCP hub; identity is the
//! session the hub resolves, and every decision is the host's.

pub mod mcp;
pub mod tools;
