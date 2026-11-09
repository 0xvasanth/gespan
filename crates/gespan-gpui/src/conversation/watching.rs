//! The composer's watch chip: everything that will wake this chat without the
//! user, so a quiet chat never hides a pending wake. Background work comes from
//! the provider; delegated sessions come from the Pilot that owns this chat.
use super::*;
use gespan_core::watching::{self, Source, Watch};

/// The user stopped waiting on a delegated session's report.
pub(crate) struct StopWatching(pub i64);
impl EventEmitter<StopWatching> for Conversation {}

#[derive(Default)]
pub(super) struct Watching {
    delegated: Vec<Watch>,
    pub(super) open: bool,
    anchor: AnchorSlot,
}

const LIVE: u32 = 0x4ade80;

impl Conversation {
    /// The delegated sessions this chat waits to hear back from.
    pub(crate) fn set_delegated_watches(&mut self, watches: Vec<Watch>, cx: &mut Context<Self>) {
        if self.watching.delegated == watches {
            return;
        }
        self.watching.delegated = watches;
        self.close_empty_watch_list();
        cx.notify();
    }

    /// An open list with nothing left in it would reopen on the next wake.
    pub(super) fn close_empty_watch_list(&mut self) {
        if self.watches().is_empty() {
            self.watching.open = false;
        }
    }

    /// Background work first, in start order, then delegated sessions.
    pub(crate) fn watches(&self) -> Vec<Watch> {
        let mut all = watching::background(&self.background.tasks);
        all.extend(self.watching.delegated.iter().cloned());
        all
    }

    pub(super) fn watch_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let watches = self.watches();
        let label = watching::chip_label(watches.len())?;
        let weak = cx.entity().downgrade();
        Some(
            div()
                .id("watch-chip")
                .debug_selector(|| "watch-chip".into())
                .role(Role::Button)
                .aria_label(format!("{label}: what will wake this chat"))
                .child(self.watching.anchor.probe())
                .cursor_pointer()
                .h(px(20.))
                .px(px(7.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.))
                .text_color(content(0.6))
                .when(self.watching.open, |d| d.bg(content(0.08)))
                .hover(|s| s.bg(content(0.06)))
                .child(div().size(px(6.)).rounded_full().bg(gpui::rgb(LIVE)))
                .child(label)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.watching.open = !this.watching.open;
                    cx.notify();
                }))
                .when(self.watching.open, |d| {
                    d.child(
                        popover("watch-list", Anchor::Slot(self.watching.anchor.clone()))
                            .side(Side::Top)
                            .align(Align::End)
                            .width(320.)
                            .max_height(320.)
                            .padding(6.)
                            .on_dismiss(move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.watching.open = false;
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .id("watch-list-rows")
                                    .flex()
                                    .flex_col()
                                    .overflow_y_scroll()
                                    .child(
                                        div()
                                            .px(px(8.))
                                            .pt(px(4.))
                                            .pb(px(6.))
                                            .text_size(px(11.))
                                            .text_color(content(0.45))
                                            .child("Wakes this chat"),
                                    )
                                    .children(watches.iter().map(|w| self.watch_row(w, cx))),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    fn watch_row(&self, watch: &Watch, cx: &mut Context<Self>) -> AnyElement {
        let tool_id = self
            .background
            .tasks
            .iter()
            .find(|t| t.id == watch.id)
            .and_then(|t| t.tool_id.clone());
        let session = match watch.source {
            Source::Session(id) => Some(id),
            _ => None,
        };
        let age = watch
            .started_at
            .map(|at| background_tasks::elapsed(gespan_core::time::now_ms().saturating_sub(at)));
        div()
            .id(SharedString::from(format!("watch-{}", watch.id)))
            .debug_selector({
                let id = watch.id.clone();
                move || format!("watch-row-{id}")
            })
            .role(Role::Group)
            .aria_label(format!("{}: {}", watch.label, watch.detail))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when_some(tool_id, |d, tool_id| {
                d.cursor_pointer()
                    .hover(|s| s.bg(content(0.04)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.watching.open = false;
                        this.reveal_tool(&tool_id, cx);
                    }))
            })
            .child(
                div()
                    .size(px(24.))
                    .flex_shrink_0()
                    .rounded(px(7.))
                    .bg(content(0.07))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(
                        match watch.source {
                            Source::Command => "terminal",
                            Source::Subagent => "bot",
                            Source::Session(_) => "message-square",
                        },
                        13.,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(content(0.9))
                            .child(watch.label.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .child(watch.detail.clone()),
                    ),
            )
            .when_some(age, |d, age| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(age),
                )
            })
            .when_some(session, |d, session| {
                d.child(
                    div()
                        .id(SharedString::from(format!("stop-watching-{session}")))
                        .debug_selector(move || format!("stop-watching-{session}"))
                        .role(Role::Button)
                        .aria_label("Stop watching this session")
                        .flex_shrink_0()
                        .cursor_pointer()
                        .px(px(6.))
                        .h(px(20.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .hover(|s| s.bg(content(0.06)).text_color(content(0.9)))
                        .child("Stop watching")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.watching
                                .delegated
                                .retain(|w| w.source != Source::Session(session));
                            this.close_empty_watch_list();
                            cx.emit(StopWatching(session));
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
    use gespan_core::provider::BackgroundTask;
    use gespan_core::store::Store;
    use gpui::{TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc, sync::Mutex};

    fn command(id: &str) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            description: "Watch CI".into(),
            agent: false,
            command: Some("gh run watch 42".into()),
            started_at: gespan_core::time::now_ms(),
            tool_id: None,
            latest: None,
            steps: 0,
        }
    }

    fn session(id: i64) -> Watch {
        Watch {
            id: format!("session:{id}"),
            source: Source::Session(id),
            label: "Monitor v0.0.9 Release Pipeline".into(),
            detail: "Reports back when it stops".into(),
            started_at: None,
        }
    }

    #[gpui::test]
    fn the_chip_lists_every_pending_wake_and_stops_watching_a_session(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("kiro", "/tmp").unwrap();
        let window = cx.update(|cx| {
            input::bind(cx);
            cx.set_reduce_motion(true);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    // Background work only outlives an answer while the turn runs.
                    view.busy = true;
                    view
                })
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        assert!(
            visual.debug_bounds("watch-chip").is_none(),
            "a chat with nothing pending shows no chip"
        );

        let stopped = Rc::new(RefCell::new(Vec::new()));
        window
            .update(visual, |view, _, cx| {
                let stopped = stopped.clone();
                cx.subscribe(&cx.entity(), move |_, _, e: &StopWatching, _| {
                    stopped.borrow_mut().push(e.0)
                })
                .detach();
                view.set_background(vec![command("ci")], cx);
                view.set_delegated_watches(vec![session(9)], cx);
                assert_eq!(view.watches().len(), 2);
            })
            .unwrap();
        visual.refresh().unwrap();
        let chip = visual.debug_bounds("watch-chip").expect("chip shows");
        assert!(
            visual.debug_bounds("watch-row-ci").is_none(),
            "closed until clicked"
        );

        visual.simulate_click(chip.center(), Default::default());
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("watch-row-ci").is_some());
        assert!(visual.debug_bounds("watch-row-session:9").is_some());
        assert!(
            visual.debug_bounds("stop-watching-ci").is_none(),
            "only delegated sessions can be stopped from the list"
        );

        let stop = visual.debug_bounds("stop-watching-9").unwrap();
        visual.simulate_click(stop.center(), Default::default());
        visual.refresh().unwrap();
        assert_eq!(*stopped.borrow(), [9]);
        assert!(visual.debug_bounds("watch-row-session:9").is_none());
        assert!(
            visual.debug_bounds("watch-row-ci").is_some(),
            "the list stays open on the remaining wake"
        );

        // The Pilot's next refresh agrees, and the background work ends.
        window
            .update(visual, |view, _, cx| {
                view.set_delegated_watches(Vec::new(), cx);
                view.stop(cx);
                assert!(view.watches().is_empty());
                assert!(!view.watching.open, "an empty list closes");
                // New work later shows the chip, not a list left open.
                view.set_background(vec![command("next")], cx);
                assert!(!view.watching.open);
                view.set_background(Vec::new(), cx);
            })
            .unwrap();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("watch-chip").is_none());
    }

    #[gpui::test]
    fn an_unchanged_refresh_keeps_the_list_open(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("kiro", "/tmp").unwrap();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Conversation::new(summary, Arc::new(Mutex::new(store)), cx))
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |view, _, cx| {
                view.set_delegated_watches(vec![session(1), session(2)], cx);
                view.watching.open = true;
                // The Pilot refreshes cards every tick with the same list.
                view.set_delegated_watches(vec![session(1), session(2)], cx);
                assert!(view.watching.open);
                view.set_delegated_watches(vec![session(2)], cx);
                assert!(view.watching.open, "one wake is still pending");
                view.set_delegated_watches(Vec::new(), cx);
                assert!(!view.watching.open);
            })
            .unwrap();
    }
}
