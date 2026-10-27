//! Windowed board columns at 1,000 cards (C6-5): only the cards in or near a
//! column's view mount, and drops, reloads and density changes still land
//! where the user sees them. The timing tests run with one test thread, as
//! `native:test` does.
use super::*;
use gespan_core::board::extras::{SortKey, SortSpec};
use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase};
use std::time::Instant;

/// The most cards one expanded board column may render (the C6 budget).
const COLUMN_BUDGET: usize = 60;
const SEEDED: i64 = 1000;

/// A board with the 1,000-card seed. With `one_column`, every column but the
/// first is deleted first, so the first column holds all 1,000 cards.
pub(super) fn seeded_board(cx: &mut TestAppContext, one_column: bool) -> Fixture {
    let f = with_board(cx);
    let board = open_id(&f, cx);
    let mut service = Service::open(&f.db).unwrap();
    if one_column {
        for column in &detail(&f, cx).columns[1..] {
            service.column_delete(&column.id).unwrap();
        }
    }
    service.seed(&board, &Default::default()).unwrap();
    reload_view(&f, cx);
    f
}

pub(super) fn reload_view(f: &Fixture, cx: &mut impl gpui::AppContext) {
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
}

/// The board with one more column, "Inbox", holding one card, "Mover 7"
/// (the 7 keeps it on screen under the filtered test's search).
fn with_inbox(f: &Fixture, cx: &mut TestAppContext) -> String {
    let board = open_id(f, cx);
    let mut service = Service::open(&f.db).unwrap();
    let inbox = service.column_create(&board, "Inbox", None).unwrap();
    let mover = service.card_create(&inbox.id, "Mover 7", "").unwrap();
    reload_view(f, cx);
    cx.run_until_parked();
    mover.id
}

pub(super) fn column_at(f: &Fixture, cx: &mut impl gpui::AppContext, index: usize) -> String {
    detail(f, cx).columns[index].id.clone()
}

/// Card numbers painted on screen, of the seeded numbers `1..=SEEDED`.
fn painted(visual: &mut VisualTestContext) -> Vec<i64> {
    visual.refresh().unwrap();
    (1..=SEEDED)
        .filter(|n| {
            visual
                .debug_bounds(selector(format!("board-card-{n}")))
                .is_some()
        })
        .collect()
}

fn rendered_cards(f: &Fixture, visual: &mut VisualTestContext) -> usize {
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| view.rendered_cards.get())
        .unwrap()
}

/// A column's visible card ids, as its list holds them.
pub(super) fn list_ids(f: &Fixture, cx: &mut impl gpui::AppContext, column: &str) -> Vec<String> {
    f.window
        .read_with(cx, |view, _| view.column_ids[column].clone())
        .unwrap()
}

pub(super) fn list_top(
    f: &Fixture,
    cx: &mut impl gpui::AppContext,
    column: &str,
) -> gpui::ListOffset {
    f.window
        .read_with(cx, |view, _| view.column_lists[column].logical_scroll_top())
        .unwrap()
}

pub(super) fn scroll_list_to(
    f: &Fixture,
    visual: &mut VisualTestContext,
    column: &str,
    item_ix: usize,
) {
    f.window
        .update(visual, |view, _, cx| {
            view.column_lists[column].scroll_to(gpui::ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        })
        .unwrap();
    visual.refresh().unwrap();
}

pub(super) fn number_of(f: &Fixture, cx: &mut impl gpui::AppContext, card: &str) -> i64 {
    detail(f, cx)
        .cards
        .iter()
        .find(|c| c.id == card)
        .unwrap()
        .number
}

pub(super) fn card_bounds(
    visual: &mut VisualTestContext,
    number: i64,
) -> gpui::Bounds<gpui::Pixels> {
    visual
        .debug_bounds(selector(format!("board-card-{number}")))
        .unwrap_or_else(|| panic!("card {number} is not painted"))
}

/// A column's card ids in stored order (every card, hidden or not).
pub(super) fn stored_order(f: &Fixture, column: &str) -> Vec<String> {
    let board = Service::open(&f.db).unwrap().default_board().unwrap();
    let mut cards: Vec<_> = Service::open(&f.db)
        .unwrap()
        .get(&board.id)
        .unwrap()
        .cards
        .into_iter()
        .filter(|card| card.column_id == column)
        .collect();
    cards.sort_by_key(|card| card.position);
    cards.into_iter().map(|card| card.id).collect()
}

/// Drag `card` from where it is painted to just between two painted cards
/// (`above`, then `below`), and drop it.
fn drag_between(visual: &mut VisualTestContext, card: i64, above: i64, below: i64) {
    visual.refresh().unwrap();
    let from = card_bounds(visual, card).center();
    let (a, b) = (card_bounds(visual, above), card_bounds(visual, below));
    assert!(a.bottom() <= b.top(), "{above} is painted above {below}");
    let to = gpui::point(b.center().x, (a.bottom() + b.top()) / 2.);
    drag(visual, from, to);
}

#[gpui::test]
fn a_1000_card_column_renders_at_most_60_cards(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    assert_eq!(list_ids(&f, cx, &column).len(), 1000);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let rendered = rendered_cards(&f, visual);
    assert!(
        (1..=COLUMN_BUDGET).contains(&rendered),
        "rendered {rendered} cards"
    );
    let shown = painted(visual);
    assert!(
        (10..=COLUMN_BUDGET).contains(&shown.len()),
        "painted {} cards",
        shown.len()
    );
    // The window starts at the top of the column, in order, with no gaps.
    assert_eq!(shown[0], 1);
    assert_eq!(shown, (1..=shown.len() as i64).collect::<Vec<_>>());
    assert!(!shown.contains(&1000));
    // The column still says how many cards it holds.
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.count_label(&column), "1000");
        })
        .unwrap();
}

#[gpui::test]
fn every_expanded_column_is_bounded_and_collapsed_columns_render_none(cx: &mut TestAppContext) {
    let f = seeded_board(cx, false);
    let collapsed = column_at(&f, cx, 1);
    Service::open(&f.db)
        .unwrap()
        .set_column_display(&collapsed, None, None, true)
        .unwrap();
    reload_view(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let rendered = rendered_cards(&f, visual);
    assert!(
        rendered <= COLUMN_BUDGET * 3,
        "rendered {rendered} cards in 3 expanded columns"
    );
    // Card n sits in column (n - 1) % 4.
    let shown = painted(visual);
    for column in 0..4 {
        let count = shown
            .iter()
            .filter(|n| ((*n - 1) % 4) as usize == column)
            .count();
        if column == 1 {
            assert_eq!(count, 0, "the collapsed column mounts no cards");
        } else {
            assert!(
                (5..=COLUMN_BUDGET).contains(&count),
                "column {column} painted {count} cards"
            );
        }
    }
    assert!(visual.debug_bounds("board-column-collapsed-1").is_some());
}

#[gpui::test]
fn scrolling_a_column_renders_the_cards_at_that_offset(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let body = center(visual, "board-column-body-0");
    for _ in 0..3 {
        visual.simulate_event(ScrollWheelEvent {
            position: body,
            delta: ScrollDelta::Pixels(gpui::point(px(0.), px(-2000.))),
            modifiers: Default::default(),
            touch_phase: TouchPhase::Moved,
        });
        visual.run_until_parked();
    }
    let top = list_top(&f, visual, &column).item_ix;
    assert!(top > 40, "scrolled to item {top}");
    let shown = painted(visual);
    assert!(shown.len() <= COLUMN_BUDGET, "painted {}", shown.len());
    assert!(!shown.contains(&1), "the first card scrolled away");
    // Exactly the cards from the top item down are painted, in order.
    let first = number_of(&f, visual, &ids[top]);
    assert_eq!(shown[0], first, "{shown:?}");
    let expected: Vec<i64> = (first..first + shown.len() as i64).collect();
    assert_eq!(shown, expected);
    let view = f
        .window
        .read_with(visual, |view, _| {
            view.column_lists[&column].viewport_bounds()
        })
        .unwrap();
    let card = card_bounds(visual, first);
    assert!(card.bottom() > view.top() && card.top() <= view.top() + px(1.));
    assert!(rendered_cards(&f, visual) <= COLUMN_BUDGET);
}

#[gpui::test]
fn a_reload_with_unchanged_ids_keeps_the_scroll_position(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    scroll_list_to(&f, visual, &column, 400);
    let before = list_top(&f, visual, &column);
    assert_eq!(before.item_ix, 400);
    let top_card = number_of(&f, visual, &ids[400]);
    let top_y = card_bounds(visual, top_card).top();
    // The other app edits a card's title: same ids, so the view stays put.
    Service::open(&f.db)
        .unwrap()
        .card_update(&ids[10], "Renamed elsewhere", "")
        .unwrap();
    reload_view(&f, visual);
    visual.refresh().unwrap();
    let after = list_top(&f, visual, &column);
    assert_eq!(after.item_ix, before.item_ix);
    assert_eq!(after.offset_in_item, before.offset_in_item);
    assert_eq!(card_bounds(visual, top_card).top(), top_y);
    // A plain reload with nothing changed keeps it too.
    reload_view(&f, visual);
    visual.refresh().unwrap();
    assert_eq!(list_top(&f, visual, &column).item_ix, 400);
    assert_eq!(card_bounds(visual, top_card).top(), top_y);
}

#[gpui::test]
fn a_card_added_above_keeps_the_top_card_in_view(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    scroll_list_to(&f, visual, &column, 500);
    let top_card = number_of(&f, visual, &ids[500]);
    let top_y = card_bounds(visual, top_card).top();
    // The other app adds a card at the top of the column, above the view.
    let mut service = Service::open(&f.db).unwrap();
    let added = service.card_create(&column, "Added elsewhere", "").unwrap();
    service.card_move(&added.id, &column, 0, "user").unwrap();
    reload_view(&f, visual);
    visual.refresh().unwrap();
    let now = list_ids(&f, visual, &column);
    assert_eq!(now.len(), 1001);
    assert_eq!(now[0], added.id);
    assert_eq!(list_top(&f, visual, &column).item_ix, 501);
    assert_eq!(card_bounds(visual, top_card).top(), top_y);
    // No blank gap: the window is still full of the cards below it.
    let shown = painted(visual);
    assert!(shown.len() >= 10 && shown.len() <= COLUMN_BUDGET);
    assert_eq!(shown[0], top_card);
    // And one removed above keeps it too.
    service.card_delete(&ids[3]).unwrap();
    reload_view(&f, visual);
    visual.refresh().unwrap();
    assert_eq!(list_top(&f, visual, &column).item_ix, 500);
    assert_eq!(card_bounds(visual, top_card).top(), top_y);
    // One reload that changes ids both above and below the view: the changed
    // run spans the top card, which still stays at the top.
    let mut service = Service::open(&f.db).unwrap();
    service.card_delete(&ids[5]).unwrap();
    let below = service.card_create(&column, "Below elsewhere", "").unwrap();
    service.card_move(&below.id, &column, 700, "user").unwrap();
    reload_view(&f, visual);
    visual.refresh().unwrap();
    let now = list_ids(&f, visual, &column);
    assert_eq!(now[700], below.id);
    assert_eq!(now[499], ids[500]);
    assert_eq!(list_top(&f, visual, &column).item_ix, 499);
    assert_eq!(card_bounds(visual, top_card).top(), top_y);
}

#[gpui::test]
fn drop_into_a_scrolled_column_lands_between_the_hovered_cards(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let mover = with_inbox(&f, cx);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    scroll_list_to(&f, visual, &column, 500);
    let (above, below) = (ids[505].clone(), ids[506].clone());
    let (a, b) = (number_of(&f, visual, &above), number_of(&f, visual, &below));
    let mover_number = number_of(&f, visual, &mover);
    drag_between(visual, mover_number, a, b);
    let order = stored_order(&f, &column);
    assert_eq!(order.len(), 1001);
    let at = order.iter().position(|id| *id == mover).expect("moved in");
    assert_eq!(at, 506);
    assert_eq!(order[at - 1], above);
    assert_eq!(order[at + 1], below);
    f.window
        .read_with(visual, |view, _| {
            assert!(!view.dragging);
            assert!(view.drop_target.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn drop_into_a_filtered_windowed_column_keeps_hidden_cards_in_place(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let mover = with_inbox(&f, cx);
    let column = column_at(&f, cx, 0);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // "7" keeps the cards whose number has a 7 (and "Mover" has none).
    f.window
        .update(visual, |view, _, cx| {
            view.search
                .update(cx, |input, cx| input.set_text("7".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    let full = stored_order(&f, &column);
    let visible = list_ids(&f, visual, &column);
    assert!(visible.len() > COLUMN_BUDGET && visible.len() < full.len());
    let place = |id: &String| full.iter().position(|card| card == id).unwrap();
    // Two neighbours on screen with hidden cards between them, far down.
    let pair = (80..visible.len() - 1)
        .find(|&i| place(&visible[i + 1]) > place(&visible[i]) + 1)
        .expect("visible neighbours with hidden cards between");
    scroll_list_to(&f, visual, &column, pair - 2);
    let (above, below) = (visible[pair].clone(), visible[pair + 1].clone());
    let (a, b) = (number_of(&f, visual, &above), number_of(&f, visual, &below));
    let hidden_between: Vec<String> = full[place(&above) + 1..place(&below)].to_vec();
    assert!(!hidden_between.is_empty());
    let mover_number = number_of(&f, visual, &mover);
    drag_between(visual, mover_number, a, b);
    // The card lands right before the hovered card below, after every hidden
    // card between the two; nothing else moves.
    let mut expected = full.clone();
    expected.insert(place(&below), mover.clone());
    assert_eq!(stored_order(&f, &column), expected);
    let order = stored_order(&f, &column);
    let at = order.iter().position(|id| *id == mover).unwrap();
    assert_eq!(order[at - 1], *hidden_between.last().unwrap());
    assert_eq!(order[at + 1], below);
}

#[gpui::test]
fn sorted_windowed_column_appends_on_drop(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let mover = with_inbox(&f, cx);
    let column = column_at(&f, cx, 0);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, _, cx| {
            view.spec.sort = SortSpec {
                key: SortKey::Priority,
                ..Default::default()
            };
            view.requery(cx);
        })
        .unwrap();
    visual.run_until_parked();
    let visible = list_ids(&f, visual, &column);
    assert_eq!(visible.len(), 1000);
    scroll_list_to(&f, visual, &column, 300);
    let (a, b) = (
        number_of(&f, visual, &visible[303]),
        number_of(&f, visual, &visible[304]),
    );
    let mover_number = number_of(&f, visual, &mover);
    drag_between(visual, mover_number, a, b);
    let order = stored_order(&f, &column);
    assert_eq!(order.len(), 1001);
    assert_eq!(order.last(), Some(&mover), "a sorted column only appends");
    // Inside a sorted column a card can't be placed: nothing is written.
    let before = stored_order(&f, &column);
    let (c, d) = (
        number_of(&f, visual, &visible[306]),
        number_of(&f, visual, &visible[309]),
    );
    let (moving, target) = (number_of(&f, visual, &visible[305]), c);
    drag_between(visual, moving, target, d);
    assert_eq!(stored_order(&f, &column), before);
}

#[gpui::test]
fn density_toggle_remeasures_without_overlap(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    patch_display(&f, r#"{"density":"compact"}"#);
    reload_view(&f, visual);
    scroll_list_to(&f, visual, &column, 200);
    let compact = painted(visual);
    let compact_height = card_bounds(visual, compact[1]).size.height;
    // Scroll back up through cards measured compact, then go comfortable.
    scroll_list_to(&f, visual, &column, 150);
    patch_display(&f, r#"{"density":"comfortable"}"#);
    reload_view(&f, visual);
    visual.refresh().unwrap();
    let shown = painted(visual);
    assert!(
        (5..=COLUMN_BUDGET).contains(&shown.len()),
        "painted {}",
        shown.len()
    );
    let mut bounds: Vec<_> = shown.iter().map(|n| card_bounds(visual, *n)).collect();
    bounds.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
    for pair in bounds.windows(2) {
        assert!(
            pair[0].bottom() <= pair[1].top(),
            "cards overlap: {:?} and {:?}",
            pair[0],
            pair[1]
        );
        assert!(pair[1].top() - pair[0].bottom() <= px(6.5), "a gap opened");
    }
    let comfortable_height = bounds[1].size.height;
    assert!(comfortable_height > compact_height);
    // Cards below the view were measured compact before the toggle. A wheel
    // scroll far past them must move by comfortable cards, not by the old
    // compact heights (which would land many cards too far).
    let pitch = bounds[1].top() - bounds[0].top();
    let from = list_top(&f, visual, &column).item_ix;
    let body = center(visual, "board-column-body-0");
    visual.simulate_event(ScrollWheelEvent {
        position: body,
        delta: ScrollDelta::Pixels(gpui::point(px(0.), px(-3000.))),
        modifiers: Default::default(),
        touch_phase: TouchPhase::Moved,
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    let moved = (list_top(&f, visual, &column).item_ix - from) as f32;
    let expected = 3000. / f32::from(pitch);
    assert!(
        (moved - expected).abs() <= 1.5,
        "a 3000px scroll moved {moved} cards; {expected} comfortable cards fit"
    );
    let shown = painted(visual);
    let mut bounds: Vec<_> = shown.iter().map(|n| card_bounds(visual, *n)).collect();
    bounds.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
    for pair in bounds.windows(2) {
        assert!(pair[0].bottom() <= pair[1].top());
    }
    // Back to compact: the cards measured comfortable shrink again.
    patch_display(&f, r#"{"density":"compact"}"#);
    reload_view(&f, visual);
    visual.refresh().unwrap();
    let shown = painted(visual);
    let mut bounds: Vec<_> = shown.iter().map(|n| card_bounds(visual, *n)).collect();
    bounds.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
    for pair in bounds.windows(2) {
        assert!(pair[0].bottom() <= pair[1].top());
    }
    assert_eq!(bounds[1].size.height, compact_height);
}

#[gpui::test]
fn native_load_of_1000_cards_meets_the_budget(cx: &mut TestAppContext) {
    let f = seeded_board(cx, false);
    let elapsed = f
        .window
        .update(cx, |view, _, cx| {
            let start = Instant::now();
            view.load(cx).unwrap();
            start.elapsed()
        })
        .unwrap();
    assert_eq!(detail(&f, cx).cards.len(), 1000);
    assert!(
        elapsed <= Duration::from_millis(100),
        "load took {elapsed:?}"
    );
}

#[gpui::test]
fn native_open_and_filter_meet_the_budget(cx: &mut TestAppContext) {
    let f = seeded_board(cx, false);
    let (store, db) = (f.store.clone(), f.db.clone());
    let start = Instant::now();
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    gpui::point(px(0.), px(0.)),
                    size(px(1400.), px(1400.)),
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| BoardView::new(store, db, window, cx)),
        )
        .unwrap()
    });
    window.update(cx, |view, _, cx| view.shown(cx)).unwrap();
    cx.run_until_parked();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    let open = start.elapsed();
    assert!(visual.debug_bounds("board-card-1").is_some());

    let start = Instant::now();
    window
        .update(visual, |view, _, cx| {
            view.search
                .update(cx, |input, cx| input.set_text("Card 5".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let filter = start.elapsed();
    assert!(visual.debug_bounds("board-card-5").is_some());
    assert!(visual.debug_bounds("board-card-1").is_none());
    assert!(open <= Duration::from_millis(400), "open took {open:?}");
    assert!(
        filter <= Duration::from_millis(150),
        "filter took {filter:?}"
    );
}

pub(super) fn wheel(visual: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>, dy: f32) {
    visual.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
        modifiers: Default::default(),
        touch_phase: TouchPhase::Moved,
    });
    visual.run_until_parked();
}

#[gpui::test]
fn a_card_dropped_first_in_a_column_viewed_from_the_top_shows_first(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let mover = with_inbox(&f, cx);
    let column = column_at(&f, cx, 0);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // Down and back up: the view has scrolled, and is at the start again.
    let body = center(visual, "board-column-body-0");
    wheel(visual, body, -2000.);
    assert!(list_top(&f, visual, &column).item_ix > 0);
    wheel(visual, body, 4000.);
    let top = list_top(&f, visual, &column);
    assert_eq!((top.item_ix, top.offset_in_item), (0, px(0.)));
    visual.refresh().unwrap();
    let first = card_bounds(visual, 1);
    let mover_number = number_of(&f, visual, &mover);
    let from = card_bounds(visual, mover_number).center();
    drag(
        visual,
        from,
        gpui::point(first.center().x, first.top() + px(2.)),
    );
    assert_eq!(stored_order(&f, &column)[0], mover);
    visual.refresh().unwrap();
    let top = list_top(&f, visual, &column);
    assert_eq!((top.item_ix, top.offset_in_item), (0, px(0.)));
    // It is painted first, at the top of the column, above card 1.
    let dropped = card_bounds(visual, mover_number);
    let view = f
        .window
        .read_with(visual, |view, _| {
            view.column_lists[&column].viewport_bounds()
        })
        .unwrap();
    assert!(dropped.top() >= view.top() && dropped.top() <= view.top() + px(8.));
    assert!(dropped.bottom() <= card_bounds(visual, 1).top());
}

#[gpui::test]
fn the_composer_keeps_every_keystroke_at_the_bottom_of_a_1000_card_column(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    scroll_list_to(&f, visual, &column, 1000);
    click(visual, "board-add-card-0");
    let titles: Vec<String> = (1..=6).map(|i| format!("Typed card {i}")).collect();
    for title in &titles {
        visual.simulate_input(title);
        f.window
            .update(visual, |view, window, cx| {
                let composer = view.composer.as_ref().expect("the composer is open");
                assert!(composer.focus.is_focused(window), "the field has focus");
                assert_eq!(
                    composer.input.read(cx).content,
                    *title,
                    "every keystroke landed"
                );
            })
            .unwrap();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
    let detail = detail(&f, visual);
    let order = stored_order(&f, &column);
    assert_eq!(order.len(), 1006);
    let last: Vec<String> = order[1000..]
        .iter()
        .map(|id| {
            detail
                .cards
                .iter()
                .find(|card| card.id == *id)
                .unwrap()
                .title
                .clone()
        })
        .collect();
    assert_eq!(last, titles);
    // The field is still on screen under the cards it added.
    let view = f
        .window
        .read_with(visual, |view, _| {
            view.column_lists[&column].viewport_bounds()
        })
        .unwrap();
    let newest = detail.cards.iter().find(|c| c.title == titles[5]).unwrap();
    let card = card_bounds(visual, newest.number);
    assert!(card.top() >= view.top() && card.bottom() <= view.bottom());
}

#[gpui::test]
fn a_card_menu_from_a_scrolled_away_focused_card_anchors_on_screen(cx: &mut TestAppContext) {
    let f = seeded_board(cx, true);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &ids[0]);
    let body = center(visual, "board-column-body-0");
    wheel(visual, body, -3000.);
    assert!(list_top(&f, visual, &column).item_ix > 20);
    assert_eq!(focused_card(&f, visual), Some(ids[0].clone()));
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| {
            let menu = view.card_menu.as_ref().expect("the card menu is open");
            assert_eq!(menu.card_id, ids[0]);
            let anchor = view.card_anchors[&ids[0]]
                .get()
                .expect("the card is painted");
            assert_ne!(anchor, gpui::Bounds::default());
            let viewport = view.column_lists[&column].viewport_bounds();
            assert!(
                anchor.top() >= viewport.top() && anchor.bottom() <= viewport.bottom(),
                "{anchor:?} is inside {viewport:?}"
            );
        })
        .unwrap();
}

#[gpui::test]
fn a_keyboard_lift_over_a_collapsed_column_keeps_its_scroll(cx: &mut TestAppContext) {
    let f = seeded_board(cx, false);
    let (first, second) = (column_at(&f, cx, 0), column_at(&f, cx, 1));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    scroll_list_to(&f, visual, &second, 100);
    Service::open(&f.db)
        .unwrap()
        .set_column_display(&second, None, None, true)
        .unwrap();
    reload_view(&f, visual);
    visual.refresh().unwrap();
    assert_eq!(list_top(&f, visual, &second).item_ix, 100);
    let card = list_ids(&f, visual, &first)[0].clone();
    focus_card(&f, visual, &card);
    // Lift, pass over the collapsed column and back, and put it down again.
    visual.simulate_keystrokes("space right left escape");
    visual.run_until_parked();
    assert_eq!(list_top(&f, visual, &second).item_ix, 100);
}

// ---- C6-8: load, handles and link lookups at scale ----

/// The reads one `load` makes, by name.
fn load_reads(f: &Fixture, cx: &mut TestAppContext) -> Vec<&'static str> {
    f.window
        .update(cx, |view, _, cx| {
            view.reads.take();
            view.load(cx).unwrap();
            view.reads.take()
        })
        .unwrap()
}

#[gpui::test]
fn load_reads_column_counts_in_one_call(cx: &mut TestAppContext) {
    // Ten cards on four columns.
    let small = with_board(cx);
    let board = open_id(&small, cx);
    Service::open(&small.db)
        .unwrap()
        .seed(
            &board,
            &gespan_board_store::seed::SeedSpec {
                cards: 10,
                ..Default::default()
            },
        )
        .unwrap();
    reload_view(&small, cx);
    let few = load_reads(&small, cx);
    let counted = |reads: &[&str], name: &str| reads.iter().filter(|r| **r == name).count();
    assert_eq!(counted(&few, "column_card_counts"), 1, "{few:?}");
    assert_eq!(counted(&few, "column_card_count"), 0, "{few:?}");

    // A thousand cards on four columns, then on twelve: the same reads.
    let f = seeded_board(cx, false);
    let many = load_reads(&f, cx);
    assert_eq!(many, few, "the reads don't grow with the cards");
    let board = open_id(&f, cx);
    let mut service = Service::open(&f.db).unwrap();
    for n in 0..8 {
        service
            .column_create(&board, &format!("Extra {n}"), None)
            .unwrap();
    }
    let wide = load_reads(&f, cx);
    assert_eq!(wide, few, "the reads don't grow with the columns");

    // The one read still gives every column its live and archived counts.
    let first = column_at(&f, cx, 0);
    let archived = list_ids(&f, cx, &first)[0].clone();
    service.archive(&archived).unwrap();
    reload_view(&f, cx);
    f.window
        .read_with(cx, |view, _| {
            let detail = view.detail.as_ref().unwrap();
            assert_eq!(view.column_counts.len(), 12);
            for column in &detail.columns {
                let live = detail
                    .cards
                    .iter()
                    .filter(|card| card.column_id == column.id)
                    .count();
                let archived = usize::from(column.id == first);
                assert_eq!(
                    view.column_counts[&column.id],
                    (live, archived),
                    "{}",
                    column.name
                );
            }
            assert_eq!(
                view.delete_blocked(&first).as_deref(),
                Some("Move or delete its 250 cards first")
            );
            let empty = &detail.columns[11].id;
            assert_eq!(view.delete_blocked(empty), None);
        })
        .unwrap();
}

#[gpui::test]
fn sync_handles_on_1000_cards_is_linear(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let board = open_id(&f, cx);
    Service::open(&f.db)
        .unwrap()
        .seed(
            &board,
            &gespan_board_store::seed::SeedSpec {
                cards: 2000,
                ..Default::default()
            },
        )
        .unwrap();
    reload_view(&f, cx);
    let (elapsed, kept) = f
        .window
        .update(cx, |view, _, cx| {
            let full = view.detail.clone().unwrap();
            assert_eq!(full.cards.len(), 2000);
            assert_eq!(view.card_focus.len(), 2000);
            let mut half = full.clone();
            half.cards.truncate(1000);
            let kept: HashSet<String> = half.cards.iter().map(|card| card.id.clone()).collect();
            // Warm up once, then put the 2,000 handles back and time it.
            view.detail = Some(half.clone());
            view.sync_handles(cx);
            view.detail = Some(full);
            view.sync_handles(cx);
            assert_eq!(view.card_anchors.len(), 2000);
            view.detail = Some(half);
            let start = Instant::now();
            view.sync_handles(cx);
            (start.elapsed(), kept)
        })
        .unwrap();
    f.window
        .read_with(cx, |view, _| {
            let focus: HashSet<String> = view.card_focus.keys().cloned().collect();
            let anchors: HashSet<String> = view.card_anchors.keys().cloned().collect();
            assert_eq!(focus, kept);
            assert_eq!(anchors, kept);
            assert_eq!(view.column_anchors.len(), 4);
            assert_eq!(view.collapsed_focus.len(), 4);
        })
        .unwrap();
    assert!(
        elapsed <= Duration::from_millis(20),
        "sync_handles took {elapsed:?}"
    );
}

/// `lookup_requests` as it was before C6-8, scanning every link for each
/// one: the reference the grouped version must match.
fn lookup_requests_reference(
    links: &[gespan_core::board::extras::CardLink],
    sessions: &HashMap<String, link_status::SessionInfo>,
    card_projects: &HashMap<String, Vec<String>>,
    fallback: Option<&str>,
) -> Vec<link_status::LookupRequest> {
    let mut requests: Vec<link_status::LookupRequest> = Vec::new();
    for link in links
        .iter()
        .filter(|link| matches!(link.kind.as_str(), "pr" | "worktree"))
    {
        let mut cwds: Vec<String> = Vec::new();
        if link.kind == "pr" {
            let card = links.iter().filter(|other| other.card_id == link.card_id);
            for other in card.clone().filter(|other| other.kind == "worktree") {
                cwds.push(other.reference.clone());
            }
            for other in card.filter(|other| other.kind == "session") {
                if let Some(session) = sessions.get(&other.reference) {
                    cwds.push(session.cwd.clone());
                }
            }
            cwds.extend(
                card_projects
                    .get(&link.card_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            if cwds.is_empty() {
                cwds.extend(fallback.map(str::to_string));
            }
        }
        match requests
            .iter_mut()
            .find(|request| request.kind == link.kind && request.reference == link.reference)
        {
            Some(request) => request.cwds.extend(cwds),
            None => requests.push(link_status::LookupRequest {
                kind: link.kind.clone(),
                reference: link.reference.clone(),
                cwds,
            }),
        }
    }
    for request in &mut requests {
        let mut seen = std::collections::BTreeSet::new();
        request
            .cwds
            .retain(|cwd| !cwd.is_empty() && seen.insert(cwd.clone()));
    }
    requests
}

fn link(card: &str, kind: &str, reference: &str) -> gespan_core::board::extras::CardLink {
    gespan_core::board::extras::CardLink {
        card_id: card.into(),
        kind: kind.into(),
        reference: reference.into(),
        role: None,
        created_at: 0,
    }
}

/// A board's links, its sessions by key and its cards' projects.
type LinkFixture = (
    Vec<gespan_core::board::extras::CardLink>,
    HashMap<String, link_status::SessionInfo>,
    HashMap<String, Vec<String>>,
);

/// `count` links over `count / 4` cards: PRs shared between cards, worktrees,
/// sessions (half of which exist), Pilots and an empty worktree path, with
/// a card's links not next to each other.
fn link_fixture(count: usize) -> LinkFixture {
    let cards = (count / 4).max(1);
    let links: Vec<_> = (0..count)
        .map(|i| {
            let card = format!("card-{}", (i * 7) % cards);
            match i % 5 {
                0 | 3 => link(
                    &card,
                    "pr",
                    &format!("https://github.com/acme/app/pull/{}", i % 97 + 1),
                ),
                1 => link(&card, "worktree", &format!("/work/wt-{}", i % 61)),
                2 => link(&card, "session", &format!("s-{}", i % 50)),
                _ if i % 20 == 4 => link(&card, "worktree", ""),
                _ => link(&card, "pilot", &format!("pilot-{}", i % 3)),
            }
        })
        .collect();
    let sessions = (0..50)
        .filter(|n| n % 2 == 0)
        .map(|n| {
            (
                format!("s-{n}"),
                link_status::SessionInfo {
                    title: format!("Session {n}"),
                    cwd: format!("/work/s-{n}"),
                },
            )
        })
        .collect();
    let projects = (0..cards)
        .filter(|n| n % 3 == 0)
        .map(|n| {
            (
                format!("card-{n}"),
                vec!["/proj/shared".to_string(), format!("/proj/{}", n % 4)],
            )
        })
        .collect();
    (links, sessions, projects)
}

#[test]
fn lookup_requests_on_300_links_groups_by_card() {
    // A small board, by hand: card a's PR works in its worktree, its
    // session's folder and its project; card b shares the PR and adds its
    // own; card c has a PR and nothing to work in but the fallback.
    let links = vec![
        link("a", "pr", "https://github.com/acme/app/pull/1"),
        link("b", "worktree", "/work/b"),
        link("a", "session", "s-1"),
        link("a", "worktree", "/work/a"),
        link("b", "pr", "https://github.com/acme/app/pull/1"),
        link("c", "pr", "https://github.com/acme/app/pull/2"),
        link("a", "session", "gone"),
        link("b", "session", "s-1"),
    ];
    let sessions = HashMap::from([(
        "s-1".to_string(),
        link_status::SessionInfo {
            title: "One".into(),
            cwd: "/work/s-1".into(),
        },
    )]);
    let projects = HashMap::from([("a".to_string(), vec!["/proj/a".to_string()])]);
    let requests = link_status::lookup_requests(&links, &sessions, &projects, Some("/fallback"));
    let request = |kind: &str, reference: &str, cwds: &[&str]| link_status::LookupRequest {
        kind: kind.into(),
        reference: reference.into(),
        cwds: cwds.iter().map(|cwd| cwd.to_string()).collect(),
    };
    assert_eq!(
        requests,
        vec![
            request(
                "pr",
                "https://github.com/acme/app/pull/1",
                &["/work/a", "/work/s-1", "/proj/a", "/work/b"],
            ),
            request("worktree", "/work/b", &[]),
            request("worktree", "/work/a", &[]),
            request("pr", "https://github.com/acme/app/pull/2", &["/fallback"]),
        ]
    );

    // 300 links: exactly what the old scan found, in the same order.
    let (links, sessions, projects) = link_fixture(300);
    let expected = lookup_requests_reference(&links, &sessions, &projects, Some("/fallback"));
    let got = link_status::lookup_requests(&links, &sessions, &projects, Some("/fallback"));
    assert!(expected.len() > 50, "{} requests", expected.len());
    assert_eq!(got, expected);
    let none = link_status::lookup_requests(&links, &sessions, &projects, None);
    assert_eq!(
        none,
        lookup_requests_reference(&links, &sessions, &projects, None)
    );

    // 3,000 links group in linear time (after a warm-up run).
    let (links, sessions, projects) = link_fixture(3000);
    let _ = link_status::lookup_requests(&links, &sessions, &projects, Some("/fallback"));
    let start = Instant::now();
    let got = link_status::lookup_requests(&links, &sessions, &projects, Some("/fallback"));
    let elapsed = start.elapsed();
    assert_eq!(
        got,
        lookup_requests_reference(&links, &sessions, &projects, Some("/fallback"))
    );
    assert!(
        elapsed <= Duration::from_millis(20),
        "lookup_requests took {elapsed:?}"
    );
}

#[gpui::test]
fn layout_spec_is_parsed_once_per_change(cx: &mut TestAppContext) {
    let f = seeded(cx, &["One", "Two"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    let parses = |visual: &mut VisualTestContext| {
        f.window
            .read_with(visual, |view, _| view.layout_parses.get())
            .unwrap()
    };
    let layout = |visual: &mut VisualTestContext| {
        f.window
            .read_with(visual, |view, _| {
                (0..10).map(|_| view.layout_kind()).last().unwrap()
            })
            .unwrap()
    };
    // Frames and calls with nothing changed parse nothing again.
    let before = parses(visual);
    for _ in 0..3 {
        f.window.update(visual, |_, _, cx| cx.notify()).unwrap();
        visual.refresh().unwrap();
    }
    assert_eq!(layout(visual), list::Layout::Board);
    assert_eq!(parses(visual) - before, 0, "nothing changed");
    // A layout change parses once, however often it is read after.
    f.window
        .update(visual, |view, _, cx| {
            view.set_layout(list::Layout::List, cx)
        })
        .unwrap();
    for _ in 0..3 {
        visual.refresh().unwrap();
    }
    assert_eq!(layout(visual), list::Layout::List);
    assert_eq!(parses(visual) - before, 1, "one parse for one change");
    // So does a change written straight into the view's layout.
    f.window
        .update(visual, |view, _, cx| {
            view.views.layout = r#"{"layout":"table"}"#.into();
            cx.notify();
        })
        .unwrap();
    visual.refresh().unwrap();
    assert_eq!(layout(visual), list::Layout::Table);
    assert_eq!(parses(visual) - before, 2);
    // And a group-by edit is read back at once.
    f.window
        .update(visual, |view, _, cx| view.set_group_by("priority", cx))
        .unwrap();
    let group = f
        .window
        .read_with(visual, |view, _| view.layout_spec().group_by)
        .unwrap();
    assert_eq!(group.as_deref(), Some("priority"));
    assert_eq!(parses(visual) - before, 3);
}

/// Lift the card titled `title` with Space.
fn lift(f: &Fixture, visual: &mut VisualTestContext, title: &str) -> String {
    visual.refresh().unwrap();
    let card = card_id(f, visual, title);
    focus_card(f, visual, &card);
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    let lifted = f
        .window
        .read_with(visual, |view, _| {
            view.kb_drag.as_ref().map(|d| d.card_id.clone())
        })
        .unwrap();
    assert_eq!(lifted.as_ref(), Some(&card));
    card
}

fn poll_reads(f: &Fixture, visual: &mut VisualTestContext) -> Vec<&'static str> {
    f.window
        .update(visual, |view, _, cx| {
            view.reads.take();
            view.poll(cx);
            view.reads.take()
        })
        .unwrap()
}

fn still_lifted(f: &Fixture, visual: &mut VisualTestContext) -> bool {
    f.window
        .read_with(visual, |view, _| view.kb_drag.is_some())
        .unwrap()
}

#[gpui::test]
fn a_lifted_card_poll_reads_one_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Lifted", "Other"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let card = lift(&f, visual, "Lifted");
    // Nothing changed: the poll reads the lifted card, never the board.
    assert_eq!(
        poll_reads(&f, visual),
        ["card", "card_meta", "data_version"]
    );
    assert!(still_lifted(&f, visual));
    // An edit elsewhere keeps the lift; the reload waits for the drop.
    let mut service = Service::open(&f.db).unwrap();
    service.card_update(&card, "Lifted, renamed", "").unwrap();
    assert_eq!(
        poll_reads(&f, visual),
        ["card", "card_meta", "data_version"]
    );
    assert!(still_lifted(&f, visual));
    f.window
        .read_with(visual, |view, _| assert!(view.pending_reload))
        .unwrap();
    // Archived elsewhere: the lift ends and the board reloads without it.
    service.archive(&card).unwrap();
    let reads = poll_reads(&f, visual);
    assert_eq!(&reads[..2], ["card", "card_meta"]);
    assert!(!reads[2..].contains(&"card"), "{reads:?}");
    assert!(!still_lifted(&f, visual));
    assert!(detail(&f, visual).cards.iter().all(|c| c.id != card));
}

#[gpui::test]
fn a_lifted_card_deleted_or_moved_away_elsewhere_ends_the_lift(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Deleted", "Moved", "Stays"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let deleted = lift(&f, visual, "Deleted");
    let mut service = Service::open(&f.db).unwrap();
    service.card_delete(&deleted).unwrap();
    let reads = poll_reads(&f, visual);
    assert_eq!(reads[0], "card");
    assert!(
        !reads.contains(&"card_meta"),
        "a missing card has no meta to read"
    );
    assert!(!still_lifted(&f, visual));
    assert!(detail(&f, visual).cards.iter().all(|c| c.id != deleted));

    let moved = lift(&f, visual, "Moved");
    let other = service.create("Elsewhere").unwrap();
    service.move_card_to_board(&moved, &other.id).unwrap();
    let reads = poll_reads(&f, visual);
    assert_eq!(reads[0], "card");
    assert!(!still_lifted(&f, visual));
    let titles: Vec<String> = detail(&f, visual)
        .cards
        .iter()
        .map(|c| c.title.clone())
        .collect();
    assert_eq!(titles, ["Stays"]);
}

/// A card row's painted height (card and the gap above it) in column 0.
fn painted_row(visual: &mut VisualTestContext, number: i64) -> Pixels {
    card_bounds(visual, number).size.height + px(6.)
}

/// On a fresh board nothing was measured yet: a jump far down the column
/// scrolls on the first guess, and must land the card in view.
fn far_reveal_lands(cx: &mut TestAppContext, display: Option<&str>) {
    let f = with_board(cx);
    if let Some(json) = display {
        patch_display(&f, json);
    }
    let board = open_id(&f, cx);
    let mut service = Service::open(&f.db).unwrap();
    for column in &detail(&f, cx).columns[1..] {
        service.column_delete(&column.id).unwrap();
    }
    service.seed(&board, &Default::default()).unwrap();
    reload_view(&f, cx);
    let column = column_at(&f, cx, 0);
    let ids = list_ids(&f, cx, &column);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    // The guess is close to what the seeded cards paint at.
    let estimate = f
        .window
        .read_with(visual, |view, _| view.item_estimate(&view.display))
        .unwrap();
    let rows: Vec<Pixels> = (1..=10).map(|n| painted_row(visual, n)).collect();
    let mean = rows.iter().fold(px(0.), |sum, row| sum + *row) / rows.len() as f32;
    assert!(
        (estimate - mean).abs() <= px(6.),
        "estimate {estimate:?}, painted mean {mean:?}"
    );
    // One jump, one frame: no settling.
    let target = 500;
    f.window
        .update(visual, |view, _, cx| {
            view.column_lists[&column].scroll_to_reveal_item(target);
            cx.notify();
        })
        .unwrap();
    visual.refresh().unwrap();
    let number = number_of(&f, visual, &ids[target]);
    let viewport = f
        .window
        .read_with(visual, |view, _| {
            view.column_lists[&column].viewport_bounds()
        })
        .unwrap();
    let card = visual
        .debug_bounds(selector(format!("board-card-{number}")))
        .unwrap_or_else(|| panic!("card {number} is not painted after the jump"));
    // One uniform guess can't know which of the cards above it carry an
    // extra chip row, so allow one row's worth (24px) out of view; the
    // reveal's settling frames scroll the rest in. A guess as far off as the
    // old fixed one leaves the card hundreds of pixels away, unpainted.
    let slack = px(24.);
    assert!(
        card.top() >= viewport.top() - slack && card.bottom() <= viewport.bottom() + slack,
        "card {number} at {card:?} is outside {viewport:?}"
    );
}

#[gpui::test]
fn a_far_reveal_on_a_fresh_comfortable_board_lands_on_target(cx: &mut TestAppContext) {
    far_reveal_lands(cx, None);
}

#[gpui::test]
fn a_far_reveal_on_a_fresh_compact_board_lands_on_target(cx: &mut TestAppContext) {
    far_reveal_lands(cx, Some(r#"{"density":"compact"}"#));
}

#[gpui::test]
fn a_far_reveal_on_a_fresh_bare_face_board_lands_on_target(cx: &mut TestAppContext) {
    far_reveal_lands(
        cx,
        Some(
            r#"{"face":{"labels":false,"subtasks":false,"linkStatus":false,"priority":false,"due":false,"comments":false,"number":true}}"#,
        ),
    );
}
