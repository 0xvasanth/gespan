//! Offers to delete a worktree with its last session, as App.tsx's
//! onRemoveHistorySession does. A chat lives in one window, so its window asks
//! and removes; the last check looks at every window.
use super::*;
use gespan_core::{
    agent::Settings,
    files::{is_equal_or_inside, path_key},
    worktree_management,
};
use std::{path::Path, sync::Weak, time::Duration};

const ANOTHER_SESSION: &str = "Another session is using this worktree.";
const AGENT_POLL: Duration = Duration::from_millis(50);
/// Ten seconds: a stopped agent exits long before this.
const AGENT_POLLS: u32 = 200;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Removal {
    Archive,
    Delete,
}
impl Removal {
    fn done(self) -> &'static str {
        match self {
            Self::Archive => "archived",
            Self::Delete => "deleted",
        }
    }
}
/// What the removal prompt says when it has to ask anyway.
#[derive(Clone)]
pub(super) struct Ask {
    pub title: String,
    pub detail: String,
}
pub(super) struct UnusedWorktree {
    project: String,
    path: String,
}
pub(super) struct WorktreeCleanup {
    worktree: UnusedWorktree,
    removal: Removal,
    /// Ends once the session's agent has exited.
    turn: Weak<()>,
}

impl Shell {
    /// The session's own worktree, if it has one this app can remove.
    fn session_worktree(&self, id: i64, cx: &App) -> Option<UnusedWorktree> {
        let session = self.session_metadata(id, cx);
        let project = &session.summary.cwd;
        let path = self.session_working_copy(id, cx);
        (path_key(path) != path_key(project)
            && !project.starts_with("remote://")
            && !path.starts_with("remote://"))
        .then(|| UnusedWorktree {
            project: project.clone(),
            path: path.to_owned(),
        })
    }
    fn worktree_users(&self, cx: &App) -> Vec<(i64, String)> {
        self.session_summaries
            .keys()
            .map(|id| (*id, self.session_working_copy(*id, cx).to_owned()))
            .collect()
    }
    fn worktree_in_use(&self, path: &str, id: i64, cx: &App) -> Option<&'static str> {
        if self
            .terminals
            .values()
            .flatten()
            .any(|terminal| is_equal_or_inside(&terminal.read(cx).cwd, path))
        {
            return Some("A terminal is still open in this worktree.");
        }
        (!worktree_management::only_used_by(path, id, &self.worktree_users(cx)))
            .then_some(ANOTHER_SESSION)
    }

    /// Archives a chat, offering to delete its worktree if nothing else uses it.
    pub(super) fn archive_session(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_metadata(id, cx).summary.archived {
            self.toggle_archive(id, cx);
        } else {
            self.remove_session(id, Removal::Archive, None, window, cx);
        }
    }
    /// Removes a chat. `ask` confirms even when there is no worktree to offer.
    pub(super) fn remove_session(
        &mut self,
        id: i64,
        removal: Removal,
        ask: Option<Ask>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ask.is_none() && self.session_worktree(id, cx).is_none() {
            return self.apply_removal(id, removal, None, window, cx);
        }
        // The window running the chat asks, stops it and removes it.
        let routed = ask.clone();
        if crate::windows::on_owner(id, cx.entity_id(), cx, move |shell, window, cx| {
            shell.remove_session(id, removal, routed, window, cx)
        }) {
            return;
        }
        if self.removal_prompt {
            return;
        }
        self.removal_prompt = true;
        let worktree = self.session_worktree(id, cx);
        let users = self.worktree_users(cx);
        let title = self.session_metadata(id, cx).display_title();
        let unused = cx.background_executor().spawn(async move {
            let worktree = worktree?;
            worktree_management::unused_after(
                Path::new(&worktree.project),
                Path::new(&worktree.path),
                id,
                &users,
            )
            .map(|_| worktree)
        });
        cx.spawn_in(window, async move |this, cx| {
            let unused = unused.await;
            let answer = this.update_in(cx, |_, window, cx| {
                removal_prompt(removal, ask, unused.as_ref(), &title, window, cx)
            });
            let answer = match answer {
                Ok(Some(prompt)) => prompt.await,
                Ok(None) => Some(0),
                Err(_) => None,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.removal_prompt = false;
                let worktree = match (answer, unused) {
                    (Some(0), _) => None,
                    (Some(1), Some(unused)) => Some(unused),
                    _ => return,
                };
                this.apply_removal(id, removal, worktree, window, cx);
            });
        })
        .detach();
    }
    fn apply_removal(
        &mut self,
        id: i64,
        removal: Removal,
        worktree: Option<UnusedWorktree>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let turn = self
            .sessions
            .get(&id)
            .map_or_else(Weak::new, |view| view.read(cx).turn_exit());
        // Otherwise closing the tab opens a blank chat in the same worktree.
        if let Some(worktree) = &worktree {
            let focused = self.worktree_focus.get(&self.workspace.project);
            if focused.is_some_and(|focus| is_equal_or_inside(&focus.path, &worktree.path)) {
                self.select_worktree(None, window, cx);
            }
        }
        match removal {
            Removal::Archive => self.toggle_archive(id, cx),
            Removal::Delete => self.delete_session(id, window, cx),
        }
        let started = match removal {
            Removal::Archive => self.archiving.contains(&id),
            Removal::Delete => self.deleting.contains(&id),
        };
        if let (true, Some(worktree)) = (started, worktree) {
            self.worktree_cleanups.insert(
                id,
                WorktreeCleanup {
                    worktree,
                    removal,
                    turn,
                },
            );
        }
    }
    /// The session is archived or deleted. Removes the worktree it took along.
    pub(super) fn session_removed(&mut self, id: i64, removed: bool, cx: &mut Context<Self>) {
        let Some(cleanup) = self.worktree_cleanups.remove(&id).filter(|_| removed) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let WorktreeCleanup {
                worktree,
                removal,
                turn,
            } = cleanup;
            let mut polls = 0;
            while turn.strong_count() > 0 && polls < AGENT_POLLS {
                cx.background_executor().timer(AGENT_POLL).await;
                polls += 1;
            }
            // Last, so nothing is awaited between this check and the removal.
            let checked = this.update(cx, |this, cx| {
                if turn.strong_count() > 0 {
                    return Err("An agent is still running in this worktree.".to_owned());
                }
                let reopened = removal == Removal::Archive
                    && (this.workspace.tab_of(id).is_some()
                        || this
                            .session_summaries
                            .get(&id)
                            .is_some_and(|summary| !summary.archived));
                if reopened {
                    return Err("The session was opened again.".to_owned());
                }
                let elsewhere = crate::windows::handles_elsewhere(cx.entity_id(), cx);
                let in_use = this.worktree_in_use(&worktree.path, id, cx).or_else(|| {
                    elsewhere.iter().find_map(|window| {
                        let shell = window.read(cx).ok()?;
                        shell.worktree_in_use(&worktree.path, id, cx)
                    })
                });
                in_use.map_or(Ok(this.store.clone()), |error| Err(error.to_owned()))
            });
            let Ok(checked) = checked else {
                return;
            };
            let path = worktree.path.clone();
            let result = match checked {
                Ok(store) => {
                    let project = worktree.project;
                    cx.background_executor()
                        .spawn(async move { remove_unused(&store, id, &project, &path) })
                        .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(detached) => {
                        this.worktree_detached(detached, &worktree.path, cx);
                        crate::windows::sessions_changed(cx.entity_id(), cx);
                    }
                    Err(error) => {
                        this.error = Some(format!(
                            "The session was {}. Its worktree was kept.\n\n{error}\n\nYou can manage it in Settings → Worktrees.",
                            removal.done()
                        ));
                    }
                }
                this.sync_git(cx);
                this.sync_settings(cx);
                this.invalidate_chrome(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

/// `None` when there is nothing to ask.
fn removal_prompt(
    removal: Removal,
    ask: Option<Ask>,
    unused: Option<&UnusedWorktree>,
    title: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<impl Future<Output = Option<usize>> + use<>> {
    if ask.is_none() && unused.is_none() {
        return None;
    }
    let (verb, level) = match removal {
        Removal::Archive => ("Archive", gpui::PromptLevel::Info),
        Removal::Delete => ("Delete", gpui::PromptLevel::Warning),
    };
    let (heading, mut detail) = match ask {
        Some(ask) => (ask.title, ask.detail),
        None => (format!("{verb} “{title}”?"), String::new()),
    };
    if let Some(unused) = unused {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(&format!(
            "No other session uses the worktree at {}. Its branch is kept. If files have uncommitted changes, the worktree stays.",
            unused.path
        ));
    }
    let buttons: &[&str] = match unused {
        Some(_) => &[verb, "Also Delete Worktree", "Cancel"],
        None => &[verb, "Cancel"],
    };
    let answer = window.prompt(level, &heading, Some(&detail), buttons, cx);
    Some(async move { answer.await.ok() })
}

/// Checks the saved sessions once more, then removes the worktree and keeps
/// any record of it detached.
fn remove_unused(
    store: &Database,
    id: i64,
    project: &str,
    path: &str,
) -> Result<Vec<(Session, Settings)>, String> {
    let users = {
        let store = locked(store)?;
        let paths = store.session_worktrees()?;
        let sessions = store.sessions()?;
        sessions
            .into_iter()
            .map(|session| {
                let cwd = paths.get(&session.id).cloned().flatten();
                (session.id, cwd.unwrap_or(session.cwd))
            })
            .collect::<Vec<_>>()
    };
    if !worktree_management::only_used_by(path, id, &users) {
        return Err(ANOTHER_SESSION.into());
    }
    let snapshot = gespan_core::git::snapshot(Path::new(project))?;
    let main = snapshot
        .worktrees
        .first()
        .ok_or("No main working copy found")?;
    worktree_management::remove_unused(Path::new(&main.path), Path::new(path))?;
    locked(store)?.detach_worktree(path, &main.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::{store::Store, worktree_management::default_root};
    use gpui::{TestAppContext, WindowHandle};

    struct Fixture {
        window: WindowHandle<Shell>,
        repo: tempfile::TempDir,
        path: String,
        /// The chat in the worktree.
        chat: i64,
        /// A chat in the main checkout, so closing the worktree's tab has
        /// somewhere to go.
        main: i64,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(default_root(self.repo.path()));
        }
    }
    fn git(repo: &Path, args: &[&str]) -> bool {
        let git = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output();
        git.is_ok_and(|output| output.status.success())
    }
    fn fixture(cx: &mut TestAppContext) -> Fixture {
        let repo = tempfile::tempdir().unwrap();
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.email", "fixture@example.invalid"],
            &["config", "user.name", "Fixture"],
        ] {
            assert!(git(repo.path(), args));
        }
        std::fs::write(repo.path().join("file"), "before").unwrap();
        assert!(git(repo.path(), &["add", "file"]));
        assert!(git(repo.path(), &["commit", "-m", "Initial"]));
        let tree = gespan_core::git::create_named_worktree(repo.path(), "feature").unwrap();
        let project = repo.path().to_string_lossy().into_owned();

        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut chat = store.create("codex", &project).unwrap();
        let main = store.create("codex", &project).unwrap();
        store
            .write_rows(
                chat.id,
                0,
                &[gespan_core::transcript::Row {
                    role: "assistant".into(),
                    text: "Worth keeping".into(),
                    first: true,
                    sent_at: None,
                }],
            )
            .unwrap();
        chat.has_messages = true;
        bind(&store, chat.id, &tree.path);
        let ids = (chat.id, main.id);
        let mut state = Workspace::default();
        state.add_project(&project);
        state.open(ids.0);
        state.open(ids.1);
        let saved = serde_json::to_string(&state).unwrap();
        let store = Arc::new(Mutex::new(store));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut shell = Shell::new(store, vec![chat, main], project, Some(saved), cx);
                    shell.ensure_session(ids.0, cx);
                    shell
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        Fixture {
            window,
            repo,
            path: tree.path,
            chat: ids.0,
            main: ids.1,
        }
    }
    fn bind(store: &Store, id: i64, path: &str) {
        let mut settings = store.agent_settings(id).unwrap().unwrap_or_default();
        settings.worktree_cwd = Some(path.to_owned());
        store.save_agent_settings(id, &settings).unwrap();
    }
    impl Fixture {
        fn run(&self, removal: Removal, cx: &mut TestAppContext) {
            let id = self.chat;
            self.window
                .update(cx, |shell, window, cx| {
                    shell.remove_session(id, removal, None, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
        }
        fn answer(&self, button: &str, cx: &mut TestAppContext) {
            assert!(cx.has_pending_prompt(), "the offer was not made");
            cx.simulate_prompt_answer(button);
            cx.run_until_parked();
        }
        fn exists(&self) -> bool {
            Path::new(&self.path).exists()
        }
        fn branch_kept(&self) -> bool {
            git(
                self.repo.path(),
                &["show-ref", "--verify", "refs/heads/feature"],
            )
        }
        fn saved_settings(&self, id: i64, cx: &mut TestAppContext) -> Settings {
            self.window
                .update(cx, |shell, _, _| {
                    let store = locked(&shell.store).unwrap();
                    store.agent_settings(id).unwrap().unwrap_or_default()
                })
                .unwrap()
        }
        fn archived(&self, cx: &mut TestAppContext) -> bool {
            let id = self.chat;
            self.window
                .update(cx, |shell, _, _| {
                    let store = locked(&shell.store).unwrap();
                    let sessions = store.sessions().unwrap();
                    sessions.iter().find(|s| s.id == id).unwrap().archived
                })
                .unwrap()
        }
        fn error(&self, cx: &mut TestAppContext) -> Option<String> {
            self.window
                .update(cx, |shell, _, _| shell.error.clone())
                .unwrap()
        }
    }

    #[gpui::test]
    fn archiving_the_last_session_can_take_its_worktree_and_keeps_the_record(
        cx: &mut TestAppContext,
    ) {
        let f = fixture(cx);
        f.run(Removal::Archive, cx);
        f.answer("Also Delete Worktree", cx);
        assert!(!f.exists());
        assert!(f.branch_kept());
        assert!(f.archived(cx));
        let settings = f.saved_settings(f.chat, cx);
        assert!(settings.worktree_removed, "the record is detached");
        let id = f.chat;
        f.window
            .update(cx, |shell, _, cx| {
                assert!(shell.sessions[&id].read(cx).settings.worktree_removed);
                let rows = locked(&shell.store).unwrap().load(id).unwrap().rows;
                assert_eq!(rows.len(), 1, "the transcript stays");
            })
            .unwrap();
        assert_eq!(f.error(cx), None);
    }

    #[gpui::test]
    fn deleting_the_last_session_can_take_its_worktree(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.run(Removal::Delete, cx);
        f.answer("Also Delete Worktree", cx);
        assert!(!f.exists());
        assert!(f.branch_kept());
        let id = f.chat;
        f.window
            .update(cx, |shell, _, _| {
                let store = locked(&shell.store).unwrap();
                assert!(!store.sessions().unwrap().iter().any(|s| s.id == id));
            })
            .unwrap();
        assert_eq!(f.error(cx), None);
    }

    #[gpui::test]
    fn the_plain_choice_leaves_the_worktree_and_cancel_leaves_everything(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.run(Removal::Archive, cx);
        f.answer("Cancel", cx);
        assert!(f.exists());
        assert!(!f.archived(cx));

        f.run(Removal::Archive, cx);
        f.answer("Archive", cx);
        assert!(f.exists());
        assert!(f.archived(cx));
        assert!(!f.saved_settings(f.chat, cx).worktree_removed);
    }

    #[gpui::test]
    fn a_shared_worktree_is_not_offered_and_nothing_asks(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let (other, path) = (f.main, f.path.clone());
        f.window
            .update(cx, |shell, _, cx| {
                bind(&locked(&shell.store).unwrap(), other, &path);
                shell.session_worktrees.insert(other, Some(path));
                cx.notify();
            })
            .unwrap();
        f.run(Removal::Archive, cx);
        assert!(!cx.has_pending_prompt());
        assert!(f.archived(cx));
        assert!(f.exists());
    }

    #[gpui::test]
    fn a_session_that_takes_the_worktree_after_the_offer_keeps_it(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.run(Removal::Archive, cx);
        let (other, path) = (f.main, f.path.clone());
        // Saved only, so the last check before git has to catch it.
        f.window
            .update(cx, |shell, _, _| {
                bind(&locked(&shell.store).unwrap(), other, &path)
            })
            .unwrap();
        f.answer("Also Delete Worktree", cx);
        assert!(f.exists());
        assert!(f.archived(cx));
        assert!(f.error(cx).unwrap().contains(ANOTHER_SESSION));
    }

    #[gpui::test]
    fn a_session_in_another_window_that_took_the_worktree_keeps_it(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.run(Removal::Archive, cx);
        let (other, path) = (f.main, f.path.clone());
        cx.update(crate::windows::open_new_window);
        cx.run_until_parked();
        let second = cx
            .update(|cx| {
                let windows = cx.windows().into_iter();
                windows
                    .filter_map(|window| window.downcast::<Shell>())
                    .find(|window| *window != f.window)
            })
            .expect("a second window");
        // Only that window has heard about it.
        second
            .update(cx, |shell, _, _| {
                shell.session_worktrees.insert(other, Some(path));
            })
            .unwrap();
        f.answer("Also Delete Worktree", cx);
        assert!(f.exists());
        assert!(f.error(cx).unwrap().contains(ANOTHER_SESSION));
    }

    #[gpui::test]
    fn uncommitted_changes_keep_the_worktree(cx: &mut TestAppContext) {
        let f = fixture(cx);
        std::fs::write(Path::new(&f.path).join("draft"), "unsaved work").unwrap();
        f.run(Removal::Delete, cx);
        f.answer("Also Delete Worktree", cx);
        assert!(Path::new(&f.path).join("draft").exists());
        let error = f.error(cx).unwrap();
        assert!(error.starts_with("The session was deleted. Its worktree was kept."));
        assert!(error.contains("uncommitted changes"));
    }

    #[gpui::test]
    fn the_worktree_waits_for_the_agent_to_exit(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let id = f.chat;
        let turn = Arc::new(());
        let alive = Arc::downgrade(&turn);
        f.window
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].update(cx, |view, _| view.set_turn_exit_for_test(alive))
            })
            .unwrap();
        f.run(Removal::Archive, cx);
        f.answer("Also Delete Worktree", cx);
        cx.executor().advance_clock(AGENT_POLL * 3);
        cx.run_until_parked();
        assert!(f.exists(), "its agent is still running");

        drop(turn);
        cx.executor().advance_clock(AGENT_POLL * 2);
        cx.run_until_parked();
        assert!(!f.exists());
        assert_eq!(f.error(cx), None);
    }

    #[gpui::test]
    fn an_agent_that_never_exits_keeps_the_worktree(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let id = f.chat;
        let turn = Arc::new(());
        let alive = Arc::downgrade(&turn);
        f.window
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].update(cx, |view, _| view.set_turn_exit_for_test(alive))
            })
            .unwrap();
        f.run(Removal::Archive, cx);
        f.answer("Also Delete Worktree", cx);
        cx.executor().advance_clock(AGENT_POLL * (AGENT_POLLS + 2));
        cx.run_until_parked();
        assert!(f.exists());
        assert!(f.error(cx).unwrap().contains("still running"));
    }

    #[gpui::test]
    fn a_focused_worktree_does_not_get_a_blank_chat_in_the_way(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let (id, main, path) = (f.chat, f.main, f.path.clone());
        f.window
            .update(cx, |shell, window, cx| {
                shell.select_worktree(
                    Some(crate::worktree_switcher::WorktreeFocus {
                        path,
                        branch: Some("feature".into()),
                    }),
                    window,
                    cx,
                );
                shell.open(id, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        f.run(Removal::Archive, cx);
        f.answer("Also Delete Worktree", cx);
        assert!(!f.exists());
        assert_eq!(f.error(cx), None);
        f.window
            .update(cx, |shell, _, _| {
                assert!(shell.worktree_focus.is_empty());
                assert!(shell.session_summaries.contains_key(&main));
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_chat_in_the_main_checkout_is_removed_without_asking(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let main = f.main;
        f.window
            .update(cx, |shell, window, cx| {
                shell.remove_session(main, Removal::Archive, None, window, cx);
                assert!(shell.archiving.contains(&main), "no lookup in the way");
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
    }

    #[gpui::test]
    fn a_confirmed_delete_still_asks_without_a_worktree(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let main = f.main;
        f.window
            .update(cx, |shell, window, cx| {
                // A blank chat would just be reused for the tab it leaves empty.
                shell.sessions[&main].update(cx, |view, _| view.summary.has_messages = true);
                let ask = Some(Ask {
                    title: "Delete it?".into(),
                    detail: "It goes for good.".into(),
                });
                shell.remove_session(main, Removal::Delete, ask, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        f.answer("Delete", cx);
        f.window
            .update(cx, |shell, _, _| {
                assert!(!shell.sessions.contains_key(&main))
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_prompt_shows_in_the_window_running_the_chat(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let id = f.chat;
        cx.update(crate::windows::open_new_window);
        cx.run_until_parked();
        let second = cx
            .update(|cx| {
                let windows = cx.windows().into_iter();
                windows
                    .filter_map(|window| window.downcast::<Shell>())
                    .find(|window| *window != f.window)
            })
            .expect("a second window");
        second
            .update(cx, |shell, window, cx| {
                shell.remove_session(id, Removal::Archive, None, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        f.answer("Also Delete Worktree", cx);
        assert!(!f.exists());
        f.window
            .update(cx, |shell, _, _| {
                assert!(shell.session_summaries[&id].archived)
            })
            .unwrap();
    }
}
