//! Moves board cards as native session turns start and finish. One app-wide
//! `LinkedTurns` keeps turn keys alive across conversation views; the card
//! moves run on a background thread so a board failure never touches a turn.
use crate::{
    pilots::Pilots,
    runtime::{Database, locked},
};
use gespan_core::{
    board::{
        Service,
        auto_move::on_linked_work,
        linked_turns::{LinkedCall, LinkedTurns, turn_pilot_id},
    },
    pilot::Roster,
};
use gpui::{App, Global, WeakEntity};
use std::path::PathBuf;

pub(crate) struct BoardTurns {
    turns: LinkedTurns,
    /// One worker applies batches in order, so a turn's "started" always
    /// lands before its "completed" or "failed".
    queue: async_channel::Sender<Vec<LinkedCall>>,
    pilots: Option<WeakEntity<Pilots>>,
}
impl Global for BoardTurns {}

/// Start moving cards with turns, against the board tables at `db_path`.
pub(crate) fn install(db_path: PathBuf, pilots: Option<WeakEntity<Pilots>>, cx: &mut App) {
    let (queue, batches) = async_channel::unbounded::<Vec<LinkedCall>>();
    cx.background_executor()
        .spawn(async move {
            let mut service = None;
            while let Ok(calls) = batches.recv().await {
                if service.is_none() {
                    match Service::open(&db_path) {
                        Ok(opened) => service = Some(opened),
                        Err(error) => {
                            eprintln!("gespan: board unavailable: {error}");
                            continue;
                        }
                    }
                }
                let Some(service) = service.as_mut() else {
                    continue;
                };
                for (kind, id, event) in calls {
                    if let Err(error) = on_linked_work(service, kind, &id, event) {
                        eprintln!("gespan: moving board cards failed: {error}");
                    }
                }
            }
        })
        .detach();
    cx.set_global(BoardTurns {
        turns: LinkedTurns::default(),
        queue,
        pilots,
    });
}

/// The Pilot a session's turn counts for: its own chat, a habit run, or a
/// session the Pilot launched.
pub(crate) fn turn_pilot(roster: &Roster, session: i64) -> Option<String> {
    let own = roster.of_session(session).map(|p| p.id.as_str());
    let habit = roster
        .pilots
        .iter()
        .find(|p| p.habit_sessions.contains(&session))
        .map(|p| p.id.as_str());
    let launcher = roster
        .pilots
        .iter()
        .find(|p| p.launches.iter().any(|l| l.session_id == session))
        .map(|p| p.id.as_str());
    turn_pilot_id(own, habit, launcher)
}

/// The Pilot behind a session, from the app's live roster.
pub(crate) fn pilot_of_session(session: i64, cx: &App) -> Option<String> {
    let pilots = cx.try_global::<BoardTurns>()?.pilots.as_ref()?.upgrade()?;
    turn_pilot(&pilots.read(cx).roster, session)
}

/// Every Pilot as `(id, name)`, for linking cards and naming link chips.
pub(crate) fn pilots(cx: &App) -> Vec<(String, String)> {
    let Some(pilots) = cx
        .try_global::<BoardTurns>()
        .and_then(|board| board.pilots.as_ref()?.upgrade())
    else {
        return Vec::new();
    };
    pilots
        .read(cx)
        .roster
        .pilots
        .iter()
        .map(|pilot| (pilot.id.clone(), pilot.name.clone()))
        .collect()
}

/// A session's turn began: record it for the Pilot behind the session. Pilot
/// chats start turns from inside a `Pilots` update, and finding the Pilot reads
/// the roster, so this waits until the current update has finished.
pub(crate) fn turn_began(store: Database, session: i64, turn: u64, cx: &mut App) {
    cx.defer(move |cx| {
        let pilot = pilot_of_session(session, cx);
        turn_started(&store, session, pilot, turn, cx);
    });
}

fn session_key(store: &Database, session: i64) -> Option<String> {
    locked(store).ok()?.persisted_session_id(session).ok()
}

pub(crate) fn turn_started(
    store: &Database,
    session: i64,
    pilot: Option<String>,
    turn: u64,
    cx: &mut App,
) {
    if !cx.has_global::<BoardTurns>() {
        return;
    }
    let Some(key) = session_key(store, session) else {
        return;
    };
    let calls = cx
        .global_mut::<BoardTurns>()
        .turns
        .started(&key, turn, pilot);
    dispatch(calls, cx);
}

/// A finished turn: `stopped` when the user stopped it, else `ok` when it
/// ended without an error.
pub(crate) fn turn_settled(
    store: &Database,
    session: i64,
    turn: u64,
    stopped: bool,
    ok: bool,
    cx: &mut App,
) {
    if !cx.has_global::<BoardTurns>() {
        return;
    }
    let Some(key) = session_key(store, session) else {
        return;
    };
    let turns = &mut cx.global_mut::<BoardTurns>().turns;
    let calls = if stopped {
        turns.cancelled(&key, turn);
        Vec::new()
    } else {
        turns.settled(&key, turn, ok)
    };
    dispatch(calls, cx);
}

/// What one runtime update says about a local turn, for the Board.
pub(crate) struct TurnUpdate {
    pub finished: bool,
    /// The user asked the turn to stop.
    pub stopping: bool,
    /// The provider finished the turn without an error (final update only).
    pub succeeded: bool,
    /// The update carries transcript rows, so the prompt was taken.
    pub has_rows: bool,
}

/// One local turn's life on the Board, fed every runtime update the
/// conversation applies. The Board turn starts on the first update that
/// shows the prompt was taken, so a rejected preflight never moves a card,
/// and a view that goes away mid-turn closes its turn like a stop.
#[derive(Default)]
pub(crate) struct TurnTracker {
    began: bool,
    settled: bool,
}

impl TurnTracker {
    pub(crate) fn observe(
        &mut self,
        store: &Database,
        session: i64,
        turn: u64,
        update: TurnUpdate,
        cx: &mut App,
    ) {
        if self.settled {
            return;
        }
        if !self.began {
            // A final update first means nothing ran: the runtime rejected
            // the prompt, or the user stopped it before it started.
            if update.finished && (update.stopping || !(update.has_rows || update.succeeded)) {
                self.settled = true;
                return;
            }
            self.began = true;
            turn_began(store.clone(), session, turn, cx);
        }
        if update.finished {
            self.settle(store, session, turn, update.stopping, update.succeeded, cx);
        }
    }

    /// The conversation stopped reading updates before the final one.
    pub(crate) fn abandon(&mut self, store: &Database, session: i64, turn: u64, cx: &mut App) {
        if self.began && !self.settled {
            self.settle(store, session, turn, true, false, cx);
        }
    }

    fn settle(
        &mut self,
        store: &Database,
        session: i64,
        turn: u64,
        stopped: bool,
        ok: bool,
        cx: &mut App,
    ) {
        self.settled = true;
        let store = store.clone();
        // Deferred like `turn_began`, so a turn's start is always queued first.
        cx.defer(move |cx| turn_settled(&store, session, turn, stopped, ok, cx));
    }
}

fn dispatch(calls: Vec<LinkedCall>, cx: &mut App) {
    if !calls.is_empty() {
        let _ = cx.global::<BoardTurns>().queue.try_send(calls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::{
        board::Service,
        pilot::{Launch, Roster},
        store::Store,
    };
    use gpui::TestAppContext;
    use std::{
        path::Path,
        sync::{Arc, Mutex},
    };

    struct Setup {
        store: Database,
        db: PathBuf,
        session: i64,
        card: String,
        board: String,
        _dir: TempDir,
    }

    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(cx: &mut TestAppContext, link: Option<(&str, &str)>) -> Setup {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gespan-board-turns-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("gespan.db");
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/work").unwrap().id;
        let persisted = store.persisted_session_id(session).unwrap();
        let store: Database = Arc::new(Mutex::new(store));
        let mut service = Service::open(&db).unwrap();
        let board = service.create("Work").unwrap();
        let backlog = service.get(&board.id).unwrap().columns[0].id.clone();
        let card = service.card_create(&backlog, "Linked", "").unwrap();
        let (kind, id) = link.unwrap_or(("session", persisted.as_str()));
        service.card_link(&card.id, Some(kind), Some(id)).unwrap();
        let path = db.clone();
        cx.update(|cx| install(path, None, cx));
        Setup {
            store,
            db,
            session,
            card: card.id,
            board: board.id,
            _dir: TempDir(dir),
        }
    }

    fn state(s: &Setup) -> (String, Vec<String>) {
        let mut service = Service::open(&s.db).unwrap();
        let detail = service.get(&s.board).unwrap();
        let card = detail.cards.iter().find(|c| c.id == s.card).unwrap();
        let column = detail
            .columns
            .iter()
            .find(|c| c.id == card.column_id)
            .unwrap()
            .name
            .clone();
        let kinds = service
            .activity(&s.board, Some(&s.card), 20)
            .unwrap()
            .into_iter()
            .map(|row| row.kind)
            .filter(|kind| kind != "created" && kind != "linked")
            .collect();
        (column, kinds)
    }

    #[gpui::test]
    fn a_linked_session_turn_moves_its_card_to_doing_then_review(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        cx.update(|cx| turn_started(&s.store, s.session, None, 1, cx));
        cx.run_until_parked();
        assert_eq!(state(&s).0, "In Progress");
        cx.update(|cx| turn_settled(&s.store, s.session, 1, false, true, cx));
        cx.run_until_parked();
        let (column, kinds) = state(&s);
        assert_eq!(column, "Review");
        assert_eq!(kinds, ["completed", "moved", "started", "moved"]);
    }

    #[gpui::test]
    fn an_error_finish_logs_failed_and_the_card_stays(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        cx.update(|cx| turn_started(&s.store, s.session, None, 1, cx));
        cx.update(|cx| turn_settled(&s.store, s.session, 1, false, false, cx));
        cx.run_until_parked();
        let (column, kinds) = state(&s);
        assert_eq!(column, "In Progress");
        assert_eq!(kinds[0], "failed");
    }

    #[gpui::test]
    fn a_stopped_turn_logs_only_started(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        cx.update(|cx| turn_started(&s.store, s.session, None, 1, cx));
        cx.update(|cx| turn_settled(&s.store, s.session, 1, true, false, cx));
        cx.run_until_parked();
        let (column, kinds) = state(&s);
        assert_eq!(column, "In Progress");
        assert_eq!(kinds, ["started", "moved"]);
    }

    #[gpui::test]
    fn a_turn_for_a_pilot_also_moves_the_pilots_card(cx: &mut TestAppContext) {
        let s = setup(cx, Some(("pilot", "p-1")));
        cx.update(|cx| turn_started(&s.store, s.session, Some("p-1".into()), 1, cx));
        cx.run_until_parked();
        assert_eq!(state(&s).0, "In Progress");
    }

    #[gpui::test]
    fn without_the_board_installed_turns_change_nothing(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        cx.update(|cx| cx.remove_global::<BoardTurns>());
        cx.update(|cx| turn_started(&s.store, s.session, None, 1, cx));
        cx.run_until_parked();
        assert_eq!(state(&s).0, "Backlog");
    }

    fn update(finished: bool, succeeded: bool, has_rows: bool) -> TurnUpdate {
        TurnUpdate {
            finished,
            stopping: false,
            succeeded,
            has_rows,
        }
    }

    fn observe(cx: &mut TestAppContext, s: &Setup, tracker: &mut TurnTracker, turn: TurnUpdate) {
        cx.update(|cx| tracker.observe(&s.store, s.session, 1, turn, cx));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_prompt_the_runtime_rejects_never_moves_its_card(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        let mut tracker = TurnTracker::default();
        // A rejected preflight's only update is a final one with no rows.
        observe(cx, &s, &mut tracker, update(true, false, false));
        assert_eq!(state(&s), ("Backlog".to_string(), Vec::<String>::new()));
    }

    #[gpui::test]
    fn a_tracked_turn_starts_on_its_first_update_and_settles_on_its_last(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        let mut tracker = TurnTracker::default();
        observe(cx, &s, &mut tracker, update(false, false, true));
        assert_eq!(state(&s).0, "In Progress");
        observe(cx, &s, &mut tracker, update(false, false, true));
        observe(cx, &s, &mut tracker, update(true, true, true));
        let (column, kinds) = state(&s);
        assert_eq!(column, "Review");
        assert_eq!(kinds, ["completed", "moved", "started", "moved"]);
    }

    #[gpui::test]
    fn a_turn_whose_only_update_is_a_failure_starts_then_fails(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        let mut tracker = TurnTracker::default();
        // A provider that fails to launch still took the prompt (its rows).
        observe(cx, &s, &mut tracker, update(true, false, true));
        let (column, kinds) = state(&s);
        assert_eq!(column, "In Progress");
        assert_eq!(kinds, ["failed", "started", "moved"]);
    }

    #[gpui::test]
    fn a_view_closed_mid_turn_frees_its_turn_for_the_next_view(cx: &mut TestAppContext) {
        let s = setup(cx, None);
        let mut closed = TurnTracker::default();
        observe(cx, &s, &mut closed, update(false, false, true));
        cx.update(|cx| closed.abandon(&s.store, s.session, 1, cx));
        cx.run_until_parked();
        assert_eq!(state(&s).1, ["started", "moved"], "closed like a stop");
        // A new view numbers its turns from 1 again; its turn still counts.
        let mut next = TurnTracker::default();
        observe(cx, &s, &mut next, update(false, false, true));
        observe(cx, &s, &mut next, update(true, true, true));
        let (column, kinds) = state(&s);
        assert_eq!(column, "Review");
        assert_eq!(kinds[0], "completed");
        let started = kinds.iter().filter(|kind| *kind == "started").count();
        assert_eq!(
            started, 2,
            "the second view's turn started afresh: {kinds:?}"
        );
    }

    fn roster_with(session: i64) -> (Roster, [String; 3]) {
        let mut roster = Roster::default();
        let ids = [0, 1, 2].map(|_| roster.create(vec!["/work".into()]).id.clone());
        roster.get_mut(&ids[0]).unwrap().session_id = Some(session);
        roster
            .get_mut(&ids[1])
            .unwrap()
            .habit_sessions
            .push(session);
        roster.get_mut(&ids[2]).unwrap().launches.push(Launch {
            session_id: session,
            cwd: "/work".into(),
            title: "Spawned".into(),
            harness: "codex".into(),
            model: None,
            origin_turn: 0,
            notify_on_complete: false,
            completed: false,
            dismissed: false,
        });
        (roster, ids)
    }

    #[test]
    fn a_sessions_pilot_is_its_own_chat_then_its_habit_then_its_launcher() {
        let (mut roster, [own, habit, launcher]) = roster_with(7);
        assert_eq!(turn_pilot(&roster, 7), Some(own.clone()));
        roster.get_mut(&own).unwrap().session_id = None;
        assert_eq!(turn_pilot(&roster, 7), Some(habit.clone()));
        roster.get_mut(&habit).unwrap().habit_sessions.clear();
        assert_eq!(turn_pilot(&roster, 7), Some(launcher));
        assert_eq!(turn_pilot(&roster, 8), None);
    }
}
