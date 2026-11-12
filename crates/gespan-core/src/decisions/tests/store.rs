//! The decision log and My tasks share `task_decisions` in gespan.db.
use crate::decisions::entry::derived_title;
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
    let root = std::fs::canonicalize(dir.path()).unwrap();
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
