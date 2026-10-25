use super::*;
use gespan_core::agent::Settings;
pub(crate) struct PilotSession {
    pub id: i64,
    pub title: String,
    pub cwd: String,
    pub busy: bool,
    pub has_messages: bool,
    pub archived: bool,
    pub updated_at: u64,
    pub needs_input: bool,
    pub branch: Option<String>,
    pub model: SharedString,
}
impl Shell {
    pub(crate) fn close_pilot_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_open(false, window, cx);
    }
    #[cfg(test)]
    pub(crate) fn pilot_manager(&self) -> Entity<crate::pilots::Pilots> {
        self.pilots.as_ref().unwrap().clone()
    }
    #[cfg(all(target_os = "macos", not(test)))]
    pub(crate) fn pilot_quick_compose(&mut self, cx: &mut Context<Self>) {
        self.create(Kind::Codex, cx);
    }
    pub(super) fn pilot_active(&self, cx: &App) -> bool {
        self.pilots
            .as_ref()
            .is_some_and(|m| m.read(cx).active.is_some())
    }
    pub(crate) fn pilot_session_views(&self, cx: &App) -> Vec<PilotSession> {
        self.order
            .iter()
            .map(|&id| {
                let metadata = self.session_metadata(id, cx);
                let live = self.sessions.get(&id).map(|v| v.read(cx));
                PilotSession {
                    id,
                    title: metadata.summary.title.clone(),
                    cwd: metadata.summary.cwd.clone(),
                    busy: metadata.busy || live.is_some_and(|v| v.pilot_external_pending()),
                    has_messages: metadata.summary.has_messages,
                    archived: metadata.summary.archived,
                    updated_at: metadata.summary.updated_at,
                    needs_input: metadata.needs_approval(),
                    branch: live.and_then(|v| v.branch_name()).map(str::to_owned),
                    model: live.map(|v| v.pilot_model_label()).unwrap_or_default(),
                }
            })
            .collect()
    }
    pub(crate) fn pilot_session_entity(&self, id: i64) -> Option<Entity<Conversation>> {
        self.sessions.get(&id).cloned()
    }
    pub(crate) fn pilot_ensure_session(
        &mut self,
        id: i64,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Conversation>> {
        self.ensure_session(id, cx)
    }
    pub(crate) fn pilot_install(
        &mut self,
        summary: Session,
        settings: Settings,
        prompt: String,
        draft: bool,
        cx: &mut Context<Self>,
    ) {
        let id = summary.id;
        self.add_loaded(summary, cx);
        let view = self.sessions[&id].clone();
        view.update(cx, |v, cx| {
            v.install_transfer(settings, (!draft).then(|| prompt.clone()), cx);
            if draft {
                v.input.update(cx, |i, cx| i.set_text(prompt, cx));
            }
        });
        self.invalidate_chrome(cx);
    }
    pub(crate) fn pilot_groups(&self, project: &str) -> gespan_core::session_groups::Groups {
        self.groups.get(project).cloned().unwrap_or_default()
    }
    /// A project's groups if the sidebar has loaded them.
    pub(crate) fn pilot_loaded_groups(
        &self,
        project: &str,
    ) -> Option<gespan_core::session_groups::Groups> {
        self.groups.get(project).cloned()
    }
    pub(crate) fn pilot_apply_groups(
        &mut self,
        project: String,
        groups: gespan_core::session_groups::Groups,
        cx: &mut Context<Self>,
    ) {
        self.groups.insert(project, groups);
        self.invalidate_chrome(cx);
    }
    pub(crate) fn pilot_remove(
        &mut self,
        id: i64,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if delete {
            self.delete_session(id, window, cx);
        } else if self.session_summaries.contains_key(&id)
            && !self.session_metadata(id, cx).summary.archived
        {
            self.toggle_archive(id, cx);
        }
    }
}
