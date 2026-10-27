//! Columns: add, rename, recolour, role, WIP limit, reorder and delete.
use super::{Edit, button, card_row, heading, palette_hexes, swatch, target};
use crate::board::{BoardView, ROLES};
use gespan_core::board::extras::ColumnDisplayPatch;
use gpui::{AnyElement, Context, div, prelude::*, px};

const CANCELLED: (&str, &str) = ("cancelled", "Cancelled");

impl BoardView {
    fn add_settings_column(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        let mut created = None;
        let ok = self.settings_write(cx, |service| {
            created = Some(service.column_create(&board_id, "New column", None)?.id);
            Ok(())
        });
        if let (true, Some(id)) = (ok, created) {
            self.start_edit(Edit::ColumnName(id), "New column", window, cx);
        }
    }

    pub(super) fn commit_wip(&mut self, id: &str, text: &str, cx: &mut Context<Self>) -> bool {
        let limit = if text.is_empty() {
            None
        } else {
            match text.parse::<i64>() {
                Ok(limit) => Some(limit),
                Err(_) => {
                    if let Some(page) = &mut self.settings {
                        page.error = Some("A WIP limit is a whole number".into());
                    }
                    return false;
                }
            }
        };
        if self.column(id).is_none() {
            return true;
        }
        // Only the limit is written: a colour or collapse set in the other
        // app since the last load stays.
        let patch = ColumnDisplayPatch {
            wip_limit: Some(limit),
            ..Default::default()
        };
        self.settings_write(cx, |service| {
            service.update_column_display(id, &patch).map(drop)
        })
    }

    fn recolor_column(&mut self, id: &str, hex: &str, cx: &mut Context<Self>) {
        let Some(column) = self.column(id) else {
            return;
        };
        let color = (column.color.as_deref() != Some(hex)).then(|| hex.to_string());
        let patch = ColumnDisplayPatch {
            color: Some(color),
            ..Default::default()
        };
        self.settings_write(cx, |service| {
            service.update_column_display(id, &patch).map(drop)
        });
    }

    fn set_settings_role(&mut self, id: &str, role: Option<&str>, cx: &mut Context<Self>) {
        let Some(name) = self.column(id).map(|column| column.name.clone()) else {
            return;
        };
        self.settings_write(cx, |service| service.column_update(id, &name, role));
    }

    fn move_settings_column(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.column_move(id, index));
    }

    /// Delete a column. One that holds cards asks where they go first.
    fn delete_settings_column(&mut self, id: &str, cx: &mut Context<Self>) {
        self.commit_pending(cx);
        if self.refuse_archived_column_delete(id, cx) {
            return;
        }
        let live = self.detail.as_ref().map_or(0, |detail| {
            detail.cards.iter().filter(|c| c.column_id == id).count()
        });
        if live > 0 {
            if let Some(page) = &mut self.settings {
                page.clear_prompts();
                page.error = None;
                page.moving = Some(id.to_string());
            }
            cx.notify();
            return;
        }
        self.settings_write(cx, |service| service.column_delete(id));
    }

    /// A column that still holds archived cards can't be deleted (the store's
    /// refusal), so say so before anything moves. True when refused.
    fn refuse_archived_column_delete(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let archived = self.column_counts.get(id).map_or(0, |counts| counts.1);
        if archived == 0 {
            return false;
        }
        let plural = if archived == 1 { "card" } else { "cards" };
        if let Some(page) = &mut self.settings {
            page.clear_prompts();
            page.error = Some(format!(
                "Couldn't delete the column: it has {archived} archived {plural}. Delete them from the Archive first"
            ));
        }
        cx.notify();
        true
    }

    /// Move the live cards to `target`, then delete. A column with archived
    /// cards is refused first, so nothing is moved for a delete that can't happen.
    fn delete_column_moving_cards(&mut self, id: &str, target: &str, cx: &mut Context<Self>) {
        if self.refuse_archived_column_delete(id, cx) {
            return;
        }
        let ids: Vec<String> = self.detail.as_ref().map_or_else(Vec::new, |detail| {
            let mut cards: Vec<_> = detail.cards.iter().filter(|c| c.column_id == id).collect();
            cards.sort_by_key(|card| card.position);
            cards.into_iter().map(|card| card.id.clone()).collect()
        });
        self.settings_write(cx, |service| {
            service.move_many(&ids, target, usize::MAX, "user")?;
            service.column_delete(id)
        });
    }

    pub(super) fn render_columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = &self.detail else {
            return div().into_any_element();
        };
        let moving = self.settings.as_ref().and_then(|page| page.moving.clone());
        let last = detail.columns.len().saturating_sub(1);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, column) in detail.columns.iter().enumerate() {
            let id = column.id.clone();
            let wip = match column.wip_limit {
                Some(limit) => format!("WIP limit {limit}"),
                None => "No WIP limit".to_string(),
            };
            let roles: Vec<AnyElement> = ROLES
                .iter()
                .copied()
                .chain([CANCELLED])
                .map(|(role, label)| (Some(role), label))
                .chain([(None, "No role")])
                .map(|(role, label)| {
                    let on = column.role.as_deref() == role;
                    let id = id.clone();
                    button(
                        format!("settings-col-{index}-role-{}", role.unwrap_or("none")),
                        label,
                        on,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_settings_role(&id, role, cx)),
                    )
                    .into_any_element()
                })
                .collect();
            let colors: Vec<AnyElement> = palette_hexes()
                .map(|hex| {
                    let on = column.color.as_deref() == Some(hex.as_str());
                    let id = id.clone();
                    let value = hex.clone();
                    swatch(format!("settings-col-{index}-color-{hex}"), &hex, on)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.recolor_column(&id, &value, cx)),
                        )
                        .into_any_element()
                })
                .collect();
            let (up_id, down_id, delete_id, wip_id) =
                (id.clone(), id.clone(), id.clone(), id.clone());
            let mut row = div().flex().flex_col().gap_2().child(
                card_row()
                    .flex_col()
                    .items_stretch()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(self.editable_text(
                                format!("settings-col-{index}-name"),
                                Edit::ColumnName(id.clone()),
                                column.name.clone(),
                                cx,
                            ))
                            .child(
                                button(format!("settings-col-{index}-up"), "Up", false)
                                    .opacity(if index == 0 { 0.4 } else { 1. })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if index > 0 {
                                            this.move_settings_column(&up_id, index - 1, cx)
                                        }
                                    })),
                            )
                            .child(
                                button(format!("settings-col-{index}-down"), "Down", false)
                                    .opacity(if index == last { 0.4 } else { 1. })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if index < last {
                                            this.move_settings_column(&down_id, index + 1, cx)
                                        }
                                    })),
                            )
                            .child(
                                button(format!("settings-col-{index}-delete"), "Delete", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete_settings_column(&delete_id, cx)
                                    })),
                            ),
                    )
                    .child(div().flex().flex_wrap().gap(px(6.)).children(roles))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex().gap(px(6.)).children(colors))
                            .child(self.editable_text_with(
                                format!("settings-col-{index}-wip"),
                                Edit::ColumnWip(wip_id),
                                wip,
                                column.wip_limit.map(|l| l.to_string()).unwrap_or_default(),
                                cx,
                            )),
                    ),
            );
            if moving.as_deref() == Some(id.as_str()) {
                let targets: Vec<AnyElement> = detail
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| other.id != id)
                    .map(|(to, other)| {
                        let (from_id, to_id) = (id.clone(), other.id.clone());
                        button(
                            format!("settings-col-{index}-move-{to}"),
                            &other.name,
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_column_moving_cards(&from_id, &to_id, cx)
                        }))
                        .into_any_element()
                    })
                    .collect();
                row = row.child(
                    div()
                        .id("settings-move-cards")
                        .debug_selector(|| "settings-move-cards".into())
                        .px(px(10.))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(crate::theme::content(0.6))
                                .child("Move cards to…"),
                        )
                        .children(targets),
                );
            }
            rows.push(row.into_any_element());
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading("COLUMNS"))
            .children(rows)
            .child(
                div().flex().child(
                    target("settings-col-add".into())
                        .aria_label("Add column")
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded_md()
                        .border_1()
                        .border_color(crate::theme::content(0.12))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .child("Add column")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_settings_column(window, cx)),
                        ),
                ),
            )
            .into_any_element()
    }
}
