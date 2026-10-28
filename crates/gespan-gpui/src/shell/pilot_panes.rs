//! Pilots share the session pane tree: a Pilot tab pulled onto a pane joins
//! its split, and a tab dropped on the visible Pilot splits beside it. A Pilot
//! pane renders the same chat as its tab, and becomes that tab again once it
//! stands alone.
use super::*;

/// The drop target standing for the visible Pilot, before it becomes a pane.
pub(super) const PILOT_VIEW: i64 = i64::MIN + 1;

impl Shell {
    pub(super) fn pilot_pane_id(&self, id: i64) -> Option<&str> {
        match self.workspace.pilot_panes.get(&id)? {
            TitleTab::Pilot { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The visible Pilot as a drop target: its whole view, by edge.
    pub(super) fn pilot_view_hit(&self, position: Point<Pixels>, cx: &App) -> Option<(i64, Edge)> {
        if !self.pilot_active(cx) || self.app_view_open() {
            return None;
        }
        let bounds = self.pilot_bounds.get()?;
        if !bounds.contains(&position) {
            return None;
        }
        let rect = Rect {
            x: 0.,
            y: 0.,
            w: bounds.size.width.as_f32(),
            h: bounds.size.height.as_f32(),
        };
        let x = (position.x - bounds.left()).as_f32();
        let y = (position.y - bounds.top()).as_f32();
        Some((PILOT_VIEW, crate::pane_layout::edge_from_point(x, y, rect)))
    }

    /// A drop on the visible Pilot first turns it into a lone pane in its own
    /// tab, so the drop can split beside it like any other pane.
    pub(super) fn resolve_pilot_target(&mut self, target: i64, cx: &mut Context<Self>) -> i64 {
        if target != PILOT_VIEW {
            return target;
        }
        let Some(pilot) = self
            .active_title_tab(cx)
            .filter(|tab| matches!(tab, TitleTab::Pilot { .. }))
        else {
            return target;
        };
        if let Some(pilots) = self.pilots.clone() {
            pilots.update(cx, |m, cx| m.close(cx));
        }
        self.workspace.pilot_tab_pane(pilot)
    }

    /// A Pilot tab released over a pane, or over another visible Pilot.
    pub(super) fn drop_pilot_tab(
        &mut self,
        pilot: TitleTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, edge)) = self.pane_drag.take().and_then(|d| d.over) else {
            return;
        };
        self.tabs_frame.update(cx, |_, cx| cx.notify());
        let target = self.resolve_pilot_target(target, cx);
        let before = self.active_leaves();
        let Some(pane) = self.workspace.place_pilot(pilot, target, edge) else {
            return;
        };
        self.begin_pane_motion(before, cx);
        self.fit_sidebar(cx);
        self.open_pilot_pane(pane, window, cx);
    }

    /// Focus a Pilot's pane, showing the tab that holds it.
    pub(super) fn open_pilot_pane(
        &mut self,
        pane: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.pilot_pane_id(pane).map(str::to_owned) else {
            return;
        };
        let Some(tab) = self.workspace.tab_of(pane) else {
            return;
        };
        if let Some(pilots) = self.pilots.clone() {
            pilots.update(cx, |m, cx| {
                m.close(cx);
                m.pane_chat(&id, cx);
            });
        }
        self.navigation_revision += 1;
        self.leave_app_views(cx);
        self.leave_app_search(cx);
        self.close_preview(cx);
        self.history.record(pane);
        self.workspace.open(pane);
        self.review_focused = false;
        self.remember_title_tab(TitleTab::Session(tab));
        self.reveal_active_tab(cx);
        self.focus_active(window, cx);
        self.persist(cx);
        cx.notify();
    }

    /// The focused Pilot pane's composer.
    pub(super) fn focus_pilot_pane(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(id) = self
            .workspace
            .active
            .and_then(|pane| self.pilot_pane_id(pane))
        else {
            return false;
        };
        match self.pilots.as_ref().and_then(|m| m.read(cx).open_chat(id)) {
            Some(chat) => window.focus(&chat.read(cx).input.focus_handle(cx), cx),
            None => window.focus(&self.focus, cx),
        }
        true
    }

    /// Opening a Pilot that lives in a split focuses its pane instead of
    /// showing the same chat a second time.
    pub(super) fn redirect_pilot_to_pane(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(pane) = self.workspace.pilot_pane(id) else {
            return false;
        };
        if let Some(pilots) = self.pilots.clone() {
            pilots.update(cx, |m, cx| m.close(cx));
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |this, window, cx| {
                this.open_pilot_pane(pane, window, cx)
            });
        })
        .detach();
        true
    }

    pub(super) fn pilot_panes_unsettled(&self, cx: &App) -> bool {
        self.workspace.pilot_panes.keys().any(|pane| {
            self.workspace
                .tab_of(*pane)
                .is_none_or(|tab| !self.workspace.layout(tab).is_split())
                || self.pilot_pane_id(*pane).is_none_or(|id| {
                    self.pilots
                        .as_ref()
                        .is_none_or(|m| m.read(cx).roster.get(id).is_none())
                })
        })
    }

    /// Panes whose Pilot was deleted close; a Pilot left alone in its tab
    /// becomes its Pilot tab again, shown when it had focus.
    pub(super) fn settle_pilot_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.pilot_panes.is_empty() {
            return;
        }
        let gone: Vec<i64> = self
            .workspace
            .pilot_panes
            .keys()
            .copied()
            .filter(|pane| {
                self.pilot_pane_id(*pane).is_none_or(|id| {
                    self.pilots
                        .as_ref()
                        .is_none_or(|m| m.read(cx).roster.get(id).is_none())
                })
            })
            .collect();
        for pane in gone {
            self.workspace.close_pane(pane);
            self.workspace.pilot_panes.remove(&pane);
        }
        let Some(pilot) = self.workspace.settle_pilot_panes() else {
            return;
        };
        self.pane_motion = None;
        self.open_title_tab(&pilot, window, cx);
        self.persist(cx);
    }

    /// A Pilot pane: its split header over the Pilot's chat.
    pub(super) fn pilot_pane_surface(
        &mut self,
        pane: i64,
        split: Option<f32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chat = self
            .pilot_pane_id(pane)
            .and_then(|id| self.pilots.as_ref()?.read(cx).open_chat(id));
        if let Some(chat) = &chat {
            chat.update(cx, |chat, cx| chat.set_beside_pane(split.is_some(), cx));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .when_some(split.filter(|p| *p > 0.), |d, p| {
                d.child(self.split_header(pane, p, true, cx))
            })
            .child(div().flex_1().min_h_0().flex().flex_col().children(chat))
            .into_any_element()
    }

    pub(super) fn pilot_pane_title(&self, pane: i64, cx: &App) -> Option<String> {
        let id = self.pilot_pane_id(pane)?;
        Some(self.pilots.as_ref()?.read(cx).roster.get(id)?.label())
    }

    pub(super) fn pilot_pane_mark(&self, pane: i64, cx: &App) -> Option<PaneMark> {
        let id = self.pilot_pane_id(pane)?;
        let pilot = self.pilots.as_ref()?.read(cx).roster.get(id)?.clone();
        Some(PaneMark::Pilot(pilot.mascot, pilot.color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};

    fn pull(visual: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(
            point(from.x, px(TITLE_BAR_HEIGHT + PANE_DROP_PULL + 10.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.run_until_parked();
    }

    #[gpui::test]
    fn a_pilot_tab_pulled_onto_a_pane_splits_and_returns_to_its_tab(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, first) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.create(Vec::new(), w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        let pilot = window
            .update(cx, |shell, w, cx| {
                let pilot = shell.active_title_tab(cx).unwrap();
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.dismiss_settings(w, cx));
                shell.open(first, w, cx);
                pilot
            })
            .unwrap();
        let TitleTab::Pilot { id, .. } = pilot.clone() else {
            panic!("expected a Pilot");
        };
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let tab = visual
            .debug_bounds(Box::leak(format!("pilot-title-tab-{id}").into_boxed_str()))
            .unwrap()
            .center();
        let pane = window
            .update(visual, |shell, _, _| shell.pane_bounds.get().unwrap())
            .unwrap();
        pull(visual, tab, point(pane.right() - px(20.), pane.center().y));
        let pilot_pane = window
            .update(visual, |shell, w, cx| {
                let pane = shell
                    .workspace
                    .pilot_pane(&id)
                    .expect("the Pilot joined the split");
                assert_eq!(shell.workspace.layout(first).leaf_ids(), [first, pane]);
                assert_eq!(shell.workspace.active, Some(pane));
                assert!(!shell.pilot_active(cx));
                assert_eq!(shell.visible_tabs(cx), [TitleTab::Session(first)]);
                assert_eq!(shell.tab_copy(first, cx).panes.len(), 2);
                let chat = shell.pilot_manager().read(cx).open_chat(&id).unwrap();
                assert!(chat.read(cx).input.focus_handle(cx).is_focused(w));
                pane
            })
            .unwrap();
        assert!(
            visual
                .debug_bounds(Box::leak(
                    format!("split-header-{pilot_pane}").into_boxed_str()
                ))
                .is_some()
        );
        // Opening the Pilot from elsewhere focuses its pane instead.
        window
            .update(visual, |shell, w, cx| {
                shell.open(first, w, cx);
                shell.pilot_manager().update(cx, |m, cx| m.open(&id, w, cx));
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert!(!shell.pilot_active(cx));
                assert_eq!(shell.workspace.active, Some(pilot_pane));
            })
            .unwrap();
        // Closing the conversation's pane leaves the Pilot as its own tab.
        window
            .update(visual, |shell, w, cx| shell.close_pane(first, w, cx))
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert!(shell.workspace.pilot_panes.is_empty());
                assert_eq!(shell.active_title_tab(cx), Some(pilot.clone()));
                assert!(shell.visible_tabs(cx).contains(&pilot));
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_tab_pulled_onto_the_visible_pilot_splits_beside_it(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, first) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.create(Vec::new(), w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        let pilot = window
            .update(cx, |shell, w, cx| {
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.dismiss_settings(w, cx));
                shell.active_title_tab(cx).unwrap()
            })
            .unwrap();
        let TitleTab::Pilot { id, .. } = pilot.clone() else {
            panic!("expected a Pilot");
        };
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let tab = visual
            .debug_bounds(Box::leak(format!("title-tab-{first}").into_boxed_str()))
            .unwrap()
            .center();
        let view = window
            .update(visual, |shell, _, _| shell.pilot_bounds.get().unwrap())
            .unwrap();
        pull(visual, tab, point(view.left() + px(20.), view.center().y));
        window
            .update(visual, |shell, _, cx| {
                let pane = shell
                    .workspace
                    .pilot_pane(&id)
                    .expect("the Pilot became a pane");
                let key = shell.workspace.tab_of(first).unwrap();
                assert_eq!(shell.workspace.layout(key).leaf_ids(), [first, pane]);
                assert_eq!(shell.workspace.active, Some(first));
                assert!(!shell.pilot_active(cx));
                assert_eq!(shell.visible_tabs(cx), [TitleTab::Session(key)]);
            })
            .unwrap();
        // Its header pulls it back onto the strip as the Pilot tab.
        let pane = window
            .update(visual, |shell, _, _| {
                shell.workspace.pilot_pane(&id).unwrap()
            })
            .unwrap();
        let grip = visual
            .debug_bounds(Box::leak(
                format!("drag-session-pane-{pane}").into_boxed_str(),
            ))
            .unwrap()
            .center();
        let key = window
            .update(visual, |shell, _, _| shell.workspace.tab_of(first).unwrap())
            .unwrap();
        let strip = visual
            .debug_bounds(Box::leak(format!("title-tab-{key}").into_boxed_str()))
            .unwrap();
        let after = point(strip.right() - px(4.), strip.center().y);
        visual.simulate_mouse_down(grip, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(
            grip + point(px(0.), px(-20.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.simulate_mouse_move(after, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(after, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert!(shell.workspace.pilot_panes.is_empty());
                let key = shell.workspace.tab_of(first).unwrap();
                assert!(!shell.workspace.layout(key).is_split());
                assert_eq!(
                    shell.visible_tabs(cx),
                    [TitleTab::Session(key), pilot.clone()]
                );
                assert_eq!(shell.active_title_tab(cx), Some(pilot.clone()));
            })
            .unwrap();
    }
}
