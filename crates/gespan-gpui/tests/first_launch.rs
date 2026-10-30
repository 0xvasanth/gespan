//! A release build must open the data an installed Gespan left behind: nothing
//! can update a Tauri install to Native, so the first launch is the whole move.
use rusqlite::{params, Connection};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

const DESKTOP_SCHEMA: &str = include_str!("../../gespan-core/tests/fixtures/desktop/schema-v1.sql");

/// Where Tauri's `app_data_dir` puts `dev.gespan.desktop`, given `home`'s
/// environment from `launch`.
fn tauri_data_dir(home: &Path) -> PathBuf {
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        home.join("AppData/Roaming")
    } else {
        home.join("xdg-data")
    };
    base.join("dev.gespan.desktop")
}

fn launch(home: &Path, xdg_data_home: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gespan-gpui"));
    command
        .args(["--gespan-data", "--check-data"])
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .env("XDG_DATA_HOME", xdg_data_home.unwrap_or(Path::new("")));
    command.output().unwrap()
}

fn saved_install(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let database = dir.join("gespan.db");
    let conn = Connection::open(&database).unwrap();
    conn.execute_batch(DESKTOP_SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at)
         VALUES('saved','/project','codex','codex:gpt','full-access','Saved in Gespan',?1,1,2)",
        params![r#"[{"role":"user","text":"Hello"}]"#],
    )
    .unwrap();
    database
}

#[test]
fn the_first_launch_opens_the_installed_gespan_data_without_changing_it() {
    let home = tempfile::tempdir().unwrap();
    let database = saved_install(&tauri_data_dir(home.path()));
    let before = std::fs::read(&database).unwrap();
    let output = launch(home.path(), Some(&home.path().join("xdg-data")));
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{log}");
    assert!(log.contains("1 saved sessions"), "{log}");
    assert_eq!(std::fs::read(&database).unwrap(), before);
}

#[test]
fn a_machine_without_gespan_reports_no_data_and_creates_none() {
    let home = tempfile::tempdir().unwrap();
    let output = launch(home.path(), Some(&home.path().join("xdg-data")));
    assert!(!output.status.success());
    assert!(!tauri_data_dir(home.path()).join("gespan.db").exists());
}

// Tauri falls back to ~/.local/share for both of these, so Native has to as well.
#[cfg(target_os = "linux")]
#[test]
fn an_empty_or_relative_xdg_data_home_still_finds_the_data() {
    let home = tempfile::tempdir().unwrap();
    saved_install(&home.path().join(".local/share/dev.gespan.desktop"));
    for xdg in [None, Some(Path::new("relative/data"))] {
        let output = launch(home.path(), xdg);
        let log = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{log}");
        assert!(log.contains("1 saved sessions"), "{log}");
    }
}
