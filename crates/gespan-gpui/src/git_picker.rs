//! The composer's searchable BranchPicker and existing-worktree list.
use crate::{input, popover, right_panel::spinner, theme::*};
use gespan_core::git::{self, Snapshot};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, Subscription, Window,
    div, prelude::*, px,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Branch,
    Base,
    Worktree,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn all_picker_modes_start_with_cached_choices_and_keep_them_after_refresh_failure(
        cx: &mut TestAppContext,
    ) {
        let cwd = "/nonexistent/gespan-cached-git-picker";
        crate::git_cache::remember(
            cwd,
            &Snapshot {
                current: Some("main".into()),
                branches: vec![
                    git::Branch {
                        name: "main".into(),
                        remote: None,
                    },
                    git::Branch {
                        name: "feature".into(),
                        remote: None,
                    },
                ],
                worktrees: vec![git::Worktree {
                    path: cwd.into(),
                    branch: Some("main".into()),
                    main: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        let pickers: Vec<_> = [Mode::Branch, Mode::Base, Mode::Worktree]
            .into_iter()
            .map(|mode| {
                let picker = cx.new(|cx| GitPicker::new(cwd.into(), mode, None, cx));
                picker.update(cx, |view, cx| {
                    assert!(
                        view.state.is_some(),
                        "choices are available before any background work"
                    );
                    assert_eq!(
                        view.choices(cx).len(),
                        if mode == Mode::Worktree { 1 } else { 2 }
                    );
                });
                picker
            })
            .collect();
        cx.run_until_parked();
        for picker in pickers {
            picker.update(cx, |view, cx| {
                assert!(view.error.is_some());
                assert!(
                    !view.choices(cx).is_empty(),
                    "a failed refresh retains usable choices"
                );
            });
        }
        assert!(crate::git_cache::get(cwd).is_some());
    }

    /// A repository on `main` whose `shared.txt` edit conflicts with `other`.
    fn conflicting_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| run_git(repo.path(), args);
        git(&["init", "-b", "main"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "core.autocrlf", "false"]);
        git(&["config", "core.safecrlf", "false"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.path().join("shared.txt"), "base\n").unwrap();
        git(&["add", "shared.txt"]);
        git(&["commit", "-m", "Base"]);
        git(&["switch", "-c", "other"]);
        std::fs::write(repo.path().join("shared.txt"), "other\n").unwrap();
        git(&["commit", "-am", "Other"]);
        git(&["switch", "main"]);
        std::fs::write(repo.path().join("shared.txt"), "mine\n").unwrap();
        repo
    }
    fn run_git(cwd: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn open_on(
        cwd: &std::path::Path,
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<GitPicker>,
        std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
        Subscription,
    ) {
        cx.update(input::bind);
        let cwd = cwd.to_owned();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| GitPicker::new(cwd, Mode::Branch, None, cx))
            })
            .unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = events.clone();
        let entity = window
            .update(cx, |view, window, cx| {
                window.focus(&view.focus(cx), cx);
                cx.entity()
            })
            .unwrap();
        let subscription = cx.update(|cx| {
            cx.subscribe(&entity, move |_, event: &Picked, _| {
                seen.borrow_mut().push(match event {
                    Picked::Changed => "changed",
                    Picked::Dismissed => "dismissed",
                    Picked::Base(_) | Picked::Worktree(_) => "other",
                })
            })
        });
        cx.run_until_parked();
        (window, events, subscription)
    }
    fn click(visual: &mut gpui::VisualTestContext, selector: &'static str) {
        visual.run_until_parked();
        let bounds = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }

    #[gpui::test]
    fn a_blocked_switch_offers_stash_and_ends_on_the_new_branch(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let (window, events, _subscription) = open_on(repo.path(), cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_input("other");
        visual.run_until_parked();
        assert!(visual.debug_bounds("git-option-1").is_none());
        click(visual, "git-option-0");
        // Git refused, so the picker asks instead of showing Git's error.
        assert!(visual.debug_bounds("switch-branch-dialog").is_some());
        assert!(visual.debug_bounds("git-picker").is_none());
        window
            .update(visual, |view, window, cx| {
                assert!(view.error.is_none());
                let blocked = view.blocked.as_ref().unwrap();
                assert_eq!(blocked.branch.name, "other");
                assert!(blocked.message.focus_handle(cx).is_focused(window));
            })
            .unwrap();
        assert_eq!(run_git(repo.path(), &["branch", "--show-current"]), "main");
        // Commit & switch needs a message; with none it does nothing.
        click(visual, "switch-commit");
        assert_eq!(run_git(repo.path(), &["branch", "--show-current"]), "main");
        assert!(events.borrow().is_empty());

        click(visual, "switch-stash");
        assert_eq!(events.borrow().as_slice(), ["changed"]);
        assert_eq!(run_git(repo.path(), &["branch", "--show-current"]), "other");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("shared.txt")).unwrap(),
            "other\n"
        );
        assert_eq!(
            run_git(repo.path(), &["stash", "list", "--format=%gs"]),
            "On main: WIP before switching to other"
        );
    }

    /// Opens the Uncommitted changes dialog with a stand-in for the provider.
    fn blocked_dialog(
        repo: &std::path::Path,
        write: MessageWriter,
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<GitPicker>,
        gpui::VisualTestContext,
        std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
        Subscription,
    ) {
        let (window, events, subscription) = open_on(repo, cx);
        window
            .update(cx, |view, _, _| view.write_message = write)
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_input("other");
        click(&mut visual, "git-option-0");
        (window, visual, events, subscription)
    }
    fn message_of(window: gpui::WindowHandle<GitPicker>, cx: &mut TestAppContext) -> String {
        window
            .read_with(cx, |view, cx| {
                let blocked = view.blocked.as_ref().unwrap();
                blocked.message.read(cx).content.to_string()
            })
            .unwrap()
    }

    #[gpui::test]
    fn generate_fills_the_message_and_commit_and_switch_uses_it(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let root = repo.path().to_owned();
        let write: MessageWriter = Arc::new(move |cwd, _| {
            assert_eq!(cwd, root, "the message describes this checkout");
            Ok("Keep the shared edit".into())
        });
        let (window, mut visual, events, _subscription) = blocked_dialog(repo.path(), write, cx);
        click(&mut visual, "switch-generate");
        assert_eq!(message_of(window, cx), "Keep the shared edit");
        window
            .read_with(cx, |view, cx| {
                let blocked = view.blocked.as_ref().unwrap();
                assert!(blocked.idle());
                assert!(!blocked.message.read(cx).read_only());
            })
            .unwrap();

        click(&mut visual, "switch-commit");
        assert_eq!(events.borrow().as_slice(), ["changed"]);
        assert_eq!(
            run_git(repo.path(), &["log", "-1", "--format=%s", "main"]),
            "Keep the shared edit"
        );
    }

    #[gpui::test]
    fn a_failed_generate_stays_in_the_dialog_and_keeps_what_was_typed(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let write: MessageWriter = Arc::new(|_, _| Err("No provider".into()));
        let (window, mut visual, events, _subscription) = blocked_dialog(repo.path(), write, cx);
        visual.simulate_input("Mine");
        click(&mut visual, "switch-generate");
        assert_eq!(message_of(window, cx), "Mine");
        window
            .read_with(cx, |view, _| {
                let blocked = view.blocked.as_ref().unwrap();
                assert_eq!(blocked.error.as_deref(), Some("No provider"));
                assert!(blocked.idle());
            })
            .unwrap();
        assert!(events.borrow().is_empty());
        // The dialog is still usable, and the error clears on the next try.
        click(&mut visual, "switch-generate");
        click(&mut visual, "switch-commit");
        assert_eq!(events.borrow().as_slice(), ["changed"]);
    }

    /// A provider that records whether it was told to stop. The test executor
    /// runs it only when the test parks, so everything done in one window update
    /// happens while the turn is in flight.
    fn recording_writer(reply: &'static str) -> (MessageWriter, Arc<AtomicBool>) {
        let stopped = Arc::new(AtomicBool::new(false));
        let seen = stopped.clone();
        let write: MessageWriter = Arc::new(move |_, cancel| {
            seen.store(cancel.load(Ordering::SeqCst), Ordering::SeqCst);
            Ok(reply.into())
        });
        (write, stopped)
    }

    #[gpui::test]
    fn while_generating_the_dialog_cannot_commit_stash_or_close(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let (write, stopped) = recording_writer("Written");
        let (window, mut visual, events, _subscription) = blocked_dialog(repo.path(), write, cx);
        visual.simulate_input("Typed");
        visual.run_until_parked();
        window
            .update(&mut visual, |view, window, cx| {
                view.generate(window, cx);
                view.resolve(Resolve::Commit, cx);
                view.resolve(Resolve::Stash, cx);
                view.cancel(cx);
                view.generate(window, cx);
                let blocked = view.blocked.as_ref().unwrap();
                assert!(blocked.generating.is_some());
                assert!(blocked.busy.is_none());
                assert!(blocked.message.read(cx).read_only());
                assert!(!view.can_commit(cx));
            })
            .unwrap();
        visual.run_until_parked();
        assert!(events.borrow().is_empty());
        assert_eq!(run_git(repo.path(), &["branch", "--show-current"]), "main");
        assert_eq!(run_git(repo.path(), &["stash", "list"]), "");
        assert!(!stopped.load(Ordering::SeqCst));
        assert_eq!(message_of(window, cx), "Written");
    }

    #[gpui::test]
    fn cancelling_generate_ignores_a_late_reply(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let (write, stopped) = recording_writer("Too late");
        let (window, mut visual, _events, _subscription) = blocked_dialog(repo.path(), write, cx);
        visual.simulate_input("Typed");
        visual.run_until_parked();
        window
            .update(&mut visual, |view, window, cx| {
                view.generate(window, cx);
                view.cancel_generate(window, cx);
                let blocked = view.blocked.as_ref().unwrap();
                assert!(blocked.idle());
                assert!(!blocked.message.read(cx).read_only());
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            stopped.load(Ordering::SeqCst),
            "the provider turn is told to stop"
        );
        assert_eq!(message_of(window, cx), "Typed");
    }

    #[gpui::test]
    fn closing_the_picker_stops_a_running_generate(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let (write, stopped) = recording_writer("Unused");
        let (window, mut visual, _events, _subscription) = blocked_dialog(repo.path(), write, cx);
        window
            .update(&mut visual, |view, window, cx| {
                view.generate(window, cx);
                view.blocked = None;
            })
            .unwrap();
        visual.run_until_parked();
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn a_blocked_switch_can_be_cancelled_or_committed(cx: &mut TestAppContext) {
        let repo = conflicting_repo();
        let (window, events, _subscription) = open_on(repo.path(), cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_input("other");
        click(visual, "git-option-0");
        click(visual, "switch-cancel");
        assert_eq!(events.borrow().as_slice(), ["dismissed"]);
        assert_eq!(run_git(repo.path(), &["branch", "--show-current"]), "main");
        assert_eq!(run_git(repo.path(), &["stash", "list"]), "");

        window
            .update(visual, |view, window, cx| {
                view.blocked = None;
                view.pick(0, window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_input("Keep my edits");
        click(visual, "switch-commit");
        assert_eq!(events.borrow().as_slice(), ["dismissed", "changed"]);
        assert_eq!(run_git(repo.path(), &["branch", "--show-current"]), "other");
        assert_eq!(
            run_git(repo.path(), &["log", "-1", "--format=%s", "main"]),
            "Keep my edits"
        );
        assert_eq!(run_git(repo.path(), &["show", "main:shared.txt"]), "mine");
        assert_eq!(run_git(repo.path(), &["status", "--porcelain"]), "");
    }
}
pub(crate) enum Picked {
    Changed,
    Base(String),
    Worktree(String),
    Dismissed,
}
pub(crate) struct GitPicker {
    cwd: PathBuf,
    mode: Mode,
    selected: Option<String>,
    state: Option<Snapshot>,
    search: Entity<input::Input>,
    active: usize,
    busy: bool,
    existing_only: bool,
    error: Option<String>,
    blocked: Option<Blocked>,
    write_message: MessageWriter,
    _changed: Subscription,
}
/// SwitchBranchDialog: Git refused the switch because it would overwrite
/// local changes, so the picker offers to stash or commit them first.
struct Blocked {
    branch: git::Branch,
    message: Entity<input::Input>,
    busy: Option<Resolve>,
    /// Set while a commit message is being written; storing true cancels it.
    generating: Option<Arc<AtomicBool>>,
    error: Option<String>,
    _subscriptions: [Subscription; 2],
}
impl Drop for Blocked {
    fn drop(&mut self) {
        if let Some(cancel) = &self.generating {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}
impl Blocked {
    fn idle(&self) -> bool {
        self.busy.is_none() && self.generating.is_none()
    }
}
/// Writes a message for a checkout's changes. Blocking.
type MessageWriter = Arc<dyn Fn(PathBuf, Arc<AtomicBool>) -> Result<String, String> + Send + Sync>;
fn write_commit_message(cwd: PathBuf, cancel: Arc<AtomicBool>) -> Result<String, String> {
    let (kind, binary) = crate::changes::pick_text_harness(gespan_core::provider::Kind::Claude)
        .ok_or("No provider CLI is installed to write a commit message.")?;
    crate::changes::generate_commit_message(kind, binary, cwd, cancel)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Resolve {
    Stash,
    Commit,
}
const MOD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};
#[derive(Clone)]
enum Choice {
    Branch(git::Branch),
    Create(String),
    Worktree(git::Worktree),
}
impl EventEmitter<Picked> for GitPicker {}
impl GitPicker {
    pub(crate) fn new(
        cwd: PathBuf,
        mode: Mode,
        selected: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            input::Input::field(
                if mode == Mode::Worktree {
                    "Search working copies"
                } else {
                    "Search branches"
                },
                if mode == Mode::Worktree {
                    "Search working copies…"
                } else {
                    "Search branches…"
                },
                content(0.4),
                cx,
            )
        });
        let changed = cx.subscribe(&search, |this, _, _: &input::Changed, cx| {
            this.active = 0;
            cx.notify();
        });
        let state = crate::git_cache::get(&cwd.to_string_lossy());
        let load_cwd = cwd.clone();
        let work = cx
            .background_executor()
            .spawn(async move { crate::git_cache::load(&load_cwd) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(state) => this.state = Some(state),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            cwd,
            mode,
            selected,
            state,
            search,
            active: 0,
            busy: false,
            existing_only: false,
            error: None,
            blocked: None,
            write_message: Arc::new(write_commit_message),
            _changed: changed,
        }
    }
    pub(crate) fn focus(&self, cx: &gpui::App) -> FocusHandle {
        match &self.blocked {
            Some(blocked) => blocked.message.focus_handle(cx),
            None => self.search.focus_handle(cx),
        }
    }
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }
    /// WorkspacePicker's submenu contains available linked copies; the parent
    /// already offers the project folder through Current checkout.
    pub(crate) fn existing_worktrees(mut self) -> Self {
        self.existing_only = true;
        self
    }
    fn choices(&self, cx: &gpui::App) -> Vec<Choice> {
        let query = self.search.read(cx).content.trim().to_owned();
        let needle = query.to_lowercase();
        let Some(state) = &self.state else {
            return Vec::new();
        };
        if self.mode == Mode::Worktree {
            return state
                .worktrees
                .iter()
                .filter(|tree| !self.existing_only || (!tree.main && !tree.missing))
                .filter(|tree| {
                    format!(
                        "{} {}",
                        tree.branch.as_deref().unwrap_or("detached"),
                        tree.path
                    )
                    .to_lowercase()
                    .contains(&needle)
                })
                .cloned()
                .map(Choice::Worktree)
                .collect();
        }
        let mut choices: Vec<_> = state
            .branches
            .iter()
            .filter(|branch| {
                format!("{} {}", branch.name, branch.remote.as_deref().unwrap_or(""))
                    .to_lowercase()
                    .contains(&needle)
            })
            .cloned()
            .map(Choice::Branch)
            .collect();
        if self.mode == Mode::Branch
            && !query.is_empty()
            && !state
                .branches
                .iter()
                .any(|b| b.remote.is_none() && b.name == query)
        {
            choices.push(Choice::Create(query));
        }
        choices
    }
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.active = self
            .active
            .saturating_add_signed(delta)
            .min(self.choices(cx).len().saturating_sub(1));
        cx.notify();
    }
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.blocked.is_some() {
            return;
        }
        let Some(choice) = self.choices(cx).get(ix).cloned() else {
            return;
        };
        match choice {
            Choice::Worktree(tree) => {
                if !tree.missing {
                    cx.emit(Picked::Worktree(tree.path));
                }
            }
            Choice::Branch(branch) if self.mode == Mode::Base => {
                cx.emit(Picked::Base(
                    branch.remote.map_or(branch.name.clone(), |remote| {
                        format!("{remote}/{}", branch.name)
                    }),
                ));
            }
            Choice::Branch(branch)
                if branch.remote.is_none()
                    && self.state.as_ref().and_then(|s| s.current.as_ref())
                        == Some(&branch.name) =>
            {
                cx.emit(Picked::Dismissed)
            }
            choice => {
                self.busy = true;
                self.error = None;
                let cwd = self.cwd.clone();
                let target = match &choice {
                    Choice::Branch(branch) => Some(branch.clone()),
                    _ => None,
                };
                let work = cx.background_executor().spawn(async move {
                    match choice {
                        Choice::Branch(branch) => git::checkout(&cwd, &branch),
                        Choice::Create(name) => {
                            git::create_branch(&cwd, &name).map_err(git::CheckoutError::Other)
                        }
                        _ => unreachable!(),
                    }
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = work.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        match result {
                            Ok(()) => cx.emit(Picked::Changed),
                            Err(git::CheckoutError::LocalChanges(error)) => match target {
                                Some(branch) => this.block(branch, window, cx),
                                None => this.error = Some(error),
                            },
                            Err(git::CheckoutError::Other(error)) => this.error = Some(error),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }
    fn block(&mut self, branch: git::Branch, window: &mut Window, cx: &mut Context<Self>) {
        let message = cx.new(|cx| {
            let mut input = input::Input::commit_message(cx);
            input.set_placeholder(format!("Message ({MOD}↩ to commit)"), cx);
            input
        });
        let subscriptions = [
            cx.subscribe(&message, |_, _, _: &input::Changed, cx| cx.notify()),
            cx.subscribe(&message, |this, _, _: &input::Submitted, cx| {
                this.resolve(Resolve::Commit, cx)
            }),
        ];
        window.focus(&message.focus_handle(cx), cx);
        self.error = None;
        self.blocked = Some(Blocked {
            branch,
            message,
            busy: None,
            generating: None,
            error: None,
            _subscriptions: subscriptions,
        });
    }
    fn can_commit(&self, cx: &gpui::App) -> bool {
        self.blocked.as_ref().is_some_and(|blocked| {
            blocked.idle() && !blocked.message.read(cx).content.trim().is_empty()
        })
    }
    /// generate: a message for the changes that would be committed. Failures
    /// stay in the dialog, like a failed switch.
    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(blocked) = self.blocked.as_mut().filter(|b| b.idle()) else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        blocked.generating = Some(cancel.clone());
        blocked.error = None;
        blocked.message.update(cx, |m, _| m.set_read_only(true));
        let (write, cwd, flag) = (self.write_message.clone(), self.cwd.clone(), cancel.clone());
        let work = cx
            .background_executor()
            .spawn(async move { write(cwd, flag) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                let Some(blocked) = &mut this.blocked else {
                    return;
                };
                blocked.generating = None;
                blocked.message.update(cx, |m, _| m.set_read_only(false));
                match result {
                    Ok(message) => blocked.message.update(cx, |m, cx| m.set_text(message, cx)),
                    Err(error) => blocked.error = Some(error),
                }
                window.focus(&blocked.message.focus_handle(cx), cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn cancel_generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(blocked) = self.blocked.as_mut() else {
            return;
        };
        if let Some(cancel) = blocked.generating.take() {
            cancel.store(true, Ordering::SeqCst);
            blocked.message.update(cx, |m, _| m.set_read_only(false));
            window.focus(&blocked.message.focus_handle(cx), cx);
            cx.notify();
        }
    }
    /// Stash & switch or Commit & switch; a failure stays in the dialog.
    fn resolve(&mut self, kind: Resolve, cx: &mut Context<Self>) {
        if kind == Resolve::Commit && !self.can_commit(cx) {
            return;
        }
        let Some(blocked) = self.blocked.as_mut().filter(|b| b.idle()) else {
            return;
        };
        blocked.busy = Some(kind);
        blocked.error = None;
        blocked.message.update(cx, |m, _| m.set_read_only(true));
        let message = blocked.message.read(cx).content.to_string();
        let branch = blocked.branch.clone();
        let cwd = self.cwd.clone();
        let work = cx.background_executor().spawn(async move {
            match kind {
                Resolve::Stash => git::stash_and_checkout(&cwd, &branch),
                Resolve::Commit => git::commit_and_checkout(&cwd, &branch, &message),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => cx.emit(Picked::Changed),
                    Err(error) => {
                        if let Some(blocked) = &mut this.blocked {
                            blocked.busy = None;
                            blocked.error = Some(error);
                            blocked.message.update(cx, |m, _| m.set_read_only(false));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.blocked.as_ref().is_some_and(|b| !b.idle()) {
            return;
        }
        cx.emit(Picked::Dismissed);
    }
    fn render_blocked(&self, blocked: &Blocked, cx: &mut Context<Self>) -> AnyElement {
        let busy = !blocked.idle();
        let generating = blocked.generating.is_some();
        let can_commit = self.can_commit(cx);
        let generate_label = if generating {
            "Cancel commit message generation"
        } else {
            "Generate commit message"
        };
        let small = |button: gpui::Stateful<gpui::Div>| {
            button
                .h(px(28.))
                .px(px(10.))
                .rounded(px(6.))
                .text_size(px(12.))
        };
        let label = |kind: Resolve, text: &'static str| {
            if blocked.busy == Some(kind) {
                "Switching…"
            } else {
                text
            }
        };
        div()
            .id("switch-branch-dialog")
            .debug_selector(|| "switch-branch-dialog".into())
            .role(Role::Dialog)
            .aria_label(format!("Switch to {}", blocked.branch.name))
            .key_context("GitPicker")
            .size_full()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .on_action(cx.listener(|this, _: &popover::Cancel, _, cx| {
                this.cancel(cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel(cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child("Uncommitted changes"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(content(0.55))
                            .child(format!(
                                "Switching to “{}” would overwrite your local changes. \
                                 Stash them for later, or commit them on this branch first.",
                                blocked.branch.name
                            )),
                    ),
            )
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .w_full()
                            .rounded_md()
                            .bg(content(0.1))
                            .py_1()
                            .pl_2()
                            .pr_8()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(content(1.))
                            .when(busy, |d| d.opacity(0.4))
                            .child(blocked.message.clone()),
                    )
                    .child(
                        div()
                            .id("switch-generate")
                            .debug_selector(|| "switch-generate".into())
                            .role(Role::Button)
                            .aria_label(generate_label)
                            .group("switch-generate")
                            .absolute()
                            .top_1()
                            .right_1()
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(content(0.1))
                            .when(blocked.busy.is_some(), |d| d.opacity(0.4))
                            .when(blocked.busy.is_none(), |d| {
                                d.hover(|s| s.bg(content(0.2))).on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if generating {
                                            this.cancel_generate(window, cx);
                                        } else {
                                            this.generate(window, cx);
                                        }
                                    },
                                ))
                            })
                            .child(if generating {
                                div()
                                    .relative()
                                    .size(px(14.))
                                    .child(
                                        div()
                                            .absolute()
                                            .group_hover("switch-generate", |s| s.opacity(0.))
                                            .child(spinner("switch-generate-spin", "loader", 14.)),
                                    )
                                    .child(
                                        icon("x", 14.)
                                            .absolute()
                                            .text_color(content(1.))
                                            .opacity(0.)
                                            .group_hover("switch-generate", |s| s.opacity(1.)),
                                    )
                                    .into_any_element()
                            } else {
                                icon("wand", 12.).text_color(content(1.)).into_any_element()
                            }),
                    ),
            )
            .when_some(blocked.error.clone(), |d, error| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        crate::modal::disabled(
                            small(crate::modal::ghost_button("switch-cancel", "Cancel")),
                            busy,
                        )
                        .debug_selector(|| "switch-cancel".into())
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        crate::modal::disabled(
                            small(
                                crate::modal::ghost_button(
                                    "switch-commit",
                                    label(Resolve::Commit, "Commit & switch"),
                                )
                                .bg(content(0.1))
                                .text_color(content(1.))
                                .font_weight(gpui::FontWeight::MEDIUM),
                            ),
                            !can_commit,
                        )
                        .debug_selector(|| "switch-commit".into())
                        .on_click(cx.listener(|this, _, _, cx| this.resolve(Resolve::Commit, cx))),
                    )
                    .child(
                        crate::modal::disabled(
                            small(crate::modal::primary_button(
                                "switch-stash",
                                label(Resolve::Stash, "Stash & switch"),
                            )),
                            busy,
                        )
                        .debug_selector(|| "switch-stash".into())
                        .on_click(cx.listener(|this, _, _, cx| this.resolve(Resolve::Stash, cx))),
                    ),
            )
            .into_any_element()
    }
}
impl gpui::Render for GitPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(blocked) = &self.blocked {
            return self.render_blocked(blocked, cx);
        }
        let choices = self.choices(cx);
        let empty = choices.is_empty();
        let rows: Vec<AnyElement> = choices
            .into_iter()
            .enumerate()
            .map(|(ix, choice)| {
                let (label, detail, selected, disabled, path) = match choice {
                    Choice::Branch(branch) => {
                        let full = branch
                            .remote
                            .as_ref()
                            .map_or(branch.name.clone(), |r| format!("{r}/{}", branch.name));
                        let selected = if self.mode == Mode::Base {
                            self.selected.as_ref() == Some(&full)
                        } else {
                            branch.remote.is_none()
                                && self.state.as_ref().and_then(|s| s.current.as_ref())
                                    == Some(&branch.name)
                        };
                        (branch.name, branch.remote, selected, false, "git-branch")
                    }
                    Choice::Create(name) => (
                        format!("Create branch ‘{name}’"),
                        None,
                        false,
                        false,
                        "plus",
                    ),
                    Choice::Worktree(tree) => {
                        let selected = if self.existing_only {
                            self.selected.as_deref().is_some_and(|path| {
                                gespan_core::files::path_key(path)
                                    == gespan_core::files::path_key(&tree.path)
                            })
                        } else {
                            std::fs::canonicalize(&tree.path).ok().is_some_and(|path| {
                                Some(path) == std::fs::canonicalize(&self.cwd).ok()
                            })
                        };
                        (
                            tree.branch.unwrap_or_else(|| {
                                format!(
                                    "Detached {}",
                                    tree.head.chars().take(7).collect::<String>()
                                )
                            }),
                            Some(if tree.main {
                                "Project folder".into()
                            } else {
                                tree.path
                            }),
                            selected,
                            tree.missing,
                            if tree.main {
                                "git-branch"
                            } else {
                                "folder-tree"
                            },
                        )
                    }
                };
                div()
                    .id(("git-option", ix))
                    .debug_selector(move || format!("git-option-{ix}"))
                    .role(Role::ListBoxOption)
                    .aria_label(label.clone())
                    .aria_selected(selected)
                    .flex_shrink_0()
                    .min_h(px(32.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(content(if disabled || self.busy { 0.35 } else { 0.9 }))
                    .when(ix == self.active, |d| d.bg(content(0.08)))
                    .child(icon(path, 14.).text_color(content(0.5)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_size(px(12.)).child(label))
                            .when_some(detail, |d, detail| {
                                d.child(
                                    div()
                                        .truncate()
                                        .text_size(px(10.))
                                        .text_color(content(0.4))
                                        .child(detail),
                                )
                            }),
                    )
                    .when(selected, |d| d.child(icon("check-2", 14.)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.active = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx)))
                    .into_any_element()
            })
            .collect();
        let existing_only = self.existing_only;
        div()
            .id("git-picker")
            .debug_selector(move || {
                if existing_only {
                    "existing-worktrees-submenu".into()
                } else {
                    "git-picker".into()
                }
            })
            .role(Role::Dialog)
            .aria_label(if existing_only {
                "Existing worktrees"
            } else if self.mode == Mode::Worktree {
                "Working copies"
            } else if self.mode == Mode::Base {
                "Worktree base branch"
            } else {
                "Branches"
            })
            .key_context("GitPicker")
            .w_full()
            .when(!existing_only, |d| d.h_full())
            .when(existing_only, |d| d.max_h(px(320.)))
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .capture_action(cx.listener(|this, _: &input::Left, window, cx| {
                if this.existing_only && this.search.read(cx).content.is_empty() {
                    window.dispatch_action(Box::new(popover::CloseSubmenu), cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, window, cx| {
                this.pick(this.active, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|_, _: &popover::Cancel, _, cx| {
                cx.emit(Picked::Dismissed);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(Picked::Dismissed);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(36.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("search", 14.).text_color(content(0.4)))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .child(
                div()
                    .id("git-choices")
                    .role(Role::ListBox)
                    .aria_label("Working copy options")
                    .when(!existing_only, |d| d.flex_1())
                    .when(existing_only, |d| d.max_h(px(284.)))
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(12.))
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child(if self.state.is_none() && self.error.is_none() {
                                    "Loading…"
                                } else if existing_only
                                    && self.search.read(cx).content.trim().is_empty()
                                {
                                    "No existing worktrees"
                                } else {
                                    "No matches"
                                }),
                        )
                    }),
            )
            .when(self.mode == Mode::Branch, |d| {
                d.child(
                    div()
                        .px(px(12.))
                        .py(px(6.))
                        .text_size(px(10.))
                        .text_color(content(0.4))
                        .child("Type a new name to create a branch"),
                )
            })
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .p(px(10.))
                        .text_size(px(11.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(error),
                )
            })
            .into_any_element()
    }
}
