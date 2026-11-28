//! Azure DevOps over HTTPS with the token saved in Settings.
use super::PullRequestHost;
use crate::inbox::{
    self, azure_devops as ado, Details, Diff, Filters, Item, LineComment, Status, Thread, Verdict,
};
use std::path::{Path, PathBuf};

pub(super) struct AzureDevOps {
    /// Where the saved connection lives.
    data_dir: PathBuf,
}

impl AzureDevOps {
    pub(super) fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
    fn dir(&self) -> PathBuf {
        self.data_dir.clone()
    }
}

fn with_provider(mut items: Vec<Item>) -> Vec<Item> {
    for item in &mut items {
        item.provider = "azuredevops".into();
    }
    items
}

impl PullRequestHost for AzureDevOps {
    fn provider(&self) -> &'static str {
        "azuredevops"
    }

    fn status(&self) -> Result<Status, String> {
        inbox::convert(ado::azure_devops_status(self.dir())?)
    }

    fn connect(&self, url: String, token: String) -> Result<Status, String> {
        inbox::convert(ado::azure_devops_set_config(self.dir(), url, token)?)
    }

    fn current_user(&self, _project: &Path) -> Result<String, String> {
        Err("Azure DevOps pull requests can't start automations yet".into())
    }

    fn permission(&self, _item: &Item, _author: &str) -> Result<String, String> {
        Err("Azure DevOps doesn't say what a comment's author may do".into())
    }

    fn repositories(&self, project: &Path) -> Result<Vec<String>, String> {
        ado::azure_devops_repo(self.dir(), project.to_string_lossy().into_owned()).map(|r| vec![r])
    }

    fn search_repos(&self, query: &str) -> Result<Vec<(String, String)>, String> {
        ado::azure_devops_search_repos(self.dir(), query)
    }

    fn work_items(
        &self,
        project: &Path,
        _repo: &str,
        kind: &str,
        _filters: &Filters,
        state: &str,
        limit: u32,
    ) -> Result<Vec<Item>, String> {
        inbox::convert(ado::azure_devops_list_work_items(
            self.dir(),
            project.to_string_lossy().into_owned(),
            kind.into(),
            false,
            state.into(),
            Some(limit),
        )?)
    }

    fn assigned(&self, kind: &str, limit: u32) -> Option<Result<Vec<Item>, String>> {
        Some(
            ado::azure_devops_list_todos(self.dir(), kind.into(), Some(limit))
                .and_then(inbox::convert)
                .map(with_provider),
        )
    }

    fn repo_pull_requests(&self, repo: &str) -> Result<Vec<Item>, String> {
        inbox::convert(ado::azure_devops_list_repo_prs(self.dir(), repo, 40)?).map(with_provider)
    }

    fn review_requests(&self) -> Result<Vec<Item>, String> {
        ado::azure_devops_review_requests(self.dir())
            .and_then(inbox::convert)
            .map(with_provider)
    }

    fn details(&self, item: &Item) -> Result<Details, String> {
        inbox::convert(ado::azure_devops_work_item_details(
            self.dir(),
            item.repo.clone(),
            item.kind.clone(),
            item.number,
        )?)
    }

    fn thread(&self, item: &Item) -> Result<Thread, String> {
        inbox::convert(ado::azure_devops_work_item_thread(
            self.dir(),
            item.repo.clone(),
            item.kind.clone(),
            item.number,
        )?)
    }

    fn comment(&self, item: &Item, body: String, reply_to: String) -> Result<String, String> {
        ado::azure_devops_work_item_comment(
            self.dir(),
            item.repo.clone(),
            item.kind.clone(),
            item.number,
            body,
            reply_to,
        )
    }

    fn resolve_thread(&self, item: &Item, thread_id: &str, resolved: bool) -> Result<(), String> {
        if item.kind != "pr" {
            return Err("This host has no resolving".into());
        }
        ado::azure_devops_resolve_thread(
            self.dir(),
            item.repo.clone(),
            item.number,
            thread_id.to_owned(),
            resolved,
        )
    }

    /// Azure DevOps sends file contents, diffed here; there is no whole-file
    /// view.
    fn diff(&self, item: &Item, _full_context: bool) -> Result<Diff, String> {
        inbox::convert(ado::azure_devops_mr_diff(
            self.dir(),
            item.repo.clone(),
            item.number,
        )?)
    }

    fn submit_review(
        &self,
        item: &Item,
        verdict: Verdict,
        body: &str,
        comments: &[LineComment],
    ) -> Result<(), String> {
        ado::azure_devops_submit_review(
            self.dir(),
            &item.repo,
            item.number,
            verdict,
            body,
            comments,
        )
    }

    fn rerun_failed(&self, item: &Item) -> Result<String, String> {
        ado::azure_devops_rerun_failed(self.dir(), &item.repo, item.number)
    }
}
