//! Response actions share their prose and recap rules. Tool chrome and private
//! reasoning aren't copied.
use crate::{tool::ToolCall, transcript::Row};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    Handoff,
    SecondOpinion,
    /// A new session that continues this one from a chosen message.
    Fork,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default)]
    pub settings: std::collections::HashMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source_session: i64,
    pub source_cwd: String,
}
pub use crate::time::now_ms;
pub fn local_clock(epoch_ms: u64) -> String {
    #[cfg(unix)]
    {
        let time = (epoch_ms / 1000) as libc::time_t;
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        // localtime_r writes the caller-owned buffer and doesn't share tm storage.
        if !unsafe { libc::localtime_r(&time, local.as_mut_ptr()) }.is_null() {
            let local = unsafe { local.assume_init() };
            return format!("{:02}:{:02}", local.tm_hour, local.tm_min);
        }
    }
    let minutes = epoch_ms / 60_000;
    format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
}
pub fn limit(text: &str, count: usize) -> String {
    if text.chars().count() <= count {
        text.into()
    } else {
        format!(
            "{}\n\n[truncated]",
            text.chars()
                .take(count.saturating_sub(13))
                .collect::<String>()
        )
    }
}
pub fn prose(rows: &[Row]) -> String {
    let mut messages: Vec<String> = Vec::new();
    let mut previous = false;
    for row in rows {
        if row.role == "assistant" {
            if !previous || row.first {
                messages.push(String::new());
            }
            messages.last_mut().unwrap().push_str(&row.text);
            previous = true;
        } else {
            previous = false;
        }
    }
    messages
        .iter()
        .map(|s| s.trim().replace("\r\n", "\n").replace('\r', "\n"))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}
/// Full assistant text after the last user row, joined like `prose`, never truncated.
pub fn last_assistant_text(rows: &[Row]) -> String {
    let start = rows
        .iter()
        .rposition(|r| r.role == "user")
        .map_or(0, |ix| ix + 1);
    prose(&rows[start..])
}
pub fn request(rows: &[Row]) -> String {
    rows.iter()
        .take_while(|r| r.role == "user")
        .map(|r| r.text.as_str())
        .collect::<String>()
        .trim()
        .into()
}
pub fn edited_files(rows: &[Row]) -> Vec<String> {
    let mut files = Vec::new();
    for row in rows.iter().filter(|r| r.role == "tool") {
        if let Ok(tool) = serde_json::from_str::<ToolCall>(&row.text) {
            let (action, path) = tool.summary();
            if matches!(action.as_str(), "Edit" | "Write")
                && !path.is_empty()
                && !files.contains(&path)
            {
                files.push(path);
            }
        }
    }
    files.truncate(40);
    files
}
pub fn second_opinion(from: &str, rows: &[Row]) -> String {
    let title = crate::provider::Kind::from_id(from).label();
    let user = request(rows);
    let report = prose(rows);
    let files = edited_files(rows);
    let sections=[
        format!("Give a second opinion on work {title} just finished in this same working copy. The files are already on disk."),
        "Review that work: what is wrong, what is missing, and what you would have done differently. Fix anything you agree is broken or incomplete. If you would leave it, say so and stop. Do not redo the task from scratch unless the work is actually wrong. Read the listed files before changing anything.".into(),
        format!("## User request\n{}",if user.is_empty(){"(no user message on this turn)".into()}else{limit(&user,400)}),
        format!("## What {title} reported\n{}",if report.is_empty(){"(no written summary — inspect the files)".into()}else{limit(&report,900)}),
        format!("## Files it edited\n{}",if files.is_empty(){"(none recorded on this turn)".into()}else{files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")}),
    ];
    limit(&sections.join("\n\n"), 1800)
}
/// What a fork at one message carries into its new session.
#[derive(Debug, PartialEq, Eq)]
pub struct Fork {
    pub rows: Vec<Row>,
    /// Forking at a prompt branches before it: the prompt returns to the
    /// composer so it can be edited or sent to another model.
    pub draft: Option<String>,
}

/// The conversation up to the message at row `at`. A response keeps its whole
/// turn; a prompt keeps only what came before it. An index past the end means
/// the last message.
pub fn fork_at(rows: &[Row], at: usize) -> Fork {
    let Some(last) = rows.len().checked_sub(1) else {
        return Fork {
            rows: Vec::new(),
            draft: None,
        };
    };
    let at = at.min(last);
    let start = rows[..=at].iter().rposition(|r| r.first).unwrap_or(0);
    if rows[start].is_prompt() {
        let end = (start + 1..rows.len())
            .find(|&i| rows[i].first)
            .unwrap_or(rows.len());
        return Fork {
            rows: rows[..start].to_vec(),
            draft: (rows[start].role == "user")
                .then(|| rows[start..end].iter().map(|r| r.text.as_str()).collect()),
        };
    }
    let end = (at + 1..rows.len())
        .find(|&i| rows[i].is_prompt())
        .unwrap_or(rows.len());
    Fork {
        rows: rows[..end].to_vec(),
        draft: None,
    }
}

/// A new session's settings when it continues `source` with its first `kept`
/// rows: the same agent setup and the receipts of those turns, but none of the
/// source's pending one-shot state. `turns` are keyed by row of the whole
/// history, as the copied rows are.
pub fn carried_settings(
    source: &crate::agent::Settings,
    turns: &std::collections::HashMap<usize, crate::agent::TurnInfo>,
    kept: usize,
) -> crate::agent::Settings {
    crate::agent::Settings {
        account: source.account.clone(),
        revision: now_ms() * 1_000_000,
        model: source.model.clone(),
        effort: source.effort.clone(),
        model_settings: source.model_settings.clone(),
        fast: source.fast,
        service_tier: source.service_tier.clone(),
        thinking: source.thinking,
        access: source.access,
        worktree_cwd: source.worktree_cwd.clone(),
        turns: turns
            .iter()
            .filter(|(turn, _)| **turn < kept)
            .map(|(turn, info)| (*turn, info.clone()))
            .collect(),
        ..Default::default()
    }
}

/// An empty recap for a chat with nothing said yet, so the next agent is told
/// not to invent prior work.
pub fn handoff(rows: &[Row]) -> String {
    if !rows
        .iter()
        .any(|r| matches!(r.role.as_str(), "user" | "assistant") && !r.text.trim().is_empty())
    {
        return String::new();
    }
    let mut users = Vec::new();
    let mut user = String::new();
    for row in rows {
        if row.role == "user" {
            if row.first && !user.is_empty() {
                users.push(std::mem::take(&mut user));
            }
            user.push_str(&row.text);
        }
    }
    if !user.is_empty() {
        users.push(user);
    }
    let last = rows
        .iter()
        .rposition(|r| r.role == "assistant" && r.first)
        .unwrap_or(rows.len());
    let report = prose(&rows[last..]);
    let files = edited_files(rows);
    let mut sections = vec![format!(
        "## Session so far\n{}",
        users
            .iter()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|s| format!("User: {}", limit(s, 240).replace('\n', " ")))
            .collect::<Vec<_>>()
            .join("\n")
    )];
    if !report.is_empty() {
        sections.push(format!("Assistant: {}", limit(&report, 500)));
    }
    if !files.is_empty() {
        sections.push(format!(
            "## Files edited in this session\n{}",
            files
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    limit(&sections.join("\n\n"), 1800)
}

pub fn outgoing_handoff_prompt(user_request: &str) -> String {
    let request = user_request.trim();
    let request = if request.is_empty() {
        "(no text)"
    } else {
        request
    };
    format!(
        "The user is switching to another coding agent. Their new message will be sent separately — do not repeat it, and do not add a Goal heading.\n\n<user_request>\n{}\n</user_request>\n\nWrite a short recap of this conversation so the next agent can continue. Under 120 words. Plain markdown. No title card. No greeting. Do not paste the whole transcript.\n\nRules:\n- Use only this conversation.\n- Do not run git, do not inspect the working tree, do not read files, do not call tools.\n- Mention files only if this chat edited them.\n- If the chat was a greeting or has no task yet, say that in one sentence. Do not invent work from uncommitted repo files.\n\nInclude only sections that have session-specific content:\n- Session so far (a few bullets, not a transcript)\n- Files edited in this session\n- Suggested next step",
        limit(request, 1500)
    )
}

pub fn choose_handoff_brief(agent: &str, rows: &[Row]) -> String {
    let agent = strip_goal_sections(agent);
    if agent.chars().count() >= 40 {
        limit(&agent, 1800)
    } else {
        strip_goal_sections(&handoff(rows))
    }
}

pub fn wrap_handoff_prompt(from: &str, brief: &str, user_request: &str) -> String {
    let provider = crate::provider::Kind::from_id(from).label();
    let body = strip_goal_sections(brief);
    let lead = format!(
        "You are continuing an existing conversation handed off from {provider}. This is not a new session. Do not say you have no prior context.\n\n{}",
        user_request.trim()
    );
    if body.is_empty() {
        format!("{lead}\n\nContinue from a {provider} session. Do not invent prior work.")
    } else {
        format!(
            "{lead}\n\nPrior conversation from {provider} — this is the thread you are joining, not optional background:\n\n<handoff>\n{body}\n</handoff>"
        )
    }
}

pub fn strip_goal_sections(markdown: &str) -> String {
    let mut skip_section = false;
    let mut removed_goal_line = false;
    markdown
        .trim()
        .lines()
        .filter(|line| {
            let heading = line.trim_start_matches('#');
            let depth = line.len() - heading.len();
            if (1..=6).contains(&depth) && heading.starts_with(char::is_whitespace) {
                let title = heading.trim_start().to_lowercase();
                skip_section = title.strip_prefix("goal").is_some_and(|rest| {
                    rest.chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                });
            }
            if skip_section {
                return false;
            }
            if !removed_goal_line && line.to_lowercase().starts_with("goal:") {
                removed_goal_line = true;
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(role: &str, text: &str, first: bool) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first,
            sent_at: None,
        }
    }
    #[test]
    fn last_assistant_text_returns_the_whole_final_reply() {
        let long = "x".repeat(30_000);
        let rows = vec![row("user", "go", true), row("assistant", &long, true)];
        assert_eq!(last_assistant_text(&rows), long);
    }
    #[test]
    fn last_assistant_text_ignores_earlier_turns_and_tool_rows() {
        let rows = vec![
            row("user", "one", true),
            row("assistant", "old answer", true),
            row("user", "two", true),
            row("tool", "{\"name\":\"Read\"}", true),
            row("assistant", "first part", true),
            row("tool", "{\"name\":\"Edit\"}", true),
            row("assistant", "final ```json\n{}\n```\r\n", true),
        ];
        assert_eq!(
            last_assistant_text(&rows),
            "first part\n\nfinal ```json\n{}\n```"
        );
    }
    #[test]
    fn last_assistant_text_is_empty_without_a_reply_after_the_last_user_row() {
        assert_eq!(last_assistant_text(&[]), "");
        let rows = vec![
            row("user", "one", true),
            row("assistant", "old", true),
            row("user", "two", true),
            row("tool", "{}", true),
        ];
        assert_eq!(last_assistant_text(&rows), "");
    }
    #[test]
    fn last_assistant_text_without_any_user_row_uses_all_assistant_text() {
        let rows = vec![row("assistant", "hello", true)];
        assert_eq!(last_assistant_text(&rows), "hello");
    }
    #[test]
    fn copied_prose_and_transfer_recaps_keep_message_boundaries() {
        let rows = vec![
            Row {
                role: "user".into(),
                text: "Check the footer".into(),
                first: true,
                sent_at: None,
            },
            Row {
                role: "assistant".into(),
                text: "I’ll inspect ".into(),
                first: true,
                sent_at: None,
            },
            Row {
                role: "assistant".into(),
                text: "the footer.".into(),
                first: false,
                sent_at: None,
            },
            Row {
                role: "reasoning".into(),
                text: "Private reasoning".into(),
                first: true,
                sent_at: None,
            },
            Row {
                role: "tool".into(),
                text: "Tool output".into(),
                first: true,
                sent_at: None,
            },
            Row {
                role: "assistant".into(),
                text: "**The footer is ready.**\n\n```rust\nlet ready = true;\n```".into(),
                first: true,
                sent_at: None,
            },
        ];
        assert_eq!(
            prose(&rows),
            "I’ll inspect the footer.\n\n**The footer is ready.**\n\n```rust\nlet ready = true;\n```"
        );
        let recap = handoff(&rows);
        assert!(recap.contains("User: Check the footer"));
        assert!(recap.contains("**The footer is ready.**"));
        assert!(!recap.contains("Private reasoning"));
        let review = second_opinion("codex", &rows);
        assert!(review.contains("## User request\nCheck the footer"));
        assert!(review.contains("## What Codex reported"));
        assert!(!review.contains("Tool output"));
    }

    /// A small chat: two turns, the second with a tool call between replies
    /// and a prompt split across two rows.
    fn chat() -> Vec<Row> {
        vec![
            row("user", "Plan the footer", true),
            row("assistant", "Here is the plan.", true),
            row("user", "Build ", true),
            row("user", "it now", false),
            row("assistant", "Building.", true),
            row("tool", "{\"name\":\"Edit\"}", true),
            row("assistant", "Done.", true),
        ]
    }
    #[test]
    fn fork_at_a_response_keeps_everything_through_that_turn() {
        let rows = chat();
        let fork = fork_at(&rows, 1);
        assert_eq!(fork.rows, rows[..2]);
        assert_eq!(fork.draft, None);
        // Any row inside the second turn keeps the whole turn, tool rows and all.
        for at in 4..rows.len() {
            assert_eq!(fork_at(&rows, at).rows, rows, "forking at row {at}");
        }
    }
    #[test]
    fn fork_at_the_last_message_copies_the_whole_chat() {
        let rows = chat();
        let fork = fork_at(&rows, rows.len() - 1);
        assert_eq!(fork.rows, rows);
        assert_eq!(fork.draft, None);
        // A stale index past the end still means "the last message".
        assert_eq!(fork_at(&rows, 99).rows, rows);
    }
    #[test]
    fn fork_at_the_first_message_starts_empty_with_that_prompt_as_a_draft() {
        let fork = fork_at(&chat(), 0);
        assert!(fork.rows.is_empty());
        assert_eq!(fork.draft.as_deref(), Some("Plan the footer"));
    }
    #[test]
    fn fork_at_a_later_prompt_branches_before_it_with_the_whole_prompt() {
        let rows = chat();
        for at in [2, 3] {
            let fork = fork_at(&rows, at);
            assert_eq!(fork.rows, rows[..2], "forking at row {at}");
            assert_eq!(fork.draft.as_deref(), Some("Build it now"));
        }
    }
    #[test]
    fn fork_of_an_empty_chat_is_empty() {
        assert_eq!(
            fork_at(&[], 0),
            Fork {
                rows: vec![],
                draft: None
            }
        );
    }
    #[test]
    fn a_handoff_from_an_empty_chat_has_no_recap() {
        assert_eq!(handoff(&[]), "");
        assert_eq!(handoff(&[row("tool", "{\"name\":\"Read\"}", true)]), "");
        // The incoming agent is told there is no prior work to invent.
        let prompt = wrap_handoff_prompt("claude", &handoff(&[]), "Start the footer");
        assert!(prompt.contains("Do not invent prior work"));
        assert!(!prompt.contains("<handoff>"));
    }
    #[test]
    fn carried_settings_keep_the_agent_setup_and_the_copied_turns_only() {
        let turn = |model: &str| crate::agent::TurnInfo {
            provider: "claude".into(),
            model: model.into(),
            ..Default::default()
        };
        let source = crate::agent::Settings {
            model: Some("opus".into()),
            effort: Some("high".into()),
            fast: true,
            access: crate::agent::AccessMode::FullAccess,
            worktree_cwd: Some("/work/tree".into()),
            handoff: Some(crate::agent::Handoff {
                from: "codex".into(),
                brief: "old".into(),
            }),
            plan: true,
            ..Default::default()
        };
        let turns = [(1, turn("first")), (4, turn("second"))].into();
        let carried = carried_settings(&source, &turns, 2);
        assert_eq!(carried.model.as_deref(), Some("opus"));
        assert_eq!(carried.effort.as_deref(), Some("high"));
        assert!(carried.fast);
        assert_eq!(carried.access, source.access);
        assert_eq!(carried.worktree_cwd.as_deref(), Some("/work/tree"));
        assert_eq!(carried.turns.keys().collect::<Vec<_>>(), [&1]);
        // One-shot state of the source chat does not leak into the new one.
        assert!(carried.handoff.is_none());
        assert!(!carried.plan);
        assert!(carried.revision > source.revision);
    }
}
