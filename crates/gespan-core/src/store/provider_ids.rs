//! Every agent session Gespan holds, by the agent's own id, including the
//! ones no session list shows.
use std::collections::HashSet;

use super::schema::SessionRecord;
use super::Store;

impl Store {
    /// `(provider, provider id)` of every session in the store: live,
    /// archived, and the ones [`Store::sessions`] hides (Ask conversations,
    /// orchestration workers, hidden Pilot and operator conversations, hidden
    /// legacy remote sessions). Sessions without a provider id are left out.
    pub fn provider_session_ids(&self) -> Result<HashSet<(String, String)>, String> {
        let rows = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter(
                session
                    .provider_session_id()
                    .is_some()
                    .and(session.provider_session_id().ne(Some(String::new()))),
            )
            .select((session.harness(), session.provider_session_id()))
            .exec(&mut db)
            .await
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(harness, id)| Some((harness, id?)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent::Settings, inbox::ask::Context, inbox::Item};
    use std::path::Path;

    fn ids(store: &Store) -> HashSet<(String, String)> {
        store.provider_session_ids().unwrap()
    }
    fn has(store: &Store, provider: &str, id: &str) -> bool {
        ids(store).contains(&(provider.to_owned(), id.to_owned()))
    }
    fn ask(store: &Store) -> i64 {
        let item = Item {
            provider: "github".into(),
            kind: "pr".into(),
            number: 7,
            title: "Item 7".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            repo: "acme/widgets".into(),
            ..Default::default()
        };
        let context = Context::new(&item, None);
        let (session, _) = store
            .open_inbox_ask("claude", "/project", &context, &Settings::default())
            .unwrap();
        session.id
    }

    #[test]
    fn lists_live_and_archived_sessions_on_both_backends() {
        for store in [
            Store::open(Path::new(":memory:")).unwrap(),
            Store::open_gespan(Path::new(":memory:")).unwrap(),
        ] {
            let live = store.create("claude", "/project").unwrap();
            store.provider_id(live.id, "live").unwrap();
            let old = store.create("codex", "/project").unwrap();
            store.provider_id(old.id, "old").unwrap();
            store.set_archived(old.id, true).unwrap();
            store.create("claude", "/project").unwrap();
            assert_eq!(
                ids(&store),
                HashSet::from([
                    ("claude".to_owned(), "live".to_owned()),
                    ("codex".to_owned(), "old".to_owned()),
                ]),
                "a session without a provider id is left out"
            );
        }
    }

    #[test]
    fn includes_ask_sessions_on_both_backends() {
        for store in [
            Store::open(Path::new(":memory:")).unwrap(),
            Store::open_gespan(Path::new(":memory:")).unwrap(),
        ] {
            let id = ask(&store);
            store.provider_id(id, "asked").unwrap();
            assert!(store.sessions().unwrap().iter().all(|s| s.id != id));
            assert!(has(&store, "claude", "asked"));
        }
    }

    #[test]
    fn includes_hidden_legacy_remote_sessions_on_the_local_store() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("claude", "/project").unwrap();
        store.provider_id(session.id, "hidden").unwrap();
        store.hide_legacy_remote_sessions([session.id]).unwrap();
        assert!(store.sessions().unwrap().is_empty());
        assert!(has(&store, "claude", "hidden"));
    }

    #[test]
    fn includes_hidden_and_worker_sessions_on_the_shared_store() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let pilot = store.create("claude", "/project").unwrap();
        let original = store.persisted_session_id(pilot.id).unwrap();
        store
            .conn()
            .execute(
                "UPDATE sessions SET sidebar_hidden=1,provider_session_id='pilot-thread' WHERE id=?1",
                [&original],
            )
            .unwrap();
        let worker = store.create("codex", "/project").unwrap();
        store.provider_id(worker.id, "worker-thread").unwrap();
        let original = store.persisted_session_id(worker.id).unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO orchestration_workers VALUES(?1,'lead')",
                [&original],
            )
            .unwrap();
        assert!(store.sessions().unwrap().is_empty());
        assert!(has(&store, "claude", "pilot-thread"));
        assert!(has(&store, "codex", "worker-thread"));
    }
}
