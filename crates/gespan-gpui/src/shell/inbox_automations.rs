//! The Inbox ⋯ menu's automations: which ones can run on a pull request,
//! running one now with the pull request as its event item, the project's
//! review automation, and a new pull request automation.
use super::automation_runs::{now, service_lock};
use super::*;
use crate::inbox::{ItemAutomation, ItemRun};
use gespan_core::automations::{self, Automation, review_setup::find_review_automation};
use gespan_core::inbox::Item;

/// The ⋯ menu's rows for `item`: the project's pull request automations.
pub(super) fn item_automations(list: &[Automation], item: &Item) -> Vec<ItemAutomation> {
    automations::pull_request_automations(list, item)
        .into_iter()
        .map(|automation| ItemAutomation {
            id: automation.id.clone(),
            name: automation.name.clone(),
            trigger: automation.trigger_label(),
            agent: gespan_core::provider::Kind::from_id(&automation.harness)
                .label()
                .into(),
        })
        .collect()
}

impl Shell {
    fn inbox_menu_failed(&self, error: String, cx: &mut Context<Self>) {
        if let Some(inbox) = &self.inbox {
            inbox.update(cx, |inbox, cx| inbox.menu_failed(error, cx));
        }
    }

    /// Answers the ⋯ menu's [`crate::inbox::Event::AutomationsWanted`].
    pub(super) fn inbox_automations_wanted(&mut self, item: Item, cx: &mut Context<Self>) {
        let Some(service) = self.automation_state.service.clone() else {
            self.initialize_automations(cx);
            return;
        };
        let key = item.key();
        let work = cx.background_executor().spawn(async move {
            let list = service_lock(&service)?.list()?;
            Ok::<_, String>(item_automations(&list, &item))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(list) => {
                    if let Some(inbox) = &this.inbox {
                        inbox.update(cx, |inbox, cx| inbox.set_item_automations(&key, list, cx));
                    }
                }
                Err(error) => tracing::warn!(%error, "pull request automations were not listed"),
            });
        })
        .detach();
    }

    /// Runs `automation` now on `item` and launches it like any run; the
    /// Inbox follows the run's record in a pill.
    pub(super) fn run_inbox_automation(
        &mut self,
        item: Item,
        automation: String,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.automation_state.service.clone() else {
            self.initialize_automations(cx);
            self.inbox_menu_failed(
                "Automations are still starting. Try again in a moment.".into(),
                cx,
            );
            return;
        };
        let key = item.key();
        let work = cx.background_executor().spawn(async move {
            automations::run_on_item(&mut *service_lock(&service)?, &automation, &item, now())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(due) => {
                    if let Some(inbox) = &this.inbox {
                        let run = ItemRun::of(&due);
                        inbox.update(cx, |inbox, cx| inbox.track_item_run(key, run, cx));
                    }
                    this.launch_automation(due, false, cx);
                    this.refresh_run_views(cx);
                }
                Err(error) => this.inbox_menu_failed(error, cx),
            });
        })
        .detach();
    }

    /// "Review with an agent": the project's review automation runs on the
    /// pull request; without one, its setup sheet opens.
    pub(super) fn review_inbox_item(
        &mut self,
        item: Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.automation_state.service.clone() else {
            self.initialize_automations(cx);
            self.inbox_menu_failed(
                "Automations are still starting. Try again in a moment.".into(),
                cx,
            );
            return;
        };
        let review = service_lock(&service)
            .and_then(|mut service| service.list())
            .map(|list| {
                find_review_automation(&list, &item.project_path)
                    .map(|automation| automation.id.clone())
            });
        match review {
            Ok(Some(id)) => self.run_inbox_automation(item, id, cx),
            Ok(None) => {
                self.open_inbox_project_automations(&item, window, cx);
                if let Some(view) = &self.automations {
                    view.update(cx, |view, cx| view.start_review_setup(cx));
                }
            }
            Err(error) => self.inbox_menu_failed(error, cx),
        }
    }

    /// "New pull request automation": the editor with a pull request trigger.
    pub(super) fn new_inbox_automation(
        &mut self,
        item: Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(event) = crate::inbox::new_automation_event(&item.provider) else {
            return;
        };
        self.open_inbox_project_automations(&item, window, cx);
        if let Some(view) = &self.automations {
            view.update(cx, |view, cx| {
                view.new_pull_request_automation(&item.provider, event, cx)
            });
        }
    }

    /// The Automations page, on the item's project.
    fn open_inbox_project_automations(
        &mut self,
        item: &Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_automations(window, cx);
        if item.project_path.is_empty() {
            return;
        }
        let projects = self.workspace.projects.clone();
        if let Some(view) = &self.automations {
            let cwd = item.project_path.clone();
            view.update(cx, |view, cx| view.set_context(cwd, projects, cx));
        }
    }
}
