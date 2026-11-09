//! The inset right panel: the project tools beside the card, headed by a row
//! of reorderable Explorer / Changes / History text tabs, with a Terminal tab
//! when the terminal lives in the panel. The title bar keeps only its toggle.
use crate::{
    changes::Changes,
    explorer::Explorer,
    git_status::{GitChanged, ProjectGit},
    history::History,
    theme::*,
};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, FontFeatures,
    FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Role, SharedString,
    Stateful, StyleRefinement, Subscription, Window, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const WIDTH_MIN: f32 = 220.;
pub(crate) const WIDTH_MAX: f32 = 640.;
pub(crate) const WIDTH_DEFAULT: f32 = 288.;
// Separators inside the inset card use its 4% hairline; dark selection is 10%.
pub(crate) fn stroke() -> Hsla {
    content(0.04)
}
pub(crate) fn selection() -> Hsla {
    content(0.1)
}
pub(crate) const DIFF_ADD: u32 = 0x6ee7b7;
pub(crate) const DIFF_DEL: u32 = 0xfda4af;
// Tailwind v4.
pub(crate) const AMBER_400: u32 = 0xffb900;
pub(crate) const SKY_400: u32 = 0x00bcff;
pub(crate) const RED_400: u32 = 0xff6467;
/// `--color-background-base` in the default dark theme, hsl(240 0% 9%).
pub(crate) const BACKGROUND_BASE: u32 = 0x171717;

pub(crate) fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RightTab {
    #[default]
    Files,
    Changes,
    Graph,
    /// The project's terminal, when it opens in the panel. Always last and
    /// never part of the saved order.
    Terminal,
}
impl RightTab {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Explorer",
            Self::Changes => "Changes",
            Self::Graph => "History",
            Self::Terminal => "Terminal",
        }
    }
    /// loadRightPanelTabOrder: unique known tabs, each missing one placed after
    /// the tab it follows by default, so an order saved without Changes gets
    /// it back between Explorer and History.
    pub(crate) fn normalize(order: &[RightTab]) -> Vec<RightTab> {
        const DEFAULT: [RightTab; 3] = [RightTab::Files, RightTab::Changes, RightTab::Graph];
        let mut out: Vec<RightTab> = Vec::new();
        for tab in order {
            if *tab != Self::Terminal && !out.contains(tab) {
                out.push(*tab);
            }
        }
        for (ix, tab) in DEFAULT.iter().enumerate() {
            if !out.contains(tab) {
                let at = DEFAULT[..ix]
                    .iter()
                    .filter_map(|before| out.iter().position(|t| t == before))
                    .max()
                    .map_or(0, |p| p + 1);
                out.insert(at, *tab);
            }
        }
        out
    }
}

pub(crate) enum PanelEvent {
    Tab(RightTab),
    Reordered(Vec<RightTab>),
}

/// Tool headers share the open-file tab bar's height and bottom rule.
pub(crate) fn pane_header(id: &'static str, label: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Toolbar)
        .aria_label(label)
        .h(px(PANE_HEADER_HEIGHT))
        .flex_shrink_0()
        .min_w_0()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(stroke())
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(content(0.85))
}

/// GitChangesPanel's `IconAction`: a 20px square, ink at 55%.
pub(crate) fn icon_action(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    path: &'static str,
    enabled: bool,
) -> Stateful<gpui::Div> {
    let id = id.into();
    let group: SharedString = format!("icon-action-{id:?}").into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .group(group.clone())
        .size(px(20.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when(!enabled, |d| d.opacity(0.4))
        .when(enabled, |d| d.hover(|s| s.bg(content(0.1))))
        .child(
            icon(path, 14.)
                .text_color(content(0.55))
                .when(enabled, |i| {
                    i.group_hover(group, |s| s.text_color(content(1.)))
                }),
        )
}

/// The `animate-spin` Loader.
pub(crate) fn spinner(id: impl Into<gpui::ElementId>, path: &'static str, side: f32) -> AnyElement {
    use gpui::{Transformation, percentage};
    crate::pulse::looping(
        id,
        std::time::Duration::from_secs(1),
        crate::pulse::Rate::Smooth,
        move |delta| {
            icon(path, side).with_transformation(Transformation::rotate(percentage(delta)))
        },
    )
    .into_any_element()
}

pub(crate) fn format_integer(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 { format!("-{out}") } else { out }
}

/// The Changes tab's diff stat, `+added` and `-deleted`, each only when
/// nonzero. Empty when the tree is clean, so the tab reads "Changes".
pub(crate) fn changes_stat(stats: Option<&gespan_core::scm::DiffStats>) -> Vec<(String, u32)> {
    let (added, deleted) = stats.map_or((0, 0), |s| (s.additions.max(0), s.deletions.max(0)));
    let mut parts = Vec::new();
    if added > 0 {
        parts.push((format!("+{}", format_integer(added)), DIFF_ADD));
    }
    if deleted > 0 {
        parts.push((format!("-{}", format_integer(deleted)), DIFF_DEL));
    }
    parts
}

/// The Changes tab's accessible name: "Changes", then its diff stat.
pub(crate) fn changes_tab_label(stats: Option<&gespan_core::scm::DiffStats>) -> String {
    std::iter::once("Changes".to_string())
        .chain(changes_stat(stats).into_iter().map(|(text, _)| text))
        .collect::<Vec<_>>()
        .join(" ")
}

struct TabDrag {
    tab: RightTab,
    origin: Pixels,
    offset: Pixels,
    active: bool,
}

pub(crate) struct RightPanel {
    git: Entity<ProjectGit>,
    pub explorer: Entity<Explorer>,
    pub changes: Entity<Changes>,
    pub history: Entity<History>,
    focus: FocusHandle,
    tab: RightTab,
    order: Vec<RightTab>,
    shown: bool,
    /// Whether the Terminal tab follows the others; the shell draws its pane.
    terminal_tab: bool,
    drag: Option<TabDrag>,
    skip_click: bool,
    tab_bounds: Vec<crate::popover::AnchorSlot>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<PanelEvent> for RightPanel {}
impl Focusable for RightPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RightPanel {
    pub(crate) fn new(
        git: Entity<ProjectGit>,
        tab: RightTab,
        order: Vec<RightTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let explorer = cx.new(|cx| Explorer::new(git.clone(), window, cx));
        let changes = cx.new(|cx| Changes::new(git.clone(), window, cx));
        let history = cx.new(|cx| History::new(git.clone(), window, cx));
        let subscriptions = vec![
            cx.observe(&git, |_, _, cx| cx.notify()),
            cx.subscribe(&git, |_, _, _: &GitChanged, cx| cx.notify()),
        ];
        Self {
            git,
            focus: cx.focus_handle(),
            explorer,
            changes,
            history,
            tab,
            order: RightTab::normalize(&order),
            shown: false,
            terminal_tab: false,
            drag: None,
            skip_click: false,
            tab_bounds: (0..5).map(|_| Default::default()).collect(),
            _subscriptions: subscriptions,
        }
    }
    pub(crate) fn tab(&self) -> RightTab {
        self.tab
    }
    /// The tabs in strip order: the saved three, with Terminal after the
    /// first when enabled.
    pub(crate) fn tabs(&self) -> Vec<RightTab> {
        let mut tabs = self.order.clone();
        if self.terminal_tab {
            tabs.insert(1.min(tabs.len()), RightTab::Terminal);
        }
        tabs
    }
    /// Adds or removes the Terminal tab; Explorer takes over when it goes.
    pub(crate) fn set_terminal_tab(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.terminal_tab == enabled {
            return;
        }
        self.terminal_tab = enabled;
        if !enabled && self.tab == RightTab::Terminal {
            self.select(RightTab::Files, cx);
        }
        cx.notify();
    }
    pub(crate) fn set_show_excluded(&mut self, show: bool, cx: &mut Context<Self>) {
        self.explorer
            .update(cx, |explorer, cx| explorer.set_show_excluded(show, cx));
    }
    /// The panes read and poll only while their tab is on screen.
    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        self.sync_panes(cx);
    }
    pub(crate) fn select(&mut self, tab: RightTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.sync_panes(cx);
        cx.emit(PanelEvent::Tab(tab));
        cx.notify();
    }
    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        let (shown, tab) = (self.shown, self.tab);
        self.explorer.update(cx, |e, cx| {
            e.set_enabled(shown && tab == RightTab::Files, cx)
        });
        self.changes.update(cx, |c, cx| {
            c.set_enabled(shown && tab == RightTab::Changes, cx)
        });
        self.history.update(cx, |h, cx| {
            h.set_enabled(shown && tab == RightTab::Graph, cx)
        });
    }
    pub(crate) fn focus_pane(&self, window: &mut Window, cx: &mut App) {
        let focus = match (self.shown, self.tab) {
            (true, RightTab::Files) => self.explorer.focus_handle(cx),
            (true, RightTab::Graph) => self.history.focus_handle(cx),
            _ => self.focus.clone(),
        };
        window.focus(&focus, cx);
    }
    pub(crate) fn finish_opening(&self, window: &mut Window, cx: &mut App) {
        if self.focus.is_focused(window) {
            self.focus_pane(window, cx);
        }
    }

    fn start_drag(&mut self, tab: RightTab, event: &MouseDownEvent) {
        self.drag = Some(TabDrag {
            tab,
            origin: event.position.x,
            offset: px(0.),
            active: false,
        });
    }
    fn move_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let tabs = self.tabs();
        let Some(drag) = &mut self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_drag(cx);
            return;
        }
        drag.offset = event.position.x - drag.origin;
        if !drag.active && drag.offset.abs() < px(4.) {
            return;
        }
        drag.active = true;
        // Swap with a neighbour once the dragged tab's centre crosses its centre.
        // Terminal holds its slot, outside the reorderable tabs.
        let Some(from) = tabs.iter().position(|t| *t == drag.tab) else {
            return;
        };
        let bounds: Vec<Option<Bounds<Pixels>>> = tabs
            .iter()
            .zip(&self.tab_bounds)
            .map(|(tab, slot)| slot.get().filter(|_| *tab != RightTab::Terminal))
            .collect();
        if let Some(Some(own)) = bounds.get(from) {
            let centre = own.center().x + drag.offset;
            let target = bounds
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != from)
                .find(|(ix, b)| {
                    b.is_some_and(|b| {
                        if *ix > from {
                            centre > b.center().x
                        } else {
                            centre < b.center().x
                        }
                    })
                })
                .map(|(ix, _)| ix);
            if let Some(to) = target
                && let Some(Some(other)) = bounds.get(to)
            {
                // A selected tab includes its label, so moving right crosses
                // the other tabs' widths rather than the selected tab's width.
                let shift = if to > from {
                    other.right() - own.right()
                } else {
                    other.left() - own.left()
                };
                let (a, b) = (
                    self.order.iter().position(|t| *t == tabs[from]),
                    self.order.iter().position(|t| *t == tabs[to]),
                );
                if let (Some(a), Some(b)) = (a, b) {
                    self.order.swap(a, b);
                }
                drag.origin += shift;
                drag.offset -= shift;
            }
        }
        cx.notify();
    }
    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.take()
            && drag.active
        {
            self.skip_click = true;
            cx.emit(PanelEvent::Reordered(self.order.clone()));
            cx.notify();
        }
    }

    /// The project tab row: equal text tabs sharing the panel's width,
    /// the selected one filled. Changes shows the working tree's diff stat.
    pub(crate) fn tab_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let stat = changes_stat(self.git.read(cx).stats.as_ref());
        let changes_label = changes_tab_label(self.git.read(cx).stats.as_ref());
        let dragging = self
            .drag
            .as_ref()
            .filter(|d| d.active)
            .map(|d| (d.tab, d.offset));
        let tabs: Vec<AnyElement> = self
            .tabs()
            .into_iter()
            .enumerate()
            .map(|(ix, tab)| {
                let active = self.tab == tab;
                let id = match tab {
                    RightTab::Files => "right-panel-tab-Explorer",
                    RightTab::Changes => "right-panel-tab-Changes",
                    RightTab::Graph => "right-panel-tab-History",
                    RightTab::Terminal => "right-panel-tab-Terminal",
                };
                let label: SharedString = if tab == RightTab::Changes {
                    changes_label.clone().into()
                } else {
                    tab.label().into()
                };
                let body = if tab == RightTab::Changes && !stat.is_empty() {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_features(tabular())
                        .children(stat.iter().map(|(text, color)| {
                            div().text_color(gpui::rgb(*color)).child(text.clone())
                        }))
                        .into_any_element()
                } else {
                    div()
                        .min_w_0()
                        .truncate()
                        .child(tab.label())
                        .into_any_element()
                };
                let shift = dragging.filter(|(t, _)| *t == tab).map(|(_, o)| o);
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    // Keep the measured slot still while its button follows
                    // the pointer; drag offsets are relative to the slot.
                    .child(self.tab_bounds[ix].probe())
                    .child(
                        div()
                            .id(id)
                            .debug_selector(move || id.into())
                            .role(Role::Tab)
                            .aria_label(label)
                            .aria_selected(active)
                            .relative()
                            .when_some(shift, |d, o| d.left(o))
                            .h(px(24.))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(content(if active || shift.is_some() { 1. } else { 0.5 }))
                            .when(active, |d| d.bg(selection()))
                            .when(!active, |d| d.hover(|s| s.text_color(content(1.))))
                            .child(body)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.start_drag(tab, event)
                                }),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if std::mem::take(&mut this.skip_click) {
                                    return;
                                }
                                this.select(tab, cx);
                                this.focus_pane(window, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("right-panel-tabs")
            .debug_selector(|| "right-panel-tabs".into())
            .role(Role::TabList)
            .aria_label("Project")
            .h(px(PANE_HEADER_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .px_2()
            .border_b_1()
            .border_color(stroke())
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.move_drag(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_drag(cx)),
            )
            .children(tabs)
            .into_any_element()
    }
}

impl Render for RightPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strip = self.tab_strip(cx);
        let pane: AnyElement = match self.tab {
            RightTab::Files => self
                .explorer
                .clone()
                .cached(StyleRefinement::default().flex_1().min_h_0().w_full())
                .into_any_element(),
            RightTab::Changes => self
                .changes
                .clone()
                .cached(StyleRefinement::default().flex_1().min_h_0().w_full())
                .into_any_element(),
            RightTab::Graph => self
                .history
                .clone()
                .cached(StyleRefinement::default().flex_1().min_h_0().w_full())
                .into_any_element(),
            // The shell paints the terminal over this pane.
            RightTab::Terminal => div().into_any_element(),
        };
        div()
            .id("right-panel")
            .track_focus(&self.focus)
            .role(Role::Group)
            .aria_label("Project tools")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(strip)
            .child(pane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tab_order_keeps_known_tabs_once() {
        assert_eq!(
            RightTab::normalize(&[]),
            [RightTab::Files, RightTab::Changes, RightTab::Graph]
        );
        assert_eq!(
            RightTab::normalize(&[RightTab::Graph, RightTab::Graph]),
            [RightTab::Files, RightTab::Changes, RightTab::Graph]
        );
        // Orders saved while Changes had no tab get it back after Explorer.
        assert_eq!(
            RightTab::normalize(&[RightTab::Files, RightTab::Graph]),
            [RightTab::Files, RightTab::Changes, RightTab::Graph]
        );
        assert_eq!(
            RightTab::normalize(&[RightTab::Graph, RightTab::Files]),
            [RightTab::Graph, RightTab::Files, RightTab::Changes]
        );
        // A reordered strip keeps its order; Terminal is never saved.
        assert_eq!(
            RightTab::normalize(&[
                RightTab::Changes,
                RightTab::Terminal,
                RightTab::Graph,
                RightTab::Files
            ]),
            [RightTab::Changes, RightTab::Graph, RightTab::Files]
        );
        assert_eq!(format_integer(1234567), "1,234,567");
        assert_eq!(format_integer(12), "12");
    }

    #[test]
    fn changes_tab_shows_only_nonzero_counts() {
        use gespan_core::scm::DiffStats;
        let stats = |additions, deletions| DiffStats {
            files: 1,
            additions,
            deletions,
        };
        assert!(changes_stat(None).is_empty());
        assert_eq!(changes_tab_label(None), "Changes");
        assert_eq!(changes_tab_label(Some(&stats(0, 0))), "Changes");
        assert_eq!(
            changes_stat(Some(&stats(5, 0))),
            [("+5".to_string(), DIFF_ADD)]
        );
        assert_eq!(changes_tab_label(Some(&stats(5, 0))), "Changes +5");
        assert_eq!(
            changes_stat(Some(&stats(0, 3))),
            [("-3".to_string(), DIFF_DEL)]
        );
        assert_eq!(
            changes_tab_label(Some(&stats(1234, 56))),
            "Changes +1,234 -56"
        );
        // Negative counts from a bad parse never show as "+-1".
        assert!(changes_stat(Some(&stats(-1, -1))).is_empty());
    }
}
