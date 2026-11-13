//! Places a visual where the provider called for it.
//!
//! A provider streams its text over stdout but publishes a visual over the
//! tool endpoint, so the publication can reach the app before the app has read
//! the text the provider wrote ahead of the call. Most providers also announce
//! the call on stdout before making it: Codex sends `item/started` for the
//! `gespan` `visual_show` MCP call and Claude streams its `tool_use` block. That
//! announcement comes after the earlier text on the same stream, so once the
//! app has processed it, everything written before the call has been applied.
//!
//! [`Anchors`] holds each publication until the announcement of a call with the
//! same `source` has been observed. A provider that announces nothing gets its
//! visual after [`GRACE`], late but never lost.

use super::Publication;
use crate::tool::{visual_tool, ToolCall, VisualStep};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// How long a publication waits for its call's announcement. Short enough that
/// a provider that announces nothing still shows its visual promptly; long
/// enough to cover a stdout reader starved by a loaded machine.
pub const GRACE: Duration = Duration::from_millis(1000);

#[derive(Default)]
pub struct Anchors {
    /// Announced `visual_show` calls not yet matched to a publication, by source.
    announced: HashMap<String, usize>,
    /// Publications released on [`GRACE`] whose announcement has not come yet,
    /// by source: that late announcement must not release another visual.
    unannounced: HashMap<String, usize>,
    /// Calls already counted. A call is reported again as its input streams in
    /// and when it starts, runs and completes.
    seen: HashSet<String>,
    held: Vec<(Publication, Instant)>,
}

impl Anchors {
    /// Notes a tool call the provider reported on stdout.
    pub fn observe(&mut self, call: &ToolCall) {
        if visual_tool(&call.name) != Some(VisualStep::Show) || self.seen.contains(&call.id) {
            return;
        }
        // Claude announces the block before its input has streamed in; the
        // call counts once its input names the source.
        let Some(source) = serde_json::from_str::<Value>(&call.input)
            .ok()
            .and_then(|input| input["source"].as_str().map(str::to_owned))
        else {
            return;
        };
        self.seen.insert(call.id.clone());
        if take(&mut self.unannounced, &source) {
            return;
        }
        *self.announced.entry(source).or_default() += 1;
    }

    /// Adds newly drained publications and returns, in arrival order, those
    /// that may be applied now: announced ones, ones held past [`GRACE`], and
    /// all of them once `flush` says the turn is ending.
    pub fn admit(
        &mut self,
        drained: Vec<Publication>,
        now: Instant,
        flush: bool,
    ) -> Vec<Publication> {
        self.held
            .extend(drained.into_iter().map(|publication| (publication, now)));
        let mut released = Vec::new();
        for (publication, at) in std::mem::take(&mut self.held) {
            let source = &publication.document.source;
            if take(&mut self.announced, source) {
                released.push(publication);
            } else if flush || now.saturating_duration_since(at) >= GRACE {
                *self.unannounced.entry(source.clone()).or_default() += 1;
                released.push(publication);
            } else {
                self.held.push((publication, at));
            }
        }
        released
    }
}

fn take(counts: &mut HashMap<String, usize>, source: &str) -> bool {
    let Some(count) = counts.get_mut(source) else {
        return false;
    };
    *count -= 1;
    if *count == 0 {
        counts.remove(source);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::VisualDocument;
    use std::sync::mpsc;

    fn publication(source: &str) -> Publication {
        Publication {
            document: VisualDocument {
                id: source.into(),
                title: "Visual".into(),
                source: source.into(),
            },
            reply: mpsc::channel().0,
            generation: 0,
        }
    }

    fn call(id: &str, name: &str, input: &str) -> ToolCall {
        let mut call = ToolCall::new(id, name);
        call.input = input.into();
        call
    }

    fn sources(released: &[Publication]) -> Vec<&str> {
        released
            .iter()
            .map(|publication| publication.document.source.as_str())
            .collect()
    }

    #[test]
    fn a_publication_waits_for_its_announcement() {
        let mut anchors = Anchors::default();
        let now = Instant::now();
        assert!(anchors.admit(vec![publication("a")], now, false).is_empty());
        // Another call's announcement does not release it.
        anchors.observe(&call("1", "gespan:visual_show", r#"{"source":"b"}"#));
        assert!(anchors.admit(Vec::new(), now, false).is_empty());
        anchors.observe(&call("2", "gespan:visual_show", r#"{"source":"a"}"#));
        assert_eq!(sources(&anchors.admit(Vec::new(), now, false)), ["a"]);
    }

    #[test]
    fn an_announcement_read_first_releases_its_publication_at_once() {
        let mut anchors = Anchors::default();
        // Claude: the block starts without input, then input streams in, then
        // the whole message repeats it. One call is one announcement.
        anchors.observe(&call("t", "mcp__gespan__visual_show", ""));
        anchors.observe(&call("t", "mcp__gespan__visual_show", r#"{"source":"a"}"#));
        anchors.observe(&call(
            "t",
            "mcp__gespan__visual_show",
            "{\n  \"source\": \"a\"\n}",
        ));
        let now = Instant::now();
        assert_eq!(
            sources(&anchors.admit(vec![publication("a"), publication("a")], now, false)),
            ["a"]
        );
        anchors.observe(&call("u", "mcp__gespan__visual_show", r#"{"source":"a"}"#));
        assert_eq!(sources(&anchors.admit(Vec::new(), now, false)), ["a"]);
    }

    #[test]
    fn other_tools_are_not_announcements() {
        let mut anchors = Anchors::default();
        for name in ["gespan:visual_check", "other:visual_show", "Bash"] {
            anchors.observe(&call(name, name, r#"{"source":"a"}"#));
        }
        assert!(anchors
            .admit(vec![publication("a")], Instant::now(), false)
            .is_empty());
    }

    #[test]
    fn a_silent_provider_gets_its_visual_after_the_grace() {
        let mut anchors = Anchors::default();
        let start = Instant::now();
        assert!(anchors
            .admit(vec![publication("a")], start, false)
            .is_empty());
        assert!(anchors
            .admit(Vec::new(), start + GRACE - Duration::from_millis(1), false)
            .is_empty());
        assert_eq!(
            sources(&anchors.admit(Vec::new(), start + GRACE, false)),
            ["a"]
        );
        // Its announcement, read late, must not release the next visual early.
        anchors.observe(&call("late", "gespan:visual_show", r#"{"source":"a"}"#));
        assert!(anchors
            .admit(vec![publication("a")], start + GRACE, false)
            .is_empty());
    }

    #[test]
    fn the_end_of_the_turn_releases_everything_in_order() {
        let mut anchors = Anchors::default();
        let now = Instant::now();
        anchors.admit(vec![publication("a"), publication("b")], now, false);
        assert_eq!(sources(&anchors.admit(Vec::new(), now, true)), ["a", "b"]);
    }
}
