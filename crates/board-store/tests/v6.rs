//! v6: `board_cards.title_from_ticket`.
use gespan_board_store::*;
use rusqlite::Connection;

fn open(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn has_column(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('board_cards')
                       WHERE name = 'title_from_ticket')",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn v6_migration_from_v5_adds_the_ticket_title_mark_idempotently() {
    let path = tempfile::NamedTempFile::new().unwrap();
    let conn = open(path.path());
    let board = create_board(&conn, "Work").unwrap();
    let column = get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    let mine = create_card(&conn, &column, "My card", "body").unwrap();
    let copied = create_card(&conn, &column, "Login fails", "").unwrap();
    set_ticket(
        &conn,
        &copied.id,
        Some(r#"{"tracker":"jira","key":"ABC-1","title":"Login fails"}"#),
    )
    .unwrap();
    // Made from a ticket and never retitled: titled with the key.
    let keyed = create_card(&conn, &column, "eng-4", "").unwrap();
    set_ticket(
        &conn,
        &keyed.id,
        Some(r#"{"tracker":"linear","key":"ENG-4"}"#),
    )
    .unwrap();
    // Linked to a ticket but titled by the user.
    let renamed = create_card(&conn, &column, "My own words", "").unwrap();
    set_ticket(
        &conn,
        &renamed.id,
        Some(r#"{"tracker":"plane","key":"P-1","title":"Something else"}"#),
    )
    .unwrap();
    // Turn it back into a v5 database.
    conn.execute_batch(
        "ALTER TABLE board_cards DROP COLUMN title_from_ticket;
         UPDATE board_schema SET version = 5;",
    )
    .unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 5);
    assert!(!has_column(&conn));
    // A v5 build still writes cards without the column.
    conn.execute(
        "INSERT INTO board_cards (id, board_id, column_id, number, title, body, position,
                                  created_at, updated_at)
         VALUES ('old', ?1, ?2, 99, 'Old build card', '', 9, 1, 1)",
        [&board.id, &column],
    )
    .unwrap();

    ensure_tables(&conn).unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), SCHEMA_VERSION);
    assert!(has_column(&conn));
    let schema_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM board_schema", [], |row| row.get(0))
        .unwrap();
    assert_eq!(schema_rows, 1);
    // Existing cards keep their data. A card titled with its ticket's
    // snapshot title (or key) is marked; the others are not.
    for id in [&mine.id, &"old".to_string()] {
        assert!(!title_from_ticket(&conn, id).unwrap(), "{id}");
    }
    assert!(title_from_ticket(&conn, &copied.id).unwrap());
    assert!(title_from_ticket(&conn, &keyed.id).unwrap());
    assert!(!title_from_ticket(&conn, &renamed.id).unwrap());
    // So a later ticket rename no longer brings the copied title back.
    set_ticket(
        &conn,
        &copied.id,
        Some(r#"{"tracker":"jira","key":"ABC-1","title":"Renamed upstream"}"#),
    )
    .unwrap();
    assert!(!run_prompt(&conn, &copied.id)
        .unwrap()
        .contains("Login fails"));
    assert!(run_prompt(&conn, &renamed.id)
        .unwrap()
        .contains("My own words"));
    assert_eq!(get_card(&conn, &mine.id).unwrap().body, "body");
    assert!(run_prompt(&conn, &mine.id)
        .unwrap()
        .starts_with(&format!("Work on board card #{}: My card", mine.number)));
    // A title equal to its ticket's snapshot is still left out.
    assert_eq!(
        run_prompt(&conn, &copied.id).unwrap(),
        format!("Work on board card #{}", copied.number)
    );

    // The mark works and survives running the migration again.
    let card =
        create_card_from_ticket(&conn, &column, r#"{"tracker":"jira","key":"ABC-2"}"#).unwrap();
    assert!(title_from_ticket(&conn, &card.id).unwrap());
    migrate::run(&conn).unwrap();
    ensure_tables(&conn).unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), SCHEMA_VERSION);
    assert!(title_from_ticket(&conn, &card.id).unwrap());

    // A second connection opening the same file finds the work done.
    let other = open(path.path());
    assert_eq!(migrate::version(&other).unwrap(), SCHEMA_VERSION);
    assert!(title_from_ticket(&other, &card.id).unwrap());
}

#[test]
fn a_fresh_database_marks_no_card_until_one_comes_from_a_ticket() {
    let path = tempfile::NamedTempFile::new().unwrap();
    let conn = open(path.path());
    assert_eq!(migrate::version(&conn).unwrap(), SCHEMA_VERSION);
    let board = create_board(&conn, "Work").unwrap();
    let column = get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    let card = create_card(&conn, &column, "Mine", "").unwrap();
    assert!(!title_from_ticket(&conn, &card.id).unwrap());
    assert!(title_from_ticket(&conn, "missing").is_err());
}
