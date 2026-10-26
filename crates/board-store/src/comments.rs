use crate::now_millis;
use crate::ops::{db, get_card, log_edit, new_id, touch, write_tx};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

const MAX_COMMENT: usize = 20_000;
/// Who wrote a comment. Besides these, `agent:<id>` names the agent session
/// that wrote it (write-back marks its comments `agent:<session_key>`); see
/// [`is_comment_author`]. Readers label every `agent*` author "Agent".
pub const COMMENT_AUTHORS: [&str; 2] = ["user", "agent"];
const AGENT_PREFIX: &str = "agent:";
const MAX_AUTHOR_ID: usize = 500;

/// Whether `author` may write a comment: one of [`COMMENT_AUTHORS`], or
/// `agent:<id>` where `<id>` is a valid id (letters, digits, `-`, `_`) of at
/// most 500 bytes.
pub fn is_comment_author(author: &str) -> bool {
    if COMMENT_AUTHORS.contains(&author) {
        return true;
    }
    author
        .strip_prefix(AGENT_PREFIX)
        .is_some_and(|id| id.len() <= MAX_AUTHOR_ID && crate::validate_id(id, "agent").is_ok())
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub card_id: String,
    pub body: String,
    pub author: String,
    pub created_at: i64,
    pub edited_at: Option<i64>,
}

fn check_body(body: &str) -> Result<String, String> {
    if body.trim().is_empty() {
        return Err("Comment is empty".into());
    }
    if body.len() > MAX_COMMENT {
        return Err("Comment is too long".into());
    }
    Ok(body.trim().to_string())
}

pub fn list_comments(conn: &Connection, card_id: &str) -> Result<Vec<Comment>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, card_id, body, author, created_at, edited_at FROM board_comments
             WHERE card_id = ?1 ORDER BY created_at, rowid",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| {
            Ok(Comment {
                id: r.get(0)?,
                card_id: r.get(1)?,
                body: r.get(2)?,
                author: r.get(3)?,
                created_at: r.get(4)?,
                edited_at: r.get(5)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

pub fn add_comment(
    conn: &Connection,
    card_id: &str,
    body: &str,
    author: &str,
) -> Result<Comment, String> {
    if !is_comment_author(author) {
        return Err(format!("Unknown comment author: {author}"));
    }
    let body = check_body(body)?;
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let comment = Comment {
        id: new_id(),
        card_id: card_id.to_string(),
        body,
        author: author.to_string(),
        created_at: now_millis(),
        edited_at: None,
    };
    tx.execute(
        "INSERT INTO board_comments (id, card_id, body, author, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            comment.id,
            comment.card_id,
            comment.body,
            comment.author,
            comment.created_at
        ],
    )
    .map_err(db)?;
    touch(&tx, card_id)?;
    log_edit(&tx, &card.board_id, card_id, "comment added")?;
    tx.commit().map_err(db)?;
    Ok(comment)
}

/// Returns the owning board id.
pub fn edit_comment(conn: &Connection, comment_id: &str, body: &str) -> Result<String, String> {
    let body = check_body(body)?;
    let tx = write_tx(conn)?;
    let row: Option<(String, String)> = tx
        .query_row(
            "SELECT card_id, body FROM board_comments WHERE id = ?1",
            [comment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db)?;
    let Some((card_id, current)) = row else {
        return Err("Comment not found".into());
    };
    let card = get_card(&tx, &card_id)?;
    if current == body {
        return Ok(card.board_id);
    }
    tx.execute(
        "UPDATE board_comments SET body = ?1, edited_at = ?2 WHERE id = ?3",
        params![body, now_millis(), comment_id],
    )
    .map_err(db)?;
    touch(&tx, &card_id)?;
    log_edit(&tx, &card.board_id, &card_id, "comment edited")?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}
