mod support;

use std::collections::BTreeSet;

use gespan_graph::{EdgeKind, NodeKind};
use EdgeKind as E;
use NodeKind as N;

/// Spec §3.1's endpoint table, written out so a stray allowed pair fails.
fn expected() -> BTreeSet<(EdgeKind, NodeKind, NodeKind)> {
    let mut set = BTreeSet::new();
    for src in [
        N::Session,
        N::Worktree,
        N::Branch,
        N::File,
        N::Card,
        N::Note,
    ] {
        set.insert((E::InProject, src, N::Project));
    }
    for src in [N::Pr, N::Ticket] {
        set.insert((E::OfRepo, src, N::Repo));
    }
    set.insert((E::OnBranch, N::Session, N::Branch));
    set.insert((E::RanIn, N::Session, N::Worktree));
    set.insert((E::Touched, N::Session, N::File));
    set.insert((E::Read, N::Session, N::File));
    set.insert((E::OpenedPr, N::Session, N::Pr));
    set.insert((E::LinksTicket, N::Session, N::Ticket));
    for dst in [N::Session, N::Pilot, N::Pr, N::Ticket] {
        set.insert((E::Tracks, N::Card, dst));
    }
    set.insert((E::Blocks, N::Card, N::Card));
    for edge in [E::Authored, E::Assigned] {
        for dst in [N::Pr, N::Ticket] {
            set.insert((edge, N::Person, dst));
        }
    }
    for src in [N::Pilot, N::Session, N::AutomationRun] {
        set.insert((E::Spawned, src, N::Session));
    }
    set.insert((E::NotedFrom, N::Note, N::Session));
    for dst in N::ALL {
        set.insert((E::About, N::Memory, dst));
    }
    set
}

#[test]
fn endpoint_rules_match_the_table_exactly() {
    let expected = expected();
    let mut checked = 0;
    for edge in EdgeKind::ALL {
        for src in NodeKind::ALL {
            for dst in NodeKind::ALL {
                checked += 1;
                assert_eq!(
                    edge.allows(src, dst),
                    expected.contains(&(edge, src, dst)),
                    "{edge:?}: {src:?} -> {dst:?}"
                );
            }
        }
    }
    assert_eq!(checked, 15 * 14 * 14);
    // `about` allows memory to every kind and nothing else.
    for src in NodeKind::ALL.into_iter().filter(|k| *k != N::Memory) {
        assert!(NodeKind::ALL.iter().all(|dst| !E::About.allows(src, *dst)));
    }
}

#[test]
fn anchors_are_the_kinds_that_mirror_a_source_row() {
    let anchors: BTreeSet<NodeKind> = NodeKind::ALL
        .into_iter()
        .filter(|k| k.is_anchor())
        .collect();
    let expected: BTreeSet<NodeKind> = [
        N::Session,
        N::Pilot,
        N::AutomationRun,
        N::Card,
        N::Note,
        N::Memory,
    ]
    .into_iter()
    .collect();
    assert_eq!(anchors, expected);
}

#[test]
fn store_enforces_endpoints_and_self_loops() {
    use gespan_graph::GraphError;
    use support::*;
    let (_dir, mut store, _path) = temp_store();
    add_node(&mut store, N::Session, "s1", 0);
    add_node(&mut store, N::File, "f", 0);
    add_node(&mut store, N::Card, "c", 0);
    let before = store.canonical_dump().unwrap();
    let s = nref(N::Session, "s1");
    let f = nref(N::File, "f");
    let c = nref(N::Card, "c");

    match store.upsert_edge(&fact(E::Touched, &f, &s, 0)) {
        Err(GraphError::InvalidEndpoints {
            edge: E::Touched,
            src: N::File,
            dst: N::Session,
        }) => {}
        other => panic!("expected InvalidEndpoints, got {other:?}"),
    }
    match store.upsert_edge(&fact(E::Blocks, &c, &c, 0)) {
        Err(GraphError::SelfLoop(node)) => assert_eq!(node, c),
        other => panic!("expected SelfLoop, got {other:?}"),
    }
    // Endpoint rules come before existence: a wrong pair of missing nodes is
    // still InvalidEndpoints.
    let ghost = nref(N::Pr, "ghost");
    assert!(matches!(
        store.upsert_edge(&fact(E::Blocks, &ghost, &c, 0)),
        Err(GraphError::InvalidEndpoints { .. })
    ));
    assert_eq!(store.canonical_dump().unwrap(), before);
}
