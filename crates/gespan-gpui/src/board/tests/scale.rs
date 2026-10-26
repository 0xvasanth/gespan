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
fn seeded_board(cx: &mut TestAppContext, one_column: bool) -> Fixture {
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

fn reload_view(f: &Fixture, cx: &mut impl gpui::AppContext) {
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

fn column_at(f: &Fixture, cx: &mut impl gpui::AppContext, index: usize) -> String {
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
fn list_ids(f: &Fixture, cx: &mut impl gpui::AppContext, column: &str) -> Vec<String> {
    f.window
        .read_with(cx, |view, _| view.column_ids[column].clone())
        .unwrap()
}

fn list_top(f: &Fixture, cx: &mut impl gpui::AppContext, column: &str) -> gpui::ListOffset {
    f.window
        .read_with(cx, |view, _| view.column_lists[column].logical_scroll_top())
        .unwrap()
}

fn scroll_list_to(f: &Fixture, visual: &mut VisualTestContext, column: &str, item_ix: usize) {
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

fn number_of(f: &Fixture, cx: &mut impl gpui::AppContext, card: &str) -> i64 {
    detail(f, cx)
        .cards
        .iter()
        .find(|c| c.id == card)
        .unwrap()
        .number
}

fn card_bounds(visual: &mut VisualTestContext, number: i64) -> gpui::Bounds<gpui::Pixels> {
    visual
        .debug_bounds(selector(format!("board-card-{number}")))
        .unwrap_or_else(|| panic!("card {number} is not painted"))
}

/// A column's card ids in stored order (every card, hidden or not).
fn stored_order(f: &Fixture, column: &str) -> Vec<String> {
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

fn wheel(visual: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>, dy: f32) {
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
