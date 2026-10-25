//! Board storage: the shared `board*` tables both apps keep in `gespan.db`, and
//! every read and write on them. The Tauri commands and the native app's
//! `gespan_core::board::Service` both call these functions, so the two apps run
//! the same SQL, validation and error strings.
use serde::Serialize;

mod comments;
mod fields;
mod labels;
mod links;
mod meta;
pub mod migrate;
mod ops;
mod relations;
mod schema;
mod settings;
mod subtasks;

pub use comments::{add_comment, edit_comment, list_comments, Comment, COMMENT_AUTHORS};
pub use fields::{
    create_field, field_values, list_fields, set_field_value, Field, FieldValue, FIELD_KINDS,
};
pub use labels::{create_label, list_labels, set_card_labels, Label};
pub use links::{add_link, board_tickets, card_links, remove_link, set_ticket, ticket, CardLink};
pub use meta::{
    archive_card, archive_cards, archived_cards, card_extras, card_meta, column_card_count,
    restore_card, set_due, set_estimate, set_priority, set_projects, set_start, unarchive_card,
    CardExtras, CardMeta, PRIORITIES,
};
pub use ops::{
    cards_for_link, create_board, create_card, create_column, delete_board, delete_card,
    delete_column, get_board, get_card, link_card, list_activity, list_boards, move_card,
    move_cards, move_column, record_card, rename_board, set_column_display, update_card,
    update_column, LINK_KINDS,
};
pub use relations::{add_relation, card_relations, remove_relation, Relation, RELATION_KINDS};
pub use schema::{ensure_tables, VERSION as SCHEMA_VERSION};
pub use settings::{board_settings, set_board_settings, BoardSettings};
pub use subtasks::{add_subtask, delete_subtask, list_subtasks, toggle_subtask, Subtask};

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub fn validate_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("Invalid {label} id"));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub position: i64,
    pub role: Option<String>,
    pub color: Option<String>,
    pub wip_limit: Option<i64>,
    pub collapsed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub board_id: String,
    pub column_id: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub position: i64,
    pub link_kind: Option<String>,
    pub link_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    pub board_id: String,
    pub card_id: Option<String>,
    pub kind: String,
    pub actor: String,
    pub from_column: Option<String>,
    pub to_column: Option<String>,
    pub detail: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDetail {
    pub board: Board,
    pub columns: Vec<Column>,
    pub cards: Vec<Card>,
}
