//! Task fields beyond title and body: priority, due date, projects, labels,
//! subtasks and comments. They live in the shared v3 tables (see
//! `gespan-board-schema`), keyed to `board_cards`, so a card deleted in either
//! app takes them with it. Only Native writes them so far.
use super::{
    clean_text, db, get_card, log_activity, new_id, now_millis, validate_id, write_tx, Board,
    NewActivity, Service,
};
use gespan_board_schema::COMMENT_AUTHOR_USER;
use rusqlite::{params, OptionalExtension};
use std::collections::{BTreeSet, HashMap};

const MAX_LABEL: usize = 40;
const MAX_SUBTASK: usize = 500;
const MAX_COMMENT: usize = 20_000;
pub use gespan_board_schema::PRIORITIES;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardExtras {
    pub priority: u8,
    pub due_at: Option<i64>,
    pub projects: Vec<String>,
    /// Label ids.
    pub labels: Vec<String>,
    pub subtasks_done: usize,
    pub subtasks_total: usize,
    pub comments: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subtask {
    pub id: String,
    pub card_id: String,
    pub title: String,
    pub done: bool,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    pub card_id: String,
    pub body: String,
    pub created_at: i64,
}

/// One `edited` row saying which native field changed.
fn log_edit(
    tx: &rusqlite::Transaction<'_>,
    board_id: &str,
    card_id: &str,
    detail: &str,
) -> Result<(), String> {
    log_activity(
        tx,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind: "edited",
            actor: "user",
            from_column: None,
            to_column: None,
            detail: Some(detail),
        },
    )
}

/// A card's stored priority and due date; a card without a meta row has neither.
fn meta(tx: &rusqlite::Transaction<'_>, card_id: &str) -> Result<(u8, Option<i64>), String> {
    let row: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT priority, due_at FROM board_card_meta WHERE card_id = ?1",
            [card_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db)?;
    Ok(row.map_or((0, None), |(priority, due)| {
        (priority.clamp(0, 4) as u8, due)
    }))
}

/// The values one card holds in a native set table, such as its projects.
fn card_set(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    card_id: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = tx.prepare(sql).map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| r.get::<_, String>(0))
        .map_err(db)?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(db);
    rows
}

/// A subtask's card, title and done state, if the subtask still exists.
fn subtask_row(
    tx: &rusqlite::Transaction<'_>,
    subtask_id: &str,
) -> Result<Option<(String, String, bool)>, String> {
    tx.query_row(
        "SELECT card_id, title, done != 0 FROM board_subtasks WHERE id = ?1",
        [subtask_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(db)
}

fn touch(tx: &rusqlite::Transaction<'_>, card_id: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
        params![now_millis(), card_id],
    )
    .map(drop)
    .map_err(db)
}

/// The local calendar day of a due date, as the user picked it, for the log.
fn day_label(at: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| at.to_string())
}

impl Service {
    /// The one board the native app shows: the oldest, made on first use.
    pub fn default_board(&mut self) -> Result<Board, String> {
        match self.list()?.into_iter().next() {
            Some(board) => Ok(board),
            None => self.create("Board"),
        }
    }

    /// Every card's extras on a board, keyed by card id. Cards with none are absent.
    pub fn extras(&mut self, board_id: &str) -> Result<HashMap<String, CardExtras>, String> {
        validate_id(board_id, "board")?;
        let mut extras: HashMap<String, CardExtras> = HashMap::new();
        let conn = &self.conn;
        let scope = "SELECT id FROM board_cards WHERE board_id = ?1";
        let mut meta = conn
            .prepare(&format!(
                "SELECT card_id, priority, due_at FROM board_card_meta WHERE card_id IN ({scope})"
            ))
            .map_err(db)?;
        for row in meta
            .query_map([board_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(db)?
        {
            let (card, priority, due_at) = row.map_err(db)?;
            let entry = extras.entry(card).or_default();
            entry.priority = priority.clamp(0, 4) as u8;
            entry.due_at = due_at;
        }
        let mut projects = conn
            .prepare(&format!(
                "SELECT card_id, project FROM board_card_projects WHERE card_id IN ({scope}) ORDER BY project"
            ))
            .map_err(db)?;
        for row in projects
            .query_map([board_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db)?
        {
            let (card, project) = row.map_err(db)?;
            extras.entry(card).or_default().projects.push(project);
        }
        let mut labels = conn
            .prepare(&format!(
                "SELECT cl.card_id, cl.label_id FROM board_card_labels cl
                 JOIN board_labels l ON l.id = cl.label_id
                 WHERE cl.card_id IN ({scope}) ORDER BY lower(l.name)"
            ))
            .map_err(db)?;
        for row in labels
            .query_map([board_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db)?
        {
            let (card, label) = row.map_err(db)?;
            extras.entry(card).or_default().labels.push(label);
        }
        let mut subtasks = conn
            .prepare(&format!(
                "SELECT card_id, SUM(done), COUNT(*) FROM board_subtasks
                 WHERE card_id IN ({scope}) GROUP BY card_id"
            ))
            .map_err(db)?;
        for row in subtasks
            .query_map([board_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(db)?
        {
            let (card, done, total) = row.map_err(db)?;
            let entry = extras.entry(card).or_default();
            entry.subtasks_done = done.max(0) as usize;
            entry.subtasks_total = total.max(0) as usize;
        }
        let mut comments = conn
            .prepare(&format!(
                "SELECT card_id, COUNT(*) FROM board_comments
                 WHERE card_id IN ({scope}) GROUP BY card_id"
            ))
            .map_err(db)?;
        for row in comments
            .query_map([board_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(db)?
        {
            let (card, count) = row.map_err(db)?;
            extras.entry(card).or_default().comments = count.max(0) as usize;
        }
        Ok(extras)
    }

    pub fn set_priority(&mut self, card_id: &str, priority: u8) -> Result<(), String> {
        validate_id(card_id, "card")?;
        let Some(name) = PRIORITIES.get(priority as usize) else {
            return Err(format!("Unknown priority: {priority}"));
        };
        let tx = write_tx(&self.conn)?;
        let card = get_card(&tx, card_id)?;
        if meta(&tx, card_id)?.0 == priority {
            return Ok(());
        }
        tx.execute(
            "INSERT INTO board_card_meta (card_id, priority) VALUES (?1, ?2)
             ON CONFLICT(card_id) DO UPDATE SET priority = excluded.priority",
            params![card_id, priority],
        )
        .map_err(db)?;
        touch(&tx, card_id)?;
        log_edit(&tx, &card.board_id, card_id, &format!("priority: {name}"))?;
        tx.commit().map_err(db)
    }

    pub fn set_due(&mut self, card_id: &str, due_at: Option<i64>) -> Result<(), String> {
        validate_id(card_id, "card")?;
        let tx = write_tx(&self.conn)?;
        let card = get_card(&tx, card_id)?;
        if meta(&tx, card_id)?.1 == due_at {
            return Ok(());
        }
        tx.execute(
            "INSERT INTO board_card_meta (card_id, due_at) VALUES (?1, ?2)
             ON CONFLICT(card_id) DO UPDATE SET due_at = excluded.due_at",
            params![card_id, due_at],
        )
        .map_err(db)?;
        touch(&tx, card_id)?;
        let detail = match due_at {
            Some(at) => format!("due: {}", day_label(at)),
            None => "due: none".into(),
        };
        log_edit(&tx, &card.board_id, card_id, &detail)?;
        tx.commit().map_err(db)
    }

    pub fn set_projects(&mut self, card_id: &str, projects: &[String]) -> Result<(), String> {
        validate_id(card_id, "card")?;
        let projects: BTreeSet<&str> = projects
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        let tx = write_tx(&self.conn)?;
        let card = get_card(&tx, card_id)?;
        let current = card_set(
            &tx,
            "SELECT project FROM board_card_projects WHERE card_id = ?1",
            card_id,
        )?;
        if current
            .iter()
            .map(String::as_str)
            .eq(projects.iter().copied())
        {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM board_card_projects WHERE card_id = ?1",
            [card_id],
        )
        .map_err(db)?;
        for project in &projects {
            tx.execute(
                "INSERT INTO board_card_projects (card_id, project) VALUES (?1, ?2)",
                params![card_id, project],
            )
            .map_err(db)?;
        }
        touch(&tx, card_id)?;
        log_edit(
            &tx,
            &card.board_id,
            card_id,
            &format!("projects: {}", projects.len()),
        )?;
        tx.commit().map_err(db)
    }

    pub fn labels(&mut self, board_id: &str) -> Result<Vec<Label>, String> {
        validate_id(board_id, "board")?;
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, board_id, name, color FROM board_labels
                 WHERE board_id = ?1 ORDER BY lower(name)",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([board_id], |r| {
                Ok(Label {
                    id: r.get(0)?,
                    board_id: r.get(1)?,
                    name: r.get(2)?,
                    color: r.get(3)?,
                })
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db);
        rows
    }

    pub fn create_label(
        &mut self,
        board_id: &str,
        name: &str,
        color: &str,
    ) -> Result<Label, String> {
        validate_id(board_id, "board")?;
        let name = clean_text(name, "Label name", MAX_LABEL)?;
        if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Label colour must be six hex digits".into());
        }
        let tx = write_tx(&self.conn)?;
        // SQLite's lower() folds only ASCII, so compare names in Rust too.
        let folded = name.to_lowercase();
        let taken = {
            let mut statement = tx
                .prepare("SELECT name FROM board_labels WHERE board_id = ?1")
                .map_err(db)?;
            let names = statement
                .query_map([board_id], |r| r.get::<_, String>(0))
                .map_err(db)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db)?;
            names
                .iter()
                .any(|existing| existing.to_lowercase() == folded)
        };
        if taken {
            return Err(format!("A label named {name} already exists"));
        }
        let label = Label {
            id: new_id(),
            board_id: board_id.to_string(),
            name,
            color: color.to_string(),
        };
        tx.execute(
            "INSERT INTO board_labels (id, board_id, name, color, position)
             SELECT ?1, ?2, ?3, ?4, COALESCE(MAX(position) + 1, 0)
             FROM board_labels WHERE board_id = ?2",
            params![label.id, label.board_id, label.name, label.color],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(label)
    }

    pub fn set_labels(&mut self, card_id: &str, label_ids: &[String]) -> Result<(), String> {
        validate_id(card_id, "card")?;
        let ids: BTreeSet<&str> = label_ids.iter().map(String::as_str).collect();
        let tx = write_tx(&self.conn)?;
        let card = get_card(&tx, card_id)?;
        let current = card_set(
            &tx,
            "SELECT label_id FROM board_card_labels WHERE card_id = ?1",
            card_id,
        )?;
        if current.iter().map(String::as_str).eq(ids.iter().copied()) {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM board_card_labels WHERE card_id = ?1",
            [card_id],
        )
        .map_err(db)?;
        let mut names = Vec::new();
        for id in &ids {
            let name: Option<String> = tx
                .query_row(
                    "SELECT name FROM board_labels WHERE id = ?1 AND board_id = ?2",
                    params![id, card.board_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?;
            let Some(name) = name else {
                return Err("Label not on this board".into());
            };
            tx.execute(
                "INSERT INTO board_card_labels (card_id, label_id) VALUES (?1, ?2)",
                params![card_id, id],
            )
            .map_err(db)?;
            names.push(name);
        }
        names.sort_by_key(|name| name.to_lowercase());
        touch(&tx, card_id)?;
        let detail = if names.is_empty() {
            "labels: none".to_string()
        } else {
            format!("labels: {}", names.join(", "))
        };
        log_edit(&tx, &card.board_id, card_id, &detail)?;
        tx.commit().map_err(db)
    }

    pub fn subtasks(&mut self, card_id: &str) -> Result<Vec<Subtask>, String> {
        validate_id(card_id, "card")?;
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, card_id, title, done, position FROM board_subtasks
                 WHERE card_id = ?1 ORDER BY position, rowid",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([card_id], |r| {
                Ok(Subtask {
                    id: r.get(0)?,
                    card_id: r.get(1)?,
                    title: r.get(2)?,
                    done: r.get::<_, i64>(3)? != 0,
                    position: r.get(4)?,
                })
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db);
        rows
    }

    pub fn add_subtask(&mut self, card_id: &str, title: &str) -> Result<Subtask, String> {
        validate_id(card_id, "card")?;
        let title = clean_text(title, "Subtask", MAX_SUBTASK)?;
        let tx = write_tx(&self.conn)?;
        let card = get_card(&tx, card_id)?;
        let position: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM board_subtasks WHERE card_id = ?1",
                [card_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        let subtask = Subtask {
            id: new_id(),
            card_id: card_id.to_string(),
            title,
            done: false,
            position,
        };
        tx.execute(
            "INSERT INTO board_subtasks (id, card_id, title, done, position)
             VALUES (?1, ?2, ?3, 0, ?4)",
            params![subtask.id, subtask.card_id, subtask.title, subtask.position],
        )
        .map_err(db)?;
        touch(&tx, card_id)?;
        log_edit(
            &tx,
            &card.board_id,
            card_id,
            &format!("subtask added: {}", subtask.title),
        )?;
        tx.commit().map_err(db)?;
        Ok(subtask)
    }

    pub fn toggle_subtask(&mut self, subtask_id: &str, done: bool) -> Result<(), String> {
        validate_id(subtask_id, "subtask")?;
        let tx = write_tx(&self.conn)?;
        let Some((card_id, title, was_done)) = subtask_row(&tx, subtask_id)? else {
            return Err("Subtask not found".into());
        };
        if was_done == done {
            return Ok(());
        }
        let card = get_card(&tx, &card_id)?;
        tx.execute(
            "UPDATE board_subtasks SET done = ?1 WHERE id = ?2",
            params![i64::from(done), subtask_id],
        )
        .map_err(db)?;
        touch(&tx, &card_id)?;
        let state = if done { "done" } else { "reopened" };
        log_edit(
            &tx,
            &card.board_id,
            &card_id,
            &format!("subtask {state}: {title}"),
        )?;
        tx.commit().map_err(db)
    }

    pub fn delete_subtask(&mut self, subtask_id: &str) -> Result<(), String> {
        validate_id(subtask_id, "subtask")?;
        let tx = write_tx(&self.conn)?;
        let Some((card_id, title, _)) = subtask_row(&tx, subtask_id)? else {
            return Ok(());
        };
        let card = get_card(&tx, &card_id)?;
        tx.execute("DELETE FROM board_subtasks WHERE id = ?1", [subtask_id])
            .map_err(db)?;
        // Close the gap so positions stay 0..n-1, as cards and columns do.
        let remaining: Vec<String> = {
            let mut statement = tx
                .prepare("SELECT id FROM board_subtasks WHERE card_id = ?1 ORDER BY position, rowid")
                .map_err(db)?;
            let ids = statement
                .query_map([&card_id], |r| r.get(0))
                .map_err(db)?
                .collect::<rusqlite::Result<_>>()
                .map_err(db)?;
            ids
        };
        let mut renumber = tx
            .prepare("UPDATE board_subtasks SET position = ?1 WHERE id = ?2 AND position <> ?1")
            .map_err(db)?;
        for (index, id) in remaining.iter().enumerate() {
            renumber.execute(params![index as i64, id]).map_err(db)?;
        }
        drop(renumber);
        touch(&tx, &card_id)?;
        log_edit(
            &tx,
            &card.board_id,
            &card_id,
            &format!("subtask removed: {title}"),
        )?;
        tx.commit().map_err(db)
    }

    pub fn comments(&mut self, card_id: &str) -> Result<Vec<Comment>, String> {
        validate_id(card_id, "card")?;
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, card_id, body, created_at FROM board_comments
                 WHERE card_id = ?1 ORDER BY created_at, rowid",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([card_id], |r| {
                Ok(Comment {
                    id: r.get(0)?,
                    card_id: r.get(1)?,
                    body: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db);
        rows
    }

    pub fn add_comment(&mut self, card_id: &str, body: &str) -> Result<Comment, String> {
        validate_id(card_id, "card")?;
        if body.trim().is_empty() {
            return Err("Comment is empty".into());
        }
        if body.len() > MAX_COMMENT {
            return Err("Comment is too long".into());
        }
        let tx = write_tx(&self.conn)?;
        let card = get_card(&tx, card_id)?;
        let comment = Comment {
            id: new_id(),
            card_id: card_id.to_string(),
            body: body.trim().to_string(),
            created_at: now_millis(),
        };
        tx.execute(
            "INSERT INTO board_comments (id, card_id, body, author, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                comment.id,
                comment.card_id,
                comment.body,
                COMMENT_AUTHOR_USER,
                comment.created_at
            ],
        )
        .map_err(db)?;
        touch(&tx, card_id)?;
        log_edit(&tx, &card.board_id, card_id, "comment added")?;
        tx.commit().map_err(db)?;
        Ok(comment)
    }
}

#[cfg(test)]
mod tests {
    use crate::board::Service;

    fn service() -> (tempfile::TempDir, Service, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let service = Service::open(&path).unwrap();
        (dir, service, path)
    }

    fn card(service: &mut Service) -> (String, String) {
        let board = service.default_board().unwrap();
        let backlog = service.get(&board.id).unwrap().columns[0].id.clone();
        let card = service.card_create(&backlog, "Task", "").unwrap();
        (board.id, card.id)
    }

    fn last_detail(service: &mut Service, board: &str, card: &str) -> Option<String> {
        service.activity(board, Some(card), 1).unwrap()[0]
            .detail
            .clone()
    }

    #[test]
    fn default_board_creates_one_board_once() {
        let (_dir, mut service, _) = service();
        let first = service.default_board().unwrap();
        let again = service.default_board().unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(service.list().unwrap().len(), 1);
        assert_eq!(service.get(&first.id).unwrap().columns.len(), 4);
    }

    #[test]
    fn default_board_is_the_oldest_of_several() {
        let (_dir, mut service, _) = service();
        // Alphabetical order differs from creation order.
        let oldest = service.create("Zeta").unwrap();
        service.create("Alpha").unwrap();
        service.create("Mid").unwrap();
        assert_eq!(service.default_board().unwrap().id, oldest.id);
        assert_eq!(service.list().unwrap().len(), 3);
    }

    #[test]
    fn priority_and_due_round_trip_and_log_an_edit() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        service.set_priority(&card, 3).unwrap();
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("priority: High")
        );
        service.set_due(&card, Some(1_793_404_800_000)).unwrap();
        let extras = service.extras(&board).unwrap();
        assert_eq!(extras[&card].priority, 3);
        assert_eq!(extras[&card].due_at, Some(1_793_404_800_000));
        service.set_due(&card, None).unwrap();
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("due: none")
        );
        assert_eq!(service.extras(&board).unwrap()[&card].due_at, None);
    }

    #[test]
    fn priority_above_urgent_is_rejected() {
        let (_dir, mut service, _) = service();
        let (_, card) = card(&mut service);
        assert!(service.set_priority(&card, 5).is_err());
    }

    #[test]
    fn set_projects_replaces_the_set_and_collapses_duplicates() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        service
            .set_projects(&card, &["/a".into(), "/b".into(), "/a".into()])
            .unwrap();
        let mut projects = service.extras(&board).unwrap()[&card].projects.clone();
        projects.sort();
        assert_eq!(projects, ["/a", "/b"]);
        service.set_projects(&card, &["/c".into()]).unwrap();
        assert_eq!(service.extras(&board).unwrap()[&card].projects, ["/c"]);
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("projects: 1")
        );
    }

    #[test]
    fn labels_are_unique_per_board_ignoring_case_and_attach_to_cards() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        assert!(service.create_label(&board, "  bug ", "4da3f5").is_err());
        let docs = service.create_label(&board, "Docs", "4da3f5").unwrap();
        // Case is folded beyond ASCII, and colours are plain hex.
        service.create_label(&board, "Ärger", "e0823d").unwrap();
        assert!(service.create_label(&board, "ärger", "e0823d").is_err());
        assert!(service.create_label(&board, "Red", "+12345").is_err());
        assert!(service.create_label(&board, "Red", "red").is_err());
        assert_eq!(service.labels(&board).unwrap().len(), 3);
        service
            .set_labels(&card, &[bug.id.clone(), docs.id.clone()])
            .unwrap();
        let mut labels = service.extras(&board).unwrap()[&card].labels.clone();
        labels.sort();
        let mut expected = vec![bug.id, docs.id];
        expected.sort();
        assert_eq!(labels, expected);
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("labels: Bug, Docs")
        );
    }

    #[test]
    fn subtasks_keep_order_toggle_and_count() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        let one = service.add_subtask(&card, " One ").unwrap();
        service.add_subtask(&card, "Two").unwrap();
        service.add_subtask(&card, "Three").unwrap();
        assert!(service.add_subtask(&card, "   ").is_err());
        service.toggle_subtask(&one.id, true).unwrap();
        let titles: Vec<String> = service
            .subtasks(&card)
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(titles, ["One", "Two", "Three"]);
        let extras = &service.extras(&board).unwrap()[&card];
        assert_eq!((extras.subtasks_done, extras.subtasks_total), (1, 3));
        service.delete_subtask(&one.id).unwrap();
        let extras = &service.extras(&board).unwrap()[&card];
        assert_eq!((extras.subtasks_done, extras.subtasks_total), (0, 2));
    }

    fn set_updated_at(path: &std::path::Path, card: &str, at: i64) {
        rusqlite::Connection::open(path)
            .unwrap()
            .execute(
                "UPDATE board_cards SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![at, card],
            )
            .unwrap();
    }

    fn updated_at(service: &mut Service, board: &str, card: &str) -> i64 {
        let detail = service.get(board).unwrap();
        detail
            .cards
            .iter()
            .find(|c| c.id == card)
            .unwrap()
            .updated_at
    }

    #[test]
    fn toggling_or_deleting_a_subtask_counts_as_a_change_to_its_card() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        let one = service.add_subtask(&card, "One").unwrap();
        set_updated_at(&path, &card, 1);
        service.toggle_subtask(&one.id, true).unwrap();
        assert!(updated_at(&mut service, &board, &card) > 1);
        set_updated_at(&path, &card, 1);
        service.delete_subtask(&one.id).unwrap();
        assert!(updated_at(&mut service, &board, &card) > 1);
        // Toggling to the state it already has is not a change.
        let two = service.add_subtask(&card, "Two").unwrap();
        service.toggle_subtask(&two.id, true).unwrap();
        set_updated_at(&path, &card, 1);
        service.toggle_subtask(&two.id, true).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
        // A subtask already gone is still not found for a toggle, and a
        // repeated delete stays a quiet no-op that touches nothing.
        set_updated_at(&path, &card, 1);
        assert_eq!(
            service.toggle_subtask(&one.id, false).unwrap_err(),
            "Subtask not found"
        );
        service.delete_subtask(&one.id).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
    }

    #[test]
    fn setting_a_field_to_the_value_it_already_has_writes_nothing() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        // A fresh card already has no priority, due date, projects or labels.
        let before = service.activity(&board, Some(&card), 100).unwrap().len();
        set_updated_at(&path, &card, 1);
        service.set_priority(&card, 0).unwrap();
        service.set_due(&card, None).unwrap();
        service.set_projects(&card, &[]).unwrap();
        service.set_labels(&card, &[]).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
        assert_eq!(
            service.activity(&board, Some(&card), 100).unwrap().len(),
            before
        );
        service.set_priority(&card, 2).unwrap();
        service.set_due(&card, Some(1_793_404_800_000)).unwrap();
        service
            .set_projects(&card, &["/b".into(), "/a".into()])
            .unwrap();
        service
            .set_labels(&card, std::slice::from_ref(&bug.id))
            .unwrap();
        let after = service.activity(&board, Some(&card), 100).unwrap().len();
        assert_eq!(after, before + 4);
        set_updated_at(&path, &card, 1);
        service.set_priority(&card, 2).unwrap();
        service.set_due(&card, Some(1_793_404_800_000)).unwrap();
        service
            .set_projects(&card, &["/a".into(), " /b ".into(), "/a".into()])
            .unwrap();
        service.set_labels(&card, &[bug.id]).unwrap();
        assert_eq!(updated_at(&mut service, &board, &card), 1);
        assert_eq!(
            service.activity(&board, Some(&card), 100).unwrap().len(),
            after
        );
        // A real change after the no-ops still writes.
        service.set_priority(&card, 3).unwrap();
        assert_eq!(
            last_detail(&mut service, &board, &card).as_deref(),
            Some("priority: High")
        );
    }

    #[test]
    fn a_database_error_while_attaching_a_label_is_not_reported_as_a_missing_label() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        assert_eq!(
            service
                .set_labels(&card, &["not-a-label".into()])
                .unwrap_err(),
            "Label not on this board"
        );
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE board_labels RENAME COLUMN name TO title;")
            .unwrap();
        let error = service.set_labels(&card, &[bug.id]).unwrap_err();
        assert_ne!(error, "Label not on this board");
        assert!(error.contains("no such column"), "{error}");
    }

    #[test]
    fn comments_come_back_oldest_first_and_count() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        service.add_comment(&card, "first").unwrap();
        service.add_comment(&card, "second").unwrap();
        assert!(service.add_comment(&card, "  ").is_err());
        let bodies: Vec<String> = service
            .comments(&card)
            .unwrap()
            .into_iter()
            .map(|c| c.body)
            .collect();
        assert_eq!(bodies, ["first", "second"]);
        assert_eq!(service.extras(&board).unwrap()[&card].comments, 2);
    }

    fn raw_strings(path: &std::path::Path, sql: &str, card: &str) -> Vec<String> {
        let conn = rusqlite::Connection::open(path).unwrap();
        let mut statement = conn.prepare(sql).unwrap();
        statement
            .query_map([card], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn native_v2_database_opens_with_every_value() {
        use gespan_board_schema::testing::{
            assert_native_v2_migrated, seed_native_v2, SEED_DUE_AT,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let seed = {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            seed_native_v2(&conn)
        };
        let mut service = Service::open(&path).unwrap();
        assert_native_v2_migrated(&rusqlite::Connection::open(&path).unwrap(), &seed);
        let a = &seed.card_ids[0];
        let extras = service.extras(&seed.board_id).unwrap();
        let extra = &extras[a];
        assert_eq!((extra.priority, extra.due_at), (4, Some(SEED_DUE_AT)));
        assert_eq!(extra.projects, ["/work/gespan", "/work/site"]);
        assert_eq!(extra.labels, seed.label_ids);
        assert_eq!((extra.subtasks_done, extra.subtasks_total), (1, 3));
        assert_eq!(extra.comments, 2);
        let subtasks: Vec<(String, i64)> = service
            .subtasks(a)
            .unwrap()
            .into_iter()
            .map(|s| (s.title, s.position))
            .collect();
        assert_eq!(
            subtasks,
            [("One".into(), 0), ("Two".into(), 1), ("Three".into(), 2)]
        );
        let labels: Vec<String> = service
            .labels(&seed.board_id)
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert_eq!(labels, ["Bug", "docs"]);
        // Reopening copies nothing twice.
        drop(service);
        Service::open(&path).unwrap();
        assert_native_v2_migrated(&rusqlite::Connection::open(&path).unwrap(), &seed);
    }

    /// Runs `change` and asserts it wrote `rows` activity rows for the card,
    /// the newest with `detail` when one is given.
    fn writes(
        service: &mut Service,
        board: &str,
        card: &str,
        rows: usize,
        detail: Option<&str>,
        change: impl FnOnce(&mut Service),
    ) {
        let before = service.activity(board, Some(card), 500).unwrap().len();
        change(service);
        let activity = service.activity(board, Some(card), 500).unwrap();
        assert_eq!(activity.len(), before + rows, "rows written for {detail:?}");
        if let Some(detail) = detail {
            assert_eq!(activity[0].kind, "edited");
            assert_eq!(activity[0].detail.as_deref(), Some(detail));
        }
    }

    #[test]
    fn each_field_change_writes_exactly_one_activity_row() {
        let (_dir, mut service, _) = service();
        let (board, card) = card(&mut service);
        let bug = service.create_label(&board, "Bug", "e5534b").unwrap();
        let (b, c) = (board.as_str(), card.as_str());
        let s = &mut service;
        writes(s, b, c, 1, Some("priority: Medium"), |s| s.set_priority(c, 2).unwrap());
        writes(s, b, c, 0, None, |s| s.set_priority(c, 2).unwrap());
        writes(s, b, c, 1, None, |s| s.set_due(c, Some(1_793_404_800_000)).unwrap());
        writes(s, b, c, 0, None, |s| s.set_due(c, Some(1_793_404_800_000)).unwrap());
        writes(s, b, c, 1, Some("projects: 1"), |s| {
            s.set_projects(c, &["/a".into()]).unwrap()
        });
        writes(s, b, c, 0, None, |s| s.set_projects(c, &["/a".into()]).unwrap());
        writes(s, b, c, 1, Some("labels: Bug"), |s| {
            s.set_labels(c, std::slice::from_ref(&bug.id)).unwrap()
        });
        writes(s, b, c, 0, None, |s| {
            s.set_labels(c, std::slice::from_ref(&bug.id)).unwrap()
        });
        let mut subtask = None;
        writes(s, b, c, 1, Some("subtask added: Write"), |s| {
            subtask = Some(s.add_subtask(c, "Write").unwrap().id)
        });
        let subtask = subtask.unwrap();
        writes(s, b, c, 1, Some("subtask done: Write"), |s| {
            s.toggle_subtask(&subtask, true).unwrap()
        });
        writes(s, b, c, 0, None, |s| s.toggle_subtask(&subtask, true).unwrap());
        writes(s, b, c, 1, Some("subtask reopened: Write"), |s| {
            s.toggle_subtask(&subtask, false).unwrap()
        });
        writes(s, b, c, 1, Some("comment added"), |s| {
            s.add_comment(c, "Looks good").unwrap();
        });
        writes(s, b, c, 1, None, |s| s.card_update(c, "Renamed", "").unwrap());
        writes(s, b, c, 0, None, |s| s.card_update(c, "Renamed", "").unwrap());
        writes(s, b, c, 1, Some("subtask removed: Write"), |s| {
            s.delete_subtask(&subtask).unwrap()
        });
        writes(s, b, c, 0, None, |s| s.delete_subtask(&subtask).unwrap());
    }

    #[test]
    fn deleting_a_subtask_keeps_positions_contiguous() {
        let (_dir, mut service, _) = service();
        let (_, card) = card(&mut service);
        service.add_subtask(&card, "One").unwrap();
        let two = service.add_subtask(&card, "Two").unwrap();
        service.add_subtask(&card, "Three").unwrap();
        service.delete_subtask(&two.id).unwrap();
        let positions = |service: &mut Service| -> Vec<(String, i64)> {
            service
                .subtasks(&card)
                .unwrap()
                .into_iter()
                .map(|s| (s.title, s.position))
                .collect()
        };
        assert_eq!(
            positions(&mut service),
            [("One".into(), 0), ("Three".into(), 1)]
        );
        service.add_subtask(&card, "Four").unwrap();
        assert_eq!(
            positions(&mut service),
            [("One".into(), 0), ("Three".into(), 1), ("Four".into(), 2)]
        );
    }

    #[test]
    fn new_labels_append_positions() {
        let (_dir, mut service, path) = service();
        let (board, _) = card(&mut service);
        service.create_label(&board, "Zeta", "e5534b").unwrap();
        service.create_label(&board, "Alpha", "4da3f5").unwrap();
        assert_eq!(
            raw_strings(
                &path,
                "SELECT name || ':' || position FROM board_labels WHERE board_id = ?1 ORDER BY position",
                &board,
            ),
            ["Zeta:0", "Alpha:1"]
        );
        // The picker still lists labels by name until the label manager lands.
        let names: Vec<String> = service
            .labels(&board)
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert_eq!(names, ["Alpha", "Zeta"]);
    }

    #[test]
    fn comments_are_written_as_the_user() {
        let (_dir, mut service, path) = service();
        let (_, card) = card(&mut service);
        service.add_comment(&card, "Hi").unwrap();
        assert_eq!(
            raw_strings(
                &path,
                "SELECT author || ':' || IFNULL(edited_at, '-') FROM board_comments WHERE card_id = ?1",
                &card,
            ),
            ["user:-"]
        );
    }

    /// Every v3 table holding one row per extra `with_every_extra` sets.
    const V3_CARD_TABLES: [&str; 6] = [
        "board_card_meta",
        "board_card_projects",
        "board_card_labels",
        "board_subtasks",
        "board_comments",
        "board_card_links",
    ];

    fn v3_rows(path: &std::path::Path, card: &str) -> Vec<(&'static str, i64)> {
        let conn = rusqlite::Connection::open(path).unwrap();
        V3_CARD_TABLES
        .into_iter()
        .map(|table| {
            let count = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE card_id = ?1"),
                    [card],
                    |row| row.get(0),
                )
                .unwrap();
            (table, count)
        })
        .collect()
    }

    fn with_every_extra(service: &mut Service, board: &str, card: &str) {
        service.set_priority(card, 2).unwrap();
        service.set_projects(card, &["/a".into()]).unwrap();
        let label = service.create_label(board, "Bug", "e5534b").unwrap();
        service.set_labels(card, &[label.id]).unwrap();
        service.add_subtask(card, "One").unwrap();
        service.add_comment(card, "Hi").unwrap();
        service.card_link(card, Some("session"), Some("s-1")).unwrap();
    }

    #[test]
    fn deleting_a_card_through_the_service_removes_its_v3_rows() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        with_every_extra(&mut service, &board, &card);
        assert!(v3_rows(&path, &card).iter().all(|(_, n)| *n == 1));
        service.card_delete(&card).unwrap();
        for (table, left) in v3_rows(&path, &card) {
            assert_eq!(left, 0, "{table}");
        }
    }

    #[test]
    fn deleting_a_card_with_a_raw_delete_removes_its_v3_rows() {
        let (_dir, mut service, path) = service();
        let (board, card) = card(&mut service);
        with_every_extra(&mut service, &board, &card);
        assert!(v3_rows(&path, &card).iter().all(|(_, n)| *n == 1));
        // The Tauri app deletes cards with a plain DELETE on its own connection.
        let tauri = rusqlite::Connection::open(&path).unwrap();
        tauri.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        tauri
            .execute("DELETE FROM board_cards WHERE id = ?1", [&card])
            .unwrap();
        for table in V3_CARD_TABLES {
            let left: i64 = tauri
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE card_id = ?1"),
                    [&card],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(left, 0, "{table}");
        }
    }
}
