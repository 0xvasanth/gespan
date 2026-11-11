//! Import sessions, from the project menu: the window that lists sessions
//! other tools kept on disk, and bringing the ticked ones into the sidebar.
use super::*;
use crate::import_dialog::{self, Event, ImportDialog};
use gespan_core::session_import::{self, Candidate, Report, Skip, Source};
use std::path::PathBuf;

/// The project menu's action id.
pub(super) const ACTION: &str = "import-sessions";

type FindSources = Arc<dyn Fn() -> Vec<Source> + Send + Sync>;

/// The import window, while it is open or loading.
pub(super) struct ImportSessions {
    dialog: Option<OpenImport>,
    /// Counts opens and closes, so a slow load never opens a window the user
    /// has moved on from.
    opening: u64,
    /// The agents with sessions on disk and their homes; tests point it at
    /// a fixture.
    pub(super) find_sources: FindSources,
}
struct OpenImport {
    view: Entity<ImportDialog>,
    sources: Vec<Source>,
    _events: Subscription,
}
impl Default for ImportSessions {
    fn default() -> Self {
        Self {
            dialog: None,
            opening: 0,
            find_sources: Arc::new(|| {
                session_import::present_sources(session_import::default_home)
            }),
        }
    }
}
impl ImportSessions {
    pub(super) fn is_open(&self) -> bool {
        self.dialog.is_some()
    }
}

/// Why a kind of skip was skipped, as the notice says it.
fn skip_reason(skip: &Skip) -> (u8, &'static str) {
    match skip {
        Skip::NoProject => (0, "no matching project"),
        Skip::FolderMissing => (1, "folder missing"),
        Skip::AlreadyImported => (2, "already imported"),
        Skip::Unreadable(_) => (3, "couldn't be read"),
        Skip::Failed(_) => (4, "couldn't be saved"),
    }
}

/// "Imported N sessions into <projects>", or "Nothing imported", then how
/// many were skipped, one part per reason. `label` names a project.
fn import_notice(report: &Report, label: impl Fn(&str) -> String) -> String {
    let mut text = match report.imported.len() {
        0 => "Nothing imported".to_owned(),
        1 => "Imported 1 session".to_owned(),
        count => format!("Imported {count} sessions"),
    };
    let mut projects: Vec<&str> = Vec::new();
    for imported in &report.imported {
        if !projects.contains(&imported.project.as_str()) {
            projects.push(&imported.project);
        }
    }
    if !projects.is_empty() {
        let names: Vec<String> = projects.into_iter().map(label).collect();
        text.push_str(" into ");
        text.push_str(&names.join(", "));
    }
    let mut reasons: Vec<(u8, &str, usize)> = Vec::new();
    for (_, skip) in &report.skipped {
        let (order, reason) = skip_reason(skip);
        match reasons.iter_mut().find(|(o, _, _)| *o == order) {
            Some((_, _, n)) => *n += 1,
            None => reasons.push((order, reason, 1)),
        }
    }
    reasons.sort_by_key(|(order, _, _)| *order);
    for (_, reason, n) in reasons {
        text.push_str(&format!(" · {n} skipped ({reason})"));
    }
    text
}

impl Shell {
    /// Opens the Import sessions window for the project at `path`, once the
    /// agents on disk and Gespan's own sessions are read off the UI thread.
    pub(super) fn open_import_dialog(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        self.close_preview(cx);
        if path.starts_with("remote://") {
            return;
        }
        self.import_sessions.dialog = None;
        self.import_sessions.opening += 1;
        let opening = self.import_sessions.opening;
        let store = self.store.clone();
        let find_sources = self.import_sessions.find_sources.clone();
        let found = cx.background_executor().spawn(async move {
            let held = locked(&store)?.provider_session_ids()?;
            Ok::<_, String>((held, find_sources()))
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = found.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.import_sessions.opening != opening {
                    return;
                }
                match found {
                    Ok((held, sources)) => this.show_import_dialog(path, held, sources, window, cx),
                    Err(error) => {
                        this.error = Some(format!("Could not open Import sessions: {error}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn show_import_dialog(
        &mut self,
        path: String,
        held: import_dialog::Held,
        sources: Vec<Source>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let projects = self.workspace.projects.clone();
        let shown = sources.clone();
        let view = cx.new(|cx| ImportDialog::new(path, projects, held, shown, window, cx));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &Event, window, cx| match event {
                Event::Close => this.close_import_dialog(window, cx),
                Event::Import(picks) => this.import_picked(picks.clone(), window, cx),
            },
        );
        window.focus(&view.focus_handle(cx), cx);
        self.import_sessions.dialog = Some(OpenImport {
            view,
            sources,
            _events: events,
        });
        cx.notify();
    }
    fn close_import_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.import_sessions.opening += 1;
        self.import_sessions.dialog = None;
        self.focus_workspace(window, cx);
        cx.notify();
    }
    /// Imports `picks` in the background. The window closes first, so a
    /// second click cannot start a second import.
    fn import_picked(
        &mut self,
        picks: Vec<Candidate>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.import_sessions.dialog.take() else {
            return;
        };
        self.close_import_dialog(window, cx);
        let navigation_revision = self.navigation_revision;
        let store = self.store.clone();
        let projects = self.workspace.projects.clone();
        let homes = open.sources;
        let work = cx.background_executor().spawn(async move {
            let home = |kind: Kind| -> Option<PathBuf> {
                homes
                    .iter()
                    .find(|s| s.kind == kind)
                    .map(|s| s.home.clone())
            };
            let report = session_import::import(
                &store,
                &picks,
                &projects,
                import_dialog::worktree_roots(),
                home,
            );
            (report, picks)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (report, picks) = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_import(report, &picks, navigation_revision, window, cx)
            });
        })
        .detach();
    }
    /// Lists each imported session at the top of the sidebar, selects the
    /// newest one from this project and says what happened. The open project
    /// never changes.
    fn finish_import(
        &mut self,
        report: Report,
        picks: &[Candidate],
        navigation_revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let used_at = |session: &Session| {
            picks
                .iter()
                .find(|pick| {
                    pick.kind.id() == session.provider
                        && session.provider_id.as_deref() == Some(pick.provider_id.as_str())
                })
                .map_or(0, |pick| pick.updated_at)
        };
        let mut imported = report.imported.clone();
        // Oldest first, so the newest ends up at the top.
        imported.sort_by_key(|item| used_at(&item.session));
        let mut newest_here = None;
        for item in imported {
            let id = item.session.id;
            if item.project == self.workspace.project {
                newest_here = Some(id);
            }
            // Before `add`, so the conversation and the sidebar start out in
            // the worktree rather than waiting for the next reload.
            self.session_worktrees.insert(id, item.worktree);
            self.add(item.session, cx);
            self.order.retain(|s| *s != id);
            self.order.insert(0, id);
        }
        let notice = import_notice(&report, |project| self.project_label(project));
        self.show_action_notice(notice, cx);
        match newest_here {
            Some(id) if self.navigation_revision == navigation_revision => {
                self.activate_session(id, window, cx)
            }
            _ => {
                self.invalidate_chrome(cx);
                cx.notify();
            }
        }
    }
    /// The Import sessions window, centred once its height is known.
    pub(super) fn import_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let view = self.import_sessions.dialog.as_ref()?.view.clone();
        let viewport = window.viewport_size();
        let width = 640f32.min(viewport.width.as_f32() - 24.);
        let measured = view.read(cx).measured_height().unwrap_or(520.);
        let height = (measured + 2.).min(viewport.height.as_f32() - 32.);
        let anchor = Anchor::Point(gpui::point(
            px((viewport.width.as_f32() - width) / 2.),
            px((viewport.height.as_f32() - height) / 2.),
        ));
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("import-sessions-scrim"))
                .child(
                    popover("import-sessions-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .max_height(viewport.height.as_f32() - 32.)
                        .priority(60)
                        .on_dismiss(move |window, cx| {
                            let _ =
                                weak.update(cx, |this, cx| this.close_import_dialog(window, cx));
                        })
                        .child(view),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::{session_import::Imported, store::Store};
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};
    use std::{
        fs,
        path::Path,
        time::{Duration, UNIX_EPOCH},
    };

    struct Fixture {
        window: WindowHandle<Shell>,
        /// The open project.
        here: String,
        /// Another project in the rail.
        there: String,
        /// A folder that is no Gespan project.
        stray: String,
        /// The user's home, holding the agents' folders.
        user: tempfile::TempDir,
        /// The chat open when the window starts.
        existing: i64,
        _folders: tempfile::TempDir,
    }

    /// A window on project `here`, with `there` beside it, and an empty
    /// Claude home under a fixture user home.
    fn fixture(cx: &mut TestAppContext) -> Fixture {
        let folders = tempfile::tempdir().unwrap();
        let folder = |name: &str| {
            let path = folders.path().join(name);
            fs::create_dir_all(&path).unwrap();
            path.to_string_lossy().into_owned()
        };
        let (here, there, stray) = (folder("here"), folder("there"), folder("stray"));
        let user = tempfile::tempdir().unwrap();
        fs::create_dir_all(user.path().join(".claude/projects")).unwrap();
        let store = Store::open(Path::new(":memory:")).unwrap();
        let existing = store.create("claude", &here).unwrap();
        let existing_id = existing.id;
        let mut state = Workspace::default();
        state.add_project(&here);
        state.add_project(&there);
        state.open(existing.id);
        let saved = serde_json::to_string(&state).unwrap();
        let store = Arc::new(Mutex::new(store));
        let home = user.path().to_path_buf();
        cx.update(|cx| {
            input::bind(cx);
            crate::popover::bind(cx);
            cx.set_reduce_motion(true);
        });
        let project = here.clone();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut shell = Shell::new(store, vec![existing], project, Some(saved), cx);
                    shell.import_sessions.find_sources =
                        Arc::new(move || session_import::sources(&home));
                    shell
                })
            })
            .unwrap()
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        Fixture {
            window,
            here,
            there,
            stray,
            user,
            existing: existing_id,
            _folders: folders,
        }
    }

    /// A Claude Code session `id` that ran in `cwd`, last written at `secs`.
    fn claude_session(user: &Path, id: &str, cwd: &str, prompt: &str, secs: u64) {
        let path = user
            .join(".claude/projects/-fixture")
            .join(format!("{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lines = [
            serde_json::json!({
                "type": "user", "uuid": "u1", "parentUuid": null,
                "cwd": cwd, "gitBranch": "main",
                "message": {"role": "user", "content": prompt},
            }),
            serde_json::json!({
                "type": "assistant", "uuid": "a1", "parentUuid": "u1",
                "message": {"role": "assistant", "id": "a1",
                            "content": [{"type": "text", "text": "Done."}]},
            }),
        ];
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(&path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn shown(visual: &mut VisualTestContext, selector: &str) -> bool {
        visual.refresh().unwrap();
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        visual.debug_bounds(selector).is_some()
    }
    fn click(visual: &mut VisualTestContext, selector: &str) {
        visual.refresh().unwrap();
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }
    fn open_project_menu(window: WindowHandle<Shell>, path: &str, cx: &mut VisualTestContext) {
        let path = path.to_owned();
        window
            .update(cx, |shell, w, cx| {
                shell.close_menu(cx);
                shell.open_menu(
                    MenuTarget::Project { path },
                    Anchor::Point(gpui::point(px(300.), px(60.))),
                    w,
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
    }
    /// Opens the window for `path` and waits for it to list from disk.
    fn open_dialog(window: WindowHandle<Shell>, path: &str, cx: &mut VisualTestContext) {
        let path = path.to_owned();
        window
            .update(cx, |shell, w, cx| shell.open_import_dialog(path, w, cx))
            .unwrap();
        cx.run_until_parked();
    }
    /// Sends `Import` with every row the dialog found, as ticking them all
    /// and clicking Import would, and checks the window shut at once.
    fn import_everything(window: WindowHandle<Shell>, cx: &mut VisualTestContext) {
        window
            .update(cx, |shell, _, cx| {
                let view = shell.import_sessions.dialog.as_ref().unwrap().view.clone();
                let picks = view.read(cx).found_candidates();
                view.update(cx, |_, cx| cx.emit(Event::Import(picks)));
            })
            .unwrap();
        window
            .update(cx, |shell, _, _| {
                assert!(
                    !shell.import_sessions.is_open(),
                    "the window closes on the first Import, before the work is done"
                );
            })
            .unwrap();
        cx.run_until_parked();
    }
    fn stored(window: WindowHandle<Shell>, cx: &mut VisualTestContext) -> Vec<Session> {
        window
            .update(cx, |shell, _, _| {
                locked(&shell.store).unwrap().sessions().unwrap()
            })
            .unwrap()
    }
    fn imported_id(sessions: &[Session], provider_id: &str) -> i64 {
        sessions
            .iter()
            .find(|s| s.provider_id.as_deref() == Some(provider_id))
            .unwrap_or_else(|| panic!("{provider_id} was not imported"))
            .id
    }
    fn notice(window: WindowHandle<Shell>, cx: &mut VisualTestContext) -> Option<String> {
        window
            .update(cx, |shell, _, _| {
                shell.action_notice.as_ref().map(|n| n.to_string())
            })
            .unwrap()
    }

    #[gpui::test]
    fn project_menu_offers_import_sessions(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        open_project_menu(f.window, &f.here, visual);
        visual.refresh().unwrap();
        let import = visual
            .debug_bounds("menu-action-import-sessions")
            .expect("the project menu offers Import sessions");
        let archive = visual.debug_bounds("menu-action-archive").unwrap();
        assert!(
            import.bottom() <= archive.top(),
            "Import sessions sits above the Archive / Delete group"
        );
        let shape = |window: WindowHandle<Shell>, cx: &mut VisualTestContext| {
            window
                .update(cx, |shell, _, cx| {
                    shell.menu.as_ref().unwrap().menu.read(cx).item_shape()
                })
                .unwrap()
        };
        let items = shape(f.window, visual);
        assert_eq!(
            &items[items.len() - 4..],
            ["Import sessions…", "---", "Archive", "Delete"],
            "its own group, just before Archive and Delete"
        );

        open_project_menu(f.window, "remote://box/home/me/app", visual);
        let items = shape(f.window, visual);
        assert!(items.contains(&"Archive".to_owned()));
        assert!(
            !items.contains(&"Import sessions…".to_owned()),
            "a remote project's sessions stay on its host"
        );
        assert!(!shown(visual, "menu-action-import-sessions"));
    }

    #[gpui::test]
    fn import_action_opens_the_import_dialog(cx: &mut TestAppContext) {
        let f = fixture(cx);
        claude_session(
            f.user.path(),
            "s-here",
            &f.here,
            "Fix the login redirect",
            1_000,
        );
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        open_project_menu(f.window, &f.here, visual);
        click(visual, "menu-action-import-sessions");
        visual.run_until_parked();
        f.window
            .update(visual, |shell, _, _| {
                assert!(shell.menu.is_none(), "the menu gives way to the window");
                assert!(shell.import_sessions.is_open());
            })
            .unwrap();
        assert!(shown(visual, "import-dialog"));
        assert!(
            shown(visual, "import-tab-claude"),
            "a tab for the agent on disk"
        );
        assert!(
            !shown(visual, "import-tab-codex"),
            "no tab for an agent not on disk"
        );
        assert!(
            shown(visual, "import-row-s-here"),
            "the session lists from disk"
        );

        // Centred in the window once its height is measured.
        visual.refresh().unwrap();
        let dialog = visual.debug_bounds("import-dialog").unwrap();
        let viewport = f
            .window
            .update(visual, |_, w, _| w.viewport_size())
            .unwrap();
        let (left, right) = (dialog.left(), viewport.width - dialog.right());
        let (top, bottom) = (dialog.top(), viewport.height - dialog.bottom());
        assert!((left - right).abs() <= px(2.), "{left:?} vs {right:?}");
        assert!((top - bottom).abs() <= px(2.), "{top:?} vs {bottom:?}");
        assert!(top >= px(16.));

        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        f.window
            .update(visual, |shell, _, _| {
                assert!(!shell.import_sessions.is_open())
            })
            .unwrap();
        assert!(!shown(visual, "import-dialog"));
    }

    #[gpui::test]
    fn importing_adds_sessions_to_the_sidebar_and_selects_the_newest(cx: &mut TestAppContext) {
        let f = fixture(cx);
        claude_session(f.user.path(), "s-old", &f.here, "Plan the footer", 1_000);
        claude_session(
            f.user.path(),
            "s-new",
            &f.here,
            "Ship the footer\nnow",
            2_000,
        );
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        open_dialog(f.window, &f.here, visual);
        click(visual, "import-row-s-old");
        click(visual, "import-row-s-new");
        click(visual, "import-confirm");
        visual.run_until_parked();
        assert!(
            !shown(visual, "import-dialog"),
            "the window closes on Import"
        );

        let sessions = stored(f.window, visual);
        assert_eq!(sessions.len(), 3, "both picks land, once each");
        let (old, new) = (
            imported_id(&sessions, "s-old"),
            imported_id(&sessions, "s-new"),
        );
        let new_session = sessions.iter().find(|s| s.id == new).unwrap();
        assert_eq!(new_session.title, "Ship the footer");
        assert_eq!(new_session.cwd, f.here);
        f.window
            .update(visual, |shell, _, _| {
                assert_eq!(&shell.order[..2], &[new, old], "newest first, at the top");
                assert_eq!(shell.workspace.active, Some(new), "the newest one opens");
                assert_eq!(shell.workspace.project, f.here);
                assert!(shell.sessions.contains_key(&old));
            })
            .unwrap();
        assert!(shown(visual, &format!("sidebar-session-{new}")));
        assert!(shown(visual, &format!("sidebar-session-{old}")));
        assert_eq!(
            notice(f.window, visual).as_deref(),
            Some("Imported 2 sessions into here")
        );
        assert!(
            shown(visual, "action-notice"),
            "the notice shows, not as an error"
        );
        f.window
            .update(visual, |shell, _, _| assert!(shell.error.is_none()))
            .unwrap();
    }

    #[gpui::test]
    fn sessions_gespan_hides_show_as_imported(cx: &mut TestAppContext) {
        let f = fixture(cx);
        claude_session(f.user.path(), "s-asked", &f.here, "Ask about the PR", 2_000);
        claude_session(f.user.path(), "s-free", &f.here, "Fresh work", 1_000);
        let here = f.here.clone();
        f.window
            .update(cx, |shell, _, _| {
                let store = locked(&shell.store).unwrap();
                let item = gespan_core::inbox::Item {
                    provider: "github".into(),
                    kind: "pr".into(),
                    number: 9,
                    title: "Item 9".into(),
                    url: "https://github.com/acme/widgets/pull/9".into(),
                    repo: "acme/widgets".into(),
                    ..Default::default()
                };
                let context = gespan_core::inbox::ask::Context::new(&item, None);
                let (ask, _) = store
                    .open_inbox_ask("claude", &here, &context, &Default::default())
                    .unwrap();
                store.provider_id(ask.id, "s-asked").unwrap();
                assert!(store.sessions().unwrap().iter().all(|s| s.id != ask.id));
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        open_dialog(f.window, &f.here, visual);
        assert!(
            shown(visual, "import-row-s-asked-imported"),
            "an Ask conversation's session is already in Gespan"
        );
        assert!(!shown(visual, "import-row-s-free-imported"));
    }

    #[gpui::test]
    fn importing_a_session_from_another_project_does_not_switch_projects(cx: &mut TestAppContext) {
        let f = fixture(cx);
        claude_session(
            f.user.path(),
            "s-there",
            &f.there,
            "Tidy the other app",
            1_000,
        );
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        open_dialog(f.window, &f.here, visual);
        import_everything(f.window, visual);

        let sessions = stored(f.window, visual);
        let there = imported_id(&sessions, "s-there");
        assert_eq!(
            sessions.iter().find(|s| s.id == there).unwrap().cwd,
            f.there
        );
        f.window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.project, f.here, "the open project stays");
                assert_eq!(
                    shell.workspace.active,
                    Some(f.existing),
                    "the open chat stays"
                );
                assert!(
                    shell.sessions.contains_key(&there),
                    "listed under its project"
                );
                assert_eq!(shell.order.first(), Some(&there));
            })
            .unwrap();
        assert_eq!(
            notice(f.window, visual).as_deref(),
            Some("Imported 1 session into there")
        );
    }

    #[gpui::test]
    fn import_notice_reports_skips(cx: &mut TestAppContext) {
        let f = fixture(cx);
        claude_session(f.user.path(), "s-here", &f.here, "Kept", 3_000);
        claude_session(f.user.path(), "s-there", &f.there, "Also kept", 2_000);
        claude_session(f.user.path(), "s-stray", &f.stray, "No project", 1_000);
        claude_session(f.user.path(), "s-stray-2", &f.stray, "Nor this", 900);
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        open_dialog(f.window, &f.here, visual);
        import_everything(f.window, visual);
        assert_eq!(
            notice(f.window, visual).as_deref(),
            Some("Imported 2 sessions into here, there · 2 skipped (no matching project)")
        );
        assert_eq!(stored(f.window, visual).len(), 3);

        // Every kind of skip gets its own reason; with nothing imported there
        // are no projects to name.
        let pick = |id: &str| Candidate {
            kind: Kind::Claude,
            provider_id: id.into(),
            cwd: "/x".into(),
            title: id.into(),
            branch: None,
            updated_at: 0,
            messages: None,
            path: "/x".into(),
        };
        let report = Report {
            imported: Vec::new(),
            skipped: vec![
                (pick("a"), Skip::Unreadable("bad line".into())),
                (pick("b"), Skip::AlreadyImported),
                (pick("c"), Skip::FolderMissing),
                (pick("d"), Skip::NoProject),
                (pick("e"), Skip::AlreadyImported),
                (pick("f"), Skip::Failed("disk full".into())),
            ],
        };
        assert_eq!(
            import_notice(&report, |p| p.to_owned()),
            "Nothing imported · 1 skipped (no matching project) · 1 skipped (folder missing) \
             · 2 skipped (already imported) · 1 skipped (couldn't be read) \
             · 1 skipped (couldn't be saved)"
        );
        let session = |project: &str| Imported {
            session: Session {
                id: 1,
                title: String::new(),
                provider: "claude".into(),
                cwd: project.into(),
                provider_id: None,
                has_messages: true,
                updated_at: 0,
                created_at: 0,
                archived: false,
                pinned: false,
                work_item: None,
            },
            project: project.into(),
            worktree: None,
        };
        assert_eq!(
            import_notice(&Report::default(), |p| p.to_owned()),
            "Nothing imported"
        );
        let report = Report {
            imported: vec![session("/w/a"), session("/w/a")],
            skipped: Vec::new(),
        };
        assert_eq!(
            import_notice(&report, |p| p.rsplit('/').next().unwrap().to_owned()),
            "Imported 2 sessions into a",
            "each project is named once"
        );
    }

    #[gpui::test]
    fn finishing_an_import_files_a_worktree_session_under_its_worktree(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let tree = format!("{}-tree", f.here);
        fs::create_dir_all(&tree).unwrap();
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        f.window
            .update(visual, |shell, window, cx| {
                let session = {
                    let mut store = locked(&shell.store).unwrap();
                    let settings = gespan_core::agent::Settings {
                        worktree_cwd: Some(tree.clone()),
                        ..Default::default()
                    };
                    let mut session = store
                        .create_from("claude", &f.here, "In the tree", &settings, &[])
                        .unwrap();
                    store.provider_id(session.id, "s-tree").unwrap();
                    session.provider_id = Some("s-tree".into());
                    session
                };
                let id = session.id;
                let report = Report {
                    imported: vec![Imported {
                        session,
                        project: f.here.clone(),
                        worktree: Some(tree.clone()),
                    }],
                    skipped: Vec::new(),
                };
                let revision = shell.navigation_revision;
                shell.finish_import(report, &[], revision, window, cx);
                assert_eq!(
                    shell.session_worktrees.get(&id),
                    Some(&Some(tree.clone())),
                    "registered at once, not on the next reload"
                );
                assert_eq!(shell.session_working_copy(id, cx), tree);
            })
            .unwrap();
    }
}
