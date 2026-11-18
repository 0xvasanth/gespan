use super::{OrchEffect, SessionLink};
use crate::orchestrator::tree::{Link, Origin, Wake};
use crate::store::Store;
use serde_json::json;
use std::path::Path;

fn session(store: &Store) -> i64 {
    store.create("claude", "/work/repo").unwrap().id
}

fn link(child: i64, parent: i64, root: i64, depth: u8) -> Link {
    Link {
        child,
        parent,
        root,
        depth,
        origin: Origin::Chat,
        wake: Wake::Settled,
    }
}

fn memory() -> Store {
    Store::open(Path::new(":memory:")).unwrap()
}

#[test]
fn links_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.sqlite3");
    let (a, b, c) = {
        let store = Store::open(&path).unwrap();
        let (a, b, c) = (session(&store), session(&store), session(&store));
        store.save_session_link(&link(b, a, a, 1)).unwrap();
        store
            .save_session_link(&Link {
                wake: Wake::Always,
                origin: Origin::Pilot,
                ..link(c, b, a, 2)
            })
            .unwrap();
        (a, b, c)
    };
    let store = Store::open(&path).unwrap();
    let tree = store.session_links().unwrap();
    assert_eq!(tree.link(b), Some(&link(b, a, a, 1)));
    let grandchild = tree.link(c).unwrap();
    assert_eq!(
        (grandchild.parent, grandchild.root, grandchild.depth),
        (b, a, 2)
    );
    assert_eq!(
        (grandchild.wake, grandchild.origin),
        (Wake::Always, Origin::Pilot)
    );
    assert_eq!(tree.descendants(a), [c, b]);
}

#[test]
fn saving_a_link_again_replaces_it() {
    let store = memory();
    let (a, b, c) = (session(&store), session(&store), session(&store));
    store.save_session_link(&link(c, a, a, 1)).unwrap();
    store.save_session_link(&link(c, b, b, 1)).unwrap();
    assert_eq!(store.session_links().unwrap().parent(c), Some(b));
    assert_eq!(store.session_links().unwrap().links().count(), 1);
}

#[test]
fn a_link_must_name_sessions_that_exist() {
    let store = memory();
    let a = session(&store);
    let real = store.persisted_session_id(a).unwrap();
    let insert = |child: String, parent: String| {
        store.orm().unwrap().run(|mut db| async move {
            SessionLink::create()
                .child_session_id(child)
                .parent_session_id(parent)
                .root_session_id("x")
                .depth(1)
                .origin("chat")
                .wake("settled")
                .created_at(1)
                .exec(&mut db)
                .await
                .map(drop)
        })
    };
    let error = insert("missing".into(), real.clone()).unwrap_err();
    assert!(error.contains("FOREIGN KEY"), "{error}");
    let error = insert(real, "missing".into()).unwrap_err();
    assert!(error.contains("FOREIGN KEY"), "{error}");
}

#[test]
fn a_link_is_at_depth_one_or_two() {
    let store = memory();
    let (a, b) = (session(&store), session(&store));
    for depth in [0, 3] {
        let error = store.save_session_link(&link(b, a, a, depth)).unwrap_err();
        assert!(
            error.contains("CHECK constraint failed"),
            "{depth}: {error}"
        );
    }
}

#[test]
fn deleting_a_parent_makes_its_children_roots() {
    let mut store = memory();
    let (a, b, c, d) = (
        session(&store),
        session(&store),
        session(&store),
        session(&store),
    );
    store.save_session_link(&link(b, a, a, 1)).unwrap();
    store.save_session_link(&link(c, b, a, 2)).unwrap();
    store.save_session_link(&link(d, a, a, 1)).unwrap();
    store.delete(a).unwrap();
    let tree = store.session_links().unwrap();
    assert_eq!(tree.link(b), None, "b is a root now");
    assert_eq!(tree.link(d), None, "d is a root now");
    assert_eq!(
        tree.link(c),
        Some(&link(c, b, b, 1)),
        "c hangs off b at depth 1"
    );
}

#[test]
fn deleting_a_child_drops_its_link_receipts_and_effects() {
    let mut store = memory();
    let (a, b) = (session(&store), session(&store));
    store.save_session_link(&link(b, a, a, 1)).unwrap();
    store
        .queue_orch_effect("report:b", "report", b, &json!({}))
        .unwrap();
    store
        .save_command_receipt("b:r1", b, "start", b, &json!({"id": 1}))
        .unwrap();
    store.delete(b).unwrap();
    assert_eq!(store.session_links().unwrap().links().count(), 0);
    assert!(store.pending_orch_effects(None).unwrap().is_empty());
    assert_eq!(store.command_receipt("b:r1").unwrap(), None);
    let left = store
        .orm()
        .unwrap()
        .run(|mut db| async move { OrchEffect::all().exec(&mut db).await })
        .unwrap();
    assert!(left.is_empty());
}

#[test]
fn a_receipt_answers_the_same_command_again_even_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.sqlite3");
    let a = {
        let store = Store::open(&path).unwrap();
        let a = session(&store);
        assert_eq!(store.command_receipt("a:r1").unwrap(), None);
        let first = store
            .save_command_receipt("a:r1", a, "start", a, &json!({"id": 41}))
            .unwrap();
        assert_eq!(first, None, "nothing ran before");
        a
    };
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.command_receipt("a:r1").unwrap(),
        Some(json!({"id": 41}))
    );
    let again = store
        .save_command_receipt("a:r1", a, "start", a, &json!({"id": 99}))
        .unwrap();
    assert_eq!(again, Some(json!({"id": 41})), "the first answer wins");
    assert_eq!(
        store.command_receipt("a:r1").unwrap(),
        Some(json!({"id": 41}))
    );
    let events = store.orch_events(a).unwrap();
    assert_eq!(
        events,
        [("start".to_owned(), json!({"id": 41}))],
        "one event"
    );
}

#[test]
fn events_keep_their_order() {
    let store = memory();
    let a = session(&store);
    for n in 0..5 {
        store
            .append_orch_event("step", a, &json!({ "n": n }))
            .unwrap();
    }
    let ns: Vec<i64> = store
        .orch_events(a)
        .unwrap()
        .into_iter()
        .map(|(_, payload)| payload["n"].as_i64().unwrap())
        .collect();
    assert_eq!(ns, [0, 1, 2, 3, 4]);
}

#[test]
fn an_effect_is_queued_once_and_retried_until_done() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.sqlite3");
    let a = {
        let store = Store::open(&path).unwrap();
        let a = session(&store);
        assert!(store
            .queue_orch_effect("e1", "report", a, &json!({"x": 1}))
            .unwrap());
        assert!(!store
            .queue_orch_effect("e1", "report", a, &json!({"x": 2}))
            .unwrap());
        store.finish_orch_effect("e1", Some("parent busy")).unwrap();
        a
    };
    // After a restart it is still waiting, with the first payload.
    let store = Store::open(&path).unwrap();
    let pending = store.pending_orch_effects(Some("report")).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].session, a);
    assert_eq!(pending[0].payload, json!({"x": 1}));
    assert_eq!(pending[0].attempts, 1);
    assert!(store
        .pending_orch_effects(Some("launch"))
        .unwrap()
        .is_empty());
    store.finish_orch_effect("e1", None).unwrap();
    assert!(store.pending_orch_effects(None).unwrap().is_empty());
    assert!(
        !store
            .queue_orch_effect("e1", "report", a, &json!({}))
            .unwrap(),
        "done stays done"
    );
}

#[test]
fn an_effect_that_keeps_failing_is_given_up() {
    let store = memory();
    let a = session(&store);
    store
        .queue_orch_effect("e2", "launch", a, &json!({}))
        .unwrap();
    for _ in 0..4 {
        store.finish_orch_effect("e2", Some("no")).unwrap();
        assert_eq!(store.pending_orch_effects(None).unwrap().len(), 1);
    }
    store.finish_orch_effect("e2", Some("no")).unwrap();
    assert!(store.pending_orch_effects(None).unwrap().is_empty());
}
