//! The shell's side of Home: opening it, the snapshot it draws from (My
//! tasks, the open agents, the Inbox and the usage cache) and where its
//! clicks go.
use super::agents_panel::AgentState;
use super::*;
use crate::conversation::approval_card::Card;
use crate::home::{self, Data, Home, OneClick, Page, Usage, Waiting, Working};
use crate::my_tasks::{Body, Source};

impl Shell {
    pub(crate) fn open_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_app_view(window, cx);
        if self.home.is_none() {
            let view = cx.new(Home::new);
            self.app_view_events.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &home::Event, window, cx| this.route_home(event, window, cx),
            ));
            self.home = Some(view);
        }
        // The tiles count what these pages hold, so make sure they exist.
        self.ensure_my_tasks(cx);
        self.refresh_my_tasks(cx);
        if !cfg!(test) {
            self.ensure_background_inbox(window, cx);
        }
        self.show_app_view(app_views::AppView::Home, window, cx);
        self.refresh_home(cx);
        if let Some(home) = &self.home {
            home.update(cx, |home, cx| home.shown(cx));
        }
    }

    /// Hands Home what the shell knows now, and the automation service once
    /// it is open.
    pub(super) fn refresh_home(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        match self.automation_state.service.clone() {
            Some(service) => home.update(cx, |home, cx| home.set_service(service, cx)),
            None if !self.secondary => self.initialize_automations(cx),
            None => {}
        }
        let data = self.home_data(cx);
        home.update(cx, |home, cx| home.set_data(data, cx));
    }

    pub(super) fn home_data(&self, cx: &App) -> Data {
        let waiting = self
            .my_tasks
            .as_ref()
            .map(|view| {
                view.read(cx)
                    .tasks()
                    .iter()
                    .map(|task| Waiting {
                        key: task.key.clone(),
                        kind: task.kind,
                        headline: task.headline.clone(),
                        source_label: task.source_label.clone(),
                        project: task.project.clone(),
                        since_ms: task.since_ms,
                        source: task.source.clone(),
                        allow: match (&task.body, &task.source) {
                            (Body::Approval { approval, cwd }, Source::Session(id)) => {
                                let card = Card::new(approval, cwd.as_deref());
                                home::one_click(card.lone_command()).map(|command| OneClick {
                                    session: *id,
                                    request: approval.id.clone(),
                                    command,
                                })
                            }
                            _ => None,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        let working = self
            .open_agents(cx)
            .into_iter()
            .filter(|agent| agent.state == AgentState::Running)
            .map(|agent| Working {
                id: agent.id,
                title: agent.title,
                provider: agent.provider,
                origin: agent.origin,
                project: agent.cwd,
                started_at: agent.started_at,
            })
            .collect();
        let inbox = self
            .inbox
            .as_ref()
            .and_then(|inbox| inbox.read(cx).home_counts());
        Data {
            waiting,
            working,
            inbox,
            usage: self.home_usage(cx),
        }
    }

    /// Each provider's tightest plan window, for the account this project
    /// uses. Providers that never reported usage are left out.
    fn home_usage(&self, cx: &App) -> Vec<Usage> {
        let Some(accounts) = crate::accounts::get(cx) else {
            return Vec::new();
        };
        let accounts = accounts.read(cx);
        let now = gespan_core::usage::now_ms();
        Kind::ALL
            .iter()
            .filter_map(|kind| {
                let profile = accounts.selected(*kind, &self.workspace.project);
                let snapshot = accounts
                    .usage
                    .iter()
                    .filter(|(key, _)| key.kind == *kind && key.profile == profile)
                    .find_map(|(_, cached)| cached.snapshot.as_ref())?;
                let tightest = snapshot
                    .plan_windows()
                    .map(|window| {
                        // A window past its reset has nothing used yet.
                        let lapsed = window.resets_at.is_some_and(|at| at <= now);
                        let used = if lapsed { 0. } else { window.used_percent };
                        (used, window.resets_at.filter(|_| !lapsed))
                    })
                    .max_by(|a, b| a.0.total_cmp(&b.0))?;
                Some(Usage {
                    provider: kind.id().to_owned(),
                    label: kind.label().to_owned(),
                    used: tightest.0,
                    resets_at: tightest.1,
                })
            })
            .collect()
    }

    fn route_home(&mut self, event: &home::Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            home::Event::Refresh => self.refresh_home(cx),
            home::Event::Task(event) => {
                self.route_my_task(event, window, cx);
                self.refresh_home(cx);
            }
            home::Event::OpenTask(key) => {
                self.open_my_tasks(window, cx);
                if let Some(view) = &self.my_tasks {
                    view.update(cx, |view, cx| view.select(key, cx));
                }
            }
            home::Event::Open(page) => self.open_home_page(*page, window, cx),
            home::Event::OpenAgent(id) => {
                if let Some(agent) = self.open_agents(cx).into_iter().find(|a| a.id == *id) {
                    self.open_agent(&agent, window, cx);
                }
            }
        }
    }

    pub(super) fn open_home_page(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match page {
            Page::MyTasks => self.open_my_tasks(window, cx),
            Page::Runs => self.show_runs(window, cx),
            Page::Inbox => self.open_inbox(window, cx),
            Page::Agents => {
                self.agents_open = true;
                self.repaint_sidebar(cx);
            }
        }
    }
}
