//! Home's own rules: the greeting, the summary sentence, what each range
//! counts, the chart's days and the timeline, from plain records.
use super::*;

const HOUR: i64 = 3_600_000;

fn run(id: &str, status: &str, created_at: i64) -> AutomationRun {
    AutomationRun {
        id: id.into(),
        automation_id: "auto-fix".into(),
        trigger: "event".into(),
        scheduled_for: created_at,
        created_at,
        started_at: Some(created_at),
        completed_at: matches!(status, "succeeded" | "failed").then_some(created_at + 60_000),
        status: status.into(),
        session_id: None,
        error: None,
        event_key: None,
        event_kind: None,
        event: None,
        event_title: None,
        prompt: None,
        pilot_message_id: None,
    }
}

fn waiting(key: &str, kind: Kind, since_ms: u64) -> Waiting {
    Waiting {
        key: key.into(),
        kind,
        headline: "Run gh pr create?".into(),
        source_label: "Auto-fix langfuse issues".into(),
        project: "/work/langfuse".into(),
        since_ms,
        source: Source::Session(1),
        allow: None,
    }
}

fn working(id: i64, origin: &str, project: &str) -> Working {
    Working {
        id,
        title: format!("Agent {id}"),
        provider: "claude".into(),
        origin: origin.into(),
        project: project.into(),
        started_at: None,
    }
}

/// Noon today in local time, so every range edge is a whole day away.
fn noon() -> i64 {
    day_start(gespan_core::time::now_ms() as i64, 0) + 12 * HOUR
}

#[test]
fn the_greeting_follows_the_local_hour() {
    for hour in 0..5 {
        assert_eq!(
            greeting(hour),
            "Good evening.",
            "{hour}:00 is still last night"
        );
    }
    for hour in 5..12 {
        assert_eq!(greeting(hour), "Good morning.", "{hour}:00");
    }
    for hour in 12..18 {
        assert_eq!(greeting(hour), "Good afternoon.", "{hour}:00");
    }
    for hour in 18..24 {
        assert_eq!(greeting(hour), "Good evening.", "{hour}:00");
    }
}

#[test]
fn the_summary_reads_the_mock_sentence_and_each_edge() {
    let counts = |succeeded, failed, running| RunCounts {
        total: succeeded + failed + running,
        succeeded,
        failed,
    };
    // The mock's own sentence.
    assert_eq!(
        summary(7, 2, counts(12, 1, 0), Range::Today),
        "7 agents are working, 2 things are waiting on you, and every automation passed today except one."
    );
    // One of each reads in the singular.
    assert_eq!(
        summary(1, 1, counts(3, 0, 0), Range::Week),
        "1 agent is working, 1 thing is waiting on you, and every automation passed in the last 7 days."
    );
    // A quiet day.
    assert_eq!(
        summary(0, 0, counts(0, 0, 0), Range::Today),
        "No agents are working, nothing is waiting on you, and no automations ran today."
    );
    // Runs started but none finished is not "no automations ran".
    assert_eq!(
        summary(0, 0, counts(0, 0, 2), Range::Today),
        "No agents are working, nothing is waiting on you, and automations are still running today."
    );
    // More than one failure is counted, not called "except one".
    assert_eq!(
        summary(3, 0, counts(10, 3, 0), Range::Month),
        "3 agents are working, nothing is waiting on you, and 3 of 13 automation runs failed in the last 30 days."
    );
    // Everything failed.
    assert_eq!(
        summary(0, 0, counts(0, 4, 0), Range::Today),
        "No agents are working, nothing is waiting on you, and all 4 automation runs failed today."
    );
    assert_eq!(
        summary(0, 0, counts(0, 1, 0), Range::Today),
        "No agents are working, nothing is waiting on you, and the one automation run failed today."
    );
}

#[test]
fn each_range_counts_only_the_runs_it_covers() {
    let now = noon();
    let runs = vec![
        run("a", "succeeded", now - HOUR),
        run("b", "failed", now - 2 * HOUR),
        run("c", "running", now - 3 * HOUR),
        // Three days ago: in 7 and 30 days, not today.
        run("d", "succeeded", day_start(now, 3) + HOUR),
        run("e", "failed", day_start(now, 3) + 2 * HOUR),
        // Twenty days ago: only in 30 days.
        run("f", "succeeded", day_start(now, 20) + HOUR),
        // Forty days ago: in none.
        run("g", "failed", day_start(now, 40) + HOUR),
    ];
    let count = |range: Range| run_counts(&runs, day_start(now, range.days() - 1));
    assert_eq!(
        count(Range::Today),
        RunCounts {
            total: 3,
            succeeded: 1,
            failed: 1
        }
    );
    assert_eq!(
        count(Range::Week),
        RunCounts {
            total: 5,
            succeeded: 2,
            failed: 2
        }
    );
    assert_eq!(
        count(Range::Month),
        RunCounts {
            total: 6,
            succeeded: 3,
            failed: 2
        }
    );
    // A run from just before today's midnight belongs to yesterday.
    let edge = vec![run("y", "succeeded", day_start(now, 0) - 1)];
    assert_eq!(run_counts(&edge, day_start(now, 0)).total, 0);
    assert_eq!(run_counts(&edge, day_start(now, 6)).total, 1);
}

#[test]
fn the_pass_rate_is_over_finished_runs_and_rounds() {
    let rate = |succeeded, failed| {
        RunCounts {
            total: succeeded + failed,
            succeeded,
            failed,
        }
        .pass_rate()
    };
    assert_eq!(rate(12, 1), Some(92), "the mock's 92%");
    assert_eq!(rate(0, 0), None, "nothing finished has no rate");
    assert_eq!(rate(1, 0), Some(100));
    assert_eq!(rate(0, 3), Some(0));
    assert_eq!(rate(2, 1), Some(67));
}

#[test]
fn the_seven_day_bars_are_built_from_the_run_records() {
    let now = noon();
    let starts: Vec<i64> = (0..7).rev().map(|ago| day_start(now, ago)).collect();
    let runs = vec![
        run("today-ok", "succeeded", now - HOUR),
        run("today-ok-2", "succeeded", now - 2 * HOUR),
        run("today-bad", "failed", now - 3 * HOUR),
        // Still running or cancelled: in neither stack.
        run("today-live", "running", now - HOUR),
        run("today-cancel", "cancelled", now - HOUR),
        run("mon-ok", "succeeded", day_start(now, 3) + HOUR),
        run("oldest-bad", "failed", day_start(now, 6)),
        // Before the chart's first day.
        run("too-old", "succeeded", day_start(now, 7) + HOUR),
    ];
    let bars = day_bars(&runs, &starts);
    assert_eq!(bars.len(), 7);
    assert_eq!(bars.iter().map(|b| b.start).collect::<Vec<_>>(), starts);
    let tally: Vec<(usize, usize)> = bars.iter().map(|b| (b.succeeded, b.failed)).collect();
    assert_eq!(
        tally,
        vec![(0, 1), (0, 0), (0, 0), (1, 0), (0, 0), (0, 0), (2, 1)],
        "oldest first, today last"
    );
}

#[test]
fn the_chart_spans_a_week_unless_the_range_is_a_month() {
    assert_eq!(Range::Today.chart_days(), 7);
    assert_eq!(Range::Week.chart_days(), 7);
    assert_eq!(Range::Month.chart_days(), 30);
}

#[test]
fn tile_lines_break_down_tasks_agents_and_the_inbox() {
    let tasks = [
        waiting("a", Kind::Approval, 0),
        waiting("q", Kind::Question, 0),
        waiting("a2", Kind::Approval, 0),
    ];
    assert_eq!(kinds_line(&tasks), "2 approvals · 1 question");
    assert_eq!(kinds_line(&[]), "Nothing waiting");
    // Decision-log proposals count too, after the agents' own requests.
    let with_decisions = [
        waiting("d", Kind::Decision, 0),
        waiting("a", Kind::Approval, 0),
        waiting("d2", Kind::Decision, 0),
    ];
    assert_eq!(
        kinds_line(&with_decisions),
        "1 approval · 2 decisions to review"
    );

    let agents = [
        working(1, "Codex", "/a"),
        working(2, "Claude Code", "/a"),
        working(3, "Claude Code", "/b"),
        working(4, "ReleasePilot", "/c"),
    ];
    assert_eq!(
        origins_line(&agents),
        "2 Claude Code · 1 Codex · 1 ReleasePilot"
    );
    assert_eq!(origins_line(&[]), "No agents running");

    let inbox = |review, assigned, total| InboxCounts {
        total,
        unread: 0,
        review,
        assigned,
        reviews: Vec::new(),
    };
    assert_eq!(
        inbox_line(Some(&inbox(3, 2, 8))),
        "3 reviews requested · 2 assigned"
    );
    assert_eq!(inbox_line(Some(&inbox(1, 0, 8))), "1 review requested");
    assert_eq!(
        inbox_line(Some(&inbox(0, 0, 8))),
        "No reviews or assignments"
    );
    assert_eq!(inbox_line(Some(&inbox(0, 0, 0))), "Nothing in the Inbox");
    assert_eq!(inbox_line(None), "Loading the Inbox");
}

#[test]
fn ages_read_in_minutes_hours_and_days() {
    assert_eq!(age(0), "1m");
    assert_eq!(age(59 * 60_000), "59m");
    assert_eq!(age(2 * HOUR as u64), "2h");
    assert_eq!(age(26 * HOUR as u64), "1d");
}

#[test]
fn the_timeline_holds_todays_runs_tasks_and_reviews_newest_first() {
    let now = noon();
    let today = day_start(now, 0);
    let mut ok = run("ok", "succeeded", now - 3 * HOUR);
    ok.event_title = Some("langfuse/langfuse#20107 Fix login".into());
    let mut bad = run("bad", "failed", now - 2 * HOUR);
    bad.automation_id = "socket".into();
    bad.error = Some("tests timed out\nmore detail".into());
    let yesterday = run("old", "succeeded", today - HOUR);
    let live = run("live", "running", now - HOUR);
    let names = HashMap::from([
        ("auto-fix".to_owned(), "Auto-fix langfuse".to_owned()),
        ("socket".to_owned(), "Auto-fix Socket".to_owned()),
    ]);
    let data = Data {
        waiting: vec![
            waiting("new", Kind::Question, (now - HOUR) as u64),
            // Raised yesterday: not today's event.
            waiting("stale", Kind::Approval, (today - HOUR) as u64),
        ],
        inbox: Some(InboxCounts {
            reviews: vec![
                ("gespan#476".into(), now - 30 * 60_000),
                ("gespan#400".into(), today - 2 * HOUR),
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let moments = timeline(&[ok, bad, yesterday, live], &names, &data, today);
    let shown: Vec<(&str, &str, Tone)> = moments
        .iter()
        .map(|m| (m.name.as_str(), m.text.as_str(), m.tone))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("gespan#476", "is waiting for your review", Tone::Running),
            (
                "Auto-fix langfuse issues",
                "asked: Run gh pr create?",
                Tone::Waiting
            ),
            ("Auto-fix Socket", "failed: tests timed out", Tone::Bad),
            (
                "Auto-fix langfuse",
                "finished langfuse#20107 Fix login",
                Tone::Ok
            ),
        ]
    );
    // An automation deleted since still shows its run.
    let orphan = run("orphan", "succeeded", now - HOUR);
    let moments = timeline(&[orphan], &HashMap::new(), &Data::default(), today);
    assert_eq!(moments[0].name, "An automation");
    assert!(timeline(&[], &names, &Data::default(), today).is_empty());
}

#[test]
fn inbox_items_are_named_by_repo_and_number_else_their_key() {
    let item = |repo: &str, number: i64, identifier: &str, title: &str| gespan_core::inbox::Item {
        repo: repo.into(),
        number,
        identifier: identifier.into(),
        title: title.into(),
        ..Default::default()
    };
    assert_eq!(
        item_reference(&item("0xvasanth/gespan", 476, "", "Fix")),
        "gespan#476"
    );
    assert_eq!(item_reference(&item("", 7, "ENG-7", "Fix")), "ENG-7");
    assert_eq!(item_reference(&item("", 0, "", "Fix login")), "Fix login");
}

#[test]
fn dark_brand_tiles_draw_in_ink_so_their_bars_show() {
    // Claude keeps its orange.
    assert_eq!(provider_color("claude"), provider_tile("claude").unwrap().0);
    // Cursor's tile is near black: it would vanish on the dark track.
    assert_eq!(provider_color("cursor"), content(0.9));
    // No tile at all (a multicolour logo) is ink too.
    assert_eq!(provider_color("antigravity"), content(0.9));
}

#[test]
fn only_a_short_lone_command_can_be_allowed_from_home() {
    // The card found a plain shell command: offered, and shown whole.
    assert_eq!(
        one_click(Some("gh pr create --fill")),
        Some("gh pr create --fill".into())
    );
    assert_eq!(
        one_click(Some("  git status \n")),
        Some("git status".into())
    );
    // No lone command: an alert, an MCP tool or extra arguments.
    assert_eq!(one_click(None), None);
    assert_eq!(one_click(Some("   ")), None);
    // More than one line, or too long to show without cutting it.
    assert_eq!(one_click(Some("cd app\nrm -rf build")), None);
    let long = "x".repeat(ONE_CLICK_MAX + 1);
    assert_eq!(one_click(Some(&long)), None);
    assert!(one_click(Some(&"x".repeat(ONE_CLICK_MAX))).is_some());
}

#[test]
fn layout_follows_the_content_width() {
    let wide = Layout {
        columns: true,
        tiles_per_row: 4,
    };
    assert_eq!(layout(0.), wide, "unmeasured lays out wide");
    assert_eq!(layout(MAX_WIDTH), wide);
    assert_eq!(layout(COLUMNS_FROM), wide);
    assert_eq!(
        layout(COLUMNS_FROM - 1.),
        Layout {
            columns: false,
            tiles_per_row: 4
        }
    );
    assert_eq!(
        layout(FOUR_TILES_FROM),
        Layout {
            columns: false,
            tiles_per_row: 4
        }
    );
    assert_eq!(
        layout(FOUR_TILES_FROM - 1.),
        Layout {
            columns: false,
            tiles_per_row: 2
        }
    );
    assert_eq!(
        layout(ONE_TILE_BELOW),
        Layout {
            columns: false,
            tiles_per_row: 2
        }
    );
    assert_eq!(
        layout(ONE_TILE_BELOW - 1.),
        Layout {
            columns: false,
            tiles_per_row: 1
        }
    );
}
