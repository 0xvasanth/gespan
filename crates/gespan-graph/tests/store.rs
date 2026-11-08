mod support;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use gespan_graph::{
    CloseOutcome, EdgeKind, GraphError, GraphStore, NodeFact, NodeKind, Scope, TimeBasis,
};
use serde_json::{json, Value};
use support::*;

#[test]
fn node_upsert_keeps_rowid_and_merges_times() {
    let (_dir, mut store, _path) = temp_store();
    let s = nref(NodeKind::Session, "s1");
    let upsert = |store: &mut GraphStore, label: &str, p, at| {
        store
            .upsert_node(&NodeFact {
                node: s.clone(),
                label: label.into(),
                props: p,
                seen_at: ts(at),
            })
            .unwrap()
    };
    let id = upsert(&mut store, "one", props(&[]), 10);
    add_node(&mut store, NodeKind::File, "f", 10);
    store
        .upsert_edge(&fact(EdgeKind::Touched, &s, &nref(NodeKind::File, "f"), 10))
        .unwrap();

    assert_eq!(upsert(&mut store, "two", props(&[]), 5), id);
    assert_eq!(
        upsert(&mut store, "three", props(&[("a", json!(1))]), 20),
        id
    );

    let (label, props_json, created, updated): (String, String, i64, i64) = store
        .connection()
        .query_row(
            "SELECT label, props_json, created_at, updated_at FROM graph_nodes WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!((created, updated), (BASE + 5, BASE + 20));
    assert_eq!(label, "three");
    assert_eq!(props_json, r#"{"a":1}"#);
    // INSERT OR REPLACE would have given a new rowid and cascaded the edge away.
    assert_eq!(count(&store, "graph_edges"), 1);
    assert_eq!(store.node(&s).unwrap(), Some(id));
}

#[test]
fn edge_to_missing_node_is_refused() {
    let (_dir, mut store, _path) = temp_store();
    let s = nref(NodeKind::Session, "s1");
    let f = nref(NodeKind::File, "f");
    match store.upsert_edge(&fact(EdgeKind::Touched, &s, &f, 0)) {
        Err(GraphError::MissingNode(missing)) => assert_eq!(missing, s, "src is checked first"),
        other => panic!("expected MissingNode, got {other:?}"),
    }
    add_node(&mut store, NodeKind::Session, "s1", 0);
    match store.upsert_edge(&fact(EdgeKind::Touched, &s, &f, 0)) {
        Err(GraphError::MissingNode(missing)) => assert_eq!(missing, f),
        other => panic!("expected MissingNode, got {other:?}"),
    }
    assert_eq!(count(&store, "graph_edges"), 0);
    assert_eq!(
        count(&store, "graph_nodes"),
        1,
        "the missing endpoint is not created"
    );
}

#[test]
fn edge_upsert_is_idempotent_and_props_update_in_place() {
    let (_dir, mut store, _path) = temp_store();
    add_node(&mut store, NodeKind::Session, "s1", 0);
    add_node(&mut store, NodeKind::File, "f", 0);
    let s = nref(NodeKind::Session, "s1");
    let f = nref(NodeKind::File, "f");
    let first = fact(EdgeKind::Touched, &s, &f, 10);

    let id = store.upsert_edge(&first).unwrap();
    assert_eq!(store.upsert_edge(&first).unwrap(), id);
    assert_eq!(count(&store, "graph_edges"), 1);

    let mut changed = fact(EdgeKind::Touched, &s, &f, 30);
    changed.props = props(&[("lines", json!(12))]);
    assert_eq!(store.upsert_edge(&changed).unwrap(), id);
    let (from, to, _, props_json) = edge_row(&store, id);
    assert_eq!(props_json, r#"{"lines":12}"#);
    assert_eq!(
        from,
        BASE + 10,
        "props are attributes; the fact's start stays"
    );
    assert_eq!(to, None);
    assert_eq!(count(&store, "graph_edges"), 1);

    let mut other_scope = first.clone();
    other_scope.scope = Scope::Owner(nref(NodeKind::Card, "c"));
    let second = store.upsert_edge(&other_scope).unwrap();
    assert_ne!(second, id);
    assert_eq!(count(&store, "graph_edges"), 2);
}

#[test]
fn structural_edge_keeps_earliest_valid_from() {
    for (scope, want) in [
        (Scope::Structural, (BASE + 20, "observed_by")),
        (
            Scope::Owner(nref(NodeKind::Session, "s1")),
            (BASE + 50, "owner_created"),
        ),
    ] {
        let (_dir, mut store, _path) = temp_store();
        add_node(&mut store, NodeKind::Session, "s1", 0);
        add_node(&mut store, NodeKind::Project, "p", 0);
        let s = nref(NodeKind::Session, "s1");
        let p = nref(NodeKind::Project, "p");
        let mut id = None;
        for (from, basis) in [
            (50, TimeBasis::OwnerCreated),
            (20, TimeBasis::ObservedBy),
            (80, TimeBasis::Native),
        ] {
            let mut f = fact(EdgeKind::InProject, &s, &p, from);
            f.basis = basis;
            f.scope = scope.clone();
            let got = store.upsert_edge(&f).unwrap();
            assert_eq!(*id.get_or_insert(got), got);
        }
        let (from, _, basis, _) = edge_row(&store, id.unwrap());
        assert_eq!((from, basis.as_str()), want, "{scope:?}");
        assert_eq!(count(&store, "graph_edges"), 1);
    }
}

#[test]
fn close_edge_uses_the_given_time() {
    let (_dir, mut store, _path) = temp_store();
    add_node(&mut store, NodeKind::Session, "s1", 0);
    add_node(&mut store, NodeKind::Branch, "main", 0);
    let s = nref(NodeKind::Session, "s1");
    let b = nref(NodeKind::Branch, "main");
    let open = fact(EdgeKind::OnBranch, &s, &b, 10);
    let key = key_of(&open);
    let id = store.upsert_edge(&open).unwrap();

    assert_eq!(
        store.close_edge(&key, ts(100)).unwrap(),
        CloseOutcome::Closed(id)
    );
    assert_eq!(edge_row(&store, id).1, Some(BASE + 100));
    assert_eq!(
        store.close_edge(&key, ts(200)).unwrap(),
        CloseOutcome::NotOpen
    );
    assert_eq!(
        edge_row(&store, id).1,
        Some(BASE + 100),
        "a closed row is never moved"
    );

    let reopened = store
        .upsert_edge(&fact(EdgeKind::OnBranch, &s, &b, 300))
        .unwrap();
    assert_ne!(reopened, id);
    assert_eq!(
        count(&store, "graph_edges"),
        2,
        "closed history plus the open row"
    );

    // A caller clock behind the fact's start clamps to valid_from.
    assert_eq!(
        store.close_edge(&key, ts(250)).unwrap(),
        CloseOutcome::Closed(reopened)
    );
    let (from, to, _, _) = edge_row(&store, reopened);
    assert_eq!(to, Some(from));
    assert_eq!(from, BASE + 300);

    let mut unknown = key.clone();
    unknown.kind = EdgeKind::RanIn;
    assert_eq!(
        store.close_edge(&unknown, ts(400)).unwrap(),
        CloseOutcome::NotOpen
    );
}

#[test]
fn already_closed_facts_converge() {
    let (_dir, mut store, _path) = temp_store();
    add_node(&mut store, NodeKind::Session, "s1", 0);
    add_node(&mut store, NodeKind::Worktree, "w1", 0);
    add_node(&mut store, NodeKind::Worktree, "w2", 0);
    let s = nref(NodeKind::Session, "s1");

    // An open row is closed by the arriving closed fact.
    let open = fact(EdgeKind::RanIn, &s, &nref(NodeKind::Worktree, "w1"), 10);
    let id = store.upsert_edge(&open).unwrap();
    let mut closed = open.clone();
    closed.valid_to = Some(ts(40));
    assert_eq!(store.upsert_edge(&closed).unwrap(), id);
    assert_eq!(edge_row(&store, id).1, Some(BASE + 40));
    assert_eq!(
        store.upsert_edge(&closed).unwrap(),
        id,
        "the same closed fact again is a no-op"
    );
    assert_eq!(count(&store, "graph_edges"), 1);

    // With no row at all, it is inserted closed, once.
    let mut fresh = fact(EdgeKind::RanIn, &s, &nref(NodeKind::Worktree, "w2"), 15);
    fresh.valid_to = Some(ts(25));
    let id2 = store.upsert_edge(&fresh).unwrap();
    assert_eq!(
        edge_row(&store, id2),
        (BASE + 15, Some(BASE + 25), "native".into(), "{}".into())
    );
    let dump = store.canonical_dump().unwrap();
    assert_eq!(store.upsert_edge(&fresh).unwrap(), id2);
    assert_eq!(count(&store, "graph_edges"), 2);
    assert_eq!(store.canonical_dump().unwrap(), dump);
}

/// Inserts the same small graph in one of two orders, with one tricky label.
fn build(store: &mut GraphStore, reversed: bool) {
    let mut nodes = vec![
        (NodeKind::Project, "p"),
        (NodeKind::Session, "s1"),
        (NodeKind::Session, "s2"),
        (NodeKind::File, "f"),
    ];
    if reversed {
        nodes.reverse();
    }
    for (kind, key) in &nodes {
        store
            .upsert_node(&NodeFact {
                node: nref(*kind, key),
                label: if *key == "f" {
                    "tab\there\nnew \"q\"".into()
                } else {
                    key.to_string()
                },
                props: props(&[]),
                seen_at: ts(1),
            })
            .unwrap();
    }
    let p = nref(NodeKind::Project, "p");
    let f = nref(NodeKind::File, "f");
    let mut edges = vec![
        fact(EdgeKind::Touched, &nref(NodeKind::Session, "s1"), &f, 2),
        fact(EdgeKind::Touched, &nref(NodeKind::Session, "s2"), &f, 3),
        fact(EdgeKind::InProject, &nref(NodeKind::Session, "s1"), &p, 4),
    ];
    edges[2].scope = Scope::Structural;
    if reversed {
        edges.reverse();
    }
    for e in &edges {
        store.upsert_edge(e).unwrap();
    }
}

#[test]
fn canonical_dump_is_total_and_order_independent() {
    let (_d1, mut a, _p1) = temp_store();
    let (_d2, mut b, _p2) = temp_store();
    build(&mut a, false);
    // b gets rowid gaps: rows inserted then removed before the real ones.
    add_node(&mut b, NodeKind::Project, "gone", 1);
    add_node(&mut b, NodeKind::Session, "gone", 1);
    b.upsert_edge(&fact(
        EdgeKind::InProject,
        &nref(NodeKind::Session, "gone"),
        &nref(NodeKind::Project, "gone"),
        1,
    ))
    .unwrap();
    b.connection()
        .execute("DELETE FROM graph_nodes WHERE key = 'gone'", [])
        .unwrap();
    build(&mut b, true);
    // Values the dump must leave out.
    for s in [&a, &b] {
        s.connection()
            .execute("UPDATE graph_nodes SET last_seen_run = 987654321", [])
            .unwrap();
        s.connection()
            .execute(
                "UPDATE graph_edges SET last_seen_run = 987654321, created_at = 123456789",
                [],
            )
            .unwrap();
    }

    let dump = a.canonical_dump().unwrap();
    assert_eq!(dump, b.canonical_dump().unwrap());
    let expected = [
        json!([
            "E",
            "in_project",
            "session",
            "s1",
            "project",
            "p",
            "backfill:blocks",
            "structural",
            BASE + 4,
            "native",
            null,
            {}
        ]),
        json!([
            "E",
            "touched",
            "session",
            "s1",
            "file",
            "f",
            "backfill:blocks",
            "session:s1",
            BASE + 2,
            "native",
            null,
            {}
        ]),
        json!([
            "E",
            "touched",
            "session",
            "s2",
            "file",
            "f",
            "backfill:blocks",
            "session:s2",
            BASE + 3,
            "native",
            null,
            {}
        ]),
        json!([
            "N",
            "file",
            "f",
            "tab\there\nnew \"q\"",
            {},
            BASE + 1,
            BASE + 1
        ]),
        json!(["N", "project", "p", "p", {}, BASE + 1, BASE + 1]),
        json!(["N", "session", "s1", "s1", {}, BASE + 1, BASE + 1]),
        json!(["N", "session", "s2", "s2", {}, BASE + 1, BASE + 1]),
    ];
    let lines: Vec<&str> = dump.split('\n').collect();
    assert_eq!(lines.len(), expected.len(), "one line per row:\n{dump}");
    for (line, want) in lines.iter().zip(&expected) {
        let parsed: Value = serde_json::from_str(line).unwrap();
        assert_eq!(&parsed, want);
    }
    assert!(!dump.contains("987654321") && !dump.contains("123456789"));
}

#[test]
fn second_handle_waits_for_writer() {
    let (_dir, writer, path) = temp_store();
    let mut second = GraphStore::open(&path).unwrap();
    let (locked, wait) = mpsc::channel();
    let hold = thread::spawn(move || {
        writer
            .connection()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        locked.send(()).unwrap();
        thread::sleep(Duration::from_millis(300));
        writer.connection().execute_batch("COMMIT").unwrap();
        writer
    });
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    let id = add_node(&mut second, NodeKind::Project, "p", 0);
    let waited = started.elapsed();
    let writer = hold.join().unwrap();
    assert!(
        waited >= Duration::from_millis(200),
        "the write should have waited for the lock: {waited:?}"
    );
    assert_eq!(
        second.node(&nref(NodeKind::Project, "p")).unwrap(),
        Some(id)
    );
    drop(writer);
}

#[test]
fn keys_and_labels_round_trip_verbatim() {
    let (_dir, mut store, _path) = temp_store();
    let key = "🦀/cafe\u{301}.rs";
    let long = "x".repeat(10_000);
    let tricky = "tab\there\nnew \"q\"";
    for (k, label) in [(key, long.as_str()), ("tricky", tricky)] {
        store
            .upsert_node(&NodeFact {
                node: nref(NodeKind::File, k),
                label: label.into(),
                props: props(&[]),
                seen_at: ts(0),
            })
            .unwrap();
    }
    assert!(store.node(&nref(NodeKind::File, key)).unwrap().is_some());
    assert_eq!(
        store.node(&nref(NodeKind::File, "🦀/café.rs")).unwrap(),
        None,
        "NFC is a different key"
    );

    let dump = store.canonical_dump().unwrap();
    let lines: Vec<Value> = dump
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    let find = |k: &str| lines.iter().find(|l| l[2] == k).unwrap().clone();
    assert_eq!(find(key)[3], Value::String(long));
    assert_eq!(find("tricky")[3], Value::String(tricky.into()));
}
