//! The board's Tauri commands. Storage is the shared `gespan-board-store`
//! crate the native app also uses; each mutating command emits `CHANGED`.
#[cfg(test)]
use gespan_board_store::create_field;
#[cfg(test)]
use gespan_board_store::ensure_tables;
use gespan_board_store::get_card;
use gespan_board_store::{
    add_comment, add_link, add_relation, add_subtask, archive_card, archive_cards, archived_cards,
    card_extras, card_links, card_meta, card_relations, cards_for_link, column_card_count,
    column_card_counts, convert_subtask, create_board, create_card, create_column, create_label,
    delete_archived_card, delete_board, delete_card, delete_column, delete_subtask, duplicate_card,
    edit_comment, field_values, get_board, latest_run_events, link_card, list_activity,
    list_boards, list_comments, list_fields, list_labels, list_subtasks, move_card,
    move_card_to_board, move_cards, move_column, move_subtask, record_card, remove_link,
    remove_relation, rename_board, restore_card, set_card_labels, set_column_display, set_due,
    set_estimate, set_field_value, set_priority, set_projects, set_start, toggle_subtask,
    unarchive_card, update_card, update_column, update_column_display, validate_id, Activity,
    Board, BoardDetail, Card, CardExtras, CardIndexRow, CardLink, CardMeta, Column,
    ColumnDisplayPatch, Comment, Field, FieldValue, Label, Relation, Subtask, View, ViewSpec,
};
use gespan_board_store::{answer_card, CardTool};
use gespan_board_store::{
    blocked_cards, create_board_from_template, delete_field, delete_label, delete_template,
    field_distinct_values, list_templates, merge_labels, move_field, move_label, on_linked_work,
    rename_field_option, run_prompt, running_card_ids, save_board_as_template,
    session_status_setting, set_session_status_setting, update_board_settings, update_field,
    update_label, write_back, BoardSettings, LinkedEvent, SettingsPatch, TemplateInfo,
};
use gespan_board_store::{
    card_index, create_view, delete_view, list_views, local_day_chrono, move_view, now_millis,
    query_cards, set_label_many, set_priority_many, update_view,
};
use gespan_tracker_client::tickets::TicketRef;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, State};

use crate::session_store::SessionStore;

pub(crate) const CHANGED: &str = "gespan:board-changed";

fn notify(app: &AppHandle, board_id: &str) {
    let _ = app.emit(CHANGED, serde_json::json!({ "boardId": board_id }));
}

#[tauri::command(async)]
pub fn board_list(store: State<'_, SessionStore>) -> Result<Vec<Board>, String> {
    let conn = store.lock_conn()?;
    list_boards(&conn)
}

#[tauri::command(async)]
pub fn board_get(store: State<'_, SessionStore>, board_id: String) -> Result<BoardDetail, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    get_board(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    name: String,
) -> Result<Board, String> {
    let conn = store.lock_conn()?;
    let board = create_board(&conn, &name)?;
    drop(conn);
    notify(&app, &board.id);
    Ok(board)
}

#[tauri::command(async)]
pub fn board_rename(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
) -> Result<(), String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    rename_board(&conn, &board_id, &name)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<(), String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    delete_board(&conn, &board_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
    role: Option<String>,
) -> Result<Column, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    let column = create_column(&conn, &board_id, &name, role.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(column)
}

#[tauri::command(async)]
pub fn board_column_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    name: String,
    role: Option<String>,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = update_column(&conn, &column_id, &name, role.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = move_column(&conn, &column_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = delete_column(&conn, &column_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    title: String,
    body: Option<String>,
) -> Result<Card, String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let card = create_card(&conn, &column_id, &title, body.as_deref().unwrap_or(""))?;
    drop(conn);
    notify(&app, &card.board_id);
    Ok(card)
}

#[tauri::command(async)]
pub fn board_card_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    title: String,
    body: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = update_card(&conn, &card_id, &title, &body)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    column_id: String,
    index: usize,
    actor: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = move_card(&conn, &card_id, &column_id, index, &actor)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_link(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    link_kind: Option<String>,
    link_id: Option<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = link_card(&conn, &card_id, link_kind.as_deref(), link_id.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

/// The ticket a card links to. A stored value that no longer parses reads as
/// no ticket.
#[tauri::command(async)]
pub fn board_card_ticket(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Option<TicketRef>, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    Ok(gespan_board_store::ticket(&conn, &card_id)?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

/// Every ticket-linked card on the board, by card id, for the card faces.
#[tauri::command(async)]
pub fn board_card_tickets(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<HashMap<String, TicketRef>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    Ok(gespan_board_store::board_tickets(&conn, &board_id)?
        .into_iter()
        .filter_map(|(card, json)| Some((card, serde_json::from_str(&json).ok()?)))
        .collect())
}

/// Links the card to a ticket (replacing any other) or, with `None`, removes it.
#[tauri::command(async)]
pub fn board_card_set_ticket(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    ticket: Option<TicketRef>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let json = ticket
        .as_ref()
        .map(|ticket| serde_json::to_string(ticket).map_err(|e| e.to_string()))
        .transpose()?;
    let conn = store.lock_conn()?;
    let board_id = gespan_board_store::set_ticket(&conn, &card_id, json.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = delete_card(&conn, &card_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

/// Commits a held delete: deletes the card only while it is still archived.
#[tauri::command(async)]
pub fn board_card_delete_archived(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<(), String> {
    let conn = store.lock_conn()?;
    let board_id = card_delete_archived_inner(&conn, &card_id)?;
    drop(conn);
    if let Some(board_id) = board_id {
        notify(&app, &board_id);
    }
    Ok(())
}

fn card_delete_archived_inner(
    conn: &rusqlite::Connection,
    card_id: &str,
) -> Result<Option<String>, String> {
    validate_id(card_id, "card")?;
    delete_archived_card(conn, card_id)
}

#[tauri::command(async)]
pub fn board_cards_for_link(
    store: State<'_, SessionStore>,
    link_kind: String,
    link_id: String,
) -> Result<Vec<Card>, String> {
    let conn = store.lock_conn()?;
    cards_for_link(&conn, &link_kind, &link_id)
}

#[tauri::command(async)]
pub fn board_activity(
    store: State<'_, SessionStore>,
    board_id: String,
    card_id: Option<String>,
    limit: usize,
) -> Result<Vec<Activity>, String> {
    validate_id(&board_id, "board")?;
    if let Some(card_id) = &card_id {
        validate_id(card_id, "card")?;
    }
    let conn = store.lock_conn()?;
    list_activity(&conn, &board_id, card_id.as_deref(), limit)
}

#[tauri::command(async)]
pub fn board_card_record(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    kind: String,
    detail: Option<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = record_card(&conn, &card_id, &kind, detail.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

fn comment_add_inner(
    conn: &rusqlite::Connection,
    card_id: &str,
    body: &str,
) -> Result<Comment, String> {
    validate_id(card_id, "card")?;
    add_comment(conn, card_id, body, "user")
}

fn subtask_delete_inner(
    conn: &rusqlite::Connection,
    subtask_id: &str,
) -> Result<Option<String>, String> {
    validate_id(subtask_id, "subtask")?;
    delete_subtask(conn, subtask_id)
}

/// A column's live and archived card counts.
#[derive(Debug, PartialEq, Serialize)]
pub struct ColumnCount {
    pub live: usize,
    pub archived: usize,
}

#[tauri::command(async)]
pub fn board_card_archive(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = archive_card(&conn, &card_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_cards_archive(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_ids: Vec<String>,
) -> Result<(), String> {
    for id in &card_ids {
        validate_id(id, "card")?;
    }
    let conn = store.lock_conn()?;
    let board_id = archive_cards(&conn, &card_ids)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_unarchive(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = unarchive_card(&conn, &card_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_restore(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    column_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = restore_card(&conn, &card_id, &column_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_archived_cards(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<Vec<Card>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    archived_cards(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_cards_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_ids: Vec<String>,
    column_id: String,
    index: usize,
    actor: String,
) -> Result<(), String> {
    for id in &card_ids {
        validate_id(id, "card")?;
    }
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = move_cards(&conn, &card_ids, &column_id, index, &actor)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_column_set_display(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    color: Option<String>,
    wip_limit: Option<i64>,
    collapsed: bool,
) -> Result<(), String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let board_id = set_column_display(&conn, &column_id, color.as_deref(), wip_limit, collapsed)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

fn column_update_display_inner(
    conn: &rusqlite::Connection,
    column_id: &str,
    patch: &ColumnDisplayPatch,
) -> Result<Column, String> {
    validate_id(column_id, "column")?;
    update_column_display(conn, column_id, patch)
}

/// Changes only the display fields the patch names (absent keeps, `null`
/// clears), so collapsing a column here never resets a colour or WIP limit
/// the native app just set. Returns the column after the change.
#[tauri::command(async)]
pub fn board_column_update_display(
    app: AppHandle,
    store: State<'_, SessionStore>,
    column_id: String,
    patch: ColumnDisplayPatch,
) -> Result<Column, String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let column = column_update_display_inner(&conn, &column_id, &patch)?;
    drop(conn);
    notify(&app, &column.board_id);
    Ok(column)
}

#[tauri::command(async)]
pub fn board_column_card_count(
    store: State<'_, SessionStore>,
    column_id: String,
) -> Result<ColumnCount, String> {
    validate_id(&column_id, "column")?;
    let conn = store.lock_conn()?;
    let (live, archived) = column_card_count(&conn, &column_id)?;
    Ok(ColumnCount { live, archived })
}

/// Every column's `[live, archived]` card counts on the board, by column id,
/// in one read.
#[tauri::command(async)]
pub fn board_column_card_counts(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<HashMap<String, (usize, usize)>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    column_card_counts(&conn, &board_id)
}

/// Card id to the kind of its newest run event (`started`, `needs_input`,
/// `completed` or `failed`), board-wide in one read; the rule the
/// `linked: failed` filter uses.
#[tauri::command(async)]
pub fn board_latest_run_events(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<HashMap<String, String>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    latest_run_events(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_card_extras(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<HashMap<String, CardExtras>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    card_extras(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_card_set_priority(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    priority: u8,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = set_priority(&conn, &card_id, priority)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_set_due(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    due_at: Option<i64>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = set_due(&conn, &card_id, due_at)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_set_projects(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    projects: Vec<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = set_projects(&conn, &card_id, &projects)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_labels(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<Vec<Label>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    list_labels(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_label_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
    color: String,
) -> Result<Label, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    let label = create_label(&conn, &board_id, &name, &color)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(label)
}

#[tauri::command(async)]
pub fn board_card_set_labels(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    label_ids: Vec<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = set_card_labels(&conn, &card_id, &label_ids)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

/// The card ids matching `spec`, in display order. The clock and the local
/// day come from this side, so the front end never sends time-zone data.
fn query_inner(
    conn: &rusqlite::Connection,
    board_id: &str,
    spec: &ViewSpec,
    running: Vec<String>,
) -> Result<Vec<String>, String> {
    validate_id(board_id, "board")?;
    let running: HashSet<String> = running.into_iter().collect();
    query_cards(
        conn,
        board_id,
        spec,
        now_millis(),
        &local_day_chrono,
        &running,
    )
}

#[tauri::command(async)]
pub fn board_query(
    store: State<'_, SessionStore>,
    board_id: String,
    spec: ViewSpec,
    running: Vec<String>,
) -> Result<Vec<String>, String> {
    let conn = store.lock_conn()?;
    query_inner(&conn, &board_id, &spec, running)
}

#[tauri::command(async)]
pub fn board_card_index(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<Vec<CardIndexRow>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    card_index(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_views(store: State<'_, SessionStore>, board_id: String) -> Result<Vec<View>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    list_views(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_view_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
    spec_json: String,
    layout_json: String,
) -> Result<View, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    let view = create_view(&conn, &board_id, &name, &spec_json, &layout_json)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(view)
}

#[tauri::command(async)]
pub fn board_view_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    view_id: String,
    name: Option<String>,
    spec_json: Option<String>,
    layout_json: Option<String>,
) -> Result<(), String> {
    validate_id(&view_id, "view")?;
    let conn = store.lock_conn()?;
    let board_id = update_view(
        &conn,
        &view_id,
        name.as_deref(),
        spec_json.as_deref(),
        layout_json.as_deref(),
    )?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_view_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    view_id: String,
) -> Result<(), String> {
    validate_id(&view_id, "view")?;
    let conn = store.lock_conn()?;
    let board_id = delete_view(&conn, &view_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_view_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    view_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&view_id, "view")?;
    let conn = store.lock_conn()?;
    let board_id = move_view(&conn, &view_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_cards_set_priority(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_ids: Vec<String>,
    priority: u8,
) -> Result<(), String> {
    for id in &card_ids {
        validate_id(id, "card")?;
    }
    let conn = store.lock_conn()?;
    let board_id = set_priority_many(&conn, &card_ids, priority)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_cards_set_label(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_ids: Vec<String>,
    label_id: String,
    on: bool,
) -> Result<(), String> {
    for id in &card_ids {
        validate_id(id, "card")?;
    }
    validate_id(&label_id, "label")?;
    let conn = store.lock_conn()?;
    let board_id = set_label_many(&conn, &card_ids, &label_id, on)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_subtasks(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Vec<Subtask>, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    list_subtasks(&conn, &card_id)
}

#[tauri::command(async)]
pub fn board_subtask_add(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    title: String,
) -> Result<Subtask, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let subtask = add_subtask(&conn, &card_id, &title)?;
    let board_id = get_card(&conn, &card_id)?.board_id;
    drop(conn);
    notify(&app, &board_id);
    Ok(subtask)
}

#[tauri::command(async)]
pub fn board_subtask_toggle(
    app: AppHandle,
    store: State<'_, SessionStore>,
    subtask_id: String,
    done: bool,
) -> Result<(), String> {
    validate_id(&subtask_id, "subtask")?;
    let conn = store.lock_conn()?;
    let board_id = toggle_subtask(&conn, &subtask_id, done)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_subtask_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    subtask_id: String,
) -> Result<(), String> {
    let conn = store.lock_conn()?;
    let board_id = subtask_delete_inner(&conn, &subtask_id)?;
    drop(conn);
    if let Some(board_id) = board_id {
        notify(&app, &board_id);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn board_comments(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Vec<Comment>, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    list_comments(&conn, &card_id)
}

#[tauri::command(async)]
pub fn board_comment_add(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    body: String,
) -> Result<Comment, String> {
    let conn = store.lock_conn()?;
    let comment = comment_add_inner(&conn, &card_id, &body)?;
    let board_id = get_card(&conn, &card_id)?.board_id;
    drop(conn);
    notify(&app, &board_id);
    Ok(comment)
}

/// Moves a card to another board; returns the board it left and the moved card.
fn move_to_board_inner(
    conn: &rusqlite::Connection,
    card_id: &str,
    board_id: &str,
) -> Result<(String, Card), String> {
    validate_id(card_id, "card")?;
    validate_id(board_id, "board")?;
    let source = get_card(conn, card_id)?.board_id;
    let card = move_card_to_board(conn, card_id, board_id)?;
    Ok((source, card))
}

#[tauri::command(async)]
pub fn board_card_meta(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<CardMeta, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    card_meta(&conn, &card_id)
}

#[tauri::command(async)]
pub fn board_card_set_start(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    start_at: Option<i64>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = set_start(&conn, &card_id, start_at)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_set_estimate(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    minutes: Option<i64>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = set_estimate(&conn, &card_id, minutes)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_relation_add(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    related_card_id: String,
    kind: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    validate_id(&related_card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = add_relation(&conn, &card_id, &related_card_id, &kind)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_relation_remove(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    related_card_id: String,
    kind: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    validate_id(&related_card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = remove_relation(&conn, &card_id, &related_card_id, &kind)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_relations(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Vec<Relation>, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    card_relations(&conn, &card_id)
}

#[tauri::command(async)]
pub fn board_fields(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<Vec<Field>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    list_fields(&conn, &board_id)
}

#[tauri::command(async)]
pub fn board_field_value_set(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    field_id: String,
    value_json: Option<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    validate_id(&field_id, "field")?;
    let conn = store.lock_conn()?;
    let board_id = set_field_value(&conn, &card_id, &field_id, value_json.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_field_values(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Vec<FieldValue>, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    field_values(&conn, &card_id)
}

#[tauri::command(async)]
pub fn board_link_add(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    kind: String,
    reference: String,
    role: Option<String>,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = add_link(&conn, &card_id, &kind, &reference, role.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_link_remove(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    kind: String,
    reference: String,
) -> Result<(), String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let board_id = remove_link(&conn, &card_id, &kind, &reference)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_card_links(
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Vec<CardLink>, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    card_links(&conn, &card_id)
}

#[tauri::command(async)]
pub fn board_comment_edit(
    app: AppHandle,
    store: State<'_, SessionStore>,
    comment_id: String,
    body: String,
) -> Result<(), String> {
    validate_id(&comment_id, "comment")?;
    let conn = store.lock_conn()?;
    let board_id = edit_comment(&conn, &comment_id, &body)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_subtask_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    subtask_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&subtask_id, "subtask")?;
    let conn = store.lock_conn()?;
    let board_id = move_subtask(&conn, &subtask_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_subtask_convert(
    app: AppHandle,
    store: State<'_, SessionStore>,
    subtask_id: String,
) -> Result<Card, String> {
    validate_id(&subtask_id, "subtask")?;
    let conn = store.lock_conn()?;
    let card = convert_subtask(&conn, &subtask_id)?;
    drop(conn);
    notify(&app, &card.board_id);
    Ok(card)
}

#[tauri::command(async)]
pub fn board_card_duplicate(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
) -> Result<Card, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    let card = duplicate_card(&conn, &card_id)?;
    drop(conn);
    notify(&app, &card.board_id);
    Ok(card)
}

/// Notifies both the board the card left and the one it joined.
#[tauri::command(async)]
pub fn board_card_move_to_board(
    app: AppHandle,
    store: State<'_, SessionStore>,
    card_id: String,
    board_id: String,
) -> Result<Card, String> {
    let conn = store.lock_conn()?;
    let (source, card) = move_to_board_inner(&conn, &card_id, &board_id)?;
    drop(conn);
    notify(&app, &source);
    notify(&app, &card.board_id);
    Ok(card)
}

/// A settings patch from the front end, in camelCase. A key that is absent
/// leaves that setting alone; for the optional settings an explicit `null`
/// clears it back to the default. Unknown keys are refused.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatchArg {
    #[serde(default)]
    run_with: Option<bool>,
    #[serde(default)]
    write_back: Option<bool>,
    #[serde(default)]
    ticket_status_changes: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    auto_move_json: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    color: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    default_view_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    display_json: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    priority_json: Option<Option<String>>,
}

/// A key that is present, `null` included, is `Some`; `default` makes an
/// absent key `None`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

impl From<SettingsPatchArg> for SettingsPatch {
    fn from(arg: SettingsPatchArg) -> Self {
        SettingsPatch {
            run_with: arg.run_with,
            write_back: arg.write_back,
            ticket_status_changes: arg.ticket_status_changes,
            auto_move_json: arg.auto_move_json,
            icon: arg.icon,
            color: arg.color,
            default_view_id: arg.default_view_id,
            display_json: arg.display_json,
            priority_json: arg.priority_json,
        }
    }
}

fn settings_update_inner(
    conn: &rusqlite::Connection,
    board_id: &str,
    patch: SettingsPatchArg,
) -> Result<BoardSettings, String> {
    validate_id(board_id, "board")?;
    update_board_settings(conn, board_id, &patch.into())
}

#[tauri::command(async)]
pub fn board_settings(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<BoardSettings, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    gespan_board_store::board_settings(&conn, &board_id)
}

/// Changes only the settings the patch names, so a toggle the native app set
/// a moment ago survives a rename here.
#[tauri::command(async)]
pub fn board_settings_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    patch: SettingsPatchArg,
) -> Result<BoardSettings, String> {
    let conn = store.lock_conn()?;
    let settings = settings_update_inner(&conn, &board_id, patch)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(settings)
}

#[tauri::command(async)]
pub fn board_templates(store: State<'_, SessionStore>) -> Result<Vec<TemplateInfo>, String> {
    let conn = store.lock_conn()?;
    list_templates(&conn)
}

#[tauri::command(async)]
pub fn board_create_from_template(
    app: AppHandle,
    store: State<'_, SessionStore>,
    name: String,
    template_id: String,
) -> Result<Board, String> {
    validate_id(&template_id, "template")?;
    let conn = store.lock_conn()?;
    let board = create_board_from_template(&conn, &name, &template_id)?;
    drop(conn);
    notify(&app, &board.id);
    Ok(board)
}

#[tauri::command(async)]
pub fn board_save_as_template(
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
) -> Result<TemplateInfo, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    save_board_as_template(&conn, &board_id, &name)
}

#[tauri::command(async)]
pub fn board_template_delete(
    store: State<'_, SessionStore>,
    template_id: String,
) -> Result<(), String> {
    validate_id(&template_id, "template")?;
    let conn = store.lock_conn()?;
    delete_template(&conn, &template_id)
}

/// Renames and/or recolours a label; a part left `null` stays as it is.
#[tauri::command(async)]
pub fn board_label_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    label_id: String,
    name: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    validate_id(&label_id, "label")?;
    let conn = store.lock_conn()?;
    let board_id = update_label(&conn, &label_id, name.as_deref(), color.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_label_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    label_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&label_id, "label")?;
    let conn = store.lock_conn()?;
    let board_id = move_label(&conn, &label_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_label_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    label_id: String,
) -> Result<(), String> {
    validate_id(&label_id, "label")?;
    let conn = store.lock_conn()?;
    let board_id = delete_label(&conn, &label_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

/// Moves every card from `fromId` to `intoId`, then deletes `fromId`.
#[tauri::command(async)]
pub fn board_labels_merge(
    app: AppHandle,
    store: State<'_, SessionStore>,
    from_id: String,
    into_id: String,
) -> Result<(), String> {
    validate_id(&from_id, "label")?;
    validate_id(&into_id, "label")?;
    let conn = store.lock_conn()?;
    let board_id = merge_labels(&conn, &from_id, &into_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_field_create(
    app: AppHandle,
    store: State<'_, SessionStore>,
    board_id: String,
    name: String,
    kind: String,
    options_json: Option<String>,
) -> Result<Field, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    let field =
        gespan_board_store::create_field(&conn, &board_id, &name, &kind, options_json.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(field)
}

/// Renames a field and/or replaces its options; a part left `null` stays.
#[tauri::command(async)]
pub fn board_field_update(
    app: AppHandle,
    store: State<'_, SessionStore>,
    field_id: String,
    name: Option<String>,
    options_json: Option<String>,
) -> Result<(), String> {
    validate_id(&field_id, "field")?;
    let conn = store.lock_conn()?;
    let board_id = update_field(&conn, &field_id, name.as_deref(), options_json.as_deref())?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_field_rename_option(
    app: AppHandle,
    store: State<'_, SessionStore>,
    field_id: String,
    from: String,
    to: String,
) -> Result<(), String> {
    validate_id(&field_id, "field")?;
    let conn = store.lock_conn()?;
    let board_id = rename_field_option(&conn, &field_id, &from, &to)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_field_move(
    app: AppHandle,
    store: State<'_, SessionStore>,
    field_id: String,
    index: usize,
) -> Result<(), String> {
    validate_id(&field_id, "field")?;
    let conn = store.lock_conn()?;
    let board_id = move_field(&conn, &field_id, index)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_field_delete(
    app: AppHandle,
    store: State<'_, SessionStore>,
    field_id: String,
) -> Result<(), String> {
    validate_id(&field_id, "field")?;
    let conn = store.lock_conn()?;
    let board_id = delete_field(&conn, &field_id)?;
    drop(conn);
    notify(&app, &board_id);
    Ok(())
}

#[tauri::command(async)]
pub fn board_field_distinct_values(
    store: State<'_, SessionStore>,
    board_id: String,
    field_id: String,
    limit: usize,
) -> Result<Vec<String>, String> {
    validate_id(&board_id, "board")?;
    validate_id(&field_id, "field")?;
    let conn = store.lock_conn()?;
    field_distinct_values(&conn, &board_id, &field_id, limit)
}

/// The boards of every card linked to `kind`/`id`, in first-seen order. A
/// partly failed batch still wrote some of them, so they are all notified.
fn linked_boards(conn: &rusqlite::Connection, kind: &str, id: &str) -> Vec<String> {
    let mut boards: Vec<String> = Vec::new();
    for card in cards_for_link(conn, kind, id).unwrap_or_default() {
        if !boards.contains(&card.board_id) {
            boards.push(card.board_id);
        }
    }
    boards
}

/// The boards to notify, and the outcome.
fn linked_event_inner(
    conn: &rusqlite::Connection,
    kind: &str,
    id: &str,
    event: LinkedEvent,
) -> (Vec<String>, Result<(), String>) {
    match on_linked_work(conn, kind, id, event) {
        Ok(boards) => (boards, Ok(())),
        Err(error) => (linked_boards(conn, kind, id), Err(error)),
    }
}

/// Applies the boards' auto-move rules for `event` to every card linked to
/// `kind`/`id`, recording the event, and notifies each board written.
#[tauri::command(async)]
pub fn board_linked_event(
    app: AppHandle,
    store: State<'_, SessionStore>,
    kind: String,
    id: String,
    event: LinkedEvent,
) -> Result<(), String> {
    let conn = store.lock_conn()?;
    let (boards, result) = linked_event_inner(&conn, &kind, &id, event);
    drop(conn);
    for board_id in &boards {
        notify(&app, board_id);
    }
    result
}

/// The card ids written and the boards to notify, in first-seen order.
fn write_back_inner(
    conn: &rusqlite::Connection,
    session_key: &str,
    summary: &str,
) -> (Vec<String>, Vec<String>, Result<(), String>) {
    let cards = match write_back(conn, session_key, summary) {
        Ok(cards) => cards,
        Err(error) => {
            return (
                Vec::new(),
                linked_boards(conn, "session", session_key),
                Err(error),
            )
        }
    };
    let mut boards: Vec<String> = Vec::new();
    for card_id in &cards {
        if let Ok(card) = get_card(conn, card_id) {
            if !boards.contains(&card.board_id) {
                boards.push(card.board_id);
            }
        }
    }
    (cards, boards, Ok(()))
}

/// Comments a finished turn's reply on every card linked to the session, on
/// boards with write-back on. Returns the card ids written.
#[tauri::command(async)]
pub fn board_write_back(
    app: AppHandle,
    store: State<'_, SessionStore>,
    session_key: String,
    summary: String,
) -> Result<Vec<String>, String> {
    let conn = store.lock_conn()?;
    let (cards, boards, result) = write_back_inner(&conn, &session_key, &summary);
    drop(conn);
    for board_id in &boards {
        notify(&app, board_id);
    }
    result.map(|()| cards)
}

/// Blocked cards by card id, to the numbers of their open blockers.
#[tauri::command(async)]
pub fn board_blocked(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<BTreeMap<String, Vec<i64>>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    blocked_cards(&conn, &board_id)
}

/// Every link on every card of the board, for the link chips.
#[tauri::command(async)]
pub fn board_links(
    store: State<'_, SessionStore>,
    board_id: String,
) -> Result<Vec<CardLink>, String> {
    validate_id(&board_id, "board")?;
    let conn = store.lock_conn()?;
    gespan_board_store::board_links(&conn, &board_id)
}

fn running_cards_inner(
    conn: &rusqlite::Connection,
    board_id: &str,
    running: &[(String, String)],
) -> Result<Vec<String>, String> {
    validate_id(board_id, "board")?;
    let mut cards: Vec<String> = running_card_ids(conn, board_id, running)?
        .into_iter()
        .collect();
    cards.sort();
    Ok(cards)
}

/// The board's cards linked to a running `[kind, id]` pair (`session` or
/// `pilot`), sorted.
#[tauri::command(async)]
pub fn board_running_cards(
    store: State<'_, SessionStore>,
    board_id: String,
    running: Vec<(String, String)>,
) -> Result<Vec<String>, String> {
    let conn = store.lock_conn()?;
    running_cards_inner(&conn, &board_id, &running)
}

/// The prompt "Run with…" starts a session with.
#[tauri::command(async)]
pub fn board_run_prompt(store: State<'_, SessionStore>, card_id: String) -> Result<String, String> {
    validate_id(&card_id, "card")?;
    let conn = store.lock_conn()?;
    run_prompt(&conn, &card_id)
}

/// The session's own "may change ticket status" override, or null when it
/// follows its boards. A failed read is an error, so the chip can say so
/// instead of showing "Board default".
#[tauri::command(async)]
pub fn board_session_status_setting(
    store: State<'_, SessionStore>,
    session_key: String,
) -> Result<Option<bool>, String> {
    read_session_status_setting(&store, &session_key)
}

fn read_session_status_setting(
    store: &SessionStore,
    session_key: &str,
) -> Result<Option<bool>, String> {
    let conn = store.lock_conn()?;
    session_status_setting(&conn, session_key)
}

/// Sets the session's override; null follows its boards again.
#[tauri::command(async)]
pub fn board_session_status_setting_set(
    store: State<'_, SessionStore>,
    session_key: String,
    may_change: Option<bool>,
) -> Result<(), String> {
    let conn = store.lock_conn()?;
    set_session_status_setting(&conn, &session_key, may_change)
}

/// A card tool an agent called through the app CLI (`card.*`), scoped to the
/// live cards linked to the session right now. `read_only` is set by the
/// app for a plan turn. Answers the board-store result; a tick notifies its
/// board.
fn agent_card_tool_inner(
    conn: &rusqlite::Connection,
    session_id: &str,
    read_only: bool,
    tool: &CardTool,
) -> Result<serde_json::Value, String> {
    answer_card(conn, session_id, tool, read_only)
}

#[tauri::command(async)]
pub fn board_agent_card_tool(
    app: AppHandle,
    store: State<'_, SessionStore>,
    session_id: String,
    read_only: bool,
    tool: CardTool,
) -> Result<serde_json::Value, String> {
    let conn = store.lock_conn()?;
    let result = agent_card_tool_inner(&conn, &session_id, read_only, &tool)?;
    drop(conn);
    if let Some(board_id) = result.get("boardId").and_then(serde_json::Value::as_str) {
        notify(&app, board_id);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        ensure_tables(&conn).unwrap();
        conn
    }

    fn col_positions(conn: &Connection, column_id: &str) -> Vec<(String, i64)> {
        let mut statement = conn
            .prepare("SELECT id, position FROM board_cards WHERE column_id = ?1 ORDER BY position")
            .unwrap();
        let rows = statement
            .query_map([column_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    }

    fn assert_contiguous(conn: &Connection, column_id: &str) {
        let positions: Vec<i64> = col_positions(conn, column_id)
            .into_iter()
            .map(|(_, position)| position)
            .collect();
        let expected: Vec<i64> = (0..positions.len() as i64).collect();
        assert_eq!(positions, expected, "column {column_id} not contiguous");
    }

    fn titles(conn: &Connection, column_id: &str) -> Vec<String> {
        col_positions(conn, column_id)
            .into_iter()
            .map(|(id, _)| get_card(conn, &id).unwrap().title)
            .collect()
    }

    /// A board plus its default columns (todo, doing, review, done).
    fn board_with_columns(conn: &Connection) -> (Board, Vec<Column>) {
        let board = create_board(conn, "Work").unwrap();
        let columns = get_board(conn, &board.id).unwrap().columns;
        (board, columns)
    }

    fn add_cards(conn: &Connection, column_id: &str, titles: &[&str]) -> Vec<Card> {
        titles
            .iter()
            .map(|title| create_card(conn, column_id, title, "").unwrap())
            .collect()
    }

    #[test]
    fn create_board_makes_four_default_columns_with_roles() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        assert_eq!(board.name, "Work");
        let summary: Vec<(&str, i64, Option<&str>)> = columns
            .iter()
            .map(|column| {
                (
                    column.name.as_str(),
                    column.position,
                    column.role.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Backlog", 0, Some("todo")),
                ("In Progress", 1, Some("doing")),
                ("Review", 2, Some("review")),
                ("Done", 3, Some("done")),
            ]
        );
        assert_eq!(list_boards(&conn).unwrap().len(), 1);
    }

    #[test]
    fn board_serializes_without_card_seq() {
        let conn = setup();
        let board = create_board(&conn, "Work").unwrap();
        let value = serde_json::to_value(&board).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["createdAt", "id", "name", "updatedAt"]);
    }

    #[test]
    fn card_create_appends_at_end_and_logs_created() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        assert_eq!(
            cards.iter().map(|card| card.position).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "b", "c"]);

        let log = list_activity(&conn, &board.id, Some(&cards[2].id), 50).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, "created");
        assert_eq!(log[0].actor, "user");
        assert_eq!(log[0].card_id.as_deref(), Some(cards[2].id.as_str()));
        assert_eq!(log[0].to_column.as_deref(), Some("Backlog"));
    }

    #[test]
    fn card_numbers_increase_per_board_and_are_never_reused() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        assert_eq!(
            cards.iter().map(|card| card.number).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        delete_card(&conn, &cards[2].id).unwrap();
        let next = create_card(&conn, &columns[0].id, "d", "").unwrap();
        assert_eq!(next.number, 4);

        let (_, other_columns) = board_with_columns(&conn);
        let first = create_card(&conn, &other_columns[0].id, "x", "").unwrap();
        assert_eq!(first.number, 1);
    }

    #[test]
    fn move_within_column_renumbers_contiguously() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c", "d"]);
        move_card(&conn, &cards[0].id, &columns[0].id, 2, "user").unwrap();
        assert_eq!(titles(&conn, &columns[0].id), vec!["b", "c", "a", "d"]);
        assert_contiguous(&conn, &columns[0].id);
    }

    #[test]
    fn move_across_columns_renumbers_both_and_logs_from_to() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        add_cards(&conn, &columns[1].id, &["x", "y"]);
        move_card(&conn, &cards[1].id, &columns[1].id, 1, "system").unwrap();

        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "c"]);
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "b", "y"]);
        assert_contiguous(&conn, &columns[0].id);
        assert_contiguous(&conn, &columns[1].id);
        assert_eq!(
            get_card(&conn, &cards[1].id).unwrap().column_id,
            columns[1].id
        );

        let log = list_activity(&conn, &board.id, Some(&cards[1].id), 50).unwrap();
        assert_eq!(log[0].kind, "moved");
        assert_eq!(log[0].actor, "system");
        assert_eq!(log[0].from_column.as_deref(), Some("Backlog"));
        assert_eq!(log[0].to_column.as_deref(), Some("In Progress"));
    }

    #[test]
    fn move_index_past_end_clamps_to_end() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        add_cards(&conn, &columns[1].id, &["x"]);
        move_card(&conn, &cards[0].id, &columns[1].id, 99, "user").unwrap();
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "a"]);
        move_card(&conn, &cards[1].id, &columns[1].id, 1_000, "user").unwrap();
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "a", "b"]);
        assert_contiguous(&conn, &columns[1].id);
        assert_contiguous(&conn, &columns[0].id);
    }

    #[test]
    fn move_to_same_position_writes_no_activity() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        let before = list_activity(&conn, &board.id, None, 500).unwrap().len();
        move_card(&conn, &cards[1].id, &columns[0].id, 1, "user").unwrap();
        // Dropping onto the end of its own column when it is already last.
        move_card(&conn, &cards[2].id, &columns[0].id, 50, "user").unwrap();
        assert_eq!(
            list_activity(&conn, &board.id, None, 500).unwrap().len(),
            before
        );
        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "b", "c"]);
    }

    #[test]
    fn move_to_column_of_other_board_rejected() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let (_, other_columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a"]);
        let error = move_card(&conn, &cards[0].id, &other_columns[0].id, 0, "user").unwrap_err();
        assert_eq!(error, "Column not on this board");
        assert_eq!(titles(&conn, &columns[0].id), vec!["a"]);
        assert!(list_activity(&conn, &board.id, Some(&cards[0].id), 50)
            .unwrap()
            .iter()
            .all(|row| row.kind != "moved"));
    }

    #[test]
    fn move_rejects_unknown_actor() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a"]);
        assert!(move_card(&conn, &cards[0].id, &columns[1].id, 0, "robot").is_err());
    }

    #[test]
    fn interleaved_moves_keep_positions_contiguous() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let mut cards = add_cards(&conn, &columns[0].id, &["a", "b", "c", "d", "e", "f"]);
        cards.extend(add_cards(&conn, &columns[2].id, &["g", "h"]));
        // Deterministic xorshift so a failure is reproducible.
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        for _ in 0..20 {
            let card = &cards[next(cards.len())];
            let column = &columns[next(columns.len())];
            move_card(&conn, &card.id, &column.id, next(6), "user").unwrap();
            for column in &columns {
                assert_contiguous(&conn, &column.id);
            }
        }
        let total: usize = columns
            .iter()
            .map(|column| col_positions(&conn, &column.id).len())
            .sum();
        assert_eq!(total, cards.len());
    }

    #[test]
    fn column_move_renumbers_columns_contiguously() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        move_column(&conn, &columns[3].id, 0).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        let order: Vec<(&str, i64)> = detail
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.position))
            .collect();
        assert_eq!(
            order,
            vec![
                ("Done", 0),
                ("Backlog", 1),
                ("In Progress", 2),
                ("Review", 3)
            ]
        );
        move_column(&conn, &columns[3].id, 99).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        assert_eq!(detail.columns.last().unwrap().name, "Done");
        let positions: Vec<i64> = detail
            .columns
            .iter()
            .map(|column| column.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
    }

    #[test]
    fn column_create_appends_and_delete_closes_the_gap() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let created = create_column(&conn, &board.id, "QA", None).unwrap();
        assert_eq!((created.position, created.role.as_deref()), (4, None));
        delete_column(&conn, &columns[1].id).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        let positions: Vec<i64> = detail
            .columns
            .iter()
            .map(|column| column.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
        assert_eq!(detail.columns.last().unwrap().name, "QA");
    }

    #[test]
    fn column_delete_refused_when_not_empty() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        add_cards(&conn, &columns[0].id, &["a", "b"]);
        let error = delete_column(&conn, &columns[0].id).unwrap_err();
        assert_eq!(error, "Column has 2 cards");
    }

    #[test]
    fn duplicate_role_rejected() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let error = create_column(&conn, &board.id, "Also done", Some("done")).unwrap_err();
        assert!(error.contains("role"), "{error}");
        let error = update_column(&conn, &columns[0].id, "Backlog", Some("review")).unwrap_err();
        assert!(error.contains("role"), "{error}");
        // Re-saving a column with its own role is fine, and clearing frees it.
        update_column(&conn, &columns[0].id, "Todo", Some("todo")).unwrap();
        update_column(&conn, &columns[3].id, "Done", None).unwrap();
        create_column(&conn, &board.id, "Shipped", Some("done")).unwrap();
        assert!(create_column(&conn, &board.id, "Bad", Some("nonsense")).is_err());
    }

    #[test]
    fn board_delete_cascades_columns_cards_activity() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        move_card(&conn, &cards[0].id, &columns[1].id, 0, "user").unwrap();
        delete_card(&conn, &cards[1].id).unwrap();
        let (_, other_columns) = board_with_columns(&conn);
        add_cards(&conn, &other_columns[0].id, &["keep"]);

        delete_board(&conn, &board.id).unwrap();
        let count = |sql: &str| -> i64 {
            conn.query_row(sql, [board.id.as_str()], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(count("SELECT COUNT(*) FROM boards WHERE id = ?1"), 0);
        assert_eq!(
            count("SELECT COUNT(*) FROM board_columns WHERE board_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM board_cards WHERE board_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM board_activity WHERE board_id = ?1"),
            0
        );
        assert_eq!(list_boards(&conn).unwrap().len(), 1);
        assert_eq!(col_positions(&conn, &other_columns[0].id).len(), 1);
    }

    #[test]
    fn card_delete_logs_deleted_with_number_title_and_null_card_id() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "Fix login", "c"]);
        delete_card(&conn, &cards[1].id).unwrap();

        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "c"]);
        assert_contiguous(&conn, &columns[0].id);
        let log = list_activity(&conn, &board.id, None, 50).unwrap();
        assert_eq!(log[0].kind, "deleted");
        assert_eq!(log[0].card_id, None);
        assert_eq!(log[0].detail.as_deref(), Some("#2 Fix login"));
        // The deleted card's own history goes with it; only the tombstone stays.
        assert!(log
            .iter()
            .all(|row| row.card_id.as_deref() != Some(cards[1].id.as_str())));
    }

    #[test]
    fn cards_for_link_finds_linked_cards() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let (_, other_columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        let other = add_cards(&conn, &other_columns[0].id, &["c"]);
        link_card(&conn, &cards[0].id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &other[0].id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &cards[1].id, Some("pilot"), Some("s-1")).unwrap();

        let found = cards_for_link(&conn, "session", "s-1").unwrap();
        let mut ids: Vec<&str> = found.iter().map(|card| card.id.as_str()).collect();
        ids.sort_unstable();
        let mut expected = vec![cards[0].id.as_str(), other[0].id.as_str()];
        expected.sort_unstable();
        assert_eq!(ids, expected);
        assert!(cards_for_link(&conn, "session", "nope").unwrap().is_empty());
    }

    #[test]
    fn link_and_unlink_log_and_validate() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        assert!(link_card(&conn, &card.id, Some("session"), None).is_err());
        assert!(link_card(&conn, &card.id, Some("bogus"), Some("x")).is_err());

        link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &card.id, None, None).unwrap();
        link_card(&conn, &card.id, None, None).unwrap();
        let kinds: Vec<String> = list_activity(&conn, &board.id, Some(&card.id), 50)
            .unwrap()
            .into_iter()
            .map(|row| row.kind)
            .collect();
        assert_eq!(kinds, vec!["unlinked", "linked", "created"]);
        let card = get_card(&conn, &card.id).unwrap();
        assert_eq!((card.link_kind, card.link_id), (None, None));
    }

    #[test]
    fn update_logs_edited_only_when_changed() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        update_card(&conn, &card.id, "a", "").unwrap();
        update_card(&conn, &card.id, "  b  ", "notes").unwrap();
        let saved = get_card(&conn, &card.id).unwrap();
        assert_eq!((saved.title.as_str(), saved.body.as_str()), ("b", "notes"));
        assert!(update_card(&conn, &card.id, "   ", "x").is_err());
        let edited = list_activity(&conn, &board.id, Some(&card.id), 50)
            .unwrap()
            .iter()
            .filter(|row| row.kind == "edited")
            .count();
        assert_eq!(edited, 1);
    }

    #[test]
    fn record_appends_system_row_without_moving() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        record_card(&conn, &card.id, "failed", Some("exit 1")).unwrap();
        let log = list_activity(&conn, &board.id, Some(&card.id), 50).unwrap();
        assert_eq!(log[0].kind, "failed");
        assert_eq!(log[0].actor, "system");
        assert_eq!(log[0].detail.as_deref(), Some("exit 1"));
        assert_eq!(get_card(&conn, &card.id).unwrap().column_id, columns[0].id);
        assert!(record_card(&conn, &card.id, "deleted", None).is_err());
    }

    #[test]
    fn activity_is_newest_first_and_limited() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        record_card(&conn, &card.id, "started", None).unwrap();
        record_card(&conn, &card.id, "completed", None).unwrap();
        let log = list_activity(&conn, &board.id, None, 2).unwrap();
        let kinds: Vec<&str> = log.iter().map(|row| row.kind.as_str()).collect();
        assert_eq!(kinds, vec!["completed", "started"]);
    }

    /// A database board-store wrote, with v3 extras the Tauri UI doesn't show
    /// yet (as the native app writes them), opens in the session store and its
    /// board reads and writes keep working.
    #[test]
    fn a_ticket_ref_in_the_shared_camel_case_shape_round_trips_through_the_store() {
        use gespan_board_store as store;
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["Task"]).remove(0);
        // The shape the session link and tests/fixtures/shared/ticket-work-item.json use.
        let ticket: TicketRef = serde_json::from_str(
            r#"{"tracker":"jira","key":"ENG-142","url":"https://acme.atlassian.net/browse/ENG-142",
                "title":"T","status":"In Progress","statusGroup":"started","fetchedAt":1761350400000}"#,
        )
        .unwrap();
        let json = serde_json::to_string(&ticket).unwrap();
        store::set_ticket(&conn, &card.id, Some(&json)).unwrap();
        let stored = store::ticket(&conn, &card.id).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<TicketRef>(&stored).unwrap(), ticket);
        let all = store::board_tickets(&conn, &card.board_id).unwrap();
        assert_eq!(all, [(card.id.clone(), stored)]);
        store::set_ticket(&conn, &card.id, None).unwrap();
        assert_eq!(store::ticket(&conn, &card.id).unwrap(), None);
    }

    #[test]
    fn a_database_written_by_board_store_opens_in_the_session_store() {
        use gespan_board_store as store;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let (board, card) = {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                .unwrap();
            ensure_tables(&conn).unwrap();
            let (board, columns) = board_with_columns(&conn);
            let card = add_cards(&conn, &columns[0].id, &["Task"]).remove(0);
            store::set_priority(&conn, &card.id, 3).unwrap();
            let label = store::create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
            store::set_card_labels(&conn, &card.id, &[label.id]).unwrap();
            store::add_comment(&conn, &card.id, "Native comment", "user").unwrap();
            store::add_link(&conn, &card.id, "pr", "owner/repo#1", None).unwrap();
            (board, card)
        };
        let session_store = SessionStore::open(path).unwrap();
        let conn = session_store.lock_conn().unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        assert_eq!(detail.cards.len(), 1);
        assert_eq!(store::card_meta(&conn, &card.id).unwrap().priority, 3);
        assert_eq!(
            cards_for_link(&conn, "pr", "owner/repo#1").unwrap().len(),
            1
        );
        move_card(&conn, &card.id, &detail.columns[1].id, 0, "user").unwrap();
        delete_card(&conn, &card.id).unwrap();
        assert!(store::list_comments(&conn, &card.id).unwrap().is_empty());
    }

    #[test]
    fn the_status_setting_command_reports_a_failed_read_instead_of_board_default() {
        let dir = tempfile::tempdir().unwrap();
        let session_store = SessionStore::open(dir.path().join("gespan.db")).unwrap();
        {
            let conn = session_store.lock_conn().unwrap();
            ensure_tables(&conn).unwrap();
            set_session_status_setting(&conn, "s1", Some(false)).unwrap();
        }
        assert_eq!(
            read_session_status_setting(&session_store, "s1"),
            Ok(Some(false))
        );
        assert_eq!(read_session_status_setting(&session_store, "s2"), Ok(None));

        session_store
            .lock_conn()
            .unwrap()
            .execute_batch("ALTER TABLE board_session_settings RENAME TO moved")
            .unwrap();
        // Both a saved override and a session with none fail, never null.
        assert!(read_session_status_setting(&session_store, "s1").is_err());
        assert!(read_session_status_setting(&session_store, "s2").is_err());
    }

    #[test]
    fn extras_commands_round_trip() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);

        set_priority(&conn, &card.id, 3).unwrap();
        set_due(&conn, &card.id, Some(1_700_000_000_000)).unwrap();
        set_projects(&conn, &card.id, &["/work/app".to_string()]).unwrap();
        let label = create_label(&conn, &board.id, "Bug", "ff0000").unwrap();
        set_card_labels(&conn, &card.id, std::slice::from_ref(&label.id)).unwrap();
        let sub = add_subtask(&conn, &card.id, "one").unwrap();
        add_subtask(&conn, &card.id, "two").unwrap();
        toggle_subtask(&conn, &sub.id, true).unwrap();
        let comment = comment_add_inner(&conn, &card.id, "hello").unwrap();
        assert_eq!(comment.author, "user");

        let extras = card_extras(&conn, &board.id).unwrap();
        let value = serde_json::to_value(&extras[&card.id]).unwrap();
        assert_eq!(value["priority"], 3);
        assert_eq!(value["dueAt"], 1_700_000_000_000_i64);
        assert_eq!(value["projects"], serde_json::json!(["/work/app"]));
        assert_eq!(value["labels"], serde_json::json!([label.id]));
        assert_eq!(value["subtasksDone"], 1);
        assert_eq!(value["subtasksTotal"], 2);
        assert_eq!(value["comments"], 1);

        let label_json = serde_json::to_value(list_labels(&conn, &board.id).unwrap()).unwrap();
        assert_eq!(label_json[0]["boardId"], board.id.as_str());
        let sub_json = serde_json::to_value(list_subtasks(&conn, &card.id).unwrap()).unwrap();
        assert_eq!(sub_json[0]["cardId"], card.id.as_str());
        let comment_json = serde_json::to_value(list_comments(&conn, &card.id).unwrap()).unwrap();
        assert!(comment_json[0].get("createdAt").is_some());
        assert!(comment_json[0]["editedAt"].is_null());

        // Clearing the due date serializes as null.
        set_due(&conn, &card.id, None).unwrap();
        let cleared = card_extras(&conn, &board.id).unwrap();
        assert!(serde_json::to_value(&cleared[&card.id]).unwrap()["dueAt"].is_null());
    }

    #[test]
    fn subtask_delete_reports_the_board_only_when_something_was_deleted() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        let sub = add_subtask(&conn, &card.id, "one").unwrap();

        assert_eq!(subtask_delete_inner(&conn, "no-such-id").unwrap(), None);
        assert_eq!(
            subtask_delete_inner(&conn, &sub.id).unwrap(),
            Some(board.id.clone())
        );
        assert!(list_subtasks(&conn, &card.id).unwrap().is_empty());
        assert!(subtask_delete_inner(&conn, "bad id!").is_err());
    }

    #[test]
    fn archive_restore_and_display_commands_round_trip() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);

        archive_cards(&conn, &[cards[0].id.clone(), cards[1].id.clone()]).unwrap();
        let (live, archived) = column_card_count(&conn, &columns[0].id).unwrap();
        let json = serde_json::to_value(ColumnCount { live, archived }).unwrap();
        assert_eq!(json, serde_json::json!({ "live": 1, "archived": 2 }));

        let listed = serde_json::to_value(archived_cards(&conn, &board.id).unwrap()).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 2);
        assert!(listed[0].get("columnId").is_some());

        restore_card(&conn, &cards[1].id, &columns[1].id, 0).unwrap();
        unarchive_card(&conn, &cards[0].id).unwrap();
        assert_eq!(titles(&conn, &columns[0].id), ["c", "a"]);
        assert_eq!(titles(&conn, &columns[1].id), ["b"]);

        move_cards(
            &conn,
            &[cards[2].id.clone(), cards[0].id.clone()],
            &columns[1].id,
            0,
            "user",
        )
        .unwrap();
        assert_eq!(titles(&conn, &columns[1].id), ["c", "a", "b"]);
        assert!(move_cards(&conn, &[cards[0].id.clone()], &columns[1].id, 0, "robot").is_err());

        set_column_display(&conn, &columns[1].id, Some("00ff00"), Some(4), true).unwrap();
        let col = &get_board(&conn, &board.id).unwrap().columns[1];
        let value = serde_json::to_value(col).unwrap();
        assert_eq!(value["wipLimit"], 4);
        assert_eq!(value["collapsed"], true);
        assert_eq!(value["color"], "00ff00");
        archive_card(&conn, &cards[2].id).unwrap();
        assert_eq!(column_card_count(&conn, &columns[1].id).unwrap(), (2, 1));
    }

    #[test]
    fn delete_archived_command_deletes_only_an_archived_card() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        archive_card(&conn, &cards[0].id).unwrap();

        // Live: untouched, nothing to notify.
        assert_eq!(
            card_delete_archived_inner(&conn, &cards[1].id).unwrap(),
            None
        );
        assert_eq!(get_card(&conn, &cards[1].id).unwrap().title, "b");
        // Archived: deleted, the board is notified.
        assert_eq!(
            card_delete_archived_inner(&conn, &cards[0].id).unwrap(),
            Some(board.id.clone())
        );
        assert!(get_card(&conn, &cards[0].id).is_err());
        // Gone: a no-op.
        assert_eq!(
            card_delete_archived_inner(&conn, &cards[0].id).unwrap(),
            None
        );
        assert!(card_delete_archived_inner(&conn, "bad id!").is_err());
    }

    #[test]
    fn card_detail_types_serialize_with_the_camel_case_keys_the_ui_reads() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["A", "B"]);
        let keys = |value: serde_json::Value| -> Vec<String> {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        set_start(&conn, &cards[0].id, Some(5)).unwrap();
        assert_eq!(
            keys(serde_json::to_value(card_meta(&conn, &cards[0].id).unwrap()).unwrap()),
            [
                "archivedAt",
                "completedAt",
                "dueAt",
                "estimateMinutes",
                "priority",
                "startAt"
            ]
        );
        add_relation(&conn, &cards[0].id, &cards[1].id, "blocks").unwrap();
        let relation = &card_relations(&conn, &cards[0].id).unwrap()[0];
        assert_eq!(
            keys(serde_json::to_value(relation).unwrap()),
            ["cardId", "kind", "relatedCardId"]
        );
        add_link(&conn, &cards[0].id, "pr", "https://x/1", None).unwrap();
        assert_eq!(
            keys(serde_json::to_value(&card_links(&conn, &cards[0].id).unwrap()[0]).unwrap()),
            ["cardId", "createdAt", "kind", "reference", "role"]
        );
        let field = create_field(&conn, &board.id, "Note", "text", None).unwrap();
        assert_eq!(
            keys(serde_json::to_value(&field).unwrap()),
            ["boardId", "id", "kind", "name", "optionsJson", "position"]
        );
        set_field_value(&conn, &cards[0].id, &field.id, Some("\"x\"")).unwrap();
        assert_eq!(
            keys(serde_json::to_value(&field_values(&conn, &cards[0].id).unwrap()[0]).unwrap()),
            ["cardId", "fieldId", "valueJson"]
        );
    }

    #[test]
    fn move_to_board_reports_the_source_board_and_the_moved_card() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let target = create_board(&conn, "Other").unwrap();
        let card = add_cards(&conn, &columns[0].id, &["A"]).remove(0);
        let (source, moved) = move_to_board_inner(&conn, &card.id, &target.id).unwrap();
        assert_eq!(source, board.id);
        assert_eq!(moved.board_id, target.id);
        assert_ne!(source, moved.board_id);
        assert!(move_to_board_inner(&conn, &card.id, "bad id!").is_err());
        assert!(move_to_board_inner(&conn, "bad id!", &target.id).is_err());
    }

    #[test]
    fn a_camel_case_spec_from_the_front_end_filters_and_sorts_through_the_command_path() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["Fix login", "Write docs", "Ship"]);
        set_priority_many(&conn, &[cards[1].id.clone(), cards[2].id.clone()], 3).unwrap();
        let spec: ViewSpec = serde_json::from_str(
            r#"{"query":"","rows":[{"field":"priority","op":"is","values":["3"]}],
                "sort":{"key":"title","dir":"desc"}}"#,
        )
        .unwrap();
        let ids = query_inner(&conn, &board.id, &spec, vec![]).unwrap();
        assert_eq!(ids, vec![cards[1].id.clone(), cards[2].id.clone()]);

        let negated: ViewSpec =
            serde_json::from_str(r#"{"rows":[{"field":"priority","op":"is_not","values":["3"]}]}"#)
                .unwrap();
        assert_eq!(
            query_inner(&conn, &board.id, &negated, vec![]).unwrap(),
            vec![cards[0].id.clone()]
        );
        // The running set arrives as a plain id list.
        let linked: ViewSpec =
            serde_json::from_str(r#"{"rows":[{"field":"linked","op":"is","values":["running"]}]}"#)
                .unwrap();
        assert_eq!(
            query_inner(&conn, &board.id, &linked, vec![cards[0].id.clone()]).unwrap(),
            vec![cards[0].id.clone()]
        );
        assert!(query_inner(&conn, "bad id!", &spec, vec![]).is_err());
    }

    #[test]
    fn views_serialize_camel_case_and_bulk_edits_apply_to_every_card() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["A", "B"]);
        let view = create_view(&conn, &board.id, "Mine", r#"{"query":"a"}"#, "{}").unwrap();
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["boardId"], board.id.as_str());
        assert_eq!(json["specJson"], r#"{"query":"a"}"#);
        assert_eq!(list_views(&conn, &board.id).unwrap().len(), 1);

        let ids: Vec<String> = cards.iter().map(|c| c.id.clone()).collect();
        set_priority_many(&conn, &ids, 4).unwrap();
        let label = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
        set_label_many(&conn, &ids, &label.id, true).unwrap();
        let index = card_index(&conn, &board.id).unwrap();
        assert!(index
            .iter()
            .all(|r| r.priority == 4 && r.labels == [label.id.clone()]));
        let row = serde_json::to_value(&index[0]).unwrap();
        assert!(row.get("cardId").is_some() && row.get("dueAt").is_some());
    }

    fn column_patch(json: serde_json::Value) -> Result<ColumnDisplayPatch, String> {
        serde_json::from_value(json).map_err(|error| error.to_string())
    }

    #[test]
    fn column_display_patch_round_trips_camel_case_with_null_clearing_and_absent_keeping() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let id = &columns[1].id;
        let set = column_update_display_inner(
            &conn,
            id,
            &column_patch(serde_json::json!({
                "color": "e5534b",
                "wipLimit": 4,
                "collapsed": false,
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (set.color.as_deref(), set.wip_limit, set.collapsed),
            (Some("e5534b"), Some(4), false)
        );
        // Absent keys keep what the other app set.
        let collapsed = column_update_display_inner(
            &conn,
            id,
            &column_patch(serde_json::json!({ "collapsed": true })).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                collapsed.color.as_deref(),
                collapsed.wip_limit,
                collapsed.collapsed
            ),
            (Some("e5534b"), Some(4), true)
        );
        // `null` clears colour and limit; `collapsed: null` leaves it alone.
        let cleared = column_update_display_inner(
            &conn,
            id,
            &column_patch(serde_json::json!({ "wipLimit": null, "collapsed": null })).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                cleared.color.as_deref(),
                cleared.wip_limit,
                cleared.collapsed
            ),
            (Some("e5534b"), None, true)
        );
        let cleared = column_update_display_inner(
            &conn,
            id,
            &column_patch(serde_json::json!({ "color": null })).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                cleared.color.as_deref(),
                cleared.wip_limit,
                cleared.collapsed
            ),
            (None, None, true)
        );
        assert_eq!(
            column_patch(serde_json::json!({ "color": null })).unwrap(),
            ColumnDisplayPatch {
                color: Some(None),
                ..Default::default()
            }
        );
        assert_eq!(
            column_patch(serde_json::json!({})).unwrap(),
            ColumnDisplayPatch::default()
        );
        // The column reads back in camelCase.
        let value = serde_json::to_value(&cleared).unwrap();
        assert!(value.get("wipLimit").is_some() && value.get("boardId").is_some());
        // Unknown and snake_case keys are refused, as are bad ids and values.
        assert!(column_patch(serde_json::json!({ "wip_limit": 3 })).is_err());
        assert!(column_patch(serde_json::json!({ "name": "x" })).is_err());
        assert!(column_patch(serde_json::json!({ "wipLimit": "3" })).is_err());
        assert_eq!(
            column_update_display_inner(&conn, "bad id", &ColumnDisplayPatch::default())
                .unwrap_err(),
            "Invalid column id"
        );
        assert_eq!(
            column_update_display_inner(
                &conn,
                id,
                &column_patch(serde_json::json!({ "wipLimit": 0 })).unwrap()
            )
            .unwrap_err(),
            "A WIP limit must be at least 1"
        );
        assert_eq!(
            column_update_display_inner(
                &conn,
                id,
                &column_patch(serde_json::json!({ "color": "red" })).unwrap()
            )
            .unwrap_err(),
            "Colour must be six hex digits"
        );
    }

    fn patch(json: serde_json::Value) -> Result<SettingsPatchArg, String> {
        serde_json::from_value(json).map_err(|error| error.to_string())
    }

    #[test]
    fn settings_patch_round_trips_camel_case_with_null_clearing_and_absent_keeping() {
        let conn = setup();
        let (board, _) = board_with_columns(&conn);
        let set = settings_update_inner(
            &conn,
            &board.id,
            patch(serde_json::json!({
                "icon": "bug",
                "color": "e5534b",
                "writeBack": false,
                "autoMoveJson": "{\"failed\":\"stay\"}",
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(set.icon.as_deref(), Some("bug"));
        assert!(!set.write_back);
        // `icon: null` clears the icon; absent keys (color, writeBack, the
        // rules) keep what the other app set.
        let after = settings_update_inner(
            &conn,
            &board.id,
            patch(serde_json::json!({ "icon": null, "runWith": false })).unwrap(),
        )
        .unwrap();
        assert_eq!(after.icon, None);
        assert_eq!(after.color.as_deref(), Some("e5534b"));
        assert!(!after.write_back && !after.run_with && after.ticket_status_changes);
        assert_eq!(
            after.auto_move_json.as_deref(),
            Some("{\"failed\":\"stay\"}")
        );
        // A `null` flag leaves it alone rather than failing.
        let same = settings_update_inner(
            &conn,
            &board.id,
            patch(serde_json::json!({ "writeBack": null })).unwrap(),
        )
        .unwrap();
        assert_eq!(same, after);
        // An empty patch writes nothing.
        assert_eq!(
            settings_update_inner(&conn, &board.id, patch(serde_json::json!({})).unwrap()).unwrap(),
            after
        );
        // Unknown and snake_case keys are refused, as are bad ids and values.
        assert!(patch(serde_json::json!({ "write_back": true })).is_err());
        assert!(patch(serde_json::json!({ "boardName": "x" })).is_err());
        assert_eq!(
            settings_update_inner(&conn, "bad id", SettingsPatchArg::default()).unwrap_err(),
            "Invalid board id"
        );
        assert!(settings_update_inner(
            &conn,
            &board.id,
            patch(serde_json::json!({ "icon": "skull" })).unwrap()
        )
        .is_err());
        // The returned settings read back in camelCase.
        let value = serde_json::to_value(&after).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "autoMoveJson",
                "color",
                "defaultViewId",
                "displayJson",
                "icon",
                "priorityJson",
                "runWith",
                "ticketStatusChanges",
                "writeBack"
            ]
        );
        let template = &list_templates(&conn).unwrap()[0];
        assert_eq!(
            serde_json::to_value(template).unwrap(),
            serde_json::json!({ "id": "blank", "name": template.name, "builtIn": true })
        );
    }

    #[test]
    fn linked_events_from_the_front_end_are_snake_case_and_run_the_store_rules() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["A"]).remove(0);
        add_link(&conn, &card.id, "session", "s-1", None).unwrap();
        let event: LinkedEvent = serde_json::from_value(serde_json::json!("started")).unwrap();
        let (boards, result) = linked_event_inner(&conn, "session", "s-1", event);
        result.unwrap();
        assert_eq!(boards, vec![board.id.clone()]);
        assert_eq!(get_card(&conn, &card.id).unwrap().column_id, columns[1].id);
        for word in [
            "needs_input",
            "completed",
            "failed",
            "pr_opened",
            "pr_merged",
        ] {
            assert!(serde_json::from_value::<LinkedEvent>(serde_json::json!(word)).is_ok());
        }
        assert!(serde_json::from_value::<LinkedEvent>(serde_json::json!("prMerged")).is_err());
        // A card that fails still leaves the board notified, with the error.
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER refuse BEFORE INSERT ON board_activity \
             WHEN NEW.card_id = '{}' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            card.id
        ))
        .unwrap();
        let (boards, result) = linked_event_inner(&conn, "session", "s-1", LinkedEvent::Completed);
        assert!(result.unwrap_err().contains("refused"));
        assert_eq!(boards, vec![board.id.clone()]);
        // Nothing linked: nothing to notify.
        let (boards, result) = linked_event_inner(&conn, "session", "nobody", LinkedEvent::Started);
        assert!(result.is_ok() && boards.is_empty());
    }

    #[test]
    fn write_back_reports_cards_and_their_boards_once_each() {
        let conn = setup();
        let (a, a_columns) = board_with_columns(&conn);
        let b = create_board(&conn, "Other").unwrap();
        let b_column = get_board(&conn, &b.id).unwrap().columns[0].id.clone();
        let one = add_cards(&conn, &a_columns[0].id, &["One", "Two"]);
        let three = create_card(&conn, &b_column, "Three", "").unwrap();
        for card in one.iter().chain([&three]) {
            add_link(&conn, &card.id, "session", "remote:env:s-1", None).unwrap();
        }
        let (cards, boards, result) = write_back_inner(&conn, "remote:env:s-1", "All done");
        result.unwrap();
        assert_eq!(cards.len(), 3);
        assert_eq!(boards.len(), 2);
        assert!(boards.contains(&a.id) && boards.contains(&b.id));
        assert_eq!(
            list_comments(&conn, &three.id).unwrap()[0].author,
            "agent:remote:env:s-1"
        );
        // An empty reply writes nothing and notifies nobody.
        let (cards, boards, result) = write_back_inner(&conn, "remote:env:s-1", "  ");
        assert!(result.is_ok() && cards.is_empty() && boards.is_empty());
        let (_, boards, result) = write_back_inner(&conn, "", "x");
        assert!(result.is_err() && boards.is_empty());
        // A comment that fails on one board still notifies every linked board,
        // since the others were written.
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER refuse BEFORE INSERT ON board_comments \
             WHEN NEW.card_id = '{}' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            three.id
        ))
        .unwrap();
        let (_, boards, result) = write_back_inner(&conn, "remote:env:s-1", "Again");
        assert!(result.unwrap_err().contains("refused"));
        assert_eq!(boards.len(), 2);
    }

    #[test]
    fn running_cards_take_kind_and_id_pairs_and_come_back_sorted() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["A", "B", "C"]);
        add_link(&conn, &cards[0].id, "session", "s-1", None).unwrap();
        add_link(&conn, &cards[1].id, "pilot", "p-1", None).unwrap();
        add_link(&conn, &cards[2].id, "session", "s-2", None).unwrap();
        let running: Vec<(String, String)> =
            serde_json::from_value(serde_json::json!([["session", "s-1"], ["pilot", "p-1"]]))
                .unwrap();
        let mut expected = vec![cards[0].id.clone(), cards[1].id.clone()];
        expected.sort();
        assert_eq!(
            running_cards_inner(&conn, &board.id, &running).unwrap(),
            expected
        );
        assert!(running_cards_inner(&conn, &board.id, &[])
            .unwrap()
            .is_empty());
        assert!(running_cards_inner(&conn, "bad id", &running).is_err());
    }

    #[test]
    fn card_tool_refuses_unlinked_subtask() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["Mine", "Theirs", "Old"]);
        add_link(&conn, &cards[0].id, "session", "s-1", None).unwrap();
        add_link(&conn, &cards[1].id, "session", "s-2", None).unwrap();
        let mine = add_subtask(&conn, &cards[0].id, "Write tests").unwrap();
        let theirs = add_subtask(&conn, &cards[1].id, "Their work").unwrap();
        let tick = |id: &str, read_only: bool| {
            agent_card_tool_inner(
                &conn,
                "s-1",
                read_only,
                &CardTool::SubtaskDone {
                    subtask_id: id.into(),
                    done: true,
                },
            )
        };
        // Another session's card, and a subtask id that doesn't exist, are
        // both out of reach, and nothing changes.
        assert_eq!(
            tick(&theirs.id, false).unwrap_err(),
            gespan_board_store::NOT_LINKED
        );
        assert_eq!(
            tick("no-such-subtask", false).unwrap_err(),
            gespan_board_store::NOT_LINKED
        );
        assert!(!list_subtasks(&conn, &cards[1].id).unwrap()[0].done);
        // A plan turn may list but not tick.
        assert_eq!(
            tick(&mine.id, true).unwrap_err(),
            "This session is read-only"
        );
        assert!(!list_subtasks(&conn, &cards[0].id).unwrap()[0].done);
        // Its own card's subtask ticks, and the answer names the board to refresh.
        let ticked = tick(&mine.id, false).unwrap();
        assert_eq!(ticked["boardId"], board.id.as_str());
        assert!(list_subtasks(&conn, &cards[0].id).unwrap()[0].done);
        // Listing shows only the session's own cards.
        let listed = agent_card_tool_inner(&conn, "s-1", true, &CardTool::Subtasks).unwrap();
        let listed = listed["cards"].as_array().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["number"], cards[0].number);
        // Unlinked mid-turn: the card leaves reach at once.
        remove_link(&conn, &cards[0].id, "session", "s-1").unwrap();
        assert_eq!(
            tick(&mine.id, false).unwrap_err(),
            gespan_board_store::NOT_LINKED
        );
        assert!(
            agent_card_tool_inner(&conn, "s-1", false, &CardTool::Subtasks).unwrap()["cards"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
