//! The `query_cards` time budget on the 1,000-card fixture, in its own file
//! like every timing test. The statement-count budgets are in `scale.rs`.
use gespan_board_store::seed::{seed_board, SeedSpec};
use gespan_board_store::*;
use rusqlite::Connection;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const BUDGET: Duration = Duration::from_millis(150);

fn row(field: &str, values: &[&str]) -> FilterRow {
    FilterRow {
        field: field.to_string(),
        op: Op::Is,
        values: values.iter().map(|v| v.to_string()).collect(),
    }
}

#[test]
fn query_cards_on_1000_cards_meets_the_budget() {
    // No trace callback on this connection: it would add to every statement.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    ensure_tables(&conn).unwrap();
    let board = create_board(&conn, "Scale").unwrap();
    let report = seed_board(&conn, &board.id, &SeedSpec::default()).unwrap();
    // A 3-row filter (label, priority, linked: failed) and a priority sort.
    let spec = ViewSpec {
        query: String::new(),
        rows: vec![
            row("labels", &[&report.label_ids[0], &report.label_ids[1]]),
            row("priority", &["2", "3", "4"]),
            row("linked", &["failed"]),
        ],
        sort: SortSpec {
            key: SortKey::Priority,
            dir: Dir::Desc,
        },
    };
    let now = 1_793_404_800_000;
    let running = HashSet::new();
    let run = || query_cards(&conn, &board.id, &spec, now, &local_day_chrono, &running).unwrap();
    let warm = run();
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let started = Instant::now();
        let shown = run();
        best = best.min(started.elapsed());
        assert_eq!(shown, warm, "every run gives the same result");
    }
    // The filter really ran: some cards match, not all, sorted by priority,
    // each with a label in the filter and a last run that failed.
    assert!(!warm.is_empty() && warm.len() < 1000, "{}", warm.len());
    let extras = card_extras(&conn, &board.id).unwrap();
    let priorities: Vec<u8> = warm.iter().map(|id| extras[id].priority).collect();
    assert!(priorities.windows(2).all(|w| w[0] >= w[1]));
    assert!(priorities.iter().all(|&p| p >= 2));
    assert!(warm.iter().all(|id| extras[id]
        .labels
        .iter()
        .any(|l| l == &report.label_ids[0] || l == &report.label_ids[1])));
    let runs = latest_run_events(&conn, &board.id).unwrap();
    assert!(warm.iter().all(|id| runs[id] == "failed"));
    assert!(
        best <= BUDGET,
        "query_cards on 1,000 cards took {best:?} at best of 3, budget {BUDGET:?}"
    );
}
