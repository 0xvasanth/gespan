//! The setup card's state and the events the shell acts on. Each step draws
//! itself in its own file from the pieces in `ui`.
use super::{SetupStep, agents, look, project, welcome};
use crate::settings::Preferences;
use gespan_core::provider::Kind;
use gpui::{Context, EventEmitter, FocusHandle, Focusable, Window, prelude::*};

/// What the card asks the shell to do. The card keeps its own copy of what
/// was picked so it can draw it; the shell applies and saves it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SetupEvent {
    Step(SetupStep),
    /// A Look pick, applied live behind the card.
    Preferences(Box<Preferences>),
    Layout { classic: bool },
    DefaultAgent(Kind),
    SignIn(Kind),
    Rescan,
    ChooseFolder,
    ChooseRemoteFolder,
    Finish(String),
}

/// One installed agent on the agents card.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Agent {
    pub kind: Kind,
    /// `None` when the agent's CLI can't say (it signs in on first use).
    pub signed_in: Option<bool>,
}

pub(crate) struct SetupCard {
    pub(super) step: SetupStep,
    pub(super) prefs: Preferences,
    pub(super) classic: bool,
    /// The folder Gespan was launched from, and the home folder, for the
    /// "Opened from" suggestion.
    pub(super) launch: String,
    pub(super) home: String,
    /// The project folder picked on the last card.
    pub(super) folder: Option<String>,
    /// Cards left with Skip or Skip setup, for the "Using defaults" line.
    pub(super) skipped: Vec<SetupStep>,
    pub(super) agents: Vec<Agent>,
    pub(super) missing: Vec<Kind>,
    pub(super) default_agent: Option<Kind>,
    /// The agent whose sign-in is running; Rescan starts no second one.
    pub(super) signing_in: Option<Kind>,
    focus: FocusHandle,
}

impl EventEmitter<SetupEvent> for SetupCard {}

impl Focusable for SetupCard {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SetupCard {
    pub(crate) fn new(
        step: SetupStep,
        prefs: Preferences,
        classic: bool,
        launch: String,
        home: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let default_agent = prefs.last_used_provider();
        let mut card = Self {
            step,
            prefs,
            classic,
            launch,
            home,
            folder: None,
            skipped: Vec::new(),
            agents: Vec::new(),
            missing: Vec::new(),
            default_agent,
            signing_in: None,
            focus: cx.focus_handle(),
        };
        card.refresh_agents(cx);
        card
    }

    pub(crate) fn step(&self) -> SetupStep {
        self.step
    }

    /// Moves to `step` and tells the shell, so a relaunch resumes there.
    pub(super) fn go(&mut self, step: SetupStep, cx: &mut Context<Self>) {
        self.step = step;
        cx.emit(SetupEvent::Step(step));
        cx.notify();
    }

    /// Skip on a card: its defaults stand.
    pub(super) fn skip(&mut self, next: SetupStep, cx: &mut Context<Self>) {
        if !self.skipped.contains(&self.step) {
            self.skipped.push(self.step);
        }
        self.go(next, cx);
    }

    /// Skip setup: straight to Project; cards not yet passed keep defaults.
    pub(super) fn skip_setup(&mut self, cx: &mut Context<Self>) {
        for step in [SetupStep::Agents, SetupStep::Look] {
            if step as u8 >= self.step as u8 && !self.skipped.contains(&step) {
                self.skipped.push(step);
            }
        }
        self.go(SetupStep::Project, cx);
    }

    pub(super) fn set_prefs(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        change(&mut self.prefs);
        cx.emit(SetupEvent::Preferences(Box::new(self.prefs.clone())));
        cx.notify();
    }

    pub(super) fn set_layout(&mut self, classic: bool, cx: &mut Context<Self>) {
        if self.classic != classic {
            self.classic = classic;
            cx.emit(SetupEvent::Layout { classic });
            cx.notify();
        }
    }

    pub(super) fn pick_agent(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.default_agent = Some(kind);
        cx.emit(SetupEvent::DefaultAgent(kind));
        cx.notify();
    }

    pub(super) fn sign_in(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if self.signing_in.is_some() {
            return;
        }
        self.signing_in = Some(kind);
        cx.emit(SetupEvent::SignIn(kind));
        cx.notify();
    }

    pub(super) fn rescan(&mut self, cx: &mut Context<Self>) {
        cx.emit(SetupEvent::Rescan);
    }

    pub(super) fn finish(&mut self, cx: &mut Context<Self>) {
        if let Some(folder) = self.folder.clone() {
            cx.emit(SetupEvent::Finish(folder));
        }
    }

    /// The folder picker or a machine's folder came back with `path`.
    pub(crate) fn set_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.folder = Some(path);
        cx.notify();
    }

    /// The sign-in the shell started has ended, either way.
    pub(crate) fn sign_in_ended(&mut self, cx: &mut Context<Self>) {
        self.signing_in = None;
        self.refresh_agents(cx);
    }

    /// Re-reads which agents are installed, then asks each CLI off the UI
    /// thread whether it is signed in.
    pub(crate) fn refresh_agents(&mut self, cx: &mut Context<Self>) {
        let installed = crate::installed_providers::snapshot(cx);
        let (found, missing): (Vec<Kind>, Vec<Kind>) =
            Kind::ALL.into_iter().partition(|kind| installed.installed(*kind));
        self.missing = missing;
        self.agents = found
            .iter()
            .map(|kind| Agent {
                kind: *kind,
                signed_in: self
                    .agents
                    .iter()
                    .find(|a| a.kind == *kind)
                    .and_then(|a| a.signed_in),
            })
            .collect();
        if self.default_agent.is_none_or(|kind| !found.contains(&kind)) {
            self.default_agent = found.first().copied();
        }
        cx.notify();
        if cfg!(test) {
            return;
        }
        let work = cx.background_executor().spawn(async move {
            let profile = gespan_core::accounts::Profile::default();
            found
                .into_iter()
                .map(|kind| (kind, gespan_core::accounts::signed_in(kind, &profile)))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let states = work.await;
            let _ = this.update(cx, |card, cx| {
                for (kind, signed_in) in states {
                    if let Some(agent) = card.agents.iter_mut().find(|a| a.kind == kind) {
                        agent.signed_in = signed_in;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Test seam: set the agents card's rows without probing.
    #[cfg(test)]
    pub(crate) fn set_agents(&mut self, agents: Vec<Agent>, cx: &mut Context<Self>) {
        let found: Vec<Kind> = agents.iter().map(|a| a.kind).collect();
        self.missing = Kind::ALL.into_iter().filter(|k| !found.contains(k)).collect();
        self.agents = agents;
        cx.notify();
    }
}

impl Render for SetupCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.step {
            SetupStep::Welcome => welcome::render(self, window, cx),
            SetupStep::Agents => agents::render(self, window, cx),
            SetupStep::Look => look::render(self, window, cx),
            SetupStep::Project => project::render(self, window, cx),
        }
    }
}
