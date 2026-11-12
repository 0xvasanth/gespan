//! Reviewing the log from My tasks: approve, reject, replace from a flag,
//! and the guard against a proposal revised after the user saw it.
use crate::decisions::review::{apply, shown_hash, similar_active, Review};
use crate::decisions::service::{self, isolated};
use crate::decisions::{DecisionStore, Edit, Entry, Kind, NewEntry, Scope, Source, Status};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const CHANGED: &str = "This proposal changed. Review it again.";

fn repo() -> Scope {
    Scope::Repo("github.com/o/r@0123456789ab".into())
}

/// A pending proposal the user answered in session 7.
fn propose(store: &DecisionStore, scope: &Scope, title: &str, text: &str) -> Entry {
    store
        .propose(
            NewEntry {
                kind: Kind::Decision,
                scope: scope.clone(),
                title: title.into(),
                text: text.into(),
                why: "the user said so".into(),
                user_answer: Some("Yes, do that".into()),
                source: Source {
                    session: Some(7),
                    question_ref: Some("q1".into()),
                    ..Source::default()
                },
            },
            10,
        )
        .unwrap()
}

fn active(store: &DecisionStore, scope: &Scope, title: &str, text: &str) -> Entry {
    let e = propose(store, scope, title, text);
    store.approve(e.id, None, false, &[], 20).unwrap()
}

fn entry(store: &DecisionStore, id: i64) -> Entry {
    store.get(id).unwrap().unwrap().0
}

fn approve(e: &Entry) -> Review {
    Review::Approve {
        shown: shown_hash(e),
        edit: None,
        pinned: false,
        replaces: vec![],
    }
}

/// The agent rewrites its proposal (through the toolkit's revise path).
fn revise(store: &DecisionStore, e: &Entry, text: &str) -> Entry {
    store
        .revise_pending(
            e.id,
            Edit {
                title: e.title.clone(),
                text: text.into(),
                why: e.why.clone(),
            },
            e.user_answer.as_deref(),
            &e.source,
        )
        .unwrap()
}

#[test]
fn approve_with_edit_and_pin() {
    let _guard = isolated();
    service::with(|store| {
        let e = propose(store, &repo(), "Errors", "Use anyhow everywhere");
        apply(
            store,
            e.id,
            Review::Approve {
                shown: shown_hash(&e),
                edit: Some(Edit {
                    title: "Errors as values".into(),
                    text: "Return Result<T, MigrationError>; never panic".into(),
                    why: "panics hid failures in A".into(),
                }),
                pinned: true,
                replaces: vec![],
            },
            50,
        )
        .unwrap();
        let after = entry(store, e.id);
        assert_eq!(after.status, Status::Active);
        assert!(after.pinned);
        assert_eq!(after.title, "Errors as values");
        assert_eq!(after.text, "Return Result<T, MigrationError>; never panic");
        assert_eq!(after.why, "panics hid failures in A");
        assert_eq!(after.decided_ms, Some(50));
        // The user's own answer stays beside the edited wording.
        assert_eq!(after.user_answer.as_deref(), Some("Yes, do that"));
    })
    .unwrap();
}

#[test]
fn an_edit_on_the_card_is_redacted_and_capped() {
    let _guard = isolated();
    service::with(|store| {
        let e = propose(store, &repo(), "Keys", "Keep keys out of logs");
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        apply(
            store,
            e.id,
            Review::Approve {
                shown: shown_hash(&e),
                edit: Some(Edit {
                    title: format!("Use {secret}"),
                    text: "x".repeat(2_000),
                    why: "because".into(),
                }),
                pinned: false,
                replaces: vec![],
            },
            50,
        )
        .unwrap();
        let after = entry(store, e.id);
        assert!(!after.title.contains(secret), "{}", after.title);
        assert_eq!(after.text.chars().count(), 1_000);
    })
    .unwrap();
}

#[test]
fn approve_replacing_two_marks_both_replaced() {
    let _guard = isolated();
    service::with(|store| {
        let a = active(store, &repo(), "Tabs", "Indent with tabs");
        let b = active(store, &repo(), "Width", "Tabs are four wide");
        let keep = active(store, &repo(), "Naming", "snake_case files");
        let new = propose(store, &repo(), "Spaces", "Indent with two spaces");
        apply(
            store,
            new.id,
            Review::Approve {
                shown: shown_hash(&new),
                edit: None,
                pinned: false,
                replaces: vec![a.id, b.id],
            },
            60,
        )
        .unwrap();
        for old in [a.id, b.id] {
            let e = entry(store, old);
            assert_eq!(e.status, Status::Replaced);
            assert_eq!(e.replaced_by, Some(new.id));
        }
        assert_eq!(entry(store, keep.id).status, Status::Active);
        let ids: Vec<i64> = store
            .active(&repo())
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        assert!(ids.contains(&new.id) && ids.contains(&keep.id), "{ids:?}");
        assert!(!ids.contains(&a.id) && !ids.contains(&b.id), "{ids:?}");
    })
    .unwrap();
}

#[test]
fn approve_refuses_when_the_proposal_changed_after_it_was_shown() {
    let _guard = isolated();
    service::with(|store| {
        let old = active(store, &repo(), "Errors", "Errors use anyhow");
        let e = propose(store, &repo(), "Errors", "Errors use thiserror");
        let shown = approve(&e);
        revise(store, &e, "Errors are plain strings");
        let Review::Approve { shown, .. } = shown else {
            unreachable!()
        };
        let review = Review::Approve {
            shown,
            edit: None,
            pinned: true,
            replaces: vec![old.id],
        };
        assert_eq!(apply(store, e.id, review, 50).unwrap_err(), CHANGED);
        let after = entry(store, e.id);
        assert_eq!(after.status, Status::Pending);
        assert_eq!(after.text, "Errors are plain strings");
        assert!(!after.pinned);
        // Nothing it would have replaced moved either.
        assert_eq!(entry(store, old.id).status, Status::Active);
        // Shown again, the revised proposal can be approved.
        apply(store, e.id, approve(&after), 60).unwrap();
        assert_eq!(entry(store, e.id).status, Status::Active);
    })
    .unwrap();
}

#[test]
fn reject_refuses_when_the_proposal_changed_after_it_was_shown() {
    let _guard = isolated();
    service::with(|store| {
        let e = propose(store, &repo(), "Errors", "Errors use thiserror");
        let shown = shown_hash(&e);
        let revised = revise(store, &e, "Errors are plain strings");
        assert_eq!(
            apply(store, e.id, Review::Reject { shown }, 50).unwrap_err(),
            CHANGED
        );
        assert_eq!(entry(store, e.id).status, Status::Pending);
        apply(
            store,
            e.id,
            Review::Reject {
                shown: shown_hash(&revised),
            },
            60,
        )
        .unwrap();
        assert_eq!(entry(store, e.id).status, Status::Rejected);
    })
    .unwrap();
}

#[test]
fn a_changed_user_answer_alone_refuses_the_approval() {
    let _guard = isolated();
    service::with(|store| {
        let e = propose(store, &repo(), "Errors", "Errors use thiserror");
        let shown = shown_hash(&e);
        store
            .revise_pending(
                e.id,
                Edit {
                    title: e.title.clone(),
                    text: e.text.clone(),
                    why: e.why.clone(),
                },
                Some("No, never do that"),
                &e.source,
            )
            .unwrap();
        let review = Review::Approve {
            shown,
            edit: None,
            pinned: false,
            replaces: vec![],
        };
        assert_eq!(apply(store, e.id, review, 50).unwrap_err(), CHANGED);
        assert_eq!(entry(store, e.id).status, Status::Pending);
    })
    .unwrap();
}

#[test]
fn replace_from_flag_creates_active_successor_and_dismisses_flag() {
    let _guard = isolated();
    service::with(|store| {
        let old = active(store, &repo(), "No panics", "Never panic");
        let flag = store
            .flag(old.id, 9, "FFI boundaries must abort instead", 30)
            .unwrap();
        let other = store.flag(old.id, 9, "another concern", 31).unwrap();
        let new = Edit {
            title: "No panics except FFI".into(),
            text: "Never panic, except at FFI boundaries".into(),
            why: "unwinding across FFI is undefined".into(),
        };
        apply(
            store,
            old.id,
            Review::ReplaceFromFlag { flag: flag.id, new },
            70,
        )
        .unwrap();
        let before = entry(store, old.id);
        assert_eq!(before.status, Status::Replaced);
        let successor = before.replaced_by.expect("a successor");
        let (next, history) = store.get(successor).unwrap().unwrap();
        assert_eq!(next.status, Status::Active);
        assert_eq!(next.kind, Kind::Decision);
        assert_eq!(next.scope, repo());
        assert_eq!(next.title, "No panics except FFI");
        assert_eq!(next.text, "Never panic, except at FFI boundaries");
        assert_eq!(next.supersedes, Some(old.id));
        assert_eq!(history.iter().map(|e| e.id).collect::<Vec<_>>(), [old.id]);
        // Both flags on the replaced entry are closed: the one acted on and
        // the other agent's, which was about wording that no longer applies.
        assert!(store.open_flags().unwrap().is_empty(), "{other:?}");
        // Nothing is left waiting for review.
        assert!(store.pending().unwrap().is_empty());
    })
    .unwrap();
}

#[test]
fn replace_from_flag_keeps_other_entries_flags_and_the_pin() {
    let _guard = isolated();
    service::with(|store| {
        let old = active(store, &repo(), "No panics", "Never panic");
        let pinned = propose(store, &repo(), "Tabs", "Indent with tabs");
        let pinned = store.approve(pinned.id, None, true, &[], 20).unwrap();
        let elsewhere = store.flag(old.id, 9, "disagree", 30).unwrap();
        let flag = store.flag(pinned.id, 9, "spaces are the norm", 31).unwrap();
        let new = Edit {
            title: "Spaces".into(),
            text: "Indent with spaces".into(),
            why: "the formatter does".into(),
        };
        apply(
            store,
            pinned.id,
            Review::ReplaceFromFlag { flag: flag.id, new },
            70,
        )
        .unwrap();
        let successor = entry(store, pinned.id).replaced_by.unwrap();
        assert!(entry(store, successor).pinned, "the pin carries over");
        let open: Vec<i64> = store
            .open_flags()
            .unwrap()
            .iter()
            .map(|(f, _)| f.id)
            .collect();
        assert_eq!(open, [elsewhere.id]);
    })
    .unwrap();
}

/// A database that refuses the last step (closing the flags), so the
/// replacement fails after its successor and the replaced mark were
/// written: the transaction must take both back.
#[test]
fn replace_from_flag_that_fails_part_way_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let store = DecisionStore::open(&path, crate::decisions::Config::app()).unwrap();
    let old = active(&store, &repo(), "No panics", "Never panic");
    let flag = store.flag(old.id, 9, "FFI must abort", 30).unwrap();
    let rows = |store: &DecisionStore| store.search(&repo(), "panic", 50).unwrap().len();
    let before = rows(&store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER refuse_dismiss BEFORE UPDATE ON decision_flags
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .unwrap();
    let new = Edit {
        title: "No panics except FFI".into(),
        text: "Never panic, except at FFI boundaries".into(),
        why: "unwinding across FFI is undefined".into(),
    };
    assert!(apply(
        &store,
        old.id,
        Review::ReplaceFromFlag { flag: flag.id, new },
        70
    )
    .is_err());
    let after = entry(&store, old.id);
    assert_eq!(after.status, Status::Active);
    assert_eq!(after.replaced_by, None);
    assert_eq!(rows(&store), before, "no successor was kept");
    assert!(store.pending().unwrap().is_empty());
    let open: Vec<i64> = store
        .open_flags()
        .unwrap()
        .iter()
        .map(|(f, _)| f.id)
        .collect();
    assert_eq!(open, [flag.id]);
}

#[test]
fn replace_from_flag_refuses_an_entry_no_longer_active() {
    let _guard = isolated();
    service::with(|store| {
        let old = active(store, &repo(), "No panics", "Never panic");
        let flag = store.flag(old.id, 9, "disagree", 30).unwrap();
        store.archive(old.id, 40).unwrap();
        let new = Edit {
            title: "t".into(),
            text: "x".into(),
            why: "y".into(),
        };
        let replace = Review::ReplaceFromFlag { flag: flag.id, new };
        assert!(apply(store, old.id, replace, 70).is_err());
        assert_eq!(entry(store, old.id).status, Status::Archived);
        assert!(store.pending().unwrap().is_empty());
    })
    .unwrap();
}

#[test]
fn a_flag_of_another_entry_changes_nothing() {
    let _guard = isolated();
    service::with(|store| {
        let a = active(store, &repo(), "No panics", "Never panic");
        let b = active(store, &repo(), "Tabs", "Indent with tabs");
        let flag = store.flag(b.id, 9, "spaces are the norm", 30).unwrap();
        let new = Edit {
            title: "t".into(),
            text: "x".into(),
            why: "y".into(),
        };
        let replace = Review::ReplaceFromFlag { flag: flag.id, new };
        assert!(apply(store, a.id, replace, 70).is_err());
        assert!(apply(store, a.id, Review::DismissFlag { flag: flag.id }, 70).is_err());
        assert_eq!(entry(store, a.id).status, Status::Active);
        assert_eq!(store.open_flags().unwrap().len(), 1);
        assert!(store.pending().unwrap().is_empty());
    })
    .unwrap();
}

#[test]
fn dismiss_flag_keeps_the_decision_active() {
    let _guard = isolated();
    service::with(|store| {
        let a = active(store, &repo(), "No panics", "Never panic");
        let flag = store.flag(a.id, 9, "disagree", 30).unwrap();
        apply(store, a.id, Review::DismissFlag { flag: flag.id }, 40).unwrap();
        assert!(store.open_flags().unwrap().is_empty());
        assert_eq!(entry(store, a.id).status, Status::Active);
    })
    .unwrap();
}

#[test]
fn similar_active_offers_same_scope_only() {
    let _guard = isolated();
    service::with(|store| {
        let other = Scope::Repo("github.com/o/other@0123456789ab".into());
        let here = active(store, &repo(), "Error handling", "Errors use anyhow");
        let elsewhere = active(store, &other, "Error handling", "Errors use anyhow");
        let global = store
            .propose(
                NewEntry {
                    kind: Kind::Preference,
                    scope: Scope::Global,
                    title: "Error handling".into(),
                    text: "Errors use anyhow".into(),
                    why: "taste".into(),
                    user_answer: None,
                    source: Source::default(),
                },
                10,
            )
            .unwrap();
        store.approve(global.id, None, false, &[], 20).unwrap();
        // Pending, rejected and unrelated entries in the same scope are no
        // candidates either.
        let pending = propose(store, &repo(), "Error handling", "Errors use thiserror");
        let rejected = propose(store, &repo(), "Error handling", "Errors are strings");
        store.reject(rejected.id, 30).unwrap();
        active(store, &repo(), "Formatting", "Run rustfmt");
        let new = propose(store, &repo(), "Errors", "Error handling uses anyhow");
        let ids: Vec<i64> = similar_active(store, &new, 3)
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [here.id], "{ids:?}");
        assert!(!ids.contains(&elsewhere.id) && !ids.contains(&global.id));
        assert!(!ids.contains(&pending.id) && !ids.contains(&new.id));
    })
    .unwrap();
}

#[test]
fn similar_active_returns_at_most_the_limit() {
    let _guard = isolated();
    service::with(|store| {
        for n in 0..5 {
            active(store, &repo(), &format!("Errors {n}"), "Errors use anyhow");
        }
        let new = propose(store, &repo(), "Errors", "Errors use anyhow");
        assert_eq!(similar_active(store, &new, 3).len(), 3);
    })
    .unwrap();
}

/// The counting hook is installed under `isolated()`'s lock, which every
/// test touching the process-wide hook holds, so no other test races it.
#[test]
fn apply_triggers_regenerate() {
    let _guard = isolated();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    service::set_regenerate(Box::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    service::with(|store| {
        let e = propose(store, &repo(), "Tabs", "Indent with tabs");
        apply(store, e.id, approve(&e), 50).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A review that fails (already approved) writes nothing.
        assert!(apply(store, e.id, approve(&e), 60).is_err());
        let shown = shown_hash(&e);
        assert!(apply(store, e.id, Review::Reject { shown }, 60).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let r = propose(store, &repo(), "Width", "Four wide");
        let shown = shown_hash(&r);
        apply(store, r.id, Review::Reject { shown }, 70).unwrap();
        assert_eq!(entry(store, r.id).status, Status::Rejected);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let flag = store.flag(e.id, 9, "disagree", 80).unwrap();
        apply(store, e.id, Review::DismissFlag { flag: flag.id }, 90).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    })
    .unwrap();
}

/// With search by meaning on, a rewording that shares no whole word with the
/// proposal is offered as what it might replace; keyword search misses it.
/// It still never leaves the proposal's own scope.
#[test]
fn similar_active_finds_rewordings_by_meaning() {
    let _guard = isolated();
    service::with(|store| {
        let other = Scope::Repo("github.com/o/other@0123456789ab".into());
        let reworded = active(store, &repo(), "Branching", "Rebasing branch history");
        let unrelated = active(store, &repo(), "Deploys", "Never deploy on Fridays");
        let elsewhere = active(store, &other, "Rebase", "Rebase branches");
        let e = propose(store, &repo(), "Rebase", "Rebase branches");
        assert!(similar_active(store, &e, 3).is_empty(), "no shared word");
        store
            .set_embedder(Some(Arc::new(crate::decisions::HashEmbedder::new())))
            .unwrap();
        let found: Vec<i64> = similar_active(store, &e, 3).iter().map(|x| x.id).collect();
        // The rewording, but not an unrelated entry, nor the same words in
        // another project.
        assert_eq!(found, [reworded.id]);
        assert!(!found.contains(&unrelated.id));
        assert!(!found.contains(&elsewhere.id));
    })
    .unwrap();
}
