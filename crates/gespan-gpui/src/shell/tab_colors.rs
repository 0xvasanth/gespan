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

/// A title tab's tooltip, as the design draws it: one row with the
/// project's mark and name in its colour, then, muted, the tab's whole title
/// (which a squeezed tab hides), its branch and whether it is working.
#[derive(Clone)]
pub(super) struct TabTooltip {
    id: i64,
    pub(super) title: SharedString,
    pub(super) project: Option<(SharedString, u32)>,
    mark: Option<(gespan_core::project_appearance::ProjectAppearance, String)>,
    pub(super) detail: SharedString,
}

impl Render for TabTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let rest = std::iter::once(self.title.to_string())
            .chain((!self.detail.is_empty()).then(|| self.detail.to_string()))
            .collect::<Vec<_>>()
            .join(" · ");
        div()
            .debug_selector(move || format!("title-tab-tooltip-{id}"))
            .max_w(px(420.))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(8.))
            .bg(gpui::rgba(0x1c1b20f5))
            .border_1()
            .border_color(gpui::rgba(0xffffff1f))
            .shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0., 0., 0., 0.4),
                offset: gpui::point(px(0.), px(8.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
                inset: false,
            }])
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.5))
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
                        .flex_shrink_0()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(color))
                        .child(name),
                )
                .child(div().flex_shrink_0().text_color(rgb(0xa3a3a3)).child("·"))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(0xa3a3a3))
                    .child(rest),
            )
    }
}

impl Shell {
    /// Session tab `id`'s tooltip, titled `title`.
    pub(super) fn tab_tooltip(
        &self,
        id: i64,
        title: SharedString,
        project: Option<&str>,
        cx: &App,
    ) -> TabTooltip {
        let mark = project.map(|p| (self.appearance(p), p.to_owned()));
        let project = project.map(|p| {
            let appearance = self.appearance(p);
            (appearance.label(p).into(), appearance.color(p))
        });
        let branch = project.as_ref().and(
            self.tab_project(&crate::workspace::TitleTab::Session(id), cx)
                .and_then(|p| {
                    crate::git_cache::worktree_branch(&p, self.session_working_copy(id, cx))
                }),
        );
        let detail = branch
            .into_iter()
            .chain(
                self.background_tab_running(id, cx)
                    .then(|| "working".to_string()),
            )
            .collect::<Vec<_>>()
            .join(" · ");
        TabTooltip {
            id,
            title,
            project,
            mark,
            detail: detail.into(),
        }
    }
}
