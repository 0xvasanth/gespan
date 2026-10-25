//! A card's links to sessions, pilots, runs, PRs, worktrees and tickets. The
//! single link both apps show today (`link_card`) is the row with role
//! `primary`; `board_cards.link_kind`/`link_id` mirror it for older readers.
use crate::now_millis;
use crate::ops::{db, get_card, log_activity, write_tx, NewActivity, LINK_KINDS};
use rusqlite::{params, Connection};
use serde::Serialize;

const MAX_REF: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardLink {
    pub card_id: String,
    pub kind: String,
    /// The linked thing's id; for tickets, the `TicketRef` JSON.
    pub reference: String,
    pub role: Option<String>,
    pub created_at: i64,
}

fn log_link(
    tx: &rusqlite::Transaction<'_>,
    board_id: &str,
    card_id: &str,
    kind: &str,
    detail: &str,
) -> Result<(), String> {
    log_activity(
        tx,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind,
            actor: "user",
            from_column: None,
            to_column: None,
            detail: Some(detail),
        },
    )
}

/// Adds a link; adding one the card already has changes nothing. Returns the
/// owning board id.
pub fn add_link(
    conn: &Connection,
    card_id: &str,
    kind: &str,
    reference: &str,
    role: Option<&str>,
) -> Result<String, String> {
    if !LINK_KINDS.contains(&kind) {
        return Err(format!("Unknown link kind: {kind}"));
    }
    if reference.is_empty() {
        return Err("Link id is required".into());
    }
    if reference.len() > MAX_REF {
        return Err("Link id is too long".into());
    }
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let added = tx
        .execute(
            "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![card_id, kind, reference, role, now_millis()],
        )
        .map_err(db)?;
    if added == 0 {
        return Ok(card.board_id);
    }
    log_link(
        &tx,
        &card.board_id,
        card_id,
        "linked",
        &format!("{kind}:{reference}"),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Removes a link, clearing the legacy link columns when it was the primary
/// link. Returns the owning board id.
pub fn remove_link(
    conn: &Connection,
    card_id: &str,
    kind: &str,
    reference: &str,
) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let removed = tx
        .execute(
            "DELETE FROM board_card_links WHERE card_id = ?1 AND kind = ?2 AND ref = ?3",
            params![card_id, kind, reference],
        )
        .map_err(db)?;
    let legacy =
        card.link_kind.as_deref() == Some(kind) && card.link_id.as_deref() == Some(reference);
    if legacy {
        tx.execute(
            "UPDATE board_cards SET link_kind = NULL, link_id = NULL, updated_at = ?1 WHERE id = ?2",
            params![now_millis(), card_id],
        )
        .map_err(db)?;
    }
    if removed == 0 && !legacy {
        return Ok(card.board_id);
    }
    log_link(
        &tx,
        &card.board_id,
        card_id,
        "unlinked",
        &format!("{kind}:{reference}"),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

pub fn card_links(conn: &Connection, card_id: &str) -> Result<Vec<CardLink>, String> {
    let mut statement = conn
        .prepare(
            "SELECT card_id, kind, ref, role, created_at FROM board_card_links
             WHERE card_id = ?1 ORDER BY kind, ref",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| {
            Ok(CardLink {
                card_id: r.get(0)?,
                kind: r.get(1)?,
                reference: r.get(2)?,
                role: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}
