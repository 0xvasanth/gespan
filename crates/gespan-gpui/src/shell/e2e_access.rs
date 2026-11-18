//! What the end-to-end harness (`crate::e2e`) reaches inside the shell.
use super::*;

impl Shell {
    pub(crate) fn e2e_store(&self) -> Database {
        self.store.clone()
    }
    pub(crate) fn e2e_session(&self, id: i64) -> Option<Entity<Conversation>> {
        self.sessions.get(&id).cloned()
    }
    pub(crate) fn e2e_sessions(&self) -> Vec<i64> {
        self.sessions.keys().copied().collect()
    }
    /// The sidebar's New chat.
    pub(crate) fn e2e_create(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.create(kind, cx);
    }
    /// Who started `id`, through `sessions_start`.
    pub(crate) fn e2e_link(&self, id: i64) -> Option<gespan_core::orchestrator::tree::Link> {
        self.session_tree.link(id).copied()
    }
    /// The sidebar's rows of sessions shown under the one that started them.
    pub(crate) fn e2e_child_rows(&mut self, cx: &mut Context<Self>) -> Vec<(i64, u8)> {
        self.refresh_sidebar_structure(cx);
        self.sidebar_list.child_rows()
    }
    /// Delivers wake reports saved in the store, as a starting app does.
    pub(crate) fn e2e_resume_wake_reports(&mut self, cx: &mut Context<Self>) {
        self.resume_mail(cx);
    }
    /// The sidebar's Delete.
    pub(crate) fn e2e_delete(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_session(id, window, cx);
    }
    /// The labels of a session's sidebar menu.
    pub(crate) fn e2e_menu(&self, id: i64, cx: &App) -> Vec<(String, String)> {
        self.session_menu_items(id, cx)
            .into_iter()
            .filter_map(|item| match item {
                crate::popover::MenuItem::Action(action) => {
                    Some((action.id.to_string(), action.label.to_string()))
                }
                _ => None,
            })
            .collect()
    }
    /// Picks an item of a session's sidebar menu.
    pub(crate) fn e2e_run_menu(
        &mut self,
        id: i64,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_session_menu(id, action, window, cx);
    }
    /// The caller's starts still being made.
    pub(crate) fn e2e_starting(&self, caller: i64) -> usize {
        self.starting.get(&caller).copied().unwrap_or(0)
    }
    /// Reaches herdr machines through `remote` instead of ssh.
    pub(crate) fn e2e_set_remote(&mut self, remote: super::orchestrator_remote::RemoteFor) {
        self.remote_for = remote;
    }
}
