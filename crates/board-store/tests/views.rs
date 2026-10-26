use gespan_board_store::*;
use rusqlite::{params, Connection};
use std::collections::HashSet;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
/// 2026-10-26 00:00 UTC.
const MIDNIGHT_UTC: i64 = 1_793_404_800_000;

fn fresh() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// A board with its default columns (Backlog, In Progress, Review, Done) and
/// the given cards in Backlog, in order.
fn board_with_cards(conn: &Connection, titles: &[&str]) -> (Board, Vec<Column>, Vec<Card>) {
    let board = create_board(conn, "Work").unwrap();
    let columns = get_board(conn, &board.id).unwrap().columns;
    let cards = titles
        .iter()
        .map(|title| create_card(conn, &columns[0].id, title, "").unwrap())
        .collect();
    (board, columns, cards)
}

fn spec(query: &str, rows: Vec<FilterRow>) -> ViewSpec {
    ViewSpec {
        query: query.to_string(),
        rows,
        sort: SortSpec::default(),
    }
}

fn row(field: &str, op: Op, values: &[&str]) -> FilterRow {
    FilterRow {
        field: field.to_string(),
        op,
        values: values.iter().map(|v| v.to_string()).collect(),
    }
}

fn run(conn: &Connection, board: &Board, spec: &ViewSpec) -> Vec<String> {
    run_at(conn, board, spec, MIDNIGHT_UTC, &fixed(0), &HashSet::new())
}

fn run_at(
    conn: &Connection,
    board: &Board,
    spec: &ViewSpec,
    now_ms: i64,
    local_day: &dyn Fn(i64) -> i64,
    running: &HashSet<String>,
) -> Vec<String> {
    query_cards(conn, &board.id, spec, now_ms, local_day, running).unwrap()
}

/// A local-day function for a zone that never changes its UTC offset.
fn fixed(offset_minutes: i64) -> impl Fn(i64) -> i64 {
    move |ms| (ms + offset_minutes * 60_000).div_euclid(DAY)
}

fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

fn ids(cards: &[&Card]) -> Vec<String> {
    cards.iter().map(|card| card.id.clone()).collect()
}

fn set_times(conn: &Connection, card: &Card, created_at: i64, updated_at: i64) {
    conn.execute(
        "UPDATE board_cards SET created_at = ?1, updated_at = ?2 WHERE id = ?3",
        params![created_at, updated_at, card.id],
    )
    .unwrap();
}

#[test]
fn search_matches_title_description_and_exact_number() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["Fix LOGIN page", "Write docs"]);
    let described = create_card(&conn, &columns[1].id, "Plain", "The login token expires").unwrap();
    let mut more = Vec::new();
    for title in ["c4", "c5", "c6", "c7", "c8", "c9", "c10", "c11", "c12"] {
        more.push(create_card(&conn, &columns[0].id, title, "").unwrap());
    }
    // #1 is card number 1 only, not 10, 11 or 12.
    assert_eq!(run(&conn, &board, &spec("#1", vec![])), ids(&[&cards[0]]));
    assert_eq!(
        run(&conn, &board, &spec("  #12 ", vec![])),
        ids(&[&more[8]])
    );
    // Case-insensitive over the title and the description, in manual order.
    assert_eq!(
        run(&conn, &board, &spec("login", vec![])),
        ids(&[&cards[0], &described])
    );
    assert_eq!(
        run(&conn, &board, &spec("TOKEN", vec![])),
        ids(&[&described])
    );
    assert!(run(&conn, &board, &spec("nothing like it", vec![])).is_empty());
    // An empty query shows every live card; archived cards never show.
    archive_card(&conn, &cards[1].id).unwrap();
    let all = run(&conn, &board, &spec("", vec![]));
    assert_eq!(all.len(), 11);
    assert!(!all.contains(&cards[1].id));
}

#[test]
fn is_and_is_not_rows_combine_with_and() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c", "d"]);
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    let ui = create_label(&conn, &board.id, "UI", "4da3f5").unwrap();
    set_card_labels(&conn, &cards[0].id, std::slice::from_ref(&bug.id)).unwrap();
    set_card_labels(&conn, &cards[1].id, &[bug.id.clone(), ui.id.clone()]).unwrap();
    set_card_labels(&conn, &cards[2].id, std::slice::from_ref(&ui.id)).unwrap();
    set_priority(&conn, &cards[0].id, 3).unwrap();
    set_priority(&conn, &cards[1].id, 4).unwrap();
    set_priority(&conn, &cards[2].id, 3).unwrap();
    move_card(&conn, &cards[3].id, &columns[1].id, 0, "user").unwrap();
    set_projects(&conn, &cards[2].id, &["/a".into()]).unwrap();

    // Values in a row OR together.
    let high_or_urgent = row("priority", Op::Is, &["3", "4"]);
    assert_eq!(
        run(&conn, &board, &spec("", vec![high_or_urgent.clone()])),
        ids(&[&cards[0], &cards[1], &cards[2]])
    );
    // Rows AND together; is_not of a multi-valued field means "has none of".
    let no_ui = row("labels", Op::IsNot, &[&ui.id]);
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![high_or_urgent.clone(), no_ui.clone()])
        ),
        ids(&[&cards[0]])
    );
    assert_eq!(
        run(&conn, &board, &spec("", vec![no_ui])),
        ids(&[&cards[0], &cards[3]])
    );
    // Status by column id, and projects by path.
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row("status", Op::IsNot, &[&columns[0].id])])
        ),
        ids(&[&cards[3]])
    );
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row("projects", Op::Is, &["/a"])])
        ),
        ids(&[&cards[2]])
    );
    // A card with no priority row is priority 0.
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row("priority", Op::Is, &["0"])])
        ),
        ids(&[&cards[3]])
    );
    // Search and rows AND together too.
    assert_eq!(
        run(&conn, &board, &spec("c", vec![high_or_urgent])),
        ids(&[&cards[2]])
    );
}

#[test]
fn due_buckets_use_local_days_across_midnight() {
    let conn = fresh();
    let titles = [
        "late",
        "today",
        "tomorrow",
        "week",
        "later",
        "none",
        "done-late",
    ];
    let (board, columns, cards) = board_with_cards(&conn, &titles);
    // Local time is UTC+05:30. "now" is 23:30 local on Oct 26 (18:00 UTC).
    let offset = 330;
    let now = MIDNIGHT_UTC + 18 * HOUR;
    let local_midnight = MIDNIGHT_UTC - 330 * 60_000; // Oct 26 00:00 local
    set_due(&conn, &cards[0].id, Some(local_midnight - 1)).unwrap(); // Oct 25 23:59:59.999
    set_due(&conn, &cards[1].id, Some(local_midnight)).unwrap();
    // 00:10 local on Oct 27 is still Oct 26 in UTC, but tomorrow locally.
    set_due(
        &conn,
        &cards[2].id,
        Some(local_midnight + DAY + 10 * 60_000),
    )
    .unwrap();
    set_due(&conn, &cards[3].id, Some(local_midnight + 8 * DAY - 1)).unwrap(); // end of day +7
    set_due(&conn, &cards[4].id, Some(local_midnight + 8 * DAY)).unwrap(); // day +8
    set_due(&conn, &cards[6].id, Some(local_midnight - 3 * DAY)).unwrap();
    move_card(&conn, &cards[6].id, &columns[3].id, 0, "user").unwrap(); // Done
    let running = HashSet::new();
    let bucket = |value: &str, now: i64, offset: i64| {
        run_at(
            &conn,
            &board,
            &spec("", vec![row("due", Op::Is, &[value])]),
            now,
            &fixed(offset),
            &running,
        )
    };

    assert_eq!(bucket("overdue", now, offset), ids(&[&cards[0]]));
    assert_eq!(bucket("today", now, offset), ids(&[&cards[1]]));
    assert_eq!(bucket("next7", now, offset), ids(&[&cards[2], &cards[3]]));
    assert_eq!(bucket("later", now, offset), ids(&[&cards[4]]));
    assert_eq!(bucket("none", now, offset), ids(&[&cards[5]]));

    // Read in UTC at the same instant, the 00:10 card is due today instead.
    assert!(bucket("today", now, 0).contains(&cards[2].id));
    // Thirty one minutes later it is Oct 27 locally: the old "today" card is
    // now overdue and the 00:10 card is due today.
    let after = now + 31 * 60_000;
    assert_eq!(
        bucket("overdue", after, offset),
        ids(&[&cards[0], &cards[1]])
    );
    assert_eq!(bucket("today", after, offset), ids(&[&cards[2]]));
    // A negative offset (UTC-07:00, as after a DST change) moves the day
    // back: at 20:00 on Oct 25 there, the first two cards both fall on
    // Oct 25, and the "end of day +7" card lands on day +8.
    let pacific = MIDNIGHT_UTC + 3 * HOUR;
    assert_eq!(bucket("today", pacific, -420), ids(&[&cards[0], &cards[1]]));
    assert_eq!(bucket("next7", pacific, -420), ids(&[&cards[2]]));
    assert_eq!(bucket("later", pacific, -420), ids(&[&cards[3], &cards[4]]));
}

/// US/Pacific in 2026: PST (UTC-8) until 10:00 UTC on Mar 8, PDT (UTC-7) after.
fn pacific_2026(ms: i64) -> i64 {
    let offset = if ms >= utc(2026, 3, 8, 10, 0) {
        -420
    } else {
        -480
    };
    (ms + offset * 60_000).div_euclid(DAY)
}

#[test]
fn due_buckets_use_each_dates_own_offset_across_dst() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["mar 9", "mar 8", "mar 2"]);
    // Due dates are stored at local midnight under their own offset.
    set_due(&conn, &cards[0].id, Some(utc(2026, 3, 9, 7, 0))).unwrap(); // Mar 9 00:00 PDT
    set_due(&conn, &cards[1].id, Some(utc(2026, 3, 8, 8, 0))).unwrap(); // Mar 8 00:00 PST
    set_due(&conn, &cards[2].id, Some(utc(2026, 3, 2, 8, 0))).unwrap(); // Mar 2 00:00 PST
    let running = HashSet::new();
    let bucket = |value: &str, now: i64, local_day: &dyn Fn(i64) -> i64| {
        run_at(
            &conn,
            &board,
            &spec("", vec![row("due", Op::Is, &[value])]),
            now,
            local_day,
            &running,
        )
    };

    // Viewing on Mar 1 (PST): Mar 9 is eight days out, so later. Reading it
    // with Mar 1's offset would call it Mar 8 and put it in next7.
    let mar_1_noon = utc(2026, 3, 1, 20, 0);
    assert_eq!(
        bucket("later", mar_1_noon, &pacific_2026),
        ids(&[&cards[0]])
    );
    assert_eq!(
        bucket("next7", mar_1_noon, &pacific_2026),
        ids(&[&cards[1], &cards[2]])
    );
    assert_eq!(
        bucket("next7", mar_1_noon, &fixed(-480)),
        ids(&cards.iter().collect::<Vec<_>>())
    );

    // 01:00 PST on Mar 8, the day of the change: the card due Mar 9 is
    // tomorrow (next7), not today, and the Mar 8 card is today.
    let mar_8_early = utc(2026, 3, 8, 9, 0);
    assert_eq!(
        bucket("today", mar_8_early, &pacific_2026),
        ids(&[&cards[1]])
    );
    assert_eq!(
        bucket("next7", mar_8_early, &pacific_2026),
        ids(&[&cards[0]])
    );
    assert_eq!(
        bucket("overdue", mar_8_early, &pacific_2026),
        ids(&[&cards[2]])
    );
    assert_eq!(
        bucket("today", mar_8_early, &fixed(-480)),
        ids(&[&cards[0], &cards[1]])
    );
    // Later the same day, after the change, nothing moves.
    let mar_8_late = utc(2026, 3, 9, 6, 30); // 23:30 PDT
    assert_eq!(
        bucket("today", mar_8_late, &pacific_2026),
        ids(&[&cards[1]])
    );
    assert_eq!(
        bucket("next7", mar_8_late, &pacific_2026),
        ids(&[&cards[0]])
    );
}

#[test]
fn local_day_chrono_counts_local_calendar_days() {
    use chrono::{Local, NaiveDate, TimeZone};
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    // Noon and the first minute of each day around both 2026 US changes fall
    // on that local date, whatever zone the machine is in.
    for (month, day) in [(3, 7), (3, 8), (3, 9), (10, 31), (11, 1), (11, 2)] {
        let date = NaiveDate::from_ymd_opt(2026, month, day).unwrap();
        let expected = (date - epoch).num_days();
        for (hour, minute) in [(0, 0), (12, 0), (23, 59)] {
            let local = date.and_hms_opt(hour, minute, 0).unwrap();
            let ms = Local
                .from_local_datetime(&local)
                .earliest()
                .unwrap()
                .timestamp_millis();
            assert_eq!(local_day_chrono(ms), expected, "{local}");
        }
    }
    // Before the epoch too.
    let old = NaiveDate::from_ymd_opt(1969, 12, 31)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();
    let ms = Local
        .from_local_datetime(&old)
        .earliest()
        .unwrap()
        .timestamp_millis();
    assert_eq!(local_day_chrono(ms), -1);
}

#[test]
fn failed_state_follows_a_card_moved_from_another_board() {
    let conn = fresh();
    let (first, _, cards) = board_with_cards(&conn, &["fails"]);
    add_link(&conn, &cards[0].id, "session", "s-1", None).unwrap();
    record_card(&conn, &cards[0].id, "failed", None).unwrap();
    let second = create_board(&conn, "Second").unwrap();
    let moved = move_card_to_board(&conn, &cards[0].id, &second.id).unwrap();
    let failed = spec("", vec![row("linked", Op::Is, &["failed"])]);
    assert_eq!(run(&conn, &second, &failed), [moved.id.as_str()]);
    assert!(run(&conn, &first, &failed).is_empty());
    // A run on the new board settles it there.
    record_card(&conn, &moved.id, "completed", None).unwrap();
    assert!(run(&conn, &second, &failed).is_empty());
}

#[test]
fn created_within_24h_and_7d() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["new", "edge", "week", "old"]);
    let now = MIDNIGHT_UTC + 12 * HOUR;
    set_times(&conn, &cards[0], now - HOUR, now - 40 * DAY);
    set_times(&conn, &cards[1], now - DAY, now - 40 * DAY); // exactly 24h ago
    set_times(&conn, &cards[2], now - DAY - 1, now - HOUR);
    set_times(&conn, &cards[3], now - 31 * DAY, now - 8 * DAY);
    let running = HashSet::new();
    let within = |field: &str, value: &str| {
        run_at(
            &conn,
            &board,
            &spec("", vec![row(field, Op::Is, &[value])]),
            now,
            &fixed(330),
            &running,
        )
    };
    assert_eq!(within("created", "24h"), ids(&[&cards[0], &cards[1]]));
    assert_eq!(
        within("created", "7d"),
        ids(&[&cards[0], &cards[1], &cards[2]])
    );
    assert_eq!(
        within("created", "30d"),
        ids(&[&cards[0], &cards[1], &cards[2]])
    );
    assert_eq!(within("updated", "24h"), ids(&[&cards[2]]));
    assert_eq!(within("updated", "30d"), ids(&[&cards[2], &cards[3]]));
    assert_eq!(
        run_at(
            &conn,
            &board,
            &spec("", vec![row("created", Op::IsNot, &["7d"])]),
            now,
            &fixed(0),
            &running
        ),
        ids(&[&cards[3]])
    );
}

#[test]
fn linked_running_comes_from_the_caller_and_failed_from_activity() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["run", "fail", "recovered", "unlinked"]);
    for card in &cards[..3] {
        add_link(
            &conn,
            &card.id,
            "session",
            &format!("s-{}", card.number),
            None,
        )
        .unwrap();
    }
    record_card(&conn, &cards[1].id, "started", None).unwrap();
    record_card(&conn, &cards[1].id, "failed", None).unwrap();
    record_card(&conn, &cards[2].id, "failed", None).unwrap();
    record_card(&conn, &cards[2].id, "started", None).unwrap();
    record_card(&conn, &cards[2].id, "completed", None).unwrap();
    // Other activity after the failure doesn't hide it.
    set_priority(&conn, &cards[1].id, 2).unwrap();

    let running: HashSet<String> = [cards[0].id.clone()].into();
    let linked = |value: &str| {
        run_at(
            &conn,
            &board,
            &spec("", vec![row("linked", Op::Is, &[value])]),
            MIDNIGHT_UTC,
            &fixed(0),
            &running,
        )
    };
    assert_eq!(linked("running"), ids(&[&cards[0]]));
    assert_eq!(linked("failed"), ids(&[&cards[1]]));
    assert_eq!(linked("none"), ids(&[&cards[3]]));
    assert!(run(
        &conn,
        &board,
        &spec("", vec![row("linked", Op::Is, &["running"])])
    )
    .is_empty());
}

#[test]
fn needs_review_is_the_review_column() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b"]);
    let review = columns
        .iter()
        .find(|c| c.role.as_deref() == Some("review"))
        .unwrap();
    move_card(&conn, &cards[1].id, &review.id, 0, "user").unwrap();
    let needs_review = spec("", vec![row("linked", Op::Is, &["needs_review"])]);
    assert_eq!(run(&conn, &board, &needs_review), ids(&[&cards[1]]));
    // Taking the role off the column takes the card out.
    update_column(&conn, &review.id, &review.name, None).unwrap();
    assert!(run(&conn, &board, &needs_review).is_empty());
}

#[test]
fn custom_field_rows_match_encoded_values() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let size = create_field(&conn, &board.id, "Size", "select", Some(r#"["S","M","L"]"#)).unwrap();
    let points = create_field(&conn, &board.id, "Points", "number", None).unwrap();
    set_field_value(&conn, &cards[0].id, &size.id, Some(r#""S""#)).unwrap();
    set_field_value(&conn, &cards[1].id, &size.id, Some(r#""L""#)).unwrap();
    set_field_value(&conn, &cards[0].id, &points.id, Some("3")).unwrap();
    set_field_value(&conn, &cards[2].id, &points.id, Some("5.0")).unwrap();

    let size_key = format!("field:{}", size.id);
    let points_key = format!("field:{}", points.id);
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row(&size_key, Op::Is, &[r#""S""#, r#""M""#])])
        ),
        ids(&[&cards[0]])
    );
    // A card with no value is never "is" and always "is not".
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row(&size_key, Op::IsNot, &[r#""S""#])])
        ),
        ids(&[&cards[1], &cards[2]])
    );
    // Values compare as JSON, not as text: 5 matches a stored 5.0.
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row(&points_key, Op::Is, &["5"])])
        ),
        ids(&[&cards[2]])
    );
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row(&points_key, Op::Is, &[" 3 "])])
        ),
        ids(&[&cards[0]])
    );
}

#[test]
fn missing_label_or_column_values_are_dropped_and_empty_rows_ignored() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    set_card_labels(&conn, &cards[0].id, std::slice::from_ref(&bug.id)).unwrap();
    let size = create_field(&conn, &board.id, "Size", "select", Some(r#"["S","M"]"#)).unwrap();
    set_field_value(&conn, &cards[1].id, &size.id, Some(r#""S""#)).unwrap();
    set_projects(&conn, &cards[2].id, &["/a".into()]).unwrap();
    let all = ids(&[&cards[0], &cards[1], &cards[2]]);

    // A deleted label among live ones: the live one still filters.
    let mixed = spec("", vec![row("labels", Op::Is, &["gone-label", &bug.id])]);
    assert_eq!(run(&conn, &board, &mixed), ids(&[&cards[0]]));
    // Only missing values: the row is ignored, for is and is_not alike.
    for op in [Op::Is, Op::IsNot] {
        for field in [
            "labels", "status", "projects", "priority", "due", "linked", "created",
        ] {
            let only_gone = spec("", vec![row(field, op, &["gone"])]);
            assert_eq!(run(&conn, &board, &only_gone), all, "{field} {op:?}");
        }
        // A deleted field, a removed select option, bad JSON and an unknown field.
        for (field, value) in [
            ("field:gone-field".to_string(), r#""S""#),
            (format!("field:{}", size.id), r#""XL""#),
            (format!("field:{}", size.id), "not json"),
            ("colour".to_string(), "red"),
        ] {
            let ignored = spec("", vec![row(&field, op, &[value])]);
            assert_eq!(run(&conn, &board, &ignored), all, "{field} {value}");
        }
        assert_eq!(
            run(&conn, &board, &spec("", vec![row("labels", op, &[])])),
            all
        );
    }
    // A row naming a column that was deleted after the view was saved.
    let extra = create_column(&conn, &board.id, "Later", None).unwrap();
    let saved = spec("", vec![row("status", Op::Is, &[&extra.id])]);
    assert!(run(&conn, &board, &saved).is_empty());
    delete_column(&conn, &extra.id).unwrap();
    assert_eq!(run(&conn, &board, &saved), all);
    // Another board's column is not this board's.
    let other = create_board(&conn, "Other").unwrap();
    let other_column = &get_board(&conn, &other.id).unwrap().columns[0];
    let foreign = spec(
        "",
        vec![row("status", Op::Is, &[&other_column.id, &columns[0].id])],
    );
    assert_eq!(run(&conn, &board, &foreign), all);
    // A project no card has any more is dropped too.
    set_projects(&conn, &cards[2].id, &[]).unwrap();
    assert_eq!(
        run(
            &conn,
            &board,
            &spec("", vec![row("projects", Op::Is, &["/a"])])
        ),
        all
    );
}

#[test]
fn sort_missing_values_last_in_both_directions_ties_keep_manual_order() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["b", "C", "a", "d", "e"]);
    // Manual order: Backlog [b, C, a, d], then In Progress [e].
    move_card(&conn, &cards[4].id, &columns[1].id, 0, "user").unwrap();
    set_due(&conn, &cards[0].id, Some(MIDNIGHT_UTC + 2 * DAY)).unwrap();
    set_due(&conn, &cards[2].id, Some(MIDNIGHT_UTC + DAY)).unwrap();
    set_due(&conn, &cards[4].id, Some(MIDNIGHT_UTC + DAY)).unwrap();
    set_priority(&conn, &cards[1].id, 2).unwrap();
    set_priority(&conn, &cards[3].id, 4).unwrap();
    set_priority(&conn, &cards[4].id, 2).unwrap();
    set_start(&conn, &cards[3].id, Some(MIDNIGHT_UTC)).unwrap();

    let sorted = |key: SortKey, dir: Dir| {
        let spec = ViewSpec {
            query: String::new(),
            rows: vec![],
            sort: SortSpec { key, dir },
        };
        run(&conn, &board, &spec)
    };
    let [b, c, a, d, e] = [0, 1, 2, 3, 4].map(|i| cards[i].id.as_str());
    assert_eq!(sorted(SortKey::Manual, Dir::Asc), [b, c, a, d, e]);
    // Ties (a and e both due tomorrow) keep manual order in both directions;
    // cards with no due date stay last in both directions.
    assert_eq!(sorted(SortKey::Due, Dir::Asc), [a, e, b, c, d]);
    assert_eq!(sorted(SortKey::Due, Dir::Desc), [b, a, e, c, d]);
    // Priority 0 (None) counts as missing.
    assert_eq!(sorted(SortKey::Priority, Dir::Desc), [d, c, e, b, a]);
    assert_eq!(sorted(SortKey::Priority, Dir::Asc), [c, e, d, b, a]);
    assert_eq!(sorted(SortKey::Start, Dir::Desc), [d, b, c, a, e]);
    // Titles ignore case.
    assert_eq!(sorted(SortKey::Title, Dir::Asc), [a, b, c, d, e]);
    assert_eq!(sorted(SortKey::Title, Dir::Desc), [e, d, c, b, a]);
    set_times(&conn, &cards[0], 30, 5);
    set_times(&conn, &cards[1], 10, 5);
    set_times(&conn, &cards[2], 20, 9);
    set_times(&conn, &cards[3], 10, 1);
    set_times(&conn, &cards[4], 40, 9);
    assert_eq!(sorted(SortKey::Created, Dir::Asc), [c, d, a, b, e]);
    assert_eq!(sorted(SortKey::Updated, Dir::Desc), [a, e, b, c, d]);
}

#[test]
fn view_spec_json_round_trip_is_stable() {
    let spec = ViewSpec {
        query: "login".into(),
        rows: vec![
            row("labels", Op::IsNot, &["l1"]),
            row("due", Op::Is, &["overdue", "today"]),
        ],
        sort: SortSpec {
            key: SortKey::Due,
            dir: Dir::Desc,
        },
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(
        json,
        r#"{"query":"login","rows":[{"field":"labels","op":"is_not","values":["l1"]},{"field":"due","op":"is","values":["overdue","today"]}],"sort":{"key":"due","dir":"desc"}}"#
    );
    let back: ViewSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(back, spec);
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    // Missing parts take the defaults: no query, no rows, manual ascending.
    let empty: ViewSpec = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, ViewSpec::default());
    assert_eq!(
        empty.sort,
        SortSpec {
            key: SortKey::Manual,
            dir: Dir::Asc
        }
    );
}

#[test]
fn views_crud_and_reorder() {
    let conn = fresh();
    let (board, _, _) = board_with_cards(&conn, &[]);
    let spec_json = r#"{"query":"","rows":[],"sort":{"key":"priority","dir":"desc"}}"#;
    let first = create_view(
        &conn,
        &board.id,
        " Mine ",
        spec_json,
        r#"{"layout":"board"}"#,
    )
    .unwrap();
    assert_eq!((first.name.as_str(), first.position), ("Mine", 0));
    assert_eq!(first.board_id, board.id);
    let second = create_view(&conn, &board.id, "Urgent", "{}", "{}").unwrap();
    let third = create_view(&conn, &board.id, "Table", "{}", r#"{"layout":"table"}"#).unwrap();
    let names = |conn: &Connection| -> Vec<(String, i64)> {
        list_views(conn, &board.id)
            .unwrap()
            .into_iter()
            .map(|v| (v.name, v.position))
            .collect()
    };
    let pairs = |list: &[(&str, i64)]| -> Vec<(String, i64)> {
        list.iter().map(|(n, p)| (n.to_string(), *p)).collect()
    };
    assert_eq!(
        names(&conn),
        pairs(&[("Mine", 0), ("Urgent", 1), ("Table", 2)])
    );

    // Bad input is refused and changes nothing.
    assert!(create_view(&conn, &board.id, "  ", "{}", "{}").is_err());
    assert!(create_view(&conn, &board.id, "X", "not json", "{}").is_err());
    assert!(create_view(&conn, &board.id, "X", r#"{"sort":{"key":"colour"}}"#, "{}").is_err());
    assert!(create_view(&conn, &board.id, "X", "{}", "nope").is_err());
    assert!(create_view(&conn, "no-board", "X", "{}", "{}").is_err());
    assert!(update_view(&conn, &second.id, None, Some("{bad"), None).is_err());
    assert_eq!(names(&conn).len(), 3);

    let returned = update_view(
        &conn,
        &second.id,
        Some("Hot"),
        None,
        Some(r#"{"layout":"list"}"#),
    )
    .unwrap();
    assert_eq!(returned, board.id);
    let hot = list_views(&conn, &board.id).unwrap().remove(1);
    assert_eq!((hot.name.as_str(), hot.spec_json.as_str()), ("Hot", "{}"));
    assert_eq!(hot.layout_json, r#"{"layout":"list"}"#);
    assert!(hot.updated_at >= hot.created_at);

    move_view(&conn, &third.id, 0).unwrap();
    assert_eq!(
        names(&conn),
        pairs(&[("Table", 0), ("Mine", 1), ("Hot", 2)])
    );
    move_view(&conn, &third.id, 99).unwrap();
    assert_eq!(
        names(&conn),
        pairs(&[("Mine", 0), ("Hot", 1), ("Table", 2)])
    );

    delete_view(&conn, &first.id).unwrap();
    assert_eq!(names(&conn), pairs(&[("Hot", 0), ("Table", 1)]));
    assert!(delete_view(&conn, &first.id).is_err());
    assert!(update_view(&conn, &first.id, Some("x"), None, None).is_err());

    // Views belong to their board and go with it.
    delete_board(&conn, &board.id).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_views"), 0);
}

#[test]
fn v4_migration_is_idempotent_and_keeps_v3_data() {
    let path = tempfile::NamedTempFile::new().unwrap();
    let conn = Connection::open(path.path()).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    let (board, _, cards) = board_with_cards(&conn, &["keep"]);
    let label = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    set_card_labels(&conn, &cards[0].id, std::slice::from_ref(&label.id)).unwrap();
    set_priority(&conn, &cards[0].id, 3).unwrap();
    // Turn it back into a v3 database: no views table, version 3, and an old
    // native label row whose copy was since removed on purpose.
    conn.execute_batch(
        "DROP TABLE board_views;
         UPDATE board_schema SET version = 3;
         CREATE TABLE board_native_labels (id TEXT PRIMARY KEY, board_id TEXT NOT NULL, name TEXT NOT NULL, color TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO board_native_labels VALUES ('l-old', ?1, 'Old', '000000')",
        [&board.id],
    )
    .unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 3);

    ensure_tables(&conn).unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 4);
    assert_eq!(SCHEMA_VERSION, 4);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_schema"), 1);
    // v3 data is untouched, and the v2 → v3 copy did not run again.
    assert_eq!(card_meta(&conn, &cards[0].id).unwrap().priority, 3);
    assert_eq!(
        card_extras(&conn, &board.id).unwrap()[&cards[0].id].labels,
        [label.id.as_str()]
    );
    assert_eq!(list_labels(&conn, &board.id).unwrap().len(), 1);
    let view = create_view(&conn, &board.id, "Mine", "{}", "{}").unwrap();

    ensure_tables(&conn).unwrap();
    migrate::run(&conn).unwrap();
    assert_eq!(migrate::version(&conn).unwrap(), 4);
    assert_eq!(list_views(&conn, &board.id).unwrap(), [view]);

    // A second connection opening the same file finds the work done.
    let other = Connection::open(path.path()).unwrap();
    ensure_tables(&other).unwrap();
    assert_eq!(count(&other, "SELECT COUNT(*) FROM board_views"), 1);
}

#[test]
fn set_priority_many_and_set_label_many_write_one_row_per_changed_card() {
    let conn = fresh();
    let (board, _, cards) = board_with_cards(&conn, &["a", "b", "c"]);
    let bug = create_label(&conn, &board.id, "bug", "e5534b").unwrap();
    let docs = create_label(&conn, &board.id, "Docs", "4da3f5").unwrap();
    set_priority(&conn, &cards[1].id, 3).unwrap();
    set_card_labels(&conn, &cards[0].id, std::slice::from_ref(&docs.id)).unwrap();
    set_card_labels(&conn, &cards[1].id, std::slice::from_ref(&bug.id)).unwrap();
    let rows = |card: &Card| list_activity(&conn, &board.id, Some(&card.id), 500).unwrap();
    let before: Vec<usize> = cards.iter().map(|c| rows(c).len()).collect();
    let all: Vec<String> = cards.iter().map(|c| c.id.clone()).collect();

    // b is already High: only a and c change. A repeated id counts once.
    let mut with_repeat = all.clone();
    with_repeat.push(cards[0].id.clone());
    assert_eq!(set_priority_many(&conn, &with_repeat, 3).unwrap(), board.id);
    for card in &cards {
        assert_eq!(card_meta(&conn, &card.id).unwrap().priority, 3);
    }
    let after: Vec<usize> = cards.iter().map(|c| rows(c).len()).collect();
    assert_eq!(after, [before[0] + 1, before[1], before[2] + 1]);
    assert_eq!(rows(&cards[0])[0].detail.as_deref(), Some("priority: High"));

    // Labels on: b already has bug.
    assert_eq!(
        set_label_many(&conn, &all, &bug.id, true).unwrap(),
        board.id
    );
    let after_on: Vec<usize> = cards.iter().map(|c| rows(c).len()).collect();
    assert_eq!(after_on, [after[0] + 1, after[1], after[2] + 1]);
    assert_eq!(
        rows(&cards[0])[0].detail.as_deref(),
        Some("labels: bug, Docs")
    );
    assert_eq!(rows(&cards[2])[0].detail.as_deref(), Some("labels: bug"));
    // Labels off: only a has Docs.
    set_label_many(&conn, &all, &docs.id, false).unwrap();
    let after_off: Vec<usize> = cards.iter().map(|c| rows(c).len()).collect();
    assert_eq!(after_off, [after_on[0] + 1, after_on[1], after_on[2]]);
    assert_eq!(rows(&cards[0])[0].detail.as_deref(), Some("labels: bug"));
    set_label_many(&conn, &[cards[2].id.clone()], &bug.id, false).unwrap();
    assert_eq!(rows(&cards[2])[0].detail.as_deref(), Some("labels: none"));

    // Bad input fails the whole batch and changes nothing.
    let other = create_board(&conn, "Other").unwrap();
    let foreign = create_label(&conn, &other.id, "x", "000000").unwrap();
    let other_column = &get_board(&conn, &other.id).unwrap().columns[0];
    let stranger = create_card(&conn, &other_column.id, "s", "").unwrap();
    let total = count(&conn, "SELECT COUNT(*) FROM board_activity");
    assert!(set_label_many(&conn, &all, &foreign.id, true).is_err());
    assert!(set_priority_many(&conn, &all, 9).is_err());
    let mut with_missing = all.clone();
    with_missing.insert(1, "missing".into());
    assert!(set_priority_many(&conn, &with_missing, 1).is_err());
    assert_eq!(card_meta(&conn, &cards[0].id).unwrap().priority, 3);
    assert!(set_priority_many(&conn, &[], 1).is_err());
    let mut mixed = all.clone();
    mixed.push(stranger.id.clone());
    assert!(set_priority_many(&conn, &mixed, 1).is_err());
    assert!(set_label_many(&conn, &mixed, &bug.id, false).is_err());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM board_activity"), total);
}

#[test]
fn card_index_has_start_estimate_fields_in_one_read() {
    let conn = fresh();
    let (board, columns, cards) = board_with_cards(&conn, &["a", "b", "gone"]);
    move_card(&conn, &cards[1].id, &columns[1].id, 0, "user").unwrap();
    let bug = create_label(&conn, &board.id, "Bug", "e5534b").unwrap();
    let points = create_field(&conn, &board.id, "Points", "number", None).unwrap();
    set_priority(&conn, &cards[0].id, 2).unwrap();
    set_due(&conn, &cards[0].id, Some(MIDNIGHT_UTC)).unwrap();
    set_start(&conn, &cards[0].id, Some(MIDNIGHT_UTC - DAY)).unwrap();
    set_estimate(&conn, &cards[0].id, Some(90)).unwrap();
    set_card_labels(&conn, &cards[0].id, std::slice::from_ref(&bug.id)).unwrap();
    set_projects(&conn, &cards[0].id, &["/b".into(), "/a".into()]).unwrap();
    set_field_value(&conn, &cards[0].id, &points.id, Some("8")).unwrap();
    archive_card(&conn, &cards[2].id).unwrap();

    let index = card_index(&conn, &board.id).unwrap();
    // Live cards only, in manual order (column, then position).
    let order: Vec<&str> = index.iter().map(|r| r.card_id.as_str()).collect();
    assert_eq!(order, [cards[0].id.as_str(), cards[1].id.as_str()]);
    let a = &index[0];
    assert_eq!(
        (a.column_id.as_str(), a.number, a.title.as_str()),
        (columns[0].id.as_str(), 1, "a")
    );
    assert_eq!(a.priority, 2);
    assert_eq!(
        (a.due_at, a.start_at, a.estimate_minutes),
        (Some(MIDNIGHT_UTC), Some(MIDNIGHT_UTC - DAY), Some(90))
    );
    assert_eq!(a.labels, [bug.id.as_str()]);
    assert_eq!(a.projects, ["/a", "/b"]);
    assert_eq!(a.fields.get(&points.id).map(String::as_str), Some("8"));
    let card = get_card(&conn, &cards[0].id).unwrap();
    assert_eq!(
        (a.created_at, a.updated_at),
        (card.created_at, card.updated_at)
    );
    let b = &index[1];
    assert_eq!(
        (b.column_id.as_str(), b.priority, b.due_at, b.start_at),
        (columns[1].id.as_str(), 0, None, None)
    );
    assert!(b.labels.is_empty() && b.projects.is_empty() && b.fields.is_empty());
    // Serialized camelCase for the Tauri side.
    let json = serde_json::to_value(a).unwrap();
    for key in [
        "cardId",
        "columnId",
        "dueAt",
        "startAt",
        "estimateMinutes",
        "createdAt",
        "updatedAt",
    ] {
        assert!(json.get(key).is_some(), "{key}");
    }
    assert!(card_index(&conn, "no-board").unwrap().is_empty());
}
