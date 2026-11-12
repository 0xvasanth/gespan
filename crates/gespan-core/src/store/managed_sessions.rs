//! Desktop metadata for automation-created sessions.
use super::Store;
use serde_json::Value;
use std::collections::HashMap;

impl Store {
    pub fn set_automation_id(&self, id: i64, automation: &str) -> Result<(), String> {
        if self.shared.is_some() {
            self.shared_patch(id, &[("automation_id", Value::String(automation.into()))])
        } else {
            self.save_setting(&format!("automation:{id}"), automation)
        }
    }

    pub fn automation_session_ids(&self) -> Result<HashMap<i64, String>, String> {
        if self.shared.is_none() {
            return Ok(self
                .settings_with_prefix("automation:")?
                .into_iter()
                .collect());
        }
        self.sql()?
            .query(
                "SELECT id,automation_id FROM sessions WHERE automation_id IS NOT NULL",
                super::db::args![],
            )?
            .iter()
            .map(|row| {
                let (id, automation) = (row.get::<String>(0)?, row.get::<String>(1)?);
                Ok((self.shared_register(&id)?, automation))
            })
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
            .conn
            .query_row(
                "SELECT automation_id FROM sessions WHERE id=?1",
                [persisted],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "automation-one");
    }
}
