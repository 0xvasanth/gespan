use crate::db::{args, err, query, with_db, with_tx, Conn, Ex, Row};
use crate::models::SubtaskRow;
use crate::ops::{clean_text, get_card_in, log_activity, log_edit, new_id, touch, NewActivity};
use serde::Serialize;

const MAX_SUBTASK: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subtask {
    pub id: String,
    pub card_id: String,
    pub title: String,
    pub done: bool,
    pub position: i64,
}

impl From<SubtaskRow> for Subtask {
    fn from(row: SubtaskRow) -> Self {
        Subtask {
            id: row.id,
            card_id: row.card_id,
            title: row.title,
            done: row.done,
            position: row.position,
        }
    }
}

fn subtask_from_row(r: &Row) -> Result<Subtask, String> {
    Ok(Subtask {
        id: r.get(0)?,
        card_id: r.get(1)?,
        title: r.get(2)?,
        done: r.get(3)?,
        position: r.get(4)?,
    })
}

pub(crate) async fn get_subtask(
    ex: &mut Ex<'_>,
    subtask_id: &str,
) -> Result<Option<Subtask>, String> {
    Ok(SubtaskRow::filter_by_id(subtask_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(Subtask::from))
}

pub(crate) async fn list_subtasks_in(
    ex: &mut Ex<'_>,
    card_id: &str,
) -> Result<Vec<Subtask>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        "SELECT id, card_id, title, done, position FROM board_subtasks
         WHERE card_id = ?1 ORDER BY position, rowid",
        args![card_id],
    )
    .await?
    .iter()
    .map(subtask_from_row)
    .collect()
}

pub fn list_subtasks(conn: &Conn, card_id: &str) -> Result<Vec<Subtask>, String> {
    with_db!(conn, |ex| list_subtasks_in(ex, card_id).await)
}

pub fn add_subtask(conn: &Conn, card_id: &str, title: &str) -> Result<Subtask, String> {
    let title = clean_text(title, "Subtask", MAX_SUBTASK)?;
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let subtask = SubtaskRow::fields();
        let position = SubtaskRow::filter(subtask.card_id().eq(card_id))
            .order_by(subtask.position().desc())
            .select(subtask.position())
            .first()
            .exec(ex)
            .await
            .map_err(err)?
            .map_or(0, |last| last + 1);
        let subtask: Subtask = SubtaskRow::create()
            .id(new_id())
            .card_id(card_id)
            .title(title)
            .done(false)
            .position(position)
            .exec(ex)
            .await
            .map_err(err)?
            .into();
        touch(ex, card_id).await?;
        log_edit(
            ex,
            &card.board_id,
            card_id,
            &format!("subtask added: {}", subtask.title),
        )
        .await?;
        Ok(subtask)
    })
}

/// Returns the owning board id.
pub fn toggle_subtask(conn: &Conn, subtask_id: &str, done: bool) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let Some(subtask) = get_subtask(ex, subtask_id).await? else {
            return Err("Subtask not found".into());
        };
        toggle_subtask_in(ex, &subtask, done, "user").await
    })
}

/// Sets `subtask` done or not inside the caller's write transaction, logging
/// the change as `actor`. Setting the state it already has changes nothing.
/// Returns the owning board id.
pub(crate) async fn toggle_subtask_in(
    ex: &mut Ex<'_>,
    subtask: &Subtask,
    done: bool,
    actor: &str,
) -> Result<String, String> {
    let card = get_card_in(ex, &subtask.card_id).await?;
    if subtask.done == done {
        return Ok(card.board_id);
    }
    SubtaskRow::filter_by_id(&subtask.id)
        .update()
        .done(done)
        .exec(ex)
        .await
        .map_err(err)?;
    touch(ex, &card.id).await?;
    let verb = if done { "done" } else { "reopened" };
    log_activity(
        ex,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(&card.id),
            kind: "edited",
            actor,
            from_column: None,
            to_column: None,
            detail: Some(&format!("subtask {verb}: {}", subtask.title)),
        },
    )
    .await?;
    Ok(card.board_id)
}

/// Deleting a subtask that is already gone is a quiet no-op. Returns the
/// owning board id when there was something to delete.
pub fn delete_subtask(conn: &Conn, subtask_id: &str) -> Result<Option<String>, String> {
    with_tx!(conn, |ex| {
        let Some(subtask) = get_subtask(ex, subtask_id).await? else {
            return Ok(None);
        };
        let card = get_card_in(ex, &subtask.card_id).await?;
        SubtaskRow::filter_by_id(subtask_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        renumber_subtasks(ex, &card.id).await?;
        touch(ex, &card.id).await?;
        log_edit(
            ex,
            &card.board_id,
            &card.id,
            &format!("subtask deleted: {}", subtask.title),
        )
        .await?;
        Ok(Some(card.board_id))
    })
}

/// Rewrites a card's subtask positions to 0..n-1 in their current order.
async fn renumber_subtasks(ex: &mut Ex<'_>, card_id: &str) -> Result<(), String> {
    for (position, subtask) in list_subtasks_in(ex, card_id).await?.iter().enumerate() {
        if subtask.position != position as i64 {
            set_position(ex, &subtask.id, position).await?;
        }
    }
    Ok(())
}

async fn set_position(ex: &mut Ex<'_>, subtask_id: &str, position: usize) -> Result<(), String> {
    SubtaskRow::filter_by_id(subtask_id)
        .update()
        .position(position as i64)
        .exec(ex)
        .await
        .map_err(err)
}

/// Moves a subtask to `index` among its card's subtasks (clamped to the end)
/// and rewrites positions to 0..n-1. Moving to where it already is changes
/// nothing and logs nothing. Returns the owning board id.
pub fn move_subtask(conn: &Conn, subtask_id: &str, index: usize) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let Some(subtask) = get_subtask(ex, subtask_id).await? else {
            return Err("Subtask not found".into());
        };
        let card = get_card_in(ex, &subtask.card_id).await?;
        let current = list_subtasks_in(ex, &card.id).await?;
        let contiguous = current
            .iter()
            .enumerate()
            .all(|(i, s)| s.position == i as i64);
        let mut ids: Vec<String> = current.into_iter().map(|s| s.id).collect();
        let old_index = ids.iter().position(|id| id == subtask_id);
        ids.retain(|id| id != subtask_id);
        let new_index = index.min(ids.len());
        ids.insert(new_index, subtask_id.to_string());
        if old_index == Some(new_index) && contiguous {
            return Ok(card.board_id);
        }
        for (position, id) in ids.iter().enumerate() {
            set_position(ex, id, position).await?;
        }
        if old_index != Some(new_index) {
            touch(ex, &card.id).await?;
            log_edit(
                ex,
                &card.board_id,
                &card.id,
                &format!("subtask moved: {}", subtask.title),
            )
            .await?;
        }
        Ok(card.board_id)
    })
}
