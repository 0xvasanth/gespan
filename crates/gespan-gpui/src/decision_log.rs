//! The app's side of the decision log: what the user wrote in a session, so a
//! proposal can be checked against the user's own words.
use crate::runtime::{Database, locked};
use gespan_core::outside_turns::without_injected;
use gespan_core::store::Store;
use gespan_core::transcript::Row;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

/// How many of a session's answers are kept, newest last.
const ANSWERS_KEPT: usize = 50;

/// The user's answers to agents' questions, per session. They reach the
/// provider, not the transcript, so they are kept here for this run of the app.
static ANSWERS: LazyLock<Mutex<HashMap<i64, VecDeque<String>>>> = LazyLock::new(Default::default);

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

/// What the user wrote in `session`: the messages they sent, as saved in its
/// transcript, then their answers to its agent's questions. Remote sessions
/// give only answers: their transcripts live on another machine.
pub(crate) fn user_texts(store: &Store, session: i64) -> Vec<String> {
    let local = !matches!(store.remote_session(session), Ok(Some(_)));
    let mut texts = if local {
        match store.load(session) {
            Ok(transcript) => typed(&transcript.rows),
            Err(error) => {
                tracing::warn!(session, %error, "the decision log could not read a transcript");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    texts.extend(answers(session));
    texts
}

/// The user's own messages among `rows`, each rejoined from its stored
/// fragments and without the context Gespan put in front of it. Agent text,
/// tools, attachments and app messages (`pilot-system`) are left out.
fn typed(rows: &[Row]) -> Vec<String> {
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
            Err(_) => answers(session),
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn decision_user_texts_skip_injected_context_and_agent_text() {
        let texts = typed(&conversation());
        assert_eq!(
            texts,
            ["Keep the log in one Postgres table", "Tabs, not spaces"]
        );
        assert!(texts.iter().all(|text| !text.contains("SQLite")));
    }

    #[test]
    fn decision_user_texts_come_from_the_saved_transcript_and_answers() {
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap().id;
        let other = store.create("codex", "/project").unwrap().id;
        store.write_rows(session, 0, &conversation()).unwrap();
        store
            .write_rows(other, 0, &[row("user", "Use SQLite", true)])
            .unwrap();
        record_answers(
            session,
            &[
                vec!["MySQL".into()],
                vec![],
                vec!["Yes, ".into(), "always".into()],
            ],
        );
        record_answers(other, &[vec!["SQLite".into()]]);
        let texts = user_texts(&store, session);
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
    fn decision_answers_keep_only_the_newest() {
        let session = 4242;
        for n in 0..(super::ANSWERS_KEPT + 5) {
            record_answers(session, &[vec![format!("answer {n}")]]);
        }
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let texts = user_texts(&store, session);
        assert_eq!(texts.len(), super::ANSWERS_KEPT);
        assert!(!texts.contains(&"answer 0".to_owned()));
        assert!(texts.contains(&format!("answer {}", super::ANSWERS_KEPT + 4)));
    }

    #[test]
    fn decision_log_source_reads_the_app_database_off_the_ui_thread() {
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap().id;
        store
            .write_rows(session, 0, &[row("user", "Ship it on Friday", true)])
            .unwrap();
        install(std::sync::Arc::new(std::sync::Mutex::new(store)));
        let texts =
            std::thread::spawn(move || gespan_core::decisions::service::user_texts(session))
                .join()
                .unwrap();
        assert_eq!(texts, ["Ship it on Friday"]);
    }
}
