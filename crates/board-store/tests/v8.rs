//! Board schema v8: `board_settings.run_in_worktree` and `run_worktree_choice`.
use gespan_board_store::*;
use rusqlite::Connection;

fn fresh() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

#[test]
fn a_new_board_runs_in_a_worktree_by_default() {
    let conn = fresh();
    let board = create_board(&conn, "New").unwrap();
    let settings = board_settings(&conn, &board.id).unwrap();
    assert!(settings.run_in_worktree);
    assert_eq!(settings.run_worktree_choice, None);
    assert!(settings.starts_in_worktree());
}

#[test]
fn a_board_from_before_v8_keeps_running_in_place() {
    let conn = fresh();
    let with_row = create_board(&conn, "With settings").unwrap();
    let without_row = create_board(&conn, "Without settings").unwrap();
    conn.execute_batch(&format!(
        "ALTER TABLE board_settings DROP COLUMN run_in_worktree;
         ALTER TABLE board_settings DROP COLUMN run_worktree_choice;
         DELETE FROM board_settings WHERE board_id = '{}';
         UPDATE board_schema SET version = 7;",
        without_row.id
    ))
    .unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 7);
    ensure_tables(&conn).unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), SCHEMA_VERSION);
    for board in [&with_row, &without_row] {
        let settings = board_settings(&conn, &board.id).unwrap();
        assert!(!settings.run_in_worktree, "{}", board.name);
        assert!(!settings.starts_in_worktree(), "{}", board.name);
    }
}

#[test]
fn the_last_choice_wins_until_it_is_cleared() {
    let conn = fresh();
    let board = create_board(&conn, "Board").unwrap();
    let choose = |choice| SettingsPatch {
        run_worktree_choice: Some(choice),
        ..Default::default()
    };
    let settings = update_board_settings(&conn, &board.id, &choose(Some(false))).unwrap();
    assert!(settings.run_in_worktree, "the setting itself didn't change");
    assert!(!settings.starts_in_worktree());
    let settings = update_board_settings(&conn, &board.id, &choose(Some(true))).unwrap();
    assert!(settings.starts_in_worktree());
    let off = SettingsPatch {
        run_in_worktree: Some(false),
        ..Default::default()
    };
    let settings = update_board_settings(&conn, &board.id, &off).unwrap();
    assert!(settings.starts_in_worktree(), "still the remembered choice");
    let settings = update_board_settings(&conn, &board.id, &choose(None)).unwrap();
    assert!(!settings.starts_in_worktree());
}

#[test]
fn the_choice_is_per_board_and_stays_out_of_templates() {
    let conn = fresh();
    let one = create_board(&conn, "One").unwrap();
    let other = create_board(&conn, "Other").unwrap();
    let patch = SettingsPatch {
        run_worktree_choice: Some(Some(false)),
        ..Default::default()
    };
    update_board_settings(&conn, &one.id, &patch).unwrap();
    assert!(board_settings(&conn, &other.id)
        .unwrap()
        .starts_in_worktree());
    let template = save_board_as_template(&conn, &one.id, "Saved").unwrap();
    let copy = create_board_from_template(&conn, "Copy", &template.id).unwrap();
    assert_eq!(
        board_settings(&conn, &copy.id).unwrap().run_worktree_choice,
        None
    );
}
