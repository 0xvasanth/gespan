//! Project colour on the title strip: every project tab leads with its
//! project's icon in the project's colour, the same mark the switcher draws,
//! and the active or hovered tab names the project in that colour. App tabs
//! have no project and stay neutral.
use super::*;

/// The project mark's side on a title tab.
pub(super) const TAB_PROJECT_MARK: f32 = 14.;
/// Between the mark and what follows it.
pub(super) const TAB_PROJECT_GAP: f32 = 6.;
/// The project name on an active or hovered tab is at most this wide.
const TAB_PROJECT_NAME_MAX: f32 = 84.;
/// Tabs narrower than this keep their title and leave out the project name.
pub(super) const TAB_PROJECT_NAME_MIN_WIDTH: f32 = 150.;

impl Shell {
    /// The colour `tab` is marked with: its project's, or none for an App
    /// tab.
    #[cfg(test)]
    pub(super) fn tab_color(&self, tab: &crate::workspace::TitleTab, cx: &App) -> Option<u32> {
        let project = self.tab_project(tab, cx)?;
        Some(self.appearance(&project).color(&project))
    }

    /// `project`'s icon in its colour, as the switcher draws it. `key` names
    /// the tab; the selector carries the colour drawn.
    pub(super) fn tab_project_mark(&self, project: &str, key: String) -> AnyElement {
        let appearance = self.appearance(project);
        let color = appearance.color(project);
        div()
            .debug_selector(move || format!("title-tab-project-{key}-{color:06x}"))
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(crate::project_settings::project_icon(
                &appearance,
                project,
                TAB_PROJECT_MARK,
            ))
            .into_any_element()
    }

    /// `project`'s name in its colour, for the active or hovered tab, so the
    /// project reads in words as well as colour and shape.
    pub(super) fn tab_project_name(&self, project: &str, key: String) -> AnyElement {
        let appearance = self.appearance(project);
        let color = appearance.color(project);
        div()
            .debug_selector(move || format!("title-tab-project-name-{key}"))
            .flex_shrink_0()
            .max_w(px(TAB_PROJECT_NAME_MAX))
            .truncate()
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(color))
            .child(appearance.label(project))
            .into_any_element()
    }
}

/// How long the pointer rests on a tab before its tooltip shows.
pub(super) const TAB_TIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
/// The tooltip's gap below the tab's plate, as the design draws it.
const TAB_TIP_GAP: f32 = 6.;

/// What a tab's chat is doing, as its tooltip says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabStatus {
    NeedsYou,
    Working,
    Idle,
}

impl TabStatus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs you",
            Self::Working => "working",
            Self::Idle => "idle",
        }
    }
}

/// A title tab's tooltip, as the design draws it: one row under the tab's
/// left edge with the project's mark and name in its colour, then, muted,
/// "· branch · status".
#[derive(Clone)]
pub(super) struct TabTooltip {
    id: i64,
    pub(super) project: Option<(SharedString, u32)>,
    mark: Option<(gespan_core::project_appearance::ProjectAppearance, String)>,
    pub(super) branch: Option<String>,
    pub(super) status: TabStatus,
}

impl TabTooltip {
    #[cfg(test)]
    pub(super) fn for_test(branch: Option<&str>, status: TabStatus) -> Self {
        Self {
            id: 0,
            project: None,
            mark: None,
            branch: branch.map(str::to_owned),
            status,
        }
    }

    /// The muted part after the project: "· main · working".
    pub(super) fn detail(&self) -> String {
        self.branch
            .iter()
            .map(String::as_str)
            .chain([self.status.label()])
            .fold(String::new(), |line, part| format!("{line}· {part} "))
            .trim_end()
            .to_owned()
    }

    fn card(&self) -> AnyElement {
        let id = self.id;
        let light = crate::theme::is_light();
        let (fill, edge, ink, shade) = if light {
            (0xfffffffa, 0x0000001a, 0x55555c, 0.12)
        } else {
            (0x1c1b20f5, 0xffffff1f, 0xa3a3a3, 0.4)
        };
        div()
            .debug_selector(move || format!("title-tab-tooltip-{id}"))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(8.))
            .bg(gpui::rgba(fill))
            .border_1()
            .border_color(gpui::rgba(edge))
            .shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0., 0., 0., shade),
                offset: gpui::point(px(0.), px(8.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
                inset: false,
            }])
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.5))
            .text_color(rgb(ink))
            .whitespace_nowrap()
            .when_some(self.mark.clone(), |d, (appearance, path)| {
                d.child(crate::project_settings::project_icon(
                    &appearance,
                    &path,
                    12.,
                ))
            })
            .when_some(self.project.clone(), |d, (name, color)| {
                d.child(
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(color))
                        .child(name),
                )
            })
            .child(self.detail())
            .into_any_element()
    }

    /// The card, lifted out of the strip and hung just under the tab's
    /// plate, from its left edge.
    pub(super) fn under_tab(&self) -> AnyElement {
        div()
            .absolute()
            .left_0()
            .top(px((TITLE_BAR_HEIGHT + TAB_PLATE_HEIGHT) / 2. + TAB_TIP_GAP))
            .child(
                gpui::deferred(gpui::anchored().snap_to_window().child(self.card()))
                    .with_priority(20),
            )
            .into_any_element()
    }
}

impl Shell {
    /// Session tab `id`'s tooltip.
    pub(super) fn tab_tooltip(&self, id: i64, project: Option<&str>, cx: &App) -> TabTooltip {
        let mark = project.map(|p| (self.appearance(p), p.to_owned()));
        let branch = project
            .and_then(|p| crate::git_cache::worktree_branch(p, self.session_working_copy(id, cx)));
        let project = project.map(|p| {
            let appearance = self.appearance(p);
            (appearance.label(p).into(), appearance.color(p))
        });
        TabTooltip {
            id,
            project,
            mark,
            branch,
            status: self.tab_status(id, cx),
        }
    }

    /// Whether tab `id` waits on the user, works or rests, as the Agents
    /// panel decides it for its chats.
    pub(super) fn tab_status(&self, id: i64, cx: &App) -> TabStatus {
        let waits = self
            .workspace
            .layout(id)
            .leaf_ids()
            .into_iter()
            .any(|pane| {
                self.sessions
                    .get(&pane)
                    .is_some_and(|v| v.read(cx).needs_approval())
            });
        if waits {
            TabStatus::NeedsYou
        } else if self.background_tab_running(id, cx) {
            TabStatus::Working
        } else {
            TabStatus::Idle
        }
    }

    /// Starts tab `id`'s tooltip wait as the pointer comes onto it.
    pub(super) fn arm_tab_tip(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.tab_tip.is_some_and(|(tip, _)| tip == id) {
            return;
        }
        self.tab_tip = Some((id, false));
        self.tab_tip_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TAB_TIP_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.tab_tip == Some((id, false)) {
                    this.tab_tip = Some((id, true));
                    this.tabs_frame.update(cx, |_, cx| cx.notify());
                }
            });
        }));
    }

    /// Takes tab `id`'s tooltip down as the pointer leaves it.
    pub(super) fn disarm_tab_tip(&mut self, id: i64) {
        if self.tab_tip.is_some_and(|(tip, _)| tip == id) {
            self.tab_tip = None;
            self.tab_tip_task = None;
        }
    }

    /// Whether tab `id`'s tooltip shows: the pointer rested on it and no
    /// tab is being dragged.
    pub(super) fn tab_tip_shown(&self, id: i64) -> bool {
        self.tab_tip == Some((id, true)) && !self.tab_drag.as_ref().is_some_and(|d| d.active)
    }
}
