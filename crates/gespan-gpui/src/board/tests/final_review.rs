//! Regression tests for the final review of the native board (#174): a
//! ticket card lands on the open board, a drawer whose card went away
//! elsewhere closes, a held delete doesn't block a settings column delete,
//! the selection drops cards in collapsed columns and groups, a live turn
//! never re-queries under a drag, and the lane list rebuilds only on change.
use super::link_run::{add_link, install_turns, new_session, started};
use super::*;
use crate::tickets::fakes::FakeTickets;
use gespan_core::tickets::Tracker;

fn poll(visual: &mut VisualTestContext) {
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn second_board(f: &Fixture) -> gespan_core::board::Board {
    Service::open(&f.db).unwrap().create("Second").unwrap()
}

// ---- N2: "New card from ticket…" on the open board ----

#[gpui::test]
fn new_card_from_ticket_lands_on_the_open_board_not_the_default_one(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let default_id = open_id(&f, cx);
    let second = second_board(&f);
    let fake: Arc<FakeTickets> = with_tickets(&f, cx, &[Tracker::Linear]);
    f.window
        .update(cx, |view, _, cx| {
            assert!(view.open_board(second.id.clone(), cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(open_id(&f, cx), second.id);
    let todo = detail(&f, cx)
        .columns
        .iter()
        .find(|column| column.role.as_deref() == Some("todo"))
        .or(detail(&f, cx).columns.first())
        .unwrap()
        .id
        .clone();
    let id = f
        .window
        .update(cx, |view, _, cx| {
            view.create_card_from_ticket(ticket("ENG-7", None), cx)
        })
        .unwrap()
        .expect("the card is created");
    cx.run_until_parked();
    let board = detail(&f, cx);
    let card = board
        .cards
        .iter()
        .find(|card| card.id == id)
        .expect("the card is on the open board");
    assert_eq!(card.board_id, second.id);
    assert_eq!(card.column_id, todo, "the open board's To do step takes it");
    // The ticket read that follows found the card's ticket on this board.
    assert_eq!(fake.fetch_count(), 1);
    assert_eq!(
        card.title, "Fix login",
        "the fetched title replaces the key"
    );
    let mut service = Service::open(&f.db).unwrap();
    assert!(
        service.get(&default_id).unwrap().cards.is_empty(),
        "nothing lands on the default board"
    );
    assert_eq!(service.card_ticket(&id).unwrap().unwrap().key, "ENG-7");
}

// ---- C6-j: a ticket title never reaches the "Run with…" prompt ----

#[gpui::test]
fn new_card_from_ticket_keeps_its_title_out_of_the_prompt_through_rename_and_duplicate(
    cx: &mut TestAppContext,
) {
    let f = with_board(cx);
    let _fake: Arc<FakeTickets> = with_tickets(&f, cx, &[Tracker::Linear]);
    let id = f
        .window
        .update(cx, |view, _, cx| {
            view.create_card_from_ticket(ticket("ENG-7", None), cx)
        })
        .unwrap()
        .expect("the card is created");
    cx.run_until_parked();
    let mut service = Service::open(&f.db).unwrap();
    assert_eq!(service.card(&id).unwrap().title, "Fix login");
    assert!(!service.run_prompt(&id).unwrap().contains("Fix login"));
    // The ticket is renamed: the snapshot no longer matches the card title.
    let mut renamed = service.card_ticket(&id).unwrap().unwrap();
    renamed.title = Some("Something else".into());
    service.set_card_ticket(&id, Some(&renamed)).unwrap();
    assert!(!service.run_prompt(&id).unwrap().contains("Fix login"));
    // The duplicate drops the ticket link but keeps the copied title out.
    let copy = service.duplicate_card(&id).unwrap();
    assert!(!service.run_prompt(&copy.id).unwrap().contains("Fix login"));
    // A title the user edits is theirs again.
    service.card_update(&id, "Fix login on Safari", "").unwrap();
    assert!(
        service
            .run_prompt(&id)
            .unwrap()
            .contains("Fix login on Safari")
    );
}

// ---- N4: a drawer whose card went away elsewhere ----

#[gpui::test]
fn a_card_deleted_elsewhere_while_typing_closes_the_drawer_and_keeps_the_text(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Task", "Other"]);
    let task = card_id(&f, cx, "Task");
    let second = second_board(&f);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-description", "Half-written notes");
    // Another connection hard-deletes the card; the poll sees it.
    Service::open(&f.db).unwrap().card_delete(&task).unwrap();
    poll(visual);
    f.window
        .read_with(visual, |view, _| {
            assert!(view.card_detail.is_none(), "the drawer closed");
            let error = view.error.clone().unwrap_or_default();
            assert!(
                error.contains("Half-written notes"),
                "the unsaved text shows: {error}"
            );
            assert_eq!(
                view.orphaned_text.as_ref().map(|(_, text)| text.as_str()),
                Some("Description: Half-written notes")
            );
        })
        .unwrap();
    // The text can be copied.
    click(visual, "board-error-copy");
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("Description: Half-written notes")
    );
    // Esc works again: another card's drawer opens and Escape closes it.
    click(visual, "board-card-2");
    assert!(drawer(&f, visual, |view, _| view.card_detail.is_some()));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(drawer(&f, visual, |view, _| view.card_detail.is_none()));
    // Settings open.
    f.window
        .update(visual, |view, window, cx| {
            view.open_settings(window, cx);
            assert!(view.settings.is_some(), "settings open");
            view.close_settings(cx);
        })
        .unwrap();
    // A board switch works.
    f.window
        .update(visual, |view, _, cx| {
            assert!(view.open_board(second.id.clone(), cx), "the switch happens")
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(open_id(&f, visual), second.id);
}

#[gpui::test]
fn escape_on_a_drawer_whose_card_was_deleted_before_the_poll_closes_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let task = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-description", "Unsaved words");
    Service::open(&f.db).unwrap().card_delete(&task).unwrap();
    // No poll yet: the save on Escape hits "Card not found".
    f.window
        .update(visual, |view, _, cx| {
            assert!(view.close_card_detail(cx));
            assert!(view.card_detail.is_none(), "Card not found closes it");
            assert!(
                view.error
                    .as_deref()
                    .is_some_and(|error| error.contains("Unsaved words"))
            );
        })
        .unwrap();
    // A board switch is not blocked either.
    let second = second_board(&f);
    f.window
        .update(visual, |view, _, cx| {
            assert!(view.open_board(second.id, cx))
        })
        .unwrap();
}

#[gpui::test]
fn a_full_page_card_archived_elsewhere_gives_the_board_back(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let task = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-full-page");
    assert!(
        !shown(visual, "board-columns"),
        "the page replaces the columns"
    );
    Service::open(&f.db).unwrap().archive(&task).unwrap();
    poll(visual);
    assert!(shown(visual, "board-columns"), "the board shows again");
    f.window
        .read_with(visual, |view, _| {
            assert!(view.card_detail.is_none());
            assert_eq!(view.error, None, "nothing was unsaved, so no error");
        })
        .unwrap();
    // A card opened next opens in the drawer, not on a blank full page.
    let other = Service::open(&f.db)
        .unwrap()
        .card_create(&column_id(&f, visual, "Backlog"), "Next", "")
        .unwrap();
    poll(visual);
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(other.id.clone(), window, cx)
        })
        .unwrap();
    assert!(shown(visual, "board-columns"));
    assert!(shown(visual, "board-drawer"));
}

// ---- N5: Settings → Columns delete after a held delete ----

#[gpui::test]
fn settings_column_delete_after_a_held_delete_is_not_refused(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Only"]);
    let only = card_id(&f, cx, "Only");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    assert!(toast(&f, visual).is_some(), "the delete is held");
    open_settings(visual, "columns");
    click(visual, "settings-col-0-delete");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.settings.as_ref().and_then(|page| page.error.clone()),
                None,
                "no refusal on a stale archived count"
            );
        })
        .unwrap();
    assert_eq!(detail(&f, visual).columns.len(), 3, "the column is gone");
    assert!(!card_exists(&f, &only), "the held delete was carried out");
}

#[gpui::test]
fn settings_column_delete_still_refuses_a_column_with_other_archived_cards(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Held", "Kept"]);
    let kept = card_id(&f, cx, "Kept");
    Service::open(&f.db).unwrap().archive(&kept).unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    open_settings(visual, "columns");
    click(visual, "settings-col-0-delete");
    f.window
        .read_with(visual, |view, _| {
            let error = view.settings.as_ref().and_then(|page| page.error.clone());
            assert!(
                error
                    .as_deref()
                    .is_some_and(|e| e.contains("1 archived card")),
                "the one really archived card still blocks it: {error:?}"
            );
        })
        .unwrap();
    assert_eq!(detail(&f, visual).columns.len(), 4);
}

// ---- N6: the selection drops cards in collapsed columns and groups ----

fn selected(f: &Fixture, cx: &mut impl gpui::AppContext) -> Vec<String> {
    let mut ids: Vec<String> = drawer(f, cx, |view, _| {
        view.selection.ids.iter().cloned().collect()
    });
    ids.sort();
    ids
}

#[gpui::test]
fn collapsing_a_column_drops_its_cards_from_the_selection(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let (a, b) = (card_id(&f, cx, "A"), card_id(&f, cx, "B"));
    let progress = column_id(&f, cx, "In Progress");
    let backlog = column_id(&f, cx, "Backlog");
    Service::open(&f.db)
        .unwrap()
        .card_move(&b, &progress, 0, "user")
        .unwrap();
    reload(&f, cx);
    f.window
        .update(cx, |view, _, cx| {
            view.toggle_select(&a, cx);
            view.toggle_select(&b, cx);
        })
        .unwrap();
    assert_eq!(selected(&f, cx).len(), 2);
    f.window
        .update(cx, |view, _, cx| {
            view.set_collapsed(backlog.clone(), true, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        selected(&f, cx),
        std::slice::from_ref(&b),
        "A sits in a collapsed column"
    );
    // Expanding it again does not bring A back.
    f.window
        .update(cx, |view, _, cx| view.set_collapsed(backlog, false, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(&f, cx), [b]);
}

#[gpui::test]
fn collapsing_a_list_group_drops_its_cards_from_the_selection(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let (a, b) = (card_id(&f, cx, "A"), card_id(&f, cx, "B"));
    let progress = column_id(&f, cx, "In Progress");
    let backlog = column_id(&f, cx, "Backlog");
    Service::open(&f.db)
        .unwrap()
        .card_move(&b, &progress, 0, "user")
        .unwrap();
    reload(&f, cx);
    f.window
        .update(cx, |view, _, cx| {
            view.set_layout(list::Layout::List, cx);
            view.toggle_select(&a, cx);
            view.toggle_select(&b, cx);
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // The first group is Backlog (status grouping).
    click(visual, "board-list-group-0");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.layout_spec().collapsed, [format!("status:{backlog}")]);
        })
        .unwrap();
    assert_eq!(
        selected(&f, visual),
        std::slice::from_ref(&b),
        "A sits in a collapsed group"
    );
    // A reload keeps it out.
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    assert_eq!(selected(&f, visual), [b]);
}

// ---- N7: a live turn never re-queries under a drag ----

#[gpui::test]
fn a_turn_starting_during_a_lift_or_drag_waits_for_the_drop_to_requery(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Busy", "Later"]);
    install_turns(&f, cx);
    let (busy, busy_key) = new_session(&f);
    let (later, later_key) = new_session(&f);
    let (busy_card, later_card) = (card_id(&f, cx, "Busy"), card_id(&f, cx, "Later"));
    add_link(&f, &busy_card, "session", &busy_key);
    add_link(&f, &later_card, "session", &later_key);
    reload(&f, cx);
    started(&f, cx, busy, None);
    // Show only running cards: Busy now.
    f.window
        .update(cx, |view, _, cx| {
            view.spec.rows.push(gespan_core::board::extras::FilterRow {
                field: "linked".into(),
                op: gespan_core::board::extras::Op::Is,
                values: vec!["running".into()],
            });
            view.requery(cx);
        })
        .unwrap();
    cx.run_until_parked();
    let backlog = column_id(&f, cx, "Backlog");
    // Every shown card, whatever column a turn's automation put it in.
    let ids = |f: &Fixture, cx: &mut TestAppContext| {
        let mut ids: Vec<String> = drawer(f, cx, |view, _| {
            view.column_ids.values().flatten().cloned().collect()
        });
        ids.sort();
        ids
    };
    assert_eq!(ids(&f, cx), std::slice::from_ref(&busy_card));
    // A keyboard lift of Busy.
    {
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        visual.update(|window, _| window.activate_window());
        focus_card(&f, visual, &busy_card);
        visual.simulate_keystrokes("space");
        visual.run_until_parked();
    }
    assert!(drawer(&f, cx, |view, _| view.kb_drag.is_some()), "lifted");
    f.window.update(cx, |view, _, _| view.reads.take()).unwrap();
    // Later's turn starts mid-lift: no re-query moves cards under the lift.
    started(&f, cx, later, None);
    let reads = f.window.update(cx, |view, _, _| view.reads.take()).unwrap();
    assert!(
        !reads.contains(&"query") && !reads.contains(&"running_cards"),
        "no query during the lift: {reads:?}"
    );
    assert_eq!(
        ids(&f, cx),
        std::slice::from_ref(&busy_card),
        "the column holds still"
    );
    assert!(drawer(&f, cx, |view, _| view.pending_reload));
    assert!(
        drawer(&f, cx, |view, _| view.kb_drag.is_some()),
        "still lifted"
    );
    // Escape cancels the lift; the deferred reload runs the query.
    {
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
    }
    assert!(drawer(&f, cx, |view, _| view.kb_drag.is_none()));
    let mut both = vec![busy_card.clone(), later_card.clone()];
    both.sort();
    let mut shown = ids(&f, cx);
    shown.sort();
    assert_eq!(shown, both, "Later shows once the lift ends");
    // A mouse drag defers the same way.
    let (third, third_key) = new_session(&f);
    let third_card = Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Third", "")
        .unwrap()
        .id;
    add_link(&f, &third_card, "session", &third_key);
    reload(&f, cx);
    f.window
        .update(cx, |view, _, cx| view.set_dragging(true, cx))
        .unwrap();
    started(&f, cx, third, None);
    assert!(drawer(&f, cx, |view, _| view.pending_reload));
    assert!(
        !ids(&f, cx).contains(&third_card),
        "no re-query during the drag"
    );
    f.window
        .update(cx, |view, _, cx| view.set_dragging(false, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(ids(&f, cx).contains(&third_card), "the drop re-queries");
}

// ---- N8: the lane list rebuilds only when its inputs change ----

#[gpui::test]
fn the_lane_list_rebuilds_only_when_the_cards_change(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    lanes_by(&f, "priority");
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    let before = drawer(&f, visual, |view, _| view.lane_rebuilds.get());
    assert!(before >= 1, "the lanes were built");
    for _ in 0..5 {
        f.window.update(visual, |_, _, cx| cx.notify()).unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
    assert_eq!(
        drawer(&f, visual, |view, _| view.lane_rebuilds.get()),
        before,
        "renders with nothing changed rebuild nothing"
    );
    // A change elsewhere: the next render rebuilds and the card shows.
    let a = card_id(&f, visual, "A");
    Service::open(&f.db).unwrap().set_priority(&a, 3).unwrap();
    poll(visual);
    let after = drawer(&f, visual, |view, _| view.lane_rebuilds.get());
    assert!(after > before, "a reload rebuilds the lanes");
    assert!(
        drawer(&f, visual, |view, _| {
            view.lane_cell_ids
                .iter()
                .any(|((lane, _), ids)| lane == "p3" && ids.contains(&a))
        }),
        "A moved to the Urgent lane"
    );
    // Switching the lane mode rebuilds too.
    lanes_by(&f, "label");
    poll(visual);
    assert!(drawer(&f, visual, |view, _| view.lane_rebuilds.get()) > after);
}
