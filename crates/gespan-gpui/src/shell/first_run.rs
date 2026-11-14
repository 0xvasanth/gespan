//! The first-run setup screen. While the workspace has setup pending, the
//! window shows only the setup card, centred on the content plane, with both
//! navs collapsed and their toggles locked. It closes only when a project is
//! picked: Esc and the shell's shortcuts do nothing. Finishing opens the
//! navs: both on a fresh install, as they were for Run setup again.
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
    /// Only setup on a fresh install drops the launch folder it didn't pick
    /// and opens both navs; Run setup again leaves every project alone and
    /// the navs as they were.
    drop_launch: bool,
}

/// What a locked toggle's tooltip says.
pub(super) const SIDEBAR_LOCKED: &str = "Finish setup to open the sidebar";
pub(super) const PANEL_LOCKED: &str = "Finish setup to open the panel";

/// A locked toggle's tooltip: a lock and why.
struct LockedTip(&'static str);
impl Render for LockedTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut background = floating_fill();
        background.a = 0.96;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(9.))
            .py(px(6.))
            .rounded(px(8.))
            .bg(background)
            .border_1()
            .border_color(content(0.12))
            .shadow(vec![
                gpui::BoxShadow::new(px(0.), px(8.), black(0.4)).blur_radius(px(24.)),
            ])
            .text_size(px(11.5))
            .line_height(px(15.))
            .text_color(content(1.))
            .child(icon("lock", 11.).text_color(content(1.)))
            .child(self.0)
    }
}

/// A dimmed title bar toggle that does nothing and says why on hover.
fn locked_toggle(id: &'static str, glyph: &'static str, tip: &'static str) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(tip)
        .aria_toggled(gpui::Toggled::False)
        .size(px(26.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .opacity(0.28)
        .cursor_not_allowed()
        .child(icon(glyph, 15.).text_color(content(0.62)))
        // A click lands on the button, not the drag area behind it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(move |_, cx| cx.new(|_| LockedTip(tip)).into())
        .into_any_element()
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

    /// Whether the sidebar and right panel are locked shut: setup shows in
    /// this window. Their toggles, shortcuts and commands do nothing, and
    /// the saved open state is left alone for finishing to restore.
    pub(super) fn panels_locked(&self) -> bool {
        self.setup_shows()
    }

    /// Setup finished: the navs slide open from closed. A fresh install
    /// opens both; Run setup again brings back the ones that were open.
    fn unlock_panels(&mut self, open_both: bool, cx: &mut Context<Self>) {
        if open_both {
            self.workspace.sidebar_visible = true;
            self.workspace.right_panel_open = true;
        }
        if self.workspace.sidebar_visible {
            self.workspace.sidebar_visible = false;
            self.set_sidebar_visible(true, cx);
        }
        if self.workspace.right_panel_open && !cx.reduce_motion() {
            let full = self.right_panel_width();
            self.right_slide = Some(PaneSlide::new(0., full, full));
        }
        self.sync_right_panel(cx);
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
        self.unlock_panels(drop_launch, cx);
        self.select_project(path, window, cx);
        self.create_session(self.default_kind(cx), true, cx);
        self.action_notice = Some("You're set up. Change any of this in Settings.".into());
        window.focus(&self.focus, cx);
        self.persist(cx);
    }

    /// The setup screen in place of the shell's body: a title bar with no
    /// tabs and both toggles locked, and the card alone, centred on the
    /// content plane. Opens the card when setup is pending and it isn't
    /// showing yet.
    pub(super) fn setup_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<AnyElement>> {
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
        let card = setup.view.clone();
        let title_bar = div()
            .id("setup-title-bar")
            .debug_selector(|| "setup-title-bar".into())
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(inset_chrome_wash())
            .child(
                titlebar_drag_area()
                    .w(px(self.traffic_lights()))
                    .flex_shrink_0(),
            )
            // The glyphs' dividers sit on the side each panel opens: the
            // `sidebar-right` asset is the mirrored one, its line on the left.
            .child(locked_toggle(
                "toggle-sidebar",
                "sidebar-right",
                SIDEBAR_LOCKED,
            ))
            .child(
                titlebar_drag_area()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child("Gespan · Setup"),
            )
            .child(locked_toggle("toggle-right-panel", "sidebar", PANEL_LOCKED))
            .child(
                titlebar_drag_area()
                    .w(px(12. + self.window_controls))
                    .flex_shrink_0(),
            );
        let plane = div()
            .id("setup-plane")
            .debug_selector(|| "setup-plane".into())
            .flex_1()
            .min_h_0()
            .mx(px(6.))
            .mb(px(6.))
            .rounded(px(super::layout::CARD_RADIUS))
            .border_1()
            .border_color(content(0.08))
            .bg(inset_pane_fill())
            .overflow_y_scroll()
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
            .child(
                div()
                    .min_h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .child(card),
            );
        Some(vec![
            title_bar.into_any_element(),
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(plane)
                .into_any_element(),
        ])
    }
}
