//! A `gespan.db` with the hand-made board schema, opened by the board
//! service: adoption runs the board's legacy upgrade, then rebuilds every
//! board table into Toasty's first migration. Every row survives and the file
//! ends with the same board schema as a fresh install.
use gespan_core::board::Service;
use rusqlite::Connection;
use std::path::Path;

/// The board tables as both apps created them before v3, with the native
/// app's `board_native_*` extras.
const V2_DDL: &str = concat!(
    include_str!("fixtures/board-v2.sql"),
    "
CREATE TABLE worktree_removals (path TEXT PRIMARY KEY, sessions_json TEXT);
INSERT INTO worktree_removals VALUES ('/w', 'Not a model table');
"
);

/// Every board table, index and trigger with the SQL SQLite keeps for it.
fn board_schema(path: &Path) -> Vec<(String, String, Option<String>)> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_master
             WHERE (tbl_name LIKE 'board%' OR name LIKE 'board%')
             ORDER BY type, name",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

fn count(path: &Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn fresh_schema(dir: &Path) -> Vec<(String, String, Option<String>)> {
    let fresh = dir.join("fresh.db");
    drop(Service::open(&fresh).unwrap());
    board_schema(&fresh)
}

#[test]
fn a_pre_v3_file_keeps_every_value_and_ends_at_the_fresh_install_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(V2_DDL)
        .unwrap();

    let mut service = Service::open(&path).unwrap();

    let extras = &service.extras("b1").unwrap()["k1"];
    assert_eq!(
        (extras.priority, extras.due_at),
        (3, Some(1_793_404_800_000))
    );
    assert_eq!(extras.projects, ["/a", "/b"]);
    assert_eq!(extras.labels, ["l-bug", "l-docs"], "old name order");
    assert_eq!(
        (extras.subtasks_done, extras.subtasks_total, extras.comments),
        (1, 2, 2)
    );
    let comments: Vec<String> = service
        .comments("k1")
        .unwrap()
        .into_iter()
        .map(|c| c.body)
        .collect();
    assert_eq!(comments, ["first", "second"]);
    // The single legacy link became a `primary` links row.
    let links = service.card_links("k1").unwrap();
    assert_eq!(
        (links[0].kind.as_str(), links[0].reference.as_str()),
        ("session", "s-1")
    );
    assert_eq!(
        service.cards_for_link("session", "s-1").unwrap()[0].id,
        "k1"
    );
    let detail = service.get("b1").unwrap();
    assert_eq!(detail.cards.len(), 2);
    assert_eq!(detail.cards[0].created_at, 20);
    assert_eq!(service.activity("b1", Some("k1"), 10).unwrap().len(), 1);
    // New writes keep counting from the board's own sequence.
    let card = service.card_create("c-todo", "Next", "").unwrap();
    assert_eq!(card.number, 3);

    // Same tables, indexes and triggers as a new database; the version table
    // and the native tables are gone; other tables are left as they were.
    assert_eq!(board_schema(&path), fresh_schema(dir.path()));
    assert_eq!(count(&path, "SELECT COUNT(*) FROM worktree_removals"), 1);
    assert_eq!(
        count(
            &path,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'board_schema'
               OR name LIKE 'board_native%'"
        ),
        0
    );
}

#[test]
fn a_v7_file_keeps_its_rows_and_its_boards_keep_running_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    {
        // The v2 file brought up to v8 by the hand-made schema, then turned
        // back into v7: no worktree settings yet.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(V2_DDL).unwrap();
        gespan_board_store::migrate::legacy_upgrade(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO board_settings (board_id, write_back) VALUES ('b1', 0);
             ALTER TABLE board_settings DROP COLUMN run_in_worktree;
             ALTER TABLE board_settings DROP COLUMN run_worktree_choice;
             UPDATE board_schema SET version = 7;",
        )
        .unwrap();
    }
    let rows = |table: &str| count(&path, &format!("SELECT COUNT(*) FROM {table}"));
    let before: Vec<i64> = [
        "boards",
        "board_cards",
        "board_card_meta",
        "board_labels",
        "board_card_labels",
        "board_subtasks",
        "board_comments",
        "board_card_links",
        "board_settings",
    ]
    .iter()
    .map(|table| rows(table))
    .collect();

    let mut service = Service::open(&path).unwrap();

    let after: Vec<i64> = [
        "boards",
        "board_cards",
        "board_card_meta",
        "board_labels",
        "board_card_labels",
        "board_subtasks",
        "board_comments",
        "board_card_links",
        "board_settings",
    ]
    .iter()
    .map(|table| rows(table))
    .collect();
    assert_eq!(after, before);
    let settings = service.settings("b1").unwrap();
    assert!(!settings.write_back, "kept");
    assert!(
        !settings.run_in_worktree,
        "a board from before v8 runs in place"
    );
    assert_eq!(board_schema(&path), fresh_schema(dir.path()));
    // The constraints hold on the adopted tables.
    assert!(Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM board_columns WHERE id = 'c-todo'", [])
        .is_err());
    service.delete("b1").unwrap();
    assert_eq!(rows("board_cards"), 0);
    assert_eq!(rows("board_card_labels"), 0);
}
