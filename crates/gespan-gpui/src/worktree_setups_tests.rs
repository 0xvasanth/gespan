use super::*;
use gpui::{TestAppContext, VisualTestContext};
use std::path::PathBuf;

fn run(id: u64, stage: Stage, copied: Copied, scripted: bool) -> Run {
    Run {
        id,
        stage,
        project: PathBuf::from("/work/app"),
        worktree: PathBuf::from("/work/app-worktrees/feature"),
        branch: Some("feature".into()),
        copied,
        scripted,
    }
}
fn copied(files: usize, names: &[&str], skipped: usize) -> Copied {
    Copied {
        files,
        skipped,
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        ..Default::default()
    }
}
fn view(cx: &mut TestAppContext) -> (Entity<WorktreeSetups>, VisualTestContext) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| WorktreeSetups {
                steps: Vec::new(),
                answers: Answers::default(),
            })
        })
        .unwrap()
    });
    let view = window.root(cx).unwrap();
    (view, VisualTestContext::from_window(window.into(), cx))
}
fn shown(visual: &mut VisualTestContext, selector: &str) -> bool {
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual
        .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        .is_some()
}
fn click(visual: &mut VisualTestContext, selector: &str) {
    visual.run_until_parked();
    visual.refresh().unwrap();
    let bounds = visual
        .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), gpui::Modifiers::none());
}

#[test]
fn the_copy_summary_names_a_few_files() {
    assert_eq!(copied_summary(&Copied::default()), None);
    assert_eq!(
        copied_summary(&copied(1, &[".env"], 0)).unwrap(),
        "Copied .env"
    );
    assert_eq!(
        copied_summary(&copied(5, &["a", "b", "c", "d"], 0)).unwrap(),
        "Copied a, b, c and 2 more"
    );
    assert_eq!(
        copied_summary(&copied(1, &[".env"], 2)).unwrap(),
        "Copied .env · Skipped 2 files"
    );
    assert_eq!(
        copied_summary(&copied(0, &[], 1)).unwrap(),
        "Skipped 1 file"
    );
}

#[test]
fn headings_say_what_happened_at_each_stage() {
    let setup = run(1, Stage::Setup, Copied::default(), true);
    let bare = run(1, Stage::Setup, copied(1, &["a"], 0), false);
    let teardown = run(1, Stage::Teardown, Copied::default(), true);
    assert_eq!(heading(&setup, &State::Running), "Setting up feature");
    assert_eq!(
        heading(&setup, &State::Failed("x".into())),
        "Setup failed for feature"
    );
    assert_eq!(
        heading(&setup, &State::Done(Outcome::Passed)),
        "Setup passed for feature"
    );
    assert_eq!(
        heading(&bare, &State::Done(Outcome::Passed)),
        "feature is ready"
    );
    assert_eq!(
        heading(&setup, &State::Done(Outcome::Overridden("x".into()))),
        "Started feature without setup"
    );
    assert_eq!(
        heading(&teardown, &State::Failed("x".into())),
        "Teardown failed for feature"
    );
}

#[gpui::test]
fn a_failure_shows_its_output_and_each_button_answers(cx: &mut TestAppContext) {
    for (button, decision) in [
        ("retry", Decision::Retry),
        ("continue", Decision::Continue),
        ("cancel", Decision::Cancel),
    ] {
        let (view, mut visual) = view(cx);
        let (answer, answered) = mpsc::channel();
        view.update(&mut visual, |view, cx| {
            view.answers.lock().unwrap().insert(7, answer);
            view.apply(
                Event::Started(run(7, Stage::Setup, Copied::default(), true)),
                cx,
            );
            view.apply(
                Event::Output {
                    id: 7,
                    line: "npm ERR! missing".into(),
                },
                cx,
            );
            view.apply(
                Event::Failed {
                    id: 7,
                    reason: "It exited with code 1.".into(),
                },
                cx,
            );
        });
        assert!(shown(&mut visual, "worktree-setup-output-7"));
        assert!(!shown(&mut visual, "worktree-setup-stop-7"));
        click(&mut visual, &format!("worktree-setup-{button}-7"));
        assert_eq!(answered.try_recv(), Ok(decision));
    }
}

#[gpui::test]
fn a_retry_starts_the_output_again_and_cancel_shows_while_running(cx: &mut TestAppContext) {
    let (view, mut visual) = view(cx);
    view.update(&mut visual, |view, cx| {
        view.apply(
            Event::Started(run(3, Stage::Setup, Copied::default(), true)),
            cx,
        );
        view.apply(
            Event::Output {
                id: 3,
                line: "first try".into(),
            },
            cx,
        );
        view.apply(
            Event::Failed {
                id: 3,
                reason: "failed".into(),
            },
            cx,
        );
        view.apply(Event::Retrying { id: 3 }, cx);
    });
    assert!(shown(&mut visual, "worktree-setup-stop-3"));
    assert!(!shown(&mut visual, "worktree-setup-retry-3"));
    view.read_with(&visual, |view, _| {
        assert_eq!(view.steps[0].tail.lines().count(), 0);
        assert_eq!(view.steps[0].state, State::Running);
    });
}

#[gpui::test]
fn a_finished_step_leaves_on_its_own(cx: &mut TestAppContext) {
    let (view, mut visual) = view(cx);
    view.update(&mut visual, |view, cx| {
        view.apply(
            Event::Started(run(4, Stage::Setup, copied(2, &["a", "b"], 0), false)),
            cx,
        );
        view.apply(
            Event::Finished {
                id: 4,
                outcome: Outcome::Passed,
            },
            cx,
        );
    });
    assert!(shown(&mut visual, "worktree-setup-step-4"));
    visual
        .executor()
        .advance_clock(LINGER + Duration::from_secs(1));
    assert!(!shown(&mut visual, "worktree-setup-step-4"));
    view.read_with(&visual, |view, _| assert!(view.is_empty()));
}

#[gpui::test]
fn events_for_a_step_that_left_are_ignored(cx: &mut TestAppContext) {
    let (view, mut visual) = view(cx);
    view.update(&mut visual, |view, cx| {
        view.apply(
            Event::Output {
                id: 9,
                line: "late".into(),
            },
            cx,
        );
        view.apply(Event::Retrying { id: 9 }, cx);
    });
    view.read_with(&visual, |view, _| assert!(view.is_empty()));
}

#[test]
fn an_answer_nobody_can_give_cancels() {
    let (events, _) = async_channel::unbounded();
    let answers = Answers::default();
    let bridge = Bridge {
        events,
        answers: answers.clone(),
    };
    let worker = std::thread::spawn(move || bridge.decide(1));
    while answers.lock().unwrap().is_empty() {
        std::thread::yield_now();
    }
    answers.lock().unwrap().clear();
    assert_eq!(worker.join().unwrap(), Decision::Cancel);
}
