//! Quick edits for priority, due date and labels. The menus are built here once
//! and used by both the task drawer and the card face. On the face they open
//! in place, anchored to the card.
use super::{
    BoardView, CardMenu,
    detail::Picker,
    undo::{Undo, archived_label, moved_label},
};
use crate::popover::{Menu, MenuAction, MenuEvent, MenuItem};
use chrono::NaiveDate;
use gespan_core::board::extras::{ColumnDisplayPatch, Label, PRIORITIES, PriorityScheme};
use gpui::{Context, Window, prelude::*};

/// Which quick-edit menu a card face opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    /// Moves the card to the picked column. Only the table offers it.
    Status,
    Priority,
    Due,
    Labels,
}

/// Urgent first, then down to the lowest level. Hidden levels are left out.
pub(crate) fn priority_items(scheme: &PriorityScheme) -> Vec<MenuItem> {
    (0..PRIORITIES.len())
        .rev()
        .filter(|&p| !scheme.hidden.contains(&(p as u8)))
        .map(|p| MenuAction::new(format!("priority-{p}"), scheme.names[p].clone()).into())
        .collect()
}

pub(crate) fn parse_priority(id: &str) -> Option<u8> {
    let priority: u8 = id.strip_prefix("priority-")?.parse().ok()?;
    ((priority as usize) < PRIORITIES.len()).then_some(priority)
}

pub(crate) fn due_items() -> Vec<MenuItem> {
    date_items("due")
}

/// The date quick picks, with ids like `{prefix}-today`. Due date, start date
/// and date fields share them.
pub(crate) fn date_items(prefix: &str) -> Vec<MenuItem> {
    vec![
        MenuAction::new(format!("{prefix}-today"), "Today").into(),
        MenuAction::new(format!("{prefix}-tomorrow"), "Tomorrow").into(),
        MenuAction::new(format!("{prefix}-next-week"), "Next week").into(),
        MenuAction::new(format!("{prefix}-in-two-weeks"), "In 2 weeks").into(),
        MenuAction::new(format!("{prefix}-pick"), "Pick a date…").into(),
        MenuItem::Separator,
        MenuAction::new(format!("{prefix}-clear"), "Clear").into(),
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
    parse_date("due", id, today)
}

pub(crate) fn parse_date(prefix: &str, id: &str, today: NaiveDate) -> Option<DueChoice> {
    let ahead = |days| {
        // A date past chrono's range is ignored, never a clear.
        today
            .checked_add_days(chrono::Days::new(days))
            .map(DueChoice::Day)
    };
    match id.strip_prefix(prefix)?.strip_prefix('-')? {
        "today" => Some(DueChoice::Day(today)),
        "tomorrow" => ahead(1),
        "next-week" => ahead(7),
        "in-two-weeks" => ahead(14),
        "clear" => Some(DueChoice::Clear),
        "pick" => Some(DueChoice::Pick),
        _ => None,
    }
}

/// Most minutes the store accepts.
const MAX_ESTIMATE: i64 = 6_000_000;

/// The one hint for an estimate that isn't a time, the same in both apps.
pub(crate) const ESTIMATE_HELP: &str = "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h";

/// A number before an `h` or `m`: `1`, `1.5` or `.5`, never `1.`, `1..5`
/// or `1.2.3`.
fn unit_number(text: &str) -> Option<f64> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    let ok = match fraction {
        None => digits(whole),
        Some(fraction) => (whole.is_empty() || digits(whole)) && digits(fraction),
    };
    ok.then(|| text.parse().ok()).flatten()
}

/// An estimate typed as `90`, `90m`, `1h`, `1h 30m`, `1.5h` or `.5h`, in
/// minutes rounded to the minute. Hex, exponents, signs, a bare decimal and
/// a unit with no number are refused with `ESTIMATE_HELP` (the shared rule).
pub(crate) fn parse_estimate(text: &str) -> Result<i64, String> {
    let text = text.trim().to_lowercase();
    let bad = || ESTIMATE_HELP.to_string();
    if text.is_empty() {
        return Err(bad());
    }
    let minutes = if text.chars().all(|c| c.is_ascii_digit()) {
        text.parse::<f64>().map_err(|_| bad())?
    } else {
        let (mut total, mut number, mut seen) = (0f64, String::new(), (false, false));
        for c in text.chars() {
            match c {
                '0'..='9' | '.' => number.push(c),
                'h' | 'm' => {
                    let value = unit_number(&number).ok_or_else(bad)?;
                    number.clear();
                    let slot = if c == 'h' { &mut seen.0 } else { &mut seen.1 };
                    if std::mem::replace(slot, true) {
                        return Err(bad());
                    }
                    total += if c == 'h' { value * 60. } else { value };
                }
                c if c.is_whitespace() => {
                    if !number.is_empty() {
                        return Err(bad());
                    }
                }
                _ => return Err(bad()),
            }
        }
        if !number.is_empty() {
            return Err(bad());
        }
        total
    };
    let minutes = minutes.round();
    if !minutes.is_finite() || minutes < 1. {
        return Err("An estimate must be at least 1 minute".into());
    }
    if minutes > MAX_ESTIMATE as f64 {
        return Err(format!("An estimate can be at most {MAX_ESTIMATE} minutes"));
    }
    Ok(minutes as i64)
}

/// `90` shows as `1h 30m`, `45` as `45m`, `120` as `2h`.
pub(crate) fn format_estimate(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// A typed text or number field value as stored JSON; empty clears it.
pub(crate) fn encode_field(kind: &str, text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match kind {
        "text" => Ok(Some(
            serde_json::Value::String(text.to_string()).to_string(),
        )),
        "number" => {
            // Plain decimals only: an optional minus, then `42`, `2.5` or
            // `.5`. Rust's parsers would also take `+3`, `1e3` and `inf`.
            let unsigned = text.strip_prefix('-').unwrap_or(text);
            if unit_number(unsigned).is_none() {
                return Err(format!("“{text}” is not a number"));
            }
            let number = match text.parse::<i64>() {
                Ok(whole) => serde_json::Number::from(whole),
                Err(_) => text
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .ok_or_else(|| format!("“{text}” is not a number"))?,
            };
            Ok(Some(number.to_string()))
        }
        "url" => {
            if !valid_field_url(text) {
                return Err("A link must start with http:// or https://".into());
            }
            Ok(Some(
                serde_json::Value::String(text.to_string()).to_string(),
            ))
        }
        _ => Err(format!("Can't type a {kind} value")),
    }
}

/// A url field holds an http or https address, at most 2,000 characters.
/// The scheme matches in any case and needs something after it, as the
/// board store checks (`HTTP://x` is fine, `https://  ` is not).
fn valid_field_url(text: &str) -> bool {
    if text.chars().count() > 2_000 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .is_some_and(|rest| !rest.trim().is_empty())
}

/// A multi-select field's stored picks, in the order stored.
pub(crate) fn field_picks(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

/// The choices of a select field, from its `["a","b"]` options.
pub(crate) fn field_options(options_json: Option<&str>) -> Vec<String> {
    options_json
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

/// A stored field value as the drawer shows it. `None` when it has no value
/// or the stored JSON doesn't fit the kind.
pub(crate) fn field_text(kind: &str, raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    match (kind, value) {
        ("text" | "select" | "url", serde_json::Value::String(text)) => Some(text),
        ("multi-select", serde_json::Value::Array(items)) => {
            let picks: Vec<&str> = items.iter().filter_map(|item| item.as_str()).collect();
            (!picks.is_empty()).then(|| picks.join(", "))
        }
        ("number", serde_json::Value::Number(number)) => Some(number.to_string()),
        ("date", serde_json::Value::Number(at)) => {
            let day = super::style::local_day(at.as_i64()?)?;
            Some(day.format("%b %-d, %Y").to_string())
        }
        ("checkbox", serde_json::Value::Bool(on)) => Some(if on { "Yes" } else { "No" }.into()),
        _ => None,
    }
}

/// Whether a checkbox field's stored value is on.
pub(crate) fn field_checked(raw: Option<&str>) -> bool {
    raw.and_then(|raw| serde_json::from_str::<bool>(raw).ok())
        .unwrap_or(false)
}

/// A PR link is an http or https URL with a host.
pub(crate) fn valid_pr_url(text: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        text.strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
    }) && !text.chars().any(char::is_whitespace)
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
        let label = format!("Sorted {}", self.column_name(&column_id));
        self.commit_pending(cx);
        if self.write(cx, |service| {
            service.move_many(&sorted, &column_id, 0, "user")
        }) {
            let undo = Undo::Sort {
                column_id,
                order: current,
            };
            self.push_undo(label, undo, cx);
        }
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
        let (places, numbers) = self.places_of(&ids);
        let label = moved_label(&numbers, &self.column_name(&done));
        self.commit_pending(cx);
        if self.write(cx, |service| {
            service.move_many(&ids, &done, usize::MAX, "user")
        }) {
            self.push_undo(label, Undo::MoveMany(places), cx);
        }
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
        let (places, numbers) = self.places_of(&ids);
        self.commit_pending(cx);
        if self.write(cx, |service| service.archive_many(&ids)) {
            self.push_undo(archived_label(&numbers), Undo::Archive(places), cx);
        }
    }

    /// Collapse or expand a column. Only `collapsed` is written, so a colour
    /// or WIP limit the other app set since the last load stays.
    pub(crate) fn set_collapsed(
        &mut self,
        column_id: String,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        if self.column(&column_id).is_none() {
            return;
        }
        if collapsed
            && self
                .composer
                .as_ref()
                .is_some_and(|composer| composer.column_id == column_id)
        {
            self.composer = None;
        }
        let patch = ColumnDisplayPatch {
            collapsed: Some(collapsed),
            ..Default::default()
        };
        self.write(cx, |service| {
            service.update_column_display(&column_id, &patch).map(drop)
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
        self.open_face_menu_at(card_id, face, None, window, cx)
    }

    /// `open_face_menu` opening at `anchor`, for rows with no card face.
    pub(crate) fn open_face_menu_at(
        &mut self,
        card_id: String,
        face: Face,
        anchor: Option<crate::popover::Anchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dragging {
            return;
        }
        if anchor.is_none() {
            // The menu anchors to the card face: bring the card into view.
            self.reveal_card(&card_id);
        }
        let items = match face {
            Face::Status => self.status_items(&card_id),
            Face::Priority => priority_items(&self.priority),
            Face::Due => due_items(),
            Face::Labels => label_items(&self.labels, &self.card_labels(&card_id)),
        };
        if items.is_empty() {
            return;
        }
        let title = match face {
            Face::Status => "Status",
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
            anchor,
            menu,
            _events: events,
        });
        cx.notify();
    }

    /// One row per column, ticked for the card's own.
    fn status_items(&self, card_id: &str) -> Vec<MenuItem> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let current = detail
            .cards
            .iter()
            .find(|card| card.id == card_id)
            .map(|card| card.column_id.as_str());
        detail
            .columns
            .iter()
            .map(|column| {
                MenuAction::new(format!("status-{}", column.id), column.name.clone())
                    .checked(current == Some(column.id.as_str()))
                    .into()
            })
            .collect()
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
            Face::Status => {
                if let Some(column_id) = id.strip_prefix("status-") {
                    self.move_with_undo(&card_id, column_id, usize::MAX, cx);
                }
            }
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

    #[test]
    fn date_quick_picks_include_in_two_weeks_for_every_prefix() {
        let today = day(2026, 10, 26);
        for prefix in ["due", "start", "field"] {
            let id = format!("{prefix}-in-two-weeks");
            assert_eq!(
                parse_date(prefix, &id, today),
                Some(DueChoice::Day(day(2026, 11, 9)))
            );
            assert_eq!(
                parse_date(prefix, &format!("{prefix}-next-week"), today),
                Some(DueChoice::Day(day(2026, 11, 2)))
            );
        }
        assert_eq!(parse_date("start", "due-today", today), None);
        assert_eq!(
            parse_due("due-in-two-weeks", today),
            Some(DueChoice::Day(day(2026, 11, 9)))
        );
    }

    #[test]
    fn estimate_parses_hours_and_minutes() {
        for (text, minutes) in [
            ("90", 90),
            ("90m", 90),
            ("1h", 60),
            ("2h", 120),
            ("1h 30m", 90),
            ("1h30m", 90),
            (" 1.5h ", 90),
            ("1H 5M", 65),
        ] {
            assert_eq!(parse_estimate(text), Ok(minutes), "{text}");
        }
        for text in [
            "", "0", "0m", "0h", "-5", "abc", "1x", "1h 1h", "1 h 30", "h", "1.5.2h", "6000001",
            "100001h",
        ] {
            assert!(parse_estimate(text).is_err(), "{text:?} should be rejected");
        }
        assert_eq!(parse_estimate("6000000"), Ok(6_000_000));
    }

    #[test]
    fn estimates_display_as_hours_and_minutes() {
        assert_eq!(format_estimate(90), "1h 30m");
        assert_eq!(format_estimate(45), "45m");
        assert_eq!(format_estimate(120), "2h");
        assert_eq!(format_estimate(60), "1h");
        assert_eq!(format_estimate(1), "1m");
    }

    #[test]
    fn field_values_encode_by_kind() {
        assert_eq!(encode_field("text", " hi "), Ok(Some("\"hi\"".into())));
        assert_eq!(encode_field("text", "  "), Ok(None));
        assert_eq!(encode_field("number", "3"), Ok(Some("3".into())));
        assert_eq!(encode_field("number", "3.5"), Ok(Some("3.5".into())));
        assert!(encode_field("number", "abc").is_err());
        assert!(encode_field("number", "NaN").is_err());
        assert!(encode_field("number", "inf").is_err());
        assert_eq!(field_options(Some(r#"["a","b"]"#)), ["a", "b"]);
        assert!(field_options(Some("nope")).is_empty());
        assert_eq!(field_text("checkbox", "true").as_deref(), Some("Yes"));
        assert_eq!(field_text("number", "\"x\""), None);
    }

    #[test]
    fn field_urls_take_the_scheme_in_any_case_like_the_board_store() {
        for good in [
            "https://example.com",
            "http://x",
            "HTTP://x",
            "HTTPS://Example.com/Path",
            "hTtPs://x",
        ] {
            assert!(valid_field_url(good), "{good:?}");
            // What is saved is the text as typed, scheme case and all.
            assert_eq!(
                encode_field("url", good).unwrap(),
                Some(serde_json::Value::String(good.into()).to_string())
            );
        }
        for bad in [
            "",
            "example.com",
            "ftp://x",
            "FTP://x",
            "HTTP://",
            "https://",
            "https://   ",
            "HTTPS:/x",
            "mailto:a@b.c",
        ] {
            assert!(!valid_field_url(bad), "{bad:?}");
            // An empty entry clears the field instead.
            if !bad.is_empty() {
                assert!(encode_field("url", bad).is_err(), "{bad:?}");
            }
        }
        // The length cap still applies, counted in characters.
        let longest = format!("HTTP://{}", "é".repeat(2_000 - 7));
        assert!(valid_field_url(&longest));
        assert!(!valid_field_url(&format!("{longest}x")));
    }

    #[test]
    fn pr_links_must_be_http_or_https() {
        assert!(valid_pr_url("https://github.com/a/b/pull/1"));
        assert!(valid_pr_url("http://example.com"));
        for bad in [
            "ftp://x.com",
            "github.com/a",
            "https://",
            "https:///x",
            "https://a b",
            "",
        ] {
            assert!(!valid_pr_url(bad), "{bad:?}");
        }
    }
}
