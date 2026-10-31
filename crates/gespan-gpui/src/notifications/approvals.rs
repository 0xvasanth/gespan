//! Pending approval/question cards and request-level announcement bookkeeping.
use crate::theme::*;
use gespan_core::agent::{Approval, Decision};
use gpui::{Context, EventEmitter, Role, div, prelude::*, px};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    time::SystemTime,
};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// Claude's field for running a command outside its sandbox.
const SANDBOX_OVERRIDE: &str = "dangerouslyDisableSandbox";
const MAX_NAMED_FIELDS: usize = 4;
const MAX_FIELD_CHARS: usize = 40;
/// The rows the card's scrolling detail shows before anything is out of view.
const DETAIL_ROWS: usize = 6;
/// Fewer cells than a row really holds, so an estimate can't undercount.
const DETAIL_ROW_CELLS: usize = 44;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InputKind {
    Approval,
    Question,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingInput {
    pub request_id: String,
    pub kind: InputKind,
    pub label: String,
    /// What is being approved, so the card can be decided without the chat.
    pub detail: String,
    /// Names the fields `detail` leaves out. It sits outside the scrolling
    /// detail, so a long command can't push it out of view.
    pub omitted: String,
    /// What the reader must not miss. These sit above the scrolling detail.
    pub alerts: Vec<String>,
    /// The request has something the card can't fully show, so Allow is only
    /// offered in the chat.
    pub needs_review: bool,
}

impl PendingInput {
    /// What a card shows for `approval`. Only the id goes back with a decision,
    /// so none of this cleanup reaches what runs.
    pub(crate) fn approval(approval: &Approval) -> Self {
        let detail = approval_detail(&approval.detail);
        let alerts = approval_alerts(&approval.detail);
        Self {
            request_id: approval.id.clone(),
            kind: InputKind::Approval,
            label: mark_hidden(&approval.title),
            needs_review: !alerts.is_empty()
                || approval.title.contains(distorts_text)
                || overflows_detail(&detail),
            detail,
            omitted: approval_omitted(&approval.detail),
            alerts,
        }
    }
}

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
fn mark_hidden(text: &str) -> String {
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

/// An approval's detail as a card shows it.
fn approval_detail(detail: &str) -> String {
    mark_hidden(&judged_text(detail))
}

/// Rows `line` takes at most. Wrapping only at spaces can't give fewer rows
/// than the card's own wrapping, and wide characters count double.
fn wrapped_rows(line: &str) -> usize {
    let (mut rows, mut used) = (1, 0);
    for word in line.split_inclusive(' ') {
        let cells: usize = word
            .chars()
            .map(|c| match c {
                '\t' => 4,
                c if c.is_ascii() => 1,
                _ => 2,
            })
            .sum();
        if used > 0 && used + cells > DETAIL_ROW_CELLS {
            rows += 1;
            used = 0;
        }
        used += cells;
        while used > DETAIL_ROW_CELLS {
            rows += 1;
            used -= DETAIL_ROW_CELLS;
        }
    }
    rows
}

/// Worked out from the text rather than measured, because a measurement only
/// exists after the first frame, and that frame would offer Allow.
fn overflows_detail(detail: &str) -> bool {
    detail.lines().map(wrapped_rows).sum::<usize>() > DETAIL_ROWS
}

/// A field name or value cut down to something that fits on the card.
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

/// The fields [`approval_detail`] leaves out, so a card never reads as the
/// whole request when it isn't. Dangerous names lead so a long list can't bury
/// them, and a sandbox override that's on has its own alert instead.
fn approval_omitted(detail: &str) -> String {
    let Some((input, key)) = tool_input(detail) else {
        return String::new();
    };
    let sandbox = sandbox_override(detail);
    let alerted = sandbox.as_ref().is_some_and(turns_sandbox_off);
    let mut others: Vec<&String> = input
        .as_object()
        .into_iter()
        .flat_map(|fields| fields.keys())
        .filter(|other| *other != key && *other != "description")
        .filter(|other| !(alerted && *other == SANDBOX_OVERRIDE))
        .collect();
    others
        .sort_unstable_by_key(|other| (!other.to_ascii_lowercase().contains("dangerous"), *other));
    let mut named: Vec<String> = others
        .iter()
        .take(MAX_NAMED_FIELDS)
        .map(|other| match &sandbox {
            Some(value) if *other == SANDBOX_OVERRIDE => {
                format!("{other}: {}", compact(&value.to_string()))
            }
            _ => compact(other),
        })
        .collect();
    if others.len() > MAX_NAMED_FIELDS {
        named.push(format!("+{} more", others.len() - MAX_NAMED_FIELDS));
    }
    if named.is_empty() {
        String::new()
    } else {
        format!("+ {}", named.join(", "))
    }
}

/// Warnings a card leads with, because the scrolling detail could hide them.
fn approval_alerts(detail: &str) -> Vec<String> {
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

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionInputs {
    pub session_id: String,
    pub title: String,
    /// Provider key accepted by `theme::provider_icon`.
    pub provider: String,
    pub harness: String,
    /// Approvals in transcript order, followed by the pending question(s).
    pub pending: Vec<PendingInput>,
    pub inbox_ask: bool,
    pub orchestration_worker: bool,
    /// Active tab, focused pane, and focused composer must all match.
    pub conversation_focused: bool,
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
    fn notice(&self) -> Option<&PendingInput> {
        self.pending
            .iter()
            .find(|p| p.kind == InputKind::Question)
            .or_else(|| {
                self.pending
                    .iter()
                    .rev()
                    .find(|p| p.kind == InputKind::Approval)
            })
    }
}

pub(crate) enum Action {
    OpenSession {
        session_id: String,
    },
    Decide {
        session_id: String,
        request_id: String,
        decision: Decision,
    },
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

#[derive(Default)]
pub(crate) struct ApprovalNotices {
    notices: Vec<(SessionInputs, PendingInput)>,
    occurred: HashMap<InputKey, SystemTime>,
}
impl EventEmitter<Action> for ApprovalNotices {}
impl ApprovalNotices {
    /// Observe pending requests from session events even while the window is
    /// occluded. Policy must retain this first observation until resolution.
    pub(crate) fn track_pending(&mut self, sessions: &[SessionInputs]) {
        let now = SystemTime::now();
        let pending: HashSet<_> = sessions
            .iter()
            .flat_map(|session| session.pending.iter().map(|input| session.key(input)))
            .collect();
        self.occurred.retain(|key, _| pending.contains(key));
        for key in pending {
            self.occurred.entry(key).or_insert(now);
        }
    }
    /// `allows` must apply known-project + agentInput project policy using the
    /// original occurrence timestamp, so resuming cannot revive muted requests.
    pub(crate) fn sync(
        &mut self,
        sessions: &[SessionInputs],
        mut allows: impl FnMut(&SessionInputs, SystemTime) -> bool,
        cx: &mut Context<Self>,
    ) {
        self.track_pending(sessions);
        let previous = std::mem::take(&mut self.notices);
        for session in sessions {
            if session.inbox_ask || session.orchestration_worker || session.conversation_focused {
                continue;
            }
            let Some(input) = session.notice() else {
                continue;
            };
            let key = session.key(input);
            let occurred = *self.occurred.entry(key).or_insert_with(SystemTime::now);
            if allows(session, occurred) {
                self.notices.push((session.clone(), input.clone()));
            }
        }
        // Shell syncs on every render; notifying unchanged notices would
        // dirty the shell again and redraw the window every frame.
        if self.notices != previous {
            cx.notify();
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.notices.is_empty()
    }
}
impl Render for ApprovalNotices {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .children(
                self.notices
                    .iter()
                    .enumerate()
                    .map(|(ix, (session, input))| {
                        let open_id = session.session_id.clone();
                        let allow_session = session.session_id.clone();
                        let deny_session = session.session_id.clone();
                        let allow_request = input.request_id.clone();
                        let needs_review = input.needs_review;
                        let primary = if needs_review {
                            "Review in chat"
                        } else {
                            "Allow"
                        };
                        let deny_request = input.request_id.clone();
                        super::surface::toast_frame()
                            .id(("approval-toast", ix))
                            .role(Role::Status)
                            .child(
                                div()
                                    .id(("approval-toast-open", ix))
                                    .role(Role::Button)
                                    .cursor_pointer()
                                    .px(px(14.))
                                    .py_3()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .hover(|s| s.bg(content(0.05)))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(Action::OpenSession {
                                            session_id: open_id.clone(),
                                        })
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(provider_icon(&session.provider, 16.))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .flex_1()
                                                    .truncate()
                                                    .text_size(px(13.))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(session.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .text_size(px(11.))
                                                    .text_color(gpui::rgb(0xffb900))
                                                    .child(
                                                        icon("circle-alert", 14.)
                                                            .text_color(gpui::rgb(0xffb900)),
                                                    )
                                                    .child(if input.kind == InputKind::Question {
                                                        "Question"
                                                    } else {
                                                        "Approval"
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .line_height(px(19.5))
                                            .line_clamp(3)
                                            .text_color(content(0.7))
                                            .child(input.label.clone()),
                                    )
                                    .children(input.alerts.iter().enumerate().map(
                                        |(alert_ix, alert)| {
                                            div()
                                                .debug_selector(move || {
                                                    format!("approval-toast-alert-{alert_ix}")
                                                })
                                                .flex()
                                                .gap_1()
                                                .rounded_md()
                                                .px(px(8.))
                                                .py_1()
                                                .bg(gpui::rgba(0xf8717126))
                                                .text_size(px(11.))
                                                .line_height(px(17.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(gpui::rgb(0xf87171))
                                                .child(
                                                    icon("circle-alert", 14.)
                                                        .flex_none()
                                                        .text_color(gpui::rgb(0xf87171)),
                                                )
                                                .child(
                                                    div()
                                                        .min_w_0()
                                                        .line_clamp(2)
                                                        .child(alert.clone()),
                                                )
                                        },
                                    ))
                                    .when(!input.detail.is_empty(), |card| {
                                        card.child(
                                            div()
                                                .id(("approval-toast-detail", ix))
                                                .debug_selector(|| "approval-toast-detail".into())
                                                .font_family(crate::fonts::monospace())
                                                .text_size(px(11.))
                                                .line_height(px(17.))
                                                .max_h(px(102.))
                                                .overflow_y_scroll()
                                                .text_color(content(0.85))
                                                .child(input.detail.clone()),
                                        )
                                    })
                                    .when(!input.omitted.is_empty(), |card| {
                                        card.child(
                                            div()
                                                .id(("approval-toast-omitted", ix))
                                                .debug_selector(|| "approval-toast-omitted".into())
                                                .font_family(crate::fonts::monospace())
                                                .text_size(px(11.))
                                                .line_height(px(17.))
                                                .flex_none()
                                                .max_h(px(68.))
                                                .overflow_y_scroll()
                                                .text_color(content(0.85))
                                                .child(input.omitted.clone()),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(content(0.4))
                                            .child(session.harness.clone()),
                                    ),
                            )
                            .when(input.kind == InputKind::Approval, |card| {
                                card.child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .border_t_1()
                                        .border_color(content(0.1))
                                        .px(px(14.))
                                        .py(px(10.))
                                        .child(
                                            div()
                                                .id(("approval-toast-allow", ix))
                                                .debug_selector(move || {
                                                    if needs_review {
                                                        "approval-toast-review".into()
                                                    } else {
                                                        "approval-toast-allow".into()
                                                    }
                                                })
                                                .role(Role::Button)
                                                .aria_label(primary)
                                                .cursor_pointer()
                                                .flex_1()
                                                .rounded_md()
                                                .px(px(10.))
                                                .py_1()
                                                .text_center()
                                                .text_size(px(11.))
                                                .bg(content(1.))
                                                .text_color({
                                                    let mut color = pane_fill();
                                                    color.a = 1.;
                                                    color
                                                })
                                                .hover(|s| s.bg(content(0.8)))
                                                .child(primary)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.emit(if needs_review {
                                                        Action::OpenSession {
                                                            session_id: allow_session.clone(),
                                                        }
                                                    } else {
                                                        Action::Decide {
                                                            session_id: allow_session.clone(),
                                                            request_id: allow_request.clone(),
                                                            decision: Decision::Allow,
                                                        }
                                                    })
                                                })),
                                        )
                                        .child(
                                            div()
                                                .id(("approval-toast-deny", ix))
                                                .debug_selector(|| "approval-toast-deny".into())
                                                .role(Role::Button)
                                                .aria_label("Deny")
                                                .cursor_pointer()
                                                .flex_1()
                                                .rounded_md()
                                                .px(px(10.))
                                                .py_1()
                                                .text_center()
                                                .text_size(px(11.))
                                                .bg(content(0.1))
                                                .text_color(content(0.7))
                                                .hover(|s| s.bg(content(0.2)))
                                                .child("Deny")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.emit(Action::Decide {
                                                        session_id: deny_session.clone(),
                                                        request_id: deny_request.clone(),
                                                        decision: Decision::Deny,
                                                    })
                                                })),
                                        ),
                                )
                            })
                    }),
            )
    }
}

#[cfg(test)]
mod detail_tests {
    use super::{Approval, PendingInput, approval_alerts, approval_detail, approval_omitted};

    #[test]
    fn a_shell_command_shows_as_it_was_sent() {
        assert_eq!(
            approval_detail("  /bin/zsh -lc 'sleep 150'\n"),
            "/bin/zsh -lc 'sleep 150'"
        );
    }

    #[test]
    fn claude_tool_input_shows_the_part_to_judge() {
        assert_eq!(
            approval_detail(r#"{"command":"rm -rf build","description":"Clean"}"#),
            "rm -rf build"
        );
        assert_eq!(
            approval_detail(r#"{"url":"https://a.dev"}"#),
            "https://a.dev"
        );
    }

    #[test]
    fn fields_the_card_leaves_out_are_named() {
        let write = r#"{"file_path":"/repo/a.md","content":"x"}"#;
        assert_eq!(approval_detail(write), "/repo/a.md");
        assert_eq!(approval_omitted(write), "+ content");
        let shell =
            r#"{"timeout":5,"dangerouslyDisableSandbox":true,"command":"ls","description":"List"}"#;
        assert_eq!(approval_detail(shell), "ls");
        assert_eq!(approval_omitted(shell), "+ timeout");
    }

    #[test]
    fn nothing_is_named_when_nothing_is_left_out() {
        assert_eq!(
            approval_omitted(r#"{"command":"ls","description":"List"}"#),
            ""
        );
        assert_eq!(approval_omitted(r#"{"query":"gespan"}"#), "");
        assert_eq!(approval_omitted("/bin/zsh -lc ls"), "");
    }

    #[test]
    fn a_field_name_cannot_break_the_line_it_is_named_on() {
        assert_eq!(
            approval_omitted(r#"{"command":"ls","a\n+ nothing else":1}"#),
            "+ a+ nothing else"
        );
    }

    #[test]
    fn other_input_is_shown_in_full_rather_than_hidden() {
        let input = r#"{"query":"gespan"}"#;
        assert_eq!(approval_detail(input), input);
        assert_eq!(approval_detail("["), "[");
        assert_eq!(approval_detail(""), "");
    }

    fn input_with_fields(names: &[&str]) -> String {
        let mut input = serde_json::json!({ "command": "ls" });
        for name in names {
            input[*name] = 1.into();
        }
        input.to_string()
    }

    #[test]
    fn a_long_field_name_is_cut_and_says_so() {
        let exact = "n".repeat(40);
        assert_eq!(
            approval_omitted(&input_with_fields(&[&exact])),
            format!("+ {exact}")
        );
        let long = "n".repeat(10_000);
        assert_eq!(
            approval_omitted(&input_with_fields(&[&long])),
            format!("+ {exact}…")
        );
        let wide = "漢".repeat(41);
        assert_eq!(
            approval_omitted(&input_with_fields(&[&wide])),
            format!("+ {}…", "漢".repeat(40))
        );
    }

    #[test]
    fn hidden_characters_are_stripped_from_field_names() {
        for hidden in [
            "\u{202E}",
            "\u{2066}",
            "\u{200B}",
            "\u{FEFF}",
            "\u{2028}",
            "\u{2029}",
            "\u{0085}",
            "\u{1B}",
            "\u{E0041}",
        ] {
            assert_eq!(
                approval_omitted(&input_with_fields(&[&format!("a{hidden}b")])),
                "+ ab",
                "{hidden:?}"
            );
        }
    }

    #[test]
    fn hidden_characters_do_not_use_up_a_names_length() {
        let padded = format!("{}name", "\u{200B}".repeat(100));
        assert_eq!(approval_omitted(&input_with_fields(&[&padded])), "+ name");
    }

    #[test]
    fn a_name_of_only_hidden_characters_is_still_counted() {
        assert_eq!(
            approval_omitted(&input_with_fields(&["\u{202E}\u{200B}", "timeout"])),
            "+ timeout, ?"
        );
    }

    #[test]
    fn a_long_list_is_cut_with_a_count_of_the_rest() {
        let four = input_with_fields(&["a", "b", "c", "d"]);
        assert_eq!(approval_omitted(&four), "+ a, b, c, d");
        let five = input_with_fields(&["a", "b", "c", "d", "e"]);
        assert_eq!(approval_omitted(&five), "+ a, b, c, d, +1 more");
        let names: Vec<String> = (0..500).map(|i| format!("f{i:03}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(
            approval_omitted(&input_with_fields(&names)),
            "+ f000, f001, f002, f003, +496 more"
        );
    }

    #[test]
    fn a_long_list_cannot_bury_a_dangerous_field() {
        assert_eq!(
            approval_omitted(&input_with_fields(&[
                "a",
                "b",
                "c",
                "d",
                "e",
                "zDangerousMode",
                "dangerouslySkipPermissions",
            ])),
            "+ dangerouslySkipPermissions, zDangerousMode, a, b, +3 more"
        );
        let mut input: serde_json::Value =
            serde_json::from_str(&input_with_fields(&["a", "b", "c", "d", "e", "f"])).unwrap();
        input["dangerouslyDisableSandbox"] = false.into();
        assert_eq!(
            approval_omitted(&input.to_string()),
            "+ dangerouslyDisableSandbox: false, a, b, c, +3 more"
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
            assert_eq!(approval_omitted(&input), "");
        }
    }

    #[test]
    fn a_sandbox_override_that_is_off_is_named_without_an_alert() {
        for value in ["false", "null"] {
            let input = format!(r#"{{"command":"ls","dangerouslyDisableSandbox":{value}}}"#);
            assert!(approval_alerts(&input).is_empty());
            assert_eq!(
                approval_omitted(&input),
                format!("+ dangerouslyDisableSandbox: {value}")
            );
        }
    }

    #[test]
    fn a_sandbox_override_is_alerted_even_when_the_input_shows_as_raw_json() {
        let input = r#"{"query":"x","dangerouslyDisableSandbox":true}"#;
        assert_eq!(approval_detail(input), input);
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
        let input = input_with_fields(&["dangerously\u{200B}DisableSandbox"]);
        assert!(approval_alerts(&input).is_empty());
    }

    #[test]
    fn hidden_characters_in_a_command_are_marked_where_they_sit() {
        let shell = serde_json::json!({ "command": "ls \u{202E}cod.exe\u{202C} r\u{200B}m" });
        assert_eq!(
            approval_detail(&shell.to_string()),
            "ls [U+202E]cod.exe[U+202C] r[U+200B]m"
        );
        assert_eq!(
            approval_detail("a\u{2028}b\u{1B}[2Kc"),
            "a[U+2028]b[U+001B][2Kc"
        );
        assert_eq!(approval_detail("flag\u{E0041}"), "flag[U+E0041]");
    }

    #[test]
    fn a_marked_command_says_so_above_the_scrolling_detail() {
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
    fn a_multi_line_command_keeps_its_lines_and_raises_no_alert() {
        for command in ["cd a\n\tls", "cd a\r\n\tls"] {
            assert_eq!(approval_detail(command), "cd a\n\tls");
            assert!(approval_alerts(command).is_empty());
        }
        let stray = "safe\rrm -rf /";
        assert_eq!(approval_detail(stray), "safe[U+000D]rm -rf /");
        assert_eq!(approval_alerts(stray).len(), 1);
    }

    #[test]
    fn a_card_cleans_what_it_shows_and_leaves_the_request_alone() {
        let raw = serde_json::json!({ "command": "r\u{200B}m -rf \u{202E}build" }).to_string();
        let approval = Approval {
            id: "request-1".into(),
            title: "Allow \u{202E}Bash?".into(),
            detail: raw.clone(),
            can_remember: true,
        };
        let card = PendingInput::approval(&approval);
        assert_eq!(card.request_id, "request-1");
        assert_eq!(card.label, "Allow [U+202E]Bash?");
        assert_eq!(card.detail, "r[U+200B]m -rf [U+202E]build");
        assert_eq!(approval.detail, raw);
        assert_eq!(approval.title, "Allow \u{202E}Bash?");
    }

    fn card_for(detail: &str) -> PendingInput {
        PendingInput::approval(&Approval {
            id: "request".into(),
            title: "Allow Bash?".into(),
            detail: detail.to_owned(),
            can_remember: true,
        })
    }

    #[test]
    fn a_plain_request_can_be_allowed_from_the_card() {
        assert!(!card_for("ls").needs_review);
        assert!(!card_for(r#"{"file_path":"/repo/a.md","content":"x"}"#).needs_review);
        assert!(!card_for(r#"{"command":"ls","dangerouslyDisableSandbox":false}"#).needs_review);
        let names: Vec<String> = (0..500).map(|i| format!("f{i:03}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert!(!card_for(&input_with_fields(&names)).needs_review);
    }

    #[test]
    fn a_request_with_an_alert_needs_the_chat() {
        assert!(card_for(r#"{"command":"ls","dangerouslyDisableSandbox":true}"#).needs_review);
        assert!(card_for("ls\u{202E}").needs_review);
    }

    #[test]
    fn a_hidden_character_in_the_title_needs_the_chat() {
        let card = PendingInput::approval(&Approval {
            id: "request".into(),
            title: "Allow \u{202E}Bash?".into(),
            detail: "ls".into(),
            can_remember: true,
        });
        assert!(card.needs_review);
    }

    #[test]
    fn a_command_needs_the_chat_once_it_passes_the_rows_on_the_card() {
        assert!(!card_for(&["ls"; 6].join("\n")).needs_review);
        assert!(card_for(&["ls"; 7].join("\n")).needs_review);
        assert!(!card_for(&"x".repeat(6 * 44)).needs_review);
        assert!(card_for(&"x".repeat(6 * 44 + 1)).needs_review);
        assert!(!card_for(&"漢".repeat(6 * 22)).needs_review);
        assert!(card_for(&"漢".repeat(6 * 22 + 1)).needs_review);
    }

    #[test]
    fn long_words_cannot_wrap_past_the_card_unnoticed() {
        // 7 rows once wrapped, though it would fit 6 rows if it broke anywhere.
        assert!(card_for(&vec!["x".repeat(30); 7].join(" ")).needs_review);
    }

    #[test]
    fn plain_unicode_is_left_alone() {
        let text = "echo café 漢字 🙂";
        assert_eq!(approval_detail(text), text);
        assert!(approval_alerts(text).is_empty());
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, point, size};
    use std::{cell::RefCell, rc::Rc};

    const WINDOW_HEIGHT: f32 = 900.;

    fn session(input: PendingInput) -> SessionInputs {
        SessionInputs {
            session_id: "s".into(),
            title: "Session".into(),
            provider: "claude".into(),
            harness: "Claude".into(),
            pending: vec![input],
            inbox_ask: false,
            orchestration_worker: false,
            conversation_focused: false,
        }
    }

    fn pending(detail: &str) -> PendingInput {
        PendingInput::approval(&Approval {
            id: "r".into(),
            title: "Run a command".into(),
            detail: detail.to_owned(),
            can_remember: true,
        })
    }

    fn card(detail: &str, cx: &mut TestAppContext) -> VisualTestContext {
        card_with(pending(detail), cx)
    }

    fn window_with(
        input: PendingInput,
        cx: &mut TestAppContext,
    ) -> gpui::WindowHandle<ApprovalNotices> {
        cx.update(|cx| {
            cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(Bounds {
                        origin: point(gpui::px(0.), gpui::px(0.)),
                        size: size(gpui::px(340.), gpui::px(WINDOW_HEIGHT)),
                    })),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut notices = ApprovalNotices::default();
                        notices.sync(&[session(input)], |_, _| true, cx);
                        notices
                    })
                },
            )
            .unwrap()
        })
    }

    fn card_with(input: PendingInput, cx: &mut TestAppContext) -> VisualTestContext {
        let window = window_with(input, cx);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        visual
    }

    fn bounds(visual: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on the card"))
    }

    fn assert_decisions_on_screen(visual: &mut VisualTestContext) {
        let primary = visual
            .debug_bounds("approval-toast-allow")
            .or_else(|| visual.debug_bounds("approval-toast-review"))
            .expect("a primary button");
        assert!(primary.bottom() <= gpui::px(WINDOW_HEIGHT));
        assert!(bounds(visual, "approval-toast-deny").bottom() <= gpui::px(WINDOW_HEIGHT));
    }

    /// Clicks `selector` on a card for `detail`, and names the action it sent.
    fn click(detail: &str, selector: &'static str, cx: &mut TestAppContext) -> Vec<String> {
        let window = window_with(pending(detail), cx);
        let actions = Rc::new(RefCell::new(Vec::new()));
        let output = actions.clone();
        window
            .update(cx, |_, _, cx| {
                cx.subscribe(&cx.entity(), move |_, _, action: &Action, _| {
                    output.borrow_mut().push(match action {
                        Action::OpenSession { session_id } => format!("open {session_id}"),
                        Action::Decide {
                            request_id,
                            decision,
                            ..
                        } => format!("decide {request_id} {decision:?}"),
                    })
                })
                .detach()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let target = bounds(&mut visual, selector);
        visual.simulate_click(target.center(), Default::default());
        visual.run_until_parked();
        actions.take()
    }

    fn assert_stacked(visual: &mut VisualTestContext, selectors: &[&'static str]) {
        for pair in selectors.windows(2) {
            let (above, below) = (bounds(visual, pair[0]), bounds(visual, pair[1]));
            assert!(
                above.bottom() <= below.top(),
                "{} ends at {:?}, below {} at {:?}",
                pair[0],
                above.bottom(),
                pair[1],
                below.top()
            );
        }
    }

    #[gpui::test]
    fn the_omitted_row_stays_in_view_below_a_long_command(cx: &mut TestAppContext) {
        let command = vec!["echo line"; 500].join("\n");
        let input = serde_json::json!({ "command": command, "timeout": 5 }).to_string();
        let mut visual = card(&input, cx);
        assert_stacked(
            &mut visual,
            &[
                "approval-toast-detail",
                "approval-toast-omitted",
                "approval-toast-review",
            ],
        );
        assert!(bounds(&mut visual, "approval-toast-detail").size.height <= gpui::px(102.));
        assert_decisions_on_screen(&mut visual);
    }

    #[gpui::test]
    fn the_omitted_row_cannot_push_allow_and_deny_away(cx: &mut TestAppContext) {
        let mut visual = card_with(
            PendingInput {
                request_id: "r".into(),
                kind: InputKind::Approval,
                label: "Run a command".into(),
                detail: "ls".into(),
                omitted: vec!["field"; 500].join("\n"),
                alerts: Vec::new(),
                needs_review: false,
            },
            cx,
        );
        assert!(bounds(&mut visual, "approval-toast-omitted").size.height <= gpui::px(68.));
        assert_stacked(
            &mut visual,
            &["approval-toast-omitted", "approval-toast-allow"],
        );
        assert_decisions_on_screen(&mut visual);
    }

    #[gpui::test]
    fn alerts_sit_above_a_command_they_could_otherwise_be_scrolled_past(cx: &mut TestAppContext) {
        let command = format!("{}\u{202E}", vec!["echo line"; 500].join("\n"));
        let input = serde_json::json!({
            "command": command,
            "dangerouslyDisableSandbox": true,
            "timeout": 5,
        })
        .to_string();
        let mut visual = card(&input, cx);
        assert_stacked(
            &mut visual,
            &[
                "approval-toast-alert-0",
                "approval-toast-alert-1",
                "approval-toast-detail",
                "approval-toast-omitted",
                "approval-toast-review",
            ],
        );
    }

    #[gpui::test]
    fn a_plain_request_is_allowed_from_the_card(cx: &mut TestAppContext) {
        assert_eq!(click("ls", "approval-toast-allow", cx), ["decide r Allow"]);
    }

    #[gpui::test]
    fn a_risky_request_opens_the_chat_and_is_not_allowed(cx: &mut TestAppContext) {
        let sandbox = r#"{"command":"ls","dangerouslyDisableSandbox":true}"#;
        assert_eq!(click(sandbox, "approval-toast-review", cx), ["open s"]);
    }

    #[gpui::test]
    fn an_overflowing_command_opens_the_chat_and_is_not_allowed(cx: &mut TestAppContext) {
        let command = vec!["echo line"; 50].join("\n");
        assert_eq!(click(&command, "approval-toast-review", cx), ["open s"]);
    }

    #[gpui::test]
    fn a_risky_card_has_no_allow_button(cx: &mut TestAppContext) {
        let mut visual = card("ls\u{202E}", cx);
        assert!(visual.debug_bounds("approval-toast-allow").is_none());
    }

    #[gpui::test]
    fn a_risky_request_can_still_be_denied_from_the_card(cx: &mut TestAppContext) {
        assert_eq!(
            click("ls\u{202E}", "approval-toast-deny", cx),
            ["decide r Deny"]
        );
    }

    #[gpui::test]
    fn a_plain_request_has_no_alert_row(cx: &mut TestAppContext) {
        let mut visual = card(r#"{"command":"ls","dangerouslyDisableSandbox":false}"#, cx);
        assert!(visual.debug_bounds("approval-toast-alert-0").is_none());
        assert!(visual.debug_bounds("approval-toast-omitted").is_some());
    }
}
