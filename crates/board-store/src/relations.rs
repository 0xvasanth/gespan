use crate::db::{args, err, execute, query, query_value, with_db, with_tx, Conn};
use crate::models::RelationRow;
use crate::ops::{get_card_in, log_edit, touch};
use serde::Serialize;
use std::collections::BTreeMap;

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
/// `duplicates` are directed and refuse a cycle of any length. Two cards have
/// one relation at most. Returns the owning board id.
pub fn add_relation(
    conn: &Conn,
    card_id: &str,
    related_card_id: &str,
    kind: &str,
) -> Result<String, String> {
    check_kind(kind)?;
    if card_id == related_card_id {
        return Err("A card can't be related to itself".into());
    }
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let related = get_card_in(ex, related_card_id).await?;
        if card.board_id != related.board_id {
            return Err("Cards are on different boards".into());
        }
        let exists: bool = query_value(
            ex,
            "SELECT EXISTS(SELECT 1 FROM board_card_relations WHERE kind = ?3 AND (
               (card_id = ?1 AND related_card_id = ?2)
               OR (?3 = 'relates' AND card_id = ?2 AND related_card_id = ?1)))",
            args![card_id, related_card_id, kind],
        )
        .await?;
        if exists {
            return Ok(card.board_id);
        }
        if kind != "relates" {
            // Would the new edge close a loop: does `related` already reach
            // `card`? A recursive CTE, so raw SQL.
            let cycle: bool = query_value(
                ex,
                "WITH RECURSIVE reach(id) AS (
                   SELECT ?1
                   UNION
                   SELECT r.related_card_id FROM board_card_relations r
                   JOIN reach ON r.card_id = reach.id WHERE r.kind = ?3
                 )
                 SELECT EXISTS(SELECT 1 FROM reach WHERE id = ?2)",
                args![related_card_id, card_id, kind],
            )
            .await?;
            if cycle {
                return Err("That would make a cycle".into());
            }
        }
        // Two cards have one relation at most: "blocked by" and "relates to" the
        // same card would say two different things about one pair.
        let linked: bool = query_value(
            ex,
            "SELECT EXISTS(SELECT 1 FROM board_card_relations WHERE
               (card_id = ?1 AND related_card_id = ?2)
               OR (card_id = ?2 AND related_card_id = ?1))",
            args![card_id, related_card_id],
        )
        .await?;
        if linked {
            return Err("These cards are already linked. Remove that link first".into());
        }
        RelationRow::create()
            .card_id(card_id)
            .related_card_id(related_card_id)
            .kind(kind)
            .exec(ex)
            .await
            .map_err(err)?;
        touch(ex, card_id).await?;
        log_edit(
            ex,
            &card.board_id,
            card_id,
            &format!("relation: {}", phrase(kind, related.number)),
        )
        .await?;
        Ok(card.board_id)
    })
}

/// Returns the owning board id.
pub fn remove_relation(
    conn: &Conn,
    card_id: &str,
    related_card_id: &str,
    kind: &str,
) -> Result<String, String> {
    check_kind(kind)?;
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let related = get_card_in(ex, related_card_id).await?;
        let removed = execute(
            ex,
            "DELETE FROM board_card_relations WHERE kind = ?3 AND (
               (card_id = ?1 AND related_card_id = ?2)
               OR (?3 = 'relates' AND card_id = ?2 AND related_card_id = ?1))",
            args![card_id, related_card_id, kind],
        )
        .await?;
        if removed == 0 {
            return Ok(card.board_id);
        }
        touch(ex, card_id).await?;
        log_edit(
            ex,
            &card.board_id,
            card_id,
            &format!("relation removed: {}", phrase(kind, related.number)),
        )
        .await?;
        Ok(card.board_id)
    })
}

/// Every relation the card is on either side of.
pub fn card_relations(conn: &Conn, card_id: &str) -> Result<Vec<Relation>, String> {
    with_db!(conn, |ex| {
        query(
            ex,
            "SELECT card_id, related_card_id, kind FROM board_card_relations
             WHERE card_id = ?1 OR related_card_id = ?1
             ORDER BY kind, card_id, related_card_id",
            args![card_id],
        )
        .await?
        .iter()
        .map(|r| {
            Ok(Relation {
                card_id: r.get(0)?,
                related_card_id: r.get(1)?,
                kind: r.get(2)?,
            })
        })
        .collect()
    })
}

/// The board's blocked cards: each live card that a `blocks` relation points
/// at from a live card whose column role is not `done` or `cancelled`, mapped
/// to those blockers' numbers, sorted.
pub fn blocked_cards(conn: &Conn, board_id: &str) -> Result<BTreeMap<String, Vec<i64>>, String> {
    let rows = with_db!(conn, |ex| {
        query(
            ex,
            "SELECT r.related_card_id, blocker.number
             FROM board_card_relations r
             JOIN board_cards blocker ON blocker.id = r.card_id
             JOIN board_cards blocked ON blocked.id = r.related_card_id
             JOIN board_columns col ON col.id = blocker.column_id
             WHERE r.kind = 'blocks'
               AND blocked.board_id = ?1
               AND blocker.board_id = ?1
               AND COALESCE(col.role, '') NOT IN ('done', 'cancelled')
               AND NOT EXISTS (SELECT 1 FROM board_card_meta m
                 WHERE m.card_id = blocker.id AND m.archived_at IS NOT NULL)
               AND NOT EXISTS (SELECT 1 FROM board_card_meta m
                 WHERE m.card_id = blocked.id AND m.archived_at IS NOT NULL)
             ORDER BY r.related_card_id, blocker.number",
            args![board_id],
        )
        .await?
        .iter()
        .map(|r| Ok((r.get::<String>(0)?, r.get::<i64>(1)?)))
        .collect::<Result<Vec<_>, String>>()
    })?;
    let mut blocked: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (card_id, number) in rows {
        blocked.entry(card_id).or_default().push(number);
    }
    Ok(blocked)
}
