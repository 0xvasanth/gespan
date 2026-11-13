use crate::{
    input::{Changed, Input},
    settings::{
        SettingsView,
        management::Target,
        widgets::{glass_secondary, note},
    },
    theme::*,
};
use gespan_core::worktree_scripts::{self, Outcome, Record, Scripts};
use gpui::{AnyElement, Context, Entity, SharedString, div, prelude::*, px};

const SCRIPTS_NOTE: &str = if cfg!(windows) {
    "Setup runs in each new worktree before anything starts there, and Teardown before a worktree is removed. They run with cmd, and are kept in Gespan, never in the repository. %GESPAN_PROJECT_ROOT%, %GESPAN_WORKTREE_PATH% and %GESPAN_BRANCH% are set; put the paths in quotes."
} else {
    "Setup runs in each new worktree before anything starts there, and Teardown before a worktree is removed. They run with sh and stop at the first failing line, and are kept in Gespan, never in the repository. $GESPAN_PROJECT_ROOT, $GESPAN_WORKTREE_PATH and $GESPAN_BRANCH are set."
};

/// The Setup and Teardown fields of one project.
pub(in crate::settings) struct WorktreeScripts {
    pub(in crate::settings) setup: Entity<Input>,
    teardown: Entity<Input>,
    saved: Scripts,
    error: Option<String>,
}

/// A worktree row's line about its last Setup.
fn setup_line(record: &Record) -> Option<String> {
    let copied = (record.copied > 0).then(|| match record.copied {
        1 => "Copied 1 file".to_owned(),
        count => format!("Copied {count} files"),
    });
    let outcome = record.outcome.as_ref().map(|outcome| {
        match outcome {
            Outcome::Passed => "Setup passed",
            Outcome::Failed(_) => "Setup failed",
            Outcome::Overridden(_) => "Setup failed, started anyway",
            Outcome::Cancelled => "Setup cancelled",
        }
        .to_owned()
    });
    let parts: Vec<String> = copied.into_iter().chain(outcome).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// A worktree's pill: what its folder and working copy hold right now.
pub(super) fn worktree_state(
    missing: bool,
    dirty: Option<bool>,
) -> (&'static str, crate::glass::Tone) {
    use crate::glass::Tone;
    match (missing, dirty) {
        (true, _) => ("Missing folder", Tone::Bad),
        (false, Some(true)) => ("Uncommitted changes", Tone::Waiting),
        (false, Some(false)) => ("Clean", Tone::Ok),
        (false, None) => ("Status unavailable", Tone::Muted),
    }
}

impl SettingsView {
    /// The fields for `main`, read again when the project changes.
    pub(in crate::settings) fn scripts_for(&mut self, main: &str, cx: &mut Context<Self>) {
        if self.worktree_scripts.contains_key(main) {
            return;
        }
        let saved = worktree_scripts::scripts(std::path::Path::new(main));
        let field = |label: &str, placeholder: &str, text: &str, cx: &mut Context<Self>| {
            let text = text.to_owned();
            let input = cx.new(|cx| {
                let mut input = Input::multiline_field(label, placeholder, cx);
                input.set_text(text, cx);
                input
            });
            cx.subscribe(&input, |_, _, _: &Changed, cx| cx.notify())
                .detach();
            input
        };
        let setup = field("Setup script", "bun install", &saved.setup, cx);
        let teardown = field(
            "Teardown script",
            "docker compose down",
            &saved.teardown,
            cx,
        );
        self.worktree_scripts.insert(
            main.to_owned(),
            WorktreeScripts {
                setup,
                teardown,
                saved,
                error: None,
            },
        );
    }

    pub(in crate::settings) fn save_worktree_scripts(
        &mut self,
        main: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(fields) = self.worktree_scripts.get_mut(main) else {
            return;
        };
        let scripts = Scripts {
            setup: fields.setup.read(cx).content.to_string(),
            teardown: fields.teardown.read(cx).content.to_string(),
        };
        match worktree_scripts::save_scripts(std::path::Path::new(main), &scripts) {
            Ok(()) => {
                fields.saved = scripts;
                fields.error = None;
            }
            Err(error) => fields.error = Some(error),
        }
        cx.notify();
    }

    /// A project's own answer: `same` follows the app.
    fn project_choice(value: Option<bool>) -> &'static str {
        match value {
            None => "same",
            Some(true) => "on",
            Some(false) => "off",
        }
    }
    fn chosen(value: &str) -> Option<bool> {
        match value {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }
    }

    /// What the delete dialog starts with: for all projects, then for this one.
    fn delete_defaults_group(&mut self, cx: &mut Context<Self>) -> AnyElement {
        const ALL: [(&str, &str); 2] = [("off", "Off"), ("on", "On")];
        const PROJECT: [(&str, &str); 3] = [("same", "Same"), ("off", "Off"), ("on", "On")];
        let project = self.selected_project.clone();
        let own = self
            .prefs
            .project_worktrees
            .get(&project)
            .copied()
            .unwrap_or_default();
        let on = |value: bool| if value { "on" } else { "off" };
        let branch = vec![
            self.segmented(
                "worktree-delete-branch",
                "Delete merged branch in all projects",
                on(self.prefs.worktree_delete_branch),
                &ALL,
                cx,
                |this, value, cx| this.update(cx, |p| p.worktree_delete_branch = value == "on"),
            ),
            self.segmented(
                "worktree-delete-branch-project",
                "Delete merged branch in this project",
                Self::project_choice(own.delete_branch),
                &PROJECT,
                cx,
                {
                    let project = project.clone();
                    move |this, value, cx| {
                        let project = project.clone();
                        this.update(cx, |p| {
                            p.project_worktrees
                                .entry(project)
                                .or_default()
                                .delete_branch = Self::chosen(value);
                        });
                    }
                },
            ),
        ];
        let sessions = vec![
            self.segmented(
                "worktree-delete-sessions",
                "Delete sessions in all projects",
                on(self.prefs.worktree_delete_sessions),
                &ALL,
                cx,
                |this, value, cx| this.update(cx, |p| p.worktree_delete_sessions = value == "on"),
            ),
            self.segmented(
                "worktree-delete-sessions-project",
                "Delete sessions in this project",
                Self::project_choice(own.delete_sessions),
                &PROJECT,
                cx,
                move |this, value, cx| {
                    let project = project.clone();
                    this.update(cx, |p| {
                        p.project_worktrees
                            .entry(project)
                            .or_default()
                            .delete_sessions = Self::chosen(value);
                    });
                },
            ),
        ];
        let rows = vec![
            self.row(
                Some("worktree-delete-branch"),
                true,
                "Also delete the merged branch",
                Some("All projects, then this project. Same follows all projects.".into()),
                branch,
            ),
            self.row(
                Some("worktree-delete-sessions"),
                false,
                "Also delete associated sessions",
                Some("All projects, then this project. Same follows all projects.".into()),
                sessions,
            ),
        ];
        self.group(
            Some("worktree-delete-defaults"),
            false,
            "When deleting a worktree",
            Some("What the delete dialog starts with. You can still change it there.".into()),
            None,
            rows,
        )
    }

    fn scripts_group(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let main = self.discovery.worktrees.main.clone();
        if main.is_empty() {
            return None;
        }
        self.scripts_for(&main, cx);
        let fields = self.worktree_scripts.get(&main)?;
        let changed = fields.setup.read(cx).content.as_str() != fields.saved.setup
            || fields.teardown.read(cx).content.as_str() != fields.saved.teardown;
        let field = |label: &'static str, input: &Entity<Input>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_size(px(12.))
                .text_color(content(0.55))
                .child(label)
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(content(0.12))
                        .text_color(content(0.9))
                        .child(input.clone()),
                )
        };
        Some(
            div()
                .id("settings-worktree-scripts")
                .debug_selector(|| "settings-worktree-scripts".into())
                .mt_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(content(0.5))
                        .child(SCRIPTS_NOTE),
                )
                .child(field("Setup script", &fields.setup))
                .child(field("Teardown script", &fields.teardown))
                .children(fields.error.clone().map(|error| {
                    div()
                        .id("settings-worktree-scripts-error")
                        .role(gpui::Role::Alert)
                        .text_size(px(12.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(error)
                }))
                .child(
                    div().flex().justify_end().child(
                        glass_secondary("save-worktree-scripts", "Save scripts", false, !changed)
                            .child("Save scripts")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if changed {
                                    this.save_worktree_scripts(&main, cx);
                                }
                            })),
                    ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn worktrees_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let project = self.project_picker(cx);
        let ready = !self.discovery_loading
            && self.discovery.worktrees.error.is_none()
            && !self.discovery.worktrees.main.is_empty();
        let mut page = vec![
            div()
                .relative()
                .child(self.reveal_probe("project-worktrees"))
                .flex()
                .items_center()
                .gap_1()
                .child(project)
                .child(
                    glass_secondary("create-settings-worktree", "Create worktree", false, !ready)
                        .child(icon("plus", 12.))
                        .child("Create worktree")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if ready {
                                this.open_management(Target::Worktree, window, cx);
                            }
                        })),
                )
                .into_any_element(),
            {
                let refresh = self.refresh_button("refresh-settings-worktrees", "Refresh", cx);
                self.glass_group(
                None,
                "Worktrees",
                Some("Sessions can share a worktree. Deleting one keeps its sessions by default and discards uncommitted changes. Its branch and commits are kept unless the branch is merged and you choose to delete it.".into()),
                Some(refresh),
                None,
                Vec::new(),
            )
            },
        ];
        if self.selected_project.is_empty() {
            page.push(
                div()
                    .mt_4()
                    .child(crate::settings::widgets::empty_card(
                        "settings-worktrees-no-project",
                        "folder-add",
                        "No project yet",
                        "Add a project to manage its worktrees.",
                    ))
                    .into_any_element(),
            );
        } else if self.discovery_loading {
            page.push(note("Loading worktrees…"));
        } else if self
            .discovery
            .worktrees
            .error
            .as_deref()
            .is_some_and(|e| e.contains("not a git repository"))
        {
            page.push(
                div()
                    .mt_4()
                    .child(crate::settings::widgets::empty_card(
                        "settings-worktrees-not-git",
                        "git-branch",
                        "Not a Git repository",
                        "Worktrees need a Git project. Run git init in this folder, or pick another project.",
                    ))
                    .into_any_element(),
            );
        } else if let Some(error) = &self.discovery.worktrees.error {
            page.push(
                div()
                    .id("settings-page-error")
                    .mt_4()
                    .role(gpui::Role::Alert)
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xf87171))
                    .child(error.clone())
                    .into_any_element(),
            );
        } else if self.discovery.worktrees.trees.is_empty() {
            page.push(
                div()
                    .mt_4()
                    .child(crate::settings::widgets::empty_card(
                        "settings-worktrees-empty",
                        "folder-tree",
                        "No additional worktrees",
                        "Create a worktree to work on another branch in a separate folder.",
                    ))
                    .into_any_element(),
            );
        } else {
            let trees = self.discovery.worktrees.trees.clone();
            page.push(
                div()
                    .mt_4()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(crate::glass::edge())
                    .bg(crate::glass::fill())
                    .overflow_hidden()
                    .children(trees.into_iter().enumerate().map(|(ix, tree)| {
                        let t = &tree.tree;
                        let path = std::path::PathBuf::from(&t.path);
                        let count = self
                            .ctx
                            .sessions
                            .iter()
                            .filter(|(_, p, _)| gespan_core::files::is_equal_or_inside(p, &t.path))
                            .count();
                        let (state, tone) = worktree_state(t.missing, tree.dirty);
                        let blocked = tree.locked || t.branch.is_none();
                        let branch = t
                            .branch
                            .as_ref()
                            .map(|b| format!("Current branch: {b}"))
                            .unwrap_or_else(|| {
                                format!(
                                    "Detached at {}",
                                    t.head.chars().take(7).collect::<String>()
                                )
                            });
                        let mut meta = format!(
                            "{branch}   {count} {} in this worktree",
                            if count == 1 { "session" } else { "sessions" }
                        );
                        if tree.unpushed > 0 {
                            meta.push_str(&format!(" · {} unpublished commits", tree.unpushed));
                        }
                        div()
                            .px_4()
                            .py(px(14.))
                            .flex()
                            .items_start()
                            .gap(px(14.))
                            .when(ix > 0, |d| {
                                d.border_t_1().border_color(crate::glass::hairline())
                            })
                            .child(
                                div()
                                    .size(px(32.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(9.))
                                    .bg(content(0.07))
                                    .border_1()
                                    .border_color(crate::glass::edge())
                                    .child(icon("git-branch", 16.).text_color(content(0.8))),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .child(
                                                        path.file_name()
                                                            .unwrap_or_default()
                                                            .to_string_lossy()
                                                            .into_owned(),
                                                    ),
                                            )
                                            .child(
                                                crate::glass::status(state, tone).debug_selector(
                                                    move || format!("tree-state-{ix}"),
                                                ),
                                            )
                                            .when(tree.locked, |d| {
                                                d.child(crate::glass::status(
                                                    "Locked",
                                                    crate::glass::Tone::Muted,
                                                ))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .truncate()
                                            .font_family(crate::fonts::monospace())
                                            .text_size(px(11.5))
                                            .text_color(content(0.55))
                                            .child(t.path.clone()),
                                    )
                                    .child(
                                        div()
                                            .mt(px(6.))
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .text_size(px(11.5))
                                            .text_color(content(0.55))
                                            .child(icon("git-branch", 12.))
                                            .child(meta),
                                    )
                                    .when_some(
                                        tree.setup.as_ref().and_then(setup_line),
                                        |row, line| {
                                            row.child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "tree-setup-{ix}"
                                                    )))
                                                    .debug_selector(move || {
                                                        format!("tree-setup-{ix}")
                                                    })
                                                    .mt_2()
                                                    .text_size(px(11.))
                                                    .text_color(content(0.55))
                                                    .child(line),
                                            )
                                        },
                                    )
                                    .when_some(tree.merged.as_ref(), |row, merged| {
                                        row.child(
                                            div()
                                                .id(SharedString::from(format!("tree-merged-{ix}")))
                                                .mt_2()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .text_size(px(11.))
                                                .text_color(content(0.55))
                                                .child(icon("check", 12.))
                                                .child(format!("Merged: {}", merged.reason())),
                                        )
                                    }),
                            )
                            .child(
                                crate::glass::icon_button(
                                    SharedString::from(format!("reveal-tree-{ix}")),
                                    "folder-open",
                                )
                                .role(gpui::Role::Button)
                                .aria_label(format!(
                                    "Reveal {}",
                                    t.branch.as_deref().unwrap_or("worktree")
                                ))
                                .when(t.missing, |d| d.opacity(0.4))
                                .when(!t.missing, |d| {
                                    d.on_click(move |_, _, cx| cx.reveal_path(&path))
                                }),
                            )
                            .child(
                                crate::glass::icon_button(
                                    SharedString::from(format!("delete-tree-{ix}")),
                                    "trash",
                                )
                                .role(gpui::Role::Button)
                                .aria_label(format!(
                                    "Delete {}",
                                    t.branch.as_deref().unwrap_or("worktree")
                                ))
                                .when(blocked, |d| d.opacity(0.4))
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if !blocked {
                                            this.open_management(
                                                Target::DeleteTree(tree.clone()),
                                                window,
                                                cx,
                                            );
                                        }
                                    },
                                )),
                            )
                    }))
                    .into_any_element(),
            );
        }
        if ready {
            page.extend(self.scripts_group(cx));
            page.push(self.delete_defaults_group(cx));
            page.push(note(format!(
                "New worktrees are created in {}.",
                self.discovery.worktrees.default_root.display()
            )));
        }
        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_row_says_what_its_setup_did() {
        let line = |copied, outcome| setup_line(&Record { copied, outcome });
        assert_eq!(line(0, None), None);
        assert_eq!(line(1, None).unwrap(), "Copied 1 file");
        assert_eq!(
            line(3, Some(Outcome::Passed)).unwrap(),
            "Copied 3 files · Setup passed"
        );
        assert_eq!(
            line(0, Some(Outcome::Failed("x".into()))).unwrap(),
            "Setup failed"
        );
        assert_eq!(
            line(0, Some(Outcome::Overridden("x".into()))).unwrap(),
            "Setup failed, started anyway"
        );
        assert_eq!(
            line(0, Some(Outcome::Cancelled)).unwrap(),
            "Setup cancelled"
        );
    }

    #[test]
    fn a_worktree_pill_says_what_its_folder_holds() {
        use crate::glass::Tone;
        assert_eq!(worktree_state(false, Some(false)), ("Clean", Tone::Ok));
        assert_eq!(
            worktree_state(false, Some(true)),
            ("Uncommitted changes", Tone::Waiting)
        );
        // A missing folder wins over whatever the last status said.
        assert_eq!(
            worktree_state(true, Some(false)),
            ("Missing folder", Tone::Bad)
        );
        assert_eq!(
            worktree_state(false, None),
            ("Status unavailable", Tone::Muted)
        );
    }
}
