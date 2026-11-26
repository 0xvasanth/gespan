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
