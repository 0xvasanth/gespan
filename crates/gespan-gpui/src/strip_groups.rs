//! Groups the title strip's tabs by project: a group sits where its first tab is, and
//! tabs with no project stay loose in place.
use std::collections::HashMap;
use std::hash::Hash;

/// A tab with no project, or one project's tabs drawn together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StripEntry<T> {
    Loose(T),
    Group { project: String, tabs: Vec<T> },
}

/// `project_of` returns None for tabs drawn on their own (App tabs).
/// A group sits where its first tab is; its tabs keep their relative order.
pub(crate) fn entries<T: Clone>(
    order: &[T],
    project_of: impl Fn(&T) -> Option<String>,
) -> Vec<StripEntry<T>> {
    let mut out: Vec<StripEntry<T>> = Vec::new();
    let mut group_at: HashMap<String, usize> = HashMap::new();
    for tab in order {
        let Some(project) = project_of(tab) else {
            out.push(StripEntry::Loose(tab.clone()));
            continue;
        };
        match group_at.get(&project) {
            Some(&i) => {
                if let StripEntry::Group { tabs, .. } = &mut out[i] {
                    tabs.push(tab.clone());
                }
            }
            None => {
                group_at.insert(project.clone(), out.len());
                out.push(StripEntry::Group {
                    project,
                    tabs: vec![tab.clone()],
                });
            }
        }
    }
    out
}

/// Reorders `order` in place so each project's tabs are contiguous, starting at its first tab's position.
pub(crate) fn make_contiguous<T: Clone + PartialEq>(
    order: &mut Vec<T>,
    project_of: impl Fn(&T) -> Option<String>,
) {
    let grouped = entries(order, project_of);
    *order = grouped
        .into_iter()
        .flat_map(|entry| match entry {
            StripEntry::Loose(tab) => vec![tab],
            StripEntry::Group { tabs, .. } => tabs,
        })
        .collect();
}

/// A tab untouched this long is idle.
pub(crate) const IDLE_AFTER_SECS: i64 = 12 * 60 * 60;

/// Tabs focused more than IDLE_AFTER_SECS before `now`. A tab missing from `focused_at` is not idle.
pub(crate) fn idle<T: Clone + Eq + Hash>(
    order: &[T],
    focused_at: &HashMap<T, i64>,
    now: i64,
    keep: impl Fn(&T) -> bool,
) -> Vec<T> {
    order
        .iter()
        .filter(|tab| {
            focused_at
                .get(*tab)
                .is_some_and(|at| now - at > IDLE_AFTER_SECS)
                && !keep(tab)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tab: &&str) -> Option<String> {
        tab.split_once(':').map(|(p, _)| p.to_string())
    }
    fn group(project: &str, tabs: &[&'static str]) -> StripEntry<&'static str> {
        StripEntry::Group {
            project: project.into(),
            tabs: tabs.to_vec(),
        }
    }

    #[test]
    fn groups_sit_at_their_first_tab() {
        let got = entries(&["a:1", "board", "b:1", "a:2"], project);
        assert_eq!(
            got,
            vec![
                group("a", &["a:1", "a:2"]),
                StripEntry::Loose("board"),
                group("b", &["b:1"]),
            ]
        );
    }

    #[test]
    fn loose_tabs_keep_their_place_between_groups() {
        let got = entries(&["board", "a:1", "notes", "b:1"], project);
        assert_eq!(
            got,
            vec![
                StripEntry::Loose("board"),
                group("a", &["a:1"]),
                StripEntry::Loose("notes"),
                group("b", &["b:1"]),
            ]
        );
    }

    #[test]
    fn empty_order_has_no_entries() {
        assert!(entries::<&str>(&[], project).is_empty());
    }

    #[test]
    fn make_contiguous_pulls_later_tabs_to_their_group() {
        let mut order = vec!["a:1", "board", "b:1", "a:2"];
        make_contiguous(&mut order, project);
        assert_eq!(order, ["a:1", "a:2", "board", "b:1"]);

        let mut settled = vec!["a:1", "a:2", "board", "b:1"];
        make_contiguous(&mut settled, project);
        assert_eq!(settled, ["a:1", "a:2", "board", "b:1"]);
    }

    #[test]
    fn idle_picks_only_tabs_past_twelve_hours() {
        let now = 100_000;
        let focused = HashMap::from([("x", now - 43_201), ("y", now - 43_199)]);
        assert_eq!(idle(&["x", "y"], &focused, now, |_| false), ["x"]);
    }

    #[test]
    fn idle_skips_unrecorded_and_kept_tabs() {
        let now = 100_000;
        let focused = HashMap::from([("x", now - 50_000), ("kept", now - 50_000)]);
        let got = idle(&["x", "unrecorded", "kept"], &focused, now, |t| {
            *t == "kept"
        });
        assert_eq!(got, ["x"]);
    }
}
