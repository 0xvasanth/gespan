//! A finished pending-review run: Gespan leaves the findings as a draft on
//! the pull request through its own forge call, then My tasks asks whether
//! to submit it or keep it as a draft. The forge is a fake, so no test
//! reaches `gh` or GitLab.
use super::automation_runs::Running;
use super::review_drafts::{DraftForge, DraftJob};
use super::*;
use gespan_core::automations::review_setup::{ReviewSetupInput, WhichPrs, review_draft};
use gespan_core::automations::{AutomationEventClaim, Service};
use gespan_core::review_draft::{DraftReview, Handle, ShownComment, diff_map::Side};
use gespan_core::review_policy::{Forge, ReviewPolicy, ReviewResults, ReviewTarget};
use gespan_core::store::Store;
use gespan_core::tasks::Kind;
use gpui::{TestAppContext, WindowHandle};
use std::path::PathBuf;

const KEY: &str = "github:pull_request_review_command:acme/app:7:c1";
const REPLY: &str = "Verdict: fix\n```gespan-review\n{\"summary\":\"Verdict: fix\",\"comments\":[{\"path\":\"src/lib.rs\",\"line\":3,\"body\":\"Off by one\"}]}\n```";

struct Rig {
    _dir: tempfile::TempDir,
    db: PathBuf,
    project: String,
    window: WindowHandle<Shell>,
    automation: String,
    run: String,
    jobs: Arc<Mutex<Vec<String>>>,
}

fn draft() -> DraftReview {
    DraftReview {
        forge: Forge::GitHub,
        repo: "acme/app".into(),
        number: 7,
        url: "https://github.com/acme/app/pull/7#pullrequestreview-42".into(),
        handle: Handle::Github { review_id: 42 },
        body: "Verdict: fix".into(),
        inline: vec![ShownComment {
            path: "src/lib.rs".into(),
            side: Side::New,
            line: 3,
            body: "Off by one".into(),
        }],
        outside: 0,
    }
}

/// A forge that records each job and answers `create`.
fn fake_forge(jobs: Arc<Mutex<Vec<String>>>, create: Result<DraftReview, String>) -> DraftForge {
    Arc::new(move |job| {
        let mut jobs = jobs.lock().unwrap();
        match job {
            DraftJob::Create { target, reply, .. } => {
                jobs.push(format!(
                    "create {}:{}#{} reply={}",
                    target.forge.id(),
                    target.repo,
                    target.number,
                    reply == REPLY
                ));
                create.clone().map(Some)
            }
            DraftJob::Submit { draft, .. } => {
                jobs.push(format!("submit {:?}", draft.handle));
                Ok(None)
            }
        }
    })
}

fn rig(cx: &mut TestAppContext, create: Result<DraftReview, String>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let project = project.to_string_lossy().into_owned();
    let db = dir.path().join("gespan.db");
    let mut upsert = review_draft(&ReviewSetupInput {
        cwd: project.clone(),
        repo: Some("acme/app".into()),
        which: WhichPrs::CommentCommand,
        harness: "codex".into(),
        model: String::new(),
        results: ReviewResults::Pending,
    });
    // Saved disabled so nothing else starts while the test drives the run.
    upsert.enabled = false;
    let mut service = Service::open(&db).unwrap();
    let automation = service.upsert(upsert).unwrap().id;
    service.set_enabled(&automation, true).unwrap();
    let run = service
        .claim_event(
            &automation,
            AutomationEventClaim {
                event_key: KEY.into(),
                event_kind: "github".into(),
                event: "pull_request_review_command".into(),
                scheduled_for: 0,
                prompt: "Review".into(),
                title: "acme/app#7 Fix the login".into(),
            },
            gespan_core::time::now_ms() as i64,
        )
        .unwrap()
        .unwrap()
        .run
        .id;
    drop(service);
    let store = Arc::new(Mutex::new(Store::open(&db).unwrap()));
    cx.update(crate::shell::bind_keys);
    cx.update(|cx| cx.set_reduce_motion(true));
    let cwd = project.clone();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(store, vec![], cwd, None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let jobs = Arc::new(Mutex::new(Vec::new()));
    let forge = fake_forge(jobs.clone(), create);
    window
        .update(cx, |shell, _, _| shell.set_draft_forge_for_test(forge))
        .unwrap();
    Rig {
        _dir: dir,
        db,
        project,
        window,
        automation,
        run,
        jobs,
    }
}

fn finish(rig: &Rig, cx: &mut TestAppContext) {
    let target = ReviewTarget::from_event_key(KEY).unwrap();
    let running = Running::for_test(
        &rig.run,
        ReviewPolicy {
            results: ReviewResults::Pending,
            target: Some(target.clone()),
        },
        &rig.automation,
        &rig.project,
    );
    rig.window
        .update(cx, |shell, _, cx| {
            shell.leave_draft_review(running, target, REPLY.into(), cx)
        })
        .unwrap();
    cx.run_until_parked();
}

fn run_status(rig: &Rig) -> (String, Option<String>) {
    let runs = Service::open(&rig.db)
        .unwrap()
        .runs(&rig.automation)
        .unwrap();
    let run = runs.iter().find(|r| r.id == rig.run).unwrap();
    (run.status.clone(), run.error.clone())
}

fn tasks(rig: &Rig, cx: &mut TestAppContext) -> Vec<(Kind, String)> {
    rig.window
        .update(cx, |shell, _, cx| shell.my_task_snapshot_for_test(cx))
        .unwrap()
}

fn decide(rig: &Rig, submit: bool, cx: &mut TestAppContext) {
    rig.window
        .update(cx, |shell, _, cx| {
            shell.decide_review_draft(&rig.run, submit, cx)
        })
        .unwrap();
    cx.run_until_parked();
}

fn jobs(rig: &Rig) -> Vec<String> {
    rig.jobs.lock().unwrap().clone()
}

#[gpui::test]
fn a_finished_review_leaves_a_draft_and_my_tasks_asks_to_submit_it(cx: &mut TestAppContext) {
    let rig = rig(cx, Ok(draft()));
    finish(&rig, cx);
    // Posted to the pull request the event named, with the agent's reply.
    assert_eq!(jobs(&rig), ["create github:acme/app#7 reply=true"]);
    assert_eq!(run_status(&rig).0, "succeeded");
    assert_eq!(
        tasks(&rig, cx),
        [(
            Kind::DraftReview,
            "Submit the review on acme/app#7, or keep it as a draft?".to_string()
        )]
    );
    decide(&rig, true, cx);
    // A second answer to the same draft sends nothing more.
    decide(&rig, true, cx);
    assert_eq!(
        jobs(&rig),
        [
            "create github:acme/app#7 reply=true",
            "submit Github { review_id: 42 }"
        ]
    );
    assert!(tasks(&rig, cx).is_empty());
    assert!(
        Service::open(&rig.db)
            .unwrap()
            .waiting_review_drafts()
            .unwrap()
            .is_empty()
    );
    // My tasks keeps the request and its answer in its history.
    let kept = Store::open(&rig.db)
        .unwrap()
        .task(&format!("draft-review:{}", rig.run))
        .unwrap()
        .unwrap();
    assert_eq!(kept.kind, Kind::DraftReview);
    assert_eq!(kept.answer.as_deref(), Some("Submitted"));
}

#[gpui::test]
fn keeping_a_draft_sends_nothing_and_stops_asking(cx: &mut TestAppContext) {
    let rig = rig(cx, Ok(draft()));
    finish(&rig, cx);
    decide(&rig, false, cx);
    assert_eq!(jobs(&rig), ["create github:acme/app#7 reply=true"]);
    assert!(tasks(&rig, cx).is_empty());
    let kept = Store::open(&rig.db)
        .unwrap()
        .task(&format!("draft-review:{}", rig.run))
        .unwrap()
        .unwrap();
    assert_eq!(kept.kind, Kind::DraftReview);
    assert_eq!(kept.answer.as_deref(), Some("Kept as draft"));
}

#[gpui::test]
fn a_draft_waiting_when_gespan_quit_is_asked_about_after_it_reopens(cx: &mut TestAppContext) {
    let rig = rig(cx, Ok(draft()));
    finish(&rig, cx);
    let store = Arc::new(Mutex::new(Store::open(&rig.db).unwrap()));
    let cwd = rig.project.clone();
    let reopened = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(store, vec![], cwd, None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let shown = reopened
        .update(cx, |shell, _, cx| shell.my_task_snapshot_for_test(cx))
        .unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].0, Kind::DraftReview);
}

#[gpui::test]
fn a_draft_the_forge_refuses_fails_the_run_with_the_reason(cx: &mut TestAppContext) {
    let rig = rig(
        cx,
        Err("You already have a pending review on this pull request.".into()),
    );
    finish(&rig, cx);
    let (status, error) = run_status(&rig);
    assert_eq!(status, "failed");
    let error = error.unwrap();
    assert!(error.contains("could not leave it as a draft"), "{error}");
    assert!(error.contains("already have a pending review"), "{error}");
    assert!(tasks(&rig, cx).is_empty());
}
