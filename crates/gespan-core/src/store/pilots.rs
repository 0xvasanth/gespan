use super::db::args;
use super::schema::{NoteRecord, SessionRecord};
use super::{Session, Store};

impl Store {
    /// Look up an existing Pilot conversation independently of the sidebar.
    /// Hidden sessions are valid conversations and must never be replaced.
    pub fn pilot_session(&self, id: i64) -> Result<Option<Session>, String> {
        let original = self.persisted_session_id(id)?;
        let row = self.orm()?.run(|mut db| async move {
            let session = SessionRecord::fields();
            SessionRecord::filter_by_id(original)
                .select((
                    (session.title(), session.harness(), session.cwd()),
                    (session.provider_session_id(), session.updated_at()),
                    (
                        session.has_user_message(),
                        session.archived(),
                        session.pinned(),
                    ),
                    session.linked_work_item_json(),
                ))
                .exec(&mut db)
                .await
        })?;
        Ok(row.into_iter().next().map(
            |(
                (title, provider, cwd),
                (provider_id, updated_at),
                (has_messages, archived, pinned),
                work_item,
            )| Session {
                id,
                title,
                provider,
                cwd,
                provider_id,
                updated_at: updated_at.max(0) as u64,
                has_messages,
                archived,
                pinned,
                created_at: 0,
                work_item: work_item.and_then(|raw| serde_json::from_str(&raw).ok()),
            },
        ))
    }
    /// Cheap freshness token; the blocks themselves are loaded only on change.
    pub fn pilot_work_revision(&self, id: i64) -> Result<Option<(u64, u64, u64)>, String> {
        let original = self.persisted_session_id(id)?;
        let (updated, notes) = self.orm()?.run(|mut db| async move {
            let updated = SessionRecord::filter_by_id(original)
                .select(SessionRecord::fields().updated_at())
                .exec(&mut db)
                .await?
                .into_iter()
                .next();
            let note = NoteRecord::fields();
            let notes = NoteRecord::all()
                .order_by(note.updated_at().desc())
                .limit(1)
                .select(note.updated_at())
                .exec(&mut db)
                .await?
                .into_iter()
                .next()
                .unwrap_or(0);
            Ok((updated, notes))
        })?;
        let updated = updated.ok_or("Query returned no rows")?;
        // Commits by other connections, as seen by this store's one.
        let version: i64 = self.sql()?.value("PRAGMA data_version", args![])?;
        Ok(Some((
            updated.max(0) as u64,
            notes.max(0) as u64,
            version.max(0) as u64,
        )))
    }
    /// Read the exact note/artifact referenced by a card, including its HTML body.
    pub fn pilot_document(&self, id: &str) -> Result<Option<crate::pilot::Document>, String> {
        self.read_pilot_document(id, true)
    }
    fn read_pilot_document(
        &self,
        id: &str,
        body: bool,
    ) -> Result<Option<crate::pilot::Document>, String> {
        let id = id.to_owned();
        let row = self.orm()?.run(|mut db| async move {
            let note = NoteRecord::fields();
            let found = NoteRecord::filter_by_id(&id)
                .select((
                    note.artifact_kind(),
                    note.title(),
                    note.created_at(),
                    note.updated_at(),
                ))
                .exec(&mut db)
                .await?
                .into_iter()
                .next();
            let Some(found) = found else {
                return Ok(None);
            };
            // The body is read only when asked for: it can be a large page.
            let text = if body {
                NoteRecord::filter_by_id(&id)
                    .select(note.body())
                    .exec(&mut db)
                    .await?
                    .into_iter()
                    .next()
                    .unwrap_or_default()
            } else {
                String::new()
            };
            Ok(Some((id, found, text)))
        })?;
        Ok(row.map(
            |(id, (kind, title, created_at, updated_at), body)| crate::pilot::Document {
                id,
                kind: kind.unwrap_or_else(|| "document".into()),
                title,
                summary: String::new(),
                body,
                created_at: created_at.max(0) as u64,
                updated_at: updated_at.max(0) as u64,
            },
        ))
    }
    /// Metadata comes from the saved conversation, in both indexed Pilot history
    /// and older sessions.blocks_json. No provider rollout is opened here.
    pub fn pilot_work(&self, id: i64) -> Result<Option<crate::pilot::Work>, String> {
        let original = self.persisted_session_id(id)?;
        let indexed = self.shared_indexed(&original)?;
        // json1: Toasty 0.11 cannot read fields inside the JSON blocks, so
        // these stay raw. Return only references and confirmed app receipts from SQLite. A Pilot
        // can have megabytes of prose/tools; none of that crosses this boundary.
        let source = if indexed {
            "SELECT block_json AS block,seq FROM pilot_blocks WHERE session_id=?1"
        } else {
            "SELECT value AS block,key AS seq FROM sessions,json_each(sessions.blocks_json) WHERE sessions.id=?1"
        };
        let query = format!(
            r#"SELECT json_object('id',json_extract(block,'$.id'),'role',json_extract(block,'$.role'),
            'artifactCards',json_extract(block,'$.artifactCards'),'pilotSpawnedSessions',json_extract(block,'$.pilotSpawnedSessions'),
            'text',CASE WHEN json_extract(block,'$.role')='tool' THEN json_extract(block,'$.text') ELSE '' END,
            'tool',CASE WHEN json_extract(block,'$.role')='tool' THEN json_extract(block,'$.tool') ELSE NULL END)
            FROM ({source}) WHERE json_type(block,'$.artifactCards')='array' OR json_type(block,'$.pilotSpawnedSessions')='array'
            OR (json_extract(block,'$.role')='tool' AND json_extract(block,'$.tool.status') IN ('completed','success')
                AND COALESCE(json_extract(block,'$.tool.preview.title'),json_extract(block,'$.tool.title'),json_extract(block,'$.text'),'') LIKE '%gespan%app%sessions.%') ORDER BY seq"#
        );
        let mut blocks = Vec::new();
        for row in self.sql()?.query(&query, args![&original])? {
            blocks.push(
                serde_json::from_str::<serde_json::Value>(&row.get::<String>(0)?)
                    .map_err(|e| e.to_string())?,
            );
        }
        let mut documents = Vec::new();
        let mut launches = Vec::new();
        for block in &blocks {
            for card in crate::pilot::work::document_cards(block) {
                let id = card["documentId"].as_str().unwrap();
                let mut document =
                    self.read_pilot_document(id, false)?
                        .unwrap_or(crate::pilot::Document {
                            id: id.into(),
                            kind: card["kind"].as_str().unwrap().into(),
                            title: card["title"].as_str().unwrap().into(),
                            summary: String::new(),
                            body: String::new(),
                            created_at: 0,
                            updated_at: 0,
                        });
                document.summary = card["summary"].as_str().unwrap_or_default().into();
                if let Some(old) = documents
                    .iter_mut()
                    .find(|d: &&mut crate::pilot::Document| d.id == id)
                {
                    *old = document;
                } else {
                    documents.push(document);
                }
            }
        }
        for (index, values) in crate::pilot::work::block_launches(&blocks)
            .into_iter()
            .enumerate()
        {
            for value in values {
                let external = value["sessionId"].as_str().unwrap();
                // Retain deleted references too: unavailable filtering needs them.
                let session_id = self.shared_register(external)?;
                if launches
                    .iter()
                    .any(|l: &crate::pilot::Launch| l.session_id == session_id)
                {
                    continue;
                }
                launches.push(crate::pilot::Launch {
                    session_id,
                    cwd: value["cwd"].as_str().unwrap().into(),
                    title: value["title"].as_str().unwrap().into(),
                    harness: value["harness"].as_str().unwrap().into(),
                    model: value["model"].as_str().map(str::to_owned),
                    origin_turn: index as u64,
                    notify_on_complete: false,
                    completed: true,
                    dismissed: false,
                });
            }
        }
        Ok(Some((documents, launches)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn hidden_pilot_summary_preserves_original_session_and_provider() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("claude", "/project").unwrap();
        let original = store.persisted_session_id(session.id).unwrap();
        store.conn().execute(
            "UPDATE sessions SET sidebar_hidden=1,provider_session_id='existing-thread' WHERE id=?1",
            [&original],
        ).unwrap();
        assert!(store.sessions().unwrap().is_empty());
        let loaded = store.pilot_session(session.id).unwrap().unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.provider, "claude");
        assert_eq!(loaded.provider_id.as_deref(), Some("existing-thread"));
        assert_eq!(store.persisted_session_id(loaded.id).unwrap(), original);
        assert_eq!(
            store
                .conn()
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn desktop_work_uses_saved_block_references_and_note_bodies_without_mutation() {
        use serde_json::json;
        for indexed in [false, true] {
            let store = Store::open_gespan(Path::new(":memory:")).unwrap();
            let pilot = store.create("codex", "/pilot").unwrap();
            let child = store.create("claude", "/moved-project").unwrap();
            let original = store.persisted_session_id(pilot.id).unwrap();
            let child_id = store.persisted_session_id(child.id).unwrap();
            let blocks = json!([
                {"id":"user","role":"user","text":"Write it","artifactCards":[{"id":"existing-artifact","kind":"html","title":"Original title","summary":"Overview"}],"pilotSpawnedSessions":[{"sessionId":child_id,"cwd":"/old-project","title":"Launch title","harness":"claude","model":"opus"}]},
                {"id":"assistant","role":"assistant","text":"Done"}
            ]);
            store.conn().execute("INSERT INTO notes(id,slug,title,body,tags_json,slug_pending,created_at,updated_at,content_kind,artifact_kind) VALUES('existing-artifact','existing-artifact','Saved title','<h1>Actual HTML</h1>','[]',0,10,20,'artifact','html')", []).unwrap();
            if indexed {
                store
                    .conn()
                    .execute("INSERT INTO pilot_transcripts VALUES(?1)", [&original])
                    .unwrap();
                for (seq, block) in blocks.as_array().unwrap().iter().enumerate() {
                    store
                        .conn()
                        .execute(
                            "INSERT INTO pilot_blocks(session_id,seq,block_id,block_json,search_text,is_user,is_draft)
                             VALUES(?1,?2,?3,?4,'',?5,0)",
                            rusqlite::params![
                                original,
                                i64::try_from(seq).unwrap(),
                                block["id"].as_str().unwrap(),
                                block.to_string(),
                                block["role"] == "user"
                            ],
                        )
                        .unwrap();
                }
            } else {
                store
                    .conn()
                    .execute(
                        "UPDATE sessions SET blocks_json=?2 WHERE id=?1",
                        rusqlite::params![original, blocks.to_string()],
                    )
                    .unwrap();
            }
            let before = store.shared_blocks(pilot.id).unwrap();
            let (documents, launches) = store.pilot_work(pilot.id).unwrap().unwrap();
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].title, "Saved title");
            assert_eq!(documents[0].kind, "html");
            assert!(documents[0].body.is_empty());
            assert_eq!(
                store
                    .pilot_document("existing-artifact")
                    .unwrap()
                    .unwrap()
                    .body,
                "<h1>Actual HTML</h1>"
            );
            assert_eq!(documents[0].summary, "Overview");
            assert_eq!(launches.len(), 1);
            assert_eq!(launches[0].session_id, child.id);
            assert_eq!(
                store
                    .pilot_session(launches[0].session_id)
                    .unwrap()
                    .unwrap()
                    .cwd,
                "/moved-project"
            );
            assert_eq!(store.shared_blocks(pilot.id).unwrap(), before);
            assert!(store.pilot_work_revision(pilot.id).unwrap().is_some());
        }
    }
}
