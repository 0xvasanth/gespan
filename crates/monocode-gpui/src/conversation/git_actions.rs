//! The commit pill above the composer. Like the changed-files card, it shows
//! once the session is idle and its working copy has uncommitted changes; one
//! click asks the session's own agent to stage or
//! commit exactly what it changed, leaving other agents' and the user's work
//! alone. The agent, not MonoCode's edit log, decides which changes are its.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GitAction {
    Stage,
    Commit,
    CommitPush,
}

/// The action the pill offers: the last one picked, across sessions.
#[derive(Clone, Copy)]
struct LastGitAction(GitAction);
impl gpui::Global for LastGitAction {}

impl GitAction {
    pub(crate) const ALL: [Self; 3] = [Self::Stage, Self::Commit, Self::CommitPush];
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Commit => "commit",
            Self::CommitPush => "commit-push",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Stage => "Stage",
            Self::Commit => "Commit",
            Self::CommitPush => "Commit & push",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Stage => "Stage only this session's changes",
            Self::Commit => "Stage and commit with a written message",
            Self::CommitPush => "Commit, then push the branch",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Stage => "plus",
            Self::Commit => "git-commit",
            Self::CommitPush => "cloud-upload",
        }
    }
    /// What the transcript shows for the turn.
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Stage => "Stage your changes",
            Self::Commit => "Commit your changes",
            Self::CommitPush => "Commit and push your changes",
        }
    }
    /// What the agent receives instead.
    pub(crate) fn prompt(self) -> String {
        let task = match self {
            Self::Stage => "Stage the changes you made in this session with git. Do not commit.",
            Self::Commit => "Commit the changes you made in this session with git.",
            Self::CommitPush => {
                "Commit the changes you made in this session with git, then push the branch."
            }
        };
        let mut rules = vec![
            "Work out which files you created, edited, renamed or deleted in this session from this conversation, then check them against `git status` and `git diff`.",
            "Name every path explicitly. Never use `git add .`, `git add -A`, `git add -u`, `git commit -a` or any other repository-wide form.",
            "If a file also holds changes that are not yours, take only your own hunks when you can separate them reliably (for example with `git apply --cached`). Otherwise leave that file out and say why.",
            "Leave everything else as it is: do not unstage, stash, reset, check out, or discard any other change.",
        ];
        if self != Self::Stage {
            rules.extend([
                "Commit only your paths, listing them explicitly (`git commit -m \"…\" -- <paths>`), so anything someone else staged stays out of the commit. If you staged partial hunks, check that `git diff --cached` holds nothing but your changes and commit the index instead.",
                "Write the message from what you did and why: a short summary line, then a body only if the change needs explaining. Follow the style of `git log`.",
                "Do not amend, and do not skip hooks. If a hook fails because of your change, fix it and retry; otherwise stop and report the failure.",
            ]);
        }
        rules.push(match self {
            Self::Stage | Self::Commit => "Do not push.",
            Self::CommitPush => {
                "Then push the current branch (`git push`, or `git push -u origin <branch>` when it has no upstream). Never force-push. If the push is rejected, stop and report it; do not pull, rebase or merge."
            }
        });
        let report = match self {
            Self::Stage => "the output of `git diff --cached --stat`",
            Self::Commit => "the commit's short hash and summary line, and the files it includes",
            Self::CommitPush => {
                "the commit's short hash and summary line, the files it includes, and where it was pushed"
            }
        };
        format!(
            "{task}\n\nOther agents and the user may be working in this same checkout, so the working tree can hold changes that are not yours. Include only what you changed:\n\n{}\n\nIf you changed nothing in this session, say so and stop. Otherwise finish with a short reply listing {report}, plus any changes you deliberately left out.",
            rules
                .iter()
                .map(|rule| format!("- {rule}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

impl Conversation {
    /// The Shell's Git watcher: whether this session's working copy is dirty.
    pub(crate) fn set_uncommitted(&mut self, uncommitted: bool, cx: &mut Context<Self>) {
        if self.uncommitted != uncommitted {
            self.uncommitted = uncommitted;
            cx.notify();
        }
    }
    fn last_git_action(cx: &gpui::App) -> GitAction {
        cx.try_global::<LastGitAction>()
            .map_or(GitAction::Commit, |last| last.0)
    }
    /// Idle with uncommitted changes: no turn running and nothing queued, so
    /// a click starts at once and the pill hides until the turn is over.
    fn git_actions_shown(&self) -> bool {
        self.uncommitted
            && self.loaded
            && !self.busy
            && !self.stopping
            && self.follow_ups.state.messages.is_empty()
            && !self.is_remote()
            && self.settings.mono.is_none()
            && !self.settings.worktree_removed
    }
    pub(super) fn git_action_menu_items(&self, cx: &gpui::App) -> Vec<MenuItem> {
        let current = Self::last_git_action(cx);
        GitAction::ALL
            .into_iter()
            .map(|action| {
                MenuAction::new(
                    SharedString::from(format!("git-action:{}", action.id())),
                    action.label(),
                )
                .icon(action.icon())
                .description(action.description())
                .checked(action == current)
                .into()
            })
            .collect()
    }
    /// A menu pick: `true` when `id` was one of the pill's actions.
    pub(super) fn git_action_choice(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(action) = id.strip_prefix("git-action:").and_then(GitAction::from_id) else {
            return false;
        };
        self.run_git_action(action, cx);
        true
    }
    /// Sends the action through the follow-up queue, which starts it straight
    /// away on an idle session without the composer's modes or attachments.
    pub(crate) fn run_git_action(&mut self, action: GitAction, cx: &mut Context<Self>) {
        if !self.git_actions_shown() {
            return;
        }
        cx.set_global(LastGitAction(action));
        let mut message = self.message(action.message().to_owned(), Vec::new(), None, cx);
        message.prepared_prompt = Some(action.prompt());
        self.follow_ups.motion.insert(
            &message.id,
            self.follow_ups.state.messages.is_empty(),
            !cx.reduce_motion(),
        );
        self.follow_ups.state.messages.push(message);
        self.save_follow_ups(cx);
        cx.notify();
    }
    /// The split pill: run the current action, or pick another from the
    /// chevron, like the footer's Actions button.
    pub(super) fn git_action_pill(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.git_actions_shown() {
            return None;
        }
        let current = Self::last_git_action(cx);
        let menu_open = self.menu_is(ComposerMenu::GitActions);
        let segment = |id: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .h_full()
                .flex()
                .items_center()
                .rounded_full()
                .text_color(content(0.7))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let main = segment("git-action-run")
            .aria_label(SharedString::from(current.message()))
            .pl(px(10.))
            .pr(px(8.))
            .gap(px(6.))
            .child(icon(current.icon(), 14.).text_color(content(0.55)))
            .child(div().text_size(px(12.)).child(current.label()))
            .on_click(cx.listener(move |this, _, _, cx| this.run_git_action(current, cx)));
        let chevron = segment("git-action-menu")
            .aria_label("Git actions")
            .aria_expanded(menu_open)
            .relative()
            .pl(px(6.))
            .pr(px(8.))
            .when(menu_open, |d| d.bg(content(0.06)).text_color(content(1.)))
            .child(self.git_action_anchor.probe())
            .child(icon("chevron-down", 14.))
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_menu(ComposerMenu::GitActions, window, cx)
            }));
        Some(
            div()
                .id("git-actions")
                .debug_selector(|| "git-actions".into())
                .role(Role::Group)
                .aria_label("Git actions")
                .relative()
                .h(px(30.))
                .flex()
                .items_stretch()
                .rounded_full()
                .border_1()
                .border_color(content(0.1))
                .child(crate::transcript::floating_surface())
                .child(main)
                .child(div().w_px().my(px(7.)).bg(content(0.1)))
                .child(chevron)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use monocode_core::store::Store;
    use std::sync::Mutex;

    #[gpui::test]
    fn the_pill_shows_only_while_idle_with_changes_and_sends_the_hidden_prompt(
        cx: &mut TestAppContext,
    ) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("claude", "/work/monocode").unwrap();
        let window = cx.update(|cx| {
            input::bind(cx);
            crate::popover::bind(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    view
                })
            })
            .unwrap()
        });
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let shown = |visual: &mut gpui::VisualTestContext| {
            visual.run_until_parked();
            visual.debug_bounds("git-actions").is_some()
        };
        assert!(!shown(visual), "no changes");

        window
            .update(visual, |view, _, cx| {
                view.set_uncommitted(true, cx);
                view.busy = true;
                cx.notify();
            })
            .unwrap();
        assert!(!shown(visual), "the agent is still working");

        window
            .update(visual, |view, _, cx| {
                view.busy = false;
                cx.notify();
            })
            .unwrap();
        assert!(shown(visual), "idle with changes");

        window
            .update(visual, |view, _, cx| {
                view.run_git_action(GitAction::Commit, cx);
                // The pill is gone, so a second click cannot send it twice.
                view.run_git_action(GitAction::Stage, cx);
                let [message] = view.follow_ups.state.messages.as_slice() else {
                    panic!("exactly one action sent");
                };
                assert_eq!(message.text, "Commit your changes");
                assert_eq!(message.prepared_prompt, Some(GitAction::Commit.prompt()));
                assert!(!message.steer);
            })
            .unwrap();
        assert!(visual.debug_bounds("git-actions").is_none());
    }

    #[test]
    fn prompts_scope_the_work_to_the_agents_own_changes() {
        for action in GitAction::ALL {
            let prompt = action.prompt();
            assert!(prompt.contains("Never use `git add .`"));
            assert!(prompt.contains("only what you changed"));
            assert_eq!(
                prompt.contains("-- <paths>"),
                action != GitAction::Stage,
                "{action:?}"
            );
            assert_eq!(
                prompt.contains("Never force-push"),
                action == GitAction::CommitPush,
                "{action:?}"
            );
        }
        assert!(GitAction::Stage.prompt().contains("Do not commit."));
    }
}
