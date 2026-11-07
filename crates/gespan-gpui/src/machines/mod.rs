//! herdr machines in Gespan. `gespan_core::machines` runs herdr and keeps the
//! connections; this holds the app-wide list the UI draws from.
pub(crate) mod sidebar;

use crate::terminal::Terminal;
use gespan_core::machines::{Agent, MachineView, Mark, Registry};
use gpui::{App, AppContext, Entity, Global, WeakEntity};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Looks up the terminal command that opens one agent, given the machine id
/// and the agent's pane. May block, so it runs off the UI thread.
pub(crate) type Attach = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

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
}

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
        attach: Arc::new(gespan_core::machines::attach_command),
        #[cfg(test)]
        reloads: 0,
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
        attach: Arc::new(|id, pane| Some(format!("herdr --machine '{id}' agent attach '{pane}'"))),
        reloads: 0,
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
    attach: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static,
    cx: &mut App,
) {
    machines.update(cx, |m, _| m.attach = Arc::new(attach));
}
