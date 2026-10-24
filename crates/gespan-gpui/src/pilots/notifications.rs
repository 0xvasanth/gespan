use super::*;

/// Forward request-level changes even when the attention boolean is unchanged.
pub(crate) struct NotificationChanged;
impl EventEmitter<NotificationChanged> for Pilots {}

/// Capture before dispatch starts another queued turn or a finished run is
/// removed. Consumers need no live entity borrow to deliver this completion.
pub(crate) struct NotificationFinished {
    pub summary: Session,
    pub body: String,
    pub habit: bool,
    pub was_cancelled: bool,
}
impl EventEmitter<NotificationFinished> for Pilots {}

impl Pilots {
    pub(crate) fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.failed_reports.clear();
        cx.notify();
    }

    pub(crate) fn all_notification_chats(&self, cx: &App) -> Vec<Entity<Conversation>> {
        let mut seen = HashSet::new();
        self.chats
            .values()
            .chain(self.running.values().map(|run| &run.view))
            .filter(|view| seen.insert(view.read(cx).summary.id))
            .cloned()
            .collect()
    }
    pub(crate) fn active_notification_session(&self, cx: &App) -> Option<i64> {
        if self.settings.is_some() {
            return None;
        }
        self.chat().map(|chat| chat.read(cx).summary.id)
    }
    pub(crate) fn notification_entity(&self, id: i64, cx: &App) -> Option<Entity<Conversation>> {
        self.chats
            .values()
            .chain(self.running.values().map(|run| &run.view))
            .find(|view| view.read(cx).summary.id == id)
            .cloned()
    }
    /// Main Pilot chats stay in their workspace. A habit run has no dedicated
    /// Pilot-chat surface: return false so the host opens that exact session.
    pub(crate) fn open_notification_session(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.prefs.pilots {
            return false;
        }
        let Some(pilot) = self.roster.of_session(id).map(|pilot| pilot.id.clone()) else {
            return false;
        };
        self.settings = None;
        self.route = None;
        self.open(&pilot, window, cx);
        true
    }
}
