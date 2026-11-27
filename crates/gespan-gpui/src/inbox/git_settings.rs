//! What Settings › Pull requests shows and changes: your repositories (from your
//! projects, with a switch each, plus those added by hand), "Add a repo"
//! search, review requests from other repos, the view Git opens on, Notify
//! me and folding bot pull requests. The Pull requests page owns all of it; Settings
//! reads it from here and asks for changes.
use super::*;
use gespan_core::inbox::{RepoHit, queue};

/// One repository row in Settings › Pull requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoRow {
    pub provider: String,
    pub repo: String,
    /// "From project rebase", or "Added by hand".
    pub source: String,
    /// Switched off: Git leaves it out.
    pub hidden: bool,
    /// Added by hand, so it can be removed.
    pub added: bool,
}

/// Settings › Pull requests's view of the Pull requests page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GitSettings {
    pub repos: Vec<RepoRow>,
    pub other_repos: bool,
    pub opens_on: queue::View,
    pub keep_filters: bool,
    pub notify: bool,
    pub fold_bots: bool,
    pub hits: Vec<RepoHit>,
    pub searching: bool,
    pub search_error: Option<String>,
    /// Still listing repositories for the first time.
    pub loading: bool,
}

/// The "Add a repo" search: its field, the last query and what it found.
#[derive(Default)]
pub(super) struct RepoSearch {
    pub(super) query: String,
    pub(super) hits: Vec<RepoHit>,
    pub(super) searching: bool,
    pub(super) error: Option<String>,
    pub(super) generation: u64,
}

impl Inbox {
    pub(crate) fn git_settings(&self) -> GitSettings {
        let prefs = &self.state.prefs;
        let mut repos: Vec<RepoRow> = Vec::new();
        for repo in &self.snapshot.repos {
            if !super::git_list::HOSTS.contains(&repo.provider.as_str()) {
                continue;
            }
            let key = queue::repo_key(&repo.provider, &repo.repo);
            if repos
                .iter()
                .any(|r| queue::repo_key(&r.provider, &r.repo) == key)
            {
                continue;
            }
            let added = prefs
                .added_repos
                .iter()
                .any(|a| queue::repo_key(&a.provider, &a.repo) == key);
            repos.push(RepoRow {
                provider: repo.provider.clone(),
                repo: repo.repo.clone(),
                source: if repo.project_path.is_empty() {
                    "Added by hand".into()
                } else {
                    format!("From project {}", self.project_label(&repo.project_path))
                },
                hidden: prefs.hidden_repos.contains(&key),
                added: added && repo.project_path.is_empty(),
            });
        }
        for added in &prefs.added_repos {
            let key = queue::repo_key(&added.provider, &added.repo);
            if !repos
                .iter()
                .any(|r| queue::repo_key(&r.provider, &r.repo) == key)
            {
                repos.push(RepoRow {
                    provider: added.provider.clone(),
                    repo: added.repo.clone(),
                    source: "Added by hand".into(),
                    hidden: false,
                    added: true,
                });
            }
        }
        // Project repositories first, in project order; added ones after.
        repos.sort_by_key(|row| row.added);
        let known: Vec<String> = repos
            .iter()
            .map(|r| queue::repo_key(&r.provider, &r.repo))
            .collect();
        GitSettings {
            repos,
            other_repos: prefs.other_repos,
            opens_on: prefs.opens_on,
            keep_filters: prefs.keep_filters,
            notify: prefs.notify,
            fold_bots: prefs.fold_bots,
            hits: self
                .repo_search
                .hits
                .iter()
                .filter(|hit| !known.contains(&queue::repo_key(&hit.provider, &hit.repo)))
                .cloned()
                .collect(),
            searching: self.repo_search.searching,
            search_error: self.repo_search.error.clone(),
            loading: self.loading && self.snapshot.repos.is_empty(),
        }
    }
    /// The "Add a repo" field Settings › Pull requests draws.
    pub(crate) fn repo_search_input(&self) -> Entity<Input> {
        self.repo_search_input.clone()
    }

    /// Changes Settings › Pull requests's switches and choices.
    pub(crate) fn change_git_prefs(
        &mut self,
        change: impl FnOnce(&mut queue::Prefs),
        cx: &mut Context<Self>,
    ) {
        let before = self.state.prefs.clone();
        change(&mut self.state.prefs);
        if self.state.prefs == before {
            return;
        }
        let refetch = before.other_repos != self.state.prefs.other_repos
            || before.added_repos != self.state.prefs.added_repos;
        self.save();
        self.sync_arrivals(cx);
        if refetch {
            self.refresh(cx);
        }
        cx.notify();
    }
    pub(crate) fn set_repo_hidden(
        &mut self,
        provider: &str,
        repo: &str,
        hidden: bool,
        cx: &mut Context<Self>,
    ) {
        let key = queue::repo_key(provider, repo);
        self.change_git_prefs(
            |prefs| {
                prefs.hidden_repos.retain(|k| *k != key);
                if hidden {
                    prefs.hidden_repos.push(key);
                }
            },
            cx,
        );
    }
    pub(crate) fn remove_added_repo(&mut self, provider: &str, repo: &str, cx: &mut Context<Self>) {
        let key = queue::repo_key(provider, repo);
        self.change_git_prefs(
            |prefs| {
                prefs
                    .added_repos
                    .retain(|a| queue::repo_key(&a.provider, &a.repo) != key)
            },
            cx,
        );
    }

    /// Looks up repositories for "Add a repo" once the query settles.
    pub(super) fn search_repos(&mut self, cx: &mut Context<Self>) {
        let query = self.repo_search_input.read(cx).content.trim().to_string();
        if query == self.repo_search.query {
            return;
        }
        self.repo_search.query = query.clone();
        self.repo_search.generation += 1;
        let generation = self.repo_search.generation;
        self.repo_search.error = None;
        if query.chars().count() < 2 {
            self.repo_search.hits.clear();
            self.repo_search.searching = false;
            cx.notify();
            return;
        }
        self.repo_search.searching = true;
        let service = self.service.clone();
        #[cfg(test)]
        let canned = self.canned_repo_hits.clone();
        let task = cx.background_executor().spawn(async move {
            #[cfg(test)]
            if let Some(hits) = canned {
                let needle = query.to_lowercase();
                return Ok(hits
                    .into_iter()
                    .filter(|hit| hit.repo.to_lowercase().contains(&needle))
                    .collect());
            }
            service.search_repos(&query)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.repo_search.generation != generation {
                    return;
                }
                this.repo_search.searching = false;
                match result {
                    Ok(hits) => this.repo_search.hits = hits,
                    Err(error) => {
                        this.repo_search.hits.clear();
                        this.repo_search.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// "Add" on a search hit: the repository joins your repos.
    pub(crate) fn add_searched_repo(&mut self, hit: RepoHit, cx: &mut Context<Self>) {
        self.repo_search_input
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.repo_search = RepoSearch::default();
        self.add_repo(hit.provider, hit.repo, cx);
    }
}
