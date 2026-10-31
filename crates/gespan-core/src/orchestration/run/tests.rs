use super::super::Task;
use super::*;

fn plan(tasks: Vec<Task>, max_workers: usize) -> Plan {
    Plan {
        title: "Work".into(),
        summary: String::new(),
        tasks,
        max_workers,
    }
}
fn task(id: &str, file: &str, deps: &[&str]) -> Task {
    Task {
        id: id.into(),
        title: id.into(),
        prompt: format!("Do {id}"),
        files: vec![file.into()],
        depends_on: deps.iter().map(|s| (*s).into()).collect(),
        harness: None,
        model: None,
        model_settings: BTreeMap::new(),
    }
}
fn run(tasks: Vec<Task>, max_workers: usize) -> Run {
    Run::new(
        plan(tasks, max_workers),
        "/project".into(),
        "claude".into(),
        vec![
            Choice {
                harness: "codex".into(),
                model: "gpt-6".into(),
                name: "GPT-6".into(),
            },
            Choice {
                harness: "codex".into(),
                model: "gpt-5".into(),
                name: "GPT-5".into(),
            },
        ],
    )
}
fn id(run: &Run, title: &str) -> String {
    run.tasks
        .iter()
        .find(|t| t.title == title)
        .unwrap()
        .id
        .clone()
}
fn call(run: &mut Run, request: &str, action: &str, input: Value) -> Result<Reply, String> {
    run.control(request, action, &input, &Needs::new())
}
fn finish_turn(run: &mut Run, title: &str) -> String {
    let task = id(run, title);
    run.settle(&task, Outcome::Completed, "All done", None);
    task
}
fn approval(request: &str) -> NeedsInput {
    NeedsInput {
        kind: "approval",
        request_id: request.into(),
        label: "Run tests".into(),
        detail: Some("cargo test".into()),
        questions: None,
    }
}
fn question(request: &str) -> NeedsInput {
    NeedsInput {
        kind: "question",
        request_id: request.into(),
        label: "Which?".into(),
        detail: None,
        questions: Some(json!([
            {"id":"q1","prompt":"Which?","allowCustom":false,"options":[
                {"id":"a","label":"Option A"},{"id":"b","label":"Option B"}]},
            {"id":"q2","prompt":"Note?","allowCustom":true,"options":[]}
        ])),
    }
}

#[test]
fn dispatch_honours_the_worker_limit_scopes_and_reviewed_dependencies() {
    let mut run = run(
        vec![
            task("a", "src", &[]),
            task("b", "src/main.rs", &[]),
            task("c", "docs", &[]),
            task("d", "tests", &["a"]),
        ],
        2,
    );
    let first = run.dispatch();
    assert_eq!(first, [id(&run, "a"), id(&run, "c")]);
    assert!(run.dispatch().is_empty(), "both slots are taken");
    finish_turn(&mut run, "a");
    finish_turn(&mut run, "c");
    // A finished dependency is not enough: it needs the lead's review.
    assert_eq!(run.dispatch(), [id(&run, "b")]);
    assert!(run.dispatch().is_empty());
    let a = id(&run, "a");
    call(&mut run, "r1", "review", json!({"taskId":a})).unwrap();
    assert_eq!(run.dispatch(), [id(&run, "d")]);
}

#[test]
fn waiting_for_names_what_a_queued_task_needs() {
    let mut run = run(
        vec![
            task("a", "src", &[]),
            task("b", "src", &[]),
            task("c", "docs", &["a"]),
        ],
        1,
    );
    run.dispatch();
    let view = run.view(&Needs::new());
    assert_eq!(view["tasks"][1]["waitingFor"], "Waiting for files: a");
    assert_eq!(view["tasks"][2]["waitingFor"], "Waiting for review: a");
    assert!(view["tasks"][0].get("prompt").is_none());
}

#[test]
fn delegate_queues_a_worker_from_the_allowed_pairs_only() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    let input = json!({"title":"New","harness":"codex","prompt":"Go","files":["docs"]});
    let reply = call(&mut run, "r1", "delegate", input).unwrap();
    assert_eq!(reply.result["status"], "queued");
    let added = run.task(reply.result["taskId"].as_str().unwrap()).unwrap();
    assert_eq!(added.model.as_deref(), Some("gpt-6"), "first allowed model");
    for (input, expected) in [
        (
            json!({"title":"X","harness":"pi","prompt":"Go","files":["a"]}),
            "not allowed",
        ),
        (
            json!({"title":"X","harness":"codex","model":"nope","prompt":"Go","files":["a"]}),
            "Choose a model ID",
        ),
        (
            json!({"title":"X","harness":"codex","prompt":"Go","files":[]}),
            "at least one",
        ),
        (
            json!({"title":"X","harness":"codex","prompt":"Go","files":["../up"]}),
            "Invalid file scope",
        ),
        (
            json!({"title":"X","harness":"codex","prompt":"Go","files":["a"],"dependsOn":["gone"]}),
            "dependsOn",
        ),
        (
            json!({"title":"X","harness":"codex","prompt":"Go","files":["a"],"priority":1}),
            "Unknown delegate field: priority",
        ),
    ] {
        let error = call(&mut run, "r2", "delegate", input).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(run.tasks.len(), 2);
}

#[test]
fn the_forty_task_limit_holds() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    for ix in 0..39 {
        let input = json!({"title":"X","harness":"codex","prompt":"Go","files":[format!("f{ix}")]});
        call(&mut run, &format!("r{ix}"), "delegate", input).unwrap();
    }
    let input = json!({"title":"X","harness":"codex","prompt":"Go","files":["f"]});
    let error = call(&mut run, "last", "delegate", input).unwrap_err();
    assert!(error.contains("40-task limit"));
}

#[test]
fn a_retried_request_id_applies_once_and_cannot_change_its_input() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    let input = json!({"title":"New","harness":"codex","prompt":"Go","files":["docs"]});
    let first = call(&mut run, "r1", "delegate", input.clone()).unwrap();
    let again = call(&mut run, "r1", "delegate", input).unwrap();
    assert_eq!(first.result, again.result);
    assert_eq!(run.tasks.len(), 2, "the retry queued nothing");
    let other = json!({"title":"Other","harness":"codex","prompt":"Go","files":["docs"]});
    let error = call(&mut run, "r1", "delegate", other).unwrap_err();
    assert!(error.contains("different input"));
}

#[test]
fn a_replayed_call_never_repeats_its_effects() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    let input = json!({"taskId":id(&run, "a"),"text":"Use the other API"});
    let first = call(&mut run, "r1", "steer", input.clone()).unwrap();
    assert_eq!(first.effects.len(), 1);
    assert!(call(&mut run, "r1", "steer", input)
        .unwrap()
        .effects
        .is_empty());
}

#[test]
fn message_and_retry_send_a_stopped_worker_another_turn() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    let a = id(&run, "a");
    let error = call(&mut run, "r1", "message", json!({"taskId":a,"text":"More"})).unwrap_err();
    assert!(error.contains("Wait for this worker"), "{error}");
    run.settle(&a, Outcome::Failed, "", Some("Broke".into()));
    call(
        &mut run,
        "r2",
        "message",
        json!({"taskId":a,"text":"Fix it"}),
    )
    .unwrap();
    let task = run.task(&a).unwrap();
    assert_eq!(task.status, Status::Queued);
    assert_eq!(task.prompt, "Fix it");
    assert_eq!(task.error, None);
    run.dispatch();
    run.settle(&a, Outcome::Failed, "", None);
    let retry = json!({"taskId":a,"text":"Wider","files":["src","tests"]});
    call(&mut run, "r3", "retry", retry).unwrap();
    assert_eq!(run.task(&a).unwrap().files, ["src", "tests"]);
    let escape = json!({"taskId":a,"text":"Wider","files":["/etc"]});
    run.dispatch();
    run.settle(&a, Outcome::Failed, "", None);
    assert!(call(&mut run, "r4", "retry", escape).is_err());
}

#[test]
fn a_task_cannot_be_redone_once_a_dependant_started() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "docs", &["a"])], 4);
    run.dispatch();
    let a = finish_turn(&mut run, "a");
    call(&mut run, "r1", "review", json!({"taskId":a})).unwrap();
    run.dispatch();
    let error = call(&mut run, "r2", "message", json!({"taskId":a,"text":"Redo"})).unwrap_err();
    assert!(error.contains("dependent task"), "{error}");
}

#[test]
fn only_a_running_worker_can_be_steered() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    let a = id(&run, "a");
    let error = call(&mut run, "r1", "steer", json!({"taskId":a,"text":"x"})).unwrap_err();
    assert!(error.contains("edit it with message"), "{error}");
    run.dispatch();
    let reply = call(&mut run, "r2", "steer", json!({"taskId":a,"text":"x"})).unwrap();
    assert_eq!(
        reply.effects,
        [Effect::Steer {
            task: a.clone(),
            text: "x".into()
        }]
    );
    finish_turn(&mut run, "a");
    let error = call(&mut run, "r3", "steer", json!({"taskId":a,"text":"x"})).unwrap_err();
    assert!(error.contains("fresh turn"), "{error}");
}

#[test]
fn cancel_stops_a_running_worker_and_drops_a_queued_one() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "src", &[])], 4);
    run.dispatch();
    let (a, b) = (id(&run, "a"), id(&run, "b"));
    let reply = call(&mut run, "r1", "cancel", json!({"taskId":a})).unwrap();
    assert_eq!(reply.effects, [Effect::Stop(a.clone())]);
    assert_eq!(run.task(&a).unwrap().status, Status::Cancelling);
    assert!(run.dispatch().is_empty(), "stopping still holds its files");
    run.settle(&a, Outcome::Failed, "partial", None);
    assert_eq!(run.task(&a).unwrap().status, Status::Cancelled);
    call(&mut run, "r2", "cancel", json!({"taskId":b})).unwrap();
    assert_eq!(run.task(&b).unwrap().status, Status::Cancelled);
    assert!(run.dispatch().is_empty());
}

#[test]
fn an_approval_is_answered_only_against_the_request_it_waits_on() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    let a = id(&run, "a");
    let needs = Needs::from([(a.clone(), approval("req-7"))]);
    let respond =
        |request: &str, decision: &str| json!({"taskId":a,"requestId":request,"decision":decision});
    let error = run
        .control("r1", "respond", &respond("old", "allow"), &needs)
        .unwrap_err();
    assert!(error.contains("Stale requestId"), "{error}");
    let error = run
        .control("r2", "respond", &respond("req-7", "maybe"), &needs)
        .unwrap_err();
    assert!(error.contains("allow"), "{error}");
    let reply = run
        .control("r3", "respond", &respond("req-7", "deny"), &needs)
        .unwrap();
    assert_eq!(
        reply.effects,
        [Effect::Approve {
            task: a.clone(),
            request: "req-7".into(),
            allow: false
        }]
    );
    let error = call(&mut run, "r4", "respond", respond("req-7", "allow")).unwrap_err();
    assert!(error.contains("not waiting"), "{error}");
}

#[test]
fn a_question_is_answered_by_option_id_and_may_be_skipped() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    let a = id(&run, "a");
    let needs = Needs::from([(a.clone(), question("q-1"))]);
    let answer = |request: &str, answers: Value| run_answer(&a, request, answers);
    let reply = run
        .control(
            "r1",
            "answer",
            &answer("q-1", json!({"q1":["b"],"q2":["free text"]})),
            &needs,
        )
        .unwrap();
    assert_eq!(
        reply.effects,
        [Effect::Answer {
            task: a.clone(),
            request: "q-1".into(),
            answers: Some(json!([["Option B"], ["free text"]])),
        }]
    );
    for (answers, expected) in [
        (json!({"q9":["a"]}), "Unknown question"),
        (json!({"q1":["z"]}), "Unknown option"),
        (json!({}), "at least one"),
        (json!("a"), "answers must be"),
    ] {
        let error = run
            .control("r2", "answer", &answer("q-1", answers), &needs)
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    let skip = json!({"taskId":a,"requestId":"q-1","skip":true});
    let reply = run.control("r3", "answer", &skip, &needs).unwrap();
    assert_eq!(reply.result["answered"], false);
    assert!(matches!(
        &reply.effects[0],
        Effect::Answer { answers: None, .. }
    ));
}
fn run_answer(task: &str, request: &str, answers: Value) -> Value {
    json!({"taskId":task,"requestId":request,"answers":answers})
}

#[test]
fn review_accepts_only_a_completed_turn() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    let a = id(&run, "a");
    let error = call(&mut run, "r1", "review", json!({"taskId":a})).unwrap_err();
    assert!(error.contains("Wait for it to finish"), "{error}");
    run.dispatch();
    run.settle(&a, Outcome::Failed, "", None);
    let error = call(&mut run, "r2", "review", json!({"taskId":a})).unwrap_err();
    assert!(error.contains("message"), "{error}");
}

#[test]
fn finish_needs_every_task_accepted_or_cancelled() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "docs", &[])], 4);
    run.dispatch();
    let a = finish_turn(&mut run, "a");
    let b = id(&run, "b");
    call(&mut run, "r1", "review", json!({"taskId":a})).unwrap();
    let error = call(&mut run, "r2", "finish", json!({})).unwrap_err();
    assert!(error.contains("b (running)"), "{error}");
    call(&mut run, "r3", "cancel", json!({"taskId":b})).unwrap();
    run.settle(&b, Outcome::Failed, "", None);
    call(&mut run, "r4", "finish", json!({})).unwrap();
    assert_eq!(run.status, RunStatus::Finished);
    assert!(!run.live());
    let error = call(&mut run, "r5", "cancel", json!({"taskId":a})).unwrap_err();
    assert!(error.contains("This run is finished"), "{error}");
    assert!(call(&mut run, "r6", "list", json!({})).is_ok());
}

#[test]
fn the_idle_lead_hears_each_result_and_each_block_once() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "docs", &[])], 4);
    run.dispatch();
    let mut announced = HashSet::new();
    assert_eq!(run.wake(&Needs::new(), &mut announced), Wake::Nothing);
    let a = finish_turn(&mut run, "a");
    let b = id(&run, "b");
    let needs = Needs::from([(b.clone(), approval("req-1"))]);
    let Wake::Send(body) = run.wake(&needs, &mut announced) else {
        panic!("the lead has news")
    };
    assert!(body.contains(&format!("{a} — a: completed")), "{body}");
    assert!(body.contains("All done"), "{body}");
    assert!(body.contains("(requestId req-1)"), "{body}");
    assert!(body.contains("cargo test"), "{body}");
    assert_eq!(run.wake(&needs, &mut announced), Wake::Nothing);
    let next = Needs::from([(b, approval("req-2"))]);
    assert!(matches!(run.wake(&next, &mut announced), Wake::Send(_)));
}

#[test]
fn twenty_automatic_turns_pause_the_run_and_stop_its_agents() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "docs", &[])], 4);
    run.dispatch();
    run.continuations = 20;
    let a = finish_turn(&mut run, "a");
    let b = id(&run, "b");
    let Wake::Paused(effects) = run.wake(&Needs::new(), &mut HashSet::new()) else {
        panic!("the limit pauses")
    };
    assert_eq!(effects, [Effect::Stop(b.clone())]);
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.task(&b).unwrap().status, Status::Interrupted);
    assert!(!run.task(&a).unwrap().delivered, "kept for after Resume");
}

#[test]
fn a_paused_run_stays_readable_and_refuses_changes_with_the_reason() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "src", &[])], 4);
    run.dispatch();
    let (a, b) = (id(&run, "a"), id(&run, "b"));
    assert_eq!(run.pause("The lead stopped"), [Effect::Stop(a.clone())]);
    assert_eq!(run.task(&b).unwrap().status, Status::Queued);
    assert!(run.task(&a).unwrap().recovery_prompt.is_some());
    assert!(run.dispatch().is_empty());
    let error = call(&mut run, "r1", "cancel", json!({"taskId":b})).unwrap_err();
    assert!(error.contains("The lead stopped"), "{error}");
    assert!(error.contains("click Resume"), "{error}");
    let got = call(&mut run, "r2", "get", json!({"taskId":a})).unwrap();
    assert_eq!(got.result["runStatus"], "paused");
    assert!(got.result["recovery"].as_str().unwrap().contains("Resume"));
    assert!(
        run.requests.is_empty(),
        "reads and refusals keep no receipt"
    );
    assert!(!run.should_wait(&Needs::new()));
}

#[test]
fn stop_cancels_running_and_queued_work_and_stops_the_lead() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "src", &[])], 4);
    run.dispatch();
    let a = id(&run, "a");
    assert_eq!(run.stop(), [Effect::StopLead, Effect::Stop(a)]);
    assert!(run.tasks.iter().all(|t| t.status == Status::Cancelled));
    assert!(!run.live());
}

#[test]
fn wait_blocks_only_while_workers_can_progress_alone() {
    assert_eq!(wait_seconds(&json!({})), Ok(20));
    assert_eq!(wait_seconds(&json!({"timeoutSeconds":0})), Ok(0));
    assert!(wait_seconds(&json!({"timeoutSeconds":26})).is_err());
    assert!(wait_seconds(&json!({"timeoutSeconds":"5"})).is_err());
    assert!(wait_seconds(&json!({"seconds":5})).is_err());
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    assert!(run.should_wait(&Needs::new()));
    let a = id(&run, "a");
    assert!(!run.should_wait(&Needs::from([(a, approval("r"))])));
    finish_turn(&mut run, "a");
    assert!(!run.should_wait(&Needs::new()), "nothing left to wait for");
}

#[test]
fn a_late_settle_from_an_old_turn_changes_nothing() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    let a = finish_turn(&mut run, "a");
    call(&mut run, "r1", "review", json!({"taskId":a})).unwrap();
    run.settle(&a, Outcome::Failed, "late", None);
    let task = run.task(&a).unwrap();
    assert!(task.accepted);
    assert_eq!(task.status, Status::Completed);
    assert_eq!(task.result, "All done");
}

#[test]
fn the_summary_carries_no_prompts_and_a_run_round_trips() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    let summary = serde_json::to_string(&run.summary()).unwrap();
    assert!(!summary.contains("Do a"));
    assert!(run.summary().live);
    let saved = serde_json::to_string(&run).unwrap();
    let restored: Run = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored, run);
}

#[test]
fn a_restart_pauses_active_work_and_keeps_what_it_can() {
    let mut run = run(
        vec![
            task("done", "src", &[]),
            task("review", "docs", &[]),
            task("running", "tests", &[]),
            task("waiting", "tests", &[]),
        ],
        4,
    );
    run.dispatch();
    let done = finish_turn(&mut run, "done");
    finish_turn(&mut run, "review");
    call(&mut run, "r1", "review", json!({"taskId":done})).unwrap();
    let running = id(&run, "running");
    let mut restored: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    restored.hydrate();
    assert_eq!(restored.status, RunStatus::Paused);
    assert!(restored.summary().paused);
    let task = restored.task(&running).unwrap();
    assert_eq!(task.status, Status::Interrupted);
    assert!(task.recovery_prompt.is_some());
    assert!(restored.tasks[0].accepted, "accepted work stays accepted");
    let unread = restored.task(&id(&run, "review")).unwrap();
    assert!(!unread.delivered, "the lead hears it again");
    assert_eq!(restored.tasks[3].status, Status::Queued);
    assert!(restored.requests.contains_key("r1"), "receipts survive");
    assert!(restored.dispatch().is_empty());
}

#[test]
fn hydrating_a_paused_run_keeps_its_reason() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    run.dispatch();
    run.pause("Saved run failed");
    run.hydrate();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.error.as_deref(), Some("Saved run failed"));
}

#[test]
fn resume_continues_interrupted_workers_and_leaves_blocked_ones_stopped() {
    let mut run = run(vec![task("a", "src", &[]), task("b", "docs", &[])], 4);
    run.dispatch();
    let (a, b) = (id(&run, "a"), id(&run, "b"));
    run.task_mut(&b).unwrap().status = Status::Blocked;
    run.continuations = 20;
    run.pause("The lead stopped");
    run.resume().unwrap();
    assert_eq!(run.status, RunStatus::Active);
    assert_eq!(run.continuations, 0);
    assert_eq!(run.error, None);
    assert_eq!(run.last_pause_reason.as_deref(), Some("The lead stopped"));
    assert_eq!(run.task(&b).unwrap().status, Status::Blocked);
    assert_eq!(run.dispatch(), std::slice::from_ref(&a));
    let task = run.task(&a).unwrap();
    assert!(
        task.recovery_prompt.is_some(),
        "the worker gets the recovery turn"
    );
    run.settle(&a, Outcome::Completed, "Done", None);
    assert_eq!(run.task(&a).unwrap().recovery_prompt, None);
    let Wake::Send(body) = run.wake(&Needs::new(), &mut HashSet::new()) else {
        panic!("the result reaches the lead")
    };
    assert!(
        body.contains("Previous interruption: The lead stopped"),
        "{body}"
    );
}

#[test]
fn only_a_paused_run_resumes() {
    let mut run = run(vec![task("a", "src", &[])], 4);
    assert!(run.resume().is_err());
    run.stop();
    assert!(run.resume().is_err());
}

#[test]
fn a_finished_worker_with_no_changes_is_not_ready_for_review() {
    let mut run = run(vec![task("a", "a.md", &[]), task("b", "b.md", &[])], 2);
    run.dispatch();
    let (a, b) = (id(&run, "a"), id(&run, "b"));
    run.settle(&a, Outcome::Completed, "Wrote it", None);
    run.settle(&b, Outcome::Completed, "My write was rejected", None);
    run.mark_no_changes(&b);

    assert_eq!(
        run.progress(),
        "a: ready for review\nb: finished with no changes"
    );
    let summary = run.summary();
    assert_eq!(summary.tasks[0].status, "completed");
    assert_eq!(summary.tasks[1].status, "unchanged");
    assert_eq!(summary.tasks[1].label(&summary, false), "No changes");
}

#[test]
fn no_changes_only_applies_to_a_finished_worker_and_clears_on_its_next_turn() {
    let mut run = run(vec![task("a", "a.md", &[])], 1);
    let a = id(&run, "a");
    run.mark_no_changes(&a);
    run.dispatch();
    assert!(
        run.tasks[0].status.active(),
        "a queued or running task is left alone"
    );
    assert!(!run.tasks[0].no_changes);

    run.settle(&a, Outcome::Failed, "", Some("Broke".into()));
    run.mark_no_changes(&a);
    assert_eq!(
        run.summary().tasks[0].status,
        "failed",
        "a failure stays a failure"
    );

    run.tasks[0].status = Status::Completed;
    run.mark_no_changes(&a);
    run.tasks[0].status = Status::Running;
    run.settle(&a, Outcome::Completed, "Second try", None);
    assert_eq!(run.summary().tasks[0].status, "completed");
}
