//! Desktop metadata for automation-created sessions.
use super::schema::SessionRecord;
use super::Store;
use serde_json::Value;
use std::collections::HashMap;

impl Store {
    pub fn set_automation_id(&self, id: i64, automation: &str) -> Result<(), String> {
        self.shared_patch(id, &[("automation_id", Value::String(automation.into()))])
    }

    pub fn automation_session_ids(&self) -> Result<HashMap<i64, String>, String> {
        let rows = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter(session.automation_id().is_some())
                .select((session.id(), session.automation_id()))
                .exec(&mut db)
                .await
        })?;
        rows.into_iter()
            .filter_map(|(id, automation)| Some((id, automation?)))
            .map(|(id, automation)| Ok((self.shared_register(&id)?, automation)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn automation_metadata_uses_the_original_desktop_session_identity() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        store
            .set_automation_id(session.id, "automation-one")
            .unwrap();
        assert_eq!(
            store
                .automation_session_ids()
                .unwrap()
                .get(&session.id)
                .map(String::as_str),
            Some("automation-one")
        );
        let persisted = store.persisted_session_id(session.id).unwrap();
        let raw: String = store
            .conn()
            .query_row(
                "SELECT automation_id FROM sessions WHERE id=?1",
                [persisted],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "automation-one");
    }
}
