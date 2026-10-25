use super::detail::activity_text;
use super::*;
use gespan_core::{board::Service, store::Store};
use gpui::{
    Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};
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
            "gespan-board-{}-{}",
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
    cx.update(crate::popover::bind);
    cx.update(|cx| cx.set_reduce_motion(true));
    let dir = Scratch::new();
    let db = dir.0.join("gespan.db");
    let store: crate::runtime::Database = Arc::new(Mutex::new(
        Store::open_gespan(Path::new(":memory:")).unwrap(),
    ));
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

fn detail(f: &Fixture, cx: &mut impl gpui::AppContext) -> gespan_core::board::BoardDetail {
    f.window
        .read_with(cx, |view, _| view.detail.clone().expect("a board is open"))
        .unwrap()
}

fn with_board(cx: &mut TestAppContext) -> Fixture {
    open(cx)
}

#[gpui::test]
fn the_board_opens_with_four_steps_and_a_board_switcher(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let detail = detail(&f, cx);
    let names: Vec<&str> = detail.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Backlog", "In Progress", "Review", "Done"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-header").is_some());
    assert!(visual.debug_bounds("board-switcher").is_some());
    assert!(visual.debug_bounds("board-new").is_none());
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

fn boards(f: &Fixture) -> Vec<gespan_core::board::Board> {
    Service::open(&f.db).unwrap().list().unwrap()
}

fn open_id(f: &Fixture, cx: &mut impl gpui::AppContext) -> String {
    f.window
        .read_with(cx, |view, _| {
            view.open_board_id.clone().expect("a board is open")
        })
        .unwrap()
}

fn open_name(f: &Fixture, cx: &mut impl gpui::AppContext) -> String {
    detail(f, cx).board.name
}

/// A second view over the same database and settings, as after a restart.
fn reopen(f: &Fixture, cx: &mut TestAppContext) -> WindowHandle<BoardView> {
    let (store, db) = (f.store.clone(), f.db.clone());
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| BoardView::new(store, db, window, cx))
        })
        .unwrap()
    });
    window.update(cx, |view, _, cx| view.shown(cx)).unwrap();
    cx.run_until_parked();
    window
}

fn saved_board(f: &Fixture) -> Option<String> {
    crate::runtime::locked(&f.store)
        .unwrap()
        .setting("board-open")
        .unwrap()
}

#[gpui::test]
fn the_board_switcher_lists_boards_and_opens_one(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let first = open_id(&f, cx);
    let sprint = Service::open(&f.db).unwrap().create("Sprint").unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    assert!(visual.debug_bounds("board-switch-0").is_some());
    assert!(visual.debug_bounds("board-switch-1").is_some());
    assert!(visual.debug_bounds("board-switch-2").is_none());
    click(visual, "board-switch-1");
    assert_eq!(open_id(&f, visual), sprint.id);
    assert_eq!(open_name(&f, visual), "Sprint");
    assert_eq!(saved_board(&f), Some(sprint.id.clone()));
    assert!(
        visual.debug_bounds("board-switch-1").is_none(),
        "picking a board closes the menu"
    );
    // The poll and later reloads keep the selection.
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    assert_eq!(open_id(&f, visual), sprint.id);
    click(visual, "board-switcher");
    click(visual, "board-switch-0");
    assert_eq!(open_id(&f, visual), first);
}

#[gpui::test]
fn a_new_board_opens_and_is_remembered_after_reload(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    click(visual, "board-new");
    visual.simulate_input("  Sprint  ");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(open_name(&f, visual), "Sprint");
    assert_eq!(boards(&f).len(), 2);
    let again = reopen(&f, visual);
    let name = again
        .read_with(visual, |view, _| view.open_board_id.clone())
        .unwrap();
    assert_eq!(name, Some(open_id(&f, visual)));
}

#[gpui::test]
fn a_blank_or_too_long_new_board_name_is_rejected_inline(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    click(visual, "board-new");
    visual.simulate_input("   ");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(boards(&f).len(), 1, "a blank name creates nothing");
    assert!(
        visual.debug_bounds("board-new-name").is_some(),
        "the field stays"
    );
    retype(visual, "board-new-name", &"x".repeat(201));
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(boards(&f).len(), 1);
    assert!(visual.debug_bounds("board-switcher-error").is_some());
    assert!(
        visual.debug_bounds("board-new-name").is_some(),
        "the name stays to correct"
    );
    retype(visual, "board-new-name", "Fine");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(open_name(&f, visual), "Fine");
    assert!(visual.debug_bounds("board-switcher-error").is_none());
}

#[gpui::test]
fn renaming_the_open_board_updates_the_switcher(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let id = open_id(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    click(visual, "board-rename");
    retype(visual, "board-rename-name", "  Roadmap ");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(open_name(&f, visual), "Roadmap");
    assert_eq!(open_id(&f, visual), id);
    assert_eq!(boards(&f)[0].name, "Roadmap");
    click(visual, "board-switcher");
    click(visual, "board-rename");
    retype(visual, "board-rename-name", "");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(
        boards(&f)[0].name,
        "Roadmap",
        "a blank name changes nothing"
    );
}

#[gpui::test]
fn deleting_the_open_board_opens_another(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let first = open_id(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    click(visual, "board-new");
    visual.simulate_input("Sprint");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    let sprint = open_id(&f, visual);
    click(visual, "board-switcher");
    click(visual, "board-delete");
    assert_eq!(boards(&f).len(), 2, "the first press only asks");
    click(visual, "board-delete-confirm");
    assert_eq!(boards(&f).len(), 1);
    assert_eq!(open_id(&f, visual), first);
    assert!(detail(&f, visual).columns.len() == 4, "a board is shown");
    assert_ne!(saved_board(&f), Some(sprint));
}

#[gpui::test]
fn deleting_the_last_board_creates_a_default_one(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let old = open_id(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    click(visual, "board-delete");
    click(visual, "board-delete-confirm");
    let now = boards(&f);
    assert_eq!(now.len(), 1);
    assert_ne!(now[0].id, old);
    assert_eq!(open_id(&f, visual), now[0].id);
    assert_eq!(detail(&f, visual).columns.len(), 4);
}

#[gpui::test]
fn the_delete_confirmation_goes_away_with_the_menu(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-switcher");
    click(visual, "board-delete");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    click(visual, "board-switcher");
    assert!(visual.debug_bounds("board-delete-confirm").is_none());
    assert_eq!(boards(&f).len(), 1);
}

#[gpui::test]
fn a_remembered_board_that_no_longer_exists_falls_back(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let first = open_id(&f, cx);
    crate::runtime::locked(&f.store)
        .unwrap()
        .save_setting("board-open", "not-a-board")
        .unwrap();
    let again = reopen(&f, cx);
    assert_eq!(
        again
            .read_with(cx, |view, _| view.open_board_id.clone())
            .unwrap(),
        Some(first)
    );
}

#[gpui::test]
fn a_board_deleted_elsewhere_falls_back_on_reload(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let first = open_id(&f, cx);
    let mut service = Service::open(&f.db).unwrap();
    let sprint = service.create("Sprint").unwrap();
    f.window
        .update(cx, |view, _, cx| view.open_board(sprint.id.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(open_id(&f, cx), sprint.id);
    service.delete(&sprint.id).unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    assert_eq!(open_id(&f, cx), first);
}

#[gpui::test]
fn switching_boards_clears_search_filter_drawer_and_menus(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Alpha", "Beta"]);
    let sprint = Service::open(&f.db).unwrap().create("Sprint").unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-search");
    visual.simulate_input("alpha");
    visual.run_until_parked();
    click(visual, "board-card-1");
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.query(cx), "alpha");
            assert!(view.card_detail.is_some(), "the drawer is open");
        })
        .unwrap();
    f.window
        .update(visual, |view, _, _| {
            view.filter.priorities.insert(0);
        })
        .unwrap();
    click(visual, "board-switcher");
    click(visual, "board-switch-1");
    assert_eq!(open_id(&f, visual), sprint.id);
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.query(cx), "");
            assert_eq!(view.filter.len(), 0);
            assert!(view.card_detail.is_none());
        })
        .unwrap();
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

    // The composer stays open; a blank title adds nothing.
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
                Some("Move or delete its 1 card first")
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
fn a_write_that_lands_while_the_board_reloads_still_appears_on_the_next_tick(
    cx: &mut TestAppContext,
) {
    let f = with_board(cx);
    let review = detail(&f, cx).columns[2].id.clone();
    let db = f.db.clone();
    let mut once = Some(review);
    f.window
        .update(cx, |view, _, cx| {
            view.after_load_reads = Some(Box::new(move || {
                if let Some(review) = once.take() {
                    Service::open(&db)
                        .unwrap()
                        .card_create(&review, "Mid-reload", "")
                        .unwrap();
                }
            }));
            view.reload(cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(
        detail(&f, cx).cards.is_empty(),
        "the reload read the board before the write"
    );
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let titles: Vec<String> = detail(&f, cx)
        .cards
        .iter()
        .map(|c| c.title.clone())
        .collect();
    assert_eq!(titles, ["Mid-reload"]);
}

#[gpui::test]
fn a_failed_reload_shows_its_error_until_a_later_reload_succeeds(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let raw = rusqlite::Connection::open(&f.db).unwrap();
    raw.execute_batch("ALTER TABLE board_labels RENAME TO hidden_labels;")
        .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-error").is_some());
    raw.execute_batch("ALTER TABLE hidden_labels RENAME TO board_labels;")
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("board-error").is_none(),
        "the banner clears once the board loads again"
    );
}

#[gpui::test]
fn a_card_deleted_elsewhere_leaves_no_focus_handle_or_anchor_behind(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Keep", "Gone"]);
    let gone = detail(&f, cx)
        .cards
        .iter()
        .find(|c| c.title == "Gone")
        .unwrap()
        .id
        .clone();
    f.window
        .read_with(cx, |view, _| {
            assert!(view.card_focus.contains_key(&gone));
            assert!(view.card_anchors.contains_key(&gone));
        })
        .unwrap();
    Service::open(&f.db).unwrap().card_delete(&gone).unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    f.window
        .read_with(cx, |view, _| {
            assert_eq!(view.card_focus.len(), 1);
            assert!(!view.card_focus.contains_key(&gone));
            assert!(!view.card_anchors.contains_key(&gone));
        })
        .unwrap();
}

#[gpui::test]
fn a_poll_during_a_drag_waits_until_the_drag_ends(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Held"]);
    let review = detail(&f, cx).columns[2].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let from = center(visual, "board-card-1");
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    for step in 1..=3 {
        let at = gpui::point(from.x + px(4. * step as f32), from.y + px(4. * step as f32));
        visual.simulate_mouse_move(at, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
    }
    Service::open(&f.db)
        .unwrap()
        .card_create(&review, "Mid-drag", "")
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    assert_eq!(detail(&f, visual).cards.len(), 1, "no reload mid-drag");
    f.window
        .read_with(visual, |view, _| {
            assert!(view.dragging && view.pending_reload)
        })
        .unwrap();
    visual.simulate_mouse_up(from, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(detail(&f, visual).cards.len(), 2);
    f.window
        .read_with(visual, |view, _| assert!(!view.pending_reload))
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
    click(visual, "menu-action-move-3");
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

fn events(
    f: &Fixture,
    visual: &mut VisualTestContext,
) -> (Rc<std::cell::RefCell<Vec<String>>>, gpui::Subscription) {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    let entity = f.window.root(visual).unwrap();
    let subscription = visual.update(|_, cx| {
        cx.subscribe(&entity, move |_, event: &Event, _| {
            log.borrow_mut().push(match event {
                Event::Close => "close".to_string(),
                Event::OpenSession(id) => format!("session:{id}"),
                Event::OpenPilot(id) => format!("pilot:{id}"),
                Event::LinkTicket(id) => format!("link-ticket:{id}"),
            })
        })
    });
    (seen, subscription)
}

fn linked(cx: &mut TestAppContext, target: Option<&str>) -> (Fixture, i64) {
    let f = seeded(cx, &["Linked"]);
    let session = f.store.lock().unwrap().create("codex", "/work").unwrap().id;
    let persisted = f
        .store
        .lock()
        .unwrap()
        .persisted_session_id(session)
        .unwrap();
    assert_ne!(
        persisted,
        session.to_string(),
        "links store the Tauri session id"
    );
    let card = detail(&f, cx).cards[0].id.clone();
    Service::open(&f.db)
        .unwrap()
        .card_link(&card, Some("session"), Some(target.unwrap_or(&persisted)))
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    cx.run_until_parked();
    (f, session)
}

#[gpui::test]
fn a_card_linked_in_tauri_form_opens_its_session(cx: &mut TestAppContext) {
    let (f, session) = linked(cx, None);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (seen, _subscription) = events(&f, visual);
    click(visual, "board-card-1");
    click(visual, "board-open-linked");
    assert_eq!(*seen.borrow(), [format!("session:{session}")]);
}

#[gpui::test]
fn a_missing_session_says_so_and_offers_no_open(cx: &mut TestAppContext) {
    let (f, _) = linked(cx, Some("deleted-session"));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-link-missing").is_some());
    assert!(visual.debug_bounds("board-open-linked").is_none());
}

#[gpui::test]
fn declining_the_delete_confirm_keeps_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Keep"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-more");
    click(visual, "menu-action-delete");
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    assert_eq!(detail(&f, visual).cards.len(), 1);
    click(visual, "board-drawer-more");
    click(visual, "menu-action-delete");
    visual.simulate_prompt_answer("Delete");
    visual.run_until_parked();
    assert!(detail(&f, visual).cards.is_empty());
    f.window
        .read_with(visual, |view, _| assert!(view.card_detail.is_none()))
        .unwrap();
}

#[gpui::test]
fn a_failed_activity_load_says_so(cx: &mut TestAppContext) {
    let f = seeded(cx, &["History"]);
    // A database path inside a plain file can never be opened.
    let blocker = f._dir.0.join("not-a-folder");
    std::fs::write(&blocker, b"").unwrap();
    f.window
        .update(cx, |view, _, _| {
            view.service = None;
            view.db_path = blocker.join("gespan.db");
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-tab-activity");
    assert!(visual.debug_bounds("board-activity-failed").is_some());
}

#[gpui::test]
fn linking_from_the_picker_stores_the_persisted_session_id(cx: &mut TestAppContext) {
    let f = seeded(cx, &["To link"]);
    let session = f.store.lock().unwrap().create("codex", "/work").unwrap().id;
    let persisted = f
        .store
        .lock()
        .unwrap()
        .persisted_session_id(session)
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-linked");
    click(visual, "board-link-target-0");
    let card = detail(&f, visual).cards[0].clone();
    assert_eq!(card.link_kind.as_deref(), Some("session"));
    assert_eq!(card.link_id.as_deref(), Some(persisted.as_str()));
}

#[test]
fn a_move_reads_from_and_to_with_actor() {
    let row = gespan_core::board::Activity {
        id: "a".into(),
        board_id: "b".into(),
        card_id: Some("c".into()),
        kind: "moved".into(),
        actor: "system".into(),
        from_column: Some("In Progress".into()),
        to_column: Some("Review".into()),
        detail: None,
        created_at: 0,
    };
    assert_eq!(
        activity_text(&row),
        "Moved from In Progress to Review · system"
    );
}

#[gpui::test]
fn a_drag_released_off_the_board_does_not_freeze_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Loose"]);
    let backlog = detail(&f, cx).columns[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let from = center(visual, "board-card-1");
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    for step in 1..=4 {
        let at = gpui::point(
            from.x + px(15. * step as f32),
            from.y + px(15. * step as f32),
        );
        visual.simulate_mouse_move(at, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
    }
    visual.simulate_mouse_up(
        gpui::point(px(-50.), px(-50.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    visual.run_until_parked();
    Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Made elsewhere", "")
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    assert_eq!(detail(&f, visual).cards.len(), 2);
    f.window
        .read_with(visual, |view, _| {
            assert!(!view.dragging);
            assert!(view.dragged_card.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn closing_the_card_detail_keeps_an_edit_made_elsewhere(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Original"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    Service::open(&f.db)
        .unwrap()
        .card_update(&card, "Edited in Tauri", "notes from Tauri")
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    click(visual, "board-drawer-close");
    let after = detail(&f, visual).cards[0].clone();
    assert_eq!(after.title, "Edited in Tauri");
    assert_eq!(after.body, "notes from Tauri");
}

#[gpui::test]
fn an_unedited_detail_shows_an_edit_made_elsewhere(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Original"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    Service::open(&f.db)
        .unwrap()
        .card_update(&card, "Edited in Tauri", "")
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    let shown = f
        .window
        .read_with(visual, |view, cx| view.card_detail_title(cx))
        .unwrap();
    assert_eq!(shown.as_deref(), Some("Edited in Tauri"));
}

#[gpui::test]
fn a_focused_card_moves_with_the_keyboard(cx: &mut TestAppContext) {
    let f = seeded(cx, &["By keys"]);
    let board = detail(&f, cx).board.id.clone();
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    f.window
        .update(visual, |view, window, cx| {
            let handle = view.card_focus.get(&card).unwrap().clone();
            window.focus(&handle, cx);
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    // Open card, then Move to In Progress, Review, Done.
    visual.simulate_keystrokes("down down down enter");
    visual.run_until_parked();
    assert_eq!(column_of(&f, visual, "By keys"), "Done");
    let moved = Service::open(&f.db)
        .unwrap()
        .activity(&board, None, 20)
        .unwrap()
        .into_iter()
        .find(|row| row.kind == "moved")
        .expect("a moved row");
    assert_eq!(moved.actor, "user");
}

fn focus_card(f: &Fixture, visual: &mut VisualTestContext, card: &str) {
    f.window
        .update(visual, |view, window, cx| {
            let handle = view.card_focus.get(card).unwrap().clone();
            window.focus(&handle, cx);
        })
        .unwrap();
}

fn card_id(f: &Fixture, cx: &mut impl gpui::AppContext, title: &str) -> String {
    detail(f, cx)
        .cards
        .iter()
        .find(|c| c.title == title)
        .unwrap()
        .id
        .clone()
}

fn focused_card(f: &Fixture, visual: &mut VisualTestContext) -> Option<String> {
    f.window
        .update(visual, |view, window, _| {
            view.card_focus
                .iter()
                .find(|(_, handle)| handle.is_focused(window))
                .map(|(id, _)| id.clone())
        })
        .unwrap()
}

/// Card titles of a column in position order, read from the store.
fn titles_in(f: &Fixture, cx: &mut impl gpui::AppContext, column: &str) -> Vec<String> {
    let detail = detail(f, cx);
    let id = detail
        .columns
        .iter()
        .find(|c| c.name == column)
        .unwrap()
        .id
        .clone();
    let mut cards: Vec<_> = detail.cards.iter().filter(|c| c.column_id == id).collect();
    cards.sort_by_key(|c| c.position);
    cards.iter().map(|c| c.title.clone()).collect()
}

fn moved_rows(f: &Fixture, cx: &mut impl gpui::AppContext) -> usize {
    let board = detail(f, cx).board.id.clone();
    Service::open(&f.db)
        .unwrap()
        .activity(&board, None, 50)
        .unwrap()
        .iter()
        .filter(|row| row.kind == "moved")
        .count()
}

#[gpui::test]
fn arrows_move_focus_between_cards_and_columns(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let in_progress = detail(&f, cx).columns[1].id.clone();
    let (a, b, c) = (
        card_id(&f, cx, "A"),
        card_id(&f, cx, "B"),
        card_id(&f, cx, "C"),
    );
    {
        let mut service = Service::open(&f.db).unwrap();
        service.card_create(&in_progress, "X", "").unwrap();
        service.card_create(&in_progress, "Y", "").unwrap();
    }
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let (x, y) = (card_id(&f, cx, "X"), card_id(&f, cx, "Y"));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("down");
    assert_eq!(focused_card(&f, visual), Some(b.clone()));
    visual.simulate_keystrokes("down down");
    assert_eq!(
        focused_card(&f, visual),
        Some(c.clone()),
        "clamped at the last card"
    );
    visual.simulate_keystrokes("up up up");
    assert_eq!(
        focused_card(&f, visual),
        Some(a.clone()),
        "clamped at the first card"
    );
    // Same index in the next column, clamped to its length.
    visual.simulate_keystrokes("down right");
    assert_eq!(focused_card(&f, visual), Some(y.clone()));
    visual.simulate_keystrokes("left");
    assert_eq!(focused_card(&f, visual), Some(b.clone()));
    // From C (index 2) the next column has two cards: clamp to the last.
    visual.simulate_keystrokes("down right");
    assert_eq!(focused_card(&f, visual), Some(y));
    visual.simulate_keystrokes("up");
    assert_eq!(focused_card(&f, visual), Some(x.clone()));
    visual.simulate_keystrokes("left");
    assert_eq!(focused_card(&f, visual), Some(a), "index 0 in Backlog");
    // The columns to the right of In Progress are empty: focus stays put.
    visual.simulate_keystrokes("right right right");
    assert_eq!(focused_card(&f, visual), Some(x));
    assert_eq!(moved_rows(&f, visual), 0, "moving focus writes nothing");
}

#[gpui::test]
fn space_lifts_arrows_move_and_space_drops_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let a = card_id(&f, cx, "A");
    let board = detail(&f, cx).board.id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.drop_target.as_ref().map(|t| t.1),
                Some(0),
                "line at the card"
            );
        })
        .unwrap();
    // Down once lands after B (A's own slot and the one after it are the same).
    visual.simulate_keystrokes("down");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.drop_target.as_ref().map(|t| t.1), Some(2));
        })
        .unwrap();
    assert_eq!(moved_rows(&f, visual), 0, "nothing is written while lifted");
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "A", "C"]);
    assert_eq!(moved_rows(&f, visual), 1);
    // Lift again, jump a column right and drop with Enter; no card actions open.
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space right enter");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "C"]);
    assert_eq!(titles_in(&f, visual, "In Progress"), ["A"]);
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
            assert!(view.card_menu.is_none());
            assert!(
                view.card_detail.is_none(),
                "dropping never opens the drawer"
            );
        })
        .unwrap();
    let moved = Service::open(&f.db)
        .unwrap()
        .activity(&board, None, 50)
        .unwrap()
        .into_iter()
        .find(|row| row.kind == "moved")
        .unwrap();
    assert_eq!(moved.actor, "user");
}

#[gpui::test]
fn dropping_a_lifted_card_where_it_started_writes_nothing(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let b = card_id(&f, cx, "B");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &b);
    visual.simulate_keystrokes("space space");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B"]);
    assert_eq!(moved_rows(&f, visual), 0);
}

#[gpui::test]
fn escape_cancels_a_keyboard_drag_without_writing(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let (seen, _subscription) = {
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        events(&f, visual)
    };
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space down right escape");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B"]);
    assert_eq!(moved_rows(&f, visual), 0);
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
        })
        .unwrap();
    assert!(
        seen.borrow().is_empty(),
        "the same Escape does not close the board"
    );
    // A second Escape, with nothing lifted, closes it as before.
    visual.simulate_keystrokes("escape");
    assert_eq!(*seen.borrow(), ["close"]);
}

#[gpui::test]
fn a_reload_during_a_keyboard_drag_keeps_the_lifted_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let backlog = detail(&f, cx).columns[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space down");
    // Another window writes a new card while A is lifted.
    Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Fresh", "")
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.pending_reload, "the reload waits for the drop");
            assert!(view.kb_drag.is_some());
            assert_eq!(view.detail.as_ref().unwrap().cards.len(), 2);
        })
        .unwrap();
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    // A went after B; the new card kept its place at the end.
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "A", "Fresh"]);
    f.window
        .read_with(visual, |view, _| assert!(!view.pending_reload))
        .unwrap();
}

#[gpui::test]
fn a_cancelled_keyboard_drag_catches_up_on_a_deferred_reload(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let a = card_id(&f, cx, "A");
    let backlog = detail(&f, cx).columns[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space");
    Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Fresh", "")
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.detail.as_ref().unwrap().cards.len(), 2);
            assert!(!view.pending_reload);
        })
        .unwrap();
}

#[gpui::test]
fn a_lifted_card_deleted_elsewhere_cancels_the_drag_on_the_next_poll(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space down");
    Service::open(&f.db).unwrap().card_delete(&a).unwrap();
    f.window
        .update(visual, |view, _, cx| view.poll(cx))
        .unwrap();
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none(), "no ghost drag over a deleted card");
            assert!(view.drop_target.is_none());
            assert!(!view.pending_reload);
            assert!(view.error.is_none(), "cancelled, not a failed move");
            assert_eq!(view.detail.as_ref().unwrap().cards.len(), 1, "reloaded");
        })
        .unwrap();
}

#[gpui::test]
fn clicking_another_card_cancels_a_lift_and_applies_the_deferred_reload(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let backlog = detail(&f, cx).columns[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space down");
    Service::open(&f.db)
        .unwrap()
        .card_create(&backlog, "Fresh", "")
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    click(visual, "board-card-2");
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
            assert!(!view.pending_reload);
            assert_eq!(view.detail.as_ref().unwrap().cards.len(), 3);
        })
        .unwrap();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "Fresh"]);
    assert_eq!(moved_rows(&f, visual), 0);
}

#[gpui::test]
fn focus_leaving_the_lifted_card_cancels_the_lift(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let (a, b) = (card_id(&f, cx, "A"), card_id(&f, cx, "B"));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space");
    focus_card(&f, visual, &b);
    visual.run_until_parked();
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn opening_the_drawer_cancels_a_lift(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let a = card_id(&f, cx, "A");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space");
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(a.clone(), window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.card_detail.is_some());
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn deactivating_the_window_cancels_a_lift(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let a = card_id(&f, cx, "A");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.refresh().unwrap();
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space");
    visual.deactivate_window();
    visual.run_until_parked();
    visual.refresh().unwrap();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.kb_drag.is_none() && view.drop_target.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn keyboard_drop_while_filtered_uses_the_full_slot(cx: &mut TestAppContext) {
    let f = seeded(
        cx,
        &["Alpha one", "Beta", "Alpha two", "Gamma", "Alpha three"],
    );
    let one = card_id(&f, cx, "Alpha one");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, _, cx| {
            view.search
                .update(cx, |input, cx| input.set_text("alpha".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(shown_cards(visual, &[1, 2, 3, 4, 5]), [1, 3, 5]);
    focus_card(&f, visual, &one);
    // Visible: one, two, three. One step down lands right before "Alpha three".
    visual.simulate_keystrokes("space down space");
    visual.run_until_parked();
    assert_eq!(
        titles_in(&f, visual, "Backlog"),
        ["Beta", "Alpha two", "Gamma", "Alpha one", "Alpha three"]
    );
    // Arrow focus skips the hidden cards too.
    let two = card_id(&f, visual, "Alpha two");
    focus_card(&f, visual, &two);
    visual.simulate_keystrokes("down");
    assert_eq!(focused_card(&f, visual), Some(one));
}

fn shown_cards(visual: &mut VisualTestContext, numbers: &[i64]) -> Vec<i64> {
    visual.refresh().unwrap();
    numbers
        .iter()
        .copied()
        .filter(|n| {
            visual
                .debug_bounds(selector(format!("board-card-{n}")))
                .is_some()
        })
        .collect()
}

#[gpui::test]
fn search_narrows_cards_and_finds_a_number(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Alpha", "Beta", "Gamma"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-search");
    visual.simulate_input("bet");
    visual.run_until_parked();
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [2]);
    visual.simulate_keystrokes("backspace backspace backspace");
    visual.simulate_input("#3");
    visual.run_until_parked();
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [3]);
    visual.simulate_keystrokes("backspace backspace");
    visual.run_until_parked();
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 2, 3]);
}

#[gpui::test]
fn a_project_filter_shows_tagged_cards_and_clear_restores(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Tagged", "Other", "Third"]);
    let tagged = detail(&f, cx).cards[0].id.clone();
    let backlog = detail(&f, cx).columns[0].id.clone();
    Service::open(&f.db)
        .unwrap()
        .set_projects(&tagged, &["/work/a".into()])
        .unwrap();
    f.window
        .update(cx, |view, _, cx| {
            view.set_projects(vec![("/work/a".into(), "Alpha".into())], cx);
            view.reload(cx);
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-project-0");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1]);
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.count_label(&backlog, cx), "1/3")
        })
        .unwrap();
    click(visual, "board-filter-clear");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 2, 3]);
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.count_label(&backlog, cx), "3")
        })
        .unwrap();
}

#[gpui::test]
fn new_task_opens_the_composer_in_the_first_step(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let first = detail(&f, cx).columns[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-new-task");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.composer.as_ref().map(|c| c.column_id.clone()),
                Some(first)
            )
        })
        .unwrap();
}

#[gpui::test]
fn a_rich_card_shows_its_labels_subtasks_and_due_date_and_a_bare_one_does_not(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Rich", "Bare"]);
    let board = detail(&f, cx).board.id.clone();
    let rich = detail(&f, cx).cards[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
    let docs = service.create_label(&board, "Docs", "4da3f5").unwrap();
    service.set_labels(&rich, &[bug.id, docs.id]).unwrap();
    service.set_priority(&rich, 3).unwrap();
    service
        .set_due(&rich, Some(gespan_core::time::now_ms() as i64))
        .unwrap();
    let first = service.add_subtask(&rich, "One").unwrap();
    service.add_subtask(&rich, "Two").unwrap();
    service.add_subtask(&rich, "Three").unwrap();
    service.toggle_subtask(&first.id, true).unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    for part in ["labels", "subtasks", "due", "priority"] {
        assert!(
            visual
                .debug_bounds(selector(format!("board-card-{part}-1")))
                .is_some(),
            "rich card lacks {part}"
        );
        assert!(
            visual
                .debug_bounds(selector(format!("board-card-{part}-2")))
                .is_none(),
            "bare card shows {part}"
        );
    }
}

fn activity_details(f: &Fixture, card: &str) -> Vec<String> {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service
        .activity(&board.id, Some(card), 20)
        .unwrap()
        .into_iter()
        .filter_map(|row| row.detail)
        .collect()
}

fn extras_of(f: &Fixture, card: &str) -> gespan_core::board::extras::CardExtras {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service
        .extras(&board.id)
        .unwrap()
        .remove(card)
        .unwrap_or_default()
}

#[gpui::test]
fn the_drawer_lists_every_property(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    visual.refresh().unwrap();
    for row in ["status", "priority", "due", "projects", "labels", "linked"] {
        assert!(
            visual
                .debug_bounds(selector(format!("board-drawer-{row}")))
                .is_some(),
            "no {row} row"
        );
    }
}

#[gpui::test]
fn picking_high_priority_marks_the_card_and_logs_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-priority");
    click(visual, "menu-action-priority-3");
    assert_eq!(extras_of(&f, &card).priority, 3);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-card-priority-1").is_some());
    assert!(activity_details(&f, &card).contains(&"priority: High".to_string()));
}

#[gpui::test]
fn due_tomorrow_shows_on_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-due");
    let before = chrono::Local::now().date_naive();
    click(visual, "menu-action-due-tomorrow");
    let after = chrono::Local::now().date_naive();
    let due = extras_of(&f, &card).due_at.expect("a due date");
    // The menu read "today" between these two reads, even across midnight.
    let day = super::style::local_day(due).unwrap();
    assert!(
        [before.succ_opt(), after.succ_opt()].contains(&Some(day)),
        "{day}"
    );
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-card-due-1").is_some());
}

#[gpui::test]
fn tagging_two_projects_stores_both_and_the_filter_finds_either(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Tagged", "Other"]);
    let card = detail(&f, cx).cards[0].id.clone();
    f.window
        .update(cx, |view, _, cx| {
            view.set_projects(
                vec![("/a".into(), "Alpha".into()), ("/b".into(), "Beta".into())],
                cx,
            )
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-projects");
    click(visual, "board-pick-project-0");
    click(visual, "board-pick-project-1");
    let mut projects = extras_of(&f, &card).projects;
    projects.sort();
    assert_eq!(projects, ["/a", "/b"]);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    click(visual, "board-drawer-close");
    click(visual, "board-filter");
    click(visual, "board-filter-project-1");
    assert_eq!(shown_cards(visual, &[1, 2]), [1]);
}

#[gpui::test]
fn creating_a_label_puts_its_chip_on_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-labels");
    visual.simulate_input("Bug");
    visual.run_until_parked();
    click(visual, "board-create-label");
    assert_eq!(extras_of(&f, &card).labels.len(), 1);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-card-labels-1").is_some());
}

#[gpui::test]
fn adding_and_ticking_a_subtask_fills_the_cards_progress(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-subtask-add");
    visual.simulate_input("Write tests");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    click(visual, "board-subtask-0");
    let extras = extras_of(&f, &card);
    assert_eq!((extras.subtasks_done, extras.subtasks_total), (1, 1));
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-card-subtasks-1").is_some());
}

#[gpui::test]
fn a_comment_shows_in_the_list_and_counts_on_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-comment-input");
    visual.simulate_input("Looks good");
    visual.run_until_parked();
    click(visual, "board-comment-send");
    assert_eq!(extras_of(&f, &card).comments, 1);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-comment-0").is_some());
}

#[gpui::test]
fn mark_complete_moves_the_task_to_done(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Finish me"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-complete");
    assert_eq!(column_of(&f, visual, "Finish me"), "Done");
}

#[gpui::test]
fn copy_reference_puts_the_number_and_title_on_the_clipboard(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Ship it"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-copy");
    let copied = visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert_eq!(copied, "#1 Ship it");
}

#[gpui::test]
fn the_open_drawer_never_covers_the_header(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let first = detail(&f, cx).columns[0].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    visual.refresh().unwrap();
    let header = visual.debug_bounds("board-header").unwrap();
    let drawer = visual.debug_bounds("board-drawer").unwrap();
    assert!(!header.intersects(&drawer), "{header:?} vs {drawer:?}");
    click(visual, "board-new-task");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.composer.as_ref().map(|c| c.column_id.clone()),
                Some(first)
            )
        })
        .unwrap();
}

#[gpui::test]
fn escape_closes_an_open_column_menu_before_the_drawer(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-column-menu-1");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| {
            assert!(view.menu.is_none(), "the menu closes first");
            assert!(view.card_detail.is_some(), "the drawer stays open");
        })
        .unwrap();
}

#[gpui::test]
fn a_project_no_longer_in_the_workspace_can_still_be_untagged_and_filtered(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Old tag", "Other"]);
    let card = detail(&f, cx).cards[0].id.clone();
    Service::open(&f.db)
        .unwrap()
        .set_projects(&card, &["/gone/legacy".into()])
        .unwrap();
    f.window
        .update(cx, |view, _, cx| {
            view.set_projects(vec![("/a".into(), "Alpha".into())], cx);
            view.reload(cx);
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-project-1");
    assert_eq!(shown_cards(visual, &[1, 2]), [1]);
    click(visual, "board-filter-clear");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    click(visual, "board-card-1");
    click(visual, "board-drawer-projects");
    click(visual, "board-pick-project-1");
    assert!(extras_of(&f, &card).projects.is_empty());
}

/// Select everything in a drawer field and type over it.
fn retype(visual: &mut VisualTestContext, field: &'static str, text: &str) {
    click(visual, field);
    let select_all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    visual.simulate_keystrokes(select_all);
    if text.is_empty() {
        visual.simulate_keystrokes("backspace");
    } else {
        visual.simulate_input(text);
    }
    visual.run_until_parked();
}

fn drawer<R>(
    f: &Fixture,
    cx: &mut impl gpui::AppContext,
    read: impl FnOnce(&BoardView, &App) -> R,
) -> R {
    f.window.read_with(cx, |view, cx| read(view, cx)).unwrap()
}

#[gpui::test]
fn editing_the_drawer_saves_the_title_and_description_on_close(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Original"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-title", "Renamed");
    retype(visual, "board-drawer-description", "Steps to reproduce");
    click(visual, "board-drawer-close");
    let card = detail(&f, visual).cards[0].clone();
    assert_eq!(card.title, "Renamed");
    assert_eq!(card.body, "Steps to reproduce");
    let kinds: Vec<String> = Service::open(&f.db)
        .unwrap()
        .activity(&card.board_id, Some(&card.id), 10)
        .unwrap()
        .into_iter()
        .map(|row| row.kind)
        .collect();
    assert!(kinds.contains(&"edited".to_string()), "{kinds:?}");
}

#[gpui::test]
fn a_cleared_title_comes_back_and_the_description_edit_still_saves(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Keep this title"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-title", "");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(
        drawer(&f, visual, |view, cx| view.card_detail_title(cx)).as_deref(),
        Some("Keep this title"),
        "an emptied title goes back to the saved one"
    );
    // Cleared again, then closed with a description edit: the edit saves.
    retype(visual, "board-drawer-title", "");
    retype(visual, "board-drawer-description", "new notes");
    click(visual, "board-drawer-close");
    let card = detail(&f, visual).cards[0].clone();
    assert_eq!(card.title, "Keep this title");
    assert_eq!(card.body, "new notes");
}

#[gpui::test]
fn a_title_the_board_rejects_stays_in_the_field_with_its_error(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Short"]);
    let long = "x".repeat(501);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-title", &long);
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    for _ in 0..2 {
        drawer(&f, visual, |view, cx| {
            assert_eq!(view.error.as_deref(), Some("Card title is too long"));
            assert_eq!(view.card_detail_title(cx).as_deref(), Some(long.as_str()));
        });
        // A later poll must not treat the rejected text as saved.
        Service::open(&f.db)
            .unwrap()
            .card_create(&detail(&f, visual).columns[1].id, "Elsewhere", "")
            .unwrap();
        visual.executor().advance_clock(Duration::from_secs(2));
        visual.run_until_parked();
    }
    assert_eq!(detail(&f, visual).cards[0].title, "Short");
}

#[gpui::test]
fn mark_complete_without_a_done_step_says_so_and_moves_nothing(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Stay put"]);
    let done = detail(&f, cx).columns[3].clone();
    Service::open(&f.db)
        .unwrap()
        .column_update(&done.id, &done.name, None)
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-complete");
    assert_eq!(column_of(&f, visual, "Stay put"), "Backlog");
    drawer(&f, visual, |view, _| {
        assert!(
            view.error
                .as_deref()
                .is_some_and(|e| e.contains("Done role"))
        );
    });
}

#[gpui::test]
fn a_failed_comment_read_is_reported_and_keeps_the_list_shown(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    Service::open(&f.db)
        .unwrap()
        .add_comment(&card, "Earlier")
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-comment-0").is_some());
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute_batch("ALTER TABLE board_comments RENAME TO hidden_comments;")
        .unwrap();
    click(visual, "board-subtask-add");
    visual.simulate_input("Next step");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-error").is_some());
    assert!(
        visual.debug_bounds("board-comment-0").is_some(),
        "a failed read is not shown as no comments"
    );
}

#[gpui::test]
fn the_link_picker_reads_its_targets_when_it_opens(cx: &mut TestAppContext) {
    let f = seeded(cx, &["To link"]);
    f.store.lock().unwrap().create("codex", "/first").unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-linked");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-link-target-0").is_some());
    assert!(visual.debug_bounds("board-link-target-1").is_none());
    // Typing filters the list read at open; it does not query again.
    f.store.lock().unwrap().create("codex", "/second").unwrap();
    visual.simulate_input("x");
    visual.simulate_keystrokes("backspace");
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-link-target-1").is_none());
    // Reopening reads the sessions again.
    click(visual, "board-drawer-linked");
    click(visual, "board-drawer-linked");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-link-target-1").is_some());
}

#[gpui::test]
fn the_drawer_fields_have_their_own_accessible_names(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    let labels = drawer(&f, visual, |view, cx| {
        view.card_detail.as_ref().unwrap().field_labels(cx)
    });
    assert_eq!(labels, ("Description".into(), "New comment".into()));
}

#[gpui::test]
fn a_card_with_more_activity_than_the_drawer_loads_shows_a_plus(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Busy", "Quiet"]);
    let busy = detail(&f, cx).cards[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    for n in 0..55 {
        service
            .card_update(&busy, &format!("Busy {n}"), "")
            .unwrap();
    }
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    drawer(&f, visual, |view, _| {
        let detail = view.card_detail.as_ref().unwrap();
        assert_eq!(detail.activity_badge(), "50+");
        assert_eq!(
            detail.activity.as_ref().unwrap().as_ref().unwrap().len(),
            50
        );
    });
    click(visual, "board-drawer-close");
    click(visual, "board-card-2");
    drawer(&f, visual, |view, _| {
        // Only its "created" row.
        assert_eq!(view.card_detail.as_ref().unwrap().activity_badge(), "1");
    });
}

#[gpui::test]
fn the_send_hint_names_the_chord_that_sends_a_comment(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let chord = super::detail::SEND_HINT
        .replace('⌘', "cmd-")
        .replace("Ctrl+", "ctrl-")
        .to_lowercase();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-comment-input");
    visual.simulate_input("Sent by keyboard");
    visual.simulate_keystrokes(&chord);
    visual.run_until_parked();
    assert_eq!(extras_of(&f, &card).comments, 1, "{chord} sends");
}

#[gpui::test]
fn label_and_priority_filters_narrow_the_cards_and_combine(cx: &mut TestAppContext) {
    let f = seeded(cx, &["High one", "Labelled", "Plain"]);
    let cards: Vec<String> = detail(&f, cx).cards.iter().map(|c| c.id.clone()).collect();
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service.set_priority(&cards[0], 3).unwrap();
    let bug = service.create_label(&board.id, "Bug", "e5534b").unwrap();
    service.set_labels(&cards[1], &[bug.id]).unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-priority-3");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1]);
    // A card with no priority set counts as None.
    click(visual, "board-filter-priority-0");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 2, 3]);
    click(visual, "board-filter-clear");
    click(visual, "board-filter-label-0");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [2]);
    // Different filters must all match.
    click(visual, "board-filter-priority-3");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), Vec::<i64>::new());
}

fn activity_kinds(f: &Fixture, card: &str) -> Vec<String> {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service
        .activity(&board.id, Some(card), 50)
        .unwrap()
        .into_iter()
        .map(|row| row.kind)
        .collect()
}

#[gpui::test]
fn a_rejected_card_title_stays_in_the_composer(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let long = "x".repeat(501);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-add-card-0");
    visual.simulate_input(&long);
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    drawer(&f, visual, |view, cx| {
        let composer = view.composer.as_ref().expect("the composer stays open");
        assert_eq!(composer.input.read(cx).content.to_string(), long);
        assert_eq!(view.error.as_deref(), Some("Card title is too long"));
    });
    assert!(detail(&f, visual).cards.is_empty());
}

#[gpui::test]
fn a_rejected_subtask_stays_in_its_field(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let long = "s".repeat(501);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-subtask-add");
    visual.simulate_input(&long);
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    drawer(&f, visual, |view, cx| {
        let (_, subtask) = view.card_detail.as_ref().unwrap().drafts(cx);
        assert_eq!(subtask.as_deref(), Some(long.as_str()));
        assert!(view.error.is_some());
    });
}

#[gpui::test]
fn closing_the_drawer_with_a_rejected_title_keeps_it_open(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Short"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-title", &"x".repeat(501));
    click(visual, "board-drawer-close");
    drawer(&f, visual, |view, _| {
        assert!(
            view.card_detail.is_some(),
            "the unsaved title is not thrown away"
        );
        assert_eq!(view.error.as_deref(), Some("Card title is too long"));
    });
}

#[gpui::test]
fn the_drawer_keeps_an_unsaved_description_and_follows_a_move_made_elsewhere(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let review = detail(&f, cx).columns[2].id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-description", "Half-written notes");
    Service::open(&f.db)
        .unwrap()
        .card_move(&card, &review, 0, "system")
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    assert_eq!(column_of(&f, visual, "Task"), "Review");
    drawer(&f, visual, |view, cx| {
        let detail = view.card_detail.as_ref().unwrap();
        assert_eq!(detail.drafts(cx).0, "Half-written notes");
        // The Activity tab shows the move without reopening the drawer.
        let kinds: Vec<&str> = detail
            .activity
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .iter()
            .map(|row| row.kind.as_str())
            .collect();
        assert_eq!(kinds.first(), Some(&"moved"));
    });
}

#[gpui::test]
fn a_status_picked_after_the_steps_were_reordered_is_the_step_it_named(cx: &mut TestAppContext) {
    // A second card in the step, so a needless move would be visible.
    let f = seeded(cx, &["Task", "Below"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let columns = detail(&f, cx).columns.clone();
    let (backlog, review) = (columns[0].id.clone(), columns[2].id.clone());
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-status");
    // Another app reorders the steps while the menu is open.
    Service::open(&f.db)
        .unwrap()
        .column_move(&review, 0)
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    // Picking the step the task is already in changes nothing.
    click(visual, selector(format!("menu-action-status-{backlog}")));
    assert!(!activity_kinds(&f, &card).contains(&"moved".to_string()));
    click(visual, "board-drawer-status");
    click(visual, selector(format!("menu-action-status-{review}")));
    assert_eq!(column_of(&f, visual, "Task"), "Review");
}

#[gpui::test]
fn leaving_the_board_closes_an_open_card_menu(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-menu-1");
    drawer(&f, visual, |view, _| assert!(view.card_menu.is_some()));
    f.window
        .update(visual, |view, _, cx| {
            view.hidden(cx);
            view.shown(cx);
        })
        .unwrap();
    drawer(&f, visual, |view, _| assert!(view.card_menu.is_none()));
}

#[gpui::test]
fn adding_a_column_clears_an_earlier_error(cx: &mut TestAppContext) {
    let f = with_board(cx);
    f.window
        .update(cx, |view, _, cx| {
            view.error = Some("Earlier failure".into());
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-add-column");
    assert_eq!(detail(&f, visual).columns.len(), 5);
    drawer(&f, visual, |view, _| assert_eq!(view.error, None));
}

#[gpui::test]
fn a_filter_never_hides_cards_behind_a_choice_that_is_gone(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let board = Service::open(&f.db).unwrap().default_board().unwrap();
    Service::open(&f.db)
        .unwrap()
        .create_label(&board.id, "Bug", "e5534b")
        .unwrap();
    f.window
        .update(cx, |view, _, cx| {
            view.set_projects(vec![("/gone".into(), "Gone".into())], cx);
            view.reload(cx);
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-label-0");
    click(visual, "board-filter-project-0");
    // The label is deleted elsewhere and the project leaves the workspace.
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute("DELETE FROM board_labels", [])
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.set_projects(Vec::new(), cx))
        .unwrap();
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    drawer(&f, visual, |view, _| {
        assert!(
            view.filter.labels.is_empty(),
            "a deleted label is unselected"
        );
        assert!(
            view.project_choices()
                .contains(&("/gone".into(), "gone".into())),
            "a selected project keeps a row so it can be unchecked"
        );
    });
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-filter-project-0").is_some());
}

#[gpui::test]
fn the_subtask_delete_button_removes_only_that_subtask(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = detail(&f, cx).cards[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    service.add_subtask(&card, "Remove me").unwrap();
    service.add_subtask(&card, "Keep me").unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-subtask-delete-0");
    let left = Service::open(&f.db).unwrap().subtasks(&card).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].title, "Keep me");
    assert!(!left[0].done, "deleting does not tick its neighbour");
}

#[gpui::test]
fn a_card_dropped_while_filtered_lands_beside_the_visible_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["First", "Hidden", "Third"]);
    let cards: Vec<String> = detail(&f, cx).cards.iter().map(|c| c.id.clone()).collect();
    let mut service = Service::open(&f.db).unwrap();
    service.set_priority(&cards[0], 3).unwrap();
    service.set_priority(&cards[2], 3).unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-priority-3");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 3]);
    let first = visual.debug_bounds("board-card-1").unwrap();
    let from = center(visual, "board-card-3");
    drag(
        visual,
        from,
        gpui::point(first.center().x, first.top() + px(2.)),
    );
    let mut backlog: Vec<_> = detail(&f, visual)
        .cards
        .into_iter()
        .filter(|card| card.column_id == detail(&f, visual).columns[0].id)
        .collect();
    backlog.sort_by_key(|card| card.position);
    let titles: Vec<String> = backlog.into_iter().map(|card| card.title).collect();
    assert_eq!(titles, ["Third", "First", "Hidden"]);
}

#[gpui::test]
fn the_header_buttons_work_from_the_keyboard(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, window, cx| {
            window.focus(&view.new_task_focus, cx)
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    drawer(&f, visual, |view, _| assert!(view.composer.is_some()));
    f.window
        .update(visual, |view, window, cx| {
            view.composer = None;
            window.focus(&view.filter_focus, cx)
        })
        .unwrap();
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    drawer(&f, visual, |view, _| assert!(view.filter_open));
}

#[gpui::test]
fn a_link_to_an_archived_session_still_shows_its_title(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let session = f.store.lock().unwrap().create("codex", "/work").unwrap();
    let persisted = f
        .store
        .lock()
        .unwrap()
        .persisted_session_id(session.id)
        .unwrap();
    f.store
        .lock()
        .unwrap()
        .set_archived(session.id, true)
        .unwrap();
    let backlog = detail(&f, cx).columns[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    let card = service.card_create(&backlog, "Archived work", "").unwrap();
    service
        .card_link(&card.id, Some("session"), Some(&persisted))
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    f.window
        .read_with(cx, |view, _| {
            assert_eq!(
                view.link_label(&card.id),
                Some(LinkLabel::Found(session.title.clone()))
            );
        })
        .unwrap();
}

// ---- ticket links ----

use crate::tickets::fakes::FakeTickets;
use gespan_core::tickets::{TicketRef, Tracker};

fn ticket(key: &str, fetched_at: Option<i64>) -> TicketRef {
    TicketRef {
        tracker: Tracker::Linear,
        key: key.into(),
        url: None,
        title: None,
        status: None,
        status_group: None,
        fetched_at,
    }
}

fn with_tickets(f: &Fixture, cx: &mut TestAppContext, connected: &[Tracker]) -> Arc<FakeTickets> {
    let fake = Arc::new(FakeTickets::new(connected));
    let backend = fake.clone();
    f.window
        .update(cx, |view, _, _| view.set_tickets(backend))
        .unwrap();
    fake
}

fn card_ticket(f: &Fixture, number: usize) -> (String, Option<TicketRef>, String) {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    let card = service
        .get(&board.id)
        .unwrap()
        .cards
        .into_iter()
        .find(|card| card.number as usize == number)
        .unwrap();
    (
        card.id.clone(),
        service.card_ticket(&card.id).unwrap(),
        card.title,
    )
}

#[gpui::test]
fn card_detail_links_a_ticket_and_shows_chip(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    let fake = with_tickets(&f, cx, &[Tracker::Linear]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    let (card_id, ..) = card_ticket(&f, 1);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("ticket-chip-detail-1").is_none());
    f.window
        .update(visual, |view, _, cx| {
            view.link_ticket(&card_id, ticket("ENG-7", None), cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    // Linked at once, then read: the chip carries the fetched title and status,
    // in the drawer and on the card.
    let (_, saved, _) = card_ticket(&f, 1);
    let saved = saved.expect("the link is stored");
    assert_eq!(
        (saved.tracker, saved.key.as_str()),
        (Tracker::Linear, "ENG-7")
    );
    assert_eq!(saved.title.as_deref(), Some("Fix login"));
    assert_eq!(saved.status.as_deref(), Some("In Progress"));
    assert_eq!(fake.fetch_count(), 1);
    for place in ["detail-1", "card-1"] {
        for part in ["", "title-", "status-"] {
            assert!(
                visual
                    .debug_bounds(selector(format!("ticket-chip-{part}{place}")))
                    .is_some(),
                "{part}{place}"
            );
        }
        assert!(
            visual
                .debug_bounds(selector(format!("ticket-chip-stale-{place}")))
                .is_none()
        );
    }
    let log = gespan_core::board::Service::open(&f.db)
        .unwrap()
        .activity(&card_ticket_board(&f), Some(&card_id), 10)
        .unwrap();
    assert_eq!(
        log.iter()
            .filter(|row| row.detail.as_deref() == Some("ticket: ENG-7"))
            .count(),
        1,
        "the refresh is not another link"
    );
}

fn card_ticket_board(f: &Fixture) -> String {
    Service::open(&f.db).unwrap().default_board().unwrap().id
}

#[gpui::test]
fn unlinking_removes_the_chip_and_the_stored_link(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    with_tickets(&f, cx, &[Tracker::Linear]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (card_id, ..) = card_ticket(&f, 1);
    f.window
        .update(visual, |view, _, cx| {
            view.link_ticket(&card_id, ticket("ENG-7", None), cx)
        })
        .unwrap();
    visual.run_until_parked();
    f.window
        .update(visual, |view, window, cx| {
            view.open_card_detail(card_id.clone(), window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "board-ticket-unlink");
    assert_eq!(card_ticket(&f, 1).1, None);
    assert!(visual.debug_bounds("ticket-chip-card-1").is_none());
    assert!(visual.debug_bounds("ticket-chip-detail-1").is_none());
}

#[gpui::test]
fn the_chip_refreshes_when_the_drawer_opens_but_not_within_a_minute_or_on_a_timer(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Fix login"]);
    let fake = with_tickets(&f, cx, &[Tracker::Linear]);
    let (card_id, ..) = card_ticket(&f, 1);
    let now = gespan_core::time::now_ms() as i64;
    // Linked a few seconds ago: opening the drawer reads nothing.
    Service::open(&f.db)
        .unwrap()
        .set_card_ticket(&card_id, Some(&ticket("ENG-7", Some(now - 5_000))))
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(fake.fetch_count(), 0, "the board loading reads nothing");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    assert_eq!(fake.fetch_count(), 0, "a fresh read is not repeated");
    // The explicit Refresh always runs.
    click(visual, "board-ticket-refresh");
    assert_eq!(fake.fetch_count(), 1);
    // A stale snapshot is read when the drawer opens.
    Service::open(&f.db)
        .unwrap()
        .set_card_ticket(&card_id, Some(&ticket("ENG-7", Some(now - 10 * 60_000))))
        .unwrap();
    f.window
        .update(visual, |view, _, cx| {
            view.reload(cx);
            assert!(view.close_card_detail(cx));
        })
        .unwrap();
    visual.run_until_parked();
    click(visual, "board-card-1");
    assert_eq!(fake.fetch_count(), 2);
    cx.executor().advance_clock(Duration::from_secs(600));
    cx.run_until_parked();
    assert_eq!(fake.fetch_count(), 2, "never on a timer");
}

#[gpui::test]
fn a_disconnected_tracker_marks_the_chip_stale(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    with_tickets(&f, cx, &[]);
    let (card_id, ..) = card_ticket(&f, 1);
    let mut stored = ticket("ENG-7", Some(1));
    stored.title = Some("Cached".into());
    Service::open(&f.db)
        .unwrap()
        .set_card_ticket(&card_id, Some(&stored))
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("ticket-chip-stale-detail-1").is_some());
    assert!(visual.debug_bounds("ticket-chip-stale-card-1").is_some());
    assert_eq!(
        card_ticket(&f, 1).1.unwrap().title.as_deref(),
        Some("Cached"),
        "a failed read keeps the cached snapshot"
    );
}

#[gpui::test]
fn new_card_from_ticket_links_and_titles_it(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let fake = with_tickets(&f, cx, &[Tracker::Linear]);
    let todo = detail(&f, cx)
        .columns
        .iter()
        .find(|column| column.role.as_deref() == Some("todo"))
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
    let card = board.cards.iter().find(|card| card.id == id).unwrap();
    assert_eq!(card.column_id, todo, "the first To do step takes it");
    assert_eq!(
        card.title, "Fix login",
        "the fetched title replaces the key"
    );
    let (_, linked, _) = card_ticket(&f, card.number as usize);
    assert_eq!(linked.unwrap().key, "ENG-7");
    assert_eq!(fake.fetch_count(), 1);
}

#[gpui::test]
fn new_card_from_ticket_keeps_the_key_title_when_the_fetch_fails(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let fake = with_tickets(&f, cx, &[Tracker::Linear]);
    *fake.offline.lock().unwrap() = true;
    f.window
        .update(cx, |view, _, cx| {
            view.create_card_from_ticket(ticket("ENG-7", None), cx)
        })
        .unwrap();
    cx.run_until_parked();
    let (_, linked, title) = card_ticket(&f, 1);
    assert_eq!(title, "ENG-7");
    assert_eq!(linked.unwrap().title, None);
}

#[gpui::test]
fn new_card_from_ticket_uses_the_first_step_when_none_is_to_do(cx: &mut TestAppContext) {
    let f = with_board(cx);
    with_tickets(&f, cx, &[Tracker::Linear]);
    let columns = detail(&f, cx).columns;
    let mut service = Service::open(&f.db).unwrap();
    // Rename the To do step's role away, and put another step first.
    service
        .column_update(&columns[0].id, &columns[0].name, None)
        .unwrap();
    f.window
        .update(cx, |view, _, cx| {
            view.create_card_from_ticket(ticket("ENG-7", None), cx)
        })
        .unwrap();
    cx.run_until_parked();
    let board = detail(&f, cx);
    assert_eq!(board.cards[0].column_id, board.columns[0].id);
}

#[gpui::test]
fn a_title_the_user_changed_is_not_replaced_by_the_ticket(cx: &mut TestAppContext) {
    let f = seeded(cx, &["My own wording"]);
    with_tickets(&f, cx, &[Tracker::Linear]);
    let (card_id, ..) = card_ticket(&f, 1);
    f.window
        .update(cx, |view, _, cx| {
            view.link_ticket(&card_id, ticket("ENG-7", None), cx)
        })
        .unwrap();
    cx.run_until_parked();
    let (_, linked, title) = card_ticket(&f, 1);
    assert_eq!(title, "My own wording");
    assert_eq!(linked.unwrap().title.as_deref(), Some("Fix login"));
}

#[gpui::test]
fn relinking_to_another_ticket_replaces_the_first(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    with_tickets(&f, cx, &[Tracker::Linear]);
    let (card_id, ..) = card_ticket(&f, 1);
    for key in ["ENG-7", "ENG-8"] {
        f.window
            .update(cx, |view, _, cx| {
                view.link_ticket(&card_id, ticket(key, None), cx)
            })
            .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(card_ticket(&f, 1).1.unwrap().key, "ENG-8");
    let links: i64 = rusqlite::Connection::open(&f.db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM board_card_links WHERE card_id = ?1",
            [&card_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(links, 1);
}

#[gpui::test]
fn the_ticket_row_asks_the_shell_for_the_link_dialog(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (seen, _subscription) = events(&f, visual);
    let (card_id, ..) = card_ticket(&f, 1);
    click(visual, "board-card-1");
    click(visual, "board-drawer-ticket");
    assert_eq!(*seen.borrow(), [format!("link-ticket:{card_id}")]);
}

#[gpui::test]
fn a_card_face_chip_with_no_url_opens_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    // Not connected: the read fails and the link keeps no url.
    with_tickets(&f, cx, &[]);
    let (card_id, ..) = card_ticket(&f, 1);
    Service::open(&f.db)
        .unwrap()
        .set_card_ticket(&card_id, Some(&ticket("ENG-7", Some(1))))
        .unwrap();
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .read_with(visual, |view, _| assert!(view.card_detail.is_none()))
        .unwrap();
    click(visual, "ticket-chip-card-1");
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.card_detail.as_ref().map(|d| d.card_id.clone()),
                Some(card_id.clone())
            )
        })
        .unwrap();
}

// ---- C2-2: face quick edits, column menu, collapse, quick add ----

fn reload(f: &Fixture, cx: &mut TestAppContext) {
    f.window.update(cx, |view, _, cx| view.reload(cx)).unwrap();
    cx.run_until_parked();
}

fn column_id(f: &Fixture, cx: &mut impl gpui::AppContext, name: &str) -> String {
    detail(f, cx)
        .columns
        .iter()
        .find(|c| c.name == name)
        .unwrap()
        .id
        .clone()
}

fn assert_gone(visual: &mut VisualTestContext, target: &'static str) {
    visual.refresh().unwrap();
    assert!(visual.debug_bounds(target).is_none(), "{target} is shown");
}

#[gpui::test]
fn clicking_priority_on_the_face_sets_it_without_opening_the_card(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Rank me", "Plain"]);
    let ranked = card_id(&f, cx, "Rank me");
    let plain = card_id(&f, cx, "Plain");
    Service::open(&f.db)
        .unwrap()
        .set_priority(&ranked, 2)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-priority-1");
    click(visual, "menu-action-priority-4");
    assert_eq!(extras_of(&f, &ranked).priority, 4);
    // Back to none from the same glyph.
    click(visual, "board-card-priority-1");
    click(visual, "menu-action-priority-0");
    assert_eq!(extras_of(&f, &ranked).priority, 0);
    // An unprioritised card has a target of its own.
    assert_gone(visual, "board-card-priority-2");
    click(visual, "board-card-priority-target-2");
    click(visual, "menu-action-priority-3");
    assert_eq!(extras_of(&f, &plain).priority, 3);
    f.window
        .read_with(visual, |view, _| {
            assert!(view.card_detail.is_none(), "the face never opens the card");
            assert!(view.card_menu.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn due_and_labels_edit_from_the_face(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Dated"]);
    let card = card_id(&f, cx, "Dated");
    let board = detail(&f, cx).board.id.clone();
    let mut service = Service::open(&f.db).unwrap();
    let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
    let docs = service.create_label(&board, "Docs", "4da3f5").unwrap();
    service
        .set_labels(&card, std::slice::from_ref(&bug.id))
        .unwrap();
    service
        .set_due(&card, Some(gespan_core::time::now_ms() as i64))
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-due-1");
    click(visual, "menu-action-due-tomorrow");
    let due = extras_of(&f, &card).due_at.expect("a due date");
    let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
    assert_eq!(super::style::local_day(due), Some(tomorrow));

    click(visual, "board-card-labels-1");
    click(visual, selector(format!("menu-action-label-{}", docs.id)));
    assert_eq!(
        extras_of(&f, &card).labels,
        [bug.id.clone(), docs.id.clone()]
    );
    click(visual, "board-card-labels-1");
    click(visual, selector(format!("menu-action-label-{}", bug.id)));
    assert_eq!(extras_of(&f, &card).labels, std::slice::from_ref(&docs.id));

    click(visual, "board-card-due-1");
    click(visual, "menu-action-due-clear");
    assert_eq!(extras_of(&f, &card).due_at, None);
    f.window
        .read_with(visual, |view, _| assert!(view.card_detail.is_none()))
        .unwrap();
}

#[gpui::test]
fn a_face_click_does_not_start_a_drag(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Steady"]);
    let card = card_id(&f, cx, "Steady");
    Service::open(&f.db)
        .unwrap()
        .set_priority(&card, 1)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // Pressing the glyph and pulling toward another column moves nothing.
    let from = center(visual, "board-card-priority-1");
    let to = center(visual, "board-column-body-2");
    drag(visual, from, to);
    assert_eq!(column_of(&f, visual, "Steady"), "Backlog");
    f.window
        .read_with(visual, |view, _| {
            assert!(!view.dragging && view.dragged_card.is_none());
            assert!(view.card_detail.is_none());
        })
        .unwrap();
    // A plain click on it is not a drag either.
    click(visual, "board-card-priority-1");
    f.window
        .read_with(visual, |view, _| {
            assert!(!view.dragging && view.dragged_card.is_none());
            assert!(view.card_detail.is_none());
            assert!(view.card_menu.is_some());
        })
        .unwrap();
}

fn open_column_menu(visual: &mut VisualTestContext, index: usize) {
    click(visual, selector(format!("board-column-menu-{index}")));
}

#[gpui::test]
fn collapse_persists_and_the_strip_accepts_a_mouse_drop(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Mover"]);
    let review = column_id(&f, cx, "Review");
    let mut service = Service::open(&f.db).unwrap();
    service.card_create(&review, "Resident", "").unwrap();
    service.card_create(&review, "Second", "").unwrap();
    // A colour and WIP limit must survive collapsing.
    service
        .set_column_display(&review, Some("4da3f5"), Some(5), false)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 2);
    click(visual, "menu-collapse");
    let column = detail(&f, visual).columns[2].clone();
    assert!(column.collapsed);
    assert_eq!(column.color.as_deref(), Some("4da3f5"));
    assert_eq!(column.wip_limit, Some(5));
    assert_gone(visual, "board-column-body-2");

    // The state is in the database, not the view.
    let board = detail(&f, visual).board.id.clone();
    let stored = Service::open(&f.db).unwrap().get(&board).unwrap();
    assert!(stored.columns[2].collapsed);

    let from = center(visual, "board-card-1");
    let to = center(visual, "board-column-collapsed-2");
    drag(visual, from, to);
    assert_eq!(
        titles_in(&f, visual, "Review"),
        ["Resident", "Second", "Mover"]
    );

    click(visual, "board-column-collapsed-2");
    let column = detail(&f, visual).columns[2].clone();
    assert!(!column.collapsed);
    assert_eq!(column.color.as_deref(), Some("4da3f5"));
    assert_eq!(column.wip_limit, Some(5));
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-column-body-2").is_some());
}

#[gpui::test]
fn enter_on_a_collapsed_strip_expands_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let review = column_id(&f, cx, "Review");
    Service::open(&f.db)
        .unwrap()
        .set_column_display(&review, None, None, true)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    f.window
        .update(visual, |view, window, cx| {
            let handle = view.collapsed_focus.get(&review).unwrap().clone();
            window.focus(&handle, cx);
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(!detail(&f, visual).columns[2].collapsed);
}

#[gpui::test]
fn a_keyboard_drop_onto_a_collapsed_column_lands_at_the_end(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let in_progress = column_id(&f, cx, "In Progress");
    let review = column_id(&f, cx, "Review");
    let mut service = Service::open(&f.db).unwrap();
    service.card_create(&in_progress, "X", "").unwrap();
    service.card_create(&in_progress, "Y", "").unwrap();
    service.card_create(&review, "R", "").unwrap();
    service
        .set_column_display(&in_progress, None, None, true)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    let a = card_id(&f, visual, "A");
    // Focus skips the collapsed column's hidden cards.
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    let r = card_id(&f, visual, "R");
    let focused = f
        .window
        .update(visual, |view, window, _| {
            view.card_focus
                .get(&r)
                .is_some_and(|h| h.is_focused(window))
        })
        .unwrap();
    assert!(focused, "right skips the collapsed column to Review");

    // Lifting A and moving one column right drops it at the end of that column.
    focus_card(&f, visual, &a);
    visual.simulate_keystrokes("space right enter");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "In Progress"), ["X", "Y", "A"]);
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B"]);
}

fn set_priorities(f: &Fixture, cx: &mut TestAppContext, priorities: &[(&str, u8)]) {
    let mut service = Service::open(&f.db).unwrap();
    for (title, priority) in priorities {
        let id = card_id(f, cx, title);
        service.set_priority(&id, *priority).unwrap();
    }
    reload(f, cx);
}

#[gpui::test]
fn sort_by_priority_orders_the_whole_column_even_when_filtered(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C", "D"]);
    set_priorities(&f, cx, &[("A", 1), ("B", 4), ("C", 0), ("D", 4)]);
    // Only A is visible while the filter is on.
    f.window
        .update(cx, |view, _, cx| {
            view.filter.priorities.insert(1);
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    click(visual, "menu-sort-priority");
    // Urgent first; ties keep their order; hidden cards are sorted too.
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "D", "A", "C"]);
    let positions: Vec<i64> = {
        let mut cards = detail(&f, visual).cards;
        cards.sort_by_key(|c| c.position);
        cards.iter().map(|c| c.position).collect()
    };
    assert_eq!(positions, [0, 1, 2, 3]);
}

#[gpui::test]
fn sort_by_due_puts_undated_last(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C", "D"]);
    let day = 86_400_000_i64;
    let now = gespan_core::time::now_ms() as i64;
    let mut service = Service::open(&f.db).unwrap();
    service
        .set_due(&card_id(&f, cx, "B"), Some(now + 5 * day))
        .unwrap();
    service
        .set_due(&card_id(&f, cx, "C"), Some(now + day))
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    click(visual, "menu-sort-due");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["C", "B", "A", "D"]);
}

#[gpui::test]
fn mark_all_done_moves_every_card_to_the_done_column(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 3);
    assert_gone(visual, "menu-mark-done");
    visual.simulate_keystrokes("escape");
    open_column_menu(visual, 0);
    click(visual, "menu-mark-done");
    assert_eq!(titles_in(&f, visual, "Backlog"), Vec::<String>::new());
    assert_eq!(titles_in(&f, visual, "Done"), ["A", "B", "C"]);
    assert_eq!(moved_rows(&f, visual), 3);
}

#[gpui::test]
fn mark_all_done_is_hidden_when_no_column_is_done(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let done = column_id(&f, cx, "Done");
    Service::open(&f.db)
        .unwrap()
        .column_update(&done, "Done", None)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("menu-sort-priority").is_some());
    assert_gone(visual, "menu-mark-done");
}

#[gpui::test]
fn archive_completed_only_on_the_done_column(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let board = detail(&f, cx).board.id.clone();
    let done = column_id(&f, cx, "Done");
    let mut service = Service::open(&f.db).unwrap();
    for title in ["A", "B"] {
        let id = card_id(&f, cx, title);
        service.card_move(&id, &done, usize::MAX, "user").unwrap();
    }
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    assert_gone(visual, "menu-archive-done");
    visual.simulate_keystrokes("escape");
    open_column_menu(visual, 3);
    click(visual, "menu-archive-done");
    assert!(detail(&f, visual).cards.is_empty());
    let archived = Service::open(&f.db).unwrap().archived(&board).unwrap();
    assert_eq!(archived.len(), 2);
}

#[gpui::test]
fn the_composer_stays_open_after_enter(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-add-card-0");
    visual.simulate_input("First");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, cx| {
            let composer = view.composer.as_ref().expect("still open");
            assert_eq!(composer.input.read(cx).content.as_ref(), "");
        })
        .unwrap();
    // Focus stayed in the field, so typing carries on without a click.
    visual.simulate_input("Second");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["First", "Second"]);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    f.window
        .read_with(visual, |view, _| assert!(view.composer.is_none()))
        .unwrap();
}

#[gpui::test]
fn delete_column_is_disabled_while_archived_cards_remain(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Shelved", "Gone soon"]);
    let backlog = column_id(&f, cx, "Backlog");
    let shelved = card_id(&f, cx, "Shelved");
    let gone = card_id(&f, cx, "Gone soon");
    let mut service = Service::open(&f.db).unwrap();
    service.archive(&shelved).unwrap();
    service.card_delete(&gone).unwrap();
    reload(&f, cx);
    assert!(detail(&f, cx).cards.is_empty(), "no live cards remain");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(
                view.delete_blocked(&backlog).as_deref(),
                Some("Move or delete its 1 card first")
            );
        })
        .unwrap();
    click(visual, "board-column-delete");
    assert_eq!(detail(&f, visual).columns.len(), 4);
    visual.simulate_keystrokes("escape");
    // Once nothing live or archived is left, delete works.
    Service::open(&f.db).unwrap().card_delete(&shelved).unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    click(visual, "board-column-delete");
    assert_eq!(detail(&f, visual).columns.len(), 3);
}
