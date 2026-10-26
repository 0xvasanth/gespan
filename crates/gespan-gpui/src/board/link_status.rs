//! One chip per card link: what the linked session, Pilot, PR or worktree is
//! doing. Sessions and Pilots read the app's live turns; PRs and worktrees ask
//! `gh` and Git, which is slow, so that lookup runs on the background executor
//! when the board opens and when a turn of a session linked to one of its
//! cards settles. Never on a timer.
use gespan_core::board::{auto_move::LinkedWorkEvent, extras::CardLink};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

/// What one link is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkState {
    Running,
    Waiting,
    Failed,
    Completed,
    Idle,
    Open,
    Merged,
    Closed,
    /// Nothing could be found out (a remote workspace's PR, or `gh` had no
    /// answer).
    Unknown,
    /// The session or Pilot no longer exists.
    Missing,
}

impl LinkState {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Idle => "idle",
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
            Self::Missing => "missing",
        }
    }
}

/// A PR's checks, when known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChecksState {
    Passing,
    Failing,
    Pending,
}

impl ChecksState {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Passing => "checks passing",
            Self::Failing => "checks failing",
            Self::Pending => "checks pending",
        }
    }
}

/// One chip on a card face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkChip {
    pub kind: String,
    pub reference: String,
    pub label: String,
    pub state: LinkState,
    pub checks: Option<ChecksState>,
    /// A worktree's uncommitted lines as (added, deleted).
    pub diff: Option<(i64, i64)>,
}

impl LinkChip {
    /// The chip's status, for its accessible description.
    pub(crate) fn status_text(&self) -> String {
        let mut parts = vec![self.state.text().to_string()];
        if let Some(checks) = self.checks {
            parts.push(checks.text().to_string());
        }
        if let Some((added, deleted)) = self.diff {
            parts.push(format!("+{added} −{deleted}"));
        }
        parts.join(", ")
    }
}

/// What the background lookup found about a PR or worktree link.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkExtra {
    pub state: Option<LinkState>,
    pub checks: Option<ChecksState>,
    pub branch: Option<String>,
    pub diff: Option<(i64, i64)>,
}

/// A session as its chip and agent fields need it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionInfo {
    pub title: String,
    /// The folder the session works in (its worktree when it has one).
    pub cwd: String,
}

/// A session's chip: running beats waiting beats failed beats completed.
pub(crate) fn session_state(running: bool, waiting: bool, failed: bool) -> LinkState {
    if waiting {
        LinkState::Waiting
    } else if running {
        LinkState::Running
    } else if failed {
        LinkState::Failed
    } else {
        LinkState::Completed
    }
}

/// `gh`'s PR state (OPEN, MERGED, CLOSED, any case); anything else is unknown.
pub(crate) fn pr_state(state: &str) -> LinkState {
    match state.trim().to_ascii_lowercase().as_str() {
        "open" => LinkState::Open,
        "merged" => LinkState::Merged,
        "closed" => LinkState::Closed,
        _ => LinkState::Unknown,
    }
}

/// Failing beats pending beats passing; skipped and cancelled checks don't
/// count, and no checks at all is unknown.
pub(crate) fn checks_state<'a>(states: impl IntoIterator<Item = &'a str>) -> Option<ChecksState> {
    let real: Vec<&str> = states
        .into_iter()
        .filter(|state| !matches!(*state, "skipping" | "cancel"))
        .collect();
    if real.contains(&"fail") {
        Some(ChecksState::Failing)
    } else if real
        .iter()
        .any(|state| matches!(*state, "pending" | "unknown"))
    {
        Some(ChecksState::Pending)
    } else if real.is_empty() {
        None
    } else {
        Some(ChecksState::Passing)
    }
}

/// `https://github.com/{owner}/{repo}/pull/{n}` as (`owner/repo`, n).
pub(crate) fn parse_pr_url(url: &str) -> Option<(String, i64)> {
    let rest = url.trim().strip_prefix("https://github.com/")?;
    let rest = rest.split(['?', '#']).next()?;
    let mut parts = rest.trim_end_matches('/').split('/');
    let (owner, repo, pull, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let name_ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    if parts.next().is_some() || pull != "pull" || !name_ok(owner) || !name_ok(repo) {
        return None;
    }
    let number = number.parse::<i64>().ok().filter(|n| *n > 0)?;
    Some((format!("{owner}/{repo}"), number))
}

/// The newest started/needs_input/completed/failed activity kind for each
/// card on the board (from any board, as a moved card keeps its history),
/// read directly so any number of newer comments or moves can't hide it.
/// The same rule the store's `linked: failed` filter uses.
pub(crate) fn latest_run_events(
    conn: &rusqlite::Connection,
    board_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT card_id, kind FROM (
               SELECT card_id, kind, ROW_NUMBER() OVER (
                 PARTITION BY card_id ORDER BY created_at DESC, rowid DESC) AS newest
               FROM board_activity
               WHERE card_id IN (SELECT id FROM board_cards WHERE board_id = ?1)
                 AND kind IN ('started', 'needs_input', 'completed', 'failed'))
             WHERE newest = 1",
        )
        .map_err(|e| e.to_string())?;
    statement
        .query_map([board_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(|e| e.to_string())
}

/// What the chips are built from, all read before building.
pub(crate) struct ChipInputs<'a> {
    pub links: &'a [CardLink],
    /// `("session", key)` and `("pilot", id)` with work running now.
    pub running: &'a BTreeSet<(String, String)>,
    pub waiting: &'a BTreeSet<String>,
    /// Session key → the session, for sessions that exist.
    pub sessions: &'a HashMap<String, SessionInfo>,
    /// Every Pilot as (id, name); `None` when the roster can't be read.
    pub pilots: Option<&'a [(String, String)]>,
    pub last_events: &'a HashMap<String, String>,
    pub extras: &'a HashMap<(String, String), LinkExtra>,
}

/// One chip per link, by card id, in the links' order.
pub(crate) fn build_chips(input: &ChipInputs<'_>) -> HashMap<String, Vec<LinkChip>> {
    let mut chips: HashMap<String, Vec<LinkChip>> = HashMap::new();
    for link in input.links {
        if let Some(chip) = chip(link, input) {
            chips.entry(link.card_id.clone()).or_default().push(chip);
        }
    }
    chips
}

fn chip(link: &CardLink, input: &ChipInputs<'_>) -> Option<LinkChip> {
    let running = input
        .running
        .contains(&(link.kind.clone(), link.reference.clone()));
    let extra = input
        .extras
        .get(&(link.kind.clone(), link.reference.clone()));
    let (label, state, checks, diff) = match link.kind.as_str() {
        "session" => match input.sessions.get(&link.reference) {
            Some(session) => {
                let failed =
                    input.last_events.get(&link.card_id).map(String::as_str) == Some("failed");
                let waiting = input.waiting.contains(&link.reference);
                let title = if session.title.trim().is_empty() {
                    "Untitled session".to_string()
                } else {
                    session.title.clone()
                };
                (title, session_state(running, waiting, failed), None, None)
            }
            None => ("Session".to_string(), LinkState::Missing, None, None),
        },
        "pilot" => {
            let state = if running {
                LinkState::Running
            } else {
                LinkState::Idle
            };
            match input.pilots {
                Some(pilots) => match pilots.iter().find(|(id, _)| *id == link.reference) {
                    Some((_, name)) => (name.clone(), state, None, None),
                    None => ("Pilot".to_string(), LinkState::Missing, None, None),
                },
                None => ("Pilot".to_string(), state, None, None),
            }
        }
        "pr" => {
            let label = parse_pr_url(&link.reference)
                .map(|(_, number)| format!("PR #{number}"))
                .unwrap_or_else(|| "PR".to_string());
            let state = extra
                .and_then(|extra| extra.state)
                .unwrap_or(LinkState::Unknown);
            (label, state, extra.and_then(|extra| extra.checks), None)
        }
        "worktree" => {
            let label = extra
                .and_then(|extra| extra.branch.clone())
                .unwrap_or_else(|| {
                    Path::new(&link.reference)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| link.reference.clone())
                });
            (
                label,
                extra
                    .and_then(|extra| extra.state)
                    .unwrap_or(LinkState::Idle),
                None,
                extra.and_then(|extra| extra.diff),
            )
        }
        _ => return None,
    };
    Some(LinkChip {
        kind: link.kind.clone(),
        reference: link.reference.clone(),
        label,
        state,
        checks,
        diff,
    })
}

/// Where the slow facts come from: `gh` and Git, or a test's stand-in. Every
/// call blocks; the lookup runs on the background executor.
pub(crate) trait LinkSource: Send + Sync {
    /// The current branch's PR in `cwd` as (url, number, state).
    fn pr_status(&self, cwd: &Path) -> Result<Option<(String, i64, String)>, String>;
    /// The check states (`pass`, `fail`, `pending`, …) of PR `number`.
    fn pr_checks(&self, cwd: &Path, repo: &str, number: i64) -> Result<Vec<String>, String>;
    fn branch(&self, path: &Path) -> Option<String>;
    fn diff(&self, path: &Path) -> Option<(i64, i64)>;
}

/// `gh` and Git, through gespan-core.
pub(crate) struct GitSource;

impl LinkSource for GitSource {
    fn pr_status(&self, cwd: &Path) -> Result<Option<(String, i64, String)>, String> {
        Ok(gespan_core::scm::pr_status(cwd)?.map(|pr| (pr.url, pr.number, pr.state)))
    }

    fn pr_checks(&self, cwd: &Path, repo: &str, number: i64) -> Result<Vec<String>, String> {
        Ok(gespan_core::scm::pr_checks(cwd, repo, number)?
            .checks
            .into_iter()
            .map(|check| check.state)
            .collect())
    }

    fn branch(&self, path: &Path) -> Option<String> {
        gespan_core::git::snapshot(path).ok()?.current
    }

    fn diff(&self, path: &Path) -> Option<(i64, i64)> {
        gespan_core::scm::diff_stats(path)
            .ok()
            .map(|stats| (stats.additions, stats.deletions))
    }
}

/// One PR or worktree link to look up. `cwds` are the folders the card works
/// in (its worktree links, its sessions' folders, its projects), best first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LookupRequest {
    pub kind: String,
    pub reference: String,
    pub cwds: Vec<String>,
}

fn is_remote(path: &str) -> bool {
    gespan_core::files::remote::is_remote(Path::new(path))
}

/// Look every request up. `pr_status` runs once per folder. A PR whose card
/// works only in remote workspaces is unknown, without an error: `gh` there
/// can't be asked.
pub(crate) fn lookup(
    source: &dyn LinkSource,
    requests: &[LookupRequest],
) -> HashMap<(String, String), LinkExtra> {
    let mut statuses: HashMap<String, Option<(String, i64, String)>> = HashMap::new();
    let mut out = HashMap::new();
    for request in requests {
        let key = (request.kind.clone(), request.reference.clone());
        let extra = match request.kind.as_str() {
            "worktree" => {
                if is_remote(&request.reference) {
                    LinkExtra::default()
                } else {
                    let path = Path::new(&request.reference);
                    LinkExtra {
                        state: None,
                        checks: None,
                        branch: source.branch(path),
                        diff: source.diff(path),
                    }
                }
            }
            "pr" => {
                let local: Vec<&String> = request.cwds.iter().filter(|c| !is_remote(c)).collect();
                if local.is_empty() && !request.cwds.is_empty() {
                    LinkExtra {
                        state: Some(LinkState::Unknown),
                        ..LinkExtra::default()
                    }
                } else {
                    let parsed = parse_pr_url(&request.reference);
                    let mut state = None;
                    for cwd in &local {
                        let status = statuses
                            .entry((*cwd).clone())
                            .or_insert_with(|| source.pr_status(Path::new(cwd)).ok().flatten());
                        if let Some((url, number, pr)) = status {
                            let same_url = url.trim_end_matches('/')
                                == request.reference.trim_end_matches('/');
                            let same_number = parsed.as_ref().is_some_and(|(repo, n)| {
                                n == number && url.contains(&format!("/{repo}/pull/"))
                            });
                            if same_url || same_number {
                                state = Some(pr_state(pr));
                                break;
                            }
                        }
                    }
                    let checks = parsed.and_then(|(repo, number)| {
                        let cwd = local
                            .first()
                            .map(|cwd| Path::new(cwd.as_str()).to_path_buf())
                            .unwrap_or_else(std::env::temp_dir);
                        let states = source.pr_checks(&cwd, &repo, number).ok()?;
                        checks_state(states.iter().map(String::as_str))
                    });
                    LinkExtra {
                        state: Some(state.unwrap_or(LinkState::Unknown)),
                        checks,
                        ..LinkExtra::default()
                    }
                }
            }
            _ => continue,
        };
        out.insert(key, extra);
    }
    out
}

/// The lookups the board's links need: one per PR and worktree link (a link
/// on two cards is looked up once, with both cards' folders).
pub(crate) fn lookup_requests(
    links: &[CardLink],
    sessions: &HashMap<String, SessionInfo>,
    card_projects: &HashMap<String, Vec<String>>,
    fallback: Option<&str>,
) -> Vec<LookupRequest> {
    let mut requests: Vec<LookupRequest> = Vec::new();
    for link in links
        .iter()
        .filter(|link| matches!(link.kind.as_str(), "pr" | "worktree"))
    {
        let mut cwds: Vec<String> = Vec::new();
        if link.kind == "pr" {
            let card = links.iter().filter(|other| other.card_id == link.card_id);
            for other in card.clone().filter(|other| other.kind == "worktree") {
                cwds.push(other.reference.clone());
            }
            for other in card.filter(|other| other.kind == "session") {
                if let Some(session) = sessions.get(&other.reference) {
                    cwds.push(session.cwd.clone());
                }
            }
            cwds.extend(
                card_projects
                    .get(&link.card_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            if cwds.is_empty() {
                cwds.extend(fallback.map(str::to_string));
            }
        }
        match requests
            .iter_mut()
            .find(|request| request.kind == link.kind && request.reference == link.reference)
        {
            Some(request) => request.cwds.extend(cwds),
            None => requests.push(LookupRequest {
                kind: link.kind.clone(),
                reference: link.reference.clone(),
                cwds,
            }),
        }
    }
    for request in &mut requests {
        let mut seen = BTreeSet::new();
        request
            .cwds
            .retain(|cwd| !cwd.is_empty() && seen.insert(cwd.clone()));
    }
    requests
}

impl super::BoardView {
    /// Read the linked sessions (for chips and agent fields) and the Pilot
    /// roster. Local reads only.
    pub(super) fn read_link_sessions(&mut self, cx: &gpui::App) -> Result<(), String> {
        let keys: BTreeSet<&str> = self
            .link_rows
            .iter()
            .filter(|link| link.kind == "session")
            .map(|link| link.reference.as_str())
            .collect();
        let mut sessions = HashMap::new();
        let mut facts = HashMap::new();
        if !keys.is_empty() {
            let store = crate::runtime::locked(&self.store)?;
            let all = store.sessions()?;
            for key in keys {
                let Some(native) = store.resolve_existing_session_id(key) else {
                    continue;
                };
                let Some(session) = all.iter().find(|session| session.id == native) else {
                    continue;
                };
                let settings = store.agent_settings(native).ok().flatten();
                let cwd = settings
                    .as_ref()
                    .and_then(|settings| settings.worktree_cwd.clone())
                    .filter(|cwd| !cwd.is_empty())
                    .unwrap_or_else(|| session.cwd.clone());
                sessions.insert(
                    key.to_string(),
                    SessionInfo {
                        title: session.title.clone(),
                        cwd,
                    },
                );
                facts.insert(
                    key.to_string(),
                    super::agent_fields::facts(
                        &session.provider,
                        session.updated_at,
                        settings.as_ref(),
                    ),
                );
            }
        }
        let mut per_card: HashMap<String, Vec<super::agent_fields::AgentFacts>> = HashMap::new();
        for link in self.link_rows.iter().filter(|link| link.kind == "session") {
            if let Some(fact) = facts.get(&link.reference) {
                per_card
                    .entry(link.card_id.clone())
                    .or_default()
                    .push(fact.clone());
            }
        }
        self.agent = per_card
            .into_iter()
            .map(|(card, facts)| (card, super::agent_fields::agent_values(&facts)))
            .collect();
        self.link_sessions = sessions;
        self.pilot_names = super::turns::pilot_roster(cx);
        Ok(())
    }

    /// The newest run event per card of the board, in one query.
    pub(super) fn read_run_events(
        &mut self,
        board_id: &str,
    ) -> Result<HashMap<String, String>, String> {
        if self.run_reader.is_none() {
            let conn = rusqlite::Connection::open_with_flags(
                &self.db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|e| e.to_string())?;
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            self.run_reader = Some(conn);
        }
        latest_run_events(self.run_reader.as_ref().expect("opened above"), board_id)
    }

    /// Build the chips from the last reads and the app's live turns.
    pub(super) fn rebuild_chips(&mut self, cx: &gpui::App) {
        let running = super::turns::running_links(cx);
        let waiting = super::turns::waiting_sessions(cx);
        let running_set: BTreeSet<(String, String)> = running.iter().cloned().collect();
        self.chips = build_chips(&ChipInputs {
            links: &self.link_rows,
            running: &running_set,
            waiting: &waiting,
            sessions: &self.link_sessions,
            pilots: self.pilot_names.as_deref(),
            last_events: &self.last_events,
            extras: &self.link_extras,
        });
        self.turns_seen = (running, waiting);
    }

    /// A turn started, settled or began waiting somewhere in the app. Chips
    /// and the running filter follow the live turns; PR and worktree facts
    /// are looked up again only when the settled turn belongs to a session
    /// linked to one of this board's cards.
    pub(super) fn turns_changed(&mut self, cx: &mut gpui::Context<Self>) {
        let (seq, settled) = super::turns::settled_since(self.settle_seen, cx);
        self.settle_seen = seq;
        if !self.visible || self.detail.is_none() {
            return;
        }
        let live = (
            super::turns::running_links(cx),
            super::turns::waiting_sessions(cx),
        );
        if live != self.turns_seen {
            self.rebuild_chips(cx);
            if let Err(error) = self.refresh_query(cx) {
                self.error = Some(error);
            }
            cx.notify();
        }
        let linked = settled.iter().any(|key| {
            self.link_rows
                .iter()
                .any(|link| link.kind == "session" && &link.reference == key)
        });
        if linked {
            self.refresh_link_status(cx);
        }
    }

    /// Look the board's PR and worktree links up on the background executor.
    pub(super) fn refresh_link_status(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.visible {
            return;
        }
        self.status_board = self.open_board_id.clone();
        self.status_lookups += 1;
        let card_projects: HashMap<String, Vec<String>> = self
            .extras
            .iter()
            .map(|(card, extras)| (card.clone(), extras.projects.clone()))
            .collect();
        let fallback = self.projects.first().map(|(path, _)| path.as_str());
        let requests = lookup_requests(
            &self.link_rows,
            &self.link_sessions,
            &card_projects,
            fallback,
        );
        if requests.is_empty() {
            self.status_task = None;
            if !self.link_extras.is_empty() {
                self.link_extras.clear();
                self.rebuild_chips(cx);
            }
            return;
        }
        let source = self.link_source.clone();
        let board = self.status_board.clone();
        let found = cx
            .background_executor()
            .spawn(async move { lookup(source.as_ref(), &requests) });
        self.status_task = Some(cx.spawn(async move |this, cx| {
            let extras = found.await;
            let _ = this.update(cx, |this, cx| {
                if this.status_board == board {
                    // A PR this refresh sees open or merged is the linked
                    // cards' pr_opened / pr_merged (sent once).
                    for ((kind, url), extra) in &extras {
                        let event = match extra.state {
                            Some(LinkState::Open) if kind == "pr" => LinkedWorkEvent::PrOpened,
                            Some(LinkState::Merged) if kind == "pr" => LinkedWorkEvent::PrMerged,
                            _ => continue,
                        };
                        super::turns::pr_seen(url, event, cx);
                    }
                    this.link_extras = extras;
                    this.rebuild_chips(cx);
                    cx.notify();
                }
            });
        }));
    }

    /// The chips a card shows.
    #[cfg(test)]
    pub(crate) fn link_chips(&self, card_id: &str) -> &[LinkChip] {
        self.chips.get(card_id).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_state_orders_waiting_over_running_over_failed_over_completed() {
        assert_eq!(session_state(true, true, true), LinkState::Waiting);
        assert_eq!(session_state(true, false, true), LinkState::Running);
        assert_eq!(session_state(false, false, true), LinkState::Failed);
        assert_eq!(session_state(false, false, false), LinkState::Completed);
    }

    #[test]
    fn checks_state_prefers_failing_then_pending_and_ignores_skipped() {
        assert_eq!(
            checks_state(["pass", "fail", "pending"]),
            Some(ChecksState::Failing)
        );
        assert_eq!(
            checks_state(["pass", "pending"]),
            Some(ChecksState::Pending)
        );
        assert_eq!(
            checks_state(["pass", "skipping"]),
            Some(ChecksState::Passing)
        );
        assert_eq!(checks_state(["skipping", "cancel"]), None);
        assert_eq!(checks_state([]), None);
    }

    #[test]
    fn parse_pr_url_reads_only_github_pull_links() {
        assert_eq!(
            parse_pr_url("https://github.com/acme/app/pull/42"),
            Some(("acme/app".into(), 42))
        );
        assert_eq!(
            parse_pr_url("https://github.com/acme/app/pull/42/?x=1#c"),
            Some(("acme/app".into(), 42))
        );
        assert_eq!(parse_pr_url("https://github.com/acme/app/issues/42"), None);
        assert_eq!(parse_pr_url("http://github.com/acme/app/pull/42"), None);
        assert_eq!(parse_pr_url("https://github.com/acme/app/pull/0"), None);
        assert_eq!(
            parse_pr_url("https://github.com/acme/app/pull/4/files"),
            None
        );
        assert_eq!(parse_pr_url("https://evil.com/acme/app/pull/4"), None);
    }
}
