//! Board storage: the shared `board*` tables both apps keep in `gespan.db`, and
//! every read and write on them. The Tauri commands and the native app's
//! `gespan_core::board::Service` both call these functions, so the two apps run
//! the same SQL, validation and error strings.
use serde::Serialize;

mod ops;
mod schema;

pub use ops::{
    cards_for_link, create_board, create_card, create_column, delete_board, delete_card,
    delete_column, get_board, link_card, list_activity, list_boards, move_card, move_column,
    record_card, rename_board, update_card, update_column,
};
// Internals the native-only extras in gespan-core build on.
#[doc(hidden)]
pub use ops::{clean_text, db, get_card, log_activity, new_id, write_tx, NewActivity};
pub use schema::ensure_tables;

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
