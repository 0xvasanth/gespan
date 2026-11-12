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
    ("gespan-core/src/orm/kind.rs", 2, "the ORM's registration"),
    (
        "gespan-core/src/provider/cursor_store.rs",
        2,
        "reads Cursor's database",
    ),
    (
        "gespan-core/src/store.rs",
        10,
        "legacy native fixtures store",
    ),
    (
        "gespan-core/src/store/gespan_import.rs",
        2,
        "reads the Tauri database",
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
    (
        "gespan-core/src/store/shared/settings.rs",
        1,
        "legacy JSON settings import",
    ),
    ("gespan-graph/src/db.rs", 1, "the graph's own Toasty driver"),
    (
        "gespan-graph/src/schema.rs",
        6,
        "constraints Toasty cannot express",
    ),
    ("gespan-graph/src/store.rs", 1, "test-only plain connection"),
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

/// `source` without its trailing `#[cfg(test)] mod tests` block.
fn production(source: &str) -> &str {
    let mut search = 0;
    while let Some(at) = source[search..].find("#[cfg(test)]") {
        let start = search + at;
        let rest = source[start + "#[cfg(test)]".len()..].trim_start();
        let rest = rest.strip_prefix("pub(crate) ").unwrap_or(rest);
        if rest.starts_with("mod tests") {
            return &source[..start];
        }
        search = start + 1;
    }
    source
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name == "tests.rs" || name.ends_with("_tests.rs")
}

/// Raw schema or open lines per production file, keyed `crate/src/...`.
fn found() -> BTreeMap<String, usize> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut counts = BTreeMap::new();
    for krate in CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(krate).join("src"), &mut files);
        for file in files.into_iter().filter(|f| !is_test_file(f)) {
            let source = std::fs::read_to_string(&file).unwrap();
            let count = raw_lines(production(&source));
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
    for (file, count) in found() {
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
fn the_toasty_guard_catches_hand_written_schema() {
    let source = "fn f() { conn.execute(\"CREATE TABLE x (y)\"); }\n\
                  #[cfg(test)]\nmod tests { fn g() { Connection::open(\"t\"); } }";
    assert_eq!(
        raw_lines(production(source)),
        1,
        "production code counts, the tests module does not"
    );
}
