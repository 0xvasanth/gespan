//! Brings any board database (none, v1, or Native's v2 extras) up to v3.
use crate::ddl::{BASE_DDL, V3_DDL};
use crate::{App, LINK_ROLE_PRIMARY, SCHEMA_VERSION};
use rusqlite::{params, Connection, OptionalExtension, Result, Transaction, TransactionBehavior};
use std::time::{SystemTime, UNIX_EPOCH};

/// Columns v3 adds to `board_columns`, with their declarations.
const COLUMN_ADDS: [(&str, &str); 3] = [
    ("color", "color TEXT"),
    ("wip_limit", "wip_limit INTEGER"),
    ("collapsed", "collapsed INTEGER NOT NULL DEFAULT 0"),
];

/// Creates or upgrades every shared board table and mirrors each card's link
/// into `board_card_links`. When Native opens the file (`App::Native`), it
/// also copies Native's old `board_native_*` data, once, and writes the
/// version marker. Tauri never copies: an older Native build may still be
/// writing those tables, and what it writes would be lost if the copy had
/// already run. The copy waits for the first v3 Native, the last writer.
///
/// It runs on every open and opens its own `BEGIN IMMEDIATE` transaction, so
/// it must not be called inside one. Two apps opening the file at the same
/// moment serialize on that lock; once the marker is there, nothing is
/// copied again. Any failure rolls the whole upgrade back and is returned.
pub fn ensure(conn: &Connection, app: App) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute_batch(BASE_DDL)?;
    add_column_fields(&tx)?;
    tx.execute_batch(V3_DDL)?;
    let migrated = tx
        .query_row(
            "SELECT 1 FROM board_schema_migrations WHERE version = ?1",
            [SCHEMA_VERSION],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !migrated {
        // Idempotent: a primary row always mirrors the card's own columns.
        backfill_links(&tx)?;
        if app == App::Native {
            copy_from_native(&tx)?;
            tx.execute(
                "INSERT INTO board_schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![SCHEMA_VERSION, now_millis()],
            )?;
        }
    }
    tx.commit()
}

/// Replaces a card's `primary` link row with `link`, or removes it for
/// `None`. Links with any other role stay, even to the same target, since
/// `role` is part of the key. This keeps `board_card_links` in step with
/// `board_cards.link_kind/link_id`; callers run it inside the transaction
/// that changes those columns.
pub fn sync_primary_link(
    conn: &Connection,
    card_id: &str,
    link: Option<(&str, &str)>,
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM board_card_links WHERE card_id = ?1 AND role = ?2",
        params![card_id, LINK_ROLE_PRIMARY],
    )?;
    if let Some((kind, reference)) = link {
        conn.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![card_id, kind, reference, LINK_ROLE_PRIMARY, now],
        )?;
    }
    Ok(())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn add_column_fields(tx: &Connection) -> Result<()> {
    for (name, declaration) in COLUMN_ADDS {
        let present = tx
            .query_row(
                "SELECT 1 FROM pragma_table_info('board_columns') WHERE name = ?1",
                [name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !present {
            tx.execute_batch(&format!(
                "ALTER TABLE board_columns ADD COLUMN {declaration}"
            ))?;
        }
    }
    Ok(())
}

/// Each Native v2 table and the statement that copies it into v3. Only
/// rows whose card (and label) still exist are copied: orphans are already
/// invisible in Native and would break the v3 foreign keys. Positions are
/// renumbered 0..n-1; labels by name, matching Native's alphabetical order.
/// Plain `INSERT`: the v3 tables are empty here, so a conflict is a real
/// error and fails the whole upgrade instead of dropping rows.
const NATIVE_COPIES: [(&str, &str); 6] = [
    (
        "board_native_meta",
        "INSERT INTO board_card_meta (card_id, priority, due_at)
         SELECT m.card_id, m.priority, m.due_at
         FROM board_native_meta m JOIN board_cards c ON c.id = m.card_id",
    ),
    (
        "board_native_projects",
        "INSERT INTO board_card_projects (card_id, project)
         SELECT p.card_id, p.project
         FROM board_native_projects p JOIN board_cards c ON c.id = p.card_id",
    ),
    (
        "board_native_labels",
        "INSERT INTO board_labels (id, board_id, name, color, position)
         SELECT l.id, l.board_id, l.name, l.color,
                ROW_NUMBER() OVER (PARTITION BY l.board_id ORDER BY lower(l.name), l.rowid) - 1
         FROM board_native_labels l JOIN boards b ON b.id = l.board_id",
    ),
    (
        "board_native_card_labels",
        "INSERT INTO board_card_labels (card_id, label_id)
         SELECT cl.card_id, cl.label_id
         FROM board_native_card_labels cl
         JOIN board_cards c ON c.id = cl.card_id
         JOIN board_labels l ON l.id = cl.label_id",
    ),
    (
        "board_native_subtasks",
        "INSERT INTO board_subtasks (id, card_id, title, done, position)
         SELECT s.id, s.card_id, s.title, s.done,
                ROW_NUMBER() OVER (PARTITION BY s.card_id ORDER BY s.position, s.rowid) - 1
         FROM board_native_subtasks s JOIN board_cards c ON c.id = s.card_id",
    ),
    (
        "board_native_comments",
        "INSERT INTO board_comments (id, card_id, body, author, created_at, edited_at)
         SELECT n.id, n.card_id, n.body, 'user', n.created_at, NULL
         FROM board_native_comments n JOIN board_cards c ON c.id = n.card_id",
    ),
];

/// Copies Native's v2 extras into the v3 tables, skipping any source table
/// that does not exist. The `board_native_*` tables are left untouched.
fn copy_from_native(tx: &Connection) -> Result<()> {
    for (source, copy) in NATIVE_COPIES {
        let exists = tx
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [source],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            tx.execute(copy, [])?;
        }
    }
    Ok(())
}

/// Mirrors every card's old single link into `board_card_links` as its
/// `primary` link, dated by the card's last update.
fn backfill_links(tx: &Connection) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
         SELECT id, link_kind, link_id, ?1, updated_at FROM board_cards
         WHERE link_kind IS NOT NULL AND link_id IS NOT NULL",
        [LINK_ROLE_PRIMARY],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_native_v2_migrated, seed_native_v2};
    use crate::{App, BASE_DDL, BOARD_TABLES};
    use rusqlite::{params, Connection};

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn
    }

    fn strings(conn: &Connection, sql: &str) -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn seed_v1_card(conn: &Connection, link: Option<(&str, &str)>) {
        conn.execute_batch(
            "INSERT INTO boards (id, name, created_at, updated_at) VALUES ('b', 'Work', 1, 1);
             INSERT INTO board_columns (id, board_id, name, position, role)
               VALUES ('col', 'b', 'Done', 0, 'done');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO board_cards (id, board_id, column_id, number, title, position,
               link_kind, link_id, created_at, updated_at)
             VALUES ('card', 'b', 'col', 1, 'Ship it', 0, ?1, ?2, 5, 7)",
            params![link.map(|l| l.0), link.map(|l| l.1)],
        )
        .unwrap();
    }

    #[test]
    fn fresh_database_gets_every_table_and_the_marker() {
        let conn = memory();
        ensure(&conn, App::Native).unwrap();
        let tables = strings(
            &conn,
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'board%' ORDER BY name",
        );
        let mut expected: Vec<String> = BOARD_TABLES.iter().map(|t| t.to_string()).collect();
        expected.push("board_schema_migrations".into());
        expected.sort();
        assert_eq!(tables, expected);
        assert_eq!(
            strings(
                &conn,
                "SELECT CAST(version AS TEXT) FROM board_schema_migrations"
            ),
            ["3"]
        );
    }

    #[test]
    fn ensure_twice_changes_nothing() {
        let conn = memory();
        ensure(&conn, App::Native).unwrap();
        let version = count(&conn, "PRAGMA schema_version");
        ensure(&conn, App::Native).unwrap();
        assert_eq!(count(&conn, "PRAGMA schema_version"), version);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            1
        );
    }

    #[test]
    fn v1_database_gains_columns_and_keeps_rows() {
        let conn = memory();
        conn.execute_batch(BASE_DDL).unwrap();
        seed_v1_card(&conn, Some(("session", "s-1")));
        ensure(&conn, App::Native).unwrap();
        let (color, wip, collapsed): (Option<String>, Option<i64>, i64) = conn
            .query_row(
                "SELECT color, wip_limit, collapsed FROM board_columns WHERE id = 'col'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((color, wip, collapsed), (None, None, 0));
        let card: (String, String, String, i64, i64) = conn
            .query_row(
                "SELECT title, link_kind, link_id, created_at, updated_at FROM board_cards",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            card,
            ("Ship it".into(), "session".into(), "s-1".into(), 5, 7)
        );
    }

    #[test]
    fn v1_link_columns_are_backfilled_as_the_primary_link() {
        let conn = memory();
        conn.execute_batch(BASE_DDL).unwrap();
        seed_v1_card(&conn, Some(("session", "s-1")));
        ensure(&conn, App::Native).unwrap();
        let links = strings(
            &conn,
            "SELECT card_id || '/' || kind || '/' || ref || '/' || role || '/' || created_at
             FROM board_card_links",
        );
        assert_eq!(links, ["card/session/s-1/primary/7"]);
    }

    #[test]
    fn ref_and_role_columns_round_trip() {
        let conn = memory();
        ensure(&conn, App::Native).unwrap();
        seed_v1_card(&conn, None);
        conn.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, created_at)
             VALUES ('card', 'pr', 'gh:1', 9)",
            [],
        )
        .unwrap();
        assert_eq!(
            strings(
                &conn,
                "SELECT role FROM board_card_links WHERE ref = 'gh:1'"
            ),
            ["primary"]
        );
    }

    #[test]
    fn sync_primary_link_replaces_and_clears() {
        let conn = memory();
        ensure(&conn, App::Native).unwrap();
        seed_v1_card(&conn, None);
        conn.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES ('card', 'session', 'origin-1', 'origin', 1)",
            [],
        )
        .unwrap();
        let primary = |conn: &Connection| {
            strings(
                conn,
                "SELECT kind || '/' || ref FROM board_card_links WHERE role = 'primary'",
            )
        };
        sync_primary_link(&conn, "card", Some(("session", "a")), 10).unwrap();
        assert_eq!(primary(&conn), ["session/a"]);
        sync_primary_link(&conn, "card", Some(("pilot", "b")), 11).unwrap();
        assert_eq!(primary(&conn), ["pilot/b"]);
        sync_primary_link(&conn, "card", None, 12).unwrap();
        assert!(primary(&conn).is_empty());
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM board_card_links WHERE role = 'origin'"
            ),
            1
        );
    }
    #[test]
    fn native_v2_data_is_copied_with_every_value() {
        let conn = memory();
        let seed = seed_native_v2(&conn);
        ensure(&conn, App::Native).unwrap();
        assert_native_v2_migrated(&conn, &seed);
    }

    #[test]
    fn orphans_are_skipped_and_positions_renumbered() {
        let conn = memory();
        let seed = seed_native_v2(&conn);
        assert_eq!(count(&conn, "PRAGMA foreign_keys"), 1);
        // The orphan rows would violate the v3 foreign keys if copied.
        ensure(&conn, App::Native).expect("orphans must not fail the open");
        assert_native_v2_migrated(&conn, &seed);
        assert_eq!(
            strings(
                &conn,
                "SELECT CAST(position AS TEXT) FROM board_subtasks ORDER BY position"
            ),
            ["0", "1", "2"]
        );
    }

    #[test]
    fn rerun_does_not_resurrect_deleted_rows() {
        let conn = memory();
        seed_native_v2(&conn);
        ensure(&conn, App::Native).unwrap();
        conn.execute_batch(
            "DELETE FROM board_labels WHERE name = 'docs';
             DELETE FROM board_subtasks WHERE title = 'Two';",
        )
        .unwrap();
        ensure(&conn, App::Native).unwrap();
        assert_eq!(strings(&conn, "SELECT name FROM board_labels"), ["Bug"]);
        assert_eq!(
            strings(&conn, "SELECT title FROM board_subtasks ORDER BY position"),
            ["One", "Three"]
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM board_card_labels WHERE label_id = 'label-docs'"
            ),
            0
        );
    }

    #[test]
    fn failed_copy_rolls_back_and_retries() {
        let conn = memory();
        let seed = seed_native_v2(&conn);
        conn.execute_batch(V3_DDL).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail BEFORE INSERT ON board_comments BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
        let error = ensure(&conn, App::Native).unwrap_err();
        assert!(error.to_string().contains("boom"), "{error}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_meta"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_labels"), 0);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            0
        );
        assert!(
            conn.is_autocommit(),
            "the failed upgrade left a transaction open"
        );
        conn.execute_batch("DROP TRIGGER fail;").unwrap();
        ensure(&conn, App::Native).unwrap();
        assert_native_v2_migrated(&conn, &seed);
    }

    #[test]
    fn database_without_native_tables_only_backfills_links() {
        let conn = memory();
        conn.execute_batch(BASE_DDL).unwrap();
        seed_v1_card(&conn, Some(("pilot", "p-1")));
        ensure(&conn, App::Native).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_links"), 1);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'board_native%'"
            ),
            0
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            1
        );
    }

    #[test]
    fn concurrent_ensure_copies_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let open = |path: &std::path::Path| {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;",
            )
            .unwrap();
            conn
        };
        let seed = seed_native_v2(&open(&path));
        // Tauri and two Native connections (Native opens one per turn) race.
        let apps = [App::Native, App::Tauri, App::Native];
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(apps.len()));
        let handles: Vec<_> = apps
            .into_iter()
            .map(|app| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let conn = open(&path);
                    barrier.wait();
                    ensure(&conn, app)
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().expect("each app's ensure succeeds");
        }
        let conn = open(&path);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_subtasks"), 3);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            1
        );
        assert_native_v2_migrated(&conn, &seed);
    }
    #[test]
    fn tauri_leaves_the_native_copy_to_native() {
        let conn = memory();
        seed_native_v2(&conn);
        ensure(&conn, App::Tauri).unwrap();
        // Tauri creates the tables and mirrors links, but copies nothing and
        // writes no marker: an older Native may still be writing its tables.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_meta"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_subtasks"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_links"), 1);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            0
        );
        ensure(&conn, App::Tauri).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_card_links"), 1);
        // The older Native build keeps writing until it is updated.
        conn.execute(
            "INSERT INTO board_native_comments (id, card_id, body, created_at)
             VALUES ('comment-3', 'card-a', 'Written by an older Native', 3000)",
            [],
        )
        .unwrap();
        ensure(&conn, App::Native).unwrap();
        assert_eq!(
            strings(&conn, "SELECT body FROM board_comments ORDER BY created_at"),
            ["First", "Second", "Written by an older Native"]
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            1
        );
    }

    #[test]
    fn tauri_on_a_fresh_database_writes_no_marker_and_native_later_does() {
        let conn = memory();
        ensure(&conn, App::Tauri).unwrap();
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            0
        );
        ensure(&conn, App::Native).unwrap();
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"),
            1
        );
    }

    #[test]
    fn a_link_can_be_primary_and_keep_another_role() {
        let conn = memory();
        ensure(&conn, App::Native).unwrap();
        seed_v1_card(&conn, None);
        conn.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES ('card', 'session', 's-1', 'origin', 1)",
            [],
        )
        .unwrap();
        let links = |conn: &Connection| {
            strings(
                conn,
                "SELECT kind || '/' || ref || '/' || role FROM board_card_links ORDER BY role, ref",
            )
        };
        sync_primary_link(&conn, "card", Some(("session", "s-1")), 2).unwrap();
        assert_eq!(links(&conn), ["session/s-1/origin", "session/s-1/primary"]);
        sync_primary_link(&conn, "card", Some(("pilot", "x")), 3).unwrap();
        assert_eq!(links(&conn), ["session/s-1/origin", "pilot/x/primary"]);
        sync_primary_link(&conn, "card", None, 4).unwrap();
        assert_eq!(links(&conn), ["session/s-1/origin"]);
    }
}
