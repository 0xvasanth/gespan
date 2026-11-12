//! What `CONSTRAINTS` keeps now that Toasty's tables have no foreign keys,
//! cascades, `CHECK` or partial and expression indexes. Each statement here
//! goes straight to SQLite on a second connection, so only the triggers and
//! indexes stand between it and the tables.
use gespan_board_store::testing::{self, Fixture};
use gespan_board_store::*;

struct Board1 {
    board: String,
    column: String,
    card: String,
    other_card: String,
    label: String,
    field: String,
}

fn setup(conn: &Fixture) -> Board1 {
    let board = create_board(conn, "Work").unwrap();
    let column = get_board(conn, &board.id).unwrap().columns[0].id.clone();
    let card = create_card(conn, &column, "One", "").unwrap();
    let other_card = create_card(conn, &column, "Two", "").unwrap();
    let label = create_label(conn, &board.id, "Bug", "e5534b").unwrap();
    let field = create_field(conn, &board.id, "Note", "text", None).unwrap();
    Board1 {
        board: board.id,
        column,
        card: card.id,
        other_card: other_card.id,
        label: label.id,
        field: field.id,
    }
}

fn fails(conn: &Fixture, sql: &str, wanted: &str) {
    let error = conn
        .raw()
        .execute_batch(sql)
        .expect_err(&format!("accepted: {sql}"));
    assert!(error.to_string().contains(wanted), "{sql}: {error}");
}

fn count(conn: &Fixture, sql: &str) -> i64 {
    conn.raw().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
// One shape for every row, whether or not it names an id.
#[allow(clippy::useless_format)]
fn a_row_naming_a_missing_parent_is_refused_like_a_foreign_key() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let (board, column, card, label, field) = (&b.board, &b.column, &b.card, &b.label, &b.field);
    let inserts = [
        format!("INSERT INTO board_columns (id, board_id, name, position, role, color, wip_limit, collapsed) VALUES ('x', 'nope', 'X', 9, NULL, NULL, NULL, 0)"),
        format!("INSERT INTO board_cards (id, board_id, column_id, number, title, body, position, link_kind, link_id, created_at, updated_at, title_from_ticket, body_from_ticket) VALUES ('x', 'nope', '{column}', 99, 'X', '', 9, NULL, NULL, 1, 1, 0, 0)"),
        format!("INSERT INTO board_cards (id, board_id, column_id, number, title, body, position, link_kind, link_id, created_at, updated_at, title_from_ticket, body_from_ticket) VALUES ('x', '{board}', 'nope', 99, 'X', '', 9, NULL, NULL, 1, 1, 0, 0)"),
        format!("INSERT INTO board_activity (id, board_id, card_id, kind, actor, created_at) VALUES ('x', 'nope', NULL, 'edited', 'user', 1)"),
        format!("INSERT INTO board_activity (id, board_id, card_id, kind, actor, created_at) VALUES ('x', '{board}', 'nope', 'edited', 'user', 1)"),
        format!("INSERT INTO board_card_meta (card_id, priority) VALUES ('nope', 0)"),
        format!("INSERT INTO board_labels (id, board_id, name, color, position) VALUES ('x', 'nope', 'X', '000000', 0)"),
        format!("INSERT INTO board_card_labels (card_id, label_id) VALUES ('nope', '{label}')"),
        format!("INSERT INTO board_card_labels (card_id, label_id) VALUES ('{card}', 'nope')"),
        format!("INSERT INTO board_subtasks (id, card_id, title, done, position) VALUES ('x', 'nope', 'X', 0, 0)"),
        format!("INSERT INTO board_comments (id, card_id, body, author, created_at) VALUES ('x', 'nope', 'X', 'user', 1)"),
        format!("INSERT INTO board_card_projects (card_id, project) VALUES ('nope', '/p')"),
        format!("INSERT INTO board_card_relations (card_id, related_card_id, kind) VALUES ('nope', '{card}', 'blocks')"),
        format!("INSERT INTO board_card_relations (card_id, related_card_id, kind) VALUES ('{card}', 'nope', 'blocks')"),
        format!("INSERT INTO board_fields (id, board_id, name, kind, options_json, position) VALUES ('x', 'nope', 'X', 'text', NULL, 0)"),
        format!("INSERT INTO board_card_field_values (card_id, field_id, value_json) VALUES ('nope', '{field}', '1')"),
        format!("INSERT INTO board_card_field_values (card_id, field_id, value_json) VALUES ('{card}', 'nope', '1')"),
        format!("INSERT INTO board_card_links (card_id, kind, ref, role, created_at) VALUES ('nope', 'session', 's', NULL, 1)"),
        format!("INSERT INTO board_settings (board_id, run_with, write_back, ticket_status_changes) VALUES ('nope', 1, 1, 1)"),
        format!("INSERT INTO board_views (id, board_id, name, position, spec_json, layout_json, created_at, updated_at) VALUES ('x', 'nope', 'X', 0, '{{}}', '{{}}', 1, 1)"),
    ];
    for sql in &inserts {
        fails(&conn, sql, "FOREIGN KEY constraint failed");
    }
    // Changing a reference to a missing row is refused too.
    set_card_labels(&conn, card, std::slice::from_ref(label)).unwrap();
    fails(
        &conn,
        &format!("UPDATE board_cards SET column_id = 'nope' WHERE id = '{card}'"),
        "FOREIGN KEY constraint failed",
    );
    fails(
        &conn,
        &format!("UPDATE board_card_labels SET label_id = 'nope' WHERE card_id = '{card}'"),
        "FOREIGN KEY constraint failed",
    );
    // A row naming rows that exist goes in, and an activity row may name no card.
    conn.raw()
        .execute_batch(&format!(
            "INSERT INTO board_activity (id, board_id, card_id, kind, actor, created_at)
               VALUES ('ok', '{board}', NULL, 'deleted', 'user', 1);
             INSERT INTO board_card_projects (card_id, project) VALUES ('{card}', '/p');"
        ))
        .unwrap();
}

#[test]
fn deleting_a_board_deletes_everything_on_it_and_nothing_else() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let keep = setup(&conn);
    for ids in [&b, &keep] {
        set_card_labels(&conn, &ids.card, std::slice::from_ref(&ids.label)).unwrap();
        set_field_value(&conn, &ids.card, &ids.field, Some(r#""x""#)).unwrap();
        add_subtask(&conn, &ids.card, "Sub").unwrap();
        add_comment(&conn, &ids.card, "Hi", "user").unwrap();
        set_projects(&conn, &ids.card, &["/p".to_string()]).unwrap();
        set_priority(&conn, &ids.card, 2).unwrap();
        add_relation(&conn, &ids.card, &ids.other_card, "blocks").unwrap();
        add_link(&conn, &ids.card, "session", "s", None).unwrap();
        create_view(&conn, &ids.board, "Mine", "{}", "{}").unwrap();
    }
    let tables = [
        "board_columns",
        "board_cards",
        "board_activity",
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
        "board_settings",
        "board_views",
    ];
    let counts = |conn: &Fixture| -> Vec<i64> {
        tables
            .iter()
            .map(|table| count(conn, &format!("SELECT COUNT(*) FROM {table}")))
            .collect()
    };
    let both = counts(&conn);
    assert!(both.iter().all(|n| *n > 0), "{both:?}");

    conn.raw()
        .execute("DELETE FROM boards WHERE id = ?1", [&b.board])
        .unwrap();

    // Exactly half of every table is left: the other board's rows.
    let half: Vec<i64> = both.iter().map(|n| n / 2).collect();
    assert_eq!(counts(&conn), half);
    assert_eq!(get_board(&conn, &keep.board).unwrap().cards.len(), 2);
}

#[test]
fn deleting_a_card_deletes_its_rows_and_relations_on_either_side() {
    let conn = testing::fresh();
    let b = setup(&conn);
    add_relation(&conn, &b.other_card, &b.card, "blocks").unwrap();
    add_subtask(&conn, &b.card, "Sub").unwrap();
    set_card_labels(&conn, &b.card, std::slice::from_ref(&b.label)).unwrap();
    set_field_value(&conn, &b.card, &b.field, Some(r#""x""#)).unwrap();
    record_card(&conn, &b.card, "started", None).unwrap();

    conn.raw()
        .execute("DELETE FROM board_cards WHERE id = ?1", [&b.card])
        .unwrap();

    for table in [
        "board_card_meta",
        "board_card_labels",
        "board_subtasks",
        "board_comments",
        "board_card_projects",
        "board_card_field_values",
        "board_card_links",
        "board_activity",
    ] {
        assert_eq!(
            count(
                &conn,
                &format!("SELECT COUNT(*) FROM {table} WHERE card_id = '{}'", b.card)
            ),
            0,
            "{table}"
        );
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_relations"), 0);
    // The other card keeps its own history.
    assert!(!list_activity(&conn, &b.board, Some(&b.other_card), 50)
        .unwrap()
        .is_empty());
}

#[test]
fn deleting_a_label_or_a_field_deletes_what_cards_held_of_it() {
    let conn = testing::fresh();
    let b = setup(&conn);
    set_card_labels(&conn, &b.card, std::slice::from_ref(&b.label)).unwrap();
    set_field_value(&conn, &b.card, &b.field, Some(r#""x""#)).unwrap();

    conn.raw()
        .execute_batch(&format!(
            "DELETE FROM board_labels WHERE id = '{}'; DELETE FROM board_fields WHERE id = '{}';",
            b.label, b.field
        ))
        .unwrap();

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_labels"), 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM board_card_field_values"),
        0
    );
}

#[test]
fn a_column_holding_a_card_cannot_be_deleted() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let sql = format!("DELETE FROM board_columns WHERE id = '{}'", b.column);
    fails(&conn, &sql, "FOREIGN KEY constraint failed");
    // Archived cards hold it too: they keep their column.
    archive_cards(&conn, &[b.card.clone(), b.other_card.clone()]).unwrap();
    fails(&conn, &sql, "FOREIGN KEY constraint failed");
    for card in [&b.card, &b.other_card] {
        delete_card(&conn, card).unwrap();
    }
    conn.raw().execute_batch(&sql).unwrap();
}

#[test]
fn a_relation_kind_outside_the_three_is_refused_like_the_check() {
    let conn = testing::fresh();
    let b = setup(&conn);
    fails(
        &conn,
        &format!(
            "INSERT INTO board_card_relations (card_id, related_card_id, kind)
             VALUES ('{}', '{}', 'parent')",
            b.card, b.other_card
        ),
        "CHECK constraint failed",
    );
    for kind in RELATION_KINDS {
        conn.raw()
            .execute_batch(&format!(
                "INSERT INTO board_card_relations (card_id, related_card_id, kind)
                 VALUES ('{}', '{}', '{kind}')",
                b.card, b.other_card
            ))
            .unwrap();
    }
    fails(
        &conn,
        "UPDATE board_card_relations SET kind = 'parent' WHERE kind = 'blocks'",
        "CHECK constraint failed",
    );
}

#[test]
fn a_role_is_held_by_one_column_per_board_and_columns_without_one_are_free() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let other = create_board(&conn, "Other").unwrap();
    let insert = |id: &str, board: &str, role: Option<&str>| {
        conn.raw().execute(
            "INSERT INTO board_columns (id, board_id, name, position, role, color, wip_limit, collapsed)
             VALUES (?1, ?2, ?1, 9, ?3, NULL, NULL, 0)",
            rusqlite::params![id, board, role],
        )
    };
    let error = insert("dup", &b.board, Some("todo")).unwrap_err();
    assert!(
        error.to_string().contains("UNIQUE constraint failed"),
        "{error}"
    );
    insert("plain-1", &b.board, None).unwrap();
    insert("plain-2", &b.board, None).unwrap();
    insert("elsewhere", &other.id, Some("cancelled")).unwrap();
    insert("mine", &b.board, Some("cancelled")).unwrap();
}

#[test]
fn label_names_are_unique_per_board_ignoring_ascii_case() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let other = create_board(&conn, "Other").unwrap();
    let insert = |id: &str, board: &str, name: &str| {
        conn.raw().execute(
            "INSERT INTO board_labels (id, board_id, name, color, position)
             VALUES (?1, ?2, ?3, '000000', 9)",
            [id, board, name],
        )
    };
    let error = insert("dup", &b.board, "BUG").unwrap_err();
    assert!(
        error.to_string().contains("UNIQUE constraint failed"),
        "{error}"
    );
    insert("elsewhere", &other.id, "bug").unwrap();
    insert("docs", &b.board, "Docs").unwrap();
}

#[test]
fn a_card_number_is_unique_on_its_board() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let other = create_board(&conn, "Other").unwrap();
    let other_column = get_board(&conn, &other.id).unwrap().columns[0].id.clone();
    let insert = |id: &str, board: &str, column: &str| {
        conn.raw().execute(
            "INSERT INTO board_cards (id, board_id, column_id, number, title, body, position,
               link_kind, link_id, created_at, updated_at, title_from_ticket, body_from_ticket)
             VALUES (?1, ?2, ?3, 1, 'X', '', 9, NULL, NULL, 1, 1, 0, 0)",
            [id, board, column],
        )
    };
    // Card 1 is already on the first board.
    let error = insert("dup", &b.board, &b.column).unwrap_err();
    assert!(
        error.to_string().contains("UNIQUE constraint failed"),
        "{error}"
    );
    insert("elsewhere", &other.id, &other_column).unwrap();
}

#[test]
fn the_constraints_run_again_on_every_open_without_failing() {
    let conn = testing::fresh();
    let b = setup(&conn);
    let objects = |conn: &Fixture| {
        count(
            conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('trigger', 'index')",
        )
    };
    let before = objects(&conn);
    let again = conn.reopen();
    assert_eq!(objects(&again), before);
    // Still enforced after reopening.
    fails(
        &again,
        &format!("DELETE FROM board_columns WHERE id = '{}'", b.column),
        "FOREIGN KEY constraint failed",
    );
}
