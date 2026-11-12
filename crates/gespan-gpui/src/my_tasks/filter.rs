//! The filter menu, in Inbox's style: PROJECTS, KIND and WAITING.
use super::*;
use crate::popover::{Align, Anchor, popover};
use gespan_core::tasks::MinWait;
use gpui::{AnyElement, Div, FontWeight, Role, SharedString, Stateful, Toggled, px};

fn caption(text: &'static str) -> Div {
    div()
        .px(px(8.))
        .pt(px(10.))
        .pb(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(content(0.4))
        .child(text)
}

fn menu_row(
    id: String,
    label: String,
    count: Option<usize>,
    checked: bool,
    role: Role,
) -> Stateful<Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .role(role)
        .aria_label(SharedString::from(label.clone()))
        .aria_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        })
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.8))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.06)))
        .child(
            div()
                .size(px(14.))
                .when(checked, |d| d.child(icon("check", 14.))),
        )
        .child(div().flex_1().min_w_0().truncate().child(label))
        .when_some(count, |d, count| {
            d.child(div().text_color(content(0.4)).child(count.to_string()))
        })
}

fn toggle<T: std::hash::Hash + Eq>(set: &mut std::collections::HashSet<T>, value: T) {
    if !set.remove(&value) {
        set.insert(value);
    }
}

impl MyTasks {
    /// Every project with a task, and any hidden one, so it can be shown again.
    /// The project and kind of each entry on the tab shown.
    fn tab_entries(&self) -> Vec<(String, Option<Kind>)> {
        match self.tab {
            Tab::Waiting => self
                .tasks
                .iter()
                .map(|t| (t.project.clone(), Some(t.kind)))
                .collect(),
            Tab::Answered => self
                .past
                .iter()
                .map(|t| (t.project.clone(), Some(t.kind)))
                .collect(),
            Tab::Decisions => self
                .decisions
                .iter()
                .map(|d| (super::past::decision_group(d).0, None))
                .collect(),
        }
    }

    fn filter_projects(&self) -> Vec<String> {
        let mut projects: Vec<String> = Vec::new();
        let entries = self.tab_entries();
        for project in entries
            .iter()
            .map(|(project, _)| project.as_str())
            .chain(self.filter.hidden_projects.iter().map(String::as_str))
        {
            if !projects.iter().any(|p| p == project) {
                projects.push(project.to_owned());
            }
        }
        projects
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_selection(cx);
    }

    pub(super) fn filter_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let mut rows: Vec<AnyElement> = vec![caption("PROJECTS").into_any_element()];
        let entries = self.tab_entries();
        for project in self.filter_projects() {
            let count = entries.iter().filter(|(p, _)| *p == project).count();
            let shown = !self.filter.hidden_projects.contains(&project);
            let key = project.clone();
            rows.push(
                menu_row(
                    format!("my-tasks-filter-project-{project}"),
                    self.project_label(&project),
                    Some(count),
                    shown,
                    Role::MenuItemCheckBox,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    toggle(&mut this.filter.hidden_projects, key.clone());
                    this.filter_changed(cx);
                }))
                .into_any_element(),
            );
        }
        let kinds = if self.tab == Tab::Decisions {
            &[][..]
        } else {
            rows.push(caption("KIND").into_any_element());
            &Kind::ALL[..]
        };
        for &kind in kinds {
            let count = entries.iter().filter(|(_, k)| *k == Some(kind)).count();
            rows.push(
                menu_row(
                    format!("my-tasks-filter-kind-{}", kind.label()),
                    kind.label().into(),
                    Some(count),
                    !self.filter.hidden_kinds.contains(&kind),
                    Role::MenuItemCheckBox,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    toggle(&mut this.filter.hidden_kinds, kind);
                    this.filter_changed(cx);
                }))
                .into_any_element(),
            );
        }
        let waits: &[(MinWait, &str, &str)] = if self.tab == Tab::Waiting {
            rows.push(caption("WAITING").into_any_element());
            &[
                (MinWait::Any, "any", "Any time"),
                (MinWait::Hour, "1h", "Over 1 hour"),
                (MinWait::EightHours, "8h", "Over 8 hours"),
            ]
        } else {
            &[]
        };
        for &(wait, id, label) in waits {
            rows.push(
                menu_row(
                    format!("my-tasks-filter-wait-{id}"),
                    label.into(),
                    None,
                    self.filter.min_wait == wait,
                    Role::MenuItemRadio,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter.min_wait = wait;
                    this.filter_changed(cx);
                }))
                .into_any_element(),
            );
        }
        if self.filter.is_active() {
            rows.push(
                menu_row(
                    "my-tasks-filter-clear".into(),
                    "Clear filters".into(),
                    None,
                    false,
                    Role::MenuItem,
                )
                .mt(px(4.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.filter = Filter::default();
                    this.filters_open = false;
                    this.filter_changed(cx);
                }))
                .into_any_element(),
            );
        }
        popover(
            "my-tasks-filter-menu",
            Anchor::Slot(self.filter_anchor.clone()),
        )
        .align(Align::End)
        .width(240.)
        .max_height(520.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.filters_open = false;
                cx.notify();
            });
        })
        .child(
            div()
                .id("my-tasks-filter-scroll")
                .overflow_y_scroll()
                .children(rows),
        )
        .into_any_element()
    }
}
