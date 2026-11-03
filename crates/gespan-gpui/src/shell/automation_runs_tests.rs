//! Automations start on their own: a due schedule or a new Inbox item opens a
//! run in the main window without the Automations view ever being open.
use super::*;
use gespan_core::automations::{AutomationRun, AutomationTrigger, AutomationUpsert, Service};
use gespan_core::store::Store;
use gpui::{TestAppContext, WindowHandle};
use std::path::{Path, PathBuf};

const HOUR: i64 = 3_600_000;

/// A project folder and the database the app and its automations share.
struct Data {
    dir: tempfile::TempDir,
    db: PathBuf,
    project: String,
}

fn data() -> Data {
    // A run starts its agent; a missing CLI makes that fail at once instead
    // of starting a real Codex.
    // SAFETY: set before the window starts any thread that reads the
    // environment; CI runs each test in its own process.
    unsafe { std::env::set_var("GESPAN_CODEX_BINARY", "/missing/gespan-test/codex") };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let db = dir.path().join("gespan.db");
    Data {
        project: project.to_string_lossy().into_owned(),
        db,
        dir,
    }
}

/// Opens the main window, which starts its automations.
fn open(data: &Data, cx: &mut TestAppContext) -> WindowHandle<Shell> {
    let store = Arc::new(Mutex::new(Store::open(&data.db).unwrap()));
    cx.update(crate::shell::bind_keys);
    let cwd = data.project.clone();
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
    let id = save(
        &data.db,
        draft(
            &data.project,
            vec![AutomationTrigger::new("time", "hourly")],
        ),
    );
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

    window
        .update(cx, |shell, _, cx| shell.resume_all(cx))
        .unwrap();
    cx.run_until_parked();
    poll(&window, cx);
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
    poll(&window, cx);
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
    drop(data.dir);
}
