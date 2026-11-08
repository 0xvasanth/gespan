//! herdr machines in Gespan. `gespan_core::machines` runs herdr and keeps the
//! connections; this holds the app-wide list the UI draws from.
pub(crate) mod sidebar;

use crate::terminal::Terminal;
use gespan_core::machines::{Agent, AgentStatus, Machine, MachineView, Mark, Online, Registry};
use gpui::{App, AppContext, Entity, Global, WeakEntity};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Looks up the terminal command that opens one agent, given the machine id
/// and the agent's pane. May block, so it runs off the UI thread.
pub(crate) type Attach = Arc<dyn Fn(&Machine, &str) -> Option<String> + Send + Sync>;

/// A machine id and one of its agents' terminal id.
type Key = (String, String);

fn key(machine_id: &str, terminal_id: &str) -> Key {
    (machine_id.to_owned(), terminal_id.to_owned())
}

/// Whether people can reach the Machines UI. herdr's commands are built for
/// POSIX shells, so it stays hidden on Windows.
pub(crate) const MACHINES_VISIBLE: bool = cfg!(not(windows));

/// Every way into the Machines UI asks this. Tests show it only when they
/// call [`show_for_test`], on every platform.
pub(crate) fn visible(cx: &App) -> bool {
    #[cfg(not(test))]
    {
        let _ = cx;
        MACHINES_VISIBLE
    }
    #[cfg(test)]
    {
        cx.has_global::<ShownForTest>()
    }
}

#[cfg(test)]
struct ShownForTest;
#[cfg(test)]
impl Global for ShownForTest {}

/// Shows the Machines UI in this test's app.
#[cfg(test)]
pub(crate) fn show_for_test(cx: &mut App) {
    cx.set_global(ShownForTest);
}

/// Why an agent on a machine is worth a desktop notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineNotice {
    NeedsAttention,
    Finished,
}

/// An agent that changed in a way worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub machine_id: String,
    pub machine_label: String,
    pub terminal_id: String,
    pub agent_label: String,
    pub notice: MachineNotice,
}

/// What changed between two snapshots of one machine's agents, per
/// terminal. An agent that blocked, or a new agent that is already blocked,
/// needs attention; one that completed a turn the user has not seen has
/// finished. A new agent that is idle only says what it was doing before.
/// `seen` is the last turn the user saw per terminal.
pub(crate) fn transitions(
    before: &[Agent],
    after: &[Agent],
    seen: impl Fn(&str) -> Option<u64>,
) -> Vec<(String, MachineNotice)> {
    after
        .iter()
        .filter_map(|agent| {
            let prev = before.iter().find(|a| a.terminal_id == agent.terminal_id);
            let notice = match agent.agent_status {
                AgentStatus::Blocked
                    if prev.is_none_or(|p| p.agent_status != AgentStatus::Blocked) =>
                {
                    MachineNotice::NeedsAttention
                }
                AgentStatus::Idle | AgentStatus::Done => {
                    let turn = agent.completion_seq.unwrap_or(0);
                    let was = prev?.completion_seq.unwrap_or(0);
                    let unseen = seen(&agent.terminal_id).unwrap_or(0);
                    if turn > was && turn > unseen {
                        MachineNotice::Finished
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            Some((agent.terminal_id.clone(), notice))
        })
        .collect()
}

/// The notices for one machine's new view. With no earlier view, or one
/// from before the machine was online, the agents are only being learned
/// about, so nothing is announced for how they already are.
fn view_transitions(
    before: Option<&MachineView>,
    after: &MachineView,
    seen: impl Fn(&str) -> Option<u64>,
) -> Vec<(String, MachineNotice)> {
    if after.snapshot.online != Online::Online {
        return Vec::new();
    }
    match before.filter(|b| b.snapshot.online == Online::Online) {
        Some(before) => transitions(&before.snapshot.agents, &after.snapshot.agents, seen),
        None => Vec::new(),
    }
}

/// The saved machines as last read from the registry.
pub(crate) struct Machines {
    registry: Option<Arc<Registry>>,
    views: Vec<MachineView>,
    list_error: Option<String>,
    /// The first read has finished.
    loaded: bool,
    /// Whether this build found a herdr to run; `None` until checked.
    herdr_found: Option<bool>,
    /// The last turn seen per agent, read along with the list.
    seen: HashMap<Key, u64>,
    /// The attach tabs this app opened, per agent.
    attach_tabs: HashMap<Key, WeakEntity<Terminal>>,
    /// Agents whose attach command is being looked up.
    attaching: HashSet<Key>,
    attach: Attach,
    #[cfg(test)]
    pub(crate) reloads: usize,
    /// The machines asked to retry, in order.
    #[cfg(test)]
    pub(crate) refreshes: Vec<String>,
}

impl gpui::EventEmitter<Notice> for Machines {}

impl Machines {
    pub(crate) fn views(&self) -> &[MachineView] {
        &self.views
    }
    pub(crate) fn list_error(&self) -> Option<&str> {
        self.list_error.as_deref()
    }
    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }
    pub(crate) fn herdr_found(&self) -> Option<bool> {
        self.herdr_found
    }
    pub(crate) fn set_herdr_found(&mut self, found: bool, cx: &mut gpui::Context<Self>) {
        if self.herdr_found != Some(found) {
            self.herdr_found = Some(found);
            cx.notify();
        }
    }
    /// The agent's mark, from the turns seen as last read.
    pub(crate) fn mark(&self, machine_id: &str, agent: &Agent) -> Option<Mark> {
        let seen = self.seen_seq(machine_id, &agent.terminal_id);
        gespan_core::machines::mark(agent, seen)
    }
    pub(crate) fn seen_seq(&self, machine_id: &str, terminal_id: &str) -> Option<u64> {
        self.seen.get(&key(machine_id, terminal_id)).copied()
    }
    /// The attach tab opened for this agent, if any; it may have closed.
    pub(crate) fn attach_tab(
        &self,
        machine_id: &str,
        terminal_id: &str,
    ) -> Option<&WeakEntity<Terminal>> {
        self.attach_tabs.get(&key(machine_id, terminal_id))
    }
    pub(crate) fn set_attach_tab(
        &mut self,
        machine_id: &str,
        terminal_id: &str,
        terminal: WeakEntity<Terminal>,
    ) {
        self.attach_tabs.retain(|_, tab| tab.upgrade().is_some());
        self.attach_tabs
            .insert(key(machine_id, terminal_id), terminal);
    }
    /// Starts looking up an agent's attach command; `None` while one is
    /// already under way.
    pub(crate) fn begin_attach(&mut self, machine_id: &str, terminal_id: &str) -> Option<Attach> {
        self.attaching
            .insert(key(machine_id, terminal_id))
            .then(|| self.attach.clone())
    }
    pub(crate) fn end_attach(&mut self, machine_id: &str, terminal_id: &str) {
        self.attaching.remove(&key(machine_id, terminal_id));
    }
    fn apply(
        &mut self,
        views: Vec<MachineView>,
        mut seen: HashMap<Key, u64>,
        list_error: Option<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        // A read that started before a turn was marked seen must not undo it.
        for (key, seq) in &mut seen {
            if let Some(local) = self.seen.get(key) {
                *seq = (*seq).max(*local);
            }
        }
        if self.loaded && self.views == views && self.seen == seen && self.list_error == list_error
        {
            return;
        }
        for view in views.iter().filter(|v| v.machine.enabled) {
            let id = &view.machine.id;
            let before = self.views.iter().find(|v| v.machine.id == *id);
            let notices = view_transitions(before, view, |terminal| {
                seen.get(&key(id, terminal)).copied()
            });
            for (terminal_id, notice) in notices {
                let Some(agent) = view
                    .snapshot
                    .agents
                    .iter()
                    .find(|a| a.terminal_id == terminal_id)
                else {
                    continue;
                };
                cx.emit(Notice {
                    machine_id: id.clone(),
                    machine_label: view.machine.label.clone(),
                    agent_label: sidebar::agent_label(agent).to_owned(),
                    terminal_id,
                    notice,
                });
            }
        }
        self.views = views;
        self.seen = seen;
        self.list_error = list_error;
        self.loaded = true;
        cx.notify();
    }
}

/// The last turn seen for every listed agent. Locks the seen store, so call
/// it off the UI thread.
fn seen_by_agent(registry: &Registry, views: &[MachineView]) -> HashMap<Key, u64> {
    views
        .iter()
        .flat_map(|view| {
            let id = &view.machine.id;
            view.snapshot.agents.iter().filter_map(move |agent| {
                let seq = registry.seen_seq(id, &agent.terminal_id)?;
                Some((key(id, &agent.terminal_id), seq))
            })
        })
        .collect()
}

/// The user opened this agent's turn `completion_seq`. Shown at once; the
/// registry records it off the UI thread.
pub(crate) fn mark_seen(machine_id: &str, terminal_id: &str, completion_seq: u64, cx: &mut App) {
    let Some(machines) = get(cx) else {
        return;
    };
    let registry = machines.update(cx, |m, cx| {
        let seen = m.seen.entry(key(machine_id, terminal_id)).or_default();
        if *seen != completion_seq {
            *seen = completion_seq;
            cx.notify();
        }
        m.registry.clone()
    });
    if let Some(registry) = registry {
        let (machine_id, terminal_id) = (machine_id.to_owned(), terminal_id.to_owned());
        cx.background_executor()
            .spawn(async move { registry.mark_seen(&machine_id, &terminal_id, completion_seq) })
            .detach();
    }
}

struct Shared(Entity<Machines>);
impl Global for Shared {}

pub(crate) fn get(cx: &App) -> Option<Entity<Machines>> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

/// Starts the registry and keeps the list current. Call once; does nothing
/// where the Machines UI is hidden.
pub(crate) fn install(cx: &mut App) {
    if !MACHINES_VISIBLE {
        return;
    }
    // One pending signal is enough: the read after it sees every change.
    let (changed, changes) = async_channel::bounded::<()>(1);
    let registry = Registry::start(move || {
        let _ = changed.try_send(());
    });
    let machines = cx.new(|_| Machines {
        registry: Some(registry.clone()),
        views: Vec::new(),
        list_error: None,
        loaded: false,
        herdr_found: None,
        seen: HashMap::new(),
        attach_tabs: HashMap::new(),
        attaching: HashSet::new(),
        attach: Arc::new(|m, pane| Some(gespan_core::machines::attach_command(m, pane))),
        #[cfg(test)]
        reloads: 0,
        #[cfg(test)]
        refreshes: Vec::new(),
    });
    cx.set_global(Shared(machines.clone()));
    let found = cx
        .background_executor()
        .spawn(async { gespan_core::machines::herdr_binary().is_some() });
    let weak = machines.downgrade();
    cx.spawn(async move |cx| {
        let found = found.await;
        let _ = weak.update(cx, |m, cx| m.set_herdr_found(found, cx));
    })
    .detach();
    let weak = machines.downgrade();
    cx.spawn(async move |cx| {
        // A change for a machine removed meanwhile only re-reads the list.
        while changes.recv().await.is_ok() {
            let registry = registry.clone();
            let (views, seen, error) = cx
                .background_executor()
                .spawn(async move {
                    let views = registry.machines();
                    let seen = seen_by_agent(&registry, &views);
                    (views, seen, registry.list_error())
                })
                .await;
            if weak
                .update(cx, |m, cx| m.apply(views, seen, error, cx))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

/// Re-reads herdr's saved machines off the UI thread. The list updates when
/// the registry reports the change.
pub(crate) fn reload(cx: &mut App) {
    let Some(machines) = get(cx) else {
        return;
    };
    #[cfg(test)]
    machines.update(cx, |m, _| m.reloads += 1);
    if let Some(registry) = machines.read(cx).registry.clone() {
        cx.background_executor()
            .spawn(async move { registry.reload() })
            .detach();
    }
}

/// Retries one machine's connection now, off the UI thread.
pub(crate) fn refresh(machine_id: &str, cx: &mut App) {
    let Some(machines) = get(cx) else {
        return;
    };
    #[cfg(test)]
    machines.update(cx, |m, _| m.refreshes.push(machine_id.to_owned()));
    if let Some(registry) = machines.read(cx).registry.clone() {
        let id = machine_id.to_owned();
        cx.background_executor()
            .spawn(async move { registry.refresh(&id) })
            .detach();
    }
}

/// A list with no registry behind it, for tests.
#[cfg(test)]
pub(crate) fn install_for_test(views: Vec<MachineView>, cx: &mut App) -> Entity<Machines> {
    let machines = cx.new(|_| Machines {
        registry: None,
        views,
        list_error: None,
        loaded: true,
        herdr_found: Some(true),
        seen: HashMap::new(),
        attach_tabs: HashMap::new(),
        attaching: HashSet::new(),
        attach: Arc::new(|m, pane| Some(format!("ssh -t -- '{}' 'attach {pane}'", m.target))),
        reloads: 0,
        refreshes: Vec::new(),
    });
    cx.set_global(Shared(machines.clone()));
    machines
}

/// Replaces the test list, as a registry change would.
#[cfg(test)]
pub(crate) fn set_for_test(
    machines: &Entity<Machines>,
    views: Vec<MachineView>,
    list_error: Option<String>,
    cx: &mut App,
) {
    machines.update(cx, |m, cx| {
        m.loaded = false;
        let seen = m.seen.clone();
        m.apply(views, seen, list_error, cx)
    });
}

/// Replaces how the test list looks up attach commands.
#[cfg(test)]
pub(crate) fn set_attach_for_test(
    machines: &Entity<Machines>,
    attach: impl Fn(&Machine, &str) -> Option<String> + Send + Sync + 'static,
    cx: &mut App,
) {
    machines.update(cx, |m, _| m.attach = Arc::new(attach));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::machines::Snapshot;
    use std::{cell::RefCell, rc::Rc};

    fn agent(terminal: &str, status: AgentStatus, seq: Option<u64>) -> Agent {
        Agent {
            terminal_id: terminal.into(),
            pane_id: format!("w1:p-{terminal}"),
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            name: Some(format!("{terminal} agent")),
            agent: None,
            title: None,
            agent_status: status,
            completion_seq: seq,
            cwd: None,
        }
    }

    fn view(online: Online, agents: Vec<Agent>) -> MachineView {
        MachineView {
            machine: Machine {
                id: "m1".into(),
                label: "box".into(),
                target: "me@box".into(),
                session: "default".into(),
                enabled: true,
            },
            snapshot: Snapshot {
                online,
                version: None,
                workspaces: Vec::new(),
                agents,
            },
            version_mismatch: None,
            needs_install: false,
        }
    }

    fn none_seen(_: &str) -> Option<u64> {
        None
    }

    #[test]
    fn notifications_working_to_blocked_needs_attention() {
        let before = [agent("a", AgentStatus::Working, None)];
        let after = [agent("a", AgentStatus::Blocked, None)];
        assert_eq!(
            transitions(&before, &after, none_seen),
            [("a".to_owned(), MachineNotice::NeedsAttention)]
        );
    }

    #[test]
    fn notifications_staying_blocked_does_not_repeat() {
        let blocked = [agent("a", AgentStatus::Blocked, None)];
        assert!(transitions(&blocked, &blocked, none_seen).is_empty());
    }

    #[test]
    fn notifications_a_new_turn_finishes_but_a_steady_idle_does_not() {
        let before = [agent("a", AgentStatus::Working, Some(1))];
        let after = [agent("a", AgentStatus::Idle, Some(2))];
        assert_eq!(
            transitions(&before, &after, |_| Some(1)),
            [("a".to_owned(), MachineNotice::Finished)]
        );
        // A first turn: herdr reports no `completion_seq` while working.
        let first = [agent("a", AgentStatus::Working, None)];
        assert_eq!(
            transitions(&first, &after, none_seen),
            [("a".to_owned(), MachineNotice::Finished)]
        );
        // Done counts the same as Idle.
        let done = [agent("a", AgentStatus::Done, Some(2))];
        assert_eq!(transitions(&before, &done, |_| Some(1)).len(), 1);
        // Nothing moved.
        assert!(transitions(&after, &after, none_seen).is_empty());
        let idle = [agent("a", AgentStatus::Idle, None)];
        assert!(transitions(&idle, &idle, none_seen).is_empty());
    }

    #[test]
    fn notifications_a_turn_already_seen_is_not_announced() {
        let before = [agent("a", AgentStatus::Working, Some(1))];
        let after = [agent("a", AgentStatus::Idle, Some(2))];
        // The user opened turn 2 before this snapshot arrived.
        assert!(transitions(&before, &after, |_| Some(2)).is_empty());
        assert!(transitions(&before, &after, |_| Some(5)).is_empty());
    }

    #[test]
    fn notifications_working_and_unknown_agents_stay_quiet() {
        let before = [agent("a", AgentStatus::Idle, Some(1))];
        let after = [
            agent("a", AgentStatus::Working, Some(1)),
            agent("b", AgentStatus::Unknown, None),
        ];
        assert!(transitions(&before, &after, none_seen).is_empty());
    }

    #[test]
    fn notifications_a_new_agent_is_announced_only_when_blocked() {
        let after = [
            agent("blocked", AgentStatus::Blocked, None),
            agent("idle", AgentStatus::Idle, Some(4)),
            agent("working", AgentStatus::Working, None),
        ];
        let before = [agent("old", AgentStatus::Idle, None)];
        assert_eq!(
            transitions(&before, &after, none_seen),
            [("blocked".to_owned(), MachineNotice::NeedsAttention)]
        );
    }

    #[test]
    fn notifications_agents_are_matched_by_terminal_not_position() {
        let before = [
            agent("a", AgentStatus::Blocked, None),
            agent("b", AgentStatus::Working, None),
        ];
        let after = [
            agent("b", AgentStatus::Blocked, None),
            agent("a", AgentStatus::Blocked, None),
        ];
        assert_eq!(
            transitions(&before, &after, none_seen),
            [("b".to_owned(), MachineNotice::NeedsAttention)]
        );
    }

    #[test]
    fn notifications_the_first_snapshot_of_a_connection_announces_nothing() {
        let blocked = view(
            Online::Online,
            vec![
                agent("a", AgentStatus::Blocked, None),
                agent("b", AgentStatus::Idle, Some(3)),
            ],
        );
        // Never seen before.
        assert!(view_transitions(None, &blocked, none_seen).is_empty());
        // Seen only while connecting or after the link dropped.
        for online in [Online::Connecting, Online::HerdrMissing] {
            let earlier = view(online, Vec::new());
            assert!(view_transitions(Some(&earlier), &blocked, none_seen).is_empty());
        }
        let offline = view(
            Online::Offline {
                reason: "no route".into(),
                retry_in: std::time::Duration::from_secs(4),
            },
            vec![agent("a", AgentStatus::Working, None)],
        );
        assert!(view_transitions(Some(&offline), &blocked, none_seen).is_empty());
    }

    #[test]
    fn notifications_a_machine_that_was_online_announces_a_brand_new_blocked_agent() {
        let earlier = view(Online::Online, vec![agent("a", AgentStatus::Blocked, None)]);
        let later = view(
            Online::Online,
            vec![
                agent("a", AgentStatus::Blocked, None),
                agent("new", AgentStatus::Blocked, None),
            ],
        );
        assert_eq!(
            view_transitions(Some(&earlier), &later, none_seen),
            [("new".to_owned(), MachineNotice::NeedsAttention)]
        );
        // A machine that drops offline announces nothing.
        let dropped = view(
            Online::Offline {
                reason: "no route".into(),
                retry_in: std::time::Duration::from_secs(4),
            },
            Vec::new(),
        );
        assert!(view_transitions(Some(&later), &dropped, none_seen).is_empty());
    }

    #[gpui::test]
    fn notifications_a_registry_change_emits_the_notice_with_labels(cx: &mut gpui::TestAppContext) {
        let machines = cx.update(|cx| {
            install_for_test(
                vec![view(
                    Online::Online,
                    vec![
                        agent("a", AgentStatus::Working, None),
                        agent("b", AgentStatus::Working, Some(1)),
                    ],
                )],
                cx,
            )
        });
        let got = Rc::new(RefCell::new(Vec::new()));
        let sink = got.clone();
        let _sub = cx.update(|cx| {
            cx.subscribe(&machines, move |_, notice: &Notice, _| {
                sink.borrow_mut().push(notice.clone())
            })
        });
        // `b`'s turn 2 is seen by the time the change arrives.
        cx.update(|cx| mark_seen("m1", "b", 2, cx));
        cx.update(|cx| {
            set_for_test(
                &machines,
                vec![view(
                    Online::Online,
                    vec![
                        agent("a", AgentStatus::Blocked, None),
                        agent("b", AgentStatus::Idle, Some(2)),
                    ],
                )],
                None,
                cx,
            )
        });
        assert_eq!(
            *got.borrow(),
            [Notice {
                machine_id: "m1".into(),
                machine_label: "box".into(),
                terminal_id: "a".into(),
                agent_label: "a agent".into(),
                notice: MachineNotice::NeedsAttention,
            }]
        );
    }
}
