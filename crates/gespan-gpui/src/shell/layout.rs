// One sidebar beside the workspace, framed either as an inset rounded card or
// as the full-height boxy frame.
use super::{CORNER_CONTROL_GAP, Shell, titlebar_drag_area};
use crate::{
    popover::{Anchor, popover},
    resource_pressure::SessionInfo,
    right_panel::RightPanel,
    theme::*,
    workspace::TitleTab,
};
use gpui::{
    AnyElement, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, Role, StyleRefinement,
    Window, div, prelude::*, px,
};
use std::collections::HashMap;

pub(super) const CARD_RADIUS: f32 = 10.;

// GPUI overflow clips rectangles. Capture outside the bordered frame so the
// final mask includes its background, border and children at the outer radius.
// A mask inside the frame only covers its inset content rectangle.
fn card_corner_mask(begin: bool) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let r = px(CARD_RADIUS)
                .min(bounds.size.width / 2.)
                .min(bounds.size.height / 2.);
            #[cfg(target_os = "macos")]
            let corners = [
                bounds.origin,
                gpui::point(bounds.right() - r, bounds.top()),
                gpui::point(bounds.right() - r, bounds.bottom() - r),
                gpui::point(bounds.left(), bounds.bottom() - r),
            ];
            #[cfg(target_os = "macos")]
            for (corner, origin) in corners.into_iter().enumerate() {
                window.paint_quad(
                    gpui::fill(gpui::Bounds::new(origin, gpui::size(r, r)), gpui::black())
                        .border_color(gpui_apple::metal_renderer::corner_clip_marker(
                            begin,
                            corner as u8,
                        )),
                );
            }
            if !begin {
                // The areas outside the curve belong to the surrounding
                // chrome, just like the gutters and footer. Restoring the
                // window's unwashed glass here leaves dark square corners.
                #[cfg(target_os = "macos")]
                let outside = inset_chrome_wash();
                // Without the mask an opaque fill covers whatever the card
                // painted past its curve. Over Windows' glass the root's
                // translucent fill is already there, and a second one darkens
                // the corners.
                #[cfg(target_os = "windows")]
                let outside = if shell_fill().a < 1. {
                    inset_chrome_wash()
                } else {
                    shell_fill().blend(inset_chrome_wash())
                };
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let outside = shell_fill().blend(inset_chrome_wash());
                let vertices = [
                    (
                        bounds.origin,
                        gpui::point(bounds.left() + r, bounds.top()),
                        gpui::point(bounds.left(), bounds.top() + r),
                    ),
                    (
                        gpui::point(bounds.right(), bounds.top()),
                        gpui::point(bounds.right(), bounds.top() + r),
                        gpui::point(bounds.right() - r, bounds.top()),
                    ),
                    (
                        gpui::point(bounds.right(), bounds.bottom()),
                        gpui::point(bounds.right() - r, bounds.bottom()),
                        gpui::point(bounds.right(), bounds.bottom() - r),
                    ),
                    (
                        gpui::point(bounds.left(), bounds.bottom()),
                        gpui::point(bounds.left(), bounds.bottom() - r),
                        gpui::point(bounds.left() + r, bounds.bottom()),
                    ),
                ];
                for (corner, start, end) in vertices {
                    let mut path = gpui::PathBuilder::fill();
                    path.move_to(corner);
                    path.line_to(start);
                    path.arc_to(gpui::point(r, r), px(0.), false, false, end);
                    path.close();
                    if let Ok(path) = path.build() {
                        window.paint_path(path, outside);
                    }
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The right panel toggle: chrome_button's 24px square.
fn panel_button(enabled: bool, dot: bool) -> gpui::Stateful<gpui::Div> {
    let id = "toggle-right-panel";
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(super::TOGGLE_RIGHT_PANEL_LABEL)
        .group(id)
        .cursor_pointer()
        .size(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(enabled, |d| d.hover(|s| s.bg(content(0.1))))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .relative()
                .size(px(14.))
                .child(
                    icon("sidebar-right", 14.)
                        .text_color(content(if enabled { 0.5 } else { 0.25 }))
                        .when(enabled, |i| {
                            i.group_hover(id, |s| s.text_color(content(1.)))
                        }),
                )
                .when(dot, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(-2.))
                            .right(px(-2.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(accent_color()),
                    )
                }),
        )
}

impl Shell {
    /// The open sidebar's drawn width.
    pub(super) fn left_sidebar_width(&self) -> f32 {
        self.workspace.sidebar_width * self.sidebar_fraction
    }
    /// Flip the Focused layout between its rounded card and boxy frame.
    pub(super) fn toggle_workspace_style(&mut self, cx: &mut Context<Self>) {
        self.workspace.new_layout_boxy = !self.workspace.new_layout_boxy;
        self.layout_changed(cx);
        self.sync_settings(cx);
    }
    pub(super) fn layout_changed(&mut self, cx: &mut Context<Self>) {
        let classic = self.workspace.boxy_layout();
        if let Some(view) = self.active_app_view {
            self.remember_title_tab(TitleTab::App(view));
        }
        if let Some(id) = self.pilots.as_ref().and_then(|m| m.read(cx).active.clone()) {
            self.remember_title_tab(self.pilot_tab(id));
            self.reveal_active_tab(cx);
        }
        self.usage_footer
            .update(cx, |footer, cx| footer.set_classic(classic, cx));
        self.resources
            .update(cx, |monitor, cx| monitor.set_classic(classic, cx));
        self.invalidate_chrome(cx);
        self.drop_peek();
        self.sync_right_panel(cx);
        self.fit_sidebar(cx);
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn layout_body(
        &mut self,
        sidebar_width: f32,
        right_shown: bool,
        right_width: f32,
        full: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let classic = self.workspace.boxy_layout();
        self.usage_footer
            .update(cx, |footer, cx| footer.set_classic(classic, cx));
        self.resources
            .update(cx, |monitor, cx| monitor.set_classic(classic, cx));
        if classic {
            return vec![self.classic_body(
                sidebar_width,
                right_shown,
                right_width,
                full,
                window,
                cx,
            )];
        }
        let card = self.workspace_content(right_shown, right_width, full, window, cx);
        let card = div()
            .id("rounded-main-pane")
            .debug_selector(|| "rounded-main-pane".into())
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .overflow_hidden()
            .child(card_corner_mask(true))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .overflow_hidden()
                    .bg(inset_pane_fill())
                    .rounded(px(CARD_RADIUS))
                    .border_1()
                    .border_color(content(0.08))
                    .child(card),
            )
            .child(card_corner_mask(false));
        let footer = self.footer_row(cx);
        vec![
            div()
                .h(px(TITLE_BAR_HEIGHT))
                .flex_shrink_0()
                .bg(inset_chrome_wash())
                .child(self.titlebar(window, cx))
                .into_any_element(),
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .relative()
                .when(sidebar_width > 0., |d| {
                    d.child(
                        // Keep the sidebar still beneath the moving card. Clip
                        // its covered part so text cannot bleed through the
                        // translucent workspace or receive covered clicks.
                        div()
                            .id("sidebar-frame")
                            .debug_selector(|| "sidebar-frame".into())
                            .absolute()
                            .left_0()
                            .top_0()
                            .w(px(sidebar_width))
                            .h_full()
                            .overflow_hidden()
                            .bg(inset_chrome_wash())
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .w(px(self.workspace.sidebar_width))
                                    .h_full()
                                    .opacity(self.sidebar_fraction)
                                    .child(self.sidebar_contents()),
                            ),
                    )
                })
                .child(div().w(px(sidebar_width)).h_full().flex_shrink_0())
                .child(
                    div()
                        .id("main-pane-frame")
                        .debug_selector(|| "main-pane-frame".into())
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        .pr_1p5()
                        .pl(px(6. * (1. - self.sidebar_fraction)))
                        .child(
                            div()
                                .absolute()
                                .right_0()
                                .top_0()
                                .bottom_0()
                                .w(px(6.))
                                .bg(inset_chrome_wash()),
                        )
                        .when(self.sidebar_fraction < 1., |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(6. * (1. - self.sidebar_fraction)))
                                    .bg(inset_chrome_wash()),
                            )
                        })
                        .child(card)
                        .child(footer),
                )
                .children(self.sidebar_resize(cx))
                .into_any_element(),
        ]
    }
    fn classic_body(
        &mut self,
        sidebar_width: f32,
        right_shown: bool,
        right_width: f32,
        full: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card = self.workspace_content(false, right_width, full, window, cx);
        let panel = self
            .right_panel
            .clone()
            .filter(|_| right_shown && self.settings.is_none())
            .map(|panel| self.right_panel_frame(panel, right_width, full, window, cx))
            .map(|panel| div().h_full().bg(pane_fill()).child(panel));
        let footer = self.footer_row(cx);
        let tab_bar = self.titlebar(window, cx);
        let navigation = self.navigation(cx);
        let corner = self.settings.is_none().then(|| self.panel_toggle(cx));
        // As in the rounded layout, boxy's title bar is its own row across the
        // workspace and the right panel, and the panel starts below it.
        let main = div()
            .id("main-pane-frame")
            .debug_selector(|| "main-pane-frame".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(pane_fill())
            .child(card)
            .child(
                footer
                    .h(px(28.))
                    .px_3()
                    .border_t_1()
                    .border_color(crate::right_panel::stroke()),
            );
        let workspace = div()
            .id("boxy-workspace-column")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(pane_fill())
            .child(tab_bar)
            .child(div().flex_1().min_h_0().flex().child(main).children(panel));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .relative()
            .when(sidebar_width > 0., |d| {
                d.child(
                    div()
                        .id("sidebar-frame")
                        .debug_selector(|| "sidebar-frame".into())
                        .relative()
                        .w(px(sidebar_width))
                        .h_full()
                        .flex_shrink_0()
                        .overflow_hidden()
                        .border_r_1()
                        .border_color(crate::right_panel::stroke())
                        .child(
                            // Full width while it slides; the frame clips it.
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .w(px(self.workspace.sidebar_width))
                                .h_full()
                                .flex()
                                .flex_col()
                                .opacity(self.sidebar_fraction)
                                .child(
                                    // The traffic lights head the sidebar, where
                                    // the window's top is; the toggle floats
                                    // above them (see below).
                                    titlebar_drag_area()
                                        .w_full()
                                        .h(px(TITLE_BAR_HEIGHT))
                                        .flex_shrink_0(),
                                )
                                .child(div().flex_1().min_h_0().child(self.sidebar_contents())),
                        ),
                )
            })
            .child(workspace)
            .children(self.sidebar_resize(cx))
            .child(
                // The traffic lights, toggle and arrows ride above the sidebar's
                // header as in the inset title bar: the toggle holds still, and
                // the arrows sit at the open sidebar's edge or after the toggle.
                div().absolute().top_0().left_0().child(navigation),
            )
            .child(
                // The right panel toggle holds the window's corner too, above
                // the strip or an open panel's tab row. Its padding keeps
                // clear of Windows' controls; Linux's sit beside it.
                div()
                    .absolute()
                    .top_0()
                    .right(px(
                        super::client_controls_width(window) - self.window_controls
                    ))
                    .h(px(TITLE_BAR_HEIGHT))
                    .children(corner),
            )
            .into_any_element()
    }
    /// Classic: the corner's buttons, `pr-3` from the edge.
    pub(super) fn corner_room(&self, cx: &gpui::App) -> f32 {
        // Client-drawn window controls hold the corner's right end.
        let controls = self.window_controls;
        if !self.pilot_active(cx) && !self.git.read(cx).available() {
            return 6. + 24. + self.open_in_editor_room(cx) + 12. + controls;
        }
        // The toggle alone; the panel heads itself with its tabs.
        6. + 24. + 12. + controls
    }
    fn corner_tabs_shown(&self) -> bool {
        self.workspace.right_panel_open && !self.app_view_open() && self.settings.is_none()
    }
    /// Classic: the panel toggle holds the corner, spanning an open panel;
    /// the panel's own row of tabs switches its tools.
    pub(super) fn classic_corner(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = div()
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center();
        if !self.git.read(cx).available() {
            return row
                .pl_1p5()
                .pr(px(12. + self.window_controls))
                .children(
                    self.open_in_editor_button(cx)
                        .map(|b| b.mr(px(CORNER_CONTROL_GAP))),
                )
                .child(self.file_search_button(cx))
                .into_any_element();
        }
        let panel_off = self.app_view_open();
        let changes = !panel_off && self.git.read(cx).has_changes();
        let open = self.corner_tabs_shown();
        // Over the panel, the corner is the toggle alone at the strip's end.
        row.pl_1p5()
            .pr(px(12. + self.window_controls))
            .child(
                // The toggle carries the changes dot.
                panel_button(!panel_off, !open && changes).when(!panel_off, |d| {
                    d.on_click(
                        cx.listener(|this, _, window, cx| this.toggle_right_panel(window, cx)),
                    )
                }),
            )
            .into_any_element()
    }
    fn sidebar_contents(&self) -> AnyElement {
        // Cached paint doesn't include inherited opacity in its key. Repaint
        // during the fade, even if the clip rounds to the same pixel on
        // successive frames.
        if self.sidebar_uncached() {
            self.sidebar_frame.clone().into_any_element()
        } else {
            self.sidebar_frame
                .clone()
                .cached(
                    StyleRefinement::default()
                        .w(px(self.workspace.sidebar_width))
                        .h_full(),
                )
                .into_any_element()
        }
    }
    fn sidebar_resize(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.workspace.sidebar_visible && self.sidebar_slide.is_none()).then(|| {
            // The boxy sidebar owns its right rule; the inset workspace
            // owns the rule just beyond the sidebar's edge.
            let border = self.workspace.sidebar_width
                + if self.workspace.boxy_layout() {
                    -0.5
                } else {
                    0.5
                };
            div()
                .id("sidebar-resize")
                .absolute()
                .left(px(border - 2.))
                .w(px(4.))
                .h_full()
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.resize = Some((event.position.x, this.workspace.sidebar_width));
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        })
    }
    /// The workspace body, its error banner and the right panel beside them.
    fn workspace_content(
        &mut self,
        right_shown: bool,
        right_width: f32,
        full: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if let Some(settings) = &self.settings {
            return div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .overflow_hidden()
                .child(settings.clone());
        }
        let panel: Option<Entity<RightPanel>> = self
            .right_panel
            .clone()
            .filter(|_| right_shown && self.settings.is_none());
        let row = div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .child(self.background_card.probe())
            .child(
                div().flex_1().min_w_0().h_full().flex().flex_col().child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        // Every project lands on a blank session, as in
                        // Gespan; this is empty only while one is created.
                        .child({
                            if let Some(pilots) =
                                self.pilots.clone().filter(|m| m.read(cx).active.is_some())
                            {
                                let session = pilots.read(cx).active_notification_session(cx);
                                let linked = session.and_then(|id| self.linked_notice(id, cx));
                                div()
                                    .relative()
                                    .size_full()
                                    .child(self.pilot_bounds.layout_probe())
                                    .child(pilots)
                                    .children(self.drop_hint(super::pilot_panes::PILOT_VIEW))
                                    .children(linked.map(|notice| {
                                        div()
                                            .absolute()
                                            .top(px(12.))
                                            .right(px(12.))
                                            .w(px(320.))
                                            .max_w_full()
                                            .child(notice)
                                    }))
                                    .into_any_element()
                            } else {
                                let body = self.workspace_body(window, cx);
                                self.with_dock(body, window, cx)
                            }
                        }),
                ),
            )
            .when_some(panel, |d, panel| {
                d.child(self.right_panel_frame(panel, right_width, full, window, cx))
            });
        // The menu bar spans the right panel too, so its headers stay level
        // with the open-file tabs.
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .children(self.menu_bar(cx))
            .child(row)
    }
    /// The usage chips with the terminal controls at the far end.
    fn footer_row(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let height = if self.workspace.boxy_layout() {
            28.
        } else {
            30.
        };
        div()
            .w_full()
            .h(px(30.))
            .flex_shrink_0()
            .when(!self.workspace.boxy_layout(), |d| {
                d.overflow_hidden().bg(inset_chrome_wash())
            })
            .flex()
            .items_center()
            .child(
                self.usage_footer
                    .clone()
                    .cached(StyleRefinement::default().flex_1().min_w_0().h(px(height))),
            )
            .map(|d| {
                let pressure = self.pressure_chip(cx);
                let terminal = self.terminal_footer(cx);
                let between = pressure.is_some() && terminal.is_some();
                d.children(pressure)
                    .when(between, |d| {
                        d.child(
                            super::terminal_dock::footer_separator()
                                .mr(px(super::terminal_dock::FOOTER_GAP + 3.))
                                .when(!self.workspace.boxy_layout(), |d| d.mb(px(2.))),
                        )
                    })
                    .children(terminal)
            })
            .child(self.hold_control(cx))
    }
    /// ResourcePressureNotice's chip, just before the terminal controls.
    fn pressure_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.resources.read(cx).chip_shown() {
            return None;
        }
        let classic = self.workspace.boxy_layout();
        // The terminal group sits one footer gap on; alone, the chip takes its inset margin.
        let alone = self.app_view_open() || self.workspace.project.is_empty();
        Some(
            div()
                .flex_shrink_0()
                .map(|d| match (classic, alone) {
                    (true, true) => d,
                    (true, false) => d.mr(px(super::terminal_dock::FOOTER_GAP)),
                    (false, true) => d.mr(px(6.)).pb(px(2.)),
                    (false, false) => d.mr(px(super::terminal_dock::FOOTER_GAP)).pb(px(2.)),
                })
                .child(self.resources.clone())
                .into_any_element(),
        )
    }
    /// A small modal, fit to the viewport: 420px, centred.
    pub(super) fn resource_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.resources.read(cx).is_open() {
            return None;
        }
        let mut sessions: HashMap<i64, SessionInfo> = self
            .sessions
            .iter()
            .map(|(id, session)| {
                let summary = &session.read(cx).summary;
                let info = SessionInfo {
                    title: summary.title.clone().into(),
                    provider: summary.provider.clone(),
                };
                (*id, info)
            })
            .collect();
        if let Some(m) = &self.pilots {
            sessions.extend(m.read(cx).resource_sessions(cx));
        }
        let content = self
            .resources
            .update(cx, |monitor, cx| monitor.dialog(&sessions, window, cx));
        let viewport = window.viewport_size();
        let width = 420f32.min(viewport.width.as_f32() - 24.);
        let height = (self.resources.read(cx).measured_height().unwrap_or(240.) + 2.)
            .min(viewport.height.as_f32() - 32.);
        let anchor = Anchor::Point(gpui::point(
            px((viewport.width.as_f32() - width) / 2.),
            px((viewport.height.as_f32() - height) / 2.),
        ));
        let monitor = self.resources.downgrade();
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("running-work-scrim"))
                .child(
                    popover("running-work-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .max_height(viewport.height.as_f32() - 32.)
                        .priority(60)
                        .on_dismiss(move |window, cx| {
                            let _ = monitor.update(cx, |monitor, cx| monitor.close(window, cx));
                        })
                        .child(content),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::right_panel::RightTab;
    use crate::{pilots::Pilots, shell::tests::window};
    use gpui::{TestAppContext, WindowHandle};
    use std::time::{Duration, Instant};

    fn mixed_window(cx: &mut TestAppContext) -> (WindowHandle<Shell>, Entity<Pilots>, String, i64) {
        let (window, _, session) = window(cx);
        let (manager, id) = window
            .update(cx, |shell, w, cx| {
                let manager = shell.pilot_manager();
                manager.update(cx, |m, cx| {
                    m.create(Vec::new(), w, cx);
                    m.dismiss_settings(w, cx);
                });
                let id = manager.read(cx).active.clone().unwrap();
                (manager, id)
            })
            .unwrap();
        cx.run_until_parked();
        (window, manager, id, session)
    }

    #[gpui::test]
    fn narrow_panels_keep_every_text_tab_in_their_row(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.toggle_right_panel(w, cx);
                shell.git.update(cx, |git, cx| {
                    git.stats = Some(gespan_core::scm::DiffStats {
                        files: 550,
                        additions: 246_137,
                        deletions: 11_081,
                    });
                    cx.notify();
                });
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let selectors = [
            "right-panel-tab-Explorer",
            "right-panel-tab-Changes",
            "right-panel-tab-History",
        ];
        for boxy in [false, true] {
            for width in [420., 320., 220.] {
                window
                    .update(visual, |shell, _, cx| {
                        shell.workspace.new_layout_boxy = boxy;
                        shell.workspace.right_panel_width = width;
                        shell
                            .right_panel
                            .as_ref()
                            .unwrap()
                            .update(cx, |panel, cx| panel.select(RightTab::Graph, cx));
                        shell.invalidate_chrome(cx);
                        cx.notify();
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.refresh().unwrap();
                let context = format!("boxy {boxy}, {width}px");
                let strip = visual.debug_bounds("right-panel-tabs").unwrap();
                let tabs = selectors.map(|selector| visual.debug_bounds(selector).unwrap());
                // Even the largest diff stat leaves every tab inside the
                // strip, clear of the toggle.
                for pair in tabs.windows(2) {
                    assert!(pair[0].right() <= pair[1].left(), "{context}");
                }
                assert!(tabs[0].left() >= strip.left(), "{context}");
                assert!(tabs[2].right() <= strip.right(), "{context}");
                let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
                assert!(toggle.bottom() <= strip.top(), "{context}");
                assert_eq!(toggle.size.width, px(24.));
                assert!(
                    visual.debug_bounds("project-diff-stat").is_none(),
                    "{context}"
                );
                assert!(
                    visual.debug_bounds("titlebar-file-search").is_none(),
                    "{context}"
                );
                // A narrow tab still switches the pane.
                visual.simulate_click(tabs[0].center(), Default::default());
                visual.run_until_parked();
                window
                    .update(visual, |shell, _, cx| {
                        assert_eq!(
                            shell.right_panel.as_ref().unwrap().read(cx).tab(),
                            RightTab::Files,
                            "{context}"
                        )
                    })
                    .unwrap();
            }
        }
    }

    #[gpui::test]
    fn boxy_title_bar_spans_the_open_panel_and_the_panel_starts_below_it(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.workspace.new_layout_boxy = true;
                shell.workspace.right_panel_width = 320.;
                shell.toggle_right_panel(w, cx);
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        let frame = visual.debug_bounds("right-panel-frame").unwrap();
        let strip = visual.debug_bounds("title-tab-strip").unwrap();
        let tabs = visual.debug_bounds("right-panel-tabs").unwrap();
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        // The session tabs run over the open panel, as with it closed.
        assert!(strip.right() > frame.left(), "{strip:?} vs {frame:?}");
        assert!(toggle.left() >= strip.right(), "the toggle ends the strip");
        // The panel sits under the title bar and heads itself with its tabs,
        // with no empty band where the corner used to be.
        assert!(frame.top() >= strip.bottom(), "{frame:?} vs {strip:?}");
        assert!(toggle.bottom() <= frame.top());
        assert!(
            (tabs.top() - frame.top()).abs() <= px(1.),
            "{tabs:?} vs {frame:?}"
        );
    }

    #[gpui::test]
    fn right_sidebar_width_and_visibility_follow_both_tab_types(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, manager, pilot, session) = mixed_window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, _, cx| {
                assert!(
                    !shell.workspace.right_panel_open,
                    "opening a Pilot respects the session's closed sidebar"
                );
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("right-panel-frame").is_none());
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("pilot-overview-panel").is_some());
        let frame = visual.debug_bounds("right-panel-frame").unwrap();
        let bounds = visual.debug_bounds("right-panel-resize").unwrap();
        assert_eq!(bounds.size.width, px(6.));
        assert!((bounds.center().x - frame.left() - px(0.5)).abs() <= px(0.5));
        // Both halves of the transparent grab area must receive presses,
        // including the half outside the panel's clipped contents.
        for x in [bounds.left() + px(0.5), bounds.right() - px(0.5)] {
            let point = gpui::point(x, bounds.center().y);
            visual.simulate_mouse_down(point, MouseButton::Left, Default::default());
            window
                .update(visual, |shell, _, _| assert!(shell.right_resize.is_some()))
                .unwrap();
            visual.simulate_mouse_up(point, MouseButton::Left, Default::default());
            visual.run_until_parked();
        }
        let handle = visual.debug_bounds("right-panel-resize").unwrap().center();
        let end = handle - gpui::point(px(30.), px(0.));
        visual.simulate_mouse_down(handle, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
        visual.run_until_parked();
        let expanded = window
            .update(visual, |shell, _, _| shell.workspace.right_panel_width)
            .unwrap();
        let handle = visual.debug_bounds("right-panel-resize").unwrap().center();
        let end = handle + gpui::point(px(30.), px(0.));
        visual.simulate_mouse_down(handle, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
        // The pointer is now over a Pilot detail layer that occludes mouse
        // events. Shrinking and releasing must still reach the shell.
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.right_panel_width, expanded - 30.);
            })
            .unwrap();
        visual.simulate_mouse_up(
            end + gpui::point(px(20.), px(0.)),
            MouseButton::Left,
            Default::default(),
        );
        visual.run_until_parked();
        let pilot_bounds = visual.debug_bounds("right-panel-frame").unwrap();
        let width = window
            .update(visual, |shell, w, cx| {
                assert!(shell.right_resize.is_none());
                let width = shell.workspace.right_panel_width;
                assert_eq!(pilot_bounds.size.width, px(width));
                shell.open(session, w, cx);
                width
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("right-panel-frame").unwrap(),
            pilot_bounds
        );
        assert!(visual.debug_bounds("pilot-overview-panel").is_none());
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |_, w, cx| {
                manager.update(cx, |m, cx| m.open(&pilot, w, cx))
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("right-panel-frame").is_none());
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.right_panel_width, width);
                let saved = crate::runtime::locked(&shell.store)
                    .unwrap()
                    .ui_state()
                    .unwrap()
                    .unwrap();
                let saved: crate::workspace::Workspace = serde_json::from_str(&saved).unwrap();
                assert_eq!(saved.right_panel_width, width);
                assert!(!saved.right_panel_open);
            })
            .unwrap();
    }

    #[gpui::test]
    fn switching_tabs_keeps_the_same_sidebar_spring_and_clipped_contents(cx: &mut TestAppContext) {
        let (window, manager, pilot, session) = mixed_window(cx);
        cx.update(|cx| cx.set_reduce_motion(false));
        window
            .update(cx, |shell, w, cx| shell.toggle_right_panel(w, cx))
            .unwrap();
        for ms in [35, 70, 110] {
            window
                .update(cx, |shell, w, cx| {
                    let slide = shell.right_slide.as_ref().unwrap();
                    slide.elapsed(Duration::from_millis(ms));
                    let now = Instant::now();
                    let before = slide.at(now);
                    if shell.pilot_active(cx) {
                        shell.open(session, w, cx);
                    } else {
                        manager.update(cx, |m, cx| m.open(&pilot, w, cx));
                    }
                    assert_eq!(
                        shell.right_slide.as_ref().unwrap().at(now),
                        before,
                        "tab switches preserve the spring's position and velocity"
                    );
                    assert!(shell.workspace.right_panel_open);
                    cx.notify();
                })
                .unwrap();
            cx.update_window(window.into(), |_, w, cx| {
                w.simulate_next_frame(cx);
                w.draw(cx).clear(cx);
            })
            .unwrap();
            let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            let frame = visual.debug_bounds("right-panel-frame").unwrap();
            let content = visual.debug_bounds("right-panel-content").unwrap();
            let width = window
                .update(visual, |shell, _, _| shell.workspace.right_panel_width)
                .unwrap();
            assert!(frame.size.width > px(0.) && frame.size.width < px(width));
            assert_eq!(content.size.width, px(width - 1.));
        }
        window
            .update(cx, |shell, w, cx| {
                shell.toggle_right_panel(w, cx);
                assert!(!shell.workspace.right_panel_open);
                assert_eq!(shell.right_slide.as_ref().unwrap().to, 0.);
                shell
                    .right_slide
                    .as_ref()
                    .unwrap()
                    .elapsed(Duration::from_millis(45));
                shell.open(session, w, cx);
                shell.toggle_right_panel(w, cx);
                assert!(shell.workspace.right_panel_open);
                assert_eq!(
                    shell.right_slide.as_ref().unwrap().to,
                    shell.workspace.right_panel_width
                );
                manager.update(cx, |m, cx| m.open(&pilot, w, cx));
                shell
                    .right_slide
                    .as_ref()
                    .unwrap()
                    .elapsed(Duration::from_secs(1));
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| assert!(shell.right_slide.is_none()))
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        assert_eq!(
            visual.debug_bounds("right-panel-frame").unwrap().size.width,
            px(window
                .update(visual, |shell, _, _| shell.workspace.right_panel_width)
                .unwrap())
        );
        assert!(visual.debug_bounds("pilot-overview-panel").is_some());
    }
}
