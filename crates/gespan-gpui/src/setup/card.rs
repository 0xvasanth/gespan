//! The setup card's state and the events the shell acts on. Each step draws
//! itself in its own file from the pieces in `ui`.
use super::{SetupStep, agents, look, project, tools, welcome};
use crate::settings::Preferences;
use gespan_core::mcp::Importable;
use gespan_core::provider::Kind;
use gpui::{Context, EventEmitter, FocusHandle, Focusable, Window, prelude::*};
use std::path::PathBuf;

/// What the card asks the shell to do. The card keeps its own copy of what
/// was picked so it can draw it; the shell applies and saves it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SetupEvent {
    Step(SetupStep),
    /// A Look pick, applied live behind the card.
    Preferences(Box<Preferences>),
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

/// The shared tools card's state.
pub(crate) struct Tools {
    /// The servers found in the agents' configs; `None` while still looking.
    pub(super) found: Option<Vec<Importable>>,
    /// Agents use only the hub's servers. On by default.
    pub(super) on: bool,
    /// The servers are being moved; Continue waits.
    pub(super) busy: bool,
    pub(super) error: Option<String>,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            found: None,
            on: true,
            busy: false,
            error: None,
        }
    }
}

pub(crate) struct SetupCard {
    pub(super) step: SetupStep,
    pub(super) prefs: Preferences,
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
    pub(super) tools: Tools,
    /// Test seam: the catalogue folder, in place of Gespan's data folder.
    #[cfg(test)]
    tools_dir: Option<PathBuf>,
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
        launch: String,
        home: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let default_agent = prefs.last_used_provider();
        let mut card = Self {
            step,
            prefs,
            launch,
            home,
            folder: None,
            skipped: Vec::new(),
            agents: Vec::new(),
            missing: Vec::new(),
            default_agent,
            signing_in: None,
            tools: Tools::default(),
            #[cfg(test)]
            tools_dir: None,
            focus: cx.focus_handle(),
        };
        card.refresh_agents(cx);
        if step == SetupStep::Tools {
            card.find_tools(cx);
        }
        card
    }

    #[cfg(test)]
    pub(crate) fn step(&self) -> SetupStep {
        self.step
    }

    /// The look and agent choices the card shows.
    #[cfg(test)]
    pub(crate) fn prefs(&self) -> &Preferences {
        &self.prefs
    }

    /// The agent whose sign-in is running.
    #[cfg(test)]
    pub(crate) fn signing_in(&self) -> Option<Kind> {
        self.signing_in
    }

    /// The installed agents and what each says about sign-in.
    pub(crate) fn agents(&self) -> &[Agent] {
        &self.agents
    }

    /// Moves to `step` and tells the shell, so a relaunch resumes there.
    pub(super) fn go(&mut self, step: SetupStep, cx: &mut Context<Self>) {
        self.step = step;
        if step == SetupStep::Tools && self.tools.found.is_none() {
            self.find_tools(cx);
        }
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
        for step in [SetupStep::Agents, SetupStep::Tools, SetupStep::Look] {
            if step as u8 >= self.step as u8 && !self.skipped.contains(&step) {
                self.skipped.push(step);
            }
        }
        self.go(SetupStep::Project, cx);
    }

    /// Looks through the agents' configs for MCP servers, off the UI thread.
    pub(super) fn find_tools(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let home = self.home.clone();
        let work = cx
            .background_executor()
            .spawn(async move { gespan_core::mcp::mcp_importable(home) });
        cx.spawn(async move |this, cx| {
            let found = work.await.unwrap_or_else(|error| {
                tracing::warn!(%error, "could not look for the agents' MCP servers");
                Vec::new()
            });
            let _ = this.update(cx, |card, cx| {
                card.tools.found = Some(found);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn toggle_tools(&mut self, cx: &mut Context<Self>) {
        self.tools.on = !self.tools.on;
        self.tools.error = None;
        cx.notify();
    }

    fn tools_dir(&self) -> Option<PathBuf> {
        #[cfg(test)]
        if self.tools_dir.is_some() {
            return self.tools_dir.clone();
        }
        gespan_core::tool_servers::data_dir()
    }

    /// Continue on the shared tools card. With the switch on, the servers
    /// shown are moved into the catalogue and approved, and agents load only
    /// the hub's, all in one edit; the card moves on once that is saved.
    pub(super) fn tools_next(&mut self, cx: &mut Context<Self>) {
        if self.tools.busy {
            return;
        }
        let Some(found) = self.tools.found.clone() else {
            return;
        };
        if !self.tools.on {
            self.go(SetupStep::Look, cx);
            return;
        }
        let Some(dir) = self.tools_dir() else {
            self.tools.error = Some("Gespan's data folder isn't available.".into());
            cx.notify();
            return;
        };
        self.tools.busy = true;
        self.tools.error = None;
        cx.notify();
        let work = cx.background_executor().spawn(async move {
            let edited = gespan_core::mcp_catalog::update(&dir, |catalog| {
                catalog.adopt(&dir, found).map(|_| ())
            })?;
            if edited.changed {
                gespan_core::tool_servers::sync_servers();
            }
            Ok::<(), String>(())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |card, cx| {
                card.tools.busy = false;
                match result {
                    Ok(()) => card.go(SetupStep::Look, cx),
                    Err(error) => {
                        tracing::warn!(%error, "could not move the agents' MCP servers");
                        card.tools.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Test seam: the catalogue folder the shared tools card writes.
    #[cfg(test)]
    pub(crate) fn set_tools_dir(&mut self, dir: PathBuf) {
        self.tools_dir = Some(dir);
    }

    /// Test seam: what discovery found.
    #[cfg(test)]
    pub(crate) fn set_found(&mut self, found: Vec<Importable>, cx: &mut Context<Self>) {
        self.tools.found = Some(found);
        cx.notify();
    }

    pub(super) fn set_prefs(
        &mut self,
        change: impl FnOnce(&mut Preferences),
        cx: &mut Context<Self>,
    ) {
        change(&mut self.prefs);
        cx.emit(SetupEvent::Preferences(Box::new(self.prefs.clone())));
        cx.notify();
    }

    pub(super) fn pick_agent(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.default_agent = Some(kind);
        cx.emit(SetupEvent::DefaultAgent(kind));
        cx.notify();
    }

    pub(crate) fn sign_in(&mut self, kind: Kind, cx: &mut Context<Self>) {
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
        let (found, missing): (Vec<Kind>, Vec<Kind>) = Kind::ALL
            .into_iter()
            .partition(|kind| installed.installed(*kind));
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
        self.missing = Kind::ALL
            .into_iter()
            .filter(|k| !found.contains(k))
            .collect();
        self.agents = agents;
        cx.notify();
    }
}

impl Render for SetupCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.step {
            SetupStep::Welcome => welcome::render(self, window, cx),
            SetupStep::Agents => agents::render(self, window, cx),
            SetupStep::Tools => tools::render(self, window, cx),
            SetupStep::Look => look::render(self, window, cx),
            SetupStep::Project => project::render(self, window, cx),
        }
    }
}
