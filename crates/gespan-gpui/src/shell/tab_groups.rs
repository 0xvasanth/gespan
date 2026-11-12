//! Project groups in the title strip: each project's tabs sit behind a chip
//! in the project's colour. The current project's group is open; every other
//! one shows only its chip, with a tab count and a working spinner, until the
//! pointer rests on the chip and the group peeks open in place.
use super::*;
use crate::strip_groups::StripEntry;
use gespan_core::files::path_key;
use gpui::Hsla;

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

/// The small branch label a tab in a worktree carries after its title.
pub(super) fn worktree_badge(id: i64, label: String) -> AnyElement {
    div()
        .debug_selector(move || format!("title-tab-worktree-{id}"))
        .flex_shrink_0()
        .max_w(px(96.))
        .px(px(5.))
        .rounded(px(4.))
        .bg(content(0.08))
        .text_size(px(10.))
        .line_height(px(16.))
        .text_color(content(0.65))
        .truncate()
        .child(label)
        .into_any_element()
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

    /// Each group's chip width, by project. A chip is sized to its name
    /// (up to a cap), plus the count and spinner it shows while collapsed.
    pub(super) fn group_chip_widths(
        &self,
        entries: &[StripEntry<TitleTab>],
        window: &Window,
        cx: &App,
    ) -> HashMap<String, f32> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                StripEntry::Group { project, tabs } => {
                    let label = self.appearance(project).label(project);
                    let mut width = 2. * CHIP_PAD
                        + CHIP_DOT
                        + CHIP_GAP
                        + text_width(&label, CHIP_TEXT, gpui::FontWeight::SEMIBOLD, window)
                            .min(CHIP_LABEL_MAX);
                    if !self.group_expanded(project) {
                        width += CHIP_GAP
                            + 2. * COUNT_PAD
                            + text_width(
                                &tabs.len().to_string(),
                                COUNT_TEXT,
                                gpui::FontWeight::MEDIUM,
                                window,
                            );
                        if self.group_running(tabs, cx) {
                            width += CHIP_GAP + SPINNER;
                        }
                    }
                    Some((project.clone(), width.ceil()))
                }
                StripEntry::Loose(_) => None,
            })
            .collect()
    }

    /// The strip room groups take beyond their tabs: chips, insets and the
    /// gap after each group.
    pub(super) fn group_room(chips: &HashMap<String, f32>) -> f32 {
        chips.values().map(|w| w + 2. * GROUP_INSET + 2.).sum()
    }

    /// A group's chip: the project's colour dot and name; collapsed, its tab
    /// count and, while any of its chats works, the spinner. Resting on a
    /// collapsed chip peeks its group open.
    pub(super) fn group_chip(
        &self,
        project: &str,
        tabs: &[TitleTab],
        expanded: bool,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.appearance(project);
        let label = appearance.label(project);
        let color = appearance.color(project);
        let name = crate::workspace::project_name(project);
        let running = !expanded && self.group_running(tabs, cx);
        let current = path_key(project) == path_key(&self.workspace.project);
        let peek = project.to_owned();
        let (count_name, running_name, chip_name) = (name.clone(), name.clone(), name);
        div()
            .id(SharedString::from(format!("tab-group-chip-{project}")))
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
                    .child(label),
            )
            .when(!expanded, |d| {
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
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && !current && this.strip_peek.as_ref() != Some(&peek) {
                    this.strip_peek = Some(peek.clone());
                    this.tabs_frame.update(cx, |_, cx| cx.notify());
                }
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
        chip_width: f32,
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
        div()
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
            .h(px(TITLE_BAR_HEIGHT))
            .relative()
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
            .child(self.group_chip(project, tabs, expanded, chip_width, cx))
            .children(children.into_iter().map(|(_, element)| element))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if !*hovered && this.strip_peek.as_ref() == Some(&peek) {
                    this.strip_peek = None;
                    this.tabs_frame.update(cx, |_, cx| cx.notify());
                }
            }))
            .into_any_element()
    }

    /// Lays the strip's tab slots out by `entries`: loose tabs in place, each
    /// group as one box. A slot easing out goes back to its project's group
    /// when it has one, else beside the slot before it. Returns the strip's
    /// direct children, each loose tab keyed by its tab.
    pub(super) fn grouped_strip(
        &self,
        entries: &[StripEntry<TitleTab>],
        slots: Vec<(TitleTab, AnyElement)>,
        chips: &HashMap<String, f32>,
        cx: &mut Context<Self>,
    ) -> Vec<(Option<TitleTab>, AnyElement)> {
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
                None => stray.push((Some(tab), element)),
            }
        }
        self.group_tab_bounds.borrow_mut().clear();
        let mut out = stray;
        for (entry, bucket) in entries.iter().zip(buckets) {
            match entry {
                StripEntry::Loose(_) => {
                    out.extend(bucket.into_iter().map(|(tab, e)| (Some(tab), e)));
                }
                StripEntry::Group { project, tabs } => {
                    let width = chips.get(project).copied().unwrap_or(0.);
                    out.push((None, self.group_box(project, tabs, bucket, width, cx)));
                }
            }
        }
        out
    }

    /// Where `tab` was last drawn in the strip, loose or inside a group.
    pub(super) fn strip_tab_bounds(&self, tab: &TitleTab) -> Option<Bounds<Pixels>> {
        match self
            .strip_children
            .iter()
            .position(|key| key.as_ref() == Some(tab))
        {
            Some(ix) => self.tab_scroll.bounds_for_item(ix),
            None => self.group_tab_bounds.borrow().get(tab).copied(),
        }
    }
}
