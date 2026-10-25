//! The board's Tauri commands. Storage is the shared `gespan-board-store`
//! crate the native app also uses; each mutating command emits `CHANGED`.
use gespan_board_store::{
    cards_for_link, create_board, create_card, create_column, delete_board, delete_card,
    delete_column, get_board, link_card, list_activity, list_boards, move_card, move_column,
    record_card, rename_board, update_card, update_column, validate_id, Activity, Board,
    BoardDetail, Card, Column,
};
#[cfg(test)]
use gespan_board_store::{ensure_tables, get_card};
use gespan_tracker_client::tickets::TicketRef;
use std::collections::HashMap;
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
}
