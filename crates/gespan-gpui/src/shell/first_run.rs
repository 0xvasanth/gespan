//! The first-run setup card over the shell. It opens whenever the workspace
//! has setup pending, sits above every other modal, and closes only when a
//! project is picked: Esc, the scrim and the shell's shortcuts do nothing.
use super::*;
use crate::setup::{
    SetupStep,
    card::{SetupCard, SetupEvent},
    rules,
};

pub(super) struct SetupOverlay {
    pub(super) view: Entity<SetupCard>,
    _subscriptions: Vec<Subscription>,
    /// The project open when "Folder on a machine…" was asked for; a change
    /// from it is the machine folder picked.
    remote_from: Option<String>,
    /// Only setup on a fresh install drops the launch folder it didn't pick;
    /// Run setup again leaves every project alone.
    drop_launch: bool,
}

fn home() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default()
}

impl Shell {
    /// Whether this window shows setup. Only the first window does; others
    /// wait for it.
    pub(super) fn setup_shows(&self) -> bool {
        self.workspace.setup.is_some() && !self.secondary
    }

    /// Opens setup on `step`, from a fresh install or Settings › General.
    pub(super) fn open_setup(
        &mut self,
        step: SetupStep,
        drop_launch: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefs = self.workspace.preferences.clone();
        let launch = self.setup_launch.clone();
        let view = cx.new(|cx| SetupCard::new(step, prefs, launch, home(), cx));
        let mut subscriptions = vec![cx.subscribe_in(&view, window, Self::on_setup_event)];
        subscriptions.push(
            cx.observe_global::<crate::installed_providers::InstalledProviders>(|this, cx| {
                if let Some(setup) = &this.setup {
                    setup.view.update(cx, |card, cx| card.refresh_agents(cx));
                }
            }),
        );
        if let Some(accounts) = crate::accounts::get(cx) {
            subscriptions.push(cx.subscribe(&accounts, |this, _, event, cx| {
                use crate::accounts::AccountEvent;
                if matches!(
                    event,
                    AccountEvent::Authenticated(..) | AccountEvent::Updated
                ) && let Some(setup) = &this.setup
                {
                    setup.view.update(cx, |card, cx| card.sign_in_ended(cx));
                }
            }));
        }
        window.focus(&view.focus_handle(cx), cx);
        self.setup = Some(SetupOverlay {
            view,
            _subscriptions: subscriptions,
            remote_from: None,
            drop_launch,
        });
        if self.workspace.setup != Some(step) {
            self.workspace.setup = Some(step);
            self.save_state(cx);
        }
        cx.notify();
    }

    /// The command palette's Run setup again: the card from Welcome, with
    /// today's choices filled in. Projects stay as they are.
    pub(super) fn run_setup_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            self.close_settings(window, cx);
        }
        self.setup = None;
        self.open_setup(SetupStep::Welcome, false, window, cx);
    }

    fn on_setup_event(
        &mut self,
        view: &Entity<SetupCard>,
        event: &SetupEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SetupEvent::Step(step) => {
                self.workspace.setup = Some(*step);
                // Leaving the agents card without a pick keeps a sensible default.
                if *step != SetupStep::Agents
                    && self.workspace.preferences.last_used_provider().is_none()
                {
                    let card = view.read(cx);
                    let installed: Vec<Kind> = card.agents().iter().map(|a| a.kind).collect();
                    let signed_in = |kind: Kind| {
                        card.agents()
                            .iter()
                            .find(|a| a.kind == kind)
                            .and_then(|a| a.signed_in)
                    };
                    if let Some(kind) = rules::skip_default(&installed, signed_in) {
                        self.workspace.preferences.use_by_default(kind);
                    }
                }
                self.save_state(cx);
            }
            SetupEvent::Preferences(prefs) => {
                self.set_preferences(*prefs.clone(), window, cx);
                window.focus(&view.focus_handle(cx), cx);
            }
            SetupEvent::DefaultAgent(kind) => {
                self.workspace.preferences.use_by_default(*kind);
                self.save_state(cx);
            }
            SetupEvent::SignIn(kind) => {
                let project = self.workspace.project.clone();
                // Tests never start a real provider login.
                #[cfg(not(test))]
                if let Some(accounts) = crate::accounts::get(cx) {
                    accounts.update(cx, |accounts, cx| {
                        let id = accounts.selected(*kind, &project).id;
                        accounts.begin_footer_auth(*kind, &id, cx);
                    });
                }
                let _ = (project, kind);
                self.watch_sign_in(view.clone(), cx);
            }
            SetupEvent::Rescan => crate::installed_providers::probe(cx),
            SetupEvent::ChooseFolder => self.choose_setup_folder(view.clone(), cx),
            SetupEvent::ChooseRemoteFolder => {
                if let Some(setup) = &mut self.setup {
                    setup.remote_from = Some(self.workspace.project.clone());
                }
                self.open_remote_project(window, cx);
            }
            SetupEvent::Finish(path) => self.finish_setup(path.clone(), window, cx),
        }
        cx.notify();
    }

    /// Clears the card's "Waiting for browser…" once no sign-in is running,
    /// including one that failed, was cancelled or never started.
    fn watch_sign_in(&mut self, view: Entity<SetupCard>, cx: &mut Context<Self>) {
        let accounts = crate::accounts::get(cx);
        cx.spawn(async move |_, cx| {
            loop {
                let running = accounts
                    .as_ref()
                    .map(|a| a.read_with(cx, |a, _| a.auth_in_progress()))
                    .unwrap_or(false);
                if !running {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
            }
            view.update(cx, |card, cx| card.sign_in_ended(cx));
        })
        .detach();
    }

    fn choose_setup_folder(&mut self, view: Entity<SetupCard>, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open project".into()),
        });
        cx.spawn(async move |_, cx| {
            // A cancelled picker leaves the card as it was.
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                view.update(cx, |card, cx| card.set_folder(path, cx));
            }
        })
        .detach();
    }

    fn finish_setup(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let drop_launch = self.setup.as_ref().is_some_and(|s| s.drop_launch);
        // Passing the picked folder as the launch folder drops nothing.
        let launch = if drop_launch {
            self.setup_launch.clone()
        } else {
            path.clone()
        };
        let launch_has_sessions = self.session_summaries.values().any(|s| s.cwd == launch);
        self.remember_project_location(&path, cx);
        self.workspace
            .finish_setup(&path, &launch, launch_has_sessions);
        self.setup = None;
        self.select_project(path, window, cx);
        self.create_session(self.default_kind(cx), true, cx);
        self.action_notice = Some("You're set up. Change any of this in Settings.".into());
        window.focus(&self.focus, cx);
        self.persist(cx);
    }

    /// The card and its scrim, above every other modal. Opens the card when
    /// setup is pending and it isn't showing yet.
    pub(super) fn setup_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.setup_shows() {
            return None;
        }
        let step = self.workspace.setup?;
        if self.setup.is_none() {
            self.open_setup(step, true, window, cx);
        }
        let setup = self.setup.as_mut()?;
        // A folder picked in the machine picker becomes the setup folder.
        if self.remote_project_picker.is_none()
            && let Some(from) = setup.remote_from.take()
            && from != self.workspace.project
        {
            let picked = self.workspace.project.clone();
            setup
                .view
                .update(cx, |card, cx| card.set_folder(picked, cx));
        }
        let view = setup.view.clone();
        let focus = view.focus_handle(cx);
        // The machine picker opens above the card; otherwise the card keeps focus.
        if self.remote_project_picker.is_none() && !focus.contains_focused(window, cx) {
            window.focus(&focus, cx);
        }
        Some(crate::modal::window_overlay(
            window,
            div()
                .id("setup-overlay")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(crate::modal::scrim("setup-scrim"))
                .track_focus(&focus)
                .key_context("Setup")
                .on_key_down(cx.listener(move |_, event: &gpui::KeyDownEvent, _, cx| {
                    let keys = &event.keystroke;
                    if keys.key == "o" && keys.modifiers.secondary() && !keys.modifiers.shift {
                        view.update(cx, |_, cx| cx.emit(SetupEvent::ChooseFolder));
                    }
                    // Nothing else reaches the shell, Esc included.
                    if !(keys.key == "q" && keys.modifiers.secondary()) {
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(setup.view.clone()),
        ))
    }
}
