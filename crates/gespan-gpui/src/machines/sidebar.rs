//! The sidebar's Machines section: each herdr machine and its agents, with
//! the same marks as sessions. Clicking an agent opens herdr's own client.
use crate::session_mark::State;
use crate::theme::*;
use gespan_core::machines::{Agent, MachineView, Mark, Online};
use gpui::{
    AnyElement, App, ClickEvent, Div, FontWeight, SharedString, Stateful, Window, div, prelude::*,
    px,
};

pub(crate) fn state_for(mark: Mark) -> State {
    match mark {
        Mark::Attention => State::Attention,
        Mark::Working => State::Working,
        Mark::Done => State::Done,
        Mark::Idle => State::Idle,
    }
}

/// The agent's name, else its title, else its kind, else its terminal id.
pub(crate) fn agent_label(agent: &Agent) -> &str {
    [&agent.name, &agent.title, &agent.agent]
        .into_iter()
        .filter_map(|text| text.as_deref())
        .find(|text| !text.is_empty())
        .unwrap_or(&agent.terminal_id)
}

/// Whether the machine's row is greyed, and what follows its name.
pub(crate) fn machine_status(view: &MachineView) -> (bool, Option<&'static str>) {
    match view.snapshot.online {
        Online::Online => (false, None),
        Online::Offline { .. } => (true, Some("· offline")),
        Online::NeedsSignIn { .. } => (true, Some("· sign-in needed")),
        _ => (true, None),
    }
}

/// A machine switched off has no connection, so it stays out of the sidebar.
pub(crate) fn listed(view: &MachineView) -> bool {
    view.machine.enabled
}

/// The machine's agents in workspace order, herdr's order within each.
pub(crate) fn agents(view: &MachineView) -> Vec<&Agent> {
    let snapshot = &view.snapshot;
    let mut agents: Vec<&Agent> = snapshot.agents.iter().collect();
    agents.sort_by_key(|agent| {
        snapshot
            .workspaces
            .iter()
            .position(|w| w.workspace_id == agent.workspace_id)
            .unwrap_or(usize::MAX)
    });
    agents
}

pub(crate) fn find<'a>(
    views: &'a [MachineView],
    machine_id: &str,
    terminal_id: &str,
) -> Option<(&'a MachineView, &'a Agent)> {
    let view = views.iter().find(|v| v.machine.id == machine_id)?;
    let agent = view
        .snapshot
        .agents
        .iter()
        .find(|a| a.terminal_id == terminal_id)?;
    Some((view, agent))
}

/// The tab title for an agent's attach terminal.
pub(crate) fn attach_title(view: &MachineView, agent: &Agent) -> String {
    format!("{} on {}", agent_label(agent), view.machine.label)
}

pub(crate) fn machine_row(view: &MachineView) -> Stateful<Div> {
    let (greyed, suffix) = machine_status(view);
    let id = view.machine.id.clone();
    let suffix_id = match view.snapshot.online {
        Online::Offline { .. } => "offline",
        _ => "sign-in",
    };
    let selector = format!("machine-{id}");
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .gap_1p5()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(content(if greyed { 0.4 } else { 0.85 }))
        .child(icon("terminal", 14.).text_color(content(if greyed { 0.3 } else { 0.5 })))
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(SharedString::from(view.machine.label.clone())),
        )
        .children(suffix.map(|suffix| {
            let selector = format!("machine-{id}-{suffix_id}");
            div()
                .debug_selector(move || selector)
                .flex_shrink_0()
                .font_weight(FontWeight::NORMAL)
                .text_color(content(0.35))
                .child(suffix)
        }))
}

/// One agent under its machine. Without `on_click` the row offers nothing.
pub(crate) fn agent_row(
    machine_id: &str,
    agent: &Agent,
    greyed: bool,
    mark: Option<AnyElement>,
    on_click: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> Stateful<Div> {
    let selector = format!("machine-agent-{machine_id}-{}", agent.terminal_id);
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .h(px(28.))
        .pl(px(18.))
        .pr_2()
        .rounded_md()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(content(if greyed { 0.4 } else { 0.8 }))
        .child(
            div()
                .size(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(greyed, |d| d.opacity(0.5))
                .children(mark),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(agent_label(agent).to_owned())),
        )
        .when_some(on_click, |d, on_click| {
            d.cursor_pointer()
                .hover(|s| s.bg(content(0.06)))
                .on_click(on_click)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::machines::{AgentStatus, Machine, Snapshot, Workspace};
    use std::time::Duration;

    fn agent(terminal: &str, workspace: &str) -> Agent {
        Agent {
            terminal_id: terminal.into(),
            pane_id: format!("{workspace}:p1"),
            workspace_id: workspace.into(),
            tab_id: format!("{workspace}:t1"),
            name: None,
            agent: None,
            title: None,
            agent_status: AgentStatus::Idle,
            completion_seq: None,
            cwd: None,
        }
    }

    fn view(online: Online, enabled: bool) -> MachineView {
        MachineView {
            machine: Machine {
                id: "m".into(),
                label: "box".into(),
                target: "me@box".into(),
                session: "default".into(),
                enabled,
            },
            snapshot: Snapshot {
                online,
                ..Default::default()
            },
            version_mismatch: None,
            needs_install: false,
        }
    }

    #[test]
    fn machines_state_for_maps_every_mark() {
        assert_eq!(state_for(Mark::Attention), State::Attention);
        assert_eq!(state_for(Mark::Working), State::Working);
        assert_eq!(state_for(Mark::Done), State::Done);
        assert_eq!(state_for(Mark::Idle), State::Idle);
    }

    #[test]
    fn machines_agent_label_prefers_name_then_title_then_kind_then_terminal() {
        let mut a = agent("term-1", "w1");
        assert_eq!(agent_label(&a), "term-1");
        a.agent = Some("claude".into());
        assert_eq!(agent_label(&a), "claude");
        a.title = Some("fix the build".into());
        assert_eq!(agent_label(&a), "fix the build");
        a.name = Some("builder".into());
        assert_eq!(agent_label(&a), "builder");
        // An empty name says nothing, so the next one is used.
        a.name = Some(String::new());
        assert_eq!(agent_label(&a), "fix the build");
    }

    #[test]
    fn machines_status_greys_every_machine_that_is_not_online() {
        let status = |online| machine_status(&view(online, true));
        assert_eq!(status(Online::Online), (false, None));
        assert_eq!(status(Online::Connecting), (true, None));
        assert_eq!(
            status(Online::Offline {
                reason: "no route".into(),
                retry_in: Duration::from_secs(4),
            }),
            (true, Some("· offline"))
        );
        assert_eq!(
            status(Online::NeedsSignIn {
                hint: "ssh me@box".into()
            }),
            (true, Some("· sign-in needed"))
        );
        assert_eq!(status(Online::HerdrMissing), (true, None));
        assert_eq!(status(Online::HerdrTooOld), (true, None));
    }

    #[test]
    fn machines_agents_follow_workspace_order_and_skip_disabled_machines() {
        let mut v = view(Online::Online, true);
        v.snapshot.workspaces = vec![
            Workspace {
                workspace_id: "w1".into(),
                label: "one".into(),
            },
            Workspace {
                workspace_id: "w2".into(),
                label: "two".into(),
            },
        ];
        v.snapshot.agents = vec![
            agent("c", "w2"),
            agent("x", "gone"),
            agent("a", "w1"),
            agent("b", "w1"),
        ];
        let order: Vec<_> = agents(&v).iter().map(|a| a.terminal_id.as_str()).collect();
        // Within a workspace herdr's order holds; an unknown one goes last.
        assert_eq!(order, ["a", "b", "c", "x"]);
        assert!(listed(&v));
        assert!(!listed(&view(Online::Online, false)));
    }
}
