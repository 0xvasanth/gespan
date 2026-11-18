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
}
