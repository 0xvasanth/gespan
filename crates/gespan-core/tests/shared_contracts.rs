//! The GPUI app shares `gespan.db`, webview storage and the mobile wire format
//! with the Tauri Gespan app. These tests pin every shared value to Tauri's.
use gespan_core::{app_update::APP_ID, mobile::catalog::recent_sessions, store::Store};
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
    for name in ["gespan-core", "gespan-gpui"] {
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
fn shared_db_path_is_gespan() {
    let path = gespan_core::store::gespan_database();
    assert!(
        path.ends_with("dev.gespan.desktop/gespan.db"),
        "{}",
        path.display()
    );
    assert!(tauri("src-tauri/tauri.conf.json").contains("\"identifier\": \"dev.gespan.desktop\""));
    assert!(tauri("src-tauri/src/session_store.rs").contains("join(\"gespan.db\")"));
}

#[test]
fn webview_storage_dir_is_gespan() {
    assert!(native_sources()
        .iter()
        .any(|(_, text)| text.contains("Library/WebKit/dev.gespan.desktop")));
}

#[test]
fn pilot_tables_match_tauri() {
    let ddl = tauri("src-tauri/src/pilot_transcript.rs");
    for table in ["pilot_transcripts", "pilot_turns", "pilot_blocks"] {
        assert!(ddl.contains(&format!("CREATE TABLE IF NOT EXISTS {table} (")));
    }
    // Opening a store must not invent legacy tables.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    drop(Store::open_gespan(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with(concat!("mo", "no", "_"))),
        "{names:?}"
    );
}

#[test]
fn storage_keys_are_gespan() {
    let all: String = native_sources().into_iter().map(|(_, t)| t).collect();
    let tauri_keys = [
        "src/features/pilots/model/pilot.ts",
        "src/features/settings/model/displayPrefs.ts",
        "src/features/settings/model/settings.ts",
    ]
    .map(tauri)
    .join("\n");
    for key in [
        "gespan:pilot-roster",
        "gespan.railPilotsPinned",
        "gespan.pilotsEnabled",
        "gespan.pilotMenuBarIcon",
    ] {
        assert!(
            tauri_keys.contains(&format!("\"{key}\"")),
            "Tauri lacks {key}"
        );
        // Preference flags are read as `gespan.{name}`.
        let needle = key
            .strip_prefix("gespan.")
            .filter(|n| n.starts_with("pilot") && !n.contains("rail"))
            .unwrap_or(key);
        assert!(all.contains(&format!("\"{needle}\"")), "GPUI lacks {key}");
    }
}

#[test]
fn pilot_files_dir_is_pilots() {
    assert!(tauri("src-tauri/src/pilot.rs").contains("const PILOTS_DIR: &str = \"pilots\";"));
    assert!(tauri("src-tauri/src/codex_pilot_store.rs").contains("join(\"codex-pilots\")"));
}

#[test]
fn mobile_contract_uses_pilots() {
    let access = tauri("src-tauri/src/mobile_access.rs");
    assert!(access.contains("catalog[\"pilots\"]") && access.contains("strip_prefix(\"pilot:\")"));
    assert!(access.contains("gespan://pair") && access.contains("dev.gespan.mobile"));
    // Behavior: a Pilot's session is carried past the per-project cap.
    let mut sessions: Vec<_> = (0..30)
        .map(|n| json!({"id": format!("s{n}"), "projectId": "p"}))
        .collect();
    sessions.push(json!({"id": "pilot-session", "projectId": "p", "pilotId": "x"}));
    let kept = recent_sessions(&json!(sessions));
    assert!(kept.iter().any(|s| s["id"] == "pilot-session"));
    sessions.pop();
    let mut old = json!({"id": "old", "projectId": "p"});
    old[concat!("mo", "no", "Id")] = json!("x");
    sessions.push(old);
    assert!(!recent_sessions(&json!(sessions))
        .iter()
        .any(|s| s["id"] == "old"));
}

#[test]
fn block_fields_match_tauri() {
    assert!(tauri("src-tauri/src/pilot_transcript.rs").contains("pilotHabit"));
}

#[test]
fn checkpoint_refs_are_gespan() {
    assert!(tauri("src-tauri/src/checkpoint/turn.rs").contains("refs/gespan/checkpoints/"));
}

#[test]
fn app_id_is_gespan_native() {
    assert_eq!(APP_ID, "dev.gespan.native");
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
    assert!(sh.contains("BASE=\"$HOME/.gespan-host\""));
    assert!(sh.contains("ENTRY=\"$BASE/bin/gespan-host\""));
    assert!(sh.contains("GESPAN_HOST_FORCE_UPGRADE") && sh.contains("GESPAN_HOST_PORT"));
    assert!(ps1.contains("'.gespan-host'"));
    assert!(ps1.contains("GESPAN_HOST_FORCE_UPGRADE") && ps1.contains("GESPAN_HOST_PORT"));
    // Release asset names are what host/package.mjs publishes.
    let package = tauri("host/package.mjs");
    assert!(package.contains("`gespan-host-${target}.${extension}`"));
    assert!(sh.contains("FILE=\"gespan-host-$OS-$ARCH.tar.gz\""));
    assert!(ps1.contains("\"gespan-host-$target.zip\""));
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
    assert!(url(&native_ssh).contains("https://github.com/0xvasanth/gespan/releases/download/"));
    // Every host-facing line (env vars, install dir, binary, platform probe) is shared verbatim.
    let host_lines = |source: &str| {
        source
            .lines()
            .map(str::trim)
            .filter(|l| {
                !l.starts_with("//")
                    && (l.contains("GESPAN_HOST_")
                        || l.contains(".gespan-host")
                        || l.contains("GESPAN_PLATFORM"))
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
        askpass.contains("GESPAN_SSH_ASKPASS_ADDRESS")
            && askpass.contains("GESPAN_SSH_ASKPASS_SECRET")
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
            "Gespan Native does not read agentRun.{field}"
        );
        assert!(
            tasks.contains(&format!("(\"{field}\","))
                || tasks.contains(&format!("value[\"{field}\"] =")),
            "Gespan Native does not write agentRun.{field}"
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

#[test]
fn flow_ddl_is_pinned() {
    use gespan_core::flow::store::{
        ensure_tables, FLOWS_DDL, FLOW_INDEXES_DDL, FLOW_RUNS_DDL, FLOW_VISITS_DDL,
    };
    // Literal copies: the Phase 3 Tauri twin must create exactly this text.
    assert_eq!(
        FLOWS_DDL,
        "CREATE TABLE IF NOT EXISTS flows (
  id TEXT PRIMARY KEY,
  definition_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
)"
    );
    assert_eq!(
        FLOW_RUNS_DDL,
        "CREATE TABLE IF NOT EXISTS flow_runs (
  id TEXT PRIMARY KEY,
  flow_id TEXT NOT NULL REFERENCES flows(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  run_json TEXT NOT NULL
)"
    );
    assert_eq!(
        FLOW_VISITS_DDL,
        "CREATE TABLE IF NOT EXISTS flow_visits (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES flow_runs(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  state_id TEXT NOT NULL,
  status TEXT NOT NULL,
  visit_json TEXT NOT NULL,
  UNIQUE (run_id, seq)
)"
    );
    assert_eq!(
        FLOW_INDEXES_DDL,
        [
            "CREATE INDEX IF NOT EXISTS flow_runs_history_idx ON flow_runs (flow_id, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS flow_runs_status_idx ON flow_runs (status)",
            "CREATE INDEX IF NOT EXISTS flow_visits_order_idx ON flow_visits (run_id, seq)",
        ]
    );
    // What SQLite actually stored matches too.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    ensure_tables(&conn).unwrap();
    let sql = |name: &str| -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE name = ?1",
            [name],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(sql("flows"), FLOWS_DDL.replace(" IF NOT EXISTS", ""));
    assert_eq!(
        sql("flow_runs"),
        FLOW_RUNS_DDL.replace(" IF NOT EXISTS", "")
    );
    assert_eq!(
        sql("flow_visits"),
        FLOW_VISITS_DDL.replace(" IF NOT EXISTS", "")
    );
}

/// Both apps call the shared board-store crate, so its error strings are the
/// ones the React board and the native board show.
#[test]
fn board_error_strings_match_tauri() {
    let store_source = tauri("crates/board-store/src/ops.rs");
    for message in [
        "Column not on this board",
        "Column has {cards} {plural}",
        "Board not found",
        "Unknown column role: {role}",
        "already has the {role} role",
    ] {
        assert!(
            store_source.contains(message),
            "board-store lacks {message}"
        );
    }
    for app in [
        "src-tauri/src/board.rs",
        "crates/gespan-core/src/board/mod.rs",
    ] {
        assert!(
            tauri(app).contains("use gespan_board_store::"),
            "{app} does not use board-store"
        );
    }
}

#[test]
fn opening_the_gespan_store_then_the_board_service_creates_the_shared_v3_board_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    drop(Store::open_gespan(&path).unwrap());
    drop(gespan_core::board::Service::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'board%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    // The shared tables are board-store's, which the Tauri app creates too.
    // The pre-v3 native tables stay for one release and nothing reads them.
    let (native, shared): (Vec<_>, Vec<_>) = names
        .iter()
        .partition(|name| name.starts_with("board_native_"));
    assert_eq!(
        shared,
        [
            "board_activity",
            "board_card_field_values",
            "board_card_labels",
            "board_card_links",
            "board_card_meta",
            "board_card_projects",
            "board_card_relations",
            "board_cards",
            "board_columns",
            "board_comments",
            "board_fields",
            "board_labels",
            "board_schema",
            "board_settings",
            "board_subtasks",
            "boards",
        ]
    );
    assert_eq!(
        native,
        [
            "board_native_card_labels",
            "board_native_comments",
            "board_native_labels",
            "board_native_meta",
            "board_native_projects",
            "board_native_subtasks",
        ]
    );
    let version: i64 = conn
        .query_row("SELECT version FROM board_schema", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, gespan_board_store::SCHEMA_VERSION);
}

/// A database board-store wrote (as the Tauri app does) opens in the native
/// board service, which reads every v3 value back.
#[test]
fn a_database_written_by_board_store_opens_in_the_native_board_service() {
    use gespan_board_store as store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let (board, card, label) = {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        store::ensure_tables(&conn).unwrap();
        let board = store::create_board(&conn, "Shared").unwrap();
        let column = store::get_board(&conn, &board.id).unwrap().columns[0].clone();
        let card = store::create_card(&conn, &column.id, "Task", "").unwrap();
        store::set_priority(&conn, &card.id, 4).unwrap();
        let label = store::create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
        store::set_card_labels(&conn, &card.id, std::slice::from_ref(&label.id)).unwrap();
        store::add_subtask(&conn, &card.id, "Step").unwrap();
        store::add_comment(&conn, &card.id, "From the other app", "agent").unwrap();
        store::link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        (board.id, card.id, label.id)
    };
    let mut service = gespan_core::board::Service::open(&path).unwrap();
    assert_eq!(service.list().unwrap()[0].id, board);
    let extras = service.extras(&board).unwrap();
    let extras = &extras[&card];
    assert_eq!(extras.priority, 4);
    assert_eq!(extras.labels, [label]);
    assert_eq!((extras.subtasks_total, extras.comments), (1, 1));
    assert_eq!(service.comments(&card).unwrap()[0].author, "agent");
    assert_eq!(
        service.cards_for_link("session", "s-1").unwrap()[0].id,
        card
    );
    service.set_priority(&card, 1).unwrap();
}

/// A native database from before v3 keeps every extra when the native app
/// opens it after the upgrade.
#[test]
fn the_native_service_migrates_a_pre_v3_database_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE boards (id TEXT PRIMARY KEY, name TEXT NOT NULL, card_seq INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE board_columns (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, name TEXT NOT NULL, position INTEGER NOT NULL, role TEXT);
             CREATE TABLE board_cards (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, column_id TEXT NOT NULL REFERENCES board_columns(id), number INTEGER NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', position INTEGER NOT NULL, link_kind TEXT, link_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE (board_id, number));
             CREATE TABLE board_activity (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, card_id TEXT REFERENCES board_cards(id) ON DELETE CASCADE, kind TEXT NOT NULL, actor TEXT NOT NULL, from_column TEXT, to_column TEXT, detail TEXT, created_at INTEGER NOT NULL);
             CREATE TABLE board_native_meta (card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE, priority INTEGER NOT NULL DEFAULT 0, due_at INTEGER);
             CREATE TABLE board_native_comments (id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, body TEXT NOT NULL, created_at INTEGER NOT NULL);
             INSERT INTO boards VALUES ('b1', 'Work', 1, 1, 1);
             INSERT INTO board_columns VALUES ('c1', 'b1', 'Backlog', 0, 'todo');
             INSERT INTO board_cards VALUES ('k1', 'b1', 'c1', 1, 'Old', '', 0, NULL, NULL, 1, 1);
             INSERT INTO board_native_meta VALUES ('k1', 2, 1793404800000);
             INSERT INTO board_native_comments VALUES ('m1', 'k1', 'kept', 5);",
        )
        .unwrap();
    }
    let mut service = gespan_core::board::Service::open(&path).unwrap();
    let extras = &service.extras("b1").unwrap()["k1"];
    assert_eq!(
        (extras.priority, extras.due_at, extras.comments),
        (2, Some(1_793_404_800_000), 1)
    );
    assert_eq!(service.comments("k1").unwrap()[0].body, "kept");
}

#[test]
fn board_setting_key_matches_tauri() {
    assert!(tauri("src/features/settings/model/settings.ts").contains("\"gespan.boardEnabled\""));
    let native = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/desktop_preferences/workspace_preferences.rs"),
    )
    .unwrap();
    assert!(native.contains("(\"board\", \"boardEnabled\", true)"));
}

/// The same file is parsed by `sessionWorkItem.test.ts`, so a drift in either
/// app's ticket wire shape fails a test on one side.
#[test]
fn ticket_work_item_fixture_parses_and_serializes_back_to_itself() {
    use gespan_core::{provider_boards::StateGroup, session_groups::WorkItem, tickets::Tracker};
    let raw = tauri("tests/fixtures/shared/ticket-work-item.json");
    let item: WorkItem = serde_json::from_str(&raw).unwrap();
    let ticket = item.ticket().expect("a ticket variant");
    assert_eq!(ticket.tracker, Tracker::Jira);
    assert_eq!(ticket.key, "ENG-142");
    assert_eq!(ticket.status_group, Some(StateGroup::Started));
    assert_eq!(ticket.fetched_at, Some(1_761_350_400_000));
    let fixture: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(serde_json::to_value(&item).unwrap(), fixture);
}
