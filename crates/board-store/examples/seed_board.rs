//! Adds a new 1,000-card board to an existing `gespan.db`, for checking the
//! board at scale by hand in either app:
//!
//! ```text
//! cargo run -p gespan-board-store --example seed_board -- --db <path> [--cards 1000] [--name "<board>"]
//! ```
//!
//! It always creates a new board (from the blank template) and never edits an
//! existing one. Prints the new board's id.
use gespan_board_store::seed::{seed_board, SeedReport, SeedSpec};
use gespan_board_store::{create_board_from_template, delete_board, ensure_tables};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str =
    "Usage: seed_board --db <path to gespan.db> [--cards 1000] [--name \"<board name>\"]";

fn main() {
    match run(std::env::args().skip(1)) {
        Ok(board_id) => println!("{board_id}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

/// Seeds a new board as the command line asks. Returns its id.
pub fn run(args: impl IntoIterator<Item = String>) -> Result<String, String> {
    run_with(args, seed_board)
}

/// `run`, with the seed step passed in so tests can make it fail after the
/// board exists.
pub fn run_with(
    args: impl IntoIterator<Item = String>,
    seed: impl FnOnce(&Connection, &str, &SeedSpec) -> Result<SeedReport, String>,
) -> Result<String, String> {
    let mut db: Option<PathBuf> = None;
    let mut cards = SeedSpec::default().cards;
    let mut name: Option<String> = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} needs a value. {USAGE}"))
        };
        match arg.as_str() {
            "--db" => db = Some(PathBuf::from(value()?)),
            "--cards" => {
                cards = value()?
                    .parse()
                    .map_err(|_| format!("--cards must be a whole number. {USAGE}"))?
            }
            "--name" => name = Some(value()?),
            _ => return Err(format!("Unknown argument: {arg}. {USAGE}")),
        }
    }
    let db = db.ok_or_else(|| USAGE.to_string())?;
    if !db.is_file() {
        return Err(format!(
            "No database at {}. Open Gespan once first.",
            db.display()
        ));
    }
    // Check it is Gespan's database before writing anything: read-only, so a
    // file that isn't one is left exactly as it was (no WAL, no tables).
    if !is_gespan_db(&db)? {
        return Err(format!(
            "{} is not a Gespan database (it has no boards table). Nothing was changed.",
            db.display()
        ));
    }
    // Open without CREATE, so a path that vanished is never made anew.
    let conn = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    ensure_tables(&conn).map_err(|e| e.to_string())?;
    let name = name.unwrap_or_else(|| format!("Scale test ({cards} cards)"));
    let board = create_board_from_template(&conn, &name, "blank")?;
    let spec = SeedSpec {
        cards,
        ..SeedSpec::default()
    };
    if let Err(error) = seed(&conn, &board.id, &spec) {
        // Leave no half-made board behind: remove the one just created.
        if let Err(cleanup) = delete_board(&conn, &board.id) {
            return Err(format!(
                "{error}. The new board {} could not be removed: {cleanup}",
                board.id
            ));
        }
        return Err(error);
    }
    Ok(board.id)
}

/// Whether the file is a SQLite database with Gespan's `boards` table, read
/// without changing it.
fn is_gespan_db(path: &Path) -> Result<bool, String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'boards')",
        [],
        |r| r.get(0),
    )
    .map_err(|e| format!("{} is not a Gespan database: {e}", path.display()))
}
