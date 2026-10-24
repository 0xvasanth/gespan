use super::*;
use gpui::{
    Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};
use rebase_core::{board::Service, store::Store};
use std::{
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// A scratch folder for one test's database, removed when the test ends.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rebase-board-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    window: WindowHandle<BoardView>,
    db: PathBuf,
    store: crate::runtime::Database,
    _dir: Scratch,
}

fn open(cx: &mut TestAppContext) -> Fixture {
    cx.update(crate::input::bind);
    cx.update(|cx| cx.set_reduce_motion(true));
    let dir = Scratch::new();
    let db = dir.0.join("rebase.db");
    let store: crate::runtime::Database =
        Arc::new(Mutex::new(Store::open(Path::new(":memory:")).unwrap()));
    let (view_store, view_db) = (store.clone(), db.clone());
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1400.), px(800.)),
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| BoardView::new(view_store, view_db, window, cx)),
        )
        .unwrap()
    });
    window.update(cx, |view, _, cx| view.shown(cx)).unwrap();
    cx.run_until_parked();
    Fixture {
        window,
        db,
        store,
        _dir: dir,
    }
}

fn selector(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn click(visual: &mut VisualTestContext, target: &'static str) {
    visual.refresh().unwrap();
    let bounds = visual
        .debug_bounds(target)
        .unwrap_or_else(|| panic!("{target} is not on screen"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
}

fn detail(f: &Fixture, cx: &mut impl gpui::AppContext) -> rebase_core::board::BoardDetail {
    f.window
        .read_with(cx, |view, _| view.detail.clone().expect("a board is open"))
        .unwrap()
}

fn with_board(cx: &mut TestAppContext) -> Fixture {
    let f = open(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-create");
    f
}

#[gpui::test]
fn create_board_shows_four_default_columns_with_their_counts(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let detail = detail(&f, cx);
    let names: Vec<&str> = detail.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Backlog", "In Progress", "Review", "Done"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    for index in 0..4 {
        assert!(
            visual
                .debug_bounds(selector(format!("board-column-{index}")))
                .is_some()
        );
        assert!(
            visual
                .debug_bounds(selector(format!("board-column-count-{index}")))
                .is_some()
        );
    }
}

#[gpui::test]
fn enter_adds_a_trimmed_card_and_an_empty_title_adds_nothing(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-add-card-0");
    visual.simulate_input("  Fix login  ");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    let titles: Vec<String> = detail(&f, visual)
        .cards
        .iter()
        .map(|c| c.title.clone())
        .collect();
    assert_eq!(titles, ["Fix login"]);

    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-add-card-0");
    visual.simulate_input("   ");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(detail(&f, visual).cards.len(), 1);
}

#[gpui::test]
fn escape_closes_the_composer_without_closing_the_board(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let closed = Rc::new(std::cell::Cell::new(false));
    let seen = closed.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let entity = f.window.root(visual).unwrap();
    let _subscription = visual.update(|_, cx| {
        cx.subscribe(&entity, move |_, event: &Event, _| {
            if matches!(event, Event::Close) {
                seen.set(true);
            }
        })
    });
    click(visual, "board-add-card-1");
    visual.simulate_input("Draft");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| assert!(view.composer.is_none()))
        .unwrap();
    assert!(!closed.get());
    assert!(detail(&f, visual).cards.is_empty());
}

#[gpui::test]
fn column_delete_is_disabled_while_the_column_has_cards(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let backlog = detail(&f, cx).columns[0].id.clone();
    Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Keep me", "")
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-column-menu-0");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.delete_blocked(&backlog).as_deref(),
                Some("Column has 1 card")
            );
        })
        .unwrap();
    click(visual, "board-column-delete");
    assert_eq!(detail(&f, visual).columns.len(), 4);
    f.window
        .read_with(visual, |view, _| {
            assert!(view.menu.is_some(), "a blocked delete keeps the menu")
        })
        .unwrap();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();

    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-column-menu-3");
    click(visual, "board-column-delete");
    let names: Vec<String> = detail(&f, visual)
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["Backlog", "In Progress", "Review"]);
}

#[gpui::test]
fn a_write_from_another_connection_appears_after_the_poll_tick(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let review = detail(&f, cx).columns[2].id.clone();
    Service::open(&f.db)
        .unwrap()
        .card_create(&review, "From Tauri", "")
        .unwrap();
    cx.run_until_parked();
    assert!(detail(&f, cx).cards.is_empty(), "no reload before the tick");
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let titles: Vec<String> = detail(&f, cx)
        .cards
        .iter()
        .map(|c| c.title.clone())
        .collect();
    assert_eq!(titles, ["From Tauri"]);
}

#[gpui::test]
fn a_poll_during_a_drag_waits_until_the_drag_ends(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let review = detail(&f, cx).columns[2].id.clone();
    f.window
        .update(cx, |view, _, cx| view.set_dragging(true, cx))
        .unwrap();
    Service::open(&f.db)
        .unwrap()
        .card_create(&review, "Mid-drag", "")
        .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(detail(&f, cx).cards.is_empty());
    f.window
        .read_with(cx, |view, _| assert!(view.pending_reload))
        .unwrap();
    f.window
        .update(cx, |view, _, cx| view.set_dragging(false, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(detail(&f, cx).cards.len(), 1);
    f.window
        .read_with(cx, |view, _| assert!(!view.pending_reload))
        .unwrap();
}

#[gpui::test]
fn a_session_link_shows_its_title_and_an_unknown_one_shows_it_is_gone(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let session = f.store.lock().unwrap().create("codex", "/work").unwrap();
    let persisted = f
        .store
        .lock()
        .unwrap()
        .persisted_session_id(session.id)
        .unwrap();
    let backlog = detail(&f, cx).columns[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    let linked = service.card_create(&backlog, "Linked", "").unwrap();
    service
        .card_link(&linked.id, Some("session"), Some(&persisted))
        .unwrap();
    let gone = service.card_create(&backlog, "Gone", "").unwrap();
    service
        .card_link(&gone.id, Some("session"), Some("deleted-session"))
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    f.window
        .read_with(cx, |view, _| {
            assert_eq!(
                view.link_label(&linked.id),
                Some(LinkLabel::Found(session.title.clone()))
            );
            assert_eq!(view.link_label(&gone.id), Some(LinkLabel::Missing));
        })
        .unwrap();
}

fn center(visual: &mut VisualTestContext, target: &'static str) -> gpui::Point<gpui::Pixels> {
    visual.refresh().unwrap();
    visual
        .debug_bounds(target)
        .unwrap_or_else(|| panic!("{target} is not on screen"))
        .center()
}

/// Press on `from`, move in steps to `to` with the button held, and release.
fn drag(
    visual: &mut VisualTestContext,
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
) {
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    for step in 1..=4 {
        let t = step as f32 / 4.;
        let at = gpui::point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        visual.simulate_mouse_move(at, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
    }
    visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
}

fn seeded(cx: &mut TestAppContext, titles: &[&str]) -> Fixture {
    let f = with_board(cx);
    let backlog = detail(&f, cx).columns[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    for title in titles {
        service.card_create(&backlog, title, "").unwrap();
    }
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    cx.run_until_parked();
    f
}

fn column_of(f: &Fixture, cx: &mut impl gpui::AppContext, title: &str) -> String {
    let detail = detail(f, cx);
    let card = detail.cards.iter().find(|c| c.title == title).unwrap();
    detail
        .columns
        .iter()
        .find(|c| c.id == card.column_id)
        .unwrap()
        .name
        .clone()
}

#[gpui::test]
fn dragging_a_card_onto_another_column_moves_it_there(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Ship it"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let from = center(visual, "board-card-1");
    let to = center(visual, "board-column-body-2");
    drag(visual, from, to);
    assert_eq!(column_of(&f, visual, "Ship it"), "Review");
    f.window
        .read_with(visual, |view, _| {
            assert!(!view.dragging);
            assert!(view.drop_target.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn dropping_a_card_back_in_its_place_writes_nothing(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Stay"]);
    let board = detail(&f, cx).board.id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let from = center(visual, "board-card-1");
    drag(visual, from, gpui::point(from.x + px(30.), from.y + px(2.)));
    let kinds: Vec<String> = Service::open(&f.db)
        .unwrap()
        .activity(&board, None, 20)
        .unwrap()
        .into_iter()
        .map(|row| row.kind)
        .collect();
    assert!(!kinds.contains(&"moved".to_string()), "{kinds:?}");
}

#[gpui::test]
fn move_to_done_moves_the_card_as_the_user(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Finish"]);
    let board = detail(&f, cx).board.id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-menu-1");
    click(visual, "board-move-to-3");
    assert_eq!(column_of(&f, visual, "Finish"), "Done");
    let moved = Service::open(&f.db)
        .unwrap()
        .activity(&board, None, 20)
        .unwrap()
        .into_iter()
        .find(|row| row.kind == "moved")
        .expect("a moved row");
    assert_eq!(moved.actor, "user");
}

#[gpui::test]
fn a_drop_on_a_column_deleted_mid_drag_reloads_and_reports_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Orphan?"]);
    let review = detail(&f, cx).columns[2].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let from = center(visual, "board-card-1");
    let to = center(visual, "board-column-body-2");
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    for step in 1..=4 {
        let t = step as f32 / 4.;
        let at = gpui::point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        visual.simulate_mouse_move(at, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
    }
    Service::open(&f.db)
        .unwrap()
        .column_delete(&review)
        .unwrap();
    visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(column_of(&f, visual, "Orphan?"), "Backlog");
    f.window
        .read_with(visual, |view, _| {
            assert!(view.error.is_some(), "the failed move is reported");
            assert_eq!(view.detail.as_ref().unwrap().columns.len(), 3);
        })
        .unwrap();
}

#[gpui::test]
fn dragging_a_column_header_reorders_the_columns(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    let backlog = visual.debug_bounds("board-column-0").unwrap();
    let from = center(visual, "board-column-header-3");
    let to = gpui::point(backlog.left() + px(20.), backlog.top() + px(60.));
    drag(visual, from, to);
    let names: Vec<String> = detail(&f, visual)
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["Done", "Backlog", "In Progress", "Review"]);
}
