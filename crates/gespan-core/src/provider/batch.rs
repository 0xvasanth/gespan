//! When the receive loop hands its pending events to the view. Streaming
//! output is coalesced to about a frame, but the turn's first visible output
//! goes out at once: it is what the user is waiting for after Send.
use super::{Event, BATCH_BYTES, TICK};
use std::time::Instant;

pub(super) struct Batch {
    last_emit: Instant,
    bytes: usize,
    /// The pending events hold the turn's first visible output.
    first_output: bool,
    /// Visible output has already gone out this turn.
    answered: bool,
}

impl Batch {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            last_emit: now,
            bytes: 0,
            first_output: false,
            answered: false,
        }
    }

    /// Counts an event queued for the next emit.
    pub(super) fn add(&mut self, event: &Event) {
        self.bytes += match event {
            Event::Text(text) | Event::Reasoning(text) | Event::Status(text) => text.len(),
            Event::Tool(tool) => tool.input.len() + tool.output.len(),
            Event::Approval(approval) => approval.detail.len(),
            Event::Question(question) => question.items.iter().map(|q| q.prompt.len()).sum(),
            _ => 64,
        };
        if !self.answered && visible(event) {
            self.first_output = true;
        }
    }

    /// Whether the pending events go out now.
    pub(super) fn due(&self, now: Instant, done: bool) -> bool {
        self.first_output
            || done
            || self.bytes >= BATCH_BYTES
            || now.saturating_duration_since(self.last_emit) >= TICK
    }

    pub(super) fn emitted(&mut self, now: Instant) {
        self.answered |= self.first_output;
        self.first_output = false;
        self.bytes = 0;
        self.last_emit = now;
    }
}

/// Output the transcript shows the user, as opposed to bookkeeping such as
/// the session id, model or context usage.
fn visible(event: &Event) -> bool {
    matches!(
        event,
        Event::Text(_)
            | Event::Reasoning(_)
            | Event::Tool(_)
            | Event::Plan(_)
            | Event::Approval(_)
            | Event::Question(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_first_text_goes_out_inside_the_window_a_bookkeeping_emit_opened() {
        let start = Instant::now();
        let mut batch = Batch::new(start);
        // Claude's message_start reports the model just before the first token.
        batch.add(&Event::Model("claude".into()));
        let opened = start + TICK;
        assert!(batch.due(opened, false));
        batch.emitted(opened);
        let token = opened + Duration::from_micros(300);
        batch.add(&Event::Text("Hel".into()));
        assert!(
            batch.due(token, false),
            "the first token waited for the coalescing window"
        );
    }

    #[test]
    fn later_output_inside_the_window_is_coalesced() {
        let start = Instant::now();
        let mut batch = Batch::new(start);
        batch.add(&Event::Text("Hel".into()));
        assert!(batch.due(start, false));
        batch.emitted(start);
        let soon = start + Duration::from_millis(2);
        batch.add(&Event::Text("lo".into()));
        assert!(!batch.due(soon, false));
        assert!(batch.due(start + TICK, false));
    }

    #[test]
    fn bookkeeping_alone_never_skips_the_window() {
        let start = Instant::now();
        let mut batch = Batch::new(start);
        batch.add(&Event::SessionId("s".into()));
        batch.add(&Event::Context {
            used: Some(1),
            window: Some(2),
        });
        assert!(!batch.due(start + Duration::from_millis(1), false));
    }

    #[test]
    fn reasoning_or_a_plan_counts_as_the_first_output() {
        for event in [
            Event::Reasoning("thinking".into()),
            Event::Plan("1. step".into()),
        ] {
            let start = Instant::now();
            let mut batch = Batch::new(start);
            batch.add(&event);
            assert!(batch.due(start, false), "{event:?}");
        }
    }

    #[test]
    fn a_large_batch_or_the_end_of_the_turn_goes_out_at_once() {
        let start = Instant::now();
        let mut batch = Batch::new(start);
        batch.add(&Event::Text("first".into()));
        batch.emitted(start);
        batch.add(&Event::Text("x".repeat(BATCH_BYTES)));
        assert!(batch.due(start, false));
        batch.emitted(start);
        batch.add(&Event::Done);
        assert!(batch.due(start, true));
    }
}
