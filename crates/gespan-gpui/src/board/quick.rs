//! Quick edits for priority, due date and labels. The menus are built here once
//! and used by both the task drawer and the card face. On the face they open
//! in place, anchored to the card.
use super::{BoardView, CardMenu, detail::Picker};
use crate::popover::{Menu, MenuAction, MenuEvent, MenuItem};
use chrono::NaiveDate;
use gespan_core::board::extras::{Label, PRIORITIES};
use gpui::{Context, Window, prelude::*};

/// Which quick-edit menu a card face opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    Priority,
    Due,
    Labels,
}

/// Urgent first, then down to "No priority".
pub(crate) fn priority_items() -> Vec<MenuItem> {
    (0..PRIORITIES.len())
        .rev()
        .map(|p| MenuAction::new(format!("priority-{p}"), PRIORITIES[p]).into())
        .collect()
}

pub(crate) fn parse_priority(id: &str) -> Option<u8> {
    let priority: u8 = id.strip_prefix("priority-")?.parse().ok()?;
    ((priority as usize) < PRIORITIES.len()).then_some(priority)
}

pub(crate) fn due_items() -> Vec<MenuItem> {
    vec![
        MenuAction::new("due-today", "Today").into(),
        MenuAction::new("due-tomorrow", "Tomorrow").into(),
        MenuAction::new("due-next-week", "Next week").into(),
        MenuAction::new("due-pick", "Pick a date…").into(),
        MenuItem::Separator,
        MenuAction::new("due-clear", "Clear").into(),
    ]
}

/// What a due menu row asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DueChoice {
    Day(NaiveDate),
    Clear,
    Pick,
}

pub(crate) fn parse_due(id: &str, today: NaiveDate) -> Option<DueChoice> {
    match id {
        "due-today" => Some(DueChoice::Day(today)),
        // A date past chrono's range is ignored, never a clear.
        "due-tomorrow" => today.succ_opt().map(DueChoice::Day),
        "due-next-week" => today
            .checked_add_days(chrono::Days::new(7))
            .map(DueChoice::Day),
        "due-clear" => Some(DueChoice::Clear),
        "due-pick" => Some(DueChoice::Pick),
        _ => None,
    }
}

/// One row per board label, ticked when the card has it.
pub(crate) fn label_items(labels: &[Label], selected: &[String]) -> Vec<MenuItem> {
    labels
        .iter()
        .map(|label| {
            MenuAction::new(format!("label-{}", label.id), label.name.clone())
                .checked(selected.contains(&label.id))
                .into()
        })
        .collect()
}

/// `labels` with `label_id` added, or removed when it is already there.
pub(crate) fn toggled(labels: &[String], label_id: &str) -> Vec<String> {
    let mut next = labels.to_vec();
    match next.iter().position(|l| l == label_id) {
        Some(index) => {
            next.remove(index);
        }
        None => next.push(label_id.to_string()),
    }
    next
}

/// How a column is reordered once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sort {
    Priority,
    DueDate,
}

/// A card as sorting sees it: its id, priority and due date.
pub(crate) type SortKey = (String, u8, Option<i64>);

/// The ids of `cards` (in their current order) reordered by `sort`. Urgent
/// comes first and the soonest due date first; ties keep their order, and a
/// card with no due date goes last.
pub(crate) fn sorted_ids(cards: &[SortKey], sort: Sort) -> Vec<String> {
    let mut cards: Vec<&SortKey> = cards.iter().collect();
    match sort {
        Sort::Priority => cards.sort_by_key(|(_, priority, _)| std::cmp::Reverse(*priority)),
        Sort::DueDate => cards.sort_by_key(|(_, _, due)| (due.is_none(), due.unwrap_or(0))),
    }
    cards.into_iter().map(|(id, _, _)| id.clone()).collect()
}

impl BoardView {
    /// Reorder the whole column, hidden cards included, in one write.
    pub(crate) fn sort_column(&mut self, column_id: String, sort: Sort, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(detail) = &self.detail else {
            return;
        };
        let mut cards: Vec<_> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .collect();
        cards.sort_by_key(|card| card.position);
        let keys: Vec<SortKey> = cards
            .iter()
            .map(|card| {
                let extras = self.extras.get(&card.id);
                (
                    card.id.clone(),
                    extras.map_or(0, |e| e.priority),
                    extras.and_then(|e| e.due_at),
                )
            })
            .collect();
        let current: Vec<String> = keys.iter().map(|(id, ..)| id.clone()).collect();
        let sorted = sorted_ids(&keys, sort);
        if sorted == current {
            cx.notify();
            return;
        }
        self.write(cx, |service| {
            service.move_many(&sorted, &column_id, 0, "user")
        });
    }

    /// The first column with the done role.
    pub(crate) fn done_column_id(&self) -> Option<String> {
        self.detail
            .as_ref()?
            .columns
            .iter()
            .find(|column| column.role.as_deref() == Some("done"))
            .map(|column| column.id.clone())
    }

    /// The live cards of a column, top to bottom, hidden by no filter.
    fn column_card_ids(&self, column_id: &str) -> Vec<String> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let mut cards: Vec<_> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .collect();
        cards.sort_by_key(|card| card.position);
        cards.into_iter().map(|card| card.id.clone()).collect()
    }

    /// Move every live card of a column to the end of the done column.
    pub(crate) fn mark_all_done(&mut self, column_id: String, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(done) = self.done_column_id().filter(|done| *done != column_id) else {
            return;
        };
        let ids = self.column_card_ids(&column_id);
        if ids.is_empty() {
            cx.notify();
            return;
        }
        self.write(cx, |service| {
            service.move_many(&ids, &done, usize::MAX, "user")
        });
    }

    /// Archive every live card of the done column.
    pub(crate) fn archive_completed(&mut self, column_id: String, cx: &mut Context<Self>) {
        self.menu = None;
        if self.done_column_id().as_deref() != Some(column_id.as_str()) {
            return;
        }
        let ids = self.column_card_ids(&column_id);
        if ids.is_empty() {
            cx.notify();
            return;
        }
        self.write(cx, |service| service.archive_many(&ids));
    }

    /// Collapse or expand a column, keeping its colour and WIP limit.
    pub(crate) fn set_collapsed(
        &mut self,
        column_id: String,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let Some(column) = self.column(&column_id).cloned() else {
            return;
        };
        if collapsed
            && self
                .composer
                .as_ref()
                .is_some_and(|composer| composer.column_id == column_id)
        {
            self.composer = None;
        }
        self.write(cx, |service| {
            service.set_column_display(
                &column_id,
                column.color.as_deref(),
                column.wip_limit,
                collapsed,
            )
        });
    }

    pub(crate) fn is_collapsed(&self, column_id: &str) -> bool {
        self.column(column_id)
            .is_some_and(|column| column.collapsed)
    }

    /// Open a quick-edit menu on a card face. The click never opens the card.
    pub(crate) fn open_face_menu(
        &mut self,
        card_id: String,
        face: Face,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dragging {
            return;
        }
        let items = match face {
            Face::Priority => priority_items(),
            Face::Due => due_items(),
            Face::Labels => label_items(&self.labels, &self.card_labels(&card_id)),
        };
        if items.is_empty() {
            return;
        }
        let title = match face {
            Face::Priority => "Priority",
            Face::Due => "Due date",
            Face::Labels => "Labels",
        };
        self.menu = None;
        let menu = cx.new(|cx| Menu::new(title, items, window, cx));
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &MenuEvent, window, cx| {
                let Some(open) = this.card_menu.take() else {
                    return;
                };
                if let MenuEvent::Picked(id, _) = event {
                    this.apply_face(face, id, &open.card_id, window, cx);
                }
                cx.notify();
            },
        );
        self.card_menu = Some(CardMenu {
            card_id,
            menu,
            _events: events,
        });
        cx.notify();
    }

    fn apply_face(
        &mut self,
        face: Face,
        id: &str,
        card_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let card_id = card_id.to_string();
        match face {
            Face::Priority => {
                if let Some(priority) = parse_priority(id) {
                    self.write(cx, |service| service.set_priority(&card_id, priority));
                }
            }
            Face::Due => match parse_due(id, chrono::Local::now().date_naive()) {
                Some(DueChoice::Day(day)) => self.set_due_day(&card_id, day, cx),
                Some(DueChoice::Clear) => {
                    self.write(cx, |service| service.set_due(&card_id, None));
                }
                // A typed date needs the drawer's field.
                Some(DueChoice::Pick) => {
                    self.open_card_detail(card_id, window, cx);
                    self.open_picker(Picker::DueDate, window, cx);
                }
                None => {}
            },
            Face::Labels => {
                if let Some(label_id) = id.strip_prefix("label-") {
                    let labels = toggled(&self.card_labels(&card_id), label_id);
                    self.write(cx, |service| service.set_labels(&card_id, &labels));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn priority_ids_parse_only_inside_the_range() {
        assert_eq!(parse_priority("priority-0"), Some(0));
        assert_eq!(parse_priority("priority-4"), Some(4));
        assert_eq!(parse_priority("priority-5"), None);
        assert_eq!(parse_priority("priority-x"), None);
        assert_eq!(parse_priority("due-today"), None);
    }

    #[test]
    fn due_ids_resolve_relative_to_today() {
        let today = day(2026, 10, 26);
        assert_eq!(
            parse_due("due-tomorrow", today),
            Some(DueChoice::Day(day(2026, 10, 27)))
        );
        assert_eq!(
            parse_due("due-next-week", today),
            Some(DueChoice::Day(day(2026, 11, 2)))
        );
        assert_eq!(parse_due("due-clear", today), Some(DueChoice::Clear));
        assert_eq!(parse_due("nope", today), None);
        assert_eq!(parse_due("due-tomorrow", NaiveDate::MAX), None);
    }

    fn key(id: &str, priority: u8, due: Option<i64>) -> SortKey {
        (id.into(), priority, due)
    }

    #[test]
    fn priority_sort_puts_urgent_first_and_keeps_ties_in_order() {
        let cards = [
            key("a", 1, None),
            key("b", 4, None),
            key("c", 0, None),
            key("d", 4, None),
            key("e", 1, None),
        ];
        assert_eq!(
            sorted_ids(&cards, Sort::Priority),
            ["b", "d", "a", "e", "c"]
        );
    }

    #[test]
    fn due_sort_is_soonest_first_with_undated_last_in_order() {
        let cards = [
            key("a", 0, None),
            key("b", 0, Some(500)),
            key("c", 0, Some(100)),
            key("d", 0, None),
            key("e", 0, Some(100)),
        ];
        assert_eq!(sorted_ids(&cards, Sort::DueDate), ["c", "e", "b", "a", "d"]);
    }

    #[test]
    fn toggling_a_label_adds_then_removes_it() {
        let on = toggled(&["a".into()], "b");
        assert_eq!(on, ["a", "b"]);
        assert_eq!(toggled(&on, "a"), ["b"]);
    }
}
