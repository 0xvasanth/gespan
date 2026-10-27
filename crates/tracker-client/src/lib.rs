//! Ticket logic shared by the native app (through `gespan-core`) and the
//! Tauri app: the Jira, Linear and Plane clients, the description
//! converters, ticket key and URL parsing, the `Tickets` service and the
//! agent ticket tools (`tools`).
//! Credentials are read through `gespan-tracker-secrets` only.
pub mod credentials;
pub mod provider_boards;
pub mod tickets;
pub mod tools;
