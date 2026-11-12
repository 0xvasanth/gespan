//! The app's side of the decision log: what the user wrote in a session, so a
//! proposal can be checked against the user's own words.
use crate::runtime::{Database, locked};
use gespan_core::outside_turns::without_injected;
use gespan_core::pilot::files::content_hash as digest;
use gespan_core::store::Store;
use gespan_core::transcript::Row;
use std::collections::{HashMap, HashSet, VecDeque};
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

/// Registers [`user_texts`] as the decision log's source of the user's words.
/// The hub calls it on its own thread, so it reads the app database itself.
pub(crate) fn install(store: Database) {
    gespan_core::decisions::service::set_user_text_source(Box::new(move |session| {
        match locked(&store) {
            Ok(store) => user_texts(&store, session),
            Err(_) => Vec::new(),
        }
    }));
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
}
