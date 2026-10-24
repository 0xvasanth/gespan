//! The board's small visual vocabulary: label colours, priority bars, due
//! dates and chips, shared by cards and the task drawer.
use crate::theme::*;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone};
use gpui::{AnyElement, Hsla, div, prelude::*, px, rgb};

/// Label colours, in the order new labels take them.
pub(crate) const PALETTE: [u32; 8] = [
    0x8a867e, 0xe5534b, 0xe0823d, 0xe8c547, 0x3fb950, 0x2bb3a8, 0x4da3f5, 0xa371f7,
];

pub(crate) fn label_color_hex(hex: &str) -> u32 {
    u32::from_str_radix(hex.trim_start_matches('#'), 16)
        .ok()
        .filter(|value| *value <= 0xffffff && hex.trim_start_matches('#').len() == 6)
        .unwrap_or(PALETTE[0])
}

pub(crate) fn label_color(hex: &str) -> Hsla {
    rgb(label_color_hex(hex)).into()
}

/// The colour a board's `count`-th label gets.
pub(crate) fn next_label_color(count: usize) -> String {
    format!("{:06x}", PALETTE[count % PALETTE.len()])
}

/// How many of the three bars a priority fills. Urgent draws its own glyph.
pub(crate) fn priority_bars(priority: u8) -> usize {
    priority.min(3) as usize
}

/// A due date as the card shows it, and whether it has passed.
pub(crate) fn due_text(due: NaiveDate, today: NaiveDate) -> (String, bool) {
    let days = (due - today).num_days();
    let text = match days {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        2..=6 => due.format("%a").to_string(),
        _ if due.year() == today.year() => due.format("%b %-d").to_string(),
        _ => due.format("%b %-d, %Y").to_string(),
    };
    (text, days < 0)
}

pub(crate) fn local_day(at: i64) -> Option<NaiveDate> {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.date_naive())
}

/// The start of `day` in local time, as stored in `due_at`: midnight, or
/// the first minute that exists when a daylight-saving change skips it.
/// `None` only for a day the zone skipped entirely.
pub(crate) fn day_start(day: NaiveDate) -> Option<i64> {
    first_instant(day, |time| {
        Local
            .from_local_datetime(&time)
            .earliest()
            .map(|time| time.timestamp_millis())
    })
}

fn first_instant(day: NaiveDate, resolve: impl Fn(NaiveDateTime) -> Option<i64>) -> Option<i64> {
    let midnight = day.and_hms_opt(0, 0, 0)?;
    (0..24 * 60)
        .map(|minute| midnight + chrono::Duration::minutes(minute))
        .find_map(resolve)
}

pub(crate) fn due_label(due_at: i64, now: i64) -> Option<(String, bool)> {
    Some(due_text(local_day(due_at)?, local_day(now)?))
}

/// Three rising bars filled to the priority, or a red "!" box for Urgent.
pub(crate) fn priority_glyph(priority: u8) -> AnyElement {
    if priority >= 4 {
        return div()
            .size(px(12.))
            .rounded(px(3.))
            .bg(diff_foreground(false))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(9.))
            .text_color(white(0.95))
            .child("!")
            .into_any_element();
    }
    let filled = priority_bars(priority);
    div()
        .h(px(12.))
        .flex()
        .items_end()
        .gap(px(1.5))
        .children((0..3).map(|bar| {
            div()
                .w(px(2.5))
                .h(px(4. + 3. * bar as f32))
                .rounded(px(1.))
                .bg(if bar < filled {
                    content(0.75)
                } else {
                    content(0.18)
                })
        }))
        .into_any_element()
}

/// A label chip: a dot and the name on a faint wash of its colour.
pub(crate) fn label_chip(name: &str, color: &str) -> gpui::Div {
    let tint = label_color(color);
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(tint.opacity(0.14))
        .text_size(px(11.))
        .text_color(content(0.85))
        .child(div().size(px(6.)).rounded_full().bg(tint))
        .child(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn due_dates_read_like_a_calendar_near_today() {
        let today = day(2026, 10, 24); // a Saturday
        assert_eq!(due_text(today, today), ("Today".to_string(), false));
        assert_eq!(
            due_text(day(2026, 10, 25), today),
            ("Tomorrow".to_string(), false)
        );
        assert_eq!(
            due_text(day(2026, 10, 23), today),
            ("Yesterday".to_string(), true)
        );
        assert_eq!(
            due_text(day(2026, 10, 28), today),
            ("Wed".to_string(), false)
        );
        assert_eq!(
            due_text(day(2026, 11, 12), today),
            ("Nov 12".to_string(), false)
        );
        assert_eq!(
            due_text(day(2026, 10, 1), today),
            ("Oct 1".to_string(), true)
        );
        assert_eq!(
            due_text(day(2027, 2, 3), today),
            ("Feb 3, 2027".to_string(), false)
        );
    }

    #[test]
    fn a_day_whose_midnight_is_skipped_starts_at_its_first_real_minute() {
        let day = day(2026, 3, 8);
        let at = |hour: u32, minute: u32| {
            day.and_hms_opt(hour, minute, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
        };
        // A zone that springs forward from 00:00 straight to 01:00.
        let skips_midnight = |time: NaiveDateTime| {
            (time.time() >= chrono::NaiveTime::from_hms_opt(1, 0, 0).unwrap())
                .then(|| time.and_utc().timestamp_millis())
        };
        assert_eq!(first_instant(day, skips_midnight), Some(at(1, 0)));
        // An ordinary day starts at midnight.
        assert_eq!(
            first_instant(day, |time| Some(time.and_utc().timestamp_millis())),
            Some(at(0, 0))
        );
        // A day the zone skipped entirely has no start.
        assert_eq!(first_instant(day, |_| None), None);
    }

    #[test]
    fn the_local_start_of_a_day_is_on_that_day() {
        // Includes days that open with a daylight-saving jump at midnight in
        // Havana and Santiago, for runs under TZ=America/Havana and the like.
        for picked in [day(2026, 11, 2), day(2026, 3, 8), day(2026, 9, 6)] {
            assert_eq!(day_start(picked).and_then(local_day), Some(picked));
        }
    }

    #[test]
    fn priority_bars_fill_to_the_level_and_urgent_is_its_own_glyph() {
        assert_eq!(priority_bars(0), 0);
        assert_eq!(priority_bars(1), 1);
        assert_eq!(priority_bars(2), 2);
        assert_eq!(priority_bars(3), 3);
        assert_eq!(priority_bars(4), 3);
    }

    #[test]
    fn label_colors_parse_and_fall_back_to_gray() {
        assert_eq!(label_color_hex("e5534b"), 0xe5534b);
        assert_eq!(label_color_hex("not a colour"), PALETTE[0]);
        assert_eq!(next_label_color(0), "8a867e");
        assert_eq!(next_label_color(9), "e5534b");
    }
}
