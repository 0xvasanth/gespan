//! The app's side of the decision log: what the user wrote in a session, so a
//! proposal can be checked against the user's own words.
use crate::runtime::{Database, locked};
use gespan_core::outside_turns::without_injected;
use gespan_core::pilot::files::content_hash as digest;
use gespan_core::store::Store;
use gespan_core::transcript::Row;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// How many of a session's answers are kept, newest last.
const ANSWERS_KEPT: usize = 50;
/// How many of a session's typed messages are remembered, newest last.
const TYPED_KEPT: usize = 500;

/// The user's answers to agents' questions, per session. They reach the
/// provider, not the transcript, so they are kept here for this run of the app.
static ANSWERS: LazyLock<Mutex<HashMap<i64, VecDeque<String>>>> = LazyLock::new(Default::default);

/// Where a session's typed messages are recorded: the SHA-256 of each `user`
/// message the human typed or sent, never the text. A session without it was
/// opened by an agent, a Pilot, a habit or a Flow (or before Gespan recorded
/// this), and nothing in it counts as the user's.
fn typed_key(session: i64) -> String {
    format!("decision-typed:{session}")
}

/// Whether this launch's message is the human's own words: typed or sent by
/// them, and not a Pilot's, an orchestration worker's or an app message.
/// Turns the app or an agent starts (habits, Flows, resumes, `sessions.*`
/// sends, reports) are launched with `user_typed` off.
pub(crate) fn typed_by_user(settings: &gespan_core::agent::Settings) -> bool {
    settings.user_typed
        && !settings.internal_message
        && settings.pilot.is_none()
        && !settings.orchestration_worker
}

/// Records `text`, a `user` message exactly as saved, as one the human typed
/// in `session`. `opens_session` says it is the session's first message: only
/// that starts the record, so a session an agent opened never gets one.
pub(crate) fn record_typed(
    store: &Store,
    session: i64,
    text: &str,
    opens_session: bool,
) -> Result<(), String> {
    let key = typed_key(session);
    let mut typed: Vec<String> = match store.setting(&key)? {
        Some(saved) => serde_json::from_str(&saved).map_err(|e| e.to_string())?,
        None if opens_session => Vec::new(),
        None => return Ok(()),
    };
    let hash = digest(text);
    typed.retain(|kept| *kept != hash);
    typed.push(hash);
    let excess = typed.len().saturating_sub(TYPED_KEPT);
    typed.drain(..excess);
    store.save_setting(
        &key,
        &serde_json::to_string(&typed).map_err(|e| e.to_string())?,
    )
}

/// The hashes of `session`'s typed messages; `None` without a usable record.
fn typed_record(store: &Store, session: i64) -> Option<HashSet<String>> {
    let saved = store.setting(&typed_key(session)).ok()??;
    serde_json::from_str::<Vec<String>>(&saved)
        .map(|typed| typed.into_iter().collect())
        .ok()
}

/// Remembers the user's answers to an agent's question in `session`: each
/// chosen option or typed answer on its own.
pub(crate) fn record_answers(session: i64, answers: &[Vec<String>]) {
    let mut kept = ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    let session = kept.entry(session).or_default();
    for answer in answers.iter().flatten() {
        let answer = answer.trim();
        if answer.is_empty() {
            continue;
        }
        session.push_back(answer.to_owned());
        if session.len() > ANSWERS_KEPT {
            session.pop_front();
        }
    }
}

fn answers(session: i64) -> Vec<String> {
    let kept = ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    kept.get(&session)
        .map(|answers| answers.iter().cloned().collect())
        .unwrap_or_default()
}

/// What the user wrote in `session`: the messages they typed, as saved in its
/// transcript, then their answers to its agent's questions. A session with no
/// record of typed messages (opened by an agent, a Pilot, a habit or a Flow)
/// gives nothing, and so does a remote one: its transcript lives elsewhere.
pub(crate) fn user_texts(store: &Store, session: i64) -> Vec<String> {
    if matches!(store.remote_session(session), Ok(Some(_))) {
        return Vec::new();
    }
    let Some(typed_hashes) = typed_record(store, session) else {
        return Vec::new();
    };
    let mut texts = match store.load(session) {
        Ok(transcript) => typed(&transcript.rows, &typed_hashes),
        Err(error) => {
            tracing::warn!(session, %error, "the decision log could not read a transcript");
            Vec::new()
        }
    };
    texts.extend(answers(session));
    texts
}

/// The human's own messages among `rows`: `user` messages, rejoined from
/// their stored fragments, whose text is in `typed_hashes`, without the
/// context Gespan put in front of them. Everything else (agent text, tools,
/// attachments, app messages, and prompts an agent or the app sent as `user`)
/// is left out.
fn typed(rows: &[Row], typed_hashes: &HashSet<String>) -> Vec<String> {
    let mut messages: Vec<String> = Vec::new();
    let mut in_user = false;
    for row in rows {
        if row.first {
            in_user = row.role == "user";
            if in_user {
                messages.push(String::new());
            }
        }
        if in_user
            && row.role == "user"
            && let Some(message) = messages.last_mut()
        {
            message.push_str(&row.text);
        }
    }
    messages
        .iter()
        .filter(|message| typed_hashes.contains(&digest(message)))
        .map(|message| without_injected(message).to_owned())
        .filter(|message| !message.is_empty())
        .collect()
}

/// Registers [`user_texts`] as the decision log's source of the user's words,
/// and the harness writer as its regeneration hook.
/// The hub calls it on its own thread, so it reads the app database itself.
pub(crate) fn install(store: Database) {
    install_harness_writer(store.clone());
    gespan_core::decisions::service::set_user_text_source(Box::new(move |session| {
        match locked(&store) {
            Ok(store) => user_texts(&store, session),
            Err(_) => Vec::new(),
        }
    }));
}

/// The open projects, as the shell last saved them, so the harness writer
/// (which runs off the UI thread) knows which checkouts to write for.
static KNOWN_PROJECTS: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Default::default);

/// Records the open projects; true when the list changed.
pub(crate) fn set_known_projects(projects: &[String]) -> bool {
    let mut known = KNOWN_PROJECTS.lock().unwrap_or_else(|e| e.into_inner());
    if *known == projects {
        return false;
    }
    *known = projects.to_vec();
    true
}

/// Every local checkout Gespan knows: the open projects and their
/// worktrees. Remote (SSH) projects get nothing in v1. Runs git, so call it
/// off the UI thread.
fn known_workspaces() -> Vec<PathBuf> {
    let projects = KNOWN_PROJECTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let mut out: Vec<PathBuf> = Vec::new();
    for project in projects {
        let path = PathBuf::from(&project);
        if gespan_core::files::remote::is_remote(&path) || !path.is_dir() {
            continue;
        }
        let trees = gespan_core::git::worktrees(&path).unwrap_or_default();
        let found = trees
            .into_iter()
            .filter(|tree| !tree.missing)
            .map(|tree| PathBuf::from(tree.path));
        for candidate in std::iter::once(path.clone()).chain(found) {
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

/// What the harness writer should do when the log's switch is applied.
#[derive(Debug, PartialEq, Eq)]
enum SwitchAction {
    Regenerate,
    RemoveAll,
}

/// The first apply at startup regenerates when the log is on; turning it on
/// regenerates; turning it off removes what Gespan wrote. An unchanged
/// switch, or the log off from the start, does nothing.
fn switch_action(previous: Option<bool>, on: bool) -> Option<SwitchAction> {
    match (previous, on) {
        (Some(was), now) if was == now => None,
        (_, true) => Some(SwitchAction::Regenerate),
        (Some(true), false) => Some(SwitchAction::RemoveAll),
        (_, false) => None,
    }
}

static SWITCH: Mutex<Option<bool>> = Mutex::new(None);
static HARNESS_STORE: Mutex<Option<Database>> = Mutex::new(None);

/// Applies the log's switch to the harness files: called with every
/// preferences apply, including the first one at startup.
pub(crate) fn decision_log_switched(on: bool) {
    let previous = SWITCH.lock().unwrap_or_else(|e| e.into_inner()).replace(on);
    match switch_action(previous, on) {
        Some(SwitchAction::Regenerate) => gespan_core::decisions::service::regenerate(),
        Some(SwitchAction::RemoveAll) => {
            let store = HARNESS_STORE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(store) = store {
                std::thread::spawn(move || write_harness_files(&store, false));
            }
        }
        None => {}
    }
}

/// The paths whose "edit replaced" notice is waiting for a window to show.
static EDIT_NOTICES: LazyLock<(
    async_channel::Sender<String>,
    async_channel::Receiver<String>,
)> = LazyLock::new(async_channel::unbounded);

/// Notices for user edits Gespan replaced, as they come in.
pub(crate) fn edit_notices() -> async_channel::Receiver<String> {
    EDIT_NOTICES.1.clone()
}

fn edit_notice_key(path: &Path) -> String {
    format!("decision-edit-notice:{}", path.display())
}

pub(crate) fn edit_notice_text(path: &Path) -> String {
    format!(
        "Gespan replaced your edit inside its section of {}. Edit decisions in My tasks instead.",
        path.display()
    )
}

/// The notices to show for `paths` (files whose Gespan section the user had
/// edited): one per path, ever. Marks each as shown.
fn unseen_edit_notices(store: &Store, paths: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    for path in paths {
        let key = edit_notice_key(path);
        if matches!(store.setting(&key), Ok(Some(_))) {
            continue;
        }
        if let Err(error) = store.save_setting(&key, "shown") {
            tracing::warn!(%error, "the decision edit notice could not be recorded");
            continue;
        }
        out.push(edit_notice_text(path));
    }
    out
}

/// Registers the harness writer as the log's regeneration hook. Each run
/// happens on its own thread, never the UI's; the core serializes them.
fn install_harness_writer(store: Database) {
    *HARNESS_STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(store.clone());
    gespan_core::decisions::service::set_regenerate(Box::new(move || {
        if !gespan_core::decisions::enabled() {
            return;
        }
        let store = store.clone();
        std::thread::spawn(move || write_harness_files(&store, true));
    }));
}

/// Regenerates (or, with `regenerate` off, removes) every harness file for
/// the real home, the app's data dir, its accounts and its known checkouts.
fn write_harness_files(store: &Database, regenerate: bool) {
    use gespan_core::decisions::harness;
    let Some(home) = gespan_core::accounts::home() else {
        tracing::warn!("the decision log found no home folder to write to");
        return;
    };
    let Ok((book, data_dir)) = locked(store).map(|s| {
        (
            s.provider_accounts().unwrap_or_default(),
            s.data_dir().map(Path::to_path_buf),
        )
    }) else {
        return;
    };
    let Some(data_dir) = data_dir else {
        return;
    };
    let workspaces = known_workspaces();
    let report = if regenerate {
        match gespan_core::decisions::service::with(|log| {
            harness::regenerate(log, &book, &data_dir, &home, &workspaces)
        }) {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(%error, "the decision log could not write harness files");
                return;
            }
        }
    } else {
        harness::remove_all(&book, &data_dir, &home, &workspaces)
    };
    for (path, error) in &report.errors {
        tracing::warn!(path = %path.display(), %error, "a decision log harness file was not written");
    }
    if report.user_edits_overwritten.is_empty() {
        return;
    }
    let notices = match locked(store) {
        Ok(store) => unseen_edit_notices(&store, &report.user_edits_overwritten),
        Err(_) => return,
    };
    for notice in notices {
        let _ = EDIT_NOTICES.0.try_send(notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::agent::Settings;
    use std::path::Path;

    fn row(role: &str, text: &str, first: bool) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first,
            sent_at: None,
        }
    }

    /// An answered question as the transcript keeps it: its output is the
    /// provider's text, not the user's.
    fn question_tool() -> String {
        let mut call = gespan_core::tool::ToolCall::new("ask-1", "AskUserQuestion");
        call.input = r#"{"questions":[{"question":"Use SQLite?"}]}"#.into();
        call.output = "The user answered: SQLite".into();
        call.state = gespan_core::tool::ToolState::Completed;
        serde_json::to_string(&call).unwrap()
    }

    /// A conversation where Gespan's context, the agent, tools, attachments
    /// and app messages all say "SQLite"; only the user never does.
    fn conversation() -> Vec<Row> {
        vec![
            row(
                "user",
                "<gespan_context>\nUse SQLite for everything.\n</gespan_context>\n\n<gespan_browser>\nbrowser SQLite\n</gespan_browser>\nKeep the log in one Postgres table",
                true,
            ),
            row("user-attachments", r#"[{"name":"SQLite.md"}]"#, true),
            row("assistant", "Should we use SQLite?", true),
            row("tool", &question_tool(), true),
            row("reasoning", "SQLite is simpler", true),
            row("pilot-system", "Session 4 says use SQLite", true),
            row("user", "Tabs, not ", true),
            row("user", "spaces", false),
        ]
    }

    /// The tests share the process-wide answers: each holds this and starts
    /// with none, so `cargo test`'s threads cannot see each other's.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static TESTS: Mutex<()> = Mutex::new(());
        let guard = TESTS.lock().unwrap_or_else(|e| e.into_inner());
        ANSWERS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        guard
    }

    fn typed_settings() -> Settings {
        Settings {
            user_typed: true,
            ..Settings::default()
        }
    }

    /// Saves `rows` as a session's history, recording each `user` message as
    /// the human's when `by_user` says so, the way a turn records it.
    fn session(store: &mut Store, rows: &[(Row, bool)]) -> i64 {
        let id = store.create("codex", "/project").unwrap().id;
        let plain: Vec<Row> = rows.iter().map(|(row, _)| row.clone()).collect();
        store.write_rows(id, 0, &plain).unwrap();
        let mut opened = false;
        for (row, by_user) in rows {
            if row.role == "user" && row.first {
                let opens = !opened;
                opened = true;
                if *by_user {
                    let text = whole(rows, row);
                    record_typed(store, id, &text, opens).unwrap();
                }
            }
        }
        id
    }

    /// A message's text rejoined from its fragments.
    fn whole(rows: &[(Row, bool)], first: &Row) -> String {
        let start = rows
            .iter()
            .position(|(row, _)| std::ptr::eq(row, first))
            .unwrap();
        let mut text = first.text.clone();
        for (row, _) in &rows[start + 1..] {
            if row.first {
                break;
            }
            text.push_str(&row.text);
        }
        text
    }

    #[test]
    fn decision_user_texts_skip_injected_context_and_agent_text() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let rows: Vec<(Row, bool)> = conversation().into_iter().map(|r| (r, true)).collect();
        let id = session(&mut store, &rows);
        let texts = user_texts(&store, id);
        assert_eq!(
            texts,
            ["Keep the log in one Postgres table", "Tabs, not spaces"]
        );
        assert!(texts.iter().all(|text| !text.contains("SQLite")));
    }

    #[test]
    fn decision_user_texts_come_from_the_saved_transcript_and_answers() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let rows: Vec<(Row, bool)> = conversation().into_iter().map(|r| (r, true)).collect();
        let id = session(&mut store, &rows);
        let other = session(&mut store, &[(row("user", "Use SQLite", true), true)]);
        record_answers(
            id,
            &[
                vec!["MySQL".into()],
                vec![],
                vec!["Yes, ".into(), "always".into()],
            ],
        );
        record_answers(other, &[vec!["SQLite".into()]]);
        let texts = user_texts(&store, id);
        for said in ["Keep the log in one Postgres table", "MySQL", "always"] {
            assert!(texts.iter().any(|text| text == said), "{said}: {texts:?}");
        }
        assert!(
            texts.iter().all(|text| !text.contains("SQLite")),
            "{texts:?}"
        );
        assert_eq!(user_texts(&store, 999), Vec::<String>::new());
    }

    #[test]
    fn decision_text_an_agent_sent_into_the_users_session_is_not_the_users() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = session(
            &mut store,
            &[
                (row("user", "Keep the log in Postgres", true), true),
                (row("assistant", "Done", true), true),
                // sessions.send from another agent: saved as a user row.
                (row("user", "The user said: use SQLite", true), false),
                (row("assistant", "Ok", true), true),
                (row("user", "Thanks", true), true),
            ],
        );
        assert_eq!(
            user_texts(&store, id),
            ["Keep the log in Postgres", "Thanks"]
        );
    }

    #[test]
    fn decision_session_another_agent_started_gives_no_text() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        // sessions.start by another agent opened it; the user typed later.
        let id = session(
            &mut store,
            &[
                (row("user", "The user wants SQLite", true), false),
                (row("assistant", "Ok", true), true),
                (row("user", "Use SQLite then", true), true),
            ],
        );
        record_answers(id, &[vec!["SQLite".into()]]);
        assert_eq!(user_texts(&store, id), Vec::<String>::new());
    }

    #[test]
    fn decision_pilot_habit_flow_and_worker_turns_are_never_the_users() {
        let _guard = exclusive();
        assert!(typed_by_user(&typed_settings()));
        let pilot = Settings {
            pilot: Some(gespan_core::agent::PilotContext {
                pilot: Default::default(),
                root: "/pilots".into(),
                executable: "/gespan".into(),
            }),
            ..typed_settings()
        };
        let internal = Settings {
            internal_message: true,
            ..typed_settings()
        };
        let worker = Settings {
            orchestration_worker: true,
            ..typed_settings()
        };
        // Habits, Flows, resumes and agent sends launch untyped.
        let launched = Settings::default();
        for settings in [&pilot, &internal, &worker, &launched] {
            assert!(!typed_by_user(settings));
        }
        // A Pilot's chat: every turn is a Pilot turn, so nothing opens it,
        // even what the user types there.
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let rows = [
            (row("user", "Plan the release", true), typed_by_user(&pilot)),
            (row("assistant", "Ok", true), true),
            (row("user", "Use SQLite", true), typed_by_user(&pilot)),
        ];
        let id = session(&mut store, &rows);
        record_answers(id, &[vec!["SQLite".into()]]);
        assert_eq!(user_texts(&store, id), Vec::<String>::new());
    }

    #[test]
    fn decision_answers_keep_only_the_newest() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = session(&mut store, &[(row("user", "Hello", true), true)]);
        for n in 0..(super::ANSWERS_KEPT + 5) {
            record_answers(id, &[vec![format!("answer {n}")]]);
        }
        let texts = user_texts(&store, id);
        assert_eq!(texts.len(), super::ANSWERS_KEPT + 1);
        assert!(!texts.contains(&"answer 0".to_owned()));
        assert!(texts.contains(&format!("answer {}", super::ANSWERS_KEPT + 4)));
    }

    #[test]
    fn decision_log_source_reads_the_app_database_off_the_ui_thread() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = session(
            &mut store,
            &[(row("user", "Ship it on Friday", true), true)],
        );
        install(std::sync::Arc::new(std::sync::Mutex::new(store)));
        let texts = std::thread::spawn(move || gespan_core::decisions::service::user_texts(id))
            .join()
            .unwrap();
        assert_eq!(texts, ["Ship it on Friday"]);
    }

    #[test]
    fn decision_edit_notice_shows_once_per_path() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let claude = PathBuf::from("home").join(".claude").join("CLAUDE.md");
        let codex = PathBuf::from("home").join(".codex").join("AGENTS.md");
        let first = unseen_edit_notices(&store, std::slice::from_ref(&claude));
        assert_eq!(
            first,
            [format!(
                "Gespan replaced your edit inside its section of {}. Edit decisions in My tasks instead.",
                claude.display()
            )]
        );
        assert_eq!(
            store
                .setting(&format!("decision-edit-notice:{}", claude.display()))
                .unwrap()
                .as_deref(),
            Some("shown")
        );
        // The same file again stays quiet; another file still gets its one.
        let next = unseen_edit_notices(&store, &[claude.clone(), codex.clone()]);
        assert_eq!(next, [edit_notice_text(&codex)]);
        assert!(unseen_edit_notices(&store, &[claude, codex]).is_empty());
    }

    #[test]
    fn decision_switch_regenerates_when_on_and_cleans_up_only_when_turned_off() {
        // At startup: on writes the files, off leaves the disk alone.
        assert_eq!(switch_action(None, true), Some(SwitchAction::Regenerate));
        assert_eq!(switch_action(None, false), None);
        // Turning it off removes what Gespan wrote; on writes it back.
        assert_eq!(
            switch_action(Some(true), false),
            Some(SwitchAction::RemoveAll)
        );
        assert_eq!(
            switch_action(Some(false), true),
            Some(SwitchAction::Regenerate)
        );
        // Other preference changes re-apply an unchanged switch: nothing.
        assert_eq!(switch_action(Some(true), true), None);
        assert_eq!(switch_action(Some(false), false), None);
    }

    #[test]
    fn decision_known_projects_report_changes_only() {
        let _guard = exclusive();
        let saved = KNOWN_PROJECTS.lock().unwrap().clone();
        let a = vec!["/a".to_string(), "/b".to_string()];
        set_known_projects(&[]);
        assert!(set_known_projects(&a));
        assert!(!set_known_projects(&a));
        assert!(set_known_projects(&a[..1]));
        *KNOWN_PROJECTS.lock().unwrap() = saved;
    }
}
