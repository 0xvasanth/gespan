//! herdr machines in Gespan. `gespan_core::machines` runs herdr and keeps the
//! connections; this holds the app-wide list the UI draws from.
use gespan_core::machines::{MachineView, Registry};
use gpui::{App, AppContext, Entity, Global};
use std::sync::Arc;

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
    fn apply(
        &mut self,
        views: Vec<MachineView>,
        list_error: Option<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.loaded && self.views == views && self.list_error == list_error {
            return;
        }
        self.views = views;
        self.list_error = list_error;
        self.loaded = true;
        cx.notify();
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
            let (views, error) = cx
                .background_executor()
                .spawn(async move { (registry.machines(), registry.list_error()) })
                .await;
            if weak.update(cx, |m, cx| m.apply(views, error, cx)).is_err() {
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
        m.apply(views, list_error, cx)
    });
}
