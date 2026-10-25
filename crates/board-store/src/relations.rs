use crate::ops::{db, get_card, log_edit, touch, write_tx};
use rusqlite::{params, Connection};
use serde::Serialize;

pub const RELATION_KINDS: [&str; 3] = ["blocks", "relates", "duplicates"];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub card_id: String,
    pub related_card_id: String,
    pub kind: String,
}

fn check_kind(kind: &str) -> Result<(), String> {
    if RELATION_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(format!("Unknown relation: {kind}"))
    }
}

fn phrase(kind: &str, number: i64) -> String {
    match kind {
        "relates" => format!("relates to #{number}"),
        _ => format!("{kind} #{number}"),
    }
}

/// `card_id` blocks, relates to, or duplicates `related_card_id`. `relates`
/// is two-way, so either direction is the same relation; `blocks` and
/// `duplicates` are directed and refuse a cycle of any length. Returns the
/// owning board id.
pub fn add_relation(
    conn: &Connection,
    card_id: &str,
    related_card_id: &str,
    kind: &str,
) -> Result<String, String> {
    check_kind(kind)?;
    if card_id == related_card_id {
        return Err("A card can't be related to itself".into());
    }
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let related = get_card(&tx, related_card_id)?;
    if card.board_id != related.board_id {
        return Err("Cards are on different boards".into());
    }
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM board_card_relations WHERE kind = ?3 AND (
               (card_id = ?1 AND related_card_id = ?2)
               OR (?3 = 'relates' AND card_id = ?2 AND related_card_id = ?1)))",
            params![card_id, related_card_id, kind],
            |r| r.get(0),
        )
        .map_err(db)?;
    if exists {
        return Ok(card.board_id);
    }
    if kind != "relates" {
        // Would the new edge close a loop: does `related` already reach `card`?
        let cycle: bool = tx
            .query_row(
                "WITH RECURSIVE reach(id) AS (
                   SELECT ?1
                   UNION
                   SELECT r.related_card_id FROM board_card_relations r
                   JOIN reach ON r.card_id = reach.id WHERE r.kind = ?3
                 )
                 SELECT EXISTS(SELECT 1 FROM reach WHERE id = ?2)",
                params![related_card_id, card_id, kind],
                |r| r.get(0),
            )
            .map_err(db)?;
        if cycle {
            return Err("That would make a cycle".into());
        }
    }
    tx.execute(
        "INSERT INTO board_card_relations (card_id, related_card_id, kind) VALUES (?1, ?2, ?3)",
        params![card_id, related_card_id, kind],
    )
    .map_err(db)?;
    touch(&tx, card_id)?;
    log_edit(
        &tx,
        &card.board_id,
        card_id,
        &format!("relation: {}", phrase(kind, related.number)),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Returns the owning board id.
pub fn remove_relation(
    conn: &Connection,
    card_id: &str,
    related_card_id: &str,
    kind: &str,
) -> Result<String, String> {
    check_kind(kind)?;
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let related = get_card(&tx, related_card_id)?;
    let removed = tx
        .execute(
            "DELETE FROM board_card_relations WHERE kind = ?3 AND (
               (card_id = ?1 AND related_card_id = ?2)
               OR (?3 = 'relates' AND card_id = ?2 AND related_card_id = ?1))",
            params![card_id, related_card_id, kind],
        )
        .map_err(db)?;
    if removed == 0 {
        return Ok(card.board_id);
    }
    touch(&tx, card_id)?;
    log_edit(
        &tx,
        &card.board_id,
        card_id,
        &format!("relation removed: {}", phrase(kind, related.number)),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Every relation the card is on either side of.
pub fn card_relations(conn: &Connection, card_id: &str) -> Result<Vec<Relation>, String> {
    let mut statement = conn
        .prepare(
            "SELECT card_id, related_card_id, kind FROM board_card_relations
             WHERE card_id = ?1 OR related_card_id = ?1
             ORDER BY kind, card_id, related_card_id",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| {
            Ok(Relation {
                card_id: r.get(0)?,
                related_card_id: r.get(1)?,
                kind: r.get(2)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}
