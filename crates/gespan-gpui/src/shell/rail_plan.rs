//! What the Rail F1 sidebar lists for one project, decided apart from
//! drawing: the sessions that need you, the ones running, and every other
//! session once, under a collapsible folder.
//!
//! A live session is listed in its section only, never again in its folder,
//! but its folder still counts it and shows its dot while closed. An open
//! folder shows its [`FOLDER_PREVIEW`] most recent sessions, then "N more".
use std::collections::{HashMap, HashSet};

/// An open folder's rows before "N more".
pub(super) const FOLDER_PREVIEW: usize = 2;

/// Work that wants attention. Ordered so the stronger state wins a folder's
/// dot: waiting on you before running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Live {
    NeedsYou,
    Running,
}

/// What a folder header shows before its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum FolderKind {
    /// A worktree, or the project folder: a branch.
    Branch,
    /// A worktree or folder that holds automation runs: a bolt.
    Automation,
    /// A Pilot's folder, by Pilot id: its mascot.
    Pilot(String),
    /// One of the user's own folders.
    Folder,
    Reminders,
    Pinned,
    /// Sessions in no folder of their own.
    Loose,
}

/// A folder as the grouping hands it over, sessions most recent first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FolderInput {
    pub key: String,
    pub label: String,
    pub kind: FolderKind,
    pub sessions: Vec<i64>,
    /// Whether the user opened (`Some(true)`) or closed it; `None` follows
    /// the default.
    pub stored_open: Option<bool>,
    /// Open unless the user closed it: the folder the project is working in.
    pub open_by_default: bool,
}

/// A folder as the rail draws it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Folder {
    pub key: String,
    pub label: String,
    pub kind: FolderKind,
    /// Every session in it, live ones included.
    pub count: usize,
    /// The strongest live state inside, shown as a dot beside the count.
    pub live: Option<Live>,
    pub open: bool,
    /// The rows under an open folder: idle sessions only.
    pub shown: Vec<i64>,
    /// Idle sessions an open folder leaves for "N more".
    pub more: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    pub needs_you: Vec<i64>,
    pub running: Vec<i64>,
    pub folders: Vec<Folder>,
    /// Sessions listed anywhere in the rail.
    pub sessions: usize,
}

/// Lays out the rail. `live` holds only sessions that need you or run;
/// `expanded` the folders whose "N more" was clicked. A session in two
/// inputs is listed under the first.
pub(super) fn plan(
    inputs: Vec<FolderInput>,
    live: &HashMap<i64, Live>,
    expanded: &HashSet<String>,
) -> Plan {
    let mut seen = HashSet::new();
    let mut plan = Plan::default();
    for input in inputs {
        let sessions: Vec<i64> = input
            .sessions
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect();
        if sessions.is_empty() {
            continue;
        }
        let mut idle = Vec::new();
        let mut strongest: Option<Live> = None;
        for id in &sessions {
            match live.get(id) {
                Some(state) => {
                    strongest = Some(strongest.map_or(*state, |s| s.min(*state)));
                    match state {
                        Live::NeedsYou => plan.needs_you.push(*id),
                        Live::Running => plan.running.push(*id),
                    }
                }
                None => idle.push(*id),
            }
        }
        let open = input.stored_open.unwrap_or(input.open_by_default);
        let (shown, more) = if !open {
            (Vec::new(), 0)
        } else if expanded.contains(&input.key) || idle.len() <= FOLDER_PREVIEW {
            (idle, 0)
        } else {
            let more = idle.len() - FOLDER_PREVIEW;
            idle.truncate(FOLDER_PREVIEW);
            (idle, more)
        };
        plan.sessions += sessions.len();
        plan.folders.push(Folder {
            key: input.key,
            label: input.label,
            kind: input.kind,
            count: sessions.len(),
            live: strongest,
            open,
            shown,
            more,
        });
    }
    plan
}

/// A session's age in the rail's narrow time column: "now", "9m", "2h",
/// "1d", "3w". Zero (unknown) is blank.
pub(super) fn age(at: u64, now: u64) -> String {
    if at == 0 {
        return String::new();
    }
    let minutes = now.saturating_sub(at) / 60_000;
    match minutes {
        0 => "now".into(),
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m if m < 60 * 24 * 7 => format!("{}d", m / (60 * 24)),
        m => format!("{}w", m / (60 * 24 * 7)),
    }
}

/// The switcher's second line: "5 worktrees · 17 sessions".
pub(super) fn switcher_summary(folders: usize, sessions: usize, by_worktree: bool) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let (one, many) = if by_worktree {
        ("worktree", "worktrees")
    } else {
        ("folder", "folders")
    };
    format!(
        "{} · {}",
        plural(folders, one, many),
        plural(sessions, "session", "sessions")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(key: &str, sessions: &[i64]) -> FolderInput {
        FolderInput {
            key: key.into(),
            label: key.into(),
            kind: FolderKind::Branch,
            sessions: sessions.to_vec(),
            stored_open: None,
            open_by_default: false,
        }
    }

    fn live(pairs: &[(i64, Live)]) -> HashMap<i64, Live> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn live_sessions_leave_their_folder_but_still_count_and_mark_it() {
        let mut main = folder("main", &[1, 2, 3]);
        main.open_by_default = true;
        let plan = plan(
            vec![main, folder("glass-home", &[4, 5, 6])],
            &live(&[(2, Live::NeedsYou), (4, Live::Running)]),
            &HashSet::new(),
        );
        assert_eq!(plan.needs_you, vec![2]);
        assert_eq!(plan.running, vec![4]);
        let [main, glass] = &plan.folders[..] else {
            panic!("two folders: {:?}", plan.folders)
        };
        // The waiting session is only under Needs you, yet main counts it
        // and wears its yellow dot.
        assert_eq!(
            (main.count, main.live, &main.shown[..]),
            (3, Some(Live::NeedsYou), &[1, 3][..])
        );
        // Closed, with a running session inside: the dot shows anyway.
        assert!(!glass.open);
        assert_eq!(
            (glass.count, glass.live, glass.shown.len()),
            (3, Some(Live::Running), 0)
        );
        assert_eq!(plan.sessions, 6);
    }

    #[test]
    fn needs_you_outranks_running_for_a_folders_dot() {
        let plan = plan(
            vec![folder("a", &[1, 2])],
            &live(&[(1, Live::Running), (2, Live::NeedsYou)]),
            &HashSet::new(),
        );
        assert_eq!(plan.folders[0].live, Some(Live::NeedsYou));
    }

    #[test]
    fn a_session_going_idle_returns_to_its_folder() {
        let mut a = folder("a", &[1, 2]);
        a.open_by_default = true;
        let waiting = plan(
            vec![a.clone()],
            &live(&[(1, Live::NeedsYou)]),
            &HashSet::new(),
        );
        assert_eq!(
            (waiting.needs_you, waiting.folders[0].shown.clone()),
            (vec![1], vec![2])
        );
        let idle = plan(vec![a], &HashMap::new(), &HashSet::new());
        assert!(idle.needs_you.is_empty());
        assert_eq!(idle.folders[0].shown, vec![1, 2]);
        assert_eq!(idle.folders[0].live, None);
    }

    #[test]
    fn an_open_folder_shows_two_then_n_more_until_expanded() {
        let mut main = folder("main", &[10, 11, 12, 13, 14, 15]);
        main.open_by_default = true;
        let capped = plan(vec![main.clone()], &HashMap::new(), &HashSet::new());
        assert_eq!(capped.folders[0].shown, vec![10, 11]);
        assert_eq!(capped.folders[0].more, 4);
        let all = plan(
            vec![main],
            &HashMap::new(),
            &HashSet::from(["main".to_owned()]),
        );
        assert_eq!(all.folders[0].shown.len(), 6);
        assert_eq!(all.folders[0].more, 0);
    }

    #[test]
    fn exactly_the_preview_needs_no_more_row() {
        let mut a = folder("a", &[1, 2]);
        a.open_by_default = true;
        let plan = plan(vec![a], &HashMap::new(), &HashSet::new());
        assert_eq!((plan.folders[0].shown.len(), plan.folders[0].more), (2, 0));
    }

    #[test]
    fn idle_folders_start_closed_and_the_users_choice_wins_either_way() {
        let mut closed_by_user = folder("main", &[1]);
        closed_by_user.open_by_default = true;
        closed_by_user.stored_open = Some(false);
        let mut opened_by_user = folder("old", &[2]);
        opened_by_user.stored_open = Some(true);
        let plan = plan(
            vec![closed_by_user, opened_by_user, folder("idle", &[3])],
            &HashMap::new(),
            &HashSet::new(),
        );
        let open: Vec<_> = plan
            .folders
            .iter()
            .map(|f| (f.key.as_str(), f.open))
            .collect();
        assert_eq!(open, [("main", false), ("old", true), ("idle", false)]);
        assert!(plan.folders[0].shown.is_empty());
        assert_eq!(plan.folders[1].shown, vec![2]);
    }

    #[test]
    fn every_session_is_listed_once_and_empty_folders_drop_out() {
        let plan = plan(
            vec![
                folder("pinned", &[1, 2]),
                folder("loose", &[2, 3]),
                folder("none", &[]),
            ],
            &HashMap::new(),
            &HashSet::new(),
        );
        let keys: Vec<_> = plan
            .folders
            .iter()
            .map(|f| (f.key.as_str(), f.count))
            .collect();
        assert_eq!(keys, [("pinned", 2), ("loose", 1)]);
        assert_eq!(plan.sessions, 3);
    }

    #[test]
    fn the_rail_stays_bounded_at_scale() {
        // 20 sessions over 6 folders, every folder open: no folder lists
        // more than its preview, so the rail stays short.
        let inputs: Vec<_> = (0..6)
            .map(|f| {
                let mut input = folder(
                    &format!("wt{f}"),
                    &(0..20)
                        .filter(|s| s % 6 == f)
                        .map(i64::from)
                        .collect::<Vec<_>>(),
                );
                input.stored_open = Some(true);
                input
            })
            .collect();
        let plan = plan(inputs, &live(&[(0, Live::Running)]), &HashSet::new());
        assert_eq!(plan.sessions, 20);
        let rows: usize = plan
            .folders
            .iter()
            .map(|f| 1 + f.shown.len() + usize::from(f.more > 0))
            .sum();
        assert!(plan.folders.iter().all(|f| f.shown.len() <= FOLDER_PREVIEW));
        assert!(rows <= 6 * (2 + FOLDER_PREVIEW), "{rows} rows");
    }

    #[test]
    fn ages_fit_the_narrow_column() {
        let now = 10 * 7 * 24 * 3_600_000;
        let ago = |ms: u64| age(now - ms, now);
        assert_eq!(ago(20_000), "now");
        assert_eq!(ago(2 * 60_000), "2m");
        assert_eq!(ago(14 * 60_000 + 59_000), "14m");
        assert_eq!(ago(2 * 3_600_000 + 5 * 60_000), "2h");
        assert_eq!(ago(30 * 3_600_000), "1d");
        assert_eq!(ago(15 * 24 * 3_600_000), "2w");
        assert_eq!(age(0, now), "");
        // A clock that ran backwards still reads as now.
        assert_eq!(age(now + 5_000, now), "now");
    }

    #[test]
    fn the_switcher_summary_names_the_grouping() {
        assert_eq!(switcher_summary(5, 17, true), "5 worktrees · 17 sessions");
        assert_eq!(switcher_summary(1, 1, true), "1 worktree · 1 session");
        assert_eq!(switcher_summary(3, 0, false), "3 folders · 0 sessions");
    }
}
