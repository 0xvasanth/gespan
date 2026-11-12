mod support;

use std::collections::BTreeSet;
use std::fs;

use gespan_graph::{EdgeKind, GraphError, GraphStore, NodeKind, FILE_NAME};
use rusqlite::{params, Connection};
use support::{temp_store, BASE};

fn names(conn: &Connection, kind: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 AND name LIKE 'graph_%'")
        .unwrap();
    let rows = stmt
        .query_map([kind], |row| row.get::<_, String>(0))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn index_sql(conn: &Connection, name: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [name],
        |row| row.get(0),
    )
    .unwrap()
}

/// The graph migrations recorded in the file, by name.
fn applied(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM __toasty_migrations ORDER BY id")
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn known_migrations() -> Vec<String> {
    let mut names: Vec<String> = gespan_graph::schema::migrations()
        .migrations()
        .iter()
        .map(|m| m.name().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn fresh_open_creates_the_tables_indexes_and_constraints() {
    let (_dir, store, _path) = temp_store();
    let conn = store.connection();
    let mut recorded = applied(conn);
    recorded.sort();
    assert!(!recorded.is_empty());
    assert_eq!(recorded, known_migrations(), "every migration is applied");

    let tables: BTreeSet<String> = [
        "graph_nodes",
        "graph_edges",
        "graph_scopes",
        "graph_run_scopes",
        "graph_runs",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(names(conn, "table"), tables, "no version table");

    // Named indexes only; SQLite's own autoindexes are `sqlite_autoindex_*`.
    let indexes: BTreeSet<String> = [
        "graph_nodes_kind_key",
        "graph_edges_open_uq",
        "graph_edges_scope_open",
        "graph_edges_scope",
        "graph_edges_out_open",
        "graph_edges_in_open",
        "graph_edges_out_all",
        "graph_edges_in_all",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(names(conn, "index"), indexes);

    for partial in [
        "graph_edges_out_open",
        "graph_edges_in_open",
        "graph_edges_open_uq",
        "graph_edges_scope_open",
    ] {
        let sql = index_sql(conn, partial);
        assert!(sql.contains("WHERE valid_to IS NULL"), "{partial}: {sql}");
    }
    for full in [
        "graph_edges_scope",
        "graph_edges_out_all",
        "graph_edges_in_all",
    ] {
        let sql = index_sql(conn, full);
        assert!(!sql.contains("WHERE"), "{full} must cover every row: {sql}");
    }
    let uq = index_sql(conn, "graph_edges_open_uq");
    assert!(uq.starts_with("CREATE UNIQUE INDEX"), "{uq}");
    assert!(uq.contains("(kind, src, dst, source, scope)"), "{uq}");
    let nodes_uq = index_sql(conn, "graph_nodes_kind_key");
    assert!(nodes_uq.starts_with("CREATE UNIQUE INDEX"), "{nodes_uq}");
    // The as-of indexes lead with the endpoint then valid_from DESC, so a
    // capped as-of read walks newest first and stops (spec §4.1).
    assert!(
        index_sql(conn, "graph_edges_out_all").contains("(src, valid_from DESC, dst, valid_to)")
    );
    assert!(index_sql(conn, "graph_edges_in_all").contains("(dst, valid_from DESC, src, valid_to)"));

    // Every constraint trigger is in place, and no other.
    let expected: BTreeSet<String> = gespan_graph::schema::constraints()
        .iter()
        .filter_map(|sql| sql.strip_prefix("CREATE TRIGGER IF NOT EXISTS "))
        .map(|rest| rest.split_whitespace().next().unwrap().to_string())
        .collect();
    assert_eq!(expected.len(), 17);
    assert_eq!(names(conn, "trigger"), expected);
}

#[test]
fn reopen_keeps_rows_and_applies_nothing_again() {
    let (_dir, store, path) = temp_store();
    store
        .connection()
        .execute(
            "INSERT INTO graph_nodes (kind, key, label, props_json, created_at, updated_at) VALUES ('project', 'p', 'P', '{}', ?1, ?1)",
            [BASE],
        )
        .unwrap();
    let before = applied(store.connection());
    drop(store);

    let store = GraphStore::open(&path).unwrap();
    let conn = store.connection();
    let label: String = conn
        .query_row(
            "SELECT label FROM graph_nodes WHERE kind = 'project' AND key = 'p'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(label, "P");
    assert_eq!(
        applied(conn),
        before,
        "a reopen must not apply or record a migration again"
    );
    // Reopening ran the constraints again; they are idempotent.
    assert_eq!(names(conn, "trigger").len(), 17);
}

#[test]
fn a_file_with_an_unknown_migration_is_refused_untouched() {
    let (_dir, store, path) = temp_store();
    drop(store);
    // A newer build's migration, recorded the way Toasty records one. The
    // plain connection is the file's only one, so closing it folds the WAL
    // back into the file.
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO __toasty_migrations (id, name, applied_at) VALUES (?1, ?2, datetime('now'))",
        params![4_242_424_242_i64, "0002_from_a_newer_build.sql"],
    )
    .unwrap();
    drop(conn);
    let before = fs::read(&path).unwrap();

    match GraphStore::open(&path) {
        Err(GraphError::UnknownMigrations(names)) => {
            assert_eq!(names, vec!["0002_from_a_newer_build.sql".to_string()]);
        }
        Err(other) => panic!("expected UnknownMigrations, got {other:?}"),
        Ok(_) => panic!("a file from a newer build must be refused"),
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "the file must be left byte-for-byte"
    );

    // Without the stranger, the same file opens again.
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "DELETE FROM __toasty_migrations WHERE name = '0002_from_a_newer_build.sql'",
        [],
    )
    .unwrap();
    drop(conn);
    assert!(GraphStore::open(&path).is_ok());
}

#[test]
fn open_refuses_non_database_and_missing_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let junk: Vec<u8> = b"not sqlite".iter().copied().cycle().take(4096).collect();
    fs::write(&path, &junk).unwrap();

    match GraphStore::open(&path) {
        Err(GraphError::Sqlite(_)) => {}
        Err(other) => panic!("expected a SQLite error, got {other:?}"),
        Ok(_) => panic!("a non-database file must be refused"),
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        junk,
        "the foreign file must be untouched"
    );
    assert!(
        !dir.path().join(format!("{FILE_NAME}-wal")).exists(),
        "no WAL may be created"
    );

    let missing = dir.path().join("no-such-dir").join(FILE_NAME);
    assert!(GraphStore::open(&missing).is_err());
    assert!(!missing.exists());
}

fn insert_node(conn: &Connection, kind: &str, key: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO graph_nodes (kind, key, label, props_json, created_at, updated_at) VALUES (?1, ?2, ?2, '{}', ?3, ?3)",
        params![kind, key, BASE],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_edge(
    conn: &Connection,
    (kind, src, dst): (&str, i64, i64),
    basis: &str,
    valid: (i64, Option<i64>),
    scope: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO graph_edges (src, dst, kind, props_json, valid_from, valid_from_basis, valid_to, source, scope, created_at)
         VALUES (?1, ?2, ?3, '{}', ?4, ?5, ?6, 'backfill:blocks', ?7, ?4)",
        params![src, dst, kind, valid.0, basis, valid.1, scope],
    )
}

fn is_constraint(result: rusqlite::Result<usize>) -> bool {
    matches!(
        result,
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

#[test]
fn sql_checks_reject_bad_rows() {
    let (_dir, store, _path) = temp_store();
    let conn = store.connection();
    assert!(
        insert_node(conn, "widget", "w").is_err(),
        "unknown node kind"
    );
    let s = insert_node(conn, "session", "s").unwrap();
    let f = insert_node(conn, "file", "f").unwrap();

    assert!(
        is_constraint(insert_edge(
            conn,
            ("frobs", s, f),
            "native",
            (BASE, None),
            "structural"
        )),
        "unknown edge kind"
    );
    assert!(
        is_constraint(insert_edge(
            conn,
            ("touched", s, f),
            "guess",
            (BASE, None),
            "structural"
        )),
        "unknown basis"
    );
    assert!(
        is_constraint(insert_edge(
            conn,
            ("touched", s, f),
            "native",
            (BASE, Some(BASE - 1)),
            "structural"
        )),
        "valid_to before valid_from"
    );
    assert!(
        is_constraint(insert_edge(
            conn,
            ("touched", s, s),
            "native",
            (BASE, None),
            "structural"
        )),
        "self-loop"
    );
    // The control row with the same shape is accepted, so the failures above
    // are the checks and not a malformed statement.
    assert_eq!(
        insert_edge(
            conn,
            ("touched", s, f),
            "native",
            (BASE, Some(BASE)),
            "structural"
        )
        .unwrap(),
        1
    );
}

/// The quoted values inside `NEW.<column> IN (...)` in a check trigger.
fn check_list(conn: &Connection, trigger: &str, column: &str) -> BTreeSet<String> {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
            [trigger],
            |r| r.get(0),
        )
        .unwrap();
    let marker = format!("NEW.{column} IN (");
    let start = sql
        .find(&marker)
        .unwrap_or_else(|| panic!("{trigger} has no {column} list"))
        + marker.len();
    let end = start + sql[start..].find(')').unwrap();
    sql[start..end]
        .split(',')
        .map(|v| v.trim().trim_matches('\'').to_string())
        .collect()
}

#[test]
fn kind_lists_match_the_check_triggers() {
    let (_dir, store, _path) = temp_store();
    let conn = store.connection();
    let nodes: BTreeSet<String> = NodeKind::ALL
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    let edges: BTreeSet<String> = EdgeKind::ALL
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    assert_eq!(nodes.len(), 14);
    assert_eq!(edges.len(), 15);
    for event in ["insert", "update"] {
        assert_eq!(
            check_list(conn, &format!("graph_nodes_kind_{event}"), "kind"),
            nodes
        );
        assert_eq!(
            check_list(conn, &format!("graph_edges_kind_{event}"), "kind"),
            edges
        );
    }
    let basis: BTreeSet<String> = ["native", "owner_created", "observed_by"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        check_list(conn, "graph_edges_basis_insert", "valid_from_basis"),
        basis
    );
    // Every listed kind is accepted, not just named.
    for (i, kind) in nodes.iter().enumerate() {
        insert_node(conn, kind, &format!("k{i}")).unwrap();
    }
}

#[test]
fn kinds_round_trip() {
    for kind in NodeKind::ALL {
        assert_eq!(kind.as_str().parse::<NodeKind>().unwrap(), kind);
    }
    for kind in EdgeKind::ALL {
        assert_eq!(kind.as_str().parse::<EdgeKind>().unwrap(), kind);
    }
    assert_eq!(NodeKind::AutomationRun.as_str(), "automation_run");
    assert_eq!(EdgeKind::LinksTicket.as_str(), "links_ticket");
    assert!(
        matches!("widget".parse::<NodeKind>(), Err(GraphError::UnknownKind(k)) if k == "widget")
    );
    assert!(
        matches!("widget".parse::<EdgeKind>(), Err(GraphError::UnknownKind(k)) if k == "widget")
    );
}

#[test]
fn one_open_row_per_key_but_scopes_may_share_a_fact() {
    let (_dir, store, _path) = temp_store();
    let conn = store.connection();
    let a = insert_node(conn, "card", "a").unwrap();
    let t = insert_node(conn, "ticket", "T-1").unwrap();

    assert_eq!(
        insert_edge(conn, ("tracks", a, t), "native", (BASE, None), "card:a").unwrap(),
        1
    );
    assert_eq!(
        insert_edge(conn, ("tracks", a, t), "native", (BASE, None), "card:b").unwrap(),
        1
    );
    assert!(
        is_constraint(insert_edge(
            conn,
            ("tracks", a, t),
            "native",
            (BASE + 5, None),
            "card:a"
        )),
        "a second open row for one key must fail the unique index"
    );
    assert_eq!(
        insert_edge(
            conn,
            ("tracks", a, t),
            "native",
            (BASE, Some(BASE + 1)),
            "card:a"
        )
        .unwrap(),
        1,
        "a closed history copy sits alongside the open row"
    );
}

#[cfg(unix)]
#[test]
fn graph_file_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, _store, path) = temp_store();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "graph.db mode {mode:o}");
}

/// What the hand-made schema's `CHECK`s and `REFERENCES ... ON DELETE
/// CASCADE` did, now triggers: updates are checked like inserts, an edge
/// must name existing nodes, deleting a node deletes its edges, and a run's
/// status is one of four.
#[test]
fn constraint_triggers_check_updates_references_and_cascade() {
    let (_dir, store, _path) = temp_store();
    let conn = store.connection();
    let s = insert_node(conn, "session", "s").unwrap();
    let f = insert_node(conn, "file", "f").unwrap();
    let p = insert_node(conn, "project", "p").unwrap();
    insert_edge(conn, ("touched", s, f), "native", (BASE, None), "session:s").unwrap();
    let edge = conn.last_insert_rowid();
    insert_edge(
        conn,
        ("in_project", f, p),
        "native",
        (BASE, None),
        "structural",
    )
    .unwrap();

    let update = |sql: &str| conn.execute(sql, [edge]);
    assert!(is_constraint(update(
        "UPDATE graph_edges SET kind = 'frobs' WHERE id = ?1"
    )));
    assert!(is_constraint(update(
        "UPDATE graph_edges SET valid_from_basis = 'guess' WHERE id = ?1"
    )));
    assert!(is_constraint(update(
        "UPDATE graph_edges SET valid_to = valid_from - 1 WHERE id = ?1"
    )));
    assert!(is_constraint(update(
        "UPDATE graph_edges SET dst = src WHERE id = ?1"
    )));
    assert!(is_constraint(conn.execute(
        "UPDATE graph_nodes SET kind = 'widget' WHERE id = ?1",
        [s]
    )));
    // Closing the edge at its start is fine.
    assert_eq!(
        update("UPDATE graph_edges SET valid_to = valid_from WHERE id = ?1").unwrap(),
        1
    );

    // References: no edge to or from a node that isn't there.
    for (src, dst) in [(s, 9_999), (9_999, f)] {
        assert!(is_constraint(insert_edge(
            conn,
            ("touched", src, dst),
            "native",
            (BASE, None),
            "session:s"
        )));
    }
    assert!(is_constraint(update(
        "UPDATE graph_edges SET src = 9999 WHERE id = ?1"
    )));

    // Deleting a node deletes every edge touching it, in either direction.
    let edges = || -> i64 {
        conn.query_row("SELECT count(*) FROM graph_edges", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(edges(), 2);
    conn.execute("DELETE FROM graph_nodes WHERE id = ?1", [f])
        .unwrap();
    assert_eq!(edges(), 0, "both of f's edges went with it");

    let run = |status: &str| {
        conn.execute(
            "INSERT INTO graph_runs (started_at, observed_at, status) VALUES (?1, ?1, ?2)",
            params![BASE, status],
        )
    };
    assert!(is_constraint(run("paused")));
    for status in ["running", "completed", "cancelled", "failed"] {
        assert_eq!(run(status).unwrap(), 1, "{status}");
    }
}

#[test]
fn timestamps_are_epoch_ms_and_never_rescaled() {
    use gespan_graph::{EdgeKind as E, NodeKind as N, Timestamp};
    use std::time::{Duration, UNIX_EPOCH};
    use support::*;

    assert_eq!(
        Timestamp::from_system_time(UNIX_EPOCH + Duration::from_millis(1_500)),
        Some(Timestamp::from_millis(1_500))
    );
    assert_eq!(
        Timestamp::from_system_time(UNIX_EPOCH - Duration::from_millis(1)),
        None
    );
    assert_eq!(Timestamp::from_millis(BASE).as_millis(), BASE);

    let (_dir, mut store, _path) = temp_store();
    add_node(&mut store, N::Session, "s1", 0);
    add_node(&mut store, N::File, "ms", 0);
    add_node(&mut store, N::File, "secs", 0);
    let s = nref(N::Session, "s1");
    store
        .upsert_edge(&fact(E::Touched, &s, &nref(N::File, "ms"), 0))
        .unwrap();
    let mut secs = fact(E::Touched, &s, &nref(N::File, "secs"), 0);
    secs.valid_from = Timestamp::from_millis(1_793_404_800);
    store.upsert_edge(&secs).unwrap();

    let dump = store.canonical_dump().unwrap();
    let froms: Vec<(String, i64)> = dump
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v[0] == "E")
        .map(|v| (v[5].as_str().unwrap().to_string(), v[8].as_i64().unwrap()))
        .collect();
    assert_eq!(
        froms,
        vec![
            ("ms".to_string(), BASE),
            ("secs".to_string(), 1_793_404_800)
        ]
    );

    // The seconds-sized time is a moment in January 1970 to the queries.
    use gespan_graph::{AsOf, Caps};
    let start = store.node(&s).unwrap().unwrap();
    let at = |t: i64| -> Vec<String> {
        store
            .neighborhood(start, Caps::DEFAULT, AsOf::At(Timestamp::from_millis(t)))
            .unwrap()
            .into_iter()
            .map(|h| h.key.as_str().to_string())
            .collect()
    };
    assert_eq!(at(BASE), vec!["ms".to_string(), "secs".to_string()]);
    assert_eq!(at(1_793_404_799), Vec::<String>::new());
    assert_eq!(at(1_793_404_800), vec!["secs".to_string()]);
}
