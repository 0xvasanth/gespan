//! Gespan's own databases go through Toasty. This test fails when production
//! code anywhere in the workspace opens a SQLite file directly or writes
//! schema SQL by hand, outside the few places allowed to:
//!
//! - the ORM itself (`orm/`), which opens and adopts files;
//! - the legacy shapes adoption upgrades from, and the constraints Toasty's
//!   DDL cannot express (`Kind::legacy_upgrade`, `Kind::constraints`);
//! - readers of other apps' databases (Cursor, Codex, WebKit, the Tauri
//!   importer), whose schemas are not ours to migrate.
//!
//! Queries follow the same rule: reads and writes of Gespan's tables use
//! Toasty's typed API. Raw query SQL (`toasty::sql`, `Store::sql()`, a
//! rusqlite connection) is only for what Toasty 0.11 cannot express, and
//! every file's raw query lines are counted in [`QUERIES`] so the number can
//! only go down.
//!
//! A new table or column is a Toasty model plus a migration:
//! `cargo run -p gespan-core --bin toasty-migrate -- <kind> <name>` (see
//! `docs/toasty.md`). Never add a `CREATE TABLE` or `ALTER TABLE`, and never
//! raise a count below to make this pass unless the new line is one of the
//! allowed kinds above.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Raw schema SQL and raw database opens.
const PATTERNS: &[&str] = &[
    "CREATE TABLE",
    "ALTER TABLE",
    "CREATE INDEX",
    "CREATE UNIQUE INDEX",
    "Connection::open",
    "open_with_flags",
    "open_in_memory",
    "PRAGMA table_info",
    "pragma_table_info",
];

/// The crates whose production code is checked.
const CRATES: &[&str] = &[
    "gespan-core",
    "board-store",
    "gespan-browser",
    "gespan-graph",
    "gespan-gpui",
    "gespan-first-launch",
];

/// Each allowed file, with how many raw schema or open lines it may hold.
const ALLOWED: &[(&str, usize, &str)] = &[
    (
        "board-store/src/constraints.rs",
        3,
        "constraints Toasty cannot express",
    ),
    (
        "board-store/src/migrate.rs",
        2,
        "legacy board upgrade for adoption",
    ),
    (
        "board-store/src/schema.rs",
        30,
        "legacy board shapes for adoption",
    ),
    ("board-store/src/testing.rs", 3, "test fixture"),
    (
        "gespan-browser/src/audit.rs",
        1,
        "constraint Toasty cannot express",
    ),
    (
        "gespan-core/src/accounts/transfer.rs",
        1,
        "reads Codex's database",
    ),
    (
        "gespan-core/src/accounts/transfer/codex.rs",
        2,
        "reads Codex's database",
    ),
    (
        "gespan-core/src/automations.rs",
        6,
        "legacy shape and constraints",
    ),
    (
        "gespan-core/src/desktop_preferences/webview.rs",
        1,
        "reads WebKit's database",
    ),
    (
        "gespan-core/src/diagnostics/metrics.rs",
        3,
        "legacy shape for adoption",
    ),
    (
        "gespan-core/src/flow/store.rs",
        6,
        "legacy shape and constraints",
    ),
    ("gespan-core/src/orm/adopt.rs", 8, "the ORM's adoption"),
    ("gespan-core/src/orm.rs", 1, "test-only checkpoint helper"),
    ("gespan-core/src/orm/kind.rs", 2, "the ORM's registration"),
    (
        "gespan-core/src/provider/cursor_store.rs",
        2,
        "reads Cursor's database",
    ),
    (
        "gespan-core/src/store/my_tasks.rs",
        4,
        "legacy shape for adoption",
    ),
    (
        "gespan-core/src/store/schema.rs",
        6,
        "constraints Toasty cannot express",
    ),
    ("gespan-graph/src/db.rs", 1, "the graph's own Toasty driver"),
    (
        "gespan-graph/src/schema.rs",
        6,
        "constraints Toasty cannot express",
    ),
    ("gespan-graph/src/store.rs", 1, "test-only plain connection"),
];

/// Raw query SQL: the statements Toasty's typed API is for.
const QUERY_PATTERNS: &[&str] = &[
    "SELECT ",
    "INSERT INTO",
    "INSERT OR ",
    "DELETE FROM",
    "REPLACE INTO",
];

/// Each file's raw query lines as of the rule that every query goes through
/// Toasty, with why it has them. "to move to Toasty" is debt: when a change
/// moves a file's queries to the typed API, lower its number (the test fails
/// until it matches, so the count can only fall). Never raise a number or add
/// a file, unless the new SQL reads another app's database, is part of
/// adoption or a constraint, or is something Toasty 0.11 cannot express
/// (see docs/toasty.md), with a comment on the statement saying which.
const QUERIES: &[(&str, usize, &str)] = &[
    (
        "board-store/src/card_ops.rs",
        3,
        "rowid tie-breaks Toasty cannot order by",
    ),
    (
        "board-store/src/comments.rs",
        1,
        "rowid tie-breaks Toasty cannot order by",
    ),
    (
        "board-store/src/constraints.rs",
        26,
        "trigger bodies of constraints Toasty cannot express",
    ),
    (
        "board-store/src/fields.rs",
        3,
        "rowid tie-breaks Toasty cannot order by",
    ),
    (
        "board-store/src/meta.rs",
        4,
        "GROUP BY counts Toasty cannot express",
    ),
    (
        "board-store/src/migrate.rs",
        24,
        "legacy board upgrade for adoption",
    ),
    (
        "board-store/src/ops.rs",
        5,
        "rowid tie-breaks Toasty cannot order by",
    ),
    (
        "board-store/src/relations.rs",
        3,
        "a recursive CTE Toasty cannot express",
    ),
    (
        "board-store/src/seed.rs",
        1,
        "rowid tie-breaks Toasty cannot order by",
    ),
    (
        "board-store/src/settings.rs",
        1,
        "a read that must fail, not panic, on a corrupt row",
    ),
    (
        "board-store/src/subtasks.rs",
        1,
        "rowid tie-breaks Toasty cannot order by",
    ),
    (
        "board-store/src/templates.rs",
        1,
        "a read that must fail, not panic, on a corrupt row",
    ),
    ("board-store/src/testing.rs", 1, "test fixture"),
    (
        "board-store/src/views.rs",
        6,
        "rowid tie-breaks and a window function Toasty cannot express",
    ),
    (
        "gespan-browser/src/audit.rs",
        2,
        "a conditional upsert Toasty cannot express",
    ),
    (
        "gespan-core/src/accounts/transfer.rs",
        2,
        "reads Codex's database",
    ),
    (
        "gespan-core/src/accounts/transfer/codex.rs",
        3,
        "reads Codex's database",
    ),
    (
        "gespan-core/src/automations.rs",
        12,
        "constraint triggers and the legacy claims upgrade for adoption",
    ),
    (
        "gespan-core/src/decisions/store.rs",
        8,
        "FTS5 / vec0 virtual tables Toasty cannot model",
    ),
    (
        "gespan-core/src/decisions/vector.rs",
        6,
        "FTS5 / vec0 virtual tables Toasty cannot model",
    ),
    (
        "gespan-core/src/desktop_preferences/webview.rs",
        1,
        "reads WebKit's database",
    ),
    (
        "gespan-core/src/flow/store.rs",
        8,
        "constraint triggers, and two reads ordered by rowid, which a model cannot name",
    ),
    (
        "gespan-core/src/inbox/azure_devops.rs",
        1,
        "an Azure DevOps WIQL query, not SQLite",
    ),
    ("gespan-core/src/orm/adopt.rs", 8, "the ORM's adoption"),
    (
        "gespan-core/src/orm.rs",
        2,
        "the ORM's migration ledger, which Toasty does not model",
    ),
    (
        "gespan-core/src/provider/cursor_store.rs",
        4,
        "reads Cursor's database",
    ),
    (
        "gespan-core/src/session_import/cursor.rs",
        2,
        "reads Cursor's database",
    ),
    (
        "gespan-core/src/store/my_tasks.rs",
        3,
        "a conditional upsert and a counted update",
    ),
    (
        "gespan-core/src/store/pilots.rs",
        3,
        "json1 reads of Pilot blocks",
    ),
    (
        "gespan-core/src/store/schema.rs",
        21,
        "legacy upgrade and constraint triggers",
    ),
    (
        "gespan-core/src/store/session_metadata.rs",
        1,
        "bounded substr projections",
    ),
    (
        "gespan-core/src/store/session_preview.rs",
        1,
        "bounded substr projections",
    ),
    (
        "gespan-core/src/store/shared.rs",
        7,
        "covering-index sidebar list, snapshot read, counted updates",
    ),
    (
        "gespan-core/src/store/shared/window.rs",
        1,
        "a json1 filter",
    ),
    ("gespan-graph/src/db.rs", 2, "the graph's own Toasty driver"),
    (
        "gespan-graph/src/query.rs",
        12,
        "a recursive CTE Toasty cannot express, and the per-node expansion, kept raw for speed",
    ),
    ("gespan-graph/src/schema.rs", 5, "constraint triggers"),
    (
        "gespan-graph/src/store.rs",
        5,
        "deletes that report a count, which Toasty cannot express",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Lines that hold raw schema SQL or a raw database open.
fn raw_lines(source: &str) -> usize {
    source
        .lines()
        .filter(|line| PATTERNS.iter().any(|pattern| line.contains(pattern)))
        .count()
}

/// Lines that hold raw query SQL. Comments do not count.
fn query_lines(source: &str) -> usize {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .filter(|line| {
            QUERY_PATTERNS.iter().any(|pattern| line.contains(pattern))
                || (line.contains("UPDATE ") && line.contains(" SET "))
        })
        .count()
}

/// `source` without its inline `#[cfg(test)] mod tests { ... }` blocks. An
/// out-of-line `mod tests;` keeps the rest of the file, which is production
/// code.
fn production(source: &str) -> String {
    let mut kept = String::new();
    let mut from = 0;
    let mut search = 0;
    while let Some(at) = source[search..].find("#[cfg(test)]") {
        let start = search + at;
        search = start + 1;
        let rest = source[start + "#[cfg(test)]".len()..].trim_start();
        let rest = rest.strip_prefix("pub(crate) ").unwrap_or(rest);
        let Some(rest) = rest.strip_prefix("mod tests") else {
            continue;
        };
        if !rest.trim_start().starts_with('{') {
            continue;
        }
        let open = source.len() - rest.len() + rest.find('{').unwrap();
        let mut depth = 0;
        let mut end = source.len();
        for (offset, c) in source[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + offset + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        kept.push_str(&source[from..start]);
        from = end;
        search = end;
    }
    kept.push_str(&source[from..]);
    kept
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name == "tests.rs" || name.ends_with("_tests.rs")
}

/// `count` lines per production file, keyed `crate/src/...`.
fn found(count: fn(&str) -> usize) -> BTreeMap<String, usize> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut counts = BTreeMap::new();
    for krate in CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(krate).join("src"), &mut files);
        for file in files.into_iter().filter(|f| !is_test_file(f)) {
            let source = std::fs::read_to_string(&file).unwrap();
            let count = count(&production(&source));
            if count > 0 {
                let key = file
                    .strip_prefix(&crates)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                counts.insert(key, count);
            }
        }
    }
    counts
}

#[test]
fn gespan_databases_are_only_reached_through_toasty() {
    let allowed: BTreeMap<&str, usize> = ALLOWED.iter().map(|(f, n, _)| (*f, *n)).collect();
    let mut problems = Vec::new();
    for (file, count) in found(raw_lines) {
        match allowed.get(file.as_str()) {
            None => problems.push(format!(
                "{file}: {count} raw SQLite schema or open line(s). Gespan's own tables are \
                 Toasty models with migrations (docs/toasty.md)."
            )),
            Some(limit) if count > *limit => problems.push(format!(
                "{file}: {count} raw schema or open lines, {limit} allowed. Add the table \
                 or column as a Toasty model and run toasty-migrate instead."
            )),
            Some(_) => {}
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn gespan_queries_only_go_through_toasty() {
    let allowed: BTreeMap<&str, usize> = QUERIES.iter().map(|(f, n, _)| (*f, *n)).collect();
    let found = found(query_lines);
    let mut problems = Vec::new();
    for (file, count) in &found {
        match allowed.get(file.as_str()) {
            None => problems.push(format!(
                "{file}: {count} raw SQL query line(s). Read and write Gespan's tables \
                 through Toasty's typed API (docs/toasty.md)."
            )),
            Some(limit) if count > limit => problems.push(format!(
                "{file}: {count} raw SQL query lines, {limit} allowed. Use Toasty's typed \
                 API instead."
            )),
            Some(limit) if count < limit => problems.push(format!(
                "{file}: {count} raw SQL query lines, down from {limit}. Lower its number \
                 in QUERIES to {count}."
            )),
            Some(_) => {}
        }
    }
    for (file, limit, _) in QUERIES {
        if !found.contains_key(*file) {
            problems.push(format!(
                "{file}: no raw SQL query lines left, {limit} listed. Remove it from QUERIES."
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_query_guard_counts_raw_statements_but_not_comments() {
    let source = "fn f() {\n\
                  db.sql(\"SELECT id FROM sessions\");\n\
                  db.sql(\"UPDATE sessions SET title = ?1\");\n\
                  // DELETE FROM sessions is what this replaces\n\
                  Session::filter_by_id(id).delete().exec(db);\n\
                  }";
    assert_eq!(query_lines(source), 2);
}

#[test]
fn the_guard_still_reads_a_file_after_its_out_of_line_tests_module() {
    let source = "#[cfg(test)]\nmod tests;\n\
                  fn f() { conn.execute(\"CREATE TABLE x (y)\"); }\n\
                  #[cfg(test)]\nmod tests { fn g() { Connection::open(\"t\"); } }\n\
                  fn h() { Connection::open(\"u\"); }";
    assert_eq!(
        raw_lines(&production(source)),
        2,
        "code after `mod tests;` and after an inline tests module both count"
    );
}

#[test]
fn the_toasty_guard_catches_hand_written_schema() {
    let source = "fn f() { conn.execute(\"CREATE TABLE x (y)\"); }\n\
                  #[cfg(test)]\nmod tests { fn g() { Connection::open(\"t\"); } }";
    assert_eq!(
        raw_lines(&production(source)),
        1,
        "production code counts, the tests module does not"
    );
}
