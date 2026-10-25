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

/// The native board source, as the Tauri board source is read with `tauri`.
fn native_board() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/board/mod.rs")).unwrap()
}

/// Every native board source file, joined.
fn native_board_sources() -> String {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/board"),
        &mut files,
    );
    files
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Board DDL written outside `gespan-board-schema`, which owns all of it.
fn local_board_ddl(source: &str) -> Vec<String> {
    regex::Regex::new(r"CREATE (?:UNIQUE )?(?:TABLE|INDEX) IF NOT EXISTS board")
        .unwrap()
        .find_iter(source)
        .map(|statement| statement.as_str().to_string())
        .collect()
}

#[test]
fn both_apps_use_the_shared_board_schema() {
    let native = native_board_sources();
    let tauri = tauri("src-tauri/src/board.rs");
    for (app, source) in [("native", &native), ("Tauri", &tauri)] {
        assert!(
            source.contains("gespan_board_schema::ensure"),
            "{app} does not open the shared schema"
        );
        assert_eq!(local_board_ddl(source), Vec::<String>::new(), "{app}");
    }
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
        "Unknown link kind: {kind}",
        "A link needs both a kind and an id",
    ] {
        assert!(tauri_source.contains(message), "Tauri lacks {message}");
        assert!(native_source.contains(message), "native lacks {message}");
    }
}

#[test]
fn opening_the_board_service_creates_exactly_the_v3_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    drop(Store::open_gespan(&path).unwrap());
    drop(gespan_core::board::Service::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let strings = |sql: &str| -> Vec<String> {
        conn.prepare(sql)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    // The contract, written out on purpose: changing the schema means
    // changing this list and docs/board-schema.md together.
    let expected: [(&str, &[&str]); 15] = [
        ("board_activity", &["id", "board_id", "card_id", "kind", "actor", "from_column", "to_column", "detail", "created_at"]),
        ("board_card_field_values", &["card_id", "field_id", "value_json"]),
        ("board_card_labels", &["card_id", "label_id"]),
        ("board_card_links", &["card_id", "kind", "ref", "role", "created_at"]),
        ("board_card_meta", &["card_id", "priority", "due_at", "start_at", "estimate_minutes", "archived_at", "completed_at"]),
        ("board_card_projects", &["card_id", "project"]),
        ("board_card_relations", &["card_id", "related_card_id", "kind", "created_at"]),
        ("board_cards", &["id", "board_id", "column_id", "number", "title", "body", "position", "link_kind", "link_id", "created_at", "updated_at"]),
        ("board_columns", &["id", "board_id", "name", "position", "role", "color", "wip_limit", "collapsed"]),
        ("board_comments", &["id", "card_id", "body", "author", "created_at", "edited_at"]),
        ("board_fields", &["id", "board_id", "name", "kind", "options_json", "position"]),
        ("board_labels", &["id", "board_id", "name", "color", "position"]),
        ("board_schema_migrations", &["version", "applied_at"]),
        ("board_subtasks", &["id", "card_id", "title", "done", "position", "converted_card_id"]),
        ("boards", &["id", "name", "card_seq", "created_at", "updated_at"]),
    ];
    let tables = strings(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'board%' ORDER BY name",
    );
    let mut shared: Vec<&str> = gespan_board_schema::BOARD_TABLES.to_vec();
    shared.push("board_schema_migrations");
    shared.sort();
    assert_eq!(tables, shared, "no board_native_* tables on a fresh database");
    assert_eq!(
        tables,
        expected.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    );
    for (table, columns) in expected {
        assert_eq!(
            strings(&format!(
                "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
            )),
            columns,
            "{table}"
        );
    }
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

/// A committed cross-app board fixture in `crates/board-schema/fixtures`.
fn board_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../board-schema/fixtures")
        .join(name)
}

fn raw_strings(conn: &rusqlite::Connection, sql: &str) -> Vec<String> {
    conn.prepare(sql)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn native_writes_the_committed_fixture() {
    use gespan_board_schema::testing::{dump, fixture_matches, SEED_DUE_AT};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let mut service = gespan_core::board::Service::open(&path).unwrap();
    let board = service.create("Fixture").unwrap();
    let columns = service.get(&board.id).unwrap().columns;
    let first = service.card_create(&columns[0].id, "First", "").unwrap();
    service.card_create(&columns[0].id, "Second", "").unwrap();
    let third = service.card_create(&columns[0].id, "Third", "").unwrap();
    service
        .card_move(&third.id, &columns[1].id, 0, "user")
        .unwrap();
    service.set_priority(&first.id, 3).unwrap();
    service.set_due(&first.id, Some(SEED_DUE_AT)).unwrap();
    service
        .set_projects(&first.id, &["/work/gespan".into()])
        .unwrap();
    let bug = service.create_label(&board.id, "Bug", "d73a4a").unwrap();
    service.set_labels(&first.id, &[bug.id]).unwrap();
    let write_test = service.add_subtask(&first.id, "Write test").unwrap();
    service.toggle_subtask(&write_test.id, true).unwrap();
    service.add_subtask(&first.id, "Ship").unwrap();
    service.add_comment(&first.id, "Looks good").unwrap();
    service
        .card_link(&first.id, Some("session"), Some("session-fixture"))
        .unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    fixture_matches(&board_fixture("written_by_native.sql"), &dump(&conn));
}

#[test]
fn native_opens_a_database_written_by_tauri() {
    use gespan_board_schema::testing::load;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    drop(gespan_core::board::Service::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let fixture = fs::read_to_string(board_fixture("written_by_tauri.sql")).unwrap();
        load(&conn, &fixture).unwrap();
    }
    let mut service = gespan_core::board::Service::open(&path).unwrap();
    let board = service.list().unwrap().remove(0);
    let detail = service.get(&board.id).unwrap();
    let names: Vec<&str> = detail.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Queue", "In Progress", "Review", "Done", "Dropped"]);
    assert_eq!(detail.columns[4].role.as_deref(), Some("cancelled"));
    let linked = service.cards_for_link("pilot", "pilot-fixture").unwrap();
    assert_eq!(
        linked.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
        ["Alpha"]
    );
    let beta = detail.cards.iter().find(|c| c.title == "Beta").unwrap();
    assert_eq!(beta.body, "Notes");
    assert_eq!(beta.column_id, detail.columns[2].id);
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw_strings(
            &conn,
            "SELECT c.title || ':' || l.kind || ':' || l.ref || ':' || l.role
             FROM board_card_links l JOIN board_cards c ON c.id = l.card_id"
        ),
        ["Alpha:pilot:pilot-fixture:primary"]
    );
    // Native's own writes still work on Tauri's rows.
    service.set_priority(&beta.id, 1).unwrap();
    assert_eq!(service.extras(&board.id).unwrap()[&beta.id].priority, 1);
}
