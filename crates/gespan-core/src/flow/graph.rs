//! A Flow as the Pipeline tab draws it: top to bottom, one row per step,
//! a parallel step as a row of its branches. Pure, so the layout and each
//! card's status are tested without a window.
use super::definition::{Flow, FlowState, Guard, GuardOp, FAILED, SUCCEEDED};
use super::run::{FlowVisit, VisitStatus};
use std::collections::{HashMap, HashSet};

/// One row of the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphRow {
    /// A step drawn as one centred card.
    Step(String),
    /// A parallel step: its branches side by side between a fork and a
    /// join. `retries` is how often a failed branch runs again (its
    /// `maxVisits` less one, the most of any branch).
    Branches {
        parallel: String,
        branches: Vec<String>,
        retries: u8,
    },
}

impl GraphRow {
    /// The state ids drawn in this row, left to right.
    pub fn cards(&self) -> Vec<&str> {
        match self {
            Self::Step(id) => vec![id.as_str()],
            Self::Branches { branches, .. } => branches.iter().map(String::as_str).collect(),
        }
    }

    /// Whether `id` is this row's step, its parallel step or a branch.
    pub fn holds(&self, id: &str) -> bool {
        match self {
            Self::Step(step) => step == id,
            Self::Branches {
                parallel, branches, ..
            } => parallel == id || branches.iter().any(|b| b == id),
        }
    }
}

/// The rows of `flow`: the path from `initial` along each step's first
/// transition to another step (an unguarded one first, so the usual path
/// reads straight down), then any step that path never reaches, in the
/// order the Flow lists them. Never panics on an invalid Flow.
fn graph_rows(flow: &Flow) -> Vec<GraphRow> {
    let branches: HashSet<&str> = flow
        .states
        .iter()
        .filter_map(|s| match s {
            FlowState::Parallel(p) => Some(p.branches.iter().map(String::as_str)),
            _ => None,
        })
        .flatten()
        .collect();
    let mut rows = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut next = flow.state(&flow.initial);
    while let Some(state) = next {
        if !seen.insert(state.id()) {
            break;
        }
        rows.push(row_of(state));
        next = main_next(flow, state).filter(|s| !branches.contains(s.id()));
    }
    for state in &flow.states {
        if !seen.contains(state.id()) && !branches.contains(state.id()) {
            seen.insert(state.id());
            rows.push(row_of(state));
        }
    }
    rows
}

fn row_of(state: &FlowState) -> GraphRow {
    match state {
        FlowState::Parallel(p) => GraphRow::Branches {
            parallel: p.id.clone(),
            branches: p.branches.clone(),
            retries: 0,
        },
        _ => GraphRow::Step(state.id().to_owned()),
    }
}

/// The step `state` usually goes on to: its first unguarded transition to
/// a step, else its first to a step that is not a failure route.
fn main_next<'a>(flow: &'a Flow, state: &FlowState) -> Option<&'a FlowState> {
    let to_step = |t: &&super::definition::Transition| t.to != SUCCEEDED && t.to != FAILED;
    let transitions = state.transitions();
    let pick = transitions
        .iter()
        .filter(to_step)
        .find(|t| t.when.is_none())
        .or_else(|| {
            transitions
                .iter()
                .filter(to_step)
                .find(|t| !matches!(t.when, Some(Guard::Status { .. })))
        })?;
    flow.state(&pick.to)
}

/// `graph_rows` with each parallel row's `retries` filled in.
pub fn layout(flow: &Flow) -> Vec<GraphRow> {
    graph_rows(flow)
        .into_iter()
        .map(|row| match row {
            GraphRow::Branches {
                parallel, branches, ..
            } => {
                let most = branches
                    .iter()
                    .filter_map(|b| flow.state(b))
                    .map(FlowState::max_visits)
                    .max()
                    .unwrap_or(1);
                GraphRow::Branches {
                    parallel,
                    branches,
                    retries: most.saturating_sub(1),
                }
            }
            row => row,
        })
        .collect()
}

/// When branch `id` runs, in words, from its `when`: "if mobile/ changed".
/// The output's description names it when there is one, else its name.
pub fn branch_condition(flow: &Flow, id: &str) -> Option<String> {
    let Some(FlowState::Agent(branch)) = flow.state(id) else {
        return None;
    };
    let Some(Guard::Output { field, op, value }) = &branch.when else {
        return None;
    };
    let parent = flow.parent_of(id)?;
    let described = flow
        .states
        .iter()
        .filter(|s| s.transitions().iter().any(|t| t.to == parent.id))
        .flat_map(super::definition::outputs_of)
        .find(|f| &f.name == field)
        .map(|f| f.description.trim().to_owned())
        .filter(|d| !d.is_empty() && d.chars().count() <= 40);
    let name = described.unwrap_or_else(|| field.replace('_', " "));
    let value = || match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    };
    Some(match op {
        GuardOp::Truthy => format!("if {name}"),
        GuardOp::Falsy => format!("unless {name}"),
        GuardOp::Eq => format!("if {name} is {}", value()),
        GuardOp::Neq => format!("unless {name} is {}", value()),
        GuardOp::Contains => format!("if {name} has {}", value()),
    })
}

/// Where a card's step stands in the shown run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardStatus {
    /// Not reached (or no run is shown).
    Idle,
    /// Not reached, and the run comes to it next.
    Next,
    Running,
    Done,
    Failed,
    /// Stopped by a cancel or by another branch failing.
    Stopped,
    /// A branch whose `when` did not match when its parallel step ran.
    Skipped,
}

/// One card's step in the shown run.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub status: CardStatus,
    /// The step's latest visit, if it has one.
    pub visit: Option<FlowVisit>,
}

/// Each drawn step's status in a run with `visits`; without a run every
/// card is idle. `rows` is the Flow's layout.
pub fn cards(flow: &Flow, rows: &[GraphRow], visits: &[FlowVisit]) -> HashMap<String, Card> {
    let mut ordered: Vec<&FlowVisit> = visits.iter().collect();
    ordered.sort_by_key(|v| v.seq);
    let latest = |id: &str| ordered.iter().rev().find(|v| v.state_id == id).copied();
    let mut out = HashMap::new();
    for row in rows {
        for id in row.cards() {
            let visit = latest(id);
            let mut status = match visit.map(|v| v.status) {
                None => CardStatus::Idle,
                Some(VisitStatus::Running | VisitStatus::WaitingApproval) => CardStatus::Running,
                Some(VisitStatus::Completed) => CardStatus::Done,
                Some(VisitStatus::Failed) => CardStatus::Failed,
                Some(VisitStatus::Cancelled) => CardStatus::Stopped,
            };
            // A branch not run since its step last started was skipped,
            // once that step is no longer running.
            if let GraphRow::Branches { parallel, .. } = row {
                if let Some(step) = latest(parallel) {
                    let ran = visit.is_some_and(|v| v.seq > step.seq);
                    if !ran {
                        status = if step.status == VisitStatus::Running {
                            CardStatus::Idle
                        } else {
                            CardStatus::Skipped
                        };
                    }
                }
            }
            let visit = visit.filter(|_| status != CardStatus::Skipped).cloned();
            out.insert(id.to_owned(), Card { status, visit });
        }
    }
    // The row after the furthest row reached is next.
    if let Some(reached) = current_row(rows, visits) {
        let after = rows.get(reached + 1);
        let still_going = rows[reached].cards().iter().any(|id| {
            out.get(*id)
                .is_some_and(|c| matches!(c.status, CardStatus::Running | CardStatus::Done))
        });
        if let (Some(after), true) = (after, still_going) {
            for id in after.cards() {
                let unconditional = branch_condition(flow, id).is_none();
                if let Some(card) = out.get_mut(id) {
                    if card.status == CardStatus::Idle && unconditional {
                        card.status = CardStatus::Next;
                    }
                }
            }
        }
    }
    out
}

/// The row of the run's newest visit: the step it is on, or ended on.
pub fn current_row(rows: &[GraphRow], visits: &[FlowVisit]) -> Option<usize> {
    let newest = visits.iter().max_by_key(|v| v.seq)?;
    rows.iter().position(|row| row.holds(&newest.state_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::definition::{
        AgentState, Join, OutputField, OutputType, ParallelState, Transition,
    };

    fn to(target: &str) -> Transition {
        Transition {
            to: target.into(),
            when: None,
        }
    }

    fn step(id: &str, transitions: Vec<Transition>) -> FlowState {
        FlowState::Agent(AgentState {
            id: id.into(),
            title: id.into(),
            harness: "claude".into(),
            model: "opus".into(),
            brief: format!("Do {id}."),
            outputs: vec![],
            transitions,
            max_visits: 1,
            when: None,
        })
    }

    fn branch(id: &str, max_visits: u8) -> FlowState {
        match step(id, vec![]) {
            FlowState::Agent(a) => FlowState::Agent(AgentState { max_visits, ..a }),
            _ => unreachable!(),
        }
    }

    fn parallel(id: &str, branches: &[&str], next: &str) -> FlowState {
        FlowState::Parallel(ParallelState {
            id: id.into(),
            title: id.into(),
            branches: branches.iter().map(|b| b.to_string()).collect(),
            join: Join::All,
            transitions: vec![to(next)],
            max_visits: 1,
        })
    }

    /// The board's Release pipeline: plan → build (3 branches, retried up
    /// to twice) → cut → finish (notes, mobile if mobile/ changed) → publish.
    fn release() -> Flow {
        let mut cut = step("cut", vec![to("finish")]);
        if let FlowState::Agent(a) = &mut cut {
            a.outputs = vec![OutputField {
                name: "mobile_changed".into(),
                r#type: OutputType::Boolean,
                description: "mobile/ changed".into(),
            }];
        }
        let mut mobile = branch("mobile", 1);
        if let FlowState::Agent(a) = &mut mobile {
            a.when = Some(Guard::Output {
                field: "mobile_changed".into(),
                op: GuardOp::Truthy,
                value: None,
            });
        }
        Flow {
            id: "release".into(),
            name: "Release".into(),
            description: String::new(),
            cwd: "/w".into(),
            inputs: vec![],
            initial: "plan".into(),
            states: vec![
                step("plan", vec![to("build")]),
                parallel("build", &["mac", "win", "linux"], "cut"),
                branch("mac", 3),
                branch("win", 3),
                branch("linux", 3),
                cut,
                parallel("finish", &["notes", "mobile"], "publish"),
                branch("notes", 1),
                mobile,
                step("publish", vec![to(SUCCEEDED)]),
            ],
            access: "auto".into(),
            owner: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn visit(seq: u32, state: &str, status: VisitStatus) -> FlowVisit {
        FlowVisit {
            id: format!("v{seq}"),
            run_id: "r".into(),
            seq,
            state_id: state.into(),
            status,
            prompt: None,
            session_id: None,
            outputs: None,
            error: None,
            retried: false,
            note: None,
            started_at: 0,
            finished_at: None,
        }
    }

    #[test]
    fn the_release_pipeline_reads_top_down_with_its_parallel_rows() {
        assert_eq!(
            layout(&release()),
            [
                GraphRow::Step("plan".into()),
                GraphRow::Branches {
                    parallel: "build".into(),
                    branches: vec!["mac".into(), "win".into(), "linux".into()],
                    retries: 2,
                },
                GraphRow::Step("cut".into()),
                GraphRow::Branches {
                    parallel: "finish".into(),
                    branches: vec!["notes".into(), "mobile".into()],
                    retries: 0,
                },
                GraphRow::Step("publish".into()),
            ]
        );
        assert_eq!(
            branch_condition(&release(), "mobile").as_deref(),
            Some("if mobile/ changed")
        );
        assert_eq!(branch_condition(&release(), "notes"), None);
    }

    #[test]
    fn the_usual_path_reads_down_and_side_routes_follow_it() {
        let mut flow = release();
        // A failure route and a guarded detour are not the usual path.
        flow.states[0] = step(
            "plan",
            vec![
                Transition {
                    to: "report".into(),
                    when: Some(Guard::Status {
                        is: crate::flow::definition::VisitResult::Failed,
                    }),
                },
                to("build"),
            ],
        );
        flow.states.push(step("report", vec![to(FAILED)]));
        let rows = layout(&flow);
        assert_eq!(rows[1].cards(), ["mac", "win", "linux"]);
        assert_eq!(rows.last(), Some(&GraphRow::Step("report".into())));
        assert_eq!(rows.len(), 6);
    }

    #[test]
    fn a_loop_back_or_a_broken_flow_still_lays_out() {
        let mut flow = release();
        flow.states[9] = step("publish", vec![to("plan")]);
        assert_eq!(layout(&flow).len(), 5);
        flow.initial = "gone".into();
        // Nothing is reachable: every step still shows, in listed order.
        assert_eq!(layout(&flow)[0], GraphRow::Step("plan".into()));
    }

    #[test]
    fn cards_show_done_running_and_next_as_the_board_does() {
        let flow = release();
        let rows = layout(&flow);
        let visits = [
            visit(1, "plan", VisitStatus::Completed),
            visit(2, "build", VisitStatus::Completed),
            visit(3, "mac", VisitStatus::Completed),
            visit(4, "win", VisitStatus::Completed),
            visit(5, "linux", VisitStatus::Completed),
            visit(6, "cut", VisitStatus::Running),
        ];
        let cards = cards(&flow, &rows, &visits);
        let status = |id: &str| cards[id].status;
        assert_eq!(status("plan"), CardStatus::Done);
        assert_eq!(status("linux"), CardStatus::Done);
        assert_eq!(status("cut"), CardStatus::Running);
        assert_eq!(status("notes"), CardStatus::Next);
        // A conditional branch shows its condition instead of "next".
        assert_eq!(status("mobile"), CardStatus::Idle);
        assert_eq!(status("publish"), CardStatus::Idle);
        assert_eq!(current_row(&rows, &visits), Some(2));
    }

    #[test]
    fn a_branch_left_out_by_its_condition_is_skipped_once_its_step_ran() {
        let flow = release();
        let rows = layout(&flow);
        let mut visits = vec![
            visit(1, "plan", VisitStatus::Completed),
            visit(2, "cut", VisitStatus::Completed),
            visit(3, "finish", VisitStatus::Running),
            visit(4, "notes", VisitStatus::Running),
        ];
        let running = cards(&flow, &rows, &visits);
        assert_eq!(running["mobile"].status, CardStatus::Idle, "still deciding");
        visits[2].status = VisitStatus::Completed;
        visits[3].status = VisitStatus::Completed;
        let done = cards(&flow, &rows, &visits);
        assert_eq!(done["mobile"].status, CardStatus::Skipped);
        assert_eq!(done["mobile"].visit, None);
        assert_eq!(done["notes"].status, CardStatus::Done);
    }

    #[test]
    fn a_retried_branch_shows_its_newest_try_and_a_stopped_one_shows_stopped() {
        let flow = release();
        let rows = layout(&flow);
        let visits = [
            visit(1, "plan", VisitStatus::Completed),
            visit(2, "build", VisitStatus::Running),
            visit(3, "mac", VisitStatus::Failed),
            visit(4, "win", VisitStatus::Cancelled),
            visit(5, "linux", VisitStatus::Completed),
            visit(6, "mac", VisitStatus::Running),
        ];
        let cards = cards(&flow, &rows, &visits);
        assert_eq!(cards["mac"].status, CardStatus::Running);
        assert_eq!(cards["mac"].visit.as_ref().map(|v| v.seq), Some(6));
        assert_eq!(cards["win"].status, CardStatus::Stopped);
        assert_eq!(current_row(&rows, &visits), Some(1));
    }

    #[test]
    fn without_a_run_every_card_is_idle() {
        let flow = release();
        let rows = layout(&flow);
        let cards = cards(&flow, &rows, &[]);
        assert_eq!(cards.len(), 8, "every step but the two parallel ones");
        assert!(cards.values().all(|c| c.status == CardStatus::Idle));
        assert_eq!(current_row(&rows, &[]), None);
    }
}
