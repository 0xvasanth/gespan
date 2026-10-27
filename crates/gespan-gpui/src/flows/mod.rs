//! Flows in Gespan Native. `gespan_core::flow` owns the rules; this is the
//! host that runs each agent visit as a background session, and the view.
pub(crate) mod diagram;
mod editor;
mod host;
mod run_view;
mod ui;
mod view;

pub(crate) use host::{Host, Probe, SessionDriver, Settled, fill_worktree};
pub(crate) use run_view::{duration, visit_status_label};
pub(crate) use ui::status_color;
pub(crate) use view::{Event, FlowsView, Waiting};
