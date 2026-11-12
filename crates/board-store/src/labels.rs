use crate::db::{err, with_db, with_tx, Conn, Ex};
use crate::meta::{card_label_ids, card_label_names, cards_on_one_board};
use crate::models::{CardLabelRow, CardRow, LabelRow};
use crate::ops::{clean_text, count, get_card_in, log_edit, new_id, touch};
use crate::views::{map_strings, rewrite_views, row_is, spec_rows};
use serde::Serialize;
use std::collections::BTreeSet;

const MAX_LABEL: usize = 40;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub color: String,
    pub position: i64,
}

impl From<LabelRow> for Label {
    fn from(row: LabelRow) -> Self {
        Label {
            id: row.id,
            board_id: row.board_id,
            name: row.name,
            color: row.color,
            position: row.position,
        }
    }
}

/// The board's labels in list order: position, then name ignoring ASCII
/// case, as SQLite's `lower` compares it.
pub(crate) async fn list_labels_in(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<Label>, String> {
    let mut labels: Vec<Label> = LabelRow::filter(LabelRow::fields().board_id().eq(board_id))
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .map(Label::from)
        .collect();
    labels.sort_by_cached_key(|label| (label.position, label.name.to_ascii_lowercase()));
    Ok(labels)
}

pub fn list_labels(conn: &Conn, board_id: &str) -> Result<Vec<Label>, String> {
    with_db!(conn, |ex| list_labels_in(ex, board_id).await)
}

/// Adds a label at the end of the board's list. Names are unique per board,
/// ignoring case.
pub fn create_label(conn: &Conn, board_id: &str, name: &str, color: &str) -> Result<Label, String> {
    let name = clean_text(name, "Label name", MAX_LABEL)?;
    check_color(color)?;
    with_tx!(conn, |ex| {
        check_name_free(ex, board_id, &name, None).await?;
        let label = LabelRow::fields();
        let position = LabelRow::filter(label.board_id().eq(board_id))
            .order_by(label.position().desc())
            .select(label.position())
            .first()
            .exec(ex)
            .await
            .map_err(err)?
            .map_or(0, |last| last + 1);
        LabelRow::create()
            .id(new_id())
            .board_id(board_id)
            .name(name)
            .color(color)
            .position(position)
            .exec(ex)
            .await
            .map(Label::from)
            .map_err(err)
    })
}

/// Replaces the card's labels. Returns the owning board id.
pub fn set_card_labels(conn: &Conn, card_id: &str, label_ids: &[String]) -> Result<String, String> {
    let ids: BTreeSet<&str> = label_ids.iter().map(String::as_str).collect();
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let current = card_label_ids(ex, card_id).await?;
        if current.iter().map(String::as_str).eq(ids.iter().copied()) {
            return Ok(card.board_id);
        }
        CardLabelRow::filter_by_card_id(card_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        let mut names = Vec::new();
        for id in &ids {
            let label = LabelRow::fields();
            let name =
                LabelRow::filter(label.id().eq(*id).and(label.board_id().eq(&card.board_id)))
                    .select(label.name())
                    .first()
                    .exec(ex)
                    .await
                    .map_err(err)?;
            let Some(name) = name else {
                return Err("Label not on this board".into());
            };
            CardLabelRow::create()
                .card_id(card_id)
                .label_id(*id)
                .exec(ex)
                .await
                .map_err(err)?;
            names.push(name);
        }
        touch(ex, card_id).await?;
        log_edit(ex, &card.board_id, card_id, &labels_detail(names)).await?;
        Ok(card.board_id)
    })
}

/// The activity detail for a card's labels: "labels: A, b" (by name, case
/// folded) or "labels: none".
fn labels_detail(mut names: Vec<String>) -> String {
    if names.is_empty() {
        return "labels: none".into();
    }
    names.sort_by_key(|name| name.to_lowercase());
    format!("labels: {}", names.join(", "))
}

/// Adds (`on`) or removes one label on several cards (all on the label's
/// board) in one transaction. Each card that changed gets one row naming all
/// its labels after the change. Returns the board id.
pub fn set_label_many(
    conn: &Conn,
    card_ids: &[String],
    label_id: &str,
    on: bool,
) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let (board_id, cards) = cards_on_one_board(ex, card_ids, "No cards to edit").await?;
        let label = LabelRow::fields();
        let on_board = count(
            ex,
            LabelRow::filter(label.id().eq(label_id).and(label.board_id().eq(&board_id))).count(),
        )
        .await?;
        if on_board == 0 {
            return Err("Label not on this board".into());
        }
        for card in &cards {
            let changed = if on {
                add_card_label(ex, &card.id, label_id).await?
            } else {
                remove_card_label(ex, &card.id, label_id).await?
            };
            if !changed {
                continue;
            }
            let names: Vec<String> = card_label_names(ex, &card.id).await?.into_iter().collect();
            touch(ex, &card.id).await?;
            log_edit(ex, &board_id, &card.id, &labels_detail(names)).await?;
        }
        Ok(board_id)
    })
}

/// Puts the label on the card. Whether it was not there before.
async fn add_card_label(ex: &mut Ex<'_>, card_id: &str, label_id: &str) -> Result<bool, String> {
    Ok(
        CardLabelRow::upsert_by_card_id_and_label_id(card_id, label_id)
            .or_ignore()
            .exec(ex)
            .await
            .map_err(err)?
            .is_some(),
    )
}

/// Takes the label off the card. Whether it was there.
async fn remove_card_label(ex: &mut Ex<'_>, card_id: &str, label_id: &str) -> Result<bool, String> {
    let row = CardLabelRow::filter_by_card_id_and_label_id(card_id, label_id);
    if row.clone().first().exec(ex).await.map_err(err)?.is_none() {
        return Ok(false);
    }
    row.delete().exec(ex).await.map_err(err)?;
    Ok(true)
}

fn check_color(color: &str) -> Result<(), String> {
    if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Label colour must be six hex digits".into());
    }
    Ok(())
}

/// Refuses a name another label on the board has, ignoring case. SQLite's
/// lower() folds only ASCII, so names are compared in Rust.
async fn check_name_free(
    ex: &mut Ex<'_>,
    board_id: &str,
    name: &str,
    except: Option<&str>,
) -> Result<(), String> {
    let folded = name.to_lowercase();
    let labels = LabelRow::filter(LabelRow::fields().board_id().eq(board_id))
        .exec(ex)
        .await
        .map_err(err)?;
    let taken = labels
        .iter()
        .any(|label| Some(label.id.as_str()) != except && label.name.to_lowercase() == folded);
    if taken {
        return Err(format!("A label named {name} already exists"));
    }
    Ok(())
}

async fn get_label(ex: &mut Ex<'_>, label_id: &str) -> Result<Label, String> {
    LabelRow::filter_by_id(label_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(Label::from)
        .ok_or_else(|| "Label not found".to_string())
}

/// The board's label ids in list order.
async fn label_ids(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<String>, String> {
    Ok(list_labels_in(ex, board_id)
        .await?
        .into_iter()
        .map(|label| label.id)
        .collect())
}

async fn write_label_positions(ex: &mut Ex<'_>, ids: &[String]) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        let label = LabelRow::fields();
        LabelRow::filter(label.id().eq(id).and(label.position().ne(index as i64)))
            .update()
            .position(index as i64)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    Ok(())
}

/// The cards (live and archived) that carry the label.
async fn cards_with(ex: &mut Ex<'_>, label_id: &str) -> Result<Vec<String>, String> {
    let label = CardLabelRow::fields();
    let cards = CardRow::all().select(CardRow::fields().id());
    let mut ids: Vec<String> = CardLabelRow::filter(
        label
            .label_id()
            .eq(label_id)
            .and(label.card_id().in_query(cards)),
    )
    .select(label.card_id())
    .exec(ex)
    .await
    .map_err(err)?;
    // In card id order, as the set this replaced gave them.
    ids.sort();
    Ok(ids)
}

/// Renames and/or recolours a label. The name stays unique on its board,
/// ignoring case. Returns the owning board id.
pub fn update_label(
    conn: &Conn,
    label_id: &str,
    name: Option<&str>,
    color: Option<&str>,
) -> Result<String, String> {
    let name = name
        .map(|name| clean_text(name, "Label name", MAX_LABEL))
        .transpose()?;
    color.map(check_color).transpose()?;
    with_tx!(conn, |ex| {
        let label = get_label(ex, label_id).await?;
        if let Some(name) = &name {
            check_name_free(ex, &label.board_id, name, Some(label_id)).await?;
        }
        if let Some(name) = name {
            LabelRow::filter_by_id(label_id)
                .update()
                .name(name)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        if let Some(color) = color {
            LabelRow::filter_by_id(label_id)
                .update()
                .color(color)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        Ok(label.board_id)
    })
}

/// Moves a label to `index` (clamped) in its board's list. Returns the
/// owning board id.
pub fn move_label(conn: &Conn, label_id: &str, index: usize) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let label = get_label(ex, label_id).await?;
        let mut ids = label_ids(ex, &label.board_id).await?;
        ids.retain(|id| id != label_id);
        ids.insert(index.min(ids.len()), label_id.to_string());
        write_label_positions(ex, &ids).await?;
        Ok(label.board_id)
    })
}

/// Deletes a label. Every card that had it (archived ones too) gets one
/// "label removed: {name}" row. Saved views lose the label from their
/// `labels` rows (a row left with no value goes) and their collapsed list
/// groups. Returns the owning board id.
pub fn delete_label(conn: &Conn, label_id: &str) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let label = get_label(ex, label_id).await?;
        let detail = format!("label removed: {}", label.name);
        for card_id in cards_with(ex, label_id).await? {
            touch(ex, &card_id).await?;
            log_edit(ex, &label.board_id, &card_id, &detail).await?;
        }
        // The label's card rows go with it (`board_labels_cascade`).
        LabelRow::filter_by_id(label_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        let ids = label_ids(ex, &label.board_id).await?;
        write_label_positions(ex, &ids).await?;
        let group = format!("label:{label_id}");
        rewrite_views(ex, &label.board_id, |spec, layout| {
            rewrite_label_rows(spec, |id| (id != label_id).then(|| id.to_string()));
            if let Some(collapsed) = layout.get_mut("collapsed") {
                map_strings(collapsed, |key| (key != group).then(|| key.to_string()));
            }
        })
        .await?;
        Ok(label.board_id)
    })
}

/// Merges `from_id` into `into_id` (both on one board): every card with
/// `from_id` gets `into_id` instead, `from_id` is deleted, and saved views'
/// `labels` rows and collapsed list groups point at `into_id`. Each card that
/// changed gets one row naming its labels after. Returns the board id.
pub fn merge_labels(conn: &Conn, from_id: &str, into_id: &str) -> Result<String, String> {
    if from_id == into_id {
        return Err("Pick a different label to merge into".into());
    }
    with_tx!(conn, |ex| {
        let from = get_label(ex, from_id).await?;
        let into = get_label(ex, into_id).await?;
        if from.board_id != into.board_id {
            return Err("Label not on this board".into());
        }
        let board_id = from.board_id;
        for card_id in cards_with(ex, from_id).await? {
            add_card_label(ex, &card_id, into_id).await?;
            remove_card_label(ex, &card_id, from_id).await?;
            let names: Vec<String> = card_label_names(ex, &card_id).await?.into_iter().collect();
            touch(ex, &card_id).await?;
            log_edit(ex, &board_id, &card_id, &labels_detail(names)).await?;
        }
        LabelRow::filter_by_id(from_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        let ids = label_ids(ex, &board_id).await?;
        write_label_positions(ex, &ids).await?;
        let (from_group, into_group) = (format!("label:{from_id}"), format!("label:{into_id}"));
        rewrite_views(ex, &board_id, |spec, layout| {
            rewrite_label_rows(spec, |id| {
                Some(if id == from_id { into_id } else { id }.to_string())
            });
            if let Some(collapsed) = layout.get_mut("collapsed") {
                map_strings(collapsed, |key| {
                    Some(if key == from_group { &into_group } else { key }.to_string())
                });
            }
        })
        .await?;
        Ok(board_id)
    })
}

/// Maps the (trimmed) values of a spec's `labels` rows through `each`,
/// dropping repeats. A row that had values and has none left is removed:
/// the engine would ignore it anyway.
fn rewrite_label_rows(spec: &mut serde_json::Value, each: impl Fn(&str) -> Option<String>) {
    let Some(rows) = spec_rows(spec) else {
        return;
    };
    rows.retain_mut(|row| {
        if !row_is(row, "labels") {
            return true;
        }
        let Some(values) = row.get_mut("values") else {
            return true;
        };
        let had = values.as_array().is_some_and(|v| !v.is_empty());
        map_strings(values, |value| each(value.trim()));
        !(had && values.as_array().is_some_and(Vec::is_empty))
    });
}
