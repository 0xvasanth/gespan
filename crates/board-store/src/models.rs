//! The board tables as Toasty models, every column version 8 of the
//! hand-made schema had, under the same table and column names.
//!
//! Toasty's DDL has no foreign keys, `ON DELETE CASCADE`, `CHECK`,
//! `DEFAULT`, partial or expression indexes. The cascades and checks the
//! hand-made tables had are triggers in [`crate::constraints`], with the
//! partial and expression unique indexes. Every value a `DEFAULT` used to
//! fill is set by the code that inserts the row.
//!
//! The unique card number per board is in [`crate::constraints`] too, not a
//! model `#[unique]`. Adoption keeps a model's unique indexes as well (it
//! creates every index of the first migration after the tables are rebuilt),
//! so it could move to the model; it stays where the other board indexes
//! that Toasty cannot write are.
//!
//! The structs are named `*Row` so they don't clash with the public `Board`,
//! `Column` and `Card` types the API returns.
use toasty::schema::Model;
use toasty::ModelSet;

#[derive(Debug, toasty::Model)]
#[table = "boards"]
pub struct BoardRow {
    #[key]
    pub id: String,
    pub name: String,
    pub card_seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_columns"]
#[index(name = "board_columns_board_idx", board_id, position)]
pub struct ColumnRow {
    #[key]
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub position: i64,
    pub role: Option<String>,
    pub color: Option<String>,
    pub wip_limit: Option<i64>,
    pub collapsed: bool,
}

#[derive(Debug, toasty::Model)]
#[table = "board_cards"]
#[index(name = "board_cards_column_idx", column_id, position)]
#[index(name = "board_cards_link_idx", link_kind, link_id)]
pub struct CardRow {
    #[key]
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
    pub title_from_ticket: bool,
    pub body_from_ticket: bool,
}

// Two indexes end in `created_at`, which this lint reads as a repeat.
#[allow(clippy::duplicated_attributes)]
#[derive(Debug, toasty::Model)]
#[table = "board_activity"]
#[index(name = "board_activity_board_idx", board_id, created_at)]
#[index(name = "board_activity_card_idx", card_id, created_at)]
pub struct ActivityRow {
    #[key]
    pub id: String,
    pub board_id: String,
    pub card_id: Option<String>,
    pub kind: String,
    pub actor: String,
    // `from_column` would read as a `from_*` constructor on the generated
    // builders, so the field has another name.
    #[column("from_column")]
    pub moved_from: Option<String>,
    pub to_column: Option<String>,
    pub detail: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_card_meta"]
pub struct CardMetaRow {
    #[key]
    pub card_id: String,
    pub priority: i64,
    pub due_at: Option<i64>,
    pub start_at: Option<i64>,
    pub estimate_minutes: Option<i64>,
    pub archived_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[table = "board_labels"]
#[index(name = "board_labels_board_idx", board_id, position)]
pub struct LabelRow {
    #[key]
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub color: String,
    pub position: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_card_labels"]
#[key(card_id, label_id)]
#[index(name = "board_card_labels_label_idx", label_id)]
pub struct CardLabelRow {
    pub card_id: String,
    pub label_id: String,
}

#[derive(Debug, toasty::Model)]
#[table = "board_subtasks"]
#[index(name = "board_subtasks_card_idx", card_id, position)]
pub struct SubtaskRow {
    #[key]
    pub id: String,
    pub card_id: String,
    pub title: String,
    pub done: bool,
    pub position: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_comments"]
#[index(name = "board_comments_card_idx", card_id, created_at)]
pub struct CommentRow {
    #[key]
    pub id: String,
    pub card_id: String,
    pub body: String,
    pub author: String,
    pub created_at: i64,
    pub edited_at: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[table = "board_card_projects"]
#[key(card_id, project)]
pub struct CardProjectRow {
    pub card_id: String,
    pub project: String,
}

#[derive(Debug, toasty::Model)]
#[table = "board_card_relations"]
#[key(card_id, related_card_id, kind)]
#[index(name = "board_card_relations_related_idx", related_card_id)]
pub struct RelationRow {
    pub card_id: String,
    pub related_card_id: String,
    pub kind: String,
}

#[derive(Debug, toasty::Model)]
#[table = "board_fields"]
#[index(name = "board_fields_board_idx", board_id, position)]
pub struct FieldRow {
    #[key]
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub kind: String,
    pub options_json: Option<String>,
    pub position: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_card_field_values"]
#[key(card_id, field_id)]
#[index(name = "board_card_field_values_field_idx", field_id)]
pub struct FieldValueRow {
    pub card_id: String,
    pub field_id: String,
    pub value_json: String,
}

#[derive(Debug, toasty::Model)]
#[table = "board_card_links"]
#[key(card_id, kind, reference)]
#[index(name = "board_card_links_ref_idx", kind, reference)]
pub struct LinkRow {
    pub card_id: String,
    pub kind: String,
    #[column("ref")]
    pub reference: String,
    pub role: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_settings"]
pub struct SettingsRow {
    #[key]
    pub board_id: String,
    pub run_with: bool,
    pub write_back: bool,
    pub ticket_status_changes: bool,
    pub auto_move_json: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub default_view_id: Option<String>,
    pub display_json: Option<String>,
    pub priority_json: Option<String>,
    pub run_in_worktree: Option<bool>,
    pub run_worktree_choice: Option<bool>,
}

#[derive(Debug, toasty::Model)]
#[table = "board_views"]
#[index(name = "board_views_board_idx", board_id, position)]
pub struct ViewRow {
    #[key]
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub position: i64,
    pub spec_json: String,
    pub layout_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_templates"]
pub struct TemplateRow {
    #[key]
    pub id: String,
    pub name: String,
    pub spec_json: String,
    pub created_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "board_session_settings"]
pub struct SessionSettingRow {
    #[key]
    pub session_key: String,
    pub may_change_status: Option<bool>,
}

/// Adds every board model to `set`. gespan-core calls this from
/// `Kind::Gespan`'s models.
pub fn register_models(set: &mut ModelSet) {
    BoardRow::register(set);
    ColumnRow::register(set);
    CardRow::register(set);
    ActivityRow::register(set);
    CardMetaRow::register(set);
    LabelRow::register(set);
    CardLabelRow::register(set);
    SubtaskRow::register(set);
    CommentRow::register(set);
    CardProjectRow::register(set);
    RelationRow::register(set);
    FieldRow::register(set);
    FieldValueRow::register(set);
    LinkRow::register(set);
    SettingsRow::register(set);
    ViewRow::register(set);
    TemplateRow::register(set);
    SessionSettingRow::register(set);
}

/// Every board model, for a database that holds only the board.
pub fn models() -> ModelSet {
    let mut set = ModelSet::new();
    register_models(&mut set);
    set
}
