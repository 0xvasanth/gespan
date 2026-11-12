//! Operations that touch several tables at once: converting a subtask to a
//! card, duplicating a card, and moving a card to another board. Each runs in
//! one write transaction.
use crate::db::{args, err, query, query_opt, with_tx, Conn, Ex};
use crate::fields::carry_value;
use crate::models::{
    BoardRow, CardLabelRow, CardMetaRow, CardProjectRow, CardRow, FieldRow, FieldValueRow,
    LabelRow, RelationRow, SubtaskRow,
};
use crate::ops::{
    column_card_ids, get_card_in, get_column, is_archived, live_card_count, log_activity, log_edit,
    new_id, next_card_number, touch, write_card_positions, NewActivity, MAX_TITLE,
};
use crate::{now_millis, Card};
use std::collections::HashMap;

const COPY_SUFFIX: &str = " (copy)";

/// Inserts a card into `column_id` at `index` (clamped) with a fresh number
/// on the column's board, and rewrites the column's positions. Writes no
/// activity.
async fn insert_card_at(
    ex: &mut Ex<'_>,
    column_id: &str,
    title: &str,
    body: &str,
    index: usize,
) -> Result<Card, String> {
    let column = get_column(ex, column_id).await?;
    let number = next_card_number(ex, &column.board_id).await?;
    let mut ids = column_card_ids(ex, column_id).await?;
    let index = index.min(ids.len());
    let now = now_millis();
    let card: Card = CardRow::create()
        .id(new_id())
        .board_id(column.board_id)
        .column_id(column.id)
        .number(number)
        .title(title)
        .body(body)
        .position(index as i64)
        .link_kind(None::<String>)
        .link_id(None::<String>)
        .created_at(now)
        .updated_at(now)
        .title_from_ticket(false)
        .body_from_ticket(false)
        .exec(ex)
        .await
        .map_err(err)?
        .into();
    ids.insert(index, card.id.clone());
    write_card_positions(ex, &ids).await?;
    Ok(card)
}

async fn live_index(ex: &mut Ex<'_>, card: &Card) -> Result<usize, String> {
    Ok(column_card_ids(ex, &card.column_id)
        .await?
        .iter()
        .position(|id| *id == card.id)
        .map_or(0, |i| i + 1))
}

async fn ensure_live(ex: &mut Ex<'_>, card_id: &str) -> Result<(), String> {
    if is_archived(ex, card_id).await? {
        return Err("Card is archived".into());
    }
    Ok(())
}

/// A card's subtask ids or titles (`column`), in list order.
async fn subtask_column(
    ex: &mut Ex<'_>,
    column: &str,
    card_id: &str,
) -> Result<Vec<String>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        &format!("SELECT {column} FROM board_subtasks WHERE card_id = ?1 ORDER BY position, rowid"),
        args![card_id],
    )
    .await?
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// Turns a subtask into a card in the parent's column, right after the
/// parent, related to the parent by `relates`. The subtask is deleted and the
/// parent logs "subtask converted: T → #M".
pub fn convert_subtask(conn: &Conn, subtask_id: &str) -> Result<Card, String> {
    with_tx!(conn, |ex| {
        let Some(subtask) = SubtaskRow::filter_by_id(subtask_id)
            .first()
            .exec(ex)
            .await
            .map_err(err)?
        else {
            return Err("Subtask not found".into());
        };
        let (parent_id, title) = (subtask.card_id, subtask.title);
        let parent = get_card_in(ex, &parent_id).await?;
        ensure_live(ex, &parent.id).await?;
        let index = live_index(ex, &parent).await?;
        let card = insert_card_at(ex, &parent.column_id, &title, "", index).await?;
        RelationRow::create()
            .card_id(&parent.id)
            .related_card_id(&card.id)
            .kind("relates")
            .exec(ex)
            .await
            .map_err(err)?;
        SubtaskRow::filter_by_id(subtask_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        let remaining = subtask_column(ex, "id", &parent.id).await?;
        for (position, id) in remaining.iter().enumerate() {
            SubtaskRow::filter_by_id(id)
                .update()
                .position(position as i64)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        touch(ex, &parent.id).await?;
        log_edit(
            ex,
            &parent.board_id,
            &parent.id,
            &format!("subtask converted: {title} → #{}", card.number),
        )
        .await?;
        Ok(card)
    })
}

/// Copies a card to "{title} (copy)" right after the original, with its body,
/// priority, due, start, estimate, labels, projects, field values and
/// subtasks (all reset to not done), and the marks that its title and
/// description came from a ticket. Links, ticket, comments, relations,
/// archive and completion state are not copied.
pub fn duplicate_card(conn: &Conn, card_id: &str) -> Result<Card, String> {
    with_tx!(conn, |ex| {
        let original = get_card_in(ex, card_id).await?;
        ensure_live(ex, card_id).await?;
        let keep = MAX_TITLE - COPY_SUFFIX.chars().count();
        let base: String = original.title.chars().take(keep).collect();
        let title = format!("{base}{COPY_SUFFIX}");
        let index = live_index(ex, &original).await?;
        let copy = insert_card_at(ex, &original.column_id, &title, &original.body, index).await?;
        copy_card_rows(ex, &original.id, &copy.id).await?;
        let subtasks = subtask_column(ex, "title", &original.id).await?;
        for (position, subtask_title) in subtasks.iter().enumerate() {
            SubtaskRow::create()
                .id(new_id())
                .card_id(&copy.id)
                .title(subtask_title)
                .done(false)
                .position(position as i64)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        let column = get_column(ex, &copy.column_id).await?;
        log_activity(
            ex,
            NewActivity {
                board_id: &copy.board_id,
                card_id: Some(&copy.id),
                kind: "created",
                actor: "user",
                from_column: None,
                to_column: Some(&column.name),
                detail: Some(&format!("duplicated from #{}", original.number)),
            },
        )
        .await?;
        Ok(copy)
    })
}

/// Copies `from`'s ticket-text marks, meta (not archive or completion),
/// labels, projects and field values onto the new card `to`. The copy drops
/// the ticket link but keeps its text, so text that came from a ticket stays
/// marked as ticket text.
async fn copy_card_rows(ex: &mut Ex<'_>, from: &str, to: &str) -> Result<(), String> {
    let original = CardRow::filter_by_id(from)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .ok_or_else(|| "Card not found".to_string())?;
    CardRow::filter_by_id(to)
        .update()
        .title_from_ticket(original.title_from_ticket)
        .body_from_ticket(original.body_from_ticket)
        .exec(ex)
        .await
        .map_err(err)?;
    if let Some(meta) = CardMetaRow::filter_by_card_id(from)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
    {
        CardMetaRow::create()
            .card_id(to)
            .priority(meta.priority)
            .due_at(meta.due_at)
            .start_at(meta.start_at)
            .estimate_minutes(meta.estimate_minutes)
            .archived_at(None::<i64>)
            .completed_at(None::<i64>)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    for label in CardLabelRow::filter_by_card_id(from)
        .exec(ex)
        .await
        .map_err(err)?
    {
        CardLabelRow::create()
            .card_id(to)
            .label_id(label.label_id)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    for project in CardProjectRow::filter_by_card_id(from)
        .exec(ex)
        .await
        .map_err(err)?
    {
        CardProjectRow::create()
            .card_id(to)
            .project(project.project)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    for value in FieldValueRow::filter_by_card_id(from)
        .exec(ex)
        .await
        .map_err(err)?
    {
        FieldValueRow::create()
            .card_id(to)
            .field_id(value.field_id)
            .value_json(value.value_json)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    Ok(())
}

/// Moves a card to the end of the target board's first `todo` column (else its
/// first column) under a new number there. Labels map by case-insensitive
/// name and field values by name and kind; what has no match is dropped.
/// Relations are removed; links, ticket, comments, subtasks, meta and
/// projects stay. Activity goes on both boards.
pub fn move_card_to_board(conn: &Conn, card_id: &str, board_id: &str) -> Result<Card, String> {
    with_tx!(conn, |ex| move_card_to_board_in(ex, card_id, board_id)
        .await)
}

async fn board_name(ex: &mut Ex<'_>, board_id: &str) -> Result<Option<String>, String> {
    BoardRow::filter_by_id(board_id)
        .select(BoardRow::fields().name())
        .first()
        .exec(ex)
        .await
        .map_err(err)
}

async fn move_card_to_board_in(
    ex: &mut Ex<'_>,
    card_id: &str,
    board_id: &str,
) -> Result<Card, String> {
    let card = get_card_in(ex, card_id).await?;
    ensure_live(ex, card_id).await?;
    if card.board_id == board_id {
        return Err("Card is already on this board".into());
    }
    let Some(target_name) = board_name(ex, board_id).await? else {
        return Err("Board not found".into());
    };
    let source_name = board_name(ex, &card.board_id)
        .await?
        .ok_or_else(|| "Board not found".to_string())?;
    let source_column = get_column(ex, &card.column_id).await?;
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    let first_column = query_opt(
        ex,
        "SELECT id, name FROM board_columns WHERE board_id = ?1
         ORDER BY (role IS NOT 'todo'), position, rowid LIMIT 1",
        args![board_id],
    )
    .await?
    .ok_or_else(|| "Board has no columns".to_string())?;
    let (column_id, column_name): (String, String) = (first_column.get(0)?, first_column.get(1)?);

    // Labels: keep those whose name exists on the target board.
    let target_labels: HashMap<String, String> =
        LabelRow::filter(LabelRow::fields().board_id().eq(board_id))
            .exec(ex)
            .await
            .map_err(err)?
            .into_iter()
            .map(|label| (label.name.to_lowercase(), label.id))
            .collect();
    let card_label_ids =
        CardLabelRow::filter_by_card_id(card_id).select(CardLabelRow::fields().label_id());
    let card_labels: Vec<String> =
        LabelRow::filter(LabelRow::fields().id().in_query(card_label_ids))
            .select(LabelRow::fields().name())
            .exec(ex)
            .await
            .map_err(err)?;
    CardLabelRow::filter_by_card_id(card_id)
        .delete()
        .exec(ex)
        .await
        .map_err(err)?;
    for name in card_labels {
        if let Some(label_id) = target_labels.get(&name.to_lowercase()) {
            CardLabelRow::upsert_by_card_id_and_label_id(card_id, label_id)
                .or_ignore()
                .exec(ex)
                .await
                .map_err(err)?;
        }
    }

    // Field values: keep those whose field (name, kind) exists on the target
    // and that are valid for it (multi-select loses options it lacks).
    let target_fields: HashMap<(String, String), (String, Option<String>)> = {
        // Raw SQL: ties on position keep insertion order (`rowid`), which
        // Toasty 0.11 cannot order by.
        let rows = query(
            ex,
            "SELECT id, name, kind, options_json FROM board_fields WHERE board_id = ?1
             ORDER BY position, rowid",
            args![board_id],
        )
        .await?;
        // On a duplicate name, the earliest field wins.
        let mut map = HashMap::new();
        for r in rows {
            let key = (r.get::<String>(1)?.to_lowercase(), r.get::<String>(2)?);
            map.entry(key)
                .or_insert((r.get::<String>(0)?, r.get::<Option<String>>(3)?));
        }
        map
    };
    let card_values = FieldValueRow::filter_by_card_id(card_id)
        .exec(ex)
        .await
        .map_err(err)?;
    let field_ids: Vec<String> = card_values.iter().map(|v| v.field_id.clone()).collect();
    let fields: HashMap<String, (String, String)> =
        FieldRow::filter(FieldRow::fields().id().in_list(field_ids))
            .exec(ex)
            .await
            .map_err(err)?
            .into_iter()
            .map(|field| (field.id, (field.name, field.kind)))
            .collect();
    let values: Vec<(String, String, String)> = card_values
        .into_iter()
        .filter_map(|v| {
            let (name, kind) = fields.get(&v.field_id)?.clone();
            Some((name, kind, v.value_json))
        })
        .collect();
    FieldValueRow::filter_by_card_id(card_id)
        .delete()
        .exec(ex)
        .await
        .map_err(err)?;
    for (name, kind, value_json) in values {
        let key = (name.to_lowercase(), kind);
        let Some((field_id, options)) = target_fields.get(&key) else {
            continue;
        };
        if let Some(value) = carry_value(&key.1, options.as_deref(), &value_json) {
            FieldValueRow::upsert_by_card_id_and_field_id(card_id, field_id)
                .value_json(value.to_string())
                .or_ignore()
                .exec(ex)
                .await
                .map_err(err)?;
        }
    }

    let relation = RelationRow::fields();
    RelationRow::filter(
        relation
            .card_id()
            .eq(card_id)
            .or(relation.related_card_id().eq(card_id)),
    )
    .delete()
    .exec(ex)
    .await
    .map_err(err)?;

    let number = next_card_number(ex, board_id).await?;
    let position = live_card_count(ex, &column_id).await?;
    CardRow::filter_by_id(card_id)
        .update()
        .board_id(board_id)
        .column_id(&column_id)
        .number(number)
        .position(position)
        .updated_at(now_millis())
        .exec(ex)
        .await
        .map_err(err)?;
    // The card already sits on the target board, so this lists only the cards left behind.
    let source_ids = column_card_ids(ex, &card.column_id).await?;
    write_card_positions(ex, &source_ids).await?;

    log_activity(
        ex,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind: "moved",
            actor: "user",
            from_column: Some(&source_column.name),
            to_column: None,
            detail: Some(&format!("moved to board {target_name}")),
        },
    )
    .await?;
    log_activity(
        ex,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind: "moved",
            actor: "user",
            from_column: None,
            to_column: Some(&column_name),
            detail: Some(&format!("moved from board {source_name}")),
        },
    )
    .await?;
    get_card_in(ex, card_id).await
}
