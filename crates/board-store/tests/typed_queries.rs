//! Behaviour the raw SQL had that the typed queries reproduce by hand:
//! SQLite's `trim` and `lower`, the order of tied rows, and the errors a
//! statement that touched no row used to give.
use gespan_board_store::testing::{self, Fixture};
use gespan_board_store::*;

fn fresh() -> Fixture {
    testing::fresh()
}

fn first_column(conn: &Fixture, board: &Board) -> String {
    get_board(conn, &board.id).unwrap().columns[0].id.clone()
}

fn ticket_marks(conn: &Fixture, card_id: &str) -> (bool, bool) {
    conn.raw()
        .query_row(
            "SELECT title_from_ticket, body_from_ticket FROM board_cards WHERE id = ?1",
            [card_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn mark_from_ticket(conn: &Fixture, card_id: &str) {
    conn.raw()
        .execute(
            "UPDATE board_cards SET title_from_ticket = 1, body_from_ticket = 1 WHERE id = ?1",
            [card_id],
        )
        .unwrap();
}

#[test]
fn update_card_keeps_ticket_marks_as_the_sql_case_did() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let card = create_card(&conn, &first_column(&conn, &board), "WEB-1", "From Jira").unwrap();

    // A body-only edit keeps both marks: the title is the same text.
    mark_from_ticket(&conn, &card.id);
    update_card(&conn, &card.id, "WEB-1", "Edited").unwrap();
    assert_eq!(ticket_marks(&conn, &card.id), (true, true));

    // A new title is the user's own; the description is still ticket text.
    update_card(&conn, &card.id, "Mine", "Edited").unwrap();
    assert_eq!(ticket_marks(&conn, &card.id), (false, true));

    // Emptying the description to spaces clears its mark.
    update_card(&conn, &card.id, "Mine", "   ").unwrap();
    assert_eq!(ticket_marks(&conn, &card.id), (false, false));

    // SQL's `trim` strips spaces only, so a description of a newline and a
    // tab is not empty to it and keeps the mark.
    mark_from_ticket(&conn, &card.id);
    update_card(&conn, &card.id, "Mine", "\n\t").unwrap();
    assert_eq!(ticket_marks(&conn, &card.id), (true, true));

    // Saving what is there already writes nothing.
    let before = get_card(&conn, &card.id).unwrap();
    update_card(&conn, &card.id, "Mine", "\n\t").unwrap();
    assert_eq!(
        get_card(&conn, &card.id).unwrap().updated_at,
        before.updated_at
    );
}

#[test]
fn rename_board_renames_and_refuses_a_missing_board() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    conn.raw()
        .execute(
            "UPDATE boards SET updated_at = 1 WHERE id = ?1",
            [&board.id],
        )
        .unwrap();
    rename_board(&conn, &board.id, "  Home ").unwrap();
    let renamed = get_board(&conn, &board.id).unwrap().board;
    assert_eq!(renamed.name, "Home");
    assert!(renamed.updated_at > 1);
    assert_eq!(
        rename_board(&conn, "missing", "Home").unwrap_err(),
        "Board not found"
    );
    assert!(rename_board(&conn, &board.id, "  ").is_err());
    assert_eq!(get_board(&conn, &board.id).unwrap().board.name, "Home");
}

#[test]
fn saved_templates_sort_by_name_folding_ascii_case_only() {
    let conn = fresh();
    let board = create_board(&conn, "Source").unwrap();
    // SQLite's `lower` folds A-Z only: "Éz" keeps its capital, so it sorts
    // before "éa" (É is a smaller byte than é), where a full Unicode fold
    // would put "éa" first.
    for name in ["éa", "beta", "Éz", "alpha", "Alpha"] {
        save_board_as_template(&conn, &board.id, name).unwrap();
    }
    let names: Vec<String> = list_templates(&conn)
        .unwrap()
        .into_iter()
        .filter(|t| !t.built_in)
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["Alpha", "alpha", "beta", "Éz", "éa"]);
}

#[test]
fn delete_template_refuses_a_missing_one() {
    let conn = fresh();
    let board = create_board(&conn, "Source").unwrap();
    let saved = save_board_as_template(&conn, &board.id, "Mine").unwrap();
    assert_eq!(
        delete_template(&conn, "missing").unwrap_err(),
        "Template not found"
    );
    delete_template(&conn, &saved.id).unwrap();
    assert_eq!(
        delete_template(&conn, &saved.id).unwrap_err(),
        "Template not found"
    );
    assert!(list_templates(&conn).unwrap().iter().all(|t| t.built_in));
}

#[test]
fn labels_tied_on_position_list_by_ascii_folded_name() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let card = create_card(&conn, &first_column(&conn, &board), "Card", "").unwrap();
    let mut ids = Vec::new();
    for name in ["éa", "beta", "Éz", "Alpha"] {
        ids.push(create_label(&conn, &board.id, name, "ff0000").unwrap().id);
    }
    // Rows an older build left with one position.
    conn.raw()
        .execute(
            "UPDATE board_labels SET position = 0 WHERE board_id = ?1",
            [&board.id],
        )
        .unwrap();
    let names: Vec<String> = list_labels(&conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|label| label.name)
        .collect();
    assert_eq!(names, ["Alpha", "beta", "Éz", "éa"]);

    // A card face lists its labels in the same order.
    set_card_labels(&conn, &card.id, &ids).unwrap();
    let in_order: Vec<String> = list_labels(&conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|label| label.id)
        .collect();
    assert_eq!(
        card_extras(&conn, &board.id).unwrap()[&card.id].labels,
        in_order
    );
    assert_eq!(card_index(&conn, &board.id).unwrap()[0].labels, in_order);
}

#[test]
fn archived_cards_list_newest_archive_first_then_highest_number() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let column = first_column(&conn, &board);
    let cards: Vec<Card> = ["a", "b", "c", "d"]
        .iter()
        .map(|title| create_card(&conn, &column, title, "").unwrap())
        .collect();
    for card in &cards[..3] {
        archive_card(&conn, &card.id).unwrap();
    }
    let set_archived = |card: &Card, at: i64| {
        conn.raw()
            .execute(
                "UPDATE board_card_meta SET archived_at = ?1 WHERE card_id = ?2",
                rusqlite::params![at, card.id],
            )
            .unwrap();
    };
    set_archived(&cards[0], 300);
    set_archived(&cards[1], 100);
    set_archived(&cards[2], 100);
    let titles: Vec<String> = archived_cards(&conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|card| card.title)
        .collect();
    assert_eq!(titles, ["a", "c", "b"]);
    // Another board's archive is its own.
    let other = create_board(&conn, "Other").unwrap();
    assert!(archived_cards(&conn, &other.id).unwrap().is_empty());
}

#[test]
fn update_field_with_only_a_name_keeps_its_options() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let field = create_field(&conn, &board.id, "Size", "select", Some(r#"["S","L"]"#)).unwrap();
    update_field(&conn, &field.id, Some("Bigness"), None).unwrap();
    let after = list_fields(&conn, &board.id).unwrap().remove(0);
    assert_eq!(after.name, "Bigness");
    assert_eq!(after.options_json.as_deref(), Some(r#"["S","L"]"#));
}

#[test]
fn update_view_with_only_a_spec_keeps_the_name_and_layout() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let view = create_view(&conn, &board.id, "Mine", "{}", r#"{"layout":"list"}"#).unwrap();
    let spec = r#"{"query":"bug"}"#;
    update_view(&conn, &view.id, None, Some(spec), None).unwrap();
    let after = list_views(&conn, &board.id).unwrap().remove(0);
    assert_eq!(after.name, "Mine");
    assert_eq!(after.spec_json, spec);
    assert_eq!(after.layout_json, r#"{"layout":"list"}"#);
    assert_eq!(
        update_view(&conn, "missing", Some("X"), None, None).unwrap_err(),
        "View not found"
    );
}

// A typed read of a row whose stored value doesn't match the model panics
// Toasty 0.11's SQLite driver and takes the connection down with it. The two
// reads the app shows an error for instead (settings and the saved-template
// list) stay raw, so a corrupt row is an error and the connection lives on.

#[test]
fn an_unreadable_settings_row_is_an_error_and_the_connection_survives() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    update_board_settings(
        &conn,
        &board.id,
        &SettingsPatch {
            write_back: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    conn.raw()
        .execute(
            "UPDATE board_settings SET run_with = 'garbage' WHERE board_id = ?1",
            [&board.id],
        )
        .unwrap();
    assert!(board_settings(&conn, &board.id).is_err());
    // The same connection still serves reads and writes.
    assert_eq!(list_boards(&conn).unwrap().len(), 1);
    conn.raw()
        .execute(
            "UPDATE board_settings SET run_with = 1 WHERE board_id = ?1",
            [&board.id],
        )
        .unwrap();
    let settings = board_settings(&conn, &board.id).unwrap();
    assert!(settings.run_with);
    assert!(!settings.write_back, "the stored value, not the default");
}

#[test]
fn a_template_with_an_unreadable_name_is_an_error_and_the_connection_survives() {
    let conn = fresh();
    let board = create_board(&conn, "Source").unwrap();
    save_board_as_template(&conn, &board.id, "Mine").unwrap();
    conn.raw()
        .execute(
            "INSERT INTO board_templates (id, name, spec_json, created_at)
             VALUES ('broken', X'00ff', '{}', 0)",
            [],
        )
        .unwrap();
    assert!(list_templates(&conn).is_err());
    // A blank board can still be made on the same connection.
    let blank = create_board_from_template(&conn, "Fallback", "blank").unwrap();
    assert_eq!(get_board(&conn, &blank.id).unwrap().board.name, "Fallback");
    conn.raw()
        .execute("DELETE FROM board_templates WHERE id = 'broken'", [])
        .unwrap();
    let saved: Vec<String> = list_templates(&conn)
        .unwrap()
        .into_iter()
        .filter(|t| !t.built_in)
        .map(|t| t.name)
        .collect();
    assert_eq!(saved, ["Mine"]);
}
