//! Label manager and custom-field management (C5-2).
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

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn json_of(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

fn board_with_cards(conn: &Connection, titles: &[&str]) -> (Board, Vec<Card>) {
    let board = create_board(conn, "Work").unwrap();
    let column = get_board(conn, &board.id).unwrap().columns[0].id.clone();
    let cards = titles
        .iter()
        .map(|title| create_card(conn, &column, title, "").unwrap())
        .collect();
    (board, cards)
}

fn label_names(conn: &Connection, board: &Board) -> Vec<(String, String, i64)> {
    list_labels(conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|label| (label.name, label.color, label.position))
        .collect()
}

fn card_label_ids(conn: &Connection, card: &Card) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT label_id FROM board_card_labels WHERE card_id = ?1 ORDER BY label_id")
        .unwrap();
    let ids = statement
        .query_map([&card.id], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    ids
}

fn value_of(conn: &Connection, card: &Card, field: &Field) -> Option<String> {
    field_values(conn, &card.id)
        .unwrap()
        .into_iter()
        .find(|value| value.field_id == field.id)
        .map(|value| value.value_json)
}

fn view(conn: &Connection, id: &str) -> View {
    list_views(conn, &get_view_board(conn, id))
        .unwrap()
        .into_iter()
        .find(|view| view.id == id)
        .unwrap()
}

fn get_view_board(conn: &Connection, id: &str) -> String {
    conn.query_row(
        "SELECT board_id FROM board_views WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn run(conn: &Connection, board: &Board, spec: Value) -> Vec<String> {
    let spec: ViewSpec = serde_json::from_value(spec).unwrap();
    query_cards(
        conn,
        &board.id,
        &spec,
        now_millis(),
        &local_day_chrono,
        &HashSet::new(),
    )
    .unwrap()
}

fn ids(cards: &[&Card]) -> Vec<String> {
    cards.iter().map(|card| card.id.clone()).collect()
}

#[test]
fn label_rename_recolour_reorder() {
    let conn = fresh();
    let (board, _) = board_with_cards(&conn, &[]);
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    let docs = create_label(&conn, &board.id, "Docs", "4da3f5").unwrap();
    let ops = create_label(&conn, &board.id, "Ops", "2bb3a8").unwrap();

    assert_eq!(
        update_label(&conn, &bug.id, Some("  Defect "), None).unwrap(),
        board.id
    );
    update_label(&conn, &docs.id, None, Some("3fb950")).unwrap();
    // Renaming to its own name in another case is allowed.
    update_label(&conn, &ops.id, Some("OPS"), None).unwrap();
    assert_eq!(
        label_names(&conn, &board),
        [
            ("Defect".to_string(), "e5534b".to_string(), 0),
            ("Docs".into(), "3fb950".into(), 1),
            ("OPS".into(), "2bb3a8".into(), 2),
        ]
    );
    assert!(update_label(&conn, &bug.id, Some("  "), None).is_err());
    assert_eq!(
        update_label(&conn, &bug.id, None, Some("red")).unwrap_err(),
        "Label colour must be six hex digits"
    );
    assert_eq!(
        update_label(&conn, "gone", Some("X"), None).unwrap_err(),
        "Label not found"
    );

    // Move to the front, then past the end (clamped).
    move_label(&conn, &ops.id, 0).unwrap();
    let order: Vec<String> = list_labels(&conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(order, ["OPS", "Defect", "Docs"]);
    move_label(&conn, &ops.id, 99).unwrap();
    let order: Vec<String> = list_labels(&conn, &board.id)
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(order, ["Defect", "Docs", "OPS"]);
    assert_eq!(move_label(&conn, "gone", 0).unwrap_err(), "Label not found");
}

#[test]
fn label_rename_refuses_a_taken_name_ignoring_case() {
    let conn = fresh();
    let (board, _) = board_with_cards(&conn, &[]);
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    let docs = create_label(&conn, &board.id, "Ärger", "4da3f5").unwrap();
    assert_eq!(
        update_label(&conn, &docs.id, Some("bug"), None).unwrap_err(),
        "A label named bug already exists"
    );
    // Non-ASCII case folding is done in Rust, not by SQLite's lower().
    assert_eq!(
        update_label(&conn, &bug.id, Some("ÄRGER"), None).unwrap_err(),
        "A label named ÄRGER already exists"
    );
    // The same name on another board is fine.
    let (other, _) = board_with_cards(&conn, &[]);
    let there = create_label(&conn, &other.id, "Other", "e5534b").unwrap();
    update_label(&conn, &there.id, Some("Bug"), None).unwrap();
    assert_eq!(list_labels(&conn, &board.id).unwrap()[0].name, "Bug");
}

#[test]
fn delete_label_writes_one_activity_row_per_card() {
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    let docs = create_label(&conn, &board.id, "Docs", "4da3f5").unwrap();
    set_card_labels(&conn, &cards[0].id, &[bug.id.clone(), docs.id.clone()]).unwrap();
    set_card_labels(&conn, &cards[1].id, std::slice::from_ref(&bug.id)).unwrap();
    // An archived card with the label is affected too.
    archive_card(&conn, &cards[1].id).unwrap();
    let saved = create_view(
        &conn,
        &board.id,
        "Bugs",
        &json!({"rows": [
            {"field": "labels", "op": "is", "values": [bug.id, docs.id]},
            {"field": "labels", "op": "is_not", "values": [bug.id]},
        ]})
        .to_string(),
        &json!({"layout": "list", "collapsed": [format!("label:{}", bug.id), "label:none"]})
            .to_string(),
    )
    .unwrap();
    let before = count(&conn, "SELECT COUNT(*) FROM board_activity");

    assert_eq!(delete_label(&conn, &bug.id).unwrap(), board.id);

    let rows: Vec<(Option<String>, String)> = {
        let mut statement = conn
            .prepare(
                "SELECT card_id, detail FROM board_activity WHERE detail LIKE 'label removed:%'
                 ORDER BY card_id",
            )
            .unwrap();
        let rows = statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    };
    let mut expected = vec![
        (Some(cards[0].id.clone()), "label removed: Bug".to_string()),
        (Some(cards[1].id.clone()), "label removed: Bug".to_string()),
    ];
    expected.sort();
    assert_eq!(rows, expected);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM board_activity"),
        before + 2,
        "the card without the label gets no row"
    );
    assert_eq!(
        card_label_ids(&conn, &cards[0]),
        std::slice::from_ref(&docs.id)
    );
    assert!(card_label_ids(&conn, &cards[1]).is_empty());
    assert_eq!(
        label_names(&conn, &board),
        [("Docs".into(), "4da3f5".into(), 0)]
    );
    // Saved-view rows lose the deleted id; a row left empty goes.
    let saved = view(&conn, &saved.id);
    assert_eq!(
        json_of(&saved.spec_json)["rows"],
        json!([{"field": "labels", "op": "is", "values": [docs.id]}])
    );
    assert_eq!(
        json_of(&saved.layout_json)["collapsed"],
        json!(["label:none"])
    );
    assert_eq!(delete_label(&conn, &bug.id).unwrap_err(), "Label not found");
}

#[test]
fn merge_labels_moves_cards_and_rewrites_saved_view_rows() {
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    let defect = create_label(&conn, &board.id, "Defect", "e0823d").unwrap();
    let docs = create_label(&conn, &board.id, "Docs", "4da3f5").unwrap();
    // a: both labels (no duplicate after); b: only the source; c: neither.
    set_card_labels(&conn, &cards[0].id, &[bug.id.clone(), defect.id.clone()]).unwrap();
    set_card_labels(&conn, &cards[1].id, &[bug.id.clone(), docs.id.clone()]).unwrap();
    let saved = create_view(
        &conn,
        &board.id,
        "Bugs",
        &json!({"query": "x", "rows": [
            {"field": "labels", "op": "is", "values": [bug.id, defect.id]},
            {"field": "labels", "op": "is_not", "values": [bug.id, docs.id]},
            {"field": "status", "op": "is", "values": [bug.id]},
        ]})
        .to_string(),
        &json!({"layout": "list", "collapsed": [
            format!("label:{}", bug.id), format!("label:{}", defect.id)
        ]})
        .to_string(),
    )
    .unwrap();

    assert_eq!(merge_labels(&conn, &bug.id, &defect.id).unwrap(), board.id);

    assert_eq!(
        card_label_ids(&conn, &cards[0]),
        std::slice::from_ref(&defect.id)
    );
    let mut b = vec![defect.id.clone(), docs.id.clone()];
    b.sort();
    assert_eq!(card_label_ids(&conn, &cards[1]), b);
    assert!(card_label_ids(&conn, &cards[2]).is_empty());
    assert_eq!(
        label_names(&conn, &board),
        [
            ("Defect".to_string(), "e0823d".to_string(), 0),
            ("Docs".into(), "4da3f5".into(), 1),
        ]
    );
    let saved = view(&conn, &saved.id);
    let spec = json_of(&saved.spec_json);
    assert_eq!(spec["query"], "x");
    assert_eq!(
        spec["rows"],
        json!([
            {"field": "labels", "op": "is", "values": [defect.id]},
            {"field": "labels", "op": "is_not", "values": [defect.id, docs.id]},
            // Only `labels` rows are rewritten.
            {"field": "status", "op": "is", "values": [bug.id]},
        ])
    );
    assert_eq!(
        json_of(&saved.layout_json)["collapsed"],
        json!([format!("label:{}", defect.id)])
    );
    // The rewritten view still filters.
    assert_eq!(
        run(&conn, &board, json!({"rows": [spec["rows"][0]]})),
        ids(&[&cards[0], &cards[1]])
    );

    assert_eq!(
        merge_labels(&conn, &defect.id, &defect.id).unwrap_err(),
        "Pick a different label to merge into"
    );
    let (other, _) = board_with_cards(&conn, &[]);
    let there = create_label(&conn, &other.id, "There", "e5534b").unwrap();
    assert_eq!(
        merge_labels(&conn, &docs.id, &there.id).unwrap_err(),
        "Label not on this board"
    );
    assert_eq!(
        merge_labels(&conn, &bug.id, &defect.id).unwrap_err(),
        "Label not found"
    );
}

#[test]
fn each_field_kind_validates_and_round_trips() {
    assert_eq!(
        FIELD_KINDS,
        [
            "text",
            "number",
            "date",
            "select",
            "multi-select",
            "checkbox",
            "url"
        ]
    );
    let options = vec!["S".to_string(), "M".to_string(), "L".to_string()];
    let ok = |kind: &str, value: Value| validate_value(kind, &options, &value);
    assert_eq!(ok("text", json!("hi")).unwrap(), json!("hi"));
    assert_eq!(
        ok("text", json!("é".repeat(2000))).unwrap(),
        json!("é".repeat(2000))
    );
    assert_eq!(
        ok("text", json!("x".repeat(2001))).unwrap_err(),
        "Text is too long"
    );
    assert!(ok("text", json!(5)).is_err());
    assert_eq!(ok("number", json!(3.5)).unwrap(), json!(3.5));
    assert_eq!(ok("number", json!(-2)).unwrap(), json!(-2));
    assert_eq!(
        ok("number", json!("3")).unwrap_err(),
        "Number must be a number"
    );
    assert_eq!(
        ok("date", json!(1_767_225_600_000_i64)).unwrap(),
        json!(1_767_225_600_000_i64)
    );
    assert_eq!(
        ok("date", json!(1.5)).unwrap_err(),
        "Date must be a timestamp"
    );
    assert_eq!(
        ok("date", json!("2026-01-01")).unwrap_err(),
        "Date must be a timestamp"
    );
    assert_eq!(ok("select", json!("M")).unwrap(), json!("M"));
    assert_eq!(
        ok("select", json!("XL")).unwrap_err(),
        "Pick one of the field's options"
    );
    assert_eq!(
        ok("select", json!(["M"])).unwrap_err(),
        "Pick one of the field's options"
    );
    // Multi-select keeps the field's option order.
    assert_eq!(
        ok("multi-select", json!(["L", "S"])).unwrap(),
        json!(["S", "L"])
    );
    for bad in [json!(["S", "S"]), json!(["XL"]), json!("S"), json!([1])] {
        assert_eq!(
            ok("multi-select", bad).unwrap_err(),
            "Pick one of the field's options"
        );
    }
    assert_eq!(ok("checkbox", json!(true)).unwrap(), json!(true));
    assert_eq!(
        ok("checkbox", json!("true")).unwrap_err(),
        "Checkbox must be true or false"
    );
    assert_eq!(
        ok("url", json!("https://example.com/a")).unwrap(),
        json!("https://example.com/a")
    );
    assert_eq!(ok("url", json!("http://x")).unwrap(), json!("http://x"));
    for bad in ["javascript:alert(1)", "ftp://x", "example.com", "https:/x"] {
        assert_eq!(
            ok("url", json!(bad)).unwrap_err(),
            "URL must start with http:// or https://"
        );
    }
    let long = format!("https://{}", "a".repeat(2000));
    assert_eq!(ok("url", json!(long)).unwrap_err(), "Text is too long");
    assert!(ok("colour", json!("x")).is_err());

    // Round trip through the store for every kind.
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a"]);
    let card = &cards[0];
    let cases: [(&str, Option<&str>, &str, &str); 7] = [
        ("text", None, r#""hello""#, "5"),
        ("number", None, "3.5", r#""3.5""#),
        ("date", None, "1767225600000", "1.5"),
        ("select", Some(r#"["S","M"]"#), r#""M""#, r#""XL""#),
        (
            "multi-select",
            Some(r#"["S","M","L"]"#),
            r#"["L","S"]"#,
            r#"["S","S"]"#,
        ),
        ("checkbox", None, "false", "0"),
        ("url", None, r#""https://x.dev""#, r#""x.dev""#),
    ];
    for (kind, options, good, bad) in cases {
        let field = create_field(&conn, &board.id, kind, kind, options).unwrap();
        set_field_value(&conn, &card.id, &field.id, Some(good)).unwrap();
        let stored = value_of(&conn, card, &field).unwrap();
        let expected = if kind == "multi-select" {
            json!(["S", "L"])
        } else {
            json_of(good)
        };
        assert_eq!(json_of(&stored), expected, "{kind}");
        assert!(
            set_field_value(&conn, &card.id, &field.id, Some(bad)).is_err(),
            "{kind} accepted {bad}"
        );
        assert_eq!(
            value_of(&conn, card, &field).unwrap(),
            stored,
            "{kind} unchanged"
        );
    }
    // An empty multi-select clears the value.
    let tags = list_fields(&conn, &board.id).unwrap()[4].clone();
    set_field_value(&conn, &card.id, &tags.id, Some("[]")).unwrap();
    assert_eq!(value_of(&conn, card, &tags), None);
    // Select options must be a list of distinct names.
    for bad in [r#"["S","S"]"#, r#"[1]"#, r#"{"a":1}"#, r#"["", "x"]"#] {
        assert!(
            create_field(&conn, &board.id, "Bad", "select", Some(bad)).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn renaming_an_option_rewrites_values_and_removing_one_clears_them() {
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let size = create_field(&conn, &board.id, "Size", "select", Some(r#"["S","M","L"]"#)).unwrap();
    let tags = create_field(
        &conn,
        &board.id,
        "Tags",
        "multi-select",
        Some(r#"["ui","api","db"]"#),
    )
    .unwrap();
    set_field_value(&conn, &cards[0].id, &size.id, Some(r#""S""#)).unwrap();
    set_field_value(&conn, &cards[1].id, &size.id, Some(r#""M""#)).unwrap();
    set_field_value(&conn, &cards[0].id, &tags.id, Some(r#"["ui","api"]"#)).unwrap();
    set_field_value(&conn, &cards[1].id, &tags.id, Some(r#"["api"]"#)).unwrap();
    set_field_value(&conn, &cards[2].id, &tags.id, Some(r#"["db"]"#)).unwrap();
    let size_key = format!("field:{}", size.id);
    let tags_key = format!("field:{}", tags.id);
    let saved = create_view(
        &conn,
        &board.id,
        "Small",
        &json!({"rows": [
            {"field": size_key, "op": "is", "values": [" \"S\" ", "\"M\""]},
            {"field": tags_key, "op": "is", "values": ["\"api\""]},
            {"field": "labels", "op": "is", "values": ["\"S\""]},
        ]})
        .to_string(),
        "{}",
    )
    .unwrap();

    assert_eq!(
        rename_field_option(&conn, &size.id, "S", "Small").unwrap(),
        board.id
    );
    rename_field_option(&conn, &tags.id, "api", "backend").unwrap();

    let fields = list_fields(&conn, &board.id).unwrap();
    assert_eq!(
        json_of(fields[0].options_json.as_deref().unwrap()),
        json!(["Small", "M", "L"])
    );
    assert_eq!(
        json_of(fields[1].options_json.as_deref().unwrap()),
        json!(["ui", "backend", "db"])
    );
    assert_eq!(
        value_of(&conn, &cards[0], &size).as_deref(),
        Some(r#""Small""#)
    );
    assert_eq!(value_of(&conn, &cards[1], &size).as_deref(), Some(r#""M""#));
    assert_eq!(
        json_of(&value_of(&conn, &cards[0], &tags).unwrap()),
        json!(["ui", "backend"])
    );
    assert_eq!(
        json_of(&value_of(&conn, &cards[1], &tags).unwrap()),
        json!(["backend"])
    );
    let spec = json_of(&view(&conn, &saved.id).spec_json);
    assert_eq!(spec["rows"][0]["values"], json!(["\"Small\"", "\"M\""]));
    assert_eq!(spec["rows"][1]["values"], json!(["\"backend\""]));
    assert_eq!(
        spec["rows"][2]["values"],
        json!(["\"S\""]),
        "other rows stay"
    );

    assert_eq!(
        rename_field_option(&conn, &size.id, "XL", "Huge").unwrap_err(),
        "Pick one of the field's options"
    );
    assert_eq!(
        rename_field_option(&conn, &size.id, "M", "l").unwrap_err(),
        "An option named l already exists"
    );
    let text = create_field(&conn, &board.id, "Note", "text", None).unwrap();
    assert_eq!(
        rename_field_option(&conn, &text.id, "a", "b").unwrap_err(),
        "Only select fields have options"
    );

    // Removing options clears select values and strips multi-select ones.
    assert_eq!(
        update_field(&conn, &size.id, None, Some(r#"["Small","L"]"#)).unwrap(),
        board.id
    );
    update_field(&conn, &tags.id, Some("Areas"), Some(r#"["ui","db"]"#)).unwrap();
    assert_eq!(
        value_of(&conn, &cards[0], &size).as_deref(),
        Some(r#""Small""#)
    );
    assert_eq!(value_of(&conn, &cards[1], &size), None);
    assert_eq!(
        json_of(&value_of(&conn, &cards[0], &tags).unwrap()),
        json!(["ui"])
    );
    assert_eq!(
        value_of(&conn, &cards[1], &tags),
        None,
        "an emptied list clears"
    );
    assert_eq!(
        json_of(&value_of(&conn, &cards[2], &tags).unwrap()),
        json!(["db"])
    );
    assert_eq!(list_fields(&conn, &board.id).unwrap()[1].name, "Areas");
    assert_eq!(
        update_field(&conn, &text.id, None, Some(r#"["a"]"#)).unwrap_err(),
        "Only select fields have options"
    );
    assert!(update_field(&conn, &size.id, None, Some(r#"["a","a"]"#)).is_err());
    assert_eq!(
        update_field(&conn, "gone", Some("X"), None).unwrap_err(),
        "Field not found"
    );
}

#[test]
fn delete_field_removes_values_view_rows_and_table_columns() {
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a", "b"]);
    let size = create_field(&conn, &board.id, "Size", "select", Some(r#"["S","M"]"#)).unwrap();
    let points = create_field(&conn, &board.id, "Points", "number", None).unwrap();
    set_field_value(&conn, &cards[0].id, &size.id, Some(r#""S""#)).unwrap();
    set_field_value(&conn, &cards[1].id, &size.id, Some(r#""M""#)).unwrap();
    set_field_value(&conn, &cards[0].id, &points.id, Some("3")).unwrap();
    let size_key = format!("field:{}", size.id);
    let points_key = format!("field:{}", points.id);
    let saved = create_view(
        &conn,
        &board.id,
        "Table",
        &json!({"rows": [
            {"field": size_key, "op": "is", "values": ["\"S\""]},
            {"field": points_key, "op": "is", "values": ["3"]},
            {"field": "priority", "op": "is", "values": ["1"]},
        ]})
        .to_string(),
        &json!({
            "layout": "table",
            "columns": ["title", size_key, points_key],
            "widths": {"title": 200, size_key.clone(): 90, points_key.clone(): 80},
            "display": {"face": {"fields": [size.id, points.id]}, "swimlane": size_key},
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(delete_field(&conn, &size.id).unwrap(), board.id);

    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM board_card_field_values"),
        1,
        "only the other field's value is left"
    );
    assert_eq!(value_of(&conn, &cards[0], &points).as_deref(), Some("3"));
    let fields = list_fields(&conn, &board.id).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].position, 0);
    let saved = view(&conn, &saved.id);
    assert_eq!(
        json_of(&saved.spec_json)["rows"],
        json!([
            {"field": points_key, "op": "is", "values": ["3"]},
            {"field": "priority", "op": "is", "values": ["1"]},
        ])
    );
    let layout = json_of(&saved.layout_json);
    assert_eq!(layout["layout"], "table");
    assert_eq!(layout["columns"], json!(["title", points_key]));
    assert_eq!(
        layout["widths"],
        json!({"title": 200, points_key.clone(): 80})
    );
    assert_eq!(layout["display"]["face"]["fields"], json!([points.id]));
    assert_eq!(layout["display"]["swimlane"], "none");
    assert_eq!(
        delete_field(&conn, &size.id).unwrap_err(),
        "Field not found"
    );
}

#[test]
fn multi_select_row_matches_any_contained_value() {
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a", "b", "c", "d"]);
    let tags = create_field(
        &conn,
        &board.id,
        "Tags",
        "multi-select",
        Some(r#"["ui","api","db"]"#),
    )
    .unwrap();
    set_field_value(&conn, &cards[0].id, &tags.id, Some(r#"["ui","api"]"#)).unwrap();
    set_field_value(&conn, &cards[1].id, &tags.id, Some(r#"["api"]"#)).unwrap();
    set_field_value(&conn, &cards[2].id, &tags.id, Some(r#"["db"]"#)).unwrap();
    let key = format!("field:{}", tags.id);
    let row =
        |op: &str, values: &[&str]| json!({"rows": [{"field": key, "op": op, "values": values}]});

    assert_eq!(
        run(&conn, &board, row("is", &["\"api\""])),
        ids(&[&cards[0], &cards[1]])
    );
    assert_eq!(
        run(&conn, &board, row("is", &["\"ui\"", "\"db\""])),
        ids(&[&cards[0], &cards[2]])
    );
    assert_eq!(
        run(&conn, &board, row("is_not", &["\"api\""])),
        ids(&[&cards[2], &cards[3]])
    );
    // A value that is no longer an option is dropped; with none left the row is ignored.
    assert_eq!(
        run(&conn, &board, row("is", &["\"gone\""])),
        ids(&[&cards[0], &cards[1], &cards[2], &cards[3]])
    );
    assert_eq!(
        run(&conn, &board, row("is", &["\"gone\"", "\"db\""])),
        ids(&[&cards[2]])
    );
}

#[test]
fn distinct_values_are_live_cards_only_most_used_first() {
    let conn = fresh();
    let (board, cards) = board_with_cards(&conn, &["a", "b", "c", "d", "e"]);
    let points = create_field(&conn, &board.id, "Points", "number", None).unwrap();
    let tags = create_field(
        &conn,
        &board.id,
        "Tags",
        "multi-select",
        Some(r#"["ui","api","db"]"#),
    )
    .unwrap();
    for (card, value) in [(0, "5"), (1, "10"), (2, "5"), (3, "9"), (4, "10")] {
        set_field_value(&conn, &cards[card].id, &points.id, Some(value)).unwrap();
    }
    set_field_value(&conn, &cards[0].id, &tags.id, Some(r#"["ui","api"]"#)).unwrap();
    set_field_value(&conn, &cards[1].id, &tags.id, Some(r#"["api"]"#)).unwrap();
    set_field_value(&conn, &cards[2].id, &tags.id, Some(r#"["db"]"#)).unwrap();
    set_field_value(&conn, &cards[4].id, &tags.id, Some(r#"["db","ui"]"#)).unwrap();
    // e is archived: its values don't count.
    archive_card(&conn, &cards[4].id).unwrap();

    // 5 twice, then 9 and 10 once each, by value (numerically).
    assert_eq!(
        field_distinct_values(&conn, &board.id, &points.id, 10).unwrap(),
        ["5", "9", "10"]
    );
    assert_eq!(
        field_distinct_values(&conn, &board.id, &points.id, 1).unwrap(),
        ["5"]
    );
    // Multi-select arrays are flattened.
    assert_eq!(
        field_distinct_values(&conn, &board.id, &tags.id, 10).unwrap(),
        [r#""api""#, r#""db""#, r#""ui""#]
    );
    // The limit is capped at 50.
    let note = create_field(&conn, &board.id, "Note", "text", None).unwrap();
    let column = get_board(&conn, &board.id).unwrap().columns[0].id.clone();
    for n in 0..60 {
        let card = create_card(&conn, &column, &format!("n{n}"), "").unwrap();
        set_field_value(
            &conn,
            &card.id,
            &note.id,
            Some(&json!(format!("v{n:02}")).to_string()),
        )
        .unwrap();
    }
    let many = field_distinct_values(&conn, &board.id, &note.id, 500).unwrap();
    assert_eq!(many.len(), 50);
    assert_eq!(many[0], r#""v00""#);
    assert!(field_distinct_values(&conn, &board.id, &note.id, 0)
        .unwrap()
        .is_empty());
    let (other, _) = board_with_cards(&conn, &[]);
    assert_eq!(
        field_distinct_values(&conn, &other.id, &note.id, 10).unwrap_err(),
        "Field not on this board"
    );
}

#[test]
fn field_and_label_positions_stay_contiguous() {
    let conn = fresh();
    let (board, _) = board_with_cards(&conn, &[]);
    let labels: Vec<Label> = ["A", "B", "C", "D"]
        .iter()
        .map(|name| create_label(&conn, &board.id, name, "8a867e").unwrap())
        .collect();
    let fields: Vec<Field> = ["W", "X", "Y", "Z"]
        .iter()
        .map(|name| create_field(&conn, &board.id, name, "text", None).unwrap())
        .collect();
    let label_order = |conn: &Connection| -> Vec<(String, i64)> {
        list_labels(conn, &board.id)
            .unwrap()
            .into_iter()
            .map(|l| (l.name, l.position))
            .collect()
    };
    let field_order = |conn: &Connection| -> Vec<(String, i64)> {
        list_fields(conn, &board.id)
            .unwrap()
            .into_iter()
            .map(|f| (f.name, f.position))
            .collect()
    };
    let pairs = |names: &[&str]| -> Vec<(String, i64)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i as i64))
            .collect()
    };

    delete_label(&conn, &labels[1].id).unwrap();
    assert_eq!(label_order(&conn), pairs(&["A", "C", "D"]));
    move_label(&conn, &labels[3].id, 0).unwrap();
    assert_eq!(label_order(&conn), pairs(&["D", "A", "C"]));
    merge_labels(&conn, &labels[0].id, &labels[2].id).unwrap();
    assert_eq!(label_order(&conn), pairs(&["D", "C"]));
    assert_eq!(
        create_label(&conn, &board.id, "E", "8a867e")
            .unwrap()
            .position,
        2
    );

    delete_field(&conn, &fields[0].id).unwrap();
    assert_eq!(field_order(&conn), pairs(&["X", "Y", "Z"]));
    assert_eq!(move_field(&conn, &fields[3].id, 1).unwrap(), board.id);
    assert_eq!(field_order(&conn), pairs(&["X", "Z", "Y"]));
    move_field(&conn, &fields[1].id, 99).unwrap();
    assert_eq!(field_order(&conn), pairs(&["Z", "Y", "X"]));
    assert_eq!(
        create_field(&conn, &board.id, "V", "text", None)
            .unwrap()
            .position,
        3
    );
    assert_eq!(move_field(&conn, "gone", 0).unwrap_err(), "Field not found");
}
