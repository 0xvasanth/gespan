//! Hold: stop every session at a safe point, keep it held across restarts,
//! and resume each one where it stopped.
use serde::{Deserialize, Serialize};

const KEY: &str = "hold";
const CONTINUE: &str = "Continue from where you left off.";

/// Holding until every tracked session reaches its safe point, then On hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Holding,
    OnHold,
}

/// Where a session stood when it was held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    /// No turn was running.
    Idle,
    /// Its turn was interrupted at a safe point and needs a continuation.
    Interrupted,
    /// Waiting on an approval or question, left pending.
    AwaitingInput,
    /// The agent had answered; only background tasks were still running.
    Background,
    /// The app quit before the live wait could be kept; the turn is gone.
    Restarted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub session: i64,
    /// `None` while the session is still settling.
    pub reason: Option<Reason>,
    /// Unix ms when it settled.
    pub held_at: u64,
    pub last_tool: Option<String>,
    pub background: Vec<String>,
    /// Pilot chats resume after their workers.
    pub pilot_chat: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoldState {
    pub phase: Phase,
    /// Unix ms when Hold was confirmed.
    pub since: u64,
    pub sessions: Vec<Held>,
}

impl HoldState {
    pub fn new(now: u64) -> Self {
        Self {
            phase: Phase::Holding,
            since: now,
            sessions: Vec::new(),
        }
    }
    pub fn track(&mut self, session: i64, pilot_chat: bool) {
        if self.sessions.iter().any(|s| s.session == session) {
            return;
        }
        self.sessions.push(Held {
            session,
            reason: None,
            held_at: 0,
            last_tool: None,
            background: Vec::new(),
            pilot_chat,
        });
    }
    pub fn settle(
        &mut self,
        session: i64,
        reason: Reason,
        now: u64,
        last_tool: Option<String>,
        background: Vec<String>,
    ) {
        if let Some(held) = self.sessions.iter_mut().find(|s| s.session == session) {
            held.reason = Some(reason);
            held.held_at = now;
            held.last_tool = last_tool;
            held.background = background;
        }
    }
    pub fn settled(&self) -> bool {
        self.sessions.iter().all(|s| s.reason.is_some())
    }
    pub fn refresh_phase(&mut self) {
        if self.settled() {
            self.phase = Phase::OnHold;
        }
    }
    /// A restart ends every live provider process, so waits that depended on
    /// one (an approval, background work, a turn still settling) become
    /// sessions that need a continuation.
    pub fn after_restart(mut self) -> Self {
        for held in &mut self.sessions {
            if !matches!(held.reason, Some(Reason::Interrupted | Reason::Idle)) {
                held.reason = Some(Reason::Restarted);
            }
        }
        self.phase = Phase::OnHold;
        self
    }
    pub fn held_ms(&self, now: u64) -> u64 {
        now.saturating_sub(self.since)
    }
}

/// The message that resumes a held turn, if it needs one.
pub fn continuation(reason: Reason) -> Option<&'static str> {
    matches!(reason, Reason::Interrupted | Reason::Restarted).then_some(CONTINUE)
}

/// Resume stages: plain sessions and Pilot workers, then Pilot chats.
pub fn resume_plan(state: &HoldState, exists: impl Fn(i64) -> bool) -> Vec<Vec<i64>> {
    let stage = |pilot: bool| -> Vec<i64> {
        state
            .sessions
            .iter()
            .filter(|s| s.pilot_chat == pilot && exists(s.session))
            .map(|s| s.session)
            .collect()
    };
    [stage(false), stage(true)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect()
}

/// Starts resumed sessions a few at a time, one stage after another: a later
/// stage begins once every session of the earlier one has started.
#[derive(Debug, Default)]
pub struct ResumeQueue {
    stages: std::collections::VecDeque<std::collections::VecDeque<i64>>,
    running: std::collections::HashSet<i64>,
}

impl ResumeQueue {
    pub fn new(plan: Vec<Vec<i64>>) -> Self {
        Self {
            stages: plan.into_iter().map(Into::into).collect(),
            running: Default::default(),
        }
    }
    /// The sessions to start now, given at most `limit` resuming at once.
    pub fn next(&mut self, limit: usize) -> Vec<i64> {
        let mut start = Vec::new();
        while self.running.len() < limit {
            while self.stages.front().is_some_and(|stage| stage.is_empty()) {
                self.stages.pop_front();
            }
            let Some(id) = self.stages.front_mut().and_then(|stage| stage.pop_front()) else {
                break;
            };
            self.running.insert(id);
            start.push(id);
        }
        start
    }
    pub fn finished(&mut self, session: i64) {
        self.running.remove(&session);
    }
    pub fn is_running(&self, session: i64) -> bool {
        self.running.contains(&session)
    }
    pub fn done(&self) -> bool {
        self.running.is_empty() && self.stages.iter().all(|stage| stage.is_empty())
    }
}

impl crate::store::Store {
    pub fn hold(&self) -> Result<Option<HoldState>, String> {
        match self.setting(KEY)? {
            Some(text) if !text.is_empty() => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| e.to_string()),
            _ => Ok(None),
        }
    }
    pub fn save_hold(&self, state: &HoldState) -> Result<(), String> {
        let text = serde_json::to_string(state).map_err(|e| e.to_string())?;
        self.save_setting(KEY, &text)
    }
    pub fn clear_hold(&self) -> Result<(), String> {
        self.save_setting(KEY, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::path::Path;

    fn state(sessions: &[(i64, bool)]) -> HoldState {
        let mut state = HoldState::new(1_000);
        for &(id, pilot) in sessions {
            state.track(id, pilot);
        }
        state
    }

    #[test]
    fn settles_to_on_hold_only_when_every_session_settled() {
        let mut state = state(&[(1, false), (2, false)]);
        state.settle(
            1,
            Reason::Interrupted,
            2_000,
            Some("cargo test".into()),
            vec![],
        );
        state.refresh_phase();
        assert_eq!(state.phase, Phase::Holding);
        assert!(!state.settled());
        state.settle(2, Reason::Idle, 3_000, None, vec![]);
        state.refresh_phase();
        assert_eq!(state.phase, Phase::OnHold);
        assert_eq!(state.sessions[0].held_at, 2_000);
        assert_eq!(state.sessions[0].last_tool.as_deref(), Some("cargo test"));
    }

    #[test]
    fn tracking_a_session_twice_keeps_one_entry() {
        let mut state = state(&[(1, false), (1, false)]);
        state.settle(1, Reason::Idle, 2_000, None, vec![]);
        assert_eq!(state.sessions.len(), 1);
        assert!(state.settled());
    }

    #[test]
    fn after_restart_turns_unsettled_and_live_waits_into_restarted() {
        let mut state = state(&[(1, false), (2, false), (3, false), (4, false), (5, false)]);
        state.settle(2, Reason::AwaitingInput, 2_000, None, vec![]);
        state.settle(
            3,
            Reason::Background,
            2_000,
            None,
            vec!["bun run dev".into()],
        );
        state.settle(4, Reason::Interrupted, 2_000, None, vec![]);
        state.settle(5, Reason::Idle, 2_000, None, vec![]);
        let state = state.after_restart();
        let reasons: Vec<_> = state.sessions.iter().map(|s| s.reason).collect();
        assert_eq!(
            reasons,
            vec![
                Some(Reason::Restarted),
                Some(Reason::Restarted),
                Some(Reason::Restarted),
                Some(Reason::Interrupted),
                Some(Reason::Idle),
            ]
        );
        assert_eq!(state.phase, Phase::OnHold);
    }

    #[test]
    fn continuation_only_for_interrupted_or_restarted() {
        assert_eq!(
            continuation(Reason::Interrupted),
            Some("Continue from where you left off.")
        );
        assert_eq!(
            continuation(Reason::Restarted),
            Some("Continue from where you left off.")
        );
        assert_eq!(continuation(Reason::Idle), None);
        assert_eq!(continuation(Reason::AwaitingInput), None);
        assert_eq!(continuation(Reason::Background), None);
    }

    #[test]
    fn resume_plan_puts_pilot_chats_last() {
        let state = state(&[(7, true), (1, false), (2, false), (8, true)]);
        assert_eq!(resume_plan(&state, |_| true), vec![vec![1, 2], vec![7, 8]]);
    }

    #[test]
    fn resume_plan_skips_unknown() {
        let state = state(&[(7, true), (1, false), (2, false)]);
        assert_eq!(resume_plan(&state, |id| id == 2), vec![vec![2]]);
        assert!(resume_plan(&state, |_| false).is_empty());
    }

    #[test]
    fn held_ms_counts_from_when_hold_started() {
        assert_eq!(state(&[]).held_ms(4_500), 3_500);
        assert_eq!(state(&[]).held_ms(500), 0);
    }

    #[test]
    fn resume_queue_starts_at_most_the_limit_and_refills_as_sessions_finish() {
        let mut queue = ResumeQueue::new(vec![vec![1, 2, 3, 4, 5], vec![9]]);
        assert_eq!(queue.next(3), vec![1, 2, 3]);
        assert!(queue.is_running(2));
        assert_eq!(queue.next(3), Vec::<i64>::new());
        queue.finished(2);
        assert_eq!(queue.next(3), vec![4]);
        queue.finished(1);
        queue.finished(3);
        // Stage two may begin as soon as the last worker has started.
        assert_eq!(queue.next(3), vec![5, 9]);
        assert!(!queue.done());
        queue.finished(4);
        queue.finished(5);
        queue.finished(9);
        assert!(queue.done());
    }

    #[test]
    fn resume_queue_ignores_unknown_finishes() {
        let mut queue = ResumeQueue::new(vec![vec![1]]);
        queue.finished(7);
        assert_eq!(queue.next(3), vec![1]);
        queue.finished(1);
        assert!(queue.done());
        assert!(ResumeQueue::new(Vec::new()).done());
    }

    #[test]
    fn store_round_trips_and_clears() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        assert_eq!(store.hold().unwrap(), None);
        let mut held = state(&[(1, false), (9, true)]);
        held.settle(
            1,
            Reason::Interrupted,
            2_000,
            Some("cargo test".into()),
            vec!["dev".into()],
        );
        store.save_hold(&held).unwrap();
        assert_eq!(store.hold().unwrap(), Some(held));
        store.clear_hold().unwrap();
        assert_eq!(store.hold().unwrap(), None);
    }
}
