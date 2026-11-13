//! How a task and its answer are kept in the database: the keys, the whole
//! request as JSON, and the answer as the user reads it and as it was sent.
use super::*;
use gespan_core::agent::Question;
use gespan_core::tasks::TaskRecord;
use serde_json::{Value, json};

pub(crate) fn approval_key(session: i64, request: &str) -> String {
    format!("approval:{session}:{request}")
}

pub(crate) fn question_key(session: i64, request: &str) -> String {
    format!("question:{session}:{request}")
}

pub(crate) fn approval_request(approval: &Approval) -> Value {
    json!({
        "id": approval.id,
        "title": approval.title,
        "detail": approval.detail,
        "can_remember": approval.can_remember,
        "tool": approval.tool.as_ref().map(|tool| json!({
            "name": tool.name,
            "input": serde_json::from_str::<Value>(&tool.input)
                .unwrap_or_else(|_| Value::String(tool.input.clone())),
        })),
    })
}

pub(crate) fn question_request(question: &Question) -> Value {
    json!({
        "id": question.id,
        "title": question.title,
        "items": question.items.iter().map(|item| json!({
            "id": item.id,
            "question": item.prompt,
            "multiple": item.multiple,
            "allow_custom": item.allow_custom,
            "options": item.options.iter().map(|option| json!({
                "label": option.label,
                "description": option.description,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The approval's answer as the user reads it and as it was sent.
pub(crate) fn approval_answer(decision: &Decision) -> Option<(&'static str, Value)> {
    Some(match decision {
        Decision::Allow => ("Allowed once", json!("allow")),
        Decision::Session => ("Allowed for this session", json!("session")),
        Decision::Deny => ("Denied", json!("deny")),
        _ => return None,
    })
}

/// The options picked for each question, as the user reads them: one
/// question's answer alone, several as "Question: answer" lines.
pub(crate) fn question_answer(question: &Question, answers: Option<&[Vec<String>]>) -> String {
    let Some(answers) = answers else {
        return "Dismissed".into();
    };
    let picked = |ix: usize| answers.get(ix).map(|a| a.join(", ")).unwrap_or_default();
    match question.items.len() {
        0 | 1 => picked(0),
        _ => question
            .items
            .iter()
            .enumerate()
            .map(|(ix, item)| format!("{}: {}", item.prompt, picked(ix)))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

impl Source {
    pub(crate) fn origin(&self) -> String {
        match self {
            Self::Session(id) => format!("session:{id}"),
            Self::FlowRun(run) => format!("flow:{run}"),
            Self::Pilot { pilot, session } => format!("pilot:{pilot}:{session}"),
            Self::DraftReview { run, url } => format!("draft-review:{run}:{url}"),
            Self::Log => "decision-log:".into(),
        }
    }

    pub(crate) fn from_origin(origin: &str) -> Option<Self> {
        let (kind, rest) = origin.split_once(':')?;
        Some(match kind {
            "session" => Self::Session(rest.parse().ok()?),
            "flow" => Self::FlowRun(rest.to_owned()),
            // A run id has no colon; the URL after it may.
            "draft-review" => {
                let (run, url) = rest.split_once(':')?;
                Self::DraftReview {
                    run: run.to_owned(),
                    url: url.to_owned(),
                }
            }
            "pilot" => {
                let (pilot, session) = rest.rsplit_once(':')?;
                Self::Pilot {
                    pilot: pilot.to_owned(),
                    session: session.parse().ok()?,
                }
            }
            _ => return None,
        })
    }
}

impl TaskItem {
    /// This task as the database keeps it while it waits.
    pub(crate) fn record(&self, cx: &gpui::App) -> TaskRecord {
        let request = match &self.body {
            Body::Approval { approval, .. } => approval_request(approval),
            Body::Question(card) => question_request(card.read(cx).request()),
            Body::Flow { prompt } => json!({ "prompt": prompt }),
            Body::Choice { options } => json!({ "options": options }),
            Body::DraftReview {
                body,
                comments,
                outside,
            } => json!({ "body": body, "comments": comments, "outside": outside }),
            Body::Decision { entry, flag, .. } => json!({
                "id": entry.label(),
                "title": entry.title,
                "text": entry.text,
                "why": entry.why,
                "user_answer": entry.user_answer,
                "flag": flag.as_ref().map(|flag| flag.reason.clone()),
            }),
        };
        TaskRecord::open(
            self.key.clone(),
            self.kind,
            self.project.clone(),
            self.source.origin(),
            self.source_label.clone(),
            self.headline.clone(),
            request,
            self.since_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::agent::{QuestionItem, QuestionOption};

    fn item(prompt: &str, labels: &[&str]) -> QuestionItem {
        QuestionItem {
            id: prompt.into(),
            prompt: prompt.into(),
            multiple: false,
            allow_custom: true,
            options: labels
                .iter()
                .map(|label| QuestionOption {
                    id: (*label).into(),
                    label: (*label).into(),
                    description: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_draft_review_origin_reads_back_with_its_url() {
        let source = Source::DraftReview {
            run: "run-1".into(),
            url: "https://github.com/acme/app/pull/7#pullrequestreview-42".into(),
        };
        assert_eq!(Source::from_origin(&source.origin()), Some(source));
    }

    #[test]
    fn a_question_keeps_every_option_and_reads_back_the_picked_ones() {
        let question = Question {
            id: "q1".into(),
            title: "Claude asks".into(),
            items: vec![
                item("Which counter?", &["Recount", "Carry K's"]),
                item("Which tests?", &["Unit", "End to end"]),
            ],
            auto_resolve_at: None,
        };
        let request = question_request(&question);
        assert_eq!(request["items"][0]["options"][1]["label"], "Carry K's");
        assert_eq!(request["items"][1]["question"], "Which tests?");
        let answers = vec![
            vec!["Carry K's".to_owned()],
            vec!["Unit".into(), "End to end".into()],
        ];
        assert_eq!(
            question_answer(&question, Some(&answers)),
            "Which counter?: Carry K's\nWhich tests?: Unit, End to end"
        );
        let one = Question {
            items: vec![item("Which counter?", &["Recount"])],
            ..question
        };
        assert_eq!(
            question_answer(&one, Some(&[vec!["My own words".into()]])),
            "My own words"
        );
        assert_eq!(question_answer(&one, None), "Dismissed");
    }

    #[test]
    fn an_approval_keeps_its_tool_input_as_json() {
        let mut call = gespan_core::tool::ToolCall::new("t1", "Bash");
        call.input = r#"{"command":"rm -rf build"}"#.into();
        let approval = Approval {
            id: "a1".into(),
            title: "Allow Bash?".into(),
            detail: String::new(),
            can_remember: true,
            tool: Some(call),
        };
        let request = approval_request(&approval);
        assert_eq!(request["tool"]["input"]["command"], "rm -rf build");
        assert_eq!(
            approval_answer(&Decision::Session).unwrap().0,
            "Allowed for this session"
        );
        assert!(approval_answer(&Decision::KeepOpen).is_none());
    }

    #[test]
    fn every_source_opens_again_from_its_origin() {
        for source in [
            Source::Session(7),
            Source::FlowRun("run-1".into()),
            Source::Pilot {
                pilot: "pilot:a".into(),
                session: 9,
            },
        ] {
            assert_eq!(Source::from_origin(&source.origin()), Some(source));
        }
        assert_eq!(Source::from_origin("nonsense"), None);
    }
}
