use gespan_board_store::*;
use rusqlite::{params, Connection};
use std::path::Path;

/// The board tables exactly as both apps created them before v3, plus the
/// native app's `board_native_*` extras. Kept verbatim here (not shared with
/// the crate) so the fixture can't drift along with the current schema.
const V2_DDL: &str = "
CREATE TABLE boards (id TEXT PRIMARY KEY, name TEXT NOT NULL, card_seq INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE board_columns (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, name TEXT NOT NULL, position INTEGER NOT NULL, role TEXT);
CREATE UNIQUE INDEX board_columns_role_idx ON board_columns (board_id, role) WHERE role IS NOT NULL;
CREATE TABLE board_cards (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, column_id TEXT NOT NULL REFERENCES board_columns(id), number INTEGER NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', position INTEGER NOT NULL, link_kind TEXT, link_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE (board_id, number));
CREATE INDEX board_cards_column_idx ON board_cards (column_id, position);
CREATE INDEX board_cards_link_idx ON board_cards (link_kind, link_id);
CREATE TABLE board_activity (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, card_id TEXT REFERENCES board_cards(id) ON DELETE CASCADE, kind TEXT NOT NULL, actor TEXT NOT NULL, from_column TEXT, to_column TEXT, detail TEXT, created_at INTEGER NOT NULL);
CREATE TABLE board_native_meta (card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE, priority INTEGER NOT NULL DEFAULT 0, due_at INTEGER);
CREATE TABLE board_native_projects (card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, project TEXT NOT NULL, PRIMARY KEY (card_id, project));
CREATE TABLE board_native_labels (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, name TEXT NOT NULL, color TEXT NOT NULL);
CREATE UNIQUE INDEX board_native_labels_name_idx ON board_native_labels (board_id, lower(name));
CREATE TABLE board_native_card_labels (card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, label_id TEXT NOT NULL REFERENCES board_native_labels(id) ON DELETE CASCADE, PRIMARY KEY (card_id, label_id));
CREATE TABLE board_native_subtasks (id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL);
CREATE TABLE board_native_comments (id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, body TEXT NOT NULL, created_at INTEGER NOT NULL);
";

const DUE: i64 = 1_793_404_800_000;

/// A v2 database with one board, two cards and every kind of native extra.
fn write_v2_fixture(conn: &Connection) {
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn.execute_batch(V2_DDL).unwrap();
    conn.execute_batch(
        "INSERT INTO boards VALUES ('b1', 'Work', 2, 10, 10);
         INSERT INTO boards VALUES ('b2', 'Other', 0, 11, 11);
         INSERT INTO board_columns VALUES ('c-todo', 'b1', 'Backlog', 0, 'todo');
         INSERT INTO board_columns VALUES ('c-doing', 'b1', 'In Progress', 1, 'doing');
         INSERT INTO board_cards VALUES ('k1', 'b1', 'c-todo', 1, 'Fix login', 'body', 0, 'session', 's-1', 20, 21);
         INSERT INTO board_cards VALUES ('k2', 'b1', 'c-todo', 2, 'Write docs', '', 1, NULL, NULL, 22, 23);
         INSERT INTO board_native_meta VALUES ('k1', 3, 1793404800000);
         INSERT INTO board_native_meta VALUES ('k2', 1, NULL);
         INSERT INTO board_native_projects VALUES ('k1', '/a'), ('k1', '/b');
         INSERT INTO board_native_labels VALUES ('l-docs', 'b1', 'Docs', '4da3f5');
         INSERT INTO board_native_labels VALUES ('l-bug', 'b1', 'bug', 'e5534b');
         INSERT INTO board_native_labels VALUES ('l-other', 'b2', 'Zed', '000000');
         INSERT INTO board_native_card_labels VALUES ('k1', 'l-docs'), ('k1', 'l-bug'), ('k2', 'l-docs');
         INSERT INTO board_native_subtasks VALUES ('s1', 'k1', 'One', 1, 0), ('s2', 'k1', 'Two', 0, 1);
         INSERT INTO board_native_comments VALUES ('m1', 'k1', 'first', 30), ('m2', 'k1', 'second', 31);",
    )
    .unwrap();
}

fn v2_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    write_v2_fixture(&conn);
    conn
}

fn fresh() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn version(conn: &Connection) -> i64 {
    count(conn, "SELECT version FROM board_schema")
}

/// A board with its default columns and the given cards in the first column.
fn board_with_cards(conn: &Connection, titles: &[&str]) -> (Board, Vec<Column>, Vec<Card>) {
    let board = create_board(conn, "Work").unwrap();
    let columns = get_board(conn, &board.id).unwrap().columns;
    let cards = titles
        .iter()
        .map(|title| create_card(conn, &columns[0].id, title, "").unwrap())
        .collect();
    (board, columns, cards)
}

fn activity_count(conn: &Connection, board: &str, card: &str) -> usize {
    list_activity(conn, board, Some(card), 500).unwrap().len()
}

fn last_detail(conn: &Connection, board: &str, card: &str) -> Option<String> {
    list_activity(conn, board, Some(card), 1).unwrap()[0]
        .detail
        .clone()
}

#[test]
fn migrates_native_v2_data_keeping_every_value() {
    let conn = v2_db();
    ensure_tables(&conn).unwrap();
    assert_eq!(version(&conn), 3);

    let meta = card_meta(&conn, "k1").unwrap();
    assert_eq!((meta.priority, meta.due_at), (3, Some(DUE)));
    assert_eq!(card_meta(&conn, "k2").unwrap().priority, 1);

    let extras = card_extras(&conn, "b1").unwrap();
    let k1 = &extras["k1"];
    assert_eq!(k1.projects, ["/a", "/b"]);
    assert_eq!(
        (k1.subtasks_done, k1.subtasks_total, k1.comments),
        (1, 2, 2)
    );
    // Positions follow the old name order, case folded: bug before Docs.
    let labels = list_labels(&conn, "b1").unwrap();
    let summary: Vec<(&str, &str, &str, i64)> = labels
        .iter()
        .map(|l| (l.id.as_str(), l.name.as_str(), l.color.as_str(), l.position))
        .collect();
    assert_eq!(
        summary,
        [
            ("l-bug", "bug", "e5534b", 0),
            ("l-docs", "Docs", "4da3f5", 1)
        ]
    );
    assert_eq!(list_labels(&conn, "b2").unwrap()[0].position, 0);
    assert_eq!(k1.labels, ["l-bug", "l-docs"]);
    assert_eq!(extras["k2"].labels, ["l-docs"]);

    let subtasks = list_subtasks(&conn, "k1").unwrap();
    let subtasks: Vec<(&str, &str, bool, i64)> = subtasks
        .iter()
        .map(|s| (s.id.as_str(), s.title.as_str(), s.done, s.position))
        .collect();
    assert_eq!(subtasks, [("s1", "One", true, 0), ("s2", "Two", false, 1)]);

    let comments = list_comments(&conn, "k1").unwrap();
    let comments: Vec<(&str, &str, &str, i64, Option<i64>)> = comments
        .iter()
        .map(|c| {
            (
                c.id.as_str(),
                c.body.as_str(),
                c.author.as_str(),
                c.created_at,
                c.edited_at,
            )
        })
        .collect();
    assert_eq!(
        comments,
        [
            ("m1", "first", "user", 30, None),
            ("m2", "second", "user", 31, None)
        ]
    );

    // The old tables and link columns stay readable for one release.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_native_meta"), 2);
    let card = get_card(&conn, "k1").unwrap();
    assert_eq!(card.link_kind.as_deref(), Some("session"));
    // Migration is not an edit: no activity rows appear.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_activity"), 0);
    // Columns gain their display settings with neutral defaults.
    let column = &get_board(&conn, "b1").unwrap().columns[0];
    assert_eq!(
        (column.color.as_deref(), column.wip_limit, column.collapsed),
        (None, None, false)
    );
}

#[test]
fn migration_is_idempotent() {
    let conn = v2_db();
    ensure_tables(&conn).unwrap();
    let tables = [
        "board_card_meta",
        "board_card_projects",
        "board_labels",
        "board_card_labels",
        "board_subtasks",
        "board_comments",
        "board_card_links",
        "board_schema",
    ];
    let counts = |conn: &Connection| -> Vec<i64> {
        tables
            .iter()
            .map(|t| count(conn, &format!("SELECT COUNT(*) FROM {t}")))
            .collect()
    };
    let before = counts(&conn);
    ensure_tables(&conn).unwrap();
    migrate::run(&conn).unwrap();
    assert_eq!(counts(&conn), before);
    assert_eq!(version(&conn), 3);

    // Edits made after the migration are not undone by the old rows.
    set_priority(&conn, "k1", 1).unwrap();
    set_card_labels(&conn, "k1", &[]).unwrap();
    ensure_tables(&conn).unwrap();
    migrate::run(&conn).unwrap();
    assert_eq!(card_meta(&conn, "k1").unwrap().priority, 1);
    assert!(card_extras(&conn, "b1").unwrap()["k1"].labels.is_empty());
}

#[test]
fn a_fresh_database_starts_at_v3_and_accepts_the_cancelled_role() {
    let conn = fresh();
    assert_eq!(version(&conn), 3);
    let (board, _, _) = board_with_cards(&conn, &[]);
    let column = create_column(&conn, &board.id, "Won't do", Some("cancelled")).unwrap();
    assert_eq!(column.role.as_deref(), Some("cancelled"));
}

#[test]
fn link_columns_backfill_links() {
    let conn = v2_db();
    ensure_tables(&conn).unwrap();
    let links = card_links(&conn, "k1").unwrap();
    let links: Vec<(&str, &str, Option<&str>)> = links
        .iter()
        .map(|l| (l.kind.as_str(), l.reference.as_str(), l.role.as_deref()))
        .collect();
    assert_eq!(links, [("session", "s-1", Some("primary"))]);
    assert!(card_links(&conn, "k2").unwrap().is_empty());
    let found: Vec<String> = cards_for_link(&conn, "session", "s-1")
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(found, ["k1"]);
}

#[test]
fn cards_for_link_finds_cards_linked_only_through_the_links_table() {
    let conn = fresh();
    let (_, _, cards) = board_with_cards(&conn, &["a", "b"]);
    add_link(&conn, &cards[1].id, "pr", "owner/repo#12", None).unwrap();
    add_link(&conn, &cards[1].id, "session", "s-9", None).unwrap();
    let found = cards_for_link(&conn, "pr", "owner/repo#12").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, cards[1].id);
    assert_eq!(cards_for_link(&conn, "session", "s-9").unwrap().len(), 1);
    assert!(add_link(&conn, &cards[0].id, "email", "x", None).is_err());
    assert!(add_link(&conn, &cards[0].id, "jira", "", None).is_err());

    remove_link(&conn, &cards[1].id, "pr", "owner/repo#12").unwrap();
    assert!(cards_for_link(&conn, "pr", "owner/repo#12")
        .unwrap()
        .is_empty());
}

#[test]
fn the_legacy_single_link_keeps_the_links_table_in_step() {
    let conn = fresh();
    let (_, _, cards) = board_with_cards(&conn, &["a"]);
    let card = &cards[0].id;
    let kinds = |conn: &Connection| -> Vec<(String, String)> {
        card_links(conn, card)
            .unwrap()
            .into_iter()
            .map(|l| (l.kind, l.reference))
            .collect()
    };
    link_card(&conn, card, Some("session"), Some("s-1")).unwrap();
    add_link(&conn, card, "pr", "r#1", None).unwrap();
    link_card(&conn, card, Some("pilot"), Some("p-1")).unwrap();
    assert_eq!(
        kinds(&conn),
        [("pilot".into(), "p-1".into()), ("pr".into(), "r#1".into())]
    );
    assert!(cards_for_link(&conn, "session", "s-1").unwrap().is_empty());
    assert_eq!(cards_for_link(&conn, "pilot", "p-1").unwrap().len(), 1);
    // Removing the primary link through the new API clears the old columns.
    remove_link(&conn, card, "pilot", "p-1").unwrap();
    let saved = get_card(&conn, card).unwrap();
    assert_eq!((saved.link_kind, saved.link_id), (None, None));
    link_card(&conn, card, Some("flow-run"), Some("f-1")).unwrap();
    link_card(&conn, card, None, None).unwrap();
    assert_eq!(kinds(&conn), [("pr".into(), "r#1".into())]);
}

#[test]
fn relation_cycle_is_refused() {
    let conn = fresh();
    let (_, _, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let (a, b, c) = (&cards[0].id, &cards[1].id, &cards[2].id);
    add_relation(&conn, a, b, "blocks").unwrap();
    assert_eq!(
        add_relation(&conn, b, a, "blocks").unwrap_err(),
        "That would make a cycle"
    );
    add_relation(&conn, b, c, "blocks").unwrap();
    assert_eq!(
        add_relation(&conn, c, a, "blocks").unwrap_err(),
        "That would make a cycle"
    );
    assert!(add_relation(&conn, a, a, "relates").is_err());
    assert!(add_relation(&conn, a, b, "follows").is_err());
    // Relates is two-way, so the reverse is the same relation.
    add_relation(&conn, a, c, "relates").unwrap();
    add_relation(&conn, c, a, "relates").unwrap();
    let relations = card_relations(&conn, a).unwrap();
    assert_eq!(relations.len(), 2, "{relations:?}");
    // A card on another board can't be related.
    let (_, _, other) = board_with_cards(&conn, &["x"]);
    assert!(add_relation(&conn, a, &other[0].id, "relates").is_err());
    // Once the chain is broken the reverse edge is fine.
    remove_relation(&conn, b, c, "blocks").unwrap();
    add_relation(&conn, c, a, "blocks").unwrap();
}

type Change<'a> = Box<dyn Fn(&Connection) -> Result<(), String> + 'a>;

#[test]
fn each_field_change_writes_one_activity_row_and_noop_writes_none() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["a"]);
    let (board, card) = (board.id.as_str(), cards[0].id.as_str());
    let bug = create_label(&conn, board, "Bug", "e5534b").unwrap();
    let points = create_field(&conn, board, "Points", "number", None).unwrap();

    let mut changes: Vec<(&str, Change)> = vec![
        (
            "priority: High",
            Box::new(|c| set_priority(c, card, 3).map(drop)),
        ),
        ("due: ", Box::new(|c| set_due(c, card, Some(DUE)).map(drop))),
        (
            "start: ",
            Box::new(|c| set_start(c, card, Some(DUE)).map(drop)),
        ),
        (
            "estimate: 90m",
            Box::new(|c| set_estimate(c, card, Some(90)).map(drop)),
        ),
        (
            "projects: 2",
            Box::new(|c| set_projects(c, card, &["/b".into(), "/a".into()]).map(drop)),
        ),
        (
            "labels: Bug",
            Box::new(move |c| set_card_labels(c, card, std::slice::from_ref(&bug.id)).map(drop)),
        ),
        (
            "Points: 5",
            Box::new(move |c| set_field_value(c, card, &points.id, Some("5")).map(drop)),
        ),
    ];
    for (detail, change) in changes.drain(..) {
        let before = activity_count(&conn, board, card);
        change(&conn).unwrap();
        assert_eq!(activity_count(&conn, board, card), before + 1, "{detail}");
        let logged = last_detail(&conn, board, card).unwrap();
        assert!(logged.starts_with(detail), "{logged} vs {detail}");
        change(&conn).unwrap();
        assert_eq!(
            activity_count(&conn, board, card),
            before + 1,
            "{detail} no-op"
        );
    }

    let before = activity_count(&conn, board, card);
    let step = add_subtask(&conn, card, "Ship it").unwrap();
    assert_eq!(
        last_detail(&conn, board, card).as_deref(),
        Some("subtask added: Ship it")
    );
    toggle_subtask(&conn, &step.id, true).unwrap();
    toggle_subtask(&conn, &step.id, true).unwrap();
    assert_eq!(
        last_detail(&conn, board, card).as_deref(),
        Some("subtask done: Ship it")
    );
    let comment = add_comment(&conn, card, "Looks good", "user").unwrap();
    assert_eq!(
        last_detail(&conn, board, card).as_deref(),
        Some("comment added")
    );
    edit_comment(&conn, &comment.id, "Looks good").unwrap();
    edit_comment(&conn, &comment.id, "Looks great").unwrap();
    assert_eq!(
        last_detail(&conn, board, card).as_deref(),
        Some("comment edited")
    );
    delete_subtask(&conn, &step.id).unwrap();
    delete_subtask(&conn, &step.id).unwrap();
    assert_eq!(
        last_detail(&conn, board, card).as_deref(),
        Some("subtask deleted: Ship it")
    );
    assert_eq!(activity_count(&conn, board, card), before + 5);

    let edited = list_comments(&conn, card).unwrap().remove(0);
    assert_eq!(edited.body, "Looks great");
    assert!(edited.edited_at.is_some());
    assert!(add_comment(&conn, card, "hi", "robot").is_err());
    assert_eq!(
        add_comment(&conn, card, "done", "agent").unwrap().author,
        "agent"
    );
}

fn live_positions(conn: &Connection, board: &str, column: &str) -> Vec<(String, i64)> {
    get_board(conn, board)
        .unwrap()
        .cards
        .into_iter()
        .filter(|c| c.column_id == column)
        .map(|c| (c.title, c.position))
        .collect()
}

fn assert_contiguous(conn: &Connection, board: &str, columns: &[Column]) {
    for column in columns {
        let positions: Vec<i64> = live_positions(conn, board, &column.id)
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        let expected: Vec<i64> = (0..positions.len() as i64).collect();
        assert_eq!(positions, expected, "{} not contiguous", column.name);
    }
}

#[test]
fn positions_stay_contiguous_after_archive_and_bulk_move() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c", "d", "e"]);
    let board = board.id.as_str();
    create_card(&conn, &columns[1].id, "x", "").unwrap();

    archive_card(&conn, &cards[1].id).unwrap();
    assert_contiguous(&conn, board, &columns);
    let titles = |conn: &Connection, column: &str| -> Vec<String> {
        live_positions(conn, board, column)
            .into_iter()
            .map(|(t, _)| t)
            .collect()
    };
    assert_eq!(titles(&conn, &columns[0].id), ["a", "c", "d", "e"]);
    assert_eq!(archived_cards(&conn, board).unwrap()[0].title, "b");
    assert!(card_meta(&conn, &cards[1].id)
        .unwrap()
        .archived_at
        .is_some());
    // A new card goes after the live cards, not after the archived one.
    let f = create_card(&conn, &columns[0].id, "f", "").unwrap();
    assert_eq!(f.position, 4);

    let moving = vec![cards[4].id.clone(), cards[0].id.clone()];
    move_cards(&conn, &moving, &columns[1].id, 1, "user").unwrap();
    assert_contiguous(&conn, board, &columns);
    assert_eq!(titles(&conn, &columns[0].id), ["c", "d", "f"]);
    assert_eq!(titles(&conn, &columns[1].id), ["x", "e", "a"]);
    let moved = list_activity(&conn, board, None, 500)
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == "moved")
        .count();
    assert_eq!(moved, 2);
    // An archived card can't be moved, in bulk or alone.
    assert!(move_cards(&conn, &[cards[1].id.clone()], &columns[2].id, 0, "user").is_err());
    assert!(move_card(&conn, &cards[1].id, &columns[2].id, 0, "user").is_err());

    unarchive_card(&conn, &cards[1].id).unwrap();
    assert_contiguous(&conn, board, &columns);
    assert_eq!(titles(&conn, &columns[0].id), ["c", "d", "f", "b"]);
    assert!(archived_cards(&conn, board).unwrap().is_empty());
}

#[test]
fn new_board_settings_default_run_with_and_write_back_on() {
    let conn = fresh();
    let (board, _, _) = board_with_cards(&conn, &[]);
    let settings = board_settings(&conn, &board.id).unwrap();
    assert!(settings.run_with && settings.write_back && settings.ticket_status_changes);
    assert_eq!(settings.auto_move_json, None);

    // A board from before v3 has no settings row and reads as the defaults.
    conn.execute("DELETE FROM board_settings", []).unwrap();
    assert_eq!(
        board_settings(&conn, &board.id).unwrap(),
        BoardSettings::default()
    );

    let off = BoardSettings {
        run_with: false,
        write_back: true,
        ticket_status_changes: false,
        auto_move_json: Some("{\"started\":\"doing\"}".into()),
    };
    set_board_settings(&conn, &board.id, &off).unwrap();
    assert_eq!(board_settings(&conn, &board.id).unwrap(), off);
    let bad = BoardSettings {
        auto_move_json: Some("{".into()),
        ..BoardSettings::default()
    };
    assert!(set_board_settings(&conn, &board.id, &bad).is_err());
    assert!(board_settings(&conn, "missing").is_err());
}

#[test]
fn column_display_settings_round_trip_and_validate() {
    let conn = fresh();
    let (board, columns, _) = board_with_cards(&conn, &[]);
    set_column_display(&conn, &columns[1].id, Some("e5534b"), Some(3), true).unwrap();
    let column = get_board(&conn, &board.id).unwrap().columns.remove(1);
    assert_eq!(
        (column.color.as_deref(), column.wip_limit, column.collapsed),
        (Some("e5534b"), Some(3), true)
    );
    assert!(set_column_display(&conn, &columns[1].id, Some("red"), None, false).is_err());
    assert!(set_column_display(&conn, &columns[1].id, None, Some(0), false).is_err());
}

#[test]
fn field_values_are_checked_against_the_field_and_the_board() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["a"]);
    let size = create_field(&conn, &board.id, "Size", "select", Some("[\"S\",\"M\"]")).unwrap();
    assert!(create_field(&conn, &board.id, "Bad", "colour", None).is_err());
    assert!(create_field(&conn, &board.id, "Bad", "select", Some("[")).is_err());
    assert!(set_field_value(&conn, &cards[0].id, &size.id, Some("not json")).is_err());
    set_field_value(&conn, &cards[0].id, &size.id, Some("\"M\"")).unwrap();
    assert_eq!(
        last_detail(&conn, &board.id, &cards[0].id).as_deref(),
        Some("Size: M")
    );
    let values = field_values(&conn, &cards[0].id).unwrap();
    assert_eq!(values[0].value_json, "\"M\"");
    set_field_value(&conn, &cards[0].id, &size.id, None).unwrap();
    assert!(field_values(&conn, &cards[0].id).unwrap().is_empty());
    let (_, _, other) = board_with_cards(&conn, &["x"]);
    assert!(set_field_value(&conn, &other[0].id, &size.id, Some("1")).is_err());
    assert_eq!(list_fields(&conn, &board.id).unwrap().len(), 1);
}

fn open_file(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn
}

#[test]
fn two_connections_migrating_at_once_copy_each_row_once() {
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        {
            let conn = open_file(&path);
            conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
            write_v2_fixture(&conn);
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let conn = open_file(&path);
                    barrier.wait();
                    ensure_tables(&conn)
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        let conn = open_file(&path);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_schema"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_labels"), 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_labels"), 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_comments"), 2);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_links"), 1);
        let columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('board_columns') WHERE name = 'collapsed'",
                params![],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(columns, 1);
    }
}

#[test]
fn deleting_a_card_takes_every_v3_row_with_it() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["a", "b"]);
    let (board, card) = (board.id.as_str(), cards[0].id.as_str());
    set_priority(&conn, card, 2).unwrap();
    set_projects(&conn, card, &["/a".into()]).unwrap();
    let label = create_label(&conn, board, "Bug", "e5534b").unwrap();
    set_card_labels(&conn, card, &[label.id]).unwrap();
    add_subtask(&conn, card, "One").unwrap();
    add_comment(&conn, card, "Hi", "user").unwrap();
    add_link(&conn, card, "pr", "r#1", None).unwrap();
    add_relation(&conn, &cards[1].id, card, "blocks").unwrap();
    let field = create_field(&conn, board, "Points", "number", None).unwrap();
    set_field_value(&conn, card, &field.id, Some("3")).unwrap();
    // The Tauri app and older builds delete with a plain DELETE.
    conn.execute("DELETE FROM board_cards WHERE id = ?1", [card])
        .unwrap();
    for table in [
        "board_card_meta",
        "board_card_projects",
        "board_card_labels",
        "board_subtasks",
        "board_comments",
        "board_card_links",
        "board_card_field_values",
    ] {
        let left: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE card_id = ?1"),
                [card],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0, "{table}");
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_relations"), 0);
}

fn column_titles(conn: &Connection, board: &str, column: &str) -> Vec<String> {
    live_positions(conn, board, column)
        .into_iter()
        .map(|(title, _)| title)
        .collect()
}

#[test]
fn restore_card_puts_the_card_back_at_its_old_index() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c", "d"]);
    archive_card(&conn, &cards[1].id).unwrap();

    let returned = restore_card(&conn, &cards[1].id, &columns[0].id, 1).unwrap();
    assert_eq!(returned, board.id);
    assert_eq!(
        column_titles(&conn, &board.id, &columns[0].id),
        ["a", "b", "c", "d"]
    );
    assert_contiguous(&conn, &board.id, &columns);
    assert!(archived_cards(&conn, &board.id).unwrap().is_empty());

    // An index past the end is clamped to the end.
    archive_card(&conn, &cards[0].id).unwrap();
    restore_card(&conn, &cards[0].id, &columns[0].id, 99).unwrap();
    assert_eq!(
        column_titles(&conn, &board.id, &columns[0].id),
        ["b", "c", "d", "a"]
    );
    assert_contiguous(&conn, &board.id, &columns);
}

#[test]
fn restore_card_logs_unarchived_and_moves_only_when_the_column_differs() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b"]);
    let id = &cards[0].id;
    let kinds = |conn: &Connection| -> Vec<String> {
        list_activity(conn, &board.id, Some(id), 50)
            .unwrap()
            .into_iter()
            .map(|a| a.kind)
            .collect()
    };
    archive_card(&conn, id).unwrap();
    let before = kinds(&conn).len();
    restore_card(&conn, id, &columns[0].id, 0).unwrap();
    let after = kinds(&conn);
    assert_eq!(after.len(), before + 1);
    assert!(!after.contains(&"moved".to_string()));

    archive_card(&conn, id).unwrap();
    let before = kinds(&conn).len();
    restore_card(&conn, id, &columns[1].id, 0).unwrap();
    assert_eq!(kinds(&conn).len(), before + 2);
    assert!(kinds(&conn).contains(&"moved".to_string()));
}

#[test]
fn restore_card_into_another_column() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    create_card(&conn, &columns[1].id, "x", "").unwrap();
    create_card(&conn, &columns[1].id, "y", "").unwrap();
    archive_card(&conn, &cards[1].id).unwrap();

    restore_card(&conn, &cards[1].id, &columns[1].id, 1).unwrap();
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["a", "c"]);
    assert_eq!(
        column_titles(&conn, &board.id, &columns[1].id),
        ["x", "b", "y"]
    );
    assert_eq!(
        get_card(&conn, &cards[1].id).unwrap().column_id,
        columns[1].id
    );
    assert_contiguous(&conn, &board.id, &columns);
}

#[test]
fn restore_card_after_its_column_was_deleted_errors() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b"]);
    let spare = create_column(&conn, &board.id, "Spare", None).unwrap();
    archive_card(&conn, &cards[0].id).unwrap();
    delete_column(&conn, &spare.id).unwrap();

    let err = restore_card(&conn, &cards[0].id, &spare.id, 0).unwrap_err();
    assert_eq!(err, "Column not on this board");
    // Nothing changed: the card is still archived.
    assert_eq!(archived_cards(&conn, &board.id).unwrap().len(), 1);
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["b"]);

    assert_eq!(
        restore_card(&conn, "no-such-card", &columns[0].id, 0).unwrap_err(),
        "Card not found"
    );
    // A column from another board is also refused.
    let other = create_board(&conn, "Other").unwrap();
    let foreign = get_board(&conn, &other.id).unwrap().columns;
    assert_eq!(
        restore_card(&conn, &cards[0].id, &foreign[0].id, 0).unwrap_err(),
        "Column not on this board"
    );
}

#[test]
fn archive_cards_is_one_transaction_and_keeps_positions_contiguous() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c", "d", "e"]);
    let doing = create_card(&conn, &columns[1].id, "x", "").unwrap();
    let doing2 = create_card(&conn, &columns[1].id, "y", "").unwrap();
    archive_card(&conn, &cards[4].id).unwrap();
    let before_e = activity_count(&conn, &board.id, &cards[4].id);

    let ids = vec![
        cards[1].id.clone(),
        cards[3].id.clone(),
        doing.id.clone(),
        cards[4].id.clone(), // already archived: no change, no row
        cards[1].id.clone(), // duplicate
    ];
    assert_eq!(archive_cards(&conn, &ids).unwrap(), board.id);
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["a", "c"]);
    assert_eq!(column_titles(&conn, &board.id, &columns[1].id), ["y"]);
    assert_contiguous(&conn, &board.id, &columns);
    assert_eq!(archived_cards(&conn, &board.id).unwrap().len(), 4);
    assert_eq!(activity_count(&conn, &board.id, &cards[4].id), before_e);
    for id in [&cards[1].id, &cards[3].id, &doing.id] {
        let rows = list_activity(&conn, &board.id, Some(id), 50).unwrap();
        assert_eq!(
            rows.iter()
                .filter(|a| a.detail.as_deref() == Some("archived"))
                .count(),
            1
        );
    }

    // A failing id rolls the whole batch back.
    let bad = vec![cards[0].id.clone(), "no-such-card".to_string()];
    assert!(archive_cards(&conn, &bad).is_err());
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["a", "c"]);

    // Cards from two boards are refused, also without partial effects.
    let other = create_board(&conn, "Other").unwrap();
    let other_cols = get_board(&conn, &other.id).unwrap().columns;
    let foreign = create_card(&conn, &other_cols[0].id, "z", "").unwrap();
    let mixed = vec![cards[0].id.clone(), foreign.id.clone()];
    assert!(archive_cards(&conn, &mixed).is_err());
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["a", "c"]);
    assert_eq!(column_titles(&conn, &board.id, &columns[1].id), ["y"]);
    let _ = doing2;
}

#[test]
fn column_card_count_counts_archived_separately() {
    let conn = fresh();
    let (_board, columns, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    assert_eq!(column_card_count(&conn, &columns[0].id).unwrap(), (3, 0));
    archive_card(&conn, &cards[0].id).unwrap();
    archive_card(&conn, &cards[2].id).unwrap();
    assert_eq!(column_card_count(&conn, &columns[0].id).unwrap(), (1, 2));
    assert_eq!(column_card_count(&conn, &columns[1].id).unwrap(), (0, 0));
    assert!(column_card_count(&conn, "missing").is_err());
}

#[test]
fn delete_archived_card_deletes_an_archived_card() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    archive_card(&conn, &cards[1].id).unwrap();

    let returned = delete_archived_card(&conn, &cards[1].id).unwrap();
    assert_eq!(returned, Some(board.id.clone()));
    assert!(get_card(&conn, &cards[1].id).is_err());
    assert!(archived_cards(&conn, &board.id).unwrap().is_empty());
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["a", "c"]);
    assert_contiguous(&conn, &board.id, &columns);
    let tombstone = &list_activity(&conn, &board.id, None, 1).unwrap()[0];
    assert_eq!(tombstone.kind, "deleted");
    assert_eq!(tombstone.detail.as_deref(), Some("#2 b"));
}

#[test]
fn delete_archived_card_leaves_a_live_card_untouched() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b"]);
    // Archived, then restored elsewhere before the held delete ran.
    archive_card(&conn, &cards[0].id).unwrap();
    unarchive_card(&conn, &cards[0].id).unwrap();
    let activity = list_activity(&conn, &board.id, None, 500).unwrap().len();

    assert_eq!(delete_archived_card(&conn, &cards[0].id).unwrap(), None);
    assert_eq!(get_card(&conn, &cards[0].id).unwrap().title, "a");
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["b", "a"]);
    assert_eq!(
        list_activity(&conn, &board.id, None, 500).unwrap().len(),
        activity
    );
}

#[test]
fn delete_archived_card_of_a_missing_card_is_a_no_op() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["a"]);
    archive_card(&conn, &cards[0].id).unwrap();
    delete_card(&conn, &cards[0].id).unwrap();

    assert_eq!(delete_archived_card(&conn, &cards[0].id).unwrap(), None);
    assert_eq!(delete_archived_card(&conn, "missing").unwrap(), None);
    assert!(archived_cards(&conn, &board.id).unwrap().is_empty());
}

// ---- C3-1: subtask reorder and convert, duplicate, move to board ----

fn subtask_titles(conn: &Connection, card: &str) -> Vec<(String, i64)> {
    list_subtasks(conn, card)
        .unwrap()
        .into_iter()
        .map(|s| (s.title, s.position))
        .collect()
}

fn relation_count(conn: &Connection, card: &str) -> usize {
    card_relations(conn, card).unwrap().len()
}

#[test]
fn move_subtask_reorders_and_stays_contiguous() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["Parent"]);
    let card = &cards[0].id;
    let a = add_subtask(&conn, card, "A").unwrap();
    add_subtask(&conn, card, "B").unwrap();
    add_subtask(&conn, card, "C").unwrap();
    assert_eq!(move_subtask(&conn, &a.id, 2).unwrap(), board.id);
    assert_eq!(
        subtask_titles(&conn, card),
        vec![("B".into(), 0), ("C".into(), 1), ("A".into(), 2)]
    );
    assert_eq!(
        last_detail(&conn, &board.id, card).as_deref(),
        Some("subtask moved: A")
    );
    // Index past the end clamps to the end: no change, no activity.
    let before = activity_count(&conn, &board.id, card);
    move_subtask(&conn, &a.id, 99).unwrap();
    assert_eq!(activity_count(&conn, &board.id, card), before);
    // Move to the front.
    move_subtask(&conn, &a.id, 0).unwrap();
    assert_eq!(
        subtask_titles(&conn, card),
        vec![("A".into(), 0), ("B".into(), 1), ("C".into(), 2)]
    );
    assert_eq!(activity_count(&conn, &board.id, card), before + 1);
    assert!(move_subtask(&conn, "missing", 0).is_err());
}

#[test]
fn convert_subtask_creates_card_after_parent_relates_and_removes_subtask() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["P1", "Parent", "P3"]);
    let parent = &cards[1];
    add_subtask(&conn, &parent.id, "Keep").unwrap();
    let sub = add_subtask(&conn, &parent.id, "Ship it").unwrap();
    add_subtask(&conn, &parent.id, "Also keep").unwrap();
    let card = convert_subtask(&conn, &sub.id).unwrap();
    assert_eq!(card.title, "Ship it");
    assert_eq!(card.number, 4);
    assert_eq!(card.column_id, columns[0].id);
    assert_eq!(
        column_titles(&conn, &board.id, &columns[0].id),
        ["P1", "Parent", "Ship it", "P3"]
    );
    assert_contiguous(&conn, &board.id, &columns);
    assert_eq!(
        card_relations(&conn, &parent.id).unwrap(),
        vec![Relation {
            card_id: parent.id.clone(),
            related_card_id: card.id.clone(),
            kind: "relates".into()
        }]
    );
    assert_eq!(
        subtask_titles(&conn, &parent.id),
        vec![("Keep".into(), 0), ("Also keep".into(), 1)]
    );
    assert_eq!(
        last_detail(&conn, &board.id, &parent.id).as_deref(),
        Some("subtask converted: Ship it → #4")
    );
    // One row on the parent for the change (plus the earlier adds/created).
    assert_eq!(activity_count(&conn, &board.id, &parent.id), 1 + 3 + 1);
}

#[test]
fn convert_subtask_at_column_end() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["First", "Last"]);
    let sub = add_subtask(&conn, &cards[1].id, "Tail").unwrap();
    convert_subtask(&conn, &sub.id).unwrap();
    assert_eq!(
        column_titles(&conn, &board.id, &columns[0].id),
        ["First", "Last", "Tail"]
    );
    assert_contiguous(&conn, &board.id, &columns);
    // Parent at the start.
    let sub = add_subtask(&conn, &cards[0].id, "Head").unwrap();
    convert_subtask(&conn, &sub.id).unwrap();
    assert_eq!(
        column_titles(&conn, &board.id, &columns[0].id),
        ["First", "Head", "Last", "Tail"]
    );
    assert_contiguous(&conn, &board.id, &columns);
}

#[test]
fn convert_missing_subtask_errors() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["Parent"]);
    let sub = add_subtask(&conn, &cards[0].id, "Gone").unwrap();
    delete_subtask(&conn, &sub.id).unwrap();
    assert_eq!(
        convert_subtask(&conn, &sub.id).unwrap_err(),
        "Subtask not found"
    );
    // Nothing was written.
    assert_eq!(column_titles(&conn, &board.id, &columns[0].id), ["Parent"]);
    assert_eq!(relation_count(&conn, &cards[0].id), 0);
    // An archived parent is refused too, and keeps its subtask.
    let sub = add_subtask(&conn, &cards[0].id, "Stay").unwrap();
    archive_card(&conn, &cards[0].id).unwrap();
    assert_eq!(
        convert_subtask(&conn, &sub.id).unwrap_err(),
        "Card is archived"
    );
    assert_eq!(subtask_titles(&conn, &cards[0].id).len(), 1);
}

fn rich_card(conn: &Connection) -> (Board, Vec<Column>, Vec<Card>, Label, Field) {
    let (board, columns, cards) = board_with_cards(conn, &["Orig", "Next"]);
    let id = &cards[0].id;
    update_card(conn, id, "Orig", "the body").unwrap();
    let label = create_label(conn, &board.id, "Bug", "e5534b").unwrap();
    set_card_labels(conn, id, std::slice::from_ref(&label.id)).unwrap();
    set_projects(conn, id, &["/a".to_string(), "/b".to_string()]).unwrap();
    set_priority(conn, id, 3).unwrap();
    set_due(conn, id, Some(DUE)).unwrap();
    set_start(conn, id, Some(DUE - 86_400_000)).unwrap();
    set_estimate(conn, id, Some(90)).unwrap();
    let field = create_field(conn, &board.id, "Points", "number", None).unwrap();
    set_field_value(conn, id, &field.id, Some("5")).unwrap();
    let sub = add_subtask(conn, id, "Step one").unwrap();
    toggle_subtask(conn, &sub.id, true).unwrap();
    add_subtask(conn, id, "Step two").unwrap();
    add_link(conn, id, "pr", "https://example.com/pr/1", None).unwrap();
    add_comment(conn, id, "hello", "user").unwrap();
    add_relation(conn, id, &cards[1].id, "relates").unwrap();
    (board, columns, cards, label, field)
}

#[test]
fn duplicate_copies_meta_labels_projects_subtasks_fields_not_links() {
    let conn = fresh();
    let (board, _, cards, label, field) = rich_card(&conn);
    let copy = duplicate_card(&conn, &cards[0].id).unwrap();
    assert_eq!(copy.title, "Orig (copy)");
    assert_eq!(copy.body, "the body");
    assert_eq!(copy.board_id, board.id);
    assert_eq!(copy.number, 3);
    let meta = card_meta(&conn, &copy.id).unwrap();
    assert_eq!(meta.priority, 3);
    assert_eq!(meta.due_at, Some(DUE));
    assert_eq!(meta.start_at, Some(DUE - 86_400_000));
    assert_eq!(meta.estimate_minutes, Some(90));
    let extras = card_extras(&conn, &board.id).unwrap();
    let e = &extras[&copy.id];
    assert_eq!(e.labels, vec![label.id.clone()]);
    assert_eq!(e.projects, vec!["/a".to_string(), "/b".to_string()]);
    let values = field_values(&conn, &copy.id).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].field_id, field.id);
    assert_eq!(values[0].value_json, "5");
    // Not copied.
    assert!(card_links(&conn, &copy.id).unwrap().is_empty());
    assert!(list_comments(&conn, &copy.id).unwrap().is_empty());
    assert_eq!(relation_count(&conn, &copy.id), 0);
    assert!(copy.link_kind.is_none());
    // The original is untouched.
    assert_eq!(card_links(&conn, &cards[0].id).unwrap().len(), 1);
    assert_eq!(relation_count(&conn, &cards[0].id), 1);
    assert_eq!(subtask_titles(&conn, &cards[0].id).len(), 2);
}

#[test]
fn duplicate_resets_subtasks_and_places_after_original() {
    let conn = fresh();
    let (board, columns, cards, _, _) = rich_card(&conn);
    let copy = duplicate_card(&conn, &cards[0].id).unwrap();
    let subs = list_subtasks(&conn, &copy.id).unwrap();
    assert_eq!(
        subs.iter()
            .map(|s| (s.title.as_str(), s.done, s.position))
            .collect::<Vec<_>>(),
        vec![("Step one", false, 0), ("Step two", false, 1)]
    );
    // The original's first subtask is still done.
    assert!(list_subtasks(&conn, &cards[0].id).unwrap()[0].done);
    assert_eq!(
        column_titles(&conn, &board.id, &columns[0].id),
        ["Orig", "Orig (copy)", "Next"]
    );
    assert_contiguous(&conn, &board.id, &columns);
    let log = list_activity(&conn, &board.id, Some(&copy.id), 10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].kind, "created");
    assert_eq!(log[0].detail.as_deref(), Some("duplicated from #1"));
    // A card without a meta row duplicates cleanly, and a long title is trimmed to fit.
    let long = "x".repeat(500);
    let c = create_card(&conn, &columns[1].id, &long, "").unwrap();
    let copy = duplicate_card(&conn, &c.id).unwrap();
    assert_eq!(copy.title.chars().count(), 500);
    assert!(copy.title.ends_with(" (copy)"));
    assert_eq!(card_meta(&conn, &copy.id).unwrap(), CardMeta::default());
}

fn two_boards(conn: &Connection) -> (Board, Vec<Column>, Board, Vec<Column>) {
    let (a, ac, _) = board_with_cards(conn, &[]);
    let b = create_board(conn, "Other").unwrap();
    let bc = get_board(conn, &b.id).unwrap().columns;
    (a, ac, b, bc)
}

#[test]
fn move_to_board_maps_labels_case_insensitively_and_drops_unknown() {
    let conn = fresh();
    let (a, ac, b, bc) = two_boards(&conn);
    let card = create_card(&conn, &ac[0].id, "Mover", "").unwrap();
    let bug = create_label(&conn, &a.id, "Bug", "e5534b").unwrap();
    let only = create_label(&conn, &a.id, "OnlyHere", "000000").unwrap();
    set_card_labels(&conn, &card.id, &[bug.id.clone(), only.id]).unwrap();
    let target_bug = create_label(&conn, &b.id, "bUG", "4da3f5").unwrap();
    let points_a = create_field(&conn, &a.id, "Points", "number", None).unwrap();
    let notes_a = create_field(&conn, &a.id, "Notes", "text", None).unwrap();
    let lone_a = create_field(&conn, &a.id, "Lone", "text", None).unwrap();
    set_field_value(&conn, &card.id, &points_a.id, Some("3")).unwrap();
    set_field_value(&conn, &card.id, &notes_a.id, Some("\"hi\"")).unwrap();
    set_field_value(&conn, &card.id, &lone_a.id, Some("\"x\"")).unwrap();
    let points_b = create_field(&conn, &b.id, "points", "number", None).unwrap();
    // Same name, different kind: dropped.
    create_field(&conn, &b.id, "Notes", "checkbox", None).unwrap();
    let moved = move_card_to_board(&conn, &card.id, &b.id).unwrap();
    assert_eq!(moved.board_id, b.id);
    assert_eq!(moved.column_id, bc[0].id);
    let extras = card_extras(&conn, &b.id).unwrap();
    assert_eq!(extras[&card.id].labels, vec![target_bug.id]);
    let values = field_values(&conn, &card.id).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].field_id, points_b.id);
    assert_eq!(values[0].value_json, "3");
    // No leftovers pointing at the old board.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM board_card_labels cl JOIN board_labels l ON l.id = cl.label_id
             JOIN board_cards c ON c.id = cl.card_id WHERE l.board_id <> c.board_id"
        ),
        0
    );
}

#[test]
fn move_to_board_without_todo_column_uses_first_column() {
    let conn = fresh();
    let (_, ac, b, bc) = two_boards(&conn);
    update_column(&conn, &bc[0].id, "Backlog", None).unwrap();
    let cols = get_board(&conn, &b.id).unwrap().columns;
    assert!(cols.iter().all(|c| c.role.as_deref() != Some("todo")));
    let existing = create_card(&conn, &cols[0].id, "Already", "").unwrap();
    let card = create_card(&conn, &ac[0].id, "Mover", "").unwrap();
    let moved = move_card_to_board(&conn, &card.id, &b.id).unwrap();
    assert_eq!(moved.column_id, cols[0].id);
    assert_eq!(moved.position, 1);
    assert_eq!(get_card(&conn, &existing.id).unwrap().position, 0);
    assert_contiguous(&conn, &b.id, &cols);
}

#[test]
fn move_to_board_prefers_todo_over_an_earlier_column() {
    let conn = fresh();
    let (_, ac, b, bc) = two_boards(&conn);
    // Put the todo column second by moving the first column behind it.
    move_column(&conn, &bc[0].id, 1).unwrap();
    let card = create_card(&conn, &ac[0].id, "Mover", "").unwrap();
    let moved = move_card_to_board(&conn, &card.id, &b.id).unwrap();
    assert_eq!(moved.column_id, bc[0].id);
}

#[test]
fn move_to_board_gets_a_new_number_and_drops_relations() {
    let conn = fresh();
    let (a, ac, b, bc) = two_boards(&conn);
    let first = create_card(&conn, &ac[0].id, "One", "").unwrap();
    let mover = create_card(&conn, &ac[0].id, "Mover", "keep body").unwrap();
    let third = create_card(&conn, &ac[0].id, "Three", "").unwrap();
    create_card(&conn, &bc[0].id, "T1", "").unwrap();
    create_card(&conn, &bc[0].id, "T2", "").unwrap();
    add_relation(&conn, &mover.id, &first.id, "blocks").unwrap();
    add_relation(&conn, &third.id, &mover.id, "relates").unwrap();
    add_link(&conn, &mover.id, "pr", "https://example.com/pr/9", None).unwrap();
    add_comment(&conn, &mover.id, "note", "agent").unwrap();
    add_subtask(&conn, &mover.id, "sub").unwrap();
    set_priority(&conn, &mover.id, 2).unwrap();
    set_projects(&conn, &mover.id, &["/p".to_string()]).unwrap();
    let moved = move_card_to_board(&conn, &mover.id, &b.id).unwrap();
    assert_eq!(moved.number, 3);
    assert_eq!(moved.position, 2);
    assert_eq!(moved.body, "keep body");
    assert_eq!(relation_count(&conn, &mover.id), 0);
    assert_eq!(relation_count(&conn, &first.id), 0);
    assert_eq!(relation_count(&conn, &third.id), 0);
    assert_eq!(card_links(&conn, &mover.id).unwrap().len(), 1);
    assert_eq!(list_comments(&conn, &mover.id).unwrap().len(), 1);
    assert_eq!(list_subtasks(&conn, &mover.id).unwrap().len(), 1);
    assert_eq!(card_meta(&conn, &mover.id).unwrap().priority, 2);
    assert_eq!(
        card_extras(&conn, &b.id).unwrap()[&mover.id].projects,
        ["/p"]
    );
    // The source board's numbering carries on and its column closes the gap.
    assert_eq!(column_titles(&conn, &a.id, &ac[0].id), ["One", "Three"]);
    assert_contiguous(&conn, &a.id, &ac);
    assert_contiguous(&conn, &b.id, &bc);
    let next = create_card(&conn, &ac[0].id, "Four", "").unwrap();
    assert_eq!(next.number, 4);
    // Errors write nothing.
    assert_eq!(
        move_card_to_board(&conn, &mover.id, &b.id).unwrap_err(),
        "Card is already on this board"
    );
    assert_eq!(
        move_card_to_board(&conn, &first.id, "nope").unwrap_err(),
        "Board not found"
    );
    assert_eq!(get_card(&conn, &first.id).unwrap().board_id, a.id);
}

#[test]
fn move_to_board_writes_activity_on_both_boards() {
    let conn = fresh();
    let (a, ac, b, _) = two_boards(&conn);
    let card = create_card(&conn, &ac[0].id, "Mover", "").unwrap();
    move_card_to_board(&conn, &card.id, &b.id).unwrap();
    let src = list_activity(&conn, &a.id, Some(&card.id), 10).unwrap();
    let dst = list_activity(&conn, &b.id, Some(&card.id), 10).unwrap();
    assert_eq!(src.len(), 2); // created, then moved to board
    assert_eq!(src[0].detail.as_deref(), Some("moved to board Other"));
    assert_eq!(dst.len(), 1);
    assert_eq!(dst[0].detail.as_deref(), Some("moved from board Work"));
}

#[test]
fn duplicate_of_an_archived_card_is_refused() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["Gone"]);
    archive_card(&conn, &cards[0].id).unwrap();
    assert_eq!(
        duplicate_card(&conn, &cards[0].id).unwrap_err(),
        "Card is archived"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM board_cards"),
        1,
        "no copy was created"
    );
    assert!(column_titles(&conn, &board.id, &columns[0].id).is_empty());
    // The number was not consumed either.
    let next = create_card(&conn, &columns[0].id, "Next", "").unwrap();
    assert_eq!(next.number, 2);
}

#[test]
fn move_to_board_of_an_archived_card_is_refused_and_changes_nothing() {
    let conn = fresh();
    let (a, ac, b, _) = two_boards(&conn);
    let card = create_card(&conn, &ac[0].id, "Archived", "").unwrap();
    let other = create_card(&conn, &ac[0].id, "Other", "").unwrap();
    let label = create_label(&conn, &a.id, "Bug", "e5534b").unwrap();
    create_label(&conn, &b.id, "Bug", "4da3f5").unwrap();
    set_card_labels(&conn, &card.id, std::slice::from_ref(&label.id)).unwrap();
    add_relation(&conn, &card.id, &other.id, "relates").unwrap();
    archive_card(&conn, &card.id).unwrap();
    assert_eq!(
        move_card_to_board(&conn, &card.id, &b.id).unwrap_err(),
        "Card is archived"
    );
    assert_eq!(get_card(&conn, &card.id).unwrap().board_id, a.id);
    assert_eq!(get_card(&conn, &card.id).unwrap().number, card.number);
    assert_eq!(relation_count(&conn, &card.id), 1);
    assert_eq!(
        count(
            &conn,
            &format!(
                "SELECT COUNT(*) FROM board_card_labels WHERE card_id = '{}' AND label_id = '{}'",
                card.id, label.id
            )
        ),
        1
    );
    // The target board's numbering was not touched.
    assert_eq!(
        count(
            &conn,
            &format!("SELECT card_seq FROM boards WHERE id = '{}'", b.id)
        ),
        0
    );
}
