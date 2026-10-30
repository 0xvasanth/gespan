//! The plan card above the composer: OrchestrationPreview.tsx's review of a
//! lead's assignments before any worker starts.
use super::composer_features::{StartOrchestration, StopOrchestration};
use super::*;
use gespan_core::{app_access, catalog, orchestration::Task};
use gpui::App;
use std::collections::HashMap;

/// One task's open picker.
struct TaskMenu {
    menu: Entity<Menu>,
    anchor: AnchorSlot,
    _events: Vec<Subscription>,
}

#[derive(Default)]
pub(super) struct PlanCard {
    anchors: HashMap<(String, &'static str), AnchorSlot>,
    menu: Option<TaskMenu>,
}

impl PlanCard {
    fn anchor(&mut self, task: &str, part: &'static str) -> AnchorSlot {
        self.anchors
            .entry((task.to_owned(), part))
            .or_default()
            .clone()
    }
}

impl Conversation {
    /// Every harness with the account this project uses for it. The lead's
    /// own harness keeps the lead's account.
    pub(super) fn orchestration_profiles(
        &self,
        cx: &App,
    ) -> Vec<(Kind, gespan_core::accounts::Profile)> {
        let hidden = cx
            .try_global::<crate::settings::Preferences>()
            .map(|p| p.hidden_for(&self.summary.cwd))
            .unwrap_or_default();
        Kind::ALL
            .into_iter()
            .filter(|kind| !hidden.iter().any(|id| id == kind.id()))
            .map(|kind| {
                let profile = if kind.id() == self.summary.provider {
                    self.settings.account.clone()
                } else {
                    self.task_account(kind, cx)
                };
                (kind, profile)
            })
            .collect()
    }
    fn task_account(&self, kind: Kind, cx: &App) -> gespan_core::accounts::Profile {
        if kind.id() == self.summary.provider {
            return self.settings.account.clone();
        }
        crate::accounts::get(cx)
            .map(|center| center.read(cx).selected(kind, &self.summary.cwd))
            .unwrap_or_default()
    }
    fn task_model(&self, task: &Task) -> Option<String> {
        task.model.clone().or_else(|| {
            (task.kind(&self.summary.provider).id() == self.summary.provider)
                .then(|| self.model.clone())
                .flatten()
        })
    }
    fn task_catalog_model(&self, task: &Task, cx: &App) -> Option<catalog::Model> {
        let kind = task.kind(&self.summary.provider);
        let model = self.task_model(task)?;
        catalog::cached_models_for(kind, &self.task_account(kind, cx))?
            .into_iter()
            .find(|entry| entry.id == model)
    }
    fn close_task_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plan_card.menu = None;
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }
    fn open_task_model(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self
            .orchestration_plan
            .as_ref()
            .and_then(|plan| plan.tasks.iter().find(|task| task.id == id))
            .cloned()
        else {
            return;
        };
        let kind = task.kind(&self.summary.provider);
        let list = cx.new(|cx| {
            let hidden = cx
                .try_global::<crate::settings::Preferences>()
                .map(|p| p.hidden_for(&self.summary.cwd))
                .unwrap_or_default();
            ModelList::new_for(
                kind,
                self.task_model(&task),
                &self.summary.cwd,
                self.task_account(kind, cx),
                cx,
            )
            .hide_providers(hidden)
            .allow_provider_switch(true)
        });
        let focus = list.read(cx).search_focus(cx);
        let picked = cx.subscribe_in(
            &list,
            window,
            move |this, _, picked: &ModelPicked, window, cx| {
                if let Some(plan) = &mut this.orchestration_plan {
                    plan.set_task_model(&id, picked.0, picked.1.clone());
                }
                this.close_task_menu(window, cx);
            },
        );
        let menu = cx.new(|cx| {
            Menu::new(
                "Model",
                vec![MenuItem::Custom(list.into(), focus)],
                window,
                cx,
            )
            .width(400.)
        });
        let closed = cx.subscribe_in(&menu, window, |this, _, _: &MenuEvent, window, cx| {
            this.close_task_menu(window, cx);
        });
        self.plan_card.menu = Some(TaskMenu {
            menu,
            anchor: self.plan_card.anchor(&task.id, "model"),
            _events: vec![picked, closed],
        });
        cx.notify();
    }
    fn open_task_effort(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self
            .orchestration_plan
            .as_ref()
            .and_then(|plan| plan.tasks.iter().find(|task| task.id == id))
            .cloned()
        else {
            return;
        };
        let kind = task.kind(&self.summary.provider);
        let Some(control) = self
            .task_catalog_model(&task, cx)
            .and_then(|model| app_access::effort_control(kind, &model))
        else {
            return;
        };
        let items = control
            .choices
            .iter()
            .map(|(value, label)| MenuAction::new(value.clone(), label.clone()).into())
            .collect();
        let menu = cx.new(|cx| Menu::new("Effort", items, window, cx).width(200.));
        let key = control.id;
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &MenuEvent, window, cx| {
                if let MenuEvent::Picked(value, _) = event
                    && let Some(plan) = &mut this.orchestration_plan
                {
                    plan.set_task_setting(&id, &key, value);
                }
                this.close_task_menu(window, cx);
            },
        );
        self.plan_card.menu = Some(TaskMenu {
            menu,
            anchor: self.plan_card.anchor(&task.id, "effort"),
            _events: vec![events],
        });
        cx.notify();
    }
    /// The model pill, and an effort pill when the model has that control.
    fn task_pills(&mut self, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let kind = task.kind(&self.summary.provider);
        let model = self.task_model(task);
        let name = crate::pickers::cached_model_name(kind, model.as_deref());
        let effort = self
            .task_catalog_model(task, cx)
            .and_then(|model| app_access::effort_control(kind, &model))
            .map(|control| {
                let value = task
                    .model_settings
                    .get(&control.id)
                    .cloned()
                    .unwrap_or_else(|| control.current.clone());
                control
                    .choices
                    .iter()
                    .find(|(id, _)| *id == value)
                    .map_or(value, |(_, label)| label.clone())
            });
        let locked = self.orchestration_progress.is_some();
        let model_anchor = self.plan_card.anchor(&task.id, "model");
        let effort_anchor = self.plan_card.anchor(&task.id, "effort");
        let pill = |key: String, label: String, anchor: AnchorSlot| {
            div()
                .id(SharedString::from(key))
                .role(Role::Button)
                .aria_label(SharedString::from(label.clone()))
                .relative()
                .flex_shrink_0()
                .h(px(22.))
                .px(px(6.))
                .rounded_md()
                .flex()
                .items_center()
                .text_size(px(11.))
                .bg(content(0.06))
                .text_color(content(0.65))
                .child(anchor.probe())
                .child(label)
                .when(!locked, |d| {
                    d.cursor_pointer().hover(|s| s.bg(content(0.1)))
                })
        };
        let id = task.id.clone();
        let mut row = div().flex().flex_shrink_0().gap_1().child(
            pill(
                format!("task-model-{}", task.id),
                format!("{name} · {}", kind.label()),
                model_anchor,
            )
            .when(!locked, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    this.open_task_model(id.clone(), window, cx)
                }))
            }),
        );
        if let Some(effort) = effort {
            let id = task.id.clone();
            row = row.child(
                pill(format!("task-effort-{}", task.id), effort, effort_anchor).when(
                    !locked,
                    |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.open_task_effort(id.clone(), window, cx)
                        }))
                    },
                ),
            );
        }
        row.into_any_element()
    }
    pub(super) fn orchestration_card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.orchestration_plan.is_none()
            && self.orchestration_error.is_none()
            && self.orchestration_progress.is_none()
        {
            return None;
        }
        let mut card = div()
            .id("orchestration-card")
            .mb_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(tint(FUCHSIA_300, 0.2))
            .bg(tint(FUCHSIA_300, 0.04))
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("share", 16.).text_color(tint(FUCHSIA_300, 0.8)))
                    .child(
                        self.orchestration_plan
                            .as_ref()
                            .map(|p| p.title.clone())
                            .unwrap_or_else(|| "Orchestrator".into()),
                    ),
            );
        if let Some(plan) = self.orchestration_plan.clone() {
            let rows: Vec<_> = plan
                .tasks
                .iter()
                .map(|task| {
                    let pills = self.task_pills(task, cx);
                    div()
                        .py_1()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .child(format!("{} · {}", task.id, task.title))
                                .child(div().text_size(px(10.)).text_color(content(0.4)).child(
                                    format!(
                                        "{}{}",
                                        task.files.join(", "),
                                        if task.depends_on.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" · after {}", task.depends_on.join(", "))
                                        }
                                    ),
                                )),
                        )
                        .child(pills)
                })
                .collect();
            card = card
                .child(
                    div()
                        .text_color(content(0.6))
                        .child(gespan_core::response::limit(&plan.summary, 600)),
                )
                .child(
                    div()
                        .id("orchestration-tasks")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .children(rows),
                );
            if self.orchestration_progress.is_none() {
                card = card.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            tool_pill("start-orchestration", "Start agents")
                                .bg(content(0.1))
                                .child("Start agents")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(plan) = this.orchestration_plan.clone() {
                                        cx.emit(StartOrchestration(plan));
                                    }
                                })),
                        )
                        .child(
                            tool_pill("dismiss-orchestration", "Dismiss plan")
                                .child("Dismiss")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.orchestration_plan = None;
                                    this.plan_card = Default::default();
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        if let Some(error) = &self.orchestration_error {
            card = card
                .child(div().text_color(tint(AMBER_400, 0.9)).child(error.clone()))
                .child(
                    tool_pill("dismiss-plan-error", "Dismiss")
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.orchestration_error = None;
                            cx.notify();
                        })),
                );
        }
        if let Some(progress) = &self.orchestration_progress {
            card = card
                .child(div().text_color(content(0.6)).child(progress.clone()))
                .child(
                    tool_pill("stop-orchestration", "Stop agents")
                        .child("Stop agents")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(StopOrchestration))),
                );
        }
        card = card.children(self.plan_card.menu.as_ref().map(|open| {
            menu_popover(&open.menu, Anchor::Slot(open.anchor.clone()), cx).side(Side::Top)
        }));
        Some(card.into_any_element())
    }
}
