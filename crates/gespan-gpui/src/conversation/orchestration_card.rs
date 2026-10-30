//! The plan card above the composer: OrchestrationPreview.tsx's review of a
//! lead's assignments before any worker starts.
use super::composer_features::{ResumeOrchestration, StartOrchestration, StopOrchestration};
use super::*;
use gespan_core::{
    app_access, catalog,
    orchestration::{Plan, Task},
};
use gpui::App;
use std::collections::{HashMap, HashSet};

/// One task's open picker.
struct TaskMenu {
    menu: Entity<Menu>,
    anchor: AnchorSlot,
    _events: Vec<Subscription>,
}

/// One open task's fields. Each edit lands in the plan as it is typed.
struct TaskEditor {
    title: Entity<input::Input>,
    prompt: Entity<input::Input>,
    _changed: [Subscription; 2],
}

#[derive(Default)]
pub(super) struct PlanCard {
    anchors: HashMap<(String, &'static str), AnchorSlot>,
    menu: Option<TaskMenu>,
    expanded: HashSet<String>,
    editors: HashMap<String, TaskEditor>,
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
    pub(crate) fn orchestration_profiles(
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
    /// The card's fields belong to one plan, and every plan change goes
    /// through here: an editor left over from another would show text that
    /// isn't what runs.
    pub(crate) fn set_orchestration_plan(&mut self, plan: Option<Plan>) {
        self.orchestration_plan = plan;
        self.plan_card = Default::default();
    }
    fn toggle_task(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.plan_card.expanded.remove(&id) {
            self.plan_card.expanded.insert(id.clone());
            let task = self
                .orchestration_plan
                .as_ref()
                .and_then(|plan| plan.tasks.iter().find(|task| task.id == id))
                .cloned();
            if let Some(task) = task.filter(|_| !self.plan_card.editors.contains_key(&id)) {
                let title = cx.new(|cx| {
                    let mut input = input::Input::field("Task", "Task", content(0.4), cx);
                    input.set_text(task.title.clone(), cx);
                    input
                });
                let prompt = cx.new(|cx| {
                    let mut input =
                        input::Input::multiline_field("Instructions", "Instructions", cx);
                    input.set_text(task.prompt.clone(), cx);
                    input
                });
                let changed = [&title, &prompt].map(|field| {
                    let id = id.clone();
                    cx.subscribe(field, move |this, _, _: &input::Changed, cx| {
                        this.task_edited(&id, cx)
                    })
                });
                self.plan_card.editors.insert(
                    id,
                    TaskEditor {
                        title,
                        prompt,
                        _changed: changed,
                    },
                );
            }
        }
        cx.notify();
    }
    fn task_edited(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(editor) = self.plan_card.editors.get(id) else {
            return;
        };
        let title = editor.title.read(cx).content.to_string();
        let prompt = editor.prompt.read(cx).content.to_string();
        if let Some(plan) = &mut self.orchestration_plan {
            plan.set_task_text(id, &title, &prompt);
        }
        self.orchestration_error = None;
        cx.notify();
    }
    fn set_plan_workers(&mut self, workers: usize, cx: &mut Context<Self>) {
        if let Some(plan) = &mut self.orchestration_plan {
            plan.max_workers = workers;
        }
        cx.notify();
    }
    /// A blank edit stays on the card with the reason; nothing starts.
    fn start_plan(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.orchestration_plan.clone() else {
            return;
        };
        match gespan_core::orchestration::validate(&plan) {
            Ok(()) => {
                self.orchestration_error = None;
                cx.emit(StartOrchestration(plan));
            }
            Err(error) => self.orchestration_error = Some(error),
        }
        cx.notify();
    }
    fn workers_row(&self, workers: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = div()
            .flex()
            .items_center()
            .gap_1p5()
            .text_size(px(11.))
            .text_color(content(0.45));
        if self.orchestration_progress.is_some() {
            return row.child(format!("{workers} parallel")).into_any_element();
        }
        row.child("Parallel workers")
            .children((1..=gespan_core::orchestration::MAX_WORKERS).map(|number| {
                let selected = number == workers;
                div()
                    .id(("plan-workers", number))
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!("{number} parallel workers")))
                    .size(px(20.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(content(0.14)).text_color(content(1.)))
                    .when(!selected, |d| d.hover(|s| s.bg(content(0.08))))
                    .child(number.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_plan_workers(number, cx)))
            }))
            .into_any_element()
    }
    fn task_details(&self, task: &Task) -> AnyElement {
        let details = div()
            .flex()
            .flex_col()
            .gap_1p5()
            .pt_1()
            .pl_4()
            .text_size(px(11.))
            .text_color(content(0.45));
        match self
            .plan_card
            .editors
            .get(&task.id)
            .filter(|_| self.orchestration_progress.is_none())
        {
            Some(editor) => {
                let field = |label: &'static str, input: Entity<input::Input>| {
                    div().child(label).child(
                        div()
                            .mt_1()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(content(0.12))
                            .text_color(content(1.))
                            .child(input),
                    )
                };
                details
                    .child(field("Task", editor.title.clone()))
                    .child(field("Instructions", editor.prompt.clone()))
            }
            None => details.child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.6))
                    .child(task.prompt.clone()),
            ),
        }
        .into_any_element()
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
                    let open = self.plan_card.expanded.contains(&task.id);
                    let id = task.id.clone();
                    let header = div()
                        .py_1()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .id(SharedString::from(format!("task-details-{}", task.id)))
                                .role(Role::Button)
                                .aria_label(SharedString::from(format!(
                                    "Details for {}",
                                    task.title
                                )))
                                .min_w_0()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_task(id.clone(), cx)
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            icon(
                                                if open {
                                                    "chevron-down"
                                                } else {
                                                    "chevron-right"
                                                },
                                                12.,
                                            )
                                            .text_color(content(0.5)),
                                        )
                                        .child(format!("{} · {}", task.id, task.title)),
                                )
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
                        .child(pills);
                    div()
                        .child(header)
                        .when(open, |d| d.child(self.task_details(task)))
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
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .child(self.workers_row(plan.max_workers, cx));
            if self.orchestration_progress.is_none() {
                card = card.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            tool_pill("start-orchestration", "Start agents")
                                .bg(content(0.1))
                                .child("Start agents")
                                .on_click(cx.listener(|this, _, _, cx| this.start_plan(cx))),
                        )
                        .child(
                            tool_pill("dismiss-orchestration", "Dismiss plan")
                                .child("Dismiss")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_orchestration_plan(None);
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
                    div()
                        .flex()
                        .gap_2()
                        .when(self.orchestration_paused, |d| {
                            d.child(
                                tool_pill("resume-orchestration", "Resume agents")
                                    .bg(content(0.1))
                                    .child("Resume agents")
                                    .on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(ResumeOrchestration)),
                                    ),
                            )
                        })
                        .child(
                            tool_pill("stop-orchestration", "Stop agents")
                                .child("Stop agents")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(StopOrchestration))),
                        ),
                );
        }
        card = card.children(self.plan_card.menu.as_ref().map(|open| {
            menu_popover(&open.menu, Anchor::Slot(open.anchor.clone()), cx).side(Side::Top)
        }));
        Some(card.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::super::menu_tests::conversation;
    use super::*;
    use gespan_core::orchestration::MAX_WORKERS;
    use gpui::{TestAppContext, WindowHandle};

    fn with_plan(window: WindowHandle<Conversation>, cx: &mut TestAppContext) {
        window
            .update(cx, |view, _, _| {
                view.set_orchestration_plan(Some(Plan {
                    title: "Work".into(),
                    summary: String::new(),
                    tasks: vec![Task {
                        id: "a".into(),
                        title: "First".into(),
                        prompt: "Do it".into(),
                        files: vec!["src".into()],
                        depends_on: Vec::new(),
                        harness: None,
                        model: None,
                        model_settings: Default::default(),
                    }],
                    max_workers: MAX_WORKERS,
                }));
            })
            .unwrap();
    }

    fn edit(
        window: WindowHandle<Conversation>,
        cx: &mut TestAppContext,
        title: &str,
        prompt: &str,
    ) {
        window
            .update(cx, |view, _, cx| {
                let editor = &view.plan_card.editors["a"];
                let (title_field, prompt_field) = (editor.title.clone(), editor.prompt.clone());
                title_field.update(cx, |input, cx| input.set_text(title.to_owned(), cx));
                prompt_field.update(cx, |input, cx| input.set_text(prompt.to_owned(), cx));
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui::test]
    fn opening_a_task_fills_its_fields_and_edits_reach_the_plan(cx: &mut TestAppContext) {
        let window = conversation(cx);
        with_plan(window, cx);
        window
            .update(cx, |view, _, cx| {
                view.toggle_task("a".into(), cx);
                let editor = &view.plan_card.editors["a"];
                assert_eq!(editor.title.read(cx).content.as_ref(), "First");
                assert_eq!(editor.prompt.read(cx).content.as_ref(), "Do it");
            })
            .unwrap();
        edit(window, cx, "Renamed", "Do it differently");
        window
            .update(cx, |view, _, cx| {
                let task = &view.orchestration_plan.as_ref().unwrap().tasks[0];
                assert_eq!(task.title, "Renamed");
                assert_eq!(task.prompt, "Do it differently");
                // Closing and reopening keeps what was typed.
                view.toggle_task("a".into(), cx);
                view.toggle_task("a".into(), cx);
                let editor = &view.plan_card.editors["a"];
                assert_eq!(editor.title.read(cx).content.as_ref(), "Renamed");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_new_plan_never_shows_the_last_plans_fields(cx: &mut TestAppContext) {
        let window = conversation(cx);
        with_plan(window, cx);
        window
            .update(cx, |view, _, cx| view.toggle_task("a".into(), cx))
            .unwrap();
        edit(window, cx, "Typed for the old plan", "Old instructions");
        // Same task ID, as a re-plan usually produces.
        with_plan(window, cx);
        window
            .update(cx, |view, _, cx| {
                assert!(view.plan_card.editors.is_empty());
                view.toggle_task("a".into(), cx);
                let editor = &view.plan_card.editors["a"];
                assert_eq!(editor.title.read(cx).content.as_ref(), "First");
                assert_eq!(editor.prompt.read(cx).content.as_ref(), "Do it");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_blank_edit_refuses_to_start_and_typing_clears_the_reason(cx: &mut TestAppContext) {
        let window = conversation(cx);
        with_plan(window, cx);
        window
            .update(cx, |view, _, cx| view.toggle_task("a".into(), cx))
            .unwrap();
        edit(window, cx, "", "Do it");
        window
            .update(cx, |view, _, cx| {
                view.start_plan(cx);
                let error = view.orchestration_error.clone().unwrap();
                assert!(error.contains("needs a title"), "{error}");
            })
            .unwrap();
        edit(window, cx, "Back", "Do it");
        window
            .update(cx, |view, _, cx| {
                assert!(view.orchestration_error.is_none());
                view.start_plan(cx);
                assert!(view.orchestration_error.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_worker_count_lands_in_the_plan(cx: &mut TestAppContext) {
        let window = conversation(cx);
        with_plan(window, cx);
        window
            .update(cx, |view, _, cx| {
                view.set_plan_workers(2, cx);
                assert_eq!(view.orchestration_plan.as_ref().unwrap().max_workers, 2);
            })
            .unwrap();
    }
}
