//! Open in editor: the focused session's worktree or folder, opened in an
//! installed editor. The project settings menu offers it, and so does the
//! boxy corner while the project has no git.
use super::*;
use gespan_core::external_editor::{self, ExternalEditor};

pub(super) const EDITOR_ITEM_PREFIX: &str = "editor:";
/// The button and the gap before the next corner control.
pub(super) const OPEN_IN_EDITOR_WIDTH: f32 = 24. + CORNER_CONTROL_GAP;

/// Lists the editors installed now.
type FindEditors = Arc<dyn Fn() -> Vec<ExternalEditor> + Send + Sync>;
/// Starts the editor with this id on this folder.
type StartEditor = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

/// Finds installed editors and starts one on a folder. Tests swap in stubs so
/// nothing real launches.
#[derive(Clone)]
pub(super) struct EditorLauncher {
    pub(super) installed: FindEditors,
    pub(super) open: StartEditor,
}

impl Default for EditorLauncher {
    fn default() -> Self {
        Self {
            installed: Arc::new(external_editor::installed),
            open: Arc::new(external_editor::open),
        }
    }
}

/// The menu's launch state while it is open.
#[derive(Default)]
pub(super) struct EditorMenu {
    pub(super) launcher: EditorLauncher,
    /// The editor starting now, and for which folder. Another pick for that
    /// folder waits for it; a pick for another folder replaces it.
    launch: Option<Launch>,
    launches: u64,
    /// Why the last pick failed, shown above the editors.
    error: Option<String>,
}

struct Launch {
    id: u64,
    path: String,
}

/// The editors to offer, the last one used first and checked; the rest keep
/// the scan's order. `None` while the scan is still running. Editor rows keep
/// the menu open until the launch settles.
pub(super) fn open_in_editor_menu(
    editors: Option<&[ExternalEditor]>,
    last: Option<&str>,
    error: Option<&str>,
) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = error
        .map(|error| vec![MenuItem::Note(error.to_owned().into()), MenuItem::Separator])
        .unwrap_or_default();
    items.extend(match editors {
        None => vec![
            MenuAction::new("editor-loading", "Looking for editors…")
                .disabled(true)
                .into(),
        ],
        Some([]) => vec![
            MenuAction::new("editor-none", "No supported editors found")
                .disabled(true)
                .into(),
        ],
        Some(editors) => {
            let last = editors.iter().find(|editor| Some(editor.id) == last);
            last.into_iter()
                .chain(editors.iter().filter(|editor| Some(*editor) != last))
                .map(|editor| {
                    MenuAction::new(format!("{EDITOR_ITEM_PREFIX}{}", editor.id), editor.name)
                        .checked(Some(editor) == last)
                        .keep_open()
                        .into()
                })
                .collect()
        }
    });
    items.extend([
        MenuItem::Separator,
        MenuAction::new("reveal", REVEAL_LABEL)
            .icon("folder-open")
            .into(),
        MenuAction::new("copy-path", "Copy path")
            .icon("copy")
            .into(),
    ]);
    items
}

/// Only a folder on this computer can open in one of its editors. Shares the
/// notification settings' project test, so home and app bundles are excluded
/// here too.
pub(super) fn local_folder(path: &str) -> bool {
    !path.starts_with("remote://") && crate::settings::looks_like_project(path)
}

impl Shell {
    /// The folder the corner offers: where a new terminal would open. Pilots,
    /// app views, search and Settings have no folder of their own.
    pub(super) fn open_in_editor_folder(&self, cx: &App) -> Option<String> {
        if self.pilot_active(cx)
            || self.app_view_open()
            || self.app_search.is_some()
            || self.settings.is_some()
        {
            return None;
        }
        Some(self.terminal_cwd(cx)).filter(|path| local_folder(path))
    }
    /// The width the button adds to the corner, or nothing without a folder
    /// or room for it.
    pub(super) fn open_in_editor_room(&self, cx: &App) -> f32 {
        if self.open_in_editor_folder(cx).is_some() && self.editor_fits_corner(cx) {
            OPEN_IN_EDITOR_WIDTH
        } else {
            0.
        }
    }
    /// An open right panel's corner holds its buttons. Too narrow for them
    /// all, the button steps aside instead of pushing past the panel's edge;
    /// the project menu still has it.
    fn editor_fits_corner(&self, cx: &App) -> bool {
        let git = self.git.read(cx);
        if !self.workspace.right_panel_open || self.app_view_open() || !git.available() {
            return true;
        }
        let padding = if self.workspace.boxy_layout() {
            24.
        } else {
            12.
        };
        self.workspace.right_panel_width >= CORNER_BUTTONS_WIDTH + OPEN_IN_EDITOR_WIDTH + padding
    }
    pub(super) fn open_in_editor_button(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let path = self.open_in_editor_folder(cx)?;
        if self.open_in_editor_room(cx) == 0. {
            return None;
        }
        let open = self
            .menu
            .as_ref()
            .is_some_and(|menu| matches!(&menu.target, MenuTarget::OpenInEditor(_)));
        let label = SharedString::from(format!("Open {} in editor", project_name(&path)));
        Some(
            icon_button("titlebar-open-in-editor", "Open in editor", "app-window")
                .debug_selector(|| "titlebar-open-in-editor".into())
                .aria_label(label)
                .aria_expanded(open)
                .relative()
                .flex_shrink_0()
                .when(open, |d| d.bg(content(0.1)))
                .child(self.editor_anchor.probe())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this
                        .menu
                        .as_ref()
                        .is_some_and(|menu| matches!(&menu.target, MenuTarget::OpenInEditor(_)))
                    {
                        this.close_menu(cx);
                        return;
                    }
                    let anchor = Anchor::Slot(this.editor_anchor.clone());
                    this.open_editor_menu(path.clone(), anchor, window, cx);
                })),
        )
    }
    /// The folder the project settings menu opens in an editor. It acts on
    /// what is on screen, so a session in a worktree opens that worktree.
    pub(super) fn project_editor_folder(&self, path: &str, cx: &App) -> String {
        if path != self.workspace.project {
            return path.into();
        }
        self.open_in_editor_folder(cx)
            .unwrap_or_else(|| path.into())
    }
    /// Opens the editor menu for `path` with a fresh scan and no old error.
    pub(super) fn open_editor_menu(
        &mut self,
        path: String,
        anchor: Anchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor_menu.error = None;
        self.open_menu(MenuTarget::OpenInEditor(path), anchor, window, cx);
        self.rescan_editors_for_menu(cx);
    }
    /// An editor installed since launch shows up the next time the menu opens.
    fn rescan_editors_for_menu(&mut self, cx: &mut Context<Self>) {
        let installed = self.editor_menu.launcher.installed.clone();
        let work = cx.background_executor().spawn(async move { installed() });
        cx.spawn(async move |this, cx| {
            let editors = work.await;
            let _ = this.update(cx, |this, cx| {
                this.external_editors = Some(editors);
                this.refresh_editor_menu(cx);
            });
        })
        .detach();
    }
    pub(super) fn editor_menu_items(&self) -> Vec<MenuItem> {
        open_in_editor_menu(
            self.external_editors.as_deref(),
            self.workspace.last_external_editor.as_deref(),
            self.editor_menu.error.as_deref(),
        )
    }
    /// The open editor menu, when there is one.
    fn editor_menu_entity(&self) -> Option<Entity<Menu>> {
        self.menu
            .as_ref()
            .filter(|menu| matches!(&menu.target, MenuTarget::OpenInEditor(_)))
            .map(|menu| menu.menu.clone())
    }
    fn refresh_editor_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.editor_menu_entity() {
            let items = self.editor_menu_items();
            menu.update(cx, |menu, cx| menu.replace_items(items, cx));
        }
    }
    /// Starts `editor` on `path`, unless an earlier pick for the same folder
    /// is still starting.
    pub(super) fn pick_external_editor(
        &mut self,
        path: String,
        editor: String,
        cx: &mut Context<Self>,
    ) {
        let menu = &mut self.editor_menu;
        if menu
            .launch
            .as_ref()
            .is_some_and(|launch| launch.path == path)
        {
            return;
        }
        menu.launches += 1;
        let id = menu.launches;
        menu.launch = Some(Launch {
            id,
            path: path.clone(),
        });
        let work = cx.background_executor().spawn({
            let (editor, path) = (editor.clone(), path.clone());
            let open = self.editor_menu.launcher.open.clone();
            async move { open(&editor, &path) }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_editor_launch(id, &path, editor, result, cx)
            });
        })
        .detach();
    }
    /// A started editor is remembered and closes its folder's menu. A failure
    /// stays in that menu; once the menu has closed or moved to another
    /// folder, it reaches the error banner. Only the latest launch clears the
    /// pending slot, so an older one finishing cannot unblock a newer one.
    pub(super) fn finish_editor_launch(
        &mut self,
        id: u64,
        path: &str,
        editor: String,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let current = self.editor_menu.launch.as_ref().is_some_and(|l| l.id == id);
        if current {
            self.editor_menu.launch = None;
        }
        let own_menu = self
            .menu
            .as_ref()
            .is_some_and(|m| m.target == MenuTarget::OpenInEditor(path.to_owned()));
        match result {
            Ok(()) => {
                self.remember_external_editor(editor, cx);
                if own_menu {
                    self.close_menu(cx);
                }
            }
            Err(error) if own_menu => {
                self.editor_menu.error = Some(error);
                self.refresh_editor_menu(cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    /// The editor that last opened a folder leads the menu next time.
    pub(super) fn remember_external_editor(&mut self, editor: String, cx: &mut Context<Self>) {
        if self.workspace.last_external_editor.as_ref() != Some(&editor) {
            self.workspace.last_external_editor = Some(editor);
            self.persist(cx);
        }
    }
    pub(super) fn run_open_in_editor_menu(
        &mut self,
        path: String,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        // Editor rows keep the menu open and arrive at pick_external_editor.
        if action == "reveal" {
            cx.reveal_path(std::path::Path::new(&path));
        } else if action == "copy-path" {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    const EDITORS: &[ExternalEditor] = &[
        ExternalEditor {
            id: "vscode",
            name: "Visual Studio Code",
        },
        ExternalEditor {
            id: "zed",
            name: "Zed",
        },
        ExternalEditor {
            id: "webstorm",
            name: "WebStorm",
        },
    ];

    fn rows(items: &[MenuItem]) -> Vec<(String, bool, bool)> {
        items
            .iter()
            .map(|item| match item {
                MenuItem::Action(action) => {
                    (action.label.to_string(), action.checked, action.disabled)
                }
                MenuItem::Separator => ("—".into(), false, false),
                _ => ("?".into(), false, false),
            })
            .collect()
    }

    #[test]
    fn last_editor_used_comes_first_and_is_checked() {
        let items = open_in_editor_menu(Some(EDITORS), Some("webstorm"), None);
        assert_eq!(
            rows(&items),
            [
                ("WebStorm".into(), true, false),
                ("Visual Studio Code".into(), false, false),
                ("Zed".into(), false, false),
                ("—".into(), false, false),
                (REVEAL_LABEL.into(), false, false),
                ("Copy path".into(), false, false),
            ]
        );
        let MenuItem::Action(first) = &items[0] else {
            panic!("expected an editor row");
        };
        assert_eq!(first.id.as_ref(), "editor:webstorm");
    }

    #[test]
    fn uninstalled_last_editor_keeps_the_scan_order() {
        let items = open_in_editor_menu(Some(EDITORS), Some("cursor"), None);
        let rows = rows(&items);
        assert_eq!(
            rows[..3].iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            ["Visual Studio Code", "Zed", "WebStorm"]
        );
        assert!(rows.iter().all(|row| !row.1), "nothing is checked");
    }

    #[test]
    fn scanning_and_empty_scans_explain_themselves_and_keep_folder_actions() {
        assert_eq!(
            rows(&open_in_editor_menu(None, None, None))[0],
            ("Looking for editors…".into(), false, true)
        );
        assert_eq!(
            rows(&open_in_editor_menu(Some(&[]), Some("zed"), None)),
            [
                ("No supported editors found".into(), false, true),
                ("—".into(), false, false),
                (REVEAL_LABEL.into(), false, false),
                ("Copy path".into(), false, false),
            ]
        );
    }

    #[test]
    fn only_local_project_folders_are_offered() {
        assert!(local_folder("/work/app"));
        assert!(local_folder("/work/app/.worktrees/feature-login"));
        assert!(local_folder("C:\\work\\app"));
        assert!(!local_folder("remote://host-1/srv/app"));
        assert!(!local_folder(""));
        assert!(!local_folder("/"));
        assert!(!local_folder("~"));
        assert!(!local_folder("C:\\"));
        assert!(!local_folder("/Applications/Zed.app/Contents"));
        if let Ok(home) = std::env::var("HOME") {
            assert!(!local_folder(&home), "home is not a project");
        }
    }

    /// The rounded and boxy workspace styles.
    const LAYOUTS: [(&str, bool); 2] = [("rounded", false), ("boxy", true)];

    fn set_layout(window: WindowHandle<Shell>, visual: &mut VisualTestContext, boxy: bool) {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = boxy;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn the_corner_holds_only_the_panel_toggle_in_both_styles(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        for (name, boxy) in LAYOUTS {
            set_layout(window, visual, boxy);
            // The corner is the panel toggle alone.
            assert!(
                visual.debug_bounds("titlebar-open-in-editor").is_none(),
                "{name}"
            );
            assert!(
                visual.debug_bounds("titlebar-file-search").is_none(),
                "{name}"
            );
            let viewport = window
                .update(visual, |_, w, _| w.viewport_size().width)
                .unwrap();
            let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
            assert_eq!(toggle.right(), viewport - px(12.), "{name}");
        }
    }

    #[gpui::test]
    fn a_session_in_a_worktree_opens_the_worktree(cx: &mut TestAppContext) {
        let (window, _, active) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.open_in_editor_folder(cx).as_deref(),
                    Some("/work/gespan")
                );
                shell.sessions[&active].update(cx, |chat, _| {
                    chat.settings.worktree_cwd =
                        Some("/work/gespan/.worktrees/feature-login".into());
                });
                assert_eq!(
                    shell.open_in_editor_folder(cx).as_deref(),
                    Some("/work/gespan/.worktrees/feature-login")
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_have_no_folder_to_open(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::General, w, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.open_in_editor_folder(cx), None);
                assert_eq!(shell.open_in_editor_room(cx), 0.);
            })
            .unwrap();
        assert!(visual.debug_bounds("titlebar-open-in-editor").is_none());
    }

    fn open_editor_menu(window: WindowHandle<Shell>, visual: &mut VisualTestContext) {
        // As the project menu's Open in editor row opens it.
        window
            .update(visual, |shell, w, cx| {
                let at = Anchor::Point(Point::default());
                shell.open_editor_menu("/work/gespan".into(), at, w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        assert!(menu_texts(window, visual).is_some(), "the menu opened");
    }

    fn menu_texts(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
    ) -> Option<Vec<String>> {
        window
            .update(visual, |shell, _, cx| {
                shell
                    .editor_menu_entity()
                    .map(|menu| menu.read(cx).item_texts())
            })
            .unwrap()
    }

    #[gpui::test]
    fn a_failed_launch_stays_in_the_open_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        open_editor_menu(window, visual);
        window
            .update(visual, |shell, _, cx| {
                shell.pick_external_editor("/work/gespan".into(), "not-an-editor".into(), cx)
            })
            .unwrap();
        visual.run_until_parked();
        let texts = menu_texts(window, visual).expect("still open");
        assert_eq!(texts[0], "Unknown external editor.");
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.last_external_editor, None);
                assert_eq!(shell.error, None, "shown in the menu, not the banner");
            })
            .unwrap();

        // Reopening starts clean.
        window
            .update(visual, |shell, _, cx| shell.close_menu(cx))
            .unwrap();
        open_editor_menu(window, visual);
        assert!(
            !menu_texts(window, visual)
                .unwrap()
                .contains(&"Unknown external editor.".into())
        );
    }

    // A real successful launch would start the developer's editor, so this
    // drives the launch's result handling directly.
    #[gpui::test]
    fn a_started_editor_is_remembered_and_closes_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        open_editor_menu(window, visual);
        window
            .update(visual, |shell, _, cx| {
                shell.editor_menu.launch = Some(Launch {
                    id: 7,
                    path: "/work/gespan".into(),
                });
                shell.finish_editor_launch(7, "/work/gespan", "zed".into(), Ok(()), cx);
                assert!(shell.editor_menu_entity().is_none());
                assert_eq!(shell.workspace.last_external_editor.as_deref(), Some("zed"));
                assert!(shell.editor_menu.launch.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_failure_after_the_menu_closed_reaches_the_banner(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.pick_external_editor("/work/gespan".into(), "not-an-editor".into(), cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.error.as_deref(), Some("Unknown external editor."));
                assert_eq!(shell.workspace.last_external_editor, None);
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_second_pick_for_the_same_folder_is_ignored_while_one_starts(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.pick_external_editor("/work/gespan".into(), "not-an-editor".into(), cx);
                assert!(shell.editor_menu.launch.is_some());
                // Would fail differently if it ran; Zed never starts on it.
                shell.pick_external_editor("/work/gespan".into(), "zed".into(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.error.as_deref(), Some("Unknown external editor."));
                assert!(shell.editor_menu.launch.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn another_folder_can_launch_while_the_first_still_starts(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.pick_external_editor("/work/gespan".into(), "not-an-editor".into(), cx);
                // No such folder, so this never starts a real editor.
                shell.pick_external_editor("/definitely/missing".into(), "zed".into(), cx);
                let launch = shell.editor_menu.launch.as_ref().unwrap();
                assert_eq!(launch.path, "/definitely/missing", "not ignored");
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.editor_menu.launch.is_none())
            })
            .unwrap();
    }

    fn open_menu_for(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
        path: &str,
        editors: Vec<ExternalEditor>,
    ) {
        window
            .update(visual, |shell, w, cx| {
                // Opened directly, so no rescan replaces these editors.
                shell.external_editors = Some(editors);
                shell.open_menu(
                    MenuTarget::OpenInEditor(path.into()),
                    Anchor::Point(Default::default()),
                    w,
                    cx,
                );
            })
            .unwrap();
        visual.run_until_parked();
    }

    #[gpui::test]
    fn an_old_launch_leaves_the_next_folders_menu_alone(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        open_menu_for(window, visual, "/work/other", EDITORS.to_vec());
        window
            .update(visual, |shell, _, cx| {
                // Started for /work/gespan before the session changed.
                shell.editor_menu.launch = Some(Launch {
                    id: 1,
                    path: "/work/gespan".into(),
                });
                shell.finish_editor_launch(1, "/work/gespan", "zed".into(), Ok(()), cx);
                assert!(shell.editor_menu_entity().is_some(), "still open");
                assert_eq!(shell.workspace.last_external_editor.as_deref(), Some("zed"));

                // An older launch's failure goes to the banner, not this menu,
                // and leaves the newer launch pending.
                shell.editor_menu.launch = Some(Launch {
                    id: 3,
                    path: "/work/other".into(),
                });
                shell.finish_editor_launch(
                    2,
                    "/work/gespan",
                    "zed".into(),
                    Err("Zed is no longer installed.".into()),
                    cx,
                );
                assert!(shell.editor_menu_entity().is_some());
                assert_eq!(shell.editor_menu.error, None);
                assert_eq!(shell.error.as_deref(), Some("Zed is no longer installed."));
                assert!(shell.editor_menu.launch.is_some(), "the newer launch stays");
            })
            .unwrap();
    }

    #[gpui::test]
    fn activating_an_editor_row_launches_it_and_keeps_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // Not in the catalog, so the launch fails without starting anything.
        let ghost = ExternalEditor {
            id: "ghost",
            name: "Ghost Editor",
        };
        open_menu_for(window, visual, "/work/gespan", vec![ghost]);
        window
            .update(visual, |shell, w, cx| {
                let menu = shell.editor_menu_entity().unwrap();
                menu.update(cx, |menu, cx| menu.pick_label("Ghost Editor", w, cx));
            })
            .unwrap();
        visual.run_until_parked();
        let texts = menu_texts(window, visual).expect("the row kept the menu open");
        assert_eq!(texts[0], "Unknown external editor.");
        assert_eq!(texts[1], "Ghost Editor");
        window
            .update(visual, |shell, _, _| {
                assert!(shell.editor_menu.launch.is_none());
                assert_eq!(shell.workspace.last_external_editor, None);
            })
            .unwrap();
    }

    #[gpui::test]
    fn activating_copy_path_copies_and_closes_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        open_menu_for(
            window,
            visual,
            "/work/gespan/.worktrees/feature-login",
            EDITORS.to_vec(),
        );
        window
            .update(visual, |shell, w, cx| {
                let menu = shell.editor_menu_entity().unwrap();
                menu.update(cx, |menu, cx| menu.pick_label("Copy path", w, cx));
            })
            .unwrap();
        visual.run_until_parked();
        assert!(menu_texts(window, visual).is_none());
        let copied = visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(
            copied.as_deref(),
            Some("/work/gespan/.worktrees/feature-login")
        );
    }

    #[gpui::test]
    fn the_open_menu_takes_the_fresh_scan(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // An editor uninstalled since the last scan.
        window
            .update(visual, |shell, _, _| {
                shell.external_editors = Some(vec![ExternalEditor {
                    id: "ghost",
                    name: "Ghost Editor",
                }]);
            })
            .unwrap();
        open_editor_menu(window, visual);
        let installed = external_editor::installed();
        let expected: Vec<String> = open_in_editor_menu(Some(&installed), None, None)
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action(action) => Some(action.label.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(menu_texts(window, visual).unwrap(), expected);
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.external_editors.as_deref(), Some(&installed[..]))
            })
            .unwrap();
    }

    #[gpui::test]
    fn copy_path_puts_the_folder_on_the_clipboard(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.run_open_in_editor_menu(
                    "/work/gespan/.worktrees/feature-login".into(),
                    "copy-path",
                    cx,
                );
            })
            .unwrap();
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(
            copied.as_deref(),
            Some("/work/gespan/.worktrees/feature-login")
        );
    }

    #[test]
    fn a_launch_failure_leads_the_menu() {
        let items = open_in_editor_menu(Some(EDITORS), None, Some("Zed is no longer installed."));
        assert!(
            matches!(&items[0], MenuItem::Note(text) if text.as_ref() == "Zed is no longer installed.")
        );
        assert!(matches!(items[1], MenuItem::Separator));
        assert_eq!(rows(&items[2..])[0].0, "Visual Studio Code");
    }

    #[gpui::test]
    fn the_remembered_editor_survives_a_restart(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        let saved = window
            .update(cx, |shell, _, cx| {
                shell.remember_external_editor("webstorm".into(), cx);
                serde_json::to_string(&shell.workspace).unwrap()
            })
            .unwrap();
        let restored = crate::workspace::Workspace::restore(Some(&saved), &[], "/work/gespan");
        assert_eq!(restored.last_external_editor.as_deref(), Some("webstorm"));
        let items = open_in_editor_menu(
            Some(EDITORS),
            restored.last_external_editor.as_deref(),
            None,
        );
        assert_eq!(rows(&items)[0], ("WebStorm".into(), true, false));
    }

    #[gpui::test]
    fn the_project_corner_leaves_open_in_editor_to_the_project_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        for boxy in [false, true] {
            set_layout(window, visual, boxy);
            for open in [false, true] {
                window
                    .update(visual, |shell, w, cx| {
                        if shell.workspace.right_panel_open != open {
                            shell.toggle_right_panel(w, cx);
                        }
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.refresh().unwrap();
                // The corner is the toggle alone, open or closed.
                assert!(
                    visual.debug_bounds("titlebar-open-in-editor").is_none(),
                    "boxy {boxy}, open {open}"
                );
                assert!(visual.debug_bounds("toggle-right-panel").is_some());
            }
        }
    }

    /// Launches recorded as (editor, folder), with an installed list that
    /// holds only the stub editor.
    fn stub_launcher(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
    ) -> Arc<Mutex<Vec<(String, String)>>> {
        let launches = Arc::new(Mutex::new(Vec::new()));
        let launcher = EditorLauncher {
            installed: Arc::new(|| {
                vec![ExternalEditor {
                    id: "stub",
                    name: "Stub Editor",
                }]
            }),
            open: Arc::new({
                let launches = launches.clone();
                move |editor, path| {
                    launches.lock().unwrap().push((editor.into(), path.into()));
                    Ok(())
                }
            }),
        };
        window
            .update(visual, |shell, _, _| shell.editor_menu.launcher = launcher)
            .unwrap();
        launches
    }

    /// Right-clicks the rail's project switcher and returns the project
    /// menu's rows.
    fn open_project_settings(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
    ) -> Vec<String> {
        let switcher = visual.debug_bounds("sidebar-project-button").unwrap();
        visual.simulate_event(gpui::MouseDownEvent {
            position: switcher.center(),
            button: gpui::MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let open = shell
                    .menu
                    .as_ref()
                    .expect("the project settings menu opened");
                assert!(matches!(open.target, MenuTarget::Project { .. }));
                open.menu.read(cx).item_shape()
            })
            .unwrap()
    }

    /// Runs the gear menu's editor row for `editor`, as picking it would.
    fn launch_from_menu(window: WindowHandle<Shell>, visual: &mut VisualTestContext, editor: &str) {
        window
            .update(visual, |shell, w, cx| {
                let target = MenuTarget::Project {
                    path: shell.workspace.project.clone(),
                };
                shell.run_menu(target, &format!("external-editor:{editor}"), None, w, cx);
            })
            .unwrap();
        visual.run_until_parked();
    }

    #[gpui::test]
    fn the_project_settings_menu_opens_the_project_in_an_editor(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let launches = stub_launcher(window, visual);
        for (name, boxy) in LAYOUTS {
            set_layout(window, visual, boxy);
            launches.lock().unwrap().clear();
            let rows = open_project_settings(window, visual);
            assert!(rows.contains(&"Open in editor".into()), "{name}: {rows:?}");
            window
                .update(visual, |shell, _, cx| shell.close_menu(cx))
                .unwrap();
            launch_from_menu(window, visual, "stub");
            assert_eq!(
                *launches.lock().unwrap(),
                [("stub".to_owned(), "/work/gespan".to_owned())],
                "{name}: opened once, on the project"
            );
        }
    }

    #[gpui::test]
    fn the_sidebar_gear_opens_the_focused_sessions_worktree(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, active) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let launches = stub_launcher(window, visual);
        for boxy in [false, true] {
            set_layout(window, visual, boxy);
            launches.lock().unwrap().clear();
            window
                .update(visual, |shell, _, cx| {
                    shell.sessions[&active].update(cx, |chat, _| {
                        chat.settings.worktree_cwd =
                            Some("/work/gespan/.worktrees/feature-login".into());
                    });
                })
                .unwrap();
            launch_from_menu(window, visual, "stub");
            assert_eq!(
                *launches.lock().unwrap(),
                [(
                    "stub".to_owned(),
                    "/work/gespan/.worktrees/feature-login".to_owned()
                )],
                "boxy {boxy}: the session's worktree, not the project root"
            );
        }
    }
}
