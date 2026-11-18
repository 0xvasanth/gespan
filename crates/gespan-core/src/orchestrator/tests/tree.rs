use crate::agent::AccessMode;
use crate::orchestrator::policy::Code;
use crate::orchestrator::start::{StartInput, WorkspaceChoice};
use crate::orchestrator::tree::{
    decide, Command, Effect, Link, Origin, Outcome, State, Tree, Wake,
};
use std::collections::HashSet;

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

/// 1 → 2 → 4, 1 → 3, and 10 → 11 in another family.
fn family() -> Tree {
    Tree::new([
        link(2, 1, 1, 1),
        link(3, 1, 1, 1),
        link(4, 2, 1, 2),
        link(11, 10, 10, 1),
    ])
}

fn input() -> StartInput {
    StartInput {
        prompt: "Write notes".into(),
        harness: None,
        model: None,
        workspace: WorkspaceChoice::Worktree,
        base: None,
        wake: Wake::Settled,
        access: None,
        title: None,
        request_id: None,
    }
}

fn start(tree: &Tree, live: &HashSet<i64>, caller: i64) -> Result<Vec<Effect>, Code> {
    decide(
        &State { tree, live },
        Command::Start {
            caller,
            caller_access: AccessMode::Auto,
            input: input(),
        },
    )
    .map_err(|refusal| refusal.code)
}

#[test]
fn depth_parent_and_root_follow_the_links() {
    let tree = family();
    assert_eq!(tree.depth(1), 0);
    assert_eq!(tree.depth(4), 2);
    assert_eq!(tree.parent(4), Some(2));
    assert_eq!(tree.parent(1), None);
    assert_eq!(tree.root(4), 1);
    assert_eq!(tree.root(99), 99, "a session with no link is its own root");
    assert_eq!(tree.children(1), [2, 3]);
}

#[test]
fn descendants_come_leaves_first() {
    let tree = family();
    let order = tree.descendants(1);
    assert_eq!(order.len(), 3);
    let at = |id| order.iter().position(|x| *x == id).unwrap();
    assert!(
        at(4) < at(2),
        "a grandchild stops before its parent: {order:?}"
    );
    assert!(tree.descendants(3).is_empty());
    assert!(tree.is_descendant(4, 1));
    assert!(!tree.is_descendant(1, 4));
    assert!(!tree.is_descendant(11, 1));
    assert!(
        !tree.is_descendant(1, 1),
        "a session is not its own descendant"
    );
}

#[test]
fn a_root_chat_starts_a_child_at_depth_one_with_its_own_mode() {
    let tree = Tree::default();
    let effects = start(&tree, &HashSet::new(), 7).unwrap();
    let [Effect::Launch(launch)] = effects.as_slice() else {
        panic!("{effects:?}")
    };
    assert_eq!((launch.parent, launch.root, launch.depth), (7, 7, 1));
    assert_eq!(launch.access, AccessMode::Auto);
}

#[test]
fn a_grandchild_carries_the_root_and_depth_two() {
    let tree = family();
    let effects = start(&tree, &HashSet::new(), 3).unwrap();
    let [Effect::Launch(launch)] = effects.as_slice() else {
        panic!("{effects:?}")
    };
    assert_eq!((launch.parent, launch.root, launch.depth), (3, 1, 2));
}

#[test]
fn depth_two_cannot_start_another() {
    assert_eq!(start(&family(), &HashSet::new(), 4), Err(Code::DepthLimit));
}

#[test]
fn five_live_children_are_the_most() {
    let mut links: Vec<Link> = (0..5).map(|n| link(100 + n, 50, 50, 1)).collect();
    let live: HashSet<i64> = (100..105).collect();
    assert_eq!(
        start(&Tree::new(links.clone()), &live, 50),
        Err(Code::FanoutLimit)
    );
    // A finished child no longer counts.
    let fewer: HashSet<i64> = (100..104).collect();
    assert!(start(&Tree::new(links.clone()), &fewer, 50).is_ok());
    // Grandchildren don't count toward the limit; only direct children do.
    links.truncate(4);
    links.push(link(200, 100, 50, 2));
    let live: HashSet<i64> = [100, 101, 102, 103, 200].into();
    assert!(start(&Tree::new(links), &live, 50).is_ok());
}

#[test]
fn a_broader_mode_is_refused_and_a_narrower_one_kept() {
    let tree = Tree::default();
    let live = HashSet::new();
    let mut wider = input();
    wider.access = Some(AccessMode::FullAccess);
    let refused = decide(
        &State {
            tree: &tree,
            live: &live,
        },
        Command::Start {
            caller: 1,
            caller_access: AccessMode::Supervised,
            input: wider,
        },
    )
    .unwrap_err();
    assert_eq!(refused.code, Code::PermissionEscalation);
    let mut narrower = input();
    narrower.access = Some(AccessMode::ReadOnly);
    let effects = decide(
        &State {
            tree: &tree,
            live: &live,
        },
        Command::Start {
            caller: 1,
            caller_access: AccessMode::Supervised,
            input: narrower,
        },
    )
    .unwrap();
    let [Effect::Launch(launch)] = effects.as_slice() else {
        panic!()
    };
    assert_eq!(launch.access, AccessMode::ReadOnly);
}

fn cancel(tree: &Tree, caller: i64, target: i64) -> Result<Vec<Effect>, Code> {
    let live = HashSet::new();
    decide(
        &State { tree, live: &live },
        Command::Cancel { caller, target },
    )
    .map_err(|refusal| refusal.code)
}

#[test]
fn cancel_stops_the_whole_subtree_leaves_first() {
    let tree = family();
    let effects = cancel(&tree, 1, 2).unwrap();
    assert_eq!(effects, [Effect::Stop(vec![4, 2])]);
    let effects = cancel(&tree, 1, 4).unwrap();
    assert_eq!(effects, [Effect::Stop(vec![4])]);
}

#[test]
fn cancel_reaches_only_the_callers_descendants() {
    let tree = family();
    assert_eq!(cancel(&tree, 1, 1), Err(Code::SelfTarget));
    assert_eq!(cancel(&tree, 2, 3), Err(Code::OutOfScope), "a sibling");
    assert_eq!(cancel(&tree, 4, 2), Err(Code::OutOfScope), "its own parent");
    assert_eq!(
        cancel(&tree, 1, 11),
        Err(Code::OutOfScope),
        "another family"
    );
    assert_eq!(cancel(&tree, 1, 999), Err(Code::OutOfScope), "unknown");
}

fn settled(tree: &Tree, session: i64, outcome: Outcome) -> Vec<Effect> {
    let live = HashSet::new();
    decide(
        &State { tree, live: &live },
        Command::Settled { session, outcome },
    )
    .unwrap()
}

fn with_wake(wake: Wake) -> Tree {
    Tree::new([Link {
        wake,
        ..link(2, 1, 1, 1)
    }])
}

#[test]
fn settled_wakes_the_parent_when_the_child_finishes_or_fails() {
    let tree = with_wake(Wake::Settled);
    for outcome in [Outcome::Finished, Outcome::Failed] {
        assert_eq!(
            settled(&tree, 2, outcome),
            [Effect::Report {
                parent: 1,
                child: 2,
                outcome
            }]
        );
    }
    assert!(settled(&tree, 2, Outcome::WaitingOnUser).is_empty());
    assert!(settled(&tree, 2, Outcome::Cancelled).is_empty());
}

#[test]
fn always_reports_every_stop() {
    let tree = with_wake(Wake::Always);
    for outcome in [
        Outcome::Finished,
        Outcome::Failed,
        Outcome::Cancelled,
        Outcome::WaitingOnUser,
    ] {
        assert_eq!(settled(&tree, 2, outcome).len(), 1, "{outcome:?}");
    }
}

#[test]
fn never_and_roots_report_nothing() {
    let tree = with_wake(Wake::Never);
    assert!(settled(&tree, 2, Outcome::Finished).is_empty());
    assert!(settled(&tree, 1, Outcome::Finished).is_empty(), "no parent");
}

#[test]
fn deleting_a_parent_turns_its_children_into_roots() {
    let mut tree = family();
    let changed = tree.remove(1);
    // 2 and 3 lose their links; 4 now hangs off root 2 at depth 1.
    assert_eq!(tree.link(2), None);
    assert_eq!(tree.link(3), None);
    assert_eq!(tree.link(4), Some(&link(4, 2, 2, 1)));
    assert_eq!(tree.link(11), Some(&link(11, 10, 10, 1)), "untouched");
    assert_eq!(changed.removed, [2, 3]);
    assert_eq!(changed.updated, [link(4, 2, 2, 1)]);
}

#[test]
fn deleting_a_child_removes_only_its_link_and_reroots_below() {
    let mut tree = family();
    let changed = tree.remove(2);
    assert_eq!(tree.link(2), None);
    assert_eq!(tree.link(4), None, "4's parent is gone, so 4 is a root");
    assert_eq!(tree.children(1), [3]);
    assert_eq!(changed.removed, [2, 4]);
    assert!(changed.updated.is_empty());
}
