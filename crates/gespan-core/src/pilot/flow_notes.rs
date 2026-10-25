//! What a Pilot hears about its Flow runs: the side review of a finished
//! step, the alerts that wake the main chat, and the roll-up of a goal.
use crate::flow::{FlowRun, FlowVisit};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteKind {
    NoChange,
    SuggestStepChange,
    NeedsAttention,
    BlueprintIdea,
    /// The review itself could not do its job (it never started, failed or
    /// timed out). Worth reading in the roll-up, never an alert.
    ReviewProblem,
}

/// A side review's short note about one finished step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowNote {
    pub run_id: String,
    pub goal: String,
    pub task: String,
    pub state_id: String,
    pub kind: NoteKind,
    pub text: String,
    pub at: i64,
}

/// The runs a Pilot started for one goal, reported once all of them end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub name: String,
    pub runs: Vec<String>,
    pub reported: bool,
}

/// Concurrent review runs per Pilot.
pub const MAX_REVIEWS: usize = 3;
/// Notes a Pilot keeps; the oldest notes of reported goals are dropped first.
pub const MAX_NOTES: usize = 500;
/// The most characters of a step's reply, and of its outputs, a review sees.
pub const STEP_TEXT_MAX: usize = 12_000;
const CONTRACT: &str = "Review only this step. You cannot change anything. Reply with one first line: NO_CHANGE, SUGGEST_STEP_CHANGE, NEEDS_ATTENTION or BLUEPRINT_IDEA, then at most a short paragraph explaining it.";

fn tag(run: &FlowRun) -> (&str, &str) {
    run.tag
        .as_ref()
        .map_or(("", ""), |t| (t.goal.as_str(), t.task.as_str()))
}

fn step_title<'a>(run: &'a FlowRun, state_id: &'a str) -> &'a str {
    run.flow
        .states
        .iter()
        .find(|s| s.id() == state_id)
        .map(|s| match s {
            crate::flow::FlowState::Agent(a) => a.title.as_str(),
            crate::flow::FlowState::Approval(a) => a.title.as_str(),
        })
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(state_id)
}

/// The internal prompt of a hidden, read-only review of one finished step.
pub fn review_prompt(run: &FlowRun, visit: &FlowVisit, step_text: &str) -> String {
    let (goal, task) = tag(run);
    let outputs = visit
        .outputs
        .as_ref()
        .filter(|o| !o.is_empty())
        .and_then(|o| {
            let sorted: std::collections::BTreeMap<_, _> = o.iter().collect();
            serde_json::to_string_pretty(&sorted).ok()
        })
        .map_or_else(
            || "(none)".to_string(),
            |o| crate::response::limit(&o, STEP_TEXT_MAX),
        );
    let reply = if step_text.trim().is_empty() {
        "(none)".to_string()
    } else {
        crate::response::limit(step_text.trim(), STEP_TEXT_MAX)
    };
    let others = run
        .flow
        .states
        .iter()
        .map(|s| s.id())
        .filter(|id| *id != visit.state_id)
        .collect::<Vec<_>>();
    let others = if others.is_empty() {
        "(none)".to_string()
    } else {
        others.join(", ")
    };
    format!(
        "[Flow step review]\nGoal: {goal}\nTask: {task}\nStep: {} ({})\nRun: {}\n\nOutputs:\n{outputs}\n\nReply:\n{reply}\n\nOther states: {others}\n\n{CONTRACT}",
        step_title(run, &visit.state_id),
        visit.state_id,
        run.id,
    )
}

/// A review's reply: its first line names the kind; anything else needs attention.
pub fn parse_note(reply: &str) -> (NoteKind, String) {
    let reply = reply.trim();
    let (first, rest) = reply.split_once('\n').unwrap_or((reply, ""));
    let first = first.trim().trim_matches(['*', '`']).trim();
    let end = first
        .find(|c: char| !(c.is_ascii_uppercase() || c == '_'))
        .unwrap_or(first.len());
    let kind = match &first[..end] {
        "NO_CHANGE" => NoteKind::NoChange,
        "SUGGEST_STEP_CHANGE" => NoteKind::SuggestStepChange,
        "NEEDS_ATTENTION" => NoteKind::NeedsAttention,
        "BLUEPRINT_IDEA" => NoteKind::BlueprintIdea,
        _ => return (NoteKind::NeedsAttention, reply.to_string()),
    };
    let tail = first[end..]
        .trim_start_matches(['*', '`', ':', '-', '—'])
        .trim();
    let text = [tail, rest.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (kind, text)
}

/// Wakes the main Pilot to take over a task whose step failed or was stopped.
pub fn failure_alert(run: &FlowRun, visit: &FlowVisit) -> String {
    let (goal, task) = tag(run);
    let what = match visit.status {
        crate::flow::VisitStatus::Cancelled => "was stopped",
        _ => "failed",
    };
    let error = visit
        .error
        .as_deref()
        .or(run.error.as_deref())
        .filter(|e| !e.trim().is_empty())
        .unwrap_or("No error was recorded");
    let session = visit.session_id.as_deref().unwrap_or("none");
    format!(
        "A step of task \"{task}\" {what}.\nGoal: {goal}\nStep: {} ({})\nError: {error}\nSession: {session}\nUse flows.run {} to inspect, then flows.resume, flows.editRun or flows.start; ask the user only if you can't recover.",
        step_title(run, &visit.state_id),
        visit.state_id,
        run.id,
    )
}

fn kind_label(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::NoChange => "no change",
        NoteKind::SuggestStepChange => "suggests a step change",
        NoteKind::NeedsAttention => "needs attention",
        NoteKind::BlueprintIdea => "blueprint idea",
        NoteKind::ReviewProblem => "Review problem",
    }
}

/// A review note worth the main Pilot's attention.
pub fn note_alert(note: &FlowNote) -> String {
    let next = match note.kind {
        NoteKind::SuggestStepChange => {
            "If it still helps, change this run's steps not yet started with flows.editRun."
        }
        NoteKind::BlueprintIdea => {
            "Note it for the blueprint and confirm the change with the user before saving it."
        }
        _ => "Check the run with flows.run and decide whether it needs you.",
    };
    let text = if note.text.trim().is_empty() {
        "(no explanation)"
    } else {
        note.text.trim()
    };
    format!(
        "Review of step {} of task \"{}\" (goal: {}): {}.\n{text}\nRun: {}\n{next}",
        note.state_id,
        note.task,
        note.goal,
        kind_label(note.kind),
        note.run_id,
    )
}

/// The one consolidated update when every run of `goal` has ended.
pub fn goal_rollup(goal: &Goal, runs: &[FlowRun], notes: &[FlowNote]) -> String {
    let mut out = format!(
        "[Goal finished] {}\nEvery task of this goal has ended. Give the user one consolidated update.\n\nTasks:",
        goal.name
    );
    for id in &goal.runs {
        let Some(run) = runs.iter().find(|r| &r.id == id) else {
            continue;
        };
        let (_, task) = tag(run);
        out.push_str(&format!(
            "\n- {task}: {} (run {})",
            run.status.as_str(),
            run.id
        ));
        if let Some(error) = run.error.as_deref().filter(|e| !e.trim().is_empty()) {
            out.push_str(&format!(" — {error}"));
        }
    }
    let ours = notes
        .iter()
        .filter(|n| n.goal == goal.name && goal.runs.contains(&n.run_id));
    let worth: Vec<_> = ours
        .clone()
        .filter(|n| n.kind != NoteKind::NoChange)
        .collect();
    let quiet = ours.filter(|n| n.kind == NoteKind::NoChange).count();
    if !worth.is_empty() {
        out.push_str("\n\nReview notes:");
        for n in worth {
            out.push_str(&format!(
                "\n- {} · {}: {}: {}",
                n.task,
                n.state_id,
                kind_label(n.kind),
                n.text.trim()
            ));
        }
    }
    out.push_str(&format!(
        "\n\n{quiet} step review{} found nothing to change.",
        if quiet == 1 { "" } else { "s" }
    ));
    out
}

/// Alerts that waited for the main chat, delivered as one message.
pub fn bundle(alerts: &[String]) -> String {
    match alerts {
        [one] => format!("[Flow update]\n\n{one}"),
        many => format!(
            "[Flow update]\n{} updates from your Flows.\n\n{}",
            many.len(),
            many.join("\n\n---\n\n")
        ),
    }
}

/// Keeps at most `MAX_NOTES`, dropping the oldest notes of reported goals first.
pub fn trim_notes(notes: &mut Vec<FlowNote>, goals: &[Goal]) {
    while notes.len() > MAX_NOTES {
        let reported = notes.iter().position(|n| {
            goals
                .iter()
                .any(|g| g.reported && g.runs.contains(&n.run_id))
        });
        notes.remove(reported.unwrap_or(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{
        AgentState, Flow, FlowState, OutputValue, RunStatus, RunTag, Transition, VisitStatus,
    };
    use std::collections::HashMap;

    fn run(id: &str, task: &str, status: RunStatus) -> FlowRun {
        let agent = |id: &str, title: &str| {
            FlowState::Agent(AgentState {
                id: id.into(),
                title: title.into(),
                harness: "codex".into(),
                model: String::new(),
                brief: "do it".into(),
                outputs: vec![],
                transitions: vec![Transition {
                    to: "$succeeded".into(),
                    when: None,
                }],
                max_visits: 3,
            })
        };
        FlowRun {
            id: id.into(),
            flow_id: "f".into(),
            flow: Flow {
                id: "f".into(),
                name: "Blueprint".into(),
                description: String::new(),
                cwd: "/work".into(),
                inputs: vec![],
                initial: "plan".into(),
                states: vec![
                    agent("plan", "Plan"),
                    agent("build", "Build"),
                    agent("check", "Check"),
                ],
                access: Default::default(),
                owner: Some("p".into()),
                created_at: 0,
                updated_at: 0,
            },
            status,
            inputs: HashMap::new(),
            worktree_cwd: None,
            current_state: Some("build".into()),
            error: None,
            created_at: 0,
            finished_at: None,
            tag: Some(RunTag {
                goal: "Ship login".into(),
                task: task.into(),
            }),
            edits: vec![],
        }
    }

    fn visit(state: &str, status: VisitStatus) -> FlowVisit {
        FlowVisit {
            id: "v1".into(),
            run_id: "r1".into(),
            seq: 1,
            state_id: state.into(),
            status,
            prompt: None,
            session_id: Some("42".into()),
            outputs: Some(HashMap::from([(
                "summary".to_string(),
                OutputValue::String("planned".into()),
            )])),
            error: None,
            retried: false,
            note: None,
            started_at: 0,
            finished_at: None,
        }
    }

    fn note(run_id: &str, kind: NoteKind, text: &str) -> FlowNote {
        FlowNote {
            run_id: run_id.into(),
            goal: "Ship login".into(),
            task: format!("task of {run_id}"),
            state_id: "plan".into(),
            kind,
            text: text.into(),
            at: 0,
        }
    }

    #[test]
    fn parse_note_reads_the_kind_line_and_defaults_to_needs_attention() {
        assert_eq!(parse_note("NO_CHANGE"), (NoteKind::NoChange, String::new()));
        assert_eq!(
            parse_note("  SUGGEST_STEP_CHANGE\nAdd tests to the build brief.\n"),
            (
                NoteKind::SuggestStepChange,
                "Add tests to the build brief.".into()
            )
        );
        assert_eq!(
            parse_note("**BLUEPRINT_IDEA**: lint before review"),
            (NoteKind::BlueprintIdea, "lint before review".into())
        );
        assert_eq!(
            parse_note("NEEDS_ATTENTION\nThe plan skips the migration."),
            (
                NoteKind::NeedsAttention,
                "The plan skips the migration.".into()
            )
        );
        // An unknown or lower-case first line is kept whole, as needing attention.
        for reply in [
            "no_change",
            "Looks fine to me.\nNO_CHANGE",
            "NO_CHANGES",
            "",
        ] {
            assert_eq!(
                parse_note(reply),
                (NoteKind::NeedsAttention, reply.trim().to_string()),
                "{reply:?}"
            );
        }
    }

    #[test]
    fn bundle_formats_one_and_many_alerts() {
        assert_eq!(bundle(&["A failed".into()]), "[Flow update]\n\nA failed");
        assert_eq!(
            bundle(&["one".into(), "two".into(), "three".into()]),
            "[Flow update]\n3 updates from your Flows.\n\none\n\n---\n\ntwo\n\n---\n\nthree"
        );
    }

    #[test]
    fn goal_rollup_lists_each_task_and_only_notes_worth_reading() {
        let goal = Goal {
            name: "Ship login".into(),
            runs: vec!["r1".into(), "r2".into(), "r3".into()],
            reported: false,
        };
        let mut failed = run("r2", "Fix the form", RunStatus::Failed);
        failed.error = Some("Interrupted by app restart".into());
        let runs = [
            run("r1", "Add the API", RunStatus::Succeeded),
            failed,
            run("r3", "Write docs", RunStatus::Cancelled),
            run("other", "Not this goal", RunStatus::Succeeded),
        ];
        let notes = [
            note("r1", NoteKind::NoChange, "fine"),
            note(
                "r1",
                NoteKind::SuggestStepChange,
                "Put tests in the build brief",
            ),
            note("r2", NoteKind::NoChange, ""),
            note("r3", NoteKind::BlueprintIdea, "Lint before review"),
            note("other", NoteKind::NeedsAttention, "Another goal's note"),
            note("r2", NoteKind::ReviewProblem, "Review timed out"),
        ];
        let text = goal_rollup(&goal, &runs, &notes);
        assert!(text.starts_with("[Goal finished] Ship login\n"), "{text}");
        for line in [
            "- Add the API: succeeded (run r1)",
            "- Fix the form: failed (run r2) — Interrupted by app restart",
            "- Write docs: cancelled (run r3)",
            "suggests a step change: Put tests in the build brief",
            "blueprint idea: Lint before review",
            "- task of r2 · plan: Review problem: Review timed out",
            "2 step reviews found nothing to change.",
        ] {
            assert!(text.contains(line), "missing {line:?} in {text}");
        }
        assert!(!text.contains("Not this goal"), "{text}");
        assert!(!text.contains("Another goal's note"), "{text}");
        assert!(
            !text.contains(": fine"),
            "NO_CHANGE notes are only counted: {text}"
        );
        let single = goal_rollup(&goal, &runs, &notes[..1]);
        assert!(
            single.ends_with("1 step review found nothing to change."),
            "{single}"
        );
        assert!(!single.contains("Review notes:"), "{single}");
    }

    #[test]
    fn review_prompt_truncates_long_step_text_and_ends_with_the_contract() {
        let run = run("r1", "Add the API", RunStatus::Running);
        let visit = visit("plan", VisitStatus::Completed);
        let long = "x".repeat(STEP_TEXT_MAX * 2);
        let prompt = review_prompt(&run, &visit, &long);
        assert!(prompt.ends_with(CONTRACT), "{prompt}");
        assert!(prompt.ends_with("Review only this step. You cannot change anything. Reply with one first line: NO_CHANGE, SUGGEST_STEP_CHANGE, NEEDS_ATTENTION or BLUEPRINT_IDEA, then at most a short paragraph explaining it."));
        assert!(prompt.contains("[truncated]"));
        assert!(prompt.chars().filter(|c| *c == 'x').count() <= STEP_TEXT_MAX);
        assert!(prompt.len() < STEP_TEXT_MAX + 1_000, "{}", prompt.len());
        for part in [
            "Goal: Ship login",
            "Task: Add the API",
            "Step: Plan (plan)",
            "\"summary\": \"planned\"",
            "Other states: build, check",
        ] {
            assert!(prompt.contains(part), "missing {part:?}");
        }
        let short = review_prompt(&run, &visit, "Planned three steps.");
        assert!(short.contains("Reply:\nPlanned three steps.\n"), "{short}");
        assert!(!short.contains("[truncated]"));
    }

    #[test]
    fn failure_alert_names_the_task_step_error_and_recovery() {
        let run = run("r1", "Add the API", RunStatus::Failed);
        let mut failed = visit("build", VisitStatus::Failed);
        failed.error = Some("Interrupted by app restart".into());
        let text = failure_alert(&run, &failed);
        for part in [
            "task \"Add the API\" failed",
            "Goal: Ship login",
            "Step: Build (build)",
            "Error: Interrupted by app restart",
            "Session: 42",
            "Use flows.run r1 to inspect, then flows.resume, flows.editRun or flows.start; ask the user only if you can't recover.",
        ] {
            assert!(text.contains(part), "missing {part:?} in {text}");
        }
        let stopped = failure_alert(&run, &visit("build", VisitStatus::Cancelled));
        assert!(stopped.contains("was stopped"), "{stopped}");
        assert!(
            stopped.contains("Error: No error was recorded"),
            "{stopped}"
        );
    }

    #[test]
    fn notes_beyond_the_cap_drop_reported_goals_first() {
        let goals = [Goal {
            name: "old".into(),
            runs: vec!["old".into()],
            reported: true,
        }];
        let mut notes: Vec<_> = (0..MAX_NOTES)
            .map(|i| {
                note(
                    if i % 2 == 0 { "live" } else { "old" },
                    NoteKind::NoChange,
                    "",
                )
            })
            .collect();
        notes.push(note("live", NoteKind::NeedsAttention, "newest"));
        trim_notes(&mut notes, &goals);
        assert_eq!(notes.len(), MAX_NOTES);
        assert_eq!(
            notes[0].run_id, "live",
            "the oldest reported-goal note went first"
        );
        assert_eq!(notes.last().unwrap().text, "newest");
        // Nothing reported: the oldest note goes.
        let mut notes: Vec<_> = (0..=MAX_NOTES)
            .map(|i| note("live", NoteKind::NoChange, &i.to_string()))
            .collect();
        trim_notes(&mut notes, &[]);
        assert_eq!(notes[0].text, "1");
    }

    #[test]
    fn a_pilot_saved_before_flows_loads_and_its_flow_state_round_trips() {
        let old: crate::pilot::Pilot =
            serde_json::from_str(r#"{"id":"p","name":"Old","outbox":[]}"#).unwrap();
        assert!(old.flow_notes.is_empty() && old.flow_alerts.is_empty() && old.goals.is_empty());
        let mut pilot = old;
        pilot
            .flow_notes
            .push(note("r1", NoteKind::BlueprintIdea, "idea"));
        pilot.flow_alerts.push("A step failed".into());
        pilot.goals.push(Goal {
            name: "g".into(),
            runs: vec!["r1".into()],
            reported: true,
        });
        let json = serde_json::to_value(&pilot).unwrap();
        assert_eq!(json["flowNotes"][0]["kind"], "blueprint-idea");
        assert_eq!(json["goals"][0]["reported"], true);
        let back: crate::pilot::Pilot = serde_json::from_value(json).unwrap();
        assert_eq!(back.flow_notes, pilot.flow_notes);
        assert_eq!(back.flow_alerts, pilot.flow_alerts);
        assert_eq!(back.goals, pilot.goals);
    }
}
