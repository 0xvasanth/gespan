//! A Pilot's chat through a quit, a crash or an update restart. The shell
//! counts running Pilot chats with its own, records them while they run and
//! marks a cut-off turn at the next launch. A Pilot's chat loads only when
//! something opens it, so what to do with one that is not open yet waits
//! here until it loads.
use super::*;
use gespan_core::hold::Reason;

/// What a Pilot's chat does as soon as it loads.
#[derive(Clone, Copy)]
pub(super) enum OnLoad {
    /// Its turn was cut off by a quit: continue it once.
    ContinueAfterQuit,
    /// Resume took it off hold.
    Resume(Reason),
}

impl Pilots {
    /// The sessions of Pilot chats with a turn running, in roster order.
    pub(crate) fn running_chats(&self, cx: &App) -> Vec<i64> {
        self.roster
            .pilots
            .iter()
            .filter_map(|pilot| self.chats.get(&pilot.id))
            .map(|view| view.read(cx))
            .filter(|view| view.busy)
            .map(|view| view.summary.id)
            .collect()
    }

    /// Whether `session` is a Pilot's chat, as opposed to a habit run or a
    /// Flow review it started.
    pub(crate) fn is_pilot_chat(&self, session: i64) -> bool {
        self.roster.of_session(session).is_some()
    }

    /// Continue the chat a quit cut off, loading it first if it is not open.
    pub(crate) fn continue_after_quit(&mut self, session: i64, cx: &mut Context<Self>) {
        self.when_loaded(session, OnLoad::ContinueAfterQuit, cx);
    }

    /// Resume a held chat that has not been opened since a restart.
    pub(crate) fn resume_when_loaded(
        &mut self,
        session: i64,
        reason: Reason,
        cx: &mut Context<Self>,
    ) {
        self.when_loaded(session, OnLoad::Resume(reason), cx);
    }

    fn when_loaded(&mut self, session: i64, action: OnLoad, cx: &mut Context<Self>) {
        let Some(id) = self
            .roster
            .of_session(session)
            .map(|pilot| pilot.id.clone())
        else {
            return;
        };
        if let Some(view) = self.chats.get(&id).cloned() {
            apply(&view, action, cx);
            return;
        }
        self.on_load.insert(id.clone(), action);
        self.ensure_chat(&id, cx);
    }

    /// Called once a Pilot's chat has loaded.
    pub(super) fn chat_loaded(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(action) = self.on_load.remove(id) else {
            return;
        };
        if let Some(view) = self.chats.get(id).cloned() {
            apply(&view, action, cx);
        }
    }

    /// At launch: load each Pilot with messages waiting in its outbox, so they
    /// go out without anyone opening it. Dispatch still keeps them on hold.
    pub(super) fn deliver_queued(&mut self, cx: &mut Context<Self>) {
        if !self.prefs.pilots {
            return;
        }
        let waiting: Vec<String> = self
            .roster
            .pilots
            .iter()
            .filter(|pilot| !pilot.outbox.is_empty())
            .map(|pilot| pilot.id.clone())
            .collect();
        for id in waiting {
            self.ensure_chat(&id, cx);
        }
    }
}

fn apply(view: &Entity<Conversation>, action: OnLoad, cx: &mut App) {
    view.update(cx, |view, cx| match action {
        OnLoad::ContinueAfterQuit => view.continue_after_quit(cx),
        OnLoad::Resume(reason) => view.resume_from_hold_when_ready(reason, cx),
    });
}
