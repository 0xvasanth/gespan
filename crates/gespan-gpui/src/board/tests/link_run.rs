//! Link status chips, blocked cards, "Run with…" and agent fields (C5-10).
use super::*;
use crate::board::{
    link_status::{ChecksState, LinkChip, LinkSource, LinkState},
    run_with::{RunOptions, RunRequest, prepare_run, prepare_run_with},
    turns,
};
use gespan_core::{
    agent::{Settings, TurnInfo},
    board::extras::{FilterRow, Op, SettingsPatch},
    session_groups::WorkItem,
};

fn new_session(f: &Fixture) -> (i64, String) {
    let store = f.store.lock().unwrap();
    let id = store.create("codex", "/work").unwrap().id;
    (id, store.persisted_session_id(id).unwrap())
}

fn add_link(f: &Fixture, card: &str, kind: &str, reference: &str) {
    Service::open(&f.db)
        .unwrap()
        .add_link(card, kind, reference, None)
        .unwrap();
}

fn install_turns(f: &Fixture, cx: &mut TestAppContext) {
    let db = f.db.clone();
    cx.update(|cx| turns::install(db, None, cx));
}

fn started(f: &Fixture, cx: &mut TestAppContext, session: i64, pilot: Option<&str>) {
    let store = f.store.clone();
    let pilot = pilot.map(str::to_string);
    cx.update(|cx| turns::turn_started(&store, session, pilot, 1, cx));
    cx.run_until_parked();
}

fn settled(f: &Fixture, cx: &mut TestAppContext, session: i64, ok: bool) {
    let store = f.store.clone();
    cx.update(|cx| turns::turn_settled(&store, session, 1, false, ok, cx));
    cx.run_until_parked();
}

fn waiting(f: &Fixture, cx: &mut TestAppContext, session: i64, on: bool) {
    let store = f.store.clone();
    cx.update(|cx| turns::set_waiting(&store, session, on, cx));
    cx.run_until_parked();
}

fn chips(f: &Fixture, cx: &mut TestAppContext, card: &str) -> Vec<LinkChip> {
    drawer(f, cx, |view, _| view.link_chips(card).to_vec())
}

fn states(f: &Fixture, cx: &mut TestAppContext, card: &str) -> Vec<LinkState> {
    chips(f, cx, card).iter().map(|chip| chip.state).collect()
}

/// A stand-in for `gh` and Git that records every call.
#[derive(Clone, Default)]
struct FakeSource {
    calls: Arc<Mutex<Vec<String>>>,
    pr: Option<(String, i64, String)>,
    checks: Vec<String>,
    branch: Option<String>,
    diff: Option<(i64, i64)>,
}

impl LinkSource for FakeSource {
    fn pr_status(&self, cwd: &Path) -> Result<Option<(String, i64, String)>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("status {}", cwd.display()));
        Ok(self.pr.clone())
    }
    fn pr_checks(&self, cwd: &Path, repo: &str, number: i64) -> Result<Vec<String>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("checks {} {repo}#{number}", cwd.display()));
        Ok(self.checks.clone())
    }
    fn branch(&self, path: &Path) -> Option<String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("branch {}", path.display()));
        self.branch.clone()
    }
    fn diff(&self, path: &Path) -> Option<(i64, i64)> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("diff {}", path.display()));
        self.diff
    }
}

/// Use `source` and look links up as on opening the board.
fn look_up_with(f: &Fixture, cx: &mut TestAppContext, source: FakeSource) {
    f.window
        .update(cx, |view, _, cx| {
            view.link_source = Arc::new(source);
            view.status_board = None;
            view.reload(cx);
        })
        .unwrap();
    cx.run_until_parked();
}

fn lookups(f: &Fixture, cx: &mut TestAppContext) -> usize {
    drawer(f, cx, |view, _| view.status_lookups)
}

#[gpui::test]
fn card_shows_one_chip_per_link_running_waiting_failed_completed(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Run", "Wait", "Fail", "Done", "Gone"]);
    install_turns(&f, cx);
    let ids: Vec<String> = ["Run", "Wait", "Fail", "Done", "Gone"]
        .iter()
        .map(|title| card_id(&f, cx, title))
        .collect();
    let sessions: Vec<(i64, String)> = (0..5).map(|_| new_session(&f)).collect();
    for (card, (_, key)) in ids.iter().take(4).zip(&sessions) {
        add_link(&f, card, "session", key);
    }
    // "Done" has a second session that never ran; "Gone" a deleted one.
    add_link(&f, &ids[3], "session", &sessions[4].1);
    add_link(&f, &ids[4], "session", "deleted-session");
    started(&f, cx, sessions[0].0, None);
    started(&f, cx, sessions[1].0, None);
    waiting(&f, cx, sessions[1].0, true);
    started(&f, cx, sessions[2].0, None);
    settled(&f, cx, sessions[2].0, false);
    started(&f, cx, sessions[3].0, None);
    settled(&f, cx, sessions[3].0, true);
    reload(&f, cx);
    assert_eq!(states(&f, cx, &ids[0]), [LinkState::Running]);
    assert_eq!(
        states(&f, cx, &ids[1]),
        [LinkState::Waiting],
        "waiting wins"
    );
    assert_eq!(states(&f, cx, &ids[2]), [LinkState::Failed]);
    assert_eq!(
        states(&f, cx, &ids[3]),
        [LinkState::Completed, LinkState::Completed],
        "one chip per link"
    );
    assert_eq!(states(&f, cx, &ids[4]), [LinkState::Missing]);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(shown(visual, "board-card-link-4-0"));
    assert!(shown(visual, "board-card-link-4-1"));
    assert!(!shown(visual, "board-card-link-4-2"));
    // The running chip follows the live turn without a reload.
    settled(&f, visual, sessions[0].0, true);
    assert_eq!(states(&f, visual, &ids[0]), [LinkState::Completed]);
    // Hiding link status on the face hides the chips.
    let mut display = DisplayOptions::default();
    display.face.link_status = false;
    let json = serde_json::to_string(&display).unwrap();
    f.window
        .update(visual, |view, _, cx| {
            view.board_settings.display_json = Some(json);
            cx.notify();
        })
        .unwrap();
    assert!(!shown(visual, "board-card-link-1-0"));
}

#[gpui::test]
fn pilot_chip_shows_running(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Pilot work", "Lost pilot"]);
    install_turns(&f, cx);
    cx.update(|cx| {
        cx.global_mut::<turns::BoardTurns>().test_pilots =
            Some(vec![("p-1".into(), "Scout".into())]);
    });
    let (card, lost) = (card_id(&f, cx, "Pilot work"), card_id(&f, cx, "Lost pilot"));
    add_link(&f, &card, "pilot", "p-1");
    add_link(&f, &lost, "pilot", "p-gone");
    reload(&f, cx);
    assert_eq!(states(&f, cx, &card), [LinkState::Idle]);
    assert_eq!(chips(&f, cx, &card)[0].label, "Scout");
    assert_eq!(states(&f, cx, &lost), [LinkState::Missing]);
    let (session, _) = new_session(&f);
    started(&f, cx, session, Some("p-1"));
    assert_eq!(states(&f, cx, &card), [LinkState::Running]);
    settled(&f, cx, session, true);
    assert_eq!(states(&f, cx, &card), [LinkState::Idle]);
}

#[gpui::test]
fn pr_chip_shows_merged_and_checks_failing(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Local PR", "Remote PR"]);
    let (local, remote) = (card_id(&f, cx, "Local PR"), card_id(&f, cx, "Remote PR"));
    let mut service = Service::open(&f.db).unwrap();
    service.set_projects(&local, &["/work/app".into()]).unwrap();
    service
        .set_projects(&remote, &["remote://env-1/app".into()])
        .unwrap();
    add_link(&f, &local, "pr", "https://github.com/acme/app/pull/42");
    add_link(&f, &remote, "pr", "https://github.com/acme/app/pull/7");
    let source = FakeSource {
        pr: Some((
            "https://github.com/acme/app/pull/42".into(),
            42,
            "MERGED".into(),
        )),
        checks: vec!["pass".into(), "fail".into()],
        ..FakeSource::default()
    };
    let calls = source.calls.clone();
    look_up_with(&f, cx, source);
    let chip = chips(&f, cx, &local).remove(0);
    assert_eq!(chip.label, "PR #42");
    assert_eq!(chip.state, LinkState::Merged);
    assert_eq!(chip.checks, Some(ChecksState::Failing));
    // A remote workspace's PR is unknown, quietly: nothing asked, no error.
    let chip = chips(&f, cx, &remote).remove(0);
    assert_eq!((chip.state, chip.checks), (LinkState::Unknown, None));
    let calls = calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        ["status /work/app", "checks /work/app acme/app#42"],
        "one status read per folder, nothing for the remote card"
    );
    drawer(&f, cx, |view, _| assert!(view.error.is_none()));
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(shown(visual, "board-card-link-1-0"));
}

#[gpui::test]
fn worktree_chip_shows_branch_and_diff_stats(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Branch work"]);
    let card = card_id(&f, cx, "Branch work");
    add_link(&f, &card, "worktree", "/work/app-wt/login");
    // Before the lookup the chip names the folder.
    reload(&f, cx);
    assert_eq!(chips(&f, cx, &card)[0].label, "login");
    look_up_with(
        &f,
        cx,
        FakeSource {
            branch: Some("feature/login".into()),
            diff: Some((12, 3)),
            ..FakeSource::default()
        },
    );
    let chip = chips(&f, cx, &card).remove(0);
    assert_eq!(chip.label, "feature/login");
    assert_eq!(chip.diff, Some((12, 3)));
    assert_eq!(chip.status_text(), "idle, +12 −3");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(shown(visual, "board-card-link-1-0"));
}

#[gpui::test]
fn link_status_looks_up_on_open_and_linked_settle_only_never_on_a_timer(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Linked"]);
    install_turns(&f, cx);
    let card = card_id(&f, cx, "Linked");
    let (linked, key) = new_session(&f);
    let (other, _) = new_session(&f);
    add_link(&f, &card, "session", &key);
    add_link(&f, &card, "worktree", "/work/wt");
    look_up_with(&f, cx, FakeSource::default());
    let opened = lookups(&f, cx);
    // A session no card on this board links settles: nothing is looked up.
    started(&f, cx, other, None);
    settled(&f, cx, other, true);
    assert_eq!(lookups(&f, cx), opened);
    // The linked session settles: one lookup.
    started(&f, cx, linked, None);
    assert_eq!(lookups(&f, cx), opened, "a start alone looks nothing up");
    settled(&f, cx, linked, true);
    assert_eq!(lookups(&f, cx), opened + 1);
    // An hour passes (and the data poll runs): no lookup.
    cx.executor().advance_clock(Duration::from_secs(3600));
    cx.run_until_parked();
    assert_eq!(lookups(&f, cx), opened + 1);
    // Opening the board again looks again.
    f.window
        .update(cx, |view, _, cx| {
            view.hidden(cx);
            view.shown(cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(lookups(&f, cx), opened + 2);
}

fn filter_running(f: &Fixture, cx: &mut TestAppContext) {
    f.window
        .update(cx, |view, _, cx| {
            view.spec.rows.push(FilterRow {
                field: "linked".into(),
                op: Op::Is,
                values: vec!["running".into()],
            });
            view.requery(cx);
        })
        .unwrap();
    cx.run_until_parked();
}

fn matched(f: &Fixture, cx: &mut TestAppContext) -> Vec<String> {
    let detail = detail(f, cx);
    let mut titles: Vec<String> = drawer(f, cx, |view, _| {
        detail
            .cards
            .iter()
            .filter(|card| view.card_matches(card))
            .map(|card| card.title.clone())
            .collect()
    });
    titles.sort();
    titles
}

#[gpui::test]
fn linked_running_filter_matches_cards_with_open_turns(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Busy", "Asking", "Idle", "Pilot"]);
    install_turns(&f, cx);
    let (busy, key) = new_session(&f);
    let (asking, asking_key) = new_session(&f);
    let (idle, idle_key) = new_session(&f);
    add_link(&f, &card_id(&f, cx, "Busy"), "session", &key);
    add_link(&f, &card_id(&f, cx, "Asking"), "session", &asking_key);
    add_link(&f, &card_id(&f, cx, "Idle"), "session", &idle_key);
    add_link(&f, &card_id(&f, cx, "Pilot"), "pilot", "p-1");
    reload(&f, cx);
    filter_running(&f, cx);
    assert!(matched(&f, cx).is_empty(), "nothing runs yet");
    started(&f, cx, busy, None);
    assert_eq!(matched(&f, cx), ["Busy"], "the running set is real now");
    // A session waiting on the user still counts as running (C5-m), even
    // one whose turn this app didn't see start; its chip says waiting.
    waiting(&f, cx, asking, true);
    assert_eq!(matched(&f, cx), ["Asking", "Busy"]);
    let asking_card = card_id(&f, cx, "Asking");
    assert_eq!(states(&f, cx, &asking_card), [LinkState::Waiting]);
    // A turn behind a Pilot runs the Pilot's card too.
    started(&f, cx, idle, Some("p-1"));
    assert_eq!(matched(&f, cx), ["Asking", "Busy", "Idle", "Pilot"]);
    settled(&f, cx, busy, true);
    settled(&f, cx, idle, true);
    waiting(&f, cx, asking, false);
    assert!(matched(&f, cx).is_empty(), "settled turns leave the filter");
}

#[gpui::test]
fn blocked_card_shows_lock_until_blocker_is_done(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Blocker", "Blocked", "Other"]);
    let (blocker, blocked) = (card_id(&f, cx, "Blocker"), card_id(&f, cx, "Blocked"));
    Service::open(&f.db)
        .unwrap()
        .add_relation(&blocker, &blocked, "blocks")
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(shown(visual, "board-card-blocked-2"));
    assert!(!shown(visual, "board-card-blocked-1"), "the blocker isn't");
    assert!(!shown(visual, "board-card-blocked-3"));
    drawer(&f, visual, |view, _| {
        assert_eq!(view.blocked.get(&blocked), Some(&vec![1]))
    });
    // The blocker reaches Done: the lock goes.
    let done = detail(&f, visual).columns[3].id.clone();
    Service::open(&f.db)
        .unwrap()
        .card_move(&blocker, &done, 0, "user")
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    assert!(!shown(visual, "board-card-blocked-2"));
}

fn run_options(f: &Fixture, cx: &mut TestAppContext, project: &str) {
    let project = project.to_string();
    f.window
        .update(cx, |view, _, cx| {
            view.set_run_options(RunOptions {
                providers: vec![
                    ("codex".into(), "Codex".into()),
                    ("claude".into(), "Claude Code".into()),
                ],
                default_provider: Some("claude".into()),
                default_models: [("claude".to_string(), "opus".to_string())].into(),
            });
            view.set_projects(vec![(project, "App".into())], cx);
        })
        .unwrap();
}

#[gpui::test]
fn run_with_on_blocked_card_asks_first_and_cancel_starts_nothing(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Blocker A", "Blocker B", "Target"]);
    run_options(&f, cx, "/work/app");
    let target = card_id(&f, cx, "Target");
    let mut service = Service::open(&f.db).unwrap();
    service
        .add_relation(&card_id(&f, cx, "Blocker A"), &target, "blocks")
        .unwrap();
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (seen, _subscription) = events(&f, visual);
    click(visual, "board-card-3");
    // The second blocker lands after the board loaded: the click re-reads.
    service
        .add_relation(&card_id(&f, visual, "Blocker B"), &target, "blocks")
        .unwrap();
    click(visual, "board-drawer-run-with");
    assert!(shown(visual, "board-run-blocked"));
    assert!(
        !shown(visual, "board-run-picker"),
        "it asks before the picker"
    );
    drawer(&f, visual, |view, _| {
        assert_eq!(
            view.run_confirm.as_ref().unwrap().message(),
            "#3 is blocked by #1, #2. Run anyway?"
        )
    });
    click(visual, "board-run-blocked-cancel");
    assert!(!shown(visual, "board-run-blocked"));
    assert!(!shown(visual, "board-run-picker"));
    assert!(seen.borrow().is_empty(), "Cancel starts nothing");
    // Escape cancels too.
    click(visual, "board-drawer-run-with");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!shown(visual, "board-run-blocked"));
    assert!(shown(visual, "board-drawer"), "only the question closed");
    // Run anyway opens the picker; Start hands the run to the shell.
    click(visual, "board-drawer-run-with");
    click(visual, "board-run-anyway");
    assert!(shown(visual, "board-run-picker"));
    click(visual, "board-run-start");
    assert_eq!(
        *seen.borrow(),
        [format!("run:{target}:claude:opus:/work/app:false")]
    );
}

#[gpui::test]
fn run_with_fails_closed_when_blockers_cant_be_read(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Target"]);
    run_options(&f, cx, "/work/app");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (seen, _subscription) = events(&f, visual);
    click(visual, "board-card-1");
    let blocker = f._dir.0.join("not-a-folder");
    std::fs::write(&blocker, b"").unwrap();
    f.window
        .update(visual, |view, _, _| {
            view.service = None;
            view.db_path = blocker.join("gespan.db");
        })
        .unwrap();
    click(visual, "board-drawer-run-with");
    assert!(!shown(visual, "board-run-blocked"));
    assert!(!shown(visual, "board-run-picker"));
    drawer(&f, visual, |view, _| {
        let error = view.error.clone().unwrap_or_default();
        assert!(
            error.starts_with("Couldn't check what blocks #1"),
            "{error}"
        );
    });
    assert!(seen.borrow().is_empty());
}

fn run_settings() -> Settings {
    Settings {
        model: Some("opus".into()),
        access: gespan_core::agent::AccessMode::from_id("read-only"),
        ..Settings::default()
    }
}

fn request(card: &str, project: &Path) -> RunRequest {
    RunRequest {
        card_id: card.into(),
        provider: "claude".into(),
        model: Some("opus".into()),
        project: project.to_string_lossy().into_owned(),
        new_worktree: false,
    }
}

#[gpui::test]
fn run_with_starts_linked_session_with_card_prompt_and_inherits_ticket(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Ship login"]);
    run_options(&f, cx, "/work/app");
    let card = card_id(&f, cx, "Ship login");
    let mut service = Service::open(&f.db).unwrap();
    service
        .card_update(&card, "Ship login", "Make the form work.")
        .unwrap();
    service.add_subtask(&card, "Wire the button").unwrap();
    let mut cached = ticket("ENG-7", Some(1_000));
    cached.title = Some("Login is broken".into());
    service.set_card_ticket(&card, Some(&cached)).unwrap();
    // The picker starts on the user's provider and model, and the project.
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    let (seen, _subscription) = events(&f, visual);
    click(visual, "board-card-1");
    click(visual, "board-drawer-run-with");
    assert!(shown(visual, "board-run-picker"));
    click(visual, "board-run-start");
    assert_eq!(
        *seen.borrow(),
        [format!("run:{card}:claude:opus:/work/app:false")]
    );
    assert!(!shown(visual, "board-run-picker"));
    // The shell's half: the session it makes.
    let expected = service.run_prompt(&card).unwrap();
    let run = prepare_run(&f.store, &f.db, &request(&card, &f._dir.0), run_settings()).unwrap();
    assert_eq!(run.prompt, expected, "exactly the card's run prompt");
    assert!(!run.prompt.contains("Login is broken"), "no ticket title");
    assert!(run.prompt.contains("Make the form work."));
    assert!(run.prompt.contains("- [ ] Wire the button"));
    assert_eq!(
        run.summary.work_item,
        Some(WorkItem::Ticket(Box::new(cached.clone())))
    );
    assert_eq!(run.summary.provider, "claude");
    // Access is what the caller chose; nothing here turns on plan mode.
    assert_eq!(run.settings.access, run_settings().access);
    assert!(!run.settings.plan);
    let store = f.store.lock().unwrap();
    let key = store.persisted_session_id(run.summary.id).unwrap();
    let saved = store.agent_settings(run.summary.id).unwrap().unwrap();
    assert_eq!(saved.model.as_deref(), Some("opus"));
    let stored = store
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == run.summary.id)
        .unwrap();
    assert_eq!(stored.work_item, Some(WorkItem::Ticket(Box::new(cached))));
    drop(store);
    let links = service.card_links(&card).unwrap();
    assert!(
        links
            .iter()
            .any(|link| link.kind == "session" && link.reference == key),
        "{links:?}"
    );
}

#[gpui::test]
fn run_with_leaves_no_session_when_the_card_link_fails(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Card"]);
    let card = card_id(&f, cx, "Card");
    let before = f.store.lock().unwrap().sessions().unwrap().len();
    let result = prepare_run_with(
        &f.store,
        &f.db,
        &request(&card, &f._dir.0),
        run_settings(),
        |_, _, _| Err("database is locked".into()),
    );
    assert_eq!(result.err().as_deref(), Some("database is locked"));
    assert_eq!(f.store.lock().unwrap().sessions().unwrap().len(), before);
    // A card that is gone fails before any session exists.
    let result = prepare_run(
        &f.store,
        &f.db,
        &request("00000000-0000-0000-0000-000000000000", &f._dir.0),
        run_settings(),
    );
    assert!(result.is_err());
    assert_eq!(f.store.lock().unwrap().sessions().unwrap().len(), before);
}

#[gpui::test]
fn run_with_with_disconnected_tracker_still_inherits_cached_ticket(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Fix login"]);
    run_options(&f, cx, "/work/app");
    // No tracker is connected.
    let fake = with_tickets(&f, cx, &[]);
    let card = card_id(&f, cx, "Fix login");
    let mut cached = ticket("ENG-9", Some(2_000));
    cached.title = Some("Fix login".into());
    cached.status = Some("In Progress".into());
    Service::open(&f.db)
        .unwrap()
        .set_card_ticket(&card, Some(&cached))
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    assert!(shown(visual, "board-drawer-run-with"), "still offered");
    let fetched = fake.fetch_count();
    let run = prepare_run(&f.store, &f.db, &request(&card, &f._dir.0), run_settings()).unwrap();
    assert_eq!(
        run.summary.work_item,
        Some(WorkItem::Ticket(Box::new(cached))),
        "the cached snapshot, as stored"
    );
    // The card title copied from the ticket is ticket content (C5-a).
    assert_eq!(run.prompt.lines().next(), Some("Work on board card #1"));
    assert_eq!(fake.fetch_count(), fetched, "no tracker call");
}

#[gpui::test]
fn run_with_hidden_when_board_turns_it_off(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Card"]);
    let board = open_id(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    assert!(shown(visual, "board-drawer-run-with"), "on by default");
    Service::open(&f.db)
        .unwrap()
        .update_settings(
            &board,
            &SettingsPatch {
                run_with: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    assert!(!shown(visual, "board-drawer-run-with"));
    Service::open(&f.db)
        .unwrap()
        .update_settings(
            &board,
            &SettingsPatch {
                run_with: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    f.window
        .update(visual, |view, _, cx| view.reload(cx))
        .unwrap();
    assert!(shown(visual, "board-drawer-run-with"));
    // Settings that can't be read hide it rather than guess.
    f.window
        .update(visual, |view, _, cx| {
            view.settings_loaded = false;
            cx.notify();
        })
        .unwrap();
    assert!(!shown(visual, "board-drawer-run-with"));
}

#[gpui::test]
fn agent_fields_show_provider_model_cost_and_run_time(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Agent card", "Plain card"]);
    let board = open_id(&f, cx);
    let card = card_id(&f, cx, "Agent card");
    let (session, key) = new_session(&f);
    let mut settings = Settings {
        model: Some("gpt-5".into()),
        ..Settings::default()
    };
    settings.turns.insert(
        0,
        TurnInfo {
            model: "gpt-5".into(),
            duration_ms: 90 * 60_000,
            duration_known: true,
            ..TurnInfo::default()
        },
    );
    f.store
        .lock()
        .unwrap()
        .save_agent_settings(session, &settings)
        .unwrap();
    add_link(&f, &card, "session", &key);
    let mut display = DisplayOptions::default();
    display.face.agent_fields = ["provider", "model", "cost", "runTime"]
        .map(String::from)
        .to_vec();
    Service::open(&f.db)
        .unwrap()
        .update_settings(
            &board,
            &SettingsPatch {
                display_json: Some(Some(serde_json::to_string(&display).unwrap())),
                ..Default::default()
            },
        )
        .unwrap();
    reload(&f, cx);
    drawer(&f, cx, |view, _| {
        let values = view.agent.get(&card).cloned().unwrap();
        assert_eq!(values.provider.as_deref(), Some("Codex"));
        assert_eq!(values.model.as_deref(), Some("gpt-5"));
        assert_eq!(values.run_time.as_deref(), Some("1h 30m"));
        assert_eq!(values.cost, None, "no cost source yet");
    });
    // Nothing is stored for them on the card.
    let stored = Service::open(&f.db).unwrap().field_values(&card).unwrap();
    assert!(stored.is_empty());
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    assert!(shown(visual, "board-card-agent-provider-1"));
    assert!(shown(visual, "board-card-agent-model-1"));
    assert!(shown(visual, "board-card-agent-runTime-1"));
    assert!(
        !shown(visual, "board-card-agent-cost-1"),
        "cost stays hidden"
    );
    assert!(
        !shown(visual, "board-card-agent-provider-2"),
        "no linked session"
    );
    // And as table columns.
    click(visual, "board-layout-table");
    f.window
        .update(visual, |view, _, cx| {
            view.toggle_table_column("provider", cx);
            view.toggle_table_column("model", cx);
            view.toggle_table_column("runTime", cx);
        })
        .unwrap();
    assert!(shown(visual, "board-table-cell-provider-1"));
    assert!(shown(visual, "board-table-cell-model-1"));
    assert!(shown(visual, "board-table-cell-runTime-1"));
    assert!(!shown(visual, "board-table-cell-cost-1"));
}
