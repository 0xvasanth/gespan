use super::*;
use rebase_core::flow::{
    AgentState, FlowInput, FlowState, OutputField, OutputType, RunStatus, Transition, VisitStatus,
};
use std::cell::Cell;

// ---- fake driver -------------------------------------------------------

/// Records what the shell would do. Sessions are numbered from 100, and a
/// follow-up reports the generation the fake session is at.
#[derive(Default)]
struct Driver {
    launches: Vec<LaunchRequest>,
    follow_ups: Vec<(i64, String)>,
    stopped: Vec<i64>,
    persisted: Vec<(FlowRun, Option<FlowVisit>)>,
    changed: Vec<String>,
    closed: Vec<i64>,
    generation: u64,
}

impl SessionDriver for Driver {
    fn launch(&mut self, request: LaunchRequest) {
        self.launches.push(request);
    }
    fn follow_up(&mut self, session: i64, prompt: String) -> Option<u64> {
        if self.closed.contains(&session) {
            return None;
        }
        self.follow_ups.push((session, prompt));
        Some(self.generation)
    }
    fn stop(&mut self, session: i64) {
        self.stopped.push(session);
    }
    fn persist(&mut self, run: FlowRun, visit: Option<FlowVisit>) {
        self.persisted.push((run, visit));
    }
    fn changed(&mut self, run_id: &str) {
        self.changed.push(run_id.to_owned());
    }
}

impl Driver {
    fn last_run(&self) -> FlowRun {
        self.persisted.last().expect("a persisted run").0.clone()
    }
    /// The latest persisted copy of every visit, in seq order.
    fn visits(&self) -> Vec<FlowVisit> {
        let mut latest: Vec<FlowVisit> = Vec::new();
        for visit in self.persisted.iter().filter_map(|(_, v)| v.clone()) {
            match latest.iter_mut().find(|x| x.id == visit.id) {
                Some(slot) => *slot = visit,
                None => latest.push(visit),
            }
        }
        latest.sort_by_key(|v| v.seq);
        latest
    }
}

fn host() -> Host {
    let tick = Cell::new(1000_i64);
    let mut n = 0;
    Host::new(Runner::new(
        move || {
            tick.set(tick.get() + 1);
            tick.get()
        },
        move || {
            n += 1;
            format!("id{n}")
        },
    ))
}

// ---- flow and turn builders ---------------------------------------------

fn agent(id: &str, outputs: Vec<OutputField>, to: &str) -> FlowState {
    FlowState::Agent(AgentState {
        id: id.into(),
        title: id.to_uppercase(),
        harness: "claude".into(),
        model: "claude:sonnet-5".into(),
        brief: format!("{id} {{{{inputs.issue}}}}"),
        outputs,
        transitions: vec![Transition {
            to: to.into(),
            when: None,
        }],
        max_visits: 3,
    })
}

/// `plan` (outputs a summary) then `build` (no outputs) then success.
fn two_states() -> Flow {
    Flow {
        id: "flow1".into(),
        name: "Fix".into(),
        description: String::new(),
        cwd: "/repo".into(),
        inputs: vec![FlowInput {
            name: "issue".into(),
            label: "Issue".into(),
            required: true,
            default: None,
        }],
        initial: "plan".into(),
        states: vec![
            agent(
                "plan",
                vec![OutputField {
                    name: "summary".into(),
                    r#type: OutputType::String,
                    description: "the plan".into(),
                }],
                "build",
            ),
            agent("build", vec![], "$succeeded"),
        ],
        created_at: 0,
        updated_at: 0,
    }
}

fn start(host: &mut Host, driver: &mut Driver) -> FlowRun {
    let inputs = HashMap::from([("issue".to_string(), "42".to_string())]);
    host.start(driver, &two_states(), &inputs).expect("start")
}

/// Reports the last launch's session as created and its prompt submitted.
fn launch(host: &mut Host, driver: &mut Driver, session: i64) {
    let request = driver.launches.last().unwrap().clone();
    let before = driver.generation;
    assert!(host.launched(driver, &request.run_id, &request.visit_id, session, before));
    host.submitted(session);
    assert_eq!(host.take_rechecks(), vec![session], "probe once submitted");
    driver.generation += 1;
}

/// Reports `session`'s turn as finished at the driver's current generation.
fn finish(host: &mut Host, driver: &mut Driver, session: i64, settled: Settled) {
    let probe = done(driver.generation);
    host.settle(driver, session, probe, settled);
}

fn done(generation: u64) -> Probe {
    Probe {
        pending: false,
        generation,
    }
}

fn reply(text: &str, worktree: Option<&str>) -> Settled {
    Settled {
        ok: true,
        message: "Ready".into(),
        text: text.into(),
        worktree: worktree.map(str::to_owned),
        ..Default::default()
    }
}

const PLAN: &str = "Planned.\n```json\n{\"summary\": \"patch it\"}\n```";

// ---- tests ---------------------------------------------------------------

#[test]
fn two_state_flow_hands_the_first_worktree_to_the_next_visit_and_succeeds() {
    let (mut host, mut driver) = (host(), Driver::default());
    let run = start(&mut host, &mut driver);
    let first = driver.launches[0].clone();
    assert_eq!(first.worktree_cwd, None, "the first visit makes a worktree");
    assert_eq!(first.title, "Fix · PLAN");
    assert!(first.prompt.starts_with("plan 42"));

    launch(&mut host, &mut driver, 100);
    finish(
        &mut host,
        &mut driver,
        100,
        reply(PLAN, Some("/repo/.worktrees/flow")),
    );
    assert_eq!(driver.launches.len(), 2);
    let second = &driver.launches[1];
    assert_eq!(
        second.worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow")
    );
    assert_eq!(second.title, "Fix · BUILD");

    launch(&mut host, &mut driver, 101);
    finish(
        &mut host,
        &mut driver,
        101,
        reply("Built.", Some("/repo/.worktrees/flow")),
    );
    let finished = driver.last_run();
    assert_eq!(finished.id, run.id);
    assert_eq!(finished.status, RunStatus::Succeeded);
    assert_eq!(
        finished.worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow")
    );
    let visits = driver.visits();
    assert_eq!(visits.len(), 2);
    assert_eq!(visits[0].session_id.as_deref(), Some("100"));
    assert_eq!(visits[1].session_id.as_deref(), Some("101"));
    assert!(visits.iter().all(|v| v.status == VisitStatus::Completed));
    assert_eq!(
        visits[0].outputs.as_ref().unwrap()["summary"],
        rebase_core::flow::OutputValue::String("patch it".into())
    );
    assert_eq!(driver.changed, vec![run.id]);
    assert!(!host.awaits(100) && !host.awaits(101));
}

#[test]
fn each_turn_settles_once_and_the_contract_retry_rearms_its_session() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    let probe = done(driver.generation);
    host.settle(&mut driver, 100, probe, reply("No block here.", None));
    assert_eq!(driver.follow_ups.len(), 1, "one retry in the same session");
    assert_eq!(driver.follow_ups[0].0, 100);
    assert!(
        driver.follow_ups[0]
            .1
            .starts_with("Your JSON block was invalid")
    );
    assert_eq!(host.take_rechecks(), vec![100], "probe after the follow-up");

    // The retry turn settles on its own and moves on.
    driver.generation += 1;
    let retry = done(driver.generation);
    host.settle(&mut driver, 100, retry, reply(PLAN, None));
    assert_eq!(driver.launches.len(), 2, "the retry's block moves on");
    let visits = driver.visits();
    assert!(visits[0].retried);
    assert_eq!(visits[0].status, VisitStatus::Completed);

    // Further reports of that finished turn are not a second settle.
    let persisted = driver.persisted.len();
    host.settle(&mut driver, 100, retry, reply(PLAN, None));
    host.settle(&mut driver, 100, retry, reply("No block here.", None));
    assert_eq!(driver.persisted.len(), persisted);
    assert_eq!(driver.follow_ups.len(), 1);
    assert_eq!(driver.launches.len(), 2);
}

#[test]
fn pending_or_submitting_sessions_do_not_settle() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    let request = driver.launches[0].clone();
    assert!(host.launched(&mut driver, &request.run_id, &request.visit_id, 100, 0));
    // Mid-submit: the runner must not see the transitional state.
    assert!(!host.ready(100, done(1)));
    host.settle(&mut driver, 100, done(1), reply(PLAN, None));
    host.submitted(100);
    let busy = Probe {
        pending: true,
        generation: 1,
    };
    assert!(!host.ready(100, busy));
    host.settle(&mut driver, 100, busy, reply(PLAN, None));
    assert_eq!(driver.launches.len(), 1);
    assert!(host.ready(100, done(1)));
    assert!(!host.ready(999, done(1)), "not a Flow session");
}

#[test]
fn a_turn_that_never_started_fails_the_visit_instead_of_hanging() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    let request = driver.launches[0].clone();
    host.launched(&mut driver, &request.run_id, &request.visit_id, 100, 7);
    host.submitted(100);
    host.settle(&mut driver, 100, done(7), reply(PLAN, None));
    let run = driver.last_run();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(driver.visits()[0].error.as_deref(), Some(NOT_STARTED));
}

#[test]
fn a_failed_turn_fails_the_visit_with_the_session_message() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    let settled = Settled {
        ok: false,
        message: "Codex exited\nboom".into(),
        ..Default::default()
    };
    finish(&mut host, &mut driver, 100, settled);
    assert_eq!(driver.last_run().status, RunStatus::Failed);
    assert_eq!(
        driver.visits()[0].error.as_deref(),
        Some("Codex exited\nboom")
    );
}

#[test]
fn cancel_stops_the_session_and_ignores_its_late_settle() {
    let (mut host, mut driver) = (host(), Driver::default());
    let run = start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    host.cancel(&mut driver, &run.id);
    assert_eq!(driver.stopped, vec![100]);
    assert_eq!(driver.last_run().status, RunStatus::Cancelled);
    assert_eq!(driver.visits()[0].status, VisitStatus::Cancelled);
    assert!(!host.awaits(100));

    // The stopped turn's own SessionChanged must not move the run.
    let persisted = driver.persisted.len();
    let stopped = Settled {
        cancelled: true,
        ..Default::default()
    };
    finish(&mut host, &mut driver, 100, stopped);
    assert_eq!(driver.persisted.len(), persisted);
    assert_eq!(driver.changed, vec![run.id]);
}

#[test]
fn a_session_stopped_by_the_user_cancels_the_run() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    let stopped = Settled {
        cancelled: true,
        ..Default::default()
    };
    finish(&mut host, &mut driver, 100, stopped);
    assert_eq!(driver.last_run().status, RunStatus::Cancelled);
    assert!(driver.stopped.is_empty(), "the turn has already stopped");
}

#[test]
fn a_visit_cancelled_while_its_session_is_created_gets_no_prompt() {
    let (mut host, mut driver) = (host(), Driver::default());
    let run = start(&mut host, &mut driver);
    let request = driver.launches[0].clone();
    host.cancel(&mut driver, &run.id);
    assert!(driver.stopped.is_empty(), "no session to stop yet");
    let submit = host.launched(&mut driver, &request.run_id, &request.visit_id, 100, 0);
    assert!(!submit);
    assert!(driver.stopped.is_empty(), "nothing was submitted to stop");
    assert!(!host.awaits(100));
}

#[test]
fn a_launch_failure_fails_the_visit_without_a_session() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    let request = driver.launches[0].clone();
    host.launch_failed(
        &mut driver,
        &request.run_id,
        &request.visit_id,
        "The Flow's project folder is unavailable".into(),
    );
    let run = driver.last_run();
    assert_eq!(run.status, RunStatus::Failed);
    let visit = &driver.visits()[0];
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.session_id, None);
    assert_eq!(
        visit.error.as_deref(),
        Some("The Flow's project folder is unavailable")
    );
}

#[test]
fn a_retry_to_a_closed_session_fails_the_visit() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    driver.closed.push(100);
    finish(&mut host, &mut driver, 100, reply("No block.", None));
    assert_eq!(driver.last_run().status, RunStatus::Failed);
    assert_eq!(driver.visits()[0].error.as_deref(), Some(SESSION_GONE));
    assert!(!host.awaits(100));
}

#[test]
fn startup_recovery_interrupts_a_running_visit() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    let run = driver.last_run();
    let visits = driver.visits();

    let (mut restarted, mut after) = (self::host(), Driver::default());
    restarted.recover(&mut after, vec![(run.clone(), visits)]);
    let recovered = after.last_run();
    assert_eq!(recovered.status, RunStatus::Failed);
    assert_eq!(
        recovered.error.as_deref(),
        Some("Interrupted by app restart")
    );
    let visit = &after.visits()[0];
    assert_eq!(visit.status, VisitStatus::Failed);
    assert_eq!(visit.error.as_deref(), Some("Interrupted by app restart"));
    assert_eq!(after.changed, vec![run.id]);
    assert!(after.launches.is_empty());
}

#[test]
fn a_reply_over_12000_chars_still_parses_its_trailing_json_block() {
    let rows = |text: String| {
        vec![
            Row {
                role: "user".into(),
                text: "plan 42".into(),
                first: true,
                sent_at: None,
            },
            Row {
                role: "assistant".into(),
                text,
                first: true,
                sent_at: None,
            },
        ]
    };
    let long = format!("{}\n\n{PLAN}", "Analysis. ".repeat(1_500));
    assert!(long.len() > 12_000);
    let rows = rows(long);
    // The excerpt automations read would have cut the block off.
    assert!(
        !rebase_core::response::limit(&rebase_core::response::prose(&rows), 12_000)
            .contains("patch it")
    );

    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    let settled = Settled::read(&rows, (true, "Ready".into()), false, None);
    finish(&mut host, &mut driver, 100, settled);
    assert!(driver.follow_ups.is_empty(), "no contract retry");
    let visit = &driver.visits()[0];
    assert_eq!(visit.status, VisitStatus::Completed);
    assert_eq!(
        visit.outputs.as_ref().unwrap()["summary"],
        rebase_core::flow::OutputValue::String("patch it".into())
    );
}

#[test]
fn closing_a_running_visits_session_fails_the_visit_and_ignores_its_late_settle() {
    let (mut host, mut driver) = (host(), Driver::default());
    let run = start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    host.session_closed(&mut driver, 100);
    let visits = driver.visits();
    assert_eq!(visits[0].status, VisitStatus::Failed);
    assert_eq!(visits[0].error.as_deref(), Some(SESSION_CLOSED));
    assert_eq!(driver.last_run().status, RunStatus::Failed);
    assert_eq!(driver.changed, vec![run.id]);
    assert!(!host.awaits(100));
    // The stopped turn reports in afterwards; the run has already moved on.
    let persisted = driver.persisted.len();
    finish(&mut host, &mut driver, 100, reply(PLAN, None));
    assert_eq!(driver.persisted.len(), persisted);
    assert_eq!(driver.launches.len(), 1);
}

#[test]
fn closing_a_session_while_its_prompt_is_submitting_fails_the_visit() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    let request = driver.launches[0].clone();
    assert!(host.launched(&mut driver, &request.run_id, &request.visit_id, 100, 0));
    host.session_closed(&mut driver, 100);
    assert_eq!(driver.visits()[0].status, VisitStatus::Failed);
    assert_eq!(driver.visits()[0].error.as_deref(), Some(SESSION_CLOSED));
}

#[test]
fn closing_a_session_that_is_not_a_running_visit_changes_nothing() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    finish(&mut host, &mut driver, 100, reply(PLAN, Some("/wt")));
    // Visit 1 completed; session 100 no longer has a turn in flight.
    let persisted = driver.persisted.len();
    host.session_closed(&mut driver, 100);
    host.session_closed(&mut driver, 555);
    assert_eq!(driver.persisted.len(), persisted);
    assert_eq!(driver.visits()[0].status, VisitStatus::Completed);
    assert_eq!(driver.visits()[1].status, VisitStatus::Running);
}

#[test]
fn a_worktree_reported_mid_turn_survives_cancel_and_resume_reuses_it() {
    let (mut host, mut driver) = (host(), Driver::default());
    let run = start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    // The conversation moves into its new worktree while the turn runs.
    host.worktree(&mut driver, 100, "/repo/.worktrees/flow");
    assert_eq!(
        driver.last_run().worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow")
    );
    // A repeat report from the next SessionChanged writes nothing new.
    let persisted = driver.persisted.len();
    host.worktree(&mut driver, 100, "/repo/.worktrees/flow");
    host.worktree(&mut driver, 100, "/elsewhere");
    assert_eq!(driver.persisted.len(), persisted);
    // Sessions that are not running a visit report nothing.
    host.worktree(&mut driver, 555, "/elsewhere");
    assert_eq!(driver.persisted.len(), persisted);

    host.cancel(&mut driver, &run.id);
    let cancelled = driver.last_run();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert_eq!(
        cancelled.worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow")
    );

    // Resume, as the shell does, from the stored run and visits.
    let visits = driver.visits();
    assert!(host.resume(&mut driver, cancelled, visits, "plan"));
    let resumed = driver.launches.last().unwrap();
    assert_eq!(driver.launches.len(), 2);
    assert_eq!(
        resumed.worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow"),
        "the resumed visit reuses the worktree"
    );
}

#[test]
fn a_recovered_run_without_a_worktree_resumes_in_its_visit_sessions_worktree() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    launch(&mut host, &mut driver, 100);
    let (run, visits) = (driver.last_run(), driver.visits());
    assert_eq!(run.worktree_cwd, None, "the restart cut the turn off first");

    let (mut restarted, mut after) = (self::host(), Driver::default());
    restarted.recover(&mut after, vec![(run, visits)]);
    let (mut run, visits) = (after.last_run(), after.visits());
    assert_eq!(run.status, RunStatus::Failed);

    // The session's saved settings still name the worktree it moved into.
    let mut asked = Vec::new();
    fill_worktree(&mut run, &visits, |session| {
        asked.push(session);
        (session == 100).then(|| "/repo/.worktrees/flow".to_string())
    });
    assert_eq!(asked, vec![100]);
    assert_eq!(run.worktree_cwd.as_deref(), Some("/repo/.worktrees/flow"));

    assert!(restarted.resume(&mut after, run, visits, "plan"));
    let resumed = after.launches.last().unwrap();
    assert_eq!(
        resumed.worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow")
    );
    assert_eq!(
        after.last_run().worktree_cwd.as_deref(),
        Some("/repo/.worktrees/flow"),
        "the resumed run records it"
    );
}

#[test]
fn filling_a_worktree_keeps_a_known_one_and_takes_the_latest_session_that_has_one() {
    let visit = |seq: u32, session: Option<&str>| FlowVisit {
        id: format!("v{seq}"),
        run_id: "r1".into(),
        seq,
        state_id: "plan".into(),
        status: VisitStatus::Failed,
        prompt: None,
        session_id: session.map(str::to_owned),
        outputs: None,
        error: None,
        retried: false,
        note: None,
        started_at: 0,
        finished_at: None,
    };
    let mut host = host();
    let mut driver = Driver::default();
    let mut run = start(&mut host, &mut driver);
    // Seq 3 failed at launch with no session; seq 2's session has none saved.
    let visits = vec![visit(3, None), visit(1, Some("100")), visit(2, Some("101"))];
    let mut asked = Vec::new();
    fill_worktree(&mut run, &visits, |session| {
        asked.push(session);
        (session == 100).then(|| "/wt-100".to_string())
    });
    assert_eq!(asked, vec![101, 100], "newest session first");
    assert_eq!(run.worktree_cwd.as_deref(), Some("/wt-100"));

    // A run that already knows its worktree is left alone.
    run.worktree_cwd = Some("/known".into());
    fill_worktree(&mut run, &visits, |_| panic!("not asked"));
    assert_eq!(run.worktree_cwd.as_deref(), Some("/known"));

    // Nothing saved anywhere: still none.
    run.worktree_cwd = None;
    fill_worktree(&mut run, &visits, |_| None);
    assert_eq!(run.worktree_cwd, None);
}

#[test]
fn a_stop_before_the_turn_starts_reports_cancelled_not_a_start_failure() {
    let (mut host, mut driver) = (host(), Driver::default());
    start(&mut host, &mut driver);
    let request = driver.launches[0].clone();
    assert!(host.launched(&mut driver, &request.run_id, &request.visit_id, 100, 7));
    host.submitted(100);
    // The generation never moved: the user stopped it before it started.
    let stopped = Settled {
        cancelled: true,
        ..Default::default()
    };
    host.settle(&mut driver, 100, done(7), stopped);
    assert_eq!(driver.last_run().status, RunStatus::Cancelled);
    let visit = &driver.visits()[0];
    assert_eq!(visit.status, VisitStatus::Cancelled);
    assert_eq!(visit.error, None);
}

#[test]
fn has_active_covers_a_run_until_it_finishes() {
    let (mut host, mut driver) = (host(), Driver::default());
    assert!(!host.has_active("flow1"));
    let run = start(&mut host, &mut driver);
    assert!(host.has_active("flow1"));
    assert!(!host.has_active("flow2"));
    host.cancel(&mut driver, &run.id);
    assert!(!host.has_active("flow1"));
}

#[test]
fn run_of_names_the_run_of_a_session_with_a_turn_in_flight() {
    let (mut host, mut driver) = (host(), Driver::default());
    let run = start(&mut host, &mut driver);
    assert_eq!(host.run_of(100), None, "not launched yet");
    launch(&mut host, &mut driver, 100);
    assert_eq!(host.run_of(100), Some(run.id.as_str()));
    assert_eq!(host.run_of(555), None);
    finish(&mut host, &mut driver, 100, reply(PLAN, Some("/wt")));
    assert_eq!(host.run_of(100), None, "its visit settled");
}
