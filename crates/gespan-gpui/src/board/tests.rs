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
    cx.update(new_card::bind);
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
                    size(px(1400.), px(1400.)),
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
        .update(visual, |view, _, cx| {
            filter_by_priority(view, 0, cx);
        })
        .unwrap();
    click(visual, "board-switcher");
    click(visual, "board-switch-1");
    assert_eq!(open_id(&f, visual), sprint.id);
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.query(cx), "");
            assert_eq!(view.spec.rows.len(), 0);
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
    click(visual, "board-filter-add-projects");
    click(visual, "board-filter-row-0-value-0");
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
    click(visual, "board-filter-add-projects");
    click(visual, "board-filter-row-0-value-1");
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
    click(visual, "board-filter-add-projects");
    click(visual, "board-filter-row-0-value-1");
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
    click(visual, "board-filter-add-priority");
    // Priorities list Urgent first: High is the second value, None the last.
    click(visual, "board-filter-row-0-value-1");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1]);
    // A card with no priority set counts as None.
    click(visual, "board-filter-row-0-value-4");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 2, 3]);
    click(visual, "board-filter-clear");
    click(visual, "board-filter-add-labels");
    click(visual, "board-filter-row-0-value-0");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [2]);
    // Different filters must all match.
    click(visual, "board-filter-add-priority");
    click(visual, "board-filter-row-1-value-1");
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
    click(visual, "board-filter-add-labels");
    click(visual, "board-filter-row-0-value-0");
    click(visual, "board-filter-add-projects");
    click(visual, "board-filter-row-1-value-0");
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
            !view.chip_text(&view.spec.rows[0]).contains("Bug"),
            "a deleted label is no longer listed"
        );
        assert!(
            view.detail
                .as_ref()
                .is_some_and(|d| d.cards.iter().all(|card| view.card_matches(card))),
            "a deleted label never hides cards"
        );
        assert!(
            view.project_choices()
                .contains(&("/gone".into(), "gone".into())),
            "a selected project keeps a row so it can be unchecked"
        );
    });
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-filter-row-1-value-0").is_some());
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
    click(visual, "board-filter-add-priority");
    click(visual, "board-filter-row-0-value-1");
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

fn filter_by_priority(view: &mut BoardView, priority: u8, cx: &mut Context<BoardView>) {
    view.spec.rows.push(gespan_core::board::extras::FilterRow {
        field: "priority".into(),
        op: gespan_core::board::extras::Op::Is,
        values: vec![priority.to_string()],
    });
    view.requery(cx);
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
            filter_by_priority(view, 1, cx);
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

// ---- undo toasts, held deletes and the Archive view ----

/// The undo toast's text, if one is showing.
fn toast(f: &Fixture, cx: &mut impl gpui::AppContext) -> Option<String> {
    f.window
        .read_with(cx, |view, _| {
            view.undo.toast.as_ref().map(|toast| toast.label.clone())
        })
        .unwrap()
}

fn card_exists(f: &Fixture, id: &str) -> bool {
    Service::open(&f.db).unwrap().card(id).is_ok()
}

fn archived_ids(f: &Fixture, cx: &mut impl gpui::AppContext) -> Vec<String> {
    let board = detail(f, cx).board.id.clone();
    Service::open(&f.db)
        .unwrap()
        .archived(&board)
        .unwrap()
        .into_iter()
        .map(|card| card.id)
        .collect()
}

/// Delete card `#number` from its drawer: no prompt, the drawer closes.
fn delete_from_drawer(f: &Fixture, visual: &mut VisualTestContext, number: usize) {
    click(visual, selector(format!("board-card-{number}")));
    click(visual, "board-drawer-more");
    click(visual, "menu-action-delete");
    assert!(!visual.has_pending_prompt(), "a held delete never prompts");
    f.window
        .read_with(visual, |view, _| assert!(view.card_detail.is_none()))
        .unwrap();
}

#[gpui::test]
fn moving_a_card_shows_undo_and_undo_moves_it_back(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);

    // Move to… from the card menu.
    click(visual, "board-card-menu-2");
    click(visual, "menu-action-move-3");
    assert_eq!(titles_in(&f, visual, "Done"), ["B"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Moved #2 to Done"));
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-undo-toast").is_some());
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C"]);
    assert_eq!(toast(&f, visual), None);
    assert_gone(visual, "board-undo-toast");

    // A mouse drop.
    let from = center(visual, "board-card-1");
    let to = center(visual, "board-column-body-2");
    drag(visual, from, to);
    assert_eq!(titles_in(&f, visual, "Review"), ["A"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Moved #1 to Review"));
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C"]);

    // A keyboard drop, reordering inside the column.
    let c = card_id(&f, visual, "C");
    focus_card(&f, visual, &c);
    visual.simulate_keystrokes("space up up enter");
    visual.run_until_parked();
    assert_eq!(titles_in(&f, visual, "Backlog"), ["C", "A", "B"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Moved #3 to Backlog"));
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C"]);

    // Left alone, the toast goes after 8 seconds.
    click(visual, "board-card-menu-1");
    click(visual, "menu-action-move-1");
    visual
        .executor()
        .advance_clock(Duration::from_millis(7_900));
    visual.run_until_parked();
    assert!(toast(&f, visual).is_some(), "still inside the window");
    visual.executor().advance_clock(Duration::from_millis(200));
    visual.run_until_parked();
    assert_eq!(toast(&f, visual), None);
    assert_gone(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "In Progress"), ["A"]);
}

#[gpui::test]
fn a_bulk_move_undo_restores_every_card_position(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let done = column_id(&f, cx, "Done");
    Service::open(&f.db)
        .unwrap()
        .card_create(&done, "X", "")
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    click(visual, "menu-mark-done");
    assert_eq!(titles_in(&f, visual, "Done"), ["X", "A", "B", "C"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Moved 3 cards to Done"));
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C"]);
    assert_eq!(titles_in(&f, visual, "Done"), ["X"]);
    let mut cards = detail(&f, visual).cards;
    cards.sort_by_key(|c| (c.column_id.clone(), c.position));
    for column in [column_id(&f, visual, "Backlog"), done] {
        let positions: Vec<i64> = cards
            .iter()
            .filter(|c| c.column_id == column)
            .map(|c| c.position)
            .collect();
        assert_eq!(positions, (0..positions.len() as i64).collect::<Vec<_>>());
    }
}

#[gpui::test]
fn sort_undo_restores_the_exact_previous_order(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C", "D"]);
    set_priorities(&f, cx, &[("A", 1), ("B", 4), ("C", 0), ("D", 4)]);
    // Sorting with a filter on still sorts, and undoes, the whole column.
    f.window
        .update(cx, |view, _, cx| {
            filter_by_priority(view, 1, cx);
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 0);
    click(visual, "menu-sort-priority");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "D", "A", "C"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Sorted Backlog"));
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C", "D"]);
}

#[gpui::test]
fn delete_hides_the_card_and_expiry_deletes_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Doomed", "Stays"]);
    let doomed = card_id(&f, cx, "Doomed");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    assert_eq!(titles_in(&f, visual, "Backlog"), ["Stays"]);
    assert_eq!(toast(&f, visual).as_deref(), Some("Deleted #1"));
    // Held: archived, not deleted, while the toast shows.
    assert!(card_exists(&f, &doomed));
    assert_eq!(archived_ids(&f, visual), std::slice::from_ref(&doomed));
    visual
        .executor()
        .advance_clock(Duration::from_millis(7_900));
    visual.run_until_parked();
    assert!(card_exists(&f, &doomed), "not before 8 seconds");
    visual.executor().advance_clock(Duration::from_millis(200));
    visual.run_until_parked();
    assert!(!card_exists(&f, &doomed), "deleted once the toast expires");
    assert_eq!(toast(&f, visual), None);
    assert_eq!(titles_in(&f, visual, "Backlog"), ["Stays"]);
}

#[gpui::test]
fn undo_delete_restores_the_card_at_its_index(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let b = card_id(&f, cx, "B");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 2);
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "C"]);
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C"]);
    assert!(archived_ids(&f, visual).is_empty());
    // The expired timer of the undone delete deletes nothing.
    visual.executor().advance_clock(Duration::from_secs(10));
    visual.run_until_parked();
    assert!(card_exists(&f, &b));
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B", "C"]);
}

#[gpui::test]
fn a_new_action_commits_the_pending_delete(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    assert!(card_exists(&f, &a));
    click(visual, "board-card-menu-2");
    click(visual, "menu-action-move-3");
    assert!(
        !card_exists(&f, &a),
        "replacing the toast commits the delete"
    );
    assert_eq!(toast(&f, visual).as_deref(), Some("Moved #2 to Done"));
    // Undo now only undoes the move.
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B"]);
    assert!(!card_exists(&f, &a));
}

#[gpui::test]
fn switching_boards_commits_a_pending_delete(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let sprint = Service::open(&f.db).unwrap().create("Sprint").unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    f.window
        .update(visual, |view, _, cx| view.open_board(sprint.id.clone(), cx))
        .unwrap();
    visual.run_until_parked();
    assert!(!card_exists(&f, &a));
    assert_eq!(toast(&f, visual), None);
}

#[gpui::test]
fn closing_the_window_with_a_pending_delete_keeps_the_card_archived(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let a = card_id(&f, cx, "A");
    let board = detail(&f, cx).board.id.clone();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    f.window
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
    visual.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert!(card_exists(&f, &a), "never half-deleted");
    let archived = Service::open(&f.db).unwrap().archived(&board).unwrap();
    assert_eq!(archived.len(), 1, "it shows in the Archive view next time");
}

#[gpui::test]
fn undo_after_the_card_was_deleted_elsewhere_reports_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let a = card_id(&f, cx, "A");
    let b = card_id(&f, cx, "B");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-menu-1");
    click(visual, "menu-action-move-3");
    Service::open(&f.db).unwrap().card_delete(&a).unwrap();
    click(visual, "board-undo");
    assert_eq!(
        toast(&f, visual).as_deref(),
        Some("Couldn't undo: the card was deleted")
    );
    assert_gone(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "C"]);

    // A bulk undo restores every card it still can.
    open_column_menu(visual, 0);
    click(visual, "menu-mark-done");
    Service::open(&f.db).unwrap().card_delete(&b).unwrap();
    click(visual, "board-undo");
    assert_eq!(
        toast(&f, visual).as_deref(),
        Some("Couldn't undo: the card was deleted")
    );
    assert_eq!(titles_in(&f, visual, "Backlog"), ["C"]);
    assert!(titles_in(&f, visual, "Done").is_empty());
    // The message goes like any toast.
    visual.executor().advance_clock(Duration::from_secs(9));
    visual.run_until_parked();
    assert_eq!(toast(&f, visual), None);
}

#[gpui::test]
fn undo_after_the_column_was_deleted_reports_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A"]);
    let a = card_id(&f, cx, "A");
    let backlog = column_id(&f, cx, "Backlog");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-menu-1");
    click(visual, "menu-action-move-3");
    Service::open(&f.db)
        .unwrap()
        .column_delete(&backlog)
        .unwrap();
    click(visual, "board-undo");
    assert_eq!(
        toast(&f, visual).as_deref(),
        Some("Couldn't undo: the column was deleted")
    );
    assert!(card_exists(&f, &a));
    assert_eq!(titles_in(&f, visual, "Done"), ["A"]);
}

#[gpui::test]
fn archive_completed_can_be_undone(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let done = column_id(&f, cx, "Done");
    let mut service = Service::open(&f.db).unwrap();
    for title in ["A", "B"] {
        let id = card_id(&f, cx, title);
        service.card_move(&id, &done, usize::MAX, "user").unwrap();
    }
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_column_menu(visual, 3);
    click(visual, "menu-archive-done");
    assert_eq!(toast(&f, visual).as_deref(), Some("Archived 2 cards"));
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Done"), ["A", "B"]);
    assert!(archived_ids(&f, visual).is_empty());
}

#[gpui::test]
fn archive_view_restores_and_deletes_forever(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    let (a, b) = (card_id(&f, cx, "A"), card_id(&f, cx, "B"));
    let mut service = Service::open(&f.db).unwrap();
    // B (#2) is archived before A (#1): the order follows archive time, not number.
    service.archive(&b).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    service.archive(&a).unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-archive");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-archive-panel").is_some());
    // Newest first: A was archived last.
    let listed: Vec<String> = f
        .window
        .read_with(visual, |view, _| {
            view.archived_cards()
                .iter()
                .map(|c| c.title.clone())
                .collect()
        })
        .unwrap();
    assert_eq!(listed, ["A", "B"]);

    // Restore puts A back at the end of its column.
    click(visual, "board-archive-restore-0");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["C", "A"]);

    // Delete forever asks first; Cancel keeps it.
    click(visual, "board-archive-delete-0");
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    assert!(card_exists(&f, &b));
    click(visual, "board-archive-delete-0");
    visual.simulate_prompt_answer("Delete forever");
    visual.run_until_parked();
    assert!(!card_exists(&f, &b));
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-archive-empty").is_some());

    // The header button closes it again.
    click(visual, "board-archive");
    assert_gone(visual, "board-archive-panel");
}

#[gpui::test]
fn a_pending_delete_is_not_listed_in_the_archive_view(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "Shelved"]);
    let shelved = card_id(&f, cx, "Shelved");
    Service::open(&f.db).unwrap().archive(&shelved).unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    click(visual, "board-archive");
    let listed: Vec<String> = f
        .window
        .read_with(visual, |view, _| {
            view.archived_cards()
                .iter()
                .map(|c| c.title.clone())
                .collect()
        })
        .unwrap();
    assert_eq!(listed, ["Shelved"]);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-archive-restore-0").is_some());
    assert!(visual.debug_bounds("board-archive-restore-1").is_none());
    // Undo brings it back to the board, still not into the archive.
    click(visual, "board-undo");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A"]);
}

#[gpui::test]
fn a_held_delete_restored_elsewhere_is_never_deleted(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let a = card_id(&f, cx, "A");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    // The other app restores it during the window.
    Service::open(&f.db).unwrap().unarchive(&a).unwrap();
    visual.executor().advance_clock(Duration::from_secs(9));
    visual.run_until_parked();
    assert!(card_exists(&f, &a), "a live card is never hard-deleted");
    assert!(archived_ids(&f, visual).is_empty());
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B", "A"]);
    f.window
        .read_with(visual, |view, _| assert!(view.error.is_none()))
        .unwrap();

    // The same holds when a new action, not expiry, commits it.
    let b = card_id(&f, visual, "B");
    delete_from_drawer(&f, visual, 2);
    Service::open(&f.db).unwrap().unarchive(&b).unwrap();
    click(visual, "board-card-menu-1");
    click(visual, "menu-action-move-3");
    assert!(card_exists(&f, &b));
    assert_eq!(titles_in(&f, visual, "Backlog"), ["B"]);
}

#[gpui::test]
fn a_held_delete_does_not_block_deleting_its_column(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Only"]);
    let only = card_id(&f, cx, "Only");
    let backlog = column_id(&f, cx, "Backlog");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    delete_from_drawer(&f, visual, 1);
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.delete_blocked(&backlog), None);
        })
        .unwrap();
    open_column_menu(visual, 0);
    click(visual, "board-column-delete");
    assert_eq!(detail(&f, visual).columns.len(), 3);
    assert!(
        !card_exists(&f, &only),
        "the held delete is carried out first"
    );
    assert_eq!(toast(&f, visual), None);
}

// ---- drawer pass 1: full page, dates, estimate, relations, fields, links ----

fn shown(visual: &mut VisualTestContext, target: &'static str) -> bool {
    visual.refresh().unwrap();
    visual.debug_bounds(target).is_some()
}

fn local_midnight(day: chrono::NaiveDate) -> i64 {
    super::style::day_start(day).unwrap()
}

fn meta_of(f: &Fixture, card: &str) -> gespan_core::board::extras::CardMeta {
    Service::open(&f.db).unwrap().card_meta(card).unwrap()
}

#[gpui::test]
fn full_page_toggle_expands_and_escape_returns_to_drawer(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    assert!(shown(visual, "board-columns"));
    click(visual, "board-drawer-full-page");
    assert!(
        !shown(visual, "board-columns"),
        "the page replaces the columns"
    );
    assert!(shown(visual, "board-drawer"));
    assert!(shown(visual, "board-header"), "the header stays");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(
        shown(visual, "board-columns"),
        "Escape returns to the drawer"
    );
    assert!(shown(visual, "board-drawer"));
    click(visual, "board-drawer-full-page");
    click(visual, "board-drawer-full-page");
    assert!(shown(visual, "board-columns"), "the toggle also returns");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!shown(visual, "board-drawer"), "a second Escape closes");
}

#[gpui::test]
fn the_open_cards_in_setting_opens_full_page_by_default(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    assert!(shown(visual, "board-columns"), "the drawer is the default");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    click(visual, "board-open-in");
    click(visual, "board-open-in-page");
    let saved = crate::runtime::locked(&f.store)
        .unwrap()
        .setting("board-card-open")
        .unwrap();
    assert_eq!(saved.as_deref(), Some("page"));
    click(visual, "board-card-1");
    assert!(!shown(visual, "board-columns"), "the card opens as a page");
    assert!(shown(visual, "board-drawer"));
    // A new window reads the saved setting.
    let window = reopen(&f, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    click(visual, "board-card-1");
    assert!(!shown(visual, "board-columns"));
    click(visual, "board-drawer-full-page");
    assert!(
        shown(visual, "board-columns"),
        "the toggle still shrinks it"
    );
}

#[gpui::test]
fn start_and_due_quick_picks_include_in_two_weeks(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    let today = chrono::Local::now().date_naive();
    let in_two_weeks = today + chrono::Days::new(14);
    click(visual, "board-drawer-start");
    click(visual, "menu-action-start-in-two-weeks");
    assert_eq!(
        meta_of(&f, &card).start_at,
        Some(local_midnight(in_two_weeks))
    );
    click(visual, "board-drawer-due");
    click(visual, "menu-action-due-in-two-weeks");
    assert_eq!(
        meta_of(&f, &card).due_at,
        Some(local_midnight(in_two_weeks))
    );
    click(visual, "board-drawer-start");
    click(visual, "menu-action-start-clear");
    assert_eq!(meta_of(&f, &card).start_at, None);
}

#[gpui::test]
fn estimate_parses_hours_and_minutes_in_the_drawer(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-estimate");
    visual.simulate_input("1h 30m");
    visual.simulate_keystrokes("enter");
    assert_eq!(meta_of(&f, &card).estimate_minutes, Some(90));
    // A bad value shows inline and leaves the estimate alone.
    click(visual, "board-drawer-estimate");
    visual.simulate_input("0");
    visual.simulate_keystrokes("enter");
    assert!(shown(visual, "board-inline-error"));
    assert_eq!(meta_of(&f, &card).estimate_minutes, Some(90));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    // Empty clears it.
    click(visual, "board-drawer-estimate");
    visual.simulate_keystrokes("enter");
    assert_eq!(meta_of(&f, &card).estimate_minutes, None);
}

fn relations_of(f: &Fixture, card: &str) -> Vec<(String, String, String)> {
    Service::open(&f.db)
        .unwrap()
        .card_relations(card)
        .unwrap()
        .into_iter()
        .map(|r| (r.card_id, r.related_card_id, r.kind))
        .collect()
}

#[gpui::test]
fn blocks_and_blocked_by_write_the_right_direction(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C", "D"]);
    let (a, b, c, d) = (
        card_id(&f, cx, "A"),
        card_id(&f, cx, "B"),
        card_id(&f, cx, "C"),
        card_id(&f, cx, "D"),
    );
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    // The card itself is never offered; B is the first of the others.
    click(visual, "board-drawer-blocks");
    visual.simulate_input("#2");
    click(visual, "board-pick-relation-0");
    assert_eq!(
        relations_of(&f, &a),
        [(a.clone(), b.clone(), "blocks".into())]
    );
    click(visual, "board-drawer-blocked-by");
    visual.simulate_input("c");
    click(visual, "board-pick-relation-0");
    assert!(relations_of(&f, &a).contains(&(c.clone(), a.clone(), "blocks".into())));
    click(visual, "board-drawer-relates");
    visual.simulate_input("D");
    click(visual, "board-pick-relation-0");
    assert!(relations_of(&f, &a).contains(&(a.clone(), d.clone(), "relates".into())));
    assert_eq!(relations_of(&f, &a).len(), 3);
    // Self is not in the list: "A" matches only itself, so nothing shows.
    click(visual, "board-drawer-relates");
    visual.simulate_input("A");
    assert!(!shown(visual, "board-pick-relation-0"));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    // Removing one writes that one only.
    click(visual, "board-relation-remove-0");
    assert_eq!(relations_of(&f, &a).len(), 2);
}

#[gpui::test]
fn a_relation_cycle_shows_the_refusal_inline(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let (a, b) = (card_id(&f, cx, "A"), card_id(&f, cx, "B"));
    Service::open(&f.db)
        .unwrap()
        .add_relation(&a, &b, "blocks")
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-2");
    // B blocking A would close a loop.
    click(visual, "board-drawer-blocks");
    visual.simulate_input("A");
    click(visual, "board-pick-relation-0");
    assert!(shown(visual, "board-inline-error"));
    assert_eq!(relations_of(&f, &b), [(a, b, "blocks".into())]);
}

fn add_field(f: &Fixture, name: &str, kind: &str, options: Option<&str>, position: i64) {
    let board = Service::open(&f.db).unwrap().default_board().unwrap();
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    conn.execute(
        "INSERT INTO board_fields (id, board_id, name, kind, options_json, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![format!("f-{name}"), board.id, name, kind, options, position],
    )
    .unwrap();
}

fn field_value(f: &Fixture, card: &str, name: &str) -> Option<String> {
    Service::open(&f.db)
        .unwrap()
        .field_values(card)
        .unwrap()
        .into_iter()
        .find(|v| v.field_id == format!("f-{name}"))
        .map(|v| v.value_json)
}

#[gpui::test]
fn custom_field_values_edit_by_kind(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    add_field(&f, "Note", "text", None, 0);
    add_field(&f, "Points", "number", None, 1);
    add_field(&f, "Ship", "date", None, 2);
    add_field(&f, "Stage", "select", Some(r#"["alpha","beta"]"#), 3);
    add_field(&f, "Ready", "checkbox", None, 4);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    for index in 0..5 {
        assert!(
            shown(visual, selector(format!("board-drawer-field-{index}"))),
            "no row for field {index}"
        );
    }
    click(visual, "board-drawer-field-0");
    visual.simulate_input("hello");
    visual.simulate_keystrokes("enter");
    assert_eq!(field_value(&f, &card, "Note").as_deref(), Some("\"hello\""));
    click(visual, "board-drawer-field-1");
    visual.simulate_input("abc");
    visual.simulate_keystrokes("enter");
    assert!(shown(visual, "board-inline-error"));
    assert_eq!(field_value(&f, &card, "Points"), None);
    visual.simulate_keystrokes("cmd-a backspace");
    visual.simulate_input("3.5");
    visual.simulate_keystrokes("enter");
    assert_eq!(field_value(&f, &card, "Points").as_deref(), Some("3.5"));
    click(visual, "board-drawer-field-2");
    click(visual, "menu-action-field-tomorrow");
    let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
    assert_eq!(
        field_value(&f, &card, "Ship"),
        Some(local_midnight(tomorrow).to_string())
    );
    click(visual, "board-drawer-field-2");
    click(visual, "menu-action-field-clear");
    assert_eq!(field_value(&f, &card, "Ship"), None);
    click(visual, "board-drawer-field-3");
    click(visual, "menu-action-field-option-1");
    assert_eq!(field_value(&f, &card, "Stage").as_deref(), Some("\"beta\""));
    click(visual, "board-drawer-field-3");
    click(visual, "menu-action-field-clear");
    assert_eq!(field_value(&f, &card, "Stage"), None);
    click(visual, "board-drawer-field-4");
    assert_eq!(field_value(&f, &card, "Ready").as_deref(), Some("true"));
    click(visual, "board-drawer-field-4");
    assert_eq!(field_value(&f, &card, "Ready").as_deref(), Some("false"));
}

fn links_of(f: &Fixture, card: &str) -> Vec<(String, String)> {
    Service::open(&f.db)
        .unwrap()
        .card_links(card)
        .unwrap()
        .into_iter()
        .map(|l| (l.kind, l.reference))
        .collect()
}

#[gpui::test]
fn links_list_adds_a_pr_url_and_removes_a_link(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-links-add");
    click(visual, "menu-action-link-pr");
    visual.simulate_input("ftp://example.com/x");
    visual.simulate_keystrokes("enter");
    assert!(
        shown(visual, "board-inline-error"),
        "only http(s) is allowed"
    );
    assert!(links_of(&f, &card).is_empty());
    visual.simulate_keystrokes("cmd-a backspace");
    visual.simulate_input("https://github.com/a/b/pull/1");
    visual.simulate_keystrokes("enter");
    assert_eq!(
        links_of(&f, &card),
        [(
            "pr".to_string(),
            "https://github.com/a/b/pull/1".to_string()
        )]
    );
    assert!(shown(visual, "board-link-row-0"));
    assert!(shown(visual, "board-link-open-0"));
    click(visual, "board-drawer-links-add");
    click(visual, "menu-action-link-worktree");
    visual.simulate_input("/tmp/work");
    visual.simulate_keystrokes("enter");
    assert_eq!(links_of(&f, &card).len(), 2);
    click(visual, "board-link-remove-0");
    assert_eq!(
        links_of(&f, &card),
        [("worktree".to_string(), "/tmp/work".to_string())]
    );
}

// ---- C3-4: Markdown toolbar, subtasks, comment authors, More menu ----

fn description_state(
    f: &Fixture,
    cx: &mut impl gpui::AppContext,
) -> (String, std::ops::Range<usize>) {
    f.window
        .read_with(cx, |view, cx| {
            view.card_detail.as_ref().unwrap().description_state(cx)
        })
        .unwrap()
}

#[gpui::test]
fn the_markdown_toolbar_formats_the_selection_and_saves_with_the_description(
    cx: &mut TestAppContext,
) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(
        visual,
        "board-drawer-description",
        "make it loud\nsecond line",
    );
    for key in [
        "bold", "italic", "heading", "bullets", "numbered", "link", "code",
    ] {
        assert!(
            shown(visual, selector(format!("board-md-{key}"))),
            "no {key} button"
        );
    }
    // Bold on a selection wraps it and selects the inside.
    f.window
        .update(visual, |view, _, cx| view.select_description(8..12, cx))
        .unwrap();
    click(visual, "board-md-bold");
    assert_eq!(
        description_state(&f, visual),
        ("make it **loud**\nsecond line".to_string(), 10..14)
    );
    // Bulleted list on a selection across both lines.
    f.window
        .update(visual, |view, _, cx| view.select_description(0..29, cx))
        .unwrap();
    click(visual, "board-md-bullets");
    assert_eq!(
        description_state(&f, visual).0,
        "- make it **loud**\n- second line"
    );
    // An empty selection puts the cursor between the markers.
    f.window
        .update(visual, |view, _, cx| view.select_description(0..0, cx))
        .unwrap();
    click(visual, "board-md-italic");
    assert_eq!(description_state(&f, visual).1, 1..1);
    // Nothing is written until the description's usual save point.
    assert_eq!(detail(&f, visual).cards[0].body, "");
    click(visual, "board-drawer-close");
    assert_eq!(
        detail(&f, visual).cards[0].body,
        "__- make it **loud**\n- second line"
    );
}

#[gpui::test]
fn the_toolbar_wraps_a_selection_at_the_end_of_the_text(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-description", "see docs");
    f.window
        .update(visual, |view, _, cx| view.select_description(4..8, cx))
        .unwrap();
    click(visual, "board-md-link");
    assert_eq!(
        description_state(&f, visual),
        ("see [docs](url)".to_string(), 11..14)
    );
}

fn add_subtasks(f: &Fixture, card: &str, titles: &[&str]) -> Vec<String> {
    let mut service = Service::open(&f.db).unwrap();
    titles
        .iter()
        .map(|title| service.add_subtask(card, title).unwrap().id)
        .collect()
}

fn subtask_titles(f: &Fixture, card: &str) -> Vec<String> {
    Service::open(&f.db)
        .unwrap()
        .subtasks(card)
        .unwrap()
        .into_iter()
        .map(|s| s.title)
        .collect()
}

#[gpui::test]
fn subtasks_reorder_with_move_up_down(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    let ids = add_subtasks(&f, &card, &["one", "two", "three"]);
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    // The first has no Move up, the last no Move down.
    click(visual, "board-subtask-menu-0");
    assert!(!shown(
        visual,
        selector(format!("menu-action-subtask-up-{}", ids[0]))
    ));
    assert!(shown(
        visual,
        selector(format!("menu-action-subtask-down-{}", ids[0]))
    ));
    click(
        visual,
        selector(format!("menu-action-subtask-down-{}", ids[0])),
    );
    assert_eq!(subtask_titles(&f, &card), ["two", "one", "three"]);
    click(visual, "board-subtask-menu-2");
    assert!(!shown(
        visual,
        selector(format!("menu-action-subtask-down-{}", ids[2]))
    ));
    click(
        visual,
        selector(format!("menu-action-subtask-up-{}", ids[2])),
    );
    assert_eq!(subtask_titles(&f, &card), ["two", "three", "one"]);
}

#[gpui::test]
fn convert_subtask_creates_a_card_and_removes_the_subtask(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Parent", "Other"]);
    let card = card_id(&f, cx, "Parent");
    let ids = add_subtasks(&f, &card, &["Ship it", "Keep"]);
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-subtask-menu-0");
    click(
        visual,
        selector(format!("menu-action-subtask-convert-{}", ids[0])),
    );
    assert_eq!(subtask_titles(&f, &card), ["Keep"]);
    let titles: Vec<String> = detail(&f, visual)
        .cards
        .iter()
        .filter(|c| c.column_id == detail(&f, visual).columns[0].id)
        .map(|c| c.title.clone())
        .collect();
    assert_eq!(titles, ["Parent", "Ship it", "Other"]);
    let new = card_id(&f, visual, "Ship it");
    assert_eq!(
        relations_of(&f, &card),
        [(card.clone(), new, "relates".to_string())]
    );
    // The drawer lists what is left.
    assert!(!shown(visual, "board-subtask-1"));
}

#[gpui::test]
fn agent_comments_show_agent(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    {
        let mut service = Service::open(&f.db).unwrap();
        service.add_comment(&card, "from me").unwrap();
        service.add_agent_comment(&card, "from the bot").unwrap();
    }
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    // The selectors carry the rendered text: who wrote it and the initial.
    assert!(shown(visual, "board-comment-author-0-You"));
    assert!(shown(visual, "board-comment-avatar-0-Y"));
    assert!(shown(visual, "board-comment-author-1-Agent"));
    assert!(shown(visual, "board-comment-avatar-1-A"));
    assert!(!shown(visual, "board-comment-author-0-Agent"));
    assert!(!shown(visual, "board-comment-author-1-You"));
    assert_eq!(super::detail::comment_author("user"), ("You", "Y", false));
    assert_eq!(super::detail::comment_author("agent"), ("Agent", "A", true));
    // Distinct avatars: the author and the initial both differ.
    assert_ne!(
        super::detail::comment_author("user"),
        super::detail::comment_author("agent")
    );
}

const MORE_ORDER: [&str; 10] = [
    "open",
    "status",
    "priority",
    "labels",
    "move-board",
    "copy-link",
    "copy-ref",
    "duplicate",
    "archive",
    "delete",
];

fn assert_more_menu_in_order(visual: &mut VisualTestContext) {
    visual.refresh().unwrap();
    let ys: Vec<_> = MORE_ORDER
        .iter()
        .map(|id| {
            visual
                .debug_bounds(selector(format!("menu-action-{id}")))
                .unwrap_or_else(|| panic!("no {id} item"))
                .origin
                .y
        })
        .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
}

#[gpui::test]
fn more_menu_items_in_order(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    click(visual, "board-drawer-more");
    assert_more_menu_in_order(visual);
}

fn right_click(visual: &mut VisualTestContext, target: &'static str) {
    visual.refresh().unwrap();
    let at = visual.debug_bounds(target).unwrap().center();
    visual.simulate_mouse_down(at, gpui::MouseButton::Right, Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

#[gpui::test]
fn right_click_on_a_card_opens_the_more_menu(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    right_click(visual, "board-card-1");
    assert_more_menu_in_order(visual);
    // It opened a menu, not the drawer.
    f.window
        .read_with(visual, |view, _| assert!(view.card_detail.is_none()))
        .unwrap();
    // Status… from the face opens the drawer on that card's status menu.
    click(visual, "menu-action-status");
    assert!(shown(visual, "board-drawer"));
    let step = detail(&f, visual).columns[1].id.clone();
    assert!(shown(
        visual,
        selector(format!("menu-action-status-{step}"))
    ));
}

#[gpui::test]
fn duplicate_from_more_menu(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task", "Other"]);
    let original = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    right_click(visual, "board-card-1");
    click(visual, "menu-action-duplicate");
    // The copy sits right after the original.
    assert_eq!(
        titles_in(&f, visual, "Backlog"),
        ["Task", "Task (copy)", "Other"]
    );
    assert_eq!(toast(&f, visual).as_deref(), Some("Duplicated #1 as #3"));
    assert_ne!(card_id(&f, visual, "Task (copy)"), original);
}

#[gpui::test]
fn duplicate_includes_an_unsaved_description_typed_in_the_drawer(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    retype(visual, "board-drawer-description", "typed, not yet saved");
    let id = card_id(&f, visual, "Task");
    // Run it straight from the menu's action, with no blur in between.
    f.window
        .update(visual, |view, _, cx| view.duplicate_card(&id, cx))
        .unwrap();
    visual.run_until_parked();
    let copy = detail(&f, visual)
        .cards
        .into_iter()
        .find(|c| c.title == "Task (copy)")
        .expect("the copy exists");
    assert_eq!(copy.body, "typed, not yet saved");
    // The original was saved first, and the drawer stays open on it.
    assert_eq!(
        Service::open(&f.db).unwrap().card(&id).unwrap().body,
        "typed, not yet saved"
    );
    assert!(shown(visual, "board-drawer"));
}

#[gpui::test]
fn the_card_menu_has_a_more_item_that_opens_the_full_menu(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-menu-1");
    assert!(shown(visual, "menu-action-more"));
    click(visual, "menu-action-more");
    assert_more_menu_in_order(visual);
    // The full menu is live: Duplicate works from it.
    click(visual, "menu-action-duplicate");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["Task", "Task (copy)"]);
}

#[gpui::test]
fn move_to_board_from_more_menu(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task", "Stay"]);
    let first = open_id(&f, cx);
    let sprint = Service::open(&f.db).unwrap().create("Sprint").unwrap();
    reload(&f, cx);
    let card = card_id(&f, cx, "Task");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    right_click(visual, "board-card-1");
    click(visual, "menu-action-move-board");
    // Only the other boards are listed.
    assert!(shown(
        visual,
        selector(format!("menu-action-to-board-{}", sprint.id))
    ));
    assert!(!shown(
        visual,
        selector(format!("menu-action-to-board-{first}"))
    ));
    click(
        visual,
        selector(format!("menu-action-to-board-{}", sprint.id)),
    );
    assert_eq!(titles_in(&f, visual, "Backlog"), ["Stay"]);
    assert_eq!(
        Service::open(&f.db).unwrap().card(&card).unwrap().board_id,
        sprint.id
    );
    assert!(toast(&f, visual).unwrap().starts_with("Moved #1 to Sprint"));
}

#[gpui::test]
fn move_to_board_with_no_other_board_says_so(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    right_click(visual, "board-card-1");
    click(visual, "menu-action-move-board");
    assert!(shown(visual, "menu-action-no-boards"));
    assert_eq!(titles_in(&f, visual, "Backlog"), ["Task"]);
}

#[gpui::test]
fn copy_link_and_copy_reference_write_the_clipboard(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix the bug"]);
    let card = card_id(&f, cx, "Fix the bug");
    let board = open_id(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    right_click(visual, "board-card-1");
    click(visual, "menu-action-copy-link");
    let copied = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(
        copied.as_deref(),
        Some(format!("gespan://board/{board}/card/{card}").as_str())
    );
    right_click(visual, "board-card-1");
    click(visual, "menu-action-copy-ref");
    let copied = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("#1 Fix the bug"));
}

#[gpui::test]
fn archive_and_delete_from_more_menu_show_undo(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B"]);
    let (a, b) = (card_id(&f, cx, "A"), card_id(&f, cx, "B"));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    right_click(visual, "board-card-1");
    click(visual, "menu-action-archive");
    assert_eq!(archived_ids(&f, visual), std::slice::from_ref(&a));
    assert_eq!(toast(&f, visual).as_deref(), Some("Archived #1"));
    click(visual, "board-undo");
    assert!(archived_ids(&f, visual).is_empty());
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A", "B"]);
    // Delete is held: archived at once, gone when the toast goes.
    right_click(visual, "board-card-2");
    click(visual, "menu-action-delete");
    assert_eq!(toast(&f, visual).as_deref(), Some("Deleted #2"));
    assert_eq!(archived_ids(&f, visual), std::slice::from_ref(&b));
    click(visual, "board-undo");
    assert!(archived_ids(&f, visual).is_empty());
    assert!(card_exists(&f, &b));
}

// ---- New card modal (⌘N) ----

const MOD: &str = if cfg!(target_os = "macos") {
    "cmd"
} else {
    "ctrl"
};

fn focus_board(f: &Fixture, cx: &mut VisualTestContext) {
    f.window
        .update(cx, |view, window, cx| window.focus(&view.focus, cx))
        .unwrap();
    cx.run_until_parked();
}

fn modal_open(f: &Fixture, cx: &mut VisualTestContext) -> bool {
    f.window
        .read_with(cx, |view, _| view.new_card.is_some())
        .unwrap()
}

fn open_modal(f: &Fixture, visual: &mut VisualTestContext) {
    focus_board(f, visual);
    visual.simulate_keystrokes(&format!("{MOD}-n"));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(modal_open(f, visual), "{MOD}-n did not open the modal");
}

fn new_card_text(f: &Fixture, cx: &mut VisualTestContext) -> (String, String) {
    f.window
        .read_with(cx, |view, cx| {
            let form = view.new_card.as_ref().unwrap();
            (
                form.title.read(cx).content.to_string(),
                form.description.read(cx).content.to_string(),
            )
        })
        .unwrap()
}

fn cards_titled(f: &Fixture, title: &str) -> Vec<gespan_core::board::Card> {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service
        .get(&board.id)
        .unwrap()
        .cards
        .into_iter()
        .filter(|card| card.title == title)
        .collect()
}

fn card_count(f: &Fixture) -> usize {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service.get(&board.id).unwrap().cards.len()
}

#[gpui::test]
fn cmd_n_opens_the_new_card_modal_on_the_board(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(!modal_open(&f, visual));
    open_modal(&f, visual);
    assert!(shown(visual, "new-card"));
    // The title takes the keys at once.
    visual.simulate_input("Hello");
    assert_eq!(new_card_text(&f, visual).0, "Hello");
    // Pressing it again does not stack a second modal or lose the text.
    visual.simulate_keystrokes(&format!("{MOD}-n"));
    assert_eq!(new_card_text(&f, visual).0, "Hello");
}

#[gpui::test]
fn the_default_column_is_the_first_todo_column_else_the_first(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let mut columns = detail(&f, cx).columns;
    for column in &mut columns {
        column.role = None;
    }
    assert_eq!(
        new_card::default_column(&columns),
        Some(columns[0].id.clone())
    );
    columns[2].role = Some("todo".into());
    columns[3].role = Some("todo".into());
    assert_eq!(
        new_card::default_column(&columns),
        Some(columns[2].id.clone())
    );
    assert_eq!(new_card::default_column(&[]), None);
}

#[gpui::test]
fn cmd_n_in_the_search_box_opens_the_modal_without_typing(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, window, cx| {
            let focus = view.search.focus_handle(cx);
            window.focus(&focus, cx)
        })
        .unwrap();
    visual.simulate_input("ab");
    visual.simulate_keystrokes(&format!("{MOD}-n"));
    visual.run_until_parked();
    assert!(modal_open(&f, visual));
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(view.search.read(cx).content.as_ref(), "ab");
        })
        .unwrap();
    assert_eq!(new_card_text(&f, visual), (String::new(), String::new()));
}

/// A stand-in for the shell: a `Workspace` context that owns ⌘N, with the
/// board inside it.
struct Workspace {
    board: gpui::Entity<BoardView>,
    focus: gpui::FocusHandle,
    new_codex: Rc<AtomicUsize>,
}
impl gpui::Render for Workspace {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.new_codex.clone();
        gpui::div()
            .size_full()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(move |_, _: &crate::shell::NewCodex, _, _| {
                count.fetch_add(1, Ordering::Relaxed);
            }))
            .child(self.board.clone())
    }
}

#[gpui::test]
fn the_workspace_new_codex_shortcut_still_works_outside_the_board(cx: &mut TestAppContext) {
    cx.update(crate::input::bind);
    cx.update(crate::popover::bind);
    cx.update(crate::shell::bind_keys);
    cx.update(new_card::bind);
    cx.update(|cx| cx.set_reduce_motion(true));
    let dir = Scratch::new();
    let db = dir.0.join("gespan.db");
    let store: crate::runtime::Database = Arc::new(Mutex::new(
        Store::open_gespan(Path::new(":memory:")).unwrap(),
    ));
    let new_codex = Rc::new(AtomicUsize::new(0));
    let count = new_codex.clone();
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let board = cx.new(|cx| BoardView::new(store, db, window, cx));
            board.update(cx, |board, cx| board.shown(cx));
            cx.new(|cx| Workspace {
                board,
                focus: cx.focus_handle(),
                new_codex: count,
            })
        })
        .unwrap()
    });
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    // Focus on the workspace itself: the shell shortcut wins.
    window
        .update(visual, |view, window, cx| window.focus(&view.focus, cx))
        .unwrap();
    visual.simulate_keystrokes(&format!("{MOD}-n"));
    assert_eq!(new_codex.load(Ordering::Relaxed), 1);
    let board = window
        .read_with(visual, |view, _| view.board.clone())
        .unwrap();
    assert!(board.read_with(visual, |board, _| board.new_card.is_none()));
    // Focus inside the board: the board's binding wins and the shell is silent.
    let board_focus = board.read_with(visual, |board, _| board.focus.clone());
    window
        .update(visual, |_, window, cx| window.focus(&board_focus, cx))
        .unwrap();
    visual.simulate_keystrokes(&format!("{MOD}-n"));
    assert_eq!(new_codex.load(Ordering::Relaxed), 1);
    assert!(board.read_with(visual, |board, _| board.new_card.is_some()));
}

#[gpui::test]
fn title_is_required(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    assert!(!shown(visual, "new-card-title-error"));
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.refresh().unwrap();
    assert!(modal_open(&f, visual));
    assert!(shown(visual, "new-card-title-error"));
    assert_eq!(card_count(&f), 0);
    // Spaces are not a title.
    visual.simulate_input("   ");
    click(visual, "new-card-create");
    assert!(shown(visual, "new-card-title-error"));
    assert_eq!(card_count(&f), 0);
    // Typing a title clears the message, and the card is made.
    visual.simulate_input("Real");
    visual.refresh().unwrap();
    assert!(!shown(visual, "new-card-title-error"));
    click(visual, "new-card-create");
    assert!(!modal_open(&f, visual));
    assert_eq!(cards_titled(&f, "Real").len(), 1);
}

fn pick(visual: &mut VisualTestContext, opener: &'static str, row: &'static str) {
    click(visual, opener);
    click(visual, row);
}

#[gpui::test]
fn cmd_enter_submits_and_sets_every_field(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let board = detail(&f, cx);
    let review = board.columns[2].id.clone();
    let bug = {
        let mut service = Service::open(&f.db).unwrap();
        service
            .create_label(&board.board.id, "Bug", "e5534b")
            .unwrap()
    };
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    f.window
        .update(visual, |view, _, cx| {
            view.set_projects(
                vec![
                    ("/work/a".into(), "Alpha".into()),
                    ("/work/b".into(), "Beta".into()),
                ],
                cx,
            )
        })
        .unwrap();
    open_modal(&f, visual);
    visual.simulate_input("  Ship it  ");
    pick(visual, "new-card-column", "menu-action-column-2");
    pick(visual, "new-card-priority", "menu-action-priority-3");
    pick(visual, "new-card-due", "menu-action-due-tomorrow");
    click(visual, "new-card-labels");
    click(visual, selector(format!("menu-action-label-{}", bug.id)));
    // A multi-toggle stays open for the next pick.
    assert!(shown(
        visual,
        selector(format!("menu-action-label-{}", bug.id))
    ));
    visual.simulate_keystrokes("escape");
    click(visual, "new-card-projects");
    click(visual, "menu-action-project-0");
    click(visual, "menu-action-project-1");
    click(visual, "menu-action-project-0");
    visual.simulate_keystrokes("escape");
    assert!(
        modal_open(&f, visual),
        "Escape closed a menu, not the modal"
    );
    click(visual, "new-card-description");
    visual.simulate_input("Line one");
    visual.simulate_keystrokes("shift-enter");
    visual.simulate_input("Line two");
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    assert!(!modal_open(&f, visual));
    let cards = cards_titled(&f, "Ship it");
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.column_id, review);
    assert_eq!(card.body, "Line one\nLine two");
    let extras = extras_of(&f, &card.id);
    assert_eq!(extras.priority, 3);
    assert_eq!(extras.labels, [bug.id]);
    assert_eq!(extras.projects, ["/work/b"]);
    let tomorrow = chrono::Local::now()
        .date_naive()
        .checked_add_days(chrono::Days::new(1))
        .unwrap();
    assert_eq!(extras.due_at, super::style::day_start(tomorrow));
}

#[gpui::test]
fn a_typed_due_date_is_used_and_a_bad_one_writes_nothing(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    visual.simulate_input("Dated");
    pick(visual, "new-card-due", "menu-action-due-pick");
    visual.simulate_input("not a date");
    click(visual, "new-card-create");
    assert!(modal_open(&f, visual));
    assert!(shown(visual, "new-card-error"));
    assert_eq!(card_count(&f), 0);
    // Correct it in the same field.
    f.window
        .update(visual, |view, _, cx| {
            let date = view.new_card.as_ref().unwrap().date.clone();
            date.update(cx, |input, cx| input.set_text("2026-11-02".into(), cx));
        })
        .unwrap();
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    assert!(!modal_open(&f, visual));
    let card = &cards_titled(&f, "Dated")[0];
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
    assert_eq!(extras_of(&f, &card.id).due_at, super::style::day_start(day));
}

#[gpui::test]
fn create_more_keeps_the_modal_open_with_fields_kept(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    click(visual, "new-card-more");
    visual.simulate_input("First");
    pick(visual, "new-card-column", "menu-action-column-1");
    pick(visual, "new-card-priority", "menu-action-priority-4");
    click(visual, "new-card-description");
    visual.simulate_input("Notes");
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(modal_open(&f, visual), "create more closed the modal");
    assert_eq!(cards_titled(&f, "First").len(), 1);
    assert_eq!(new_card_text(&f, visual), (String::new(), String::new()));
    // The other fields stay, and the title has the keys again.
    visual.simulate_input("Second");
    assert_eq!(new_card_text(&f, visual).0, "Second");
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    let board = detail(&f, visual);
    let (first, second) = (
        &cards_titled(&f, "First")[0],
        &cards_titled(&f, "Second")[0],
    );
    assert_eq!(second.column_id, board.columns[1].id);
    assert_eq!(second.column_id, first.column_id);
    assert_eq!(extras_of(&f, &second.id).priority, 4);
    assert_eq!(extras_of(&f, &first.id).priority, 4);
    assert!(modal_open(&f, visual));
    // Unticked, the next create closes it.
    click(visual, "new-card-more");
    visual.simulate_input("Third");
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    assert!(!modal_open(&f, visual));
    assert_eq!(card_count(&f), 3);
}

#[gpui::test]
fn escape_closes_the_modal_without_creating_or_closing_the_board(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (closed, _events) = events(&f, visual);
    open_modal(&f, visual);
    visual.simulate_input("Draft");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!modal_open(&f, visual));
    assert_eq!(card_count(&f), 0);
    assert!(closed.borrow().is_empty(), "Escape also closed the board");
    // Cancel does the same, and a reopened modal starts fresh.
    open_modal(&f, visual);
    assert_eq!(new_card_text(&f, visual).0, "");
    click(visual, "new-card-cancel");
    assert!(!modal_open(&f, visual));
}

#[gpui::test]
fn a_failed_setter_keeps_the_created_card_and_says_so(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    visual.simulate_input("Odd");
    f.window
        .update(visual, |view, _, _| {
            view.new_card.as_mut().unwrap().labels = vec!["no-such-label".into()];
        })
        .unwrap();
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    visual.refresh().unwrap();
    // The card exists exactly once, the modal closed (Create more is off), and
    // the board says what was not saved.
    assert_eq!(cards_titled(&f, "Odd").len(), 1);
    assert!(!modal_open(&f, visual));
    assert!(shown(visual, "board-error"));
    // With Create more on, the modal stays and shows it inside.
    open_modal(&f, visual);
    click(visual, "new-card-more");
    visual.simulate_input("Odd two");
    f.window
        .update(visual, |view, _, _| {
            view.new_card.as_mut().unwrap().labels = vec!["no-such-label".into()];
        })
        .unwrap();
    visual.simulate_keystrokes(&format!("{MOD}-enter"));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(cards_titled(&f, "Odd two").len(), 1);
    assert!(modal_open(&f, visual));
    assert!(shown(visual, "new-card-error"));
    // A retry would need a new title: the modal never re-sends the old card.
    assert_eq!(new_card_text(&f, visual).0, "");
}

fn new_card_focused(f: &Fixture, visual: &mut VisualTestContext) -> Option<&'static str> {
    f.window
        .update(visual, |view, window, cx| {
            let form = view.new_card.as_ref()?;
            let named = [
                ("column", new_card::Property::Column),
                ("priority", new_card::Property::Priority),
                ("due", new_card::Property::Due),
                ("labels", new_card::Property::Labels),
                ("projects", new_card::Property::Projects),
            ];
            for (name, property) in named {
                if form.focus[&property].is_focused(window) {
                    return Some(name);
                }
            }
            if form.more_focus.is_focused(window) {
                return Some("more");
            }
            if form.create_focus.is_focused(window) {
                return Some("create");
            }
            if form.description.focus_handle(cx).is_focused(window) {
                return Some("description");
            }
            form.title
                .focus_handle(cx)
                .is_focused(window)
                .then_some("title")
        })
        .unwrap()
}

/// What the shell's Tab binding does: move focus along the tab order.
fn tab(visual: &mut VisualTestContext, steps: i32) {
    for _ in 0..steps.abs() {
        visual.update(|window, cx| {
            if steps > 0 {
                window.focus_next(cx)
            } else {
                window.focus_prev(cx)
            }
        });
    }
    visual.run_until_parked();
    visual.refresh().unwrap();
}

#[gpui::test]
fn the_modal_is_reachable_by_keyboard_in_order(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    let mut seen = vec![new_card_focused(&f, visual).unwrap()];
    for _ in 0..8 {
        tab(visual, 1);
        visual.refresh().unwrap();
        seen.push(new_card_focused(&f, visual).unwrap_or("elsewhere"));
    }
    assert_eq!(
        seen[..8],
        [
            "title",
            "description",
            "column",
            "priority",
            "due",
            "labels",
            "projects",
            "more"
        ]
    );
    assert_eq!(seen[8], "create");
    // Back to Column: Enter opens its menu, a pick sets it, focus returns.
    tab(visual, -6);
    assert_eq!(new_card_focused(&f, visual), Some("column"));
    visual.simulate_keystrokes("enter");
    visual.refresh().unwrap();
    assert!(shown(visual, "menu-action-column-2"));
    click(visual, "menu-action-column-2");
    assert_eq!(new_card_focused(&f, visual), Some("column"));
    let third = detail(&f, visual).columns[2].id.clone();
    f.window
        .read_with(visual, |view, _| {
            assert_eq!(view.new_card.as_ref().unwrap().column_id, third)
        })
        .unwrap();
    // Down opens a pill's menu too; Space toggles Create more.
    tab(visual, 1);
    visual.simulate_keystrokes("down");
    visual.refresh().unwrap();
    assert!(shown(visual, "menu-action-priority-3"));
    visual.simulate_keystrokes("escape");
    tab(visual, 4);
    assert_eq!(new_card_focused(&f, visual), Some("more"));
    visual.simulate_keystrokes("space");
    let on = |visual: &mut VisualTestContext| {
        f.window
            .read_with(visual, |view, _| {
                view.new_card.as_ref().unwrap().create_more
            })
            .unwrap()
    };
    assert!(on(visual));
    visual.simulate_keystrokes("space");
    assert!(!on(visual));
    assert!(modal_open(&f, visual), "Escape on a menu closed the modal");
}

#[gpui::test]
fn a_quick_pick_drops_a_half_typed_date(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    pick(visual, "new-card-due", "menu-action-due-pick");
    visual.simulate_input("2026-1");
    pick(visual, "new-card-due", "menu-action-due-today");
    f.window
        .read_with(visual, |view, cx| {
            let form = view.new_card.as_ref().unwrap();
            assert_eq!(form.date.read(cx).content.to_string(), "");
            assert!(!form.typing_date);
        })
        .unwrap();
}

#[gpui::test]
fn opening_the_modal_closes_the_archive_view(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    f.window
        .update(visual, |view, _, cx| view.toggle_archive(cx))
        .unwrap();
    open_modal(&f, visual);
    f.window
        .read_with(visual, |view, _| assert!(!view.archive_open))
        .unwrap();
}

#[gpui::test]
fn a_rapid_double_submit_makes_one_card(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    open_modal(&f, visual);
    visual.simulate_input("Once");
    visual.simulate_keystrokes(&format!("{MOD}-enter {MOD}-enter"));
    visual.run_until_parked();
    assert_eq!(cards_titled(&f, "Once").len(), 1);
    // With Create more the second press meets an empty title.
    open_modal(&f, visual);
    click(visual, "new-card-more");
    visual.simulate_input("Twice");
    visual.simulate_keystrokes(&format!("{MOD}-enter {MOD}-enter"));
    visual.run_until_parked();
    assert_eq!(cards_titled(&f, "Twice").len(), 1);
    assert_eq!(card_count(&f), 2);
}

// ---- Filter builder, sort and saved views ----

fn visible_titles(f: &Fixture, cx: &mut impl gpui::AppContext, column: &str) -> Vec<String> {
    let column_id = column_id(f, cx, column);
    f.window
        .read_with(cx, |view, _| {
            let detail = view.detail.as_ref().unwrap();
            view.column_cards(detail, &column_id)
                .iter()
                .map(|card| card.title.clone())
                .collect()
        })
        .unwrap()
}

fn stored_views(f: &Fixture) -> Vec<gespan_core::board::extras::View> {
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    service.views(&board.id).unwrap()
}

fn due_in(f: &Fixture, cx: &mut TestAppContext, title: &str, days: i64) {
    let now = gespan_core::time::now_ms() as i64;
    let id = card_id(f, cx, title);
    Service::open(&f.db)
        .unwrap()
        .set_due(&id, Some(now + days * 86_400_000))
        .unwrap();
}

#[gpui::test]
fn search_finds_text_in_the_description(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let backlog = detail(&f, cx).columns[0].id.clone();
    let mut service = Service::open(&f.db).unwrap();
    service
        .card_create(&backlog, "Plain title", "the needle is in here")
        .unwrap();
    service.card_create(&backlog, "Other", "nothing").unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-search");
    visual.simulate_input("NEEDLE");
    visual.run_until_parked();
    assert_eq!(shown_cards(visual, &[1, 2]), [1]);
    f.window
        .read_with(visual, |view, cx| {
            assert_eq!(
                view.count_label(&view.detail.as_ref().unwrap().columns[0].id, cx),
                "1/2"
            )
        })
        .unwrap();
}

#[gpui::test]
fn filter_builder_adds_where_and_rows_and_shows_chips(cx: &mut TestAppContext) {
    let f = seeded(cx, &["High", "Low", "Plain"]);
    set_priorities(&f, cx, &[("High", 3), ("Low", 1)]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-add-priority");
    // Urgent first, so High is the second value.
    click(visual, "board-filter-row-0-value-1");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1]);
    click(visual, "board-filter-add-status");
    assert_eq!(BoardView::lead_label(0), "Where");
    assert_eq!(BoardView::lead_label(1), "and");
    click(visual, "board-filter-row-1-value-0");
    // The second row ANDs with the first: High and in Backlog.
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1]);
    drawer(&f, visual, |view, _| {
        assert_eq!(view.spec.rows.len(), 2);
        assert_eq!(view.chip_text(&view.spec.rows[0]), "Priority is High");
        assert_eq!(view.chip_text(&view.spec.rows[1]), "Status is Backlog");
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("board-chip-0").is_some());
    assert!(visual.debug_bounds("board-chip-1").is_some());
    // "is not" flips the first row: everything but High.
    click(visual, "board-filter");
    click(visual, "board-filter-row-0-op");
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [2, 3]);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    // A chip's x removes just its row.
    click(visual, "board-chip-0-remove");
    drawer(&f, visual, |view, _| {
        assert_eq!(view.spec.rows.len(), 1);
        assert_eq!(view.chip_text(&view.spec.rows[0]), "Status is Backlog");
    });
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 2, 3]);
}

#[gpui::test]
fn clear_all_removes_every_row(cx: &mut TestAppContext) {
    let f = seeded(cx, &["High", "Plain"]);
    set_priorities(&f, cx, &[("High", 3)]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-add-priority");
    click(visual, "board-filter-row-0-value-1");
    click(visual, "board-filter-add-status");
    click(visual, "board-filter-row-1-value-3");
    assert_eq!(shown_cards(visual, &[1, 2]), Vec::<i64>::new());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    click(visual, "board-chips-clear");
    drawer(&f, visual, |view, _| assert!(view.spec.rows.is_empty()));
    assert_eq!(shown_cards(visual, &[1, 2]), [1, 2]);
    assert!(visual.debug_bounds("board-chips").is_none());
}

fn sort_by(visual: &mut VisualTestContext, key: &'static str) {
    click(visual, "board-sort");
    click(visual, key);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
}

#[gpui::test]
fn sort_by_due_orders_columns_and_disables_in_column_drag(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Later", "Soon", "Undated", "Sooner"]);
    due_in(&f, cx, "Later", 9);
    due_in(&f, cx, "Soon", 3);
    due_in(&f, cx, "Sooner", 1);
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    sort_by(visual, "board-sort-due");
    assert_eq!(
        visible_titles(&f, visual, "Backlog"),
        ["Sooner", "Soon", "Later", "Undated"],
        "undated cards sort last"
    );
    assert!(visual.debug_bounds("board-sorted-by").is_some());
    drawer(&f, visual, |view, _| assert!(view.sorted()));
    // Descending keeps undated cards last.
    click(visual, "board-sort");
    click(visual, "board-sort-dir-desc");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(
        visible_titles(&f, visual, "Backlog"),
        ["Later", "Soon", "Sooner", "Undated"]
    );
    // Dragging the last card to the top of its own column changes nothing.
    let stored = titles_in(&f, visual, "Backlog");
    let top = visual.debug_bounds("board-card-1").unwrap();
    let from = center(visual, "board-card-3");
    drag(
        visual,
        from,
        gpui::point(top.center().x, top.top() + px(2.)),
    );
    assert_eq!(titles_in(&f, visual, "Backlog"), stored);
    let (backlog, moved) = (
        column_id(&f, visual, "Backlog"),
        card_id(&f, visual, "Undated"),
    );
    drawer(&f, visual, |view, _| {
        assert_eq!(view.resolve_drop(&moved, &backlog, 0), None);
    });
    // Back to manual: reordering inside the column works again.
    sort_by(visual, "board-sort-manual");
    drawer(&f, visual, |view, _| {
        assert!(!view.sorted());
        assert_eq!(
            view.resolve_drop(&moved, &backlog, 0),
            Some((backlog.clone(), 0))
        );
    });
}

#[gpui::test]
fn cross_column_drop_still_works_while_sorted(cx: &mut TestAppContext) {
    let f = seeded(cx, &["A", "B", "C"]);
    due_in(&f, cx, "A", 5);
    due_in(&f, cx, "B", 2);
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    // Put one card in Review so the drop has something to append after.
    let review = column_id(&f, visual, "Review");
    let seeded_card = card_id(&f, visual, "C");
    Service::open(&f.db)
        .unwrap()
        .card_move(&seeded_card, &review, 0, "user")
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    sort_by(visual, "board-sort-due");
    assert_eq!(visible_titles(&f, visual, "Backlog"), ["B", "A"]);
    // B is first on screen (card number 2); drop it on the Review column.
    let from = center(visual, "board-card-2");
    let to = center(visual, "board-column-body-2");
    drag(visual, from, to);
    assert_eq!(titles_in(&f, visual, "Review"), ["C", "B"], "appended");
    assert_eq!(titles_in(&f, visual, "Backlog"), ["A"]);
    drawer(&f, visual, |view, _| assert!(view.sorted()));
}

#[gpui::test]
fn save_view_creates_a_tab_and_switching_restores_spec_and_layout(cx: &mut TestAppContext) {
    let f = seeded(cx, &["High one", "High two", "Low"]);
    set_priorities(&f, cx, &[("High one", 3), ("High two", 3), ("Low", 1)]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-filter");
    click(visual, "board-filter-add-priority");
    click(visual, "board-filter-row-0-value-1");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    sort_by(visual, "board-sort-title");
    click(visual, "board-search");
    visual.simulate_input("two");
    visual.run_until_parked();
    // A layout the view carries comes back as it was saved.
    let layout = r#"{"layout":"board","widths":{"title":220}}"#;
    f.window
        .update(visual, |view, _, _| view.views.layout = layout.to_string())
        .unwrap();
    click(visual, "board-view-new");
    visual.simulate_input("Urgent");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    let saved = stored_views(&f);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Urgent");
    assert_eq!(saved[0].layout_json, layout);
    let spec: gespan_core::board::extras::ViewSpec =
        serde_json::from_str(&saved[0].spec_json).unwrap();
    assert_eq!(spec.query, "two");
    assert_eq!(spec.rows.len(), 1);
    assert!(visual.debug_bounds("board-view-tab-0").is_some());
    // Saving shows the new tab as the active, unchanged view.
    drawer(&f, visual, |view, cx| {
        assert_eq!(view.views.active.as_deref(), Some(saved[0].id.as_str()));
        assert!(!view.view_dirty(cx));
    });
    // All cards: nothing searched, filtered or sorted, default layout.
    click(visual, "board-view-tab-all");
    drawer(&f, visual, |view, cx| {
        assert_eq!(view.current_spec(cx), Default::default());
        assert_eq!(view.views.layout, views::DEFAULT_LAYOUT);
        assert_eq!(view.search.read(cx).content, "");
    });
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [1, 2, 3]);
    // The saved tab brings everything back.
    click(visual, "board-view-tab-0");
    drawer(&f, visual, |view, cx| {
        assert_eq!(view.current_spec(cx), spec);
        assert_eq!(view.views.layout, layout);
        assert_eq!(view.search.read(cx).content, "two");
    });
    assert_eq!(shown_cards(visual, &[1, 2, 3]), [2]);
}

#[gpui::test]
fn unsaved_changes_show_a_dot_with_save_and_revert(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Alpha", "Beta"]);
    let board = Service::open(&f.db).unwrap().default_board().unwrap();
    let spec = serde_json::to_string(&Default::default() as &gespan_core::board::extras::ViewSpec)
        .unwrap();
    Service::open(&f.db)
        .unwrap()
        .view_create(&board.id, "Mine", &spec, views::DEFAULT_LAYOUT)
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-view-tab-0");
    assert!(visual.debug_bounds("board-view-dot-0").is_none());
    assert!(visual.debug_bounds("board-view-save-changes").is_none());
    sort_by(visual, "board-sort-title");
    assert!(
        visual.debug_bounds("board-view-dot-0").is_some(),
        "a change shows the dot"
    );
    assert!(visual.debug_bounds("board-view-revert").is_some());
    // Revert goes back to what was saved.
    click(visual, "board-view-revert");
    assert!(visual.debug_bounds("board-view-dot-0").is_none());
    drawer(&f, visual, |view, _| assert!(!view.sorted()));
    // Save writes the change and clears the dot.
    sort_by(visual, "board-sort-title");
    click(visual, "board-view-save-changes");
    assert!(visual.debug_bounds("board-view-dot-0").is_none());
    let stored: gespan_core::board::extras::ViewSpec =
        serde_json::from_str(&stored_views(&f)[0].spec_json).unwrap();
    assert_eq!(stored.sort.key, gespan_core::board::extras::SortKey::Title);
    // Typing in the search is a change too.
    click(visual, "board-search");
    visual.simulate_input("alp");
    visual.run_until_parked();
    assert!(visual.debug_bounds("board-view-dot-0").is_some());
}

#[gpui::test]
fn all_cards_tab_cannot_be_deleted(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Alpha"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-view-tab-all").is_some());
    assert!(
        visual.debug_bounds("board-view-menu-all").is_none(),
        "the built-in tab has no menu"
    );
    // Even asked to, nothing is deleted and the tab stays first.
    f.window
        .update(visual, |view, _, cx| view.delete_view("all", cx))
        .unwrap();
    drawer(&f, visual, |view, _| assert_eq!(view.views.active, None));
    // A saved tab does have Rename, Duplicate and Delete.
    click(visual, "board-view-new");
    visual.simulate_input("Temp");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    click(visual, "board-view-menu-0");
    assert!(visual.debug_bounds("board-view-rename").is_some());
    assert!(visual.debug_bounds("board-view-duplicate").is_some());
    click(visual, "board-view-duplicate");
    assert_eq!(stored_views(&f).len(), 2);
    assert_eq!(stored_views(&f)[1].name, "Temp copy");
    click(visual, "board-view-menu-1");
    click(visual, "board-view-delete");
    assert_eq!(stored_views(&f).len(), 1);
    // Deleting the showing view falls back to All cards.
    click(visual, "board-view-tab-0");
    click(visual, "board-view-menu-0");
    click(visual, "board-view-delete");
    assert!(stored_views(&f).is_empty());
    drawer(&f, visual, |view, _| assert_eq!(view.views.active, None));
    assert!(visual.debug_bounds("board-view-tab-all").is_some());
}

#[gpui::test]
fn a_view_with_a_deleted_label_still_loads(cx: &mut TestAppContext) {
    let f = seeded(cx, &["One", "Two"]);
    let mut service = Service::open(&f.db).unwrap();
    let board = service.default_board().unwrap();
    let bug = service.create_label(&board.id, "Bug", "e5534b").unwrap();
    let spec = format!(
        r#"{{"query":"","rows":[{{"field":"labels","op":"is","values":["{}"]}}],"sort":{{"key":"manual","dir":"asc"}}}}"#,
        bug.id
    );
    service
        .view_create(&board.id, "Bugs", &spec, views::DEFAULT_LAYOUT)
        .unwrap();
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute("DELETE FROM board_labels", [])
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-view-tab-0");
    drawer(&f, visual, |view, _| {
        assert_eq!(view.error, None);
        assert_eq!(view.load_error, None);
        assert_eq!(view.spec.rows.len(), 1, "the row stays as it was saved");
    });
    assert_eq!(
        shown_cards(visual, &[1, 2]),
        [1, 2],
        "a row whose values are all gone is ignored, not zero results"
    );
}
