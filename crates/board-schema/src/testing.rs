//! Helpers for the app crates' tests (feature `testing`): a Native v2
//! database seed with its exact expected v3 result, and normalized dumps
//! for the committed cross-app fixtures.
use crate::{BASE_DDL, BOARD_TABLES};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;

/// Native's pre-v3 extras tables, exactly as `gespan-core` created them.
pub const NATIVE_V2_DDL: &str = "CREATE TABLE IF NOT EXISTS board_native_meta (
  card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE,
  priority INTEGER NOT NULL DEFAULT 0,
  due_at INTEGER
);
CREATE TABLE IF NOT EXISTS board_native_projects (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  project TEXT NOT NULL,
  PRIMARY KEY (card_id, project)
);
CREATE TABLE IF NOT EXISTS board_native_labels (
  id TEXT PRIMARY KEY,
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  color TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS board_native_labels_name_idx
  ON board_native_labels (board_id, lower(name));
CREATE TABLE IF NOT EXISTS board_native_card_labels (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  label_id TEXT NOT NULL REFERENCES board_native_labels(id) ON DELETE CASCADE,
  PRIMARY KEY (card_id, label_id)
);
CREATE TABLE IF NOT EXISTS board_native_subtasks (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS board_native_subtasks_card_idx
  ON board_native_subtasks (card_id, position);
CREATE TABLE IF NOT EXISTS board_native_comments (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  body TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS board_native_comments_card_idx
  ON board_native_comments (card_id, created_at);";

/// Native v2 tables `seed_native_v2` fills; `ensure` must leave them as is.
pub const NATIVE_V2_TABLES: [&str; 6] = [
    "board_native_meta",
    "board_native_projects",
    "board_native_labels",
    "board_native_card_labels",
    "board_native_subtasks",
    "board_native_comments",
];

/// Due date seeded on card A, Unix ms.
pub const SEED_DUE_AT: i64 = 1_792_929_600_000;

#[derive(Clone, Debug)]
pub struct NativeV2Seed {
    pub board_id: String,
    /// Cards A, B and C, in Backlog order.
    pub card_ids: Vec<String>,
    /// Labels `Bug` and `docs`.
    pub label_ids: Vec<String>,
    /// Row count of each `NATIVE_V2_TABLES` entry right after seeding.
    pub native_counts: Vec<i64>,
}

/// A database as a Native build before v3 left it: the v1 tables plus
/// `board_native_*`, with gapped subtask positions and orphan rows whose
/// card or label is gone. No v3 table exists yet.
pub fn seed_native_v2(conn: &Connection) -> NativeV2Seed {
    conn.execute_batch(BASE_DDL).unwrap();
    conn.execute_batch(NATIVE_V2_DDL).unwrap();
    conn.execute_batch(
        "INSERT INTO boards (id, name, card_seq, created_at, updated_at)
           VALUES ('board-1', 'Seeded', 3, 100, 100);
         INSERT INTO board_columns (id, board_id, name, position, role) VALUES
           ('col-todo', 'board-1', 'Backlog', 0, 'todo'),
           ('col-doing', 'board-1', 'In Progress', 1, 'doing'),
           ('col-review', 'board-1', 'Review', 2, 'review'),
           ('col-done', 'board-1', 'Done', 3, 'done');
         INSERT INTO board_cards (id, board_id, column_id, number, title, body, position,
             link_kind, link_id, created_at, updated_at) VALUES
           ('card-a', 'board-1', 'col-todo', 1, 'A', '', 0, 'session', 's-1', 100, 150),
           ('card-b', 'board-1', 'col-todo', 2, 'B', '', 1, NULL, NULL, 100, 100),
           ('card-c', 'board-1', 'col-todo', 3, 'C', '', 2, NULL, NULL, 100, 100);
         INSERT INTO board_native_meta (card_id, priority, due_at) VALUES
           ('card-a', 4, 1792929600000),
           ('card-b', 2, NULL);
         INSERT INTO board_native_projects (card_id, project) VALUES
           ('card-a', '/work/gespan'),
           ('card-a', '/work/site');
         -- docs goes in first so rowid order and name order disagree.
         INSERT INTO board_native_labels (id, board_id, name, color) VALUES
           ('label-docs', 'board-1', 'docs', '0075ca'),
           ('label-bug', 'board-1', 'Bug', 'd73a4a');
         INSERT INTO board_native_card_labels (card_id, label_id) VALUES
           ('card-a', 'label-bug'),
           ('card-a', 'label-docs'),
           ('card-b', 'label-bug');
         -- Gapped positions, inserted out of order.
         INSERT INTO board_native_subtasks (id, card_id, title, done, position) VALUES
           ('sub-3', 'card-a', 'Three', 0, 9),
           ('sub-1', 'card-a', 'One', 1, 0),
           ('sub-2', 'card-a', 'Two', 0, 5);
         INSERT INTO board_native_comments (id, card_id, body, created_at) VALUES
           ('comment-1', 'card-a', 'First', 1000),
           ('comment-2', 'card-a', 'Second', 2000);
         PRAGMA foreign_keys = OFF;
         INSERT INTO board_native_subtasks (id, card_id, title, done, position)
           VALUES ('sub-ghost', 'ghost', 'Lost', 0, 0);
         INSERT INTO board_native_comments (id, card_id, body, created_at)
           VALUES ('comment-ghost', 'ghost', 'Lost', 1);
         INSERT INTO board_native_meta (card_id, priority, due_at) VALUES ('ghost', 1, 5);
         INSERT INTO board_native_projects (card_id, project) VALUES ('ghost', '/lost');
         INSERT INTO board_native_card_labels (card_id, label_id) VALUES ('card-a', 'ghost-label');
         PRAGMA foreign_keys = ON;",
    )
    .unwrap();
    NativeV2Seed {
        board_id: "board-1".into(),
        card_ids: vec!["card-a".into(), "card-b".into(), "card-c".into()],
        label_ids: vec!["label-bug".into(), "label-docs".into()],
        native_counts: native_counts(conn),
    }
}

fn native_counts(conn: &Connection) -> Vec<i64> {
    NATIVE_V2_TABLES
        .iter()
        .map(|table| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
}

fn rows(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Asserts `ensure` turned `seed` into exactly the expected v3 rows.
pub fn assert_native_v2_migrated(conn: &Connection, seed: &NativeV2Seed) {
    let [a, b, _c] = [&seed.card_ids[0], &seed.card_ids[1], &seed.card_ids[2]];
    assert_eq!(
        rows(
            conn,
            "SELECT card_id || ':' || priority || ':' || IFNULL(due_at, '-') || ':'
               || IFNULL(start_at, '-') || IFNULL(estimate_minutes, '-')
               || IFNULL(archived_at, '-') || IFNULL(completed_at, '-')
             FROM board_card_meta ORDER BY card_id"
        ),
        [
            format!("{a}:4:{SEED_DUE_AT}:----"),
            format!("{b}:2:-:----"),
        ],
        "board_card_meta"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT card_id || ':' || project FROM board_card_projects ORDER BY card_id, project"
        ),
        [format!("{a}:/work/gespan"), format!("{a}:/work/site")],
        "board_card_projects"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT id || ':' || board_id || ':' || name || ':' || color || ':' || position
             FROM board_labels ORDER BY position"
        ),
        [
            format!("{}:{}:Bug:d73a4a:0", seed.label_ids[0], seed.board_id),
            format!("{}:{}:docs:0075ca:1", seed.label_ids[1], seed.board_id),
        ],
        "board_labels"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT card_id || ':' || label_id FROM board_card_labels ORDER BY card_id, label_id"
        ),
        [
            format!("{a}:{}", seed.label_ids[0]),
            format!("{a}:{}", seed.label_ids[1]),
            format!("{b}:{}", seed.label_ids[0]),
        ],
        "board_card_labels"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT card_id || ':' || title || ':' || done || ':' || position || ':'
               || IFNULL(converted_card_id, '-')
             FROM board_subtasks ORDER BY card_id, position"
        ),
        [
            format!("{a}:One:1:0:-"),
            format!("{a}:Two:0:1:-"),
            format!("{a}:Three:0:2:-"),
        ],
        "board_subtasks"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT card_id || ':' || body || ':' || author || ':' || created_at || ':'
               || IFNULL(edited_at, '-')
             FROM board_comments ORDER BY created_at"
        ),
        [
            format!("{a}:First:user:1000:-"),
            format!("{a}:Second:user:2000:-"),
        ],
        "board_comments"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT card_id || ':' || kind || ':' || ref || ':' || role FROM board_card_links"
        ),
        [format!("{a}:session:s-1:primary")],
        "board_card_links"
    );
    assert_eq!(
        rows(
            conn,
            "SELECT column_id || ':' || group_concat(position, ',')
             FROM (SELECT column_id, position FROM board_cards ORDER BY column_id, position)
             GROUP BY column_id"
        ),
        ["col-todo:0,1,2"],
        "card positions"
    );
    let ghosts: i64 = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM board_card_meta WHERE card_id = 'ghost')
                  + (SELECT COUNT(*) FROM board_card_projects WHERE card_id = 'ghost')
                  + (SELECT COUNT(*) FROM board_subtasks WHERE card_id = 'ghost')
                  + (SELECT COUNT(*) FROM board_comments WHERE card_id = 'ghost')
                  + (SELECT COUNT(*) FROM board_card_labels WHERE label_id = 'ghost-label')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ghosts, 0, "orphan rows were copied");
    assert_eq!(native_counts(conn), seed.native_counts, "board_native_* changed");
}

/// Set to `1` to rewrite the committed fixtures instead of comparing them.
pub const BLESS_VAR: &str = "GESPAN_BLESS_BOARD_FIXTURES";

/// Every row of every shared board table as `INSERT` statements, in
/// `BOARD_TABLES` order and then rowid order, normalized so two runs of the
/// same scenario dump the same text: UUIDs become `id-0001`, `id-0002`, …
/// by first appearance, and every non-NULL `*_at` time except the user's
/// own `due_at`/`start_at` becomes `0`.
pub fn dump(conn: &Connection) -> String {
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut out = String::new();
    for table in BOARD_TABLES {
        let columns = rows(
            conn,
            &format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid"),
        );
        let mut stmt = conn
            .prepare(&format!("SELECT {} FROM {table} ORDER BY rowid", columns.join(", ")))
            .unwrap();
        let mut result = stmt.query([]).unwrap();
        while let Some(row) = result.next().unwrap() {
            let values: Vec<String> = columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    let value = row.get_ref(index).unwrap();
                    let zero_time =
                        column.ends_with("_at") && column != "due_at" && column != "start_at";
                    literal(value, zero_time, &mut ids)
                })
                .collect();
            out.push_str(&format!(
                "INSERT INTO {table} ({}) VALUES ({});\n",
                columns.join(", "),
                values.join(", ")
            ));
        }
    }
    out
}

fn literal(value: ValueRef<'_>, zero_time: bool, ids: &mut HashMap<String, String>) -> String {
    match value {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(_) if zero_time => "0".into(),
        ValueRef::Integer(number) => number.to_string(),
        ValueRef::Text(bytes) => {
            let text = std::str::from_utf8(bytes).expect("board text is UTF-8");
            let text = if is_uuid(text) {
                let next = format!("id-{:04}", ids.len() + 1);
                ids.entry(text.to_ascii_lowercase()).or_insert(next).clone()
            } else {
                text.to_string()
            };
            format!("'{}'", text.replace('\'', "''"))
        }
        other => panic!("board tables hold no {:?} values", other.data_type()),
    }
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Runs a `dump` against a database the caller already opened through its
/// app's own open path, in one transaction: all of it lands, or none.
pub fn load(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(sql)?;
    tx.commit()
}

/// Compares `actual` with the committed fixture at `path`, or rewrites the
/// fixture when `GESPAN_BLESS_BOARD_FIXTURES=1`.
pub fn fixture_matches(path: &Path, actual: &str) {
    if std::env::var(BLESS_VAR).as_deref() == Ok("1") {
        std::fs::write(path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        expected == actual,
        "{} does not match what the app writes now. If the board contract changed on \
         purpose, rerun the test with {BLESS_VAR}=1 and commit the new fixture.\n\
         --- expected\n{expected}\n--- actual\n{actual}",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ensure;

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        ensure(&conn).unwrap();
        conn
    }

    /// One board, column and card with the given ids, written at `now`.
    fn scenario(conn: &Connection, ids: [&str; 3], now: i64, title: &str) {
        let [board, column, card] = ids;
        conn.execute(
            "INSERT INTO boards (id, name, card_seq, created_at, updated_at) VALUES (?1, 'B', 1, ?2, ?2)",
            rusqlite::params![board, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO board_columns (id, board_id, name, position, role) VALUES (?1, ?2, 'Backlog', 0, 'todo')",
            rusqlite::params![column, board],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO board_cards (id, board_id, column_id, number, title, body, position,
               link_kind, link_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, '', 0, 'session', ?2, ?5, ?5)",
            rusqlite::params![card, board, column, title, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO board_card_meta (card_id, priority, due_at) VALUES (?1, 3, ?2)",
            rusqlite::params![card, SEED_DUE_AT],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO board_comments (id, card_id, body, created_at) VALUES ('c-1', ?1, 'Hi', ?2)",
            rusqlite::params![card, now],
        )
        .unwrap();
        crate::sync_primary_link(conn, card, Some(("session", board)), now).unwrap();
    }

    const IDS_ONE: [&str; 3] = [
        "0b6e1f4c-5a1d-4c1e-9f3a-1a2b3c4d5e6f",
        "1c7f2a5d-6b2e-4d2f-8a4b-2b3c4d5e6f70",
        "2d8a3b6e-7c3f-4e3a-9b5c-3c4d5e6f7081",
    ];
    const IDS_TWO: [&str; 3] = [
        "AAAAAAAA-5a1d-4c1e-9f3a-1a2b3c4d5e6f",
        "bbbbbbbb-6b2e-4d2f-8a4b-2b3c4d5e6f70",
        "cccccccc-7c3f-4e3a-9b5c-3c4d5e6f7081",
    ];

    #[test]
    fn dump_normalizes_ids_and_times_but_keeps_due() {
        let one = fresh();
        scenario(&one, IDS_ONE, 1_000, "Card");
        let two = fresh();
        scenario(&two, IDS_TWO, 987_654, "Card");
        let dump_one = dump(&one);
        assert_eq!(dump_one, dump(&two));
        assert!(dump_one.contains("1792929600000"), "{dump_one}");
        assert!(!dump_one.contains("987654") && !dump_one.contains("1000,"), "{dump_one}");
        // The board id is reused as the card's link ref: same placeholder.
        assert!(
            dump_one.contains(
                "INSERT INTO board_cards (id, board_id, column_id, number, title, body, position, link_kind, link_id, created_at, updated_at) VALUES ('id-0003', 'id-0001', 'id-0002', 1, 'Card', '', 0, 'session', 'id-0001', 0, 0);"
            ),
            "{dump_one}"
        );
        // A NULL timestamp stays NULL, and non-UUID ids are kept.
        assert!(
            dump_one.contains("VALUES ('c-1', 'id-0003', 'Hi', 'user', 0, NULL);"),
            "{dump_one}"
        );
    }

    #[test]
    fn dump_follows_board_tables_order() {
        let conn = fresh();
        scenario(&conn, IDS_ONE, 1, "Card");
        let dumped = dump(&conn);
        let tables: Vec<&str> = dumped
            .lines()
            .map(|line| line.split(' ').nth(2).unwrap())
            .collect();
        assert_eq!(
            tables,
            ["boards", "board_columns", "board_cards", "board_card_meta", "board_comments", "board_card_links"]
        );
    }

    #[test]
    fn load_of_a_dump_dumps_the_same() {
        let x = Connection::open_in_memory().unwrap();
        x.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        seed_native_v2(&x);
        ensure(&x).unwrap();
        scenario(&x, IDS_ONE, 5, "Extra");
        let dumped = dump(&x);
        let y = fresh();
        load(&y, &dumped).unwrap();
        assert_eq!(dump(&y), dumped);
    }

    #[test]
    fn strings_with_quotes_round_trip() {
        let x = fresh();
        scenario(&x, IDS_ONE, 5, "It's \"done\"");
        let dumped = dump(&x);
        assert!(dumped.contains("'It''s \"done\"'"), "{dumped}");
        let y = fresh();
        load(&y, &dumped).unwrap();
        let title: String = y
            .query_row("SELECT title FROM board_cards", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "It's \"done\"");
    }

    #[test]
    fn a_failing_load_leaves_nothing_behind() {
        let y = fresh();
        let error = load(
            &y,
            "INSERT INTO boards (id, name, card_seq, created_at, updated_at) VALUES ('b', 'B', 0, 0, 0);
             INSERT INTO board_columns (id, board_id, name, position, role) VALUES ('c', 'missing', 'X', 0, NULL);",
        );
        assert!(error.is_err());
        let boards: i64 = y.query_row("SELECT COUNT(*) FROM boards", [], |r| r.get(0)).unwrap();
        assert_eq!(boards, 0);
    }

    #[test]
    fn fixture_matches_accepts_equal_and_names_the_bless_variable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.sql");
        std::fs::write(&path, "same\n").unwrap();
        fixture_matches(&path, "same\n");
        let failure = std::panic::catch_unwind(|| fixture_matches(&path, "different\n"))
            .unwrap_err();
        let message = failure
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(message.contains(BLESS_VAR), "{message}");
    }
}
