use crate::db::{args, err, query_value, with_db, with_tx, Conn};
use crate::models::{CardRow, ColumnRow, RelationRow};
use crate::ops::{count, get_card_in, live, log_edit, touch};
use serde::Serialize;
use std::collections::BTreeMap;
use toasty::stmt::Expr;

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

/// `card_id` → `related_card_id` as `kind`, or either way round for the
/// two-way `relates`.
fn same_relation(card_id: &str, related_card_id: &str, kind: &str) -> Expr<bool> {
    let relation = RelationRow::fields();
    let forward = relation
        .card_id()
        .eq(card_id)
        .and(relation.related_card_id().eq(related_card_id));
    let pair = if kind == "relates" {
        forward.or(relation
            .card_id()
            .eq(related_card_id)
            .and(relation.related_card_id().eq(card_id)))
    } else {
        forward
    };
    relation.kind().eq(kind).and(pair)
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
        let exists = count(
            ex,
            RelationRow::filter(same_relation(card_id, related_card_id, kind)).count(),
        )
        .await?;
        if exists > 0 {
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
        let relation = RelationRow::fields();
        let linked = count(
            ex,
            RelationRow::filter(
                relation
                    .card_id()
                    .eq(card_id)
                    .and(relation.related_card_id().eq(related_card_id))
                    .or(relation
                        .card_id()
                        .eq(related_card_id)
                        .and(relation.related_card_id().eq(card_id))),
            )
            .count(),
        )
        .await?;
        if linked > 0 {
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
        let found = count(
            ex,
            RelationRow::filter(same_relation(card_id, related_card_id, kind)).count(),
        )
        .await?;
        if found == 0 {
            return Ok(card.board_id);
        }
        RelationRow::filter(same_relation(card_id, related_card_id, kind))
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
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
        let relation = RelationRow::fields();
        Ok(RelationRow::filter(
            relation
                .card_id()
                .eq(card_id)
                .or(relation.related_card_id().eq(card_id)),
        )
        .order_by((
            relation.kind().asc(),
            relation.card_id().asc(),
            relation.related_card_id().asc(),
        ))
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .map(|row| Relation {
            card_id: row.card_id,
            related_card_id: row.related_card_id,
            kind: row.kind,
        })
        .collect())
    })
}

/// The board's blocked cards: each live card that a `blocks` relation points
/// at from a live card whose column role is not `done` or `cancelled`, mapped
/// to those blockers' numbers, sorted.
pub fn blocked_cards(conn: &Conn, board_id: &str) -> Result<BTreeMap<String, Vec<i64>>, String> {
    let rows = with_db!(conn, |ex| {
        let card = CardRow::fields();
        let column = ColumnRow::fields();
        // Columns whose cards still block: any role but done and cancelled.
        let open_columns = ColumnRow::filter(
            column.board_id().eq(board_id).and(
                column.role().is_none().or(column
                    .role()
                    .in_list([Some("done".to_string()), Some("cancelled".to_string())])
                    .not()),
            ),
        )
        .select(column.id());
        let blockers: BTreeMap<String, i64> = CardRow::filter(
            card.board_id()
                .eq(board_id)
                .and(card.column_id().in_query(open_columns))
                .and(live()),
        )
        .select((card.id(), card.number()))
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .collect();
        let blockable = CardRow::filter(card.board_id().eq(board_id).and(live())).select(card.id());
        let relation = RelationRow::fields();
        let relations = RelationRow::filter(
            relation
                .kind()
                .eq("blocks")
                .and(
                    relation
                        .card_id()
                        .in_list(blockers.keys().cloned().collect::<Vec<_>>()),
                )
                .and(relation.related_card_id().in_query(blockable)),
        )
        .exec(ex)
        .await
        .map_err(err)?;
        let mut rows: Vec<(String, i64)> = relations
            .into_iter()
            .filter_map(|r| Some((r.related_card_id, *blockers.get(&r.card_id)?)))
            .collect();
        rows.sort();
        Ok::<_, String>(rows)
    })?;
    let mut blocked: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (card_id, number) in rows {
        blocked.entry(card_id).or_default().push(number);
    }
    Ok(blocked)
}
