use super::*;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::{cell::RefCell, rc::Rc};

fn picker(
    cx: &mut TestAppContext,
    cwd: &str,
) -> (WindowHandle<Completion>, Rc<RefCell<Vec<String>>>) {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let seen = opened.clone();
    let window = cx.update(|cx| {
        input::bind(cx);
        bind(cx);
        crate::popover::bind(cx);
        cx.on_action(move |open: &crate::file_editor::OpenPath, _| {
            seen.borrow_mut().push(open.0.clone())
        });
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let input = cx.new(input::Input::new);
                // Below the caret: the bare window leaves no room above it.
                let mut completion = Completion::new(input, cwd.into(), cx).inline_field();
                completion.loaded = Some(Instant::now());
                completion
            })
        })
        .unwrap()
    });
    window
        .update(cx, |view, window, cx| {
            window.focus(&view.input.focus_handle(cx), cx);
            view.input.update(cx, |input, cx| {
                input.set_text("Please /Code-Review now".into(), cx)
            });
            // The caret sits at the end of the command, as when typing it.
            let at = "Please /Code-Review".len();
            view.input
                .update(cx, |input, cx| input.select_range(at..at, cx));
        })
        .unwrap();
    cx.run_until_parked();
    (window, opened)
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector}"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
}

#[gpui::test]
fn new_skill_writes_a_project_starter_then_clears_the_command_and_opens_it(
    cx: &mut TestAppContext,
) {
    let project = tempfile::tempdir().unwrap();
    let (window, opened) = picker(cx, project.path().to_str().unwrap());
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(700.), px(700.)));
    visual.run_until_parked();
    click(visual, "completion-new-skill");
    window
        .update(visual, |view, window, cx| {
            let form = view.creating.as_ref().expect("the form");
            assert_eq!(
                form.name.read(cx).content,
                "code-review",
                "named from the command"
            );
            assert!(!form.personal, "a project folder defaults to Project");
            assert!(form.name.focus_handle(cx).is_focused(window));
        })
        .unwrap();
    assert!(
        visual.debug_bounds("completion-new-skill-form").is_some(),
        "it stays open while the field has focus"
    );
    assert!(visual.debug_bounds("composer-completion-list").is_none());
    click(visual, "completion-new-skill-create");
    let path = project
        .path()
        .join(".agents")
        .join("skills")
        .join("code-review")
        .join("SKILL.md");
    let written = std::fs::read_to_string(&path).expect("the starter is written");
    assert!(written.starts_with("---\nname: code-review\n"), "{written}");
    window
        .update(visual, |view, window, cx| {
            assert!(view.creating.is_none());
            assert!(view.token.is_none(), "the picker closes");
            assert_eq!(
                view.input.read(cx).content,
                "Please now",
                "only the command goes"
            );
            assert!(view.input.focus_handle(cx).is_focused(window));
            assert!(view.loaded.is_none(), "the next / reads the new skill");
        })
        .unwrap();
    assert_eq!(*opened.borrow(), [path.to_string_lossy().into_owned()]);
}

#[gpui::test]
fn the_form_cancels_with_escape_and_enter_never_picks_a_listed_command(cx: &mut TestAppContext) {
    let project = tempfile::tempdir().unwrap();
    let (window, _) = picker(cx, project.path().to_str().unwrap());
    window
        .update(cx, |view, window, cx| view.start_new_skill(window, cx))
        .unwrap();
    cx.run_until_parked();
    // An invalid name cannot be created, by Enter or by the button.
    window
        .update(cx, |view, _, cx| {
            let name = view.creating.as_ref().unwrap().name.clone();
            name.update(cx, |input, cx| input.set_text("--".into(), cx));
        })
        .unwrap();
    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.creating.as_ref().is_some_and(|form| !form.busy));
            assert_eq!(
                view.input.read(cx).content,
                "Please /Code-Review now",
                "no command was picked"
            );
        })
        .unwrap();
    assert!(!project.path().join(".agents").exists());
    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(view.creating.is_none(), "back to the list");
            assert!(view.token.is_some(), "with the picker still open");
            assert!(view.input.focus_handle(cx).is_focused(window));
        })
        .unwrap();
}

#[gpui::test]
fn an_existing_skill_is_kept_and_the_error_shown(cx: &mut TestAppContext) {
    let project = tempfile::tempdir().unwrap();
    let existing = project
        .path()
        .join(".agents")
        .join("skills")
        .join("code-review");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "mine").unwrap();
    let (window, opened) = picker(cx, project.path().to_str().unwrap());
    window
        .update(cx, |view, window, cx| {
            view.start_new_skill(window, cx);
            view.create_new_skill(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            let form = view.creating.as_ref().expect("the form stays");
            assert!(!form.busy);
            assert!(
                form.error
                    .as_deref()
                    .is_some_and(|e| e.starts_with("Could not create skill")),
                "{:?}",
                form.error
            );
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(existing.join("SKILL.md")).unwrap(),
        "mine"
    );
    assert!(opened.borrow().is_empty());
}

#[gpui::test]
fn without_a_project_folder_only_personal_skills_are_offered(cx: &mut TestAppContext) {
    let (window, _) = picker(cx, "~");
    window
        .update(cx, |view, window, cx| {
            assert!(view.offers_new_skill());
            view.start_new_skill(window, cx);
            assert!(view.creating.as_ref().unwrap().personal);
        })
        .unwrap();
}
