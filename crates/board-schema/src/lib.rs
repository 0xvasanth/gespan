//! The shared board schema in `gespan.db`, read and written by both the
//! Tauri app and Native. Every board `CREATE` statement, the v3 additions,
//! the one-time copy from Native's old `board_native_*` tables and the shared
//! vocabulary live here, so the two apps cannot drift apart on storage.
//! See `docs/board-schema.md` for the contract.
mod ddl;
mod migrate;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use ddl::BASE_DDL;
pub use migrate::{ensure, sync_primary_link};

/// Which app is opening the database. Only Native copies its old
/// `board_native_*` tables, because only Native writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Tauri,
    Native,
}

/// The board schema version this crate writes into `board_schema_migrations`.
pub const SCHEMA_VERSION: i64 = 3;

/// Column roles. A board has at most one column per role.
pub const ROLES: [&str; 5] = ["todo", "doing", "review", "done", "cancelled"];

/// `board_card_meta.priority` is an index into this list; 0 is no priority.
pub const PRIORITIES: [&str; 5] = ["None", "Low", "Medium", "High", "Urgent"];

/// `board_card_relations.kind`, read as "`card_id` <kind> `related_card_id`".
pub const RELATION_KINDS: [&str; 3] = ["blocks", "relates", "duplicates"];

/// `board_fields.kind`; each kind fixes the JSON shape of its `value_json`.
pub const FIELD_KINDS: [&str; 7] = [
    "text",
    "number",
    "select",
    "multi-select",
    "date",
    "checkbox",
    "url",
];

/// `board_card_links.role` of the link mirrored from `board_cards.link_kind/link_id`.
pub const LINK_ROLE_PRIMARY: &str = "primary";

/// `board_comments.author` for comments a person wrote in either app.
pub const COMMENT_AUTHOR_USER: &str = "user";

/// Every shared board table, in an order that inserts parents before
/// children. The `board_schema_migrations` marker is not part of it.
pub const BOARD_TABLES: [&str; 14] = [
    "boards",
    "board_columns",
    "board_cards",
    "board_card_meta",
    "board_labels",
    "board_card_labels",
    "board_subtasks",
    "board_comments",
    "board_card_projects",
    "board_card_relations",
    "board_fields",
    "board_card_field_values",
    "board_card_links",
    "board_activity",
];
