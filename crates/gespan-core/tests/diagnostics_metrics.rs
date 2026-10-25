//! The process-wide counters as Native opens them. Its own test binary, so
//! opening the global store cannot leak into other tests.
use gespan_core::diagnostics::metrics::{self, ErrorKind};
use gespan_core::store::Store;
use rusqlite::Connection;
use std::path::Path;

fn schema(path: &Path) -> Vec<(String, String)> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn
        .prepare("SELECT type, name FROM sqlite_master ORDER BY type, name")
        .unwrap();
    let rows = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

#[test]
fn counts_land_beside_gespan_db_and_leave_it_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("gespan.db");
    drop(Store::open_gespan(&shared).unwrap());
    let before = schema(&shared);

    assert!(metrics::last_days(7).unwrap().is_empty());
    metrics::open(&metrics::database_path(&shared), "9.9.9").unwrap();
    metrics::failure("worktree.create", ErrorKind::NotFound);
    metrics::failure("worktree.create", ErrorKind::NotFound);
    metrics::success("worktree.create");
    let tracked = metrics::track("remote.ssh_connect", Err::<(), _>("Operation timed out"));
    assert!(tracked.is_err());

    let rows: Vec<_> = metrics::last_days(7)
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.kind, c.count, c.app_version))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "remote.ssh_connect".into(),
                Some(ErrorKind::Timeout),
                1,
                "9.9.9".into()
            ),
            (
                "worktree.create".into(),
                Some(ErrorKind::NotFound),
                2,
                "9.9.9".into()
            ),
            ("worktree.create".into(), None, 1, "9.9.9".into()),
        ]
    );
    assert_eq!(schema(&shared), before);
    assert!(dir.path().join("gespan-native.db").exists());
}
