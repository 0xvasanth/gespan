use crate::db::{args, err, query, with_db, with_tx, Conn, Ex};
use crate::models::CommentRow;
use crate::now_millis;
use crate::ops::{get_card_in, log_edit, new_id, touch};
use serde::Serialize;

const MAX_COMMENT: usize = 20_000;
/// Who wrote a comment. Besides these, `agent:<id>` names the agent session
/// that wrote it (write-back marks its comments `agent:<session_key>`); see
/// [`is_comment_author`]. Readers label every `agent*` author "Agent".
pub const COMMENT_AUTHORS: [&str; 2] = ["user", "agent"];
const AGENT_PREFIX: &str = "agent:";

/// Whether `author` may write a comment: one of [`COMMENT_AUTHORS`], or
/// `agent:<session_key>` where the key is one a session can have (not blank,
/// at most 500 bytes, no control characters; colons allowed, as in
/// `remote:{env}:{session}`).
pub fn is_comment_author(author: &str) -> bool {
    if COMMENT_AUTHORS.contains(&author) {
        return true;
    }
    author
        .strip_prefix(AGENT_PREFIX)
        .is_some_and(|key| crate::settings::check_session_key(key).is_ok())
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

impl From<CommentRow> for Comment {
    fn from(row: CommentRow) -> Self {
        Comment {
            id: row.id,
            card_id: row.card_id,
            body: row.body,
            author: row.author,
            created_at: row.created_at,
            edited_at: row.edited_at,
        }
    }
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

pub(crate) async fn list_comments_in(
    ex: &mut Ex<'_>,
    card_id: &str,
) -> Result<Vec<Comment>, String> {
    // Raw SQL: comments made in the same millisecond keep insertion order
    // (`rowid`), which Toasty 0.11 cannot order by.
    query(
        ex,
        "SELECT id, card_id, body, author, created_at, edited_at FROM board_comments
         WHERE card_id = ?1 ORDER BY created_at, rowid",
        args![card_id],
    )
    .await?
    .iter()
    .map(|r| {
        Ok(Comment {
            id: r.get(0)?,
            card_id: r.get(1)?,
            body: r.get(2)?,
            author: r.get(3)?,
            created_at: r.get(4)?,
            edited_at: r.get(5)?,
        })
    })
    .collect()
}

pub fn list_comments(conn: &Conn, card_id: &str) -> Result<Vec<Comment>, String> {
    with_db!(conn, |ex| list_comments_in(ex, card_id).await)
}

pub fn add_comment(
    conn: &Conn,
    card_id: &str,
    body: &str,
    author: &str,
) -> Result<Comment, String> {
    if !is_comment_author(author) {
        return Err(format!("Unknown comment author: {author}"));
    }
    let body = check_body(body)?;
    with_tx!(conn, |ex| add_comment_in(ex, card_id, body, author).await)
}

/// [`add_comment`] inside the caller's write transaction; `author` is not
/// checked here.
pub(crate) async fn add_comment_in(
    ex: &mut Ex<'_>,
    card_id: &str,
    body: String,
    author: &str,
) -> Result<Comment, String> {
    let card = get_card_in(ex, card_id).await?;
    let comment: Comment = CommentRow::create()
        .id(new_id())
        .card_id(card_id)
        .body(body)
        .author(author)
        .created_at(now_millis())
        .edited_at(None::<i64>)
        .exec(ex)
        .await
        .map_err(err)?
        .into();
    touch(ex, card_id).await?;
    log_edit(ex, &card.board_id, card_id, "comment added").await?;
    Ok(comment)
}

/// Returns the owning board id.
pub fn edit_comment(conn: &Conn, comment_id: &str, body: &str) -> Result<String, String> {
    let body = check_body(body)?;
    with_tx!(conn, |ex| {
        let Some(comment) = CommentRow::filter_by_id(comment_id)
            .first()
            .exec(ex)
            .await
            .map_err(err)?
        else {
            return Err("Comment not found".into());
        };
        let card = get_card_in(ex, &comment.card_id).await?;
        if comment.body == body {
            return Ok(card.board_id);
        }
        CommentRow::filter_by_id(comment_id)
            .update()
            .body(body)
            .edited_at(Some(now_millis()))
            .exec(ex)
            .await
            .map_err(err)?;
        touch(ex, &comment.card_id).await?;
        log_edit(ex, &card.board_id, &comment.card_id, "comment edited").await?;
        Ok(card.board_id)
    })
}
