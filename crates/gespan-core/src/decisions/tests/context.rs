use super::super::context::{reads_decision_file, start_context};
use super::super::scope_for;
use super::super::service::{self, isolated};
use crate::provider::Kind;
use gespan_memory::{Kind as EntryKind, NewEntry, Scope, Source, STANDING_INSTRUCTION};
use std::path::Path;

/// Proposes an entry for `scope` as the user's, and approves it.
fn approve(scope: &Scope, title: &str) -> i64 {
    service::with(|store| {
        let entry = store
            .propose(
                NewEntry {
                    kind: EntryKind::Decision,
                    scope: scope.clone(),
                    title: title.into(),
                    text: format!("{title}, always."),
                    why: "the user said so".into(),
                    user_answer: None,
                    source: Source {
                        session: Some(1),
                        question_ref: Some("q".into()),
                        ..Source::default()
                    },
                },
                10,
            )
            .unwrap();
        store.approve(entry.id, None, false, &[], 20).unwrap().id
    })
    .unwrap()
}

fn context(kind: Kind, workspace: &Path, already: Option<&str>) -> Option<(String, String)> {
    start_context(kind, workspace, already)
}

#[test]
fn no_context_when_disabled() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    approve(&scope_for(dir.path()), "Use tabs");
    service::set_enabled(false);
    assert_eq!(context(Kind::Codex, dir.path(), None), None);
    service::set_enabled(true);
    assert!(context(Kind::Codex, dir.path(), None).is_some());
}

#[test]
fn no_context_for_harness_that_reads_the_file() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    approve(&scope_for(dir.path()), "Use tabs");
    // Before Gespan writes its project files every harness gets the block.
    for kind in Kind::ALL {
        assert!(context(kind, dir.path(), None).is_some(), "{kind:?}");
    }
    let report = service::with(|store| {
        crate::decisions::harness::regenerate(
            store,
            &crate::accounts::Book::default(),
            data.path(),
            &|_| None,
            &[dir.path().to_path_buf()],
        )
    })
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for kind in Kind::ALL {
        let reads = matches!(kind, Kind::Claude | Kind::Cursor | Kind::Kiro);
        assert_eq!(reads_decision_file(kind), reads, "{kind:?}");
        // Exactly one delivery: the file, or the injected block.
        assert_eq!(context(kind, dir.path(), None).is_none(), reads, "{kind:?}");
    }
}

#[test]
fn context_once_then_again_only_after_the_log_changes() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let scope = scope_for(dir.path());
    approve(&scope, "Use tabs");
    let (block, hash) = context(Kind::Codex, dir.path(), None).unwrap();
    assert!(block.starts_with(STANDING_INSTRUCTION), "{block}");
    assert!(block.contains("Use tabs"), "{block}");
    assert!(!block.contains("<gespan_context>"), "the caller wraps it");
    // Already delivered: the same log gives nothing.
    assert_eq!(context(Kind::Codex, dir.path(), Some(&hash)), None);
    assert_eq!(context(Kind::OpenCode, dir.path(), Some(&hash)), None);
    // Another project's decision is not this session's change.
    let other = tempfile::tempdir().unwrap();
    approve(&scope_for(other.path()), "Use spaces");
    assert_eq!(context(Kind::Codex, dir.path(), Some(&hash)), None);
    // Approving one of this project's entries changes the block.
    approve(&scope, "Name branches after tickets");
    let (again, new_hash) = context(Kind::Codex, dir.path(), Some(&hash)).unwrap();
    assert_ne!(new_hash, hash);
    assert!(again.contains("Name branches after tickets"), "{again}");
    assert!(!again.contains("Use spaces"), "{again}");
    assert_eq!(context(Kind::Codex, dir.path(), Some(&new_hash)), None);
}
