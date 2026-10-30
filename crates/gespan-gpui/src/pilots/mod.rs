//! Pilot workspaces: one runtime per identity, shared by the rail, title tabs,
//! overview, settings and floating windows.
pub(crate) mod avatar;
pub(crate) mod cards;
mod changes;
mod color_picker;
#[cfg(test)]
mod dictation_tests;
mod files;
mod filters;
mod floating;
mod flow_reviews;
mod host;
mod html;
mod memory_ui;
mod notifications;
mod overview;
mod panel_stack;
mod panels;
mod roadmap;
mod scheduler;
mod search;
#[cfg(all(target_os = "macos", not(test)))]
mod status;
#[cfg(test)]
mod tests;
mod work;
use crate::{
    conversation::{Conversation, PilotSubmitted, SessionChanged},
    input::{self, Input},
    popover::{Anchor, AnchorSlot, Menu, MenuEvent, menu_popover, popover},
    runtime::{Database, locked},
    shell::Shell,
    theme::*,
};
use gespan_core::{
    agent::{AccessMode, Settings},
    pilot::{
        self, Message, Pilot, Roster,
        files::AgentFiles,
        habits::{self, Habit},
    },
    provider::Kind,
    store::Session,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Focusable, MouseButton, Role, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px, rgb,
};
use panels::{OpenPicker, OverviewTab, Route, SettingsSection};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};
pub(crate) struct Changed;
pub(crate) struct TabOpened(pub String);
pub(crate) struct ShowDetails(pub String);
pub(crate) use notifications::{NotificationChanged, NotificationFinished};
#[derive(Clone, PartialEq)]
pub(crate) struct OverviewChrome {
    active: Option<String>,
    tab: OverviewTab,
    routed: bool,
    counts: (usize, usize, usize, usize),
}
struct RunningHabit {
    pilot: String,
    habit: String,
    view: Entity<Conversation>,
    started: u64,
    launched: u64,
    failure: Option<String>,
    waiting_since: Option<u64>,
    cards: Vec<serde_json::Value>,
    /// Completion reports waiting for the run to finish its turn.
    mailbox: Vec<Message>,
}
pub(crate) struct Pilots {
    #[cfg(all(target_os = "macos", not(test)))]
    status: Option<status::Status>,
    parent_window: Option<gpui::AnyWindowHandle>,
    owner: WeakEntity<Shell>,
    store: Database,
    root: PathBuf,
    prefs: crate::settings::Preferences,
    pub(crate) roster: Roster,
    pub(crate) active: Option<String>,
    chats: HashMap<String, Entity<Conversation>>,
    bridge: Option<pilot::bridge::Server>,
    observations: Vec<Subscription>,
    _bridge_task: Option<gpui::Task<()>>,
    _clock: Option<gpui::Task<()>>,
    writes: Option<gpui::Task<()>>,
    loading_chats: HashSet<String>,
    work: work::Cache,
    pub(crate) error: Option<String>,
    notice_region: AnchorSlot,
    pub(crate) details: Entity<overview::Overview>,
    original_chrome: bool,
    overview_tab: OverviewTab,
    overview_previous: OverviewTab,
    overview_reveal: Option<crate::project_tabs::TabReveal<OverviewTab>>,
    /// Each Pilot's SOP Roadmap tab, made when first shown.
    roadmaps: HashMap<String, (Entity<roadmap::Roadmap>, [Subscription; 2])>,
    /// Roadmaps to reload when next drawn.
    stale_roadmaps: HashSet<String>,
    route: Option<Route>,
    settings: Option<SettingsSection>,
    settings_focus: gpui::FocusHandle,
    fields: HashMap<String, Entity<Input>>,
    memory_edit: Option<String>,
    memory_adding: bool,
    memory_pending: HashSet<String>,
    field_blurs: HashSet<gpui::EntityId>,
    file_versions: HashMap<String, (String, String)>,
    saving_files: HashSet<String>,
    file_load_revision: u64,
    files: Option<AgentFiles>,
    habits: Vec<Habit>,
    habit_ui: scheduler::HabitUi,
    picker: Option<OpenPicker>,
    settings_anchor: AnchorSlot,
    model_anchor: AnchorSlot,
    projects_anchor: AnchorSlot,
    access_anchor: AnchorSlot,
    menu: Option<(String, Entity<Menu>, Vec<Subscription>)>,
    menu_anchor: Anchor,
    documents: HashMap<String, Entity<crate::transcript::TranscriptView>>,
    html_documents: HashMap<String, Entity<html::Html>>,
    project_changes: changes::ProjectChanges,
    review: Option<Entity<crate::review::ReviewPane>>,
    activity: Option<(String, Entity<crate::transcript::TranscriptView>)>,
    activity_turn: Option<usize>,
    rail_ids: Vec<String>,
    running: HashMap<i64, RunningHabit>,
    active_run: Option<i64>,
    /// Completion reports per `(pilot_id, source session)`, held until all of them are built.
    completion_work: HashMap<(String, i64), Vec<host::PendingReport>>,
    /// The open background dialog, centred in the window it opened in.
    background: Option<(
        Entity<crate::project_settings::BackgroundDialog>,
        Subscription,
        gpui::Point<gpui::Pixels>,
    )>,
    backgrounds: HashMap<String, Entity<crate::project_background::ProjectBackground>>,
    drag: Option<(String, gpui::Point<gpui::Pixels>, Option<String>)>,
    rail_bounds: HashMap<String, AnchorSlot>,
    skip_click: bool,
    floating: Vec<gpui::AnyWindowHandle>,
    scheduling: bool,
    persisted_messages: HashSet<String>,
    failed_reports: HashSet<String>,
    inflight: HashMap<String, Message>,
    /// Each loaded chat's last seen (provider, model): a switch retries its
    /// failed queued messages.
    chat_choices: HashMap<String, (String, Option<String>)>,
    card_refresh: HashMap<String, u64>,
    session_filters: filters::Filters,
    session_filter_anchor: AnchorSlot,
    app_results: HashMap<(i64, String), (String, Result<serde_json::Value, String>)>,
    app_pending: HashSet<(i64, String)>,
    /// Side reviews of finished Flow steps, by review session.
    reviews: HashMap<i64, flow_reviews::RunningReview>,
    /// Completed steps waiting for a review slot, oldest first.
    review_queue: std::collections::VecDeque<flow_reviews::QueuedReview>,
    flow_watch: flow_reviews::Watch,
    /// Prompts hidden runs were given; tests never send them.
    #[cfg(test)]
    pub(crate) hidden_prompts: Vec<(i64, String)>,
    /// Hidden runs cannot start, as when their session refuses the prompt.
    #[cfg(test)]
    pub(crate) hidden_dispatch_fails: bool,
    /// Every Flow event handed to a Pilot, in order.
    #[cfg(test)]
    pub(crate) flow_events: Vec<(String, crate::shell::pilot_flows::PilotFlowEvent)>,
    /// Every Pilot a successful mutating `flows.*` call changed, in order.
    #[cfg(test)]
    pub(crate) flows_changes: Vec<String>,
}
impl EventEmitter<Changed> for Pilots {}
impl EventEmitter<TabOpened> for Pilots {}
impl EventEmitter<ShowDetails> for Pilots {}
impl Pilots {
    pub(crate) fn resource_sessions(
        &self,
        cx: &App,
    ) -> Vec<(i64, crate::resource_pressure::SessionInfo)> {
        self.chats
            .values()
            .chain(self.running.values().map(|r| &r.view))
            .chain(self.reviews.values().map(|r| &r.view))
            .map(|v| {
                let s = &v.read(cx).summary;
                (
                    s.id,
                    crate::resource_pressure::SessionInfo {
                        title: s.title.clone().into(),
                        provider: s.provider.clone(),
                    },
                )
            })
            .collect()
    }
    pub(crate) fn stop_session(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(view) = self
            .chats
            .values()
            .chain(self.running.values().map(|r| &r.view))
            .chain(self.reviews.values().map(|r| &r.view))
            .find(|v| v.read(cx).summary.id == id)
        {
            view.update(cx, |v, cx| v.stop_if_busy(cx));
        }
    }
    pub(crate) fn bind_window(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.parent_window = Some(window.window_handle());
        self.sync_status(cx);
    }
    /// The menu-bar item's Hold or Resume label follows the hold.
    pub(crate) fn hold_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_status(cx);
    }
    fn sync_status(&mut self, _cx: &mut Context<Self>) {
        #[cfg(all(target_os = "macos", not(test)))]
        {
            if !self.prefs.pilot_menu_bar_icon {
                self.status = None;
                return;
            }
            if self.status.is_none() {
                self.status = status::Status::new();
            }
            let entity = _cx.weak_entity();
            let app = std::cell::RefCell::new(_cx.to_async());
            let run = std::rc::Rc::new(move |id: &str| {
                let _ = entity.update(&mut *app.borrow_mut(), |this, cx| {
                    if id == "quit" || id == "hold" {
                        // Both go through the window, so Hold asks first and
                        // Quit waits for anything still finishing.
                        if let Some(handle) = this.parent_window {
                            let owner = this.owner.clone();
                            let hold = id == "hold";
                            let app: &mut App = cx;
                            app.defer(move |cx| {
                                let _ = handle.update(cx, |_, window, cx| {
                                    window.activate_window();
                                    let _ = owner.update(cx, |o, cx| {
                                        if hold {
                                            o.pilot_toggle_hold(window, cx)
                                        } else {
                                            o.pilot_request_quit(window, cx)
                                        }
                                    });
                                });
                            });
                        } else if id == "quit" {
                            cx.quit();
                        }
                    } else if id == "composer" {
                        // The composer is its own panel: the workspace window
                        // stays where it is, as in the Tauri menu.
                        if let Some(handle) = this.parent_window {
                            let owner = this.owner.clone();
                            let app: &mut App = cx;
                            app.defer(move |cx| {
                                let _ = handle.update(cx, |_, window, cx| {
                                    let _ =
                                        owner.update(cx, |o, cx| o.pilot_quick_compose(window, cx));
                                });
                            });
                        }
                    } else if this.roster.get(id).is_some() {
                        // Only the floating chat opens; the workspace keeps
                        // whichever Pilot or session it was showing.
                        this.open_floating(id.into(), cx);
                    }
                });
            });
            let pilots = self
                .roster
                .pilots
                .iter()
                .map(|m| (m.id.clone(), m.label()))
                .collect::<Vec<_>>();
            let hold = crate::quick_open::hold_label(_cx);
            if let Some(status) = &mut self.status {
                status.update(&pilots, hold, run);
            }
        }
    }
    pub(crate) fn hide_html(&self, cx: &App) {
        for view in self.html_documents.values() {
            view.read(cx).hide();
        }
    }
    pub(crate) fn new(
        store: Database,
        owner: WeakEntity<Shell>,
        prefs: crate::settings::Preferences,
        active: Option<String>,
        notice_region: AnchorSlot,
        cx: &mut Context<Self>,
    ) -> Self {
        let (roster, root, roster_error) = match locked(&store) {
            Ok(db) => match Roster::load(&db) {
                Ok(roster) => (roster, pilot::root(&db), None),
                Err(error) => (Roster::default(), pilot::root(&db), Some(error)),
            },
            Err(error) => (
                Roster::default(),
                std::env::temp_dir().join(pilot::new_id()),
                Some(error),
            ),
        };
        let (bridge, rx) = if cx.is_test() {
            (None, None)
        } else {
            match pilot::bridge::Server::start() {
                Ok((server, rx)) => (Some(server), Some(rx)),
                Err(_) => (None, None),
            }
        };
        let persisted_messages = roster
            .pilots
            .iter()
            .flat_map(|m| m.outbox.iter().map(|q| q.id.clone()))
            .collect();
        let session_filters = locked(&store)
            .ok()
            .and_then(|s| s.setting("pilot-session-filters").ok().flatten())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let manager = cx.entity();
        let details = cx.new(|cx| overview::Overview::new(manager, cx));
        let mut this = Self {
            #[cfg(all(target_os = "macos", not(test)))]
            status: None,
            parent_window: None,
            owner,
            store,
            root,
            prefs,
            roster,
            active: None,
            chats: HashMap::new(),
            bridge,
            observations: Vec::new(),
            _bridge_task: None,
            _clock: None,
            writes: None,
            loading_chats: HashSet::new(),
            work: work::Cache::default(),
            error: roster_error,
            notice_region,
            details,
            original_chrome: false,
            overview_tab: OverviewTab::Sessions,
            overview_previous: OverviewTab::Sessions,
            overview_reveal: None,
            roadmaps: HashMap::new(),
            stale_roadmaps: HashSet::new(),
            route: None,
            settings: None,
            settings_focus: cx.focus_handle(),
            fields: HashMap::new(),
            memory_edit: None,
            memory_adding: false,
            memory_pending: HashSet::new(),
            field_blurs: HashSet::new(),
            file_versions: HashMap::new(),
            saving_files: HashSet::new(),
            file_load_revision: 0,
            files: None,
            habits: Vec::new(),
            habit_ui: scheduler::HabitUi::default(),
            picker: None,
            settings_anchor: AnchorSlot::default(),
            model_anchor: AnchorSlot::default(),
            projects_anchor: AnchorSlot::default(),
            access_anchor: AnchorSlot::default(),
            menu: None,
            menu_anchor: Anchor::Point(gpui::Point::default()),
            documents: HashMap::new(),
            html_documents: HashMap::new(),
            project_changes: Default::default(),
            review: None,
            activity: None,
            activity_turn: None,
            rail_ids: Vec::new(),
            running: HashMap::new(),
            active_run: None,
            completion_work: HashMap::new(),
            background: None,
            backgrounds: HashMap::new(),
            drag: None,
            rail_bounds: HashMap::new(),
            skip_click: false,
            floating: Vec::new(),
            scheduling: false,
            persisted_messages,
            failed_reports: HashSet::new(),
            inflight: HashMap::new(),
            chat_choices: HashMap::new(),
            card_refresh: HashMap::new(),
            session_filters,
            session_filter_anchor: AnchorSlot::default(),
            app_results: HashMap::new(),
            app_pending: HashSet::new(),
            reviews: HashMap::new(),
            review_queue: Default::default(),
            flow_watch: Default::default(),
            #[cfg(test)]
            hidden_prompts: Vec::new(),
            #[cfg(test)]
            hidden_dispatch_fails: false,
            #[cfg(test)]
            flow_events: Vec::new(),
            #[cfg(test)]
            flows_changes: Vec::new(),
        };
        if let Some(rx) = rx {
            this._bridge_task = Some(cx.spawn(async move |this, cx| {
                while let Ok(call) = rx.recv().await {
                    if this
                        .update_in(cx, |this, window, cx| this.app_call(call, window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        this._clock = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        if this.prefs.pilots
            && let Some(id) = active.filter(|id| this.roster.get(id).is_some())
        {
            this.active = Some(id.clone());
            this.ensure_chat(&id, cx);
            this.reload_files(&id, cx);
        }
        this.file_loose_launches(cx);
        this
    }
    /// Every Pilot-owned conversation Hold must stop: chats (resumed last),
    /// running habits and Flow reviews.
    pub(crate) fn hold_views(&self) -> Vec<(Entity<Conversation>, bool)> {
        let chats = self.chats.values().map(|view| (view.clone(), true));
        let habits = self.running.values().map(|run| (run.view.clone(), true));
        let reviews = self
            .reviews
            .values()
            .map(|review| (review.view.clone(), false));
        chats.chain(habits).chain(reviews).collect()
    }
    pub(crate) fn is_pilot_session(&self, id: i64) -> bool {
        self.roster.hidden_sessions.contains(&id)
            || self.roster.of_session(id).is_some()
            || self
                .roster
                .pilots
                .iter()
                .any(|m| m.habit_sessions.contains(&id))
    }
    pub(crate) fn hidden_session(&self, id: i64) -> bool {
        self.is_pilot_session(id)
    }
    pub(crate) fn set_preferences(
        &mut self,
        prefs: crate::settings::Preferences,
        cx: &mut Context<Self>,
    ) {
        let was = self.prefs.pilots;
        self.prefs = prefs;
        if was && !self.prefs.pilots {
            self.active = None;
        }
        cx.notify();
    }
    pub(crate) fn set_original_chrome(&mut self, original: bool, cx: &mut Context<Self>) {
        if self.original_chrome == original {
            return;
        }
        self.original_chrome = original;
        self.details
            .update(cx, |view, cx| view.set_original_chrome(original, cx));
        cx.notify();
    }
    /// The session the details panel shows as a page, if any.
    pub(crate) fn routed_session(&self) -> Option<i64> {
        match self.route {
            Some(Route::Session(id)) => Some(id),
            _ => None,
        }
    }
    pub(crate) fn overview_chrome(&self) -> OverviewChrome {
        let counts = self.current().map_or((0, 0, 0, 0), |pilot| {
            (
                pilot.launches.len(),
                pilot.documents.len(),
                self.habits.len(),
                self.project_changes.count(&pilot.id),
            )
        });
        OverviewChrome {
            active: self.active.clone(),
            tab: self.overview_tab,
            routed: self.route.is_some(),
            counts,
        }
    }
    fn show_overview(&self, cx: &mut Context<Self>) {
        if let Some(id) = self.active.clone() {
            cx.emit(ShowDetails(id));
        }
    }
    /// TitleBar.tsx's PilotTitle, used by the original full-width Pilot view.
    pub(crate) fn original_title(&self, inset: f32, cx: &App) -> AnyElement {
        let Some(pilot) = self.current() else {
            return div().flex_1().min_w_0().into_any_element();
        };
        let state = self
            .chats
            .get(&pilot.id)
            .map(|chat| chat.read(cx).pilot_status())
            .unwrap_or("idle");
        let status = match state {
            "working" => "Working",
            "needs-you" => "Needs you",
            _ => "Idle",
        };
        div()
            .id("original-pilot-title")
            .debug_selector(|| "original-pilot-title".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(px(inset))
            .pr_4()
            .flex()
            .items_center()
            .gap_2()
            .child(avatar::avatar(
                &pilot.mascot,
                pilot.color,
                state,
                16.,
                state == "idle",
            ))
            .child(
                div()
                    .debug_selector(|| "original-pilot-name".into())
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .child(pilot.label()),
            )
            .child(
                div()
                    .debug_selector(|| "original-pilot-status".into())
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(if state == "needs-you" {
                        accent_color()
                    } else {
                        content(0.45)
                    })
                    .child(status),
            )
            .into_any_element()
    }
    pub(crate) fn open_current_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active.clone() else {
            return;
        };
        self.settings = Some(SettingsSection::General);
        window.focus(&self.settings_focus, cx);
        self.reload_files(&id, cx);
        cx.notify();
    }
    pub(crate) fn current(&self) -> Option<&Pilot> {
        self.active.as_deref().and_then(|id| self.roster.get(id))
    }
    pub(crate) fn chat(&self) -> Option<Entity<Conversation>> {
        self.active
            .as_ref()
            .and_then(|id| self.chats.get(id))
            .cloned()
    }
    /// A Pilot's chat when it is open, without opening it.
    pub(crate) fn open_chat(&self, id: &str) -> Option<Entity<Conversation>> {
        self.chats.get(id).cloned()
    }
    /// A Pilot's chat for a paired phone, opening it in the background when
    /// it is not open yet.
    pub(crate) fn mobile_chat(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Conversation>> {
        self.pane_chat(id, cx)
    }
    /// A Pilot's chat for a split pane, loading it when it is not open yet.
    pub(crate) fn pane_chat(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Conversation>> {
        self.ensure_chat(id, cx);
        self.chats.get(id).cloned()
    }
    /// A phone's message to a Pilot, queued as one typed in its chat.
    pub(crate) fn mobile_send(
        &mut self,
        id: &str,
        text: String,
        attachments: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.roster.get(id).is_none() {
            return Err("Pilot is no longer available".into());
        }
        self.ensure_chat(id, cx);
        self.enqueue(id, text, attachments, false, cx);
        Ok(())
    }
    pub(crate) fn account_changed(
        &mut self,
        kind: Kind,
        id: &str,
        removed: bool,
        cx: &mut Context<Self>,
    ) {
        for view in self.chats.values() {
            if view.read(cx).summary.provider == kind.id()
                && view.read(cx).settings.account.id == id
            {
                view.update(cx, |view, cx| {
                    if removed {
                        view.account_unavailable(cx)
                    } else {
                        view.account_authenticated(cx)
                    }
                });
            }
        }
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.sync_status(cx);
        cx.emit(Changed);
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let roster = self.roster.clone();
        let store = self.store.clone();
        let queued = roster
            .pilots
            .iter()
            .map(|m| {
                (
                    m.id.clone(),
                    m.outbox.iter().map(|q| q.id.clone()).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        // Every outbox change saves: a message the roadmap sent may be gone.
        self.sync_roadmap_queues(cx);
        let previous = self.writes.take();
        self.writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                let db = locked(&store)?;
                roster.save(&db)
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
                Ok(()) => {
                    for (id, messages) in queued {
                        this.persisted_messages.extend(messages);
                        this.dispatch(&id, cx);
                    }
                }
            });
        }));
        self.changed(cx);
    }
    fn ensure_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.chats.contains_key(id) || !self.loading_chats.insert(id.into()) {
            return;
        }
        let Some(pilot) = self.roster.get(id).cloned() else {
            return;
        };
        let store = self.store.clone();
        let prefs = self.prefs.clone();
        let known = crate::installed_providers::snapshot(cx);
        let id = id.to_string();
        let work = cx.background_executor().spawn(async move {
            let db = locked(&store)?;
            if let Some(missing) = &pilot.unavailable_session {
                return Err(format!("This Pilot's saved conversation ({missing}) is unavailable. Its identity and history have been preserved."));
            }
            if let Some(session) = pilot.session_id {
                let summary = db.pilot_session(session)?
                    .ok_or("This Pilot's saved conversation is unavailable. Reset its conversation explicitly to start a new one.")?;
                let delivered = pilot
                    .outbox
                    .iter()
                    .filter(|q| {
                        db.setting(&format!("pilot-delivered:{}:{}", summary.id, q.id))
                            .ok()
                            .flatten()
                            .is_some()
                    })
                    .map(|q| q.id.clone())
                    .collect::<Vec<_>>();
                return Ok::<_, String>((summary, None, delivered));
            }
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".into());
            // A new Pilot's chat starts where a new session would.
            let (provider, model) =
                crate::installed_providers::new_session_choice_resolving(&known, &prefs, None);
            let mut summary = db.create(provider.id(), &home)?;
            summary.title = pilot.label();
            db.title(summary.id, &summary.title)?;
            let mut settings = Settings {
                access: AccessMode::Auto,
                model,
                ..Default::default()
            };
            prefs.apply_last_controls(&mut settings, provider);
            db.configure_agent(summary.id, provider.id(), &settings)?;
            Ok((summary, Some(settings), Vec::new()))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading_chats.remove(&id);
                let (summary, settings, delivered) = match result {
                    Ok(value) => value,
                    Err(e) => {
                        this.error = Some(e);
                        this.changed(cx);
                        return;
                    }
                };
                if !this.roster.hidden_sessions.contains(&summary.id) {
                    this.roster.hidden_sessions.push(summary.id);
                }
                let Some(pilot) = this.roster.get_mut(&id) else {
                    return;
                };
                pilot.session_id = Some(summary.id);
                pilot.outbox.retain(|q| !delivered.contains(&q.id));
                let pilot = pilot.clone();
                let view = cx.new(|cx| Conversation::new(summary, this.store.clone(), cx));
                let root = this.root.clone();
                let env = this
                    .bridge
                    .as_ref()
                    .map(|b| b.environment(view.read(cx).summary.id))
                    .unwrap_or_default();
                view.update(cx, |view, cx| {
                    view.configure_pilot(pilot, root, env, cx);
                    if let Some(settings) = settings {
                        let config = view.settings.pilot.take();
                        let env = std::mem::take(&mut view.settings.environment);
                        view.install_transfer(settings, None, cx);
                        view.settings.pilot = config;
                        view.settings.environment = env;
                    }
                    view.load(cx);
                });
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, event: &PilotSubmitted, cx| {
                        this.enqueue(
                            &source,
                            event.text.clone(),
                            event
                                .attachments
                                .iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect(),
                            false,
                            cx,
                        );
                    },
                ));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, _: &SessionChanged, cx| {
                        this.chat_changed(&source, cx);
                    },
                ));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, _: &crate::conversation::UsageResumed, cx| {
                        let empty = this.roster.get(&source).is_none_or(|m| m.outbox.is_empty());
                        if empty {
                            this.enqueue(
                                &source,
                                "Continue from where you left off.".into(),
                                Vec::new(),
                                false,
                                cx,
                            );
                        } else {
                            if let Some(pilot) = this.roster.get_mut(&source) {
                                for message in &mut pilot.outbox {
                                    message.error = None;
                                }
                            }
                            this.save(cx);
                            this.configure_chat(&source, cx);
                        }
                    },
                ));
                let source = id.clone();
                this.observations
                    .push(cx.observe(&view, move |this, _, cx| {
                        if this.active.as_ref() == Some(&source) {
                            cx.notify();
                        }
                        this.dispatch(&source, cx);
                    }));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, event: &crate::conversation::PilotQueueChanged, cx| {
                        this.queue_changed(&source, &event.id, event.remove, cx)
                    },
                ));
                let transcript = view.read(cx).transcript.clone();
                let target = view.clone();
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &transcript,
                    move |this, _, event: &crate::transcript::PilotResponseAction, cx| match event {
                        crate::transcript::PilotResponseAction::Queue { id, remove } => {
                            this.queue_changed(&source, id, *remove, cx)
                        }
                        crate::transcript::PilotResponseAction::Activity(turn) => {
                            this.activity_turn = Some(*turn);
                            this.show_overview(cx);
                            this.route = None;
                            this.overview_tab = OverviewTab::Activity;
                            cx.notify();
                        }
                        crate::transcript::PilotResponseAction::Quote { text, window } => {
                            let target = target.clone();
                            let text = text.clone();
                            let handle = *window;
                            let app: &mut App = cx;
                            app.defer(move |cx| {
                                let _ = handle.update(cx, |_, window, cx| {
                                    target.update(cx, |v, cx| v.pilot_quote(&text, window, cx))
                                });
                            });
                        }
                    },
                ));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &transcript,
                    move |this, _, event: &cards::CardAction, cx| {
                        this.card_action(&source, &event.0, cx)
                    },
                ));
                if this.active.as_deref() == Some(&id) {
                    // Opened before its chat loaded: `open` had no dictation
                    // to keep, so other composers stop now.
                    let dictation = view.read(cx).dictation.clone();
                    crate::conversation::dictation::switched_to(&dictation, cx);
                }
                this.chats.insert(id.clone(), view);
                if let Some(handle) = this.parent_window {
                    let owner = cx.weak_entity();
                    let id = id.clone();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if this.active.as_deref() == Some(&id)
                                    && this.settings_focus.is_focused(window)
                                {
                                    this.focus_active(window, cx);
                                }
                            });
                        });
                    });
                }
                this.save(cx);
                this.dispatch(&id, cx);
            });
        })
        .detach();
    }
    /// Retry clears a queued message's error; discard drops it.
    fn queue_changed(&mut self, source: &str, id: &str, remove: bool, cx: &mut Context<Self>) {
        if let Some(pilot) = self.roster.get_mut(source) {
            if remove {
                pilot.outbox.retain(|m| m.id != id);
            } else if let Some(message) = pilot.outbox.iter_mut().find(|m| m.id == id) {
                message.error = None;
            }
        }
        self.save(cx);
        self.configure_chat(source, cx);
    }
    fn configure_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        self.refresh_work(id, cx);
        if let Some(pilot) = self.roster.get(id) {
            // The shell may be mid-update when this runs; reach it afterwards.
            let launches = pilot.launches.iter().map(|l| l.session_id).collect();
            let owner = self.owner.clone();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |shell, cx| shell.pilot_track_sessions(launches, cx));
            });
        }
        if let (Some(pilot), Some(view)) =
            (self.roster.get(id).cloned(), self.chats.get(id).cloned())
        {
            let env = self
                .bridge
                .as_ref()
                .map(|b| b.environment(view.read(cx).summary.id))
                .unwrap_or_default();
            let root = self.root.clone();
            view.update(cx, |view, cx| view.configure_pilot(pilot, root, env, cx));
        }
    }
    /// Queues a message for Pilot `id`'s chat; returns the message's id.
    fn enqueue(
        &mut self,
        id: &str,
        text: String,
        attachments: Vec<String>,
        internal: bool,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let message_id = pilot::new_id();
        self.roster.get_mut(id)?.outbox.push(Message {
            id: message_id.clone(),
            text,
            sent_at: Some(gespan_core::time::now_ms()),
            internal,
            error: None,
            attachments,
            report: Vec::new(),
        });
        self.save(cx);
        self.configure_chat(id, cx);
        self.dispatch(id, cx);
        Some(message_id)
    }
    fn dispatch(&mut self, id: &str, cx: &mut Context<Self>) {
        let delivered = self
            .chats
            .get(id)
            .map(|v| v.read(cx).pilot_delivered().clone())
            .unwrap_or_default();
        let failures = self
            .chats
            .get(id)
            .cloned()
            .map(|view| view.update(cx, |view, _| view.pilot_take_failures()))
            .unwrap_or_default();
        let mut changed = false;
        if let Some(pilot) = self.roster.get_mut(id) {
            let before = pilot.outbox.len();
            pilot
                .outbox
                .retain(|message| !delivered.contains(&message.id));
            changed = before != pilot.outbox.len();
            for (id, error) in failures {
                if let Some(message) = pilot.outbox.iter_mut().find(|m| m.id == id) {
                    message.error = Some(error);
                    changed = true;
                }
            }
        }
        if changed {
            self.save(cx);
            self.configure_chat(id, cx);
        }

        if let Some(view) = self
            .chats
            .get(id)
            .cloned()
            .filter(|v| !self.failed_reports.contains(id) && v.read(cx).ready_for_external(cx))
        {
            let reports = self
                .roster
                .get_mut(id)
                .map(|m| std::mem::take(&mut m.pending_reports))
                .unwrap_or_default();
            if !reports.is_empty() {
                let mut failed = Vec::new();
                for report in reports {
                    if self.failed_reports.contains(id) {
                        failed.push(report);
                        continue;
                    }
                    let (role, text) = report
                        .strip_prefix("pilot-card:")
                        .map(|s| ("pilot-card", s.to_string()))
                        .unwrap_or_else(|| ("assistant", report.clone()));
                    if let Err(error) = view.update(cx, |v, cx| v.pilot_post(role, text, cx)) {
                        self.error = Some(error);
                        self.failed_reports.insert(id.into());
                        failed.push(report);
                    }
                }
                if let Some(pilot) = self.roster.get_mut(id) {
                    pilot.pending_reports.extend(failed);
                }
                self.save(cx);
            }
        }
        let Some(view) = self.chats.get(id).cloned() else {
            return;
        };
        let busy = view.read(cx).busy;
        let Some(message) = self.next_outbox_message(id, busy) else {
            return;
        };
        if view.update(cx, |v, cx| v.pilot_dispatch(&message, cx)) && !busy {
            self.inflight.insert(id.into(), message);
        }
    }
    /// A failed message stays at the head of the outbox. When the user moves
    /// the chat to another provider or model, the queue is retried there; the
    /// caller dispatches.
    /// Ignores the values a chat loads with, and a model a running turn reports.
    fn retry_after_provider_switch(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(view) = self.chats.get(id) else {
            return;
        };
        let view = view.read(cx);
        if !view.pilot_history_ready() {
            self.chat_choices.remove(id);
            return;
        }
        let busy = view.busy;
        let choice = (view.summary.provider.clone(), view.pilot_model());
        let Some(previous) = self.chat_choices.insert(id.into(), choice.clone()) else {
            return;
        };
        if previous == choice || (busy && previous.0 == choice.0) {
            return;
        }
        let mut retried = false;
        if let Some(pilot) = self.roster.get_mut(id) {
            for message in &mut pilot.outbox {
                retried |= message.error.take().is_some();
            }
        }
        if retried {
            self.save(cx);
            self.configure_chat(id, cx);
        }
    }
    /// The outbox message `dispatch` sends next: the first one the chat can
    /// take now. A failed head holds the queue until it is retried.
    fn next_outbox_message(&self, id: &str, busy: bool) -> Option<Message> {
        self.roster
            .get(id)
            .and_then(|m| m.outbox.iter().find(|message| !busy || !message.internal))
            .cloned()
            .filter(|m| m.error.is_none() && self.persisted_messages.contains(&m.id))
    }
    fn chat_changed(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(view) = self.chats.get(id) else {
            return;
        };
        let view = view.read(cx);
        let row_count = view.pilot_rows(cx).len();
        let finished = !view.busy;
        let completion =
            (finished && self.inflight.contains_key(id)).then(|| NotificationFinished {
                summary: view.summary.clone(),
                body: view.notification_reply_body(cx),
                habit: false,
                was_cancelled: view.notification_was_cancelled(),
            });
        cx.emit(NotificationChanged);
        if let Some(completion) = completion {
            cx.emit(completion);
        }
        if self.active.as_deref() == Some(id)
            && let Some(pilot) = self.roster.get_mut(id)
        {
            pilot.seen_rows = row_count;
        }
        if finished {
            if let Some(mut sent) = self.inflight.remove(id) {
                let (ok, status) = self.chats[id].read(cx).external_outcome(cx);
                if !ok && !status.starts_with("Stopped") {
                    if let Some(pilot) = self.roster.get_mut(id) {
                        if let Some(queued) = pilot.outbox.iter_mut().find(|m| m.id == sent.id) {
                            queued.error = Some(status);
                        } else {
                            sent.id = pilot::new_id();
                            sent.error = Some(status);
                            pilot.outbox.insert(0, sent);
                        }
                    }
                    self.save(cx);
                    self.configure_chat(id, cx);
                }
            }
            self.check_completions(cx);
            self.reload_files(id, cx);
            if self.flow_alert_turns(id) {
                // A turn that read a failure may have ended: its goal can
                // roll up now.
                self.settle_flows(id, cx);
            } else {
                self.flush_flow_alerts(id, cx);
            }
            self.pump_reviews(cx);
        }
        self.retry_after_provider_switch(id, cx);
        self.dispatch(id, cx);
        self.changed(cx);
    }
    pub(crate) fn open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.prefs.pilots || self.roster.get(id).is_none() {
            return;
        }
        if self.active.as_deref() != Some(id) {
            self.save_open_file(cx);
            self.pause_changes(cx);
            self.route = None;
            self.details.update(cx, |details, cx| details.reset(cx));
            self.fields.retain(|k, _| k.starts_with("file:"));
            self.files = None;
            self.activity_turn = None;
            self.memory_adding = false;
            self.memory_edit = None;
            self.settings = None;
        }
        self.active = Some(id.into());
        self.reload_roadmap(cx);
        self.ensure_chat(id, cx);
        // One dictation at a time, as when switching sessions.
        if let Some(chat) = self.chats.get(id) {
            let dictation = chat.read(cx).dictation.clone();
            crate::conversation::dictation::switched_to(&dictation, cx);
        }
        self.reload_files(id, cx);
        cx.emit(TabOpened(id.into()));
        self.focus_active(window, cx);
        if let Some(view) = self.chats.get(id) {
            let count = view.read(cx).pilot_rows(cx).len();
            if let Some(pilot) = self.roster.get_mut(id) {
                pilot.seen_rows = count;
            }
        }
        self.save(cx);
    }
    pub(crate) fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_none()
            && let Some(chat) = self.chat()
        {
            window.focus(&chat.read(cx).input.focus_handle(cx), cx);
        } else {
            window.focus(&self.settings_focus, cx);
        }
    }
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.save_open_file(cx);
        self.pause_changes(cx);
        self.hide_html(cx);
        if self.active.take().is_some() {
            self.details.update(cx, |details, cx| details.reset(cx));
            self.settings = None;
            self.picker = None;
            self.save(cx);
        }
    }
    pub(crate) fn dismiss_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.picker.take().is_some() || self.menu.take().is_some() {
            cx.notify();
            return true;
        }
        if self.settings.take().is_none() {
            return false;
        }
        self.save_open_file(cx);
        if let Some(chat) = self.chat() {
            window.focus(&chat.read(cx).input.focus_handle(cx), cx);
        }
        cx.notify();
        true
    }
    pub(crate) fn create(
        &mut self,
        projects: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.roster.create(projects).id.clone();
        self.save(cx);
        self.open(&id, window, cx);
        self.settings = Some(SettingsSection::General);
        window.focus(&self.settings_focus, cx);
        self.changed(cx);
    }
    fn reload_files(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(id) {
            return;
        }
        self.file_load_revision += 1;
        let revision = self.file_load_revision;
        let Some(pilot) = self.roster.get(id).cloned() else {
            return;
        };
        let root = self.root.clone();
        let id = id.to_string();
        let work = cx.background_executor().spawn(async move {
            Ok::<_, String>((
                pilot::load_files(&root, &pilot)?,
                habits::load(&root, &pilot.id)?,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.active.as_deref() == Some(&id) && this.file_load_revision == revision {
                    match result {
                        Ok((files, habits)) => {
                            this.files = Some(files);
                            this.habits = habits;
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(crate) fn rail(&mut self, project: Option<String>, cx: &mut Context<Self>) -> AnyElement {
        self.rail_impl(project, false, false, cx)
    }
    /// Original navigation lists every Pilot, before the projects; the inset
    /// layout continues to list only the current project's Pilots.
    pub(crate) fn project_navigation_rail(
        &mut self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.rail_impl(None, true, compact, cx)
    }
    fn rail_impl(
        &mut self,
        project: Option<String>,
        original: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.prefs.pilots {
            return div().into_any_element();
        }
        let roster: Vec<_> = self
            .roster
            .pilots
            .iter()
            .filter(|m| project.as_ref().is_none_or(|p| m.works_on(p)))
            .cloned()
            .collect();
        self.rail_ids = roster.iter().map(|m| m.id.clone()).collect();
        for pilot in &roster {
            self.rail_bounds.entry(pilot.id.clone()).or_default();
        }
        let count = roster.len();
        let has = count > 0;
        if original && !has && compact {
            return div().into_any_element();
        }
        let add_project = project.clone();
        let heading = if original {
            div()
                .h(px(30.))
                .pl(px(10.))
                .pr_3()
                .pt_1()
                .pb(px(6.))
                .flex()
                .items_center()
                .gap_1()
                .when(compact, |d| d.px(px(14.)))
                .when(!compact, |d| {
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_1()
                            .text_size(px(12.))
                            .line_height(px(20.))
                            .text_color(content(0.5))
                            .child("Pilots"),
                    )
                })
        } else {
            section_label("Pilots")
        }
        .when(has, |d| {
            d.child(
                icon_button("new-pilot", "New pilot", "plus")
                    .debug_selector(|| "sidebar-pilot-button".into())
                    .size(px(20.))
                    .when(!original, |d| d.mr(px(-2.)))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.create(add_project.clone().into_iter().collect(), w, cx)
                    })),
            )
        });
        div()
            .when(!original, |d| d.px_2().pb_3())
            .when(original && !compact, |d| d.pb_2())
            .when(compact, |d| {
                d.w_full().flex().flex_col().items_center().gap(px(6.))
            })
            .when(!compact, |d| d.child(heading))
            .when(!has, |d| {
                d.child(
                    div()
                        .id("new-pilot-row")
                        .debug_selector(|| "new-pilot".into())
                        .role(Role::Button)
                        .aria_label("New pilot")
                        .h(px(32.))
                        .when(original, |d| {
                            d.ml(px(6.)).mr_2().font_weight(gpui::FontWeight::MEDIUM)
                        })
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .text_color(content(0.5))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon("plus", 16.))
                        .child("New pilot")
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.create(project.clone().into_iter().collect(), w, cx)
                        })),
                )
            })
            .children(roster.into_iter().enumerate().map(|(index, pilot)| {
                let active = self.active.as_deref() == Some(&pilot.id);
                let state = self
                    .chats
                    .get(&pilot.id)
                    .map(|v| v.read(cx).pilot_status())
                    .unwrap_or("idle");
                let unread = self
                    .chats
                    .get(&pilot.id)
                    .map(|v| {
                        v.read(cx)
                            .pilot_rows(cx)
                            .iter()
                            .skip(pilot.seen_rows)
                            .filter(|r| {
                                r.role == "assistant" && r.first && !r.text.trim().is_empty()
                            })
                            .count()
                    })
                    .unwrap_or(0);
                // Like PilotRailSection, an open Pilot has already been seen.
                // Activity belongs on the mascot, rather than an idle dot at
                // the end of every row.
                let unread = if active { 0 } else { unread };
                let label = if unread == 0 {
                    format!("{}, {state}", pilot.label())
                } else {
                    format!(
                        "{}, {state}, {unread} new {}",
                        pilot.label(),
                        if unread == 1 { "reply" } else { "replies" }
                    )
                };
                let id = pilot.id.clone();
                let options = id.clone();
                let context = id.clone();
                let drag = id.clone();
                let slot = self.rail_bounds[&id].clone();
                div()
                    .id(SharedString::from(format!("pilot-rail-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("pilot-rail-{id}")
                    })
                    .group("pilot-rail-row")
                    .role(Role::Button)
                    .aria_label(label)
                    .h(px(32.))
                    .when(original && !compact, |d| d.ml(px(6.)).mr_2())
                    .when(original && !compact && index + 1 < count, |d| d.mb_px())
                    .when(compact, |d| d.w(px(32.)))
                    .px_2()
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .opacity(if active { 1. } else { 0.65 })
                    .when(active, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.08)).opacity(1.))
                    .child(
                        div()
                            .size(px(16.))
                            .relative()
                            .flex_shrink_0()
                            .child(avatar::avatar(&pilot.mascot, pilot.color, state, 16., true))
                            .when(original, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right(px(-2.))
                                        .top(px(-2.))
                                        .size(px(6.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(rgb(crate::right_panel::BACKGROUND_BASE))
                                        .bg(if state == "working" {
                                            rgb(pilot.color).into()
                                        } else if state == "needs-you" {
                                            accent_color()
                                        } else {
                                            content(0.3)
                                        }),
                                )
                            }),
                    )
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when(unread > 0, |d| d.pr(px(24.)))
                                .group_hover("pilot-rail-row", |s| s.pr(px(24.)))
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(pilot.label()),
                        )
                    })
                    .when(unread > 0 && !compact, |d| {
                        d.child(
                            div()
                                .absolute()
                                .right_1()
                                .top(px(4.))
                                .size(px(24.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .group_hover("pilot-rail-row", |s| s.invisible())
                                .child(
                                    div()
                                        .debug_selector({
                                            let id = pilot.id.clone();
                                            move || format!("pilot-rail-unread-{id}")
                                        })
                                        .when(!original, |d| d.size(px(6.)))
                                        .when(original, |d| {
                                            d.h(px(16.))
                                                .min_w(px(16.))
                                                .px_1()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_size(px(10.))
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(rgb(0xffffff))
                                                .child(if unread > 99 {
                                                    "99+".into()
                                                } else {
                                                    unread.to_string()
                                                })
                                        })
                                        .rounded_full()
                                        .bg(accent_color()),
                                ),
                        )
                    })
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .absolute()
                                .right_1()
                                .top(px(4.))
                                .invisible()
                                .group_hover("pilot-rail-row", |s| s.visible())
                                .child(
                                    icon_button(
                                        "pilot-options",
                                        "Pilot options",
                                        "more-horizontal",
                                    )
                                    .debug_selector({
                                        let id = pilot.id.clone();
                                        move || format!("pilot-rail-options-{id}")
                                    })
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(
                                        move |this, _, w, cx| {
                                            cx.stop_propagation();
                                            this.open_options(&options, None, w, cx);
                                        },
                                    )),
                                ),
                        )
                    })
                    .child(slot.probe())
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if std::mem::take(&mut this.skip_click) {
                            return;
                        }
                        this.open(&id, w, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, w, cx| {
                            this.open_options(&context, Some(event.position), w, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, _| {
                            this.skip_click = false;
                            this.drag = Some((drag.clone(), event.position, None));
                        }),
                    )
            }))
            // PilotRailSection owns its menu and background dialog; the Pilot
            // view paints them instead while it is open.
            .when(self.current().is_none(), |d| {
                d.children(self.options_surface(cx))
                    .children(self.background_modal())
            })
            .into_any_element()
    }
    pub(crate) fn title_tab(&self, id: &str, width: f32, cx: &Context<Self>) -> Option<AnyElement> {
        self.roster.get(id).map(|pilot| {
            let id = pilot.id.clone();
            let close = id.clone();
            let active = self.active.as_deref() == Some(&id);
            let selector = format!("pilot-title-tab-{id}");
            let group: SharedString = selector.clone().into();
            div()
                .id(SharedString::from(format!("pilot-title-{id}")))
                .debug_selector(move || selector.clone())
                .role(Role::Tab)
                .aria_label(pilot.label())
                .aria_selected(active)
                .h(px(TITLE_BAR_HEIGHT))
                .w(px(width))
                .flex_shrink_0()
                .relative()
                .flex()
                .items_center()
                .group(group.clone())
                .child(
                    title_tab_plate(active, false)
                        .w_full()
                        .min_w_0()
                        .pr_7()
                        .gap(px(6.))
                        .group_hover(group.clone(), move |s| {
                            s.opacity(1.).bg(title_tab_fill(active, true))
                        })
                        .child(avatar::avatar(
                            &pilot.mascot,
                            pilot.color,
                            "idle",
                            16.,
                            true,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(20.))
                                .text_size(px(TITLE_TAB_TEXT_SIZE))
                                .text_color(content(if active { 1. } else { 0.5 }))
                                .group_hover(group.clone(), |s| s.text_color(content(1.)))
                                .child(crate::tab_label::tab_label(pilot.label())),
                        ),
                )
                .child(
                    div()
                        .id("close-pilot-tab")
                        .debug_selector(|| "close-pilot-tab-button".into())
                        .role(Role::Button)
                        .aria_label("Close Pilot tab")
                        .absolute()
                        .right_1()
                        .top(px(10.))
                        .size(px(20.))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .hover(|s| s.bg(content(0.1)))
                        .child(icon("close", 12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            let owner = this.owner.clone();
                            let close = close.clone();
                            window.defer(cx, move |window, cx| {
                                let _ = owner.update(cx, |shell, cx| {
                                    shell.close_pilot_tab(&close, window, cx)
                                });
                            });
                        })),
                )
                .on_mouse_up(
                    MouseButton::Middle,
                    cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            let owner = this.owner.clone();
                            let id = id.clone();
                            window.defer(cx, move |window, cx| {
                                let _ = owner
                                    .update(cx, |shell, cx| shell.close_pilot_tab(&id, window, cx));
                            });
                        }
                    }),
                )
                .on_click(cx.listener(move |this, _, w, cx| {
                    let owner = this.owner.clone();
                    let id = id.clone();
                    w.defer(cx, move |w, cx| {
                        let _ = owner.update(cx, |shell, cx| shell.click_pilot_tab(&id, w, cx));
                    });
                }))
                .into_any_element()
        })
    }
}
impl gpui::Render for Pilots {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.hide_html(cx);
        let Some(pilot) = self.current().cloned() else {
            return div().into_any_element();
        };
        self.ensure_changes_summary(&pilot, cx);
        let bg = self
            .backgrounds
            .entry(pilot.id.clone())
            .or_insert_with(|| cx.new(|_| crate::project_background::ProjectBackground::default()))
            .clone();
        let empty = self.chat().is_none_or(|v| !v.read(cx).summary.has_messages);
        bg.update(cx, |bg, cx| {
            bg.configure(pilot.background.clone(), empty, None, cx)
        });
        div()
            .size_full()
            .min_w_0()
            .flex()
            .relative()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape"
                    && this.settings.is_some()
                    && this.picker.is_none()
                    && this.menu.is_none()
                {
                    this.save_open_file(cx);
                    this.settings = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_any_mouse_down(|_, window, _| {
                #[cfg(target_os = "macos")]
                crate::traffic_lights::focus_content(window);
                #[cfg(not(target_os = "macos"))]
                let _ = window;
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .child(bg)
                    .children(
                        self.attention_views(&pilot.id, cx)
                            .into_iter()
                            .map(|view| view.update(cx, |v, cx| v.pilot_attention(window, cx))),
                    )
                    .map(|d| {
                        if self.floating_shows(&pilot.id, cx) {
                            d.child(self.floating_placeholder(&pilot, cx))
                        } else if let Some(chat) = self.chat() {
                            d.child(div().flex_1().min_h_0().child(chat))
                        } else {
                            d
                        }
                    })
                    .when(!self.original_chrome, |d| d.child(self.topbar(&pilot, cx))),
            )
            .children(self.settings_modal(&pilot, window, cx))
            .children(self.picker_surface(cx))
            .children(self.options_surface(cx))
            .children(self.background_modal())
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, _| {
                if let Some((_, start, over)) = &mut this.drag
                    && (event.position - *start).magnitude() > 5.
                {
                    this.skip_click = true;
                    *over = this.rail_bounds.iter().find_map(|(id, s)| {
                        s.get()
                            .filter(|b| b.contains(&event.position))
                            .map(|_| id.clone())
                    });
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_reorder(cx);
                }),
            )
            .into_any_element()
    }
}
