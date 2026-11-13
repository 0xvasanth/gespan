//! Runs use the same conversation lifecycle as an ordinary native submission.
use super::*;
use gespan_core::automations::{self, AutomationAction, AutomationRun, DueAutomationRun, Service};

pub(super) type ServiceHandle = Arc<Mutex<Service>>;

pub(super) struct Running {
    pub(super) run_id: String,
    submitting: bool,
    generation_before: u64,
    /// A review run's policy, to leave its findings as a draft review when
    /// it finishes.
    pub(super) review: Option<gespan_core::review_policy::ReviewPolicy>,
    pub(super) automation_id: String,
    pub(super) cwd: String,
    pub(super) title: String,
}

#[cfg(test)]
impl Running {
    pub(super) fn for_test(
        run_id: &str,
        review: gespan_core::review_policy::ReviewPolicy,
        automation_id: &str,
        cwd: &str,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            submitting: false,
            generation_before: 0,
            review: Some(review),
            automation_id: automation_id.into(),
            cwd: cwd.into(),
            title: "acme/app#7 Fix the login".into(),
        }
    }

    /// A run with no review policy, still submitting its prompt, so a chat
    /// change does not settle it.
    pub(super) fn submitting_for_test(run_id: &str, automation_id: &str, cwd: &str) -> Self {
        Self {
            run_id: run_id.into(),
            submitting: true,
            generation_before: 0,
            review: None,
            automation_id: automation_id.into(),
            cwd: cwd.into(),
            title: "Check the build".into(),
        }
    }
}

pub(super) struct State {
    pub service: Option<ServiceHandle>,
    pub open_requested: Option<u64>,
    /// Runs asked for before the service opened, at this navigation.
    pub runs_requested: Option<(u64, crate::runs::RunsFilter)>,
    opening: bool,
    pub(super) polling: bool,
    recovered: bool,
    started_before: i64,
    pending: HashSet<String>,
    pub running: HashMap<i64, Running>,
    reserved: HashSet<i64>,
    writes: Option<gpui::Task<()>>,
    event_retries: HashMap<String, gespan_core::inbox::Item>,
    waiting_events: HashMap<String, gespan_core::inbox::Item>,
    events_polling: bool,
    retry_revision: Arc<AtomicU64>,
    pub session_automations: HashMap<i64, String>,
    /// When each pull or merge request's comments were last read, by Inbox
    /// item key and its update time then.
    comments_read: HashMap<String, i64>,
    /// The GitHub and GitLab items the Inbox listed at its last refresh,
    /// until they have been looked at for comments, checks, reviews, merges
    /// and assignments. The Inbox offers an undated item to automations only
    /// when it first appears, so these arrive separately.
    listed: Vec<gespan_core::inbox::Item>,
    /// Draft reviews left on pull and merge requests, waiting for the user
    /// to submit them or keep them as drafts.
    pub(super) review_drafts: Vec<automations::review_drafts::SavedDraft>,
    /// Drafts whose answer is being sent, so a second click does nothing.
    pub(super) deciding_drafts: HashSet<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            service: None,
            open_requested: None,
            runs_requested: None,
            opening: false,
            polling: false,
            recovered: false,
            started_before: now(),
            pending: HashSet::new(),
            running: HashMap::new(),
            reserved: HashSet::new(),
            writes: None,
            event_retries: HashMap::new(),
            waiting_events: HashMap::new(),
            events_polling: false,
            retry_revision: Arc::new(AtomicU64::new(0)),
            session_automations: HashMap::new(),
            comments_read: HashMap::new(),
            listed: Vec::new(),
            review_drafts: Vec::new(),
            deciding_drafts: HashSet::new(),
        }
    }
}

pub(super) fn now() -> i64 {
    gespan_core::time::now_ms().min(i64::MAX as u64) as i64
}

pub(super) fn service_lock(
    service: &ServiceHandle,
) -> Result<std::sync::MutexGuard<'_, Service>, String> {
    service
        .lock()
        .map_err(|_| "Automations are unavailable".into())
}

impl Shell {
    /// Loads which sessions automations started, for the Automation runs
    /// folder, without opening the automation service.
    pub(super) fn load_automation_sessions(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.automation_session_ids() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(sessions) => {
                    this.automation_state.session_automations = sessions;
                    this.sort_saved_automation_tabs(cx);
                    this.notify_sidebar(cx);
                }
                Err(error) => tracing::warn!(%error, "automation runs failed to load"),
            });
        })
        .detach();
    }
    pub(super) fn initialize_automations(&mut self, cx: &mut Context<Self>) {
        if self.automation_state.opening || self.automation_state.service.is_some() {
            return;
        }
        self.automation_state.opening = true;
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let path = locked(&store)?.database_path().to_path_buf();
            let service = Service::open(&path).map(|service| Arc::new(Mutex::new(service)))?;
            let retries = locked(&store)?
                .setting("native-automation-inbox-retries")?
                .and_then(|raw| serde_json::from_str::<Vec<gespan_core::inbox::Item>>(&raw).ok())
                .unwrap_or_default();
            let sessions = locked(&store)?.automation_session_ids()?;
            Ok::<_, String>((service, retries, sessions))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.automation_state.opening = false;
                match result {
                    Ok((service, retries, sessions)) => {
                        this.automation_state.session_automations = sessions;
                        this.sort_saved_automation_tabs(cx);
                        // Their sessions move into the Automation runs folder.
                        this.notify_sidebar(cx);
                        this.automation_state.service = Some(service);
                        for item in retries {
                            this.automation_state.event_retries.insert(item.key(), item);
                        }
                        if !cfg!(test) {
                            this.ensure_background_inbox(window, cx);
                        }
                        this.poll_automations(cx);
                        this.load_review_drafts(cx);
                        let runs = this
                            .automation_state
                            .runs_requested
                            .take()
                            .filter(|(revision, _)| *revision == this.navigation_revision);
                        if this
                            .automation_state
                            .open_requested
                            .take()
                            .is_some_and(|revision| revision == this.navigation_revision)
                        {
                            this.open_automations(window, cx);
                        } else if let Some((_, filter)) = runs {
                            this.open_runs(filter, window, cx);
                        } else if this.navigation_revision == 0
                            && this.active_app_view.is_none()
                            && this.settings.is_none()
                        {
                            let saved = this.workspace.scope_focus.get(&this.tab_scope()).cloned();
                            if let Some(tab @ TitleTab::App(_)) = saved {
                                this.open_title_tab(&tab, window, cx);
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "automations failed to open");
                        this.error = Some(format!("Could not open automations: {error}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn poll_automations(&mut self, cx: &mut Context<Self>) {
        if self.secondary {
            return;
        }
        // A rejected or deferred submission still needs a terminal receipt.
        let sessions: Vec<_> = self.automation_state.running.keys().copied().collect();
        for id in sessions {
            self.automation_session_changed(id, cx);
        }
        // Due runs stay due while held and are claimed once on Resume.
        if crate::hold::active(cx) {
            return;
        }
        let Some(service) = self.automation_state.service.clone() else {
            self.initialize_automations(cx);
            return;
        };
        self.claim_inbox_events(cx);
        if self.automation_state.polling {
            return;
        }
        self.automation_state.polling = true;
        let recover = !self.automation_state.recovered;
        let cutoff = self.automation_state.started_before;
        let work = cx.background_executor().spawn(async move {
            let mut service = service_lock(&service)?;
            let mut runs = if recover {
                service.recover(cutoff, now())?
            } else {
                Vec::new()
            };
            runs.extend(service.claim_due(now())?);
            let has_events = service.list()?.iter().any(|automation| {
                automation.enabled
                    && automation.triggers.as_ref().is_some_and(|triggers| {
                        triggers.iter().any(|trigger| trigger.kind != "time")
                    })
            });
            Ok::<_, String>((runs, has_events))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.automation_state.polling = false;
                match result {
                    Ok((runs, has_events)) => {
                        this.automation_state.recovered = true;
                        for run in runs {
                            this.launch_automation(run, false, cx);
                        }
                        if has_events {
                            this.ensure_background_inbox(window, cx);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "automation schedule poll failed");
                        this.error = Some(format!("Could not evaluate automations: {error}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Deliveries wait, merged by item, while Hold is on or a claim is in
    /// flight: an item without a creation time is offered only once.
    pub(super) fn inbox_automation_events(
        &mut self,
        items: Vec<gespan_core::inbox::Item>,
        cx: &mut Context<Self>,
    ) {
        let waiting = &mut self.automation_state.waiting_events;
        for item in items {
            waiting.insert(item.key(), item);
        }
        if waiting.len() > 500 {
            let mut items: Vec<_> = std::mem::take(waiting).into_values().collect();
            items.sort_by_key(|item| std::cmp::Reverse(item.updated_ms()));
            items.truncate(500);
            *waiting = items.into_iter().map(|item| (item.key(), item)).collect();
        }
        self.claim_inbox_events(cx);
    }

    /// The GitHub and GitLab items the Inbox lists now. Something new on one
    /// of them can start a run; listing them again starts nothing else.
    pub(super) fn inbox_listed(&mut self, listed: Vec<gespan_core::inbox::Item>) {
        self.automation_state.listed = listed;
    }

    /// How many listed items still wait to be looked at, and how many pull
    /// requests have had their comments read.
    #[cfg(test)]
    pub(super) fn listed_watch_for_test(&self) -> (usize, usize) {
        (
            self.automation_state.listed.len(),
            self.automation_state.comments_read.len(),
        )
    }

    fn claim_inbox_events(&mut self, cx: &mut Context<Self>) {
        // Items stay waiting while held and are claimed on Resume.
        if self.automation_state.events_polling || crate::hold::active(cx) {
            return;
        }
        let Some(service) = self.automation_state.service.clone() else {
            return;
        };
        let mut waiting = std::mem::take(&mut self.automation_state.waiting_events);
        for (key, item) in &self.automation_state.event_retries {
            waiting.entry(key.clone()).or_insert_with(|| item.clone());
        }
        let items: Vec<_> = waiting.into_values().collect();
        // Looked at once per refresh; a failed read waits for the next one.
        let listed = std::mem::take(&mut self.automation_state.listed);
        if items.is_empty() && listed.is_empty() {
            return;
        }
        self.automation_state.events_polling = true;
        let attempted = items.clone();
        let read = self.automation_state.comments_read.clone();
        let store = self.store.clone();
        let cwd = self.workspace.project.clone();
        let work = cx.background_executor().spawn(async move {
            // The providers are asked before the service is locked for claiming.
            let watches = automations::event_watches(&mut *service_lock(&service)?, now())?;
            let watched = if watches.is_empty() || listed.is_empty() {
                Default::default()
            } else {
                let data_dir = locked(&store)?
                    .data_dir()
                    .map(std::path::Path::to_path_buf)
                    .ok_or("Gespan has no data folder")?;
                let mut source =
                    gespan_core::inbox::events::LiveEvents::new(gespan_core::inbox::Service {
                        data_dir,
                        cwd: cwd.into(),
                    });
                gespan_core::inbox::events::watch_events(&watches, &listed, &read, &mut source)
            };
            for error in &watched.errors {
                tracing::warn!(%error, "an automation event was not looked up");
            }
            let mut items = items;
            items.extend(watched.items);
            let read = watched.read;
            let mut service = service_lock(&service)?;
            automations::claim_inbox_events_retry(&mut service, &items, now())
                .map(|(runs, retry)| (runs, retry, read))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.automation_state.events_polling = false;
                match result {
                    Ok((runs, retry, read)) => {
                        this.automation_state.comments_read.extend(read);
                        this.set_event_retries(retry, cx);
                        for run in runs {
                            this.launch_automation(run, false, cx);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "automation inbox poll failed");
                        this.error = Some(format!("Could not evaluate Inbox automations: {error}"));
                        // Kept across a restart, like a failed claim.
                        this.set_event_retries(attempted, cx);
                        cx.notify();
                        return;
                    }
                }
                if !this.automation_state.waiting_events.is_empty()
                    || !this.automation_state.listed.is_empty()
                {
                    this.claim_inbox_events(cx);
                }
            });
        })
        .detach();
    }

    /// Items whose claim failed are kept across restarts: one without a
    /// creation time is offered by the Inbox only once.
    fn set_event_retries(
        &mut self,
        mut retry: Vec<gespan_core::inbox::Item>,
        cx: &mut Context<Self>,
    ) {
        retry.sort_by_key(|item| std::cmp::Reverse(item.updated_ms()));
        retry.truncate(500);
        let retries: HashMap<_, _> = retry.into_iter().map(|item| (item.key(), item)).collect();
        if retries.len() == self.automation_state.event_retries.len()
            && retries
                .keys()
                .all(|key| self.automation_state.event_retries.contains_key(key))
        {
            return;
        }
        self.automation_state.event_retries = retries;
        let pending = self
            .automation_state
            .event_retries
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let store = self.store.clone();
        let revision = self
            .automation_state
            .retry_revision
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let generation = self.automation_state.retry_revision.clone();
        cx.background_executor()
            .spawn(async move {
                if let Ok(saved) = serde_json::to_string(&pending) {
                    let _ = locked(&store).and_then(|store| {
                        if generation.load(Ordering::SeqCst) == revision {
                            store.save_setting("native-automation-inbox-retries", &saved)
                        } else {
                            Ok(())
                        }
                    });
                }
            })
            .detach();
    }

    pub(super) fn launch_automation(
        &mut self,
        due: DueAutomationRun,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(error) = review_pilot_block(&due.automation) {
            tracing::warn!(
                automation = %due.run.automation_id,
                run = %due.run.id,
                "review automation set to wake a Pilot was not run"
            );
            self.update_automation_run(due.run.id.clone(), "failed", None, Some(error.into()), cx);
            if reveal {
                self.error = Some(error.into());
                cx.notify();
            }
            return;
        }
        if let AutomationAction::Pilot { pilot_id } = &due.automation.action {
            let pilot_id = pilot_id.clone();
            self.wake_pilot(due, &pilot_id, reveal, cx);
            return;
        }
        if self.automation_state.pending.contains(&due.run.id)
            || self
                .automation_state
                .running
                .values()
                .any(|run| run.run_id == due.run.id)
        {
            return;
        }
        self.automation_state.pending.insert(due.run.id.clone());
        let automation = &due.automation;
        let kind = Kind::from_id(&automation.harness);
        let reuse = automation
            .reuse_session
            .then_some(automation.last_session_id.as_deref())
            .flatten()
            .and_then(|id| locked(&self.store).ok()?.resolve_existing_session_id(id))
            .filter(|id| {
                !self.automation_state.running.contains_key(id)
                    && !self.automation_state.reserved.contains(id)
            })
            .filter(|id| {
                self.session_summaries.get(id).is_some_and(|summary| {
                    summary.provider == automation.harness
                        && !summary.archived
                        && !self.draft_sessions.contains(id)
                        && self
                            .sessions
                            .get(id)
                            .is_none_or(|view| view.read(cx).available_for_managed_run(cx))
                        && match automation.workspace_mode.as_str() {
                            "current" => {
                                self.session_worktrees
                                    .get(id)
                                    .and_then(Option::as_deref)
                                    .is_none()
                                    && gespan_core::files::path_key(&summary.cwd)
                                        == gespan_core::files::path_key(&automation.cwd)
                            }
                            "existing" => self
                                .session_worktrees
                                .get(id)
                                .and_then(Option::as_deref)
                                .is_some_and(|cwd| {
                                    gespan_core::files::path_key(cwd)
                                        == gespan_core::files::path_key(&automation.worktree_cwd)
                                }),
                            _ => false,
                        }
                })
            });
        if let Some(id) = reuse {
            self.ensure_session(id, cx);
        }
        if let Some(id) = reuse {
            self.automation_state.reserved.insert(id);
        }
        let settings = reuse
            .and_then(|id| {
                self.sessions
                    .get(&id)
                    .map(|view| view.read(cx).settings.clone())
            })
            .unwrap_or_default();
        let mut settings = managed_settings(settings, kind, &automation.cwd, &automation.model, cx);
        settings.model_settings = automation.model_settings.clone();
        settings.effort = [
            "effort",
            "reasoning",
            "reasoningEffort",
            "reasoning_effort",
            "variant",
        ]
        .into_iter()
        .find_map(|key| automation.model_settings.get(key).cloned());
        settings.fast = automation
            .model_settings
            .get("fast")
            .is_some_and(|value| value == "true");
        settings.service_tier = automation
            .model_settings
            .get("serviceTier")
            .or_else(|| automation.model_settings.get("service_tier"))
            .cloned();
        settings.thinking = automation
            .model_settings
            .get("thinking")
            .and_then(|value| value.parse().ok());
        settings.review = review_launch(automation, &due.run);
        settings.access = launch_access(automation, settings.review.as_ref());
        WorktreeChoice::from_mode(&automation.workspace_mode, &automation.worktree_cwd)
            .apply(&mut settings);
        let store = self.store.clone();
        let launch = due.clone();
        let revision = self.navigation_revision;
        let work = cx.background_executor().spawn(async move {
            let automation = &launch.automation;
            launch_available(
                "automation",
                &automation.harness,
                &automation.cwd,
                (automation.workspace_mode == "existing")
                    .then_some(automation.worktree_cwd.as_str()),
            )?;
            let prompt = launch
                .run
                .prompt
                .clone()
                .unwrap_or_else(|| automation.prompt.clone());
            let prompt = match settings.review.as_ref() {
                Some(policy) => {
                    let context = match policy.target.as_ref() {
                        Some(target) if gespan_core::review_policy::prefetches(kind, target) => {
                            let data_dir = locked(&store)?
                                .data_dir()
                                .map(std::path::Path::to_path_buf)
                                .ok_or("Gespan has no data folder")?;
                            Some(gespan_core::review_policy::prefetch_context(
                                std::path::Path::new(&automation.cwd),
                                &data_dir,
                                target,
                            )?)
                        }
                        _ => None,
                    };
                    review_prompt(&prompt, policy, context.as_deref())
                }
                None => prompt,
            };
            let store = locked(&store)?;
            let mut settings = settings;
            let mut summary = if let Some(id) = reuse {
                if let Some(previous) = store.agent_settings(id)? {
                    settings.turns = previous.turns;
                    settings.revision = previous.revision;
                }
                store
                    .sessions()?
                    .into_iter()
                    .find(|session| session.id == id)
                    .ok_or("The reusable session no longer exists")?
            } else {
                let mut summary =
                    store.create(&launch.automation.harness, &launch.automation.cwd)?;
                if launch.run.trigger != "event" {
                    summary.title = launch.automation.name.clone();
                    store.title(summary.id, &summary.title)?;
                }
                summary
            };
            store.set_automation_id(summary.id, &launch.automation.id)?;
            if let Some(item) = automation_work_item(&launch.run) {
                store.set_work_item(summary.id, Some(&item))?;
                summary.work_item = Some(item);
            }
            if reuse.is_none() {
                store.save_agent_settings(summary.id, &settings)?;
            }
            let persisted = store.persisted_session_id(summary.id)?;
            Ok::<_, String>((summary, settings, persisted, prompt))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.automation_state.pending.remove(&due.run.id);
                if let Some(id) = reuse {
                    this.automation_state.reserved.remove(&id);
                }
                match result {
                    Ok((summary, settings, persisted, prompt)) => {
                        let id = summary.id;
                        if reuse.is_some()
                            && this
                                .sessions
                                .get(&id)
                                .is_none_or(|view| !view.read(cx).available_for_managed_run(cx))
                        {
                            let mut retry = due.clone();
                            retry.automation.reuse_session = false;
                            this.launch_automation(retry, reveal, cx);
                            return;
                        }
                        let linked = summary.work_item.clone();
                        let alert = (!reveal)
                            .then(|| event_alert_body(&due.run))
                            .flatten()
                            .map(|body| (summary.clone(), body));
                        this.add(summary, cx);
                        if let Some(linked) = linked {
                            this.sessions[&id]
                                .update(cx, |view, _| view.summary.work_item = Some(linked));
                        }
                        this.automation_state
                            .session_automations
                            .insert(id, due.automation.id.clone());
                        this.notify_sidebar(cx);
                        this.automation_state.running.insert(
                            id,
                            Running {
                                run_id: due.run.id.clone(),
                                submitting: true,
                                generation_before: this.sessions[&id]
                                    .read(cx)
                                    .submission_generation,
                                review: settings.review.clone(),
                                automation_id: due.automation.id.clone(),
                                cwd: due.automation.cwd.clone(),
                                title: due
                                    .run
                                    .event_title
                                    .clone()
                                    .unwrap_or_else(|| due.automation.name.clone()),
                            },
                        );
                        this.update_automation_run(
                            due.run.id.clone(),
                            "running",
                            Some(persisted),
                            None,
                            cx,
                        );
                        if !due.automation.session_folder_id.is_empty() {
                            this.groups
                                .entry(due.automation.cwd.clone())
                                .or_default()
                                .add_to_folder(&due.automation.session_folder_id, id);
                            this.persist_automation_folder(&due.automation.cwd, cx);
                        }
                        this.submit_in_background(id, prompt, settings, cx);
                        if let Some(run) = this.automation_state.running.get_mut(&id) {
                            run.submitting = false;
                        }
                        this.automation_session_changed(id, cx);
                        if let Some((summary, body)) = alert {
                            this.deliver_automation_notice(
                                &summary,
                                &due.automation.name,
                                &body,
                                cx,
                            );
                        }
                        if reveal && this.navigation_revision == revision {
                            this.activate_session(id, window, cx);
                        }
                        this.persist(cx);
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            automation = %due.run.automation_id,
                            run = %due.run.id,
                            "automation run failed to launch"
                        );
                        this.update_automation_run(
                            due.run.id.clone(),
                            "failed",
                            None,
                            Some(error.clone()),
                            cx,
                        );
                        if reveal {
                            this.error = Some(error);
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }

    /// Delivers a run to its Pilot: the prompt is queued on the Pilot's chat,
    /// where it waits while the Pilot is busy. The run is done once queued.
    fn wake_pilot(
        &mut self,
        due: DueAutomationRun,
        pilot_id: &str,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        let text = pilot_message(&due);
        let sent = match self.pilots.clone() {
            Some(pilots) => pilots.update(cx, |pilots, cx| {
                let message = pilots.automation_send(pilot_id, text, cx)?;
                Ok((message, pilots.automation_target(pilot_id, cx)))
            }),
            None => Err(format!("Pilot {pilot_id} is no longer available")),
        };
        match sent {
            Ok((message, target)) => {
                let run_id = due.run.id.clone();
                self.write_automation_run(
                    move |service| service.deliver_run(&run_id, &message, now()),
                    cx,
                );
                if !reveal
                    && let Some(body) = event_alert_body(&due.run)
                    && let Some((pilot, Some(chat))) = target
                {
                    self.deliver_pilot_automation_notice(&chat, &due.automation, &pilot, &body, cx);
                }
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    automation = %due.run.automation_id,
                    run = %due.run.id,
                    "automation run was not delivered to its Pilot"
                );
                self.update_automation_run(
                    due.run.id.clone(),
                    "failed",
                    None,
                    Some(error.clone()),
                    cx,
                );
                if reveal {
                    self.error = Some(error);
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn automation_session_changed(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(run) = self.automation_state.running.get(&id) else {
            return;
        };
        if run.submitting {
            return;
        }
        let Some(view) = self.sessions.get(&id) else {
            return;
        };
        let view = view.read(cx);
        if view.external_submission_pending() {
            return;
        }
        let cancelled = view.notification_was_cancelled();
        let (mut ok, mut message) = view.external_outcome(cx);
        if view.submission_generation == run.generation_before {
            ok = false;
            message = "The selected agent session could not start this run.".into();
        }
        let status = if cancelled {
            "cancelled"
        } else if ok {
            "succeeded"
        } else {
            "failed"
        };
        let run = self.automation_state.running.remove(&id).unwrap();
        // A finished pending-review run is not done until its findings are
        // on the pull or merge request: Gespan leaves them there itself.
        if status == "succeeded"
            && let Some(target) = pending_review_target(run.review.as_ref())
        {
            let reply = view.last_reply(cx);
            self.leave_draft_review(run, target, reply, cx);
            return;
        }
        self.update_automation_run(
            run.run_id,
            status,
            None,
            (!ok && !cancelled).then_some(message),
            cx,
        );
    }

    pub(super) fn finish_automation_run(
        &mut self,
        run_id: String,
        status: &str,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.update_automation_run(run_id, status, None, error, cx);
    }

    fn update_automation_run(
        &mut self,
        run_id: String,
        status: &str,
        session: Option<String>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let status = status.to_string();
        self.write_automation_run(
            move |service| service.update_run(&run_id, &status, session, error, now()),
            cx,
        );
    }

    /// Saves a run's result after the writes before it, then refreshes the
    /// views that list runs.
    fn write_automation_run(
        &mut self,
        write: impl FnOnce(&mut Service) -> Result<AutomationRun, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.automation_state.service.clone() else {
            return;
        };
        let previous = self.automation_state.writes.take();
        self.automation_state.writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { write(&mut *service_lock(&service)?) });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match &result {
                    Err(error) => {
                        tracing::warn!(%error, "automation run result was not saved");
                        this.error = Some(format!("Could not save automation result: {error}"));
                    }
                    // A run the Inbox started from an item follows its record.
                    Ok(run) => {
                        if let Some(inbox) = &this.inbox {
                            inbox.update(cx, |inbox, cx| inbox.run_updated(run, cx));
                        }
                    }
                }
                this.refresh_run_views(cx);
                cx.notify();
            });
        }));
    }

    /// Background runs append a tab without changing the focused project, then
    /// submit their prompt.
    pub(super) fn submit_in_background(
        &mut self,
        id: i64,
        prompt: String,
        settings: gespan_core::agent::Settings,
        cx: &mut Context<Self>,
    ) {
        self.add_run_tab(id);
        self.sessions[&id].update(cx, |view, cx| view.quick_submit(prompt, settings, cx));
    }

    /// Gives a run's chat a tab without changing the focused project. Not
    /// opened by you, it shows only while the run works.
    pub(super) fn add_run_tab(&mut self, id: i64) {
        if !self.workspace.tabs.contains(&id) {
            self.workspace.tabs.push(id);
            self.workspace.title_order.push(TitleTab::Session(id));
            self.workspace.background_tabs.insert(id);
        }
    }

    fn persist_automation_folder(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let cwd = cwd.to_string();
        let groups = self.groups.get(&cwd).cloned().unwrap_or_default();
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { locked(&store)?.save_groups(&cwd, &groups) });
            if let Err(error) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(error);
                    cx.notify();
                });
            }
        }));
    }
}

/// Where a background run works: a new worktree from `HEAD`, an existing
/// worktree, or the project folder itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorktreeChoice {
    New,
    Reuse(String),
    Current,
}

impl WorktreeChoice {
    /// An automation's `workspace_mode` and saved `worktree_cwd`.
    fn from_mode(mode: &str, worktree_cwd: &str) -> Self {
        match mode {
            "worktree" => Self::New,
            "existing" => Self::Reuse(worktree_cwd.into()),
            _ => Self::Current,
        }
    }

    pub(super) fn apply(&self, settings: &mut gespan_core::agent::Settings) {
        settings.new_worktree = *self == Self::New;
        settings.worktree_base = (*self == Self::New).then(|| "HEAD".into());
        settings.worktree_cwd = match self {
            Self::Reuse(path) => Some(path.clone()),
            Self::New | Self::Current => None,
        };
    }
}

/// A background run's settings: the harness's selected account and native
/// model, with no composer mode or prepared prompt.
pub(super) fn managed_settings(
    mut settings: gespan_core::agent::Settings,
    kind: Kind,
    cwd: &str,
    model: &str,
    cx: &App,
) -> gespan_core::agent::Settings {
    settings.account = crate::accounts::get(cx)
        .map(|accounts| accounts.read(cx).selected(kind, cwd))
        .unwrap_or_default();
    settings.model = gespan_core::store::native_model(model, kind.id());
    settings.plan = false;
    settings.operator = false;
    settings.orchestrator = false;
    settings.prepared_prompt = None;
    settings
}

/// Fails when a background run's agent, project folder or worktree is gone;
/// `owner` names the run in the message ("The automation's agent ...").
pub(super) fn launch_available(
    owner: &str,
    harness: &str,
    cwd: &str,
    worktree: Option<&str>,
) -> Result<(), String> {
    if Kind::parse(harness).is_none() {
        return Err(format!("The {owner}'s agent is unavailable"));
    }
    if !std::path::Path::new(cwd).is_dir() {
        return Err(format!("The {owner}'s project folder is unavailable"));
    }
    if worktree.is_some_and(|path| !std::path::Path::new(path).is_dir()) {
        return Err(format!("The {owner}'s worktree is unavailable"));
    }
    Ok(())
}

/// What a run started by an Inbox event says in its alert, such as
/// `Issue opened: acme/app#12 Fix login`. Scheduled and hand-started runs
/// raise none.
fn event_alert_body(run: &gespan_core::automations::AutomationRun) -> Option<String> {
    if run.trigger != "event" {
        return None;
    }
    let label = gespan_core::automations::event_label(
        run.event_kind.as_deref().unwrap_or_default(),
        run.event.as_deref().unwrap_or_default(),
    );
    Some(match run.event_title.as_deref().map(str::trim) {
        Some(title) if !title.is_empty() => format!("{label}: {title}"),
        _ => label,
    })
}

/// What a Pilot is sent for a run: which automation and run it is, what
/// started it, then the run's prompt as it would be given to a session.
fn pilot_message(due: &DueAutomationRun) -> String {
    let run = &due.run;
    let trigger = match run.trigger.as_str() {
        "event" => automations::event_label(
            run.event_kind.as_deref().unwrap_or_default(),
            run.event.as_deref().unwrap_or_default(),
        ),
        "manual" => "Manual".into(),
        _ => "Scheduled".into(),
    };
    let name = due.automation.name.replace(['\n', '\r'], " ");
    let prompt = run.prompt.as_deref().unwrap_or(&due.automation.prompt);
    format!(
        "[Automation: {name}]\nTrigger: {trigger}  ·  Run: {}\n{prompt}",
        run.id
    )
}

/// The policy a review automation's run carries, or `None` when the
/// automation does not review. Results the harness cannot honour are
/// clamped to keeping them in Gespan, so the run is told not to post.
fn review_launch(
    automation: &automations::Automation,
    run: &AutomationRun,
) -> Option<gespan_core::review_policy::ReviewPolicy> {
    use gespan_core::review_policy::{ReviewPolicy, ReviewTarget, effective_results};
    let stored = automation.review_results?;
    // The pull or merge request comes from the event Gespan claimed, never
    // from anything the agent says.
    let target = run
        .event_key
        .as_deref()
        .and_then(ReviewTarget::from_event_key);
    Some(ReviewPolicy {
        results: effective_results(Kind::from_id(&automation.harness), stored, target.as_ref()),
        target,
    })
}

/// Where a finished run leaves its draft review: only a pending-review run
/// with a pull or merge request to leave it on.
pub(super) fn pending_review_target(
    policy: Option<&gespan_core::review_policy::ReviewPolicy>,
) -> Option<gespan_core::review_policy::ReviewTarget> {
    let policy = policy?;
    (policy.results == gespan_core::review_policy::ReviewResults::Pending)
        .then(|| policy.target.clone())
        .flatten()
}

/// Why a run cannot be delivered, for a review automation saved with a Pilot
/// action before saving refused that: a Pilot would not carry the policy.
fn review_pilot_block(automation: &automations::Automation) -> Option<&'static str> {
    (automation.review_results.is_some()
        && matches!(automation.action, AutomationAction::Pilot { .. }))
    .then_some(automations::REVIEW_PILOT_ERROR)
}

/// A review run is read-only whatever run mode the automation was edited to.
fn launch_access(
    automation: &automations::Automation,
    review: Option<&gespan_core::review_policy::ReviewPolicy>,
) -> gespan_core::agent::AccessMode {
    if review.is_some() {
        gespan_core::agent::AccessMode::ReadOnly
    } else {
        gespan_core::agent::AccessMode::from_id(&automation.runtime_mode)
    }
}

/// The prompt a review run is given: any prefetched pull request first, the
/// policy's instruction last.
fn review_prompt(
    prompt: &str,
    policy: &gespan_core::review_policy::ReviewPolicy,
    context: Option<&str>,
) -> String {
    let instruction = gespan_core::review_policy::instruction(policy);
    match context {
        Some(context) => format!("{context}\n\n{prompt}\n\n{instruction}"),
        None => format!("{prompt}\n\n{instruction}"),
    }
}

fn automation_work_item(
    run: &gespan_core::automations::AutomationRun,
) -> Option<gespan_core::session_groups::WorkItem> {
    if run.trigger != "event" || run.event_kind.as_deref() != Some("github") {
        return None;
    }
    let mut parts = run.event_key.as_deref()?.split(':');
    if parts.next()? != "github" {
        return None;
    }
    let kind = match parts.next()? {
        "pr" => "pull",
        "issue" => "issues",
        _ => return None,
    };
    let repo = parts.next()?;
    let number = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    gespan_core::session_groups::parse_github_work_item(&format!(
        "https://github.com/{repo}/{kind}/{number}"
    ))
    .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_run(event_key: Option<&str>) -> gespan_core::automations::AutomationRun {
        gespan_core::automations::AutomationRun {
            id: "run".into(),
            automation_id: "auto".into(),
            trigger: "event".into(),
            scheduled_for: 0,
            created_at: 0,
            started_at: None,
            completed_at: None,
            status: "pending".into(),
            session_id: None,
            error: None,
            event_key: event_key.map(Into::into),
            event_kind: None,
            event: None,
            event_title: None,
            prompt: None,
            pilot_message_id: None,
        }
    }

    fn review_automation(
        harness: &str,
        results: Option<gespan_core::review_policy::ReviewResults>,
    ) -> gespan_core::automations::Automation {
        let mut automation: gespan_core::automations::Automation =
            serde_json::from_value(serde_json::json!({
                "id": "auto", "name": "Review", "prompt": "Review it",
                "harness": harness, "model": "model", "cwd": "/repo",
                "workspaceMode": "current", "reuseSession": false,
                "runtimeMode": "full-access", "scheduleKind": "manual",
                "minute": 0, "time": "09:00", "dayOfWeek": 1,
                "missedRunGraceMinutes": 0, "enabled": true,
                "nextRunAt": 0, "createdAt": 0, "updatedAt": 0
            }))
            .unwrap();
        automation.review_results = results;
        automation
    }

    #[test]
    fn review_runs_carry_policy_and_instruction() {
        use gespan_core::review_policy::{ReviewResults, ReviewTarget};
        let key = "github:pull_request_review_requested:acme/app:7:abc";
        let target = Some(ReviewTarget {
            forge: gespan_core::review_policy::Forge::GitHub,
            repo: "acme/app".into(),
            number: 7,
        });

        // Not a review automation.
        let plain = review_automation("claude", None);
        assert_eq!(review_launch(&plain, &review_run(Some(key))), None);

        let draft = review_automation("claude", Some(ReviewResults::Draft));
        let policy = review_launch(&draft, &review_run(Some(key))).unwrap();
        assert_eq!(policy.results, ReviewResults::Draft);
        assert_eq!(policy.target, target);

        // Run now has no event key, so no target.
        let policy = review_launch(&draft, &review_run(None)).unwrap();
        assert_eq!(policy.results, ReviewResults::Draft);
        assert_eq!(policy.target, None);

        // A key that is not a pull request has no target either.
        let policy = review_launch(&draft, &review_run(Some("linear:issue:lin-1"))).unwrap();
        assert_eq!(policy.target, None);
    }

    #[test]
    fn only_a_pending_review_run_with_a_target_leaves_a_draft() {
        use gespan_core::review_policy::{ReviewPolicy, ReviewResults, ReviewTarget};
        let key = "gitlab:merge_request_review_command:group/sub/app:12:n1";
        let pending = review_automation("claude", Some(ReviewResults::Pending));
        let policy = review_launch(&pending, &review_run(Some(key))).unwrap();
        assert_eq!(policy.results, ReviewResults::Pending);
        assert_eq!(
            pending_review_target(Some(&policy)),
            ReviewTarget::from_event_key(key)
        );
        // Without the event's pull request there is nowhere to leave it.
        let untargeted = review_launch(&pending, &review_run(None)).unwrap();
        assert_eq!(pending_review_target(Some(&untargeted)), None);
        for results in [
            ReviewResults::Gespan,
            ReviewResults::Draft,
            ReviewResults::Post,
        ] {
            let other = ReviewPolicy {
                results,
                target: ReviewTarget::from_event_key(key),
            };
            assert_eq!(pending_review_target(Some(&other)), None, "{results:?}");
        }
        assert_eq!(pending_review_target(None), None);
        // Posting is clamped on GitLab, where only `gh` could post.
        let post = review_automation("claude", Some(ReviewResults::Post));
        assert_eq!(
            review_launch(&post, &review_run(Some(key)))
                .unwrap()
                .results,
            ReviewResults::Gespan
        );
    }

    #[test]
    fn review_runs_carry_results_the_harness_cannot_honour_clamp_to_gespan() {
        use gespan_core::review_policy::ReviewResults;
        let key = "github:pull_request_review_requested:acme/app:7:abc";
        for results in [ReviewResults::Draft, ReviewResults::Post] {
            let pi = review_automation("pi", Some(results));
            let policy = review_launch(&pi, &review_run(Some(key))).unwrap();
            assert_eq!(policy.results, ReviewResults::Gespan, "{results:?}");
        }
        let claude = review_automation("claude", Some(ReviewResults::Post));
        assert_eq!(
            review_launch(&claude, &review_run(Some(key)))
                .unwrap()
                .results,
            ReviewResults::Post
        );
    }

    #[test]
    fn review_runs_carry_prompt_with_context_first_and_instruction_last() {
        let policy = gespan_core::review_policy::ReviewPolicy {
            results: gespan_core::review_policy::ReviewResults::Gespan,
            target: None,
        };
        let instruction = gespan_core::review_policy::instruction(&policy);
        assert_eq!(
            review_prompt("Review it", &policy, Some("CONTEXT")),
            format!("CONTEXT\n\nReview it\n\n{instruction}")
        );
        assert_eq!(
            review_prompt("Review it", &policy, None),
            format!("Review it\n\n{instruction}")
        );
    }

    #[test]
    fn review_runs_carry_a_pilot_block_for_rows_saved_before_validation() {
        use gespan_core::review_policy::ReviewResults;
        let mut automation = review_automation("claude", Some(ReviewResults::Post));
        automation.action = AutomationAction::Pilot {
            pilot_id: "pilot-1".into(),
        };
        assert_eq!(
            review_pilot_block(&automation),
            Some("A pull request review automation can't wake a Pilot.")
        );
        automation.review_results = None;
        assert_eq!(review_pilot_block(&automation), None);
        let session = review_automation("claude", Some(ReviewResults::Post));
        assert_eq!(review_pilot_block(&session), None);
    }

    #[test]
    fn review_runs_carry_read_only_access_whatever_the_run_mode() {
        use gespan_core::agent::AccessMode;
        use gespan_core::review_policy::ReviewResults;
        let mut review = review_automation("claude", Some(ReviewResults::Gespan));
        review.runtime_mode = "full-access".into();
        let policy = review_launch(&review, &review_run(None));
        assert_eq!(
            launch_access(&review, policy.as_ref()),
            AccessMode::ReadOnly
        );
        let mut plain = review_automation("claude", None);
        plain.runtime_mode = "full-access".into();
        assert_eq!(
            launch_access(&plain, None),
            AccessMode::from_id("full-access")
        );
        assert_ne!(AccessMode::from_id("full-access"), AccessMode::ReadOnly);
    }

    #[test]
    fn only_event_runs_have_an_alert_and_older_runs_fall_back_to_the_event() {
        let run = |trigger: &str, title: Option<&str>| gespan_core::automations::AutomationRun {
            id: "run".into(),
            automation_id: "auto".into(),
            trigger: trigger.into(),
            scheduled_for: 0,
            created_at: 0,
            started_at: None,
            completed_at: None,
            status: "pending".into(),
            session_id: None,
            error: None,
            event_key: Some("linear:issue:lin-1".into()),
            event_kind: Some("linear".into()),
            event: Some("issue_created".into()),
            event_title: title.map(Into::into),
            prompt: None,
            pilot_message_id: None,
        };
        assert_eq!(
            event_alert_body(&run("event", Some("ENG-1 Crash"))).as_deref(),
            Some("Issue created: ENG-1 Crash")
        );
        // A run claimed before titles were kept, or with a blank one.
        assert_eq!(
            event_alert_body(&run("event", None)).as_deref(),
            Some("Issue created")
        );
        assert_eq!(
            event_alert_body(&run("event", Some("  "))).as_deref(),
            Some("Issue created")
        );
        for trigger in ["scheduled", "manual"] {
            assert_eq!(event_alert_body(&run(trigger, Some("ENG-1 Crash"))), None);
        }
    }

    #[test]
    fn automation_workspace_modes_keep_their_worktree_settings() {
        // The values the inline launch code set before the shared helper.
        let cases = [
            ("worktree", true, Some("HEAD"), None),
            ("existing", false, None, Some("/repo/.worktrees/a")),
            ("current", false, None, None),
            ("", false, None, None),
        ];
        for (mode, new_worktree, base, cwd) in cases {
            let mut settings = gespan_core::agent::Settings {
                new_worktree: !new_worktree,
                worktree_base: Some("stale".into()),
                worktree_cwd: Some("/stale".into()),
                ..Default::default()
            };
            WorktreeChoice::from_mode(mode, "/repo/.worktrees/a").apply(&mut settings);
            assert_eq!(settings.new_worktree, new_worktree, "{mode}");
            assert_eq!(settings.worktree_base.as_deref(), base, "{mode}");
            assert_eq!(settings.worktree_cwd.as_deref(), cwd, "{mode}");
        }
    }

    #[test]
    fn launch_checks_keep_the_automation_messages_and_order() {
        let here = env!("CARGO_MANIFEST_DIR");
        assert_eq!(
            launch_available("automation", "nope", "/missing", Some("/missing")),
            Err("The automation's agent is unavailable".into())
        );
        assert_eq!(
            launch_available("automation", "codex", "/missing", Some("/missing")),
            Err("The automation's project folder is unavailable".into())
        );
        assert_eq!(
            launch_available("automation", "codex", here, Some("/missing")),
            Err("The automation's worktree is unavailable".into())
        );
        assert_eq!(
            launch_available("automation", "codex", here, Some(here)),
            Ok(())
        );
        // Only the "existing" mode passes a worktree to check.
        assert_eq!(launch_available("automation", "codex", here, None), Ok(()));
        assert_eq!(
            launch_available("Flow", "codex", "/missing", None),
            Err("The Flow's project folder is unavailable".into())
        );
    }
}
