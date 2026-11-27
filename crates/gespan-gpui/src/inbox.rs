//! Git: the review queue for pull and merge requests from every connected
//! host, with the selected one's detail beside it. The module keeps its old
//! name (`inbox`) because automations and Home read the same feed; everything
//! a user sees says Git.
mod ask_panel;
mod avatar;
mod checks;
mod diff;
mod document;
mod git_list;
mod git_settings;
#[cfg(test)]
mod git_tests;
mod menu;
mod review_bar;
mod ui;
pub(crate) use avatar::image as avatar_image;
pub(crate) use menu::{ItemAutomation, ItemRun, new_automation_event};
pub(crate) use ui::relative_time;

use crate::{
    diff_view::{DiffEvent, DiffView},
    input::{self, Input},
    popover::AnchorSlot,
    runtime::{Database, locked},
    theme::*,
};
use gespan_core::{
    inbox::{self as model, Checks, Details, Diff, Item, Service, Snapshot, Thread, queue},
    store::Session,
};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, ListAlignment,
    ListState, Pixels, Subscription, Window, px,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

impl Inbox {
    /// The GitHub and GitLab items in the current list, for automations that
    /// wait for something to happen on them.
    pub(crate) fn watched_items(&self) -> Vec<Item> {
        self.snapshot
            .items
            .iter()
            .filter(|item| matches!(item.provider.as_str(), "github" | "gitlab"))
            .cloned()
            .collect()
    }
}
pub(crate) enum Event {
    Close,
    OpenSession(i64),
    /// A new session with the item as a card above its composer, to send.
    ComposeSession {
        item: Box<Item>,
        prompt: String,
    },
    /// A check repair for the agent, reported back by its `repair` id.
    StartRepair {
        item: Box<Item>,
        prompt: String,
        repair: String,
    },
    /// The item's Ask conversation, for the panel beside it.
    Ask {
        item: Box<Item>,
        description: Option<String>,
    },
    /// An empty conversation in place of `session`, for the same panel.
    RestartAsk {
        key: String,
        session: i64,
    },
    AskClosed,
    ItemsChanged(Vec<Item>),
    /// The ⋯ menu opened on a pull request: the Shell answers with
    /// `set_item_automations`.
    AutomationsWanted(Box<Item>),
    /// Run an automation now with this pull request as its event item.
    RunAutomation {
        item: Box<Item>,
        automation: String,
    },
    /// Run the project's review automation on this pull request, or open its
    /// setup when there is none.
    ReviewWithAgent(Box<Item>),
    /// The automation editor, with a pull request trigger.
    NewPullRequestAutomation(Box<Item>),
    /// Runs, filtered to this automation.
    OpenRuns {
        automation: String,
        name: String,
    },
    /// A tracker was connected or disconnected.
    ConnectionsChanged,
    /// Add connection: Settings › Connections holds the catalogue.
    OpenConnections,
    /// A pull request started waiting on you: tell the user, per Settings ›
    /// Git › Notify me.
    ReviewNotice {
        item: Box<Item>,
        body: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Summary,
    Code,
    Checks,
}

#[cfg(test)]
impl Inbox {
    pub(crate) fn connections_menu_open(&self) -> bool {
        self.connections_open
    }
}

pub(crate) struct Inbox {
    /// The detail pane's selectable text.
    pub(crate) text: crate::selectable::SelectionScope,
    store: Database,
    service: Service,
    projects: Vec<PathBuf>,
    appearances:
        std::collections::HashMap<String, gespan_core::project_appearance::ProjectAppearance>,
    state: queue::State,
    snapshot: Snapshot,
    /// Pull requests that arrived while the list was open, held behind the
    /// "N new since" banner.
    arrivals: queue::Arrivals,
    /// The list's page, from 0.
    page: usize,
    /// Review requests from other repos are unfolded.
    others_open: bool,
    /// The "Add repo" menu on the other-repos row.
    add_repo_open: bool,
    add_repo_anchor: AnchorSlot,
    /// The Filters dropdown row whose choices are open.
    filter_row: Option<git_list::FilterRow>,
    /// "+ Add filter" lists the filters left to add.
    adding_filter: bool,
    /// "Save as view…" is asking for a name.
    naming_view: bool,
    view_name: Entity<Input>,
    /// The review bar: its comment, tone, Polish and submission.
    review: review_bar::ReviewBar,
    review_body: Entity<Input>,
    /// Settings › Pull requests's "Add a repo" field and what it found.
    repo_search_input: Entity<Input>,
    repo_search: git_settings::RepoSearch,
    /// What a test's repository search finds.
    #[cfg(test)]
    pub(crate) canned_repo_hits: Option<Vec<gespan_core::inbox::RepoHit>>,
    /// When the last refresh landed.
    synced_ms: Option<i64>,
    /// What waited on you after the last refresh, so only newcomers notify.
    /// `None` until the first refresh, which never notifies.
    waiting_on_you: Option<std::collections::HashSet<String>>,
    /// Reviews a test submitted, instead of reaching a host.
    #[cfg(test)]
    pub(crate) submitted_reviews: std::sync::Arc<std::sync::Mutex<Vec<SubmittedReview>>>,
    sessions: Vec<Session>,
    focus: FocusHandle,
    search: Entity<Input>,
    comment: Entity<Input>,
    summary_scroll: ListState,
    summary_width: std::rc::Rc<std::cell::Cell<Pixels>>,
    activity: Vec<ui::Activity>,
    activity_revision: u64,
    summary_rows: std::cell::RefCell<ui::SummaryRows>,
    focus_comment: bool,
    focus_review: bool,
    selected: Option<String>,
    /// The address last copied and the press that copied it, until the tick fades.
    copied_link: Option<(String, u64)>,
    details: Option<Details>,
    thread: Option<Thread>,
    document: Option<Entity<document::InboxDocument>>,
    comments: Vec<(model::Comment, Entity<document::InboxDocument>)>,
    diff: Option<Diff>,
    /// Comments and actions made here, whose update must not read as unread.
    self_activity: gespan_core::inbox::SelfActivity,
    diff_view: Entity<DiffView>,
    checks: Option<Checks>,
    ci: checks::CiState,
    tab: Tab,
    loading: bool,
    detail_loading: bool,
    extra_loading: bool,
    posting: bool,
    acting: bool,
    error: Option<String>,
    detail_error: Option<String>,
    extra_error: Option<String>,
    post_error: Option<String>,
    thread_error: Option<String>,
    expanded_comments: std::collections::HashSet<String>,
    reply: Option<(String, String)>,
    generation: u64,
    detail_generation: u64,
    extra_request: u64,
    filters_open: bool,
    connections_open: bool,
    filter_anchor: AnchorSlot,
    connection_anchor: AnchorSlot,
    connect_provider: Option<String>,
    /// The connect dialog is over Settings, not the Inbox: it stays there on
    /// success instead of switching the Inbox to the new source.
    connect_in_settings: bool,
    connect_url: Entity<Input>,
    connect_email: Entity<Input>,
    connect_workspace: Entity<Input>,
    connect_token: Entity<Input>,
    connecting: bool,
    connect_error: Option<String>,
    start_project: PathBuf,
    project_picker: bool,
    project_anchor: AnchorSlot,
    /// The document's ⋯ menu.
    action_menu: bool,
    action_anchor: AnchorSlot,
    /// The pull request automations the ⋯ menu offers, for `automations_for`.
    item_automations: Vec<ItemAutomation>,
    automations_for: Option<String>,
    /// Runs started from an item's ⋯ menu, by item key.
    item_runs: std::collections::HashMap<String, ItemRun>,
    action_error: Option<String>,
    action_notice: Option<String>,
    confirm_action: Option<String>,
    resize_start: Option<(f32, f32)>,
    ask: ask_panel::AskPanel,
    #[cfg(test)]
    offline: Offline,
    /// Refreshes stay away from the real gh and trackers unless a test opts in.
    #[cfg(test)]
    live_refresh: bool,
    /// What a test's providers return to an offline refresh.
    #[cfg(test)]
    pub(crate) fetched: Snapshot,
    active: bool,
    refresh_queued: bool,
    notifications: model::NotificationTracker,
    last_refresh: Instant,
    _subscriptions: Vec<Subscription>,
}

/// Tests keep the inbox away from every provider.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Offline {
    Live,
    /// Details arrive at once, written from the item.
    Instant,
    /// Details stay loading until the test finishes them.
    Held,
}

/// The inbox opens scoped to the project it was opened from.
/// The pull request's review threads that sit on the current diff, for the
/// Files tab: a thread whose line is outdated stays in the discussion only.
fn remote_threads(thread: &Thread) -> Vec<crate::diff_view::RemoteThread> {
    use gespan_core::inbox::Side;
    thread
        .comments
        .iter()
        .filter(|c| c.kind == "review_comment" && !c.path.is_empty())
        .filter_map(|first| {
            let side = match first.side.as_str() {
                "RIGHT" => Side::New,
                "LEFT" => Side::Old,
                _ => return None,
            };
            let line = u32::try_from(first.line?).ok().filter(|l| *l > 0)?;
            Some(crate::diff_view::RemoteThread {
                path: first.path.clone(),
                line,
                side,
                resolved: first.resolved,
                comments: std::iter::once(first)
                    .chain(first.replies.iter())
                    .map(|c| crate::diff_view::RemoteComment {
                        author: c.author.clone(),
                        when: ui::relative_time(&c.created_at),
                        body: c.body.trim().to_string(),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// A review a test sent: item key, verdict, summary and line comments.
#[cfg(test)]
pub(crate) type SubmittedReview = (
    String,
    gespan_core::inbox::Verdict,
    String,
    Vec<gespan_core::inbox::LineComment>,
);

impl EventEmitter<Event> for Inbox {}
impl Focusable for Inbox {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Inbox {
    fn copy_link(&mut self, url: String, cx: &mut Context<Self>) {
        if url.is_empty() {
            return;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
        let press = self.copied_link.as_ref().map_or(0, |(_, press)| press + 1);
        self.copied_link = Some((url, press));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this.copied_link.as_ref().is_some_and(|(_, p)| *p == press) {
                    this.copied_link = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(crate) fn new(
        store: Database,
        projects: Vec<PathBuf>,
        cwd: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_inner(store, projects, cwd, true, cx)
    }

    #[cfg(test)]
    pub(crate) fn offline(store: Database, cx: &mut Context<Self>) -> Self {
        Self::new_inner(store, vec![], "/missing/inbox".into(), false, cx)
    }

    fn new_inner(
        store: Database,
        mut projects: Vec<PathBuf>,
        cwd: PathBuf,
        poll: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        if !projects.contains(&cwd) {
            projects.push(cwd.clone());
        }
        let (state, data_dir) = locked(&store)
            .map(|db| {
                (
                    queue::State::load(&db),
                    db.data_dir()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| cwd.clone()),
                )
            })
            .unwrap_or_else(|_| {
                (
                    queue::State {
                        feed: Default::default(),
                        prefs: Default::default(),
                    },
                    cwd.clone(),
                )
            });
        let search = cx.new(|cx| {
            Input::field(
                "Search pull requests",
                "Search pull requests",
                content(0.4),
                cx,
            )
        });
        let comment = cx.new(Input::inbox_comment);
        let view_name = cx.new(|cx| Input::field("View name", "Name this view", content(0.4), cx));
        let repo_search_input = cx.new(|cx| {
            Input::field(
                "Add a repo",
                "Add a repo: search the repos you can reach",
                content(0.4),
                cx,
            )
        });
        let review_body =
            cx.new(|cx| Input::multiline_field("Review comment", "Leave a review comment…", cx));
        let connect_url = cx.new(|cx| Input::field("Instance URL", "https://", content(0.4), cx));
        let connect_email = cx.new(|cx| Input::field("Email", "you@example.com", content(0.4), cx));
        let connect_workspace =
            cx.new(|cx| Input::field("Workspace", "Workspace slug", content(0.4), cx));
        let connect_token =
            cx.new(|cx| Input::field("API token", "Personal access token", content(0.4), cx));
        // A line comment in Files joins the pull request's review.
        let diff_view = cx.new(|cx| {
            let mut view = DiffView::new(cx);
            view.review_comments = true;
            view
        });
        let subscriptions = vec![
            cx.subscribe(&search, |this, _, _: &input::Changed, cx| {
                this.page = 0;
                cx.notify()
            }),
            cx.subscribe(&repo_search_input, |this, _, _: &input::Changed, cx| {
                this.search_repos(cx)
            }),
            cx.subscribe(&view_name, |this, _, _: &input::Submitted, cx| {
                this.save_view(cx)
            }),
            cx.subscribe(&review_body, |this, body, _: &input::Changed, cx| {
                // Typing clears the last notice; the draft clearing itself
                // after a sent review keeps it.
                if !body.read(cx).content.is_empty() {
                    this.review.draft_changed();
                }
                cx.notify()
            }),
            cx.subscribe(&comment, |this, _, _: &input::Changed, cx| {
                let count = this.summary_scroll.item_count();
                this.summary_scroll
                    .remeasure_items(count.saturating_sub(1)..count);
                cx.notify();
            }),
            cx.subscribe(&comment, |this, _, _: &input::Submitted, cx| {
                this.post_comment(cx)
            }),
            cx.subscribe(&diff_view, |this, _, event: &DiffEvent, cx| {
                if let DiffEvent::Refresh = event {
                    this.refresh_files(cx);
                    return;
                }
                if let DiffEvent::Polish { text, tone } = event {
                    this.polish_line(text.clone(), *tone, cx);
                    return;
                }
                if let DiffEvent::DraftsChanged = event {
                    this.review.draft_changed();
                    cx.notify();
                    return;
                }
                if let DiffEvent::Comment(text) = event {
                    // A line picked in Files starts a review comment there.
                    if this.tab == Tab::Code && this.review_open() {
                        this.review_body
                            .update(cx, |input, cx| input.set_text(text.clone(), cx));
                        this.review.draft_changed();
                        this.focus_review = true;
                        cx.notify();
                        return;
                    }
                    this.comment
                        .update(cx, |input, cx| input.set_text(text.clone(), cx));
                    this.tab = Tab::Summary;
                    this.summary_scroll.scroll_to_end();
                    this.focus_comment = true;
                    cx.notify();
                }
            }),
        ];
        if poll {
            cx.spawn(async move |this, cx| {
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    return;
                }
                loop {
                    cx.background_executor()
                        .timer(Duration::from_secs(model::ci_repair::CHECKS_POLL_SECS))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            let interval = Duration::from_secs(if this.active { 120 } else { 300 });
                            if this.last_refresh.elapsed() >= interval {
                                this.refresh(cx);
                            }
                            if this.active
                                && this.selected.is_some()
                                && !this.detail_loading
                                && !this.posting
                                && !this.acting
                            {
                                this.load_detail(false, cx);
                                if this.tab == Tab::Checks
                                    && !this.extra_loading
                                    && this
                                        .item()
                                        .is_some_and(|item| model::ci_repair::polls_checks(&item))
                                {
                                    this.refresh_checks(cx);
                                }
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        let ci = locked(&store)
            .map(|db| checks::CiState::load(&db))
            .unwrap_or_default();
        Self {
            text: crate::selectable::SelectionScope::new(cx),
            service: Service {
                data_dir,
                cwd: cwd.clone(),
            },
            store,
            projects,
            appearances: Default::default(),
            state,
            snapshot: Snapshot::default(),
            arrivals: Default::default(),
            page: 0,
            others_open: false,
            add_repo_open: false,
            add_repo_anchor: AnchorSlot::default(),
            filter_row: None,
            adding_filter: false,
            naming_view: false,
            view_name,
            review: Default::default(),
            review_body,
            focus_review: false,
            repo_search_input,
            repo_search: Default::default(),
            #[cfg(test)]
            canned_repo_hits: None,
            synced_ms: None,
            waiting_on_you: None,
            #[cfg(test)]
            submitted_reviews: Default::default(),
            sessions: vec![],
            focus: cx.focus_handle(),
            search,
            comment,
            summary_scroll: ListState::new(0, ListAlignment::Top, px(200.)),
            summary_width: Default::default(),
            activity: vec![],
            activity_revision: 0,
            summary_rows: Default::default(),
            focus_comment: false,
            selected: None,
            copied_link: None,
            details: None,
            thread: None,
            document: None,
            comments: vec![],
            diff: None,
            self_activity: Default::default(),
            diff_view,
            checks: None,
            ci,
            tab: Tab::Summary,
            loading: false,
            detail_loading: false,
            extra_loading: false,
            posting: false,
            acting: false,
            error: None,
            detail_error: None,
            extra_error: None,
            post_error: None,
            thread_error: None,
            expanded_comments: Default::default(),
            reply: None,
            generation: 0,
            detail_generation: 0,
            extra_request: 0,
            filters_open: false,
            connections_open: false,
            filter_anchor: AnchorSlot::default(),
            connection_anchor: AnchorSlot::default(),
            connect_provider: None,
            connect_in_settings: false,
            connect_url,
            connect_email,
            connect_workspace,
            connect_token,
            connecting: false,
            connect_error: None,
            start_project: cwd,
            project_picker: false,
            project_anchor: AnchorSlot::default(),
            action_menu: false,
            action_anchor: AnchorSlot::default(),
            item_automations: vec![],
            automations_for: None,
            item_runs: Default::default(),
            action_error: None,
            action_notice: None,
            confirm_action: None,
            resize_start: None,
            ask: Default::default(),
            #[cfg(test)]
            offline: Offline::Live,
            #[cfg(test)]
            live_refresh: false,
            #[cfg(test)]
            fetched: Snapshot::default(),
            active: false,
            refresh_queued: false,
            notifications: model::NotificationTracker::default(),
            last_refresh: Instant::now(),
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn set_context(
        &mut self,
        mut projects: Vec<PathBuf>,
        cwd: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !projects.contains(&cwd) {
            projects.push(cwd.clone());
        }
        if self.projects != projects || self.service.cwd != cwd {
            let switched = self.service.cwd != cwd;
            self.projects = projects;
            self.service.cwd = cwd.clone();
            self.start_project = cwd;
            if switched {
                let visible = self.visible_items(cx);
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|key| !visible.iter().any(|item| &item.key() == key))
                {
                    self.deselect(cx);
                    if self.active
                        && let Some(item) = visible.first()
                    {
                        self.select(item.key(), cx);
                    }
                }
            }
            self.refresh(cx);
        }
    }

    pub(crate) fn set_appearances(
        &mut self,
        appearances: std::collections::HashMap<
            String,
            gespan_core::project_appearance::ProjectAppearance,
        >,
        cx: &mut Context<Self>,
    ) {
        if self.appearances != appearances {
            self.appearances = appearances;
            cx.notify();
        }
    }

    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            if let Some(item) = self.item() {
                self.state.feed.seen.mark(&item);
                self.save();
            } else if let Some(item) = self.visible_items(cx).first() {
                self.select(item.key(), cx);
            }
        }
        cx.notify();
    }

    /// Never reaches a provider; `items` are what it lists.
    #[cfg(test)]
    pub(crate) fn offline_with(
        store: Database,
        cwd: PathBuf,
        items: Vec<Item>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut inbox = Self::new_inner(store, vec![], cwd, false, cx);
        inbox.offline = Offline::Instant;
        inbox.selected = items.first().map(Item::key);
        inbox.snapshot.items = items;
        inbox.load_detail(true, cx);
        inbox
    }
    #[cfg(test)]
    pub(crate) fn select_for_test(&mut self, key: String, cx: &mut Context<Self>) {
        self.select(key, cx);
    }
    #[cfg(test)]
    pub(crate) fn hold_details_for_test(&mut self, cx: &mut Context<Self>) {
        self.offline = Offline::Held;
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn finish_details_for_test(&mut self, cx: &mut Context<Self>) {
        self.offline = Offline::Instant;
        self.load_detail(false, cx);
    }
    /// Items that arrived since the inbox first loaded, so they are unread.
    #[cfg(test)]
    pub(crate) fn receive_unread(&mut self, items: Vec<Item>) {
        self.state.feed.seen.seeded = true;
        self.snapshot.items = items;
    }
    /// An update our own comment or action caused is not unread.
    fn skip_own_updates(&mut self, items: &[Item], now: u64) {
        for item in items {
            if self.state.feed.seen.unseen(item) && self.self_activity.consume(item, now) {
                self.state.feed.seen.mark(item);
            }
        }
    }
    /// Mark all as read: every item the inbox has loaded.
    pub(crate) fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let items = std::mem::take(&mut self.snapshot.items);
        for item in &items {
            self.state.feed.seen.mark(item);
        }
        self.snapshot.items = items;
        self.save();
        cx.notify();
    }
    /// Home's Inbox tile and timeline: what the last load holds. `None`
    /// while the first load is still running.
    pub(crate) fn home_counts(&self) -> Option<crate::home::InboxCounts> {
        if self.loading && self.snapshot.items.is_empty() {
            return None;
        }
        let items: Vec<&Item> = self
            .snapshot
            .items
            .iter()
            .filter(|item| queue::listed(item))
            .collect();
        let asked = |item: &&&Item| queue::review_requested_from_me(item) && !queue::is_mine(item);
        Some(crate::home::InboxCounts {
            total: items.len(),
            unread: self.unread_count(),
            review: items.iter().filter(asked).count(),
            blocked: items
                .iter()
                .filter(|item| {
                    queue::is_mine(item) && queue::waiting(item).group == queue::Group::You
                })
                .count(),
            reviews: items
                .iter()
                .filter(asked)
                .filter_map(|item| {
                    let at = chrono::DateTime::parse_from_rfc3339(&item.updated_at).ok()?;
                    Some((crate::home::item_reference(item), at.timestamp_millis()))
                })
                .collect(),
        })
    }
    /// Unread pull requests in the lists the user sees, for the rail badge.
    pub(crate) fn unread_count(&self) -> usize {
        self.snapshot
            .items
            .iter()
            .filter(|item| queue::listed(item))
            .filter(|item| !queue::is_bot(&item.author) || !self.state.prefs.fold_bots)
            .filter(|item| self.state.feed.seen.unseen(item))
            .count()
    }

    pub(crate) fn connection_statuses(&self) -> std::collections::HashMap<String, (bool, String)> {
        let mut statuses = self
            .snapshot
            .connections
            .iter()
            .map(|(provider, status)| {
                let description = if !status.connected {
                    "Not connected".into()
                } else if !status.site.is_empty() {
                    status.site.clone()
                } else if !status.url.is_empty() {
                    status.url.clone()
                } else if !status.organization.is_empty() {
                    status.organization.clone()
                } else {
                    "Connected".into()
                };
                (provider.clone(), (status.connected, description))
            })
            .collect::<std::collections::HashMap<_, _>>();
        for (provider, error) in &self.snapshot.errors {
            if !statuses.contains_key(provider) {
                statuses.insert(provider.clone(), (false, error.clone()));
            }
        }
        statuses
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            self.refresh_queued = true;
            return;
        }
        self.loading = true;
        self.last_refresh = Instant::now();
        self.generation += 1;
        let generation = self.generation;
        let service = self.service.clone();
        let state = self.state.feed.clone();
        let added: Vec<(String, String)> = self
            .state
            .prefs
            .added_repos
            .iter()
            .map(|repo| (repo.provider.clone(), repo.repo.clone()))
            .collect();
        let other_repos = self.state.prefs.other_repos;
        let projects = self.projects.clone();
        let scope = self.fetch_scope();
        let store = self.store.clone();
        #[cfg(not(test))]
        let live = true;
        #[cfg(test)]
        let live = self.live_refresh;
        #[cfg(not(test))]
        let fetched = Snapshot::default();
        #[cfg(test)]
        let fetched = self.fetched.clone();
        let task = cx.background_executor().spawn(async move {
            let snapshot = if live {
                service.list_git(&projects, &state, &added, other_repos)
            } else {
                fetched
            };
            let sessions = locked(&store)
                .and_then(|db| db.sessions())
                .unwrap_or_default();
            (snapshot, sessions)
        });
        cx.spawn(async move |this, cx| {
            let (mut snapshot, sessions) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                if scope != this.fetch_scope() {
                    this.refresh_queued = false;
                    this.refresh(cx);
                    return;
                }
                // Log a failure when it starts or changes, not on every refresh.
                for (provider, error) in &snapshot.errors {
                    if this.snapshot.errors.get(provider) != Some(error) {
                        tracing::warn!(%provider, %error, "inbox provider fetch failed");
                    }
                }
                for (repo, error) in &snapshot.repo_errors {
                    if this.snapshot.repo_errors.get(repo) != Some(error) {
                        tracing::warn!(%repo, %error, "inbox repository fetch failed");
                    }
                }
                let appeared = this.notifications.observe(&snapshot, scope);
                let candidates = model::automation_candidates(&snapshot.items, appeared);
                // A temporary outage must not erase the last usable provider list.
                for provider in snapshot.errors.keys() {
                    snapshot.items.extend(
                        this.snapshot
                            .items
                            .iter()
                            .filter(|i| &i.provider == provider)
                            .cloned(),
                    );
                }
                snapshot
                    .items
                    .sort_by_key(|item| std::cmp::Reverse(item.updated_ms()));
                this.skip_own_updates(&snapshot.items, gespan_core::time::now_ms());
                this.state.feed.seen.seed(&snapshot.items);
                this.snapshot = snapshot;
                this.sessions = sessions;
                this.synced_ms = Some(gespan_core::time::now_ms() as i64);
                if this.active
                    && let Some(item) = this.item()
                {
                    this.state.feed.seen.mark(&item);
                }
                this.save();
                this.notify_review_queue(cx);
                cx.emit(Event::ItemsChanged(candidates));
                if this.selected.is_some() && this.item().is_none() {
                    this.deselect(cx);
                }
                if this.active
                    && this.selected.is_none()
                    && let Some(item) = this.visible_items(cx).first()
                {
                    this.select(item.key(), cx);
                }
                if std::mem::take(&mut this.refresh_queued) {
                    this.refresh(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Rows are assigned to the active checkout, so a fetch made for another
    /// one is stale even when the project list is the same.
    fn fetch_scope(&self) -> String {
        format!(
            "{}:{}:{}",
            self.state.feed.fetch_scope(&self.projects),
            gespan_core::files::path_key(&self.service.cwd.to_string_lossy()),
            serde_json::to_string(&(&self.state.prefs.added_repos, self.state.prefs.other_repos))
                .unwrap_or_default()
        )
    }
    /// Tells the user about pull requests that started waiting on them since
    /// the last refresh: a review asked of them, or their own one blocked.
    fn notify_review_queue(&mut self, cx: &mut Context<Self>) {
        let now: std::collections::HashSet<String> = self
            .snapshot
            .items
            .iter()
            .filter(|item| queue::open_pr(item))
            .filter(|item| !queue::is_bot(&item.author) || !self.state.prefs.fold_bots)
            .filter(|item| queue::waiting(item).group == queue::Group::You)
            .map(Item::key)
            .collect();
        let previous = self.waiting_on_you.replace(now.clone());
        let Some(previous) = previous else {
            return;
        };
        if !self.state.prefs.notify {
            return;
        }
        for item in self.snapshot.items.iter().filter(|item| {
            now.contains(&item.key())
                && !previous.contains(&item.key())
                && self.state.feed.seen.unseen(item)
        }) {
            cx.emit(Event::ReviewNotice {
                item: Box::new(item.clone()),
                body: queue::waiting(item).why,
            });
        }
    }
    fn save(&mut self) {
        if let Err(error) = locked(&self.store).and_then(|db| self.state.save(&db)) {
            self.error = Some(error);
        }
    }
    /// What the list holds back behind "N new since" is tied to this: the
    /// view, its filters and the search.
    fn view_scope(&self, cx: &App) -> String {
        format!(
            "{}|{}|{}",
            self.state.prefs.view,
            serde_json::to_string(&self.state.prefs.filters()).unwrap_or_default(),
            self.search.read(cx).content.as_ref()
        )
    }
    /// The pull requests the list shows, in list order across every page,
    /// without those held behind the new-arrivals banner.
    fn visible_items(&self, cx: &App) -> Vec<Item> {
        let rows = self
            .state
            .prefs
            .visible(&self.snapshot.items, self.search.read(cx).content.as_ref());
        let (shown, _) = self.arrivals.split(&self.view_scope(cx), rows);
        queue::grouped(&shown)
            .into_iter()
            .flat_map(|(_, rows)| rows.into_iter().cloned())
            .collect()
    }
    /// Seeds the held-back set when the view changes, before a render reads it.
    fn sync_arrivals(&mut self, cx: &App) {
        let scope = self.view_scope(cx);
        let rows = self
            .state
            .prefs
            .visible(&self.snapshot.items, self.search.read(cx).content.as_ref());
        if self
            .arrivals
            .seed(&scope, &rows, gespan_core::time::now_ms() as i64)
        {
            self.page = 0;
        }
    }
    fn item(&self) -> Option<Item> {
        self.snapshot
            .items
            .iter()
            .find(|item| Some(item.key()) == self.selected)
            .cloned()
    }
    fn select(&mut self, key: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&key) {
            return;
        }
        self.selected = Some(key);
        self.reset_detail(cx);
        // Your pull request with failing checks opens on them.
        if let Some(item) = self.item()
            && item.provider == "github"
            && item.kind == "pr"
            && item.check_summary.failed > 0
            && queue::is_mine(&item)
        {
            self.set_tab(Tab::Checks, cx);
        }
        if let Some(item) = self.item() {
            if self.active {
                self.state.feed.seen.mark(&item);
                self.save();
            }
            if !item.project_path.is_empty() {
                self.start_project = PathBuf::from(&item.project_path);
            }
        }
        self.load_detail(true, cx);
        self.sync_ask(cx);
        cx.notify();
    }
    fn deselect(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.reset_detail(cx);
        cx.notify();
    }
    /// Drops the detail pane's state and invalidates its in-flight requests.
    fn reset_detail(&mut self, cx: &mut Context<Self>) {
        self.detail_generation += 1;
        self.detail_loading = false;
        self.detail_error = None;
        self.summary_scroll = ListState::new(0, ListAlignment::Top, px(200.));
        self.activity.clear();
        self.activity_revision += 1;
        self.summary_rows = Default::default();
        self.focus_comment = false;
        self.tab = Tab::Summary;
        self.diff = None;
        self.review.reset();
        self.diff_view.update(cx, |view, cx| {
            view.set_drafts(vec![], cx);
            view.set_threads(vec![], cx);
        });
        self.review_body
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.extra_request += 1;
        self.diff_view
            .update(cx, |view, cx| view.set_files(vec![], cx));
        self.checks = None;
        self.ci.reset();
        self.extra_loading = false;
        self.extra_error = None;
        self.details = None;
        self.thread = None;
        self.document = None;
        self.comments.clear();
        self.reply = None;
        self.post_error = None;
        self.thread_error = None;
        self.expanded_comments.clear();
        self.confirm_action = None;
        self.action_error = None;
        self.action_notice = None;
        self.action_menu = false;
        self.comment
            .update(cx, |input, cx| input.set_text(String::new(), cx));
    }
    fn load_detail(&mut self, clear: bool, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        #[cfg(test)]
        if self.offline != Offline::Live {
            self.detail_loading = self.offline == Offline::Held;
            if self.offline == Offline::Instant {
                self.details = Some(Details {
                    body: format!("Body of {}", item.title),
                    ..Default::default()
                });
                self.sync_ask(cx);
            }
            return;
        }
        self.detail_generation += 1;
        let generation = self.detail_generation;
        self.detail_loading = true;
        self.detail_error = None;
        self.thread_error = None;
        if clear {
            self.document = None;
            self.comments.clear();
        }
        let service = self.service.clone();
        let task = cx
            .background_executor()
            .spawn(async move { (service.details(&item), service.thread(&item)) });
        cx.spawn(async move |this, cx| {
            let (details, thread) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.detail_generation != generation {
                    return;
                }
                this.detail_loading = false;
                match details {
                    Ok(details) => {
                        if this
                            .details
                            .as_ref()
                            .is_none_or(|old| old.body != details.body)
                        {
                            this.document = Some(cx.new(|cx| {
                                document::InboxDocument::description(
                                    details.body.clone(),
                                    this.document_cwd(),
                                    cx,
                                )
                            }));
                        }
                        this.details = Some(details);
                    }
                    Err(error) => this.detail_error = Some(error),
                }
                this.sync_ask(cx);
                match thread {
                    Ok(thread) => {
                        let mut rows = vec![];
                        for comment in &thread.comments {
                            let document = this
                                .comments
                                .iter()
                                .find(|(old, _)| old.id == comment.id && old.body == comment.body)
                                .map(|(_, document)| document.clone())
                                .unwrap_or_else(|| {
                                    cx.new(|cx| {
                                        document::InboxDocument::new(
                                            comment.body.clone(),
                                            this.document_cwd(),
                                            cx,
                                        )
                                    })
                                });
                            rows.push((comment.clone(), document));
                            for reply in &comment.replies {
                                let document = this
                                    .comments
                                    .iter()
                                    .find(|(old, _)| old.id == reply.id && old.body == reply.body)
                                    .map(|(_, document)| document.clone())
                                    .unwrap_or_else(|| {
                                        cx.new(|cx| {
                                            document::InboxDocument::new(
                                                reply.body.clone(),
                                                this.document_cwd(),
                                                cx,
                                            )
                                        })
                                    });
                                rows.push((reply.clone(), document));
                            }
                        }
                        this.comments = rows;
                        let threads = remote_threads(&thread);
                        this.diff_view
                            .update(cx, |view, cx| view.set_threads(threads, cx));
                        this.thread = Some(thread);
                        this.rebuild_activity();
                    }
                    Err(error) => this.thread_error = Some(error),
                }
                this.summary_scroll.remeasure();
                cx.notify();
            });
        })
        .detach();
    }
    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.extra_error = None;
        let Some(item) = self.item() else {
            return;
        };
        if tab == Tab::Summary
            || (tab == Tab::Code && self.diff.is_some())
            || (tab == Tab::Checks && self.checks.is_some())
        {
            self.extra_loading = false;
            cx.notify();
            return;
        }
        self.extra_loading = true;
        self.extra_request += 1;
        let request = self.extra_request;
        let key = item.key();
        let service = self.service.clone();
        let task = cx.background_executor().spawn(async move {
            if tab == Tab::Code {
                service.diff(&item).map(|diff| {
                    let files = diff::models(&diff);
                    (Some((diff, files)), None)
                })
            } else {
                service.checks(&item).map(|checks| (None, Some(checks)))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.selected.as_ref() != Some(&key)
                    || this.tab != tab
                    || request != this.extra_request
                {
                    return;
                }
                this.extra_loading = false;
                match result {
                    Ok((diff, checks)) => {
                        if let Some((diff, files)) = diff {
                            this.diff_view.update(cx, |view, cx| {
                                view.file_count = Some(diff.files.len());
                                view.totals = Some((
                                    diff.additions.max(0).min(u32::MAX as i64) as u32,
                                    diff.deletions.max(0).min(u32::MAX as i64) as u32,
                                ));
                                view.set_files(files, cx);
                            });
                            this.diff = Some(diff);
                        }
                        if let Some(checks) = checks {
                            this.checks = Some(checks);
                            // A failing pull request of yours opens on the
                            // failed job's log.
                            if this.item().is_some_and(|item| queue::is_mine(&item)) {
                                this.open_first_failure(cx);
                            }
                        }
                    }
                    Err(error) => this.extra_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn post_comment(&mut self, cx: &mut Context<Self>) {
        if self.posting {
            return;
        }
        let Some(item) = self.item() else {
            return;
        };
        let body = self.comment.read(cx).content.trim().to_string();
        if body.is_empty() {
            return;
        }
        self.posting = true;
        self.post_error = None;
        let key = item.key();
        let service = self.service.clone();
        let reply = self.reply.as_ref().map(|r| r.0.clone()).unwrap_or_default();
        let target = gespan_core::inbox::SelfTarget::of(&item);
        let task = cx
            .background_executor()
            .spawn(async move { service.comment(&item, body, reply) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.posting = false;
                if this.selected.as_ref() != Some(&key) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(_) => {
                        this.self_activity
                            .record(target, gespan_core::time::now_ms());
                        this.comment
                            .update(cx, |input, cx| input.set_text(String::new(), cx));
                        this.reply = None;
                        this.load_detail(false, cx);
                        this.refresh(cx);
                    }
                    Err(error) => this.post_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// The Files toolbar's refresh: the diff and its review threads again,
    /// from the host. Drafts stay.
    fn refresh_files(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        self.load_detail(false, cx);
        if self.tab == Tab::Code {
            self.set_tab(Tab::Code, cx);
        }
    }
    fn document_cwd(&self) -> PathBuf {
        self.item()
            .filter(|item| !item.project_path.is_empty())
            .map(|item| PathBuf::from(item.project_path))
            .unwrap_or_else(|| self.start_project.clone())
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        if let Some(mut item) = self.item() {
            if item.project_path.is_empty() {
                item.project_path = self.start_project.to_string_lossy().into_owned();
            }
            let body = self.details.as_ref().map(|d| d.body.as_str()).unwrap_or("");
            let discussion = self
                .comments
                .iter()
                .map(|(c, _)| format!("{}: {}", c.author, c.body))
                .collect::<Vec<_>>()
                .join("\n\n");
            let prompt = item.prompt(&format!("{body}\n\n{discussion}"), false);
            cx.emit(Event::ComposeSession {
                item: Box::new(item),
                prompt,
            });
        }
    }
    /// The Settings rows' Connect and Manage: the same dialog, over Settings.
    pub(crate) fn open_connection_in_settings(
        &mut self,
        provider: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = match provider.as_str() {
            "github" => None,
            "linear" => Some(self.connect_token.focus_handle(cx)),
            _ => Some(self.connect_url.focus_handle(cx)),
        };
        self.open_connection(provider, cx);
        self.connect_in_settings = true;
        if let Some(field) = field {
            window.focus(&field, cx);
        }
    }
    /// The dialog Settings shows, while one was opened from there.
    pub(crate) fn settings_dialog(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        (self.connect_in_settings && self.connect_provider.is_some())
            .then(|| crate::modal::window_overlay(window, self.connection_dialog(cx)))
    }
    #[cfg(test)]
    pub(crate) fn connecting_to(&self, provider: &str) -> bool {
        self.connect_provider.as_deref() == Some(provider)
    }
    #[cfg(test)]
    pub(crate) fn seed_linear_teams(
        &mut self,
        status: gespan_core::inbox::Status,
        teams: Vec<gespan_core::inbox::OptionRow>,
        cx: &mut Context<Self>,
    ) {
        self.snapshot.connections.insert("linear".into(), status);
        self.snapshot.linear_teams = teams;
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn filters_open(&self) -> bool {
        self.filters_open
    }
    #[cfg(test)]
    pub(crate) fn test_open_filters(&mut self, cx: &mut Context<Self>) {
        self.filters_open = true;
        cx.notify();
    }
    pub(crate) fn holds_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }
    /// Escape on the dialog Settings shows. True when it closed one.
    pub(crate) fn close_settings_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.connect_in_settings || self.connect_provider.is_none() || self.connecting {
            return false;
        }
        self.connect_provider = None;
        self.connect_token
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        cx.notify();
        true
    }
    #[cfg(test)]
    pub(crate) fn hidden_linear_teams(&self) -> &[String] {
        &self.state.feed.hidden_linear_teams
    }
    pub(crate) fn linear_teams(&self) -> Vec<(gespan_core::inbox::OptionRow, bool)> {
        self.snapshot
            .linear_teams
            .iter()
            .map(|team| {
                let shown = !self.state.feed.hidden_linear_teams.contains(&team.id);
                (team.clone(), shown)
            })
            .collect()
    }
    pub(crate) fn open_connection(&mut self, provider: String, cx: &mut Context<Self>) {
        self.connect_in_settings = false;
        self.connections_open = false;
        self.connect_error = None;
        let status = self
            .snapshot
            .connections
            .get(&provider)
            .cloned()
            .unwrap_or_default();
        let url = if !status.site.is_empty() {
            status.site
        } else if !status.url.is_empty() {
            status.url
        } else if !status.organization.is_empty() {
            status.organization
        } else if provider == "gitlab" {
            "https://gitlab.com".into()
        } else if provider == "plane" {
            "https://api.plane.so".into()
        } else {
            String::new()
        };
        self.connect_url
            .update(cx, |input, cx| input.set_text(url, cx));
        self.connect_email
            .update(cx, |input, cx| input.set_text(status.email, cx));
        self.connect_workspace
            .update(cx, |input, cx| input.set_text(status.workspace, cx));
        self.connect_token
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.connect_provider = Some(provider);
        cx.notify();
    }
    fn connect(&mut self, disconnect: bool, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let Some(provider) = self.connect_provider.clone() else {
            return;
        };
        let url = self.connect_url.read(cx).content.to_string();
        // Plane's workspace travels where Jira's email does.
        let email = if provider == "plane" {
            self.connect_workspace.read(cx).content.to_string()
        } else {
            self.connect_email.read(cx).content.to_string()
        };
        let token = if disconnect {
            String::new()
        } else {
            self.connect_token.read(cx).content.to_string()
        };
        if provider != "github" && !disconnect && token.trim().is_empty() {
            self.connect_error = Some(
                if provider == "plane" {
                    "Enter a Plane API key"
                } else {
                    "Enter an API token"
                }
                .into(),
            );
            cx.notify();
            return;
        }
        self.connecting = true;
        self.connect_error = None;
        let service = self.service.clone();
        let source = provider.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.connect(&source, url, email, token) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.connecting = false;
                match result {
                    Ok(status) => {
                        let connected = status.connected;
                        this.snapshot.connections.insert(provider.clone(), status);
                        cx.emit(Event::ConnectionsChanged);
                        this.connect_token.update(cx, |input, cx| input.set_text(String::new(), cx));
                        if connected || disconnect {
                            this.connect_provider = None;
                            this.refresh(cx);
                        } else { this.connect_error = Some("GitHub is not connected. Run gh auth login in a terminal, then check again.".into()); }
                    }
                    Err(error) => this.connect_error = Some(error),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn run_action(&mut self, cx: &mut Context<Self>) {
        if self.acting {
            return;
        }
        let (Some(item), Some(action)) = (self.item(), self.confirm_action.clone()) else {
            return;
        };
        self.acting = true;
        self.action_error = None;
        let service = self.service.clone();
        let key = item.key();
        let merging = matches!(action.as_str(), "merge" | "squash" | "rebase");
        let target = gespan_core::inbox::SelfTarget::of(&item);
        let task = cx
            .background_executor()
            .spawn(async move { service.action(&item, &action) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.acting = false;
                match result {
                    Ok(item) => {
                        this.self_activity
                            .record(target, gespan_core::time::now_ms());
                        this.action_notice = (merging
                            && !item.state.eq_ignore_ascii_case("merged"))
                        .then(|| "Merge queued or auto-merge enabled.".into());
                        if let Some(old) = this.snapshot.items.iter_mut().find(|i| i.key() == key) {
                            *old = item;
                        }
                        this.confirm_action = None;
                        this.load_detail(false, cx);
                        this.refresh(cx);
                    }
                    Err(error) => this.action_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only review comments on the current diff become threads in Files,
    /// each with its replies, on its side.
    #[test]
    fn the_pull_requests_review_threads_go_to_their_lines() {
        let comment =
            |id: &str, kind: &str, path: &str, line: Option<i64>, side: &str| model::Comment {
                id: id.into(),
                kind: kind.into(),
                author: "cubic".into(),
                body: format!(" {id} body "),
                created_at: "2026-11-27T01:00:00Z".into(),
                path: path.into(),
                line,
                side: side.into(),
                ..Default::default()
            };
        let mut current = comment("c1", "review_comment", "CHANGELOG.md", Some(12), "RIGHT");
        current.replies = vec![comment(
            "c2",
            "review_comment",
            "CHANGELOG.md",
            Some(12),
            "RIGHT",
        )];
        current.resolved = true;
        let thread = Thread {
            comments: vec![
                current,
                comment("c3", "review_comment", "src/old.rs", Some(7), "LEFT"),
                comment("c4", "review_comment", "src/old.rs", Some(9), ""),
                comment("c5", "comment", "", None, ""),
                comment("c6", "review", "", None, ""),
            ],
            ..Default::default()
        };
        let threads = remote_threads(&thread);
        assert_eq!(
            threads.len(),
            2,
            "outdated and general comments stay in the discussion"
        );
        assert_eq!(threads[0].path, "CHANGELOG.md");
        assert_eq!(
            (threads[0].line, threads[0].side),
            (12, gespan_core::inbox::Side::New)
        );
        assert!(threads[0].resolved);
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(threads[0].comments[0].body, "c1 body");
        assert_eq!(threads[1].side, gespan_core::inbox::Side::Old);
    }
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    fn item(provider: &str, number: i64) -> Item {
        Item {
            provider: provider.into(),
            kind: "pr".into(),
            repo: "acme/app".into(),
            number,
            title: format!("Change {number}"),
            ..Default::default()
        }
    }

    fn inbox(cx: &mut TestAppContext) -> (WindowHandle<Inbox>, [String; 2]) {
        cx.update(|cx| cx.set_global(crate::settings::Preferences::default()));
        let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let store: Database = std::sync::Arc::new(std::sync::Mutex::new(store));
        let (github, gitlab) = (item("github", 1), item("gitlab", 2));
        let keys = [github.key(), gitlab.key()];
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Inbox::new_inner(store, vec![], "/missing/inbox".into(), false, cx))
            })
            .unwrap()
        });
        // After the inbox's own first load, which would replace them.
        cx.run_until_parked();
        window
            .update(cx, |inbox, _, _| {
                inbox.snapshot.items = vec![github, gitlab]
            })
            .unwrap();
        (window, keys)
    }

    /// Files shows hunks only: the Hunks / Full file switch is gone, and the
    /// diff loads once per pull request, on every host.
    #[gpui::test]
    fn files_show_hunks_only_with_no_full_file_switch(cx: &mut TestAppContext) {
        let (window, [github, gitlab]) = inbox(cx);
        for key in [github, gitlab] {
            window
                .update(cx, |inbox, _, cx| {
                    inbox.select(key.clone(), cx);
                    inbox.set_tab(Tab::Code, cx);
                })
                .unwrap();
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            visual.refresh().unwrap();
            for gone in ["inbox-diff-context", "inbox-diff-hunks", "inbox-diff-full"] {
                assert!(visual.debug_bounds(gone).is_none(), "{gone} is still drawn");
            }
            // Someone else's pull request: the review bar takes the Ask bar's place.
            assert!(visual.debug_bounds("git-review-bar").is_some());
            assert!(visual.debug_bounds("inbox-ask-bar").is_none());
        }
    }

    #[gpui::test]
    fn an_update_from_our_own_comment_does_not_read_as_unread(cx: &mut TestAppContext) {
        let (window, _) = inbox(cx);
        window
            .update(cx, |inbox, _, _| {
                let mut ours = item("github", 1);
                let mut theirs = item("github", 2);
                inbox.state.feed.seen.seed(&[ours.clone(), theirs.clone()]);
                inbox.self_activity.record(
                    gespan_core::inbox::SelfTarget::of(&ours),
                    gespan_core::time::now_ms(),
                );
                // Both get a newer revision; only #1's came from us.
                ours.updated_at = "2030-01-01T00:00:00Z".into();
                theirs.updated_at = "2030-01-01T00:00:00Z".into();
                let now = gespan_core::time::now_ms();
                inbox.skip_own_updates(&[ours.clone(), theirs.clone()], now);
                inbox.snapshot.items = vec![ours.clone(), theirs];
                assert_eq!(inbox.unread_count(), 1, "only someone else's update");
                // Its next revision is announced as usual.
                ours.updated_at = "2031-01-01T00:00:00Z".into();
                inbox.skip_own_updates(std::slice::from_ref(&ours), now);
                assert!(inbox.state.feed.seen.unseen(&ours));
            })
            .unwrap();
    }
}
