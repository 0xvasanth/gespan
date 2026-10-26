//! Windowed swimlanes at 1,000 cards (C6-7): one list of lane rows (a
//! header, then bands of 4 cards per cell), so only the rows in or near view
//! mount. Drops, keyboard drags, arrow focus, Shift ranges and the
//! announcements all count from the data, never from the mounted rows.
use super::scale::{reload_view, seeded_board};
use super::*;
use crate::board::swimlanes::{LANE_BAND, LaneRow};
use std::collections::HashSet;

/// The most cards one expanded column may render (the C6 budget).
const COLUMN_BUDGET: usize = 60;

fn header(lane: &str) -> LaneRow {
    LaneRow::Header { lane: lane.into() }
}

fn band(lane: &str, band: usize) -> LaneRow {
    LaneRow::Band {
        lane: lane.into(),
        band,
    }
}

/// The seeded 1,000-card board (4 columns) in priority lanes. Card `n` sits
/// in column `(n - 1) % 4` with priority `n % 5`, so every cell holds 50.
fn seeded_lanes(cx: &mut TestAppContext) -> Fixture {
    let f = seeded_board(cx, false);
    lanes_by(&f, "priority");
    reload_view(&f, cx);
    cx.run_until_parked();
    f
}

fn press(visual: &mut VisualTestContext, keys: &str) {
    visual.simulate_keystrokes(keys);
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn press_n(visual: &mut VisualTestContext, key: &str, times: usize) {
    for _ in 0..times {
        visual.simulate_keystrokes(key);
        visual.run_until_parked();
    }
    visual.refresh().unwrap();
}

/// The frames the app draws after a far reveal, which keeps checking for a
/// few frames until the card is fully in view.
fn frames(f: &Fixture, visual: &mut VisualTestContext) {
    for _ in 0..6 {
        f.window.update(visual, |_, _, _| {}).unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
}

/// The other app wrote: the board's two-second poll reloads.
fn poll_tick(visual: &mut VisualTestContext) {
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    visual.refresh().unwrap();
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

fn id_of(f: &Fixture, cx: &mut impl gpui::AppContext, number: i64) -> String {
    detail(f, cx)
        .cards
        .iter()
        .find(|card| card.number == number)
        .unwrap()
        .id
        .clone()
}

fn painted(visual: &mut VisualTestContext, number: i64) -> bool {
    visual.refresh().unwrap();
    visual
        .debug_bounds(selector(format!("board-card-{number}")))
        .is_some()
}

/// The seeded card numbers painted on screen.
fn painted_numbers(visual: &mut VisualTestContext) -> Vec<i64> {
    visual.refresh().unwrap();
    (1..=1000)
        .filter(|n| {
            visual
                .debug_bounds(selector(format!("board-card-{n}")))
                .is_some()
        })
        .collect()
}

/// The card is painted and lies fully inside the lanes' view.
fn in_lane_view(f: &Fixture, visual: &mut VisualTestContext, number: i64) -> bool {
    visual.refresh().unwrap();
    let Some(card) = visual.debug_bounds(selector(format!("board-card-{number}"))) else {
        return false;
    };
    let view = f
        .window
        .read_with(visual, |view, _| {
            view.lane_list.as_ref().unwrap().viewport_bounds()
        })
        .unwrap();
    card.top() >= view.top() && card.bottom() <= view.bottom()
}

fn rows(f: &Fixture, cx: &mut impl gpui::AppContext) -> Vec<LaneRow> {
    f.window
        .read_with(cx, |view, _| view.lane_rows(&view.lanes()))
        .unwrap()
}

/// Scroll the lanes so row `ix` is at the top of the view.
fn scroll_lanes_to(f: &Fixture, visual: &mut VisualTestContext, ix: usize) {
    f.window
        .update(visual, |view, _, cx| {
            view.lane_list
                .as_ref()
                .unwrap()
                .scroll_to(gpui::ListOffset {
                    item_ix: ix,
                    offset_in_item: px(0.),
                });
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn row_index(f: &Fixture, cx: &mut impl gpui::AppContext, row: &LaneRow) -> usize {
    rows(f, cx).iter().position(|r| r == row).unwrap()
}

fn said(f: &Fixture, cx: &mut impl gpui::AppContext) -> Option<String> {
    f.window
        .read_with(cx, |view, _| view.lane_announcement())
        .unwrap()
}

fn rendered(f: &Fixture, visual: &mut VisualTestContext) -> usize {
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| view.rendered_cards.get())
        .unwrap()
}

#[gpui::test]
fn swimlanes_with_1000_cards_render_at_most_60_per_column(cx: &mut TestAppContext) {
    let f = seeded_lanes(cx);
    // Five lanes, each a header and 13 bands (50 cards a cell, 4 a band).
    let all = rows(&f, cx);
    assert_eq!(all.len(), 5 * (1 + 50usize.div_ceil(LANE_BAND)));
    assert_eq!(all[0], header("p4"));
    assert_eq!(all[13], band("p4", 12));
    assert_eq!(all[14], header("p3"));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    for (place, scroll) in [("top", None), ("bottom", Some(all.len() - 3))] {
        if let Some(ix) = scroll {
            scroll_lanes_to(&f, visual, ix);
        }
        let count = rendered(&f, visual);
        assert!(
            (1..=COLUMN_BUDGET * 4).contains(&count),
            "{place}: rendered {count} cards in 4 columns"
        );
        let shown = painted_numbers(visual);
        for column in 0..4 {
            let in_column = shown
                .iter()
                .filter(|n| ((*n - 1) % 4) as usize == column)
                .count();
            assert!(
                (1..=COLUMN_BUDGET).contains(&in_column),
                "{place}: column {column} painted {in_column} cards"
            );
        }
        if place == "top" {
            // Urgent (p4) first: card 9 leads Urgent × Backlog; None's
            // card 1000 is far below.
            assert!(shown.contains(&9));
            assert!(!shown.contains(&1000));
        } else {
            assert!(shown.contains(&1000), "the last card of None paints");
            assert!(!shown.contains(&9));
        }
    }
    // The column heads still count every card.
    f.window
        .read_with(visual, |view, _| {
            let column = view.detail.as_ref().unwrap().columns[0].id.clone();
            assert_eq!(view.count_label(&column), "250");
        })
        .unwrap();
}

#[gpui::test]
fn a_lane_with_no_cards_shows_its_header_and_an_empty_band(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Only"]);
    lanes_by(&f, "priority");
    reload(&f, cx);
    assert_eq!(
        rows(&f, cx),
        [
            header("p4"),
            band("p4", 0),
            header("p3"),
            band("p3", 0),
            header("p2"),
            band("p2", 0),
            header("p1"),
            band("p1", 0),
            header("p0"),
            band("p0", 0),
        ]
    );
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(shown(visual, "board-lane-0"), "Urgent's header shows");
    let empty = visual
        .debug_bounds("board-lane-cell-0-1")
        .expect("Urgent × In Progress has a band");
    assert!(empty.size.height >= px(40.), "{:?}", empty.size);
    // The empty band takes a drop: the card moves and becomes Urgent.
    let from = center(visual, "board-card-1");
    drag(visual, from, empty.center());
    assert_eq!(column_of(&f, visual, "Only"), "In Progress");
    let only = card_id(&f, visual, "Only");
    assert_eq!(extras_of(&f, &only).priority, 4);
}

#[gpui::test]
fn drop_on_any_band_of_a_cell_appends_to_that_cell(cx: &mut TestAppContext) {
    let titles = [
        "M1", "M2", "M3", "M4", "M5", "M6", "M7", "M8", "M9", "Mover A", "Mover B",
    ];
    let f = seeded(cx, &titles);
    let progress = detail(&f, cx).columns[1].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    for title in &titles[..9] {
        let id = card_id(&f, cx, title);
        service
            .card_move(&id, &progress, usize::MAX, "user")
            .unwrap();
        service.set_priority(&id, 2).unwrap();
    }
    lanes_by(&f, "priority");
    reload(&f, cx);
    // Medium (lane 2) × In Progress holds 9 cards: 3 bands.
    let medium: Vec<LaneRow> = rows(&f, cx)
        .into_iter()
        .filter(|row| matches!(row, LaneRow::Band { lane, .. } if lane == "p2"))
        .collect();
    assert_eq!(medium, [band("p2", 0), band("p2", 1), band("p2", 2)]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // Mover A (#10) onto the middle band, between M5 and M8.
    let from = center(visual, "board-card-10");
    let to = center(visual, "board-lane-band-2-1-1");
    drag(visual, from, to);
    let mut expected: Vec<&str> = titles[..9].to_vec();
    expected.push("Mover A");
    assert_eq!(titles_in(&f, visual, "In Progress"), expected);
    let a = card_id(&f, visual, "Mover A");
    assert_eq!(extras_of(&f, &a).priority, 2);
    // Mover B (#11) onto the last band: it also lands at the end.
    let from = center(visual, "board-card-11");
    let to = center(visual, "board-lane-band-2-2-1");
    drag(visual, from, to);
    expected.push("Mover B");
    assert_eq!(titles_in(&f, visual, "In Progress"), expected);
    // A value-setting lane drop keeps no Undo (C5-o).
    let undo = drawer(&f, visual, |view, _| {
        view.undo.toast.as_ref().map(|toast| toast.undo.is_some())
    });
    assert_eq!(undo, Some(false));
}

#[gpui::test]
fn keyboard_drag_down_reaches_a_lane_whose_cards_are_not_mounted(cx: &mut TestAppContext) {
    let f = seeded_lanes(cx);
    let nine = id_of(&f, cx, 9);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    // None × In Progress starts with card 10, far below.
    assert!(!painted(visual, 10));
    assert!(!shown(visual, "board-lane-4"));
    focus_card(&f, visual, &nine);
    press(visual, "space");
    assert_eq!(
        said(&f, visual).as_deref(),
        Some("Picked up card #9, Card 9. Lane Urgent, column Backlog, position 1 of 50.")
    );
    // One lane a step: High, Medium, Low, None.
    press_n(visual, "down", 4);
    frames(&f, visual);
    let lane = drawer(&f, visual, |view, _| view.kb_drag.clone().unwrap().lane);
    assert_eq!(lane.as_deref(), Some("p0"));
    assert!(shown(visual, "board-lane-4"), "None's header scrolled in");
    assert_eq!(
        said(&f, visual).as_deref(),
        Some("Card #9, Card 9 is over lane None, column Backlog, position 51 of 51.")
    );
    press(visual, "right");
    assert_eq!(
        said(&f, visual).as_deref(),
        Some("Card #9, Card 9 is over lane None, column In Progress, position 51 of 51.")
    );
    press(visual, "enter");
    assert_eq!(
        said(&f, visual).as_deref(),
        Some("Dropped card #9 in lane None, column In Progress, position 51 of 51.")
    );
    // The store holds the new column and lane value.
    let card = detail(&f, visual)
        .cards
        .into_iter()
        .find(|card| card.id == nine)
        .unwrap();
    assert_eq!(card.column_id, detail(&f, visual).columns[1].id);
    assert_eq!(extras_of(&f, &nine).priority, 0);
    let undo = drawer(&f, visual, |view, _| {
        view.undo.toast.as_ref().map(|toast| toast.undo.is_some())
    });
    assert_eq!(undo, Some(false), "a value-setting drop keeps no Undo");
}

#[gpui::test]
fn shift_range_in_swimlanes_follows_lane_major_order(cx: &mut TestAppContext) {
    let f = seeded_lanes(cx);
    // Lane by lane (Urgent first), column by column, then by position.
    let mut order: Vec<i64> = Vec::new();
    for priority in [4, 3, 2, 1, 0] {
        for column in 0..4 {
            order.extend((1..=1000).filter(|n| n % 5 == priority && (n - 1) % 4 == column));
        }
    }
    let from = order.iter().position(|n| *n == 9).unwrap();
    let to = order.iter().position(|n| *n == 5).unwrap();
    let numbers = detail(&f, cx)
        .cards
        .iter()
        .map(|card| (card.number, card.id.clone()))
        .collect::<HashMap<_, _>>();
    let expected: Vec<String> = order[from..=to]
        .iter()
        .map(|n| numbers[n].clone())
        .collect();
    assert_eq!(expected.len(), 801, "four whole lanes and one card");
    f.window
        .read_with(cx, |view, _| {
            let all: Vec<String> = order.iter().map(|n| numbers[n].clone()).collect();
            assert_eq!(view.display_order(), all, "display order is lane-major");
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // Anchor on card 9 (Urgent × Backlog), then scroll down to None.
    let nine = numbers[&9].clone();
    f.window
        .update(visual, |view, _, cx| view.toggle_select(&nine, cx))
        .unwrap();
    let none = row_index(&f, visual, &header("p0"));
    scroll_lanes_to(&f, visual, none);
    assert!(!painted(visual, 9), "the anchor is not mounted");
    let at = center(visual, "board-card-5");
    visual.simulate_click(at, gpui::Modifiers::shift());
    visual.run_until_parked();
    let selected = drawer(&f, visual, |view, _| view.selection.ids.clone());
    assert_eq!(selected.len(), 801);
    assert_eq!(selected, expected.into_iter().collect::<HashSet<_>>());
}

#[gpui::test]
fn lane_order_matches_the_tauri_rows_for_the_same_board(cx: &mut TestAppContext) {
    // The fixed board from Tauri's C6-13: columns Todo and Done; lanes
    // Urgent (p4), Medium (p2) and None (p0). Urgent holds u1–u5 in Todo and
    // u6–u7 in Done, Medium nothing, None n1–n8 in Done.
    let f = with_board(cx);
    let board = open_id(&f, cx);
    let columns = detail(&f, cx).columns;
    let mut service = Service::open(&f.db).unwrap();
    service
        .column_update(&columns[0].id, "Todo", columns[0].role.as_deref())
        .unwrap();
    service.column_delete(&columns[1].id).unwrap();
    service.column_delete(&columns[2].id).unwrap();
    service
        .update_settings(
            &board,
            &gespan_core::board::extras::SettingsPatch {
                priority_json: Some(Some(
                    r#"{"names":["None","Low","Medium","High","Urgent"],"hidden":[1,3]}"#.into(),
                )),
                ..Default::default()
            },
        )
        .unwrap();
    let (todo, done) = (columns[0].id.clone(), columns[3].id.clone());
    let mut titles = Vec::new();
    for (column, prefix, range, priority) in [
        (&todo, "u", 1..=5, 4),
        (&done, "u", 6..=7, 4),
        (&done, "n", 1..=8, 0),
    ] {
        for n in range {
            let title = format!("{prefix}{n}");
            let card = service.card_create(column, &title, "").unwrap();
            service.set_priority(&card.id, priority).unwrap();
            titles.push(title);
        }
    }
    lanes_by(&f, "priority");
    reload(&f, cx);
    let labels: Vec<String> = f
        .window
        .read_with(cx, |view, _| {
            view.lanes().into_iter().map(|lane| lane.label).collect()
        })
        .unwrap();
    assert_eq!(labels, ["Urgent", "Medium", "None"]);
    assert_eq!(
        rows(&f, cx),
        [
            header("p4"),
            band("p4", 0),
            band("p4", 1),
            header("p2"),
            band("p2", 0),
            header("p0"),
            band("p0", 0),
            band("p0", 1),
        ]
    );
    let order: Vec<String> = f
        .window
        .read_with(cx, |view, _| {
            let detail = view.detail.as_ref().unwrap();
            view.display_order()
                .iter()
                .map(|id| {
                    detail
                        .cards
                        .iter()
                        .find(|card| &card.id == id)
                        .unwrap()
                        .title
                        .clone()
                })
                .collect()
        })
        .unwrap();
    assert_eq!(order, titles, "u1…u5, u6, u7, n1…n8");
}

#[gpui::test]
fn arrow_keys_move_focus_across_lanes_and_columns_and_reach_unmounted_cards(
    cx: &mut TestAppContext,
) {
    let f = seeded_lanes(cx);
    let id = |f: &Fixture, cx: &mut VisualTestContext, n: i64| id_of(f, cx, n);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    // Urgent × Backlog: 9, 29, …, 989 (50 cards).
    let nine = id(&f, visual, 9);
    focus_card(&f, visual, &nine);
    assert!(
        !painted(visual, 989),
        "the cell's last card starts unmounted"
    );
    press_n(visual, "down", 49);
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), Some(id(&f, visual, 989)));
    assert!(in_lane_view(&f, visual, 989), "989 scrolled into view");
    // Down past the cell's end: into High × Backlog, whose first card is 13.
    press(visual, "down");
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), Some(id(&f, visual, 13)));
    assert!(in_lane_view(&f, visual, 13));
    // Right: the same index in High × In Progress (18).
    press(visual, "right");
    assert_eq!(focused_card(&f, visual), Some(id(&f, visual, 18)));
    // Up from the cell's first card: Urgent × In Progress's last card (994).
    press(visual, "up");
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), Some(id(&f, visual, 994)));
    assert!(in_lane_view(&f, visual, 994));
    // Scrolled away to the bottom, up twice reaches 954 (the band above
    // 994's), which is unmounted.
    let last = rows(&f, visual).len() - 3;
    scroll_lanes_to(&f, visual, last);
    assert!(!painted(visual, 954));
    press_n(visual, "up", 2);
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), Some(id(&f, visual, 954)));
    assert!(in_lane_view(&f, visual, 954));
    // Left: Urgent × Backlog at the same index (48th card: 949).
    press(visual, "left");
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), Some(id(&f, visual, 949)));
    assert!(in_lane_view(&f, visual, 949));
}

#[gpui::test]
fn up_and_down_skip_lanes_with_no_cards_in_that_column(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Top", "Bottom"]);
    let top = card_id(&f, cx, "Top");
    let bottom = card_id(&f, cx, "Bottom");
    Service::open(&f.db).unwrap().set_priority(&top, 4).unwrap();
    lanes_by(&f, "priority");
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // Urgent [Top], High, Medium and Low empty, None [Bottom].
    focus_card(&f, visual, &top);
    press(visual, "down");
    assert_eq!(focused_card(&f, visual), Some(bottom.clone()));
    press(visual, "up");
    assert_eq!(focused_card(&f, visual), Some(top));
    // A keyboard drag steps one lane at a time: down once is High.
    press(visual, "space down");
    let lane = drawer(&f, visual, |view, _| view.kb_drag.clone().unwrap().lane);
    assert_eq!(lane.as_deref(), Some("p3"));
    press(visual, "escape");
    assert_eq!(
        said(&f, visual).as_deref(),
        Some(
            "Moving card #1 was cancelled. It is back in lane Urgent, column Backlog, position 1 of 1."
        )
    );
}

#[gpui::test]
fn a_reload_that_moves_the_focused_card_across_a_band_edge_keeps_focus_on_it(
    cx: &mut TestAppContext,
) {
    let titles = ["K1", "K2", "K3", "K4", "K5", "K6"];
    let f = seeded(cx, &titles);
    lanes_by(&f, "priority");
    reload(&f, cx);
    let k4 = card_id(&f, cx, "K4");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &k4);
    visual.refresh().unwrap();
    // The other app adds a card at the top of None × Backlog: K4 moves from
    // band 0 to band 1.
    let backlog = detail(&f, visual).columns[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    let new = service.card_create(&backlog, "K0", "").unwrap();
    service.card_move(&new.id, &backlog, 0, "user").unwrap();
    poll_tick(visual);
    frames(&f, visual);
    let cell = drawer(&f, visual, |view, _| {
        view.lane_cell_ids[&("p0".to_string(), backlog.clone())].clone()
    });
    assert_eq!(
        cell.iter().position(|id| *id == k4),
        Some(4),
        "now in band 1"
    );
    assert_eq!(focused_card(&f, visual), Some(k4.clone()));
    assert!(painted(visual, 4));
    press(visual, "down");
    assert_eq!(focused_card(&f, visual), Some(card_id(&f, visual, "K5")));
}

#[gpui::test]
fn a_focused_lane_card_hidden_by_a_filter_moves_focus_to_the_nearest_card_in_its_cell(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Keep 1", "Keep 2", "Drop 3", "Keep 4", "Solo"]);
    let solo = card_id(&f, cx, "Solo");
    Service::open(&f.db)
        .unwrap()
        .set_priority(&solo, 1)
        .unwrap();
    lanes_by(&f, "priority");
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    // "Drop 3" sits between Keep 2 and Keep 4 in None × Backlog: the next
    // card still shown takes focus.
    let drop = card_id(&f, visual, "Drop 3");
    focus_card(&f, visual, &drop);
    visual.refresh().unwrap();
    set_search(&f, visual, "Keep");
    frames(&f, visual);
    assert_eq!(
        focused_card(&f, visual),
        Some(card_id(&f, visual, "Keep 4"))
    );
    // Solo is alone in Low × Backlog: hidden, its lane's header takes focus.
    set_search(&f, visual, "");
    focus_card(&f, visual, &solo);
    visual.refresh().unwrap();
    set_search(&f, visual, "Keep");
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), None);
    let on_header = f
        .window
        .update(visual, |view, window, _| {
            view.lane_header_focus["p1"].is_focused(window)
        })
        .unwrap();
    assert!(on_header, "Low's header has focus");
}

#[gpui::test]
fn the_focused_card_and_the_drawer_card_stay_mounted_when_their_lanes_scroll_away(
    cx: &mut TestAppContext,
) {
    let f = seeded_lanes(cx);
    let nine = id_of(&f, cx, 9);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &nine);
    visual.refresh().unwrap();
    let last = rows(&f, visual).len() - 3;
    scroll_lanes_to(&f, visual, last);
    // Its band (9, 29, 49, 69) stays mounted; the next band does not.
    assert!(painted(visual, 9), "the focused card stays mounted");
    assert!(!painted(visual, 89), "the next band does not");
    // Down from far away: the next card scrolls back in and takes focus.
    press(visual, "down");
    frames(&f, visual);
    assert_eq!(focused_card(&f, visual), Some(id_of(&f, visual, 29)));
    assert!(in_lane_view(&f, visual, 29));
    // The drawer's card stays mounted while focus is in the drawer.
    let thirteen = id_of(&f, visual, 13);
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(thirteen.clone(), window, cx);
            window.focus(&view.drawer_focus.clone(), cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    scroll_lanes_to(&f, visual, last);
    assert!(painted(visual, 13), "the drawer's card stays mounted");
    assert!(!painted(visual, 29));
}
