use super::*;
use crate::flow::contract::retry_prompt;
use crate::flow::definition::{
    AgentState, FlowInput, FlowState, Guard, GuardOp, OutputField, OutputType, OutputValue,
    Transition, VisitResult, FAILED, SUCCEEDED,
};
use crate::flow::run::VisitStatus;
use crate::flow::starter_flows;
use serde_json::json;
use std::cell::Cell;

// ---- fake host ---------------------------------------------------------

/// Drives a `Runner` the way the GPUI host will: it records every effect and
/// reports a session id (`sess-<visit id>`) for every `Launch`.
struct Host {
    runner: Runner,
    log: Vec<Effect>,
}

impl Host {
    fn new() -> Self {
        let tick = Cell::new(1000_i64);
        let mut n = 0;
        let runner = Runner::new(
            move || {
                tick.set(tick.get() + 1);
                tick.get()
            },
            move || {
                n += 1;
                format!("id{n}")
            },
        );
        Self {
            runner,
            log: Vec::new(),
        }
    }

    /// Records `effects`, then reports a session for every launch in them.
    fn apply(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        self.log.extend(effects.iter().cloned());
        for e in &effects {
            if let Effect::Launch(l) = e {
                let session = format!("sess-{}", l.visit_id);
                let more = self.runner.on_session(&l.run_id, &l.visit_id, &session);
                self.log.extend(more);
            }
        }
        effects
    }

    fn start(&mut self, flow: &Flow, inputs: &[(&str, &str)]) -> (FlowRun, Vec<Effect>) {
        let given = inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let (run, effects) = self.runner.start(flow, &given, None).expect("start");
        let effects = self.apply(effects);
        (run, effects)
    }

    fn launches(&self) -> Vec<LaunchRequest> {
        self.log
            .iter()
            .filter_map(|e| match e {
                Effect::Launch(l) => Some(l.clone()),
                _ => None,
            })
            .collect()
    }

    fn last_launch(&self) -> LaunchRequest {
        self.launches().pop().expect("a launch")
    }

    fn settle(&mut self, run_id: &str, outcome: AgentOutcome) -> Vec<Effect> {
        let visit_id = self.last_launch().visit_id;
        let effects = self.runner.on_agent_settled(run_id, &visit_id, outcome);
        self.apply(effects)
    }

    fn reply(&mut self, run_id: &str, text: &str) -> Vec<Effect> {
        self.settle(
            run_id,
            AgentOutcome {
                status: AgentStatus::Completed,
                text: text.into(),
                error: None,
            },
        )
    }

    fn outputs(&mut self, run_id: &str, value: serde_json::Value) -> Vec<Effect> {
        self.reply(run_id, &block(value))
    }

    fn fail(&mut self, run_id: &str, error: Option<&str>) -> Vec<Effect> {
        self.settle(
            run_id,
            AgentOutcome {
                status: AgentStatus::Failed,
                text: String::new(),
                error: error.map(str::to_owned),
            },
        )
    }

    fn approve(&mut self, run_id: &str, approved: bool, note: &str) -> Vec<Effect> {
        let effects = self.runner.resolve_approval(run_id, approved, note);
        self.apply(effects)
    }

    fn last_run(&self) -> FlowRun {
        self.log
            .iter()
            .rev()
            .find_map(|e| match e {
                Effect::Persist { run, .. } => Some(run.clone()),
                _ => None,
            })
            .expect("a persisted run")
    }

    /// The latest persisted copy of every visit, in seq order.
    fn visits(&self) -> Vec<FlowVisit> {
        let mut latest: Vec<FlowVisit> = Vec::new();
        for e in &self.log {
            if let Effect::Persist { visit: Some(v), .. } = e {
                match latest.iter_mut().find(|x| x.id == v.id) {
                    Some(slot) => *slot = v.clone(),
                    None => latest.push(v.clone()),
                }
            }
        }
        latest.sort_by_key(|v| v.seq);
        latest
    }

    fn follow_ups(&self) -> Vec<(String, String)> {
        self.log
            .iter()
            .filter_map(|e| match e {
                Effect::FollowUp { session_id, prompt } => {
                    Some((session_id.clone(), prompt.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn finished(&self) -> Vec<RunStatus> {
        self.log
            .iter()
            .filter_map(|e| match e {
                Effect::Finished { status, .. } => Some(*status),
                _ => None,
            })
            .collect()
    }
}

// ---- flow builders -----------------------------------------------------

fn block(value: serde_json::Value) -> String {
    format!("Done.\n```json\n{value}\n```")
}

fn fix_an_issue() -> Flow {
    let mut flow = starter_flows()
        .into_iter()
        .find(|f| f.name == "Fix an issue")
        .unwrap();
    flow.id = "flow1".into();
    flow.cwd = "/repo".into();
    flow
}

fn research() -> Flow {
    let mut flow = starter_flows()
        .into_iter()
        .find(|f| f.name == "Research and summarise")
        .unwrap();
    flow.id = "flow2".into();
    flow.cwd = "/notes".into();
    flow
}

fn field(name: &str, ty: OutputType) -> OutputField {
    OutputField {
        name: name.into(),
        r#type: ty,
        description: format!("the {name}"),
    }
}

fn to(target: &str) -> Transition {
    Transition {
        to: target.into(),
        when: None,
    }
}

fn when_truthy(target: &str, f: &str) -> Transition {
    Transition {
        to: target.into(),
        when: Some(Guard::Output {
            field: f.into(),
            op: GuardOp::Truthy,
            value: None,
        }),
    }
}

fn when_failed(target: &str) -> Transition {
    Transition {
        to: target.into(),
        when: Some(Guard::Status {
            is: VisitResult::Failed,
        }),
    }
}

fn agent(
    id: &str,
    brief: &str,
    outputs: Vec<OutputField>,
    transitions: Vec<Transition>,
    max_visits: u8,
) -> FlowState {
    FlowState::Agent(AgentState {
        id: id.into(),
        title: id.to_uppercase(),
        harness: "claude".into(),
        model: "claude:sonnet-5".into(),
        brief: brief.into(),
        outputs,
        transitions,
        max_visits,
    })
}

fn custom(initial: &str, states: Vec<FlowState>) -> Flow {
    Flow {
        id: "custom".into(),
        name: "Custom".into(),
        description: String::new(),
        cwd: "/work".into(),
        inputs: vec![],
        initial: initial.into(),
        states,
        access: "auto".into(),
        owner: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn kinds(effects: &[Effect]) -> Vec<&'static str> {
    effects
        .iter()
        .map(|e| match e {
            Effect::Launch(_) => "launch",
            Effect::FollowUp { .. } => "follow-up",
            Effect::Cancel { .. } => "cancel",
            Effect::Persist { .. } => "persist",
            Effect::NeedsApproval { .. } => "needs-approval",
            Effect::Finished { .. } => "finished",
            Effect::VisitFinished { .. } => "visit-finished",
        })
        .collect()
}

fn triage_out() -> serde_json::Value {
    json!({"plan": "Patch parser", "files": ["src/parse.rs"]})
}

// ---- happy path & loop -------------------------------------------------

#[test]
fn happy_path_runs_four_visits_in_order_and_succeeds() {
    let mut host = Host::new();
    let flow = fix_an_issue();
    let (run, effects) = host.start(&flow, &[("issue", "Crash on empty input")]);
    assert_eq!(kinds(&effects), ["persist", "persist", "launch"]);
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.flow, flow);
    assert_eq!(run.inputs["notes"], "");

    let first = host.last_launch();
    assert_eq!(first.title, "Fix an issue · Triage");
    assert_eq!(first.harness, "claude");
    assert_eq!(first.model, "claude:sonnet-5");
    assert_eq!(first.cwd, "/repo");
    assert!(first.prompt.contains("Crash on empty input"));
    assert!(first.prompt.contains("- plan (string): Step-by-step plan"));

    host.outputs(&run.id, triage_out());
    let implement = host.last_launch();
    assert_eq!(implement.harness, "codex");
    assert!(implement.prompt.contains("Patch parser"));
    assert!(implement.prompt.contains("- src/parse.rs"));

    host.outputs(&run.id, json!({"summary": "Guarded empty input"}));
    host.outputs(&run.id, json!({"passed": true, "failures": ""}));
    let effects = host.approve(&run.id, true, "");
    assert_eq!(*kinds(&effects).last().unwrap(), "finished");

    let visits = host.visits();
    let seqs: Vec<u32> = visits.iter().map(|v| v.seq).collect();
    let states: Vec<&str> = visits.iter().map(|v| v.state_id.as_str()).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    assert_eq!(states, ["triage", "implement", "test", "review"]);
    assert!(visits.iter().all(|v| v.status == VisitStatus::Completed));
    assert_eq!(
        visits[3].outputs.as_ref().unwrap()["approved"],
        OutputValue::Bool(true)
    );

    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Succeeded);
    assert_eq!(last.error, None);
    assert!(last.finished_at.is_some());
    assert_eq!(host.finished(), [RunStatus::Succeeded]);
}

#[test]
fn loop_feeds_test_failures_into_the_second_implement_visit() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    host.outputs(&run.id, triage_out());
    let first_implement = host.last_launch();
    // The optional `{{states.test.outputs.failures?}}` renders empty at first.
    assert!(first_implement
        .prompt
        .contains("Test failures from the previous attempt, if any:\n\n\n---"));

    host.outputs(&run.id, json!({"summary": "Try one"}));
    host.outputs(
        &run.id,
        json!({"passed": false, "failures": "test_empty_input panicked"}),
    );
    let second_implement = host.last_launch();
    assert!(second_implement
        .prompt
        .contains("test_empty_input panicked"));
    assert_ne!(second_implement.visit_id, first_implement.visit_id);

    host.outputs(&run.id, json!({"summary": "Try two"}));
    let retest = host.last_launch();
    assert!(retest.prompt.contains("Try two"), "latest visit wins");
    assert!(!retest.prompt.contains("Try one"));

    host.outputs(&run.id, json!({"passed": true, "failures": ""}));
    host.approve(&run.id, true, "");
    let states: Vec<String> = host.visits().into_iter().map(|v| v.state_id).collect();
    assert_eq!(
        states,
        ["triage", "implement", "test", "implement", "test", "review"]
    );
    assert_eq!(host.last_run().status, RunStatus::Succeeded);
}

#[test]
fn shared_worktree_only_the_first_launch_has_none() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    assert_eq!(host.last_launch().worktree_cwd, None);

    let effects = host.runner.on_worktree(&run.id, "/wt/flow-1");
    assert_eq!(kinds(&effects), ["persist"]);
    host.apply(effects);
    assert_eq!(host.last_run().worktree_cwd.as_deref(), Some("/wt/flow-1"));
    // A second report never moves the run to another worktree.
    assert!(host.runner.on_worktree(&run.id, "/wt/other").is_empty());

    host.outputs(&run.id, triage_out());
    host.outputs(&run.id, json!({"summary": "s"}));
    host.outputs(&run.id, json!({"passed": false, "failures": "f"}));
    let launches = host.launches();
    assert_eq!(launches.len(), 4);
    assert_eq!(launches[0].worktree_cwd, None);
    for l in &launches[1..] {
        assert_eq!(l.worktree_cwd.as_deref(), Some("/wt/flow-1"));
        assert_eq!(l.cwd, "/repo");
    }
}

#[test]
fn every_agent_visit_is_a_new_launch_and_follow_up_is_only_for_retry() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    host.outputs(&run.id, triage_out());
    host.outputs(&run.id, json!({"summary": "s"}));
    host.outputs(&run.id, json!({"passed": false, "failures": "f"}));
    host.outputs(&run.id, json!({"summary": "s2"}));
    let launches = host.launches();
    assert_eq!(launches.len(), 5);
    let mut ids: Vec<&str> = launches.iter().map(|l| l.visit_id.as_str()).collect();
    ids.dedup();
    assert_eq!(ids.len(), 5, "each visit launches its own session");
    assert!(host.follow_ups().is_empty());

    host.reply(&run.id, "Tests ran but I forgot the block.");
    assert_eq!(host.follow_ups().len(), 1);
    assert_eq!(host.launches().len(), 5, "the retry does not launch");
}

// ---- output contract ---------------------------------------------------

#[test]
fn contract_retry_in_the_same_session_then_succeeds_with_retried_flag() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let triage = host.last_launch();
    let effects = host.reply(&run.id, "Here is my plan, no JSON.");
    assert_eq!(kinds(&effects), ["persist", "follow-up"]);
    assert_eq!(
        host.follow_ups(),
        [(
            format!("sess-{}", triage.visit_id),
            retry_prompt(&["No ```json block found".to_string()])
        )]
    );
    let visit = host.visits().pop().unwrap();
    assert!(visit.retried);
    assert_eq!(visit.status, VisitStatus::Running);

    host.outputs(&run.id, triage_out());
    let visits = host.visits();
    assert_eq!(visits[0].status, VisitStatus::Completed);
    assert!(visits[0].retried);
    assert_eq!(visits[1].state_id, "implement");
    assert_eq!(host.follow_ups().len(), 1);
}

#[test]
fn contract_failing_twice_fails_the_run_with_output_contract_not_met() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    host.reply(&run.id, "no json");
    let effects = host.outputs(&run.id, json!({"plan": 3, "files": []}));
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    let error = "Output contract not met: field plan must be string";
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some(error));
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some(error));
    assert_eq!(host.follow_ups().len(), 1, "exactly one retry");
    assert_eq!(host.launches().len(), 1);
}

#[test]
fn contract_failure_can_follow_a_status_failed_transition() {
    let flow = custom(
        "a",
        vec![
            agent(
                "a",
                "Do it",
                vec![field("done", OutputType::Boolean)],
                vec![when_failed("b"), to(SUCCEEDED)],
                3,
            ),
            agent("b", "Recover", vec![], vec![to(SUCCEEDED)], 3),
        ],
    );
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    host.reply(&run.id, "nope");
    host.reply(&run.id, "still nope");
    assert_eq!(host.last_launch().title, "Custom · B");
    assert_eq!(host.last_run().status, RunStatus::Running);
}

#[test]
fn zero_output_state_ignores_an_unprompted_json_block() {
    let flow = custom(
        "a",
        vec![agent("a", "Chat", vec![], vec![to(SUCCEEDED)], 3)],
    );
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    assert_eq!(
        host.last_launch().prompt,
        "Chat",
        "no footer for zero outputs"
    );
    host.outputs(&run.id, json!({"anything": 1}));
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Completed);
    assert_eq!(visit.outputs, Some(HashMap::new()));
    assert_eq!(host.last_run().status, RunStatus::Succeeded);
    assert!(host.follow_ups().is_empty());
}

// ---- guards, caps, no match --------------------------------------------

#[test]
fn status_guard_routes_a_failed_agent_to_failed_terminal() {
    let flow = custom(
        "a",
        vec![agent(
            "a",
            "Work",
            vec![],
            vec![when_failed(FAILED), to(SUCCEEDED)],
            3,
        )],
    );
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    let effects = host.fail(&run.id, Some("harness crashed"));
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some("harness crashed"));
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("Reached $failed from a"));
}

#[test]
fn failed_agent_without_status_transition_fails_with_its_error() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    host.fail(&run.id, None);
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("Agent failed"));
}

#[test]
fn max_visits_cap_fails_the_run_without_a_new_visit() {
    let flow = custom(
        "a",
        vec![agent(
            "a",
            "Again?",
            vec![field("again", OutputType::Boolean)],
            vec![when_truthy("a", "again"), to(SUCCEEDED)],
            2,
        )],
    );
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    host.outputs(&run.id, json!({"again": true}));
    let effects = host.outputs(&run.id, json!({"again": true}));
    assert!(kinds(&effects).ends_with(&["finished", "visit-finished"]));
    assert_eq!(host.launches().len(), 2);
    assert_eq!(host.visits().len(), 2);
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("State a exceeded 2 visits"));
    assert_eq!(host.finished(), [RunStatus::Failed]);
}

#[test]
fn run_step_cap_fails_after_fifty_visits() {
    let ids = ["s1", "s2", "s3", "s4", "s5", "s6"];
    let states = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            agent(
                id,
                "Step",
                vec![field("done", OutputType::Boolean)],
                vec![when_truthy(SUCCEEDED, "done"), to(ids[(i + 1) % ids.len()])],
                10,
            )
        })
        .collect();
    let flow = custom("s1", states);
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    for _ in 0..50 {
        assert_eq!(host.last_run().status, RunStatus::Running);
        host.outputs(&run.id, json!({"done": false}));
    }
    assert_eq!(host.launches().len(), 50);
    assert_eq!(host.visits().len(), MAX_RUN_STEPS as usize);
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("Run exceeded 50 steps"));
}

#[test]
fn no_matching_transition_fails_the_run() {
    let flow = custom(
        "a",
        vec![agent(
            "a",
            "Check",
            vec![field("passed", OutputType::Boolean)],
            vec![when_truthy(SUCCEEDED, "passed")],
            3,
        )],
    );
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    host.outputs(&run.id, json!({"passed": false}));
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Completed);
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("No transition matched from a"));
}

#[test]
fn missing_required_binding_fails_the_visit_without_a_launch() {
    let flow = custom(
        "a",
        vec![
            agent("a", "Use {{states.b.outputs.x}}", vec![], vec![to("b")], 3),
            agent(
                "b",
                "Make x",
                vec![field("x", OutputType::String)],
                vec![to(SUCCEEDED)],
                3,
            ),
        ],
    );
    let mut host = Host::new();
    let (run, effects) = host.start(&flow, &[]);
    assert_eq!(kinds(&effects), ["persist", "persist", "finished"]);
    assert!(host.launches().is_empty());
    let error = "Binding states.b.outputs.x has no value yet";
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some(error));
    let last = host.last_run();
    assert_eq!(last.id, run.id);
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some(error));
}

// ---- approval ----------------------------------------------------------

#[test]
fn approval_waits_then_reject_note_reaches_the_next_brief() {
    let mut host = Host::new();
    let (run, _) = host.start(&research(), &[("topic", "Rust GUIs")]);
    let first = host.last_launch();
    assert!(first
        .prompt
        .contains("Reviewer feedback on the previous summary, if any:\n\n\n---"));
    host.outputs(
        &run.id,
        json!({"findings": ["GPUI is fast"], "sources": ["zed.dev"]}),
    );
    let effects = host.outputs(&run.id, json!({"summary": "GPUI is fast."}));
    assert_eq!(
        kinds(&effects),
        ["persist", "persist", "needs-approval", "visit-finished"]
    );
    let review = host.visits().pop().unwrap();
    assert_eq!(review.status, VisitStatus::WaitingApproval);
    assert!(review.prompt.as_deref().unwrap().contains("GPUI is fast."));
    assert_eq!(host.last_run().status, RunStatus::WaitingApproval);
    assert!(matches!(
        effects.get(2),
        Some(Effect::NeedsApproval { run_id, visit_id }) if *run_id == run.id && *visit_id == review.id
    ));

    host.approve(&run.id, false, "Cite more sources");
    let review = host.visits()[2].clone();
    assert_eq!(review.status, VisitStatus::Completed);
    assert_eq!(review.note.as_deref(), Some("Cite more sources"));
    assert_eq!(
        review.outputs.unwrap()["approved"],
        OutputValue::Bool(false)
    );
    let again = host.last_launch();
    assert_ne!(again.visit_id, first.visit_id);
    assert!(again.prompt.contains("Cite more sources"));
    assert_eq!(host.last_run().status, RunStatus::Running);
    // A second decision for the same approval is stale.
    assert!(host.runner.resolve_approval(&run.id, true, "").is_empty());
}

// ---- cancel, stale events ----------------------------------------------

#[test]
fn cancel_stops_the_session_and_marks_visit_and_run_cancelled() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let launch = host.last_launch();
    let effects = host.runner.cancel(&run.id);
    assert_eq!(kinds(&effects), ["persist", "cancel", "finished"]);
    assert_eq!(
        effects[1],
        Effect::Cancel {
            session_id: format!("sess-{}", launch.visit_id)
        }
    );
    host.apply(effects);
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Cancelled);
    assert!(visit.finished_at.is_some());
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Cancelled);
    assert!(last.finished_at.is_some());
    assert_eq!(host.finished(), [RunStatus::Cancelled]);

    // The cancelled session settling afterwards, or a second cancel, is stale.
    assert!(host.reply(&run.id, &block(triage_out())).is_empty());
    assert!(host.runner.cancel(&run.id).is_empty());
}

#[test]
fn cancel_while_waiting_for_approval_has_no_session_to_stop() {
    let mut host = Host::new();
    let (run, _) = host.start(&research(), &[("topic", "t")]);
    host.outputs(&run.id, json!({"findings": [], "sources": []}));
    host.outputs(&run.id, json!({"summary": "s"}));
    let effects = host.runner.cancel(&run.id);
    assert_eq!(kinds(&effects), ["persist", "finished"]);
}

#[test]
fn agent_reporting_cancelled_cancels_the_run() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let effects = host.settle(
        &run.id,
        AgentOutcome {
            status: AgentStatus::Cancelled,
            text: String::new(),
            error: None,
        },
    );
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    assert_eq!(host.visits()[0].status, VisitStatus::Cancelled);
    assert_eq!(host.last_run().status, RunStatus::Cancelled);
}

#[test]
fn stale_and_unknown_events_are_ignored() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let triage = host.last_launch();
    let done = AgentOutcome {
        status: AgentStatus::Completed,
        text: block(triage_out()),
        error: None,
    };
    assert!(host
        .runner
        .on_agent_settled("nope", &triage.visit_id, done.clone())
        .is_empty());
    assert!(host
        .runner
        .on_agent_settled(&run.id, "nope", done.clone())
        .is_empty());
    assert!(host.runner.on_session(&run.id, "nope", "s").is_empty());
    assert!(host.runner.resolve_approval(&run.id, true, "").is_empty());
    assert!(host.runner.cancel("nope").is_empty());
    assert!(host.runner.on_worktree("nope", "/wt").is_empty());

    host.reply(&run.id, &done.text);
    // The triage visit already settled: a duplicate report is ignored.
    assert!(host
        .runner
        .on_agent_settled(&run.id, &triage.visit_id, done)
        .is_empty());
    assert_eq!(host.launches().len(), 2);
}

#[test]
fn first_visit_failing_before_a_worktree_fails_the_run_without_hanging() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    assert_eq!(host.last_launch().worktree_cwd, None);
    let effects = host.fail(&run.id, Some("Not a git repository"));
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    assert_eq!(
        effects.get(1),
        Some(&Effect::Finished {
            run_id: run.id.clone(),
            status: RunStatus::Failed
        })
    );
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("Not a git repository"));
    assert_eq!(last.worktree_cwd, None);
    assert_eq!(host.launches().len(), 1);
}

// ---- recover & resume --------------------------------------------------

#[test]
fn recover_fails_running_visits_and_keeps_waiting_approvals() {
    let mut first = Host::new();
    let (running, _) = first.start(&fix_an_issue(), &[("issue", "Bug")]);
    first.runner.on_worktree(&running.id, "/wt/a");
    let running_snapshot = (first.last_run(), first.visits());

    let mut second = Host::new();
    let (waiting, _) = second.start(&research(), &[("topic", "t")]);
    second.outputs(&waiting.id, json!({"findings": [], "sources": []}));
    second.outputs(&waiting.id, json!({"summary": "s"}));
    let waiting_snapshot = (second.last_run(), second.visits());
    assert_eq!(waiting_snapshot.0.status, RunStatus::WaitingApproval);

    // A fresh app start: a new runner loads both active runs.
    let mut host = Host::new();
    let effects = host
        .runner
        .recover(vec![running_snapshot, waiting_snapshot]);
    // Only the interrupted run changes; the waiting run emits nothing.
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    host.apply(effects);
    let last = host.last_run();
    assert_eq!(last.id, running.id);
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("Interrupted by app restart"));
    assert!(last.finished_at.is_some());
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some("Interrupted by app restart"));

    // The waiting run is live again: approving it moves on.
    let effects = host.runner.resolve_approval(&waiting.id, true, "LGTM");
    assert_eq!(*kinds(&effects).last().unwrap(), "finished");
    host.apply(effects);
    assert_eq!(host.last_run().status, RunStatus::Succeeded);
}

#[test]
fn resume_appends_the_next_seq_and_reuses_the_worktree() {
    let mut first = Host::new();
    let (run, _) = first.start(&fix_an_issue(), &[("issue", "Bug")]);
    first.runner.on_worktree(&run.id, "/wt/a");
    first.outputs(&run.id, triage_out());
    let mut visits = first.visits();
    let snapshot = (first.last_run(), visits.clone());

    // The app restarts mid-implement; recovery fails the running visit.
    let mut host = Host::new();
    let effects = host.runner.recover(vec![snapshot]);
    host.apply(effects);
    let failed = host.last_run();
    assert_eq!(failed.status, RunStatus::Failed);
    visits[1] = host.visits().pop().unwrap();
    assert_eq!(visits[1].status, VisitStatus::Failed);

    let effects = host
        .runner
        .resume(failed.clone(), visits.clone(), "implement");
    assert_eq!(kinds(&effects), ["persist", "persist", "launch"]);
    host.apply(effects);
    let resumed = host.last_run();
    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(resumed.error, None);
    assert_eq!(resumed.finished_at, None);
    assert_eq!(resumed.current_state.as_deref(), Some("implement"));
    let launch = host.last_launch();
    assert_eq!(launch.worktree_cwd.as_deref(), Some("/wt/a"));
    assert!(
        launch.prompt.contains("Patch parser"),
        "earlier outputs bind"
    );
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.seq, 3);
    assert_eq!(visit.state_id, "implement");

    // Only a failed or cancelled run can resume.
    assert!(host.runner.resume(resumed, visits, "implement").is_empty());
}

#[test]
fn resume_rejects_unknown_state() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    host.fail(&run.id, Some("x"));
    let (failed, visits) = (host.last_run(), host.visits());
    assert!(host.runner.resume(failed, visits, "nope").is_empty());
}

// ---- start & snapshot --------------------------------------------------

#[test]
fn start_refuses_invalid_flow_or_missing_inputs() {
    let mut runner = Host::new().runner;
    let mut broken = fix_an_issue();
    broken.initial = "nowhere".into();
    assert!(runner.start(&broken, &HashMap::new(), None).is_err());
    let missing = runner
        .start(&fix_an_issue(), &HashMap::new(), None)
        .unwrap_err();
    assert_eq!(missing, ["issue"]);
}

#[test]
fn run_keeps_its_snapshot_when_the_flow_is_edited() {
    let mut host = Host::new();
    let mut flow = fix_an_issue();
    let (run, _) = host.start(&flow, &[("issue", "Bug")]);
    if let FlowState::Agent(a) = &mut flow.states[1] {
        a.brief = "EDITED".into();
    }
    flow.name = "Renamed".into();
    host.outputs(&run.id, triage_out());
    let implement = host.last_launch();
    assert!(!implement.prompt.contains("EDITED"));
    assert_eq!(implement.title, "Fix an issue · Implement");
    assert_eq!(host.last_run().flow.name, "Fix an issue");
}

#[test]
fn persist_precedes_launch_and_ids_come_from_the_injected_source() {
    let mut host = Host::new();
    let (run, effects) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    assert_eq!(run.id, "id1");
    assert_eq!(run.created_at, 1001);
    let Effect::Persist {
        visit: Some(visit), ..
    } = &effects[1]
    else {
        panic!("expected the visit persist");
    };
    assert_eq!(visit.id, "id2");
    assert_eq!(visit.seq, 1);
    assert_eq!(visit.status, VisitStatus::Running);
    let Effect::Launch(launch) = &effects[2] else {
        panic!("expected a launch");
    };
    assert_eq!(launch.visit_id, visit.id);
    assert_eq!(visit.prompt.as_deref(), Some(launch.prompt.as_str()));
    // The reported session id is persisted on the visit.
    assert_eq!(host.visits()[0].session_id.as_deref(), Some("sess-id2"));
}

// ---- fix round 1 -------------------------------------------------------

#[test]
fn cancel_before_the_session_is_reported_stops_the_late_session() {
    let mut host = Host::new();
    let flow = fix_an_issue();
    // Start without the fake host's automatic session reports.
    let given = HashMap::from([("issue".to_string(), "Bug".to_string())]);
    let (run, effects) = host.runner.start(&flow, &given, None).unwrap();
    let Some(Effect::Launch(launch)) = effects.last() else {
        panic!("expected a launch");
    };
    let effects = host.runner.cancel(&run.id);
    assert_eq!(kinds(&effects), ["persist", "finished"], "no session yet");

    let late = host
        .runner
        .on_session(&run.id, &launch.visit_id, "sess-late");
    assert_eq!(
        late,
        [Effect::Cancel {
            session_id: "sess-late".into()
        }]
    );
    // The tombstone is cleared: a duplicate report does nothing.
    assert!(host
        .runner
        .on_session(&run.id, &launch.visit_id, "sess-late")
        .is_empty());
}

#[test]
fn cancel_during_a_contract_retry_stops_its_session() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let launch = host.last_launch();
    host.reply(&run.id, "no json");
    let visit = host.visits().pop().unwrap();
    assert!(visit.retried);
    assert_eq!(visit.status, VisitStatus::Running);

    let effects = host.runner.cancel(&run.id);
    assert_eq!(kinds(&effects), ["persist", "cancel", "finished"]);
    assert_eq!(
        effects[1],
        Effect::Cancel {
            session_id: format!("sess-{}", launch.visit_id)
        }
    );
}

#[test]
fn recover_emits_finished_after_persist_for_each_failed_run() {
    let mut first = Host::new();
    let (a, _) = first.start(&fix_an_issue(), &[("issue", "Bug")]);
    let snap_a = (first.last_run(), first.visits());
    let mut second = Host::new();
    let (b, _) = second.start(&research(), &[("topic", "t")]);
    let snap_b = (second.last_run(), second.visits());

    let mut host = Host::new();
    let effects = host.runner.recover(vec![snap_a, snap_b]);
    assert_eq!(
        kinds(&effects),
        [
            "persist",
            "finished",
            "visit-finished",
            "persist",
            "finished",
            "visit-finished"
        ]
    );
    assert_eq!(
        effects[1],
        Effect::Finished {
            run_id: a.id,
            status: RunStatus::Failed
        }
    );
    assert_eq!(
        effects[4],
        Effect::Finished {
            run_id: b.id,
            status: RunStatus::Failed
        }
    );
}

#[test]
fn settling_a_visit_whose_state_left_the_snapshot_fails_the_run() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    // Corrupt the in-memory snapshot: the running visit's state disappears.
    let active = host.runner.runs.get_mut(&run.id).unwrap();
    active.run.flow.states.retain(|s| s.id() != "triage");

    let effects = host.outputs(&run.id, triage_out());
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    let visit = host.visits().pop().unwrap();
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some("Unknown state triage"));
    let last = host.last_run();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("Unknown state triage"));
    assert_eq!(host.finished(), [RunStatus::Failed]);
}

#[test]
fn has_active_tracks_a_flows_in_memory_runs() {
    let mut host = Host::new();
    let flow = fix_an_issue();
    assert!(!host.runner.has_active(&flow.id));
    let (run, _) = host.start(&flow, &[("issue", "Bug")]);
    assert!(host.runner.has_active(&flow.id));
    assert!(!host.runner.has_active("another-flow"));
    host.runner.cancel(&run.id);
    assert!(
        !host.runner.has_active(&flow.id),
        "a finished run is dropped"
    );

    // A run waiting for approval is still active.
    let research = research();
    let (run, _) = host.start(&research, &[("topic", "t")]);
    host.outputs(&run.id, json!({"findings": [], "sources": []}));
    host.outputs(&run.id, json!({"summary": "s"}));
    assert_eq!(host.last_run().status, RunStatus::WaitingApproval);
    assert!(host.runner.has_active(&research.id));
}

#[test]
fn launches_carry_the_snapshots_access_mode() {
    let mut host = Host::new();
    let mut flow = fix_an_issue();
    assert_eq!(flow.access, "auto");
    host.start(&flow, &[("issue", "Bug")]);
    assert_eq!(host.last_launch().access, "auto");
    flow.access = "read-only".into();
    flow.id = "flow-ro".into();
    host.start(&flow, &[("issue", "Bug")]);
    assert_eq!(host.last_launch().access, "read-only");
}

// ---- Pilot Flows: VisitFinished, tags, run edits -------------------------

/// `s1 -> s2 -> s3 -> $succeeded`, no outputs.
fn three_steps() -> Flow {
    custom(
        "s1",
        vec![
            agent("s1", "First brief", vec![], vec![to("s2")], 3),
            agent("s2", "Second brief", vec![], vec![to("s3")], 3),
            agent("s3", "Third brief", vec![], vec![to(SUCCEEDED)], 3),
        ],
    )
}

fn set_brief(flow: &mut Flow, state: &str, brief: &str) {
    let found = flow.states.iter_mut().find(|s| s.id() == state);
    let Some(FlowState::Agent(a)) = found else {
        panic!("no agent state {state}");
    };
    a.brief = brief.into();
}

fn visit_finished(effects: &[Effect]) -> Vec<(FlowRun, FlowVisit)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::VisitFinished { run, visit } => Some((run.clone(), visit.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_finished_visit_is_reported_after_the_next_launch() {
    let flow = custom(
        "s1",
        vec![
            agent("s1", "One", vec![], vec![to("s2")], 3),
            agent("s2", "Two", vec![], vec![to(SUCCEEDED)], 3),
        ],
    );
    let mut host = Host::new();
    let (run, _) = host.start(&flow, &[]);
    let effects = host.reply(&run.id, "done");
    assert_eq!(
        kinds(&effects),
        ["persist", "persist", "launch", "visit-finished"]
    );
    let Effect::Launch(launch) = &effects[2] else {
        panic!("expected a launch");
    };
    assert_eq!(launch.title, "Custom · S2");
    let Some(Effect::VisitFinished { run: after, visit }) = effects.last() else {
        panic!("expected VisitFinished last");
    };
    assert_eq!(visit.state_id, "s1");
    assert_eq!(visit.status, VisitStatus::Completed);
    assert!(visit.finished_at.is_some());
    assert_eq!(after.id, run.id);
    assert_eq!(after.status, RunStatus::Running);
    assert_eq!(after.current_state.as_deref(), Some("s2"));

    // The last step reports its visit after the run's Finished.
    let effects = host.reply(&run.id, "done");
    assert_eq!(kinds(&effects), ["persist", "finished", "visit-finished"]);
    let reported = visit_finished(&effects);
    assert_eq!(reported[0].1.state_id, "s2");
    assert_eq!(reported[0].0.status, RunStatus::Succeeded);
}

#[test]
fn a_failed_visit_and_a_failed_run_are_both_reported() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let effects = host.fail(&run.id, Some("harness crashed"));
    assert!(effects.contains(&Effect::Finished {
        run_id: run.id.clone(),
        status: RunStatus::Failed,
    }));
    let reported = visit_finished(&effects);
    assert_eq!(reported.len(), 1);
    let (after, visit) = &reported[0];
    assert_eq!(visit.state_id, "triage");
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some("harness crashed"));
    assert_eq!(after.status, RunStatus::Failed);
}

#[test]
fn cancel_and_contract_retry_do_not_report_a_finished_visit() {
    let mut host = Host::new();
    let (run, _) = host.start(&fix_an_issue(), &[("issue", "Bug")]);
    let effects = host.reply(&run.id, "no json");
    assert_eq!(kinds(&effects), ["persist", "follow-up"]);

    let effects = host.runner.cancel(&run.id);
    assert_eq!(kinds(&effects), ["persist", "cancel", "finished"]);
    host.apply(effects);
    assert!(visit_finished(&host.log).is_empty());
}

#[test]
fn recover_reports_each_interrupted_visit() {
    let mut first = Host::new();
    let (a, _) = first.start(&fix_an_issue(), &[("issue", "Bug")]);
    let snap_a = (first.last_run(), first.visits());
    let mut second = Host::new();
    let (b, _) = second.start(&research(), &[("topic", "t")]);
    let snap_b = (second.last_run(), second.visits());
    let visit_a = snap_a.1[0].id.clone();
    let visit_b = snap_b.1[0].id.clone();

    let mut host = Host::new();
    let effects = host.runner.recover(vec![snap_a, snap_b]);
    let reported = visit_finished(&effects);
    assert_eq!(reported.len(), 2);
    for ((run, visit), (run_id, visit_id)) in
        reported.iter().zip([(&a.id, &visit_a), (&b.id, &visit_b)])
    {
        assert_eq!(&run.id, run_id);
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(&visit.id, visit_id);
        assert_eq!(visit.status, VisitStatus::Failed);
        assert_eq!(visit.error.as_deref(), Some("Interrupted by app restart"));
    }
}

#[test]
fn start_keeps_the_tag_on_the_run() {
    let tag = RunTag {
        goal: "goal-1".into(),
        task: "task-1".into(),
    };
    let mut runner = Host::new().runner;
    let (run, effects) = runner
        .start(&three_steps(), &HashMap::new(), Some(tag.clone()))
        .unwrap();
    assert_eq!(run.tag.as_ref(), Some(&tag));
    let Effect::Persist { run: persisted, .. } = &effects[0] else {
        panic!("expected a persist");
    };
    assert_eq!(persisted.tag.as_ref(), Some(&tag));
    assert!(run.edits.is_empty());

    let (untagged, _) = runner.start(&three_steps(), &HashMap::new(), None).unwrap();
    assert_eq!(untagged.tag, None);
}

#[test]
fn tagged_runs_title_step_sessions_by_task() {
    let flow = three_steps();
    let tag = RunTag {
        goal: "goal-1".into(),
        task: "Ship the login fix".into(),
    };
    let mut runner = Host::new().runner;
    let launch = |effects: Vec<Effect>| {
        effects
            .into_iter()
            .find_map(|e| match e {
                Effect::Launch(l) => Some(l),
                _ => None,
            })
            .expect("a launch")
    };
    let (_, effects) = runner
        .start(&flow, &HashMap::new(), Some(tag.clone()))
        .unwrap();
    assert_eq!(launch(effects).title, "Ship the login fix · S1");
    let (_, effects) = runner.start(&flow, &HashMap::new(), None).unwrap();
    assert_eq!(launch(effects).title, format!("{} · S1", flow.name));
}

#[test]
fn edit_run_changes_only_states_not_yet_entered() {
    let mut host = Host::new();
    let flow = three_steps();
    let (run, _) = host.start(&flow, &[]);
    host.reply(&run.id, "done");
    assert_eq!(host.last_launch().title, "Custom · S2");

    let mut edited = flow.clone();
    set_brief(&mut edited, "s3", "Brand new third brief");
    let effects = host
        .runner
        .edit_run(&run.id, edited.clone(), "Sharpen the last step")
        .expect("edit");
    assert_eq!(kinds(&effects), ["persist"]);
    let Effect::Persist {
        run: persisted,
        visit,
    } = &effects[0]
    else {
        unreachable!();
    };
    assert!(visit.is_none());
    assert_eq!(persisted.flow, edited);
    assert_eq!(persisted.edits.len(), 1);
    assert_eq!(persisted.edits[0].states, ["s3"]);
    assert_eq!(persisted.edits[0].note, "Sharpen the last step");
    assert!(persisted.edits[0].at > persisted.created_at);
    host.apply(effects);

    host.reply(&run.id, "done");
    let third = host.last_launch();
    assert_eq!(third.title, "Custom · S3");
    assert!(third.prompt.contains("Brand new third brief"));
    assert_eq!(host.last_run().edits.len(), 1);

    // A new state counts as changed; an unchanged one does not.
    let mut added = three_steps();
    set_brief(&mut added, "s3", "Brand new third brief");
    if let FlowState::Agent(a) = &mut added.states[2] {
        a.transitions = vec![to("s4")];
    }
    added
        .states
        .push(agent("s4", "Fourth", vec![], vec![to(SUCCEEDED)], 3));
    let err = host.runner.edit_run(&run.id, added, "late").unwrap_err();
    assert_eq!(err, ["State \"s3\" has already run and cannot be changed"]);
}

#[test]
fn edit_run_lists_new_states_as_changed() {
    let mut host = Host::new();
    let flow = three_steps();
    let (run, _) = host.start(&flow, &[]);
    let mut edited = flow.clone();
    if let FlowState::Agent(a) = &mut edited.states[2] {
        a.transitions = vec![to("s4")];
    }
    edited
        .states
        .push(agent("s4", "Fourth", vec![], vec![to(SUCCEEDED)], 3));
    let effects = host.runner.edit_run(&run.id, edited, "add a step").unwrap();
    let Effect::Persist { run: persisted, .. } = &effects[0] else {
        panic!("expected a persist");
    };
    assert_eq!(persisted.edits[0].states, ["s3", "s4"]);
}

#[test]
fn edit_run_refuses_visited_states_identity_changes_and_invalid_flows() {
    let mut host = Host::new();
    let flow = three_steps();
    let (run, _) = host.start(&flow, &[]);
    host.reply(&run.id, "done");
    let snapshot = |host: &Host| {
        let active = &host.runner.runs[&run.id];
        (active.run.flow.clone(), active.run.edits.clone())
    };
    let before = snapshot(&host);
    let identity =
        "A run edit cannot change the Flow's id, owner, initial state, access, folder or inputs";

    let cases: Vec<(Flow, Vec<String>)> = vec![
        {
            let mut f = flow.clone();
            f.id = "other".into();
            (f, vec![identity.into()])
        },
        {
            let mut f = flow.clone();
            f.owner = Some("pilot-1".into());
            (f, vec![identity.into()])
        },
        {
            let mut f = flow.clone();
            f.initial = "s3".into();
            (f, vec![identity.into()])
        },
        {
            let mut f = flow.clone();
            f.access = "read-only".into();
            (f, vec![identity.into()])
        },
        {
            let mut f = flow.clone();
            f.cwd = "/elsewhere".into();
            (f, vec![identity.into()])
        },
        {
            let mut f = flow.clone();
            f.inputs.push(FlowInput {
                name: "extra".into(),
                label: "Extra".into(),
                required: true,
                default: None,
            });
            (f, vec![identity.into()])
        },
        {
            // A finished visit's state.
            let mut f = flow.clone();
            set_brief(&mut f, "s1", "Rewritten");
            (
                f,
                vec!["State \"s1\" has already run and cannot be changed".into()],
            )
        },
        {
            // The running visit's state.
            let mut f = flow.clone();
            set_brief(&mut f, "s2", "Rewritten");
            (
                f,
                vec!["State \"s2\" has already run and cannot be changed".into()],
            )
        },
        {
            // A visited state that the edit drops.
            let mut f = flow.clone();
            f.states.retain(|s| s.id() != "s2");
            if let FlowState::Agent(a) = &mut f.states[0] {
                a.transitions = vec![to("s3")];
            }
            (
                f,
                vec![
                    "State \"s1\" has already run and cannot be changed".into(),
                    "State \"s2\" has already run and cannot be changed".into(),
                ],
            )
        },
        {
            let mut f = flow.clone();
            if let FlowState::Agent(a) = &mut f.states[2] {
                a.transitions = vec![to("nowhere")];
            }
            let errors = validate_flow(&f).errors;
            assert!(!errors.is_empty());
            (f, errors)
        },
    ];
    for (edited, expected) in cases {
        let err = host.runner.edit_run(&run.id, edited, "n").unwrap_err();
        assert_eq!(err, expected);
        assert_eq!(snapshot(&host), before, "the snapshot is unchanged");
    }

    // Name and description may change; no state does.
    let mut renamed = flow.clone();
    renamed.name = "Renamed".into();
    renamed.description = "New words".into();
    let effects = host
        .runner
        .edit_run(&run.id, renamed.clone(), "rename")
        .unwrap();
    let Effect::Persist { run: persisted, .. } = &effects[0] else {
        panic!("expected a persist");
    };
    assert_eq!(persisted.flow, renamed);
    assert!(persisted.edits[0].states.is_empty());

    let err = host.runner.edit_run("nope", flow.clone(), "n").unwrap_err();
    assert_eq!(err, ["Run not found"]);

    // A finished run is no longer active.
    host.reply(&run.id, "done");
    host.reply(&run.id, "done");
    assert_eq!(host.last_run().status, RunStatus::Succeeded);
    let err = host.runner.edit_run(&run.id, flow, "n").unwrap_err();
    assert_eq!(err, ["Run not found"]);
}
