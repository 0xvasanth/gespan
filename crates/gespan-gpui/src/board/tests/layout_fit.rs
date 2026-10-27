//! Cards, list rows and table rows fill the space they are given, whatever
//! their titles: no card sized to its title, no row cut off mid-view.
use super::*;
use crate::board::{column::ITEM_GAP, table::TABLE_INSET};

const LONG_WORD: &str = "dummydummydummydummydummydummydummydummydummydummydummydummy";

fn bounds(visual: &mut VisualTestContext, target: String) -> Bounds<gpui::Pixels> {
    visual.refresh().unwrap();
    visual
        .debug_bounds(selector(target.clone()))
        .unwrap_or_else(|| panic!("{target} is not on screen"))
}

fn show(f: &Fixture, cx: &mut TestAppContext, layout: list::Layout) -> VisualTestContext {
    f.window
        .update(cx, |view, _, cx| view.set_layout(layout, cx))
        .unwrap();
    cx.run_until_parked();
    VisualTestContext::from_window(f.window.into(), cx)
}

#[gpui::test]
fn cards_take_the_column_width_whatever_their_titles(cx: &mut TestAppContext) {
    let f = seeded(cx, &["first work", "hibkjhbk", LONG_WORD]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let column = bounds(visual, "board-column-0".into());
    let cards: Vec<_> = (1..=3)
        .map(|number| bounds(visual, format!("board-card-{number}")))
        .collect();
    for (number, card) in (1..=3).zip(&cards) {
        assert_eq!(
            card.size.width, cards[0].size.width,
            "card #{number} is as wide as the others"
        );
        assert!(
            card.left() >= column.left() && card.right() <= column.right(),
            "card #{number} stays inside its column: {card:?} in {column:?}"
        );
    }
    // Only the gaps either side separate a card from its column's edges.
    assert!(
        cards[0].size.width >= column.size.width - px(4. * ITEM_GAP),
        "a short title does not shrink the card: {:?} in {column:?}",
        cards[0]
    );
}

#[gpui::test]
fn list_rows_and_group_headers_span_the_list(cx: &mut TestAppContext) {
    let f = seeded(cx, &["first work", "hibkjhbk", LONG_WORD]);
    let visual = &mut show(&f, cx, list::Layout::List);
    let list = bounds(visual, "board-list".into());
    let header = bounds(visual, "board-list-group-0".into());
    assert_eq!(header.size.width, list.size.width, "the group header");
    for number in 1..=3 {
        let row = bounds(visual, format!("board-card-{number}"));
        assert_eq!(row.left(), list.left(), "row #{number} starts at the edge");
        assert_eq!(
            row.size.width, list.size.width,
            "row #{number} spans the list"
        );
    }
}

#[gpui::test]
fn table_rows_fill_the_view_with_titles_inset_from_the_edge(cx: &mut TestAppContext) {
    let f = seeded(cx, &["first work", "hibkjhbk", LONG_WORD]);
    let visual = &mut show(&f, cx, list::Layout::Table);
    let table = bounds(visual, "board-table".into());
    let header = bounds(visual, "board-table-header".into());
    assert_eq!(header.size.width, table.size.width, "the header row");
    for number in 1..=3 {
        let row = bounds(visual, format!("board-card-{number}"));
        assert_eq!(
            row.size.width, table.size.width,
            "row #{number} fills the table"
        );
        let title = bounds(visual, format!("board-table-cell-title-{number}"));
        assert!(
            title.left() >= table.left() + px(TABLE_INSET),
            "row #{number}'s title is inset from the table's edge"
        );
        let heading = bounds(visual, "board-table-col-title".into());
        assert_eq!(
            title.left(),
            heading.left(),
            "the title lines up with its heading"
        );
        assert_eq!(title.size.width, heading.size.width);
    }
    // Columns keep the widths they were given, so resizing still works on a
    // wide window; the spare room sits after the last column.
    let title = bounds(visual, "board-table-col-title".into());
    assert_eq!(title.size.width, px(320.));
    let labels = bounds(visual, "board-table-col-labels".into());
    assert!(labels.right() < table.right() - px(TABLE_INSET));
}

#[gpui::test]
fn a_narrow_table_still_scrolls_sideways_instead_of_squeezing_columns(cx: &mut TestAppContext) {
    let f = seeded(cx, &["first work"]);
    let visual = &mut show(&f, cx, list::Layout::Table);
    let wide = bounds(visual, "board-table-col-due".into()).size.width;
    visual.simulate_resize(size(px(500.), px(800.)));
    let row = bounds(visual, "board-card-1".into());
    let table = bounds(visual, "board-table".into());
    assert!(
        row.size.width > table.size.width,
        "the row keeps the columns' width"
    );
    assert_eq!(
        bounds(visual, "board-table-col-due".into()).size.width,
        wide,
        "a column keeps its width in a narrow window"
    );
}

#[gpui::test]
fn an_open_drawer_picker_floats_over_the_rows_below_and_takes_clicks(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Rich"]);
    let board = detail(&f, cx).board.id.clone();
    let card = detail(&f, cx).cards[0].id.clone();
    Service::open(&f.db)
        .unwrap()
        .create_label(&board, "Bug", "e5534b")
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(card.clone(), window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    let before: Vec<_> = ["linked", "ticket", "blocks"]
        .map(|row| bounds(visual, format!("board-drawer-{row}")))
        .into();
    let labels = bounds(visual, "board-drawer-labels".into());
    click(visual, "board-drawer-labels");
    let picker = bounds(visual, "board-picker".into());
    assert!(
        picker.top() >= labels.bottom(),
        "the picker opens under its row"
    );
    assert!(
        picker.bottom() > before[0].top(),
        "the picker overlaps the row below it: {picker:?}"
    );
    for (row, was) in ["linked", "ticket", "blocks"].iter().zip(&before) {
        assert_eq!(
            bounds(visual, format!("board-drawer-{row}")).origin,
            was.origin,
            "{row} stays put while the picker is open"
        );
    }
    // Where the picker covers the Linked work row, a click lands on the picker.
    click(visual, "board-pick-label-0");
    assert_eq!(extras_of(&f, &card).labels.len(), 1, "the label was chosen");
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("board-pick-label-0").is_some(),
        "the click went to the picker, not the row under it"
    );
}

#[gpui::test]
fn a_long_one_word_title_wraps_in_the_drawer_from_its_first_letter(cx: &mut TestAppContext) {
    let f = seeded(cx, &[LONG_WORD]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(card.clone(), window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    let drawer = bounds(visual, "board-drawer".into());
    let title = bounds(visual, "board-drawer-title".into());
    assert!(
        title.left() >= drawer.left() && title.right() <= drawer.right(),
        "the title stays inside the drawer: {title:?} in {drawer:?}"
    );
    assert!(
        title.size.height >= px(48.),
        "the title wraps onto a second line instead of scrolling: {title:?}"
    );
    // The full title is still what the field holds and saves.
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.card_detail_title(cx).as_deref(), Some(LONG_WORD))
        })
        .unwrap();
}
