//! The 1,000-card fixture and the read budgets it guards: every board-wide
//! read runs the same number of SQL statements at 10 and at 1,000 cards (no
//! per-card queries). The time budget is in `scale_timing.rs`.
use gespan_board_store::seed::{seed_board, SeedReport, SeedSpec};
use gespan_board_store::*;
use rusqlite::trace::{TraceEvent, TraceEventCodes};
use rusqlite::Connection;
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};

#[allow(dead_code)]
#[path = "../examples/seed_board.rs"]
mod seed_example;

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
}

/// Counts each statement as it starts. Trigger subprograms report as `-- …`
/// and are part of the statement that fired them, so they don't count.
fn count_statement(event: TraceEvent<'_>) {
    if let TraceEvent::Stmt(_, sql) = event {
        if !sql.starts_with("--") {
            STATEMENTS.with(|count| count.set(count.get() + 1));
        }
    }
}

fn fresh() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    conn.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, Some(count_statement));
    conn
}

/// How many SQL statements `read` ran on this thread.
fn statements<T>(read: impl FnOnce() -> T) -> (usize, T) {
    STATEMENTS.with(|count| count.set(0));
    let value = read();
    (STATEMENTS.with(Cell::get), value)
}

fn seeded(conn: &Connection, cards: usize) -> SeedReport {
    let board = create_board(conn, "Scale").unwrap();
    let spec = SeedSpec {
        cards,
        ..SeedSpec::default()
    };
    seed_board(conn, &board.id, &spec).unwrap()
}

fn count(conn: &Connection, sql: &str, board_id: &str) -> i64 {
    conn.query_row(sql, [board_id], |row| row.get(0)).unwrap()
}

fn row(field: &str, values: &[&str]) -> FilterRow {
    FilterRow {
        field: field.to_string(),
        op: Op::Is,
        values: values.iter().map(|v| v.to_string()).collect(),
    }
}

/// The 3-row filter and sort of the time budget.
fn budget_spec(report: &SeedReport) -> ViewSpec {
    ViewSpec {
        query: String::new(),
        rows: vec![
            row("labels", &[&report.label_ids[0], &report.label_ids[1]]),
            row("priority", &["2", "3", "4"]),
            row("linked", &["failed"]),
        ],
        sort: SortSpec {
            key: SortKey::Priority,
            dir: Dir::Desc,
        },
    }
}

/// Everything a seeded board holds, by names and positions rather than ids,
/// so two seeds can be compared.
fn fingerprint(conn: &Connection, board_id: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(
            "SELECT c.title || '|' || col.name || '|' || c.position || '|' || c.number
               || '|' || COALESCE(m.priority, 0) || '|' || COALESCE(m.due_at, '-')
               || '|' || COALESCE((SELECT group_concat(name) FROM
                    (SELECT l.name FROM board_card_labels cl
                     JOIN board_labels l ON l.id = cl.label_id
                     WHERE cl.card_id = c.id ORDER BY l.position)), '')
               || '|' || COALESCE((SELECT group_concat(pair) FROM
                    (SELECT f.name || '=' || v.value_json AS pair
                     FROM board_card_field_values v JOIN board_fields f ON f.id = v.field_id
                     WHERE v.card_id = c.id ORDER BY f.position)), '')
               || '|' || COALESCE((SELECT group_concat(link) FROM
                    (SELECT kind || ':' || ref AS link FROM board_card_links
                     WHERE card_id = c.id ORDER BY kind, ref)), '')
               || '|' || COALESCE((SELECT group_concat(kind || '@' || created_at) FROM
                    (SELECT kind, created_at FROM board_activity WHERE card_id = c.id
                     ORDER BY created_at, rowid)), '')
               || '|' || COALESCE((SELECT group_concat(title || ':' || done) FROM
                    (SELECT title, done FROM board_subtasks WHERE card_id = c.id
                     ORDER BY position)), '')
             FROM board_cards c JOIN board_columns col ON col.id = c.column_id
             LEFT JOIN board_card_meta m ON m.card_id = c.id
             WHERE c.board_id = ?1 ORDER BY c.number",
        )
        .unwrap();
    let rows = statement
        .query_map([board_id], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

#[test]
fn seed_is_deterministic_and_fills_every_part() {
    let conn = fresh();
    let report = seeded(&conn, 1000);
    let board = get_board(&conn, &report.board_id).unwrap();

    // 1,000 live cards, titled and numbered 1..=1000, round-robin over the
    // four columns, each column holding positions 0..n-1.
    assert_eq!(report.card_ids.len(), 1000);
    assert_eq!(board.cards.len(), 1000);
    assert_eq!(
        report.column_ids,
        board
            .columns
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    );
    let by_id: HashMap<&str, &Card> = board.cards.iter().map(|c| (c.id.as_str(), c)).collect();
    for (index, card_id) in report.card_ids.iter().enumerate() {
        let n = index + 1;
        let card = by_id[card_id.as_str()];
        assert_eq!(card.title, format!("Card {n}"));
        assert_eq!(card.number, n as i64);
        assert_eq!(card.column_id, report.column_ids[index % 4]);
    }
    for column in &board.columns {
        let mut positions: Vec<i64> = board
            .cards
            .iter()
            .filter(|c| c.column_id == column.id)
            .map(|c| c.position)
            .collect();
        positions.sort_unstable();
        assert_eq!(positions, (0..250).collect::<Vec<i64>>(), "{}", column.name);
    }
    // A new card gets the next number, not a duplicate.
    let next = create_card(&conn, &report.column_ids[0], "After", "").unwrap();
    assert_eq!(next.number, 1001);
    assert_eq!(next.position, 250);
    delete_card(&conn, &next.id).unwrap();

    // Priority n % 5, a due date on every 7th card, label n % 8.
    let extras = card_extras(&conn, &report.board_id).unwrap();
    let labels = list_labels(&conn, &report.board_id).unwrap();
    assert_eq!(labels.len(), 8);
    assert_eq!(
        labels.iter().map(|l| l.id.clone()).collect::<Vec<_>>(),
        report.label_ids
    );
    for (index, card_id) in report.card_ids.iter().enumerate() {
        let n = index + 1;
        let extra = &extras[card_id];
        assert_eq!(extra.priority as usize, n % 5, "card {n}");
        assert_eq!(extra.due_at.is_some(), n % 7 == 0, "card {n}");
        assert_eq!(
            extra.labels,
            vec![report.label_ids[n % 8].clone()],
            "card {n}"
        );
        assert_eq!(extra.subtasks_total, 2, "card {n}");
    }
    let done: usize = extras.values().map(|e| e.subtasks_done).sum();
    assert_eq!(done, 1000, "one of each card's two subtasks is done");

    // Field 0 is a select with options A B C D, field 1 is text; every card
    // holds a value for both.
    let fields = list_fields(&conn, &report.board_id).unwrap();
    assert_eq!(
        fields.iter().map(|f| f.id.clone()).collect::<Vec<_>>(),
        report.field_ids
    );
    assert_eq!(fields[0].kind, "select");
    assert_eq!(
        fields[0].options_json.as_deref(),
        Some(r#"["A","B","C","D"]"#)
    );
    assert_eq!(fields[1].kind, "text");
    let index = card_index(&conn, &report.board_id).unwrap();
    assert_eq!(index.len(), 1000);
    let card_7 = index
        .iter()
        .find(|r| r.card_id == report.card_ids[6])
        .unwrap();
    assert_eq!(card_7.fields[&report.field_ids[0]], r#""D""#);
    assert_eq!(card_7.fields[&report.field_ids[1]], r#""Note 7""#);
    assert!(index.iter().all(|r| r.fields.len() == 2));

    // Every 10th card has a session link seed-{n}.
    let links = board_links(&conn, &report.board_id).unwrap();
    assert_eq!(links.len(), 100);
    assert!(links.iter().any(|l| l.card_id == report.card_ids[9]
        && l.kind == "session"
        && l.reference == "seed-10"));
    assert!(links.iter().all(|l| l.kind == "session"));

    // Three run events per card, cycling started / completed / failed, so a
    // third of the cards last failed.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM board_activity WHERE board_id = ?1
             AND kind IN ('started', 'completed', 'failed')",
            &report.board_id
        ),
        3000
    );
    let runs = latest_run_events(&conn, &report.board_id).unwrap();
    assert_eq!(runs.len(), 1000);
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for kind in runs.values() {
        *kinds.entry(kind.as_str()).or_default() += 1;
    }
    assert_eq!(kinds.len(), 3);
    assert!(
        kinds.values().all(|&n| (333..=334).contains(&n)),
        "{kinds:?}"
    );

    // Deterministic: a second seed elsewhere holds exactly the same data.
    let other = fresh();
    let again = seeded(&other, 1000);
    assert_eq!(
        fingerprint(&conn, &report.board_id),
        fingerprint(&other, &again.board_id)
    );
    assert_eq!(fingerprint(&conn, &report.board_id).len(), 1000);
}

#[test]
fn seed_rejects_an_unknown_board_and_writes_nothing() {
    let conn = fresh();
    assert_eq!(
        seed_board(&conn, "missing", &SeedSpec::default()).unwrap_err(),
        "Board not found"
    );
    assert!(seed_board(&conn, "bad id", &SeedSpec::default()).is_err());
    let cards: i64 = conn
        .query_row("SELECT COUNT(*) FROM board_cards", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cards, 0);
}

#[test]
fn board_wide_reads_use_the_same_statements_at_10_and_1000_cards() {
    let conn = fresh();
    let small = seeded(&conn, 10);
    let large = seeded(&conn, 1000);
    for report in [&small, &large] {
        // Two open blockers, so `blocked_cards` has rows to read.
        add_relation(&conn, &report.card_ids[0], &report.card_ids[1], "blocks").unwrap();
        add_relation(&conn, &report.card_ids[2], &report.card_ids[3], "blocks").unwrap();
    }
    let now = 1_793_404_800_000;
    let day = |ms: i64| ms.div_euclid(86_400_000);
    let running = HashSet::new();
    type Read<'a> = Box<dyn Fn(&SeedReport) -> usize + 'a>;
    let reads: Vec<(&str, Read<'_>)> = vec![
        (
            "query_cards",
            Box::new(|r| {
                query_cards(&conn, &r.board_id, &budget_spec(r), now, &day, &running)
                    .unwrap()
                    .len()
            }),
        ),
        (
            "card_index",
            Box::new(|r| card_index(&conn, &r.board_id).unwrap().len()),
        ),
        (
            "card_extras",
            Box::new(|r| card_extras(&conn, &r.board_id).unwrap().len()),
        ),
        (
            "get_board",
            Box::new(|r| get_board(&conn, &r.board_id).unwrap().cards.len()),
        ),
        (
            "board_links",
            Box::new(|r| board_links(&conn, &r.board_id).unwrap().len()),
        ),
        (
            "blocked_cards",
            Box::new(|r| blocked_cards(&conn, &r.board_id).unwrap().len()),
        ),
        (
            "latest_run_events",
            Box::new(|r| latest_run_events(&conn, &r.board_id).unwrap().len()),
        ),
        (
            "column_card_counts",
            Box::new(|r| column_card_counts(&conn, &r.board_id).unwrap().len()),
        ),
    ];
    for (name, read) in &reads {
        let (at_10, small_rows) = statements(|| read(&small));
        let (at_1000, large_rows) = statements(|| read(&large));
        assert!(at_10 > 0, "{name} ran no statement");
        assert_eq!(
            at_10, at_1000,
            "{name}: {at_10} statements at 10 cards, {at_1000} at 1,000"
        );
        assert!(small_rows > 0 && large_rows > 0, "{name} read nothing");
    }
    // Sanity: the large board really is 100x the small one.
    assert_eq!(card_index(&conn, &large.board_id).unwrap().len(), 1000);
    assert_eq!(card_index(&conn, &small.board_id).unwrap().len(), 10);
}

#[test]
fn column_card_counts_equal_per_column_counts() {
    let conn = fresh();
    let report = seeded(&conn, 1000);
    // Archive a few cards in two columns, and add an empty column.
    archive_cards(
        &conn,
        &[
            report.card_ids[0].clone(),
            report.card_ids[4].clone(),
            report.card_ids[1].clone(),
        ],
    )
    .unwrap();
    let empty = create_column(&conn, &report.board_id, "Empty", None).unwrap();
    // Another board's cards never count.
    seeded(&conn, 10);

    let (ran, counts) = statements(|| column_card_counts(&conn, &report.board_id).unwrap());
    assert_eq!(ran, 1, "column_card_counts runs one statement");
    let columns = get_board(&conn, &report.board_id).unwrap().columns;
    assert_eq!(counts.len(), columns.len());
    for column in &columns {
        assert_eq!(
            counts[&column.id],
            column_card_count(&conn, &column.id).unwrap(),
            "{}",
            column.name
        );
    }
    assert_eq!(counts[&report.column_ids[0]], (248, 2));
    assert_eq!(counts[&report.column_ids[1]], (249, 1));
    assert_eq!(counts[&report.column_ids[2]], (250, 0));
    assert_eq!(counts[&empty.id], (0, 0));

    let bare = create_board(&conn, "Bare").unwrap();
    for column in get_board(&conn, &bare.id).unwrap().columns {
        delete_column(&conn, &column.id).unwrap();
    }
    assert!(column_card_counts(&conn, &bare.id).unwrap().is_empty());
    assert_eq!(
        column_card_counts(&conn, "missing").unwrap_err(),
        "Board not found"
    );
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn seed_example_creates_a_new_board_and_leaves_others() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let path_text = path.to_str().unwrap();
    let existing = {
        let conn = Connection::open(&path).unwrap();
        ensure_tables(&conn).unwrap();
        let board = create_board(&conn, "Mine").unwrap();
        let column = get_board(&conn, &board.id).unwrap().columns[0].id.clone();
        create_card(&conn, &column, "Keep me", "").unwrap();
        board
    };
    let before = {
        let conn = Connection::open(&path).unwrap();
        (
            fingerprint(&conn, &existing.id),
            get_board(&conn, &existing.id).unwrap().board.updated_at,
        )
    };

    let first = seed_example::run(args(&["--db", path_text, "--cards", "25"])).unwrap();
    let second =
        seed_example::run(args(&["--db", path_text, "--name", "Load", "--cards", "3"])).unwrap();
    assert_ne!(first, second);

    let conn = Connection::open(&path).unwrap();
    let boards = list_boards(&conn).unwrap();
    let names: Vec<&str> = boards.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Mine", "Scale test (25 cards)", "Load"]);
    assert_eq!(boards[1].id, first);
    assert_eq!(get_board(&conn, &first).unwrap().cards.len(), 25);
    assert_eq!(get_board(&conn, &first).unwrap().columns.len(), 4);
    assert_eq!(get_board(&conn, &second).unwrap().cards.len(), 3);
    assert_eq!(list_labels(&conn, &first).unwrap().len(), 8);
    // The board that was already there is untouched.
    assert_eq!(
        (
            fingerprint(&conn, &existing.id),
            get_board(&conn, &existing.id).unwrap().board.updated_at,
        ),
        before
    );
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");

    // The default is 1,000 cards.
    let default = seed_example::run(args(&["--db", path_text])).unwrap();
    assert_eq!(get_board(&conn, &default).unwrap().cards.len(), 1000);
    assert_eq!(
        get_board(&conn, &default).unwrap().board.name,
        "Scale test (1000 cards)"
    );
    // Bad arguments are refused before anything is written.
    assert!(seed_example::run(args(&["--db", path_text, "--cards", "many"])).is_err());
    assert!(seed_example::run(args(&["--cards", "5"])).is_err());
    assert_eq!(list_boards(&conn).unwrap().len(), 4);
}

#[test]
fn seed_example_refuses_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let error = seed_example::run(args(&["--db", path.to_str().unwrap()])).unwrap_err();
    assert_eq!(
        error,
        format!("No database at {}. Open Gespan once first.", path.display())
    );
    assert!(!path.exists(), "the example never creates a database");
}

fn existing_board(path: &std::path::Path) -> Board {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    let board = create_board(&conn, "Mine").unwrap();
    let column = get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    create_card(&conn, &column, "Keep me", "").unwrap();
    board
}

#[test]
fn seed_example_refuses_a_file_that_is_not_a_gespan_database() {
    let dir = tempfile::tempdir().unwrap();
    // A real SQLite database, but someone else's.
    let plain = dir.path().join("notes.db");
    {
        let conn = Connection::open(&plain).unwrap();
        conn.execute_batch("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('hi');")
            .unwrap();
    }
    // And a file that isn't SQLite at all.
    let text = dir.path().join("readme.txt");
    std::fs::write(
        &text,
        "not a database, just text that is long enough to matter\n",
    )
    .unwrap();
    for path in [&plain, &text] {
        let before = std::fs::read(path).unwrap();
        let error = seed_example::run(args(&["--db", path.to_str().unwrap()])).unwrap_err();
        assert!(
            error.starts_with(&format!("{} is not a Gespan database", path.display())),
            "{error}"
        );
        assert_eq!(
            std::fs::read(path).unwrap(),
            before,
            "{} changed",
            path.display()
        );
        let wal = format!("{}-wal", path.display());
        assert!(!std::path::Path::new(&wal).exists(), "{wal} was created");
    }
    let tables: Vec<String> = Connection::open(&plain)
        .unwrap()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(tables, ["notes"]);
}

#[test]
fn seed_example_removes_only_its_board_when_the_seed_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let mine = existing_board(&path);
    let before = fingerprint(&Connection::open(&path).unwrap(), &mine.id);
    let mut seen = None;
    let error = seed_example::run_with(args(&["--db", path.to_str().unwrap()]), |conn, id, _| {
        // The board exists when the seed runs.
        assert!(get_board(conn, id).is_ok());
        seen = Some(id.to_string());
        Err("Seed failed".into())
    })
    .unwrap_err();
    assert_eq!(error, "Seed failed");
    let conn = Connection::open(&path).unwrap();
    let boards = list_boards(&conn).unwrap();
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0].id, mine.id);
    assert!(get_board(&conn, &seen.unwrap()).is_err());
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM board_columns WHERE board_id NOT IN (SELECT id FROM boards)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "the removed board's columns went with it");
    assert_eq!(fingerprint(&conn, &mine.id), before);
}

#[test]
fn seed_example_reports_a_failed_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    existing_board(&path);
    let mut seen = String::new();
    let error = seed_example::run_with(args(&["--db", path.to_str().unwrap()]), |conn, id, _| {
        // Block the cleanup on this connection only.
        conn.execute_batch(
            "CREATE TEMP TRIGGER keep_boards BEFORE DELETE ON boards
             BEGIN SELECT RAISE(ABORT, 'boards are kept'); END;",
        )
        .unwrap();
        seen = id.to_string();
        Err("Seed failed".into())
    })
    .unwrap_err();
    assert_eq!(
        error,
        format!("Seed failed. The new board {seen} could not be removed: boards are kept")
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(list_boards(&conn).unwrap().len(), 2);
}
