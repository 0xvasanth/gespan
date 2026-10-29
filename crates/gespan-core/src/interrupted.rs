//! Chats a quit or crash cut short, continued on the next launch.
//! Ported from src/features/sessions/model/inFlight.ts; the marker text and
//! the `in_flight_sessions` table are shared, so either app resumes them.
use crate::tool::{ToolCall, ToolState};
use crate::transcript::Row;

/// The system row that ends a cut-off turn. The Tauri app matches this text.
pub const MESSAGE: &str = "Turn interrupted when Gespan quit.";
/// Sent once to a chat whose last row is [`MESSAGE`].
pub const CONTINUE: &str = "Continue from where you left off.";

pub fn quit_message(count: usize) -> String {
    if count == 1 {
        "1 chat is still running. Quit anyway? It will resume when you reopen Gespan.".into()
    } else {
        format!(
            "{count} chats are still running. Quit anyway? They will resume when you reopen Gespan."
        )
    }
}

/// The marker is still the last row: nothing has been sent since the quit.
pub fn ends_interrupted(rows: &[Row]) -> bool {
    rows.last()
        .is_some_and(|row| row.role == "system" && row.text == MESSAGE)
}

/// Row writes that end the last turn as interrupted: its running tools become
/// cancelled, then the marker is appended. Empty when there is no prompt to
/// continue, or the turn already ends with the marker.
pub fn interrupt(rows: &[Row], now: u64) -> Vec<(usize, Row)> {
    let Some(prompt) = rows.iter().rposition(Row::is_prompt) else {
        return Vec::new();
    };
    if ends_interrupted(rows) {
        return Vec::new();
    }
    let mut writes: Vec<(usize, Row)> = rows
        .iter()
        .enumerate()
        .skip(prompt + 1)
        .filter(|(_, row)| row.role == "tool")
        .filter_map(|(ix, row)| {
            let mut tool = serde_json::from_str::<ToolCall>(&row.text).ok()?;
            if tool.state != ToolState::Running {
                return None;
            }
            tool.state = ToolState::Cancelled;
            if tool.output.is_empty() {
                tool.output = "Cancelled".into();
            }
            let text = serde_json::to_string(&tool).ok()?;
            Some((
                ix,
                Row {
                    text,
                    ..row.clone()
                },
            ))
        })
        .collect();
    writes.push((
        rows.len(),
        Row {
            role: "system".into(),
            text: MESSAGE.into(),
            first: true,
            sent_at: Some(now),
        },
    ));
    writes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }

    fn tool(id: &str, state: ToolState, output: &str) -> Row {
        let mut call = ToolCall::new(id, "Bash");
        call.state = state;
        call.output = output.into();
        row("tool", &serde_json::to_string(&call).unwrap())
    }

    fn state(row: &Row) -> (ToolState, String) {
        let call: ToolCall = serde_json::from_str(&row.text).unwrap();
        (call.state, call.output)
    }

    #[test]
    fn a_cut_off_turn_gets_the_marker_and_its_running_tools_are_cancelled() {
        let rows = vec![
            row("user", "Old"),
            tool("old", ToolState::Running, ""),
            row("user", "Fix the tests"),
            row("assistant", "Running them"),
            tool("done", ToolState::Completed, "ok"),
            tool("live", ToolState::Running, ""),
            tool("partial", ToolState::Running, "3 passed"),
        ];
        let writes = interrupt(&rows, 42);
        let at: Vec<usize> = writes.iter().map(|(ix, _)| *ix).collect();
        // Only this turn's running tools; an older turn is left as it was.
        assert_eq!(at, vec![5, 6, 7]);
        assert_eq!(
            state(&writes[0].1),
            (ToolState::Cancelled, "Cancelled".into())
        );
        assert_eq!(
            state(&writes[1].1),
            (ToolState::Cancelled, "3 passed".into())
        );
        let marker = &writes[2].1;
        assert_eq!(
            (marker.role.as_str(), marker.text.as_str()),
            ("system", MESSAGE)
        );
        assert_eq!(marker.sent_at, Some(42));
        assert!(marker.first);
    }

    #[test]
    fn marking_twice_adds_nothing() {
        let mut rows = vec![row("user", "Go"), row("assistant", "Working")];
        for (ix, write) in interrupt(&rows, 1) {
            if ix == rows.len() {
                rows.push(write);
            } else {
                rows[ix] = write;
            }
        }
        assert!(ends_interrupted(&rows));
        assert!(interrupt(&rows, 2).is_empty());
    }

    #[test]
    fn a_later_turn_after_continue_can_be_interrupted_again() {
        let rows = vec![
            row("user", "Go"),
            row("system", MESSAGE),
            row("user", CONTINUE),
            row("assistant", "Picking up"),
        ];
        assert!(!ends_interrupted(&rows));
        assert_eq!(interrupt(&rows, 1).len(), 1);
    }

    #[test]
    fn a_chat_with_no_prompt_is_not_marked() {
        assert!(interrupt(&[], 1).is_empty());
        assert!(interrupt(&[row("system", "Ready")], 1).is_empty());
    }

    #[test]
    fn the_marker_must_be_the_last_row_and_a_system_row() {
        assert!(!ends_interrupted(&[row("assistant", MESSAGE)]));
        assert!(!ends_interrupted(&[
            row("system", MESSAGE),
            row("assistant", "more")
        ]));
        assert!(ends_interrupted(&[
            row("user", "Go"),
            row("system", MESSAGE)
        ]));
    }

    #[test]
    fn the_quit_question_counts_chats() {
        assert_eq!(
            quit_message(1),
            "1 chat is still running. Quit anyway? It will resume when you reopen Gespan."
        );
        assert_eq!(
            quit_message(3),
            "3 chats are still running. Quit anyway? They will resume when you reopen Gespan."
        );
    }
}
