//! Project groups in the title strip: each project's tabs sit behind a chip
//! in the project's colour. The current project's group is open; every other
//! one shows only its chip, with a tab count and a working spinner, until the
//! pointer rests on the chip and the group peeks open in place.
use super::*;
use gespan_core::files::path_key;
use gpui::Hsla;

/// How long the pointer rests on a collapsed chip before its group peeks.
pub(super) const PEEK_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// A group's padding plus its hairline, on each side.
const GROUP_INSET: f32 = 4.;
const GROUP_PLATE: f32 = TAB_PLATE_HEIGHT + 2. * GROUP_INSET - 2.;
const CHIP_PAD: f32 = 9.;
const CHIP_GAP: f32 = 6.;
const CHIP_DOT: f32 = 8.;
const CHIP_TEXT: f32 = 12.;
const CHIP_LABEL_MAX: f32 = 140.;
const COUNT_TEXT: f32 = 11.;
const COUNT_PAD: f32 = 5.;
const SPINNER: f32 = 12.;

/// One of the strip's direct children: a loose tab, or a project's group box.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum StripChild {
    Tab(TitleTab),
    Group(String),
}

/// Each group's chip as a frame draws it, by project.
#[derive(Default)]
pub(super) struct GroupChips {
    /// Chip widths as drawn.
    pub(super) widths: HashMap<String, f32>,
    /// Chip widths with every group but the current one collapsed, as if
    /// nothing peeked.
    pub(super) resting: HashMap<String, f32>,
    /// Other projects' groups with a chat at work.
    pub(super) running: HashSet<String>,
}

/// Bounds of the tabs drawn inside groups, as last laid out.
pub(super) type GroupTabBounds = std::rc::Rc<std::cell::RefCell<HashMap<TitleTab, Bounds<Pixels>>>>;

fn text_width(text: &str, size: f32, weight: gpui::FontWeight, window: &Window) -> f32 {
    let mut font = window.text_style().font();
    font.weight = weight;
    let run = gpui::TextRun {
        len: text.len(),
        font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_owned().into(), px(size), &[run], None)
        .width
        .as_f32()
        .ceil()
}

fn tint(color: u32, alpha: f32) -> Hsla {
    Hsla {
        a: alpha,
        ..rgb(color).into()
    }
}

impl Shell {
    /// Whether `project`'s group shows its tabs: the current project's
    /// always, another project's while it peeks.
    pub(super) fn group_expanded(&self, project: &str) -> bool {
        path_key(project) == path_key(&self.workspace.project)
            || self.strip_peek.as_deref() == Some(project)
    }

    /// The tabs the strip draws as plates, in strip order: loose tabs and
    /// the tabs of open groups.
    pub(super) fn drawn_strip_tabs(&self, entries: &[StripEntry<TitleTab>]) -> Vec<TitleTab> {
        entries
            .iter()
            .flat_map(|entry| match entry {
                StripEntry::Loose(tab) => vec![tab.clone()],
                StripEntry::Group { project, tabs } if self.group_expanded(project) => tabs.clone(),
                StripEntry::Group { .. } => Vec::new(),
            })
            .collect()
    }

    /// Whether any pane of any chat tab in `tabs` is working.
    pub(super) fn group_running(&self, tabs: &[TitleTab], cx: &App) -> bool {
        tabs.iter().filter_map(TitleTab::session).any(|tab| {
            self.workspace
                .layout(tab)
                .leaf_ids()
                .into_iter()
                .filter(|pane| self.session_summaries.contains_key(pane))
                .any(|pane| self.session_metadata(pane, cx).busy)
        })
    }

    /// Each group's chip, measured once a frame. A chip is sized to its
    /// name (up to a cap), plus the count and spinner it shows while
    /// collapsed.
    pub(super) fn group_chip_widths(
        &self,
        entries: &[StripEntry<TitleTab>],
        window: &Window,
        cx: &App,
    ) -> GroupChips {
        let mut chips = GroupChips::default();
        for entry in entries {
            let StripEntry::Group { project, tabs } = entry else {
                continue;
            };
            let label = self.appearance(project).label(project);
            let open = 2. * CHIP_PAD
                + CHIP_DOT
                + CHIP_GAP
                + text_width(&label, CHIP_TEXT, gpui::FontWeight::SEMIBOLD, window)
                    .min(CHIP_LABEL_MAX);
            let current = path_key(project) == path_key(&self.workspace.project);
            // Only another project's group ever collapses.
            let collapsed = if current {
                open
            } else {
                let running = self.group_running(tabs, cx);
                if running {
                    chips.running.insert(project.clone());
                }
                open + CHIP_GAP
                    + 2. * COUNT_PAD
                    + text_width(
                        &tabs.len().to_string(),
                        COUNT_TEXT,
                        gpui::FontWeight::MEDIUM,
                        window,
                    )
                    + if running { CHIP_GAP + SPINNER } else { 0. }
            };
            // A peeking chip keeps its collapsed size, so it never shrinks
            // out from under the pointer that opened it.
            chips.widths.insert(project.clone(), collapsed.ceil());
            chips.resting.insert(project.clone(), collapsed.ceil());
        }
        chips
    }

    /// The strip room groups take beyond their tabs: chips, insets and the
    /// gap after each group.
    pub(super) fn group_room(chips: &HashMap<String, f32>) -> f32 {
        chips.values().map(|w| w + 2. * GROUP_INSET + 2.).sum()
    }

    /// A group's chip: the project's colour dot and name; collapsed, its tab
    /// count and, while any of its chats works (`running`), the spinner.
    /// Resting on a collapsed chip peeks its group open.
    pub(super) fn group_chip(
        &self,
        project: &str,
        tabs: &[TitleTab],
        (width, running): (f32, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.appearance(project);
        let label = appearance.label(project);
        let label_for_close = label.clone();
        let color = appearance.color(project);
        let name = crate::workspace::project_name(project);
        let current = path_key(project) == path_key(&self.workspace.project);
        // Another project's chip looks the same peeking or not: only the
        // tabs after it open.
        let collapsed = !current;
        let running = collapsed && running;
        let (peek, left, open, close, drag) = (
            project.to_owned(),
            project.to_owned(),
            project.to_owned(),
            project.to_owned(),
            project.to_owned(),
        );
        let hover_group = SharedString::from(format!("tab-group-chip-{project}"));
        let (count_name, running_name, close_name, chip_name) =
            (name.clone(), name.clone(), name.clone(), name);
        div()
            .id(SharedString::from(format!("tab-group-chip-{project}")))
            .group(hover_group.clone())
            .relative()
            .debug_selector(move || format!("tab-group-chip-{chip_name}"))
            .role(Role::Button)
            .aria_label(format!("{label}, {} tabs", tabs.len()))
            .cursor_pointer()
            .flex_shrink_0()
            .w(px(width))
            .h(px(TAB_PLATE_HEIGHT))
            .px(px(CHIP_PAD))
            .rounded(px(7.))
            .bg(tint(color, 0.28))
            .flex()
            .items_center()
            .gap(px(CHIP_GAP))
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(CHIP_DOT))
                    .rounded_full()
                    .bg(rgb(color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(CHIP_TEXT))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(content(0.95))
                    .truncate()
                    // The close button takes the label's tail on hover.
                    .when(!collapsed, |d| {
                        d.group_hover(hover_group.clone(), |s| s.pr(px(16.)))
                    })
                    .child(label),
            )
            .when(collapsed, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("tab-group-count-{count_name}"))
                        .flex_shrink_0()
                        .px(px(COUNT_PAD))
                        .rounded(px(8.))
                        .bg(gpui::black().opacity(0.25))
                        .text_size(px(COUNT_TEXT))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(content(0.75))
                        .group_hover(hover_group.clone(), |s| s.opacity(0.))
                        .child(tabs.len().to_string()),
                )
            })
            .when(running, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("tab-group-running-{running_name}"))
                        .flex_shrink_0()
                        .size(px(SPINNER))
                        .child(crate::session_mark::spinner(
                            SharedString::from(format!("tab-group-spinner-{project}")),
                            crate::pulse::Rate::Half,
                        )),
                )
            })
            .child(
                div()
                    .id(SharedString::from(format!("close-tab-group-{project}")))
                    .debug_selector(move || format!("close-tab-group-{close_name}"))
                    .role(Role::Button)
                    .aria_label(format!("Close {label_for_close} tabs"))
                    .absolute()
                    .right(px(4.))
                    .top(px((TAB_PLATE_HEIGHT - 20.) / 2.))
                    .size(px(20.))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.)
                    .group_hover(hover_group, |s| s.opacity(1.))
                    .hover(|s| s.bg(content(0.1)))
                    .child(icon("close", 12.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_group(&close, window, cx);
                    })),
            )
            // Only real pointer motion peeks: a chip sliding under a still
            // pointer as the strip lays out again does not. It peeks once the
            // pointer rests, so passing over a chip leaves the strip still.
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if !current {
                    this.arm_peek(&peek, cx);
                }
            }))
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                if !*hovered {
                    this.disarm_peek(&left);
                }
            }))
            // Pressed, the chip drags its whole group along the strip.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| {
                    this.start_strip_drag(StripChild::Group(drag.clone()), event, cx);
                }),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if std::mem::take(&mut this.skip_tab_click) {
                    return;
                }
                this.strip_peek = None;
                this.peek_pending = None;
                if !current {
                    this.select_project(open.clone(), window, cx);
                }
                this.tabs_frame.update(cx, |_, cx| cx.notify());
            }))
            .into_any_element()
    }

    /// One project's group: its chip, then its tabs when open, over a wash of
    /// the project's colour (solid hairline open, dashed while peeking).
    /// Leaving its box ends its peek. `children` are the tab slots drawn in
    /// it, keyed so drags can find where each tab is.
    fn group_box(
        &self,
        project: &str,
        tabs: &[TitleTab],
        children: Vec<(TitleTab, AnyElement)>,
        (chip, tabs_width): ((f32, bool), f32),
        (shift, dragged): (f32, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = path_key(project) == path_key(&self.workspace.project);
        let expanded = self.group_expanded(project);
        let peeking = expanded && !current;
        let color = self.appearance(project).color(project);
        let name = crate::workspace::project_name(project);
        let peek = project.to_owned();
        // The wash and the chip lead the children, so the tabs start at 2.
        let keys: Vec<Option<TitleTab>> = [None, None]
            .into_iter()
            .chain(children.iter().map(|(key, _)| Some(key.clone())))
            .collect();
        let store = self.group_tab_bounds.clone();
        let group = div()
            .on_children_prepainted(move |bounds, _, _| {
                let mut store = store.borrow_mut();
                for (key, bounds) in keys.iter().zip(bounds) {
                    if let Some(key) = key {
                        store.insert(key.clone(), bounds);
                    }
                }
            })
            .id(SharedString::from(format!("tab-group-{project}")))
            .debug_selector(move || format!("tab-group-{name}"))
            .role(Role::Group)
            .flex_shrink_0()
            // Sized from its slots rather than measured: a slot easing in
            // or out is clipped, and layout would size the box as if it
            // took no room, so the box slipped out from under the pointer
            // and the strip after it jumped.
            .w(px(chip.0 + 2. * GROUP_INSET + tabs_width))
            .h(px(TITLE_BAR_HEIGHT))
            .relative()
            .left(px(shift))
            .px(px(GROUP_INSET))
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((TITLE_BAR_HEIGHT - GROUP_PLATE) / 2. - 1.))
                    .w_full()
                    .h(px(GROUP_PLATE + 2.))
                    .rounded(px(9.))
                    .border_1()
                    .map(|d| {
                        if current {
                            d.bg(tint(color, 0.12)).border_color(tint(color, 0.45))
                        } else if peeking {
                            d.bg(tint(color, 0.08))
                                .border_color(tint(color, 0.55))
                                .border_dashed()
                        } else {
                            d.border_color(gpui::transparent_black())
                        }
                    }),
            )
            .child(self.group_chip(project, tabs, chip, cx))
            .children(children.into_iter().map(|(_, element)| element))
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                // A drag holds the strip still; its release settles the peek.
                if !*hovered && this.tab_drag.is_none() && this.strip_peek.as_ref() == Some(&peek) {
                    // Left to the right, what is under the pointer holds
                    // still while this group eases shut.
                    let pointer = window.mouse_position();
                    if this
                        .group_box_bounds(&peek)
                        .is_some_and(|b| pointer.x >= b.right())
                    {
                        this.hold_strip_as_closing(&peek);
                    }
                    this.strip_peek = None;
                    this.tabs_frame.update(cx, |_, cx| cx.notify());
                }
            }));
        // A group dragged by its chip draws over the boxes it passes.
        if dragged {
            gpui::deferred(group).with_priority(1).into_any_element()
        } else {
            group.into_any_element()
        }
    }

    /// Lays the strip's tab slots out by `entries`: loose tabs in place, each
    /// group as one box. A slot easing out goes back to its project's group
    /// when it has one, else beside the slot before it. Returns the strip's
    /// direct children, each loose tab keyed by its tab. Each box sits
    /// `shifts` from its slot while a chip drag passes it.
    pub(super) fn grouped_strip(
        &self,
        entries: &[StripEntry<TitleTab>],
        slots: Vec<(TitleTab, AnyElement)>,
        (chips, group_tabs): (&GroupChips, &HashMap<String, f32>),
        (shifts, dragged): (&HashMap<String, f32>, Option<&str>),
        cx: &mut Context<Self>,
    ) -> Vec<(StripChild, AnyElement)> {
        let mut home: HashMap<TitleTab, usize> = HashMap::new();
        let mut groups: HashMap<&str, usize> = HashMap::new();
        for (ix, entry) in entries.iter().enumerate() {
            match entry {
                StripEntry::Loose(tab) => {
                    home.insert(tab.clone(), ix);
                }
                StripEntry::Group { project, tabs } => {
                    groups.insert(project, ix);
                    for tab in tabs {
                        home.insert(tab.clone(), ix);
                    }
                }
            }
        }
        let mut buckets: Vec<Vec<(TitleTab, AnyElement)>> =
            entries.iter().map(|_| Vec::new()).collect();
        let mut stray = Vec::new();
        let mut last = None;
        for (tab, element) in slots {
            let at = home.get(&tab).copied().or_else(|| {
                self.tab_project(&tab, cx)
                    .and_then(|project| groups.get(project.as_str()).copied())
                    .or(last)
            });
            match at {
                Some(ix) => {
                    buckets[ix].push((tab, element));
                    last = Some(ix);
                }
                None => stray.push((StripChild::Tab(tab), element)),
            }
        }
        self.group_tab_bounds.borrow_mut().clear();
        let mut out = stray;
        for (entry, bucket) in entries.iter().zip(buckets) {
            match entry {
                StripEntry::Loose(_) => {
                    out.extend(bucket.into_iter().map(|(tab, e)| (StripChild::Tab(tab), e)));
                }
                StripEntry::Group { project, tabs } => {
                    let width = chips.widths.get(project).copied().unwrap_or(0.);
                    let chip = (width, chips.running.contains(project));
                    let tabs_width = group_tabs.get(project).copied().unwrap_or(0.);
                    let shift = shifts.get(project).copied().unwrap_or(0.);
                    let motion = (shift, dragged == Some(project.as_str()));
                    out.push((
                        StripChild::Group(project.clone()),
                        self.group_box(project, tabs, bucket, (chip, tabs_width), motion, cx),
                    ));
                }
            }
        }
        out
    }

    /// Peeks `project`'s group once the pointer has rested on its chip for
    /// [`PEEK_DELAY`]. Moving on within the chip keeps the same wait. From
    /// a group that was just peeking it peeks at once, so the strip still
    /// holds the chip under the pointer as the other one closes.
    pub(super) fn arm_peek(&mut self, project: &str, cx: &mut Context<Self>) {
        if self.strip_peek.as_deref() == Some(project)
            || self.tab_drag.is_some()
            || self
                .peek_pending
                .as_ref()
                .is_some_and(|(p, _)| p == project)
        {
            return;
        }
        if self.drawn_peek.as_deref().is_some_and(|p| p != project) {
            self.peek_pending = None;
            self.peek_group(project, cx);
            return;
        }
        let armed = project.to_owned();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PEEK_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.peek_pending.as_ref().is_some_and(|(p, _)| *p == armed) {
                    this.peek_pending = None;
                    this.peek_group(&armed, cx);
                }
            });
        });
        self.peek_pending = Some((project.to_owned(), task));
    }

    /// The pointer left `project`'s chip before it peeked.
    pub(super) fn disarm_peek(&mut self, project: &str) {
        if self
            .peek_pending
            .as_ref()
            .is_some_and(|(p, _)| p == project)
        {
            self.peek_pending = None;
        }
    }

    /// Opens `project`'s group in place while the pointer rests on its
    /// chip. A group left of it that was peeking closes; the next frame
    /// scrolls the strip back by what it gave up, so the chip stays under
    /// the pointer when the strip can scroll that far.
    pub(super) fn peek_group(&mut self, project: &str, cx: &mut Context<Self>) {
        // A drag measured the strip when it began, so nothing opens under it.
        if self.strip_peek.as_deref() == Some(project) || self.tab_drag.is_some() {
            return;
        }
        let place = |p: &str| {
            self.strip_children
                .iter()
                .position(|child| matches!(child, StripChild::Group(g) if g == p))
        };
        let at = place(project);
        if let Some(left) = self
            .drawn_peek
            .clone()
            .filter(|p| matches!((place(p), at), (Some(p), Some(at)) if p < at))
        {
            self.hold_strip_as_closing(&left);
        }
        self.strip_peek = Some(project.to_owned());
        self.tabs_frame.update(cx, |_, cx| cx.notify());
    }

    /// `project`'s group is closing left of the pointer: from now on the
    /// strip scrolls back by whatever its tabs give up, frame by frame.
    fn hold_strip_as_closing(&mut self, project: &str) {
        if let Some(&width) = self.group_tabs_width.get(project).filter(|w| **w > 0.) {
            self.peek_anchor = Some((project.to_owned(), width));
        }
    }

    /// Scrolls the strip right by what the closing group's tabs gave up
    /// since the last frame, clamped to the scroll range, so the strip to
    /// its right holds still as it eases shut. `widths` are this frame's
    /// tab widths of each group drawn open.
    pub(super) fn hold_strip_still(&mut self, widths: HashMap<String, f32>) {
        if let Some((project, last)) = self.peek_anchor.take() {
            let now = widths.get(&project).copied().unwrap_or(0.);
            let given_up = last - now;
            if given_up > 0. {
                let offset = self.tab_scroll.offset();
                let x = (offset.x + px(given_up)).min(px(0.));
                self.tab_scroll.set_offset(gpui::point(x, offset.y));
            }
            if now > 0. && self.strip_peek.as_deref() != Some(project.as_str()) {
                self.peek_anchor = Some((project, now));
            }
        }
        self.group_tabs_width = widths;
    }

    /// Closes every tab in `project`'s group, pins aside, by the usual
    /// closing rules. When the active tab goes with them, the nearest tab
    /// to the right of the group opens, else the one to its left.
    pub(super) fn close_group(
        &mut self,
        project: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let strip = self.global_strip_tabs(cx);
        let inside = |tab: &TitleTab| {
            self.tab_project(tab, cx)
                .is_some_and(|p| path_key(&p) == path_key(project))
        };
        let closing: Vec<TitleTab> = strip.iter().filter(|tab| inside(tab)).cloned().collect();
        let (Some(first), Some(last)) = (
            strip.iter().position(&inside),
            strip.iter().rposition(&inside),
        ) else {
            return;
        };
        let next = strip[last + 1..]
            .iter()
            .chain(strip[..first].iter().rev())
            .find(|tab| !inside(tab))
            .cloned();
        if self.strip_peek.as_deref() == Some(project) {
            self.strip_peek = None;
        }
        self.close_title_tabs(&closing, next, window, cx);
    }

    /// Ends a peek held through a drag once the pointer is outside its box.
    pub(super) fn settle_peek(&mut self, pointer: Point<Pixels>) {
        let Some(project) = self.strip_peek.clone() else {
            return;
        };
        if !self
            .group_box_bounds(&project)
            .is_some_and(|b| b.contains(&pointer))
        {
            self.strip_peek = None;
        }
    }

    /// Where `project`'s group box was last drawn, after the strip's scroll.
    pub(super) fn group_box_bounds(&self, project: &str) -> Option<Bounds<Pixels>> {
        let ix = self
            .strip_children
            .iter()
            .position(|child| matches!(child, StripChild::Group(p) if p == project))?;
        self.drawn_bounds(ix)
    }

    /// A direct child's bounds as drawn, after the strip's scroll.
    fn drawn_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let offset = self.tab_scroll.offset();
        self.tab_scroll.bounds_for_item(ix).map(|b| Bounds {
            origin: gpui::point(b.origin.x + offset.x, b.origin.y),
            size: b.size,
        })
    }

    /// Where `tab` was last drawn in the strip, loose or inside a group.
    pub(super) fn strip_tab_bounds(&self, tab: &TitleTab) -> Option<Bounds<Pixels>> {
        match self
            .strip_children
            .iter()
            .position(|child| *child == StripChild::Tab(tab.clone()))
        {
            Some(ix) => self.drawn_bounds(ix),
            None => self.group_tab_bounds.borrow().get(tab).copied(),
        }
    }
}
