//! Mobile access: a paired phone browses and drives the desktop workspace over
//! the QUIC link in `gespan-mobile-link`. The host here is app-agnostic; the
//! app answers [`host::Event`]s from its own state.
pub mod catalog;
pub mod host;
mod router;
pub mod watch;

pub use gespan_mobile_link::Address;
pub use host::{Device, Event, Invite, MobileHost, Push, Request, Status, WatchStart};
