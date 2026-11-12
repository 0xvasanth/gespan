//! What will wake a chat without the user: work it left running in the
//! background and delegated sessions it asked to hear back from. The
//! composer's watch chip lists these, so a quiet chat never hides a pending wake.
use crate::pilot::{completions::launched_from, Launch};
use crate::provider::BackgroundTask;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Command,
    Subagent,
    /// A delegated session that reports back when it stops.
    Session(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub id: String,
    pub source: Source,
    pub label: String,
    /// What the chat is waiting on.
    pub detail: String,
    /// Unix milliseconds, when known.
    pub started_at: Option<u64>,
}

/// Background commands and subagents, in start order.
pub fn background(tasks: &[BackgroundTask]) -> Vec<Watch> {
    tasks
        .iter()
        .map(|task| {
            let command = task
                .command
                .as_deref()
                .map(first_line)
                .filter(|c| !c.is_empty());
            let label = Some(task.description.trim())
                .filter(|d| !d.is_empty())
                .or(command)
                .unwrap_or(if task.agent {
                    "Subagent"
                } else {
                    "Background command"
                });
            let detail = task
                .latest
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .or(command.filter(|c| *c != label))
                .unwrap_or("Wakes this chat when it finishes");
            Watch {
                id: task.id.clone(),
                source: if task.agent {
                    Source::Subagent
                } else {
                    Source::Command
                },
                label: label.into(),
                detail: detail.into(),
                started_at: Some(task.started_at),
            }
        })
        .collect()
}

/// Sessions `chat` launched with `notifyOnComplete` whose report has not
/// landed yet. `state` is a launch card's live state (working, input, draft,
/// failed, stopped or done), when known.
pub fn delegated<'a>(
    launches: &[Launch],
    chat: i64,
    state: impl Fn(i64) -> Option<&'a str>,
) -> Vec<Watch> {
    launches
        .iter()
        .filter(|l| l.notify_on_complete && !l.completed && !l.dismissed && launched_by(l, chat))
        .map(|l| {
            let title = l.title.trim();
            Watch {
                id: format!("session:{}", l.session_id),
                source: Source::Session(l.session_id),
                label: if title.is_empty() {
                    format!("Session {}", l.session_id)
                } else {
                    title.into()
                },
                detail: match state(l.session_id) {
                    Some("input") => "Needs input before it can finish",
                    Some("failed" | "stopped" | "done") => "Stopped; its report is on the way",
                    _ => "Reports back when it stops",
                }
                .into(),
                started_at: None,
            }
        })
        .collect()
}

/// `origin_turn` keeps only the low 32 bits of the launching chat's id, so
/// compare on those: real session ids are wider than 32 bits.
fn launched_by(launch: &Launch, chat: i64) -> bool {
    launched_from(launch.origin_turn, chat)
}

/// The chip's text, or nothing when no wake is pending.
pub fn chip_label(count: usize) -> Option<String> {
    (count > 0).then(|| format!("{count} watching"))
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(session: i64, source: i64, title: &str) -> Launch {
        Launch {
            session_id: session,
            cwd: String::new(),
            title: title.into(),
            harness: "kiro".into(),
            model: None,
            origin_turn: ((source as u64) << 32) | 3,
            notify_on_complete: true,
            completed: false,
            dismissed: false,
        }
    }

    fn task(id: &str, agent: bool, description: &str, command: Option<&str>) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            description: description.into(),
            agent,
            command: command.map(Into::into),
            started_at: 1_000,
            tool_id: None,
            latest: None,
            steps: 0,
        }
    }

    #[test]
    fn only_pending_reports_for_this_chat_are_watched() {
        let mut completed = launch(2, 7, "Done already");
        completed.completed = true;
        let mut dismissed = launch(3, 7, "Dismissed");
        dismissed.dismissed = true;
        let mut silent = launch(4, 7, "Fire and forget");
        silent.notify_on_complete = false;
        let other_chat = launch(5, 8, "A habit's launch");
        let launches = [
            launch(1, 7, "Monitor v0.0.9 Release Pipeline"),
            completed,
            dismissed,
            silent,
            other_chat,
        ];
        let got = delegated(&launches, 7, |_| Some("working"));
        assert_eq!(
            got,
            vec![Watch {
                id: "session:1".into(),
                source: Source::Session(1),
                label: "Monitor v0.0.9 Release Pipeline".into(),
                detail: "Reports back when it stops".into(),
                started_at: None,
            }]
        );
    }

    #[test]
    fn a_chat_with_a_wide_session_id_still_finds_its_launches() {
        // ReleasePilot's real chat id, and the origin turn its launch saved.
        let chat = 3_471_940_774_678_665;
        let mut mine = launch(1, 0, "Monitor v0.0.9 Release Pipeline");
        mine.origin_turn = 3_787_052_889_157_127_898;
        let mut other = launch(2, 0, "Another chat's launch");
        other.origin_turn = ((chat as u64 + 1) << 32) | 3;
        let got = delegated(&[mine, other], chat, |_| None);
        assert_eq!(
            got.iter().map(|w| &*w.label).collect::<Vec<_>>(),
            ["Monitor v0.0.9 Release Pipeline"]
        );
    }

    #[test]
    fn a_delegated_watch_says_what_it_is_waiting_on() {
        let launches = [launch(1, 7, "a"), launch(2, 7, "b"), launch(3, 7, "c")];
        let got = delegated(&launches, 7, |id| match id {
            1 => Some("input"),
            2 => Some("failed"),
            _ => None,
        });
        let details: Vec<_> = got.iter().map(|w| w.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "Needs input before it can finish",
                "Stopped; its report is on the way",
                "Reports back when it stops",
            ]
        );
    }

    #[test]
    fn an_untitled_launch_is_named_by_its_session() {
        let got = delegated(&[launch(42, 7, "  ")], 7, |_| None);
        assert_eq!(got[0].label, "Session 42");
    }

    #[test]
    fn background_work_keeps_order_kind_and_start_time() {
        let tasks = [
            task("b1", false, "Watch CI", Some("gh run watch 42")),
            task("a1", true, "Review the diff", None),
        ];
        let got = background(&tasks);
        assert_eq!(
            got.iter().map(|w| (&*w.id, &w.source)).collect::<Vec<_>>(),
            [("b1", &Source::Command), ("a1", &Source::Subagent)]
        );
        assert_eq!(got[0].detail, "gh run watch 42");
        assert_eq!(got[1].detail, "Wakes this chat when it finishes");
        assert!(got.iter().all(|w| w.started_at == Some(1_000)));
    }

    #[test]
    fn a_command_without_description_is_named_by_its_first_line() {
        let got = background(&[task("b", false, " ", Some("cargo test\n  --all"))]);
        assert_eq!(got[0].label, "cargo test");
        // The label already shows the command; the detail does not repeat it.
        assert_eq!(got[0].detail, "Wakes this chat when it finishes");
        let got = background(&[task("a", true, "", None)]);
        assert_eq!(got[0].label, "Subagent");
    }

    #[test]
    fn latest_progress_beats_the_command_as_detail() {
        let mut t = task("b", false, "Watch CI", Some("gh run watch 42"));
        t.latest = Some("3 of 5 jobs done".into());
        assert_eq!(background(&[t])[0].detail, "3 of 5 jobs done");
    }

    #[test]
    fn the_chip_hides_when_nothing_is_watching() {
        assert_eq!(chip_label(0), None);
        assert_eq!(chip_label(1).as_deref(), Some("1 watching"));
        assert_eq!(chip_label(3).as_deref(), Some("3 watching"));
    }
}
