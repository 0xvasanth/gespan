//! SessionReview.tsx: the latest turn's recorded workspace changes, under its
//! reply, with Undo, Keep and Review for the whole turn or one file at a time.
//! The checkpoint runs off the UI thread.
use crate::{
    file_icons::file_type_icon,
    right_panel::{DIFF_ADD, DIFF_DEL, format_integer, tabular},
    theme::*,
};
use gespan_core::checkpoint::{CheckpointFile, CheckpointStatus};
use gpui::{
    Context, EventEmitter, FontWeight, Role, SharedString, Task, Window, div, prelude::*, px, rgb,
};

/// The card shows this many files until expanded.
const COLLAPSED: usize = 3;

pub(crate) enum SessionReviewEvent {
    /// Review the session's changes, focused on one file when given.
    Open { path: Option<String> },
    /// Undo or Keep rewrote the workspace.
    Changed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Keep,
    Undo,
}

pub(crate) struct SessionReview {
    session: i64,
    cwd: String,
    files: Vec<CheckpointFile>,
    error: Option<String>,
    expanded: bool,
    acting: Option<Action>,
    /// The card represents a settled turn, so it hides while one runs.
    busy: bool,
    /// Another session is running in this project.
    undo_locked: bool,
    generation: u64,
    _load: Option<Task<()>>,
}
impl EventEmitter<SessionReviewEvent> for SessionReview {}

impl SessionReview {
    pub(crate) fn new(session: i64, cwd: String) -> Self {
        Self {
            session,
            cwd,
            files: Vec::new(),
            error: None,
            expanded: false,
            acting: None,
            busy: false,
            undo_locked: false,
            generation: 0,
            _load: None,
        }
    }
    pub(crate) fn session(&self) -> (i64, &str) {
        (self.session, &self.cwd)
    }
    fn valid(&self) -> bool {
        !self.cwd.is_empty() && self.cwd != "~"
    }
    /// The checkout the session works in; a worktree replaces the project.
    pub(crate) fn set_cwd(&mut self, cwd: String, cx: &mut Context<Self>) {
        if self.cwd != cwd {
            self.cwd = cwd;
            self.publish(Ok(CheckpointStatus::default()), cx);
            self.refresh(cx);
        }
    }
    /// sessionReviewCache.startTurn: a new turn clears the card until it settles.
    pub(crate) fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        if self.busy == busy {
            return;
        }
        self.busy = busy;
        // The settled turn is reread by the conversation's refresh.
        if busy {
            self.generation += 1;
            self._load = None;
            self.publish(Ok(CheckpointStatus::default()), cx);
        }
        cx.notify();
    }
    pub(crate) fn set_undo_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        if self.undo_locked != locked {
            self.undo_locked = locked;
            cx.notify();
        }
    }
    /// The card has a settled result or an error to show.
    pub(crate) fn shown(&self) -> bool {
        !self.busy && (self.error.is_some() || !self.files.is_empty())
    }
    #[cfg(test)]
    pub(crate) fn set_files(&mut self, files: Vec<CheckpointFile>, cx: &mut Context<Self>) {
        self.publish(Ok(CheckpointStatus { files }), cx);
    }
    /// Reread the recorded turn, as after a turn, Git change or reload.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.valid() || self.busy {
            return;
        }
        let Some(store) = gespan_core::checkpoint::store() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let (session, cwd) = (self.session.to_string(), self.cwd.clone());
        let work = cx
            .background_executor()
            .spawn(async move { store.exclusive(|s| s.status(&session, &cwd)) });
        self._load = Some(cx.spawn(async move |this, cx| {
            let status = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && !this.busy {
                    this.publish(status, cx);
                }
            });
        }));
    }
    fn publish(&mut self, status: Result<CheckpointStatus, String>, cx: &mut Context<Self>) {
        let (files, error) = match status {
            Ok(status) => (status.files, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        if files.len() <= COLLAPSED {
            self.expanded = false;
        }
        if self.files != files || self.error != error {
            self.files = files;
            self.error = error;
            cx.notify();
        }
    }
    /// Undo or Keep every recorded file, or only `relative`.
    fn run(&mut self, action: Action, relative: Option<String>, cx: &mut Context<Self>) {
        if self.acting.is_some() {
            return;
        }
        let Some(store) = gespan_core::checkpoint::store() else {
            return;
        };
        self.acting = Some(action);
        cx.notify();
        self.generation += 1;
        let generation = self.generation;
        let (session, cwd) = (self.session.to_string(), self.cwd.clone());
        let work = cx.background_executor().spawn(async move {
            let relative = relative.as_deref();
            store.exclusive(|s| match action {
                Action::Keep => s.keep(&session, &cwd, relative),
                Action::Undo => s.undo(&session, &cwd, relative),
            })
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let status = work.await;
            let _ = this.update(cx, |this, cx| {
                this.acting = None;
                match status {
                    Ok(status) if this.generation == generation => {
                        this.publish(Ok(status), cx);
                        cx.emit(SessionReviewEvent::Changed);
                    }
                    Ok(_) => cx.emit(SessionReviewEvent::Changed),
                    // The workspace moved underneath; show what remains.
                    Err(_) => this.refresh(cx),
                }
                cx.notify();
            });
        }));
    }
}

/// The header's text buttons: h-7 px-2.5 at 11px, ink 50%.
fn text_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .text_size(px(11.))
        .text_color(content(0.5))
        .when(!enabled, |d| d.opacity(0.35))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        })
        .child(label)
}

/// DiffCounts: `+a -d` at 11px semibold, both shown even when zero.
fn counts(additions: i64, deletions: i64, gap: f32) -> gpui::Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(gap))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .font_features(tabular())
        .child(
            div()
                .text_color(rgb(DIFF_ADD))
                .child(format!("+{}", format_integer(additions))),
        )
        .child(
            div()
                .text_color(rgb(DIFF_DEL))
                .child(format!("-{}", format_integer(deletions))),
        )
}

fn card() -> gpui::Div {
    div()
        .w_full()
        .rounded(px(12.))
        .border_1()
        .border_color(content(0.06))
        .bg(content(0.03))
}

impl Render for SessionReview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.busy {
            return div().into_any_element();
        }
        if let Some(error) = self.error.clone() {
            return card()
                .id("session-review")
                .role(Role::Status)
                .px_3()
                .py(px(10.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(0.8))
                        .child("Couldn’t record changes"),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(error),
                )
                .into_any_element();
        }
        if self.files.is_empty() {
            return div().into_any_element();
        }
        let disabled = self.acting.is_some();
        let can_undo = !self.undo_locked && self.files.iter().all(|f| f.undoable);
        let visible = if self.expanded {
            self.files.len()
        } else {
            self.files.len().min(COLLAPSED)
        };
        let hidden = self.files.len() - visible;
        let additions = self.files.iter().map(|f| f.additions).sum();
        let deletions = self.files.iter().map(|f| f.deletions).sum();
        let count = self.files.len();
        let title: SharedString = format!(
            "Changed {count} {}",
            if count == 1 { "file" } else { "files" }
        )
        .into();
        let header = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .px_3()
            .py(px(10.))
            .child(
                div()
                    .size(px(32.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(content(0.08))
                    .child(icon("file-diff", 16.).text_color(content(0.55))),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.8))
                            .child(title),
                    )
                    .child(
                        counts(additions, deletions, 6.)
                            .line_height(px(16.5))
                            .mt(px(-2.)),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        text_button("session-review-undo", "Undo", !disabled && can_undo)
                            .aria_label(if can_undo {
                                "Undo the recorded changes"
                            } else if self.undo_locked {
                                "Undo is unavailable while another session is running in this project"
                            } else {
                                "Undo is unavailable because the workspace was shared, the branch moved, or a recorded file changed afterward"
                            })
                            .when(!disabled && can_undo, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.run(Action::Undo, None, cx)),
                                )
                            }),
                    )
                    .child(
                        text_button("session-review-keep", "Keep", !disabled)
                            .aria_label("Keep the recorded changes and dismiss this card")
                            .when(!disabled, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.run(Action::Keep, None, cx)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("session-review-open")
                            .role(Role::Button)
                            .aria_label("Review changes")
                            .h(px(28.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .bg(content(0.08))
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.75))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.12)).text_color(content(1.)))
                            .child("Review")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SessionReviewEvent::Open { path: None })
                            })),
                    ),
            );
        let rows = self.files[..visible].iter().enumerate().map(|(ix, file)| {
            let name = std::path::Path::new(&file.relative)
                .file_name()
                .map_or_else(
                    || file.relative.clone(),
                    |n| n.to_string_lossy().into_owned(),
                );
            let path = file.path.clone();
            let group: SharedString = format!("session-review-file-{ix}").into();
            let can_undo = !disabled && !self.undo_locked && file.undoable;
            let (undo_file, keep_file) = (file.relative.clone(), file.relative.clone());
            div()
                .id(("session-review-file", ix))
                .group(group.clone())
                .h(px(32.))
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .text_color(content(0.65))
                .hover(|s| s.bg(content(0.05)).text_color(content(1.)))
                .child(
                    div()
                        .id(("session-review-file-open", ix))
                        .role(Role::Button)
                        .aria_label(SharedString::from(file.relative.clone()))
                        .h_full()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_3()
                        .cursor_pointer()
                        .child(file_type_icon(&name, false, false, 15.))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .font_family(crate::fonts::monospace())
                                .text_size(px(12.))
                                .child(file.relative.clone()),
                        )
                        .child(if file.exact {
                            counts(file.additions, file.deletions, 8.).into_any_element()
                        } else {
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(content(0.45))
                                .child("Diff unavailable")
                                .into_any_element()
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SessionReviewEvent::Open {
                                path: Some(path.clone()),
                            })
                        })),
                )
                // One file's Undo and Keep, revealed while the row is hovered.
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .pl_1()
                        .pr(px(6.))
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .child(
                            text_button(("session-review-file-undo", ix), "Undo", can_undo)
                                .debug_selector(move || format!("session-review-file-undo-{ix}"))
                                .aria_label(SharedString::from(if can_undo {
                                    format!("Undo the recorded changes to {}", file.relative)
                                } else if self.undo_locked {
                                    "Undo is unavailable while another session is running in this project".into()
                                } else {
                                    format!(
                                        "Undo is unavailable because {} changed outside this session",
                                        file.relative
                                    )
                                }))
                                .when(can_undo, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.run(Action::Undo, Some(undo_file.clone()), cx)
                                    }))
                                }),
                        )
                        .child(
                            text_button(("session-review-file-keep", ix), "Keep", !disabled)
                                .debug_selector(move || format!("session-review-file-keep-{ix}"))
                                .aria_label(SharedString::from(format!(
                                    "Keep the recorded changes to {}",
                                    file.relative
                                )))
                                .when(!disabled, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.run(Action::Keep, Some(keep_file.clone()), cx)
                                    }))
                                }),
                        ),
                )
        });
        let list = div()
            .id("session-review-files")
            .border_t_1()
            .border_color(content(0.05))
            .py_1()
            .when(self.expanded, |d| d.max_h(px(256.)).overflow_y_scroll())
            .children(rows);
        let expanded = self.expanded;
        card()
            .id("session-review")
            .overflow_hidden()
            .child(header)
            .child(list)
            .when(count > COLLAPSED, |d| {
                d.child(
                    div()
                        .id("session-review-more")
                        .role(Role::Button)
                        .aria_expanded(expanded)
                        .group("session-review-more")
                        .h(px(32.))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .border_t_1()
                        .border_color(content(0.05))
                        .px_3()
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .hover(|s| s.bg(content(0.05)).text_color(content(0.7)))
                        .child(
                            icon(
                                if expanded {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                14.,
                            )
                            .text_color(content(0.45))
                            .group_hover("session-review-more", |s| s.text_color(content(0.7))),
                        )
                        .child(if expanded {
                            "Show fewer files".to_string()
                        } else {
                            format!(
                                "Show {hidden} more {}",
                                if hidden == 1 { "file" } else { "files" }
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.expanded = !this.expanded;
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::checkpoint::Store;
    use gpui::{Subscription, TestAppContext, VisualTestContext, WindowHandle};
    use std::{cell::Cell, path::Path, process::Command, rc::Rc};

    /// The app-wide checkpoint store, in a temporary folder for this run.
    fn store() -> &'static Store {
        if gespan_core::checkpoint::store().is_none() {
            let dir = tempfile::tempdir().unwrap().keep();
            gespan_core::checkpoint::init(dir.join("checkpoints"));
        }
        gespan_core::checkpoint::store().unwrap()
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn read(dir: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(name)).ok()
    }

    fn open(
        session: i64,
        cwd: &str,
        cx: &mut TestAppContext,
    ) -> (WindowHandle<SessionReview>, Rc<Cell<usize>>, Subscription) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut review = SessionReview::new(session, cwd.into());
                    review.refresh(cx);
                    review
                })
            })
            .unwrap()
        });
        let changed = Rc::new(Cell::new(0));
        let count = changed.clone();
        let events = window
            .update(cx, |_, _, cx| {
                cx.subscribe(&cx.entity(), move |_, _, event: &SessionReviewEvent, _| {
                    if matches!(event, SessionReviewEvent::Changed) {
                        count.set(count.get() + 1);
                    }
                })
            })
            .unwrap();
        (window, changed, events)
    }

    fn rows(window: &WindowHandle<SessionReview>, visual: &mut VisualTestContext) -> Vec<String> {
        visual.run_until_parked();
        window
            .read_with(visual, |review, _| {
                review.files.iter().map(|f| f.relative.clone()).collect()
            })
            .unwrap()
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        visual.refresh().unwrap();
        let bounds = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }

    #[gpui::test]
    fn undo_and_keep_one_file_from_its_row(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        for (key, value) in [
            ("user.name", "Test"),
            ("user.email", "test@example.invalid"),
            ("core.autocrlf", "false"),
            ("core.safecrlf", "false"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", key, value]);
        }
        std::fs::write(repo.join("a.txt"), "original\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "remove me\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "initial"]);

        let (session, cwd) = (910_001, repo.to_str().unwrap());
        let store = store();
        store.begin_turn(&session.to_string(), cwd, "turn").unwrap();
        std::fs::write(repo.join("a.txt"), "agent\n").unwrap();
        std::fs::write(repo.join("new.txt"), "created\n").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        store
            .finish_turn(&session.to_string(), cwd, "turn")
            .unwrap();

        let (window, changed, _events) = open(session, cwd, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert_eq!(rows(&window, visual), ["a.txt", "gone.txt", "new.txt"]);

        // Undo on the first row restores only that file.
        click(visual, "session-review-file-undo-0");
        assert_eq!(read(&repo, "a.txt").as_deref(), Some("original\n"));
        assert_eq!(read(&repo, "new.txt").as_deref(), Some("created\n"));
        assert_eq!(rows(&window, visual), ["gone.txt", "new.txt"]);
        assert_eq!(changed.get(), 1);

        // Keep on the created file leaves it on disk and drops its row.
        click(visual, "session-review-file-keep-1");
        assert_eq!(read(&repo, "new.txt").as_deref(), Some("created\n"));
        assert_eq!(rows(&window, visual), ["gone.txt"]);
        assert_eq!(changed.get(), 2);

        // Undoing the deletion brings the file back and empties the card.
        click(visual, "session-review-file-undo-0");
        assert_eq!(read(&repo, "gone.txt").as_deref(), Some("remove me\n"));
        assert!(rows(&window, visual).is_empty());
        window
            .read_with(visual, |review, _| assert!(!review.shown()))
            .unwrap();
        assert!(visual.debug_bounds("session-review-file-undo-0").is_none());
    }

    #[gpui::test]
    fn a_folder_without_git_undoes_and_keeps_files_but_not_ones_changed_since(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("notes");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), "one\n").unwrap();
        std::fs::write(folder.join("b.txt"), "two\n").unwrap();

        let (session, cwd) = (910_002.to_string(), folder.to_str().unwrap());
        let store = store();
        store.begin_turn(&session, cwd, "turn").unwrap();
        for (name, text) in [
            ("a.txt", "one!\n"),
            ("b.txt", "two!\n"),
            ("c.txt", "three\n"),
        ] {
            let paths = [folder.join(name).to_string_lossy().into_owned()];
            store.prepare(&session, cwd, &paths).unwrap();
            std::fs::write(folder.join(name), text).unwrap();
            store.capture_edit(&session, cwd, &paths).unwrap();
        }
        store.finish_turn(&session, cwd, "turn").unwrap();
        // Someone else edits b.txt afterwards, so it can no longer be undone.
        std::fs::write(folder.join("b.txt"), "someone else\n").unwrap();

        let (window, changed, _events) = open(910_002, cwd, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert_eq!(rows(&window, visual), ["a.txt", "b.txt", "c.txt"]);

        click(visual, "session-review-file-undo-1");
        assert_eq!(read(&folder, "b.txt").as_deref(), Some("someone else\n"));
        assert_eq!(rows(&window, visual), ["a.txt", "b.txt", "c.txt"]);
        assert_eq!(changed.get(), 0, "a disabled Undo does nothing");

        click(visual, "session-review-file-undo-0");
        assert_eq!(read(&folder, "a.txt").as_deref(), Some("one\n"));
        assert_eq!(rows(&window, visual), ["b.txt", "c.txt"]);

        click(visual, "session-review-file-keep-1");
        assert_eq!(read(&folder, "c.txt").as_deref(), Some("three\n"));
        assert_eq!(rows(&window, visual), ["b.txt"]);
        assert_eq!(changed.get(), 2);
    }
}
