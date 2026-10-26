use gespan_board_store::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashSet;

fn fresh() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn open(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn json_of(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

fn patch_icon(icon: &str) -> SettingsPatch {
    SettingsPatch {
        icon: Some(Some(icon.into())),
        ..SettingsPatch::default()
    }
}

fn set_raw(conn: &Connection, board: &Board, column: &str, value: &str) {
    conn.execute(
        &format!("UPDATE board_settings SET {column} = ?1 WHERE board_id = ?2"),
        [value, &board.id],
    )
    .unwrap();
}

/// (name, role, colour) of each column, in order.
fn column_summary(
    conn: &Connection,
    board: &Board,
) -> Vec<(String, Option<String>, Option<String>)> {
    get_board(conn, &board.id)
        .unwrap()
        .columns
        .into_iter()
        .map(|column| (column.name, column.role, column.color))
        .collect()
}

fn label_summary(conn: &Connection, board: &Board) -> Vec<(String, String)> {
    list_labels(conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|label| (label.name, label.color))
        .collect()
}

fn field_summary(conn: &Connection, board: &Board) -> Vec<(String, String, Option<String>)> {
    list_fields(conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|field| (field.name, field.kind, field.options_json))
        .collect()
}

fn cols(
    list: &[(&str, Option<&str>, Option<&str>)],
) -> Vec<(String, Option<String>, Option<String>)> {
    list.iter()
        .map(|(name, role, color)| {
            (
                name.to_string(),
                role.map(str::to_string),
                color.map(str::to_string),
            )
        })
        .collect()
}

#[test]
fn v5_migration_is_idempotent_and_keeps_v4_views() {
    let path = tempfile::NamedTempFile::new().unwrap();
    let conn = open(path.path());
    let board = create_board(&conn, "Work").unwrap();
    let view = create_view(&conn, &board.id, "Mine", "{}", r#"{"layout":"list"}"#).unwrap();
    update_board_settings(
        &conn,
        &board.id,
        &SettingsPatch {
            write_back: Some(false),
            auto_move_json: Some(Some(r#"{"started":"stay"}"#.into())),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    // Turn it back into a v4 database: no v5 tables or settings columns.
    conn.execute_batch(
        "DROP TABLE board_templates;
         DROP TABLE board_session_settings;
         ALTER TABLE board_settings DROP COLUMN icon;
         ALTER TABLE board_settings DROP COLUMN color;
         ALTER TABLE board_settings DROP COLUMN default_view_id;
         ALTER TABLE board_settings DROP COLUMN display_json;
         ALTER TABLE board_settings DROP COLUMN priority_json;
         UPDATE board_schema SET version = 4;",
    )
    .unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 4);

    ensure_tables(&conn).unwrap();
    assert_eq!(SCHEMA_VERSION, 5);
    assert_eq!(migrate::version(&conn).unwrap(), 5);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_schema"), 1);
    assert_eq!(
        list_views(&conn, &board.id).unwrap(),
        std::slice::from_ref(&view)
    );
    // v4 settings survive and the new columns read as the defaults.
    let settings = board_settings(&conn, &board.id).unwrap();
    assert_eq!(
        settings,
        BoardSettings {
            write_back: false,
            auto_move_json: Some(r#"{"started":"stay"}"#.into()),
            ..BoardSettings::default()
        }
    );
    // The new tables and columns work.
    update_board_settings(&conn, &board.id, &patch_icon("bug")).unwrap();
    set_session_status_setting(&conn, "s1", Some(false)).unwrap();
    save_board_as_template(&conn, &board.id, "Mine").unwrap();

    ensure_tables(&conn).unwrap();
    migrate::run(&conn).unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 5);
    assert_eq!(list_views(&conn, &board.id).unwrap(), [view]);
    assert_eq!(
        board_settings(&conn, &board.id).unwrap().icon.as_deref(),
        Some("bug")
    );
    assert_eq!(session_status_setting(&conn, "s1"), Some(false));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_templates"), 1);

    // A second connection opening the same file finds the work done.
    let other = open(path.path());
    assert_eq!(migrate::version(&other).unwrap(), 5);
    assert_eq!(count(&other, "SELECT COUNT(*) FROM board_templates"), 1);
}

#[test]
fn settings_patch_changes_only_named_fields() {
    let path = tempfile::NamedTempFile::new().unwrap();
    // Two apps on one database: native toggles write-back, Tauri sets the icon.
    let native = open(path.path());
    let tauri = open(path.path());
    let board = create_board(&native, "Work").unwrap();
    update_board_settings(
        &native,
        &board.id,
        &SettingsPatch {
            color: Some(Some("4da3f5".into())),
            ..SettingsPatch::default()
        },
    )
    .unwrap();

    let after_native = update_board_settings(
        &native,
        &board.id,
        &SettingsPatch {
            write_back: Some(false),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    assert!(!after_native.write_back);
    let after_tauri = update_board_settings(&tauri, &board.id, &patch_icon("rocket")).unwrap();
    let expected = BoardSettings {
        write_back: false,
        icon: Some("rocket".into()),
        color: Some("4da3f5".into()),
        ..BoardSettings::default()
    };
    assert_eq!(after_tauri, expected);
    assert_eq!(board_settings(&native, &board.id).unwrap(), expected);

    // `Some(None)` clears just that field; an empty patch changes nothing.
    let cleared = update_board_settings(
        &tauri,
        &board.id,
        &SettingsPatch {
            icon: Some(None),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(
        cleared,
        BoardSettings {
            icon: None,
            ..expected.clone()
        }
    );
    assert_eq!(
        update_board_settings(&native, &board.id, &SettingsPatch::default()).unwrap(),
        cleared
    );

    // A board with no settings row (made before v3) gets one holding only
    // what the patch names.
    native.execute("DELETE FROM board_settings", []).unwrap();
    let patched = update_board_settings(
        &native,
        &board.id,
        &SettingsPatch {
            ticket_status_changes: Some(false),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(
        patched,
        BoardSettings {
            ticket_status_changes: false,
            ..BoardSettings::default()
        }
    );
    assert_eq!(
        update_board_settings(&native, "missing", &patch_icon("bug")).unwrap_err(),
        "Board not found"
    );
}

#[test]
fn settings_round_trip_icon_colour_default_view_display_priority() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let view = create_view(&conn, &board.id, "Mine", "{}", "{}").unwrap();
    let display = r#"{"face":{"labels":false,"fields":["f1"],"agentFields":["cost","runTime"]},"density":"compact","swimlane":"field:f1"}"#;
    let priority = r#"{"names":["None","P4","P3","P2","P1"],"hidden":[1]}"#;
    let settings = BoardSettings {
        run_with: false,
        icon: Some("star".into()),
        color: Some("a371f7".into()),
        default_view_id: Some(view.id.clone()),
        display_json: Some(display.into()),
        priority_json: Some(priority.into()),
        ..BoardSettings::default()
    };
    set_board_settings(&conn, &board.id, &settings).unwrap();
    let stored = board_settings(&conn, &board.id).unwrap();
    assert_eq!(stored, settings);

    // The shape both apps exchange is camelCase and reads back the same.
    let value = serde_json::to_value(&stored).unwrap();
    assert_eq!(value["defaultViewId"], json!(view.id));
    assert_eq!(value["displayJson"], json!(display));
    assert_eq!(
        serde_json::from_value::<BoardSettings>(value).unwrap(),
        stored
    );
    // Fields an older writer left out read as the defaults.
    assert_eq!(
        serde_json::from_str::<BoardSettings>(r#"{"writeBack":false}"#).unwrap(),
        BoardSettings {
            write_back: false,
            ..BoardSettings::default()
        }
    );

    let options = display_options(&stored);
    assert!(!options.face.labels && options.face.priority);
    assert_eq!(options.face.fields, ["f1"]);
    assert_eq!(options.face.agent_fields, ["cost", "runTime"]);
    assert_eq!(options.density, Density::Compact);
    assert_eq!(options.swimlane, "field:f1");
    assert_eq!(
        priority_scheme(&stored),
        PriorityScheme {
            names: ["None", "P4", "P3", "P2", "P1"].map(str::to_string),
            hidden: vec![1],
        }
    );

    // Bad values are refused and nothing is written.
    let refused = [
        (patch_icon("cat"), "Unknown board icon: cat"),
        (
            SettingsPatch {
                color: Some(Some("blue".into())),
                ..SettingsPatch::default()
            },
            "Colour must be six hex digits",
        ),
        (
            SettingsPatch {
                display_json: Some(Some(r#"{"swimlane":"due"}"#.into())),
                ..SettingsPatch::default()
            },
            "Unknown swimlane: due",
        ),
        (
            SettingsPatch {
                display_json: Some(Some(r#"{"face":{"agentFields":["tokens"]}}"#.into())),
                ..SettingsPatch::default()
            },
            "Unknown agent field: tokens",
        ),
        (
            SettingsPatch {
                display_json: Some(Some(r#"{"density":"roomy"}"#.into())),
                ..SettingsPatch::default()
            },
            "Display options is not valid",
        ),
        (
            SettingsPatch {
                priority_json: Some(Some("{".into())),
                ..SettingsPatch::default()
            },
            "Priority scheme is not valid",
        ),
        (
            SettingsPatch {
                auto_move_json: Some(Some("{".into())),
                ..SettingsPatch::default()
            },
            "Auto-move rules are not valid JSON",
        ),
        (
            SettingsPatch {
                auto_move_json: Some(Some(format!(r#"{{"x":"{}"}}"#, "a".repeat(20_000)))),
                ..SettingsPatch::default()
            },
            "Auto-move rules are too long",
        ),
    ];
    for (patch, error) in refused {
        let mut with_toggle = patch;
        with_toggle.write_back = Some(false);
        assert_eq!(
            update_board_settings(&conn, &board.id, &with_toggle).unwrap_err(),
            error
        );
    }
    assert_eq!(board_settings(&conn, &board.id).unwrap(), settings);
    for icon in BOARD_ICONS {
        update_board_settings(&conn, &board.id, &patch_icon(icon)).unwrap();
    }
}

#[test]
fn default_view_must_belong_to_the_board() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let other = create_board(&conn, "Other").unwrap();
    let mine = create_view(&conn, &board.id, "Mine", "{}", "{}").unwrap();
    let theirs = create_view(&conn, &other.id, "Theirs", "{}", "{}").unwrap();
    let default_view = |id: &str| SettingsPatch {
        default_view_id: Some(Some(id.into())),
        write_back: Some(false),
        ..SettingsPatch::default()
    };

    for id in [theirs.id.as_str(), "missing"] {
        assert_eq!(
            update_board_settings(&conn, &board.id, &default_view(id)).unwrap_err(),
            "The default view must be a view on this board"
        );
    }
    // The refused patch changed nothing, not even its other fields.
    assert_eq!(
        board_settings(&conn, &board.id).unwrap(),
        BoardSettings::default()
    );
    let set = update_board_settings(&conn, &board.id, &default_view(&mine.id)).unwrap();
    assert_eq!(set.default_view_id.as_deref(), Some(mine.id.as_str()));
    assert!(!set.write_back);
    // "All cards" again.
    let cleared = update_board_settings(
        &conn,
        &board.id,
        &SettingsPatch {
            default_view_id: Some(None),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(cleared.default_view_id, None);
}

#[test]
fn deleting_the_default_view_clears_it() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let other = create_board(&conn, "Other").unwrap();
    let first = create_view(&conn, &board.id, "First", "{}", "{}").unwrap();
    let second = create_view(&conn, &board.id, "Second", "{}", "{}").unwrap();
    let theirs = create_view(&conn, &other.id, "Theirs", "{}", "{}").unwrap();
    let default_view = |id: &str| SettingsPatch {
        default_view_id: Some(Some(id.into())),
        ..SettingsPatch::default()
    };
    update_board_settings(&conn, &board.id, &default_view(&first.id)).unwrap();
    update_board_settings(&conn, &other.id, &default_view(&theirs.id)).unwrap();
    update_board_settings(&conn, &board.id, &patch_icon("flag")).unwrap();

    // Deleting another view keeps the default.
    delete_view(&conn, &second.id).unwrap();
    assert_eq!(
        board_settings(&conn, &board.id)
            .unwrap()
            .default_view_id
            .as_deref(),
        Some(first.id.as_str())
    );
    delete_view(&conn, &first.id).unwrap();
    let settings = board_settings(&conn, &board.id).unwrap();
    assert_eq!(settings.default_view_id, None);
    assert_eq!(settings.icon.as_deref(), Some("flag"));
    assert_eq!(
        board_settings(&conn, &other.id)
            .unwrap()
            .default_view_id
            .as_deref(),
        Some(theirs.id.as_str())
    );
}

#[test]
fn priority_scheme_refuses_empty_names_and_hiding_none() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let write = |raw: &str| {
        update_board_settings(
            &conn,
            &board.id,
            &SettingsPatch {
                priority_json: Some(Some(raw.into())),
                ..SettingsPatch::default()
            },
        )
    };
    let names_error = "Priority names must be 1 to 20 characters";
    let long = "x".repeat(21);
    let cases = [
        (
            r#"{"names":["None","","Medium","High","Urgent"]}"#.to_string(),
            names_error,
        ),
        (
            r#"{"names":["None","   ","Medium","High","Urgent"]}"#.to_string(),
            names_error,
        ),
        (
            format!(r#"{{"names":["None","{long}","Medium","High","Urgent"]}}"#),
            names_error,
        ),
        (
            r#"{"hidden":[0]}"#.to_string(),
            "Priority level 0 can't be hidden",
        ),
        (
            r#"{"hidden":[2,0]}"#.to_string(),
            "Priority level 0 can't be hidden",
        ),
        (
            r#"{"hidden":[5]}"#.to_string(),
            "Hidden priority levels are not valid",
        ),
        (
            r#"{"hidden":[2,2]}"#.to_string(),
            "Hidden priority levels are not valid",
        ),
        (
            r#"{"names":["None","Low"]}"#.to_string(),
            "Priority scheme is not valid",
        ),
    ];
    for (raw, error) in cases {
        assert_eq!(write(&raw).unwrap_err(), error, "{raw}");
    }
    assert_eq!(
        board_settings(&conn, &board.id).unwrap().priority_json,
        None
    );

    // 20 characters (not bytes) is fine, as is hiding every other level.
    // Names are stored trimmed, as normalised JSON (unknown keys dropped).
    let twenty = "é".repeat(20);
    let raw = format!(
        r#"{{"names":[" None","  {twenty}  ","Medium","High","Urgent "],"hidden":[4,1,3,2],"x":1}}"#
    );
    let settings = write(&raw).unwrap();
    assert_eq!(
        json_of(settings.priority_json.as_deref().unwrap()),
        json!({"names": ["None", twenty, "Medium", "High", "Urgent"], "hidden": [4, 1, 3, 2]})
    );
    let scheme = priority_scheme(&settings);
    assert_eq!(scheme.names[1], twenty);
    assert_eq!(scheme.hidden, [4, 1, 3, 2]);
    // Missing keys take the defaults.
    assert_eq!(
        priority_scheme(&write(r#"{"hidden":[1]}"#).unwrap()),
        PriorityScheme {
            hidden: vec![1],
            ..PriorityScheme::default()
        }
    );
    assert_eq!(
        PriorityScheme::default().names,
        ["None", "Low", "Medium", "High", "Urgent"]
    );
    // A bad value written some other way reads as the defaults.
    set_raw(&conn, &board, "priority_json", r#"{"hidden":[0]}"#);
    assert_eq!(
        priority_scheme(&board_settings(&conn, &board.id).unwrap()),
        PriorityScheme::default()
    );
}

#[test]
fn display_options_default_when_missing_or_unknown_keys() {
    let conn = fresh();
    let board = create_board(&conn, "Work").unwrap();
    let defaults = display_options(&board_settings(&conn, &board.id).unwrap());
    let face = &defaults.face;
    assert!(
        face.labels
            && face.priority
            && face.due
            && face.subtasks
            && face.comments
            && face.link_status
            && face.number
    );
    assert!(face.fields.is_empty() && face.agent_fields.is_empty());
    assert_eq!(defaults.density, Density::Comfortable);
    assert_eq!(defaults.swimlane, "none");
    assert_eq!(defaults, DisplayOptions::default());

    let read = |raw: &str| {
        set_raw(&conn, &board, "display_json", raw);
        display_options(&board_settings(&conn, &board.id).unwrap())
    };
    // Unknown keys are ignored; bad JSON or bad values read as the defaults.
    for raw in [
        r#"{"future":{"x":1}}"#,
        "{",
        "[]",
        r#"{"density":"roomy"}"#,
        r#"{"swimlane":"due"}"#,
        r#"{"face":{"agentFields":["tokens"]}}"#,
    ] {
        assert_eq!(read(raw), DisplayOptions::default(), "{raw}");
    }
    // Keys that are present apply; the rest stay default.
    let partial =
        read(r#"{"face":{"due":false,"linkStatus":false},"future":1,"swimlane":"label"}"#);
    assert!(!partial.face.due && !partial.face.link_status && partial.face.labels);
    assert_eq!(partial.swimlane, "label");
    assert_eq!(partial.density, Density::Comfortable);
    // Writing unknown keys is allowed too (a newer build may add some).
    update_board_settings(
        &conn,
        &board.id,
        &SettingsPatch {
            display_json: Some(Some(r#"{"future":1,"swimlane":"project"}"#.into())),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
}

#[test]
fn session_override_wins_else_strictest_linked_board_else_on() {
    let conn = fresh();
    let open_board = create_board(&conn, "Open").unwrap();
    let strict = create_board(&conn, "Strict").unwrap();
    update_board_settings(
        &conn,
        &strict.id,
        &SettingsPatch {
            ticket_status_changes: Some(false),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    let column = |board: &Board| get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    let on_open = create_card(&conn, &column(&open_board), "a", "").unwrap();
    let on_strict = create_card(&conn, &column(&strict), "b", "").unwrap();

    // No card links it: on.
    assert!(may_change_ticket_status(&conn, "s1"));
    add_link(&conn, &on_open.id, "session", "s1", None).unwrap();
    assert!(may_change_ticket_status(&conn, "s1"));
    // A pilot link with the same reference is not this session.
    add_link(&conn, &on_strict.id, "pilot", "s1", None).unwrap();
    assert!(may_change_ticket_status(&conn, "s1"));
    // Linked on a board that forbids it too: the stricter wins.
    add_link(&conn, &on_strict.id, "session", "s1", None).unwrap();
    assert!(!may_change_ticket_status(&conn, "s1"));
    // A board with no settings row reads as on, so the strict board decides.
    conn.execute(
        "DELETE FROM board_settings WHERE board_id = ?1",
        [&open_board.id],
    )
    .unwrap();
    assert!(!may_change_ticket_status(&conn, "s1"));

    // The session override wins in both directions.
    assert_eq!(session_status_setting(&conn, "s1"), None);
    set_session_status_setting(&conn, "s1", Some(true)).unwrap();
    assert_eq!(session_status_setting(&conn, "s1"), Some(true));
    assert!(may_change_ticket_status(&conn, "s1"));
    set_session_status_setting(&conn, "s2", Some(false)).unwrap();
    assert!(!may_change_ticket_status(&conn, "s2"));
    // Back to following the boards.
    set_session_status_setting(&conn, "s1", None).unwrap();
    assert_eq!(session_status_setting(&conn, "s1"), None);
    assert!(!may_change_ticket_status(&conn, "s1"));
    // Unlinking the strict card leaves only the open board.
    remove_link(&conn, &on_strict.id, "session", "s1").unwrap();
    assert!(may_change_ticket_status(&conn, "s1"));
    assert!(set_session_status_setting(&conn, " ", Some(true)).is_err());
}

#[test]
fn session_link_on_an_archived_card_still_counts() {
    let conn = fresh();
    let open_board = create_board(&conn, "Open").unwrap();
    let strict = create_board(&conn, "Strict").unwrap();
    update_board_settings(
        &conn,
        &strict.id,
        &SettingsPatch {
            ticket_status_changes: Some(false),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    let column = |board: &Board| get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    let live = create_card(&conn, &column(&open_board), "a", "").unwrap();
    let archived = create_card(&conn, &column(&strict), "b", "").unwrap();
    add_link(&conn, &live.id, "session", "s1", None).unwrap();
    add_link(&conn, &archived.id, "session", "s1", None).unwrap();
    archive_card(&conn, &archived.id).unwrap();
    assert!(!may_change_ticket_status(&conn, "s1"));
    // Unarchiving changes nothing; unlinking it does.
    unarchive_card(&conn, &archived.id).unwrap();
    assert!(!may_change_ticket_status(&conn, "s1"));
    archive_card(&conn, &archived.id).unwrap();
    remove_link(&conn, &archived.id, "session", "s1").unwrap();
    assert!(may_change_ticket_status(&conn, "s1"));
}

#[test]
fn an_unreadable_session_override_refuses() {
    let conn = fresh();
    let board = create_board(&conn, "Open").unwrap();
    let column = get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    let card = create_card(&conn, &column, "a", "").unwrap();
    // The board alone would allow it; the override says no.
    add_link(&conn, &card.id, "session", "s1", None).unwrap();
    set_session_status_setting(&conn, "s1", Some(false)).unwrap();
    assert!(!may_change_ticket_status(&conn, "s1"));

    // When the override can't be read, the answer is no, never the boards'.
    conn.execute_batch("ALTER TABLE board_session_settings RENAME TO moved")
        .unwrap();
    assert_eq!(session_status_setting(&conn, "s1"), None);
    assert!(!may_change_ticket_status(&conn, "s1"));
    // That includes a session with no override that would otherwise be on.
    assert!(!may_change_ticket_status(&conn, "s2"));

    conn.execute_batch("ALTER TABLE moved RENAME TO board_session_settings")
        .unwrap();
    assert!(!may_change_ticket_status(&conn, "s1"));
    assert!(may_change_ticket_status(&conn, "s2"));
}

#[test]
fn each_built_in_template_creates_its_columns_labels_and_fields() {
    let conn = fresh();
    assert_eq!(
        BUILT_IN_TEMPLATES,
        ["blank", "agent-pipeline", "bug-triage", "release-checklist"]
    );
    let listed: Vec<(String, String, bool)> = list_templates(&conn)
        .unwrap()
        .into_iter()
        .map(|t| (t.id, t.name, t.built_in))
        .collect();
    let built_in = |id: &str, name: &str| (id.to_string(), name.to_string(), true);
    assert_eq!(
        listed,
        [
            built_in("blank", "Blank"),
            built_in("agent-pipeline", "Agent pipeline"),
            built_in("bug-triage", "Bug triage"),
            built_in("release-checklist", "Release checklist"),
        ]
    );

    let blank = create_board_from_template(&conn, "Blank", "blank").unwrap();
    let plain = create_board(&conn, "Plain").unwrap();
    let default_columns = cols(&[
        ("Backlog", Some("todo"), None),
        ("In Progress", Some("doing"), None),
        ("Review", Some("review"), None),
        ("Done", Some("done"), None),
    ]);
    assert_eq!(column_summary(&conn, &blank), default_columns);
    assert_eq!(column_summary(&conn, &plain), default_columns);
    for board in [&blank, &plain] {
        assert!(list_labels(&conn, &board.id).unwrap().is_empty());
        assert!(list_fields(&conn, &board.id).unwrap().is_empty());
        assert!(list_views(&conn, &board.id).unwrap().is_empty());
        assert_eq!(
            board_settings(&conn, &board.id).unwrap(),
            BoardSettings::default()
        );
    }

    let pipeline = create_board_from_template(&conn, "Agents", "agent-pipeline").unwrap();
    assert_eq!(pipeline.name, "Agents");
    assert_eq!(
        column_summary(&conn, &pipeline),
        cols(&[
            ("Queued", Some("todo"), None),
            ("Running", Some("doing"), None),
            ("Needs review", Some("review"), None),
            ("Merged", None, Some("a371f7")),
            ("Done", Some("done"), None),
        ])
    );
    assert!(list_labels(&conn, &pipeline.id).unwrap().is_empty());
    assert!(list_fields(&conn, &pipeline.id).unwrap().is_empty());

    let bugs = create_board_from_template(&conn, "Bugs", "bug-triage").unwrap();
    assert_eq!(
        column_summary(&conn, &bugs),
        cols(&[
            ("New", Some("todo"), None),
            ("Triaged", None, None),
            ("In progress", Some("doing"), None),
            ("Fixed", Some("review"), None),
            ("Closed", Some("done"), None),
            ("Won't fix", Some("cancelled"), None),
        ])
    );
    assert_eq!(
        label_summary(&conn, &bugs),
        [
            ("Bug".to_string(), "e5534b".to_string()),
            ("Regression".into(), "e0823d".into()),
            ("Needs repro".into(), "e8c547".into()),
        ]
    );
    let fields = field_summary(&conn, &bugs);
    assert_eq!(fields.len(), 1);
    assert_eq!(
        (fields[0].0.as_str(), fields[0].1.as_str()),
        ("Severity", "select")
    );
    assert_eq!(
        json_of(fields[0].2.as_deref().unwrap()),
        json!(["S1", "S2", "S3", "S4"])
    );
    // The select field takes its options.
    let severity = &list_fields(&conn, &bugs.id).unwrap()[0];
    let card = create_card(
        &conn,
        &get_board(&conn, &bugs.id).unwrap().columns[0].id,
        "x",
        "",
    )
    .unwrap();
    set_field_value(&conn, &card.id, &severity.id, Some(r#""S2""#)).unwrap();

    let release = create_board_from_template(&conn, "Release", "release-checklist").unwrap();
    assert_eq!(
        column_summary(&conn, &release),
        cols(&[
            ("To do", Some("todo"), None),
            ("In progress", Some("doing"), None),
            ("Blocked", None, Some("e5534b")),
            ("Done", Some("done"), None),
        ])
    );
    assert_eq!(
        label_summary(&conn, &release),
        [
            ("Docs".to_string(), "4da3f5".to_string()),
            ("QA".into(), "3fb950".into()),
            ("Ops".into(), "2bb3a8".into()),
        ]
    );
    assert_eq!(
        field_summary(&conn, &release),
        [("Release".to_string(), "text".to_string(), None)]
    );
    let views = list_views(&conn, &release.id).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "By label");
    assert_eq!(
        json_of(&views[0].layout_json),
        json!({"layout": "list", "groupBy": "label"})
    );
    let spec: ViewSpec = serde_json::from_str(&views[0].spec_json).unwrap();
    assert_eq!(spec, ViewSpec::default());

    assert_eq!(
        create_board_from_template(&conn, "X", "missing").unwrap_err(),
        "Template not found"
    );
    assert_eq!(
        create_board_from_template(&conn, "  ", "blank").unwrap_err(),
        "Board name is required"
    );
}

#[test]
fn agent_pipeline_template_has_merged_rule_on_the_new_column_id() {
    let conn = fresh();
    let first = create_board_from_template(&conn, "One", "agent-pipeline").unwrap();
    let second = create_board_from_template(&conn, "Two", "agent-pipeline").unwrap();
    let mut merged_ids = HashSet::new();
    for board in [&first, &second] {
        let merged = get_board(&conn, &board.id)
            .unwrap()
            .columns
            .into_iter()
            .find(|column| column.name == "Merged")
            .unwrap();
        let rules = json_of(
            board_settings(&conn, &board.id)
                .unwrap()
                .auto_move_json
                .as_deref()
                .unwrap(),
        );
        assert_eq!(
            rules,
            json!({
                "started": "default",
                "needs_input": "stay",
                "completed": "default",
                "failed": "stay",
                "pr_opened": "stay",
                "pr_merged": {"column": merged.id},
            })
        );
        merged_ids.insert(merged.id);
    }
    // Each board's rule points at its own column.
    assert_eq!(merged_ids.len(), 2);
}

#[test]
fn save_as_template_copies_structure_and_settings_but_no_cards() {
    let conn = fresh();
    let source = create_board(&conn, "Source").unwrap();
    let columns = get_board(&conn, &source.id).unwrap().columns;
    update_column(&conn, &columns[0].id, "Inbox", Some("todo")).unwrap();
    let extra = create_column(&conn, &source.id, "Parked", None).unwrap();
    set_column_display(&conn, &extra.id, Some("e8c547"), Some(3), false).unwrap();
    let label = create_label(&conn, &source.id, "Bug", "e5534b").unwrap();
    let field = create_field(&conn, &source.id, "Size", "select", Some(r#"["S","M"]"#)).unwrap();
    let view = create_view(
        &conn,
        &source.id,
        "Bugs",
        &json!({"rows": [{"field": "labels", "op": "is", "values": [label.id]}]}).to_string(),
        r#"{"layout":"table"}"#,
    )
    .unwrap();
    let card = create_card(&conn, &columns[0].id, "Work", "").unwrap();
    set_card_labels(&conn, &card.id, std::slice::from_ref(&label.id)).unwrap();
    let priority = r#"{"names":["None","Low","Mid","High","Fire"],"hidden":[2]}"#;
    update_board_settings(
        &conn,
        &source.id,
        &SettingsPatch {
            write_back: Some(false),
            icon: Some(Some("bolt".into())),
            color: Some(Some("3fb950".into())),
            default_view_id: Some(Some(view.id.clone())),
            display_json: Some(Some(
                json!({"face": {"fields": [field.id]}, "swimlane": format!("field:{}", field.id)})
                    .to_string(),
            )),
            priority_json: Some(Some(priority.into())),
            ..SettingsPatch::default()
        },
    )
    .unwrap();

    let saved = save_board_as_template(&conn, &source.id, "  Team flow ").unwrap();
    assert_eq!(saved.name, "Team flow");
    assert!(!saved.built_in);
    let made = create_board_from_template(&conn, "Copy", &saved.id).unwrap();

    let detail = get_board(&conn, &made.id).unwrap();
    assert!(detail.cards.is_empty());
    assert!(card_index(&conn, &made.id).unwrap().is_empty());
    assert_eq!(column_summary(&conn, &made), column_summary(&conn, &source));
    let parked = &detail.columns[4];
    assert_eq!(parked.wip_limit, Some(3));
    assert_ne!(parked.id, extra.id);
    assert_eq!(label_summary(&conn, &made), label_summary(&conn, &source));
    assert_eq!(field_summary(&conn, &made), field_summary(&conn, &source));
    let new_label = &list_labels(&conn, &made.id).unwrap()[0];
    let new_field = &list_fields(&conn, &made.id).unwrap()[0];
    assert_ne!(new_label.id, label.id);
    assert_ne!(new_field.id, field.id);

    let views = list_views(&conn, &made.id).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Bugs");
    assert_ne!(views[0].id, view.id);
    assert_eq!(
        json_of(&views[0].spec_json)["rows"][0]["values"],
        json!([new_label.id])
    );

    let settings = board_settings(&conn, &made.id).unwrap();
    assert!(!settings.write_back && settings.run_with && settings.ticket_status_changes);
    assert_eq!(settings.icon.as_deref(), Some("bolt"));
    assert_eq!(settings.color.as_deref(), Some("3fb950"));
    assert_eq!(
        settings.default_view_id.as_deref(),
        Some(views[0].id.as_str())
    );
    assert_eq!(settings.priority_json.as_deref(), Some(priority));
    let display = display_options(&settings);
    assert_eq!(display.face.fields, std::slice::from_ref(&new_field.id));
    assert_eq!(display.swimlane, format!("field:{}", new_field.id));

    // The source is untouched, and the template outlives it.
    assert_eq!(get_board(&conn, &source.id).unwrap().cards.len(), 1);
    delete_board(&conn, &source.id).unwrap();
    let again = create_board_from_template(&conn, "Again", &saved.id).unwrap();
    assert_eq!(get_board(&conn, &again.id).unwrap().columns.len(), 5);

    // Saved templates list after the built-ins, by name.
    let empty = create_board(&conn, "Empty").unwrap();
    save_board_as_template(&conn, &empty.id, "alpha").unwrap();
    let names: Vec<String> = list_templates(&conn)
        .unwrap()
        .into_iter()
        .filter(|t| !t.built_in)
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["alpha", "Team flow"]);
    assert_eq!(list_templates(&conn).unwrap().len(), 6);
    assert_eq!(
        save_board_as_template(&conn, "missing", "Nope").unwrap_err(),
        "Board not found"
    );
}

#[test]
fn template_view_rows_and_layout_columns_are_remapped() {
    let conn = fresh();
    let source = create_board(&conn, "Source").unwrap();
    let columns = get_board(&conn, &source.id).unwrap().columns;
    let (todo, done) = (&columns[0], &columns[3]);
    let label = create_label(&conn, &source.id, "Bug", "e5534b").unwrap();
    let field = create_field(&conn, &source.id, "Size", "select", Some(r#"["S","M"]"#)).unwrap();
    let f = format!("field:{}", field.id);
    // Ids named "gone-*" no longer exist on the board (deleted since saving).
    let spec = json!({
        "query": "fix",
        "rows": [
            {"field": "status", "op": "is", "values": [todo.id, "gone-col"]},
            {"field": "status", "op": "is_not", "values": ["gone-col"]},
            {"field": "labels", "op": "is", "values": [label.id, "gone-label"]},
            {"field": f, "op": "is", "values": ["\"S\""]},
            {"field": "field:gone-field", "op": "is", "values": ["1"]},
            {"field": "priority", "op": "is", "values": ["3"]},
            {"field": "projects", "op": "is", "values": ["/repo"]},
        ],
        "sort": {"key": "due", "dir": "desc"},
    });
    let layout = json!({
        "layout": "table",
        "groupBy": "label",
        "columns": ["title", f, "field:gone-field", "status"],
        "widths": {"title": 300, f.as_str(): 200, "field:gone-field": 90},
        "collapsed": [
            format!("status:{}", done.id), format!("label:{}", label.id),
            "label:none", "priority:p1", "status:gone-col", "label:gone-label",
        ],
        "display": {"face": {"fields": [field.id, "gone-field"], "due": false}, "swimlane": f},
        "future": {"x": 1},
    });
    let view = create_view(
        &conn,
        &source.id,
        "Mine",
        &spec.to_string(),
        &layout.to_string(),
    )
    .unwrap();
    create_view(
        &conn,
        &source.id,
        "Gone lane",
        "{}",
        r#"{"display":{"swimlane":"field:gone-field"}}"#,
    )
    .unwrap();
    update_board_settings(
        &conn,
        &source.id,
        &SettingsPatch {
            default_view_id: Some(Some(view.id.clone())),
            display_json: Some(Some(r#"{"swimlane":"field:gone-field","density":"compact"}"#.into())),
            auto_move_json: Some(Some(
                json!({"completed": {"column": done.id}, "failed": {"column": "gone-col"}, "started": "default"})
                    .to_string(),
            )),
            ..SettingsPatch::default()
        },
    )
    .unwrap();

    let template = save_board_as_template(&conn, &source.id, "Remap").unwrap();
    let made = create_board_from_template(&conn, "Made", &template.id).unwrap();
    let new_columns = get_board(&conn, &made.id).unwrap().columns;
    let (new_todo, new_done) = (&new_columns[0], &new_columns[3]);
    let new_label = &list_labels(&conn, &made.id).unwrap()[0];
    let new_field = &list_fields(&conn, &made.id).unwrap()[0];
    let nf = format!("field:{}", new_field.id);
    let views = list_views(&conn, &made.id).unwrap();

    assert_eq!(
        json_of(&views[0].spec_json),
        json!({
            "query": "fix",
            "rows": [
                {"field": "status", "op": "is", "values": [new_todo.id]},
                {"field": "labels", "op": "is", "values": [new_label.id]},
                {"field": nf, "op": "is", "values": ["\"S\""]},
                {"field": "priority", "op": "is", "values": ["3"]},
                {"field": "projects", "op": "is", "values": ["/repo"]},
            ],
            "sort": {"key": "due", "dir": "desc"},
        })
    );
    assert_eq!(
        json_of(&views[0].layout_json),
        json!({
            "layout": "table",
            "groupBy": "label",
            "columns": ["title", nf, "status"],
            "widths": {"title": 300, nf.as_str(): 200},
            "collapsed": [
                format!("status:{}", new_done.id), format!("label:{}", new_label.id),
                "label:none", "priority:p1",
            ],
            "display": {"face": {"fields": [new_field.id], "due": false}, "swimlane": nf},
            "future": {"x": 1},
        })
    );
    // A swimlane on a field that has no match falls back to none.
    assert_eq!(
        json_of(&views[1].layout_json),
        json!({"display": {"swimlane": "none"}})
    );
    // The remapped spec runs on the new board.
    let card = create_card(&conn, &new_todo.id, "fix it", "").unwrap();
    set_card_labels(&conn, &card.id, std::slice::from_ref(&new_label.id)).unwrap();
    set_field_value(&conn, &card.id, &new_field.id, Some(r#""S""#)).unwrap();
    set_priority(&conn, &card.id, 3).unwrap();
    set_projects(&conn, &card.id, &["/repo".to_string()]).unwrap();
    let parsed: ViewSpec = serde_json::from_str(&views[0].spec_json).unwrap();
    assert_eq!(
        query_cards(
            &conn,
            &made.id,
            &parsed,
            now_millis(),
            &local_day_chrono,
            &HashSet::new()
        )
        .unwrap(),
        [card.id]
    );

    let settings = board_settings(&conn, &made.id).unwrap();
    assert_eq!(
        settings.default_view_id.as_deref(),
        Some(views[0].id.as_str())
    );
    assert_eq!(
        json_of(settings.display_json.as_deref().unwrap()),
        json!({"swimlane": "none", "density": "compact"})
    );
    // A rule whose column has no match stays put.
    assert_eq!(
        json_of(settings.auto_move_json.as_deref().unwrap()),
        json!({"completed": {"column": new_done.id}, "failed": "stay", "started": "default"})
    );
}

#[test]
fn built_in_templates_cannot_be_deleted() {
    let conn = fresh();
    for id in BUILT_IN_TEMPLATES {
        assert_eq!(
            delete_template(&conn, id).unwrap_err(),
            "Built-in templates can't be deleted"
        );
        create_board_from_template(&conn, id, id).unwrap();
    }
    assert_eq!(
        delete_template(&conn, "missing").unwrap_err(),
        "Template not found"
    );
    let board = create_board(&conn, "Work").unwrap();
    let saved = save_board_as_template(&conn, &board.id, "Mine").unwrap();
    delete_template(&conn, &saved.id).unwrap();
    assert_eq!(
        list_templates(&conn).unwrap().len(),
        BUILT_IN_TEMPLATES.len()
    );
    assert_eq!(
        create_board_from_template(&conn, "Again", &saved.id).unwrap_err(),
        "Template not found"
    );
}
