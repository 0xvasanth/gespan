//! Writes the next Toasty migration for one database kind from its models.
//!
//! `cargo run -p gespan-core --bin toasty-migrate -- <kind> <name> [--reset]`
//! (from `crates/gespan-gpui`, or with `--manifest-path
//! crates/gespan-core/Cargo.toml`). `--reset` regenerates the kind's
//! migrations from scratch; use it only before they ship.
// A developer tool run from a terminal: output goes to stdout (the crate
// keeps stderr for the app's structured logs) and failures set the exit code.
use gespan_core::orm::{migrations, Kind};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reset = args.iter().any(|arg| arg == "--reset");
    let positional: Vec<&str> = args
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .map(String::as_str)
        .collect();
    let kinds = Kind::ALL.map(Kind::name).join(", ");
    let [kind, name] = positional[..] else {
        println!("usage: toasty-migrate <kind> <name> [--reset]\nkinds: {kinds}");
        return ExitCode::FAILURE;
    };
    let Some(kind) = Kind::from_name(kind) else {
        println!("Unknown kind {kind}; kinds: {kinds}");
        return ExitCode::FAILURE;
    };
    match migrations::generate(kind, name, reset) {
        Ok(Some(path)) => {
            println!("Wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("The {} models match the last migration.", kind.name());
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{error}");
            ExitCode::FAILURE
        }
    }
}
