//! Plans a local plan-mode turn leaves for review, as the Tauri app's
//! PlanPreview had them: the turn's final reply becomes a `plan` row the user
//! opens and builds.
use crate::transcript::Row;

pub const ROLE: &str = "plan";
/// The message a Build shows in the transcript, as in the Tauri app.
pub const BUILD_LABEL: &str = "Build approved plan";
const BUILD_PREAMBLE: &str = "The user reviewed and explicitly approved the following implementation plan. Implement it now, using this exact edited version as the source of truth.";

/// What the provider receives when the user builds `plan`.
pub fn build_prompt(plan: &str) -> String {
    format!(
        "{BUILD_PREAMBLE}\n\n<approved_plan>\n{}\n</approved_plan>",
        plan.trim()
    )
}

/// The approved plan inside a Build message, or None for any other message.
pub fn approved_plan(prompt: &str) -> Option<&str> {
    let body = prompt.trim().strip_prefix(BUILD_PREAMBLE)?.trim();
    body.strip_prefix("<approved_plan>")?
        .strip_suffix("</approved_plan>")
        .map(str::trim)
}

/// A user message that builds a plan: ours, or the Tauri app's label.
pub fn is_build(prompt: &str) -> bool {
    approved_plan(prompt).is_some() || prompt.trim() == BUILD_LABEL
}

/// Provider messages that arrive as ordinary text although no work happened.
fn is_provider_failure(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim().trim_end_matches(['.', '!']).to_lowercase();
        let line = line
            .strip_prefix("you've ")
            .or_else(|| line.strip_prefix("you have "))
            .unwrap_or(&line);
        line == "upgrade your plan to continue"
            || [
                "reached your usage limit",
                "reached your request limit",
                "reached your spend limit",
            ]
            .iter()
            .chain(&[
                "reached usage limit",
                "reached request limit",
                "reached spend limit",
            ])
            .any(|phrase| line.starts_with(phrase))
            || ["usage", "rate", "request"].iter().any(|kind| {
                line.starts_with(&format!("{kind} limit reached"))
                    || line.starts_with(&format!("{kind} limit exceeded"))
            })
            || line.starts_with("authentication required")
            || line.starts_with("please sign in to continue")
            || line.starts_with("please log in to continue")
    })
}

/// A reply reads as a plan when it has a Markdown heading or two list steps.
pub fn is_reviewable(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || is_provider_failure(text) {
        return false;
    }
    let heading = text.lines().any(|line| {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = line.trim_start_matches(' ');
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        indent <= 3
            && (1..=6).contains(&hashes)
            && rest[hashes..].starts_with([' ', '\t'])
            && !rest[hashes..].trim().is_empty()
    });
    let steps = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            let bullet = line
                .strip_prefix(['-', '*', '+'])
                .is_some_and(|rest| rest.starts_with(' ') && !rest.trim().is_empty());
            let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            let numbered = digits > 0
                && line[digits..]
                    .strip_prefix(['.', ')'])
                    .is_some_and(|rest| rest.starts_with(' ') && !rest.trim().is_empty());
            bullet || numbered
        })
        .count();
    heading || steps >= 2
}

/// Codex wraps a finished plan in `<proposed_plan>` lines: the text before,
/// the plan, and the text after.
fn proposed_plan(text: &str) -> Option<(&str, &str, &str)> {
    let mut offset = 0;
    let mut open = None;
    for line in text.split_inclusive('\n') {
        let tag = line.trim();
        match (open, tag) {
            (None, "<proposed_plan>") => open = Some((offset, offset + line.len())),
            (Some((start, body)), "</proposed_plan>") => {
                let plan = text[body..offset].trim();
                if plan.is_empty() {
                    return None;
                }
                return Some((
                    text[..start].trim(),
                    plan,
                    text[offset + line.len()..].trim(),
                ));
            }
            _ => {}
        }
        offset += line.len();
    }
    None
}

/// Lifts the plan out of a finished turn's rows from `turn_start`: a
/// `<proposed_plan>` block in any reply, or else, after a plan-mode turn,
/// its last reply when it reads as a plan. Returns the first changed row.
pub fn promote(rows: &mut Vec<Row>, turn_start: usize, planning: bool) -> Option<usize> {
    let turn_start = turn_start.min(rows.len());
    if rows[turn_start..].iter().any(|row| row.role == ROLE) {
        return None;
    }
    for at in (turn_start..rows.len()).rev() {
        if rows[at].role != "assistant" {
            continue;
        }
        let Some((before, plan, after)) = proposed_plan(&rows[at].text) else {
            continue;
        };
        let (before, plan, after) = (before.to_owned(), plan.to_owned(), after.to_owned());
        let template = rows[at].clone();
        let mut lifted = Vec::new();
        if !before.is_empty() {
            lifted.push(Row {
                text: before,
                ..template.clone()
            });
        }
        lifted.push(Row {
            role: ROLE.into(),
            text: plan,
            first: true,
            sent_at: if lifted.is_empty() {
                template.sent_at
            } else {
                None
            },
        });
        if !after.is_empty() {
            lifted.push(Row {
                role: "assistant".into(),
                text: after,
                first: true,
                sent_at: None,
            });
        }
        rows.splice(at..=at, lifted);
        return Some(at);
    }
    if !planning {
        return None;
    }
    let at = (turn_start..rows.len())
        .rev()
        .find(|&at| rows[at].role == "assistant" && !rows[at].text.trim().is_empty())?;
    if !is_reviewable(&rows[at].text) {
        return None;
    }
    // A reply streamed in several rows is one message: keep its rows together.
    let first = (turn_start..=at)
        .rev()
        .find(|&i| rows[i].role != "assistant" || rows[i].first)
        .filter(|&i| rows[i].role == "assistant")
        .unwrap_or(at);
    let text = rows[first..=at]
        .iter()
        .map(|row| row.text.as_str())
        .collect::<String>();
    if !is_reviewable(&text) {
        return None;
    }
    let sent_at = rows[first].sent_at;
    rows.splice(
        first..=at,
        [Row {
            role: ROLE.into(),
            text: text.trim().into(),
            first: true,
            sent_at,
        }],
    );
    Some(first)
}

/// Whether the plan at `at` was built: a later message builds it, and the
/// turn that does is still running when it is the last one and `running`.
pub fn build_state(rows: &[Row], at: usize, running: bool) -> Option<BuildState> {
    let build = rows
        .iter()
        .enumerate()
        .skip(at + 1)
        .find(|(_, row)| row.is_prompt())
        .filter(|(_, row)| is_build(&row.text))?
        .0;
    let last = !rows[build + 1..].iter().any(Row::is_prompt);
    Some(if running && last {
        BuildState::Building
    } else {
        BuildState::Built
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildState {
    Building,
    Built,
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
    fn roles(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.role.as_str()).collect()
    }

    #[test]
    fn a_plan_has_a_heading_or_two_steps_and_is_never_a_provider_failure() {
        assert!(is_reviewable("# Add login\n\nWire the form."));
        assert!(is_reviewable("   ### Plan\n"));
        assert!(is_reviewable("Steps:\n- inspect\n- change"));
        assert!(is_reviewable("1. inspect\n2) change"));
        assert!(!is_reviewable("Notes\n    # indented code, not a heading"));
        assert!(!is_reviewable("#hashtag without a space"));
        assert!(!is_reviewable("- only one step"));
        assert!(!is_reviewable("I looked around; nothing to do."));
        assert!(!is_reviewable("   "));
        assert!(!is_reviewable("# Plan\n\nUpgrade your plan to continue."));
        assert!(!is_reviewable("- a\n- b\nYou've reached your usage limit"));
        assert!(!is_reviewable("# Plan\nRate limit exceeded, retry later"));
        assert!(!is_reviewable("# Plan\nPlease sign in to continue"));
    }

    #[test]
    fn a_plan_mode_turn_turns_its_last_reply_into_a_plan() {
        let mut rows = vec![
            row("user", "earlier"),
            row("assistant", "# Old answer\n- a\n- b"),
            row("user", "Plan the login"),
            row("assistant", "Let me look first."),
            row("tool", "{}"),
            row("assistant", "# Login plan\n\n"),
            Row {
                first: false,
                ..row("assistant", "1. Add the form\n2. Wire auth\n")
            },
        ];
        assert_eq!(promote(&mut rows, 2, true), Some(5));
        assert_eq!(
            roles(&rows),
            ["user", "assistant", "user", "assistant", "tool", "plan"]
        );
        assert_eq!(
            rows[5].text,
            "# Login plan\n\n1. Add the form\n2. Wire auth"
        );
        assert_eq!(rows[1].role, "assistant", "earlier turns stay as they were");
        // Promoting again changes nothing.
        assert_eq!(promote(&mut rows, 2, true), None);
    }

    #[test]
    fn only_a_plan_mode_turn_with_a_plan_shaped_reply_is_promoted() {
        let mut rows = vec![row("user", "Plan it"), row("assistant", "# Plan\n- a\n- b")];
        assert_eq!(promote(&mut rows, 0, false), None, "not a plan-mode turn");
        let mut chat = vec![
            row("user", "Plan it"),
            row("assistant", "Sure, nothing to change."),
        ];
        assert_eq!(promote(&mut chat, 0, true), None);
        assert_eq!(roles(&chat), ["user", "assistant"]);
        let mut empty = vec![row("user", "Plan it"), row("tool", "{}")];
        assert_eq!(promote(&mut empty, 0, true), None);
        let mut past_end = vec![row("user", "Plan it")];
        assert_eq!(promote(&mut past_end, 5, true), None);
    }

    #[test]
    fn a_proposed_plan_block_is_lifted_out_of_any_turn_keeping_the_text_around_it() {
        let mut rows = vec![
            row("user", "revise the plan"),
            Row {
                sent_at: Some(7),
                ..row(
                    "assistant",
                    "Here is the revision.\n<proposed_plan>\n# Plan\n- one\n</proposed_plan>\nTell me if it fits.",
                )
            },
        ];
        assert_eq!(promote(&mut rows, 0, false), Some(1));
        assert_eq!(roles(&rows), ["user", "assistant", "plan", "assistant"]);
        assert_eq!(rows[1].text, "Here is the revision.");
        assert_eq!(rows[1].sent_at, Some(7));
        assert_eq!(rows[2].text, "# Plan\n- one");
        assert_eq!(rows[3].text, "Tell me if it fits.");

        let mut bare = vec![
            row("user", "plan"),
            row(
                "assistant",
                "  <proposed_plan>\r\nShip it\r\n  </proposed_plan>  ",
            ),
        ];
        assert_eq!(promote(&mut bare, 0, true), Some(1));
        assert_eq!(roles(&bare), ["user", "plan"]);
        assert_eq!(bare[1].text, "Ship it");

        let mut unclosed = vec![row("user", "x"), row("assistant", "<proposed_plan>\nhalf")];
        assert_eq!(promote(&mut unclosed, 0, false), None);
        let mut blank = vec![
            row("user", "x"),
            row("assistant", "<proposed_plan>\n\n</proposed_plan>"),
        ];
        assert_eq!(promote(&mut blank, 0, false), None);
        let mut inline = vec![
            row("user", "x"),
            row("assistant", "use <proposed_plan> tags"),
        ];
        assert_eq!(promote(&mut inline, 0, false), None);
    }

    #[test]
    fn a_build_message_carries_the_exact_plan_and_marks_it_built() {
        let prompt = build_prompt("\n# Plan\n- edited step\n");
        assert_eq!(approved_plan(&prompt), Some("# Plan\n- edited step"));
        assert!(is_build(&prompt));
        assert!(is_build(" Build approved plan "));
        assert!(!is_build("Build the thing"));
        assert_eq!(approved_plan("<approved_plan>x</approved_plan>"), None);

        let mut rows = vec![row("user", "plan it"), row(ROLE, "# Plan\n- a")];
        assert_eq!(build_state(&rows, 1, false), None);
        rows.push(row("user", "change step two"));
        rows.push(row("user", &prompt));
        assert_eq!(
            build_state(&rows, 1, true),
            None,
            "the next message was not a build"
        );
        let mut built = vec![
            row("user", "plan it"),
            row(ROLE, "# Plan"),
            row("user", &prompt),
        ];
        assert_eq!(build_state(&built, 1, true), Some(BuildState::Building));
        assert_eq!(build_state(&built, 1, false), Some(BuildState::Built));
        built.push(row("assistant", "done"));
        built.push(row("user", "thanks"));
        assert_eq!(build_state(&built, 1, true), Some(BuildState::Built));
    }
}
