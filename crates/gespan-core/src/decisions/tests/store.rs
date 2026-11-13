//! The decision log and My tasks share `task_decisions` in gespan.db.
use crate::decisions::entry::derived_title;
// Scopes are keyed by paths without Windows' `\\?\` prefix, as `scope_for`
// gives them.
use crate::decisions::files::without_verbatim;
use crate::decisions::{scope_for, Config, DecisionStore, Kind, NewEntry, Scope, Source, Status};
use crate::orm::tests::file_at_migration;
use crate::orm::Kind as DbKind;
use crate::store::Store;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn config() -> Config {
    Config {
        redact: crate::pilot::memory::redact,
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one commit and an origin, so its scope is a repo scope.
fn repo(root: &Path) -> PathBuf {
    let dir = root.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    git(&dir, &["remote", "add", "origin", "git@github.com:o/r.git"]);
    std::fs::write(dir.join("README.md"), "readme\n").unwrap();
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// The app's store and the decision log, both on one gespan.db.
fn both(dir: &TempDir) -> (Store, DecisionStore) {
    let path = dir.path().join("gespan.db");
    let store = Store::open_gespan(&path).unwrap();
    let log = DecisionStore::open(&path, config()).unwrap();
    (store, log)
}

fn proposal(scope: Scope, title: &str, text: &str) -> NewEntry {
    NewEntry {
        kind: Kind::Decision,
        scope,
        title: title.into(),
        text: text.into(),
        why: "because".into(),
        user_answer: None,
        source: Source::default(),
    }
}

fn count(path: &Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn adopted_task_decisions_become_active_project_decisions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("gespan.db");
    // gespan.db as main's builds left it: only the first migration, with two
    // decisions saved from My tasks, one of them archived.
    file_at_migration(DbKind::Gespan, &path, 1);
    let statement = "  Carry K's fsidCount counter on every write.\r\nSecond line ";
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO task_decisions (id, project, statement, why, created_at, updated_at)
             VALUES (7, '/work/gespan', ?1, 'it is cheaper', 10, 20)",
            [statement],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task_decisions VALUES
               (9, '/work/gespan', 'Recount at read time', '', 11, 21, 31)",
            [],
        )
        .unwrap();
    }

    let log = DecisionStore::open(&path, config()).unwrap();

    let scope = Scope::Project("/work/gespan".into());
    let active = log.active(&scope).unwrap();
    let [live] = &active[..] else {
        panic!("only the live decision is active: {active:?}");
    };
    assert_eq!(live.id, 7);
    assert_eq!(live.kind, Kind::Decision);
    assert_eq!(live.status, Status::Active);
    assert_eq!(live.text, statement);
    assert_eq!(live.why, "it is cheaper");
    assert!(!live.pinned);
    // The migration titles old rows by the rule My tasks uses for new ones.
    assert_eq!(live.title, derived_title(statement));
    assert_eq!(live.title, "Carry K's fsidCount counter on every write.");
    let (archived, _) = log.get(9).unwrap().unwrap();
    assert_eq!(archived.status, Status::Archived);
    assert_eq!(archived.scope, scope);
    // Rows that predate the search index are found too.
    let hits = log.search(&scope, "fsidCount", 5).unwrap();
    assert_eq!(hits.iter().map(|e| e.id).collect::<Vec<_>>(), [7]);
    // The app's store reads the same rows.
    drop(log);
    let store = Store::open_gespan(&path).unwrap();
    let decisions = store.decisions().unwrap();
    assert_eq!(decisions.iter().map(|d| d.id).collect::<Vec<_>>(), [7]);
}

#[test]
fn my_tasks_saved_decision_reaches_the_decision_log() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let root = without_verbatim(&std::fs::canonicalize(dir.path()).unwrap());
    let repo = repo(&root);
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    for project in [&repo, &plain] {
        let project_text = project.to_string_lossy().into_owned();
        let id = store
            .save_decision(&project_text, "Keep one table\nfor both", "simpler", &[], 5)
            .unwrap();
        let scope = scope_for(project);
        let active = log.active(&scope).unwrap();
        let entry = active
            .iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("{scope} holds the saved decision: {active:?}"));
        assert_eq!(entry.kind, Kind::Decision);
        assert_eq!(entry.status, Status::Active);
        assert_eq!(entry.title, "Keep one table");
        assert_eq!(entry.text, "Keep one table\nfor both");
        assert_eq!(entry.why, "simpler");
        assert_eq!(entry.decided_ms, Some(5));
    }
    assert!(matches!(scope_for(&repo), Scope::Repo(_)));
    // Main's list still groups it by the project path.
    let projects: Vec<_> = store
        .decisions()
        .unwrap()
        .into_iter()
        .map(|d| d.project)
        .collect();
    assert!(projects.contains(&repo.to_string_lossy().into_owned()));
}

#[test]
fn a_saved_decision_keeps_the_answer_it_came_from() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let answered = crate::tasks::TaskRecord::open(
        "question:7:q1",
        crate::tasks::Kind::Question,
        "/work/gespan",
        "session:7",
        "Fix fsid",
        "How should fsidCount be kept?",
        serde_json::json!({}),
        1,
    )
    .answered("Carry K's counter", serde_json::json!([]), 2);
    store.answer_task(&answered).unwrap();
    let id = store
        .save_decision(
            "/work/gespan",
            "Carry K's counter",
            "",
            &["question:7:q1".into()],
            3,
        )
        .unwrap();
    let (entry, _) = log.get(id).unwrap().unwrap();
    assert_eq!(entry.user_answer.as_deref(), Some("Carry K's counter"));
    assert_eq!(entry.source.question_ref.as_deref(), Some("question:7:q1"));
}

#[test]
fn decision_log_entry_shows_in_my_tasks_decisions() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let scope = Scope::Project("/work/gespan".into());
    let pending = log
        .propose(proposal(scope.clone(), "Tabs", "Indent with tabs"), 1)
        .unwrap();
    assert!(
        store.decisions().unwrap().is_empty(),
        "a pending proposal is not a decision yet"
    );
    log.approve(pending.id, None, false, &[], 2).unwrap();
    let decisions = store.decisions().unwrap();
    let [shown] = &decisions[..] else {
        panic!("the approved entry is listed: {decisions:?}");
    };
    assert_eq!(shown.id, pending.id);
    assert_eq!(shown.project, "/work/gespan");
    assert_eq!(shown.statement, "Indent with tabs");
    assert_eq!(shown.why, "because");

    // Archiving it on either side takes it off both lists.
    store.archive_decision(pending.id, 3).unwrap();
    assert!(store.decisions().unwrap().is_empty());
    assert!(log.active(&scope).unwrap().is_empty());
    assert_eq!(
        log.get(pending.id).unwrap().unwrap().0.status,
        Status::Archived
    );
    let other = log
        .propose(proposal(scope.clone(), "Spaces", "Indent with spaces"), 4)
        .unwrap();
    log.approve(other.id, None, false, &[], 5).unwrap();
    log.archive(other.id, 6).unwrap();
    assert!(store.decisions().unwrap().is_empty());
}

#[test]
fn editing_in_my_tasks_rewrites_the_entry_and_keeps_a_title_of_its_own() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let saved = store
        .save_decision("/work/gespan", "Recount at read time", "", &[], 1)
        .unwrap();
    store
        .edit_decision(saved, "Recount on every read", 2)
        .unwrap();
    let (entry, _) = log.get(saved).unwrap().unwrap();
    assert_eq!(entry.text, "Recount on every read");
    assert_eq!(
        entry.title, "Recount on every read",
        "a derived title follows"
    );
    assert_eq!(log.search(&entry.scope, "every", 5).unwrap().len(), 1);
    assert!(log.search(&entry.scope, "time", 5).unwrap().is_empty());

    let scope = Scope::Project("/work/gespan".into());
    let proposed = log
        .propose(proposal(scope, "Short title", "A long text"), 3)
        .unwrap();
    log.approve(proposed.id, None, false, &[], 4).unwrap();
    store
        .edit_decision(proposed.id, "A longer text", 5)
        .unwrap();
    let (entry, _) = log.get(proposed.id).unwrap().unwrap();
    assert_eq!(entry.text, "A longer text");
    assert_eq!(entry.title, "Short title", "an agent's own title is kept");
}

#[test]
fn search_finds_my_tasks_decisions() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let id = store
        .save_decision(
            "/work/gespan",
            "Store axolotl ids",
            "they are stable",
            &[],
            1,
        )
        .unwrap();
    let scope = Scope::Project("/work/gespan".into());
    let found = |q: &str| -> Vec<i64> {
        log.search(&scope, q, 5)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect()
    };
    assert_eq!(found("axolotl"), [id], "the statement is indexed");
    assert_eq!(found("stable"), [id], "the why is indexed");
    store.archive_decision(id, 2).unwrap();
    assert!(
        found("axolotl").is_empty(),
        "an archived one is not visible"
    );
}

#[test]
fn either_handle_can_open_a_new_file_first() {
    for decision_log_first in [true, false] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gespan.db");
        let (store, log) = if decision_log_first {
            let log = DecisionStore::open(&path, config()).unwrap();
            (Store::open_gespan(&path).unwrap(), log)
        } else {
            let store = Store::open_gespan(&path).unwrap();
            (store, DecisionStore::open(&path, config()).unwrap())
        };
        let migrations = DbKind::Gespan.migrations().migrations().len() as i64;
        assert_eq!(
            count(&path, "SELECT COUNT(*) FROM __toasty_migrations"),
            migrations,
            "each migration is applied once"
        );
        assert_eq!(
            count(&path, "SELECT COUNT(*) FROM schema_migrations"),
            1,
            "a new file is marked past the Tauri migrations once"
        );
        assert_eq!(
            count(
                &path,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'task_decisions_fts'"
            ),
            1
        );
        // Both handles write the one file.
        let id = store.save_decision("/p", "Shared", "", &[], 1).unwrap();
        assert!(log.get(id).unwrap().is_some());
    }
}

/// gespan.db at migration 1 holding one My tasks decision for `project`.
fn legacy_with_decision(path: &Path, project: &Path) {
    file_at_migration(DbKind::Gespan, path, 1);
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO task_decisions (id, project, statement, why, created_at, updated_at)
             VALUES (3, ?1, 'Ship from main only', 'one release line', 10, 20)",
            [project.to_string_lossy()],
        )
        .unwrap();
}

#[test]
fn legacy_rows_are_rescoped_to_their_projects_scope() {
    let dir = TempDir::new().unwrap();
    let root = without_verbatim(&std::fs::canonicalize(dir.path()).unwrap());
    let repo = repo(&root);
    let path = root.join("gespan.db");
    legacy_with_decision(&path, &repo);
    let log = DecisionStore::open(&path, config()).unwrap();
    let scope = scope_for(&repo);
    assert!(matches!(scope, Scope::Repo(_)));
    assert!(
        log.active(&scope).unwrap().is_empty(),
        "the migration can only key it by its path"
    );

    assert_eq!(log.rescope_project_rows(scope_for).unwrap(), 1);

    let ids: Vec<i64> = log.active(&scope).unwrap().iter().map(|e| e.id).collect();
    assert_eq!(ids, [3]);
    // Every start runs it again: nothing left to move, nothing changes.
    let calls = std::cell::Cell::new(0);
    let again = log
        .rescope_project_rows(|p| {
            calls.set(calls.get() + 1);
            scope_for(p)
        })
        .unwrap();
    assert_eq!((again, calls.get()), (0, 0));
    assert_eq!(log.get(3).unwrap().unwrap().0.scope, scope);
}

#[test]
fn rescoping_leaves_rows_already_in_their_scope_alone() {
    let dir = TempDir::new().unwrap();
    let root = without_verbatim(&std::fs::canonicalize(dir.path()).unwrap());
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let path = root.join("gespan.db");
    legacy_with_decision(&path, &plain);
    let log = DecisionStore::open(&path, config()).unwrap();
    // A project with no git: its scope is the path the migration used.
    assert_eq!(log.rescope_project_rows(scope_for).unwrap(), 0);
    let entry = log.get(3).unwrap().unwrap().0;
    assert_eq!(entry.scope, Scope::Project(plain.to_string_lossy().into()));
    // A row an agent proposed in another scope is never touched.
    let other = log
        .propose(proposal(Scope::Global, "Pref", "Prefer tabs"), 1)
        .unwrap();
    log.rescope_project_rows(|_| Scope::Repo("x".into()))
        .unwrap();
    assert_eq!(log.get(other.id).unwrap().unwrap().0.scope, Scope::Global);
}

#[test]
fn save_decision_in_keeps_the_scope_it_is_given() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let scope = Scope::Repo("github.com/o/r@abc".into());
    let id = store
        .save_decision_in(&scope, "/work/gespan", "Use one table", "", &[], 1)
        .unwrap();
    let ids: Vec<i64> = log.active(&scope).unwrap().iter().map(|e| e.id).collect();
    assert_eq!(ids, [id]);
    assert_eq!(store.decisions().unwrap()[0].project, "/work/gespan");
}

#[test]
fn my_tasks_text_is_redacted_and_capped_like_the_logs() {
    let dir = TempDir::new().unwrap();
    let (store, log) = both(&dir);
    let secret = "sk-abcdefghijklmnopqrstuvwx";
    let github = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    let answered = crate::tasks::TaskRecord::open(
        "question:7:q1",
        crate::tasks::Kind::Question,
        "/work/gespan",
        "session:7",
        "Keys",
        "Which key?",
        serde_json::json!({}),
        1,
    )
    .answered(format!("use {secret}"), serde_json::json!([]), 2);
    store.answer_task(&answered).unwrap();
    let id = store
        .save_decision(
            "/work/gespan",
            &format!("Call the API with {secret}"),
            &format!("{github} works"),
            &["question:7:q1".into()],
            3,
        )
        .unwrap();
    let entry = log.get(id).unwrap().unwrap().0;
    for field in [
        &entry.title,
        &entry.text,
        &entry.why,
        entry.user_answer.as_ref().unwrap(),
    ] {
        assert!(
            !field.contains(secret) && !field.contains(github),
            "{field}"
        );
        assert!(field.contains("redacted"), "{field}");
    }
    store
        .edit_decision(id, &format!("Rotate {github} {}", "x".repeat(2_000)), 4)
        .unwrap();
    let entry = log.get(id).unwrap().unwrap().0;
    assert!(!entry.text.contains(github) && !entry.title.contains(github));
    assert_eq!(
        entry.text.chars().count(),
        crate::decisions::MAX_FIELD_CHARS
    );
    let saved = &store.decisions().unwrap()[0];
    assert!(!saved.statement.contains(github));
    let block = crate::decisions::render_block(
        &[],
        &log.active(&entry.scope).unwrap(),
        crate::decisions::START_BUDGET,
    );
    assert!(block.included.contains(&id));
    assert!(!block.text.contains(secret) && !block.text.contains(github));
}

#[test]
fn a_pending_entry_needs_a_non_empty_user_source_to_be_visible() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("gespan.db");
    let log = DecisionStore::open(&path, config()).unwrap();
    let scope = Scope::Repo("github.com/o/r".into());
    let propose = |title: &str| {
        log.propose(proposal(scope.clone(), title, &format!("{title} tabs")), 1)
            .unwrap()
            .id
    };
    let (none, empty, turn, answered) = (
        propose("None"),
        propose("Empty"),
        propose("Turn"),
        propose("Answered"),
    );
    let other = log
        .propose(
            NewEntry {
                source: Source {
                    question_ref: Some("q-1".into()),
                    ..Source::default()
                },
                ..proposal(
                    Scope::Repo("github.com/o/other".into()),
                    "Other",
                    "Other tabs",
                )
            },
            1,
        )
        .unwrap()
        .id;
    // My tasks and older builds may leave an empty string, which the log's
    // own writes never do: it is no user source.
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE task_decisions SET question_ref = '', user_turn = '' WHERE id = ?1",
        [empty],
    )
    .unwrap();
    conn.execute(
        "UPDATE task_decisions SET question_ref = '', user_turn = 'said so' WHERE id = ?1",
        [turn],
    )
    .unwrap();
    conn.execute(
        "UPDATE task_decisions SET question_ref = 'q-2' WHERE id = ?1",
        [answered],
    )
    .unwrap();
    drop(conn);

    let mut visible: Vec<i64> = log.visible(&scope).unwrap().iter().map(|e| e.id).collect();
    visible.sort();
    assert_eq!(visible, [turn, answered]);
    let mut found: Vec<i64> = log
        .search(&scope, "tabs", 10)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    found.sort();
    assert_eq!(
        found,
        [turn, answered],
        "search sees exactly what visible sees"
    );
    assert!(![none, empty, other].iter().any(|id| found.contains(id)));
}

#[test]
fn keyword_search_widens_its_pool_past_better_matches_it_cannot_show() {
    let dir = TempDir::new().unwrap();
    let log = DecisionStore::open(&dir.path().join("gespan.db"), config()).unwrap();
    let scope = Scope::Repo("github.com/o/r".into());
    // 70 active entries in another repo and 10 hidden pending ones here,
    // all matching "tabs" better (short, repeated) than the entries this
    // scope can see: more than the first pool of 64 holds.
    for i in 0..70 {
        let other = Scope::Repo("github.com/o/other".into());
        let e = log
            .propose(proposal(other, &format!("O{i}"), "tabs tabs"), 1)
            .unwrap();
        log.approve(e.id, None, false, &[], 2).unwrap();
    }
    for i in 0..10 {
        log.propose(proposal(scope.clone(), &format!("H{i}"), "tabs tabs"), 1)
            .unwrap();
    }
    // Four visible entries with the same text, so bm25 ties them and the
    // newest come first; plus one that ranks above them (more "tabs").
    let long = "tabs then many other words about indentation and the editor setup";
    let mut tied = Vec::new();
    for i in 0..4 {
        let e = log
            .propose(proposal(scope.clone(), &format!("V{i}"), long), 1)
            .unwrap();
        log.approve(e.id, None, false, &[], 2).unwrap();
        tied.push(e.id);
    }
    let best = log
        .propose(
            proposal(
                scope.clone(),
                "Best",
                "tabs tabs then many other words about indentation and the editor",
            ),
            1,
        )
        .unwrap();
    log.approve(best.id, None, false, &[], 2).unwrap();

    let found: Vec<i64> = log
        .search(&scope, "tabs", 3)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(found, [best.id, tied[3], tied[2]]);
    let all: Vec<i64> = log
        .search(&scope, "tabs", 10)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(all, [best.id, tied[3], tied[2], tied[1], tied[0]]);
}

/// A source made only of whitespace is no source: it must not make a
/// pending entry look like the user's words to other sessions.
#[test]
fn blank_source_text_is_trimmed_to_nothing() {
    let dir = TempDir::new().unwrap();
    let log = DecisionStore::open(&dir.path().join("gespan.db"), config()).unwrap();
    let mut new = proposal(Scope::Global, "Tabs", "Use tabs");
    new.source = Source {
        session: Some(3),
        question_ref: Some("  \n ".into()),
        user_turn: Some("\t".into()),
        import: Some("  ~/.claude/CLAUDE.md  ".into()),
    };
    let entry = log.propose(new, 1).unwrap();
    assert_eq!(entry.source.question_ref, None);
    assert_eq!(entry.source.user_turn, None);
    assert_eq!(entry.source.import.as_deref(), Some("~/.claude/CLAUDE.md"));
}
