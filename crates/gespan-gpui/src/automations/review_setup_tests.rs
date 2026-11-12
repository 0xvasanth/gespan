//! The two-click review setup sheet: its readiness rules, the choices its
//! sentence offers, and that Turn on saves exactly what the sentence shows.
use super::layout_tests::{Drawn, Page, click, open, page};
use super::review_setup::*;
use super::*;
use gespan_core::automations::review_setup::{
    ReviewSetupInput, WhichPrs, find_review_automation, review_draft,
};
use gespan_core::review_policy::ReviewResults;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

const CWD: &str = "/work/app";
const CARD: &str = "template-review-pull-requests";

fn repos(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn check(installed: bool, authenticated: bool, found: &[&str], remote: bool) -> GithubCheck {
    GithubCheck {
        installed,
        authenticated,
        repos: repos(found),
        remote,
    }
}

#[test]
fn readiness_rules() {
    use Readiness::*;
    let one = repos(&["acme/app"]);
    let none: Vec<String> = Vec::new();
    // (installed, authenticated, repos, remote, chosen repo) -> state
    type Row<'a> = (bool, bool, &'a [String], bool, Option<&'a str>, Readiness);
    let rows: Vec<Row> = vec![
        (false, false, &one, false, None, NotInstalled),
        (true, false, &one, false, None, SignedOut),
        (true, true, &one, false, None, Connected),
        (true, true, &none, false, None, NoGithubRemote),
        // Picking a repository is what a project off GitHub is missing.
        (true, true, &none, false, Some("acme/app"), Connected),
        // A missing CLI or sign-in still comes first.
        (false, false, &none, false, None, NotInstalled),
        (true, false, &none, false, None, SignedOut),
        // Remote wins over everything, even a ready local check.
        (true, true, &one, true, None, Remote),
        (false, false, &none, true, None, Remote),
        (true, false, &one, true, Some("acme/app"), Remote),
    ];
    for (installed, authenticated, found, remote, chosen, expected) in rows {
        assert_eq!(
            readiness(installed, authenticated, found, remote, chosen),
            expected,
            "installed={installed} authenticated={authenticated} repos={found:?} remote={remote} chosen={chosen:?}"
        );
    }
    for state in [
        Checking,
        NotInstalled,
        SignedOut,
        Connected,
        NoGithubRemote,
        Remote,
    ] {
        assert_eq!(can_turn_on(state), state == Connected, "{state:?}");
    }
}

#[test]
fn default_repo_only_when_single() {
    assert_eq!(default_repo(&[]), None);
    assert_eq!(
        default_repo(&repos(&["acme/app"])),
        Some("acme/app".to_string())
    );
    assert_eq!(default_repo(&repos(&["acme/app", "acme/api"])), None);
}

#[test]
fn results_follow_the_harness() {
    assert_eq!(results_for("pi"), [ReviewResults::Gespan]);
    assert_eq!(
        results_for("claude"),
        [
            ReviewResults::Gespan,
            ReviewResults::Draft,
            ReviewResults::Post
        ]
    );
}

/// A page whose GitHub check answers `answer` instead of running `gh`.
fn open_with(
    page: &Page,
    answer: GithubCheck,
    cx: &mut TestAppContext,
) -> (WindowHandle<Automations>, VisualTestContext) {
    let (window, mut visual) = open(page, CWD, cx);
    window
        .update(&mut visual, |view, _, cx| {
            view.github_probe = Arc::new(move |_| answer.clone());
            view.category = "review".into();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    (window, visual)
}

fn connected() -> GithubCheck {
    check(true, true, &["acme/app"], false)
}

fn sheet(window: &WindowHandle<Automations>, visual: &VisualTestContext) -> Option<ReviewSetup> {
    window
        .read_with(visual, |view, _| view.review_setup.clone())
        .unwrap()
}

fn input(window: &WindowHandle<Automations>, visual: &VisualTestContext) -> ReviewSetupInput {
    sheet(window, visual).expect("the sheet is open").input
}

fn choose(
    window: &WindowHandle<Automations>,
    visual: &mut VisualTestContext,
    field: Choice,
    value: &str,
) {
    window
        .update(visual, |view, _, cx| {
            view.apply_choice(field, value.into(), cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn pick_model(
    window: &WindowHandle<Automations>,
    visual: &mut VisualTestContext,
    kind: Kind,
    model: &str,
) {
    window
        .update(visual, |view, _, cx| {
            view.set_review_model(kind, Some(model.into()), cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn stored(page: &Page) -> Vec<Automation> {
    page.service.lock().unwrap().list().unwrap()
}

#[gpui::test]
fn the_card_opens_the_sheet_with_the_defaults(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, connected(), cx);
    click(&mut visual, CARD);
    let setup = sheet(&window, &visual).expect("the card opens the sheet");
    assert_eq!(setup.readiness, Readiness::Connected);
    assert_eq!(setup.input.cwd, CWD);
    assert_eq!(setup.input.repo.as_deref(), Some("acme/app"));
    assert_eq!(setup.input.which, WhichPrs::ReviewRequested);
    assert_eq!(setup.input.results, ReviewResults::Gespan);
    let editor_open = window
        .read_with(&visual, |view, _| view.editor.is_some())
        .unwrap();
    assert!(!editor_open, "the card no longer opens the editor");
    for drawn in [
        "review-setup",
        "review-setup-repo",
        "review-setup-which",
        "review-setup-model",
        "review-setup-results",
        "review-setup-turn-on",
        "review-setup-customize",
        "review-setup-cancel",
    ] {
        assert!(visual.drawn(drawn).is_some(), "{drawn} is not drawn");
    }
    let sentence = window
        .read_with(&visual, |view, _| view.review_sentence())
        .unwrap();
    assert!(
        sentence.starts_with(
            "When a pull request in acme/app asks for your review, Gespan reviews it with "
        ),
        "{sentence}"
    );
    assert!(
        sentence.ends_with(", without changing any files, and keeps the review in Gespan."),
        "{sentence}"
    );
    assert_eq!(
        window
            .read_with(&visual, |view, _| view.readiness_message())
            .unwrap(),
        "GitHub connected ✓"
    );

    // Cancel goes back to the gallery without saving.
    click(&mut visual, "review-setup-cancel");
    assert!(sheet(&window, &visual).is_none());
    assert!(stored(&page).is_empty());
}

#[gpui::test]
fn stale_readiness_result_is_ignored(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, check(true, false, &["acme/app"], false), cx);
    // Open, then reopen before the first check comes back.
    let first = window
        .update(&mut visual, |view, _, cx| {
            view.open_review_setup(cx);
            view.review_setup.as_ref().unwrap().generation
        })
        .unwrap();
    let second = window
        .update(&mut visual, |view, _, cx| {
            view.review_setup = None;
            view.open_review_setup(cx);
            view.review_setup.as_ref().unwrap().generation
        })
        .unwrap();
    assert_ne!(first, second);
    window
        .update(&mut visual, |view, _, cx| {
            view.github_checked(first, connected(), cx)
        })
        .unwrap();
    let setup = sheet(&window, &visual).unwrap();
    assert_eq!(setup.readiness, Readiness::Checking);
    assert!(!can_turn_on(setup.readiness));
    // Turn on does nothing while it is disabled.
    visual.refresh().unwrap();
    let bounds = visual.drawn("review-setup-turn-on").unwrap();
    visual.simulate_click(bounds.center(), Default::default());
    // The current check's answer is the one that lands.
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(
        sheet(&window, &visual).unwrap().readiness,
        Readiness::SignedOut
    );
    assert_eq!(
        window
            .read_with(&visual, |view, _| view.readiness_message())
            .unwrap(),
        "Sign in to GitHub to continue."
    );
    assert!(visual.drawn("review-setup-sign-in").is_some());
    assert!(visual.drawn("review-setup-check-again").is_some());
    assert!(stored(&page).is_empty(), "nothing saves while signed out");
}

#[gpui::test]
fn check_again_picks_up_a_sign_in(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, check(true, false, &["acme/app"], false), cx);
    click(&mut visual, CARD);
    assert_eq!(
        sheet(&window, &visual).unwrap().readiness,
        Readiness::SignedOut
    );
    window
        .update(&mut visual, |view, _, _| {
            view.github_probe = Arc::new(|_| connected());
        })
        .unwrap();
    click(&mut visual, "review-setup-check-again");
    assert_eq!(
        sheet(&window, &visual).unwrap().readiness,
        Readiness::Connected
    );
}

#[gpui::test]
fn sign_in_asks_the_shell_for_a_terminal(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, check(true, false, &["acme/app"], false), cx);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let seen = asked.clone();
    let view = window.root(&mut visual).unwrap();
    let _subscription = visual.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &Event, _| {
            if let Event::SignInGithub { cwd } = event {
                seen.borrow_mut().push(cwd.clone());
            }
        })
    });
    click(&mut visual, CARD);
    click(&mut visual, "review-setup-sign-in");
    assert_eq!(*asked.borrow(), vec![CWD.to_string()]);
}

#[gpui::test]
fn not_installed_and_remote_and_off_github_cannot_turn_on(cx: &mut TestAppContext) {
    let cases = [
        (
            check(false, false, &[], false),
            Readiness::NotInstalled,
            "GitHub CLI isn't installed.",
            vec!["review-setup-get-gh", "review-setup-check-again"],
        ),
        (
            check(true, true, &[], false),
            Readiness::NoGithubRemote,
            "This project isn't on GitHub.",
            vec![],
        ),
        (
            check(true, true, &["acme/app"], true),
            Readiness::Remote,
            "Quick setup isn't available for remote projects yet.",
            vec![],
        ),
    ];
    for (answer, state, message, buttons) in cases {
        let page = page();
        let (window, mut visual) = open_with(&page, answer, cx);
        click(&mut visual, CARD);
        assert_eq!(sheet(&window, &visual).unwrap().readiness, state);
        assert_eq!(
            window
                .read_with(&visual, |view, _| view.readiness_message())
                .unwrap(),
            message
        );
        for button in buttons {
            assert!(visual.drawn(button).is_some(), "{state:?}: {button}");
        }
        click(&mut visual, "review-setup-turn-on");
        // Not only the button: turning on itself refuses.
        window
            .update(&mut visual, |view, _, cx| view.turn_on_review(cx))
            .unwrap();
        visual.run_until_parked();
        assert!(stored(&page).is_empty(), "{state:?} saved");
        if state == Readiness::NoGithubRemote {
            let sentence = window
                .read_with(&visual, |view, _| view.review_sentence())
                .unwrap();
            assert!(
                sentence.starts_with("When a pull request in Choose a repository… "),
                "{sentence}"
            );
        }
        // Customize is the way on from every state.
        assert!(visual.drawn("review-setup-customize").is_some());
    }
}

#[gpui::test]
fn results_menu_respects_harness(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, connected(), cx);
    click(&mut visual, CARD);
    let results_menu = |visual: &mut VisualTestContext| {
        click(visual, "review-setup-results");
        window
            .read_with(visual, |view, _| {
                view.choice
                    .as_ref()
                    .map(|menu| menu.options.clone())
                    .unwrap_or_default()
            })
            .unwrap()
    };

    pick_model(&window, &mut visual, Kind::Claude, "claude-sonnet-4-5");
    assert_eq!(
        results_menu(&mut visual),
        vec![
            ("gespan".into(), "keeps the review in Gespan".into()),
            ("draft".into(), "drafts a review for you to approve".into()),
            ("post".into(), "posts the review on the PR".into()),
        ]
    );
    choose(&window, &mut visual, Choice::SetupResults, "post");
    assert_eq!(input(&window, &visual).results, ReviewResults::Post);

    // Pi can only keep the review in Gespan, so the choice falls back to it.
    pick_model(&window, &mut visual, Kind::Pi, "pi-model");
    let setup = input(&window, &visual);
    assert_eq!(
        (setup.harness.as_str(), setup.model.as_str()),
        ("pi", "pi-model")
    );
    assert_eq!(setup.results, ReviewResults::Gespan);
    assert_eq!(
        results_menu(&mut visual),
        vec![("gespan".into(), "keeps the review in Gespan".into())]
    );
    // A value the harness can't do is refused even if asked for directly.
    choose(&window, &mut visual, Choice::SetupResults, "post");
    assert_eq!(input(&window, &visual).results, ReviewResults::Gespan);
}

#[gpui::test]
fn turn_on_saves_the_shown_values(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, connected(), cx);
    click(&mut visual, CARD);
    pick_model(&window, &mut visual, Kind::Claude, "claude-sonnet-4-5");
    choose(
        &window,
        &mut visual,
        Choice::SetupWhich,
        "opened-and-drafts",
    );
    choose(&window, &mut visual, Choice::SetupResults, "post");
    let shown = input(&window, &visual);
    let sentence = window
        .read_with(&visual, |view, _| view.review_sentence())
        .unwrap();
    assert!(
        sentence.starts_with(
            "When a pull request in acme/app is opened, drafts too, Gespan reviews it with "
        ),
        "{sentence}"
    );
    assert!(
        sentence.ends_with("and posts the review on the PR."),
        "{sentence}"
    );

    click(&mut visual, "review-setup-turn-on");

    let saved = stored(&page);
    assert_eq!(saved.len(), 1);
    let saved = &saved[0];
    let expected = review_draft(&shown);
    assert!(saved.enabled);
    assert_eq!(saved.name, expected.name);
    assert_eq!(saved.cwd, CWD);
    assert_eq!(saved.prompt, expected.prompt);
    assert_eq!(
        (saved.harness.as_str(), saved.model.as_str()),
        ("claude", "claude-sonnet-4-5")
    );
    assert_eq!(saved.workspace_mode, "current");
    assert!(!saved.reuse_session);
    assert_eq!(saved.runtime_mode, "read-only");
    assert_eq!(saved.action, AutomationAction::Session);
    assert_eq!(saved.review_results, Some(ReviewResults::Post));
    let triggers = |list: &[AutomationTrigger]| {
        list.iter()
            .map(|t| (t.kind.clone(), t.event.clone(), t.repos.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        triggers(saved.triggers.as_deref().unwrap_or_default()),
        triggers(expected.triggers.as_deref().unwrap())
    );
    assert_eq!(
        triggers(saved.triggers.as_deref().unwrap_or_default()),
        vec![
            (
                "github".into(),
                "pull_request_opened".into(),
                repos(&["acme/app"])
            ),
            ("github".into(), "draft_opened".into(), repos(&["acme/app"])),
        ]
    );
    let (selected, sheet_open, editing) = window
        .read_with(&visual, |view, _| {
            (
                view.selected.clone(),
                view.review_setup.is_some(),
                view.editor.as_ref().map(|e| (e.draft.id.clone(), e.saved)),
            )
        })
        .unwrap();
    assert_eq!(selected.as_deref(), Some(saved.id.as_str()));
    assert!(!sheet_open);
    assert_eq!(editing, Some((saved.id.clone(), true)));
    assert!(
        visual
            .drawn(&format!("automation-card-{}", saved.id))
            .is_some(),
        "the new automation shows in the list"
    );
}

#[gpui::test]
fn existing_review_automation_opens_in_editor(cx: &mut TestAppContext) {
    let page = page();
    let existing = page
        .service
        .lock()
        .unwrap()
        .upsert(review_draft(&ReviewSetupInput {
            cwd: CWD.into(),
            repo: None,
            which: WhichPrs::Opened,
            harness: "claude".into(),
            model: "claude-sonnet-4-5".into(),
            results: ReviewResults::Gespan,
        }))
        .unwrap();
    assert!(find_review_automation(&stored(&page), CWD).is_some());
    let (window, mut visual) = open_with(&page, connected(), cx);
    click(&mut visual, CARD);
    let (selected, sheet_open, editing) = window
        .read_with(&visual, |view, _| {
            (
                view.selected.clone(),
                view.review_setup.is_some(),
                view.editor.as_ref().map(|e| e.draft.id.clone()),
            )
        })
        .unwrap();
    assert!(!sheet_open, "no second setup for the same project");
    assert_eq!(selected.as_deref(), Some(existing.id.as_str()));
    assert_eq!(editing.as_deref(), Some(existing.id.as_str()));
    assert_eq!(stored(&page).len(), 1);
}

#[gpui::test]
fn customize_carries_the_draft(cx: &mut TestAppContext) {
    let page = page();
    let (window, mut visual) = open_with(&page, connected(), cx);
    click(&mut visual, CARD);
    pick_model(&window, &mut visual, Kind::Claude, "claude-sonnet-4-5");
    choose(&window, &mut visual, Choice::SetupWhich, "opened");
    choose(&window, &mut visual, Choice::SetupResults, "draft");
    choose(&window, &mut visual, Choice::SetupRepo, "");
    let shown = input(&window, &visual);
    assert_eq!(shown.repo, None);
    click(&mut visual, "review-setup-customize");
    let (draft, saved, sheet_open) = window
        .read_with(&visual, |view, _| {
            let editor = view.editor.as_ref().expect("the editor opens");
            (
                editor.draft.clone(),
                editor.saved,
                view.review_setup.is_some(),
            )
        })
        .unwrap();
    assert!(!sheet_open);
    assert!(!saved, "Customize does not save");
    let expected = review_draft(&shown);
    assert_eq!(draft.name, expected.name);
    assert_eq!(draft.prompt, expected.prompt);
    assert_eq!(draft.harness, "claude");
    assert_eq!(draft.model, "claude-sonnet-4-5");
    assert_eq!(draft.runtime_mode, "read-only");
    assert_eq!(draft.workspace_mode, "current");
    assert_eq!(draft.review_results, Some(ReviewResults::Draft));
    let events = draft
        .triggers
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.event, t.repos))
        .collect::<Vec<_>>();
    assert_eq!(events, vec![("pull_request_opened".to_string(), vec![])]);
    assert!(stored(&page).is_empty());
}

#[gpui::test]
fn sheet_layout_fits(cx: &mut TestAppContext) {
    for width in [760., 1600.] {
        let page = page();
        let (window, mut visual) = open_with(&page, connected(), cx);
        visual.simulate_resize(gpui::size(px(width), px(800.)));
        visual.run_until_parked();
        click(&mut visual, CARD);
        // Long names make the sentence wrap rather than overflow.
        pick_model(&window, &mut visual, Kind::Claude, "claude-sonnet-4-5");
        choose(
            &window,
            &mut visual,
            Choice::SetupWhich,
            "opened-and-drafts",
        );
        choose(&window, &mut visual, Choice::SetupResults, "draft");
        let sheet = visual.drawn("review-setup").expect("sheet drawn");
        assert!(
            sheet.right() <= px(width),
            "{width}: sheet ends at {:?}",
            sheet.right()
        );
        for part in [
            "review-setup-sentence",
            "review-setup-repo",
            "review-setup-which",
            "review-setup-model",
            "review-setup-results",
            "review-setup-readiness",
            "review-setup-turn-on",
            "review-setup-customize",
            "review-setup-cancel",
        ] {
            let bounds = visual
                .drawn(part)
                .unwrap_or_else(|| panic!("{width}: {part} is not drawn"));
            assert!(
                bounds.left() >= sheet.left() && bounds.right() <= sheet.right(),
                "{width}: {part} {bounds:?} overflows the sheet {sheet:?}"
            );
        }
    }
}

use std::{cell::RefCell, rc::Rc};
