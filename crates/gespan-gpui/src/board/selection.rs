//! Bulk select: ⌘-click (Ctrl-click elsewhere) toggles a card, Shift-click
//! selects a range in the order the cards are shown, and a floating bar
//! applies one change to every selected card. Works in board, list and table.
//! The menus the bar and the layouts open share one slot, `PopMenu`.
use super::{
    BoardView,
    list::Layout,
    quick::{label_items, parse_priority, priority_items},
    undo::{Undo, archived_label, deleted_label, moved_label},
};
use crate::{
    popover::{Anchor, AnchorSlot, Menu, MenuAction, MenuEvent, MenuItem, menu_popover},
    theme::*,
};
use gpui::{
    AnyElement, ClickEvent, Context, Role, SharedString, Subscription, Window, div, prelude::*, px,
};
use std::collections::HashSet;

/// The cards selected and where a Shift-click range starts.
#[derive(Default)]
pub(crate) struct Selection {
    pub ids: HashSet<String>,
    pub anchor: Option<String>,
}

/// Which menu the shared slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuKind {
    BulkStatus,
    BulkPriority,
    BulkLabels,
    TableColumns,
    GroupBy,
}

pub(super) struct PopMenu {
    kind: MenuKind,
    menu: gpui::Entity<Menu>,
    _events: Subscription,
}

/// Where each menu opens from.
#[derive(Default)]
pub(super) struct PopAnchors {
    status: AnchorSlot,
    priority: AnchorSlot,
    labels: AnchorSlot,
    pub columns: AnchorSlot,
    pub group: AnchorSlot,
}

impl PopAnchors {
    fn of(&self, kind: MenuKind) -> AnchorSlot {
        match kind {
            MenuKind::BulkStatus => self.status.clone(),
            MenuKind::BulkPriority => self.priority.clone(),
            MenuKind::BulkLabels => self.labels.clone(),
            MenuKind::TableColumns => self.columns.clone(),
            MenuKind::GroupBy => self.group.clone(),
        }
    }
}

impl BoardView {
    pub(crate) fn is_selected(&self, card_id: &str) -> bool {
        self.selection.ids.contains(card_id)
    }

    pub(crate) fn selected_count(&self) -> usize {
        self.selection.ids.len()
    }

    /// The cards in the order the open layout shows them: the board's columns
    /// left to right, the list's groups top to bottom, or the table's rows.
    /// A card in several list groups counts once, at its first place. With
    /// swimlanes the board reads lane by lane, then column by column.
    pub(crate) fn display_order(&self) -> Vec<String> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        match self.layout_kind() {
            Layout::Board if self.lane_mode().is_some() => self.lane_order(),
            Layout::Board => detail
                .columns
                .iter()
                .filter(|column| !column.collapsed)
                .flat_map(|column| self.column_cards(detail, &column.id))
                .map(|card| card.id.clone())
                .collect(),
            Layout::List => {
                let spec = self.layout_spec();
                let mut seen = HashSet::new();
                self.groups(&spec)
                    .iter()
                    .filter(|group| !spec.collapsed.contains(&group.key))
                    .flat_map(|group| group.cards.iter())
                    .filter(|&&index| seen.insert(index))
                    .map(|&index| detail.cards[index].id.clone())
                    .collect()
            }
            Layout::Table => self
                .visible_order()
                .into_iter()
                .map(|index| detail.cards[index].id.clone())
                .collect(),
        }
    }

    /// The selected cards, shown ones first in display order.
    fn selected_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .display_order()
            .into_iter()
            .filter(|id| self.selection.ids.contains(id))
            .collect();
        let mut hidden: Vec<String> = self
            .selection
            .ids
            .iter()
            .filter(|id| !ids.contains(id))
            .cloned()
            .collect();
        hidden.sort();
        ids.extend(hidden);
        ids
    }

    /// A click on a card: ⌘/Ctrl toggles it, Shift selects a range, a plain
    /// click opens it.
    pub(crate) fn card_click(
        &mut self,
        card_id: &str,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        if modifiers.secondary() {
            self.toggle_select(card_id, cx);
        } else if modifiers.shift {
            self.select_range(card_id, cx);
        } else {
            self.open_card_detail(card_id.to_string(), window, cx);
        }
    }

    pub(crate) fn toggle_select(&mut self, card_id: &str, cx: &mut Context<Self>) {
        if !self.selection.ids.remove(card_id) {
            self.selection.ids.insert(card_id.to_string());
        }
        self.selection.anchor = Some(card_id.to_string());
        cx.notify();
    }

    /// Select the cards from the anchor to `card_id` in the current order. With
    /// no anchor, or one no longer shown, only `card_id` is selected.
    pub(crate) fn select_range(&mut self, card_id: &str, cx: &mut Context<Self>) {
        let order = self.display_order();
        let to = order.iter().position(|id| id == card_id);
        let from = self
            .selection
            .anchor
            .as_ref()
            .and_then(|anchor| order.iter().position(|id| id == anchor));
        match (from, to) {
            (Some(from), Some(to)) => {
                let (low, high) = (from.min(to), from.max(to));
                self.selection.ids = order[low..=high].iter().cloned().collect();
            }
            _ => {
                self.selection.ids = HashSet::from([card_id.to_string()]);
                self.selection.anchor = Some(card_id.to_string());
            }
        }
        cx.notify();
    }

    /// Escape's last layer. True when there was a selection to clear.
    pub(crate) fn clear_selection(&mut self) -> bool {
        self.selection.anchor = None;
        let had = !self.selection.ids.is_empty();
        self.selection.ids.clear();
        had
    }

    /// Forget selected cards that left the board (archived, deleted, moved to
    /// another board), that the search or filter now hides, or that sit in a
    /// collapsed column or list group, so the bar and every bulk action cover
    /// only what is on screen (the display order).
    pub(super) fn prune_selection(&mut self) {
        let Some(detail) = &self.detail else {
            return;
        };
        if self.selection.ids.is_empty() && self.selection.anchor.is_none() {
            return;
        }
        let live: HashSet<&str> = detail.cards.iter().map(|card| card.id.as_str()).collect();
        let shown: HashSet<String> = self.display_order().into_iter().collect();
        self.selection.ids.retain(|id| shown.contains(id));
        if let Some(anchor) = &self.selection.anchor
            && !live.contains(anchor.as_str())
        {
            self.selection.anchor = None;
        }
    }

    pub(super) fn bulk_priority(&mut self, priority: u8, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        if !ids.is_empty() {
            self.write(cx, |service| service.set_priority_many(&ids, priority));
        }
    }

    /// Add the label to every selected card, or take it off when they all
    /// have it already.
    pub(super) fn bulk_label(&mut self, label_id: &str, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        let all = ids.iter().all(|id| {
            self.extras
                .get(id)
                .is_some_and(|extras| extras.labels.iter().any(|l| l == label_id))
        });
        self.write(cx, |service| service.set_label_many(&ids, label_id, !all));
    }

    pub(super) fn bulk_move(&mut self, column_id: &str, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        let (places, numbers) = self.places_of(&ids);
        let label = moved_label(&numbers, &self.column_name(column_id));
        self.commit_pending(cx);
        if self.write(cx, |service| {
            service.move_many(&ids, column_id, usize::MAX, "user")
        }) {
            self.clear_selection();
            self.push_undo(label, Undo::MoveMany(places), cx);
        }
    }

    pub(super) fn bulk_archive(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        let (places, numbers) = self.places_of(&ids);
        self.commit_pending(cx);
        if self.write(cx, |service| service.archive_many(&ids)) {
            self.clear_selection();
            self.push_undo(archived_label(&numbers), Undo::Archive(places), cx);
        }
    }

    /// A held delete of every selected card: archived now, deleted when the
    /// toast goes, brought back by Undo.
    pub(crate) fn bulk_delete(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        let (places, numbers) = self.places_of(&ids);
        self.commit_pending(cx);
        if self.write(cx, |service| service.archive_many(&ids)) {
            self.clear_selection();
            self.push_undo(deleted_label(&numbers), Undo::DeleteMany(places), cx);
        }
    }

    pub(super) fn open_pop_menu(
        &mut self,
        kind: MenuKind,
        items: Vec<MenuItem>,
        title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return;
        }
        self.menu = None;
        let menu = cx.new(|cx| Menu::new(title, items, window, cx));
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &MenuEvent, window, cx| {
                if this.pop.take().is_none() {
                    return;
                }
                if let MenuEvent::Picked(id, _) = event {
                    this.pick_pop(kind, id, cx);
                }
                // Back to the board so Escape keeps working from here.
                window.focus(&this.focus, cx);
                cx.notify();
            },
        );
        self.pop = Some(PopMenu {
            kind,
            menu,
            _events: events,
        });
        cx.notify();
    }

    fn pick_pop(&mut self, kind: MenuKind, id: &str, cx: &mut Context<Self>) {
        match kind {
            MenuKind::BulkStatus => {
                if let Some(column_id) = id.strip_prefix("status-") {
                    self.bulk_move(column_id, cx);
                }
            }
            MenuKind::BulkPriority => {
                if let Some(priority) = parse_priority(id) {
                    self.bulk_priority(priority, cx);
                }
            }
            MenuKind::BulkLabels => {
                if let Some(label_id) = id.strip_prefix("label-") {
                    self.bulk_label(label_id, cx);
                }
            }
            MenuKind::TableColumns => {
                if let Some(column) = id.strip_prefix("column-") {
                    self.toggle_table_column(column, cx);
                }
            }
            MenuKind::GroupBy => {
                if let Some(key) = id.strip_prefix("group-") {
                    self.set_group_by(key, cx);
                }
            }
        }
    }

    fn open_bulk_menu(&mut self, kind: MenuKind, window: &mut Window, cx: &mut Context<Self>) {
        let (items, title) = match kind {
            MenuKind::BulkStatus => {
                let columns = self.detail.iter().flat_map(|detail| detail.columns.iter());
                let items = columns
                    .map(|column| {
                        MenuAction::new(format!("status-{}", column.id), column.name.clone()).into()
                    })
                    .collect();
                (items, "Status")
            }
            MenuKind::BulkPriority => (priority_items(&self.priority), "Priority"),
            _ => {
                // Ticked when every selected card has the label.
                let ids = self.selected_ids();
                let on: Vec<String> = self
                    .labels
                    .iter()
                    .filter(|label| {
                        !ids.is_empty()
                            && ids.iter().all(|id| {
                                self.extras
                                    .get(id)
                                    .is_some_and(|e| e.labels.contains(&label.id))
                            })
                    })
                    .map(|label| label.id.clone())
                    .collect();
                (label_items(&self.labels, &on), "Labels")
            }
        };
        self.open_pop_menu(kind, items, title, window, cx);
    }

    pub(super) fn render_pop_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.pop.as_ref()?;
        let anchor = Anchor::Slot(self.pop_anchors.of(open.kind));
        Some(
            menu_popover(&open.menu, anchor, cx)
                .solid()
                .into_any_element(),
        )
    }

    /// The floating bar: "N selected" and what to do with them.
    pub(super) fn render_selection_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.selected_count();
        if count == 0 {
            return None;
        }
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .relative()
                .role(Role::Button)
                .aria_label(label)
                .h(px(24.))
                .px(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(content(0.8))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.08)))
                .child(label)
        };
        let menu_button = |id: &'static str, label: &'static str, kind: MenuKind| {
            button(id, label)
                .child(self.pop_anchors.of(kind).probe())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.open_bulk_menu(kind, window, cx)),
                )
        };
        let summary: SharedString = format!("{count} selected").into();
        // Above the undo toast when one is showing.
        let bottom = if self.undo.toast.is_some() { 64. } else { 16. };
        Some(
            div()
                .absolute()
                .bottom(px(bottom))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    crate::notifications::surface::toast_frame()
                        .id("board-bulk-bar")
                        .debug_selector(|| "board-bulk-bar".into())
                        .role(Role::Toolbar)
                        .aria_label("Selected cards")
                        .px(px(10.))
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .id("board-bulk-count")
                                .debug_selector(|| "board-bulk-count".into())
                                .pr(px(6.))
                                .text_size(px(12.))
                                .text_color(content(0.9))
                                .child(summary),
                        )
                        .child(menu_button(
                            "board-bulk-status",
                            "Status…",
                            MenuKind::BulkStatus,
                        ))
                        .child(menu_button(
                            "board-bulk-priority",
                            "Priority…",
                            MenuKind::BulkPriority,
                        ))
                        .child(menu_button(
                            "board-bulk-labels",
                            "Labels…",
                            MenuKind::BulkLabels,
                        ))
                        .child(
                            button("board-bulk-archive", "Archive")
                                .on_click(cx.listener(|this, _, _, cx| this.bulk_archive(cx))),
                        )
                        .child(
                            button("board-bulk-delete", "Delete")
                                .text_color(diff_foreground(false))
                                .on_click(cx.listener(|this, _, _, cx| this.bulk_delete(cx))),
                        )
                        .child(button("board-bulk-clear", "Clear").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.clear_selection();
                                cx.notify();
                            },
                        ))),
                )
                .into_any_element(),
        )
    }
}
