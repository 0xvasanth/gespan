//! The GPUI app shares `rebase.db`, webview storage and the mobile wire format
//! with the Tauri Rebase app. These tests pin every shared value to Tauri's.
use rebase_core::{app_update::APP_ID, mobile::catalog::recent_sessions, store::Store};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every Rust source of both native crates, with its text.
fn native_sources() -> Vec<(PathBuf, String)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for name in ["rebase-core", "rebase-gpui"] {
        rust_files(&crates.join(name).join("src"), &mut files);
    }
    files
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            (path, text)
        })
        .collect()
}

/// Source lines matching `needle`.
fn offenders(needle: impl Fn(&str) -> bool) -> Vec<String> {
    native_sources()
        .iter()
        .flat_map(|(path, text)| {
            text.lines()
                .enumerate()
                .filter(|(_, line)| needle(line))
                .map(|(n, line)| format!("{}:{}: {}", path.display(), n + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A file of the Tauri app (the source of truth), relative to the repo root.
fn tauri(path: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
}

#[test]
fn shared_db_path_is_rebase() {
    let path = rebase_core::store::rebase_database();
    assert!(
        path.ends_with("com.rebase.desktop/rebase.db"),
        "{}",
        path.display()
    );
    assert!(tauri("src-tauri/tauri.conf.json").contains("\"identifier\": \"com.rebase.desktop\""));
    assert!(tauri("src-tauri/src/session_store.rs").contains("join(\"rebase.db\")"));
    let bad = offenders(|l| l.contains("com.monocode.desktop") || l.contains("monocode.db\""));
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn webview_storage_dir_is_rebase() {
    let bad = offenders(|l| l.contains("WebKit/com.monocode") || l.contains("join(\"com.monocode"));
    assert!(bad.is_empty(), "{bad:#?}");
    assert!(native_sources()
        .iter()
        .any(|(_, text)| text.contains("Library/WebKit/com.rebase.desktop")));
}

#[test]
fn pilot_tables_match_tauri() {
    let ddl = tauri("src-tauri/src/pilot_transcript.rs");
    for table in ["pilot_transcripts", "pilot_turns", "pilot_blocks"] {
        assert!(ddl.contains(&format!("CREATE TABLE IF NOT EXISTS {table} (")));
    }
    let bad = offenders(|l| {
        !l.contains("fn ")
            && ["mono_transcripts", "mono_turns", "mono_blocks"]
                .iter()
                .any(|t| l.contains(t))
    });
    assert!(bad.is_empty(), "{bad:#?}");
    // Opening a store must not invent legacy tables.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rebase.db");
    drop(Store::open_rebase(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!names.iter().any(|n| n.starts_with("mono_")), "{names:?}");
}

#[test]
fn storage_keys_are_rebase() {
    let bad: Vec<_> = offenders(|l| l.contains("\"monocode:") || l.contains("\"monocode."))
        .into_iter()
        .filter(|l| !l.contains("monocode.local"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
    let all: String = native_sources().into_iter().map(|(_, t)| t).collect();
    assert!(
        !all.contains("railMonosPinned")
            && !all.contains("monosEnabled")
            && !all.contains("monoMenuBarIcon")
    );
    let tauri_keys = [
        "src/features/pilots/model/pilot.ts",
        "src/features/settings/model/displayPrefs.ts",
        "src/features/settings/model/settings.ts",
    ]
    .map(tauri)
    .join("\n");
    for key in [
        "rebase:pilot-roster",
        "rebase.railPilotsPinned",
        "rebase.pilotsEnabled",
        "rebase.pilotMenuBarIcon",
    ] {
        assert!(
            tauri_keys.contains(&format!("\"{key}\"")),
            "Tauri lacks {key}"
        );
        // Preference flags are read as `rebase.{name}`.
        let needle = key
            .strip_prefix("rebase.")
            .filter(|n| n.starts_with("pilot") && !n.contains("rail"))
            .unwrap_or(key);
        assert!(all.contains(&format!("\"{needle}\"")), "GPUI lacks {key}");
    }
}

#[test]
fn pilot_files_dir_is_pilots() {
    assert!(tauri("src-tauri/src/pilot.rs").contains("const PILOTS_DIR: &str = \"pilots\";"));
    assert!(tauri("src-tauri/src/codex_pilot_store.rs").contains("join(\"codex-pilots\")"));
    let bad = offenders(|l| l.contains(".join(\"monos\")") || l.contains("codex-monos"));
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn mobile_contract_uses_pilots() {
    let access = tauri("src-tauri/src/mobile_access.rs");
    assert!(access.contains("catalog[\"pilots\"]") && access.contains("strip_prefix(\"pilot:\")"));
    assert!(access.contains("rebase://pair") && access.contains("com.rebase.mobile"));
    let bad = offenders(|l| {
        (l.contains("\"monos\"") && (l.contains("json!") || l.contains("catalog[")))
            || l.contains("monoId")
            || l.contains("strip_prefix(\"mono:\")")
            || l.contains("monocode://")
            || l.contains("com.monocode.mobile")
    });
    assert!(bad.is_empty(), "{bad:#?}");
    // Behavior: a Pilot's session is carried past the per-project cap.
    let mut sessions: Vec<_> = (0..30)
        .map(|n| json!({"id": format!("s{n}"), "projectId": "p"}))
        .collect();
    sessions.push(json!({"id": "pilot-session", "projectId": "p", "pilotId": "x"}));
    let kept = recent_sessions(&json!(sessions));
    assert!(kept.iter().any(|s| s["id"] == "pilot-session"));
    sessions.pop();
    sessions.push(json!({"id": "old", "projectId": "p", "monoId": "x"}));
    assert!(!recent_sessions(&json!(sessions))
        .iter()
        .any(|s| s["id"] == "old"));
}

#[test]
fn block_fields_match_tauri() {
    assert!(tauri("src-tauri/src/pilot_transcript.rs").contains("pilotHabit"));
    let bad = offenders(|l| {
        ["monoHabit", "monoCard", "monoSpawnedSessions"]
            .iter()
            .any(|f| l.contains(f))
    });
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn checkpoint_refs_are_rebase() {
    assert!(tauri("src-tauri/src/checkpoint/turn.rs").contains("refs/rebase/checkpoints/"));
    let bad = offenders(|l| l.contains("refs/monocode") || l.contains("checkpoint@monocode"));
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn app_id_is_rebase_native() {
    assert_eq!(APP_ID, "com.rebase.native");
}

/// The native app installs and pairs the same remote host as Tauri, so its
/// bootstrap scripts, askpass handshake and release URL must not drift.
fn native(path: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/remote")
            .join(path),
    )
    .unwrap()
    .replace("\r\n", "\n")
}

#[test]
fn remote_bootstrap_scripts_are_identical_to_tauri() {
    for (tauri_path, native_path) in [
        ("src-tauri/src/remote_bootstrap.sh", "remote_bootstrap.sh"),
        ("src-tauri/src/remote_bootstrap.ps1", "remote_bootstrap.ps1"),
        ("host/windows-acl.ps1", "windows-acl.ps1"),
        ("src-tauri/src/ssh_askpass.rs", "ssh_askpass.rs"),
    ] {
        assert_eq!(
            tauri(tauri_path).replace("\r\n", "\n"),
            native(native_path),
            "{native_path} drifted from {tauri_path}"
        );
    }
}

#[test]
fn remote_host_install_names_and_assets_match_tauri_and_the_host_package() {
    let sh = native("remote_bootstrap.sh");
    let ps1 = native("remote_bootstrap.ps1");
    // Install dir, binary, env vars.
    assert!(sh.contains("BASE=\"$HOME/.rebase-host\""));
    assert!(sh.contains("ENTRY=\"$BASE/bin/rebase-host\""));
    assert!(sh.contains("REBASE_HOST_FORCE_UPGRADE") && sh.contains("REBASE_HOST_PORT"));
    assert!(ps1.contains("'.rebase-host'"));
    assert!(ps1.contains("REBASE_HOST_FORCE_UPGRADE") && ps1.contains("REBASE_HOST_PORT"));
    // Release asset names are what host/package.mjs publishes.
    let package = tauri("host/package.mjs");
    assert!(package.contains("`rebase-host-${target}.${extension}`"));
    assert!(sh.contains("FILE=\"rebase-host-$OS-$ARCH.tar.gz\""));
    assert!(ps1.contains("\"rebase-host-$target.zip\""));
    assert!(!sh.to_lowercase().contains("monocode") && !ps1.to_lowercase().contains("monocode"));
}

#[test]
fn remote_ssh_release_url_env_and_pairing_match_tauri() {
    let native_ssh = native("ssh.rs");
    let tauri_ssh = tauri("src-tauri/src/remote_ssh.rs");
    let url = |source: &str| {
        source
            .lines()
            .find(|l| l.contains("/releases/download/v{version}"))
            .map(|l| l.trim().to_string())
            .unwrap()
    };
    assert_eq!(url(&native_ssh), url(&tauri_ssh));
    assert!(url(&native_ssh).contains("https://github.com/0xvasanth/rebase/releases/download/"));
    // Every host-facing line (env vars, install dir, binary, platform probe) is shared verbatim.
    let host_lines = |source: &str| {
        source
            .lines()
            .map(str::trim)
            .filter(|l| {
                !l.starts_with("//")
                    && (l.contains("REBASE_HOST_")
                        || l.contains(".rebase-host")
                        || l.contains("REBASE_PLATFORM"))
            })
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let shared = host_lines(&tauri_ssh);
    assert!(shared.len() >= 4, "{shared:#?}");
    for line in &shared {
        assert!(
            native_ssh.contains(line.as_str()),
            "missing in native ssh.rs: {line}"
        );
    }
    let askpass = native("ssh_askpass.rs");
    assert!(
        askpass.contains("REBASE_SSH_ASKPASS_ADDRESS")
            && askpass.contains("REBASE_SSH_ASKPASS_SECRET")
    );
}

/// Both apps persist the same `agentRun` object and the Claude wake marker row in
/// `sessions.blocks_json`. Renaming any of these on one side silently drops the field
/// when the other app loads or saves the block, so each literal is pinned on both sides.
/// Mirror of the "shares the agentRun fields" test in `src/test/sharedContracts.test.ts`.
#[test]
fn agent_run_and_wake_marker_json_names_match() {
    let session = tauri("src/features/sessions/model/session.ts");
    let store = tauri("src/features/sessions/data/sessionStore.ts");
    let claude = tauri("src/integrations/harness/providers/claude/claude.ts");
    let agent_run = session
        .split("export type AgentRunMeta = {")
        .nth(1)
        .and_then(|rest| rest.split("\n};").next())
        .expect("AgentRunMeta in session.ts");
    let tasks = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tool/tasks.rs"))
        .unwrap();
    for field in ["agentId", "resumeOf", "activity", "continuation"] {
        assert!(
            agent_run.contains(&format!("{field}?:")),
            "{field} missing on Tauri AgentRunMeta"
        );
        assert!(
            store.contains(&format!("\"{field}\"")),
            "{field} not kept by the Tauri store"
        );
        assert!(
            tasks.contains(&format!("agent.get(\"{field}\")"))
                || tasks.contains(&format!("agent[\"{field}\"]")),
            "Rebase Native does not read agentRun.{field}"
        );
        assert!(
            tasks.contains(&format!("(\"{field}\","))
                || tasks.contains(&format!("value[\"{field}\"] =")),
            "Rebase Native does not write agentRun.{field}"
        );
    }

    // The row: `role: "system"`, `interjection: { customType }`, `sentAt`.
    let interjection = session
        .split("export type InterjectionMeta = {")
        .nth(1)
        .expect("InterjectionMeta in session.ts");
    assert!(interjection.trim_start().starts_with("customType: string;"));
    assert!(session.contains("sentAt?: number;"));
    let marker = claude
        .split("function emitWakeMarker")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("emitWakeMarker in claude.ts");
    assert!(
        marker.contains("customType: \"task-notification\""),
        "{marker}"
    );
    assert!(marker.contains("sentAt:"), "{marker}");
    let blocks = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/store/shared/blocks.rs"),
    )
    .unwrap();
    assert!(blocks.contains("block[\"interjection\"][\"customType\"]"));
    assert!(blocks.contains("json!({\"customType\":note.custom_type})"));
    assert!(blocks.contains("block[\"sentAt\"]"));
    assert!(native_sources()
        .iter()
        .any(|(_, text)| text.contains("custom_type: \"task-notification\".into()")));
}

/// The native board source, as the Tauri board source is read with `tauri`.
fn native_board() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/board/mod.rs")).unwrap()
}

/// Every board table and index statement, whitespace-normalized.
fn board_ddl(source: &str) -> Vec<String> {
    regex::Regex::new(r"CREATE (?:UNIQUE )?(?:TABLE|INDEX) IF NOT EXISTS board[^;]*;")
        .unwrap()
        .find_iter(source)
        .map(|statement| {
            statement
                .as_str()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn board_tables_match_tauri() {
    let tauri_ddl = board_ddl(&tauri("src-tauri/src/board.rs"));
    // Four tables and five indexes; an empty match would pass vacuously.
    assert_eq!(tauri_ddl.len(), 9, "{tauri_ddl:#?}");
    assert_eq!(board_ddl(&native_board()), tauri_ddl);
}

#[test]
fn board_error_strings_match_tauri() {
    let tauri_source = tauri("src-tauri/src/board.rs");
    let native_source = native_board();
    for message in [
        "Column not on this board",
        "Column has {cards} {plural}",
        "Board not found",
        "Unknown column role: {role}",
        "already has the {role} role",
    ] {
        assert!(tauri_source.contains(message), "Tauri lacks {message}");
        assert!(native_source.contains(message), "native lacks {message}");
    }
}

#[test]
fn opening_the_rebase_store_then_the_board_service_creates_the_four_shared_board_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rebase.db");
    drop(Store::open_rebase(&path).unwrap());
    drop(rebase_core::board::Service::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'board%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    // The shared tables are exactly Tauri's four; anything else the native app
    // adds is its own, prefixed so Tauri never mistakes it for shared data.
    let (native, shared): (Vec<_>, Vec<_>) = names
        .iter()
        .partition(|name| name.starts_with("board_native_"));
    assert_eq!(
        shared,
        ["board_activity", "board_cards", "board_columns", "boards"]
    );
    assert!(!native.is_empty());
}

#[test]
fn board_setting_key_matches_tauri() {
    assert!(tauri("src/features/settings/model/settings.ts").contains("\"rebase.boardEnabled\""));
    let native = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/desktop_preferences/workspace_preferences.rs"),
    )
    .unwrap();
    assert!(native.contains("(\"board\", \"boardEnabled\", true)"));
}
