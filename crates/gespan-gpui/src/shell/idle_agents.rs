//! The desktop idle policy spares every conversation currently on screen.
use super::*;

impl Shell {
    pub(crate) fn keep_visible_idle_agents(&self, cx: &App) {
        let mut ids = Vec::new();
        if !self.app_view_open()
            && !self.pilot_active(cx)
            && let Some(tab) = self.workspace.active_tab()
        {
            ids.extend(
                self.workspace
                    .layout(tab)
                    .leaf_ids()
                    .into_iter()
                    .map(|pane| self.workspace.session_for_pane(pane)),
            );
        }
        if let Some(preview) = &self.session_preview {
            ids.push(preview.id);
        }
        if let Some(pilots) = &self.pilots {
            ids.extend(pilots.read(cx).visible_idle_sessions(cx));
        }
        gespan_core::provider::keep_idle_agents(ids);
    }
}
