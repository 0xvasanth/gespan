//! The sidebar's Agents footer and the panel it opens upward: every agent
//! this window has open, grouped as waiting on the user, running and idle.
//!
//! Running comes from `running_chats`, the same list the quit guard and the
//! next launch use, so the footer can never disagree with what Gespan
//! would interrupt. Nothing is hidden behind a minimum or a cap: one
//! running agent shows, and the twentieth is in the list.
use super::*;
use gpui::{Div, Hsla, Stateful};

/// Up to this many agents the list hugs its rows; beyond it the list is
/// [`LIST_HEIGHT`] tall and scrolls inside the panel.
pub(super) const LIST_ROWS: usize = 6;
pub(super) const LIST_HEIGHT: f32 = 252.;
const FOOTER_HEIGHT: f32 = 36.;
/// Between the footer and the panel above it.
const PANEL_GAP: f32 = 8.;
/// The sidebar's inset around the footer.
const INSET: f32 = 8.;
/// The waiting yellow, and its ink: lighter on dark glass, deeper on white.
const WAIT: u32 = 0xe8c547;
const WAIT_INK: (u32, u32) = (0xf5dc84, 0xa86b00);
const TILE: crate::tab_tile::TileSize = crate::tab_tile::TileSize {
    w: 18.,
    h: 16.,
    radius: 4.,
};

/// A Pilot chat's Pilot: its id and name.
type PilotRef = (String, String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum AgentState {
    NeedsYou,
    Running,
    Idle,
}

impl AgentState {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Running => "Running",
            Self::Idle => "Idle",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs-you",
            Self::Running => "running",
            Self::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Agent {
    pub id: i64,
    /// The Pilot whose chat this is; opening it opens the Pilot.
    pub pilot: Option<String>,
    pub title: String,
    pub provider: String,
    /// "Claude Code", or the Pilot's name.
    pub origin: String,
    pub cwd: String,
    pub state: AgentState,
    /// A question rather than a tool approval.
    pub asks: bool,
    /// When the running turn started.
    pub started_at: Option<u64>,
    pub updated_at: u64,
}

/// Waiting first, then running, then idle. Waiting and running keep the
/// longest-waiting first; idle shows the most recent first.
pub(super) fn sort_agents(agents: &mut [Agent]) {
    agents.sort_by(|a, b| {
        a.state.cmp(&b.state).then_with(|| match a.state {
            AgentState::Idle => b.updated_at.cmp(&a.updated_at),
            _ => a
                .started_at
                .unwrap_or(u64::MAX)
                .cmp(&b.started_at.unwrap_or(u64::MAX)),
        })
    });
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub needs_you: usize,
    pub running: usize,
    pub idle: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Strong,
    Wait,
    Dim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dot {
    Wait,
    Running,
    Idle,
}

impl Counts {
    pub(super) fn of(agents: &[Agent]) -> Self {
        let count = |state| agents.iter().filter(|a| a.state == state).count();
        Self {
            needs_you: count(AgentState::NeedsYou),
            running: count(AgentState::Running),
            idle: count(AgentState::Idle),
        }
    }
    /// The collapsed footer's words, each in its tone. With nothing
    /// running or waiting it counts the idle agents instead.
    pub(super) fn footer_parts(&self) -> Vec<(String, Tone)> {
        let mut parts = Vec::new();
        if self.running > 0 {
            parts.push((format!("{} running", self.running), Tone::Strong));
        }
        if self.needs_you > 0 {
            parts.push((format!("{} needs you", self.needs_you), Tone::Wait));
        }
        if parts.is_empty() {
            let idle = match self.idle {
                0 => "No agents".to_owned(),
                1 => "1 agent idle".to_owned(),
                n => format!("{n} agents idle"),
            };
            parts.push((idle, Tone::Dim));
        } else if self.needs_you == 0 && self.idle > 0 {
            parts.push((format!("{} idle", self.idle), Tone::Dim));
        }
        parts
    }
    pub(super) fn footer_text(&self) -> String {
        self.footer_parts()
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join(" · ")
    }
    /// The open panel's header: every group that has agents.
    pub(super) fn header_text(&self) -> String {
        let parts: Vec<String> = [
            (self.running, "running"),
            (self.needs_you, "needs you"),
            (self.idle, "idle"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect();
        if parts.is_empty() {
            "Nothing open".into()
        } else {
            parts.join(" · ")
        }
    }
    pub(super) fn dot(&self) -> Dot {
        if self.needs_you > 0 {
            Dot::Wait
        } else if self.running > 0 {
            Dot::Running
        } else {
            Dot::Idle
        }
    }
}

/// The list's fixed height once it holds more than [`LIST_ROWS`] agents;
/// `None` lets it hug its rows.
pub(super) fn list_height(agents: usize) -> Option<f32> {
    (agents > LIST_ROWS).then_some(LIST_HEIGHT)
}

/// A running turn's age on its pill: "45s", "6m", "1h 5m".
pub(crate) fn elapsed(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds < 60 {
        return format!("{}s", seconds.max(1));
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if rest == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {rest}m")
    }
}

fn wait_ink() -> Hsla {
    rgb(if is_light() { WAIT_INK.1 } else { WAIT_INK.0 }).into()
}

fn dot_color(dot: Dot) -> Hsla {
    match dot {
        Dot::Wait => rgb(WAIT).into(),
        Dot::Running => accent_color(),
        Dot::Idle => content(0.35),
    }
}

fn status_dot(dot: Dot) -> Div {
    let color = dot_color(dot);
    // A soft ring around the dot while something runs or waits.
    div()
        .size(px(13.))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .when(dot != Dot::Idle, |d| d.bg(color.opacity(0.25)))
        .child(div().size(px(7.)).rounded_full().bg(color))
}

fn pill(ink: Hsla, fill: Hsla, edge: Hsla, label: String) -> Div {
    div()
        .flex_shrink_0()
        .h(px(20.))
        .pl(px(8.))
        .pr(px(9.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(fill)
        .border_1()
        .border_color(edge)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .child(
            div()
                .size(px(6.))
                .rounded_full()
                .bg(ink)
                // The mock's 3px halo, so a live dot reads as live.
                .shadow(vec![
                    BoxShadow::new(px(0.), px(0.), ink.opacity(0.25)).spread_radius(px(3.)),
                ]),
        )
        .child(label)
}

impl Shell {
    /// Every agent open in this window: local chats and Pilot chats.
    /// Remote chats belong to their host, and an idle chat counts once it
    /// has talked to an agent and is not archived.
    pub(super) fn open_agents(&self, cx: &App) -> Vec<Agent> {
        let running: HashSet<i64> = self.running_chats(cx).into_iter().collect();
        let now = gespan_core::time::now_ms();
        let mut views: Vec<(Option<PilotRef>, &Entity<Conversation>)> =
            self.sessions.values().map(|view| (None, view)).collect();
        let pilot_chats = self
            .pilots
            .as_ref()
            .map(|pilots| pilots.read(cx).open_chats())
            .unwrap_or_default();
        views.extend(
            pilot_chats
                .iter()
                .map(|(id, name, view)| (Some((id.clone(), name.clone())), view)),
        );
        let mut agents: Vec<Agent> = views
            .into_iter()
            .filter_map(|(pilot, view)| {
                let chat = view.read(cx);
                if chat.is_remote() {
                    return None;
                }
                let id = chat.summary.id;
                let state = if chat.needs_approval() {
                    AgentState::NeedsYou
                } else if running.contains(&id) {
                    AgentState::Running
                } else if chat.summary.has_messages && !chat.summary.archived {
                    AgentState::Idle
                } else {
                    return None;
                };
                let started_at = (state != AgentState::Idle)
                    .then(|| chat.transcript.read(cx).live_turn(&chat.summary.cwd).1)
                    .flatten()
                    .filter(|at| *at <= now);
                let (pilot, origin) = match pilot {
                    Some((pilot, name)) => (Some(pilot), name),
                    None => (None, Kind::from_id(&chat.summary.provider).label().into()),
                };
                Some(Agent {
                    id,
                    pilot,
                    title: chat.display_title(),
                    provider: chat.summary.provider.clone(),
                    origin,
                    cwd: chat.summary.cwd.clone(),
                    state,
                    asks: state == AgentState::NeedsYou
                        && chat.pending_approvals().next().is_none(),
                    started_at,
                    updated_at: chat.summary.updated_at,
                })
            })
            .collect();
        sort_agents(&mut agents);
        agents
    }

    /// Repaints the sidebar without rebuilding its session index.
    pub(super) fn repaint_sidebar(&self, cx: &mut Context<Self>) {
        self.sidebar_frame.update(cx, |_, cx| cx.notify());
    }

    pub(super) fn toggle_agents_panel(&mut self, cx: &mut Context<Self>) {
        self.agents_open = !self.agents_open;
        self.repaint_sidebar(cx);
    }

    /// Escape: whether there was a panel to close.
    pub(super) fn close_agents_panel(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.agents_open {
            return false;
        }
        self.agents_open = false;
        self.repaint_sidebar(cx);
        true
    }

    pub(super) fn open_agent(
        &mut self,
        agent: &Agent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agents_open = false;
        match &agent.pilot {
            Some(pilot) => {
                if let Some(pilots) = self.pilots.clone() {
                    self.workspace.right_panel_open = true;
                    pilots.update(cx, |pilots, cx| pilots.open(pilot, window, cx));
                }
            }
            None => {
                self.selected_sessions.clear();
                self.selection_anchor = Some(agent.id);
                self.open(agent.id, window, cx);
            }
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }

    /// Repaints the open panel once a second while a running turn's time shows.
    fn tick_agents(&mut self, ticking: bool, cx: &mut Context<Self>) {
        if !ticking {
            self.agents_tick = None;
        } else if self.agents_tick.is_none() {
            self.agents_tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(1))
                        .await;
                    if this
                        .update(cx, |this, cx| this.repaint_sidebar(cx))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }

    /// The footer pinned to the sidebar's bottom, and the panel above it
    /// while open. `None` when Settings turns Activity off.
    pub(super) fn agents_footer(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.workspace.preferences.working_agents {
            self.agents_open = false;
            self.tick_agents(false, cx);
            return None;
        }
        let agents = self.open_agents(cx);
        let counts = Counts::of(&agents);
        let open = self.agents_open;
        self.tick_agents(
            open && agents
                .iter()
                .any(|a| a.state != AgentState::Idle && a.started_at.is_some()),
            cx,
        );
        let parts = counts.footer_parts();
        let footer = div()
            .id("agents-footer")
            .debug_selector(|| "agents-footer".into())
            .role(Role::Button)
            .aria_label(format!("Agents: {}", counts.footer_text()))
            .aria_expanded(open)
            .cursor_pointer()
            .w_full()
            .h(px(FOOTER_HEIGHT))
            .px(px(10.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .bg(content(if open { 0.08 } else { 0.04 }))
            .border_1()
            .border_color(content(0.07))
            .hover(|s| s.bg(content(0.08)))
            .text_size(px(12.))
            .child(status_dot(counts.dot()))
            .child(div().flex_1().min_w_0().truncate().flex().children(
                parts.into_iter().enumerate().map(|(ix, (text, tone))| {
                    let text = if ix == 0 { text } else { format!(" · {text}") };
                    div()
                        .flex_shrink_0()
                        .text_color(match tone {
                            Tone::Strong => content(1.),
                            Tone::Wait => wait_ink(),
                            Tone::Dim => content(0.5),
                        })
                        .when(tone == Tone::Strong, |d| {
                            d.font_weight(FontWeight::SEMIBOLD)
                        })
                        .child(text)
                }),
            ))
            .child(
                icon(if open { "chevron-down" } else { "chevron-up" }, 13.)
                    .text_color(content(0.55)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.sidebar_focus, cx);
                this.toggle_agents_panel(cx);
            }));
        let panel = open.then(|| self.agents_panel(agents, counts, cx));
        Some(
            div()
                .relative()
                .flex_shrink_0()
                .px(px(INSET))
                .pt_1()
                .pb(px(INSET))
                .children(panel)
                .child(footer)
                .into_any_element(),
        )
    }

    fn agents_panel(
        &mut self,
        agents: Vec<Agent>,
        counts: Counts,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fixed = list_height(agents.len());
        let now = gespan_core::time::now_ms();
        let mut list: Vec<AnyElement> = Vec::new();
        for state in [AgentState::NeedsYou, AgentState::Running, AgentState::Idle] {
            let group: Vec<&Agent> = agents.iter().filter(|a| a.state == state).collect();
            if group.is_empty() {
                continue;
            }
            list.push(
                div()
                    .debug_selector(move || format!("agents-group-{}", state.key()))
                    .flex()
                    .justify_between()
                    .px(px(8.))
                    .pt(px(8.))
                    .pb(px(4.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(0.45))
                    .child(state.label().to_uppercase())
                    .child(group.len().to_string())
                    .into_any_element(),
            );
            for agent in group {
                list.push(self.agent_row(agent.clone(), now, cx));
            }
        }
        if list.is_empty() {
            list.push(
                div()
                    .px(px(8.))
                    .py(px(10.))
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child("No agents open. Start a chat and it shows here.")
                    .into_any_element(),
            );
        }
        // Dark: a frosted plate only a little lighter than the sidebar
        // (#16161a at 72%), not a slab of the window's base colour.
        let tint = if is_light() {
            background_base().opacity(0.72)
        } else {
            Hsla::from(rgb(0x16161a)).opacity(0.72)
        };
        let shadow = if is_light() { black(0.12) } else { black(0.45) };
        // Once the list scrolls, the footer says how many rows sit below the
        // first six instead of offering Hold all (the status bar keeps it).
        let more = agents.len().saturating_sub(LIST_ROWS);
        let hold = if fixed.is_some() {
            div()
                .debug_selector(|| "agents-scroll-more".into())
                .px(px(8.))
                .text_size(px(11.5))
                .text_color(content(0.45))
                .child(format!("Scroll for {more} more"))
                .into_any_element()
        } else {
            self.agents_hold_button(cx)
        };
        div()
            .id("agents-panel")
            .debug_selector(|| "agents-panel".into())
            .role(Role::Dialog)
            .aria_label("Agents")
            .occlude()
            .absolute()
            .left(px(INSET))
            .right(px(INSET))
            .bottom(px(INSET + FOOTER_HEIGHT + PANEL_GAP))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .shadow(vec![
                BoxShadow::new(px(0.), px(-12.), shadow).blur_radius(px(40.)),
            ])
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        crate::popover::paint_glass(
                            bounds,
                            gpui::Corners::all(px(12.)),
                            tint,
                            window,
                        )
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .pt(px(10.))
                    .pb(px(8.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .child(status_dot(counts.dot()))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.5))
                            .line_height(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child("Agents"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "agents-header-counts".into())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(content(0.5))
                            .child(counts.header_text()),
                    ),
            )
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("agents-list")
                            .debug_selector(|| "agents-list".into())
                            .role(Role::List)
                            .aria_label(if fixed.is_some() {
                                "Agents, scrollable"
                            } else {
                                "Agents"
                            })
                            .flex()
                            .flex_col()
                            .p(px(4.))
                            .when_some(fixed, |d, height| {
                                d.h(px(height))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.agents_scroll)
                            })
                            .children(list),
                    )
                    .when(fixed.is_some(), |d| {
                        // More below: the last rows fade into the footer.
                        let base = background_base();
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(px(24.))
                                .bg(gpui::linear_gradient(
                                    180.,
                                    gpui::linear_color_stop(base.opacity(0.), 0.),
                                    gpui::linear_color_stop(base.opacity(0.6), 1.),
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(content(0.07))
                    .child(hold)
                    .child(div().flex_1())
                    .child(
                        ghost_button("agents-open-runs", "Open Runs").on_click(cx.listener(
                            |this, _, window, cx| {
                                this.agents_open = false;
                                this.show_runs(window, cx);
                                this.invalidate_chrome(cx);
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(content(0.12))
                    .shadow(vec![BoxShadow::new(px(0.), px(1.), content(0.12)).inset()]),
            )
            .into_any_element()
    }

    /// Hold all, as the status bar's: a click arms it and a second one holds.
    /// On hold it offers Resume; while holding it only says so.
    fn agents_hold_button(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::hold_confirm::Label;
        match crate::hold::phase(cx) {
            None => {
                let now = cx.background_executor().now();
                let label = match self.hold.press.label(now) {
                    Label::Rest => "Hold all",
                    Label::Hint | Label::Armed => "Click again to hold",
                    Label::Done => "Holding…",
                };
                ghost_button("agents-hold-all", label)
                    .aria_label("Hold everything: click twice to confirm")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.hold_button_keyboard_press(cx);
                        this.repaint_sidebar(cx);
                    }))
                    .into_any_element()
            }
            Some(gespan_core::hold::Phase::OnHold) => ghost_button("agents-hold-all", "Resume")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_hold(window, cx);
                    this.repaint_sidebar(cx);
                }))
                .into_any_element(),
            Some(gespan_core::hold::Phase::Holding) => div()
                .debug_selector(|| "agents-hold-all".into())
                .px(px(8.))
                .text_size(px(12.))
                .text_color(content(0.5))
                .child(self.hold_summary(cx))
                .into_any_element(),
        }
    }

    fn agent_row(&mut self, agent: Agent, now: u64, cx: &mut Context<Self>) -> AnyElement {
        let id = agent.id;
        let project = self.project_label(&agent.cwd);
        let meta = format!("{} · {project}", agent.origin);
        let (status, said): (AnyElement, String) = match agent.state {
            AgentState::NeedsYou => {
                let ink = wait_ink();
                let fill: Hsla = rgb(WAIT).into();
                let label = if agent.asks { "Answer" } else { "Approve" };
                (
                    pill(ink, fill.opacity(0.1), fill.opacity(0.22), label.into())
                        .into_any_element(),
                    format!("needs you: {}", label.to_lowercase()),
                )
            }
            AgentState::Running => {
                let accent = accent_color();
                let time = agent.started_at.map(|at| elapsed(now.saturating_sub(at)));
                (
                    pill(
                        accent,
                        accent.opacity(0.14),
                        accent.opacity(0.28),
                        time.clone().unwrap_or_else(|| "Running".into()),
                    )
                    .into_any_element(),
                    time.map_or_else(|| "running".into(), |t| format!("running for {t}")),
                )
            }
            AgentState::Idle => {
                let age = gespan_core::time::relative(agent.updated_at, now);
                (
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.5))
                        .text_color(content(0.45))
                        .child(age.clone())
                        .into_any_element(),
                    format!("idle, {age}"),
                )
            }
        };
        let selected = agent.pilot.is_none()
            && self.workspace.active == Some(id)
            && !self.pilot_active(cx)
            && !self.app_view_open();
        let face = crate::tab_tile::Face::provider(&agent.provider);
        let glyph = provider_glyph(&agent.provider, 10.);
        div()
            .id(("agents-row", id as usize))
            .debug_selector(move || format!("agents-row-{id}"))
            .role(Role::Button)
            .aria_label(format!("{}, {meta}, {said}", agent.title))
            .cursor_pointer()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(9.))
            .px(px(8.))
            .py(px(7.))
            .rounded(px(8.))
            .when(selected, |d| d.bg(content(0.08)))
            .hover(|s| s.bg(content(0.06)))
            .child(crate::tab_tile::mark_tile(&TILE, face, glyph))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .line_height(px(15.))
                            .text_color(content(0.92))
                            .child(agent.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .line_height(px(13.))
                            .text_color(content(0.5))
                            .child(meta),
                    ),
            )
            .child(status)
            .on_click(cx.listener(move |this, _, window, cx| this.open_agent(&agent, window, cx)))
            .into_any()
    }
}

fn ghost_button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .cursor_pointer()
        .flex_shrink_0()
        .h(px(26.))
        .px(px(8.))
        .rounded_md()
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(content(0.6))
        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
        .child(label)
}
