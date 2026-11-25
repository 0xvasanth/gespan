//! A Pilot turn's steps as trace lines: each tool call, each background job
//! and each finished-job notice becomes one [`trace::Line`].
use crate::trace::{self, Line, Status};
use gespan_core::provider::BackgroundTask;
use gespan_core::tool::{ToolCall, ToolState};
use gespan_core::transcript::Interjection;

/// The app action a call made, for a Pilot's `gespan app …` or `app_call`.
pub(crate) fn app_action(call: &ToolCall) -> Option<String> {
    let input = serde_json::from_str::<serde_json::Value>(&call.input).ok()?;
    input["command"]
        .as_str()
        .and_then(gespan_core::pilot::work::command_action)
        .or_else(|| gespan_core::pilot::work::app_call_action(&call.name, &input))
}

/// "exit 1" from a shell's output, when it names the code.
fn exit_code(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("exit code")? + "exit code".len();
    let digits: String = lower[at..]
        .trim_start_matches([' ', ':', '='])
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The step's name: the model's own words for it when it gave them, else
/// what the call does, as "Read src/app.rs".
pub(crate) fn step_title(call: &ToolCall, cwd: Option<&str>) -> String {
    let shown = call.presentation_in(cwd);
    if let Some(description) = shown.description.as_deref().map(str::trim)
        && !description.is_empty()
    {
        return description.to_owned();
    }
    let (action, target) = call.summary();
    [action, target]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn step_icon(call: &ToolCall, action: Option<&str>) -> &'static str {
    if let Some(action) = action {
        return match action.split('.').next().unwrap_or_default() {
            "sessions" => "rocket",
            "projects" | "folders" | "worktrees" => "folder",
            "pilot" if action.ends_with("addProject") => "folder",
            "pilot" => "message-square",
            "artifacts" | "notes" => "file",
            "memory" | "soul" => "search",
            _ => "zap",
        };
    }
    match call.summary().0.as_str() {
        "Run" => "terminal",
        "Edit" | "Write" => "pencil-edit-02",
        "Read" => "file",
        "Find" | "List" | "Search" | "Glob" | "Grep" => "search",
        "Agent" | "Task" => "rocket",
        _ => "zap",
    }
}

/// One tool call as a line. `started_at` is when its row arrived; a pending
/// background task the call started keeps it running under the task's name.
/// A call still marked running in a turn that is no longer `live` never
/// finished, so it shows dimmed as pending.
pub(crate) fn tool_line(
    call: &ToolCall,
    live: bool,
    started_at: Option<u64>,
    background: Option<&BackgroundTask>,
    cwd: Option<&str>,
    now_ms: u64,
) -> Line {
    let action = app_action(call);
    let mut line = Line::new(
        format!("tool:{}", call.id),
        step_icon(call, action.as_deref()),
        step_title(call, cwd),
    );
    let shown = call.presentation_in(cwd);
    let shell = shown.command.is_some() && action.is_none();
    let output = call.output.trim();
    let mut detail = String::new();
    if let Some(command) = shown.command.as_deref().filter(|_| shell) {
        detail.push_str("$ ");
        detail.push_str(command.trim());
    }
    if !output.is_empty() {
        if !detail.is_empty() {
            detail.push('\n');
        }
        detail.push_str(output);
    }
    line.detail = (!detail.is_empty()).then_some(detail);
    if shown.steps.len() > 1 {
        line.children = shown
            .steps
            .iter()
            .enumerate()
            .map(|(ix, step)| {
                let title = [step.action, step.target.as_str()]
                    .into_iter()
                    .filter(|part| !part.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut child = Line::new(
                    format!("tool:{}:{ix}", call.id),
                    "terminal",
                    if title.is_empty() {
                        step.command.clone()
                    } else {
                        title
                    },
                );
                child.description = step.op.map(str::to_owned).unwrap_or_default();
                child
            })
            .collect();
    }
    if let Some(edit) = call.edit_preview().filter(|e| !e.content_only) {
        line.description = format!("+{} −{}", edit.additions, edit.deletions);
    }
    match (&call.state, background) {
        (_, Some(task)) => {
            line.status = Status::Running;
            if !task.description.trim().is_empty() {
                line.title = task.description.trim().to_owned();
            }
            line.since = Some(task.started_at);
            line.description = task
                .latest
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .unwrap_or("in the background")
                .to_owned();
        }
        (ToolState::Running, None) if !live => {
            line.status = Status::Pending;
            line.description = "didn't finish".into();
        }
        (ToolState::Running, None) => {
            line.status = Status::Running;
            line.since = call
                .live_elapsed_secs
                .map(|secs| now_ms.saturating_sub(secs * 1000))
                .or(started_at)
                .or(Some(now_ms));
            line.description.clear();
        }
        (ToolState::Failed, None) => {
            line.status = Status::Error;
            line.title = format!("Failed to {}", lower_first(&line.title));
            line.description = match exit_code(output) {
                Some(code) => format!("exit {code}"),
                None => "failed".into(),
            };
        }
        (ToolState::Cancelled, None) => {
            line.icon = "circle-x";
            line.description = "stopped".into();
        }
        (ToolState::Completed, None) => {
            if shell && line.description.is_empty() {
                line.description = match exit_code(output) {
                    Some(code) if code != 0 => {
                        line.status = Status::Error;
                        line.title = format!("Failed to {}", lower_first(&line.title));
                        format!("exit {code}")
                    }
                    _ => "exit 0".into(),
                };
            }
        }
    }
    line
}

/// A background job's completion notice as one line, "… · 31m · exit 0".
/// `ran_for` is how long it ran, when its start is known.
pub(crate) fn finished_line(key: usize, note: &Interjection, ran_for: Option<u64>) -> Option<Line> {
    if note.custom_type != "task-notification" {
        return None;
    }
    let summary = note.text.trim();
    let mut line = Line::new(
        format!("finished:{key}"),
        "check",
        if summary.is_empty() {
            "Background job finished".to_owned()
        } else {
            summary.to_owned()
        },
    );
    let code = exit_code(summary);
    let failed = code.is_some_and(|c| c != 0)
        || ["failed", "error", "killed"]
            .iter()
            .any(|w| summary.to_ascii_lowercase().contains(w));
    let mut parts = Vec::new();
    if let Some(secs) = ran_for {
        parts.push(trace::duration(secs));
    }
    if let Some(code) = code {
        parts.push(format!("exit {code}"));
    }
    line.description = parts.join(" · ");
    if failed {
        line.status = Status::Error;
    }
    Some(line)
}

/// The thinking line at the live end of a turn whose latest step is prose.
pub(crate) fn thinking_line(group: usize, since: Option<u64>) -> Line {
    let mut line = Line::new(format!("thinking:{group}"), "loader", "Thinking");
    line.status = Status::Running;
    line.since = since;
    line
}

/// A settled turn's one line: "Worked for 4m 10s · 7 steps · 3 sessions started".
pub(crate) fn settled_summary(worked_secs: Option<u64>, steps: usize, sessions: usize) -> String {
    let mut parts = vec![match worked_secs {
        Some(secs) => format!("Worked for {}", trace::duration(secs.max(1))),
        None => "Worked".to_owned(),
    }];
    if steps > 0 {
        parts.push(format!(
            "{steps} {}",
            if steps == 1 { "step" } else { "steps" }
        ));
    }
    if sessions > 0 {
        parts.push(format!(
            "{sessions} {} started",
            if sessions == 1 { "session" } else { "sessions" }
        ));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(input: &str, output: &str, state: ToolState) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: "Bash".into(),
            input: input.into(),
            output: output.into(),
            state,
            panel: None,
            display: None,
            live_elapsed_secs: None,
        }
    }
    const COPY: &str = r#"{"command":"rsync -a fixtures/ db/fixtures","description":"Copy fixtures to the test databases"}"#;

    #[test]
    fn a_step_is_named_by_the_tools_description() {
        let line = tool_line(
            &call(COPY, "", ToolState::Completed),
            true,
            None,
            None,
            None,
            0,
        );
        assert_eq!(line.title, "Copy fixtures to the test databases");
        assert_eq!(line.status, Status::Done);
        assert_eq!(line.description, "exit 0");
        assert_eq!(line.icon, "terminal");
        assert!(
            line.detail.as_deref().unwrap().starts_with("$ rsync"),
            "its command opens inside the line"
        );
    }

    #[test]
    fn a_running_step_counts_from_its_heartbeat_or_its_row() {
        let mut running = call(COPY, "", ToolState::Running);
        let line = tool_line(&running, true, Some(5_000), None, None, 9_000);
        assert_eq!(line.status, Status::Running);
        assert_eq!(line.shown_description(9_000), "4s");
        running.live_elapsed_secs = Some(70);
        let line = tool_line(&running, true, Some(5_000), None, None, 100_000);
        assert_eq!(line.shown_description(100_000), "1m 10s");
        let stopped = tool_line(&running, false, Some(5_000), None, None, 100_000);
        assert_eq!(
            stopped.status,
            Status::Pending,
            "a stopped turn's call never finished"
        );
        assert_eq!(stopped.shown_description(200_000), "didn't finish");
    }

    #[test]
    fn a_failed_step_is_red_and_says_what_failed() {
        let line = tool_line(
            &call(
                COPY,
                "Exit code 1\nrsync: connection refused",
                ToolState::Failed,
            ),
            true,
            None,
            None,
            None,
            0,
        );
        assert_eq!(line.status, Status::Error);
        assert_eq!(line.title, "Failed to copy fixtures to the test databases");
        assert_eq!(line.description, "exit 1");
        assert!(line.detail.unwrap().contains("connection refused"));
    }

    #[test]
    fn a_backgrounded_step_stays_running_under_its_task() {
        let task = BackgroundTask {
            id: "b1".into(),
            description: "Copy the community DB to staging".into(),
            agent: false,
            command: None,
            started_at: 1_000,
            tool_id: Some("c1".into()),
            latest: None,
            steps: 0,
        };
        let line = tool_line(
            &call(COPY, "Command running in background", ToolState::Completed),
            true,
            None,
            Some(&task),
            None,
            73_000,
        );
        assert_eq!(line.status, Status::Running);
        assert_eq!(line.title, "Copy the community DB to staging");
        assert_eq!(line.shown_description(73_000), "1m 12s · in the background");
    }

    #[test]
    fn a_finished_job_is_one_line_with_its_time_and_exit() {
        let note = Interjection {
            text: "Set up the work machines completed (exit code 0)".into(),
            custom_type: "task-notification".into(),
            severity: None,
        };
        let line = finished_line(3, &note, Some(31 * 60)).unwrap();
        assert_eq!(line.status, Status::Done);
        assert_eq!(line.description, "31m · exit 0");
        let failed = Interjection {
            text: "Copy DB failed (exit code 1)".into(),
            ..note.clone()
        };
        assert_eq!(
            finished_line(4, &failed, None).unwrap().status,
            Status::Error
        );
        let other = Interjection {
            custom_type: "advisor".into(),
            ..note
        };
        assert!(finished_line(5, &other, None).is_none());
    }

    #[test]
    fn the_settled_line_counts_steps_and_sessions() {
        assert_eq!(
            settled_summary(Some(250), 7, 3),
            "Worked for 4m 10s · 7 steps · 3 sessions started"
        );
        assert_eq!(settled_summary(Some(3), 1, 0), "Worked for 3s · 1 step");
        assert_eq!(
            settled_summary(None, 2, 1),
            "Worked · 2 steps · 1 session started"
        );
    }

    #[test]
    fn session_calls_get_the_launch_icon() {
        let start = ToolCall {
            name: "Bash".into(),
            ..call(
                r#"{"command":"gespan app sessions.start --json {}"}"#,
                "",
                ToolState::Completed,
            )
        };
        let line = tool_line(&start, true, None, None, None, 0);
        assert_eq!(line.icon, "rocket");
        assert_ne!(
            line.description, "exit 0",
            "an app call is not a shell step"
        );
    }
}
