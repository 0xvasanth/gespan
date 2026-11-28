//! The Pull requests page's review queue: which pull requests it lists, under which
//! view and filters, grouped by who they are waiting on, and what it keeps
//! between launches.
//!
//! Everything here works on fetched [`Item`]s and is the same for every
//! host: the adapters fill the review fields ([`Item::reviews`],
//! [`Item::check_summary`]...), and this module decides what they mean.
use super::{Item, Seen, ViewState};
use crate::store::Store;
use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// Rows on one page of the list.
pub const PAGE_SIZE: usize = 25;
/// Where the Pull requests page keeps its views, filters and settings.
pub const STATE_KEY: &str = "native-git-state";

/// Who a pull request is waiting on, in the order the list shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    You,
    Others,
    Ready,
}
impl Group {
    pub const ALL: [Group; 3] = [Group::You, Group::Others, Group::Ready];
    pub fn label(self) -> &'static str {
        match self {
            Group::You => "Waiting on you",
            Group::Others => "Waiting on others",
            Group::Ready => "Ready to merge",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Group::You => "you",
            Group::Others => "others",
            Group::Ready => "ready",
        }
    }
}

/// How a why-line reads: urgent (a review asked of you), broken (failed
/// checks) or plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Hot,
    Bad,
}

/// Where a pull request sits and the line under its title saying why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub group: Group,
    pub why: String,
    pub tone: Tone,
}

fn same(a: &str, b: &str) -> bool {
    !a.is_empty() && a.trim().eq_ignore_ascii_case(b.trim())
}
/// A login without GitHub's `app/` prefix or `[bot]` suffix.
fn bare(login: &str) -> &str {
    let login = login.trim();
    let login = login.strip_prefix("app/").unwrap_or(login);
    login.strip_suffix("[bot]").unwrap_or(login)
}

/// Dependency and housekeeping bots, whose pull requests fold into Bots.
pub fn is_bot(login: &str) -> bool {
    let raw = login.trim().to_lowercase();
    if raw.starts_with("app/") || raw.ends_with("[bot]") {
        return true;
    }
    matches!(
        bare(&raw),
        "dependabot"
            | "dependabot-preview"
            | "renovate"
            | "renovate-bot"
            | "renovatebot"
            | "greenkeeper"
            | "snyk-bot"
            | "pre-commit-ci"
            | "github-actions"
            | "mergify"
            | "imgbot"
    )
}

/// An open (or draft) pull request: merged, closed and abandoned ones stay
/// in the feed for automations but off the Pull requests page.
pub fn open_pr(item: &Item) -> bool {
    item.kind == "pr"
        && item.occurrence.is_none()
        && matches!(item.status(), "Open" | "Draft")
        && !item.state.eq_ignore_ascii_case("abandoned")
}
/// What the Pull requests page lists: open pull requests in your repositories.
pub fn listed(item: &Item) -> bool {
    open_pr(item) && !item.other_repo
}

/// You opened it.
pub fn is_mine(item: &Item) -> bool {
    same(&item.viewer, &item.author)
}
/// Your review is asked for.
pub fn review_requested_from_me(item: &Item) -> bool {
    item.attention_reason
        .eq_ignore_ascii_case("review_requested")
        || item
            .requested_reviewers
            .iter()
            .any(|login| same(login, &item.viewer))
}
/// You have reviewed it.
pub fn reviewed_by_me(item: &Item) -> bool {
    item.reviews
        .iter()
        .any(|review| same(&review.login, &item.viewer))
}
/// It belongs in Needs me: yours, asked of you, reviewed by you, assigned
/// to you, or flagged for you by the host (a GitLab or Azure DevOps to-do).
pub fn involves_me(item: &Item) -> bool {
    is_mine(item)
        || review_requested_from_me(item)
        || reviewed_by_me(item)
        || item
            .assignees
            .iter()
            .any(|person| same(&person.login, &item.viewer))
        || !item.attention_reason.is_empty()
}

fn who(login: &str, item: &Item) -> String {
    if same(login, &item.viewer) {
        "you".into()
    } else {
        bare(login).to_string()
    }
}
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
fn changes_requested(item: &Item) -> Vec<String> {
    item.reviews
        .iter()
        .filter(|review| review.state == "changes_requested")
        .map(|review| who(&review.login, item))
        .collect()
}
fn approvers(item: &Item) -> Vec<String> {
    let mut names: Vec<String> = item
        .reviews
        .iter()
        .filter(|review| review.state == "approved")
        .map(|review| who(&review.login, item))
        .collect();
    // "you" reads first: "approved by you and cubic".
    names.sort_by_key(|name| name != "you");
    names
}
fn approved(item: &Item) -> bool {
    match item.review_decision.as_str() {
        "approved" => true,
        "" => !approvers(item).is_empty() && changes_requested(item).is_empty(),
        _ => false,
    }
}
fn failed_line(item: &Item) -> String {
    let checks = &item.check_summary;
    match (checks.failed, checks.failed_names.as_slice()) {
        (1, [name, ..]) => format!("{name} failed"),
        (1, _) => "a check failed".into(),
        (n, _) => format!("{n} checks failed"),
    }
}

/// Where `item` waits and why. Drafts wait on their author; your pull
/// request waits on you once a reviewer asks for changes or a check fails;
/// one asked of you waits on you; an approved one with green checks is
/// ready to merge; everything else waits on someone else.
pub fn waiting(item: &Item) -> Waiting {
    let mine = is_mine(item);
    let failed = item.check_summary.failed > 0;
    let pending = item.check_summary.pending > 0;
    let others = |why: String| Waiting {
        group: Group::Others,
        why,
        tone: Tone::Plain,
    };
    if item.draft {
        return others(if mine {
            "your draft".into()
        } else {
            format!("draft by {}", bare(&item.author))
        });
    }
    let changes = changes_requested(item);
    let changes_from_others: Vec<String> =
        changes.iter().filter(|n| *n != "you").cloned().collect();
    if mine
        && (failed
            || !changes_from_others.is_empty()
            || item.review_decision == "changes_requested")
    {
        let mut parts = Vec::new();
        if !changes_from_others.is_empty() {
            parts.push(format!(
                "{} requested changes",
                and_list(&changes_from_others)
            ));
        } else if item.review_decision == "changes_requested" {
            parts.push("changes requested".into());
        }
        if failed {
            parts.push(failed_line(item));
        }
        return Waiting {
            group: Group::You,
            why: parts.join(" · "),
            tone: if failed { Tone::Bad } else { Tone::Hot },
        };
    }
    if !mine && review_requested_from_me(item) {
        return Waiting {
            group: Group::You,
            why: if item.author.is_empty() {
                "your review is requested".into()
            } else {
                format!("{} asked for your review", bare(&item.author))
            },
            tone: Tone::Hot,
        };
    }
    if approved(item) && !failed {
        let names = approvers(item);
        let by = if names.is_empty() {
            "approved".to_string()
        } else {
            format!("approved by {}", and_list(&names))
        };
        if pending {
            return others(format!("{by} · checks running"));
        }
        return Waiting {
            group: Group::Ready,
            why: format!("{by} · ready to merge"),
            tone: Tone::Plain,
        };
    }
    if mine {
        let waiting_on: Vec<String> = item
            .requested_reviewers
            .iter()
            .map(|login| bare(login).to_string())
            .collect();
        return others(if !waiting_on.is_empty() {
            format!("waiting on {}", and_list(&waiting_on))
        } else if !approvers(item).is_empty() {
            "waiting on a second reviewer".into()
        } else if failed {
            failed_line(item)
        } else {
            "waiting for review".into()
        });
    }
    if changes.iter().any(|name| name == "you") {
        return others(format!(
            "you requested changes · waiting on {}",
            bare(&item.author)
        ));
    }
    if failed {
        return others(failed_line(item));
    }
    others(match attention_label(&item.attention_reason) {
        Some(label) => label,
        None if item.author.is_empty() => "open".into(),
        None => format!("opened by {}", bare(&item.author)),
    })
}

/// What a host's to-do reason says, in words; `None` when there is none.
pub fn attention_label(reason: &str) -> Option<String> {
    let reason = reason.trim().to_lowercase();
    if reason.is_empty() {
        return None;
    }
    Some(
        match reason.as_str() {
            "assigned" => "assigned to you",
            "mentioned" | "directly_addressed" => "mentioned you",
            "review_requested" => "review requested",
            "review_submitted" => "review submitted",
            "approval_required" => "approval required",
            "build_failed" => "pipeline failed",
            "unmergeable" => "cannot be merged",
            "merge_train_removed" => "removed from merge train",
            "member_access_requested" => "access requested",
            "marked" => "added to your to-dos",
            other => return Some(other.replace('_', " ")),
        }
        .into(),
    )
}

/// The pill at the end of a row, from its checks: failures, then progress
/// while some run, else Passing. `None` when the host reported no checks.
pub fn check_pill(item: &Item) -> Option<(String, Tone)> {
    let checks = &item.check_summary;
    if checks.is_empty() {
        return None;
    }
    Some(if checks.failed > 0 {
        (format!("{} failed", checks.failed), Tone::Bad)
    } else if checks.pending > 0 {
        (format!("{}/{}", checks.passed, checks.total), Tone::Hot)
    } else {
        ("Passing".into(), Tone::Plain)
    })
}

/// The short repository name rows show: the last path segment.
pub fn short_repo(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}
/// GitLab numbers merge requests `!214`; the rest use `#`.
pub fn number_label(item: &Item) -> String {
    if item.provider == "gitlab" {
        format!("!{}", item.number)
    } else {
        format!("#{}", item.number)
    }
}

/// The list's views. Saved views carry their own filters over a base view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    NeedsMe,
    Mine,
    All,
    Bots,
}
impl View {
    pub const TABS: [View; 3] = [View::NeedsMe, View::Mine, View::All];
    pub fn id(self) -> &'static str {
        match self {
            View::NeedsMe => "needs_me",
            View::Mine => "mine",
            View::All => "all",
            View::Bots => "bots",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            View::NeedsMe => "Needs me",
            View::Mine => "Mine",
            View::All => "All open",
            View::Bots => "Bots",
        }
    }
    fn includes(self, item: &Item, fold_bots: bool) -> bool {
        let bot = is_bot(&item.author);
        match self {
            View::Bots => bot,
            _ if bot && fold_bots => false,
            View::NeedsMe => involves_me(item),
            View::Mine => is_mine(item),
            View::All => true,
        }
    }
}

/// Drafts: all of them, none, or only your own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Drafts {
    All,
    Hide,
    #[default]
    MineOnly,
}
impl Drafts {
    pub const ALL: [Drafts; 3] = [Drafts::All, Drafts::MineOnly, Drafts::Hide];
    pub fn label(self) -> &'static str {
        match self {
            Drafts::All => "Show all",
            Drafts::Hide => "Hide",
            Drafts::MineOnly => "Mine only",
        }
    }
}

/// The filters "+ Add filter" offers beyond Repos, Host, Author and Drafts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extra {
    Label,
    Reviewer,
    Checks,
    Size,
    Updated,
}
impl Extra {
    pub const ALL: [Extra; 5] = [
        Extra::Label,
        Extra::Reviewer,
        Extra::Checks,
        Extra::Size,
        Extra::Updated,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Extra::Label => "Label",
            Extra::Reviewer => "Reviewer",
            Extra::Checks => "Checks",
            Extra::Size => "Size",
            Extra::Updated => "Updated",
        }
    }
    /// The choices a filter offers that do not come from the items.
    pub fn fixed_options(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Extra::Checks => &[
                ("failing", "Failing"),
                ("pending", "Running"),
                ("passing", "Passing"),
            ],
            Extra::Size => &[
                ("s", "Small (under 50 lines)"),
                ("m", "Medium (50–300)"),
                ("l", "Large (over 300)"),
            ],
            Extra::Updated => &[
                ("today", "Today"),
                ("7d", "Last 7 days"),
                ("30d", "Last 30 days"),
            ],
            Extra::Label | Extra::Reviewer => &[],
        }
    }
}

fn size_of(item: &Item) -> &'static str {
    match item.additions + item.deletions {
        n if n < 50 => "s",
        n if n <= 300 => "m",
        _ => "l",
    }
}
fn checks_of(item: &Item) -> &'static str {
    let checks = &item.check_summary;
    if checks.failed > 0 {
        "failing"
    } else if checks.pending > 0 {
        "pending"
    } else if checks.total > 0 {
        "passing"
    } else {
        ""
    }
}
fn updated_since(window: &str) -> i64 {
    let now = Local::now();
    match window {
        "today" => Local
            .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
            .single()
            .map(|d| d.timestamp_millis())
            .unwrap_or(0),
        "7d" => now.timestamp_millis() - 7 * 86_400_000,
        "30d" => now.timestamp_millis() - 30 * 86_400_000,
        _ => 0,
    }
}

/// `provider:repo`, lowercased: how repositories are named in settings and
/// filters.
pub fn repo_key(provider: &str, repo: &str) -> String {
    format!("{}:{}", provider, repo.trim().to_lowercase())
}

/// One view's filters. Empty lists match everything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Filters {
    /// Repositories (`repo_key`) left out of this view.
    pub hidden_repos: Vec<String>,
    /// Hosts shown; empty is every host.
    pub hosts: Vec<String>,
    pub authors: Vec<String>,
    pub drafts: Drafts,
    /// Filters added with "+ Add filter", in the order they were added.
    pub extras: Vec<Extra>,
    pub labels: Vec<String>,
    pub reviewers: Vec<String>,
    pub checks: Vec<String>,
    pub sizes: Vec<String>,
    /// `today`, `7d` or `30d`; empty is any time.
    pub updated: String,
}
impl Filters {
    /// How many filters narrow the view, for the Filters button's count.
    pub fn active(&self) -> usize {
        [
            !self.hidden_repos.is_empty(),
            !self.hosts.is_empty(),
            !self.authors.is_empty(),
            self.drafts != Drafts::default(),
            !self.labels.is_empty(),
            !self.reviewers.is_empty(),
            !self.checks.is_empty(),
            !self.sizes.is_empty(),
            !self.updated.is_empty(),
        ]
        .into_iter()
        .filter(|on| *on)
        .count()
    }
    pub fn values(&mut self, extra: Extra) -> &mut Vec<String> {
        match extra {
            Extra::Label => &mut self.labels,
            Extra::Reviewer => &mut self.reviewers,
            Extra::Checks => &mut self.checks,
            Extra::Size => &mut self.sizes,
            // Updated is one window; the list holds at most one.
            Extra::Updated => unreachable!("updated is a single value"),
        }
    }
    /// Drops an added filter and what it held.
    pub fn remove_extra(&mut self, extra: Extra) {
        self.extras.retain(|e| *e != extra);
        match extra {
            Extra::Updated => self.updated.clear(),
            other => self.values(other).clear(),
        }
    }
    pub fn matches(&self, item: &Item, search: &str) -> bool {
        let any = |list: &[String], value: &str| {
            list.is_empty() || list.iter().any(|v| v.eq_ignore_ascii_case(value))
        };
        if self
            .hidden_repos
            .contains(&repo_key(&item.provider, &item.repo))
        {
            return false;
        }
        if !any(&self.hosts, &item.provider) || !any(&self.authors, bare(&item.author)) {
            return false;
        }
        if item.draft {
            match self.drafts {
                Drafts::All => {}
                Drafts::Hide => return false,
                Drafts::MineOnly if !is_mine(item) => return false,
                Drafts::MineOnly => {}
            }
        }
        if !self.labels.is_empty()
            && !item
                .labels
                .iter()
                .any(|label| any(&self.labels, &label.name))
        {
            return false;
        }
        if !self.reviewers.is_empty()
            && !item
                .requested_reviewers
                .iter()
                .map(String::as_str)
                .chain(item.reviews.iter().map(|r| r.login.as_str()))
                .any(|login| any(&self.reviewers, bare(login)))
        {
            return false;
        }
        if !any(&self.checks, checks_of(item)) || !any(&self.sizes, size_of(item)) {
            return false;
        }
        let since = updated_since(&self.updated);
        if since > 0 && item.updated_ms() < since {
            return false;
        }
        let text = format!(
            "{} {} {} {} {} {}",
            item.title,
            item.repo,
            item.number,
            number_label(item),
            item.author,
            item.labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        search
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }
}

/// A view saved with "Save as view…": a name, the view it narrows, and
/// its filters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub base: View,
    pub filters: Filters,
}

/// A repository added by hand in Settings › Pull requests.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddedRepo {
    pub provider: String,
    pub repo: String,
}

/// What the Pull requests page keeps: the open view, each view's filters, saved
/// views, and Settings › Pull requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// `View::id`, or `saved:<id>`.
    pub view: String,
    pub filters: BTreeMap<String, Filters>,
    pub saved: Vec<SavedView>,
    pub opens_on: View,
    pub keep_filters: bool,
    pub notify: bool,
    pub fold_bots: bool,
    pub other_repos: bool,
    /// Project repositories (`repo_key`) switched off in Settings › Pull requests.
    pub hidden_repos: Vec<String>,
    pub added_repos: Vec<AddedRepo>,
    /// The project folder the user picked to review a repository (`repo_key`)
    /// that more than one open project points at. See [`crate::code_review`].
    pub review_folders: BTreeMap<String, String>,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            view: View::NeedsMe.id().into(),
            filters: BTreeMap::new(),
            saved: Vec::new(),
            opens_on: View::NeedsMe,
            keep_filters: true,
            notify: true,
            fold_bots: true,
            other_repos: true,
            hidden_repos: Vec::new(),
            added_repos: Vec::new(),
            review_folders: BTreeMap::new(),
        }
    }
}

/// The view the list shows: a built-in one or a saved one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Current<'a> {
    Builtin(View),
    Saved(&'a SavedView),
}

impl Prefs {
    pub fn current(&self) -> Current<'_> {
        if let Some(id) = self.view.strip_prefix("saved:") {
            if let Some(saved) = self.saved.iter().find(|v| v.id == id) {
                return Current::Saved(saved);
            }
        }
        Current::Builtin(
            [View::NeedsMe, View::Mine, View::All, View::Bots]
                .into_iter()
                .find(|v| v.id() == self.view)
                .unwrap_or_default(),
        )
    }
    /// The built-in view under the current one.
    pub fn base(&self) -> View {
        match self.current() {
            Current::Builtin(view) => view,
            Current::Saved(saved) => saved.base,
        }
    }
    pub fn filters(&self) -> Filters {
        match self.current() {
            Current::Saved(saved) => saved.filters.clone(),
            Current::Builtin(view) => self.filters.get(view.id()).cloned().unwrap_or_default(),
        }
    }
    /// Changes the current view's filters: a saved view's own, else the
    /// built-in view's.
    pub fn update_filters(&mut self, change: impl FnOnce(&mut Filters)) {
        let view = self.view.clone();
        if let Some(id) = view.strip_prefix("saved:") {
            if let Some(saved) = self.saved.iter_mut().find(|v| v.id == id) {
                change(&mut saved.filters);
                return;
            }
        }
        let id = self.base().id();
        change(self.filters.entry(id.to_string()).or_default());
    }
    pub fn set_view(&mut self, id: &str) {
        self.view = id.to_string();
    }
    /// Saves the current view and filters under `name`, and opens it.
    pub fn save_view(&mut self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let base = self.base();
        let filters = self.filters();
        let id = format!(
            "{}",
            self.saved
                .iter()
                .filter_map(|v| v.id.parse::<u64>().ok())
                .max()
                .map_or(1, |n| n + 1)
        );
        self.saved.push(SavedView {
            id: id.clone(),
            name: name.to_string(),
            base,
            filters,
        });
        self.view = format!("saved:{id}");
        Some(id)
    }
    pub fn delete_view(&mut self, id: &str) {
        self.saved.retain(|v| v.id != id);
        if self.view == format!("saved:{id}") {
            self.view = self.opens_on.id().into();
        }
    }
    /// Pull requests in the current view, filtered and searched, newest
    /// first. Rows from other repos are left to their fold.
    pub fn visible<'a>(&self, items: &'a [Item], search: &str) -> Vec<&'a Item> {
        let base = self.base();
        let filters = self.filters();
        let mut rows: Vec<&Item> = items
            .iter()
            .filter(|item| listed(item))
            .filter(|item| {
                !self
                    .hidden_repos
                    .contains(&repo_key(&item.provider, &item.repo))
            })
            .filter(|item| base.includes(item, self.fold_bots))
            .filter(|item| filters.matches(item, search))
            .collect();
        rows.sort_by(|a, b| {
            b.updated_ms()
                .cmp(&a.updated_ms())
                .then_with(|| a.title.cmp(&b.title))
        });
        rows
    }
    /// How many pull requests a view holds before its filters, for tab
    /// counts. Needs me counts only what waits on you.
    pub fn count(&self, items: &[Item], view: View) -> usize {
        items
            .iter()
            .filter(|item| listed(item))
            .filter(|item| {
                !self
                    .hidden_repos
                    .contains(&repo_key(&item.provider, &item.repo))
            })
            .filter(|item| view.includes(item, self.fold_bots))
            .filter(|item| view != View::NeedsMe || waiting(item).group == Group::You)
            .count()
    }
    /// Bot pull requests the current view leaves out, for "N bot PRs hidden".
    pub fn hidden_bots(&self, items: &[Item]) -> usize {
        if !self.fold_bots || self.base() == View::Bots {
            return 0;
        }
        items
            .iter()
            .filter(|item| listed(item))
            .filter(|item| is_bot(&item.author))
            .count()
    }
}

/// Rows grouped by who they wait on, groups in [`Group::ALL`] order and
/// rows in the order given.
pub fn grouped<'a>(rows: &[&'a Item]) -> Vec<(Group, Vec<&'a Item>)> {
    Group::ALL
        .into_iter()
        .map(|group| {
            (
                group,
                rows.iter()
                    .copied()
                    .filter(|item| waiting(item).group == group)
                    .collect::<Vec<_>>(),
            )
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// One page of the grouped list: rows `start..end` of `total`, in group
/// order, still under their group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<'a> {
    pub index: usize,
    pub pages: usize,
    pub start: usize,
    pub end: usize,
    pub total: usize,
    pub groups: Vec<(Group, Vec<&'a Item>)>,
    /// Each group's full size, for its heading.
    pub group_totals: Vec<(Group, usize)>,
}
pub fn page<'a>(rows: &[&'a Item], index: usize) -> Page<'a> {
    let groups = grouped(rows);
    let group_totals = groups.iter().map(|(g, rows)| (*g, rows.len())).collect();
    let ordered: Vec<(Group, &Item)> = groups
        .iter()
        .flat_map(|(group, rows)| rows.iter().map(move |item| (*group, *item)))
        .collect();
    let total = ordered.len();
    let pages = total.div_ceil(PAGE_SIZE).max(1);
    let index = index.min(pages - 1);
    let start = index * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);
    let mut out: Vec<(Group, Vec<&Item>)> = Vec::new();
    for (group, item) in &ordered[start.min(total)..end] {
        match out.last_mut() {
            Some((g, rows)) if g == group => rows.push(item),
            _ => out.push((*group, vec![item])),
        }
    }
    Page {
        index,
        pages,
        start,
        end,
        total,
        groups: out,
        group_totals,
    }
}

/// Holds back pull requests that arrive while the list is open, so it does
/// not jump: they wait behind "N new since HH:MM · Show".
#[derive(Clone, Debug, Default)]
pub struct Arrivals {
    /// What the shown list was seeded for: a view, its filters and search.
    scope: Option<String>,
    shown: HashSet<String>,
    since_ms: i64,
}
impl Arrivals {
    /// Takes in everything when `scope` (a view, its filters and search)
    /// is new. Says whether it was.
    pub fn seed(&mut self, scope: &str, rows: &[&Item], now_ms: i64) -> bool {
        if self.scope.as_deref() == Some(scope) {
            return false;
        }
        self.scope = Some(scope.to_string());
        self.shown = rows.iter().map(|item| item.key()).collect();
        self.since_ms = now_ms;
        true
    }
    /// Splits `rows` into those shown and the count held back. Under a scope
    /// that was never seeded, everything shows.
    pub fn split<'a>(&self, scope: &str, rows: Vec<&'a Item>) -> (Vec<&'a Item>, usize) {
        if self.scope.as_deref() != Some(scope) {
            return (rows, 0);
        }
        let (shown, held): (Vec<&Item>, Vec<&Item>) = rows
            .into_iter()
            .partition(|item| self.shown.contains(&item.key()));
        (shown, held.len())
    }
    /// "Show": everything in `rows` joins the list from now on.
    pub fn show(&mut self, rows: &[&Item], now_ms: i64) {
        self.shown.extend(rows.iter().map(|item| item.key()));
        self.since_ms = now_ms;
    }
    /// When the list last took everything in, for "since HH:MM".
    pub fn since_ms(&self) -> i64 {
        self.since_ms
    }
    /// Forget the seed, so the next one takes everything in.
    pub fn reset(&mut self) {
        self.scope = None;
    }
}

/// Review requests in repositories outside yours, for the folded row:
/// how many, and their repositories in first-seen order.
pub fn other_requests(items: &[Item]) -> (Vec<&Item>, Vec<String>) {
    let rows: Vec<&Item> = items
        .iter()
        .filter(|item| item.other_repo && open_pr(item))
        .collect();
    let mut repos: Vec<String> = Vec::new();
    for item in &rows {
        if !repos.iter().any(|r| r.eq_ignore_ascii_case(&item.repo)) {
            repos.push(item.repo.clone());
        }
    }
    (rows, repos)
}

/// The Pull requests page's whole kept state: [`Prefs`], and the fetch and read state
/// it shares with automations and Home.
#[derive(Clone, Debug)]
pub struct State {
    /// What is fetched and what has been read. Its filters stay at their
    /// defaults so every open pull request and issue is fetched; automations
    /// listen to the same feed.
    pub feed: ViewState,
    pub prefs: Prefs,
}
impl State {
    /// Loads both halves. A first launch after the rename derives the Git
    /// page's settings from the Inbox's: its review view opens Needs me.
    pub fn load(store: &Store) -> Self {
        let mut feed = ViewState::load(store);
        let prefs = store
            .setting(STATE_KEY)
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<Prefs>(&text).ok())
            .unwrap_or_else(|| {
                // Someone who used the Inbox keeps how it opened; a new user
                // opens on Needs me.
                let used_inbox = store.setting("native-inbox-state").ok().flatten().is_some();
                if used_inbox {
                    Prefs::migrated(&feed)
                } else {
                    Prefs::default()
                }
            });
        // The feed's own filters were the Inbox's views; the page filters
        // itself. Open items only: the page lists nothing else, and
        // automations fetch merges and closes on their own.
        feed.filters = super::Filters::default();
        feed.filters.status.open = true;
        feed.source = "github".into();
        let mut state = Self { feed, prefs };
        if !state.prefs.keep_filters {
            state.prefs.filters.clear();
        }
        state.prefs.view = state.prefs.opens_on.id().into();
        state
    }
    pub fn save(&self, store: &Store) -> Result<(), String> {
        self.feed.save(store)?;
        let mut prefs = self.prefs.clone();
        if !prefs.keep_filters {
            prefs.filters.clear();
        }
        store.save_setting(
            STATE_KEY,
            &serde_json::to_string(&prefs).map_err(|e| e.to_string())?,
        )
    }
    pub fn seen(&self) -> &Seen {
        &self.feed.seen
    }
}
impl Prefs {
    /// Settings for a first launch after Inbox became Git.
    pub fn migrated(old: &ViewState) -> Self {
        let mut prefs = Self::default();
        if !(old.filters.review_requested || old.filters.assigned_to_me) {
            // The Inbox opened on All: keep that.
            prefs.opens_on = View::All;
            prefs.view = View::All.id().into();
        }
        prefs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::{CheckSummary, Label, Person, Review};

    fn pr(number: i64, author: &str) -> Item {
        Item {
            provider: "github".into(),
            kind: "pr".into(),
            number,
            title: format!("PR {number}"),
            repo: "0xvasanth/gespan".into(),
            author: author.into(),
            viewer: "vasanth".into(),
            state: "OPEN".into(),
            updated_at: format!("2026-11-26T10:{:02}:00Z", number % 60),
            ..Default::default()
        }
    }
    fn review(login: &str, state: &str) -> Review {
        Review {
            login: login.into(),
            state: state.into(),
        }
    }
    fn checks(passed: u32, failed: u32, pending: u32, names: &[&str]) -> CheckSummary {
        CheckSummary {
            total: passed + failed + pending,
            passed,
            failed,
            pending,
            failed_names: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn a_review_asked_of_you_waits_on_you_and_names_who_asked() {
        let mut item = pr(562, "devon");
        item.requested_reviewers = vec!["Vasanth".into(), "cubic".into()];
        assert_eq!(
            waiting(&item),
            Waiting {
                group: Group::You,
                why: "devon asked for your review".into(),
                tone: Tone::Hot
            }
        );
        // GitLab and Azure DevOps say so through the to-do reason instead.
        let mut todo = pr(214, "priya");
        todo.provider = "gitlab".into();
        todo.attention_reason = "review_requested".into();
        assert_eq!(waiting(&todo).group, Group::You);
    }

    #[test]
    fn your_pull_request_waits_on_you_once_changes_are_requested_or_checks_fail() {
        let mut item = pr(560, "vasanth");
        item.reviews = vec![
            review("mira", "changes_requested"),
            review("cubic", "commented"),
        ];
        item.check_summary = checks(6, 1, 0, &["Windows tests"]);
        let w = waiting(&item);
        assert_eq!(w.group, Group::You);
        assert_eq!(w.why, "mira requested changes · Windows tests failed");
        assert_eq!(w.tone, Tone::Bad, "a failure outranks a review");

        item.check_summary = checks(5, 2, 0, &["a", "b"]);
        assert_eq!(
            waiting(&item).why,
            "mira requested changes · 2 checks failed"
        );

        item.reviews.clear();
        item.check_summary = checks(7, 0, 0, &[]);
        item.review_decision = "changes_requested".into();
        assert_eq!(
            waiting(&item),
            Waiting {
                group: Group::You,
                why: "changes requested".into(),
                tone: Tone::Hot
            },
            "a host that names no reviewer still blocks the author"
        );
    }

    #[test]
    fn approved_and_green_is_ready_and_approved_but_running_is_not() {
        let mut item = pr(557, "lena");
        item.review_decision = "approved".into();
        item.reviews = vec![review("cubic", "approved"), review("vasanth", "approved")];
        item.check_summary = checks(7, 0, 0, &[]);
        assert_eq!(
            waiting(&item),
            Waiting {
                group: Group::Ready,
                why: "approved by you and cubic · ready to merge".into(),
                tone: Tone::Plain
            }
        );
        item.check_summary = checks(4, 0, 3, &[]);
        let running = waiting(&item);
        assert_eq!(running.group, Group::Others);
        assert_eq!(running.why, "approved by you and cubic · checks running");
        item.check_summary = checks(4, 1, 0, &["lint"]);
        assert_ne!(
            waiting(&item).group,
            Group::Ready,
            "a failure is never ready"
        );
        // No checks at all: approval alone is enough.
        item.check_summary = CheckSummary::default();
        assert_eq!(waiting(&item).group, Group::Ready);
    }

    #[test]
    fn a_host_without_a_decision_reads_one_from_the_reviews() {
        let mut item = pr(9, "lena");
        item.provider = "azuredevops".into();
        item.reviews = vec![review("ana", "approved")];
        assert_eq!(waiting(&item).group, Group::Ready);
        item.reviews.push(review("bo", "changes_requested"));
        assert_ne!(waiting(&item).group, Group::Ready);
    }

    #[test]
    fn your_open_pull_request_says_who_it_waits_on() {
        let mut item = pr(20131, "vasanth");
        item.requested_reviewers = vec!["mira".into(), "app/cubic-dev-ai".into()];
        assert_eq!(waiting(&item).why, "waiting on mira and cubic-dev-ai");
        item.requested_reviewers.clear();
        item.reviews = vec![review("mira", "approved")];
        item.review_decision = "review_required".into();
        assert_eq!(waiting(&item).why, "waiting on a second reviewer");
        item.reviews.clear();
        assert_eq!(waiting(&item).why, "waiting for review");
    }

    #[test]
    fn drafts_wait_on_their_author_even_when_checks_fail() {
        let mut item = pr(563, "vasanth");
        item.draft = true;
        item.check_summary = checks(2, 1, 4, &["x"]);
        assert_eq!(waiting(&item).why, "your draft");
        assert_eq!(waiting(&item).group, Group::Others);
        let mut theirs = pr(1, "sam");
        theirs.draft = true;
        theirs.requested_reviewers = vec!["vasanth".into()];
        assert_eq!(waiting(&theirs).why, "draft by sam");
    }

    #[test]
    fn someone_elses_request_you_blocked_waits_on_its_author() {
        let mut item = pr(3, "sam");
        item.reviews = vec![review("vasanth", "changes_requested")];
        assert_eq!(waiting(&item).why, "you requested changes · waiting on sam");
        assert!(involves_me(&item));
    }

    #[test]
    fn check_pills_say_failures_then_progress_then_passing() {
        let mut item = pr(1, "a");
        assert_eq!(check_pill(&item), None);
        item.check_summary = checks(4, 0, 3, &[]);
        assert_eq!(check_pill(&item), Some(("4/7".into(), Tone::Hot)));
        item.check_summary = checks(4, 2, 1, &[]);
        assert_eq!(check_pill(&item), Some(("2 failed".into(), Tone::Bad)));
        item.check_summary = checks(7, 0, 0, &[]);
        assert_eq!(check_pill(&item), Some(("Passing".into(), Tone::Plain)));
    }

    #[test]
    fn bots_are_recognised_however_the_host_spells_them() {
        for login in [
            "app/dependabot",
            "dependabot[bot]",
            "renovate",
            "Renovate[bot]",
            "app/some-app",
        ] {
            assert!(is_bot(login), "{login}");
        }
        for login in ["vasanth", "botanist", "renovator", ""] {
            assert!(!is_bot(login), "{login}");
        }
    }

    fn feed() -> Vec<Item> {
        let mut asked = pr(1, "devon");
        asked.requested_reviewers = vec!["vasanth".into()];
        let mut mine = pr(2, "vasanth");
        mine.requested_reviewers = vec!["mira".into()];
        let mut mine_draft = pr(3, "vasanth");
        mine_draft.draft = true;
        let mut their_draft = pr(4, "sam");
        their_draft.draft = true;
        let other = pr(5, "lena");
        let bot = pr(6, "app/dependabot");
        let mut issue = pr(7, "devon");
        issue.kind = "issue".into();
        let mut elsewhere = pr(8, "zed");
        elsewhere.other_repo = true;
        elsewhere.repo = "acme/web".into();
        elsewhere.requested_reviewers = vec!["vasanth".into()];
        let mut merged = pr(9, "devon");
        merged.state = "MERGED".into();
        merged.requested_reviewers = vec!["vasanth".into()];
        let mut abandoned = pr(10, "devon");
        abandoned.provider = "azuredevops".into();
        abandoned.state = "abandoned".into();
        vec![
            asked,
            mine,
            mine_draft,
            their_draft,
            other,
            bot,
            issue,
            elsewhere,
            merged,
            abandoned,
        ]
    }
    fn numbers(rows: &[&Item]) -> Vec<i64> {
        let mut n: Vec<i64> = rows.iter().map(|i| i.number).collect();
        n.sort();
        n
    }

    #[test]
    fn views_hold_pull_requests_only_and_fold_bots_and_other_repos() {
        let items = feed();
        let mut prefs = Prefs::default();
        assert_eq!(numbers(&prefs.visible(&items, "")), vec![1, 2, 3]);
        prefs.set_view("mine");
        assert_eq!(numbers(&prefs.visible(&items, "")), vec![2, 3]);
        prefs.set_view("all");
        assert_eq!(
            numbers(&prefs.visible(&items, "")),
            vec![1, 2, 3, 5],
            "issues, other drafts, bots, other repos and closed ones stay out"
        );
        assert_eq!(prefs.hidden_bots(&items), 1);
        prefs.set_view("bots");
        assert_eq!(numbers(&prefs.visible(&items, "")), vec![6]);
        assert_eq!(prefs.hidden_bots(&items), 0);
        prefs.fold_bots = false;
        prefs.set_view("all");
        assert!(numbers(&prefs.visible(&items, "")).contains(&6));
        assert_eq!(
            prefs.count(&items, View::NeedsMe),
            1,
            "only what waits on you"
        );
        assert_eq!(
            prefs.count(&items, View::All),
            6,
            "counts come before filters, so drafts count"
        );
        let (rows, repos) = other_requests(&items);
        assert_eq!((rows.len(), repos), (1, vec!["acme/web".to_string()]));
    }

    #[test]
    fn each_view_keeps_its_own_filters_and_settings_hide_repos_everywhere() {
        let mut items = feed();
        items[4].provider = "gitlab".into();
        let mut prefs = Prefs::default();
        prefs.set_view("all");
        prefs.update_filters(|f| f.hosts = vec!["gitlab".into()]);
        assert_eq!(numbers(&prefs.visible(&items, "")), vec![5]);
        prefs.set_view("needs_me");
        assert_eq!(prefs.filters(), Filters::default(), "Needs me has its own");
        prefs.set_view("all");
        assert_eq!(prefs.filters().active(), 1);
        prefs.hidden_repos = vec![repo_key("gitlab", "0xvasanth/Gespan")];
        assert!(prefs.visible(&items, "").is_empty());
    }

    #[test]
    fn filters_narrow_by_drafts_labels_reviewers_checks_size_and_search() {
        let mut item = pr(10, "sam");
        item.labels = vec![Label {
            name: "ui".into(),
            ..Default::default()
        }];
        item.reviews = vec![review("mira", "commented")];
        item.additions = 40;
        item.deletions = 30;
        item.check_summary = checks(1, 1, 0, &[]);
        item.title = "Rail: Git takes Inbox's place".into();
        let mut f = Filters::default();
        assert!(f.matches(&item, "rail git"));
        assert!(!f.matches(&item, "rail board"));
        assert!(f.matches(&item, "#10"));
        f.labels = vec!["UI".into()];
        f.reviewers = vec!["mira".into()];
        f.checks = vec!["failing".into()];
        f.sizes = vec!["m".into()];
        assert!(f.matches(&item, ""));
        f.sizes = vec!["s".into()];
        assert!(!f.matches(&item, ""), "70 lines is medium");
        f.sizes.clear();
        f.checks = vec!["passing".into()];
        assert!(!f.matches(&item, ""));
        f = Filters::default();
        item.draft = true;
        assert!(
            !f.matches(&item, ""),
            "someone else's draft is hidden by default"
        );
        f.drafts = Drafts::All;
        assert!(f.matches(&item, ""));
        item.author = "vasanth".into();
        f.drafts = Drafts::Hide;
        assert!(!f.matches(&item, ""));
        f.authors = vec!["dependabot".into()];
        item.draft = false;
        item.author = "app/dependabot".into();
        assert!(
            f.matches(&item, ""),
            "authors compare without the app/ prefix"
        );
        let mut updated = Filters {
            updated: "7d".into(),
            ..Default::default()
        };
        item.updated_at = "2020-01-01T00:00:00Z".into();
        assert!(!updated.matches(&item, ""));
        updated.remove_extra(Extra::Updated);
        assert!(updated.matches(&item, ""));
    }

    #[test]
    fn saved_views_keep_their_base_and_filters_and_deleting_one_reopens_the_default() {
        let mut prefs = Prefs::default();
        prefs.set_view("all");
        prefs.update_filters(|f| f.authors = vec!["sam".into()]);
        let id = prefs.save_view("  langfuse reviews ").unwrap();
        assert_eq!(prefs.view, format!("saved:{id}"));
        assert_eq!(prefs.base(), View::All);
        assert_eq!(prefs.saved[0].name, "langfuse reviews");
        prefs.update_filters(|f| f.hosts = vec!["github".into()]);
        assert_eq!(prefs.saved[0].filters.hosts, vec!["github".to_string()]);
        assert!(
            prefs.filters.get("all").unwrap().hosts.is_empty(),
            "changing a saved view leaves All open alone"
        );
        assert_eq!(prefs.save_view("  "), None);
        let second = prefs.save_view("other").unwrap();
        assert_ne!(id, second);
        prefs.delete_view(&second);
        assert_eq!(prefs.base(), View::NeedsMe);
        assert_eq!(prefs.saved.len(), 1);
    }

    #[test]
    fn pages_hold_25_rows_in_group_order_with_each_groups_full_count() {
        let mut items = Vec::new();
        for n in 0..30 {
            let mut item = pr(n, "devon");
            item.requested_reviewers = vec!["vasanth".into()];
            items.push(item);
        }
        for n in 30..33 {
            let mut item = pr(n, "vasanth");
            item.requested_reviewers = vec!["mira".into()];
            items.push(item);
        }
        let rows: Vec<&Item> = items.iter().collect();
        let first = page(&rows, 0);
        assert_eq!(
            (first.start, first.end, first.total, first.pages),
            (0, 25, 33, 2)
        );
        assert_eq!(first.groups.len(), 1);
        assert_eq!(
            first.group_totals,
            vec![(Group::You, 30), (Group::Others, 3)]
        );
        let second = page(&rows, 1);
        assert_eq!((second.start, second.end), (25, 33));
        assert_eq!(
            second
                .groups
                .iter()
                .map(|(g, r)| (*g, r.len()))
                .collect::<Vec<_>>(),
            vec![(Group::You, 5), (Group::Others, 3)]
        );
        assert_eq!(page(&rows, 9).index, 1, "past the end is the last page");
        let empty = page(&[], 0);
        assert_eq!((empty.pages, empty.total, empty.groups.len()), (1, 0, 0));
    }

    #[test]
    fn arrivals_wait_behind_the_banner_until_shown_and_a_new_scope_shows_all() {
        let items: Vec<Item> = (1..=3).map(|n| pr(n, "devon")).collect();
        let all: Vec<&Item> = items.iter().collect();
        let mut arrivals = Arrivals::default();
        assert_eq!(
            arrivals.split("needs_me", all.clone()).1,
            0,
            "unseeded shows all"
        );
        assert!(arrivals.seed("needs_me", &all[..2], 100));
        assert!(
            !arrivals.seed("needs_me", &all, 150),
            "a seed sticks to its scope"
        );
        let (shown, new) = arrivals.split("needs_me", all.clone());
        assert_eq!((numbers(&shown), new), (vec![1, 2], 1));
        assert_eq!(arrivals.since_ms(), 100);
        arrivals.show(&all, 300);
        assert_eq!(arrivals.split("needs_me", all.clone()), (all.clone(), 0));
        assert_eq!(arrivals.since_ms(), 300);
        // A removed one just goes; another scope starts over.
        let (shown, new) = arrivals.split("needs_me", all[2..].to_vec());
        assert_eq!((shown.len(), new), (1, 0));
        assert!(arrivals.seed("all", &all[2..], 500));
        assert_eq!(arrivals.split("all", all.clone()).1, 2);
        arrivals.reset();
        assert!(arrivals.seed("all", &all, 600));
    }

    #[test]
    fn the_first_launch_after_the_rename_keeps_how_the_inbox_opened() {
        let mut old = ViewState::default();
        assert_eq!(Prefs::migrated(&old).opens_on, View::All);
        old.filters.review_requested = true;
        assert_eq!(Prefs::migrated(&old).opens_on, View::NeedsMe);
        let prefs = Prefs::migrated(&old);
        assert!(prefs.fold_bots && prefs.other_repos && prefs.keep_filters && prefs.notify);
    }

    #[test]
    fn a_new_user_opens_on_needs_me_and_an_inbox_user_keeps_all() {
        let fresh = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(State::load(&fresh).prefs.opens_on, View::NeedsMe);
        let used = Store::open(std::path::Path::new(":memory:")).unwrap();
        ViewState::default().save(&used).unwrap();
        assert_eq!(State::load(&used).prefs.opens_on, View::All);
    }

    #[test]
    fn state_round_trips_and_drops_filters_when_they_are_not_kept() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut old = ViewState::default();
        old.filters.assigned_to_me = true;
        old.seen.seeded = true;
        old.seen.items.insert("github:x".into(), 5);
        old.list_width = 333.;
        old.save(&store).unwrap();
        let mut state = State::load(&store);
        assert_eq!(state.prefs.opens_on, View::NeedsMe);
        assert!(
            !state.feed.filters.assigned_to_me,
            "the feed fetches everything"
        );
        assert_eq!(state.feed.seen.items.get("github:x"), Some(&5));
        assert_eq!(state.feed.list_width, 333.);
        state.prefs.set_view("all");
        state
            .prefs
            .update_filters(|f| f.hosts = vec!["gitlab".into()]);
        state.prefs.added_repos.push(AddedRepo {
            provider: "github".into(),
            repo: "acme/web".into(),
        });
        state.save(&store).unwrap();
        let again = State::load(&store);
        assert_eq!(again.prefs.filters["all"].hosts, vec!["gitlab".to_string()]);
        assert_eq!(again.prefs.added_repos.len(), 1);
        assert_eq!(again.prefs.view, "needs_me", "it opens on Opens on");
        let mut forget = again.clone();
        forget.prefs.keep_filters = false;
        forget.save(&store).unwrap();
        assert!(State::load(&store).prefs.filters.is_empty());
        let _ = Person::default();
    }
}
