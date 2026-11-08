//! Actions an agent runs, each shown in a read-only tab of its project's
//! dock. Core runs them and keeps their output; a project with no window
//! open just isn't shown.
use super::Shell;
use crate::terminal::{Terminal, TerminalEvent};
use gespan_core::project_actions::runs::{self, Mirror, RunEvent, Status};
use gpui::{App, AppContext as _, Context, Entity, WeakEntity, WindowHandle};
use std::{collections::HashMap, sync::Arc};

struct Bridge(async_channel::Sender<RunEvent>);
impl Mirror for Bridge {
    fn event(&self, event: RunEvent) {
        let _ = self.0.try_send(event);
    }
}

/// Starts showing agent runs in the docks. Call once.
pub(crate) fn install(cx: &mut App) {
    let (sender, events) = async_channel::unbounded();
    runs::set_mirror(Arc::new(Bridge(sender)));
    cx.spawn(async move |cx| {
        let mut tabs = Tabs::default();
        while let Ok(event) = events.recv().await {
            let batch = burst(event, &events);
            cx.update(|cx| {
                for event in batch {
                    tabs.apply(event, cx);
                }
            });
        }
    })
    .detach();
}

/// The most output one update shows, so a chatty run can't make one huge
/// frame; the rest waits for the next update.
const BURST_BYTES: usize = 256 * 1024;

/// `first` and whatever else is waiting, up to [`BURST_BYTES`] of output.
fn burst(first: RunEvent, events: &async_channel::Receiver<RunEvent>) -> Vec<RunEvent> {
    let size = |event: &RunEvent| match event {
        RunEvent::Output { bytes, .. } => bytes.len(),
        _ => 0,
    };
    let mut bytes = size(&first);
    let mut batch = vec![first];
    while bytes < BURST_BYTES
        && let Ok(more) = events.try_recv()
    {
        bytes += size(&more);
        batch.push(more);
    }
    batch
}

/// Each run's tab, while the run lasts.
#[derive(Default)]
struct Tabs(HashMap<String, WeakEntity<Terminal>>);
impl Tabs {
    fn apply(&mut self, event: RunEvent, cx: &mut App) {
        match event {
            RunEvent::Started {
                run_id,
                project,
                checkout,
                label,
                ..
            } => {
                self.0.retain(|_, tab| tab.upgrade().is_some());
                let Some(window) = window_for(&project, cx) else {
                    return;
                };
                let title = format!("{label} (agent)");
                let cwd = checkout.to_string_lossy().into_owned();
                let opened = window.update(cx, |shell, _, cx| {
                    shell.open_mirror(&project, title, cwd, cx)
                });
                if let Ok(Some(tab)) = opened {
                    self.0.insert(run_id, tab.downgrade());
                }
            }
            RunEvent::Output { run_id, bytes } => {
                if let Some(tab) = self.0.get(&run_id).and_then(WeakEntity::upgrade) {
                    tab.update(cx, |tab, cx| tab.feed(&bytes, cx));
                }
            }
            RunEvent::Finished {
                run_id,
                status,
                exit_code,
            } => {
                if let Some(tab) = self.0.remove(&run_id).and_then(|tab| tab.upgrade()) {
                    let label = status_label(status, exit_code);
                    tab.update(cx, |tab, cx| tab.finish(label, cx));
                }
            }
        }
    }
}

/// "exited 0", "exited 3" or "stopped".
fn status_label(status: Status, exit_code: Option<i32>) -> String {
    match (status, exit_code) {
        (Status::Stopped, _) => "stopped".into(),
        (_, Some(code)) => format!("exited {code}"),
        (_, None) => "exited".into(),
    }
}

/// The window showing `project`. Every window's rail lists every project,
/// so one only listing it doesn't count.
fn window_for(project: &str, cx: &App) -> Option<WindowHandle<Shell>> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<Shell>())
        .find(|window| {
            window
                .read(cx)
                .is_ok_and(|shell| shell.workspace.project == project)
        })
}

impl Shell {
    /// A read-only tab named `title` at the end of `project`'s dock. The
    /// dock stays shut if it was, and its active tab stays active.
    fn open_mirror(
        &mut self,
        project: &str,
        title: String,
        cwd: String,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Terminal>> {
        if project.starts_with("remote://") {
            return None;
        }
        // A restored dock's shells come back first, so tabs and shells line up.
        self.ensure_terminals(project, cx);
        let dock = self.workspace.docks.entry(project.to_owned()).or_default();
        dock.add_mirror(cwd.clone());
        let terminal = cx.new(|cx| Terminal::mirror(title, cwd, cx));
        self.observations
            .push(cx.subscribe(&terminal, |_, _, _: &TerminalEvent, cx| cx.notify()));
        self.terminals
            .entry(project.to_owned())
            .or_default()
            .push(terminal.clone());
        cx.notify();
        Some(terminal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use gpui::TestAppContext;

    const PROJECT: &str = "/work/gespan";
    const CHECKOUT: &str = "/work/gespan-agent";

    fn started(run: &str, project: &str, label: &str) -> RunEvent {
        RunEvent::Started {
            run_id: run.into(),
            project: project.into(),
            checkout: CHECKOUT.into(),
            label: label.into(),
            command: "npm test".into(),
        }
    }
    fn output(run: &str, text: &str) -> RunEvent {
        RunEvent::Output {
            run_id: run.into(),
            bytes: text.as_bytes().to_vec(),
        }
    }
    fn finished(run: &str, status: Status, exit_code: Option<i32>) -> RunEvent {
        RunEvent::Finished {
            run_id: run.into(),
            status,
            exit_code,
        }
    }
    fn tabs(shell: &Shell, cx: &App) -> Vec<(String, bool)> {
        shell
            .terminals
            .get(PROJECT)
            .into_iter()
            .flatten()
            .map(|t| (t.read(cx).title.clone(), t.read(cx).read_only()))
            .collect()
    }

    #[gpui::test]
    fn an_agent_run_opens_a_read_only_mirror_tab(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut runs = Tabs::default();
        cx.update(|cx| {
            runs.apply(started("r1", PROJECT, "test"), cx);
            runs.apply(output("r1", "PASS src/app.test.ts\n"), cx);
            // No window shows this project; its run is not shown.
            runs.apply(started("r2", "/work/elsewhere", "build"), cx);
            runs.apply(output("r2", "built\n"), cx);
        });
        cx.run_until_parked();
        assert_eq!(runs.0.len(), 1);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.workspace.project, PROJECT);
                assert_eq!(tabs(shell, cx), [("test (agent)".to_string(), true)]);
                let tab = shell.terminals[PROJECT][0].read(cx);
                assert_eq!(tab.cwd, CHECKOUT);
                assert_eq!(
                    tab.visible_text().lines().next(),
                    Some("PASS src/app.test.ts")
                );
                let dock = &shell.workspace.docks[PROJECT];
                assert!(dock.is_mirror(0));
                // The dock was shut, and stays shut.
                assert!(!dock.open);
                assert!(!shell.terminals.contains_key("/work/elsewhere"));
                assert!(!shell.workspace.docks.contains_key("/work/elsewhere"));
            })
            .unwrap();
        // With the dock open on the user's shell, the mirror joins at the
        // end without taking over.
        window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.new_terminal_in(Some("/work/gespan/web".into()), window, cx);
            })
            .unwrap();
        cx.update(|cx| runs.apply(started("r3", PROJECT, "lint"), cx));
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    tabs(shell, cx),
                    [
                        ("test (agent)".to_string(), true),
                        ("web".to_string(), false),
                        ("lint (agent)".to_string(), true),
                    ]
                );
                let dock = &shell.workspace.docks[PROJECT];
                assert!(dock.open);
                assert_eq!(dock.active, 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_run_for_a_project_no_window_is_showing_opens_no_tab(cx: &mut TestAppContext) {
        // The window lists this project in its rail but shows another.
        const BACKGROUND: &str = "/work/another-project";
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, _| {
                assert!(shell.workspace.projects.iter().any(|p| p == BACKGROUND));
                assert_ne!(shell.workspace.project, BACKGROUND);
            })
            .unwrap();
        let mut runs = Tabs::default();
        cx.update(|cx| {
            runs.apply(started("r1", BACKGROUND, "test"), cx);
            runs.apply(output("r1", "PASS\n"), cx);
            runs.apply(finished("r1", Status::Exited, Some(0)), cx);
        });
        assert!(runs.0.is_empty());
        window
            .update(cx, |shell, _, _| {
                // Neither a tab nor its dock's restored shells appear.
                assert!(shell.terminals.values().all(Vec::is_empty));
                assert!(!shell.workspace.docks.contains_key(BACKGROUND));
            })
            .unwrap();
    }

    #[test]
    fn a_burst_shows_at_most_a_bounded_amount_of_output() {
        let (sender, events) = async_channel::unbounded();
        let chunk = "x".repeat(100 * 1024);
        for _ in 0..4 {
            sender.try_send(output("r1", &chunk)).unwrap();
        }
        sender
            .try_send(finished("r1", Status::Exited, Some(0)))
            .unwrap();
        // 100 KB + 100 KB + 100 KB reaches the cap; the rest waits.
        let first = burst(started("r1", PROJECT, "test"), &events);
        assert_eq!(first.len(), 4);
        let rest = burst(events.try_recv().unwrap(), &events);
        assert_eq!(rest.len(), 2);
        assert!(matches!(rest[1], RunEvent::Finished { .. }));
        // A single event larger than the cap still goes through on its own.
        sender
            .try_send(output("r2", &"y".repeat(BURST_BYTES)))
            .unwrap();
        sender.try_send(output("r2", "z")).unwrap();
        assert_eq!(burst(events.try_recv().unwrap(), &events).len(), 1);
    }

    #[gpui::test]
    fn the_panel_terminal_is_a_shell_even_when_the_dock_holds_only_a_mirror(
        cx: &mut TestAppContext,
    ) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut runs = Tabs::default();
        cx.update(|cx| runs.apply(started("r1", PROJECT, "test"), cx));
        window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.workspace.classic_layout = false;
                shell.workspace.new_layout_boxy = true;
                shell.layout_changed(cx);
                shell.workspace.preferences.terminal_in_panel = true;
                assert!(shell.terminal_in_panel(cx));
                shell.toggle_terminal(window, cx);
                assert_eq!(
                    tabs(shell, cx),
                    [
                        ("test (agent)".to_string(), true),
                        ("gespan".to_string(), false),
                    ]
                );
                let dock = &shell.workspace.docks[PROJECT];
                assert_eq!(dock.active, 1);
                assert!(!dock.open);
                // With a shell there already, no second one starts.
                shell.toggle_terminal(window, cx);
                shell.toggle_terminal(window, cx);
                assert_eq!(tabs(shell, cx).len(), 2);
            })
            .unwrap();
    }

    #[gpui::test]
    fn mirror_tabs_are_not_saved_with_the_dock(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut runs = Tabs::default();
        window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.new_terminal_in(Some("/work/gespan/a".into()), window, cx);
            })
            .unwrap();
        cx.update(|cx| runs.apply(started("r1", PROJECT, "test"), cx));
        window
            .update(cx, |shell, window, cx| {
                shell.new_terminal_in(Some("/work/gespan/b".into()), window, cx);
                let saved = |shell: &Shell| {
                    let json = serde_json::to_string(&shell.workspace).unwrap();
                    assert!(!json.contains(CHECKOUT), "{json}");
                    Workspace::restore(Some(&json), &[], PROJECT).docks[PROJECT].clone()
                };
                let dock = saved(shell);
                assert_eq!(dock.terminals, ["/work/gespan/a", "/work/gespan/b"]);
                assert_eq!(dock.active, 1);
                // Closing a shell before the mirror keeps the mirror unsaved.
                shell.close_terminal(0, window, cx);
                assert_eq!(tabs(shell, cx)[0], ("test (agent)".to_string(), true));
                assert!(shell.workspace.docks[PROJECT].is_mirror(0));
                let dock = saved(shell);
                assert_eq!(dock.terminals, ["/work/gespan/b"]);
                assert_eq!(dock.active, 0);
                // Closing the mirror leaves the shell as it was.
                shell.close_terminal(0, window, cx);
                assert_eq!(tabs(shell, cx), [("b".to_string(), false)]);
                assert!(!shell.workspace.docks[PROJECT].is_mirror(0));
                assert_eq!(saved(shell).terminals, ["/work/gespan/b"]);
            })
            .unwrap();
        // The closed tab's run still finishing is harmless.
        cx.update(|cx| {
            runs.apply(output("r1", "late\n"), cx);
            runs.apply(finished("r1", Status::Exited, Some(0)), cx);
        });
        assert!(runs.0.is_empty());
    }

    #[gpui::test]
    fn a_finished_run_marks_its_tab(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut runs = Tabs::default();
        cx.update(|cx| {
            runs.apply(started("pass", PROJECT, "test"), cx);
            runs.apply(started("fail", PROJECT, "lint"), cx);
            runs.apply(started("stop", PROJECT, "dev"), cx);
            runs.apply(output("pass", "ok\n"), cx);
            runs.apply(finished("pass", Status::Exited, Some(0)), cx);
            runs.apply(finished("fail", Status::Exited, Some(3)), cx);
            runs.apply(finished("stop", Status::Stopped, None), cx);
        });
        assert!(runs.0.is_empty());
        window
            .update(cx, |shell, _, cx| {
                let marks: Vec<_> = shell.terminals[PROJECT]
                    .iter()
                    .map(|t| {
                        let t = t.read(cx);
                        (t.title.clone(), t.exited, t.status().map(str::to_owned))
                    })
                    .collect();
                assert_eq!(
                    marks,
                    [
                        ("test (agent)".into(), true, Some("exited 0".into())),
                        ("lint (agent)".into(), true, Some("exited 3".into())),
                        ("dev (agent)".into(), true, Some("stopped".into())),
                    ]
                );
                let text = shell.terminals[PROJECT][0].read(cx).visible_text();
                assert!(text.starts_with("ok\n[exited 0]"), "{text}");
            })
            .unwrap();
    }
}
