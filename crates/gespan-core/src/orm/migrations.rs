//! Migration files on disk: `migrations/<kind>/history.toml`, the SQL in
//! `migrations/`, and the schema snapshot each migration leaves in
//! `snapshots/`. The `toasty-migrate` tool and the drift test use this; the
//! app only uses the embedded copies (`Kind::migrations`).
use super::Kind;
use std::path::{Path, PathBuf};
use toasty::migration::{History, HistoryEntry, Snapshot};
use toasty::schema::{db, diff};

/// `migrations/<kind>/` in this crate's sources.
pub fn dir(kind: Kind) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join(kind.name())
}

fn history_path(kind: Kind) -> PathBuf {
    dir(kind).join("history.toml")
}

pub fn history(kind: Kind) -> Result<History, String> {
    History::load_or_default(history_path(kind)).map_err(|e| e.to_string())
}

/// The schema the last migration leaves, or an empty one before the first.
pub fn last_snapshot(kind: Kind) -> Result<db::Schema, String> {
    let history = history(kind)?;
    let Some(last) = history.entries().last() else {
        return Ok(db::Schema::default());
    };
    let path = dir(kind).join("snapshots").join(&last.snapshot_name);
    Snapshot::load(&path)
        .map(|snapshot| snapshot.schema)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))
}

/// The schema this kind's models make on SQLite today.
pub fn model_schema(kind: Kind) -> Result<db::Schema, String> {
    let driver = super::driver::Driver::new(Path::new(":memory:"));
    let capability = toasty::db::Driver::capability(&driver);
    super::schema(kind.models(), capability)
        .map(|schema| schema.db)
        .map_err(|e| format!("The {} models do not build: {e}", kind.name()))
}

/// The SQL that would take the last snapshot to the models, if they differ.
pub fn pending_sql(kind: Kind) -> Result<Option<String>, String> {
    Ok(diff_models(kind)?.map(|(sql, _)| sql))
}

fn diff_models(kind: Kind) -> Result<Option<(String, Snapshot)>, String> {
    let driver = super::driver::Driver::new(Path::new(":memory:"));
    let previous = last_snapshot(kind)?;
    let next = model_schema(kind)?;
    Ok(
        toasty::migration::generate(&driver, &previous, &next, &diff::RenameHints::new()).map(
            |generated| {
                let db::Migration::Sql(sql) = generated.migration;
                (sql, generated.snapshot)
            },
        ),
    )
}

/// Writes the next migration, named `NNNN_<name>.sql`, with its snapshot and
/// history entry. Returns the SQL file, or `None` when the models match the
/// last snapshot. With `reset`, every earlier migration of this kind is
/// removed first and the new one starts from an empty schema: only for a kind
/// whose migrations have never shipped.
pub fn generate(kind: Kind, name: &str, reset: bool) -> Result<Option<PathBuf>, String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!(
            "Migration names use lowercase letters, digits and _: {name}"
        ));
    }
    let dir = dir(kind);
    let io = |e: std::io::Error| e.to_string();
    if reset {
        for entry in history(kind)?.entries() {
            for file in [
                dir.join("migrations").join(&entry.name),
                dir.join("snapshots").join(&entry.snapshot_name),
            ] {
                if file.exists() {
                    std::fs::remove_file(file).map_err(io)?;
                }
            }
        }
        History::new()
            .save(history_path(kind))
            .map_err(|e| e.to_string())?;
    }
    let Some((sql, snapshot)) = diff_models(kind)? else {
        return Ok(None);
    };
    let mut history = history(kind)?;
    let number = history.next_migration_number();
    let file_name = format!("{number:04}_{name}.sql");
    let snapshot_name = format!("{number:04}_snapshot.toml");
    std::fs::create_dir_all(dir.join("migrations")).map_err(io)?;
    std::fs::create_dir_all(dir.join("snapshots")).map_err(io)?;
    let sql_path = dir.join("migrations").join(&file_name);
    std::fs::write(&sql_path, format!("{sql}\n")).map_err(io)?;
    snapshot
        .save(dir.join("snapshots").join(&snapshot_name))
        .map_err(|e| e.to_string())?;
    history.add_entry(HistoryEntry {
        // SQLite keeps the id as a signed integer.
        id: rand::random::<u64>() >> 1,
        name: file_name,
        snapshot_name,
        checksum: None,
    });
    history
        .save(history_path(kind))
        .map_err(|e| e.to_string())?;
    Ok(Some(sql_path))
}
