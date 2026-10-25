//! Brings any board database (none, v1, or Native's v2 extras) up to v3.
use crate::ddl::{BASE_DDL, V3_DDL};
use crate::{LINK_ROLE_PRIMARY, SCHEMA_VERSION};
use rusqlite::{params, Connection, OptionalExtension, Result, Transaction, TransactionBehavior};
use std::time::{SystemTime, UNIX_EPOCH};

/// Columns v3 adds to `board_columns`, with their declarations.
const COLUMN_ADDS: [(&str, &str); 3] = [
    ("color", "color TEXT"),
    ("wip_limit", "wip_limit INTEGER"),
    ("collapsed", "collapsed INTEGER NOT NULL DEFAULT 0"),
];

/// Creates or upgrades every shared board table, then copies Native's old
/// `board_native_*` data and backfills `board_card_links` exactly once.
///
/// It runs on every open and opens its own `BEGIN IMMEDIATE` transaction, so
/// it must not be called inside one. Two apps opening the file at the same
/// moment serialize on that lock; the second sees the version marker and
/// copies nothing. Any failure rolls the whole upgrade back and is returned.
pub fn ensure(conn: &Connection) -> Result<()> {
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
        copy_from_native(&tx)?;
        backfill_links(&tx)?;
        tx.execute(
            "INSERT INTO board_schema_migrations (version, applied_at) VALUES (?1, ?2)",
            params![SCHEMA_VERSION, now_millis()],
        )?;
    }
    tx.commit()
}

/// Replaces a card's `primary` link row with `link`, or removes it for
/// `None`, leaving links of every other role alone. This keeps
/// `board_card_links` in step with `board_cards.link_kind/link_id`; callers
/// run it inside the transaction that changes those columns.
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
            "INSERT OR REPLACE INTO board_card_links (card_id, kind, ref, role, created_at)
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
            tx.execute_batch(&format!("ALTER TABLE board_columns ADD COLUMN {declaration}"))?;
        }
    }
    Ok(())
}

/// Copies Native's v2 extras into the v3 tables. Filled in by the next step.
fn copy_from_native(_tx: &Connection) -> Result<()> {
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
    use crate::{BASE_DDL, BOARD_TABLES};
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
        ensure(&conn).unwrap();
        let tables = strings(
            &conn,
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'board%' ORDER BY name",
        );
        let mut expected: Vec<String> = BOARD_TABLES.iter().map(|t| t.to_string()).collect();
        expected.push("board_schema_migrations".into());
        expected.sort();
        assert_eq!(tables, expected);
        assert_eq!(
            strings(&conn, "SELECT CAST(version AS TEXT) FROM board_schema_migrations"),
            ["3"]
        );
    }

    #[test]
    fn ensure_twice_changes_nothing() {
        let conn = memory();
        ensure(&conn).unwrap();
        let version = count(&conn, "PRAGMA schema_version");
        ensure(&conn).unwrap();
        assert_eq!(count(&conn, "PRAGMA schema_version"), version);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_schema_migrations"), 1);
    }

    #[test]
    fn v1_database_gains_columns_and_keeps_rows() {
        let conn = memory();
        conn.execute_batch(BASE_DDL).unwrap();
        seed_v1_card(&conn, Some(("session", "s-1")));
        ensure(&conn).unwrap();
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
        assert_eq!(card, ("Ship it".into(), "session".into(), "s-1".into(), 5, 7));
    }

    #[test]
    fn v1_link_columns_are_backfilled_as_the_primary_link() {
        let conn = memory();
        conn.execute_batch(BASE_DDL).unwrap();
        seed_v1_card(&conn, Some(("session", "s-1")));
        ensure(&conn).unwrap();
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
        ensure(&conn).unwrap();
        seed_v1_card(&conn, None);
        conn.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, created_at)
             VALUES ('card', 'pr', 'gh:1', 9)",
            [],
        )
        .unwrap();
        assert_eq!(
            strings(&conn, "SELECT role FROM board_card_links WHERE ref = 'gh:1'"),
            ["primary"]
        );
    }

    #[test]
    fn sync_primary_link_replaces_and_clears() {
        let conn = memory();
        ensure(&conn).unwrap();
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
            count(&conn, "SELECT COUNT(*) FROM board_card_links WHERE role = 'origin'"),
            1
        );
    }
}
