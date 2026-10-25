//! Ticket logic shared by the native app (through `gespan-core`) and the
//! Tauri app: the Jira, Linear and Plane clients, the description
//! converters, ticket key and URL parsing, and the `Tickets` service.
//! Credentials are read through `gespan-tracker-secrets` only.
pub mod credentials;
pub mod provider_boards;
pub mod tickets;
