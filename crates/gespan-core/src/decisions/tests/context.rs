use super::super::context::{reads_decision_file, start_context};
use super::super::scope_for;
use super::super::service::{self, isolated};
use crate::decisions::{Kind as EntryKind, NewEntry, Scope, Source, STANDING_INSTRUCTION};
use crate::provider::Kind;
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

/// With search by meaning on, the session-start block keeps preferences and
/// pinned decisions only; the other decisions come per turn or by search.
#[test]
fn with_search_by_meaning_the_start_block_has_preferences_and_pinned_only() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let scope = scope_for(dir.path());
    approve(&scope, "Use tabs");
    let pinned = service::with(|store| {
        let e = store
            .propose(
                NewEntry {
                    kind: EntryKind::Decision,
                    scope: scope.clone(),
                    title: "Name branches after tickets".into(),
                    text: "Always.".into(),
                    why: "the user said so".into(),
                    user_answer: None,
                    source: Source::default(),
                },
                10,
            )
            .unwrap();
        store.approve(e.id, None, true, &[], 20).unwrap();
        let p = store
            .propose(
                NewEntry {
                    kind: EntryKind::Preference,
                    scope: Scope::Global,
                    title: "Answer tersely".into(),
                    text: "Short answers.".into(),
                    why: "taste".into(),
                    user_answer: None,
                    source: Source::default(),
                },
                10,
            )
            .unwrap();
        store.approve(p.id, None, false, &[], 20).unwrap();
    });
    assert!(pinned.is_ok());
    let (all, _) = context(Kind::Codex, dir.path(), None).unwrap();
    assert!(all.contains("Use tabs"), "{all}");
    service::with(|store| {
        store
            .set_embedder(Some(std::sync::Arc::new(
                crate::decisions::HashEmbedder::new(),
            )))
            .unwrap()
    })
    .unwrap();
    let (block, _) = context(Kind::Codex, dir.path(), None).unwrap();
    assert!(block.contains("Name branches after tickets"), "{block}");
    assert!(block.contains("Answer tersely"), "{block}");
    assert!(!block.contains("Use tabs"), "{block}");
}

// --- The per-turn related block ---

use crate::decisions::context::{related_context, related_in_flight, MAX_RELATED_WORKERS};
use crate::decisions::{Embedder, Error, HashEmbedder, TURN_BUDGET};
use crate::diagnostics::metrics::{self, ErrorKind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Proposes an entry the user stated, and approves it (pinned or not) unless
/// `pending`.
fn entry(kind: EntryKind, scope: &Scope, title: &str, text: &str, pinned: bool, pending: bool) {
    service::with(|store| {
        let e = store
            .propose(
                NewEntry {
                    kind,
                    scope: scope.clone(),
                    title: title.into(),
                    text: text.into(),
                    why: "agreed in review".into(),
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
        if !pending {
            store.approve(e.id, None, pinned, &[], 20).unwrap();
        }
    })
    .unwrap();
}

fn embed_with(embedder: Arc<dyn Embedder>) {
    service::with(|store| store.set_embedder(Some(embedder)).unwrap()).unwrap();
}

/// Waits until no related-decisions worker is left running, so no stuck
/// thread outlives its test.
fn drained() {
    let until = Instant::now() + Duration::from_secs(10);
    while related_in_flight() > 0 {
        assert!(Instant::now() < until, "a related worker never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn skipped(counts: &[metrics::Count]) -> Vec<(Option<ErrorKind>, u64)> {
    counts
        .iter()
        .filter(|c| c.name == "decisions.related.skipped")
        .map(|c| (c.kind, c.count))
        .collect()
}

const TIMELY: Duration = Duration::from_secs(10);

#[test]
fn related_block_within_turn_budget() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let scope = scope_for(dir.path());
    let long = "Indent nested blocks with tabs in every source file. ".repeat(18);
    for n in 0..12 {
        entry(
            EntryKind::Decision,
            &scope,
            &format!("Tabs rule {n}"),
            &long,
            false,
            false,
        );
    }
    entry(
        EntryKind::Decision,
        &scope,
        "Deploy on Fridays",
        "Ship releases weekly.",
        false,
        false,
    );
    // Search by meaning off: there is no per-turn block.
    assert_eq!(related_context(dir.path(), "tabs", TIMELY), None);
    embed_with(Arc::new(HashEmbedder::new()));

    let block = related_context(dir.path(), "Should I indent with tabs?", TIMELY).unwrap();
    assert!(block.len() <= TURN_BUDGET, "{} bytes", block.len());
    assert!(block.starts_with(STANDING_INSTRUCTION), "{block}");
    assert!(block.contains("Tabs rule"), "{block}");
    // Unrelated entries never appear, and there was more than fit.
    assert!(!block.contains("Deploy on Fridays"), "{block}");
    assert!(block.contains("use decisions_search"), "{block}");
    // Nothing related: no block at all.
    assert_eq!(related_context(dir.path(), "zebra", TIMELY), None);
    // The log off: nothing.
    service::set_enabled(false);
    assert_eq!(related_context(dir.path(), "tabs", TIMELY), None);
    service::set_enabled(true);
    drained();
}

#[test]
fn excludes_entries_already_in_the_start_block() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let scope = scope_for(dir.path());
    entry(
        EntryKind::Decision,
        &scope,
        "Pinned tabs",
        "Indent with tabs.",
        true,
        false,
    );
    entry(
        EntryKind::Preference,
        &Scope::Global,
        "Preferred tabs",
        "Tabs over spaces.",
        false,
        false,
    );
    entry(
        EntryKind::Decision,
        &scope,
        "Makefile tabs",
        "Makefiles keep their tabs.",
        false,
        false,
    );
    entry(
        EntryKind::Decision,
        &scope,
        "Yaml tabs",
        "No tabs in yaml.",
        false,
        true,
    );
    embed_with(Arc::new(HashEmbedder::new()));
    // The start block has exactly the pinned decision and the preference.
    let (start, _) = start_context(Kind::Codex, dir.path(), None).unwrap();
    assert!(start.contains("Pinned tabs") && start.contains("Preferred tabs"));

    let block = related_context(dir.path(), "tabs", TIMELY).unwrap();
    assert!(block.contains("Makefile tabs"), "{block}");
    assert!(!block.contains("Pinned tabs"), "{block}");
    assert!(!block.contains("Preferred tabs"), "{block}");
    // A pending entry the user stated comes marked, under its own heading.
    assert!(
        block.contains("[not yet approved — the user said this in session 1] (Yaml tabs)"),
        "{block}"
    );
    let pending_at = block.find("## Not yet approved").unwrap();
    assert!(block.find("Yaml tabs").unwrap() > pending_at, "{block}");
    assert!(block.find("Makefile tabs").unwrap() < pending_at, "{block}");

    // Only entries already in the start block are related: no block.
    let (none, counts) =
        metrics::capture(|| related_context(dir.path(), "Pinned preferred", TIMELY));
    assert_eq!(none, None);
    assert!(skipped(&counts).is_empty(), "nothing related is no skip");
    drained();
}

/// Embeds documents at once and queries after `delay`.
struct Sleepy {
    inner: HashEmbedder,
    delay: Duration,
}

impl Embedder for Sleepy {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dims(&self) -> usize {
        self.inner.dims()
    }
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, Error> {
        self.inner.embed_document(text)
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, Error> {
        std::thread::sleep(self.delay);
        self.inner.embed_query(text)
    }
    fn max_distance(&self) -> f32 {
        self.inner.max_distance()
    }
}

#[test]
fn slow_embedder_past_deadline_returns_none_and_counts_skip() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let scope = scope_for(dir.path());
    entry(
        EntryKind::Decision,
        &scope,
        "Makefile tabs",
        "Makefiles keep their tabs.",
        false,
        false,
    );
    embed_with(Arc::new(Sleepy {
        inner: HashEmbedder::new(),
        delay: Duration::from_millis(600),
    }));
    let ((block, took), counts) = metrics::capture(|| {
        let start = Instant::now();
        let block = related_context(dir.path(), "tabs", Duration::from_millis(150));
        (block, start.elapsed())
    });
    assert_eq!(block, None);
    assert!(
        took < Duration::from_millis(200),
        "the turn waited {took:?}"
    );
    assert_eq!(skipped(&counts), [(Some(ErrorKind::Timeout), 1)]);
    drained();
    // A quick lookup afterwards is not counted.
    embed_with(Arc::new(HashEmbedder::new()));
    let (block, counts) = metrics::capture(|| related_context(dir.path(), "tabs", TIMELY));
    assert!(block.unwrap().contains("Makefile tabs"));
    assert!(skipped(&counts).is_empty());
}

/// Blocks every query until released, counting the queries that started.
struct Stuck {
    inner: HashEmbedder,
    started: AtomicUsize,
    open: Mutex<bool>,
    opened: Condvar,
}

impl Stuck {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }
}

impl Embedder for Stuck {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dims(&self) -> usize {
        self.inner.dims()
    }
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, Error> {
        self.inner.embed_document(text)
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, Error> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
        drop(open);
        self.inner.embed_query(text)
    }
    fn max_distance(&self) -> f32 {
        self.inner.max_distance()
    }
}

#[test]
fn a_stuck_embedder_does_not_spawn_a_thread_per_turn() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    entry(
        EntryKind::Decision,
        &scope_for(dir.path()),
        "Makefile tabs",
        "Makefiles keep their tabs.",
        false,
        false,
    );
    let stuck = Arc::new(Stuck {
        inner: HashEmbedder::new(),
        started: AtomicUsize::new(0),
        open: Mutex::new(false),
        opened: Condvar::new(),
    });
    embed_with(stuck.clone());
    let turns = 3 * MAX_RELATED_WORKERS;
    let (blocks, counts) = metrics::capture(|| {
        (0..turns)
            .map(|_| related_context(dir.path(), "tabs", Duration::from_millis(30)))
            .collect::<Vec<_>>()
    });
    assert!(blocks.iter().all(Option::is_none));
    // Every turn went without the block and was counted ...
    let total: u64 = skipped(&counts).iter().map(|(_, n)| n).sum();
    assert_eq!(total, turns as u64);
    // ... but only the first few started a lookup; the rest found the
    // workers busy and started none.
    assert_eq!(related_in_flight(), MAX_RELATED_WORKERS);
    let until = Instant::now() + Duration::from_secs(5);
    while stuck.started.load(Ordering::SeqCst) < MAX_RELATED_WORKERS && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(stuck.started.load(Ordering::SeqCst), MAX_RELATED_WORKERS);
    stuck.release();
    drained();
    // Once they finish, the next turn gets its block again.
    let block = related_context(dir.path(), "tabs", TIMELY).unwrap();
    assert!(block.contains("Makefile tabs"), "{block}");
    assert_eq!(
        stuck.started.load(Ordering::SeqCst),
        MAX_RELATED_WORKERS + 1
    );
}
