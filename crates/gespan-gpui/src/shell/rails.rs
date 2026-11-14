//! Orchestrated workers belong to their lead: the sidebar never lists them
//! on their own.
use super::*;

impl Shell {
    /// Orchestrated workers live under their lead, never in the list.
    pub(super) fn orchestration_worker(&self, id: i64) -> bool {
        self.orchestration_summaries
            .values()
            .any(|s| s.tasks.iter().any(|t| t.session == Some(id)))
    }
}
