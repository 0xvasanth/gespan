//! Automations start on their own: a due schedule or a new Inbox item opens a
//! run in the main window without the Automations view ever being open.
use super::*;
use gespan_core::automations::{
    AutomationRun, AutomationTrigger, AutomationUpsert, DueAutomationRun, Service,
};
use gespan_core::store::Store;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::path::{Path, PathBuf};

const HOUR: i64 = 3_600_000;

const CODEX_BINARY: &str = "GESPAN_CODEX_BINARY";

/// A project folder and the database the app and its automations share.
struct Data {
    _dir: tempfile::TempDir,
    db: PathBuf,
    project: String,
    codex_binary: Option<std::ffi::OsString>,
}

impl Drop for Data {
    fn drop(&mut self) {
        // SAFETY: as in `data`; later tests in this process see the
        // environment they started with.
        unsafe {
            match self.codex_binary.take() {
                Some(value) => std::env::set_var(CODEX_BINARY, value),
                None => std::env::remove_var(CODEX_BINARY),
            }
        }
    }
}

fn data() -> Data {
    // A run starts its agent; a missing CLI makes that fail at once instead
    // of starting a real Codex.
    let codex_binary = std::env::var_os(CODEX_BINARY);
    // SAFETY: set before the window starts any thread that reads the
    // environment, and restored when `Data` drops; CI runs each test in its
    // own process.
    unsafe { std::env::set_var(CODEX_BINARY, "/missing/gespan-test/codex") };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let db = dir.path().join("gespan.db");
    Data {
        project: project.to_string_lossy().into_owned(),
        db,
        _dir: dir,
        codex_binary,
    }
}

/// Opens the main window, which starts its automations.
fn open(data: &Data, cx: &mut TestAppContext) -> WindowHandle<Shell> {
    let store = Arc::new(Mutex::new(Store::open(&data.db).unwrap()));
    cx.update(crate::shell::bind_keys);
    let cwd = data.project.clone();
    // The empty session's attract game repaints at 30Hz while motion is on;
    // advancing the clock a minute would paint it ~1,900 times for nothing
    // these tests check. Reduced motion keeps it on its resting frame.
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(store, vec![], cwd, None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    window
}

fn draft(project: &str, triggers: Vec<AutomationTrigger>) -> AutomationUpsert {
    let mut draft = AutomationUpsert::new(project.into(), "codex".into(), String::new());
    draft.name = "Check the build".into();
    draft.prompt = "Check the build and report".into();
    draft.workspace_mode = "current".into();
    draft.triggers = Some(triggers);
    draft.sync_triggers(gespan_core::time::now_ms() as i64);
    draft
}

fn save(db: &Path, draft: AutomationUpsert) -> String {
    Service::open(db).unwrap().upsert(draft).unwrap().id
}

/// Moves a saved automation's next occurrence to `at`, as if time passed.
fn reschedule(db: &Path, id: &str, at: i64) {
    let conn = rusqlite::Connection::open(db).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT definition_json FROM automations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let mut definition: serde_json::Value = serde_json::from_str(&raw).unwrap();
    definition["nextRunAt"] = at.into();
    conn.execute(
        "UPDATE automations SET definition_json = ?1, next_run_at = ?2 WHERE id = ?3",
        rusqlite::params![definition.to_string(), at, id],
    )
    .unwrap();
}

fn scheduled_due(data: &Data, ago: i64) -> String {
    // Hourly at half an hour from now, so a test that runs across the slot's
    // minute on the real clock is not due a second time.
    let mut hourly = AutomationTrigger::new("time", "hourly");
    hourly.minute = (i64::from(chrono::Timelike::minute(&chrono::Local::now())) + 30) % 60;
    let id = save(&data.db, draft(&data.project, vec![hourly]));
    reschedule(&data.db, &id, now_ms() - ago);
    id
}

fn now_ms() -> i64 {
    gespan_core::time::now_ms() as i64
}

fn runs(db: &Path, id: &str) -> Vec<AutomationRun> {
    Service::open(db).unwrap().runs(id).unwrap()
}

fn poll(window: &WindowHandle<Shell>, cx: &mut TestAppContext) {
    window
        .update(cx, |shell, _, cx| shell.poll_automations(cx))
        .unwrap();
    cx.run_until_parked();
}

/// The sessions the app opened for `automation`.
fn sessions(window: &WindowHandle<Shell>, automation: &str, cx: &mut TestAppContext) -> usize {
    window
        .update(cx, |shell, _, _| {
            shell
                .automation_state
                .session_automations
                .values()
                .filter(|id| *id == automation)
                .count()
        })
        .unwrap()
}

/// The run was taken up by the app: it opened a session and gave it the
/// prompt. The test agent is missing, so the run then reports that.
fn assert_launched(run: &AutomationRun, trigger: &str) {
    assert_eq!(run.trigger, trigger);
    assert!(run.session_id.is_some(), "{run:?}");
    assert_ne!(run.status, "pending", "{run:?}");
    assert_ne!(run.status, "skipped", "{run:?}");
}

#[gpui::test]
fn a_schedule_that_comes_due_while_open_launches_one_run(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let id = scheduled_due(&data, 20_000);
    poll(&window, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "scheduled");
    assert_eq!(sessions(&window, &id, cx), 1);

    // Later polls of the same occurrence start nothing more.
    poll(&window, cx);
    poll(&window, cx);
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert_eq!(sessions(&window, &id, cx), 1);
}

#[gpui::test]
fn a_schedule_that_passed_while_closed_runs_once_when_the_app_opens(cx: &mut TestAppContext) {
    let data = data();
    // Due three hours ago, before the app was started: one run catches up.
    let id = scheduled_due(&data, 3 * HOUR);
    let window = open(&data, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "scheduled");
    assert!(
        now_ms() - saved[0].scheduled_for < HOUR,
        "the latest occurrence"
    );
    assert_eq!(sessions(&window, &id, cx), 1);
    poll(&window, cx);
    assert_eq!(runs(&data.db, &id).len(), 1);
}

#[gpui::test]
fn a_disabled_automation_does_not_run(cx: &mut TestAppContext) {
    let data = data();
    let mut paused = draft(
        &data.project,
        vec![AutomationTrigger::new("time", "hourly")],
    );
    paused.enabled = false;
    let id = save(&data.db, paused);
    reschedule(&data.db, &id, now_ms() - 20_000);
    let window = open(&data, cx);
    poll(&window, cx);
    assert!(runs(&data.db, &id).is_empty());
    assert_eq!(sessions(&window, &id, cx), 0);
}

#[gpui::test]
fn a_schedule_due_while_held_runs_once_on_resume(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    window
        .update(cx, |shell, _, cx| shell.hold_all(cx))
        .unwrap();
    cx.run_until_parked();
    let id = scheduled_due(&data, 20_000);
    poll(&window, cx);
    assert!(runs(&data.db, &id).is_empty(), "nothing starts while held");

    // Resume starts it without waiting for the next clock tick.
    window
        .update(cx, |shell, _, cx| shell.resume_all(cx))
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "scheduled");
}

fn issue(data: &Data, number: i64, created_at: i64) -> gespan_core::inbox::Item {
    gespan_core::inbox::Item {
        provider: "github".into(),
        kind: "issue".into(),
        repo: "acme/app".into(),
        project_path: data.project.clone(),
        number,
        title: format!("Issue {number}"),
        url: format!("https://github.com/acme/app/issues/{number}"),
        created_at: chrono::DateTime::from_timestamp_millis(created_at)
            .unwrap()
            .to_rfc3339(),
        updated_at: chrono::DateTime::from_timestamp_millis(created_at)
            .unwrap()
            .to_rfc3339(),
        ..Default::default()
    }
}

fn on_issue(data: &Data) -> String {
    save(
        &data.db,
        draft(
            &data.project,
            vec![AutomationTrigger::new("github", "issue_opened")],
        ),
    )
}

fn deliver(
    window: &WindowHandle<Shell>,
    items: Vec<gespan_core::inbox::Item>,
    cx: &mut TestAppContext,
) {
    window
        .update(cx, |shell, _, cx| shell.inbox_automation_events(items, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn a_new_inbox_item_launches_one_run_however_often_it_is_fetched(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    let window = open(&data, cx);
    let opened = issue(&data, 12, now_ms());
    deliver(&window, vec![opened.clone()], cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(
        saved[0].event_key.as_deref(),
        Some("github:issue:acme/app:12")
    );

    // Each Inbox refresh offers the item again.
    deliver(&window, vec![opened.clone()], cx);
    deliver(&window, vec![opened], cx);
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert_eq!(sessions(&window, &id, cx), 1);
}

#[gpui::test]
fn an_issue_opened_while_the_app_was_closed_runs_when_it_opens(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    // The automation was set up yesterday; issue 30 already existed then and
    // issue 31 was opened overnight while Gespan was closed.
    let conn = rusqlite::Connection::open(&data.db).unwrap();
    let since = now_ms() - 24 * HOUR;
    conn.execute(
        "UPDATE automations SET definition_json = json_set(definition_json, '$.eventsSince', ?1)
         WHERE id = ?2",
        rusqlite::params![since, id],
    )
    .unwrap();
    let window = open(&data, cx);
    // The first fetch after starting: nothing has "appeared" yet.
    deliver(
        &window,
        vec![
            issue(&data, 30, since - HOUR),
            issue(&data, 31, now_ms() - 2 * HOUR),
        ],
        cx,
    );
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(
        saved[0].event_key.as_deref(),
        Some("github:issue:acme/app:31")
    );
}

#[gpui::test]
fn inbox_items_wait_while_held_and_run_once_on_resume(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    let window = open(&data, cx);
    window
        .update(cx, |shell, _, cx| shell.hold_all(cx))
        .unwrap();
    cx.run_until_parked();
    deliver(&window, vec![issue(&data, 5, now_ms())], cx);
    assert!(runs(&data.db, &id).is_empty(), "nothing starts while held");

    window
        .update(cx, |shell, _, cx| shell.resume_all(cx))
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
}

#[gpui::test]
fn a_disabled_automation_ignores_new_inbox_items(cx: &mut TestAppContext) {
    let data = data();
    let mut paused = draft(
        &data.project,
        vec![AutomationTrigger::new("github", "issue_opened")],
    );
    paused.enabled = false;
    let id = save(&data.db, paused);
    let window = open(&data, cx);
    deliver(&window, vec![issue(&data, 9, now_ms())], cx);
    assert!(runs(&data.db, &id).is_empty());
    assert_eq!(sessions(&window, &id, cx), 0);
}

#[gpui::test]
fn the_window_clock_runs_a_due_schedule_without_anything_else_happening(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let id = scheduled_due(&data, 20_000);
    // Only the 30-second clock the window starts; nobody calls a poll.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "scheduled");
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.run_until_parked();
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert_eq!(sessions(&window, &id, cx), 1);
}

#[gpui::test]
fn a_further_window_leaves_automations_to_the_first(cx: &mut TestAppContext) {
    let data = data();
    let first = open(&data, cx);
    cx.update(crate::windows::open_new_window);
    cx.run_until_parked();
    let second = cx
        .update(|cx| cx.windows())
        .into_iter()
        .filter_map(|window| window.downcast::<Shell>())
        .find(|window| *window != first)
        .expect("a second window");
    assert!(second.update(cx, |shell, _, _| shell.secondary).unwrap());
    let id = scheduled_due(&data, 20_000);
    poll(&second, cx);
    assert!(
        runs(&data.db, &id).is_empty(),
        "only the first window claims"
    );
    poll(&first, cx);
    poll(&second, cx);
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert_eq!(sessions(&first, &id, cx), 1);
}

#[gpui::test]
fn a_run_claimed_before_gespan_quit_starts_once_when_it_reopens(cx: &mut TestAppContext) {
    let data = data();
    let id = scheduled_due(&data, 20_000);
    // The schedule was claimed, then Gespan quit before handing it a session.
    let claimed = Service::open(&data.db)
        .unwrap()
        .claim_due(now_ms())
        .unwrap();
    assert_eq!(claimed.len(), 1);
    let window = open(&data, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "recovered, not claimed again: {saved:?}");
    assert_eq!(saved[0].id, claimed[0].run.id);
    assert_launched(&saved[0], "scheduled");
    poll(&window, cx);
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert_eq!(sessions(&window, &id, cx), 1);
}

#[gpui::test]
fn a_run_interrupted_by_quitting_is_cancelled_not_restarted(cx: &mut TestAppContext) {
    let data = data();
    let id = scheduled_due(&data, 20_000);
    let mut service = Service::open(&data.db).unwrap();
    let run = service.claim_due(now_ms()).unwrap().remove(0).run;
    service
        .update_run(&run.id, "running", Some("1".into()), None, now_ms())
        .unwrap();
    drop(service);
    let window = open(&data, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].status, "cancelled");
    assert_eq!(sessions(&window, &id, cx), 0);
}

#[gpui::test]
fn several_inbox_deliveries_in_a_row_run_every_new_item(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    let window = open(&data, cx);
    // A second refresh lands while the first claim is still in flight.
    let (one, two) = (issue(&data, 1, now_ms()), issue(&data, 2, now_ms()));
    window
        .update(cx, |shell, _, cx| {
            shell.inbox_automation_events(vec![one.clone()], cx);
            shell.inbox_automation_events(vec![one, two], cx);
        })
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 2, "{saved:?}");
    for run in &saved {
        assert_launched(run, "event");
    }
}

#[gpui::test]
fn the_inbox_first_fetch_after_opening_runs_an_issue_opened_while_closed(cx: &mut TestAppContext) {
    let data = data();
    // Opened before the automation exists, so no background Inbox has
    // fetched yet.
    let window = open(&data, cx);
    assert!(
        window
            .update(cx, |shell, _, _| shell.inbox.is_none())
            .unwrap()
    );
    let id = on_issue(&data);
    let since = now_ms() - 24 * HOUR;
    rusqlite::Connection::open(&data.db)
        .unwrap()
        .execute(
            "UPDATE automations SET definition_json = json_set(definition_json, '$.eventsSince', ?1)
             WHERE id = ?2",
            rusqlite::params![since, id],
        )
        .unwrap();
    // The real Inbox's first fetch only sets its "appeared" baseline; the
    // issue opened overnight and the backlog from before are both in it.
    let fetched = gespan_core::inbox::Snapshot {
        items: vec![
            issue(&data, 40, since - HOUR),
            issue(&data, 41, now_ms() - 3 * HOUR),
        ],
        ..Default::default()
    };
    window
        .update(cx, |shell, window, cx| {
            shell.ensure_background_inbox(window, cx);
            let inbox = shell.inbox.clone().unwrap();
            inbox.update(cx, |inbox, _| inbox.fetched = fetched);
        })
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(
        saved[0].event_key.as_deref(),
        Some("github:issue:acme/app:41")
    );
    // Later refreshes list the same items and start nothing more.
    window
        .update(cx, |shell, _, cx| {
            let inbox = shell.inbox.clone().unwrap();
            inbox.update(cx, |inbox, cx| inbox.refresh(cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert_eq!(sessions(&window, &id, cx), 1);
}

#[gpui::test]
fn a_new_item_in_a_later_inbox_fetch_runs_while_open(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    let window = open(&data, cx);
    window
        .update(cx, |shell, window, cx| {
            shell.ensure_background_inbox(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(runs(&data.db, &id).is_empty());
    let opened = gespan_core::inbox::Snapshot {
        items: vec![issue(&data, 50, now_ms())],
        ..Default::default()
    };
    window
        .update(cx, |shell, _, cx| {
            let inbox = shell.inbox.clone().unwrap();
            inbox.update(cx, |inbox, cx| {
                inbox.fetched = opened;
                inbox.refresh(cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
}

/// A Linear issue: trackers give no creation time, so the Inbox offers it
/// only in the refresh where it first appears.
fn tracker_issue(id: &str) -> gespan_core::inbox::Item {
    gespan_core::inbox::Item {
        provider: "linear".into(),
        kind: "issue".into(),
        id: id.into(),
        identifier: format!("ENG-{id}"),
        title: "Crash on start".into(),
        ..Default::default()
    }
}

fn on_tracker_issue(data: &Data) -> String {
    save(
        &data.db,
        draft(
            &data.project,
            vec![AutomationTrigger::new("linear", "issue_created")],
        ),
    )
}

#[gpui::test]
fn an_undated_item_that_appeared_while_held_still_runs_after_later_refreshes(
    cx: &mut TestAppContext,
) {
    let data = data();
    let id = on_tracker_issue(&data);
    let window = open(&data, cx);
    window
        .update(cx, |shell, _, cx| shell.hold_all(cx))
        .unwrap();
    cx.run_until_parked();
    deliver(&window, vec![tracker_issue("7")], cx);
    // Later refreshes no longer offer it: it is not new to the Inbox now.
    deliver(&window, vec![], cx);
    deliver(&window, vec![issue(&data, 3, now_ms() - 30 * HOUR)], cx);
    assert!(runs(&data.db, &id).is_empty());
    window
        .update(cx, |shell, _, cx| shell.resume_all(cx))
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(saved[0].event_key.as_deref(), Some("linear:issue:7"));
}

#[gpui::test]
fn an_undated_item_whose_claim_failed_is_retried_after_a_restart(cx: &mut TestAppContext) {
    let data = data();
    let id = on_tracker_issue(&data);
    let window = open(&data, cx);
    // Evaluating automations fails while their table is unreadable.
    let conn = rusqlite::Connection::open(&data.db).unwrap();
    conn.execute_batch("ALTER TABLE automations RENAME TO automations_away")
        .unwrap();
    deliver(&window, vec![tracker_issue("8")], cx);
    conn.execute_batch("ALTER TABLE automations_away RENAME TO automations")
        .unwrap();
    assert!(runs(&data.db, &id).is_empty());
    // Store settings live in the Settings database next to the store's.
    let name = data.db.file_name().unwrap().to_string_lossy();
    let settings = data.db.with_file_name(format!("{name}.gpui.sqlite3"));
    let saved: String = rusqlite::Connection::open(settings)
        .unwrap()
        .query_row(
            "SELECT value FROM settings WHERE key = 'native-automation-inbox-retries'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(saved.contains("ENG-8"), "{saved}");
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    // Gespan restarts before the Inbox would offer the item again.
    let reopened = open(&data, cx);
    poll(&reopened, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(saved[0].event_key.as_deref(), Some("linear:issue:8"));
    assert_eq!(sessions(&reopened, &id, cx), 1);
}

#[gpui::test]
fn a_run_whose_project_folder_is_gone_fails_with_a_reason_and_the_schedule_moves_on(
    cx: &mut TestAppContext,
) {
    let data = data();
    let mut missing = draft(
        &data.project,
        vec![AutomationTrigger::new("time", "hourly")],
    );
    missing.cwd = format!("{}/deleted", data.project);
    let id = save(&data.db, missing);
    reschedule(&data.db, &id, now_ms() - 20_000);
    let window = open(&data, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].status, "failed", "never left pending: {saved:?}");
    assert!(
        saved[0]
            .error
            .as_deref()
            .unwrap()
            .contains("project folder is unavailable"),
        "{saved:?}"
    );
    assert_eq!(sessions(&window, &id, cx), 0);
    let next = Service::open(&data.db)
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|automation| automation.id == id)
        .unwrap()
        .next_run_at;
    assert!(next > now_ms(), "the next occurrence is still scheduled");
}

#[gpui::test]
fn several_automations_due_at_once_each_run_once_in_their_own_session(cx: &mut TestAppContext) {
    let data = data();
    let ids: Vec<_> = (0..3).map(|_| scheduled_due(&data, 20_000)).collect();
    let issue_id = on_issue(&data);
    let window = open(&data, cx);
    deliver(&window, vec![issue(&data, 70, now_ms())], cx);
    for id in ids.iter().chain(std::iter::once(&issue_id)) {
        let saved = runs(&data.db, id);
        assert_eq!(saved.len(), 1, "{id}: {saved:?}");
        assert_eq!(sessions(&window, id, cx), 1, "{id}");
    }
    let distinct: std::collections::HashSet<_> = ids
        .iter()
        .chain(std::iter::once(&issue_id))
        .map(|id| runs(&data.db, id)[0].session_id.clone().unwrap())
        .collect();
    assert_eq!(distinct.len(), 4, "every run has its own session");
}

#[gpui::test]
fn inbox_items_delivered_before_automations_finish_opening_still_run(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    let store = Arc::new(Mutex::new(Store::open(&data.db).unwrap()));
    cx.update(crate::shell::bind_keys);
    let cwd = data.project.clone();
    let item = issue(&data, 80, now_ms());
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(store, vec![], cwd, None, cx);
                // The automation service opens in the background; an Inbox
                // refresh can land first.
                assert!(shell.automation_state.service.is_none());
                shell.inbox_automation_events(vec![item], cx);
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(sessions(&window, &id, cx), 1);
}

#[gpui::test]
fn an_automation_with_a_schedule_and_an_event_runs_for_each(cx: &mut TestAppContext) {
    let data = data();
    let id = save(
        &data.db,
        draft(
            &data.project,
            vec![
                AutomationTrigger::new("time", "hourly"),
                AutomationTrigger::new("github", "issue_opened"),
            ],
        ),
    );
    reschedule(&data.db, &id, now_ms() - 20_000);
    let window = open(&data, cx);
    deliver(&window, vec![issue(&data, 90, now_ms())], cx);
    let saved = runs(&data.db, &id);
    let mut triggers: Vec<_> = saved.iter().map(|run| run.trigger.as_str()).collect();
    triggers.sort_unstable();
    assert_eq!(triggers, ["event", "scheduled"], "{saved:?}");
    for run in &saved {
        assert_launched(run, &run.trigger);
    }
    assert_eq!(sessions(&window, &id, cx), 2);
}

#[gpui::test]
fn run_now_starts_a_run_and_shows_its_session(cx: &mut TestAppContext) {
    let data = data();
    let id = save(&data.db, draft(&data.project, vec![]));
    let window = open(&data, cx);
    let automation = Service::open(&data.db)
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|automation| automation.id == id)
        .unwrap();
    let run = Service::open(&data.db)
        .unwrap()
        .run_now(&id, now_ms())
        .unwrap();
    // What the Automations view's Run now emits.
    window
        .update(cx, |shell, _, cx| {
            shell.launch_automation(DueAutomationRun { automation, run }, true, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "manual");
    let active = window
        .update(cx, |shell, _, _| shell.workspace.active)
        .unwrap();
    let session = window
        .update(cx, |shell, _, _| {
            shell
                .automation_state
                .session_automations
                .iter()
                .find(|(_, automation)| *automation == &id)
                .map(|(session, _)| *session)
        })
        .unwrap();
    assert_eq!(active, session, "Run now opens the run's session");
}

#[gpui::test]
fn a_launched_run_shows_in_the_automation_runs_folder(cx: &mut TestAppContext) {
    let data = data();
    let id = save(&data.db, draft(&data.project, vec![]));
    let window = open(&data, cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    // The Automation runs folder is one of "My folders".
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.preferences.sidebar_group_by = "folders".into();
            shell.notify_sidebar(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("rail-folder-fixed:automations")
            .is_none()
    );
    let automation = Service::open(&data.db)
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|automation| automation.id == id)
        .unwrap();
    let run = Service::open(&data.db)
        .unwrap()
        .run_now(&id, now_ms())
        .unwrap();
    // Launched the way a due run is, without the background reveal.
    window
        .update(visual, |shell, _, cx| {
            shell.launch_automation(DueAutomationRun { automation, run }, false, cx)
        })
        .unwrap();
    visual.run_until_parked();
    let session = window
        .update(visual, |shell, _, _| {
            shell
                .automation_state
                .session_automations
                .iter()
                .find(|(_, automation)| *automation == &id)
                .map(|(session, _)| *session)
        })
        .unwrap()
        .expect("the run has a session");
    let fixed = window
        .update(visual, |shell, _, cx| {
            let ids = shell.sidebar_session_ids(cx);
            shell
                .session_entries(&ids, cx)
                .into_iter()
                .find_map(|entry| match entry {
                    gespan_core::session_groups::Entry::Fixed { id, sessions, .. }
                        if id == "fixed:automations" =>
                    {
                        Some(sessions)
                    }
                    _ => None,
                })
        })
        .unwrap();
    assert_eq!(fixed, Some(vec![session]));
    assert!(
        visual
            .debug_bounds("rail-folder-fixed:automations")
            .is_some(),
        "the folder header is drawn"
    );
}

#[gpui::test]
fn a_run_reuses_the_last_conversation_when_asked(cx: &mut TestAppContext) {
    let data = data();
    let mut reusing = draft(
        &data.project,
        vec![AutomationTrigger::new("time", "hourly")],
    );
    reusing.reuse_session = true;
    let id = save(&data.db, reusing);
    reschedule(&data.db, &id, now_ms() - 20_000);
    let window = open(&data, cx);
    assert_eq!(runs(&data.db, &id).len(), 1);
    // The next hour comes due.
    reschedule(&data.db, &id, now_ms() - 10_000);
    poll(&window, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 2, "{saved:?}");
    assert_eq!(saved[0].session_id, saved[1].session_id, "{saved:?}");
    assert_eq!(sessions(&window, &id, cx), 1);
}

/// The automation-run alerts that reached the OS: the session each opens,
/// its subtitle and its body.
fn run_alerts(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<(i64, String, String)> {
    window
        .update(cx, |shell, _, _| {
            shell
                .notification_state
                .session_notices
                .iter()
                .filter(|notice| notice.category == "automationRuns")
                .map(|notice| match notice.target {
                    crate::notifications::delivery::Target::Session(id) => {
                        (id, notice.subtitle.clone(), notice.body.clone())
                    }
                    other => panic!("an automation alert opens its session, not {other:?}"),
                })
                .collect()
        })
        .unwrap()
}

fn allow_notices(window: &WindowHandle<Shell>, cx: &mut TestAppContext) {
    window
        .update(cx, |shell, _, _| shell.allow_notices_for_test(true))
        .unwrap();
}

#[gpui::test]
fn a_run_started_by_a_new_issue_raises_one_alert_that_opens_its_session(cx: &mut TestAppContext) {
    let data = data();
    let id = on_issue(&data);
    let window = open(&data, cx);
    allow_notices(&window, cx);
    let opened = issue(&data, 12, now_ms());
    deliver(&window, vec![opened.clone()], cx);
    let session = window
        .update(cx, |shell, _, _| {
            shell
                .automation_state
                .session_automations
                .iter()
                .find(|(_, automation)| *automation == &id)
                .map(|(session, _)| *session)
        })
        .unwrap()
        .expect("the run opened a session");
    assert_eq!(
        run_alerts(&window, cx),
        [(
            session,
            "Check the build".to_string(),
            "Issue opened: acme/app#12 Issue 12".to_string()
        )]
    );

    // Each Inbox refresh offers the item again; it neither runs nor alerts again.
    deliver(&window, vec![opened.clone()], cx);
    deliver(&window, vec![opened], cx);
    assert_eq!(run_alerts(&window, cx).len(), 1);

    // A second issue is a second run and a second alert.
    deliver(&window, vec![issue(&data, 13, now_ms())], cx);
    let bodies: Vec<_> = run_alerts(&window, cx).into_iter().map(|a| a.2).collect();
    assert_eq!(
        bodies,
        [
            "Issue opened: acme/app#12 Issue 12",
            "Issue opened: acme/app#13 Issue 13"
        ]
    );
}

#[gpui::test]
fn each_kind_of_github_event_names_itself_in_the_alert(cx: &mut TestAppContext) {
    let data = data();
    save(
        &data.db,
        draft(
            &data.project,
            vec![
                AutomationTrigger::new("github", "pull_request_opened"),
                AutomationTrigger::new("github", "draft_opened"),
            ],
        ),
    );
    let window = open(&data, cx);
    allow_notices(&window, cx);
    let pull = |number: i64, draft: bool| gespan_core::inbox::Item {
        kind: "pr".into(),
        draft,
        title: format!("Change {number}"),
        url: format!("https://github.com/acme/app/pull/{number}"),
        ..issue(&data, number, now_ms())
    };
    deliver(&window, vec![pull(20, false)], cx);
    deliver(&window, vec![pull(21, true)], cx);
    let bodies: Vec<_> = run_alerts(&window, cx).into_iter().map(|a| a.2).collect();
    assert_eq!(
        bodies,
        [
            "Pull request opened: acme/app#20 Change 20",
            "Draft opened: acme/app#21 Change 21"
        ]
    );
}

#[gpui::test]
fn scheduled_and_hand_started_runs_raise_no_start_alert(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    allow_notices(&window, cx);
    // A schedule repeats on its own; an alert for every hour would be noise.
    let scheduled = scheduled_due(&data, 20_000);
    poll(&window, cx);
    assert_eq!(runs(&data.db, &scheduled).len(), 1);

    // Run now opens the session in front of the user, who needs no alert.
    let id = save(&data.db, draft(&data.project, vec![]));
    let automation = Service::open(&data.db)
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|automation| automation.id == id)
        .unwrap();
    let run = Service::open(&data.db)
        .unwrap()
        .run_now(&id, now_ms())
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            shell.launch_automation(DueAutomationRun { automation, run }, true, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(runs(&data.db, &id).len(), 1);
    assert!(run_alerts(&window, cx).is_empty());
}

#[gpui::test]
fn muting_automation_runs_for_the_project_or_notifications_silences_the_alert(
    cx: &mut TestAppContext,
) {
    let data = data();
    let id = on_issue(&data);
    let window = open(&data, cx);
    allow_notices(&window, cx);
    let project = data.project.clone();
    window
        .update(cx, |shell, _, _| {
            let prefs = &mut shell.workspace.preferences;
            let key = prefs.known_notification_project_id(&project).unwrap();
            prefs.project_notifications =
                serde_json::json!({ key: { "disabled": ["automationRuns"] } });
        })
        .unwrap();
    deliver(&window, vec![issue(&data, 40, now_ms())], cx);
    assert_eq!(runs(&data.db, &id).len(), 1, "the run still starts");
    assert!(run_alerts(&window, cx).is_empty());

    // Other categories muted do not silence it.
    window
        .update(cx, |shell, _, _| {
            let prefs = &mut shell.workspace.preferences;
            let key = prefs.known_notification_project_id(&project).unwrap();
            prefs.project_notifications =
                serde_json::json!({ key: { "disabled": ["agentFinished"] } });
        })
        .unwrap();
    deliver(&window, vec![issue(&data, 41, now_ms())], cx);
    assert_eq!(run_alerts(&window, cx).len(), 1);

    // Turning notifications off silences it too.
    window
        .update(cx, |shell, _, _| shell.allow_notices_for_test(false))
        .unwrap();
    deliver(&window, vec![issue(&data, 42, now_ms())], cx);
    assert_eq!(runs(&data.db, &id).len(), 3);
    assert_eq!(run_alerts(&window, cx).len(), 1);
}

#[gpui::test]
fn runs_asked_for_before_the_service_opens_show_once_it_does(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    window
        .update(cx, |shell, window, cx| {
            // As at startup: the service has not opened yet.
            shell.automation_state.service = None;
            shell.open_runs(
                crate::runs::RunsFilter {
                    status: Some("failed"),
                    ..Default::default()
                },
                window,
                cx,
            );
            assert_eq!(shell.active_app_view, None, "nothing to list yet");
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.active_app_view, Some(app_views::AppView::Runs));
            let runs = shell.runs.as_ref().expect("Runs opened").read(cx);
            assert_eq!(runs.filter().status, Some("failed"));
        })
        .unwrap();
}

#[gpui::test]
fn open_thread_in_runs_opens_the_runs_session(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let id = scheduled_due(&data, 20_000);
    poll(&window, cx);
    assert_eq!(sessions(&window, &id, cx), 1);
    let run = runs(&data.db, &id).remove(0);
    let session = run.session_id.clone().expect("the run started a session");
    window
        .update(cx, |shell, window, cx| {
            shell.open_runs(Default::default(), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let native = window
        .update(cx, |shell, _, cx| {
            let view = shell.runs.clone().expect("Runs opened");
            let rows = view.read(cx).rows();
            assert_eq!(rows.len(), 1);
            assert!(rows[0].can_open());
            view.update(cx, |view, cx| view.open_thread(&run.id, cx));
            locked(&shell.store)
                .unwrap()
                .resolve_existing_session_id(&session)
                .expect("the session exists")
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(cx, |shell, _| {
            assert_eq!(shell.active_app_view, None, "Runs gives way to the thread");
            assert_eq!(shell.workspace.active, Some(native));
        })
        .unwrap();
}

#[gpui::test]
fn see_all_in_runs_opens_runs_filtered_to_that_automation(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let first = scheduled_due(&data, 20_000);
    let second = scheduled_due(&data, 20_000);
    poll(&window, cx);
    assert_eq!(runs(&data.db, &first).len(), 1);
    assert_eq!(runs(&data.db, &second).len(), 1);
    window
        .update(cx, |shell, window, cx| shell.open_automations(window, cx))
        .unwrap();
    cx.run_until_parked();
    // "See all in Runs" on the first automation.
    window
        .update(cx, |shell, _, cx| {
            let view = shell.automations.clone().expect("Automations opened");
            view.update(cx, |_, cx| {
                cx.emit(crate::automations::OpenRuns {
                    automation: first.clone(),
                })
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.active_app_view, Some(app_views::AppView::Runs));
            let view = shell.runs.clone().expect("Runs opened");
            assert_eq!(view.read(cx).filter().automation.as_ref(), Some(&first));
            let rows = view.read(cx).rows();
            assert_eq!(rows.len(), 1, "only the chosen automation's runs");
            let expected = runs(&data.db, &first).remove(0).id;
            assert_eq!(rows[0].id, expected);
            // The way back to every automation.
            view.update(cx, |view, cx| {
                let cleared = crate::runs::RunsFilter {
                    automation: None,
                    ..view.filter().clone()
                };
                view.set_filter(cleared, cx);
            });
        })
        .unwrap();
    // Every automation's runs load again.
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let view = shell.runs.clone().expect("Runs opened");
            assert_eq!(view.read(cx).rows().len(), 2);
        })
        .unwrap();
}

#[gpui::test]
fn the_sidebar_runs_entry_drops_the_automation_filter(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let first = scheduled_due(&data, 20_000);
    scheduled_due(&data, 20_000);
    poll(&window, cx);
    window
        .update(cx, |shell, window, cx| {
            shell.open_runs(
                crate::runs::RunsFilter {
                    automation: Some(first.clone()),
                    status: Some("failed"),
                    ..Default::default()
                },
                window,
                cx,
            )
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, window, cx| shell.show_runs(window, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let view = shell.runs.clone().expect("Runs opened");
            let filter = view.read(cx).filter().clone();
            assert_eq!(filter.automation, None, "back to every automation");
            assert_eq!(filter.status, Some("failed"), "the rest is kept");
        })
        .unwrap();
}

// --- "Wake a Pilot": the run is queued on a Pilot's chat instead of starting
// a session.

use gespan_core::automations::{AutomationAction, AutomationEventClaim};

/// Creates a Pilot as the Pilots page does, so it has a chat, and names it.
fn add_pilot(
    window: &WindowHandle<Shell>,
    data: &Data,
    name: &str,
    cx: &mut TestAppContext,
) -> String {
    let project = data.project.clone();
    let id = window
        .update(cx, |shell, window, cx| {
            shell.pilot_manager().update(cx, |pilots, cx| {
                pilots.create(vec![project], window, cx);
                let id = pilots.current().unwrap().id.clone();
                pilots.roster.get_mut(&id).unwrap().name = name.into();
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    id
}

fn waking(data: &Data, pilot: &str, triggers: Vec<AutomationTrigger>) -> String {
    let mut draft = draft(&data.project, triggers);
    draft.action = AutomationAction::Pilot {
        pilot_id: pilot.into(),
    };
    save(&data.db, draft)
}

fn saved_automation(data: &Data, id: &str) -> gespan_core::automations::Automation {
    Service::open(&data.db)
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|automation| automation.id == id)
        .unwrap()
}

/// Hands `due` to the app and returns the Pilot's queue as it stands before
/// anything else runs: the queue is filled at once, not after a wait.
fn launch_for_pilot(
    window: &WindowHandle<Shell>,
    due: DueAutomationRun,
    pilot: &str,
    cx: &mut TestAppContext,
) -> Vec<gespan_core::pilot::Message> {
    let pilot = pilot.to_string();
    let queued = window
        .update(cx, |shell, _, cx| {
            shell.launch_automation(due, false, cx);
            shell
                .pilot_manager()
                .read(cx)
                .roster
                .get(&pilot)
                .map(|pilot| pilot.outbox.clone())
                .unwrap_or_default()
        })
        .unwrap();
    cx.run_until_parked();
    queued
}

fn all_sessions(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> usize {
    window
        .update(cx, |shell, _, _| shell.session_summaries.len())
        .unwrap()
}

#[gpui::test]
fn an_event_run_for_a_pilot_queues_one_internal_message_and_starts_no_session(
    cx: &mut TestAppContext,
) {
    let data = data();
    let window = open(&data, cx);
    let pilot = add_pilot(&window, &data, "ReviewPilot", cx);
    let id = waking(
        &data,
        &pilot,
        vec![AutomationTrigger::new("github", "issue_opened")],
    );
    let prompt = "Check the build and report\n\n<external>Issue 12: login is broken</external>";
    let due = Service::open(&data.db)
        .unwrap()
        .claim_event(
            &id,
            AutomationEventClaim {
                event_key: "github:issue:acme/app:12".into(),
                event_kind: "github".into(),
                event: "issue_opened".into(),
                scheduled_for: now_ms(),
                prompt: prompt.into(),
                title: "acme/app#12 Issue 12".into(),
            },
            now_ms(),
        )
        .unwrap()
        .expect("the event is claimed");
    let run = due.run.id.clone();
    let before = all_sessions(&window, cx);
    let queued = launch_for_pilot(&window, due.clone(), &pilot, cx);

    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(
        queued[0].internal,
        "the Pilot's own queue, not a typed message"
    );
    assert!(queued[0].attachments.is_empty());
    assert_eq!(
        queued[0].text,
        format!("[Automation: Check the build]\nTrigger: Issue opened  ·  Run: {run}\n{prompt}"),
        "the header, then the run prompt exactly as a session would get it"
    );

    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].status, "succeeded", "delivered is done: {saved:?}");
    assert_eq!(
        saved[0].pilot_message_id.as_deref(),
        Some(queued[0].id.as_str())
    );
    assert_eq!(saved[0].session_id, None);
    assert_eq!(saved[0].error, None);
    assert!(saved[0].completed_at.is_some());
    assert_eq!(sessions(&window, &id, cx), 0);
    assert_eq!(all_sessions(&window, cx), before, "no session was opened");
    assert_eq!(saved_automation(&data, &id).last_session_id, None);

    // Handing over the same run again (a restart between queueing and saving
    // the result) does not queue it twice while it still waits.
    let again = launch_for_pilot(&window, due, &pilot, cx);
    assert_eq!(
        again.iter().filter(|m| m.text.contains(&run)).count(),
        1,
        "{again:?}"
    );
}

#[gpui::test]
fn scheduled_and_hand_started_pilot_runs_say_what_started_them(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let pilot = add_pilot(&window, &data, "ReviewPilot", cx);
    let id = waking(&data, &pilot, vec![]);
    let automation = saved_automation(&data, &id);
    let manual = Service::open(&data.db)
        .unwrap()
        .run_now(&id, now_ms())
        .unwrap();
    let queued = launch_for_pilot(
        &window,
        DueAutomationRun {
            automation: automation.clone(),
            run: manual.clone(),
        },
        &pilot,
        cx,
    );
    let text = &queued.last().unwrap().text;
    assert_eq!(
        text,
        &format!(
            "[Automation: Check the build]\nTrigger: Manual  ·  Run: {}\nCheck the build and report",
            manual.id
        )
    );
    let mut scheduled = Service::open(&data.db)
        .unwrap()
        .run_now(&id, now_ms())
        .unwrap();
    scheduled.trigger = "scheduled".into();
    let queued = launch_for_pilot(
        &window,
        DueAutomationRun {
            automation,
            run: scheduled.clone(),
        },
        &pilot,
        cx,
    );
    assert!(
        queued.iter().any(|m| m.text.starts_with(&format!(
            "[Automation: Check the build]\nTrigger: Scheduled  ·  Run: {}\n",
            scheduled.id
        ))),
        "{queued:?}"
    );
}

#[gpui::test]
fn a_due_schedule_for_a_pilot_is_delivered_once_end_to_end(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let pilot = add_pilot(&window, &data, "ReviewPilot", cx);
    let mut hourly = AutomationTrigger::new("time", "hourly");
    hourly.minute = (i64::from(chrono::Timelike::minute(&chrono::Local::now())) + 30) % 60;
    let id = waking(&data, &pilot, vec![hourly]);
    reschedule(&data.db, &id, now_ms() - 20_000);
    let before = all_sessions(&window, cx);
    poll(&window, cx);
    poll(&window, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].trigger, "scheduled");
    assert_eq!(saved[0].status, "succeeded");
    assert!(saved[0].pilot_message_id.is_some());
    assert_eq!(saved[0].session_id, None);
    assert_eq!(all_sessions(&window, cx), before);
}

#[gpui::test]
fn a_run_for_a_pilot_that_is_gone_fails_with_a_reason_and_starts_nothing(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    let kept = add_pilot(&window, &data, "Other", cx);
    let id = waking(&data, "0123456789abcdef", vec![]);
    let automation = saved_automation(&data, &id);
    let run = Service::open(&data.db)
        .unwrap()
        .run_now(&id, now_ms())
        .unwrap();
    let before = all_sessions(&window, cx);
    let queued = launch_for_pilot(&window, DueAutomationRun { automation, run }, &kept, cx);
    assert!(queued.is_empty(), "another Pilot is not woken instead");
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].status, "failed");
    assert_eq!(
        saved[0].error.as_deref(),
        Some("Pilot 0123456789abcdef is no longer available")
    );
    assert_eq!(saved[0].pilot_message_id, None);
    assert_eq!(saved[0].session_id, None);
    assert_eq!(sessions(&window, &id, cx), 0);
    assert_eq!(all_sessions(&window, cx), before);
}

#[gpui::test]
fn a_pilot_run_claimed_before_gespan_quit_is_queued_once_when_it_reopens(cx: &mut TestAppContext) {
    let data = data();
    // Creating the Pilot saves it in the app's database; then Gespan quits.
    let pilot = {
        let window = open(&data, cx);
        let pilot = add_pilot(&window, &data, "ReviewPilot", cx);
        window
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        pilot
    };
    let mut hourly = AutomationTrigger::new("time", "hourly");
    hourly.minute = (i64::from(chrono::Timelike::minute(&chrono::Local::now())) + 30) % 60;
    let id = waking(&data, &pilot, vec![hourly]);
    reschedule(&data.db, &id, now_ms() - 20_000);
    let mut service = Service::open(&data.db).unwrap();
    // One run was already delivered; another was claimed but never handed over.
    let delivered = service.run_now(&id, now_ms()).unwrap();
    service
        .deliver_run(&delivered.id, "0123456789abcdef", now_ms())
        .unwrap();
    let claimed = service.claim_due(now_ms()).unwrap();
    assert_eq!(claimed.len(), 1);
    drop(service);

    let window = open(&data, cx);
    poll(&window, cx);
    poll(&window, cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 2, "recovered, not claimed again: {saved:?}");
    let pending = saved
        .iter()
        .find(|run| run.id == claimed[0].run.id)
        .unwrap();
    assert_eq!(pending.status, "succeeded");
    let message = pending
        .pilot_message_id
        .clone()
        .expect("queued on recovery");
    let resent = saved.iter().find(|run| run.id == delivered.id).unwrap();
    assert_eq!(
        resent.pilot_message_id.as_deref(),
        Some("0123456789abcdef"),
        "a delivered run is left as it was"
    );
    let texts: Vec<String> = window
        .update(cx, |shell, _, cx| {
            let pilots = shell.pilot_manager();
            let pilots = pilots.read(cx);
            let chat = pilots.open_chat(&pilot);
            let mut texts: Vec<String> = pilots
                .roster
                .get(&pilot)
                .unwrap()
                .outbox
                .iter()
                .map(|m| format!("{}\n{}", m.id, m.text))
                .collect();
            if let Some(chat) = chat {
                texts.extend(
                    chat.read(cx)
                        .pilot_rows(cx)
                        .iter()
                        .map(|row| row.text.clone()),
                );
            }
            texts
        })
        .unwrap();
    assert!(
        !texts.iter().any(|text| text.contains(&delivered.id)),
        "the delivered run is not sent again: {texts:?}"
    );
    let _ = message;
    assert_eq!(sessions(&window, &id, cx), 0);
}

#[gpui::test]
fn a_pilot_run_started_by_an_issue_alerts_with_the_pilots_name_and_opens_its_chat(
    cx: &mut TestAppContext,
) {
    let data = data();
    let window = open(&data, cx);
    let pilot = add_pilot(&window, &data, "ReviewPilot", cx);
    let chat = window
        .update(cx, |shell, _, cx| {
            shell
                .pilot_manager()
                .read(cx)
                .roster
                .get(&pilot)
                .unwrap()
                .session_id
        })
        .unwrap()
        .expect("the Pilot has a chat");
    let id = waking(
        &data,
        &pilot,
        vec![AutomationTrigger::new("github", "issue_opened")],
    );
    // The page is not showing the Pilot's chat.
    window
        .update(cx, |shell, window, cx| shell.close_app_view(window, cx))
        .unwrap();
    allow_notices(&window, cx);
    let opened = issue(&data, 12, now_ms());
    deliver(&window, vec![opened.clone()], cx);
    deliver(&window, vec![opened], cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].status, "succeeded");
    assert_eq!(
        run_alerts(&window, cx),
        [(
            chat,
            "Check the build".to_string(),
            "Issue opened: acme/app#12 Issue 12 → ReviewPilot".to_string()
        )]
    );
}

// --- GitLab merge request comments.

fn merge_request(data: &Data, number: i64, updated: i64) -> gespan_core::inbox::Item {
    gespan_core::inbox::Item {
        provider: "gitlab".into(),
        kind: "pr".into(),
        number,
        state: "opened".into(),
        title: format!("MR {number}"),
        url: format!("https://gitlab.com/acme/app/-/merge_requests/{number}"),
        repo: "acme/app".into(),
        project_path: data.project.clone(),
        updated_at: chrono::DateTime::from_timestamp_millis(updated)
            .unwrap()
            .to_rfc3339(),
        ..Default::default()
    }
}

fn mr_comment(
    mr: &gespan_core::inbox::Item,
    id: &str,
    author: &str,
    body: &str,
    me: &str,
) -> Vec<gespan_core::inbox::Item> {
    gespan_core::inbox::events::comment_items(
        mr,
        &[gespan_core::inbox::Comment {
            id: id.into(),
            kind: "comment".into(),
            author: author.into(),
            // Someone who can write to the repository.
            permission: "write".into(),
            body: body.into(),
            created_at: chrono::DateTime::from_timestamp_millis(now_ms())
                .unwrap()
                .to_rfc3339(),
            url: format!("{}#note_{id}", mr.url),
            ..Default::default()
        }],
        me,
    )
}

fn on_mr_comment(data: &Data) -> AutomationUpsert {
    draft(
        &data.project,
        vec![AutomationTrigger::new("gitlab", "merge_request_comment")],
    )
}

#[gpui::test]
fn a_new_merge_request_comment_launches_one_run_however_often_it_is_offered(
    cx: &mut TestAppContext,
) {
    let data = data();
    let id = save(&data.db, on_mr_comment(&data));
    let window = open(&data, cx);
    let mr = merge_request(&data, 7, now_ms());
    let first = mr_comment(&mr, "101", "alice", "Please rename this", "gespan-bot");
    deliver(&window, first.clone(), cx);
    deliver(&window, first.clone(), cx);
    let saved = runs(&data.db, &id);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_launched(&saved[0], "event");
    assert_eq!(
        saved[0].event_key.as_deref(),
        Some("gitlab:merge_request_comment:acme/app:7:101")
    );
    assert_eq!(sessions(&window, &id, cx), 1);

    // A second comment on the same merge request is a second run; the
    // connected account's own reply is not.
    let mut next = first;
    next.extend(mr_comment(
        &mr,
        "102",
        "bob",
        "And add a test",
        "gespan-bot",
    ));
    next.extend(mr_comment(&mr, "103", "gespan-bot", "Done", "gespan-bot"));
    deliver(&window, next, cx);
    let keys: Vec<_> = runs(&data.db, &id)
        .into_iter()
        .filter_map(|run| run.event_key)
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys.contains(&"gitlab:merge_request_comment:acme/app:7:102".to_string()));
    assert_eq!(sessions(&window, &id, cx), 2);
}

#[gpui::test]
fn a_merge_request_comment_can_wake_a_pilot_with_the_comment_in_its_message(
    cx: &mut TestAppContext,
) {
    let data = data();
    let window = open(&data, cx);
    let pilot = add_pilot(&window, &data, "ReviewPilot", cx);
    let mut draft = on_mr_comment(&data);
    draft.action = AutomationAction::Pilot {
        pilot_id: pilot.clone(),
    };
    let id = save(&data.db, draft);
    let mr = merge_request(&data, 7, now_ms());
    let comment = mr_comment(&mr, "101", "alice", "Please rename this", "");
    let due = gespan_core::automations::claim_inbox_events(
        &mut Service::open(&data.db).unwrap(),
        &comment,
        now_ms(),
    )
    .unwrap()
    .remove(0);
    let before = all_sessions(&window, cx);
    let queued = launch_for_pilot(&window, due, &pilot, cx);
    assert_eq!(queued.len(), 1, "{queued:?}");
    let text = &queued[0].text;
    assert!(
        text.starts_with("[Automation: Check the build]\nTrigger: Merge request comment  ·  Run: "),
        "{text}"
    );
    for part in [
        "Check the build and report",
        "Respond to this comment on a GitLab merge request:",
        "Comment by @alice:\nPlease rename this",
        "https://gitlab.com/acme/app/-/merge_requests/7#note_101",
    ] {
        assert!(text.contains(part), "{part} missing from {text}");
    }
    let saved = runs(&data.db, &id);
    assert_eq!(saved[0].status, "succeeded");
    assert_eq!(all_sessions(&window, cx), before);
}

#[gpui::test]
fn listing_merge_requests_again_starts_no_opened_run_and_is_looked_at_once(
    cx: &mut TestAppContext,
) {
    let data = data();
    // One automation waits for comments, another for opened merge requests.
    let on_comments = save(&data.db, on_mr_comment(&data));
    let on_opened = save(
        &data.db,
        draft(
            &data.project,
            vec![AutomationTrigger::new("gitlab", "merge_request_opened")],
        ),
    );
    let window = open(&data, cx);
    // The Inbox lists an existing merge request on every refresh. GitLab is
    // not connected here, so its comments cannot be read.
    let listed = vec![merge_request(&data, 7, now_ms())];
    for _ in 0..2 {
        let listed = listed.clone();
        window
            .update(cx, |shell, _, cx| {
                shell.inbox_listed(listed);
                shell.inbox_automation_events(Vec::new(), cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    assert!(
        runs(&data.db, &on_opened).is_empty(),
        "a backlog is not new"
    );
    assert!(runs(&data.db, &on_comments).is_empty());
    let (left, read) = window
        .update(cx, |shell, _, _| shell.listed_watch_for_test())
        .unwrap();
    assert_eq!(left, 0, "the list is looked at once per refresh");
    assert_eq!(read, 0, "a failed read is not marked as read");
}

#[gpui::test]
fn github_comments_and_failed_checks_each_launch_their_own_run(cx: &mut TestAppContext) {
    let data = data();
    let on_comment = save(
        &data.db,
        draft(
            &data.project,
            vec![AutomationTrigger::new("github", "pull_request_comment")],
        ),
    );
    let on_check = save(
        &data.db,
        draft(
            &data.project,
            vec![AutomationTrigger::new("github", "check_failed")],
        ),
    );
    let window = open(&data, cx);
    let mut pr = merge_request(&data, 12, now_ms());
    pr.provider = "github".into();
    pr.url = "https://github.com/acme/app/pull/12".into();
    let mut found = mr_comment(&pr, "IC_1", "alice", "Can you add a test?", "gespan-bot");
    // The account's own reply would wake the agent that wrote it.
    found.extend(mr_comment(
        &pr,
        "IC_2",
        "gespan-bot",
        "Added one",
        "gespan-bot",
    ));
    found.extend(gespan_core::inbox::events::check_failed_items(
        &pr,
        &gespan_core::inbox::Checks {
            head_oid: "abc1234def".into(),
            checks: vec![gespan_core::inbox::Check {
                name: "test".into(),
                workflow: "CI".into(),
                state: "fail".into(),
                completed_at: pr.updated_at.clone().into(),
                ..Default::default()
            }],
        },
    ));
    deliver(&window, found.clone(), cx);
    deliver(&window, found, cx);
    let commented = runs(&data.db, &on_comment);
    assert_eq!(commented.len(), 1, "{commented:?}");
    assert_launched(&commented[0], "event");
    assert_eq!(
        commented[0].event_key.as_deref(),
        Some("github:pull_request_comment:acme/app:12:ic_1")
    );
    let checked = runs(&data.db, &on_check);
    assert_eq!(checked.len(), 1, "{checked:?}");
    assert_launched(&checked[0], "event");
    assert_eq!(
        checked[0].event_key.as_deref(),
        Some("github:check_failed:acme/app:12:abc1234def:ci_/_test")
    );
    assert_eq!(sessions(&window, &on_comment, cx), 1);
    assert_eq!(sessions(&window, &on_check, cx), 1);
}

#[gpui::test]
fn escape_in_automations_closes_its_innermost_layer_first(cx: &mut TestAppContext) {
    let data = data();
    let window = open(&data, cx);
    window
        .update(cx, |shell, window, cx| shell.open_automations(window, cx))
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    let click = |visual: &mut VisualTestContext, selector: &'static str| {
        visual.refresh().unwrap();
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    };
    let state = |visual: &mut VisualTestContext| {
        window
            .update(visual, |shell, _, cx| {
                let view = shell.automations.clone().unwrap();
                let view = view.read(cx);
                (
                    shell.active_app_view == Some(app_views::AppView::Automations),
                    view.editing(),
                    view.project_picker_open(),
                )
            })
            .unwrap()
    };
    click(&mut visual, "automation-scratch");
    click(&mut visual, "automation-project");
    assert_eq!(state(&mut visual), (true, true, true));

    // The picker first, then the editor, then the page itself.
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(state(&mut visual), (true, true, false));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(state(&mut visual), (true, false, false));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!state(&mut visual).0, "the last Escape closes Automations");
}
