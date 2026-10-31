//! Ask sessions, kept out of every session list. The shared store marks them
//! in its `sessions.inbox_ask` column, as the Tauri app does. The local
//! fallback has no such column, so it uses an `inbox-ask:<id>` setting, which
//! `local_sessions` and `delete` already honour.
use super::{Session, Store};
use crate::{agent::Settings, inbox::ask::Context};
use rusqlite::params;

impl Store {
    /// Newest first. A row with an unreadable context is left alone.
    fn inbox_asks(&self) -> Result<Vec<(Session, Context)>, String> {
        let rows = if self.shared.is_some() {
            self.shared_inbox_asks()?
        } else {
            self.local_inbox_asks()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(session, raw)| Some((session, serde_json::from_str(&raw).ok()?)))
            .collect())
    }
    pub fn inbox_ask(&self, key: &str) -> Result<Option<(Session, Context)>, String> {
        Ok(self
            .inbox_asks()?
            .into_iter()
            .find(|(_, context)| context.key == key))
    }
    /// The item's existing conversation, or a new one under `settings`.
    pub fn open_inbox_ask(
        &self,
        provider: &str,
        cwd: &str,
        context: &Context,
        settings: &Settings,
    ) -> Result<(Session, Context), String> {
        if let Some(existing) = self.inbox_ask(&context.key)? {
            return Ok(existing);
        }
        // A shared database older than the column would list the session.
        if self.shared.is_some() && !self.shared_supports_inbox_ask() {
            return Err("This Gespan database cannot save Ask conversations yet".into());
        }
        let raw = serde_json::to_string(context).map_err(|e| e.to_string())?;
        // A session that never got its mark would show up in the sidebar.
        let tx = super::write_transaction(&self.conn).map_err(|e| e.to_string())?;
        let mut session = self.local_create(provider, cwd)?;
        session.title = context.session_title();
        self.title(session.id, &session.title)?;
        if self.shared.is_some() {
            self.shared_patch(session.id, &[("inbox_ask", serde_json::json!(raw))])?;
        } else {
            self.conn
                .execute(
                    "INSERT INTO native_settings(key,value) VALUES(?1,?2)",
                    params![format!("inbox-ask:{}", session.id), raw],
                )
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.save_agent_settings(session.id, settings)?;
        Ok((session, context.clone()))
    }
    /// Replaces the conversation with an empty one on the same terms.
    pub fn restart_inbox_ask(&mut self, id: i64) -> Result<(Session, Context), String> {
        let (session, context) = self
            .inbox_asks()?
            .into_iter()
            .find(|(session, _)| session.id == id)
            .ok_or("This Ask conversation no longer exists")?;
        let saved = self.agent_settings(id)?.unwrap_or_default();
        let settings = Settings {
            account: saved.account,
            model: saved.model,
            effort: saved.effort,
            access: saved.access,
            fast: saved.fast,
            service_tier: saved.service_tier,
            thinking: saved.thinking,
            model_settings: saved.model_settings,
            ..Default::default()
        };
        // Checked first: a restart that cannot start keeps the conversation.
        crate::provider::Kind::parse(&session.provider)
            .ok_or_else(|| format!("Unknown provider: {}", session.provider))?;
        self.delete(id)?;
        self.open_inbox_ask(&session.provider, &session.cwd, &context, &settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        inbox::{ask, Item},
        transcript::Row,
    };
    use std::path::Path;

    fn item(number: i64) -> Item {
        Item {
            provider: "github".into(),
            kind: "pr".into(),
            number,
            title: format!("Item {number}"),
            url: format!("https://github.com/acme/widgets/pull/{number}"),
            repo: "acme/widgets".into(),
            ..Default::default()
        }
    }
    fn context(number: i64) -> Context {
        Context::new(&item(number), Some(format!("About {number}")))
    }
    /// Every rule must hold on both the shared store and the local fallback.
    fn stores() -> [Store; 2] {
        [
            Store::open(Path::new(":memory:")).unwrap(),
            Store::open_gespan(Path::new(":memory:")).unwrap(),
        ]
    }
    fn open(store: &Store, number: i64) -> (Session, Context) {
        store
            .open_inbox_ask("codex", "/project", &context(number), &Settings::default())
            .unwrap()
    }
    fn prompt_row(text: &str) -> Row {
        Row {
            role: "user".into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }

    #[test]
    fn asking_again_reuses_the_session_and_each_item_has_its_own() {
        for store in stores() {
            let (first, saved) = open(&store, 1);
            assert_eq!(first.title, "Ask · Item 1");
            assert_eq!(first.cwd, "/project");
            assert_eq!(saved, context(1));
            let (again, saved) = open(&store, 1);
            assert_eq!(again.id, first.id);
            assert_eq!(saved, context(1));
            let (other, saved) = open(&store, 2);
            assert_ne!(other.id, first.id);
            assert_eq!(saved, context(2));
            let found = store.inbox_ask(&context(1).key).unwrap().unwrap();
            assert_eq!(found.0.id, first.id);
            assert!(store.inbox_ask("github:unknown").unwrap().is_none());
        }
    }

    #[test]
    fn the_saved_context_wins_over_a_newer_copy_of_the_item() {
        for store in stores() {
            let (first, _) = open(&store, 1);
            let mut renamed = item(1);
            renamed.title = "Renamed upstream".into();
            let (again, saved) = store
                .open_inbox_ask(
                    "codex",
                    "/project",
                    &Context::new(&renamed, None),
                    &Settings::default(),
                )
                .unwrap();
            assert_eq!(again.id, first.id);
            assert_eq!(saved, context(1));
        }
    }

    #[test]
    fn a_reopened_session_reports_its_messages() {
        for mut store in stores() {
            let (first, _) = open(&store, 1);
            assert!(!first.has_messages);
            store
                .write_rows(first.id, 0, &[prompt_row("Why is this failing?")])
                .unwrap();
            assert!(open(&store, 1).0.has_messages);
        }
    }

    #[test]
    fn ask_sessions_stay_out_of_the_session_list_and_search() {
        for store in stores() {
            let visible = store.create("codex", "/project").unwrap();
            let (asked, _) = open(&store, 1);
            let ids: Vec<_> = store.sessions().unwrap().iter().map(|s| s.id).collect();
            assert_eq!(ids, [visible.id]);
            assert_ne!(asked.id, visible.id);
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (hits, _) = store.search_sessions("Item 1", cancel).unwrap();
            assert!(hits.is_empty());
        }
    }

    #[test]
    fn a_restart_that_cannot_start_keeps_the_conversation() {
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let context = ask::Context::new(&item(1), None);
        let (session, _) = store
            .open_inbox_ask("codex", "/project", &context, &Default::default())
            .unwrap();
        store
            .conn
            .execute("UPDATE sessions SET harness='nonsense'", [])
            .unwrap();
        assert!(store.restart_inbox_ask(session.id).is_err());
        assert!(
            store.inbox_ask(&context.key).unwrap().is_some(),
            "the old conversation survives"
        );
    }
    #[test]
    fn restart_replaces_the_session_and_keeps_its_terms() {
        for mut store in stores() {
            let (first, _) = open(&store, 1);
            let (other, _) = open(&store, 2);
            store
                .write_rows(first.id, 0, &[prompt_row("Old question")])
                .unwrap();
            let settings = Settings {
                model: Some("gpt-test".into()),
                effort: Some("high".into()),
                ..Default::default()
            };
            store.save_agent_settings(first.id, &settings).unwrap();
            let (fresh, saved) = store.restart_inbox_ask(first.id).unwrap();
            assert_ne!(fresh.id, first.id);
            assert_eq!(fresh.title, "Ask · Item 1");
            assert_eq!(fresh.cwd, "/project");
            assert_eq!(fresh.provider, "codex");
            assert!(!fresh.has_messages);
            assert_eq!(saved, context(1));
            let found = store.inbox_ask(&context(1).key).unwrap().unwrap();
            assert_eq!(found.0.id, fresh.id, "the next Ask finds the new session");
            assert!(store.load(fresh.id).unwrap().rows.is_empty());
            let saved = store.agent_settings(fresh.id).unwrap().unwrap();
            assert_eq!(saved.model.as_deref(), Some("gpt-test"));
            assert_eq!(saved.effort.as_deref(), Some("high"));
            let found = store.inbox_ask(&context(2).key).unwrap().unwrap();
            assert_eq!(found.0.id, other.id, "other items keep their conversation");
            assert!(store.sessions().unwrap().is_empty());
            assert!(store.restart_inbox_ask(first.id).is_err());
        }
    }

    #[test]
    fn a_row_with_an_unreadable_context_is_not_a_match() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let (first, _) = open(&store, 1);
        let original = store.persisted_session_id(first.id).unwrap();
        store
            .conn
            .execute(
                "UPDATE sessions SET inbox_ask='ask' WHERE id=?1",
                [&original],
            )
            .unwrap();
        assert!(store.inbox_ask(&context(1).key).unwrap().is_none());
        assert!(store.sessions().unwrap().is_empty(), "still hidden");
        let (fresh, _) = open(&store, 1);
        assert_ne!(fresh.id, first.id);
    }

    #[test]
    fn the_key_matches_a_session_saved_by_the_tauri_app() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let original = store.persisted_session_id(session.id).unwrap();
        store
            .conn
            .execute(
                "UPDATE sessions SET inbox_ask=?2 WHERE id=?1",
                params![
                    original,
                    r#"{"key":"github:github.com:/acme/widgets/pull/1","title":"Item 1","url":"https://github.com/acme/widgets/pull/1","provider":"github"}"#
                ],
            )
            .unwrap();
        let key = ask::key(&item(1));
        assert_eq!(store.inbox_ask(&key).unwrap().unwrap().0.id, session.id);
    }
}
