//! Built-in starter Flows offered by "New Flow". Ids, cwd and timestamps are
//! left empty for the caller to fill when the Flow is saved.
use super::definition::{
    AgentState, ApprovalState, Flow, FlowInput, FlowState, Guard, GuardOp, OutputField, OutputType,
    Transition, DEFAULT_MAX_VISITS, SUCCEEDED,
};

// Claude uses the app's default model; an empty model means "harness default"
// (Codex models are discovered at runtime).
const CLAUDE_MODEL: &str = "claude:sonnet-5";
const CODEX_MODEL: &str = "";

fn input(name: &str, label: &str, required: bool) -> FlowInput {
    FlowInput {
        name: name.into(),
        label: label.into(),
        required,
        default: None,
    }
}

fn output(name: &str, ty: OutputType, description: &str) -> OutputField {
    OutputField {
        name: name.into(),
        r#type: ty,
        description: description.into(),
    }
}

fn to(target: &str) -> Transition {
    Transition {
        to: target.into(),
        when: None,
    }
}

fn truthy(target: &str, field: &str) -> Transition {
    Transition {
        to: target.into(),
        when: Some(Guard::Output {
            field: field.into(),
            op: GuardOp::Truthy,
            value: None,
        }),
    }
}

fn agent(
    id: &str,
    title: &str,
    harness: &str,
    model: &str,
    brief: &str,
    outputs: Vec<OutputField>,
    transitions: Vec<Transition>,
) -> FlowState {
    FlowState::Agent(AgentState {
        id: id.into(),
        title: title.into(),
        harness: harness.into(),
        model: model.into(),
        brief: brief.into(),
        outputs,
        transitions,
        max_visits: DEFAULT_MAX_VISITS,
    })
}

fn approval(id: &str, title: &str, message: &str, transitions: Vec<Transition>) -> FlowState {
    FlowState::Approval(ApprovalState {
        id: id.into(),
        title: title.into(),
        message: message.into(),
        transitions,
        max_visits: DEFAULT_MAX_VISITS,
    })
}

fn flow(
    name: &str,
    description: &str,
    inputs: Vec<FlowInput>,
    initial: &str,
    states: Vec<FlowState>,
) -> Flow {
    Flow {
        id: String::new(),
        name: name.into(),
        description: description.into(),
        cwd: String::new(),
        inputs,
        initial: initial.into(),
        states,
        created_at: 0,
        updated_at: 0,
    }
}

fn fix_an_issue() -> Flow {
    flow(
        "Fix an issue",
        "Triage an issue, implement a fix, run the tests, and get approval, looping back on failure.",
        vec![input("issue", "Issue", true), input("notes", "Notes", false)],
        "triage",
        vec![
            agent(
                "triage",
                "Triage",
                "claude",
                CLAUDE_MODEL,
                "Investigate this issue and write a short plan for fixing it. Do not change any files.\n\nIssue:\n{{inputs.issue}}\n\nNotes:\n{{inputs.notes?}}",
                vec![
                    output("plan", OutputType::String, "Step-by-step plan for the fix"),
                    output("files", OutputType::StringList, "Paths of files likely to change"),
                ],
                vec![to("implement")],
            ),
            agent(
                "implement",
                "Implement",
                "codex",
                CODEX_MODEL,
                "Implement the fix for this issue.\n\nIssue:\n{{inputs.issue}}\n\nPlan:\n{{states.triage.outputs.plan}}\n\nFiles likely to change:\n{{states.triage.outputs.files}}\n\nTest failures from the previous attempt, if any:\n{{states.test.outputs.failures?}}",
                vec![output("summary", OutputType::String, "What you changed and why")],
                vec![to("test")],
            ),
            agent(
                "test",
                "Test",
                "claude",
                CLAUDE_MODEL,
                "Run the project's tests and checks against the current changes. Do not edit files.\n\nChange summary:\n{{states.implement.outputs.summary}}",
                vec![
                    output("passed", OutputType::Boolean, "True if all tests and checks passed"),
                    output("failures", OutputType::String, "Failing tests and error output, empty if none"),
                ],
                vec![truthy("review", "passed"), to("implement")],
            ),
            approval(
                "review",
                "Review",
                "Review the changes.\n\nSummary:\n{{states.implement.outputs.summary}}\n\nApprove to finish, or reject to send it back to implement.",
                vec![truthy(SUCCEEDED, "approved"), to("implement")],
            ),
        ],
    )
}

/// A non-code starter: no step edits files or runs project tests.
fn research_and_summarise() -> Flow {
    flow(
        "Research and summarise",
        "Research a topic, write a summary, and get approval, looping back with feedback if it is rejected.",
        vec![input("topic", "Topic", true), input("audience", "Audience", false)],
        "research",
        vec![
            agent(
                "research",
                "Research",
                "claude",
                CLAUDE_MODEL,
                "Research this topic and collect the key findings. Do not change any files.\n\nTopic:\n{{inputs.topic}}\n\nAudience:\n{{inputs.audience?}}\n\nReviewer feedback on the previous summary, if any:\n{{states.review.outputs.note?}}",
                vec![
                    output("findings", OutputType::StringList, "Key findings, one per item"),
                    output("sources", OutputType::StringList, "Sources consulted, as links or titles"),
                ],
                vec![to("summarise")],
            ),
            agent(
                "summarise",
                "Summarise",
                "claude",
                CLAUDE_MODEL,
                "Write a clear, concise summary of these findings. Do not change any files.\n\nTopic:\n{{inputs.topic}}\n\nAudience:\n{{inputs.audience?}}\n\nFindings:\n{{states.research.outputs.findings}}\n\nSources:\n{{states.research.outputs.sources}}",
                vec![output("summary", OutputType::String, "The finished summary")],
                vec![to("review")],
            ),
            approval(
                "review",
                "Review",
                "Review the summary.\n\n{{states.summarise.outputs.summary}}\n\nApprove to finish, or reject with a note to send it back to research.",
                vec![truthy(SUCCEEDED, "approved"), to("research")],
            ),
        ],
    )
}

pub fn starter_flows() -> Vec<Flow> {
    vec![fix_an_issue(), research_and_summarise()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::definition::{validate_flow, Validation};
    use crate::provider::Kind;

    fn by_name(name: &str) -> Flow {
        starter_flows()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap()
    }

    fn ids(f: &Flow) -> Vec<&str> {
        f.states.iter().map(FlowState::id).collect()
    }

    #[test]
    fn starters_are_unsaved_templates() {
        let starters = starter_flows();
        let names: Vec<&str> = starters.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Fix an issue", "Research and summarise"]);
        for f in &starters {
            assert!(f.id.is_empty() && f.cwd.is_empty());
            assert_eq!((f.created_at, f.updated_at), (0, 0));
        }
    }

    #[test]
    fn fix_an_issue_has_the_expected_loop() {
        let f = by_name("Fix an issue");
        assert_eq!(ids(&f), ["triage", "implement", "test", "review"]);
        let inputs: Vec<_> = f
            .inputs
            .iter()
            .map(|i| (i.name.as_str(), i.required))
            .collect();
        assert_eq!(inputs, [("issue", true), ("notes", false)]);
    }

    #[test]
    fn starters_validate_clean_and_use_native_harnesses() {
        for f in starter_flows() {
            assert_eq!(
                validate_flow(&Flow {
                    id: "x".into(),
                    cwd: "/r".into(),
                    ..f.clone()
                }),
                Validation::default()
            );
            for state in &f.states {
                if let FlowState::Agent(a) = state {
                    assert!(Kind::parse(&a.harness).is_some(), "{}", a.harness);
                }
            }
        }
    }

    #[test]
    fn research_starter_validates_without_code_steps() {
        let f = Flow {
            id: "x".into(),
            cwd: "/r".into(),
            ..by_name("Research and summarise")
        };
        assert_eq!(validate_flow(&f), Validation::default());
        assert_eq!(ids(&f), ["research", "summarise", "review"]);
        for state in &f.states {
            if let FlowState::Agent(a) = state {
                assert!(a.brief.contains("Do not change any files"), "{}", a.id);
            }
        }
        assert!(!ids(&f).iter().any(|id| ["implement", "test"].contains(id)));
    }
}
