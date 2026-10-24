use super::detail::activity_text;
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
    cx.update(crate::popover::bind);
    cx.update(|cx| cx.set_reduce_motion(true));
    let dir = Scratch::new();
    let db = dir.0.join("rebase.db");
    let store: crate::runtime::Database = Arc::new(Mutex::new(
        Store::open_rebase(Path::new(":memory:")).unwrap(),
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

fn detail(f: &Fixture, cx: &mut impl gpui::AppContext) -> rebase_core::board::BoardDetail {
    f.window
        .read_with(cx, |view, _| view.detail.clone().expect("a board is open"))
        .unwrap()
}

fn with_board(cx: &mut TestAppContext) -> Fixture {
    open(cx)
}

#[gpui::test]
fn the_board_opens_with_four_steps_and_no_board_switcher(cx: &mut TestAppContext) {
    let f = with_board(cx);
    let detail = detail(&f, cx);
    let names: Vec<&str> = detail.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Backlog", "In Progress", "Review", "Done"]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-header").is_some());
    assert!(visual.debug_bounds("board-switcher").is_none());
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
    raw.execute_batch("ALTER TABLE board_native_labels RENAME TO hidden_labels;")
        .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-error").is_some());
    raw.execute_batch("ALTER TABLE hidden_labels RENAME TO board_native_labels;")
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
            view.db_path = blocker.join("rebase.db");
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
    let row = rebase_core::board::Activity {
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
        .set_due(&rich, Some(rebase_core::time::now_ms() as i64))
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

fn extras_of(f: &Fixture, card: &str) -> rebase_core::board::extras::CardExtras {
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
    click(visual, "menu-action-due-tomorrow");
    let due = extras_of(&f, &card).due_at.expect("a due date");
    let now = rebase_core::time::now_ms() as i64;
    assert_eq!(
        super::style::due_label(due, now).map(|(text, _)| text),
        Some("Tomorrow".to_string())
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
        .execute_batch("ALTER TABLE board_native_comments RENAME TO hidden_comments;")
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
