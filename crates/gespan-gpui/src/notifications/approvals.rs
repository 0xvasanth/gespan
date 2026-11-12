//! Requests waiting on the user, as the phone and the system notifications
//! name them, and which of them have been announced. They are answered on
//! My tasks or in their chat; the safety markings here are shared with the
//! chat's approval card.
use gespan_core::agent::Approval;
use serde_json::Value;
use std::{collections::HashSet, fmt::Write};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// Claude's field for running a command outside its sandbox.
const SANDBOX_OVERRIDE: &str = "dangerouslyDisableSandbox";
/// The longest value an alert quotes.
const MAX_FIELD_CHARS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InputKind {
    Approval,
    Question,
}

/// One request a session waits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingInput {
    pub request_id: String,
    pub kind: InputKind,
    /// What is asked, with hidden characters marked.
    pub label: String,
}

impl PendingInput {
    /// `approval` in a session whose folder is `cwd`, named as its card in
    /// the chat names it (an MCP tool by its own name).
    pub(crate) fn approval_in(approval: &Approval, cwd: Option<&str>) -> Self {
        Self {
            request_id: approval.id.clone(),
            kind: InputKind::Approval,
            label: crate::conversation::approval_card::Card::new(approval, cwd).title(approval),
        }
    }
}

/// The command, file or URL a JSON tool input is about, and its key.
fn tool_input(detail: &str) -> Option<(Value, &'static str)> {
    let input = serde_json::from_str::<Value>(detail.trim()).ok()?;
    let key = ["command", "file_path", "path", "url"]
        .into_iter()
        .find(|key| input[*key].is_string())?;
    Some((input, key))
}

/// Characters that change how text reads without showing up themselves:
/// controls, bidi overrides, zero-width marks and line separators.
fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c.general_category(),
            GeneralCategory::Format
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
        )
}

/// Line breaks and tabs are how a multi-line command really reads.
fn distorts_text(c: char) -> bool {
    is_hidden(c) && !matches!(c, '\n' | '\t')
}

/// `text` with each hidden character replaced by a tag such as `[U+202E]`.
/// Removing them would show something other than what runs: `r\u{200B}m` is
/// not `rm`. The card is display only, so what runs keeps them.
pub(crate) fn mark_hidden(text: &str) -> String {
    let mut marked = String::with_capacity(text.len());
    for c in text.chars() {
        if distorts_text(c) {
            let _ = write!(marked, "[U+{:04X}]", c as u32);
        } else {
            marked.push(c);
        }
    }
    marked
}

/// The part of an approval to judge. Claude sends the tool's input as JSON,
/// where the command or file is what matters.
fn judged_text(detail: &str) -> String {
    tool_input(detail)
        .and_then(|(input, key)| input[key].as_str().map(str::to_owned))
        .unwrap_or_else(|| detail.trim().to_owned())
        .replace("\r\n", "\n")
}

/// A value cut down to something an alert can quote.
fn compact(text: &str) -> String {
    let visible: Vec<char> = text.chars().filter(|c| !is_hidden(*c)).collect();
    match visible.len() {
        0 => "?".to_owned(),
        len if len > MAX_FIELD_CHARS => {
            let kept: String = visible[..MAX_FIELD_CHARS].iter().collect();
            format!("{kept}…")
        }
        _ => visible.into_iter().collect(),
    }
}

fn sandbox_override(detail: &str) -> Option<Value> {
    serde_json::from_str::<Value>(detail.trim())
        .ok()?
        .get(SANDBOX_OVERRIDE)
        .cloned()
}

/// Anything but an explicit "no" counts, so an odd value can't read as safe.
fn turns_sandbox_off(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

/// Warnings a card leads with, because the scrolling detail could hide them.
pub(crate) fn approval_alerts(detail: &str) -> Vec<String> {
    let sandbox = sandbox_override(detail)
        .filter(turns_sandbox_off)
        .map(|value| {
            format!(
                "Sandbox off ({SANDBOX_OVERRIDE}: {})",
                compact(&value.to_string())
            )
        });
    let hidden = judged_text(detail)
        .contains(distorts_text)
        .then(|| "Hidden characters, marked [U+…]".to_owned());
    sandbox.into_iter().chain(hidden).collect()
}

/// What one session waits on.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionInputs {
    pub session_id: String,
    /// Approvals in transcript order, followed by the pending question(s).
    pub pending: Vec<PendingInput>,
    pub inbox_ask: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InputKey {
    pub session_id: String,
    pub kind: InputKind,
    pub request_id: String,
}
impl SessionInputs {
    fn key(&self, input: &PendingInput) -> InputKey {
        InputKey {
            session_id: self.session_id.clone(),
            kind: input.kind,
            request_id: input.request_id.clone(),
        }
    }
}

/// Mirrors useInputNotifications: coalesce each update per session, retain
/// unannounced requests, and forget resolved keys. Mark before policy checks.
#[derive(Default)]
pub(crate) struct InputAnnouncements {
    notified: HashSet<InputKey>,
}
impl InputAnnouncements {
    pub(crate) fn reconcile(
        &mut self,
        sessions: &[SessionInputs],
    ) -> Vec<(SessionInputs, PendingInput)> {
        let pending: HashSet<_> = sessions
            .iter()
            .filter(|s| !s.inbox_ask)
            .flat_map(|s| s.pending.iter().map(|p| s.key(p)))
            .collect();
        self.notified.retain(|key| pending.contains(key));
        let mut announcements = Vec::new();
        for session in sessions.iter().filter(|s| !s.inbox_ask) {
            if let Some(input) = session
                .pending
                .iter()
                .find(|p| !self.notified.contains(&session.key(p)))
            {
                self.notified.insert(session.key(input));
                announcements.push((session.clone(), input.clone()));
            }
        }
        announcements
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(id: &str) -> PendingInput {
        PendingInput {
            request_id: id.into(),
            kind: InputKind::Approval,
            label: "Allow Bash?".into(),
        }
    }

    fn waiting(session_id: &str, ids: &[&str]) -> SessionInputs {
        SessionInputs {
            session_id: session_id.into(),
            pending: ids.iter().map(|id| approval(id)).collect(),
            inbox_ask: false,
        }
    }

    #[test]
    fn each_request_is_announced_once_until_it_is_resolved() {
        let mut announced = InputAnnouncements::default();
        let first = announced.reconcile(&[waiting("a", &["a1"])]);
        assert_eq!(first.len(), 1);
        assert!(announced.reconcile(&[waiting("a", &["a1"])]).is_empty());
        // A second request in the same session gets its own announcement.
        let next = announced.reconcile(&[waiting("a", &["a1", "a2"])]);
        assert_eq!(next[0].1.request_id, "a2");
        // Resolved and asked again: a new announcement.
        announced.reconcile(&[waiting("a", &[])]);
        assert_eq!(announced.reconcile(&[waiting("a", &["a1"])]).len(), 1);
    }

    #[test]
    fn an_approval_is_named_like_its_card_with_hidden_characters_marked() {
        let mut call = gespan_core::tool::ToolCall::new("t1", "mcp__github__create_issue");
        call.input = serde_json::json!({ "owner": "acme" }).to_string();
        let mcp = Approval {
            id: "a1".into(),
            title: "Allow mcp__github__create_issue?".into(),
            detail: String::new(),
            can_remember: false,
            tool: Some(call),
        };
        assert_eq!(
            PendingInput::approval_in(&mcp, None).label,
            "Allow create_issue?"
        );
        let sneaky = Approval {
            title: "Allow r\u{200B}m?".into(),
            tool: None,
            ..mcp
        };
        assert_eq!(
            PendingInput::approval_in(&sneaky, None).label,
            "Allow r[U+200B]m?"
        );
    }

    #[test]
    fn a_sandbox_override_that_is_on_is_alerted_with_its_value() {
        for (value, shown) in [
            ("true", "true"),
            ("1", "1"),
            ("\"yes\"", "\"yes\""),
            ("{}", "{}"),
        ] {
            let input = format!(r#"{{"command":"ls","dangerouslyDisableSandbox":{value}}}"#);
            assert_eq!(
                approval_alerts(&input),
                [format!("Sandbox off (dangerouslyDisableSandbox: {shown})")]
            );
        }
    }

    #[test]
    fn a_sandbox_override_that_is_off_raises_no_alert() {
        for value in ["false", "null"] {
            let input = format!(r#"{{"command":"ls","dangerouslyDisableSandbox":{value}}}"#);
            assert!(approval_alerts(&input).is_empty());
        }
    }

    #[test]
    fn a_sandbox_override_is_alerted_whatever_else_the_input_holds() {
        let input = r#"{"query":"x","dangerouslyDisableSandbox":true}"#;
        assert_eq!(
            approval_alerts(input),
            ["Sandbox off (dangerouslyDisableSandbox: true)"]
        );
    }

    #[test]
    fn a_sandbox_override_cannot_carry_a_long_or_hidden_value() {
        let value = format!("\u{202E}{}", "x".repeat(5_000));
        let input = serde_json::json!({ "command": "ls", "dangerouslyDisableSandbox": value });
        let [alert] = &approval_alerts(&input.to_string())[..] else {
            panic!("one alert");
        };
        assert!(alert.chars().count() < 100, "{alert}");
        assert!(!alert.contains('\u{202E}'), "{alert}");
        assert!(alert.ends_with("…)"), "{alert}");
    }

    #[test]
    fn a_lookalike_of_the_sandbox_field_raises_no_alert() {
        let input =
            serde_json::json!({ "command": "ls", "dangerously\u{200B}DisableSandbox": true });
        assert!(approval_alerts(&input.to_string()).is_empty());
    }

    #[test]
    fn hidden_characters_are_marked_where_they_sit() {
        assert_eq!(
            mark_hidden("ls \u{202E}cod.exe\u{202C} r\u{200B}m"),
            "ls [U+202E]cod.exe[U+202C] r[U+200B]m"
        );
        assert_eq!(
            mark_hidden("a\u{2028}b\u{1B}[2Kc"),
            "a[U+2028]b[U+001B][2Kc"
        );
        assert_eq!(mark_hidden("flag\u{E0041}"), "flag[U+E0041]");
    }

    #[test]
    fn a_marked_command_is_alerted() {
        assert_eq!(
            approval_alerts("ls\u{202E}"),
            ["Hidden characters, marked [U+…]"]
        );
        let both = serde_json::json!({
            "command": "ls\u{200B}",
            "dangerouslyDisableSandbox": true,
        });
        assert_eq!(approval_alerts(&both.to_string()).len(), 2);
    }

    #[test]
    fn a_multi_line_command_raises_no_alert_but_a_stray_return_does() {
        for command in ["cd a\n\tls", "cd a\r\n\tls"] {
            assert!(approval_alerts(command).is_empty());
        }
        assert_eq!(approval_alerts("safe\rrm -rf /").len(), 1);
    }

    #[test]
    fn plain_unicode_is_left_alone() {
        let text = "echo café 漢字 🙂";
        assert_eq!(mark_hidden(text), text);
        assert!(approval_alerts(text).is_empty());
    }
}
