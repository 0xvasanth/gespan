//! Deferred board and drawer fixes (C6-9): the switcher, the fallback when
//! the open board is deleted elsewhere, drawer Undo, column display patches,
//! the links list, the shared estimate grammar, the card menu, the new-card
//! modal's due field and the column body-height cache.
use super::link_run::{add_link, install_turns};
use super::*;
use crate::board::{
    detail::more::{card_reference, copy_label},
    quick::{ESTIMATE_HELP, encode_field, parse_estimate},
    turns,
};
use gespan_core::board::extras::ColumnDisplayPatch;

/// Make every title edit fail, as a full disk or a lost lock would.
fn break_title_saves(f: &Fixture) {
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER no_title_saves BEFORE UPDATE OF title ON board_cards
             BEGIN SELECT RAISE(ABORT, 'database is read-only'); END;",
        )
        .unwrap();
}

fn mend_title_saves(f: &Fixture) {
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute_batch("DROP TRIGGER no_title_saves;")
        .unwrap();
}

fn boards_named(f: &Fixture, name: &str) -> usize {
    boards(f).iter().filter(|board| board.name == name).count()
}

/// A second board, "Sprint", with two cards, opened in the view.
fn open_sprint(f: &Fixture, cx: &mut TestAppContext) -> gespan_core::board::Board {
    let mut service = Service::open(&f.db).unwrap();
    let sprint = service.create("Sprint").unwrap();
    let todo = service.get(&sprint.id).unwrap().columns[0].id.clone();
    service.card_create(&todo, "Alpha", "").unwrap();
    service.card_create(&todo, "Beta", "").unwrap();
    let id = sprint.id.clone();
    f.window
        .update(cx, |view, _, cx| assert!(view.open_board(id, cx)))
        .unwrap();
    cx.run_until_parked();
    sprint
}

/// Let the poll tick see a write from another connection.
fn poll_tick(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
}

fn picker_state(f: &Fixture, cx: &mut impl gpui::AppContext) -> (bool, String, Option<String>) {
    drawer(f, cx, |view, cx| {
        view.card_detail.as_ref().unwrap().picker_state(cx)
    })
}

fn select_all(visual: &mut VisualTestContext) {
    visual.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
}

fn store_column(f: &Fixture, id: &str) -> gespan_core::board::Column {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service
        .get(&board.id)
        .unwrap()
        .columns
        .into_iter()
        .find(|column| column.id == id)
        .unwrap()
}

// ---- R48: the switcher ----

#[gpui::test]
fn failed_open_after_create_keeps_the_name_and_shows_the_error(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let first = open_id(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-title", "Edited");
    break_title_saves(&f);
    click(visual, "board-switcher");
    click(visual, "board-new");
    visual.simulate_input("Planning");
    press_enter(visual);
    // The board was made, but the drawer's edit couldn't save, so it isn't
    // open: the name stays and the menu says why.
    assert_eq!(boards_named(&f, "Planning"), 1);
    assert_eq!(open_id(&f, visual), first);
    assert!(shown(visual, "board-switcher-error"));
    let (name, error) = drawer(&f, visual, |view, cx| {
        (view.switcher_name_text(cx), view.switcher_error_text())
    });
    assert_eq!(name.as_deref(), Some("Planning"));
    let error = error.unwrap();
    assert!(
        error.starts_with("Created \"Planning\" but couldn't open it: "),
        "{error}"
    );
    assert!(error.contains("read-only"), "{error}");
    // A second Enter makes no duplicate.
    press_enter(visual);
    assert_eq!(boards_named(&f, "Planning"), 1);
    assert_eq!(open_id(&f, visual), first);
    // Once the edit can save, Enter opens the board made the first time.
    mend_title_saves(&f);
    press_enter(visual);
    assert_eq!(boards_named(&f, "Planning"), 1);
    assert_eq!(open_name(&f, visual), "Planning");
    let task = Service::open(&f.db)
        .unwrap()
        .get(&first)
        .unwrap()
        .cards
        .remove(0);
    assert_eq!(
        task.title, "Edited",
        "the drawer's edit was saved on the way"
    );
}

#[gpui::test]
fn a_changed_name_after_a_failed_open_makes_that_board(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-title", "Edited");
    break_title_saves(&f);
    click(visual, "board-switcher");
    click(visual, "board-new");
    visual.simulate_input("Planning");
    press_enter(visual);
    // A different name is a different board, not a retry of the first.
    mend_title_saves(&f);
    visual.simulate_input(" two");
    press_enter(visual);
    assert_eq!(boards_named(&f, "Planning"), 1);
    assert_eq!(boards_named(&f, "Planning two"), 1);
    assert_eq!(open_name(&f, visual), "Planning two");
}

// ---- R49 and R54: the open board deleted elsewhere ----

#[gpui::test]
fn board_deleted_elsewhere_resets_composer_menus_filters_and_search(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let first = open_id(&f, cx);
    let sprint = open_sprint(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-search");
    visual.simulate_input("alp");
    visual.run_until_parked();
    let (todo, alpha) = (
        detail(&f, visual).columns[0].id.clone(),
        card_id(&f, visual, "Alpha"),
    );
    f.window
        .update(visual, |view, window, cx| {
            filter_by_priority(view, 1, cx);
            view.open_composer(todo.clone(), window, cx);
            view.filter_open = true;
            view.sort_open = true;
            view.menu = Some(Menu::Column {
                column_id: todo.clone(),
                roles: false,
            });
            view.selection.ids.insert(alpha.clone());
            view.open_card_detail(alpha.clone(), window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    Service::open(&f.db).unwrap().delete(&sprint.id).unwrap();
    poll_tick(visual);
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.open_board_id.as_deref(), Some(first.as_str()));
            assert!(view.composer.is_none(), "the composer stayed");
            assert!(view.menu.is_none(), "the column menu stayed");
            assert!(!view.filter_open && !view.sort_open, "a panel stayed open");
            assert!(view.spec.rows.is_empty(), "the filter stayed");
            assert_eq!(view.search.read(cx).content.to_string(), "");
            assert!(view.selection.ids.is_empty(), "the selection stayed");
            assert!(view.card_detail.is_none(), "the drawer stayed");
            assert!(view.load_error.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn escape_in_the_new_board_name_input_cancels(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (closed, _events) = events(&f, visual);
    let before = boards(&f).len();
    click(visual, "board-switcher");
    click(visual, "board-new");
    visual.simulate_input("Not this one");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!shown(visual, "board-new-name"), "the name field stayed");
    assert!(!shown(visual, "board-switcher-menu"), "the switcher stayed");
    assert_eq!(boards(&f).len(), before, "Escape made a board");
    assert!(closed.borrow().is_empty(), "Escape also closed the board");
    // The next New board starts empty.
    click(visual, "board-switcher");
    click(visual, "board-new");
    let name = drawer(&f, visual, |view, cx| view.switcher_name_text(cx));
    assert_eq!(name.as_deref(), Some(""));
}

#[gpui::test]
fn undo_toast_clears_when_load_falls_back_to_another_board(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let sprint = open_sprint(&f, cx);
    let (alpha, done) = (
        card_id(&f, cx, "Alpha"),
        detail(&f, cx).columns[3].id.clone(),
    );
    f.window
        .update(cx, |view, _, cx| {
            assert!(view.move_with_undo(&alpha, &done, usize::MAX, cx))
        })
        .unwrap();
    assert!(toast(&f, cx).is_some());
    Service::open(&f.db).unwrap().delete(&sprint.id).unwrap();
    poll_tick(cx);
    assert_eq!(toast(&f, cx), None, "the old board's Undo stayed");
    let error = f
        .window
        .read_with(cx, |view, _| view.error.clone())
        .unwrap();
    assert_eq!(error, None);
}

#[gpui::test]
fn a_held_delete_toast_clears_quietly_when_its_board_goes(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let sprint = open_sprint(&f, cx);
    let alpha = card_id(&f, cx, "Alpha");
    f.window
        .update(cx, |view, _, cx| view.delete_card(&alpha, cx))
        .unwrap();
    assert_eq!(toast(&f, cx).as_deref(), Some("Deleted #1"));
    Service::open(&f.db).unwrap().delete(&sprint.id).unwrap();
    poll_tick(cx);
    assert_eq!(toast(&f, cx), None);
    let error = f
        .window
        .read_with(cx, |view, _| view.error.clone())
        .unwrap();
    assert_eq!(error, None, "the gone card's delete is not an error");
}

// ---- R54: the drawer's Status picker ----

#[gpui::test]
fn status_picker_move_in_the_drawer_offers_undo(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let review = column_id(&f, cx, "Review");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-status");
    click(visual, selector(format!("menu-action-status-{review}")));
    assert_eq!(titles_in(&f, visual, "Review"), ["Task"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Moved #1 to Review"));
    drawer(&f, visual, |view, _| assert!(view.card_detail.is_some()));
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["Task"]);
    assert!(titles_in(&f, visual, "Review").is_empty());
    // Picking the step the card is already in writes nothing and says nothing.
    let backlog = column_id(&f, visual, "Backlog");
    click(visual, "board-drawer-status");
    click(visual, selector(format!("menu-action-status-{backlog}")));
    assert_eq!(toast(&f, visual), None);
}

// ---- R53 and R86: column display patches ----

#[gpui::test]
fn collapse_keeps_a_wip_limit_written_by_the_other_app(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let review = column_id(&f, cx, "Review");
    // The other app sets a colour and a limit; this view hasn't reloaded.
    Service::open(&f.db)
        .unwrap()
        .update_column_display(
            &review,
            &ColumnDisplayPatch {
                color: Some(Some("4da3f5".into())),
                wip_limit: Some(Some(5)),
                ..Default::default()
            },
        )
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-column-menu-2");
    click(visual, "menu-collapse");
    let column = store_column(&f, &review);
    assert_eq!(
        (column.color.as_deref(), column.wip_limit, column.collapsed),
        (Some("4da3f5"), Some(5), true)
    );
    // The other app changes the limit again; expanding keeps it too.
    Service::open(&f.db)
        .unwrap()
        .update_column_display(
            &review,
            &ColumnDisplayPatch {
                wip_limit: Some(Some(7)),
                ..Default::default()
            },
        )
        .unwrap();
    f.window
        .update(visual, |view, _, cx| {
            view.set_collapsed(review.clone(), false, cx)
        })
        .unwrap();
    let column = store_column(&f, &review);
    assert_eq!(
        (column.color.as_deref(), column.wip_limit, column.collapsed),
        (Some("4da3f5"), Some(7), false)
    );
}

#[gpui::test]
fn settings_colour_and_limit_keep_what_the_other_app_wrote(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let backlog = column_id(&f, cx, "Backlog");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_settings(visual, "columns");
    // The other app colours and collapses the column after the page opened.
    Service::open(&f.db)
        .unwrap()
        .update_column_display(
            &backlog,
            &ColumnDisplayPatch {
                color: Some(Some("3fb950".into())),
                collapsed: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    click(visual, "settings-col-0-wip");
    retype(visual, "settings-edit", "4");
    press_enter(visual);
    let column = store_column(&f, &backlog);
    assert_eq!(
        (column.color.as_deref(), column.wip_limit, column.collapsed),
        (Some("3fb950"), Some(4), true)
    );
    // Now it sets a limit; a colour picked here keeps it.
    Service::open(&f.db)
        .unwrap()
        .update_column_display(
            &backlog,
            &ColumnDisplayPatch {
                wip_limit: Some(Some(9)),
                ..Default::default()
            },
        )
        .unwrap();
    click(visual, "settings-col-0-color-4da3f5");
    let column = store_column(&f, &backlog);
    assert_eq!(
        (column.color.as_deref(), column.wip_limit, column.collapsed),
        (Some("4da3f5"), Some(9), true)
    );
}

// ---- Controller addition: the header delete offers "Move cards to…" ----

#[gpui::test]
fn header_delete_with_cards_offers_move_cards_to(cx: &mut TestAppContext) {
    let f = seeded(cx, &["One", "Two"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-column-menu-0");
    assert!(shown(visual, "board-column-move-cards"));
    // Every other column is a target; the column itself is not.
    assert!(!shown(visual, "board-column-move-delete-0"));
    for index in 1..4 {
        assert!(shown(
            visual,
            selector(format!("board-column-move-delete-{index}"))
        ));
    }
    // Delete itself stays blocked.
    click(visual, "board-column-delete");
    assert_eq!(column_names(&f, visual).len(), 4);
    click(visual, "board-column-move-delete-2");
    let names: Vec<String> = column_names(&f, visual).into_iter().map(|c| c.0).collect();
    assert_eq!(names, ["In Progress", "Review", "Done"]);
    assert_eq!(
        titles_in(&f, visual, "Review"),
        ["One", "Two"],
        "order kept"
    );
    drawer(&f, visual, |view, _| assert!(view.menu.is_none()));

    // A column with no cards offers nothing to move.
    click(visual, "board-column-menu-0");
    assert!(!shown(visual, "board-column-move-cards"));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();

    // Archived cards still block the delete, so no move is offered then.
    let review = column_id(&f, visual, "Review");
    let mut service = Service::open(&f.db).unwrap();
    let kept = service.card_create(&review, "Archived", "").unwrap();
    service.archive_many(&[kept.id]).unwrap();
    reload(&f, visual);
    click(visual, "board-column-menu-1");
    assert!(!shown(visual, "board-column-move-cards"));
    assert_eq!(titles_in(&f, visual, "Review"), ["One", "Two"]);
}

// ---- R61: the links list and the estimate ----

#[gpui::test]
fn links_list_hides_the_primary_link_and_names_sessions_and_pilots(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    install_turns(&f, cx);
    cx.update(|cx| {
        cx.global_mut::<turns::BoardTurns>().test_pilots =
            Some(vec![("p-1".into(), "Scout".into())]);
    });
    let card = card_id(&f, cx, "Task");
    let (primary, extra) = {
        let store = f.store.lock().unwrap();
        let primary = store.create("codex", "/work/a").unwrap();
        let extra = store.create("codex", "/work/b").unwrap();
        (
            (
                store.persisted_session_id(primary.id).unwrap(),
                primary.title,
            ),
            (store.persisted_session_id(extra.id).unwrap(), extra.title),
        )
    };
    Service::open(&f.db)
        .unwrap()
        .card_link(&card, Some("session"), Some(&primary.0))
        .unwrap();
    // The primary also stored as an extra row (an older build's promotion).
    add_link(&f, &card, "session", &primary.0);
    add_link(&f, &card, "session", &extra.0);
    add_link(&f, &card, "pilot", "p-1");
    add_link(&f, &card, "pilot", "p-gone");
    add_link(&f, &card, "pr", "https://github.com/a/b/pull/1");
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    let rows: Vec<(String, String)> = drawer(&f, visual, |view, _| {
        let card = view.card(&card).unwrap();
        view.card_detail
            .as_ref()
            .unwrap()
            .link_rows(card)
            .into_iter()
            .map(|link| (link.kind.clone(), view.link_row_text(link)))
            .collect()
    });
    let mut expected = vec![
        ("pilot".to_string(), "Scout".to_string()),
        ("pilot".to_string(), "No longer exists".to_string()),
        (
            "pr".to_string(),
            "https://github.com/a/b/pull/1".to_string(),
        ),
        ("session".to_string(), extra.1.clone()),
    ];
    let mut rows_sorted = rows.clone();
    rows_sorted.sort();
    expected.sort();
    assert_eq!(rows_sorted, expected, "{rows:?}");
    // Four rows are drawn: the primary is only in the Linked work row.
    assert!(shown(visual, "board-link-row-3"));
    assert!(!shown(visual, "board-link-row-4"));
    assert!(shown(visual, "board-open-linked"));
}

#[test]
fn estimate_grammar_matches_the_shared_rule() {
    for (text, minutes) in [
        ("90", 90),
        ("90m", 90),
        ("1h", 60),
        ("1h 30m", 90),
        ("1.5h", 90),
        (".5h", 30),
        ("0.5h", 30),
        ("1.25h", 75),
        ("0.01h", 1),
        (" 1H 5M ", 65),
    ] {
        assert_eq!(parse_estimate(text), Ok(minutes), "{text}");
    }
    for text in [
        "1.m", "0x10", "1e3", "-5", "-1h", "h", "m", "1h m", "1.5", "1..5h", "1.2.3h", "1.h",
        "+1h", "abc",
    ] {
        assert_eq!(
            parse_estimate(text),
            Err(ESTIMATE_HELP.to_string()),
            "{text:?}"
        );
    }
    assert_eq!(
        ESTIMATE_HELP,
        "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h"
    );
    assert!(parse_estimate("0m").is_err());
    // A lone 0 parses but is refused with the shared (Tauri `cardProps.ts`)
    // minimum message.
    assert_eq!(
        parse_estimate("0"),
        Err("An estimate must be at least 1 minute".to_string())
    );
}

#[test]
fn field_numbers_reject_hex_and_exponent() {
    for text in [
        "0x10", "1e3", "1E3", "0b11", "0o7", "Infinity", "inf", "NaN", "1.", "+3", "1_000", "--1",
    ] {
        assert!(encode_field("number", text).is_err(), "{text:?}");
    }
    assert_eq!(encode_field("number", "-2.5"), Ok(Some("-2.5".into())));
    assert_eq!(encode_field("number", ".5"), Ok(Some("0.5".into())));
    assert_eq!(encode_field("number", " 42 "), Ok(Some("42".into())));
    assert_eq!(encode_field("number", "-7"), Ok(Some("-7".into())));
}

#[gpui::test]
fn estimate_picker_prefills_the_current_value(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    Service::open(&f.db)
        .unwrap()
        .set_estimate(&card, Some(90))
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-estimate");
    assert_eq!(picker_state(&f, visual).1, "1h 30m");
    // A rejected value keeps the typed text with the shared hint.
    select_all(visual);
    visual.simulate_input("1.m");
    visual.simulate_keystrokes("enter");
    assert_eq!(
        picker_state(&f, visual),
        (true, "1.m".to_string(), Some(ESTIMATE_HELP.to_string()))
    );
    assert_eq!(meta_of(&f, &card).estimate_minutes, Some(90));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    // With no estimate the field starts empty.
    Service::open(&f.db)
        .unwrap()
        .set_estimate(&card, None)
        .unwrap();
    reload(&f, visual);
    click(visual, "board-drawer-estimate");
    assert_eq!(picker_state(&f, visual).1, "");
}

#[gpui::test]
fn escape_in_the_estimate_puts_the_saved_value_back_without_a_write(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    Service::open(&f.db)
        .unwrap()
        .set_estimate(&card, Some(90))
        .unwrap();
    reload(&f, cx);
    let before = activity_details(&f, &card).len();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-estimate");
    select_all(visual);
    visual.simulate_input("2h");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(meta_of(&f, &card).estimate_minutes, Some(90));
    assert_eq!(activity_details(&f, &card).len(), before, "Escape wrote");
    assert!(!picker_state(&f, visual).0, "Escape closes the field");
    drawer(&f, visual, |view, _| assert!(view.card_detail.is_some()));
    // The saved value is what shows when it opens again.
    click(visual, "board-drawer-estimate");
    assert_eq!(picker_state(&f, visual).1, "1h 30m");
}

#[gpui::test]
fn escape_in_a_custom_field_input_keeps_the_typed_text_and_the_drawer(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    add_field(&f, "Note", "text", None, 0);
    add_field(&f, "Points", "number", None, 1);
    add_field(&f, "Docs", "url", None, 2);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    // Text: Escape leaves the field, and leaving saves what was typed.
    click(visual, "board-drawer-field-0");
    visual.simulate_input("half a thought");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(
        field_value(&f, &card, "Note").as_deref(),
        Some("\"half a thought\"")
    );
    assert!(!picker_state(&f, visual).0);
    drawer(&f, visual, |view, _| assert!(view.card_detail.is_some()));
    // URL: the same.
    click(visual, "board-drawer-field-2");
    visual.simulate_input("https://example.com/spec");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(
        field_value(&f, &card, "Docs").as_deref(),
        Some("\"https://example.com/spec\"")
    );
    // Number: a refused value keeps the text and says why; the next Escape
    // drops it and the drawer still stays.
    click(visual, "board-drawer-field-1");
    visual.simulate_input("1e3");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    let (open, text, error) = picker_state(&f, visual);
    assert!(open);
    assert_eq!(text, "1e3");
    assert_eq!(error.as_deref(), Some("“1e3” is not a number"));
    assert_eq!(field_value(&f, &card, "Points"), None);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!picker_state(&f, visual).0);
    assert_eq!(field_value(&f, &card, "Points"), None);
    drawer(&f, visual, |view, _| assert!(view.card_detail.is_some()));
    // Unchanged text writes nothing and just closes the field.
    let before = activity_details(&f, &card).len();
    click(visual, "board-drawer-field-0");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(activity_details(&f, &card).len(), before);
    assert!(!picker_state(&f, visual).0);
    drawer(&f, visual, |view, _| assert!(view.card_detail.is_some()));
}

// ---- R63: the card menu and the copy row ----

fn card_menu_shape(f: &Fixture, visual: &mut VisualTestContext) -> Vec<String> {
    drawer(f, visual, |view, cx| {
        view.card_menu.as_ref().unwrap().menu.read(cx).item_shape()
    })
}

#[gpui::test]
fn card_menu_has_no_double_separator_without_move_rows(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-menu-1");
    assert_eq!(
        card_menu_shape(&f, visual),
        [
            "Open card",
            "---",
            "Move to In Progress",
            "Move to Review",
            "Move to Done",
            "---",
            "More…"
        ]
    );
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    // A one-column board has no Move rows: one separator, not two.
    let mut service = Service::open(&f.db).unwrap();
    for column in detail(&f, visual).columns.iter().skip(1) {
        service.column_delete(&column.id).unwrap();
    }
    reload(&f, visual);
    click(visual, "board-card-menu-1");
    assert_eq!(card_menu_shape(&f, visual), ["Open card", "---", "More…"]);
}

#[test]
fn copy_number_and_title_has_no_trailing_space_for_an_empty_title() {
    assert_eq!(copy_label(7, ""), "Copy #7");
    assert_eq!(copy_label(7, "   "), "Copy #7");
    assert_eq!(card_reference(7, ""), "#7");
    assert_eq!(card_reference(7, "  "), "#7");
    assert_eq!(copy_label(7, "Fix it"), "Copy #7 Fix it");
    assert_eq!(card_reference(7, "Fix it"), "#7 Fix it");
}

// ---- R66: the new-card modal's due field ----

#[gpui::test]
fn typed_due_reset_keeps_form_due(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    visual.simulate_input("Dated");
    let today = chrono::Local::now().date_naive();
    let form_due = |visual: &mut VisualTestContext| {
        f.window
            .read_with(visual, |view, cx| {
                let form = view.new_card.as_ref().unwrap();
                (
                    form.due,
                    form.typing_date,
                    form.date.read(cx).content.to_string(),
                )
            })
            .unwrap()
    };
    pick(visual, "new-card-due", "menu-action-due-tomorrow");
    let tomorrow = today.succ_opt().unwrap();
    assert_eq!(form_due(visual), (Some(tomorrow), false, String::new()));
    // A half-typed date, then a quick pick: the text goes, the pick stays.
    pick(visual, "new-card-due", "menu-action-due-pick");
    visual.simulate_input("2026-1");
    assert_eq!(form_due(visual).0, Some(tomorrow), "typing cleared the due");
    pick(visual, "new-card-due", "menu-action-due-today");
    assert_eq!(form_due(visual), (Some(today), false, String::new()));
    // The typed field opened again but left empty keeps the picked due.
    pick(visual, "new-card-due", "menu-action-due-pick");
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    assert!(!modal_open(&f, visual));
    let card = &cards_titled(&f, "Dated")[0];
    assert_eq!(
        extras_of(&f, &card.id).due_at,
        super::super::style::day_start(today)
    );
}

#[gpui::test]
fn tab_order_includes_the_revealed_due_field(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    pick(visual, "new-card-due", "menu-action-due-pick");
    let date_focused = |visual: &mut VisualTestContext| {
        f.window
            .update(visual, |view, window, cx| {
                let form = view.new_card.as_ref().unwrap();
                form.date.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    };
    assert!(date_focused(visual), "the revealed field takes focus");
    // Shift-Tab walks back through the pills; Tab comes forward to it.
    tab(visual, -1);
    assert_eq!(new_card_focused(&f, visual), Some("projects"));
    tab(visual, 1);
    assert!(date_focused(visual), "Tab skips the date field");
    tab(visual, 1);
    assert_eq!(new_card_focused(&f, visual), Some("more"));
    tab(visual, -1);
    assert!(date_focused(visual));
    // From the title the whole order holds the date between the pills and
    // Create more.
    f.window
        .update(visual, |view, window, cx| {
            let title = view.new_card.as_ref().unwrap().title.focus_handle(cx);
            window.focus(&title, cx);
        })
        .unwrap();
    let mut seen = Vec::new();
    for _ in 0..9 {
        tab(visual, 1);
        seen.push(if date_focused(visual) {
            "date"
        } else {
            new_card_focused(&f, visual).unwrap_or("elsewhere")
        });
    }
    assert_eq!(
        seen,
        [
            "description",
            "column",
            "priority",
            "due",
            "labels",
            "projects",
            "date",
            "more",
            "create"
        ]
    );
}

// ---- Controller addition: the C6-8 body-height cache ----

#[gpui::test]
fn column_body_height_follows_cards_and_density_and_prunes_deleted_cards(cx: &mut TestAppContext) {
    let f = seeded(cx, &["One", "Two"]);
    let backlog = column_id(&f, cx, "Backlog");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    let height = |visual: &mut VisualTestContext| {
        f.window
            .read_with(visual, |view, _| view.column_body_height(&backlog))
            .unwrap()
    };
    let two = height(visual);
    // A third card makes the column taller.
    let third = Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Three", "")
        .unwrap();
    reload(&f, visual);
    visual.refresh().unwrap();
    let three = height(visual);
    assert!(three > two, "{three:?} after adding, {two:?} before");
    // Compact cards make it shorter, comfortable ones bring it back.
    patch_display(&f, r#"{"density":"compact"}"#);
    reload(&f, visual);
    visual.refresh().unwrap();
    let compact = height(visual);
    assert!(
        compact < three,
        "{compact:?} compact, {three:?} comfortable"
    );
    patch_display(&f, r#"{"density":"comfortable"}"#);
    reload(&f, visual);
    visual.refresh().unwrap();
    assert!(height(visual) > compact);
    // A deleted card leaves no painted height behind, and the column shrinks.
    let painted = |visual: &mut VisualTestContext, id: &str| {
        f.window
            .read_with(visual, |view, _| {
                view.item_heights.borrow().contains_key(id)
            })
            .unwrap()
    };
    assert!(painted(visual, &third.id));
    let before = height(visual);
    Service::open(&f.db)
        .unwrap()
        .card_delete(&third.id)
        .unwrap();
    reload(&f, visual);
    visual.refresh().unwrap();
    assert!(
        !painted(visual, &third.id),
        "the deleted card's height stayed"
    );
    assert!(height(visual) < before);
}
