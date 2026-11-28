//! What the "Two zones" left nav lists inside its "This project" card,
//! decided apart from drawing (design: glass canvas, Source of truth ›
//! SoTSidebar).
//!
//! Under New thread and the Pilots come two lists. **Now** holds the live
//! sessions, the ones waiting on you first, then the running ones, each
//! with its age. **Sessions** is one flat list of everything else, newest
//! first, each row with its branch as a small label: no per-branch folders.
//!
//! Every session shows exactly once: in Now, in one of the user's own
//! folders (when Settings groups by folders), or in Sessions.
use std::collections::{HashMap, HashSet};

/// Now's rows before "N more".
pub(super) const NOW_PREVIEW: usize = 5;
/// Sessions' rows before "N more".
pub(super) const SESSIONS_PREVIEW: usize = 3;
/// An open user folder's rows before "N more".
pub(super) const FOLDER_PREVIEW: usize = 2;
/// The keys `expanded` holds once a list's "N more" is clicked.
pub(super) const NOW_KEY: &str = "now";
pub(super) const SESSIONS_KEY: &str = "sessions";
/// A branch label's longest length, in characters, before it truncates in
/// the middle.
pub(super) const BRANCH_LABEL_MAX: usize = 16;

/// Work that wants attention. Ordered so the stronger state sorts first:
/// waiting on you before running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Live {
    NeedsYou,
    Running,
}

/// What a user folder header shows before its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum FolderKind {
    /// A folder that holds automation runs: a bolt.
    Automation,
    /// One of the user's own folders.
    Folder,
    Reminders,
    Pinned,
}

/// One of the user's folders, sessions in any order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FolderInput {
    pub key: String,
    pub label: String,
    pub kind: FolderKind,
    pub sessions: Vec<i64>,
    /// Whether the user left it open.
    pub open: bool,
}

/// A user folder as the rail draws it. Its live sessions are in Now.
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
    /// Live sessions: needs you first, then running, newest first in each.
    pub now: Vec<(i64, Live)>,
    /// Live sessions left for Now's "N more".
    pub now_more: usize,
    /// Whether every one of those is running, none waiting on you.
    pub now_more_running: bool,
    pub folders: Vec<Folder>,
    /// The flat Sessions list, newest first.
    pub sessions: Vec<i64>,
    /// Sessions left for Sessions' "N more".
    pub more: usize,
    /// Sessions listed anywhere in the rail.
    pub total: usize,
}

/// Every input the planner reads about a session.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Facts<'a> {
    /// Only sessions that need you or run.
    pub live: Option<&'a HashMap<i64, Live>>,
    /// When each session last changed; missing reads as never.
    pub updated: Option<&'a HashMap<i64, u64>>,
}

impl Facts<'_> {
    fn live(&self, id: i64) -> Option<Live> {
        self.live.and_then(|live| live.get(&id).copied())
    }
    fn updated(&self, id: i64) -> u64 {
        self.updated
            .and_then(|at| at.get(&id).copied())
            .unwrap_or_default()
    }
    /// Newest first; the id breaks ties.
    fn newest_first(&self, ids: &mut [i64]) {
        ids.sort_by_key(|id| std::cmp::Reverse((self.updated(*id), *id)));
    }
}

/// The first `preview` of `ids` and how many are left, or all of them.
fn capped(mut ids: Vec<i64>, preview: usize, all: bool) -> (Vec<i64>, usize) {
    let keep = if all {
        ids.len()
    } else {
        preview.min(ids.len())
    };
    let more = ids.len() - keep;
    ids.truncate(keep);
    (ids, more)
}

/// Lays out the project card's lists. `loose` holds every session in no
/// user folder; `expanded` the keys whose "N more" was clicked. A session
/// in two inputs is listed under the first.
pub(super) fn plan(
    folders: Vec<FolderInput>,
    loose: Vec<i64>,
    facts: Facts,
    expanded: &HashSet<String>,
) -> Plan {
    let mut seen = HashSet::new();
    let mut plan = Plan::default();
    let mut live = Vec::new();
    for folder in folders {
        let mut sessions: Vec<i64> = folder
            .sessions
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect();
        if sessions.is_empty() {
            continue;
        }
        plan.total += sessions.len();
        let count = sessions.len();
        let strongest = sessions.iter().filter_map(|id| facts.live(*id)).min();
        live.extend(
            sessions
                .iter()
                .copied()
                .filter(|id| facts.live(*id).is_some()),
        );
        sessions.retain(|id| facts.live(*id).is_none());
        facts.newest_first(&mut sessions);
        let (shown, more) = if folder.open {
            capped(sessions, FOLDER_PREVIEW, expanded.contains(&folder.key))
        } else {
            (Vec::new(), 0)
        };
        plan.folders.push(Folder {
            key: folder.key,
            label: folder.label,
            kind: folder.kind,
            count,
            live: strongest,
            open: folder.open,
            shown,
            more,
        });
    }
    let mut idle: Vec<i64> = loose.into_iter().filter(|id| seen.insert(*id)).collect();
    plan.total += idle.len();
    live.extend(idle.iter().copied().filter(|id| facts.live(*id).is_some()));
    idle.retain(|id| facts.live(*id).is_none());
    facts.newest_first(&mut idle);
    (plan.sessions, plan.more) = capped(idle, SESSIONS_PREVIEW, expanded.contains(SESSIONS_KEY));
    facts.newest_first(&mut live);
    live.sort_by_key(|id| facts.live(*id));
    let (now, now_more) = capped(live.clone(), NOW_PREVIEW, expanded.contains(NOW_KEY));
    plan.now_more_running = live
        .iter()
        .skip(now.len())
        .all(|id| facts.live(*id) == Some(Live::Running));
    plan.now = now
        .into_iter()
        .filter_map(|id| Some((id, facts.live(id)?)))
        .collect();
    plan.now_more = now_more;
    plan
}

/// A branch as a row's trailing label: without the `claude/` an agent's
/// worktree branch starts with or the 6-hex suffix its worktree name ends
/// with, then cut in the middle to [`BRANCH_LABEL_MAX`] characters, so both
/// the start and the end that tell branches apart stay readable.
pub(super) fn branch_label(branch: &str) -> String {
    let name = branch.strip_prefix("claude/").unwrap_or(branch);
    let name = match name.rsplit_once('-') {
        Some((stem, tail))
            if !stem.is_empty()
                && tail.len() == 6
                && tail.bytes().all(|b| b.is_ascii_hexdigit())
                && tail.bytes().any(|b| b.is_ascii_digit()) =>
        {
            stem
        }
        _ => name,
    };
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= BRANCH_LABEL_MAX {
        return name.to_owned();
    }
    // One character goes to the ellipsis; the start keeps the larger half.
    let room = BRANCH_LABEL_MAX - 1;
    let tail = room / 2;
    let head = room - tail;
    let mut out: String = chars.iter().take(head).collect();
    out.push('…');
    out.extend(chars.iter().skip(chars.len() - tail));
    out
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

/// The switcher's second line: the project's folder, then its sessions.
/// The machine is the chip beside the name.
/// The worktree or folder count moves to the tooltip to make room.
pub(super) fn switcher_line(place: &str, sessions: usize) -> String {
    let noun = if sessions == 1 { "session" } else { "sessions" };
    format!("{place} · {sessions} {noun}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(key: &str, sessions: &[i64], open: bool) -> FolderInput {
        FolderInput {
            key: key.into(),
            label: key.into(),
            kind: FolderKind::Folder,
            sessions: sessions.to_vec(),
            open,
        }
    }

    fn live(pairs: &[(i64, Live)]) -> HashMap<i64, Live> {
        pairs.iter().copied().collect()
    }

    /// Session `id` last changed at `id` minutes, so higher ids are newer.
    fn by_id(ids: impl IntoIterator<Item = i64>) -> HashMap<i64, u64> {
        ids.into_iter().map(|id| (id, id as u64 * 60_000)).collect()
    }

    fn run(folders: Vec<FolderInput>, loose: &[i64], live: &HashMap<i64, Live>) -> Plan {
        expand(folders, loose, live, &[])
    }

    fn expand(
        folders: Vec<FolderInput>,
        loose: &[i64],
        live: &HashMap<i64, Live>,
        expanded: &[&str],
    ) -> Plan {
        let updated = by_id(0..100);
        plan(
            folders,
            loose.to_vec(),
            Facts {
                live: Some(live),
                updated: Some(&updated),
            },
            &expanded.iter().map(|k| (*k).to_owned()).collect(),
        )
    }

    fn now_ids(plan: &Plan) -> Vec<i64> {
        plan.now.iter().map(|(id, _)| *id).collect()
    }

    /// Every session the plan lists, wherever it sits.
    fn listed(plan: &Plan) -> Vec<i64> {
        let mut ids: Vec<i64> = now_ids(plan)
            .into_iter()
            .chain(plan.folders.iter().flat_map(|f| f.shown.clone()))
            .chain(plan.sessions.clone())
            .collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn each_session_is_listed_exactly_once_whichever_inputs_repeat_it() {
        // 5 sits in a folder and is also loose; 3 is live.
        let plan = expand(
            vec![folder("folder:docs", &[5], true)],
            &[1, 2, 3, 4, 5, 2],
            &live(&[(3, Live::Running)]),
            &["sessions"],
        );
        assert_eq!(listed(&plan), vec![1, 2, 3, 4, 5]);
        assert_eq!(now_ids(&plan), vec![3]);
        assert_eq!(plan.folders[0].shown, vec![5]);
        assert_eq!(plan.sessions, vec![4, 2, 1]);
        assert_eq!(plan.total, 5);
    }

    #[test]
    fn now_lists_needs_you_first_then_running_each_newest_first() {
        let plan = run(
            vec![],
            &[1, 2, 3, 4, 5],
            &live(&[
                (1, Live::Running),
                (2, Live::NeedsYou),
                (4, Live::Running),
                (3, Live::NeedsYou),
            ]),
        );
        assert_eq!(
            plan.now,
            vec![
                (3, Live::NeedsYou),
                (2, Live::NeedsYou),
                (4, Live::Running),
                (1, Live::Running)
            ]
        );
        assert_eq!(plan.sessions, vec![5]);
    }

    #[test]
    fn a_session_moves_to_now_when_it_goes_live_and_back_when_idle() {
        let ids = [1, 2, 3, 4];
        let idle = expand(vec![], &ids, &HashMap::new(), &["sessions"]);
        assert!(idle.now.is_empty());
        assert_eq!(idle.sessions, vec![4, 3, 2, 1]);
        let busy = expand(vec![], &ids, &live(&[(2, Live::Running)]), &["sessions"]);
        assert_eq!(busy.now, vec![(2, Live::Running)]);
        assert_eq!(busy.sessions, vec![4, 3, 1], "out of Sessions while live");
        let back = expand(vec![], &ids, &HashMap::new(), &["sessions"]);
        assert_eq!(back, idle);
    }

    #[test]
    fn now_shows_five_then_n_more_until_expanded() {
        let ids: Vec<i64> = (1..=11).collect();
        let states: HashMap<i64, Live> = ids.iter().map(|id| (*id, Live::Running)).collect();
        let capped = run(vec![], &ids, &states);
        assert_eq!(now_ids(&capped), vec![11, 10, 9, 8, 7]);
        assert_eq!(capped.now_more, 6);
        assert!(capped.now_more_running);
        // Seven waiting: two of them are past the five, so not "running".
        let waiting: HashMap<i64, Live> = (1..=7).map(|id| (id, Live::NeedsYou)).collect();
        let busy = run(vec![], &ids, &waiting);
        assert_eq!(busy.now_more, 2);
        assert!(!busy.now_more_running);
        let all = expand(vec![], &ids, &states, &["now"]);
        assert_eq!((all.now.len(), all.now_more), (11, 0));
    }

    #[test]
    fn sessions_show_three_then_n_more_until_expanded() {
        let ids: Vec<i64> = (1..=15).collect();
        let capped = run(vec![], &ids, &HashMap::new());
        assert_eq!(capped.sessions, vec![15, 14, 13]);
        assert_eq!(capped.more, 12);
        let all = expand(vec![], &ids, &HashMap::new(), &["sessions"]);
        assert_eq!((all.sessions.len(), all.more), (15, 0));
    }

    #[test]
    fn exactly_the_preview_needs_no_more_row() {
        let plan = run(vec![], &[1, 2, 3], &HashMap::new());
        assert_eq!((plan.sessions.len(), plan.more), (3, 0));
        let states: HashMap<i64, Live> = (1..=5).map(|id| (id, Live::NeedsYou)).collect();
        let now = run(vec![], &[1, 2, 3, 4, 5], &states);
        assert_eq!((now.now.len(), now.now_more), (5, 0));
    }

    #[test]
    fn a_folders_live_sessions_go_to_now_but_still_count_and_mark_it() {
        let plan = run(
            vec![
                folder("folder:open", &[1, 2, 3, 4], true),
                folder("folder:closed", &[5, 6], false),
            ],
            &[],
            &live(&[(1, Live::Running), (6, Live::NeedsYou)]),
        );
        assert_eq!(plan.now, vec![(6, Live::NeedsYou), (1, Live::Running)]);
        let [open, closed] = &plan.folders[..] else {
            panic!("two folders: {:?}", plan.folders)
        };
        assert_eq!(
            (open.count, open.live, open.shown.clone(), open.more),
            (4, Some(Live::Running), vec![4, 3], 1)
        );
        assert_eq!(
            (closed.count, closed.live, closed.shown.len()),
            (2, Some(Live::NeedsYou), 0)
        );
    }

    #[test]
    fn empty_folders_drop_out() {
        let plan = run(
            vec![folder("folder:none", &[], true)],
            &[1],
            &HashMap::new(),
        );
        assert!(plan.folders.is_empty());
        assert_eq!(plan.sessions, vec![1]);
    }

    #[test]
    fn branch_labels_drop_agent_noise_and_truncate_in_the_middle() {
        assert_eq!(branch_label("main"), "main");
        assert_eq!(branch_label("auto/langfuse"), "auto/langfuse");
        assert_eq!(
            branch_label("claude/pilot-live-turn-e83707"),
            "pilot-live-turn"
        );
        // A word that happens to be six hex letters is not a suffix.
        assert_eq!(branch_label("fix-facade"), "fix-facade");
        assert_eq!(branch_label("claude/-abc123"), "-abc123");
        // Exactly the limit stays whole; one more cuts in the middle and
        // keeps both ends.
        assert_eq!(branch_label("glassy-ui-core-p1"), "glassy-u…core-p1");
        assert_eq!(branch_label("glassy-ui-core-p").chars().count(), 16);
        assert_eq!(branch_label("glassy-ui-core-p"), "glassy-ui-core-p");
        let long = branch_label("feature/very-long-branch-name-for-the-rail");
        assert_eq!(long.chars().count(), BRANCH_LABEL_MAX);
        assert_eq!(long, "feature/…he-rail");
        // Characters, not bytes: no panic on a multi-byte name.
        let wide = branch_label("ブランチ名がとても長いブランチの名前です");
        assert_eq!(wide.chars().count(), BRANCH_LABEL_MAX);
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
    fn the_switcher_line_puts_the_folder_before_the_sessions() {
        assert_eq!(
            switcher_line("~/repo/ae", 1),
            "~/repo/ae · 1 session"
        );
        assert_eq!(switcher_line("~/farm/ae", 0), "~/farm/ae · 0 sessions");
        assert_eq!(switcher_line("~", 12), "~ · 12 sessions");
    }

    #[test]
    fn the_switcher_summary_names_the_grouping() {
        assert_eq!(switcher_summary(5, 17, true), "5 worktrees · 17 sessions");
        assert_eq!(switcher_summary(1, 1, true), "1 worktree · 1 session");
        assert_eq!(switcher_summary(3, 0, false), "3 folders · 0 sessions");
    }
}
