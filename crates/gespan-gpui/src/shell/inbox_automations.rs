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
    pub(super) fn inbox_menu_failed(&self, error: String, cx: &mut Context<Self>) {
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
        let (kind, model) = crate::installed_providers::new_session_choice(
            &self.workspace.preferences,
            Some(&item.project_path),
            cx,
        );
        let review = service_lock(&service).and_then(|mut service| {
            review_for(
                &mut service,
                &item,
                kind.id().into(),
                model.unwrap_or_default(),
            )
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

/// The automation that reviews `item`: its repo's system code review, else
/// the project's own review automation. With neither, and a project folder to
/// review in, the repo's code review is made paused, so this review runs now
/// and nothing reviews on its own until Settings › Git turns it on.
fn review_for(
    service: &mut gespan_core::automations::Service,
    item: &gespan_core::inbox::Item,
    harness: String,
    model: String,
) -> Result<Option<String>, String> {
    use gespan_core::automations::system::{self, ReviewSettings};
    let repo = gespan_core::inbox::queue::repo_key(&item.provider, &item.repo);
    let list = service.list()?;
    if let Some(review) = list
        .iter()
        .find(|automation| system::reviewed_repo(automation) == Some(repo.as_str()))
    {
        return Ok(Some(review.id.clone()));
    }
    if let Some(own) = find_review_automation(&list, &item.project_path) {
        return Ok(Some(own.id.clone()));
    }
    if item.project_path.is_empty() || !matches!(item.provider.as_str(), "github" | "gitlab") {
        return Ok(None);
    }
    let made = service.ensure_code_review(
        &repo,
        &item.project_path,
        &ReviewSettings {
            which: system::which_choices(&repo)[0],
            harness,
            model,
            results: gespan_core::review_policy::ReviewResults::Gespan,
            enabled: false,
        },
    )?;
    Ok(Some(made.id))
}

#[cfg(test)]
mod review_for_tests {
    use super::review_for;
    use gespan_core::{
        automations::{
            Service,
            review_setup::{ReviewSetupInput, WhichPrs, review_draft},
            system::{self, ReviewSettings},
        },
        inbox::Item,
        review_policy::ReviewResults,
    };

    fn service() -> (tempfile::TempDir, Service) {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open(&dir.path().join("gespan.db")).unwrap();
        (dir, service)
    }
    fn pr(provider: &str, repo: &str, project: &str) -> Item {
        Item {
            provider: provider.into(),
            kind: "pr".into(),
            number: 9,
            repo: repo.into(),
            project_path: project.into(),
            title: "Fix".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_repos_system_review_wins() {
        let (_dir, mut service) = service();
        // The project's own hand-made review exists too.
        let mut own = review_draft(&ReviewSetupInput {
            cwd: "/work/app".into(),
            repo: None,
            which: WhichPrs::Opened,
            harness: "codex".into(),
            model: "m".into(),
            results: ReviewResults::Gespan,
        });
        own.name = "My review".into();
        service.upsert(own).unwrap();
        let system = service
            .ensure_code_review(
                "github:acme/app",
                "/work/app",
                &ReviewSettings {
                    which: WhichPrs::ReviewRequested,
                    harness: "codex".into(),
                    model: "m".into(),
                    results: ReviewResults::Gespan,
                    enabled: true,
                },
            )
            .unwrap();
        let picked = review_for(
            &mut service,
            &pr("github", "Acme/App", "/work/app"),
            "claude".into(),
            String::new(),
        );
        assert_eq!(picked, Ok(Some(system.id)));
    }

    #[test]
    fn without_one_the_projects_own_review_runs() {
        let (_dir, mut service) = service();
        let own = review_draft(&ReviewSetupInput {
            cwd: "/work/app".into(),
            repo: None,
            which: WhichPrs::Opened,
            harness: "codex".into(),
            model: "m".into(),
            results: ReviewResults::Gespan,
        });
        let own = service.upsert(own).unwrap();
        let picked = review_for(
            &mut service,
            &pr("github", "acme/app", "/work/app"),
            "claude".into(),
            String::new(),
        );
        assert_eq!(picked, Ok(Some(own.id)));
        assert!(
            service.code_reviews().unwrap().is_empty(),
            "nothing new is made"
        );
    }

    #[test]
    fn the_first_review_makes_the_repos_review_paused() {
        let (_dir, mut service) = service();
        let picked = review_for(
            &mut service,
            &pr("github", "acme/app", "/work/app"),
            "claude".into(),
            "opus".into(),
        )
        .unwrap()
        .unwrap();
        let reviews = service.code_reviews().unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].id, picked);
        assert_eq!(system::reviewed_repo(&reviews[0]), Some("github:acme/app"));
        assert!(
            !reviews[0].enabled,
            "a click reviews once; nothing reviews on its own"
        );
        assert_eq!(reviews[0].harness, "claude");
        // The run it makes is for this pull request.
        let due = gespan_core::automations::run_on_item(
            &mut service,
            &picked,
            &pr("github", "acme/app", "/work/app"),
            1,
        )
        .unwrap();
        assert_eq!(due.automation.id, picked);
    }

    #[test]
    fn nothing_is_made_without_a_folder_or_on_other_hosts() {
        let (_dir, mut service) = service();
        assert_eq!(
            review_for(
                &mut service,
                &pr("github", "acme/web", ""),
                "claude".into(),
                String::new()
            ),
            Ok(None)
        );
        assert_eq!(
            review_for(
                &mut service,
                &pr("azuredevops", "acme/app", "/work/app"),
                "claude".into(),
                String::new()
            ),
            Ok(None)
        );
        assert!(service.code_reviews().unwrap().is_empty());
    }
}
