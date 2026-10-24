//! The title bar corner's Open in editor button (OpenInEditorButton.tsx):
//! the focused session's worktree or folder, opened in an installed editor.
use super::*;
use rebase_core::external_editor::{self, ExternalEditor};

pub(super) const EDITOR_ITEM_PREFIX: &str = "editor:";
/// The button and the gap before the next corner control.
pub(super) const OPEN_IN_EDITOR_WIDTH: f32 = 24. + CORNER_CONTROL_GAP;

/// The editors to offer, the last one used first and checked; the rest keep
/// the scan's order. `None` while the scan is still running.
pub(super) fn open_in_editor_menu(
    editors: Option<&[ExternalEditor]>,
    last: Option<&str>,
) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = match editors {
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
                        .into()
                })
                .collect()
        }
    };
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

/// Only a folder on this computer can open in one of its editors.
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
    /// The corner's file search and panel toggle, and the button when shown.
    pub(super) fn corner_buttons_width(&self, cx: &App) -> f32 {
        CORNER_BUTTONS_WIDTH + self.open_in_editor_room(cx)
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
    /// An open right panel shares the corner with its tool tabs and diff
    /// stat. Too narrow for all of them, the button steps aside instead of
    /// pushing the tabs off the panel's edge; the project menu still has it.
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
        self.workspace.right_panel_width
            >= crate::right_panel::TAB_ICONS_WIDTH
                + crate::right_panel::diff_stat_width(git.stats.as_ref())
                + CORNER_BUTTONS_WIDTH
                + OPEN_IN_EDITOR_WIDTH
                + padding
    }
    /// Original layout: the strip's end keeps the button `pl-1.5 pr-3` clear
    /// of the edge, where the new layouts have their corner.
    pub(super) fn classic_editor_room(&self, cx: &App) -> f32 {
        if self.workspace.classic_layout && self.open_in_editor_folder(cx).is_some() {
            6. + 24. + 12.
        } else {
            0.
        }
    }
    pub(super) fn open_in_editor_button(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let path = self.open_in_editor_folder(cx)?;
        if !self.workspace.classic_layout && self.open_in_editor_room(cx) == 0. {
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
                    this.open_menu(MenuTarget::OpenInEditor(path.clone()), anchor, window, cx);
                    this.rescan_editors_for_menu(cx);
                })),
        )
    }
    /// An editor installed since launch shows up the next time the menu opens.
    fn rescan_editors_for_menu(&mut self, cx: &mut Context<Self>) {
        let work = cx
            .background_executor()
            .spawn(async { external_editor::installed() });
        cx.spawn(async move |this, cx| {
            let editors = work.await;
            let _ = this.update(cx, |this, cx| {
                this.external_editors = Some(editors);
                let Some(open) = this
                    .menu
                    .as_ref()
                    .filter(|menu| matches!(&menu.target, MenuTarget::OpenInEditor(_)))
                else {
                    return;
                };
                let items = open_in_editor_menu(
                    this.external_editors.as_deref(),
                    this.workspace.last_external_editor.as_deref(),
                );
                open.menu
                    .clone()
                    .update(cx, |menu, cx| menu.replace_items(items, cx));
            });
        })
        .detach();
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
        if let Some(editor) = action.strip_prefix(EDITOR_ITEM_PREFIX) {
            let editor = editor.to_owned();
            let work = cx.background_executor().spawn({
                let editor = editor.clone();
                async move { external_editor::open(&editor, &path) }
            });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| match result {
                    Ok(()) => this.remember_external_editor(editor, cx),
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                });
            })
            .detach();
        } else if action == "reveal" {
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
        let items = open_in_editor_menu(Some(EDITORS), Some("webstorm"));
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
        let items = open_in_editor_menu(Some(EDITORS), Some("cursor"));
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
            rows(&open_in_editor_menu(None, None))[0],
            ("Looking for editors…".into(), false, true)
        );
        assert_eq!(
            rows(&open_in_editor_menu(Some(&[]), Some("zed"))),
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

    /// The new rounded layout, the new boxy one and the original layout.
    const LAYOUTS: [(&str, bool, bool); 3] = [
        ("new", false, false),
        ("new boxy", false, true),
        ("original", true, false),
    ];

    fn set_layout(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
        classic: bool,
        boxy: bool,
    ) {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.classic_layout = classic;
                shell.workspace.new_layout_boxy = boxy;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn button_holds_the_title_bar_corner_in_every_layout(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        for (name, classic, boxy) in LAYOUTS {
            set_layout(window, visual, classic, boxy);
            let button = visual
                .debug_bounds("titlebar-open-in-editor")
                .unwrap_or_else(|| panic!("{name}: no Open in editor button"));
            let strip = visual.debug_bounds("title-tab-strip").unwrap();
            let (viewport, right_drawn) = window
                .update(visual, |shell, w, _| {
                    (w.viewport_size().width, shell.right_drawn)
                })
                .unwrap();
            assert_eq!(button.size.width, px(24.), "{name}");
            assert!(
                button.top() >= px(0.) && button.bottom() <= px(TITLE_BAR_HEIGHT),
                "{name}: in the title bar"
            );
            assert!(
                strip.right() <= button.left(),
                "{name}: the tabs make room for it"
            );
            if classic {
                // The strip ends with it, `pr-3` clear of the column's edge.
                assert_eq!(
                    button.right(),
                    viewport - px(right_drawn) - px(12.),
                    "{name}"
                );
            } else {
                let search = visual.debug_bounds("titlebar-file-search").unwrap();
                let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
                assert_eq!(
                    search.left() - button.right(),
                    px(CORNER_CONTROL_GAP),
                    "{name}"
                );
                assert_eq!(toggle.right(), viewport - px(12.), "{name}");
            }
        }
    }

    #[gpui::test]
    fn button_toggles_the_menu_for_the_focused_folder(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        for (name, classic, boxy) in LAYOUTS {
            set_layout(window, visual, classic, boxy);
            let button = visual.debug_bounds("titlebar-open-in-editor").unwrap();
            visual.simulate_click(button.center(), Default::default());
            visual.run_until_parked();
            window
                .update(visual, |shell, _, _| {
                    let target = shell.menu.as_ref().map(|menu| menu.target.clone());
                    assert!(
                        target == Some(MenuTarget::OpenInEditor("/work/rebase".into())),
                        "{name}: the menu opens for the focused folder"
                    );
                })
                .unwrap();
            visual.refresh().unwrap();
            visual.simulate_click(button.center(), Default::default());
            visual.run_until_parked();
            window
                .update(visual, |shell, _, _| {
                    assert!(shell.menu.is_none(), "{name}: a second press closes it")
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn a_session_in_a_worktree_opens_the_worktree(cx: &mut TestAppContext) {
        let (window, _, active) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.open_in_editor_folder(cx).as_deref(),
                    Some("/work/rebase")
                );
                shell.sessions[&active].update(cx, |chat, _| {
                    chat.settings.worktree_cwd =
                        Some("/work/rebase/.worktrees/feature-login".into());
                });
                assert_eq!(
                    shell.open_in_editor_folder(cx).as_deref(),
                    Some("/work/rebase/.worktrees/feature-login")
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

    #[gpui::test]
    fn a_failed_launch_reports_why_and_is_not_remembered(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.run_open_in_editor_menu("/work/rebase".into(), "editor:not-an-editor", cx);
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
    fn the_remembered_editor_survives_a_restart(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        let saved = window
            .update(cx, |shell, _, cx| {
                shell.remember_external_editor("webstorm".into(), cx);
                serde_json::to_string(&shell.workspace).unwrap()
            })
            .unwrap();
        let restored = crate::workspace::Workspace::restore(Some(&saved), &[], "/work/rebase");
        assert_eq!(restored.last_external_editor.as_deref(), Some("webstorm"));
        let items = open_in_editor_menu(Some(EDITORS), restored.last_external_editor.as_deref());
        assert_eq!(rows(&items)[0], ("WebStorm".into(), true, false));
    }

    #[gpui::test]
    fn a_narrow_open_panel_keeps_its_tabs_and_drops_the_button(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, w, cx| shell.toggle_right_panel(w, cx))
            .unwrap();
        for boxy in [false, true] {
            set_layout(window, visual, false, boxy);
            for (width, shown) in [(crate::right_panel::WIDTH_MIN, false), (420., true)] {
                window
                    .update(visual, |shell, _, cx| {
                        shell.workspace.right_panel_width = width;
                        shell.invalidate_chrome(cx);
                        cx.notify();
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.refresh().unwrap();
                assert_eq!(
                    visual.debug_bounds("titlebar-open-in-editor").is_some(),
                    shown,
                    "boxy {boxy}, panel {width}px"
                );
                assert!(visual.debug_bounds("toggle-right-panel").is_some());
            }
        }
    }
}
