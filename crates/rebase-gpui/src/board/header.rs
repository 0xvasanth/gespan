//! The board's header row: title, search, filter and New task, like gr8r's.
use super::{BoardView, menu_row};
use crate::{
    popover::{Anchor, popover},
    theme::*,
};
use gpui::{
    AnyElement, App, Context, Focusable, FontWeight, Role, SharedString, div, prelude::*, px,
};
use rebase_core::board::{Card, extras::PRIORITIES};

impl BoardView {
    /// The workspace projects a task can be tagged with, as (path, name).
    pub(crate) fn set_projects(&mut self, projects: Vec<(String, String)>, cx: &mut Context<Self>) {
        if self.projects != projects {
            self.projects = projects;
            cx.notify();
        }
    }

    pub(crate) fn project_name(&self, path: &str) -> String {
        self.projects
            .iter()
            .find(|(project, _)| project == path)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| {
                std::path::Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string())
            })
    }

    /// Projects a task can be tagged or filtered by: the workspace's, then any
    /// a task still carries after leaving the workspace, so it can be untagged.
    pub(crate) fn project_choices(&self) -> Vec<(String, String)> {
        let mut choices = self.projects.clone();
        let mut extra: Vec<(String, String)> = self
            .extras
            .values()
            .flat_map(|extras| extras.projects.iter())
            .filter(|path| !self.projects.iter().any(|(known, _)| known == *path))
            .map(|path| (path.clone(), self.project_name(path)))
            .collect();
        extra.sort();
        extra.dedup();
        choices.extend(extra);
        choices
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).content.trim().to_lowercase()
    }

    fn narrowing(&self, cx: &App) -> bool {
        !self.query(cx).is_empty() || self.filter.len() > 0
    }

    /// Whether a card passes the search and the filter.
    pub(crate) fn card_visible(&self, card: &Card, cx: &App) -> bool {
        let query = self.query(cx);
        let matches_query = if query.is_empty() {
            true
        } else if let Some(number) = query.strip_prefix('#') {
            card.number.to_string() == number
        } else {
            card.title.to_lowercase().contains(&query)
        };
        if !matches_query {
            return false;
        }
        let extras = self.extras.get(&card.id);
        let filter = &self.filter;
        let projects_ok = filter.projects.is_empty()
            || extras.is_some_and(|e| e.projects.iter().any(|p| filter.projects.contains(p)));
        let labels_ok = filter.labels.is_empty()
            || extras.is_some_and(|e| e.labels.iter().any(|l| filter.labels.contains(l)));
        let priority = extras.map_or(0, |e| e.priority);
        let priority_ok = filter.priorities.is_empty() || filter.priorities.contains(&priority);
        projects_ok && labels_ok && priority_ok
    }

    /// A column's count: `shown/total` while searching or filtering, else the total.
    pub(crate) fn count_label(&self, column_id: &str, cx: &App) -> String {
        let Some(detail) = &self.detail else {
            return "0".into();
        };
        let cards = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id);
        let total = cards.clone().count();
        if !self.narrowing(cx) {
            return total.to_string();
        }
        let shown = cards.filter(|card| self.card_visible(card, cx)).count();
        format!("{shown}/{total}")
    }

    fn new_task(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let first = self
            .detail
            .as_ref()
            .and_then(|detail| detail.columns.iter().min_by_key(|c| c.position))
            .map(|column| column.id.clone());
        if let Some(first) = first {
            self.open_composer(first, window, cx);
        }
    }

    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.filter.len();
        let search_focus = self.search.focus_handle(cx);
        div()
            .id("board-header")
            .debug_selector(|| "board-header".into())
            .h(px(PANE_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(content(0.07))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("kanban", 14.).text_color(content(0.55)))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Board"),
                    ),
            )
            .child(
                div()
                    .id("board-search")
                    .debug_selector(|| "board-search".into())
                    .w(px(220.))
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.08))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_text()
                    .child(icon("search", 12.).text_color(content(0.4)))
                    .child(div().flex_1().min_w_0().child(self.search.clone()))
                    .on_click(move |_, window, cx| window.focus(&search_focus, cx)),
            )
            .child(
                div()
                    .id("board-filter")
                    .debug_selector(|| "board-filter".into())
                    .relative()
                    .role(Role::Button)
                    .aria_label("Filter")
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(content(if active > 0 { 0.9 } else { 0.6 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .when(self.filter_open || active > 0, |d| d.bg(content(0.05)))
                    .child(self.filter_anchor.probe())
                    .child(icon("filter", 12.))
                    .child("Filter")
                    .when(active > 0, |d| {
                        d.child(
                            div()
                                .px(px(5.))
                                .rounded_full()
                                .bg(accent_color().opacity(0.2))
                                .text_color(accent_color())
                                .text_size(px(11.))
                                .child(active.to_string()),
                        )
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.filter_open = !this.filter_open;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("board-new-task")
                    .debug_selector(|| "board-new-task".into())
                    .role(Role::Button)
                    .aria_label("New task")
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .bg(accent_color())
                    .text_color(white(0.95))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child(icon("plus", 12.))
                    .child("New task")
                    .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx))),
            )
    }

    pub(super) fn render_filter(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.filter_open {
            return None;
        }
        let weak = cx.weak_entity();
        let section = |label: &'static str| {
            div()
                .px(px(8.))
                .pt(px(6.))
                .pb(px(2.))
                .text_size(px(11.))
                .text_color(content(0.45))
                .child(label)
                .into_any_element()
        };
        let check = |on: bool| {
            div()
                .size(px(12.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| {
                    d.child(icon("check", 12.).text_color(accent_color()))
                })
        };
        let mut rows: Vec<AnyElement> = vec![section("Projects")];
        let projects = self.project_choices();
        if projects.is_empty() {
            rows.push(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .text_size(px(12.))
                    .text_color(content(0.4))
                    .child("No projects")
                    .into_any_element(),
            );
        }
        for (index, (path, name)) in projects.iter().enumerate() {
            let on = self.filter.projects.contains(path);
            let path = path.clone();
            rows.push(
                menu_row(format!("board-filter-project-{index}").into(), name.clone())
                    .flex()
                    .gap_2()
                    .child(check(on))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.filter.projects.remove(&path) {
                            this.filter.projects.insert(path.clone());
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        if !self.labels.is_empty() {
            rows.push(section("Labels"));
        }
        for (index, label) in self.labels.iter().enumerate() {
            let on = self.filter.labels.contains(&label.id);
            let id = label.id.clone();
            rows.push(
                menu_row(
                    format!("board-filter-label-{index}").into(),
                    label.name.clone(),
                )
                .flex()
                .gap_2()
                .child(check(on))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.filter.labels.remove(&id) {
                        this.filter.labels.insert(id.clone());
                    }
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        rows.push(section("Priority"));
        for priority in (0..PRIORITIES.len() as u8).rev() {
            let on = self.filter.priorities.contains(&priority);
            rows.push(
                menu_row(
                    format!("board-filter-priority-{priority}").into(),
                    SharedString::from(PRIORITIES[priority as usize]),
                )
                .flex()
                .gap_2()
                .child(check(on))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.filter.priorities.remove(&priority) {
                        this.filter.priorities.insert(priority);
                    }
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        if self.filter.len() > 0 {
            rows.push(div().h(px(1.)).my_1().bg(content(0.07)).into_any_element());
            rows.push(
                menu_row("board-filter-clear".into(), "Clear filters")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter = Default::default();
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        Some(
            popover(
                "board-filter-menu",
                Anchor::Slot(self.filter_anchor.clone()),
            )
            .width(240.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.filter_open = false;
                    cx.notify();
                });
            })
            .child(
                div()
                    .id("board-filter-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element(),
        )
    }
}
