//! Flows in Gespan. `gespan_core::flow` owns the rules; this is the
//! host that runs each agent visit as a background session, and the view.
use gpui::App;

mod editor;
mod host;
mod run_view;
mod ui;
mod view;

pub(crate) use host::{Host, Probe, SessionDriver, Settled, fill_worktree};
pub(crate) use run_view::{duration, visit_status_label};
pub(crate) use ui::status_color;
pub(crate) use view::{Event, FlowsView, Waiting};

/// Whether people can reach the Flows view and its navigation entries. Set
/// this to `true` to bring them back. A Pilot's SOP Roadmap tab shows its
/// Flows either way, so people can see the workflow their Pilot follows.
/// Only the UI hangs off this: Flows, their runs and the `flows.*` actions
/// Pilots use keep working either way.
pub(crate) const FLOWS_VISIBLE: bool = false;

/// Every way into the Flows UI asks this.
pub(crate) fn visible(cx: &App) -> bool {
    FLOWS_VISIBLE || shown_for_test(cx)
}

#[cfg(not(test))]
fn shown_for_test(_: &App) -> bool {
    false
}

#[cfg(test)]
struct ShownForTest;
#[cfg(test)]
impl gpui::Global for ShownForTest {}

#[cfg(test)]
fn shown_for_test(cx: &App) -> bool {
    cx.has_global::<ShownForTest>()
}

/// Shows the Flows UI in this test's app, so its tests go on covering it.
#[cfg(test)]
pub(crate) fn show_for_test(cx: &mut App) {
    cx.set_global(ShownForTest);
}
