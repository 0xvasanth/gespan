//! Session turns as board events: one "started" when a turn begins and at
//! most one terminal event when it settles, for the session and, when a Pilot
//! is behind the turn, for that Pilot. Mirrors
//! `src/features/board/model/linkedTurns.ts`; it does no I/O, callers dispatch
//! the returned calls with `auto_move::on_linked_work`.
use super::auto_move::LinkedWorkEvent;
use std::collections::HashMap;

/// A board call to make: link kind, link id, event.
pub type LinkedCall = (&'static str, String, LinkedWorkEvent);

/// The Pilot a turn counts for: the Pilot whose own chat the session is, else
/// the Pilot a habit run works for, else the Pilot that spawned or prompted it.
pub fn turn_pilot_id(
    own: Option<&str>,
    habit: Option<&str>,
    spawned_by: Option<&str>,
) -> Option<String> {
    own.or(habit).or(spawned_by).map(str::to_string)
}

/// Open turns, keyed by session and turn number, to the Pilot seen at start.
#[derive(Default)]
pub struct LinkedTurns {
    open: HashMap<(String, u64), Option<String>>,
}

impl LinkedTurns {
    pub fn started(
        &mut self,
        session_key: &str,
        gen: u64,
        pilot_id: Option<String>,
    ) -> Vec<LinkedCall> {
        let key = (session_key.to_string(), gen);
        if self.open.contains_key(&key) {
            return Vec::new();
        }
        let calls = calls(session_key, pilot_id.as_deref(), LinkedWorkEvent::Started);
        self.open.insert(key, pilot_id);
        calls
    }

    /// A finished turn: `ok` is false when it ended with an error.
    pub fn settled(&mut self, session_key: &str, gen: u64, ok: bool) -> Vec<LinkedCall> {
        let Some(pilot_id) = self.open.remove(&(session_key.to_string(), gen)) else {
            return Vec::new();
        };
        let event = if ok {
            LinkedWorkEvent::Completed
        } else {
            LinkedWorkEvent::Failed
        };
        calls(session_key, pilot_id.as_deref(), event)
    }

    /// A turn the user stopped or a newer turn replaced reports nothing.
    pub fn cancelled(&mut self, session_key: &str, gen: u64) {
        self.open.remove(&(session_key.to_string(), gen));
    }

    /// The links whose work runs now: `("session", key)` for every session
    /// with an open turn and `("pilot", id)` for the Pilot behind it, sorted
    /// and without repeats. This is the running set board queries and link
    /// chips read.
    pub fn open_links(&self) -> Vec<(String, String)> {
        let mut links: Vec<(String, String)> = self
            .open
            .iter()
            .flat_map(|((session_key, _), pilot_id)| {
                std::iter::once(("session".to_string(), session_key.clone())).chain(
                    pilot_id
                        .iter()
                        .map(|pilot_id| ("pilot".to_string(), pilot_id.clone())),
                )
            })
            .collect();
        links.sort();
        links.dedup();
        links
    }
}

fn calls(session_key: &str, pilot_id: Option<&str>, event: LinkedWorkEvent) -> Vec<LinkedCall> {
    let mut calls = vec![("session", session_key.to_string(), event)];
    if let Some(pilot_id) = pilot_id {
        calls.push(("pilot", pilot_id.to_string(), event));
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::auto_move::LinkedWorkEvent::{Completed, Failed, Started};

    fn call(kind: &'static str, id: &str, event: LinkedWorkEvent) -> LinkedCall {
        (kind, id.to_string(), event)
    }

    #[test]
    fn reports_a_started_turn_once_for_the_session_only_when_it_has_no_pilot() {
        let mut turns = LinkedTurns::default();
        assert_eq!(
            turns.started("s-1", 1, None),
            vec![call("session", "s-1", Started)]
        );
    }

    #[test]
    fn also_reports_a_pilots_turn_for_the_pilot() {
        let mut turns = LinkedTurns::default();
        assert_eq!(
            turns.started("s-1", 1, Some("p-1".into())),
            vec![
                call("session", "s-1", Started),
                call("pilot", "p-1", Started)
            ]
        );
    }

    #[test]
    fn reports_a_successful_settle_as_completed_for_the_pilot_seen_at_start() {
        let mut turns = LinkedTurns::default();
        turns.started("s-1", 1, Some("p-1".into()));
        assert_eq!(
            turns.settled("s-1", 1, true),
            vec![
                call("session", "s-1", Completed),
                call("pilot", "p-1", Completed)
            ]
        );
    }

    #[test]
    fn reports_an_error_settle_as_failed() {
        let mut turns = LinkedTurns::default();
        turns.started("s-1", 1, None);
        assert_eq!(
            turns.settled("s-1", 1, false),
            vec![call("session", "s-1", Failed)]
        );
    }

    #[test]
    fn reports_nothing_when_a_turn_is_cancelled_and_stays_closed_afterward() {
        let mut turns = LinkedTurns::default();
        turns.started("s-1", 1, Some("p-1".into()));
        turns.cancelled("s-1", 1);
        assert!(turns.settled("s-1", 1, true).is_empty());
    }

    #[test]
    fn never_repeats_an_event_when_the_same_turn_starts_or_settles_again() {
        let mut turns = LinkedTurns::default();
        assert_eq!(turns.started("s-1", 1, None).len(), 1);
        assert!(turns.started("s-1", 1, None).is_empty());
        assert_eq!(turns.settled("s-1", 1, true).len(), 1);
        assert!(turns.settled("s-1", 1, true).is_empty());
    }

    #[test]
    fn ignores_a_settle_for_a_turn_that_never_started() {
        let mut turns = LinkedTurns::default();
        assert!(turns.settled("s-1", 7, true).is_empty());
    }

    #[test]
    fn reports_every_new_turn_of_the_same_session() {
        let mut turns = LinkedTurns::default();
        turns.started("s-1", 1, None);
        turns.settled("s-1", 1, true);
        assert_eq!(
            turns.started("s-1", 2, None),
            vec![call("session", "s-1", Started)]
        );
        assert_eq!(
            turns.settled("s-1", 2, false),
            vec![call("session", "s-1", Failed)]
        );
    }

    #[test]
    fn keeps_turns_of_different_sessions_and_superseded_turns_apart() {
        let mut turns = LinkedTurns::default();
        turns.started("s-1", 1, None);
        turns.started("s-2", 1, Some("p-2".into()));
        turns.started("s-1", 2, None);
        assert_eq!(turns.settled("s-2", 1, true).len(), 2);
        assert_eq!(
            turns.settled("s-1", 1, true),
            vec![call("session", "s-1", Completed)]
        );
        assert_eq!(
            turns.settled("s-1", 2, false),
            vec![call("session", "s-1", Failed)]
        );
    }

    #[test]
    fn open_links_lists_open_session_and_pilot_turns() {
        let link = |kind: &str, id: &str| (kind.to_string(), id.to_string());
        let mut turns = LinkedTurns::default();
        assert!(turns.open_links().is_empty());
        turns.started("s-1", 1, Some("p-1".into()));
        turns.started("s-2", 1, None);
        // A superseded turn and a second session of the same Pilot list once.
        turns.started("s-1", 2, Some("p-1".into()));
        turns.started("s-3", 1, Some("p-1".into()));
        assert_eq!(
            turns.open_links(),
            vec![
                link("pilot", "p-1"),
                link("session", "s-1"),
                link("session", "s-2"),
                link("session", "s-3"),
            ]
        );
        // Settled and cancelled turns stop counting; the Pilot stays while
        // any of its turns is open.
        turns.settled("s-1", 1, true);
        turns.cancelled("s-1", 2);
        turns.settled("s-2", 1, false);
        assert_eq!(
            turns.open_links(),
            vec![link("pilot", "p-1"), link("session", "s-3")]
        );
        turns.settled("s-3", 1, true);
        assert!(turns.open_links().is_empty());
    }

    #[test]
    fn pilot_is_the_one_whose_own_chat_the_session_is() {
        assert_eq!(
            turn_pilot_id(Some("own"), None, None).as_deref(),
            Some("own")
        );
    }

    #[test]
    fn pilot_is_the_one_a_habit_run_works_for() {
        assert_eq!(
            turn_pilot_id(None, Some("habit"), None).as_deref(),
            Some("habit")
        );
    }

    #[test]
    fn pilot_is_the_one_that_spawned_or_prompted_the_session() {
        assert_eq!(
            turn_pilot_id(None, None, Some("spawner")).as_deref(),
            Some("spawner")
        );
    }

    #[test]
    fn prefers_the_sessions_own_pilot_then_its_habits_then_its_spawner() {
        assert_eq!(
            turn_pilot_id(Some("own"), Some("habit"), Some("spawner")).as_deref(),
            Some("own")
        );
        assert_eq!(
            turn_pilot_id(None, Some("habit"), Some("spawner")).as_deref(),
            Some("habit")
        );
    }

    #[test]
    fn is_nobody_for_a_session_no_pilot_is_behind() {
        assert_eq!(turn_pilot_id(None, None, None), None);
    }

    #[test]
    fn a_habit_runs_turn_reports_to_its_pilot() {
        let mut turns = LinkedTurns::default();
        let pilot = turn_pilot_id(None, Some("p-habit"), None);
        turns.started("s-9", 1, pilot);
        assert_eq!(
            turns.settled("s-9", 1, true),
            vec![
                call("session", "s-9", Completed),
                call("pilot", "p-habit", Completed)
            ]
        );
    }
}
