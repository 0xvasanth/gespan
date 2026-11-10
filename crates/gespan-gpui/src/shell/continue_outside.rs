//! Continue outside: a session's own agent command, so it can be picked up
//! in a terminal outside Gespan. A session is never locked in. The chip under
//! each composer opens this panel.
use super::*;
use crate::tab_tile::{Face, TileSize, mark_tile};
use gespan_core::provider::Kind;
use gespan_core::resume_outside::{self, Resume, ShellStyle};
use gpui::EventEmitter;

/// The agent's brand tile at the head of the panel.
const HEADER_TILE: TileSize = TileSize {
    w: 34.,
    h: 30.,
    radius: 8.,
};
const TERMINAL_APP_LABEL: &str = if cfg!(target_os = "macos") {
    "Terminal app"
} else {
    "New window"
};

/// What the focused session offers outside Gespan.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Outside {
    /// The provider id, for its logo and tile colour.
    pub(super) provider: String,
    pub(super) agent: &'static str,
    /// Where the agent runs: the session's worktree, else its folder.
    pub(super) cwd: String,
    pub(super) resume: Resume,
}

impl Outside {
    pub(super) fn new(provider: &str, provider_id: Option<&str>, cwd: String) -> Self {
        let kind = Kind::from_id(provider);
        Self {
            provider: provider.to_owned(),
            agent: kind.label(),
            cwd,
            resume: resume_outside::resume(kind, provider_id),
        }
    }
    /// The line to paste in any terminal.
    pub(super) fn line(&self) -> Option<String> {
        self.resume.line(&self.cwd, ShellStyle::native())
    }
    /// The agent's own session id, whatever else the panel can offer.
    fn session_id(&self) -> Option<&str> {
        match &self.resume {
            Resume::Command { args, .. } => args.last().map(String::as_str),
            Resume::IdOnly(id) => Some(id),
            Resume::NotStarted => None,
        }
    }
}

/// What the panel says for `outside`, kept apart from drawing so each case
/// can be checked.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PanelCopy {
    pub(super) title: &'static str,
    pub(super) subtitle: String,
    /// The monospace box: the command, or the id for an agent without one.
    pub(super) code: Option<String>,
    /// The quiet line under the box, or the reason there is nothing yet.
    pub(super) note: String,
    /// Whether the dock and terminal app buttons apply.
    pub(super) runnable: bool,
}

pub(super) fn panel_copy(outside: &Outside) -> PanelCopy {
    let agent = outside.agent;
    let folder = gespan_core::files::leaf_name(&outside.cwd);
    let subtitle = format!("{agent} · {folder}");
    match &outside.resume {
        // A folder name no shell quoting keeps literal: the id, to run by hand.
        Resume::Command { .. } if outside.line().is_none() => PanelCopy {
            title: "Continue outside Gespan",
            subtitle,
            code: outside.session_id().map(str::to_owned),
            note: format!(
                "This folder's name can't go into a command safely. Open a terminal in {folder} and resume {agent} with this session ID."
            ),
            runnable: false,
        },
        Resume::Command { .. } => PanelCopy {
            title: "Continue in your terminal",
            subtitle,
            code: outside.resume.command_line(ShellStyle::native()),
            note: format!("Copy adds cd {folder}, so it works from any terminal."),
            runnable: true,
        },
        Resume::IdOnly(id) => PanelCopy {
            title: "Continue outside Gespan",
            subtitle,
            code: Some(id.clone()),
            note: format!(
                "{agent} has no resume command Gespan has checked yet. Use this session ID to resume it yourself."
            ),
            runnable: false,
        },
        Resume::NotStarted => PanelCopy {
            title: "Continue outside Gespan",
            subtitle,
            code: None,
            note: format!(
                "{agent} hasn't started this session yet. Send a message first, then continue it anywhere."
            ),
            runnable: false,
        },
    }
}

/// A button in the panel was pressed: "dock" or "terminal-app".
pub(super) struct OutsidePicked(pub(super) &'static str);

/// The Continue outside popover's body.
pub(super) struct OutsidePanel {
    focus: FocusHandle,
    outside: Outside,
    pub(super) error: Option<String>,
    copied: bool,
    copies: u64,
}
impl EventEmitter<OutsidePicked> for OutsidePanel {}

impl OutsidePanel {
    pub(super) fn new(outside: Outside, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            outside,
            error: None,
            copied: false,
            copies: 0,
        }
    }
    pub(super) fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }
    /// The whole line for a command, the id alone otherwise.
    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self
            .outside
            .line()
            .or_else(|| self.outside.session_id().map(str::to_owned));
        let Some(text) = text else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = true;
        self.copies += 1;
        let copy = self.copies;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1600))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.copies == copy {
                    this.copied = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

fn panel_button(
    id: &'static str,
    label: &'static str,
    icon_path: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .flex_1()
        .min_w_0()
        .h(px(30.))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(6.))
        .bg(content(0.06))
        .border_1()
        .border_color(content(0.06))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.1)))
        .child(icon(icon_path, 14.).text_color(content(0.7)))
        .child(
            div()
                .text_size(px(12.))
                .text_color(content(0.9))
                .truncate()
                .child(label),
        )
}

impl Render for OutsidePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let copy = panel_copy(&self.outside);
        let copied = self.copied;
        div()
            .track_focus(&self.focus)
            .p_2()
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(self.error.clone().map(|error| {
                div()
                    .px_2()
                    .py_1p5()
                    .rounded(px(6.))
                    .bg(gpui::rgba(0xf43f5e1f))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(content(0.9))
                    .child(error)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(mark_tile(
                        &HEADER_TILE,
                        Face::provider(&self.outside.provider),
                        provider_glyph(&self.outside.provider, 16.),
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(0.95))
                                    .child(copy.title),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(content(0.5))
                                    .truncate()
                                    .child(copy.subtitle),
                            ),
                    ),
            )
            .children(copy.code.map(|code| {
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .pl(px(10.))
                    .pr(px(4.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .bg(content(0.04))
                    .border_1()
                    .border_color(content(0.08))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py(px(4.))
                            .font_family(crate::fonts::monospace())
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .text_color(content(0.9))
                            .child(code),
                    )
                    .child(
                        div()
                            .id("continue-outside-copy")
                            .debug_selector(|| "continue-outside-copy".into())
                            .role(Role::Button)
                            .aria_label(if copied { "Copied" } else { "Copy" })
                            .flex_shrink_0()
                            .h(px(26.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.08)))
                            .when(copied, |d| d.bg(accent_color().opacity(0.14)))
                            .child(
                                icon(if copied { "check" } else { "copy" }, 13.)
                                    .text_color(if copied { accent_color() } else { content(0.6) }),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(if copied { accent_color() } else { content(0.6) })
                                    .child(if copied { "Copied" } else { "Copy" }),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                    )
            }))
            .when(copy.runnable, |d| {
                d.child(
                    div()
                        .flex()
                        .gap_1p5()
                        .child(
                            panel_button("continue-outside-dock", "Terminal dock", "terminal")
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(OutsidePicked("dock"))),
                                ),
                        )
                        .child(
                            panel_button(
                                "continue-outside-terminal-app",
                                TERMINAL_APP_LABEL,
                                "external-link",
                            )
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(OutsidePicked("terminal-app"))),
                            ),
                        ),
                )
            })
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(content(0.45))
                    .child(copy.note),
            )
    }
}

impl Shell {
    /// Session `id`'s way out. Remote projects have none.
    pub(super) fn continue_outside(&self, id: i64, cx: &App) -> Option<Outside> {
        let view = self.sessions.get(&id)?.read(cx);
        let cwd = view
            .settings
            .worktree_cwd
            .clone()
            .filter(|_| !view.settings.worktree_removed)
            .unwrap_or_else(|| view.summary.cwd.clone());
        if cwd.is_empty() || cwd.starts_with("remote://") {
            return None;
        }
        Some(Outside::new(
            &view.summary.provider,
            view.summary.provider_id.as_deref(),
            cwd,
        ))
    }
    pub(super) fn observe_continue_outside(
        &mut self,
        view: &Entity<Conversation>,
        id: i64,
        cx: &mut Context<Self>,
    ) {
        self.observations.push(cx.subscribe(
            view,
            move |this, _, event: &crate::conversation::ContinueOutside, cx| {
                this.pending_continue = Some((id, event.0.clone()));
                cx.notify();
            },
        ));
    }
    /// Opens a chip's panel once a render has the window, or closes it when
    /// the same chip is pressed again.
    pub(super) fn open_pending_continue(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (id, anchor) = self.pending_continue.take()?;
        let target = MenuTarget::ContinueOutside(id);
        if self.menu_open(&target) {
            self.close_menu(cx);
        } else if self.sessions.contains_key(&id) {
            self.open_menu(target, Anchor::Slot(anchor), window, cx);
        }
        None
    }
    /// The popover's one row: the panel, wired back to the shell.
    pub(super) fn continue_outside_items(
        &self,
        id: i64,
        events: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<MenuItem> {
        let Some(outside) = self.continue_outside(id, cx) else {
            return Vec::new();
        };
        let panel = cx.new(|cx| OutsidePanel::new(outside, cx));
        events.push(cx.subscribe_in(
            &panel,
            window,
            move |this, _, OutsidePicked(action), window, cx| match *action {
                "terminal-app" => this.resume_in_terminal_app(id, cx),
                action => {
                    this.run_continue_outside_menu(id, action, window, cx);
                    this.close_menu(cx);
                }
            },
        ));
        let focus = panel.read(cx).focus();
        vec![MenuItem::Custom(panel.into(), focus)]
    }
    pub(super) fn run_continue_outside_menu(
        &mut self,
        id: i64,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(outside) = self.continue_outside(id, cx) else {
            return;
        };
        if action == "dock"
            && let Some(command) = outside
                .resume
                .clean_command_line(ShellStyle::native(), |name| {
                    std::env::var_os(name).is_some()
                })
        {
            self.resume_in_dock(outside.cwd, &command, window, cx);
        }
    }
    /// Starts the system terminal off the UI thread; the panel stays open
    /// until it settles so a failure can show in it.
    pub(super) fn resume_in_terminal_app(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(outside) = self.continue_outside(id, cx) else {
            return;
        };
        let style = ShellStyle::native();
        let (Some(line), Some(command)) = (outside.line(), outside.resume.command_line(style))
        else {
            return;
        };
        let open = self.terminal_app.clone();
        let work = cx
            .background_executor()
            .spawn(async move { open(&outside.cwd, &line, &command) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| this.finish_terminal_app(result, cx));
        })
        .detach();
    }
    pub(super) fn finish_terminal_app(
        &mut self,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let panel = self.continue_outside_panel(cx);
        match (result, panel) {
            (Ok(()), Some(_)) => self.close_menu(cx),
            (Ok(()), None) => {}
            (Err(error), Some(panel)) => panel.update(cx, |panel, cx| {
                panel.error = Some(error);
                cx.notify();
            }),
            (Err(error), None) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    /// The open popover's panel.
    pub(super) fn continue_outside_panel(&self, cx: &App) -> Option<Entity<OutsidePanel>> {
        self.menu
            .as_ref()
            .filter(|open| matches!(open.target, MenuTarget::ContinueOutside(_)))
            .and_then(|open| open.menu.read(cx).custom_view::<OutsidePanel>())
    }
    /// A new dock terminal in the session's folder, running `command`.
    fn resume_in_dock(
        &mut self,
        cwd: String,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.workspace.project.clone();
        self.ensure_terminals(&project, cx);
        let in_panel = self.terminal_in_panel(cx);
        let viewport = self.viewport();
        let dock = self.workspace.docks.entry(project.clone()).or_default();
        let was_open = dock.open && !dock.terminals.is_empty();
        dock.size = dock.side.clamp(dock.size, viewport);
        dock.add(cwd.clone());
        dock.open = !in_panel;
        let terminal = self.spawn_terminal(cwd, &[], cx);
        self.terminals
            .entry(project.clone())
            .or_default()
            .push(terminal.clone());
        terminal.update(cx, |t, cx| t.run(command, cx));
        if in_panel {
            self.show_panel_terminal(window, cx);
        } else {
            if let Some(dock) = self.workspace.docks.get(&project) {
                self.dock_tab_scroll.scroll_to_item(dock.active);
            }
            if !was_open {
                self.slide_dock(true, cx);
            }
            self.focus_terminal(window, cx);
            self.fit_sidebar(cx);
        }
        self.persist(cx);
        cx.notify();
    }
}

/// Opens a system terminal in a folder (`cwd`, `line`, `command`, as
/// `resume_outside::open_in_terminal` takes them). Tests swap in a stub so
/// nothing real launches.
pub(super) type OpenTerminalApp = Arc<dyn Fn(&str, &str, &str) -> Result<(), String> + Send + Sync>;

pub(super) fn default_terminal_app() -> OpenTerminalApp {
    Arc::new(resume_outside::open_in_terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    const ID: &str = "ee1ca4af-bcce-4cbe-90ab-1f76e13c9ef0";

    #[test]
    fn a_resumable_session_shows_its_command_and_both_ways_to_run_it() {
        let outside = Outside::new("claude", Some(ID), "/work/gespan".into());
        let copy = panel_copy(&outside);
        assert_eq!(copy.title, "Continue in your terminal");
        assert_eq!(copy.subtitle, "Claude Code · gespan");
        // The box shows the command alone; Copy adds the cd.
        assert_eq!(copy.code, Some(format!("claude --resume {ID}")));
        assert!(copy.note.contains("cd gespan"), "{}", copy.note);
        assert!(copy.runnable);
    }

    #[test]
    fn a_folder_no_quoting_keeps_literal_offers_the_id_to_run_by_hand() {
        let cwd = if cfg!(windows) {
            r"C:\work\%USERNAME%"
        } else {
            "/work/a\nb"
        };
        let outside = Outside::new("claude", Some(ID), cwd.into());
        let copy = panel_copy(&outside);
        assert_eq!(copy.code.as_deref(), Some(ID), "the id, not a command");
        assert!(!copy.runnable, "no dock or terminal app");
        assert!(copy.note.contains("can't go into a command safely"));
    }

    #[test]
    fn an_agent_without_a_checked_command_offers_only_its_id() {
        let outside = Outside::new("grok", Some("g-1"), "/work/gespan".into());
        let copy = panel_copy(&outside);
        assert_eq!(copy.code.as_deref(), Some("g-1"));
        assert!(copy.note.starts_with("Grok"), "{}", copy.note);
        assert!(!copy.runnable, "no command to run");
    }

    #[test]
    fn a_session_the_agent_never_started_explains_itself() {
        let outside = Outside::new("codex", None, "/work/gespan".into());
        let copy = panel_copy(&outside);
        assert_eq!(copy.code, None);
        assert!(!copy.runnable);
        assert!(copy.note.contains("hasn't started this session yet"));
    }

    fn line(command: &str) -> String {
        if cfg!(windows) {
            format!("cd /d /work/gespan && {command}")
        } else {
            format!("cd /work/gespan && {command}")
        }
    }

    fn set_provider_id(
        window: WindowHandle<Shell>,
        cx: &mut TestAppContext,
        active: i64,
        id: Option<&str>,
    ) {
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&active].update(cx, |chat, _| {
                    chat.summary.provider_id = id.map(str::to_owned)
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_focused_session_runs_in_its_worktree(cx: &mut TestAppContext) {
        let (window, _, active) = window(cx);
        set_provider_id(window, cx, active, Some(ID));
        window
            .update(cx, |shell, _, cx| {
                let outside = shell.continue_outside(active, cx).unwrap();
                assert_eq!(outside.agent, "Claude Code");
                assert_eq!(outside.line(), Some(line(&format!("claude --resume {ID}"))));
                shell.sessions[&active].update(cx, |chat, _| {
                    chat.settings.worktree_cwd = Some("/work/gespan/.worktrees/login".into());
                });
                assert_eq!(
                    shell.continue_outside(active, cx).unwrap().cwd,
                    "/work/gespan/.worktrees/login"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn clearing_the_agent_id_takes_the_command_away(cx: &mut TestAppContext) {
        let (window, _, active) = window(cx);
        set_provider_id(window, cx, active, Some(ID));
        set_provider_id(window, cx, active, None);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.continue_outside(active, cx).unwrap().resume,
                    Resume::NotStarted
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_remote_project_has_no_way_out_here(cx: &mut TestAppContext) {
        let (window, _, active) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&active].update(cx, |chat, _| {
                    chat.summary.cwd = "remote://host-1/srv/app".into();
                });
                assert_eq!(shell.continue_outside(active, cx), None);
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_chip_sits_under_the_composer_not_in_the_title_bar(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        let chip = visual.debug_bounds("continue-outside").expect("chip shown");
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        assert!(chip.top() > toggle.bottom(), "below the title bar");
        assert!(visual.debug_bounds("titlebar-continue-outside").is_none());
    }

    #[gpui::test]
    fn pressing_the_chip_toggles_its_own_sessions_panel(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, active) = window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let press = |visual: &mut VisualTestContext| {
            window
                .update(visual, |shell, _, cx| {
                    shell.sessions[&active].update(cx, |_, cx| {
                        cx.emit(crate::conversation::ContinueOutside(AnchorSlot::default()))
                    });
                })
                .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
        };
        press(visual);
        window
            .update(visual, |shell, _, cx| {
                assert!(shell.menu_open(&MenuTarget::ContinueOutside(active)));
                assert!(shell.continue_outside_panel(cx).is_some());
            })
            .unwrap();
        press(visual);
        window
            .update(visual, |shell, _, _| {
                assert!(shell.menu.is_none(), "closed")
            })
            .unwrap();
    }

    fn open_continue_menu(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
        active: i64,
    ) {
        window
            .update(visual, |shell, w, cx| {
                shell.open_menu(
                    MenuTarget::ContinueOutside(active),
                    Anchor::Point(Default::default()),
                    w,
                    cx,
                );
            })
            .unwrap();
        visual.run_until_parked();
    }

    #[gpui::test]
    fn copy_command_puts_the_whole_line_on_the_clipboard(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, active) = window(cx);
        set_provider_id(window, cx, active, Some(ID));
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        open_continue_menu(window, visual, active);
        window
            .update(visual, |shell, _, cx| {
                let panel = shell.continue_outside_panel(cx).expect("panel open");
                panel.update(cx, |panel, cx| panel.copy(cx));
            })
            .unwrap();
        let copied = visual.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied, Some(line(&format!("claude --resume {ID}"))));
    }

    #[gpui::test]
    fn the_dock_opens_a_terminal_in_the_sessions_folder(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, active) = window(cx);
        set_provider_id(window, cx, active, Some(ID));
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, _, cx| {
                shell.sessions[&active].update(cx, |chat, _| {
                    chat.settings.worktree_cwd = Some("/work/gespan/.worktrees/login".into());
                });
            })
            .unwrap();
        let before = window
            .update(visual, |shell, _, _| {
                shell.terminals.get("/work/gespan").map_or(0, Vec::len)
            })
            .unwrap();
        window
            .update(visual, |shell, w, cx| {
                shell.run_continue_outside_menu(active, "dock", w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let terminals = &shell.terminals["/work/gespan"];
                assert_eq!(terminals.len(), before + 1, "one new tab");
                let terminal = terminals.last().unwrap().read(cx);
                assert_eq!(terminal.cwd, "/work/gespan/.worktrees/login");
                assert!(shell.workspace.docks["/work/gespan"].open, "dock shown");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_failed_terminal_app_stays_in_the_open_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, active) = window(cx);
        set_provider_id(window, cx, active, Some(ID));
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let launched = Arc::new(Mutex::new(Vec::<String>::new()));
        window
            .update(visual, |shell, _, _| {
                let launched = launched.clone();
                shell.terminal_app = Arc::new(move |_, line, _| {
                    launched.lock().unwrap().push(line.to_owned());
                    Err("No terminal app found. Copy the command instead.".into())
                });
            })
            .unwrap();
        open_continue_menu(window, visual, active);
        window
            .update(visual, |shell, _, cx| {
                shell.resume_in_terminal_app(active, cx)
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            *launched.lock().unwrap(),
            [line(&format!("claude --resume {ID}"))]
        );
        window
            .update(visual, |shell, _, cx| {
                let panel = shell.continue_outside_panel(cx).expect("menu stays open");
                assert_eq!(
                    panel.read(cx).error.as_deref(),
                    Some("No terminal app found. Copy the command instead.")
                );
                assert_eq!(shell.error, None, "shown in the menu, not the banner");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_started_terminal_app_closes_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, active) = window(cx);
        set_provider_id(window, cx, active, Some(ID));
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, _, _| {
                shell.terminal_app = Arc::new(|_, _, _| Ok(()))
            })
            .unwrap();
        open_continue_menu(window, visual, active);
        window
            .update(visual, |shell, _, cx| {
                shell.resume_in_terminal_app(active, cx)
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.menu.is_none()))
            .unwrap();
    }
}
