//! Background session names never join or rewrite the user's provider history.
use super::*;
use gespan_core::session_title::display_title;

impl Conversation {
    /// Tauri's sessionDisplayTitle: the harness prefix belongs to persistence,
    /// while every conversation headline shows only the user's title.
    pub(crate) fn display_title(&self) -> String {
        display_title(&self.summary.title, &self.summary.provider).replace('\n', " ")
    }
    pub(crate) fn rename(&mut self, title: String, cx: &mut Context<Self>) {
        self.cancel_title_generation();
        self.summary.title = title;
        cx.emit(SessionChanged);
        cx.notify();
    }
    pub(super) fn cancel_title_generation(&mut self) {
        if let Some(cancel) = self.title_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn generate_title(&mut self, message: &str, cx: &mut Context<Self>) {
        if self.summary.has_messages
            || self.settings.pilot.is_some()
            || self.summary.title
                != format!(
                    "New {} session",
                    Kind::from_id(&self.summary.provider).label()
                )
        {
            return;
        }
        self.cancel_title_generation();
        let placeholder = self.summary.title.clone();
        let seed = message
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(42)
            .collect::<String>();
        if seed.is_empty() {
            return;
        }
        self.summary.title = seed.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.title_cancel = Some(cancel.clone());
        let kind = Kind::from_id(&self.summary.provider);
        let cwd = self
            .settings
            .worktree_cwd
            .clone()
            .unwrap_or_else(|| self.summary.cwd.clone());
        let message = message.to_owned();
        let account = self.settings.account.clone();
        let id = self.summary.id;
        let store = self.store.clone();
        let expected = seed.clone();
        let flag = cancel.clone();
        let work = cx.background_executor().spawn(async move {
            // Persist the fallback before generation so a fast response cannot
            // race its initial write, and never overwrite a saved manual name.
            if !locked(&store)
                .and_then(|s| s.replace_title(id, &placeholder, &expected))
                .unwrap_or(false)
            {
                return None;
            }
            // GPUI unit tests use synthetic conversations. Live-provider checks
            // are opt-in; the core runner is tested with isolated CLI fixtures.
            if cfg!(test) {
                None
            } else {
                gespan_core::session_title::generate_for(
                    kind,
                    Path::new(&cwd),
                    &message,
                    account,
                    flag,
                )
            }
        });
        cx.spawn(async move |this, cx| {
            let generated = work.await;
            let _ = this.update(cx, |this, cx| {
                if !cancel.load(Ordering::Relaxed)
                    && let Some(generated) = generated
                {
                    this.apply_generated_title(&seed, generated.title, cx);
                }
            });
        })
        .detach();
    }
    pub(super) fn apply_generated_title(
        &mut self,
        seed: &str,
        title: String,
        cx: &mut Context<Self>,
    ) {
        if self.summary.title != seed {
            return;
        }
        self.summary.title = title.clone();
        let store = self.store.clone();
        let id = self.summary.id;
        let seed = seed.to_owned();
        cx.background_executor()
            .spawn(async move {
                let _ = locked(&store).and_then(|s| s.replace_title(id, &seed, &title));
            })
            .detach();
        cx.emit(SessionChanged);
        cx.notify();
    }
}

impl Conversation {
    /// Tauri renames a new worktree's placeholder branch after the work its
    /// first message asks for. Local sessions only: a remote host's branches
    /// stay as created, as they do in Tauri.
    pub(super) fn name_worktree_branch(&mut self, worktree: String, cx: &mut Context<Self>) {
        let Some(message) = self.branch_seed.take() else {
            return;
        };
        if self.is_remote() || self.settings.pilot.is_some() {
            return;
        }
        let kind = Kind::from_id(&self.summary.provider);
        let account = self.settings.account.clone();
        let root = self.summary.cwd.clone();
        let tree = worktree.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(previous) = self.branch_cancel.replace(cancel.clone()) {
            previous.store(true, Ordering::Relaxed);
        }
        #[cfg(test)]
        let suggested = self.test_branch_name.clone();
        let flag = cancel.clone();
        let work = cx.background_executor().spawn(async move {
            // Read the placeholder first, so a rename the user makes while the
            // name is generated is kept.
            let from = gespan_core::git::worktree_branch(Path::new(&root), Path::new(&tree))?;
            #[cfg(test)]
            let fragment = {
                let _ = (kind, account, &message, &flag);
                suggested
            };
            // GPUI unit tests have no live provider, as for titles.
            #[cfg(not(test))]
            let fragment = gespan_core::session_title::generate_branch_name(
                kind,
                Path::new(&tree),
                &message,
                account,
                flag.clone(),
            );
            fragment
                .filter(|_| !flag.load(Ordering::Relaxed))
                .map(|fragment| (from, fragment))
        });
        cx.spawn(async move |this, cx| {
            if let Some((from, fragment)) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.rename_worktree_branch(worktree, from, fragment, cx)
                });
            }
        })
        .detach();
    }
    /// Gives the session's worktree branch the generated name, then shows it.
    pub(super) fn rename_worktree_branch(
        &mut self,
        worktree: String,
        from: String,
        fragment: String,
        cx: &mut Context<Self>,
    ) {
        let root = self.summary.cwd.clone();
        let tree = worktree.clone();
        let work = cx.background_executor().spawn(async move {
            gespan_core::git::rename_app_branch(
                Path::new(&root),
                Path::new(&tree),
                &from,
                &fragment,
            )
        });
        cx.spawn(async move |this, cx| {
            // A failed rename keeps the placeholder; the session works the same.
            if work.await.is_ok() {
                let _ = this.update(cx, |this, cx| {
                    if this.execution_cwd() == worktree {
                        this.refresh_git(cx);
                    }
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_removes_only_the_sessions_own_canonical_provider_prefix() {
        assert_eq!(display_title("codex · Fix sidebar", "codex"), "Fix sidebar");
        assert_eq!(
            display_title("claude · Fix sidebar", "claude"),
            "Fix sidebar"
        );
        assert_eq!(
            display_title("claude · Notes about Claude", "codex"),
            "claude · Notes about Claude"
        );
        assert_eq!(
            display_title("My codex · notes", "codex"),
            "My codex · notes"
        );
        assert_eq!(
            display_title("New feature proposal", "codex"),
            "New feature proposal"
        );
        for title in ["codex", "Codex", "New Codex session"] {
            assert_eq!(display_title(title, "codex"), "New session");
        }
    }
    #[gpui::test]
    fn late_generated_titles_preserve_manual_names_and_conversation_history(
        cx: &mut gpui::TestAppContext,
    ) {
        let store = Arc::new(std::sync::Mutex::new(
            gespan_core::store::Store::open(Path::new(":memory:")).unwrap(),
        ));
        let summary = locked(&store).unwrap().create("codex", "/tmp").unwrap();
        let id = summary.id;
        let view = cx.new(|cx| Conversation::new(summary, store.clone(), cx));
        view.update(cx, |v, cx| {
            v.summary.title = "The first prompt".into();
            v.apply_generated_title("The first prompt", "Improve session filtering".into(), cx);
        });
        cx.run_until_parked();
        view.update(cx, |v, cx| {
            assert_eq!(v.summary.title, "Improve session filtering");
            let cancelled = Arc::new(AtomicBool::new(false));
            v.title_cancel = Some(cancelled.clone());
            v.rename("My session name".into(), cx);
            assert!(cancelled.load(Ordering::Relaxed));
            v.apply_generated_title(
                "Improve session filtering",
                "Late generated title".into(),
                cx,
            );
            assert_eq!(v.summary.title, "My session name");
        });
        assert!(locked(&store).unwrap().load(id).unwrap().rows.is_empty());
    }

    fn git(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(cwd)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A one-commit repository in a private temporary directory, a session
    /// in it, and an app worktree for that session.
    fn worktree_session(
        cx: &mut gpui::TestAppContext,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        String,
        gpui::Entity<Conversation>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("file"), "x\n").unwrap();
        git(&repo, &["add", "file"]);
        git(&repo, &["commit", "-m", "init"]);
        let store = Arc::new(std::sync::Mutex::new(
            gespan_core::store::Store::open(Path::new(":memory:")).unwrap(),
        ));
        let summary = locked(&store)
            .unwrap()
            .create("codex", &repo.to_string_lossy())
            .unwrap();
        let tree = gespan_core::git::create_worktree(&repo, "main", summary.id).unwrap();
        let tree = tree.to_string_lossy().into_owned();
        let view = cx.new(|cx| Conversation::new(summary, store.clone(), cx));
        view.update(cx, |v, _| v.settings.worktree_cwd = Some(tree.clone()));
        (temp, repo, tree, view)
    }

    fn branch(repo: &Path, tree: &str) -> Option<String> {
        gespan_core::git::worktree_branch(repo, Path::new(tree))
    }

    #[gpui::test]
    fn a_suggested_name_renames_the_new_worktree_branch(cx: &mut gpui::TestAppContext) {
        let (_temp, repo, tree, view) = worktree_session(cx);
        view.update(cx, |v, cx| {
            v.test_branch_name = Some("Fix the login flow".into());
            v.branch_seed = Some("Fix the login flow, it loops".into());
            v.name_worktree_branch(tree.clone(), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            branch(&repo, &tree).as_deref(),
            Some("gespan/fix-the-login-flow")
        );
        view.update(cx, |v, _| {
            assert_eq!(
                v.branch.as_deref(),
                Some("gespan/fix-the-login-flow"),
                "shown"
            );
        });
    }

    #[gpui::test]
    fn no_suggestion_keeps_the_placeholder(cx: &mut gpui::TestAppContext) {
        let (_temp, repo, tree, view) = worktree_session(cx);
        let placeholder = branch(&repo, &tree);
        view.update(cx, |v, cx| {
            v.branch_seed = Some("Fix the login flow".into());
            v.name_worktree_branch(tree.clone(), cx);
        });
        cx.run_until_parked();
        assert_eq!(branch(&repo, &tree), placeholder);
    }

    #[gpui::test]
    fn naming_takes_its_message_once(cx: &mut gpui::TestAppContext) {
        let (_temp, repo, tree, view) = worktree_session(cx);
        let placeholder = branch(&repo, &tree);
        view.update(cx, |v, cx| {
            v.test_branch_name = Some("fix login".into());
            v.branch_seed = Some("Fix the login flow".into());
            v.name_worktree_branch(tree.clone(), cx);
            assert_eq!(v.branch_seed, None);
            // A second worktree event without a new message renames nothing.
            v.test_branch_name = Some("something else".into());
            v.name_worktree_branch(tree.clone(), cx);
        });
        cx.run_until_parked();
        assert_ne!(branch(&repo, &tree), placeholder);
        assert_eq!(branch(&repo, &tree).as_deref(), Some("gespan/fix-login"));
    }

    #[gpui::test]
    fn closing_the_conversation_cancels_naming(cx: &mut gpui::TestAppContext) {
        let (_temp, _repo, tree, view) = worktree_session(cx);
        let cancel = view.update(cx, |v, cx| {
            v.branch_seed = Some("Fix the login flow".into());
            v.name_worktree_branch(tree.clone(), cx);
            v.branch_cancel.clone().unwrap()
        });
        assert!(!cancel.load(Ordering::Relaxed));
        drop(view);
        // GPUI drops released entities when it next flushes effects.
        cx.update(|_| {});
        cx.run_until_parked();
        assert!(cancel.load(Ordering::Relaxed));
    }
}
