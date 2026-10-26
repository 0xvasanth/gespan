//! The List layout, and the layout state List and Table share.
//!
//! What a view's layout carries (which layout, how the list groups, which
//! groups are collapsed, the table's columns and widths) is the view's
//! `layout_json`, so it marks the view unsaved and is saved with it. Rows come
//! from the query's order and are virtualised: only the rows in view render.
use super::{
    BoardView,
    selection::MenuKind,
    style::{label_chip, local_day, priority_glyph},
};
use crate::{popover::MenuAction, theme::*};
use chrono::NaiveDate;
use gespan_core::board::extras::PRIORITIES;
use gpui::{
    AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, Role, Stateful, div, prelude::*,
    px, uniform_list,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, rc::Rc};

pub(super) const ROW_HEIGHT: f32 = 32.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    Board,
    List,
    Table,
}

impl Layout {
    const ALL: [Layout; 3] = [Layout::Board, Layout::List, Layout::Table];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Layout::Board => "board",
            Layout::List => "list",
            Layout::Table => "table",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Layout::Board => "Board",
            Layout::List => "List",
            Layout::Table => "Table",
        }
    }

    fn parse(id: &str) -> Layout {
        match id {
            "list" => Layout::List,
            "table" => Layout::Table,
            _ => Layout::Board,
        }
    }
}

fn board_layout() -> String {
    "board".into()
}

/// A view's `layout_json`. Keys this build doesn't know are kept as they are.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub(super) struct LayoutSpec {
    #[serde(default = "board_layout")]
    pub layout: String,
    #[serde(rename = "groupBy", skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collapsed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub widths: BTreeMap<String, i64>,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

impl Default for LayoutSpec {
    fn default() -> Self {
        Self {
            layout: board_layout(),
            group_by: None,
            collapsed: Vec::new(),
            columns: None,
            widths: BTreeMap::new(),
            rest: serde_json::Map::new(),
        }
    }
}

/// How the list groups: (id, label).
pub(super) const GROUP_BYS: [(&str, &str); 5] = [
    ("status", "Status"),
    ("priority", "Priority"),
    ("label", "Label"),
    ("project", "Project"),
    ("due", "Due date"),
];

fn group_label(group_by: &str) -> &'static str {
    GROUP_BYS
        .iter()
        .find(|(id, _)| *id == group_by)
        .map_or("Status", |(_, label)| label)
}

/// A group of the list: its key (what "collapsed" remembers), its heading and
/// the cards in it, as indices into the board's cards.
pub(super) struct Group {
    pub key: String,
    pub label: String,
    pub cards: Vec<usize>,
}

/// One row of the list.
pub(super) enum ListRow {
    Header {
        index: usize,
        key: String,
        label: String,
        count: usize,
        collapsed: bool,
    },
    /// A card (index into the board's cards) and the group showing it.
    Card(usize, usize),
}

fn due_bucket(due_at: Option<i64>, done: bool, today: NaiveDate) -> (&'static str, &'static str) {
    let Some(day) = due_at.and_then(local_day) else {
        return ("none", "No due date");
    };
    let days = (day - today).num_days();
    match days {
        d if d < 0 && !done => ("overdue", "Overdue"),
        d if d < 0 => ("earlier", "Earlier"),
        0 => ("today", "Today"),
        1..=7 => ("next7", "Next 7 days"),
        _ => ("later", "Later"),
    }
}

impl BoardView {
    pub(super) fn layout_spec(&self) -> LayoutSpec {
        serde_json::from_str(&self.views.layout).unwrap_or_default()
    }

    pub(crate) fn layout_kind(&self) -> Layout {
        Layout::parse(&self.layout_spec().layout)
    }

    /// Change the layout state and store it in the view's `layout_json`.
    pub(super) fn edit_layout(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut LayoutSpec),
    ) {
        let mut spec = self.layout_spec();
        change(&mut spec);
        if let Ok(json) = serde_json::to_string(&spec) {
            self.views.layout = json;
        }
        cx.notify();
    }

    pub(crate) fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.pop = None;
        self.table_edit = None;
        self.resizing = None;
        self.edit_layout(cx, |spec| spec.layout = layout.id().to_string());
    }

    pub(super) fn set_group_by(&mut self, key: &str, cx: &mut Context<Self>) {
        if GROUP_BYS.iter().any(|(id, _)| *id == key) {
            let key = key.to_string();
            self.edit_layout(cx, |spec| spec.group_by = Some(key));
        }
    }

    fn toggle_group(&mut self, key: &str, cx: &mut Context<Self>) {
        let key = key.to_string();
        self.edit_layout(cx, |spec| {
            match spec.collapsed.iter().position(|k| *k == key) {
                Some(at) => {
                    spec.collapsed.remove(at);
                }
                None => spec.collapsed.push(key),
            }
        });
    }

    /// The cards the search and filter show, as indices into the board's
    /// cards, in the query's order (the columns' own when nothing sorts).
    pub(super) fn visible_order(&self) -> Vec<usize> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let columns: std::collections::HashMap<&str, usize> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(at, column)| (column.id.as_str(), at))
            .collect();
        let mut order: Vec<usize> = (0..detail.cards.len())
            .filter(|&at| self.card_matches(&detail.cards[at]))
            .collect();
        order.sort_by_key(|&at| {
            let card = &detail.cards[at];
            (
                self.rank.get(&card.id).copied().unwrap_or(usize::MAX),
                columns
                    .get(card.column_id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX),
                card.position,
            )
        });
        order
    }

    /// The list's groups, empty ones left out, each in the visible order.
    pub(super) fn groups(&self, spec: &LayoutSpec) -> Vec<Group> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let order = self.visible_order();
        let by = spec.group_by.as_deref().unwrap_or("status");
        let today = chrono::Local::now().date_naive();
        let done = |at: usize| {
            self.column(&detail.cards[at].column_id)
                .is_some_and(|column| column.role.as_deref() == Some("done"))
        };
        let extras = |at: usize| self.extras.get(&detail.cards[at].id);
        // Every group there could be, in display order, with the keys each
        // card belongs to; "none" groups close every list but priority.
        let mut heads: Vec<(String, String)> = match by {
            "priority" => (0..PRIORITIES.len())
                .rev()
                .map(|p| (format!("p{p}"), PRIORITIES[p].to_string()))
                .collect(),
            "label" => self
                .labels
                .iter()
                .map(|label| (label.id.clone(), label.name.clone()))
                .chain([("none".into(), "No label".into())])
                .collect(),
            "project" => self
                .project_choices()
                .into_iter()
                .chain([("none".into(), "No project".into())])
                .collect(),
            "due" => [
                ("overdue", "Overdue"),
                ("today", "Today"),
                ("next7", "Next 7 days"),
                ("later", "Later"),
                ("earlier", "Earlier"),
                ("none", "No due date"),
            ]
            .map(|(key, label)| (key.to_string(), label.to_string()))
            .into(),
            _ => detail
                .columns
                .iter()
                .map(|column| (column.id.clone(), column.name.clone()))
                .chain([("none".into(), "No status".into())])
                .collect(),
        };
        let keys = |at: usize| -> Vec<String> {
            let card = &detail.cards[at];
            match by {
                "priority" => vec![format!("p{}", extras(at).map_or(0, |e| e.priority))],
                "label" => match extras(at).map(|e| e.labels.clone()).unwrap_or_default() {
                    labels if labels.is_empty() => vec!["none".into()],
                    labels => labels,
                },
                "project" => match extras(at).map(|e| e.projects.clone()).unwrap_or_default() {
                    projects if projects.is_empty() => vec!["none".into()],
                    projects => projects,
                },
                "due" => vec![
                    due_bucket(extras(at).and_then(|e| e.due_at), done(at), today)
                        .0
                        .to_string(),
                ],
                _ => vec![card.column_id.clone()],
            }
        };
        let mut groups: Vec<Group> = heads
            .drain(..)
            .map(|(key, label)| Group {
                // Keys carry the mode (what the Tauri app stores too), so a
                // collapsed "No label" never matches "No project".
                key: format!("{by}:{key}"),
                label,
                cards: Vec::new(),
            })
            .collect();
        let none = format!("{by}:none");
        for at in order {
            let raw = keys(at);
            let mut found = false;
            for key in &raw {
                let key = format!("{by}:{key}");
                if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                    group.cards.push(at);
                    found = true;
                }
            }
            // Nothing the board still lists (a deleted label, an unknown
            // column): the card goes in the catch-all group, never lost.
            if !found && let Some(group) = groups.iter_mut().find(|group| group.key == none) {
                group.cards.push(at);
            }
        }
        groups.retain(|group| !group.cards.is_empty());
        groups
    }

    fn list_rows(&self, spec: &LayoutSpec) -> Vec<ListRow> {
        let mut rows = Vec::new();
        for (index, group) in self.groups(spec).into_iter().enumerate() {
            let collapsed = spec.collapsed.contains(&group.key);
            rows.push(ListRow::Header {
                index,
                key: group.key,
                label: group.label,
                count: group.cards.len(),
                collapsed,
            });
            if !collapsed {
                rows.extend(group.cards.into_iter().map(|at| ListRow::Card(at, index)));
            }
        }
        rows
    }

    /// The row every card shows in the list and the table: selectable,
    /// openable, with the card's right-click menu.
    pub(super) fn row_shell(
        &self,
        card: &gespan_core::board::Card,
        scope: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui::Div> {
        let number = card.number;
        let selected = self.is_selected(&card.id);
        let (click_id, menu_id) = (card.id.clone(), card.id.clone());
        div()
            .id(gpui::SharedString::from(format!(
                "board-card-{scope}-{number}"
            )))
            .debug_selector(move || format!("board-card-{number}"))
            .role(Role::ListItem)
            .aria_label(card.title.clone())
            .aria_selected(selected)
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(content(0.05))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(|s| s.bg(content(0.04)))
            .when(selected, |d| d.bg(accent_color().opacity(0.14)))
            .on_click(cx.listener(move |this, event, window, cx| {
                this.card_click(&click_id, event, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_more_menu(
                        menu_id.clone(),
                        crate::popover::Anchor::Point(event.position),
                        window,
                        cx,
                    )
                }),
            )
    }

    fn render_list_row(
        &self,
        row: &ListRow,
        spec: &LayoutSpec,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = &self.detail else {
            return div().into_any_element();
        };
        match row {
            ListRow::Header {
                index,
                key,
                label,
                count,
                collapsed,
            } => {
                let key = key.clone();
                let index = *index;
                div()
                    .id(("board-list-group", index))
                    .debug_selector(move || format!("board-list-group-{index}"))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .h(px(ROW_HEIGHT))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(content(0.03))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(0.75))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child(
                        icon(
                            if *collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                        )
                        .text_color(content(0.5)),
                    )
                    .child(label.clone())
                    .child(div().text_color(content(0.4)).child(count.to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(&key, cx)))
                    .into_any_element()
            }
            ListRow::Card(at, group) => {
                let Some(card) = detail.cards.get(*at) else {
                    return div().into_any_element();
                };
                let extras = self.extras.get(&card.id).cloned().unwrap_or_default();
                let done = self
                    .column(&card.column_id)
                    .is_some_and(|column| column.role.as_deref() == Some("done"));
                let now = gespan_core::time::now_ms();
                let due = extras
                    .due_at
                    .and_then(|at| super::style::due_label(at, now as i64));
                let by_status = spec.group_by.as_deref().unwrap_or("status") == "status";
                self.row_shell(card, *group, cx)
                    .child(div().w(px(18.)).child(priority_glyph(extras.priority)))
                    .child(
                        div()
                            .w(px(44.))
                            .font_family("Menlo")
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(format!("#{}", card.number)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(content(if done { 0.6 } else { 0.92 }))
                            .child(card.title.clone()),
                    )
                    .children(extras.labels.iter().filter_map(|id| {
                        let label = self.labels.iter().find(|label| &label.id == id)?;
                        Some(label_chip(&label.name, &label.color))
                    }))
                    .when(!by_status, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(content(0.5))
                                .child(self.column_name(&card.column_id)),
                        )
                    })
                    .children(due.map(|(text, overdue)| {
                        div()
                            .w(px(70.))
                            .text_size(px(11.))
                            .text_color(if overdue && !done {
                                diff_foreground(false)
                            } else {
                                content(0.5)
                            })
                            .child(text)
                    }))
                    .child(
                        div()
                            .w(px(60.))
                            .text_size(px(11.))
                            .text_color(content(0.35))
                            .child(gespan_core::time::relative(
                                card.updated_at.max(0) as u64,
                                now,
                            )),
                    )
                    .into_any_element()
            }
        }
    }

    /// The layout buttons at the end of the view strip.
    pub(super) fn render_layout_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.layout_kind();
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .children(Layout::ALL.map(|layout| {
                let id = format!("board-layout-{}", layout.id());
                let selector = id.clone();
                div()
                    .id(gpui::SharedString::from(id))
                    .debug_selector(move || selector.clone())
                    .role(Role::Button)
                    .aria_label(layout.label())
                    .aria_toggled(if layout == current {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .h(px(22.))
                    .px(px(8.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .text_size(px(11.))
                    .text_color(content(if layout == current { 0.95 } else { 0.55 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.08)))
                    .when(layout == current, |d| d.bg(content(0.07)))
                    .child(layout.label())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)))
            }))
    }

    pub(super) fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let spec = self.layout_spec();
        let rows = Rc::new(self.list_rows(&spec));
        let group_by = spec.group_by.clone().unwrap_or_else(|| "status".into());
        let spec = Rc::new(spec);
        let entity = cx.entity();
        let items: Vec<_> = GROUP_BYS
            .iter()
            .map(|(id, label)| {
                MenuAction::new(format!("group-{id}"), *label)
                    .checked(*id == group_by)
                    .into()
            })
            .collect();
        let count = rows.len();
        div()
            .id("board-list")
            .debug_selector(|| "board-list".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(content(0.07))
                    .child(
                        div()
                            .id("board-list-group-by")
                            .debug_selector(|| "board-list-group-by".into())
                            .relative()
                            .role(Role::Button)
                            .aria_label("Group by")
                            .h(px(22.))
                            .px(px(8.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(content(0.7))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.08)))
                            .child(self.pop_anchors.group.probe())
                            .child(format!("Group by {}", group_label(&group_by)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let items = items.clone();
                                this.open_pop_menu(MenuKind::GroupBy, items, "Group by", window, cx)
                            })),
                    ),
            )
            .child(
                uniform_list("board-list-rows", count, move |range, _, cx| {
                    entity.update(cx, |this, cx| {
                        range
                            .filter_map(|at| rows.get(at))
                            .map(|row| this.render_list_row(row, &spec, cx))
                            .collect::<Vec<_>>()
                    })
                })
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_layout_is_the_stored_default() {
        assert_eq!(
            serde_json::to_string(&LayoutSpec::default()).unwrap(),
            super::super::views::DEFAULT_LAYOUT
        );
        // Nothing stored, or stored badly, shows the board.
        assert_eq!(
            serde_json::from_str::<LayoutSpec>("{}").unwrap().layout,
            "board"
        );
        assert_eq!(Layout::parse("nonsense"), Layout::Board);
        assert_eq!(Layout::parse("table"), Layout::Table);
    }

    #[test]
    fn layout_state_round_trips_and_keeps_keys_it_does_not_know() {
        let stored = r#"{"layout":"table","columns":["title","due"],"widths":{"title":220},"collapsed":["label:a"],"future":{"x":1}}"#;
        let spec: LayoutSpec = serde_json::from_str(stored).unwrap();
        assert_eq!(spec.widths["title"], 220);
        let again: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(
            again,
            serde_json::from_str::<serde_json::Value>(stored).unwrap()
        );
    }

    #[test]
    fn due_buckets_follow_local_days_and_done_cards_are_never_overdue() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let at = |days: i64| {
            let day = today + chrono::Duration::days(days);
            Some(super::super::style::day_start(day).unwrap())
        };
        assert_eq!(due_bucket(None, false, today).0, "none");
        assert_eq!(due_bucket(at(-1), false, today).0, "overdue");
        assert_eq!(due_bucket(at(-1), true, today).0, "earlier");
        assert_eq!(due_bucket(at(0), false, today).0, "today");
        assert_eq!(due_bucket(at(7), false, today).0, "next7");
        assert_eq!(due_bucket(at(8), false, today).0, "later");
    }
}
