//! Keyboard focus, keyboard drags and their announcements in windowed board
//! columns (C6-6): focus and lifts that move onto cards outside a column's
//! window land on the right card, and every count comes from the data.
use super::scale::{
    card_bounds, column_at, list_ids, list_top, number_of, reload_view, scroll_list_to,
    seeded_board, stored_order, wheel,
};
use super::*;

fn press(visual: &mut VisualTestContext, keys: &str) {
    visual.simulate_keystrokes(keys);
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn press_n(visual: &mut VisualTestContext, key: &str, times: usize) {
    for _ in 0..times {
        visual.simulate_keystrokes(key);
    }
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn announcement(f: &Fixture, visual: &mut VisualTestContext) -> Option<String> {
    f.window
        .read_with(visual, |view, _| view.board_announcement())
        .unwrap()
}

fn activate(visual: &mut VisualTestContext) {
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

/// The card is painted and lies fully inside its column's view.
fn in_view(f: &Fixture, visual: &mut VisualTestContext, column: &str, number: i64) -> bool {
    visual.refresh().unwrap();
    let Some(card) = visual.debug_bounds(selector(format!("board-card-{number}"))) else {
        return false;
    };
    let view = f
        .window
        .read_with(visual, |view, _| {
            view.column_lists[column].viewport_bounds()
        })
        .unwrap();
    card.top() >= view.top() && card.bottom() <= view.bottom()
}

fn set_search(f: &Fixture, visual: &mut VisualTestContext, text: &str) {
    f.window
        .update(visual, |view, _, cx| {
            view.search
                .update(cx, |input, cx| input.set_text(text.into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn rename(f: &Fixture, card: &str, title: &str) {
    Service::open(&f.db)
        .unwrap()
        .card_update(card, title, "")
        .unwrap();
}

#[gpui::test]
fn arrow_down_past_the_window_scrolls_and_focuses_the_next_card(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[0]);
    assert!(
        !in_view(&f, visual, &column, 61),
        "card 61 starts far below"
    );
    press_n(visual, "down", 60);
    assert_eq!(focused_card(&f, visual), Some(ids[60].clone()));
    assert!(
        list_top(&f, visual, &column).item_ix > 0,
        "the column scrolled"
    );
    assert!(
        in_view(&f, visual, &column, 61),
        "the focused card is in view"
    );
    // And back up past the top edge of the window.
    press_n(visual, "up", 60);
    assert_eq!(focused_card(&f, visual), Some(ids[0].clone()));
    assert!(in_view(&f, visual, &column, 1));
}

#[gpui::test]
fn focus_survives_scrolling_the_focused_card_away_and_arrow_resumes_after_it(
    cx: &mut TestAppContext,
) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[5]);
    let body = center(visual, "board-column-body-0");
    wheel(visual, body, -6000.);
    assert!(list_top(&f, visual, &column).item_ix > 40);
    assert_eq!(focused_card(&f, visual), Some(ids[5].clone()));
    // Arrow from far away: the next card scrolls back into view and focuses.
    press(visual, "down");
    assert_eq!(focused_card(&f, visual), Some(ids[6].clone()));
    assert!(in_view(&f, visual, &column, 7));
    // Away and back with the wheel: focus is still on the card.
    wheel(visual, body, -6000.);
    wheel(visual, body, 12000.);
    assert_eq!(focused_card(&f, visual), Some(ids[6].clone()));
    press(visual, "down");
    assert_eq!(focused_card(&f, visual), Some(ids[7].clone()));
}

#[gpui::test]
fn focus_survives_window_deactivation_while_scrolled_away(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    activate(visual);
    focus_card(&f, visual, &ids[5]);
    visual.deactivate_window();
    visual.run_until_parked();
    let body = center(visual, "board-column-body-0");
    wheel(visual, body, -6000.);
    assert!(list_top(&f, visual, &column).item_ix > 40);
    activate(visual);
    assert_eq!(focused_card(&f, visual), Some(ids[5].clone()));
    press(visual, "down");
    assert_eq!(focused_card(&f, visual), Some(ids[6].clone()));
}

#[gpui::test]
fn focus_survives_a_board_changed_reload_while_scrolled_away(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[5]);
    scroll_list_to(&f, visual, &column, 500);
    rename(&f, &ids[500], "Renamed in the other app");
    reload_view(&f, visual);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(list_top(&f, visual, &column).item_ix, 500);
    assert_eq!(focused_card(&f, visual), Some(ids[5].clone()));
    press(visual, "down");
    assert_eq!(focused_card(&f, visual), Some(ids[6].clone()));
}

#[gpui::test]
fn a_focused_card_hidden_by_a_filter_moves_focus_to_the_nearest_visible_card(
    cx: &mut TestAppContext,
) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    set_search(&f, visual, "card");
    assert_eq!(list_ids(&f, visual, &column).len(), 1000);
    scroll_list_to(&f, visual, &column, 500);
    focus_card(&f, visual, &ids[499]);
    visual.refresh().unwrap();
    // The other app renames it out of the results: the next card takes focus.
    rename(&f, &ids[499], "Gone");
    reload_view(&f, visual);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(list_ids(&f, visual, &column).len(), 999);
    assert_eq!(focused_card(&f, visual), Some(ids[500].clone()));
    assert!(in_view(&f, visual, &column, 501));
    // The last card has no next one: the previous card takes focus.
    focus_card(&f, visual, &ids[999]);
    visual.refresh().unwrap();
    rename(&f, &ids[999], "Gone too");
    reload_view(&f, visual);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(focused_card(&f, visual), Some(ids[998].clone()));
    // Focus the user put elsewhere stays there.
    f.window
        .update(visual, |view, window, cx| {
            window.focus(&view.search.focus_handle(cx), cx)
        })
        .unwrap();
    visual.refresh().unwrap();
    rename(&f, &ids[998], "Gone as well");
    reload_view(&f, visual);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(focused_card(&f, visual), None);
    let in_search = f
        .window
        .update(visual, |view, window, cx| {
            view.search.focus_handle(cx).is_focused(window)
        })
        .unwrap();
    assert!(in_search, "the search box keeps focus");
}

#[gpui::test]
fn a_focused_card_hidden_with_no_neighbour_left_moves_focus_to_the_columns_first_card(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Card one", "Card drop", "Card two"]);
    let (backlog, in_progress) = {
        let detail = detail(&f, cx);
        (detail.columns[0].id.clone(), detail.columns[1].id.clone())
    };
    let drop = card_id(&f, cx, "Card drop");
    let (one, two) = (card_id(&f, cx, "Card one"), card_id(&f, cx, "Card two"));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    set_search(&f, visual, "card");
    focus_card(&f, visual, &drop);
    visual.refresh().unwrap();
    // The other app moves both neighbours away, adds a card and renames the
    // focused one out of the search.
    let fresh = {
        let mut service = Service::open(&f.db).unwrap();
        service.card_move(&one, &in_progress, 0, "user").unwrap();
        service.card_move(&two, &in_progress, 1, "user").unwrap();
        let fresh = service.card_create(&backlog, "Card new", "").unwrap();
        service.card_update(&drop, "Gone", "").unwrap();
        fresh.id
    };
    reload_view(&f, visual);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(list_ids(&f, visual, &backlog), std::slice::from_ref(&fresh));
    assert_eq!(focused_card(&f, visual), Some(fresh));
}

#[gpui::test]
fn lift_move_50_down_and_drop_lands_at_index_50(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[0]);
    press(visual, "space");
    press_n(visual, "down", 50);
    // The drop line sits above card 52, in view.
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.drop_target, Some((column.clone(), 51)));
        })
        .unwrap();
    assert!(in_view(&f, visual, &column, 52));
    press(visual, "space");
    let order = stored_order(&f, &column);
    assert_eq!(order.len(), 1000);
    assert_eq!(order[50], ids[0]);
    assert_eq!(
        order[48..52],
        [
            ids[49].clone(),
            ids[50].clone(),
            ids[0].clone(),
            ids[51].clone()
        ]
    );
    assert_eq!(order[0], ids[1]);
    // Up from deep in the column lands at the real index too.
    focus_card(&f, visual, &ids[700]);
    press(visual, "space");
    press_n(visual, "up", 60);
    press(visual, "space");
    let order = stored_order(&f, &column);
    assert_eq!(order[640], ids[700]);
}

#[gpui::test]
fn keyboard_drag_announces_position_of_the_full_count(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert_eq!(announcement(&f, visual), None);
    focus_card(&f, visual, &ids[0]);
    press(visual, "space");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Picked up card #1, Card 1. Backlog, position 1 of 1000.")
    );
    assert!(visual.debug_bounds("board-drag-announce").is_some());
    press(visual, "down");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Card #1 is over Backlog, position 2 of 1000.")
    );
    press_n(visual, "down", 49);
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Card #1 is over Backlog, position 51 of 1000.")
    );
    press(visual, "space");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Dropped card #1 in Backlog, position 51 of 1000.")
    );
    assert_eq!(stored_order(&f, &column)[50], ids[0]);
    // A cancelled lift says where the card still is.
    focus_card(&f, visual, &ids[0]);
    press(visual, "space down down escape");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Moving card #1 was cancelled. It is back in Backlog, position 51 of 1000.")
    );
    // The column's label counts every visible card, mounted or not.
    let label = f
        .window
        .read_with(visual, |view, _| view.column_label(&column))
        .unwrap();
    assert_eq!(label, "Backlog, 1000 cards");
}

#[gpui::test]
fn keyboard_drag_into_another_column_counts_the_card_it_brings(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    press(visual, "space down");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Card #1 is over Backlog, position 2 of 2.")
    );
    press(visual, "right");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Card #1 is over In Progress, position 1 of 1.")
    );
    press(visual, "enter");
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Dropped card #1 in In Progress, position 1 of 1.")
    );
    assert_eq!(titles_in(&f, visual, "In Progress"), ["A"]);
}

#[gpui::test]
fn cards_and_header_buttons_are_tab_stops(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    f.window
        .update(visual, |view, window, cx| {
            window.focus(&view.search.focus_handle(cx), cx)
        })
        .unwrap();
    visual.refresh().unwrap();
    // Tab forward from the search field until the first card has focus.
    let mut header = std::collections::HashSet::new();
    let mut reached = None;
    for step in 1..=40 {
        tab(visual, 1);
        let (on_header, card) = f
            .window
            .update(visual, |view, window, _| {
                let on_header = [
                    ("filter", &view.filter_focus),
                    ("sort", &view.sort_focus),
                    ("new task", &view.new_task_focus),
                    ("switcher", &view.switcher_focus),
                ]
                .into_iter()
                .find(|(_, handle)| handle.is_focused(window))
                .map(|(name, _)| name);
                let card = view
                    .card_focus
                    .iter()
                    .find(|(_, handle)| handle.is_focused(window))
                    .map(|(id, _)| id.clone());
                (on_header, card)
            })
            .unwrap();
        header.extend(on_header);
        if let Some(card) = card {
            reached = Some((step, card));
            break;
        }
    }
    let (_, card) = reached.expect("Tab reaches a card");
    assert_eq!(card, ids[0], "the first card is the first card Tab reaches");
    // The next Tab goes on to the next card.
    tab(visual, 1);
    assert_eq!(focused_card(&f, visual), Some(ids[1].clone()));
    // Shift-Tab comes back.
    tab(visual, -1);
    assert_eq!(focused_card(&f, visual), Some(ids[0].clone()));
    // The header buttons after the search field are stops on the way.
    assert!(header.contains("filter"), "{header:?}");
    assert!(header.contains("new task"), "{header:?}");
    let handles_are_stops = f
        .window
        .read_with(visual, |view, _| {
            view.card_focus.values().all(|handle| handle.tab_stop)
                && view.filter_focus.tab_stop
                && view.sort_focus.tab_stop
                && view.new_task_focus.tab_stop
                && view.switcher_focus.tab_stop
        })
        .unwrap();
    assert!(handles_are_stops);
}

#[gpui::test]
fn shift_range_from_an_unmounted_anchor_selects_the_display_slice(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    let anchor = card_bounds(visual, 3).center();
    visual.simulate_click(anchor, gpui::Modifiers::secondary_key());
    visual.run_until_parked();
    // Focus leaves the card, so nothing keeps it mounted.
    f.window
        .update(visual, |view, window, cx| {
            window.focus(&view.search.focus_handle(cx), cx)
        })
        .unwrap();
    scroll_list_to(&f, visual, &column, 600);
    assert!(
        visual.debug_bounds("board-card-3").is_none(),
        "the anchor is no longer mounted"
    );
    let far = number_of(&f, visual, &ids[600]);
    let target = card_bounds(visual, far).center();
    visual.simulate_click(target, gpui::Modifiers::shift());
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.selected_count(), 599);
            assert!(ids[2..=600].iter().all(|id| view.is_selected(id)));
            assert!(!view.is_selected(&ids[1]) && !view.is_selected(&ids[601]));
        })
        .unwrap();
}

#[gpui::test]
fn arrow_moves_keep_the_lift_in_an_active_window(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    activate(visual);
    focus_card(&f, visual, &ids[0]);
    press(visual, "space");
    press_n(visual, "down", 50);
    f.window
        .read_with(visual, |view, _| {
            let drag = view.kb_drag.as_ref().expect("still lifted");
            assert_eq!(drag.card_id, ids[0]);
            assert_eq!(drag.slot, 51);
        })
        .unwrap();
    // Arrows moved the drop line, not focus.
    assert_eq!(focused_card(&f, visual), Some(ids[0].clone()));
    press_n(visual, "up", 20);
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.kb_drag.as_ref().map(|drag| drag.slot), Some(31));
        })
        .unwrap();
    assert_eq!(focused_card(&f, visual), Some(ids[0].clone()));
    press(visual, "space");
    assert_eq!(stored_order(&f, &column)[30], ids[0]);
}

#[gpui::test]
fn arrow_keys_do_nothing_while_typing(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    f.window
        .update(visual, |view, window, cx| {
            window.focus(&view.search.focus_handle(cx), cx)
        })
        .unwrap();
    press(visual, "down down right");
    assert_eq!(focused_card(&f, visual), None);
    let in_search = f
        .window
        .update(visual, |view, window, cx| {
            view.search.focus_handle(cx).is_focused(window)
        })
        .unwrap();
    assert!(in_search);
}

#[gpui::test]
fn click_elsewhere_in_an_active_window_cancels_the_lift(cx: &mut TestAppContext) {
    let f = seeded_board(cx, false);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let (seen, _subscription) = {
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        events(&f, visual)
    };
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    activate(visual);
    // A drawer open on one card and a selection on another.
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(ids[3].clone(), window, cx);
            view.toggle_select(&ids[2], cx);
        })
        .unwrap();
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[0]);
    press(visual, "space down");
    assert!(
        f.window
            .read_with(visual, |view, _| view.kb_drag.is_some())
            .unwrap()
    );
    // A click on the header of another column, which takes no focus.
    click(visual, "board-column-header-1");
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
            assert_eq!(
                view.card_detail.as_ref().map(|d| d.card_id.clone()),
                Some(ids[3].clone()),
                "the drawer stays open"
            );
            assert_eq!(view.selected_count(), 1, "the selection stays");
        })
        .unwrap();
    assert!(seen.borrow().is_empty(), "nothing closed the board");
    assert_eq!(moved_rows(&f, visual), 0);
    assert_eq!(
        announcement(&f, visual).as_deref(),
        Some("Moving card #1 was cancelled. It is back in Backlog, position 1 of 250.")
    );
    // A later Space lifts afresh rather than dropping a ghost.
    focus_card(&f, visual, &ids[0]);
    press(visual, "space space");
    assert_eq!(moved_rows(&f, visual), 0);
}

#[gpui::test]
fn space_drop_never_opens_the_drawer(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    activate(visual);
    focus_card(&f, visual, &ids[0]);
    press(visual, "space");
    press_n(visual, "down", 50);
    press(visual, "space");
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none());
            assert!(view.card_detail.is_none(), "no drawer after a Space drop");
            assert!(view.card_menu.is_none());
        })
        .unwrap();
    assert_eq!(stored_order(&f, &column)[50], ids[0]);
    // Dropping it back where it is opens nothing either.
    focus_card(&f, visual, &ids[0]);
    press(visual, "space space");
    f.window
        .read_with(visual, |view, _| {
            assert!(view.card_detail.is_none() && view.card_menu.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn shortcuts_act_on_a_card_focused_by_arrow_keys(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[0]);
    press_n(visual, "down", 50);
    press(visual, "e");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.card_detail.as_ref().map(|d| d.card_id.clone()),
                Some(ids[50].clone())
            );
        })
        .unwrap();
}

#[gpui::test]
fn the_card_whose_drawer_is_open_stays_mounted(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(ids[0].clone(), window, cx);
            window.focus(&view.drawer_focus, cx);
        })
        .unwrap();
    visual.refresh().unwrap();
    assert_eq!(focused_card(&f, visual), None, "focus is in the drawer");
    scroll_list_to(&f, visual, &column, 600);
    assert!(list_top(&f, visual, &column).item_ix >= 590);
    assert!(
        visual.debug_bounds("board-card-1").is_some(),
        "the drawer's card is still mounted"
    );
    assert!(visual.debug_bounds("board-card-2").is_none());
    // Closed, it goes like any other card scrolled away.
    f.window
        .update(visual, |view, _, cx| {
            view.close_card_detail(cx);
            cx.notify();
        })
        .unwrap();
    visual.refresh().unwrap();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-card-1").is_none());
}
