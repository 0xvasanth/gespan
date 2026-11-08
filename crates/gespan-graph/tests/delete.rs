mod support;

use std::collections::BTreeSet;

use gespan_graph::{
    DeleteReport, EdgeKind as E, GraphError, GraphStore, NodeKind as N, NodeRef, Scope, SessionId,
    FILE_NAME,
};
use support::*;

fn keys(store: &GraphStore) -> BTreeSet<String> {
    let mut stmt = store
        .connection()
        .prepare("SELECT kind || ':' || key FROM graph_nodes")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn dangling_edges(store: &GraphStore) -> i64 {
    store
        .connection()
        .query_row(
            "SELECT count(*) FROM graph_edges
             WHERE src NOT IN (SELECT id FROM graph_nodes) OR dst NOT IN (SELECT id FROM graph_nodes)",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn link(store: &mut GraphStore, kind: E, src: &NodeRef, dst: &NodeRef) {
    store.upsert_edge(&fact(kind, src, dst, 10)).unwrap();
}

fn add_scope_row(store: &GraphStore, scope: &str) {
    store
        .connection()
        .execute(
            "INSERT INTO graph_scopes (source, scope, observed_updated_at, observed_at) VALUES ('backfill:blocks', ?1, ?2, ?2)",
            rusqlite::params![scope, BASE],
        )
        .unwrap();
}

fn scope_rows(store: &GraphStore) -> BTreeSet<String> {
    let mut stmt = store
        .connection()
        .prepare("SELECT scope FROM graph_scopes")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// Deletes and checks the report against raw counts taken before and after.
fn delete_checked(store: &mut GraphStore, node: &NodeRef) -> DeleteReport {
    let (nodes, edges) = (count(store, "graph_nodes"), count(store, "graph_edges"));
    let report = store.delete_anchor(node).unwrap();
    assert_eq!(
        report.nodes_removed as i64,
        nodes - count(store, "graph_nodes"),
        "{node:?}"
    );
    assert_eq!(
        report.edges_removed as i64,
        edges - count(store, "graph_edges"),
        "{node:?}"
    );
    assert_eq!(dangling_edges(store), 0);
    report
}

#[test]
fn delete_session_keeps_shared_nodes() {
    let (_dir, mut store, _path) = temp_store();
    for (kind, key) in [
        (N::Session, "A"),
        (N::Session, "B"),
        (N::Project, "P"),
        (N::File, "F"),
        (N::File, "G"),
        (N::Worktree, "W"),
        (N::Branch, "X"),
        (N::Pr, "PR"),
        (N::Repo, "R"),
    ] {
        add_node(&mut store, kind, key, 0);
    }
    let r = |kind, key| nref(kind, key);
    let (a, b, p) = (r(N::Session, "A"), r(N::Session, "B"), r(N::Project, "P"));
    link(&mut store, E::InProject, &a, &p);
    link(&mut store, E::InProject, &b, &p);
    link(&mut store, E::Touched, &a, &r(N::File, "F"));
    link(&mut store, E::Touched, &b, &r(N::File, "F"));
    link(&mut store, E::Touched, &a, &r(N::File, "G"));
    // G, W and X also join the shared project, which must not keep them.
    link(&mut store, E::InProject, &r(N::File, "G"), &p);
    link(&mut store, E::RanIn, &a, &r(N::Worktree, "W"));
    link(&mut store, E::InProject, &r(N::Worktree, "W"), &p);
    link(&mut store, E::OnBranch, &a, &r(N::Branch, "X"));
    link(&mut store, E::InProject, &r(N::Branch, "X"), &p);
    link(&mut store, E::OpenedPr, &a, &r(N::Pr, "PR"));
    link(&mut store, E::OfRepo, &r(N::Pr, "PR"), &r(N::Repo, "R"));

    let report = store.delete_session(&SessionId("A".into())).unwrap();
    assert_eq!(keys(&store), set(&["file:F", "project:P", "session:B"]));
    assert_eq!(
        report,
        DeleteReport {
            nodes_removed: 6,
            edges_removed: 10
        }
    );
    assert_eq!(count(&store, "graph_edges"), 2);
    assert_eq!(dangling_edges(&store), 0);
}

#[test]
fn delete_anchor_removes_scope_owned_edges() {
    for parent_present in [true, false] {
        let (_dir, mut store, _path) = temp_store();
        if parent_present {
            add_node(&mut store, N::Session, "P", 0);
        }
        add_node(&mut store, N::Session, "C", 0);
        add_node(&mut store, N::Pilot, "X", 0);
        let (c, x) = (nref(N::Session, "C"), nref(N::Pilot, "X"));
        let owner = Scope::Owner(nref(N::Session, "P"));
        // History first: a closed fact arriving while the key is open would
        // close that row instead (spec §4.3).
        let mut history = fact(E::Spawned, &x, &c, 10);
        history.scope = owner.clone();
        history.valid_to = Some(ts(20));
        store.upsert_edge(&history).unwrap();
        let mut open = fact(E::Spawned, &x, &c, 30);
        open.scope = owner;
        store.upsert_edge(&open).unwrap();
        assert_eq!(count(&store, "graph_edges"), 2);

        let report = delete_checked(&mut store, &nref(N::Session, "P"));
        assert_eq!(report.edges_removed, 2, "parent present: {parent_present}");
        assert_eq!(report.nodes_removed, u64::from(parent_present));
        assert_eq!(keys(&store), set(&["pilot:X", "session:C"]));
    }
}

#[test]
fn delete_anchor_every_kind() {
    let (_dir, mut store, _path) = temp_store();
    for (kind, key) in [
        (N::Card, "K"),
        (N::Ticket, "T"),
        (N::Note, "NT"),
        (N::Project, "P"),
        (N::Session, "S"),
        (N::AutomationRun, "AR"),
        (N::Session, "S2"),
        (N::Pilot, "PI"),
        (N::Session, "S3"),
        (N::Memory, "M"),
        (N::File, "FM"),
    ] {
        add_node(&mut store, kind, key, 0);
    }
    let r = |kind, key| nref(kind, key);
    link(&mut store, E::Tracks, &r(N::Card, "K"), &r(N::Ticket, "T"));
    link(
        &mut store,
        E::InProject,
        &r(N::Note, "NT"),
        &r(N::Project, "P"),
    );
    link(
        &mut store,
        E::InProject,
        &r(N::Session, "S"),
        &r(N::Project, "P"),
    );
    link(
        &mut store,
        E::Spawned,
        &r(N::AutomationRun, "AR"),
        &r(N::Session, "S2"),
    );
    link(
        &mut store,
        E::Spawned,
        &r(N::Pilot, "PI"),
        &r(N::Session, "S3"),
    );
    link(&mut store, E::About, &r(N::Memory, "M"), &r(N::File, "FM"));
    for scope in [
        "card:K",
        "note:NT",
        "automation_run:AR",
        "pilot:PI",
        "memory:M",
        "session:keep",
    ] {
        add_scope_row(&store, scope);
    }

    let steps: [(NodeRef, &[&str], &str); 5] = [
        (r(N::Card, "K"), &["card:K", "ticket:T"], "card:K"),
        (r(N::Note, "NT"), &["note:NT"], "note:NT"),
        (
            r(N::AutomationRun, "AR"),
            &["automation_run:AR"],
            "automation_run:AR",
        ),
        (r(N::Pilot, "PI"), &["pilot:PI"], "pilot:PI"),
        (r(N::Memory, "M"), &["memory:M", "file:FM"], "memory:M"),
    ];
    for (anchor, gone, scope) in steps {
        let before = keys(&store);
        let report = delete_checked(&mut store, &anchor);
        let removed: BTreeSet<String> = before.difference(&keys(&store)).cloned().collect();
        assert_eq!(removed, set(gone), "{anchor:?}");
        assert_eq!(report.edges_removed, 1, "{anchor:?}");
        assert!(!scope_rows(&store).contains(scope), "{scope} row must go");
        assert!(scope_rows(&store).contains("session:keep"));
    }
    // The note's project is shared with a session, so it stays.
    assert_eq!(
        keys(&store),
        set(&["project:P", "session:S", "session:S2", "session:S3"])
    );
    match store.delete_anchor(&r(N::Project, "P")) {
        Err(GraphError::NotAnchor(N::Project)) => {}
        other => panic!("expected NotAnchor, got {other:?}"),
    }
    assert!(keys(&store).contains("project:P"));
}

#[test]
fn delete_unknown_session_is_ok() {
    let (_dir, mut store, _path) = temp_store();
    add_node(&mut store, N::Session, "S", 0);
    let dump = store.canonical_dump().unwrap();
    let report = store.delete_session(&SessionId("ghost".into())).unwrap();
    assert_eq!(
        report,
        DeleteReport {
            nodes_removed: 0,
            edges_removed: 0
        }
    );
    assert_eq!(store.canonical_dump().unwrap(), dump);
    assert!(matches!(
        store.delete_session(&SessionId(String::new())),
        Err(GraphError::EmptyKey)
    ));
}

#[test]
fn gc_orphans_counts_closed_edges_and_is_idempotent() {
    let (_dir, mut store, _path) = temp_store();
    for (kind, key) in [
        (N::Session, "S"),
        (N::File, "F"),
        (N::File, "Z"),
        (N::Project, "Q"),
        (N::Pr, "PR"),
        (N::Repo, "R"),
        (N::Repo, "R-orphan"),
    ] {
        add_node(&mut store, kind, key, 0);
    }
    let s = nref(N::Session, "S");
    let mut closed = fact(E::Touched, &s, &nref(N::File, "F"), 10);
    closed.valid_to = Some(ts(20));
    store.upsert_edge(&closed).unwrap();
    // Z and Q only join each other, never an anchor.
    link(
        &mut store,
        E::InProject,
        &nref(N::File, "Z"),
        &nref(N::Project, "Q"),
    );
    // R's PR survives because a session opened it, so R stays too.
    link(&mut store, E::OpenedPr, &s, &nref(N::Pr, "PR"));
    link(
        &mut store,
        E::OfRepo,
        &nref(N::Pr, "PR"),
        &nref(N::Repo, "R"),
    );

    assert_eq!(store.gc_orphans().unwrap(), 3);
    assert_eq!(
        keys(&store),
        set(&["file:F", "pr:PR", "repo:R", "session:S"])
    );
    assert_eq!(store.gc_orphans().unwrap(), 0);
    assert_eq!(dangling_edges(&store), 0);
}

#[test]
fn wipe_removes_db_wal_and_shm() {
    let (dir, mut store, path) = temp_store();
    add_node(&mut store, N::Session, "S", 0);
    let side = |suffix: &str| dir.path().join(format!("{FILE_NAME}{suffix}"));
    assert!(
        side("-wal").exists(),
        "the WAL must exist for this test to prove anything"
    );
    assert!(side("-shm").exists());

    store.wipe().unwrap();
    for suffix in ["", "-wal", "-shm"] {
        assert!(
            !side(suffix).exists(),
            "{FILE_NAME}{suffix} survived the wipe"
        );
    }
    let reopened = GraphStore::open(&path).unwrap();
    assert_eq!(reopened.canonical_dump().unwrap(), "");
    drop(reopened);

    let empty = tempfile::tempdir().unwrap();
    GraphStore::wipe_files(&empty.path().join(FILE_NAME)).unwrap();
}

#[test]
fn a_wipe_that_cannot_remove_the_wal_keeps_the_database() {
    let (dir, mut store, path) = temp_store();
    add_node(&mut store, N::Session, "S", 0);
    drop(store);
    // Closing checkpointed and removed the WAL. A directory in its place
    // makes removing it fail on every OS.
    let wal = dir.path().join(format!("{FILE_NAME}-wal"));
    assert!(!wal.exists());
    std::fs::create_dir(&wal).unwrap();

    assert!(matches!(
        GraphStore::wipe_files(&path),
        Err(GraphError::Io(_))
    ));
    // Deleting the database first would strand a WAL that a fresh graph.db
    // could replay; a failed wipe must leave the database as it was.
    assert!(path.exists(), "graph.db went while its WAL stayed");
    std::fs::remove_dir(&wal).unwrap();
    let reopened = GraphStore::open(&path).unwrap();
    assert!(reopened.node(&nref(N::Session, "S")).unwrap().is_some());
}
